use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{DateTime, Local};
use serde_json::{json, Value};
use tauri::{
    image::Image,
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIcon, TrayIconBuilder},
    utils::config::WindowEffectsConfig,
    window::{Effect, EffectState},
    ActivationPolicy, AppHandle, Emitter, Manager, RunEvent, TitleBarStyle, WebviewUrl, WebviewWindowBuilder, WindowEvent,
    Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Notify;

mod context;
mod desktop;
mod hooks;
mod icons;
mod notifications;
mod plans;
mod sessions;
mod transcripts;
mod wallpaper;

/**************************
 * A P P  C O N S T A N T S
 *************************/

// The choices for "Check usage every" in Settings, in seconds
const INTERVALS: [u64; 5] = [30, 60, 300, 600, 1800];

// Used on the first launch, or if the saved interval is no longer one of the options above
const DEFAULT_INTERVAL: u64 = 60;

// The choices for what the menu bar shows, from "Shows" in Settings, which has their labels. The first one is the
// default. "rings_model" adds a third ring for a model's own weekly limit, like Fable's, and Settings only offers it
// while Claude reports one.
const TITLE_MODES: [&str; 7] = ["both", "5h", "7d", "5h_reset", "ring", "rings", "rings_model"];

// The thresholds for the context alert in Settings, as shares of a conversation's context window
const CONTEXT_LEVELS: [u32; 5] = [50, 60, 70, 80, 90];

// The thresholds for the 5-hour and weekly alerts in Settings
const ALERT_LEVELS: [u32; 6] = [50, 60, 70, 80, 90, 95];

// The choices for "Once usage passes", for the note Claude gets near a limit
const NEAR_LEVELS: [u32; 6] = [70, 75, 80, 85, 90, 95];

// The choices for "Only after runs of at least", in seconds
const MIN_RUNS: [u64; 4] = [0, 60, 300, 900];

// The choices for how long Headroom holds a permission request for an answer before Claude Code asks in the terminal,
// in seconds. The default is two minutes.
const APPROVAL_HOLDS: [u64; 5] = [30, 60, 120, 300, 600];

// The choices for when the daily recap goes out, as 24-hour times
const RECAP_TIMES: [&str; 4] = ["17:00", "18:00", "19:00", "21:00"];

// A break between replies longer than this doesn't count toward time spent in the recap
const BREAK: chrono::Duration = chrono::Duration::minutes(5);

// Only events this recent get a notification. Headroom reads the whole log when it starts, and nobody wants to hear
// about everything that happened while it wasn't running.
const FRESH: chrono::Duration = chrono::Duration::seconds(30);

// Once any limit reaches these, the "✻" in the menu bar turns into an orange or red dot. The menu bar title can only
// be plain text, so a colored emoji is the one way we have to add color to it.
const WARN_AT: f64 = 80.0;
const CRITICAL_AT: f64 = 95.0;

// The usage ring, which sits to the left of the title and fills up clockwise from the top as the highest limit gets
// used. It replaces the "✻" and colored dots when it's on. Unlike the title, the ring is an image, so it can be any
// color. The track is a see-through gray so it works on both light and dark menu bars. Colors are RGBA.
//
// "Two rings" puts the 5-hour limit in its own ring, in teal so the two are easy to tell apart at a glance, next to
// the weekly one.
const RING_SIZE: u32 = 36; // pixels, drawn for a Retina menu bar (18 points)
const RING_GAP: u32 = 8;
const RING_OUTER: f64 = 15.0;
const RING_INNER: f64 = 11.5;
const RING_TRACK: [u8; 4] = [142, 142, 147, 140];
const RING_FILL: [u8; 4] = [217, 119, 87, 255];
const RING_FIVE_HOUR: [u8; 4] = [52, 170, 160, 255];
const RING_CRITICAL: [u8; 4] = [229, 72, 77, 255];
const RING_MODEL: [u8; 4] = [143, 107, 208, 255];
// The pace arrows: orange when a limit is ahead of its window, green when it's behind (Settings' --orange and --green)
const PACE_AHEAD: [u8; 4] = [232, 145, 45, 255];
const PACE_BEHIND: [u8; 4] = [58, 167, 87, 255];

// The limits we track. Everything that's per limit (the menu line, the alert, the saved setting) is built from this
// list, in this order. The Opus and Sonnet limits are only reported for some plans, so their lines are only in the
// menu while the API is sending them, and they don't get alerts.
const LIMITS: [LimitInfo; 4] = [
    LimitInfo { key: "5h", label: "5-hour", json_key: "five_hour", optional: false },
    LimitInfo { key: "7d", label: "Weekly", json_key: "seven_day", optional: false },
    LimitInfo { key: "7d_opus", label: "Opus weekly", json_key: "seven_day_opus", optional: true },
    LimitInfo { key: "7d_sonnet", label: "Sonnet weekly", json_key: "seven_day_sonnet", optional: true },
];

// Opened by "Open Usage Page"
const USAGE_PAGE: &str = "https://claude.ai/settings/usage";

// How often we check GitHub for a new release, on top of the check when the app starts
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

// The keychain service that the token from Settings is saved under. This is also the name used in the README for
// removing the token by hand with `security delete-generic-password`.
const KEYCHAIN_SERVICE: &str = "Headroom";

// Both of these point the user to Settings, since a saved token is the fix either way
const TOKEN_REJECTED: &str = "Saved token rejected, set a new one in Settings";
const LOGIN_EXPIRED: &str = "Claude Code login expired, set a token in Settings";

/******************
 * A P P  S T A T E
 *****************/

#[derive(Clone, Copy)]
struct LimitInfo {
    /// Used in menu item IDs, setting names, and the `anthropic-ratelimit-unified-<key>-*` headers.
    key: &'static str,
    label: &'static str,

    /// The field for this limit in the /api/oauth/usage response.
    json_key: &'static str,

    /// Is this limit only reported for some plans?
    optional: bool,
}

/// Usage for one limit.
#[derive(Clone)]
struct Window {
    /// How much of the limit has been used, from 0 to 100.
    pct: f64,
    resets_at: Option<DateTime<Local>>,
}

/// Usage for each limit, in the same order as `LIMITS`. Any of them can be missing if the response didn't include it.
type Usage = [Option<Window>; 4];

/// A weekly limit for one model that isn't in `LIMITS`, like Fable's on some plans. Claude adds these as it adds
/// models, so they're picked up by the model's name rather than listed. They're shown in the menu and Settings, with
/// no alerts.
#[derive(Clone)]
struct ModelLimit {
    /// "7d_fable", the same shape as the listed ones.
    key: String,
    /// "Fable weekly".
    label: String,
    window: Window,
}

/// The model in a rate limit header's name, like "fable" in "7d_fable", if it's one we don't list already. Only single
/// words: other names with the same shape aren't models.
fn unlisted_model(name: &str) -> Option<String> {
    let model = name.strip_prefix("7d_")?;
    let known = LIMITS.iter().any(|l| l.key == format!("7d_{model}"));
    (!known && !model.is_empty() && model.chars().all(|c| c.is_ascii_alphabetic())).then(|| model.to_string())
}

/// A model's weekly limit, from the model's name as Claude shows it ("Fable").
fn model_limit(name: &str, window: Window) -> ModelLimit {
    let mut chars = name.chars();
    let name: String = chars.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(chars).collect();
    let key: String = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    ModelLimit { key: format!("7d_{key}"), label: format!("{name} weekly"), window }
}

/// The limits in a /api/oauth/usage response: the ones in `LIMITS`, from their own fields, and each model's own weekly
/// limit, which only comes in the `limits` list, as a weekly limit "scoped" to the model.
fn parse_usage(v: &Value) -> (Usage, Vec<ModelLimit>) {
    let when = |t: &Value| {
        t.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Local))
    };
    // `utilization` is already a percentage here, and `resets_at` is an RFC 3339 timestamp. Limits that don't apply to
    // the user's plan come back as null, which gives us `None`.
    let usage = LIMITS.map(|limit| {
        let w = &v[limit.json_key];
        Some(Window { pct: whole_pct(w["utilization"].as_f64()?), resets_at: when(&w["resets_at"]) })
    });
    let models = v["limits"].as_array().into_iter().flatten().filter_map(|l| {
        let name = l["scope"]["model"]["display_name"].as_str()?;
        if l["kind"] != "weekly_scoped" || name.trim().is_empty() {
            return None;
        }
        let window = Window { pct: whole_pct(l["percent"].as_f64()?), resets_at: when(&l["resets_at"]) };
        let model = model_limit(name.trim(), window);
        // Opus and Sonnet have fields of their own too, so they aren't counted twice
        let listed = LIMITS.iter().zip(&usage).any(|(info, w)| info.key == model.key && w.is_some());
        (!listed).then_some(model)
    });
    let models = models.collect();
    (usage, models)
}

/// Where the token came from. This decides which endpoint we call.
enum Source {
    Saved,
    ClaudeCode,
}

/// Why a check failed. `retry_after` is set when we were rate limited and the response told us how long to wait.
struct FetchError {
    message: String,
    retry_after: Option<Duration>,
}

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self { message, retry_after: None }
    }
}

impl From<&str> for FetchError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

/// Where the alert stands for one of the limits.
struct AlertState {
    /// The percentage picked in Settings, or `None` when it's set to "Off".
    threshold: Option<u32>,

    /// Have we already sent the notification for this crossing? This keeps us from sending the same alert on every
    /// check while usage stays above the threshold.
    fired: bool,
}

/// Which Claude Code session notifications are on, from Settings → Alerts.
struct SessionAlerts {
    /// A session asked permission or asked a question.
    answer: bool,
    /// A session finished its turn.
    done: bool,
    /// Skip "finished" while the app the session runs in is in front, since the user is probably watching it.
    only_away: bool,
    /// Skip "finished" for turns shorter than this many seconds.
    min_run: u64,
    /// A usage limit reset while sessions were stopped at it.
    reset: bool,
}

/// How the note Claude gets near a limit is set up.
struct NearLimit {
    on: bool,
    /// The threshold, from `NEAR_LEVELS`.
    at: u32,
    /// Send it after a working session's next tool call too, rather than only with the user's next message.
    right_away: bool,
    /// The user's own words for what Claude should do, or empty for the default.
    message: String,
}

impl NearLimit {
    fn for_hooks(&self) -> hooks::NoteSettings {
        let near = self.on.then_some(self.at);
        hooks::NoteSettings { near, right_away: self.right_away, message: self.message.clone() }
    }
}

/// Today's use of Claude Code, from the transcripts, for the daily recap.
#[derive(Default)]
struct Today {
    sessions: usize,
    /// Time spent, not counting breaks longer than `BREAK`.
    active: chrono::Duration,
    /// The projects that took the most, biggest first, at most three.
    projects: Vec<String>,
}

/// Everything in the menu that belongs to one limit.
struct Limit {
    info: LimitInfo,

    /// The disabled line at the top of the menu, e.g. "5-hour: 28%, resets 3:10 PM".
    line: MenuItem<Wry>,

    /// Is `line` currently in the menu? This is always true for limits that aren't `optional`.
    shown: Mutex<bool>,

    /// Optional limits don't get alerts, so theirs is always off.
    alert: Mutex<AlertState>,
}

/// Everything the refresh loop and the menu handlers share.
struct State {
    /// Used to send notifications, check for updates, and restart after an update.
    app: AppHandle,

    /// Cuts the current wait short, e.g. when the user clicks "Refresh Now" or changes a setting.
    wake: Notify,

    /// The number of seconds between checks, picked from "Check usage every".
    interval: Mutex<u64>,

    /// Which of `TITLE_MODES` the menu bar title uses, picked from "Shows".
    title_mode: Mutex<&'static str>,

    /// Is the usage ring on? Picked from "Usage ring".
    show_ring: Mutex<bool>,

    /// Where the settings are saved. On macOS that's ~/Library/Application Support/io.github.dylan-hepworth.headroom.
    config_dir: PathBuf,

    /// The menu bar item itself, and the menu it opens. We hold on to the menu so the Opus, Sonnet and per-model lines
    /// can be added and removed.
    tray: TrayIcon,
    menu: Menu<Wry>,

    /// The disabled line under the usage lines. It shows when we last checked, or why the last check failed.
    status: MenuItem<Wry>,
    limits: [Limit; 4],

    /// The usage from the last successful check. The title is redrawn from this between checks so the time until
    /// reset keeps counting down. It's `None` after a failed check, so the "⚠" stays up until the next good one.
    last_usage: Mutex<Option<Usage>>,
    /// The per-model weekly limits from the last good check that aren't in `LIMITS`.
    model_limits: Mutex<Vec<ModelLimit>>,
    /// Their line in the menu, under the others, e.g. "Fable weekly: 16%, resets 2:00 PM". Only in the menu while
    /// there are any.
    model_line: MenuItem<Wry>,
    model_line_shown: Mutex<bool>,
    /// The per-model limits read with the Claude Code login alongside a saved token (see `limits_from_login`).
    login_check: Mutex<LoginCheck>,

    /// When we're allowed to check again after being rate limited. We skip checks until then, including
    /// "Refresh Now", since checking early would only get rate limited again.
    retry_at: Mutex<Option<Instant>>,

    /// Are our Claude Code hooks installed? Picked from "Hooks" in Settings.
    hooks_on: Mutex<bool>,

    /// The Claude Code sessions we've heard about through the hooks.
    sessions: Mutex<sessions::Sessions>,

    /// Does the menu bar show how many sessions are waiting on the user?
    waiting_count: Mutex<bool>,

    session_alerts: Mutex<SessionAlerts>,

    /// The note Claude gets once usage passes a threshold, from Settings → Hooks.
    near_limit: Mutex<NearLimit>,

    /// What Claude Code's transcripts say about recent use, and each project's share of it in the current 5-hour
    /// window, in percent, biggest first.
    transcripts: Mutex<transcripts::Transcripts>,
    shares: Mutex<Vec<(String, f64)>>,

    /// Approving permission requests from Headroom: whether it's on, and how long to hold a request (from
    /// `APPROVAL_HOLDS`).
    approvals: Mutex<(bool, u64)>,
    /// The requests the popover has already opened for, so closing it keeps it closed until a new one comes in.
    seen_requests: Mutex<HashSet<String>>,
    /// Does the popover open by itself when a session needs an answer? From Settings → Hooks.
    popover_auto: Mutex<bool>,
    /// Requests show in a few lines each, rather than on the whole card.
    compact_cards: Mutex<bool>,
    /// "Have Claude ask what's next": Claude ends each turn by asking, in the popover (see hooks.rs).
    ask_next: Mutex<bool>,
    /// Hands-free, with it: each question comes with what Claude said that turn, to read in the popover.
    hands_free: Mutex<bool>,
    /// Show whether each limit is ahead of or behind its window, as an arrow in the menu bar and in Settings' rings.
    pace_arrows: Mutex<bool>,
    /// Each limit's pace when the title was last drawn, so Settings hears when an arrow changes between checks.
    paces: Mutex<Vec<Option<Pace>>>,
    /// Leave alerts on screen until they're closed ("Keep alerts on screen"), rather than taking them away.
    persistent_alerts: Mutex<bool>,
    /// Alert when a conversation's context passes this share of its window, if on.
    context_alert: Mutex<Option<u32>>,
    contexts: Mutex<context::Contexts>,
    /// What the Claude app says about its chats, for the menu bar's dots.
    desktop: Mutex<desktop::Desktop>,
    /// What each recent notification opens when clicked, by its ID, title, and text (see `notify_to`).
    clicks: Mutex<std::collections::VecDeque<(String, String, String, OnClick)>>,
    /// "Show Waiting Requests", in the menu.
    show_requests: MenuItem<Wry>,
    /// "Pause Alerts and Requests", in the menu, and its "Resume Now".
    pause_menu: Submenu<Wry>,
    resume_item: MenuItem<Wry>,
    /// While paused, no alerts go out and no requests are held for the popover. Until a time, or until resumed.
    paused: Mutex<Option<Paused>>,
    /// Hands-free: the popover is showing the list of chats, opened by clicking the menu bar item (see `tray_clicked`).
    list_mode: Mutex<bool>,
    /// The app in front, as last written for the hooks, so they hear when it changes (see hooks.rs `looked_at`).
    told_front: Mutex<Option<String>>,
    /// Each chat's icon (see icons.rs).
    icons: Mutex<icons::Icons>,

    /// The daily recap: whether it's on, and when it goes out (from `RECAP_TIMES`). Also today's use so far.
    recap: Mutex<(bool, &'static str)>,
    today: Mutex<Today>,

    /// When the 5-hour and weekly limits were last due to reset. When a check comes back with a later time, a new
    /// window has started, which is how we know a limit just reset.
    resets: Mutex<[Option<DateTime<Local>>; 2]>,

    /// Do we check for updates on our own, at launch and once a day? "Check for Updates…" works either way.
    auto_update: Mutex<bool>,

    /// "Check for Updates…", which turns into "Install Update (x.y.z)" once we've found one.
    update_item: MenuItem<Wry>,
    pending_update: Mutex<Option<Update>>,
}

/**************************
 * T O K E N  S T O R A G E
 *************************/

/// Get the keychain entry that holds the token from Settings.
fn saved_token_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, "token").map_err(|e| e.to_string())
}

/// `claude setup-token` prints a long token that Terminal wraps across lines. Copying it from there usually picks up a
/// space or newline in the middle, and the API rejects the whole thing as an invalid token.
fn strip_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Read the saved token, if there is one.
fn saved_token() -> Option<String> {
    let token = strip_whitespace(&saved_token_entry().ok()?.get_password().ok()?);
    (!token.is_empty()).then_some(token)
}

/// Called by the settings window, so it can show whether a token is saved.
#[tauri::command]
fn has_token() -> bool {
    saved_token().is_some()
}

/// Called when the user saves a token in Settings.
#[tauri::command]
fn save_token(token: String, state: tauri::State<Arc<State>>) -> Result<(), String> {
    let token = strip_whitespace(&token);
    if token.is_empty() {
        return Err("Token is empty".into());
    }
    saved_token_entry()?.set_password(&token).map_err(|e| e.to_string())?;
    token_changed(&state);
    Ok(())
}

/// Called when the user removes the token in Settings. Once the saved token is gone, we fall back to the Claude Code
/// login.
#[tauri::command]
fn clear_token(state: tauri::State<Arc<State>>) -> Result<(), String> {
    // Not having an entry to delete isn't an error. It just means there was no token saved to begin with.
    match saved_token_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.to_string()),
    }
    token_changed(&state);
    Ok(())
}

/// Check again right away with the new token. Any rate limit we were waiting out belonged to the old token (and
/// usually the other endpoint), so it doesn't apply anymore.
fn token_changed(state: &State) {
    *state.retry_at.lock().unwrap() = None;
    state.wake.notify_one();
    changed(state);
}

/// Get the token to use for this check. A token from Settings always wins. Otherwise we fall back to the login
/// Claude Code keeps in the keychain.
fn read_token() -> Result<(String, Source), String> {
    if let Some(token) = saved_token() {
        return Ok((token, Source::Saved));
    }
    claude_code_login().map(|token| (token, Source::ClaudeCode)).map_err(|e| e.message.to_string())
}

/// Why the Claude Code login couldn't be used, and whether it's worth trying again later.
struct LoginError {
    message: &'static str,
    /// False when the user turned down macOS's request for keychain access, so we don't keep asking.
    try_again: bool,
}

/// The access token from the login Claude Code keeps in the keychain, if it hasn't expired.
fn claude_code_login() -> Result<String, LoginError> {
    let fail = |message| LoginError { message, try_again: true };
    // Claude Code saves its login as JSON under "Claude Code-credentials". The first time we read it, macOS asks the
    // user to allow access.
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .map_err(|_| fail("Can't read the keychain"))?;
    if !out.status.success() {
        // 44 is "not found". Anything else is most likely the user saying no to the keychain prompt.
        let missing = out.status.code() == Some(44);
        return Err(LoginError { message: "No Claude Code login found", try_again: missing });
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| fail("Unreadable credentials"))?;
    let oauth = &v["claudeAiOauth"];

    // Claude Code refreshes this login on its own while it's in use, but we don't. Refreshing it from here would swap
    // out the refresh token Claude Code is holding and sign it out. So if it has expired, we point the user to
    // Settings rather than sending a request we already know will fail.
    let expires_ms = oauth["expiresAt"].as_i64().unwrap_or(i64::MAX);
    if expires_ms < chrono::Utc::now().timestamp_millis() {
        return Err(fail(LOGIN_EXPIRED));
    }
    oauth["accessToken"].as_str().map(String::from).ok_or_else(|| fail("No access token, set a token in Settings"))
}

/****************************
 * F E T C H I N G  U S A G E
 ***************************/

/// Send a request with the auth headers both endpoints need.
async fn send(req: reqwest::RequestBuilder, token: &str) -> Result<reqwest::Response, String> {
    req.bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|_| "Network error".to_string())
}

/// The error for a 429, with however long the `retry-after` header says to wait.
fn rate_limited(resp: &reqwest::Response) -> FetchError {
    let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()?.parse().ok());
    FetchError { message: "Rate limited".into(), retry_after: retry_after.map(Duration::from_secs) }
}

/// Turn a unix timestamp in seconds into local time.
fn local_time(secs: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(secs, 0).map(|d| d.with_timezone(&Local))
}

/// Get usage with the Claude Code login from /api/oauth/usage.
async fn fetch_usage_from_endpoint(
    client: &reqwest::Client,
    token: &str,
) -> Result<(Usage, Vec<ModelLimit>), FetchError> {
    let resp = send(client.get("https://api.anthropic.com/api/oauth/usage"), token).await?;
    match resp.status().as_u16() {
        200 => {}
        401 | 403 => return Err(LOGIN_EXPIRED.into()),
        // /api/oauth/usage rate limits a lot sooner than /v1/messages does, and when it does, it usually asks us to
        // wait a full hour
        429 => return Err(rate_limited(&resp)),
        code => return Err(format!("HTTP {code}").into()),
    }
    let v: Value = resp.json().await.map_err(|_| "Bad response")?;
    Ok(parse_usage(&v))
}

/// Get usage with a token from Settings. Tokens from `claude setup-token` don't have the `user:profile` scope
/// that /api/oauth/usage requires (it comes back as a 403), but every /v1/messages response carries the same numbers
/// in its rate limit headers. So we send the smallest request we can, a single output token from Haiku, and read the
/// headers off of that. Each check does count a tiny amount against the user's limit, which is why the README
/// suggests an interval of 5 minutes or longer.
async fn fetch_usage_from_headers(
    client: &reqwest::Client,
    token: &str,
) -> Result<(Usage, Vec<ModelLimit>), FetchError> {
    let body = json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "hi" }]
    });
    let req = client
        .post("https://api.anthropic.com/v1/messages")
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    let resp = send(req, token).await?;

    // Unlike /api/oauth/usage, the headers give utilization as a fraction (0.28) and the reset time in unix seconds.
    // Limits that don't apply to the user's plan just don't have headers.
    let header = |key: &str, field: &str| {
        let name = format!("anthropic-ratelimit-unified-{key}-{field}");
        resp.headers().get(name)?.to_str().ok()?.parse::<f64>().ok()
    };
    let window = |key: &str| {
        Some(Window {
            pct: whole_pct(header(key, "utilization")? * 100.0),
            resets_at: header(key, "reset").and_then(|secs| local_time(secs as i64)),
        })
    };
    let usage = LIMITS.map(|limit| window(limit.key));
    let models = resp.headers().keys().filter_map(|name| {
        let key = name.as_str().strip_prefix("anthropic-ratelimit-unified-")?.strip_suffix("-utilization")?;
        Some(model_limit(&unlisted_model(key)?, window(key)?))
    });
    let models: Vec<ModelLimit> = models.collect();

    // Check for the headers before looking at the status code. Once the user hits their limit, /v1/messages returns a
    // 429 but still sends the headers, and that's exactly when we want to be showing the numbers.
    if usage.iter().any(Option::is_some) {
        return Ok((usage, models));
    }
    match resp.status().as_u16() {
        401 | 403 => Err(TOKEN_REJECTED.into()),
        429 => Err(rate_limited(&resp)),
        code => Err(format!("HTTP {code}").into()),
    }
}

/// Get the current usage from whichever endpoint works with the token we have.
async fn fetch_usage(client: &reqwest::Client, state: &State) -> Result<(Usage, Vec<ModelLimit>), FetchError> {
    match read_token()? {
        (token, Source::Saved) => {
            let (usage, mut models) = fetch_usage_from_headers(client, &token).await?;
            models.extend(limits_from_login(client, state, &usage).await);
            Ok((usage, models))
        }
        (token, Source::ClaudeCode) => fetch_usage_from_endpoint(client, &token).await,
    }
}

/// How often the Claude Code login is used for the per-model limits, alongside a saved token. Claude's usage page
/// rate limits a lot sooner than the token's check does.
const LOGIN_CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

/// The per-model limits Claude's usage page lists (Fable's weekly limit, say, or Opus and Sonnet on plans that have
/// them), and when to look again.
#[derive(Default)]
struct LoginCheck {
    next: Option<Instant>,
    limits: Vec<ModelLimit>,
}

/// With a token from Settings, the check only reports the 5-hour and weekly limits: they come from the rate limit
/// headers on a Haiku request, and the per-model ones aren't there. Claude's usage page has them, and the Claude Code
/// login can read it, so while that login is fresh we ask with it too, every few minutes. It expires every few hours
/// when Claude Code isn't in use, and we don't refresh it (see `claude_code_login`), so these limits come and go with
/// it. Any limit the token's check already reported is left to it.
async fn limits_from_login(client: &reqwest::Client, state: &State, usage: &Usage) -> Vec<ModelLimit> {
    let due = state.login_check.lock().unwrap().next.is_none_or(|t| Instant::now() >= t);
    if due {
        let now = Instant::now();
        let (next, limits) = match claude_code_login() {
            Err(LoginError { try_again: false, .. }) => (None, Some(vec![])),
            Err(_) => (Some(now + LOGIN_CHECK_EVERY), Some(vec![])),
            Ok(token) => match fetch_usage_from_endpoint(client, &token).await {
                Ok((listed, mut models)) => {
                    // Opus and Sonnet are listed limits, but only this page reports them to a token user
                    let named = LIMITS.iter().zip(listed).skip(2).filter_map(|(info, w)| {
                        Some(ModelLimit { key: info.key.into(), label: info.label.into(), window: w? })
                    });
                    models.splice(0..0, named);
                    (Some(now + LOGIN_CHECK_EVERY), Some(models))
                }
                // Keep what we had until it's allowed to ask again
                Err(FetchError { retry_after: Some(wait), .. }) => (Some(now + wait), None),
                Err(_) => (Some(now + LOGIN_CHECK_EVERY), Some(vec![])),
            },
        };
        let mut check = state.login_check.lock().unwrap();
        // Turned down at the keychain prompt: not again until Headroom restarts
        check.next = Some(next.unwrap_or(now + Duration::from_secs(365 * 24 * 60 * 60)));
        if let Some(limits) = limits {
            check.limits = limits;
        }
    }
    let reported: Vec<&str> = LIMITS.iter().zip(usage).filter(|(_, w)| w.is_some()).map(|(l, _)| l.key).collect();
    let check = state.login_check.lock().unwrap();
    check.limits.iter().filter(|m| !reported.contains(&m.key.as_str())).cloned().collect()
}

/// Check usage on the chosen interval until the app quits. After a 429 we hold off for however long the response
/// asked, even if that's longer than the interval.
async fn refresh_loop(state: Arc<State>) {
    // Give up on a request after 15 seconds so a stalled connection doesn't hold up the next check
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap();

    loop {
        let is_rate_limited = state.retry_at.lock().unwrap().is_some_and(|t| Instant::now() < t);
        if !is_rate_limited {
            match fetch_usage(&client, &state).await {
                Ok((usage, models)) => {
                    *state.model_limits.lock().unwrap() = models;
                    show_usage(&state, usage)
                }
                Err(FetchError { message, retry_after: None }) => show_error(&state, &message),
                Err(FetchError { message, retry_after: Some(wait) }) => {
                    *state.retry_at.lock().unwrap() = Some(Instant::now() + wait);
                    let at = Local::now() + chrono::Duration::from_std(wait).unwrap_or_default();
                    show_error(&state, &format!("{message}, trying again at {}", at.format("%-I:%M %p")));
                }
            }
        }

        let interval = Duration::from_secs(*state.interval.lock().unwrap());
        let retry_at = *state.retry_at.lock().unwrap();
        let backoff = retry_at.map_or(Duration::ZERO, |t| t.saturating_duration_since(Instant::now()));
        tokio::select! {
            _ = tokio::time::sleep(interval.max(backoff)) => {}
            _ = state.wake.notified() => {}
        }
    }
}

/********************************
 * H E L P E R  F U N C T I O N S
 *******************************/

/// Format a time, e.g. "3:10 PM", with the day only when it isn't today ("Sat 3:10 PM").
fn fmt_when(t: DateTime<Local>) -> String {
    let fmt = if t.date_naive() == Local::now().date_naive() { "%-I:%M %p" } else { "%a %-I:%M %p" };
    t.format(fmt).to_string()
}

/// Format a reset time for the menu and the alert text, e.g. ", resets 3:10 PM". The day only gets added when the
/// reset isn't today. When there's no reset time we return an empty string, so callers can append it either way.
fn fmt_reset(t: Option<DateTime<Local>>) -> String {
    let Some(t) = t else { return String::new() };
    format!(", resets {}", fmt_when(t))
}

/// Format the time until a limit resets for the menu bar, e.g. "2h 14m" or "9m".
fn fmt_time_left(w: &Option<Window>) -> String {
    let Some(t) = w.as_ref().and_then(|w| w.resets_at) else { return "–".into() };
    fmt_minutes((t - Local::now()).num_minutes().max(0))
}

/// Format a number of minutes as hours and minutes, e.g. "2h 14m", or only minutes when it's under an hour.
fn fmt_minutes(mins: i64) -> String {
    if mins >= 60 {
        format!("{}h {}m", mins / 60, mins % 60)
    } else {
        format!("{mins}m")
    }
}

/// Round usage up to a whole percent, the way claude.ai shows it, so the two always agree. It's rounded to a thousandth
/// first, so float noise (0.76 * 100 is 76.00000000000001) doesn't tip a whole number up to the next one.
fn whole_pct(pct: f64) -> f64 {
    ((pct * 1000.0).round() / 1000.0).ceil()
}

/// Format a limit's percentage for the menu bar, or a dash when we don't have it.
fn fmt_pct(w: &Option<Window>) -> String {
    w.as_ref().map_or("–".into(), |w| format!("{:.0}%", w.pct))
}

/// Whether a limit is being used faster or slower than its window is going by.
#[derive(Clone, Copy, PartialEq)]
enum Pace {
    Ahead,
    Behind,
}

/// A limit's pace: ahead once its usage is more than 5 points past the share of its window that's gone, behind once
/// it's more than 5 short, and neither in between. Settings gets it from here too (see `limit_json`), so the two never
/// disagree.
fn pace(key: &str, w: &Window) -> Option<Pace> {
    let length = window_length(key);
    let left = w.resets_at? - Local::now();
    let gone = (length - left).num_seconds() as f64 / length.num_seconds() as f64 * 100.0;
    match w.pct - gone.clamp(0.0, 100.0) {
        d if d > 5.0 => Some(Pace::Ahead),
        d if d < -5.0 => Some(Pace::Behind),
        _ => None,
    }
}

/// The arrow for a limit's pace in the menu bar title, if pace arrows are on and it has one.
fn pace_mark(state: &State, key: &str, w: &Option<Window>) -> &'static str {
    let on = *state.pace_arrows.lock().unwrap();
    match w.as_ref().filter(|_| on).and_then(|w| pace(key, w)) {
        Some(Pace::Ahead) => "↑",
        Some(Pace::Behind) => "↓",
        None => "",
    }
}

/// Draw usage rings side by side, one for each `(pct, color, pace)`, each filled to its percentage, with the pace arrow
/// inside if there is one. A ring turns red at `CRITICAL_AT` whatever its color. Each pixel is sampled on a 4x4 grid
/// so the edges come out smooth.
fn ring_icon(rings: &[(f64, [u8; 4], Option<Pace>)]) -> Image<'static> {
    const SAMPLES: u32 = 4;
    let width = RING_SIZE * rings.len() as u32 + RING_GAP * (rings.len() as u32 - 1);
    let mut rgba = vec![0u8; (width * RING_SIZE * 4) as usize];
    for (i, &(pct, color, pace)) in rings.iter().enumerate() {
        let fill = if pct >= CRITICAL_AT { RING_CRITICAL } else { color };
        let filled = pct.clamp(0.0, 100.0) / 100.0;
        let left = i as u32 * (RING_SIZE + RING_GAP);
        let center = RING_SIZE as f64 / 2.0;
        for y in 0..RING_SIZE {
            for x in 0..RING_SIZE {
                // Add up how much of this pixel is covered by the filled part of the ring and by the track
                let (mut fill_hits, mut track_hits, mut arrow_hits) = (0, 0, 0);
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let dx = x as f64 + (sx as f64 + 0.5) / SAMPLES as f64 - center;
                        let dy = y as f64 + (sy as f64 + 0.5) / SAMPLES as f64 - center;
                        let r = dx.hypot(dy);
                        if pace.is_some_and(|pace| in_arrow(dx, dy, pace)) {
                            arrow_hits += 1;
                            continue;
                        }
                        if !(RING_INNER..=RING_OUTER).contains(&r) {
                            continue;
                        }
                        // How far around the ring this point is, from 0 at 12 o'clock going clockwise to 1
                        let around = (dx.atan2(-dy) / std::f64::consts::TAU).rem_euclid(1.0);
                        if around < filled {
                            fill_hits += 1;
                        } else {
                            track_hits += 1;
                        }
                    }
                }
                // Blend them by coverage. Track, fill and arrow never overlap, so their alphas add up.
                let total = (SAMPLES * SAMPLES) as f64;
                let arrow = if pace == Some(Pace::Ahead) { PACE_AHEAD } else { PACE_BEHIND };
                let fa = fill_hits as f64 / total * fill[3] as f64;
                let ta = track_hits as f64 / total * RING_TRACK[3] as f64;
                let aa = arrow_hits as f64 / total * arrow[3] as f64;
                let alpha = fa + ta + aa;
                let at = ((y * width + left + x) * 4) as usize;
                for c in 0..3 {
                    let sum = fill[c] as f64 * fa + RING_TRACK[c] as f64 * ta + arrow[c] as f64 * aa;
                    rgba[at + c] = if alpha > 0.0 { (sum / alpha).round() as u8 } else { 0 };
                }
                rgba[at + 3] = alpha.round().min(255.0) as u8;
            }
        }
    }
    Image::new_owned(rgba, width, RING_SIZE)
}

/// Is this point, measured from a ring's center in pixels, part of its pace arrow? The arrow is a triangle head and a
/// stem, pointing up when ahead and down when behind, sized to sit inside the ring.
fn in_arrow(dx: f64, dy: f64, pace: Pace) -> bool {
    // Measured as if pointing up; a down arrow is the same shape flipped
    let y = if pace == Pace::Ahead { dy } else { -dy };
    let head = (-6.5..=-1.0).contains(&y) && dx.abs() <= (y + 6.5) * 0.75;
    let stem = (-2.0..=6.5).contains(&y) && dx.abs() <= 1.25;
    head || stem
}

/// The rings the menu bar shows right now, if any, filled from `usage` (or empty when there isn't any yet).
fn tray_rings(state: &State, usage: Option<&Usage>) -> Option<Image<'static>> {
    let pct = |i: usize| usage.and_then(|u| u[i].as_ref()).map_or(0.0, |w| w.pct);
    // A single ring fills with the weekly limit, but turns red when any limit is nearly used up. Being at 96% of the
    // 5-hour limit matters whether or not the ring is showing it.
    let highest = usage.map_or(0.0, |u| u.iter().flatten().map(|w| w.pct).fold(0.0, f64::max));
    let arrows = *state.pace_arrows.lock().unwrap();
    let pace_of = |i: usize| usage.and_then(|u| u[i].as_ref()).filter(|_| arrows).and_then(|w| pace(LIMITS[i].key, w));
    let weekly = (pct(1), if highest >= CRITICAL_AT { RING_CRITICAL } else { RING_FILL }, pace_of(1));
    match *state.title_mode.lock().unwrap() {
        "rings" => Some(ring_icon(&[(pct(0), RING_FIVE_HOUR, pace_of(0)), (pct(1), RING_FILL, pace_of(1))])),
        "rings_model" => {
            let model = state.model_limits.lock().unwrap().first().cloned();
            let mut rings = vec![(pct(0), RING_FIVE_HOUR, pace_of(0)), (pct(1), RING_FILL, pace_of(1))];
            // Empty like the other two when the last check failed, rather than showing what it said before
            rings.extend(model.map(|m| match usage {
                Some(_) => (m.window.pct, RING_MODEL, pace(&m.key, &m.window).filter(|_| arrows)),
                None => (0.0, RING_MODEL, None),
            }));
            Some(ring_icon(&rings))
        }
        "ring" => Some(ring_icon(&[weekly])),
        _ if *state.show_ring.lock().unwrap() => Some(ring_icon(&[weekly])),
        _ => None,
    }
}

/// Read a saved setting. Each setting is its own small file in the config directory.
fn load_setting(config_dir: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(config_dir.join(key)).ok().map(|s| s.trim().to_string())
}

/// Save a setting to the config directory.
fn store_setting(state: &State, key: &str, value: &str) {
    // If the write fails, the worst case is the setting going back to its default on the next launch, so it's not
    // worth bothering the user about
    let _ = std::fs::create_dir_all(&state.config_dir);
    let _ = std::fs::write(state.config_dir.join(key), value);
}

/// Let the settings window know something changed, so it can show the latest. It's a no-op when the window isn't open.
fn changed(state: &State) {
    let _ = state.app.emit("changed", ());
}

/// Where clicking a notification takes you.
#[derive(Clone)]
enum OnClick {
    /// A Claude Code session: its request in the panel if it's waiting there, its chat if it's in the Claude app, or
    /// otherwise the app it runs in (a terminal, usually).
    Session { id: String, app: Option<String> },
    /// A pane in Settings.
    Pane(&'static str),
    /// A setting in Settings: its pane, and the row's title.
    Setting(&'static str, &'static str),
}

/// Send a notification that `on_click` says what to open for, unless alerts are paused. Whether it stays on screen is
/// up to macOS: each app's alert style is the user's choice, in System Settings. With "Keep alerts on screen" off,
/// Headroom takes each one away itself after a few seconds.
fn notify_to(state: &State, title: &str, body: &str, on_click: Option<OnClick>) {
    if is_paused(state) {
        return;
    }
    alert(state, title, body, on_click);
}

/// A notification about a session: its title starts with the chat's emoji, or its picture goes beside the text.
fn notify_about(state: &State, session: &str, title: &str, body: &str, on_click: Option<OnClick>) {
    if is_paused(state) {
        return;
    }
    let icon = icon_of(state, session).unwrap_or_default();
    let title = match icons::emoji(&icon) {
        Some(emoji) => format!("{emoji} {title}"),
        None => title.to_string(),
    };
    let picture = icons::picture_file(&icon, &state.config_dir);
    alert_with(state, &title, body, on_click, picture.as_deref());
}

/// Send a notification, paused or not: `notify_to` without the check, for the one the user asks for ("Send Test
/// Alert"). Each gets an ID, which is what a click comes back with, or its title and text when it goes through the
/// plugin (the last few dozen are plenty to remember).
fn alert(state: &State, title: &str, body: &str, on_click: Option<OnClick>) {
    alert_with(state, title, body, on_click, None);
}

fn alert_with(state: &State, title: &str, body: &str, on_click: Option<OnClick>, picture: Option<&Path>) {
    static SENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = format!("headroom-{}-{}", std::process::id(), SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    if let Some(on_click) = on_click {
        let mut clicks = state.clicks.lock().unwrap();
        clicks.push_back((id.clone(), title.to_string(), body.to_string(), on_click));
        if clicks.len() > 40 {
            clicks.pop_front();
        }
    }
    let sent = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
    let plugin = !notifications::available();
    if plugin {
        // Running with `tauri dev`, or a copy built without a Developer ID
        let _ = state.app.notification().builder().title(title).body(body).show();
    } else {
        notifications::send(&id, title, body, picture);
    }
    if *state.persistent_alerts.lock().unwrap() {
        return;
    }
    let (app, title, body) = (state.app.clone(), title.to_string(), body.to_string());
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let _ = app.run_on_main_thread(move || {
            if plugin {
                notifications::withdraw_plugin(&title, &body, sent);
            } else {
                notifications::withdraw(&id);
            }
        });
    });
}

/// A notification was clicked. Open whatever it was about.
fn notification_clicked(app: &AppHandle, clicked: notifications::Clicked) {
    let state = app.state::<Arc<State>>();
    let clicks = state.clicks.lock().unwrap();
    let on_click = clicks
        .iter()
        .rev()
        .find(|(id, title, body, _)| match &clicked {
            notifications::Clicked::Id(clicked) => id == clicked,
            notifications::Clicked::Text { title: t, body: b } => title == t && body == b,
        })
        .map(|(_, _, _, on_click)| on_click.clone());
    drop(clicks);
    match on_click {
        Some(OnClick::Pane(pane)) => open_settings(app, pane),
        Some(OnClick::Setting(pane, setting)) => open_settings_at(app, pane, Some(setting)),
        Some(OnClick::Session { id, app: host }) => {
            let held = held_requests(&state);
            let sessions = state.sessions.lock().unwrap();
            let waiting = held.iter().any(|r| sessions.session_of(r["id"].as_str().unwrap_or_default()) == Some(id.clone()));
            drop(sessions);
            if waiting {
                return show_popover(app, true);
            }
            open_session(&state, &id, host);
        }
        None => {}
    }
}

/// Bring a session up where it runs: its chat, if it's in the Claude app, or else the app it runs in (a terminal,
/// usually, which can only be brought to the front, not to the right tab).
fn open_session(state: &State, id: &str, host: Option<String>) {
    // The chat the hooks said it was, or failing that, the one the Claude app's files say
    let chat = state.sessions.lock().unwrap().host_chat(id);
    let chat = chat.or_else(|| state.desktop.lock().unwrap().chat_id(id));
    if let Some(chat) = chat {
        // The link the app's own Spotlight entries use
        let link = format!("claude://code/continue?session={chat}&source=desktop_action");
        let _ = Command::new("open").arg(link).spawn();
    } else if let Some(host) = host {
        let _ = Command::new("open").args(["-b", &host]).spawn();
    }
}

/// Open Headroom's own page in System Settings → Notifications, where its alert style is chosen.
#[tauri::command]
fn open_notification_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=io.github.dylan-hepworth.headroom")
        .spawn();
}

/*********************************
 * U P D A T I N G  T H E  M E N U
 ********************************/

/// Update the menu with the result of a good check, and send any alerts that are due.
fn show_usage(state: &State, usage: Usage) {
    for (i, (limit, w)) in state.limits.iter().zip(&usage).enumerate() {
        let label = limit.info.label;
        let text = match w {
            Some(w) => format!("{label}: {:.0}%{}", w.pct, fmt_reset(w.resets_at)),
            None => format!("{label}: n/a"),
        };
        let _ = limit.line.set_text(text);
        if limit.info.optional {
            set_line_shown(state, i, w.is_some());
        }
        if let Some(w) = w {
            check_alert(state, limit, w);
        }
    }
    show_model_line(state);
    let _ = state.status.set_text(format!("Updated {}", Local::now().format("%-I:%M:%S %p")));
    check_resets(state, &usage);
    let window_start = usage[0].as_ref().and_then(|w| w.resets_at).map(|t| t - window_length("5h"));
    track_today(state, &usage);
    *state.last_usage.lock().unwrap() = Some(usage);
    save_limits(state);
    read_transcripts(state, window_start.unwrap_or_else(|| Local::now() - window_length("5h")));
    update_title(state);
    changed(state);
}

/// Go over the transcripts for each project's share of Claude Code use in the current 5-hour window, and for today's
/// use for the recap. Reading them can take a moment, so it happens off to the side, and the settings window hears
/// about it when it's done.
fn read_transcripts(state: &State, window_start: DateTime<Local>) {
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<State>>();
        let midnight = start_of_today();
        let replies = state.transcripts.lock().unwrap().since(window_start.min(midnight));

        let in_window: Vec<_> = replies.iter().filter(|r| r.at >= window_start).collect();
        let total: f64 = in_window.iter().map(|r| r.weight).sum();
        let mut shares = share_by_project(&in_window);
        for share in &mut shares {
            share.1 = if total > 0.0 { share.1 / total * 100.0 } else { 0.0 };
        }
        *state.shares.lock().unwrap() = shares;

        let today: Vec<_> = replies.iter().filter(|r| r.at >= midnight).collect();
        let mut times: HashMap<&str, Vec<DateTime<Local>>> = HashMap::new();
        for reply in &today {
            times.entry(&reply.session).or_default().push(reply.at);
        }
        let active = times.values_mut().map(|list| active_time(list)).sum();
        let projects = share_by_project(&today).into_iter().take(3).map(|(p, _)| p).collect();
        *state.today.lock().unwrap() = Today { sessions: times.len(), active, projects };
        changed(&state);
    });
}

/// Time spent in one session, from the times of its replies: the gaps between them, leaving out breaks.
fn active_time(replies: &mut [DateTime<Local>]) -> chrono::Duration {
    replies.sort();
    replies.windows(2).map(|pair| pair[1] - pair[0]).filter(|gap| *gap <= BREAK).sum()
}

/// Add up the replies' weights by project, biggest first.
fn share_by_project(replies: &[&transcripts::Reply]) -> Vec<(String, f64)> {
    let mut by_project: HashMap<String, f64> = HashMap::new();
    for reply in replies {
        *by_project.entry(reply.project.clone()).or_default() += reply.weight;
    }
    let mut list: Vec<_> = by_project.into_iter().collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    list
}

fn start_of_today() -> DateTime<Local> {
    let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
    midnight.and_local_timezone(Local).earliest().unwrap_or_else(Local::now)
}

/// Keep track of the day's highest 5-hour usage, and where the weekly limit stood at the first check of the day, for
/// the recap. Both are saved as "<date> <percent>", so a restart partway through the day doesn't lose them.
fn track_today(state: &State, usage: &Usage) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let saved = |key: &str| saved_on(state, key, &today);
    if let Some(five) = usage[0].as_ref() {
        if saved("recap_peak").is_none_or(|peak| five.pct > peak) {
            store_setting(state, "recap_peak", &format!("{today} {}", five.pct));
        }
    }
    if let Some(weekly) = usage[1].as_ref() {
        if saved("recap_week_start").is_none() {
            store_setting(state, "recap_week_start", &format!("{today} {}", weekly.pct));
        }
    }
}

/// One of the percentages `track_today` saves, if it was saved on `date`.
fn saved_on(state: &State, key: &str, date: &str) -> Option<f64> {
    let value = load_setting(&state.config_dir, key)?;
    let (day, pct) = value.split_once(' ')?;
    (day == date).then(|| pct.parse().ok()).flatten()
}

/// The recap notification's title and body, e.g. "Today: 6h 20m across 11 sessions" and "Mostly headroom, blog and
/// site. Your 5-hour limit peaked at 91%, and the weekly limit went up 14%." `None` on a day with no Claude Code use.
fn recap_text(state: &State) -> Option<(String, String)> {
    let today = state.today.lock().unwrap();
    if today.sessions == 0 {
        return None;
    }
    let time = fmt_minutes(today.active.num_minutes());
    let sessions = if today.sessions == 1 { "1 session".to_string() } else { format!("{} sessions", today.sessions) };
    let title = format!("Today: {time} across {sessions}");

    let mut body = vec![];
    match today.projects.as_slice() {
        [] => {}
        [one] => body.push(format!("All in {one}.")),
        [rest @ .., last] => body.push(format!("Mostly {} and {last}.", rest.join(", "))),
    }
    drop(today);

    let date = Local::now().format("%Y-%m-%d").to_string();
    let saved = |key: &str| saved_on(state, key, &date);
    let weekly_now = state.last_usage.lock().unwrap().as_ref().and_then(|u| u[1].as_ref().map(|w| w.pct));
    let week = match (saved("recap_week_start"), weekly_now) {
        (Some(start), Some(now)) if now >= start => Some(format!("the weekly limit went up {:.0}%", now - start)),
        _ => None,
    };
    match (saved("recap_peak"), week) {
        (Some(peak), Some(week)) => body.push(format!("Your 5-hour limit peaked at {peak:.0}%, and {week}.")),
        (Some(peak), None) => body.push(format!("Your 5-hour limit peaked at {peak:.0}%.")),
        (None, Some(week)) => body.push(format!("Over the day, {week}.")),
        (None, None) => {}
    }
    Some((title, body.join(" ")))
}

/// Once a minute, see whether it's time for today's recap. It goes out once a day, at or after the time picked in
/// Settings, and not on days without any Claude Code use.
async fn recap_loop(state: Arc<State>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let (on, at) = *state.recap.lock().unwrap();
        let now = Local::now();
        let today = now.format("%Y-%m-%d").to_string();
        if !on || now.format("%H:%M").to_string().as_str() < at {
            continue;
        }
        if load_setting(&state.config_dir, "recap_sent").as_deref() == Some(today.as_str()) {
            continue;
        }
        if let Some((title, body)) = recap_text(&state) {
            notify_to(&state, &title, &body, Some(OnClick::Pane("recap")));
            store_setting(&state, "recap_sent", &today);
        }
    }
}

/// Leave the latest usage where the prompt hook can find it, for the note Claude gets near a limit.
fn save_limits(state: &State) {
    let note = state.near_limit.lock().unwrap().for_hooks();
    let usage = state.last_usage.lock().unwrap();
    let Some(usage) = usage.as_ref() else { return };
    let limits = state
        .limits
        .iter()
        .zip(usage)
        .filter_map(|(l, w)| {
            let w = w.as_ref()?;
            // "5-hour" reads fine in the note as it is; "Weekly" and "Opus weekly" read better in lowercase
            let label = if l.info.key == "5h" { l.info.label.to_string() } else { l.info.label.to_lowercase() };
            Some(json!({
                "label": label,
                "pct": w.pct,
                "resets": w.resets_at.map(fmt_when),
                "resets_at": w.resets_at.map(|t| t.timestamp_millis()),
            }))
        })
        .collect();
    hooks::write_limits(limits, &note);
}

/// Show why a check failed. There's no room for the reason in the menu bar, so we flag it there and put the reason
/// in the menu. The usage lines are left alone so the user still has the last numbers we got.
fn show_error(state: &State, message: &str) {
    *state.last_usage.lock().unwrap() = None;
    show_flag(state, "⚠");
    let _ = state.status.set_text(format!("{message} ({})", Local::now().format("%-I:%M:%S %p")));
    changed(state);
}

/// Redraw the menu bar title from the last good check, using whatever the menu bar is set to show.
fn update_title(state: &State) {
    // A copy, so the lock isn't held while the tray calls below wait on the main thread, which may want it
    let Some(usage) = state.last_usage.lock().unwrap().clone() else { return };
    let usage = &usage;
    // The arrows move with the clock, not just with each check
    let models = state.model_limits.lock().unwrap().clone();
    let paces: Vec<Option<Pace>> = (LIMITS.iter().zip(usage.iter()))
        .map(|(l, w)| w.as_ref().and_then(|w| pace(l.key, w)))
        .chain(models.iter().map(|m| pace(&m.key, &m.window)))
        .collect();
    if *state.paces.lock().unwrap() != paces {
        *state.paces.lock().unwrap() = paces;
        changed(state);
    }
    let rings = tray_rings(state, Some(usage));
    let has_rings = rings.is_some();
    // The icon goes first, so the menu bar item is sized for the new title with the new icon, not the old one
    let _ = state.tray.set_icon(rings);
    let text = title_text(state, usage);
    if has_rings || text.is_empty() {
        set_title(state, text);
    } else {
        let highest = usage.iter().flatten().map(|w| w.pct).fold(0.0, f64::max);
        let glyph = if highest >= CRITICAL_AT {
            "🔴"
        } else if highest >= WARN_AT {
            "🟠"
        } else {
            "✻"
        };
        set_title(state, format!("{glyph} {text}"));
    }
}

/// How many Claude Code sessions need an answer from the user, and how many are done and waiting for their next
/// message. Both are zero when the hooks or this part of the title are off.
fn waiting_counts(state: &State) -> (usize, usize) {
    if !*state.hooks_on.lock().unwrap() || !*state.waiting_count.lock().unwrap() {
        return (0, 0);
    }
    state.sessions.lock().unwrap().waiting()
}

/// Set the menu bar title to `text`, followed by the waiting sessions: a small yellow dot and a count for ones that
/// need an answer, and a blue one for ones that are done, the colors the Claude app uses in its list of chats.
///
/// A plain title can't have colors, so we set the plain version first (Tauri sizes the menu bar item from it) and then
/// swap in a styled copy on the status item's button. Emoji dots would be simpler, but they're big and shaded.
fn set_title(state: &State, text: String) {
    let (answer, done) = waiting_counts(state);
    let dots: Vec<(usize, [f64; 3])> =
        [(answer, [0.95, 0.71, 0.11]), (done, [0.23, 0.55, 0.96])].into_iter().filter(|(n, _)| *n > 0).collect();
    let paused = is_paused(state);
    let plain = std::iter::once(text.clone())
        .chain(dots.iter().map(|(n, _)| format!("●{n}")))
        .chain(paused.then(|| PAUSED_MARK.to_string()))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // An empty title rather than none: asked for no title, the tray library leaves the old one where it was
    let _ = state.tray.set_title(Some(plain));
    if dots.is_empty() && !text.contains(['↑', '↓']) {
        return;
    }

    let _ = state.tray.with_inner_tray_icon(move |tray| {
        use objc2::{AnyThread, MainThreadMarker, runtime::AnyObject};
        use objc2_app_kit::{
            NSBaselineOffsetAttributeName, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
        };
        use objc2_foundation::{NSAttributedString, NSDictionary, NSMutableAttributedString, NSNumber, NSString};

        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(button) = tray.ns_status_item().and_then(|item| item.button(mtm)) else { return };
        let Some(font) = button.font() else { return };
        let size = font.pointSize();
        let label = NSColor::labelColor();
        let dot_font = NSFont::systemFontOfSize(size * 0.62);
        let lift = NSNumber::new_f64(size * 0.14);

        let title = NSMutableAttributedString::new();
        let add = |s: &str, attrs: &NSDictionary<NSString, AnyObject>| unsafe {
            title.appendAttributedString(&NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &NSString::from_str(s),
                Some(attrs),
            ));
        };
        let normal = unsafe {
            NSDictionary::<NSString, AnyObject>::from_slices(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[&*font as &AnyObject, &*label as &AnyObject],
            )
        };
        // The text, with any pace arrows in their colors
        let arrow = |[r, g, b, _]: [u8; 4]| {
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0);
            unsafe {
                NSDictionary::<NSString, AnyObject>::from_slices(
                    &[NSFontAttributeName, NSForegroundColorAttributeName],
                    &[&*font as &AnyObject, &*color as &AnyObject],
                )
            }
        };
        let (ahead, behind) = (arrow(PACE_AHEAD), arrow(PACE_BEHIND));
        let mut run = String::new();
        for c in text.chars() {
            if c == '↑' || c == '↓' {
                add(&std::mem::take(&mut run), &normal);
                add(&c.to_string(), if c == '↑' { &ahead } else { &behind });
            } else {
                run.push(c);
            }
        }
        if !run.is_empty() {
            add(&run, &normal);
        }
        for (i, (count, [r, g, b])) in dots.iter().enumerate() {
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(*r, *g, *b, 1.0);
            let dot = unsafe {
                NSDictionary::<NSString, AnyObject>::from_slices(
                    &[NSFontAttributeName, NSForegroundColorAttributeName, NSBaselineOffsetAttributeName],
                    &[&*dot_font as &AnyObject, &*color as &AnyObject, &*lift as &AnyObject],
                )
            };
            if i > 0 || !text.is_empty() {
                add(" ", &normal);
            }
            add("●", &dot);
            add(&count.to_string(), &normal);
        }
        if paused {
            add(&format!(" {PAUSED_MARK}"), &normal);
        }
        button.setAttributedTitle(&title);
    });
}

/// The numbers in the menu bar title, e.g. "28% · 60%", using whatever the menu bar is set to show.
fn title_text(state: &State, usage: &Usage) -> String {
    let [five_hour, weekly, ..] = usage;
    let five = format!("{}{}", fmt_pct(five_hour), pace_mark(state, "5h", five_hour));
    let week = format!("{}{}", fmt_pct(weekly), pace_mark(state, "7d", weekly));
    match *state.title_mode.lock().unwrap() {
        "5h" => five,
        "7d" => week,
        "5h_reset" => format!("{five} · {}", fmt_time_left(five_hour)),
        "ring" | "rings" | "rings_model" => String::new(),
        _ => format!("{five} · {week}"),
    }
}

/// Add or remove an optional limit's line from the menu.
fn set_line_shown(state: &State, i: usize, show: bool) {
    let limit = &state.limits[i];
    let mut shown = limit.shown.lock().unwrap();
    if *shown == show {
        return;
    }
    if show {
        // The usage lines start at the top of the menu, so the line goes right after whichever lines above it are in
        // the menu right now
        let position = state.limits[..i].iter().filter(|l| *l.shown.lock().unwrap()).count();
        let _ = state.menu.insert(&limit.line, position);
    } else {
        let _ = state.menu.remove(&limit.line);
    }
    *shown = show;
}

/// Put the per-model limits in the menu, under the other limits, or take the line out when there aren't any.
fn show_model_line(state: &State) {
    let models = state.model_limits.lock().unwrap();
    let text = models
        .iter()
        .map(|m| format!("{}: {:.0}%{}", m.label, m.window.pct, fmt_reset(m.window.resets_at)))
        .collect::<Vec<_>>()
        .join("  ·  ");
    drop(models);
    let mut shown = state.model_line_shown.lock().unwrap();
    if text.is_empty() {
        if *shown {
            let _ = state.menu.remove(&state.model_line);
            *shown = false;
        }
        return;
    }
    let _ = state.model_line.set_text(text);
    if !*shown {
        let position = state.limits.iter().filter(|l| *l.shown.lock().unwrap()).count();
        let _ = state.menu.insert(&state.model_line, position);
        *shown = true;
    }
}

/// Send a notification if this limit has passed its alert threshold since the last check.
fn check_alert(state: &State, limit: &Limit, w: &Window) {
    let mut alert = limit.alert.lock().unwrap();
    let Some(t) = alert.threshold else { return };

    // Once usage drops back under the threshold (usually because the limit reset), the alert can go off again the
    // next time usage climbs past it. Until then, we only send it once.
    if w.pct < t as f64 {
        alert.fired = false;
    } else if !alert.fired {
        alert.fired = true;
        notify_to(
            state,
            &format!("Claude {} usage at {:.0}%", limit.info.label, w.pct),
            &format!("You've passed your {t}% alert{}.", fmt_reset(w.resets_at)),
            Some(OnClick::Pane("usage")),
        );
    }
}

/***************
 * U P D A T E S
 **************/

/// Check GitHub for a newer release. When `manual` is true (the user clicked "Check for Updates…"), we also let
/// them know when there's nothing new or the check failed. The automatic checks stay quiet about both.
async fn check_for_update(state: &State, manual: bool) {
    let result = match state.app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    match result {
        Ok(Some(update)) => {
            let _ = state.update_item.set_text(format!("Install Update ({})", update.version));

            // Only notify the first time we find a version, so the daily check doesn't keep repeating itself
            let mut pending = state.pending_update.lock().unwrap();
            if manual || pending.as_ref().is_none_or(|p| p.version != update.version) {
                let body = format!("Version {} is available. Click to install it.", update.version);
                notify_to(state, "Headroom", &body, Some(OnClick::Setting("general", "Updates")));
            }
            *pending = Some(update);
            drop(pending);
            changed(state);
        }
        Ok(None) if manual => {
            notify_to(state, "Headroom", "You're on the latest version.", Some(OnClick::Setting("general", "Updates")))
        }
        Err(_) if manual => {
            let body = "Couldn't check for updates. Try again later.";
            notify_to(state, "Headroom", body, Some(OnClick::Setting("general", "Updates")));
        }
        _ => {}
    }
}

/// Download and install the update we found, then restart into the new version.
async fn install_update(state: &State, update: Update) {
    let _ = state.update_item.set_text("Installing Update…");
    if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
        state.app.restart();
    }

    // Put the menu item back the way it was so the user can try again
    let _ = state.update_item.set_text(format!("Install Update ({})", update.version));
    let body = "Couldn't install the update. Try again later.";
    notify_to(state, "Headroom", body, Some(OnClick::Setting("general", "Updates")));
    *state.pending_update.lock().unwrap() = Some(update);
}

/***********************************
 * C H A N G I N G  S E T T I N G S
 **********************************/

/// Called when the user picks how often to check usage.
fn set_interval(state: &State, secs: u64) {
    *state.interval.lock().unwrap() = secs;
    store_setting(state, "interval", &secs.to_string());
    changed(state);

    // Wake the refresh loop so the new interval starts now, instead of after the old one runs out
    state.wake.notify_one();
}

/// Called when the user picks what the menu bar shows.
fn set_title_mode(state: &State, mode: &'static str) {
    *state.title_mode.lock().unwrap() = mode;
    store_setting(state, "title", mode);
    redraw(state);
    changed(state);
}

/// Called when the user turns the usage ring on or off.
fn set_ring(state: &State, show: bool) {
    *state.show_ring.lock().unwrap() = show;
    store_setting(state, "ring", if show { "on" } else { "off" });
    redraw(state);
    changed(state);
}

/// Redraw the menu bar item after a change to what it shows. Without usage to draw from (before the first check, or
/// after a failed one), it gets the placeholder instead.
fn redraw(state: &State) {
    if state.last_usage.lock().unwrap().is_some() {
        update_title(state);
    } else {
        let loading = state.status.text().unwrap_or_default().starts_with("Loading");
        show_flag(state, if loading { "…" } else { "⚠" });
    }
}

/// Show `flag` ("…" while loading, "⚠" after a failed check) in the menu bar, next to empty rings or the "✻".
fn show_flag(state: &State, flag: &str) {
    let rings = tray_rings(state, None);
    let title = if rings.is_some() { flag.to_string() } else { format!("✻ {flag}") };
    let _ = state.tray.set_icon(rings);
    set_title(state, title);
}

/// Called when the user picks an alert threshold for the 5-hour or weekly limit.
fn set_alert(state: &State, key: &str, threshold: Option<u32>) {
    let Some(limit) = state.limits.iter().find(|l| l.info.key == key && !l.info.optional) else { return };

    // Clear `fired` whenever the threshold changes. If usage is already past the new threshold, the user gets an
    // alert on the next check, which also lets them know alerts are working.
    *limit.alert.lock().unwrap() = AlertState { threshold, fired: false };
    store_setting(state, &format!("alert_{key}"), &threshold.map_or("off".into(), |t| t.to_string()));
    state.wake.notify_one();
    changed(state);
}

/// Called when the user turns "Open at login" on or off. macOS keeps track of this setting, not us.
fn set_launch_at_login(state: &State, enable: bool) -> Result<(), String> {
    let autolaunch = state.app.autolaunch();
    let result = if enable { autolaunch.enable() } else { autolaunch.disable() };
    changed(state);
    result.map_err(|e| e.to_string())
}

/// Called when the user clicks "Check for Updates…", or "Install Update" once we've found one.
fn update_clicked(state: Arc<State>) {
    tauri::async_runtime::spawn(async move {
        let pending = state.pending_update.lock().unwrap().take();
        match pending {
            Some(update) => install_update(&state, update).await,
            None => check_for_update(&state, true).await,
        }
    });
}

/// Open the settings window at `pane`, or bring it to the front (and switch to `pane`) if it's already open.
///
/// Headroom has no Dock icon, so while the window is open we give it one, along with the usual app menu. Without the
/// menu, keyboard shortcuts like Cmd+V don't reach the window, and pasting a token is the main thing people do here.
fn open_settings(app: &AppHandle, pane: &str) {
    open_settings_at(app, pane, None);
}

/// Open the settings window at `pane`, scrolled to `setting` there (a row's title, like "Updates") and outlining it.
fn open_settings_at(app: &AppHandle, pane: &str, setting: Option<&str>) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = match setting {
            Some(setting) => w.emit("show-setting", json!({ "pane": pane, "setting": setting })),
            None => w.emit("show-pane", pane),
        };
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }

    let url = match setting {
        Some(setting) => format!("index.html?pane={pane}&setting={}", setting.replace(' ', "%20")),
        None => format!("index.html?pane={pane}"),
    };
    let url = WebviewUrl::App(url.into());
    let Ok(w) = WebviewWindowBuilder::new(app, "settings", url)
        .title("Headroom")
        .inner_size(860.0, 580.0)
        .min_inner_size(720.0, 480.0)
        .maximizable(false)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .effects(WindowEffectsConfig {
            effects: vec![Effect::Sidebar],
            state: Some(EffectState::FollowsWindowActiveState),
            ..Default::default()
        })
        .center()
        .build()
    else {
        return;
    };
    round_corners(&w);
    in_dock_while_open(app, &w);
    let _ = w.set_focus();
}

/// How many of Headroom's windows are open that put it in the Dock and the app switcher.
static DOCK_WINDOWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Headroom lives in the menu bar, but while one of its windows is open, it's in the Dock and the app switcher too, with
/// the usual menus, like any app. It goes back to the menu bar alone once the last of them is closed.
fn in_dock_while_open(app: &AppHandle, w: &tauri::WebviewWindow) {
    use std::sync::atomic::Ordering::SeqCst;
    DOCK_WINDOWS.fetch_add(1, SeqCst);
    let _ = app.set_activation_policy(ActivationPolicy::Regular);
    if let Ok(menu) = Menu::default(app) {
        let _ = app.set_menu(menu);
    }
    let handle = app.clone();
    w.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            if DOCK_WINDOWS.fetch_sub(1, SeqCst) == 1 {
                let _ = handle.set_activation_policy(ActivationPolicy::Accessory);
            }
        }
    });
}

/// Open the agent planner (see Planner.tsx), or bring it forward.
fn open_planner(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("planner") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let Ok(w) = WebviewWindowBuilder::new(app, "planner", WebviewUrl::App("index.html?planner".into()))
        .title("Plan Agents")
        .inner_size(1280.0, 760.0)
        .min_inner_size(900.0, 560.0)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .center()
        .build()
    else {
        return;
    };
    in_dock_while_open(app, &w);
    let _ = w.set_focus();
}

#[tauri::command]
fn open_planner_now(app: AppHandle) {
    open_planner(&app);
}

/// The saved agent plans (see plans.rs).
#[tauri::command]
fn plans(state: tauri::State<Arc<State>>) -> Vec<Value> {
    plans::list(&state.config_dir)
}

#[tauri::command]
fn save_plan(plan: Value, state: tauri::State<Arc<State>>) -> Result<(), String> {
    plans::save(&state.config_dir, &plan)
}

#[tauri::command]
fn delete_plan(id: String, state: tauri::State<Arc<State>>) -> Result<(), String> {
    plans::delete(&state.config_dir, &id)
}

/// The popover's panel width, and the room around it for its shadow, in points. The window is the panel plus that room
/// on each side; the page draws the panel inside it (see Popover.tsx).
const POPOVER_WIDTH: f64 = 404.0;
const POPOVER_ROOM: f64 = 40.0;

/// Where the popover's panel goes: its right edge a little past the menu bar item, so the arrow can point at the item's
/// middle, but never past the right edge of the item's screen. Returns where the window's top left corner goes, in
/// AppKit's screen coordinates (up from the bottom of the main screen), and how far the arrow's point is from the
/// panel's right edge, in points. Asks the menu bar item's own window, which is right on any screen, at any scale.
fn popover_place(state: &State) -> Option<(f64, f64, f64)> {
    state
        .tray
        .with_inner_tray_icon(|tray| {
            let mtm = objc2::MainThreadMarker::new()?;
            let item = tray.ns_status_item()?.button(mtm)?.window()?;
            let frame = item.frame();
            let screen = item.screen()?.frame();
            let middle = frame.origin.x + frame.size.width / 2.0;
            let right = (middle + 60.0).min(screen.origin.x + screen.size.width - 8.0);
            let left = right + POPOVER_ROOM - (POPOVER_WIDTH + 2.0 * POPOVER_ROOM);
            Some((left, frame.origin.y - 1.0, right - middle))
        })
        .ok()?
}

/// Do something with a window's NSWindow. Only on the main thread.
fn with_ns_window(window: &tauri::WebviewWindow, f: impl FnOnce(&objc2_app_kit::NSWindow)) {
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    if let Ok(ns_window) = window.ns_window() {
        // SAFETY: Tauri hands back the window's NSWindow, which lives as long as `window`, and we're on the main thread
        f(unsafe { &*ns_window.cast::<objc2_app_kit::NSWindow>() });
    }
}

/// The popover's window, made (hidden) if it isn't yet. Making a web view brings the app to the front, so this is done
/// ahead of time, when Headroom starts or approvals are turned on, rather than when a request comes in while the user
/// is busy in another app.
fn popover_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("popover") {
        return Some(w);
    }
    WebviewWindowBuilder::new(app, "popover", WebviewUrl::App("index.html?popover".into()))
        .title("Headroom")
        .inner_size(POPOVER_WIDTH + 2.0 * POPOVER_ROOM, 300.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .accept_first_mouse(true)
        .focused(false)
        .visible(false)
        // So an image dropped on the panel reaches the page, for an answer, rather than the window
        .disable_drag_drop_handler()
        .build()
        .ok()
}

/// Open the popover under the menu bar item. Opened by the user (the menu bar item, or Show Waiting Requests), it takes
/// the keyboard, so ↩ and esc answer. Opened by itself for a new request, it doesn't: it comes to the front without
/// taking the keyboard from whatever the user is typing in, until they click it.
fn show_popover(app: &AppHandle, focus: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = popover_window(&handle) else { return };
        POPOVER_SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The panel draws its own shadow. And it's opaque again, if it was shown again while fading out.
        with_ns_window(&window, |w| {
            w.setHasShadow(false);
            w.setAlphaValue(1.0);
        });
        if let Some((left, top, arrow)) = popover_place(&handle.state::<Arc<State>>()) {
            with_ns_window(&window, |w| w.setFrameTopLeftPoint(objc2_foundation::NSPoint::new(left, top)));
            let _ = window.emit("popover-arrow", arrow);
        }
        if focus {
            let _ = window.show();
            let _ = window.set_focus();
        } else {
            with_ns_window(&window, |w| w.orderFrontRegardless());
        }
        // The list of chats, or the requests (see `tray_clicked`)
        let _ = window.emit("popover-open", *handle.state::<Arc<State>>().list_mode.lock().unwrap());
    });
}

/// How many times the popover has been shown, so putting it away can tell it was shown again in the meantime.
static POPOVER_SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Put the popover away, fading it out. If it had the keyboard, and the settings window isn't open, the app the user
/// was in before gets it back; otherwise Headroom would stay the active app with no window showing.
///
/// It's the window that fades, not the page: the web view draws the panel's shadow on a layer of its own, which a
/// fade in the page leaves behind until the window goes.
fn hide_popover(app: &AppHandle) {
    const STEPS: u32 = 12;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window("popover") else { return };
        if !window.is_visible().unwrap_or(false) {
            return;
        }
        // The page stops any pause, since the pointer can't be over a hidden panel
        let _ = window.emit("popover-hide", ());
        let shown = POPOVER_SHOWN.load(std::sync::atomic::Ordering::Relaxed);
        tauri::async_runtime::spawn(async move {
            for step in 1..=STEPS {
                tokio::time::sleep(Duration::from_millis(16)).await;
                let app = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    // Shown again for a new request while it faded: it stays (show_popover makes it opaque again)
                    if POPOVER_SHOWN.load(std::sync::atomic::Ordering::Relaxed) != shown {
                        return;
                    }
                    let Some(window) = app.get_webview_window("popover") else { return };
                    if step < STEPS {
                        // Easing in: slow to start, then quicker
                        let t = step as f64 / STEPS as f64;
                        with_ns_window(&window, |w| w.setAlphaValue(1.0 - t * t));
                        return;
                    }
                    let had_keyboard = window.is_focused().unwrap_or(false);
                    let _ = window.hide();
                    with_ns_window(&window, |w| w.setAlphaValue(1.0));
                    if had_keyboard && app.get_webview_window("settings").is_none() {
                        let _ = app.hide();
                    }
                });
            }
        });
    });
}

/// The popover's page asks where its arrow should point when it loads, since it may have missed the event.
#[tauri::command]
fn popover_arrow(state: tauri::State<Arc<State>>) -> f64 {
    popover_place(&state).map_or(60.0, |(_, _, arrow)| arrow)
}

/// The popover's page reports how tall it needs to be, and the window follows, keeping its top edge where it is so the
/// arrow stays against the menu bar. (Resizing an NSWindow the usual way keeps its bottom edge still instead.)
#[tauri::command]
fn popover_resize(height: f64, app: AppHandle) {
    let Some(window) = app.get_webview_window("popover") else { return };
    let height = height.clamp(80.0, 900.0);
    with_ns_window(&window, |w| {
        let mut frame = w.frame();
        let top = frame.origin.y + frame.size.height;
        frame.size.height = height;
        frame.origin.y = top - height;
        w.setFrame_display(frame, true);
    });
}

/// The ✕ in the popover, or the list put away by clicking somewhere else. The requests keep waiting, and the menu bar
/// item brings it back.
#[tauri::command]
fn close_popover(app: AppHandle) {
    let state = app.state::<Arc<State>>();
    let was_list = std::mem::take(&mut *state.list_mode.lock().unwrap());
    hide_popover(&app);
    if was_list {
        LIST_CLOSED.store(Local::now().timestamp_millis(), std::sync::atomic::Ordering::Relaxed);
        // Anything that came in while the list was open gets its card now, if they open by themselves
        sync_popover(&state);
    }
}

/// When the list was last put away, in milliseconds. Clicking the menu bar item to close it takes the list's window
/// out of focus first, which already closes it, so that click mustn't open it again.
static LIST_CLOSED: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Open the popover from Settings, to answer a question there.
#[tauri::command]
fn show_popover_now(app: AppHandle) {
    show_popover(&app, true);
}

/// Where the pointer is over the popover, in the page's own coordinates, or None when it's somewhere else. A window
/// that opened by itself in an app that isn't active doesn't hear about the pointer moving over it, so the page asks
/// (for pausing the countdown while it's pointed at).
#[tauri::command]
fn popover_pointer(app: AppHandle) -> Option<(f64, f64)> {
    let window = app.get_webview_window("popover")?;
    let mut inside = None;
    with_ns_window(&window, |w| {
        if !w.isVisible() {
            return;
        }
        let frame = w.frame();
        let at = objc2_app_kit::NSEvent::mouseLocation();
        let (x, y) = (at.x - frame.origin.x, frame.origin.y + frame.size.height - at.y);
        if (0.0..frame.size.width).contains(&x) && (0.0..frame.size.height).contains(&y) {
            inside = Some((x, y));
        }
    });
    inside
}

/// The requests waiting for an answer, for the popover. Lighter than get_state, which the popover doesn't need.
#[tauri::command]
fn get_asks(state: tauri::State<Arc<State>>) -> Vec<Value> {
    held_requests(&state)
}

/// Give the settings window the larger corner radius that System Settings and Finder have on recent macOS.
///
/// AppKit picks the radius from whether the window has a toolbar, so we attach an empty one. The title bar is
/// transparent, so the toolbar draws nothing and the page shows through it. The toolbar is created with an identifier
/// on purpose: creating one without an identifier crashes on older macOS versions.
fn round_corners(window: &tauri::WebviewWindow) {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSToolbar, NSWindow, NSWindowToolbarStyle};
    use objc2_foundation::NSString;

    let _ = window.with_webview(|webview| unsafe {
        let ns_window: &NSWindow = &*webview.ns_window().cast();
        let Some(mtm) = MainThreadMarker::new() else { return };
        if ns_window.toolbar().is_some() {
            return;
        }
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("settings"));
        ns_window.setToolbar(Some(&toolbar));
        ns_window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    });
}

/// Route a menu click to whatever handles it.
fn menu_clicked(state: Arc<State>, id: &str) {
    if let Some(how) = id.strip_prefix("pause_") {
        let now = Local::now();
        let until = match how {
            "30" => Paused::Until(now + chrono::Duration::minutes(30)),
            "60" => Paused::Until(now + chrono::Duration::hours(1)),
            "180" => Paused::Until(now + chrono::Duration::hours(3)),
            "tomorrow" => Paused::Until(tomorrow_morning(now)),
            _ => Paused::UntilResumed,
        };
        return set_paused(&state, Some(until));
    }
    match id {
        "resume" => set_paused(&state, None),
        "quit" => state.app.exit(0),
        "refresh" => state.wake.notify_one(),
        "usage" => {
            let _ = Command::new("open").arg(USAGE_PAGE).spawn();
        }
        "settings" => open_settings(&state.app, "general"),
        "planner" => open_planner(&state.app),
        "show_requests" => show_popover(&state.app, true),
        "update" => update_clicked(state),
        _ => {}
    }
}

/*********************
 * P A U S I N G
 ********************/

/// After the menu bar title while alerts and requests are paused, so a quiet Headroom isn't taken for a broken one. The
/// selector after it asks for the plain glyph, not the emoji.
const PAUSED_MARK: &str = "⏸\u{FE0E}";

/// How long alerts and requests are paused for.
#[derive(Clone, Copy, PartialEq)]
enum Paused {
    Until(DateTime<Local>),
    UntilResumed,
}

/// 8:00 the next morning.
fn tomorrow_morning(now: DateTime<Local>) -> DateTime<Local> {
    let day = now.date_naive().succ_opt().unwrap_or(now.date_naive());
    let morning = day.and_hms_opt(8, 0, 0).and_then(|t| t.and_local_timezone(Local).earliest());
    morning.unwrap_or(now + chrono::Duration::hours(12))
}

/// Are alerts and requests paused right now? A pause that's run out doesn't count, even before the menu catches up.
fn is_paused(state: &State) -> bool {
    match *state.paused.lock().unwrap() {
        Some(Paused::Until(until)) => Local::now() < until,
        Some(Paused::UntilResumed) => true,
        None => false,
    }
}

/// Pause alerts and requests, or resume them. Pausing lets go of any request being held, which goes back to the
/// session's own prompt, and puts the popover away.
fn set_paused(state: &State, paused: Option<Paused>) {
    *state.paused.lock().unwrap() = paused;
    let saved = match paused {
        Some(Paused::Until(until)) => until.to_rfc3339(),
        Some(Paused::UntilResumed) => "until_resumed".into(),
        None => "off".into(),
    };
    store_setting(state, "paused", &saved);
    show_paused(state);
    redraw(state);
    tell_hooks_about_approvals(state);
    sync_popover(state);
    changed(state);
}

/// Bring the menu up to date with the pause, and end one that's run out.
fn show_paused(state: &State) {
    let mut paused = state.paused.lock().unwrap();
    if matches!(*paused, Some(Paused::Until(until)) if Local::now() >= until) {
        *paused = None;
        drop(paused);
        return set_paused(state, None);
    }
    let text = match *paused {
        Some(Paused::Until(until)) => format!("Paused Until {}", fmt_when(until)),
        Some(Paused::UntilResumed) => "Paused".into(),
        None => "Pause Alerts and Requests".into(),
    };
    let on = paused.is_some();
    drop(paused);
    let _ = state.pause_menu.set_text(text);
    let _ = state.resume_item.set_enabled(on);
}

/// A pause saved with the settings, unless it's run out since.
fn load_paused(config_dir: &Path) -> Option<Paused> {
    match load_setting(config_dir, "paused").as_deref() {
        Some("until_resumed") => Some(Paused::UntilResumed),
        Some("off") | None => None,
        Some(when) => {
            let until = DateTime::parse_from_rfc3339(when).ok()?.with_timezone(&Local);
            (Local::now() < until).then_some(Paused::Until(until))
        }
    }
}

/*****************************************
 * T H E  S E T T I N G S  W I N D O W
 ****************************************/

/// How long each limit's window is, so we can tell how far through it we are.
fn window_length(key: &str) -> chrono::Duration {
    if key == "5h" { chrono::Duration::hours(5) } else { chrono::Duration::days(7) }
}

/// One limit as the settings window shows it: how far through its window we are, and when usage would hit 100% at
/// the pace it's been going, if that's before it resets.
fn limit_json(key: &str, label: &str, w: &Window) -> Value {
    let mut out = json!({ "key": key, "label": label, "pct": w.pct.round() });
    let Some(resets_at) = w.resets_at else { return out };
    let now = Local::now();
    let length = window_length(key);
    let used = (length - (resets_at - now)).num_seconds().max(0) as f64;
    out["resets"] = fmt_when(resets_at).into();
    out["resetsAt"] = resets_at.timestamp_millis().into();
    out["elapsed"] = (used / length.num_seconds() as f64 * 100.0).clamp(0.0, 100.0).round().into();
    out["paceArrow"] = match pace(key, w) {
        Some(Pace::Ahead) => "ahead".into(),
        Some(Pace::Behind) => "behind".into(),
        None => Value::Null,
    };

    // Too early in the window, the pace swings wildly with every request, so we don't guess until it's settled a bit
    if w.pct > 0.0 && used > length.num_seconds() as f64 * 0.05 {
        let secs_to_full = (100.0 - w.pct).max(0.0) * used / w.pct;
        let full_at = now + chrono::Duration::seconds(secs_to_full as i64);
        if full_at < resets_at {
            out["pace"] = fmt_when(full_at).into();
        }
    }
    out
}

/// Everything the settings window shows. It asks for this when it opens and again whenever we emit "changed".
#[tauri::command]
fn get_state(state: tauri::State<Arc<State>>) -> Value {
    let alert = |key: &str| -> Value {
        let limit = state.limits.iter().find(|l| l.info.key == key);
        match limit.and_then(|l| l.alert.lock().unwrap().threshold) {
            Some(t) => t.into(),
            None => "off".into(),
        }
    };
    let last_usage = state.last_usage.lock().unwrap();
    let mut limits: Vec<Value> = match last_usage.as_ref() {
        Some(usage) => state
            .limits
            .iter()
            .zip(usage)
            .filter_map(|(l, w)| w.as_ref().map(|w| limit_json(l.info.key, l.info.label, w)))
            .collect(),
        None => vec![],
    };
    if last_usage.is_some() {
        limits.extend(state.model_limits.lock().unwrap().iter().map(|m| limit_json(&m.key, &m.label, &m.window)));
    }
    let menu_title = last_usage.as_ref().map(|usage| title_text(&state, usage));
    drop(last_usage);
    let alerts = state.session_alerts.lock().unwrap();
    let near = state.near_limit.lock().unwrap();
    let recap = *state.recap.lock().unwrap();
    let approvals = *state.approvals.lock().unwrap();
    let shares: Vec<Value> =
        state.shares.lock().unwrap().iter().map(|(project, pct)| json!({ "project": project, "pct": pct })).collect();
    json!({
        "version": state.app.package_info().version.to_string(),
        "status": state.status.text().unwrap_or_default(),
        "ok": menu_title.is_some(),
        "limits": limits,
        "menuTitle": menu_title,
        "signIn": if saved_token().is_some() { "token" } else { "claude_code" },
        "update": state.pending_update.lock().unwrap().as_ref().map(|u| u.version.clone()),
        "settings": {
            "interval": *state.interval.lock().unwrap(),
            "title": *state.title_mode.lock().unwrap(),
            "ring": *state.show_ring.lock().unwrap(),
            "alert5h": alert("5h"),
            "alert7d": alert("7d"),
            "launchAtLogin": state.app.autolaunch().is_enabled().unwrap_or(false),
            "autoUpdate": *state.auto_update.lock().unwrap(),
            "hooks": *state.hooks_on.lock().unwrap(),
            "waitingCount": *state.waiting_count.lock().unwrap(),
            "notifyWaiting": alerts.answer,
            "notifyDone": alerts.done,
            "doneOnlyAway": alerts.only_away,
            "doneMinRun": alerts.min_run,
            "resumeAlert": alerts.reset,
            "nearLimit": near.on,
            "nearLimitAt": near.at,
            "nearLimitRightAway": near.right_away,
            "nearLimitMessage": near.message,
            "nearLimitDefault": hooks::DEFAULT_ADVICE,
            "recap": recap.0,
            "recapAt": recap.1,
            "approvals": approvals.0,
            "popoverAuto": *state.popover_auto.lock().unwrap(),
            "compactCards": *state.compact_cards.lock().unwrap(),
            "askNext": *state.ask_next.lock().unwrap(),
            "handsFree": *state.hands_free.lock().unwrap(),
            "paused": match *state.paused.lock().unwrap() {
                Some(Paused::Until(until)) if Local::now() < until => json!({ "until": fmt_when(until) }),
                Some(Paused::UntilResumed) => json!({}),
                _ => Value::Null,
            },
            "paceArrows": *state.pace_arrows.lock().unwrap(),
            "persistentAlerts": *state.persistent_alerts.lock().unwrap(),
            "contextAlert": state.context_alert.lock().unwrap().map_or(json!("off"), |t| json!(t)),
            "approvalTimeout": approvals.1,
        },
        "recap": recap_text(&state).map(|(title, body)| json!({ "title": title, "body": body })),
        "asks": held_requests(&state),
        "shares": shares,
        "sessions": if *state.hooks_on.lock().unwrap() {
            let answerable = approvals.0;
            let list = state.sessions.lock().unwrap().to_json(hold_time(&state), &hooks::extra_time(), answerable);
            with_icons(&state, list.as_array().cloned().unwrap_or_default(), "id").into()
        } else {
            json!([])
        },
    })
}

/// Change a setting from the settings window. This goes through the same code as the menu, so the two stay in step.
#[tauri::command]
fn set_setting(key: String, value: Value, state: tauri::State<Arc<State>>) -> Result<(), String> {
    let bad = || format!("Can't set {key} to {value}");
    match key.as_str() {
        "interval" => {
            let secs = value.as_u64().filter(|s| INTERVALS.contains(s)).ok_or_else(bad)?;
            set_interval(&state, secs);
        }
        "title" => {
            let mode = TITLE_MODES.iter().find(|m| value.as_str() == Some(**m)).copied().ok_or_else(bad)?;
            set_title_mode(&state, mode);
        }
        "ring" => set_ring(&state, value.as_bool().ok_or_else(bad)?),
        "launchAtLogin" => set_launch_at_login(&state, value.as_bool().ok_or_else(bad)?)?,
        "autoUpdate" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.auto_update.lock().unwrap() = on;
            store_setting(&state, "auto_update", if on { "on" } else { "off" });
            changed(&state);
        }
        "hooks" => set_hooks(&state, value.as_bool().ok_or_else(bad)?)?,
        "waitingCount" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.waiting_count.lock().unwrap() = on;
            store_setting(&state, "waiting_count", if on { "on" } else { "off" });
            redraw(&state);
            changed(&state);
        }
        "notifyWaiting" | "notifyDone" | "doneOnlyAway" | "resumeAlert" => {
            let on = value.as_bool().ok_or_else(bad)?;
            let mut alerts = state.session_alerts.lock().unwrap();
            let (field, name) = match key.as_str() {
                "notifyWaiting" => (&mut alerts.answer, "notify_answer"),
                "notifyDone" => (&mut alerts.done, "notify_done"),
                "doneOnlyAway" => (&mut alerts.only_away, "notify_done_only_away"),
                _ => (&mut alerts.reset, "notify_reset"),
            };
            *field = on;
            drop(alerts);
            store_setting(&state, name, if on { "on" } else { "off" });
            changed(&state);
        }
        "nearLimit" | "nearLimitRightAway" => {
            let on = value.as_bool().ok_or_else(bad)?;
            let mut near = state.near_limit.lock().unwrap();
            let (field, name) = if key == "nearLimit" {
                (&mut near.on, "near_limit")
            } else {
                (&mut near.right_away, "near_limit_right_away")
            };
            *field = on;
            drop(near);
            // Which hooks Claude Code waits for depends on both of these, so the hooks get rewritten
            if *state.hooks_on.lock().unwrap() {
                install_hooks(&state)?;
            }
            store_setting(&state, name, if on { "on" } else { "off" });
            save_limits(&state);
            changed(&state);
        }
        "nearLimitAt" => {
            let at = value.as_u64().map(|t| t as u32).filter(|t| NEAR_LEVELS.contains(t)).ok_or_else(bad)?;
            state.near_limit.lock().unwrap().at = at;
            store_setting(&state, "near_limit_at", &at.to_string());
            save_limits(&state);
            changed(&state);
        }
        "nearLimitMessage" => {
            let message = value.as_str().ok_or_else(bad)?.trim().to_string();
            store_setting(&state, "near_limit_message", &message);
            state.near_limit.lock().unwrap().message = message;
            save_limits(&state);
            changed(&state);
        }
        "popoverAuto" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.popover_auto.lock().unwrap() = on;
            store_setting(&state, "popover_auto", if on { "on" } else { "off" });
            changed(&state);
        }
        "compactCards" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.compact_cards.lock().unwrap() = on;
            store_setting(&state, "compact_cards", if on { "on" } else { "off" });
            changed(&state);
        }
        // Only resuming from Settings: pausing is in the menu
        "paused" if value.is_null() => {
            let state = state.inner().clone();
            let app = state.app.clone();
            let _ = app.run_on_main_thread(move || set_paused(&state, None));
        }
        "handsFree" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.hands_free.lock().unwrap() = on;
            store_setting(&state, "hands_free", if on { "on" } else { "off" });
            // Claude Code waits for the hook after each tool call only while it's on (see `install_hooks`)
            if *state.hooks_on.lock().unwrap() {
                install_hooks(&state)?;
            }
            tell_hooks_about_approvals(&state);
            // Which click opens the menu changes with it
            sync_popover(&state);
            changed(&state);
        }
        "askNext" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.ask_next.lock().unwrap() = on;
            store_setting(&state, "ask_next", if on { "on" } else { "off" });
            // Hands-free goes with it
            if *state.hooks_on.lock().unwrap() {
                install_hooks(&state)?;
            }
            // The hooks read it at each session's next message or end of turn
            tell_hooks_about_approvals(&state);
            sync_popover(&state);
            changed(&state);
        }
        "paceArrows" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.pace_arrows.lock().unwrap() = on;
            store_setting(&state, "pace_arrows", if on { "on" } else { "off" });
            redraw(&state);
            changed(&state);
        }
        "contextAlert" => {
            let at = match value.as_str() {
                Some("off") => None,
                _ => Some(value.as_u64().map(|t| t as u32).filter(|t| CONTEXT_LEVELS.contains(t)).ok_or_else(bad)?),
            };
            *state.context_alert.lock().unwrap() = at;
            store_setting(&state, "context_alert", &at.map_or("off".to_string(), |t| t.to_string()));
            changed(&state);
        }
        "persistentAlerts" => {
            let on = value.as_bool().ok_or_else(bad)?;
            *state.persistent_alerts.lock().unwrap() = on;
            store_setting(&state, "persistent_alerts", if on { "on" } else { "off" });
            changed(&state);
        }
        "approvals" | "approvalTimeout" => {
            if key == "approvals" {
                let on = value.as_bool().ok_or_else(bad)?;
                state.approvals.lock().unwrap().0 = on;
                store_setting(&state, "approvals", if on { "on" } else { "off" });
            } else {
                let hold = value.as_u64().filter(|s| APPROVAL_HOLDS.contains(s)).ok_or_else(bad)?;
                state.approvals.lock().unwrap().1 = hold;
                store_setting(&state, "approval_hold", &hold.to_string());
            }
            // The permission hook's timeout follows the hold, so the hooks get rewritten
            if *state.hooks_on.lock().unwrap() {
                install_hooks(&state)?;
            }
            tell_hooks_about_approvals(&state);
            changed(&state);
        }
        "recap" => {
            let on = value.as_bool().ok_or_else(bad)?;
            state.recap.lock().unwrap().0 = on;
            store_setting(&state, "recap", if on { "on" } else { "off" });
            changed(&state);
        }
        "recapAt" => {
            let at = RECAP_TIMES.iter().find(|t| value.as_str() == Some(**t)).copied().ok_or_else(bad)?;
            state.recap.lock().unwrap().1 = at;
            store_setting(&state, "recap_at", at);
            changed(&state);
        }
        "doneMinRun" => {
            let secs = value.as_u64().filter(|s| MIN_RUNS.contains(s)).ok_or_else(bad)?;
            state.session_alerts.lock().unwrap().min_run = secs;
            store_setting(&state, "notify_done_min_run", &secs.to_string());
            changed(&state);
        }
        "alert5h" | "alert7d" => {
            let threshold = match value.as_u64() {
                Some(t) if ALERT_LEVELS.contains(&(t as u32)) => Some(t as u32),
                None if value.as_str() == Some("off") => None,
                _ => return Err(bad()),
            };
            set_alert(&state, &key[5..], threshold);
        }
        _ => return Err(bad()),
    }
    Ok(())
}

/// Write our hooks into ~/.claude/settings.json as the settings stand now: which ones Claude Code waits for depends on
/// the near-limit note, approvals, and hands-free.
fn install_hooks(state: &State) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let note = state.near_limit.lock().unwrap().for_hooks();
    let (approvals, hold) = *state.approvals.lock().unwrap();
    // Hands-free, the hook after each tool call is waited for, so a message sent from the list can go in with it
    let hands_free = *state.ask_next.lock().unwrap() && *state.hands_free.lock().unwrap();
    hooks::install(&exe, &note, approvals.then_some(hold), hands_free)
}

/// Add or remove our Claude Code hooks. Sessions that were already running when they're added don't pick them up until
/// they restart, since Claude Code reads its settings when a session starts.
fn set_hooks(state: &State, on: bool) -> Result<(), String> {
    if on {
        install_hooks(state)?;
    } else {
        hooks::uninstall()?;
    }
    *state.hooks_on.lock().unwrap() = on;
    store_setting(state, "hooks", if on { "on" } else { "off" });
    // Hooks still holding a request in a running session let go of it, and the popover goes with it
    tell_hooks_about_approvals(state);
    redraw(state);
    sync_popover(state);
    changed(state);
    Ok(())
}

/// Keep up with the log the hooks write. Checking a file's size five times a second costs next to nothing, and it
/// means a session that starts or stops waiting on you shows up in the menu bar almost as soon as it happens.
async fn follow_sessions(state: Arc<State>) {
    let Some(log) = hooks::log_path() else { return };
    let mut ticks = 0u32;
    loop {
        if *state.hooks_on.lock().unwrap() {
            let mut sessions = state.sessions.lock().unwrap();
            let (mut news, changes) = sessions.read(&log);
            // Once a minute, drop the sessions that have gone quiet
            if ticks % 300 == 0 {
                news |= sessions.forget_quiet();
            }
            let released = sessions.take_released();
            // Every two seconds, each conversation's context, if its alert is on
            let names = (ticks % 10 == 0).then(|| sessions.names());
            drop(sessions);
            if let (Some(names), Some(at)) = (names, *state.context_alert.lock().unwrap()) {
                alert_full_contexts(&state, &names, at);
            }
            // Every second, what the Claude app says about finished chats
            if ticks % 5 == 0 && follow_claude_app(&state) {
                redraw(&state);
                changed(&state);
            }
            // Requests answered some other way, in the terminal say: their hooks stop waiting for one from here
            for id in released {
                let _ = hooks::answer(&id, "terminal");
            }
            if news {
                redraw(&state);
                sync_popover(&state);
                changed(&state);
            } else if ticks % 5 == 0 {
                // Once a second anyway: a request can run out, or its hook can be stopped, without a word in the log
                sync_popover(&state);
            }
            // Every five seconds, so a hook holding a request can tell Headroom is still running, and within a second
            // of the app in front changing, so a turn held open for a reply lets go when the user goes to its chat
            if ticks % 25 == 0 || (ticks % 5 == 0 && *state.told_front.lock().unwrap() != frontmost_app()) {
                tell_hooks_about_approvals(&state);
            }
            for change in changes {
                notify_session(&state, change);
            }
        }
        ticks = ticks.wrapping_add(1);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Check the finished sessions against the Claude app: how it sorted each turn, and which chat is open there. Returns
/// whether any session's dot changed.
fn follow_claude_app(state: &State) -> bool {
    let finished = state.sessions.lock().unwrap().finished();
    let mut desktop = state.desktop.lock().unwrap();
    let mut news = desktop.follow_log();
    let seen: Vec<_> = finished.iter().map(|id| (id, desktop.chat(id), desktop.is_open(id))).collect();
    drop(desktop);
    // The hooks go by which chat is open, to hold a request from any other (see hooks.rs `watching`)
    if news {
        tell_hooks_about_approvals(state);
    }
    let mut sessions = state.sessions.lock().unwrap();
    for (id, chat, open) in seen {
        news |= sessions.from_app(id, chat, open);
    }
    news
}

/// Alert about each conversation whose context has just passed `at` percent of its window.
fn alert_full_contexts(state: &State, names: &[(String, String)], at: u32) {
    let ids: Vec<String> = names.iter().map(|(id, _)| id.clone()).collect();
    let crossings = state.contexts.lock().unwrap().update(&ids, at);
    let tokens = |n: u64| if n >= 1_000_000 { format!("{}M", n / 1_000_000) } else { format!("{}k", n / 1000) };
    for c in crossings {
        let name = names.iter().find(|(id, _)| *id == c.session).map_or("A conversation", |(_, name)| name.as_str());
        let body = format!(
            "{} of its {} context. Claude Code compacts it on its own at about {}%.",
            tokens(c.tokens),
            tokens(c.window),
            context::compacts_at(c.window)
        );
        let app = state.sessions.lock().unwrap().app_of(&c.session);
        let on_click = OnClick::Session { id: c.session.clone(), app };
        notify_about(state, &c.session, &format!("{name}: context is {}% full", c.pct), &body, Some(on_click));
    }
}

/// Send a notification about a session, if the user wants to hear about this kind of change.
fn notify_session(state: &State, change: sessions::Change) {
    use sessions::ChangeKind;

    if Local::now() - change.at > FRESH {
        return;
    }
    let alerts = state.session_alerts.lock().unwrap();
    let project = &change.project;
    let on_click = Some(OnClick::Session { id: change.session.clone(), app: change.app.clone() });
    match change.kind {
        ChangeKind::NeedsAnswer { question, tool, detail, request, late } if alerts.answer => {
            drop(alerts);
            // Not when the popover is about to open for it anyway, or its chat is the one open in the Claude app. If it
            // runs out in the popover unanswered, the alert goes then, since whoever it was for wasn't there.
            let popover_auto = *state.popover_auto.lock().unwrap();
            if late && !popover_auto {
                return;
            }
            let popover =
                popover_auto && request.is_some_and(|id| held_requests(state).iter().any(|r| r["id"] == id.as_str()));
            let in_chat = change.app.as_deref() == Some(CLAUDE_APP) && looking_at(state, &change.session, &change.app);
            if popover || in_chat {
                return;
            }
            let (title, body) = if question {
                (format!("{project} has a question"), detail)
            } else if detail.is_empty() {
                (format!("{project} needs permission"), format!("To use {tool}"))
            } else {
                (format!("{project} needs permission"), format!("{tool}: {detail}"))
            };
            notify_about(state, &change.session, &title, &body, on_click);
        }
        ChangeKind::Finished { reply, ran } if alerts.done => {
            let too_quick = ran.is_some_and(|ran| ran.num_seconds() < alerts.min_run as i64);
            let only_away = alerts.only_away;
            drop(alerts);
            let watching = only_away && looking_at(state, &change.session, &change.app);
            if !too_quick && !watching {
                let body = reply.unwrap_or_else(|| "Ready for your next message.".into());
                notify_about(state, &change.session, &format!("{project} is done"), &body, on_click);
            }
        }
        _ => {}
    }
}

/// The Claude app's bundle ID.
const CLAUDE_APP: &str = "com.anthropic.claudefordesktop";

/// Is the user looking at this session? Its app is the one in front, and if that's the Claude app, its chat is the one
/// open there. A terminal can only be told apart by the app, not the tab.
fn looking_at(state: &State, session: &str, app: &Option<String>) -> bool {
    let in_front = app.is_some() && *app == frontmost_app();
    if !in_front || app.as_deref() != Some(CLAUDE_APP) {
        return in_front;
    }
    // The chat the hooks said it is, or failing that, the one the Claude app's files say
    let chat = state.sessions.lock().unwrap().host_chat(session);
    let mut desktop = state.desktop.lock().unwrap();
    match chat {
        Some(chat) => desktop.open_chat().as_deref() == Some(chat.as_str()),
        None => desktop.is_open(session),
    }
}

/// The bundle ID of the app in front, like "com.mitchellh.ghostty".
fn frontmost_app() -> Option<String> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(app.bundleIdentifier()?.to_string())
}

/// After a good check, see whether the 5-hour or weekly limit just reset, and if any sessions had stopped at a limit,
/// let the user know they can pick them back up.
fn check_resets(state: &State, usage: &Usage) {
    let mut resets = state.resets.lock().unwrap();
    let mut reset = None;
    for (i, label) in [(0, "5-hour"), (1, "weekly")] {
        let Some(now) = usage[i].as_ref().and_then(|w| w.resets_at) else { continue };
        // The reset time wobbles by a few seconds from one check to the next, so only a real jump counts
        if resets[i].is_some_and(|before| now - before > chrono::Duration::minutes(10)) {
            reset = Some(label);
        }
        resets[i] = Some(now);
    }
    drop(resets);

    let Some(label) = reset else { return };
    if !state.session_alerts.lock().unwrap().reset {
        return;
    }
    let stopped = state.sessions.lock().unwrap().limited();
    let body = match stopped.as_slice() {
        [] => return,
        [one] => format!("{one} stopped when it hit the limit. You can pick it back up."),
        [rest @ .., last] => {
            format!("{} and {last} stopped when they hit the limit. You can pick them back up.", rest.join(", "))
        }
    };
    notify_to(state, &format!("Your {label} limit reset"), &body, Some(OnClick::Pane("sessions")));
}

/// The wallpaper and the main screen's size, for the menu bar preview in Settings. Finding the screen has to happen on
/// the main thread; making the image (the first time) takes a moment, so it happens off it.
#[tauri::command]
async fn get_wallpaper(state: tauri::State<'_, Arc<State>>) -> Result<Value, String> {
    let (send, receive) = std::sync::mpsc::channel();
    state.app.run_on_main_thread(move || {
        let _ = send.send(wallpaper::main_screen());
    }).map_err(|e| e.to_string())?;
    let screen = receive.recv().map_err(|e| e.to_string())?.ok_or("No screen")?;
    let cache = state.app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&cache);
    let picture = screen.picture.clone();
    let url = tauri::async_runtime::spawn_blocking(move || wallpaper::data_url(&cache, picture))
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "url": url, "width": screen.width, "height": screen.height, "menuBar": screen.menu_bar }))
}

/// How long permission requests are held for an answer.
fn hold_time(state: &State) -> chrono::Duration {
    chrono::Duration::seconds(state.approvals.lock().unwrap().1 as i64)
}

/// Let the hooks know whether Headroom is taking requests (approvals and the hooks both on), how long to hold one,
/// whether Claude should ask what's next (only with approvals, so the question comes here), and which chat is open in
/// the Claude app.
fn tell_hooks_about_approvals(state: &State) {
    let (approvals, hold) = *state.approvals.lock().unwrap();
    let hooks_on = *state.hooks_on.lock().unwrap();
    // Paused, nothing's held, but whether Claude asks what's next stays as it was: the question goes to the session's
    // own prompt for now, rather than every session being told to stop and then to start again
    let on = approvals && hooks_on && !is_paused(state);
    let ask = approvals && hooks_on && *state.ask_next.lock().unwrap();
    let hands_free = ask && *state.hands_free.lock().unwrap();
    let open_chat = state.desktop.lock().unwrap().open_chat();
    let front = frontmost_app();
    *state.told_front.lock().unwrap() = front.clone();
    hooks::write_approvals(&hooks::ForHooks { on, hold, ask, hands_free, open_chat, front });
}

/// The requests Headroom is holding for an answer, for the popover. None while approvals or the hooks are off: a hook
/// that's still holding one then gives up within a second or so, and it goes back to the terminal.
fn held_requests(state: &State) -> Vec<Value> {
    if !state.approvals.lock().unwrap().0 || !*state.hooks_on.lock().unwrap() || is_paused(state) {
        return vec![];
    }
    let extra = hooks::extra_time();
    let held = state.sessions.lock().unwrap().held(hold_time(state), &extra);
    with_icons(state, held, "session")
}

/// A session's icon (see icons.rs), or none for one Headroom doesn't know.
fn icon_of(state: &State, session: &str) -> Option<Value> {
    let (key, others) = {
        let sessions = state.sessions.lock().unwrap();
        (sessions.icon_key(session)?, sessions.icon_keys())
    };
    Some(state.icons.lock().unwrap().of(&key.0, &key.1, &others))
}

/// The same rows, each with its session's icon, the session's ID being under `key`.
fn with_icons(state: &State, mut rows: Vec<Value>, key: &str) -> Vec<Value> {
    for row in &mut rows {
        if let Some(icon) = row[key].as_str().and_then(|id| icon_of(state, id)) {
            row["icon"] = icon;
        }
    }
    rows
}

/// Every emoji macOS has a name for, for the icon picker's search (see icons.rs). Read once, the first time.
#[tauri::command]
async fn emoji_names() -> Vec<(String, String)> {
    static NAMES: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    NAMES.get_or_init(icons::emoji_names).clone()
}

/// Give a session's chat an icon, or every chat in its folder with `whole_project`. `None` puts back the one it had to
/// start with.
#[tauri::command]
fn set_icon(
    session: String,
    icon: Option<Value>,
    whole_project: bool,
    state: tauri::State<Arc<State>>,
) -> Result<(), String> {
    if icon.as_ref().is_some_and(|i| !icons::valid(i)) {
        return Err("That can't be an icon".into());
    }
    let (chat, cwd, others) = {
        let sessions = state.sessions.lock().unwrap();
        let (chat, cwd) = sessions.icon_key(&session).ok_or("Headroom doesn't know that chat any more")?;
        let others = sessions.chats_in(&cwd);
        (chat, cwd, others)
    };
    state.icons.lock().unwrap().set(&chat, &cwd, icon, whole_project, &others);
    changed(&state);
    Ok(())
}

/// Answer a held request from the popover or Settings: "allow", "session" or "deny" for a permission request,
/// "answer:" and the answer for a question, or "terminal" for either, to hand it back to Claude Code's own prompt.
/// "chat" hands it back too, and opens the session where it runs, to answer it there.
fn answer(state: &State, id: &str, choice: &str) -> Result<(), String> {
    if choice == "chat" {
        let (session, host) = {
            let sessions = state.sessions.lock().unwrap();
            let session = sessions.session_of(id);
            let host = session.as_deref().and_then(|s| sessions.app_of(s));
            (session, host)
        };
        // Handed back already (it ran out a moment ago, say), it's waiting there all the same
        let _ = answer(state, id, "terminal");
        // The panel would sit over the chat, and the rest of what's waiting keeps behind the yellow dot
        hide_popover(&state.app);
        let session = session.ok_or("Headroom can't tell which session that was")?;
        open_session(state, &session, host);
        return Ok(());
    }
    // A click that lands after the hook gave up would be answering a prompt that's in the terminal now. A finished
    // turn held open for a reply isn't among the popover's requests, but in the list of what's waiting.
    let held = held_requests(state).iter().any(|r| r["id"] == id);
    if !held && !pending(state).iter().any(|p| p["replyId"] == id) {
        sync_popover(state);
        changed(state);
        return Err(if choice.starts_with("reply:") {
            "That chat stopped waiting for a reply".into()
        } else {
            "That request went back to the terminal".into()
        });
    }
    let kind = state.sessions.lock().unwrap().kind_of(id);
    let fits = match kind {
        _ if choice == "terminal" => true,
        Some("permission") => matches!(choice, "allow" | "session" | "deny"),
        Some("question") => choice.starts_with("answer:") || choice.starts_with("answers:"),
        Some("reply") => choice.starts_with("reply:"),
        _ => false,
    };
    if !fits {
        return Err(format!("{choice} doesn't answer that request"));
    }
    hooks::answer(id, choice)?;
    let mut sessions = state.sessions.lock().unwrap();
    if choice == "terminal" {
        sessions.handed_back(id);
        drop(sessions);
    } else {
        let session = sessions.session_of(id);
        sessions.answered(id);
        drop(sessions);
        // Logged too, so that reading the log again after a restart doesn't bring the request back
        if let Some(session) = session {
            hooks::log(&json!({
                "hook_event_name": "HeadroomAnswered",
                "session_id": session,
                "request_id": id,
                "at": chrono::Utc::now().timestamp_millis(),
            }));
        }
    }
    redraw(state);
    sync_popover(state);
    changed(state);
    Ok(())
}

/// Save an image the user added to an answer, and return where, for the answer to point Claude to: all a held question
/// can send back is words. They're kept in Headroom's folder for a week (see hooks.rs `forget_old_notes`). Only the
/// kinds Claude Code can open: the popover turns any other into a PNG first.
#[tauri::command]
fn save_attachment(data: String) -> Result<String, String> {
    use base64::Engine;
    const MAX_BYTES: usize = 20 * 1024 * 1024;
    let (head, body) = data.split_once(',').ok_or("That isn't an image")?;
    let kind = head.strip_prefix("data:image/").and_then(|h| h.strip_suffix(";base64")).ok_or("That isn't an image")?;
    let ext = match kind {
        "png" => "png",
        "jpeg" | "jpg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        _ => return Err(format!("Headroom can't attach .{kind} images")),
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(body).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("That image is over 20 MB".into());
    }
    let dir = hooks::log_path().ok_or("Can't find Headroom's folder")?.with_file_name("attachments");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    static SAVED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SAVED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("{}-{n}.{ext}", Local::now().format("%Y-%m-%d-%H%M%S")));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn answer_request(id: String, choice: String, state: tauri::State<Arc<State>>) -> Result<(), String> {
    answer(&state, &id, &choice)
}

/// Give a held request back the time it spent paused, while the user pointed at it near the end. The popover sends
/// this every second it stays paused.
#[tauri::command]
fn extend_request(id: String, ms: i64, state: tauri::State<Arc<State>>) -> Result<(), String> {
    if !held_requests(&state).iter().any(|r| r["id"] == id.as_str()) {
        return Ok(());
    }
    let total = (hooks::extra_time().get(&id).copied().unwrap_or(0) + ms.clamp(0, 5_000)).min(120_000);
    hooks::extend(&id, total as u64)?;
    changed(&state);
    Ok(())
}

/// Keep the menu bar item and the popover in step with the requests being held. While any are waiting, clicking the
/// item opens the popover instead of the menu (a right click still opens the menu), and a request the popover hasn't
/// opened for yet opens it by itself, if that's on. Once they're all answered, the popover goes away.
///
/// This runs on the main thread, where the tray is changed anyway, so two calls can't overlap and undo each other.
fn sync_popover(state: &State) {
    let app = state.app.clone();
    let _ = state.app.run_on_main_thread(move || sync_popover_now(&app.state::<Arc<State>>()));
}

fn sync_popover_now(state: &State) {
    // Made ahead of time while approvals are on (see popover_window)
    if state.approvals.lock().unwrap().0 && *state.hooks_on.lock().unwrap() {
        popover_window(&state.app);
    }
    let held = held_requests(state);
    // A left click opens the menu, unless it opens the requests, or hands-free, the list of chats
    let _ = state.tray.set_show_menu_on_left_click(held.is_empty() && !hands_free_on(state));
    let ids: HashSet<String> = held.iter().filter_map(|r| r["id"].as_str().map(String::from)).collect();
    // While the list's open, a new request shows there; it gets its card once the list goes (see `close_popover`)
    if *state.list_mode.lock().unwrap() {
        let _ = state.show_requests.set_enabled(!held.is_empty());
        return;
    }
    let mut seen = state.seen_requests.lock().unwrap();
    if *seen == ids {
        return;
    }
    let new = ids.iter().any(|id| !seen.contains(id));
    *seen = ids;
    drop(seen);
    // A request can run out, or its hook be stopped, without a word in the log, so the pages hear about it here
    changed(state);

    let waiting = !held.is_empty();
    let _ = state.show_requests.set_enabled(waiting);
    if waiting {
        // Not over the list of what's waiting, which shows the new request too
        if new && *state.popover_auto.lock().unwrap() && !*state.list_mode.lock().unwrap() {
            show_popover(&state.app, false);
        }
        return;
    }
    // Not right away: the last card's receipt, or its dissolve on the way to the terminal, shows for a moment first
    let app = state.app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(550)).await;
        let state = app.state::<Arc<State>>();
        if held_requests(&state).is_empty() && !*state.list_mode.lock().unwrap() {
            hide_popover(&app);
        }
    });
}

/// A click on the menu bar item. With requests waiting, a left click opens or closes the popover with them. Otherwise,
/// hands-free, it opens or closes the list of chats, and without, the menu opens as usual, which tray-icon handles
/// itself. A right click always opens the menu.
fn tray_clicked(tray: &TrayIcon, event: tauri::tray::TrayIconEvent) {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event else {
        return;
    };
    let app = tray.app_handle();
    let state = app.state::<Arc<State>>();
    let showing = app.get_webview_window("popover").is_some_and(|w| w.is_visible().unwrap_or(false));
    let waiting = !held_requests(&state).is_empty();
    let closed = LIST_CLOSED.load(std::sync::atomic::Ordering::Relaxed);
    let just_closed = Local::now().timestamp_millis() - closed < 400;
    // Hands-free, a left click opens the list of chats, and a right click the menu. Not while requests are waiting:
    // those come first, and the click brings them up
    if hands_free_on(&state) && !waiting {
        if showing {
            close_popover(app.clone());
        } else if !just_closed {
            *state.list_mode.lock().unwrap() = true;
            show_popover(app, true);
        }
        return;
    }
    if !waiting {
        // The last one ran out since we last looked, so the next click should open the menu again
        sync_popover(&state);
        return;
    }
    // The list, open when they came in (or put away by this very click), gives way to them
    let list = std::mem::take(&mut *state.list_mode.lock().unwrap());
    if (list && showing) || just_closed {
        show_popover(app, true);
        return;
    }
    if showing {
        hide_popover(app);
    } else {
        show_popover(app, true);
    }
}

/// Is hands-free really on: set, along with what it needs (approvals, the hooks, and asking what's next), and not paused?
fn hands_free_on(state: &State) -> bool {
    state.approvals.lock().unwrap().0
        && *state.hooks_on.lock().unwrap()
        && *state.ask_next.lock().unwrap()
        && *state.hands_free.lock().unwrap()
        && !is_paused(state)
}

/// What's waiting on the user, for the list (see sessions.rs `pending`).
fn pending(state: &State) -> Vec<Value> {
    if !*state.hooks_on.lock().unwrap() {
        return vec![];
    }
    let extra = hooks::extra_time();
    let rows = state.sessions.lock().unwrap().pending(hold_time(state), &extra);
    let mut rows = with_icons(state, rows, "id");
    let processes = rows.iter().any(|r| r["state"] == "working").then(processes).unwrap_or_default();
    for row in &mut rows {
        let id = row["id"].as_str().unwrap_or_default().to_string();
        row["queued"] = hooks::queued(&id).into();
        let parent = state.sessions.lock().unwrap().hook_parent(&id);
        let running = parent.is_some_and(|p| !running_commands(&processes, p).is_empty());
        row["canStop"] = (row["state"] == "working" && running).into();
    }
    rows
}

/// Every process: its ID, its parent's, its process group, and its command line.
fn processes() -> Vec<(i32, i32, i32, String)> {
    let Ok(out) = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid=,pgid=,command="]).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            // Three numbers, padded to line up, then the command line with its own spaces
            let mut rest = line.trim_start();
            let mut number = || {
                let (n, after) = rest.split_once(char::is_whitespace)?;
                rest = after.trim_start();
                n.parse::<i32>().ok()
            };
            let (pid, ppid, pgid) = (number()?, number()?, number()?);
            Some((pid, ppid, pgid, rest.to_string()))
        })
        .collect()
}

/// The process groups of the commands a session's Claude Code is running for Claude, found from the process that ran
/// its hooks. Claude Code runs each command in a shell of its own, in a process group of its own, which starts by
/// loading a snapshot of the user's shell setup; that's how they're told apart from its other children, like MCP
/// servers.
fn running_commands(processes: &[(i32, i32, i32, String)], hook_parent: i32) -> Vec<i32> {
    let shell = |command: &str| {
        let name = command.split_whitespace().next().unwrap_or_default();
        matches!(name.rsplit('/').next(), Some("sh" | "bash" | "zsh" | "dash"))
    };
    // A shell in between the hook and Claude Code goes when the hook does; it's Claude Code's commands we're after
    let claude = match processes.iter().find(|p| p.0 == hook_parent) {
        Some((_, ppid, _, command)) if shell(command) && !command.contains("/shell-snapshots/") => *ppid,
        Some(_) => hook_parent,
        None => return vec![],
    };
    let claude_group = processes.iter().find(|p| p.0 == claude).map(|p| p.2);
    processes
        .iter()
        .filter(|(pid, ppid, pgid, command)| {
            *ppid == claude && pgid == pid && Some(*pgid) != claude_group && command.contains("/shell-snapshots/")
        })
        .map(|p| p.2)
        .collect()
}

/// Send Now: stop the command a working session is running, the way Esc in Claude Code does, so the message the user
/// sent it from the list goes in with the step's end right away, rather than whenever the command would have finished.
#[tauri::command]
fn stop_step(session: String, state: tauri::State<Arc<State>>) -> Result<(), String> {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGTERM: i32 = 15;
    const SIGKILL: i32 = 9;
    let parent = state.sessions.lock().unwrap().hook_parent(&session);
    let parent = parent.ok_or("Headroom can't tell which process that is")?;
    let groups = running_commands(&processes(), parent);
    if groups.is_empty() {
        return Err("It isn't running a command right now, so it'll see your message after what it's doing".into());
    }
    for group in &groups {
        // SAFETY: signals the command's own process group, which is never Claude Code's or Headroom's
        unsafe { kill(-group, SIGTERM) };
    }
    // Anything that doesn't stop when asked is made to, a moment later
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let left = processes();
        for group in groups.iter().filter(|g| left.iter().any(|p| p.2 == **g)) {
            // SAFETY: as above
            unsafe { kill(-group, SIGKILL) };
        }
    });
    Ok(())
}

#[tauri::command]
fn pending_sessions(state: tauri::State<Arc<State>>) -> Vec<Value> {
    pending(&state)
}

/// Mark finished sessions as seen from the list, the way opening their chats would. A turn held open for a reply lets
/// go, since none's coming from here.
#[tauri::command]
fn mark_seen(sessions: Vec<String>, state: tauri::State<Arc<State>>) {
    let rows = pending(&state);
    for id in &sessions {
        let reply = rows.iter().find(|p| p["id"] == id.as_str()).and_then(|p| p["replyId"].as_str().map(String::from));
        if let Some(reply) = reply {
            let _ = answer(&state, &reply, "terminal");
        }
        if state.sessions.lock().unwrap().mark_seen(id) {
            let at = Local::now().timestamp_millis();
            hooks::log(&json!({ "hook_event_name": "HeadroomSeen", "session_id": id, "at": at }));
        }
    }
    redraw(&state);
    changed(&state);
}

/// Open a session from the list, where it runs, and put the list away.
#[tauri::command]
fn open_pending(session: String, state: tauri::State<Arc<State>>) {
    *state.list_mode.lock().unwrap() = false;
    // A request Headroom's holding for it goes back to its own prompt, which is what the user will find there
    let row = pending(&state).into_iter().find(|p| p["id"] == session.as_str());
    let held = row.and_then(|p| p["heldId"].as_str().map(String::from));
    if let Some(held) = held {
        let _ = answer(&state, &held, "terminal");
    }
    hide_popover(&state.app);
    let host = state.sessions.lock().unwrap().app_of(&session);
    open_session(&state, &session, host);
}

/// A message for a session in the list, from the user. One at work gets it after its next step, or when it finishes, if
/// that comes first. One that's finished, with its turn held open for a reply, gets it as the reply.
#[tauri::command]
fn send_to_session(session: String, text: String, state: tauri::State<Arc<State>>) -> Result<(), String> {
    let row = pending(&state).into_iter().find(|p| p["id"] == session.as_str()).unwrap_or_default();
    match (row["state"].as_str(), row["replyId"].as_str()) {
        (Some("working"), _) => hooks::interject(&session, &text),
        (_, Some(reply)) => answer(&state, reply, &format!("reply:{text}")),
        _ => Err("That chat isn't waiting for a message any more".into()),
    }
}

/// The list gave way to a request's card: the popover's showing requests again, like any other time.
#[tauri::command]
fn popover_cards(state: tauri::State<Arc<State>>) {
    *state.list_mode.lock().unwrap() = false;
    // Gone in the meantime: nothing to show
    if held_requests(&state).is_empty() {
        hide_popover(&state.app);
    }
}

#[tauri::command]
fn refresh_now(state: tauri::State<Arc<State>>) {
    state.wake.notify_one();
}

#[tauri::command]
fn open_usage_page() {
    let _ = Command::new("open").arg(USAGE_PAGE).spawn();
}

#[tauri::command]
fn send_test_alert(state: tauri::State<Arc<State>>) {
    // In case macOS's question from when Headroom opened went unanswered
    notifications::ask();
    alert(&state, "Headroom", "Alerts are working.", None);
}

#[tauri::command]
fn check_updates(state: tauri::State<Arc<State>>) {
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move { check_for_update(&state, true).await });
}

#[tauri::command]
fn install_update_now(state: tauri::State<Arc<State>>) {
    update_clicked(state.inner().clone());
}

/*********************************
 * B U I L D I N G  T H E  M E N U
 ********************************/

/// Build the menu line for one limit, and load its saved alert threshold.
fn build_limit<M: Manager<Wry>>(app: &M, config_dir: &Path, info: LimitInfo) -> tauri::Result<Limit> {
    let key = info.key;
    let line = MenuItem::with_id(app, key, format!("{}: …", info.label), false, None::<&str>)?;

    // Anything that isn't one of `ALERT_LEVELS` (including a missing setting) is treated as "Off"
    let threshold = load_setting(config_dir, &format!("alert_{key}"))
        .and_then(|s| s.parse().ok())
        .filter(|t| ALERT_LEVELS.contains(t) && !info.optional);

    Ok(Limit {
        info,
        line,
        shown: Mutex::new(!info.optional),
        alert: Mutex::new(AlertState { threshold, fired: false }),
    })
}

/// Build the menu and the menu bar item, and return the state shared with the refresh loop.
fn build_state(app: &tauri::App) -> tauri::Result<State> {
    let config_dir = app.path().app_config_dir()?;
    let icons = icons::Icons::load(&config_dir);

    // If there's no saved interval, or it's no longer one of the options in `INTERVALS`, we'll use the default. Same
    // idea for what the menu bar shows.
    let interval = load_setting(&config_dir, "interval")
        .and_then(|s| s.parse().ok())
        .filter(|s| INTERVALS.contains(s))
        .unwrap_or(DEFAULT_INTERVAL);
    let saved_mode = load_setting(&config_dir, "title");
    let title_mode = TITLE_MODES.iter().find(|m| saved_mode.as_deref() == Some(**m)).copied().unwrap_or(TITLE_MODES[0]);
    let show_ring = load_setting(&config_dir, "ring").as_deref() != Some("off");

    let limits = [
        build_limit(app, &config_dir, LIMITS[0])?,
        build_limit(app, &config_dir, LIMITS[1])?,
        build_limit(app, &config_dir, LIMITS[2])?,
        build_limit(app, &config_dir, LIMITS[3])?,
    ];
    let status = MenuItem::with_id(app, "status", "Loading…", false, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", "Check for Updates…", true, None::<&str>)?;
    // Settings that are on unless they've been turned off
    let on = |key: &str| load_setting(&config_dir, key).as_deref() != Some("off");
    let auto_update = on("auto_update");
    let hooks_on = load_setting(&config_dir, "hooks").as_deref() == Some("on");
    let waiting_count = on("waiting_count");
    let approvals = (
        load_setting(&config_dir, "approvals").as_deref() == Some("on"),
        load_setting(&config_dir, "approval_hold")
            .and_then(|s| s.parse().ok())
            .filter(|s| APPROVAL_HOLDS.contains(s))
            .unwrap_or(120),
    );
    let saved_recap = load_setting(&config_dir, "recap_at");
    let recap_at = RECAP_TIMES.iter().find(|t| saved_recap.as_deref() == Some(**t)).copied().unwrap_or("18:00");
    let recap = (on("recap"), recap_at);
    let near_limit = NearLimit {
        on: load_setting(&config_dir, "near_limit").as_deref() == Some("on"),
        at: load_setting(&config_dir, "near_limit_at")
            .and_then(|s| s.parse().ok())
            .filter(|t| NEAR_LEVELS.contains(t))
            .unwrap_or(85),
        right_away: load_setting(&config_dir, "near_limit_right_away").as_deref() == Some("on"),
        message: load_setting(&config_dir, "near_limit_message").unwrap_or_default(),
    };
    let session_alerts = SessionAlerts {
        answer: on("notify_answer"),
        done: on("notify_done"),
        only_away: on("notify_done_only_away"),
        min_run: load_setting(&config_dir, "notify_done_min_run")
            .and_then(|s| s.parse().ok())
            .filter(|s| MIN_RUNS.contains(s))
            .unwrap_or(60),
        reset: on("notify_reset"),
    };
    let popover_auto = on("popover_auto");
    let compact_cards = load_setting(&config_dir, "compact_cards").as_deref() == Some("on");
    // Off until turned on, unlike the settings `on` reads
    let ask_next = load_setting(&config_dir, "ask_next").as_deref() == Some("on");
    let hands_free = load_setting(&config_dir, "hands_free").as_deref() == Some("on");
    let pace_arrows = load_setting(&config_dir, "pace_arrows").as_deref() == Some("on");
    let paused = load_paused(&config_dir);
    let persistent_alerts = on("persistent_alerts");
    let context_alert = match load_setting(&config_dir, "context_alert").as_deref() {
        Some("off") => None,
        saved => saved.and_then(|s| s.parse().ok()).filter(|t| CONTEXT_LEVELS.contains(t)).or(Some(70)),
    };

    // The usage lines go first, then the status line. `set_line_shown` counts on that order when it adds the Opus and
    // Sonnet lines. Everything you'd set once and leave is in Settings, to keep this menu short.
    let separator = || PredefinedMenuItem::separator(app);
    let separators = [separator()?, separator()?];
    let usage_page = MenuItem::with_id(app, "usage", "Open Usage Page", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh Now", true, Some("CmdOrCtrl+R"))?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let planner = MenuItem::with_id(app, "planner", "Plan Agents…", true, None::<&str>)?;
    let show_requests = MenuItem::with_id(app, "show_requests", "Show Waiting Requests", false, None::<&str>)?;
    let resume_item = MenuItem::with_id(app, "resume", "Resume Now", false, None::<&str>)?;
    let pause_menu = Submenu::with_items(
        app,
        "Pause Alerts and Requests",
        true,
        &[
            &resume_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "pause_30", "For 30 Minutes", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_60", "For 1 Hour", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_180", "For 3 Hours", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_tomorrow", "Until Tomorrow Morning", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_resumed", "Until I Resume", true, None::<&str>)?,
        ],
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, Some("CmdOrCtrl+Q"))?;
    let mut items: Vec<&dyn IsMenuItem<Wry>> =
        limits.iter().filter(|l| !l.info.optional).map(|l| &l.line as &dyn IsMenuItem<Wry>).collect();
    items.extend([
        &status as &dyn IsMenuItem<Wry>,
        &separators[0],
        &usage_page,
        &refresh,
        &separators[1],
        &show_requests,
        &pause_menu,
        &planner,
        &settings,
        &update_item,
        &quit,
    ]);
    let menu = Menu::with_items(app, &items)?;
    drop(items);
    let model_line = MenuItem::with_id(app, "model_line", "", false, None::<&str>)?;

    // The real title gets filled in after the first check. The ring is in color, so it's not a template image (macOS
    // would turn a template image black or white to match the menu bar).
    let mut tray = TrayIconBuilder::with_id("usage").menu(&menu).icon_as_template(false).on_tray_icon_event(tray_clicked);
    tray = match title_mode {
        "rings" | "rings_model" => {
            tray.icon(ring_icon(&[(0.0, RING_FIVE_HOUR, None), (0.0, RING_FILL, None)])).title("…")
        }
        _ if show_ring || title_mode == "ring" => tray.icon(ring_icon(&[(0.0, RING_FILL, None)])).title("…"),
        _ => tray.title("✻ …"),
    };
    let tray = tray.build(app)?;

    Ok(State {
        app: app.handle().clone(),
        wake: Notify::new(),
        interval: Mutex::new(interval),
        title_mode: Mutex::new(title_mode),
        show_ring: Mutex::new(show_ring),
        config_dir,
        tray,
        menu,
        status,
        limits,
        last_usage: Mutex::new(None),
        model_limits: Mutex::new(vec![]),
        model_line,
        model_line_shown: Mutex::new(false),
        login_check: Mutex::new(LoginCheck::default()),
        retry_at: Mutex::new(None),
        auto_update: Mutex::new(auto_update),
        hooks_on: Mutex::new(hooks_on),
        sessions: Mutex::new(sessions::Sessions::default()),
        waiting_count: Mutex::new(waiting_count),
        session_alerts: Mutex::new(session_alerts),
        near_limit: Mutex::new(near_limit),
        transcripts: Mutex::new(transcripts::Transcripts::default()),
        shares: Mutex::new(vec![]),
        recap: Mutex::new(recap),
        approvals: Mutex::new(approvals),
        seen_requests: Mutex::new(HashSet::new()),
        popover_auto: Mutex::new(popover_auto),
        compact_cards: Mutex::new(compact_cards),
        ask_next: Mutex::new(ask_next),
        hands_free: Mutex::new(hands_free),
        pace_arrows: Mutex::new(pace_arrows),
        paces: Mutex::new(vec![]),
        persistent_alerts: Mutex::new(persistent_alerts),
        context_alert: Mutex::new(context_alert),
        contexts: Mutex::new(context::Contexts::default()),
        desktop: Mutex::new(desktop::Desktop::default()),
        clicks: Mutex::new(std::collections::VecDeque::new()),
        show_requests,
        pause_menu,
        resume_item,
        paused: Mutex::new(paused),
        list_mode: Mutex::new(false),
        told_front: Mutex::new(None),
        icons: Mutex::new(icons),
        today: Mutex::new(Today::default()),
        resets: Mutex::new([None, None]),
        update_item,
        pending_update: Mutex::new(None),
    })
}

/*********
 * M A I N
 ********/

fn main() {
    // Claude Code runs this same binary for each hook event (see hooks.rs). That run does its one job and exits,
    // without starting the app.
    if std::env::args().any(|a| a == hooks::HOOK_ARG) {
        hooks::run();
        return;
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            // Run as a menu bar only app, with no Dock icon and no app menu
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            let state = Arc::new(build_state(app)?);
            let handle = app.handle().clone();
            notifications::start(move |clicked| {
                let app = handle.clone();
                let _ = handle.run_on_main_thread(move || notification_clicked(&app, clicked));
            });
            app.manage(state.clone());
            show_paused(&state);
            tauri::async_runtime::spawn(refresh_loop(state.clone()));

            // If the hooks are on, point them at this copy of Headroom, in case it moved since they were added
            if *state.hooks_on.lock().unwrap() {
                let _ = install_hooks(&state);
            }
            tauri::async_runtime::spawn(follow_sessions(state.clone()));
            tauri::async_runtime::spawn(recap_loop(state.clone()));

            // Redraw the title every 30 seconds between checks so the time until reset keeps counting down
            let title_state = state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    update_title(&title_state);
                    // And the pause: its time in the menu, and its end
                    let state = title_state.clone();
                    let _ = title_state.app.run_on_main_thread(move || show_paused(&state));
                }
            });

            // Check for updates once at launch, then once a day, unless that's been turned off in Settings
            tauri::async_runtime::spawn(async move {
                loop {
                    if *state.auto_update.lock().unwrap() {
                        check_for_update(&state, false).await;
                    }
                    tokio::time::sleep(UPDATE_CHECK_EVERY).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            has_token,
            save_token,
            clear_token,
            get_state,
            get_wallpaper,
            set_setting,
            answer_request,
            save_attachment,
            extend_request,
            get_asks,
            pending_sessions,
            send_to_session,
            stop_step,
            set_icon,
            plans,
            save_plan,
            delete_plan,
            open_planner_now,
            emoji_names,
            mark_seen,
            open_pending,
            popover_cards,
            popover_pointer,
            show_popover_now,
            open_notification_settings,
            popover_arrow,
            popover_resize,
            close_popover,
            refresh_now,
            open_usage_page,
            send_test_alert,
            check_updates,
            install_update_now
        ])
        .on_menu_event(|app, event| menu_clicked(app.state::<Arc<State>>().inner().clone(), event.id().as_ref()))
        .build(tauri::generate_context!())
        .expect("failed to build app");

    // Closing the last window would normally quit the app. Only an exit that comes with a code (from "Quit") is
    // allowed through.
    app.run(|_, event| {
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_the_commands_claude_is_running() {
        let p = |pid, ppid, pgid, command: &str| (pid, ppid, pgid, command.to_string());
        let snapshot = "/bin/zsh -c source /Users/me/.claude/shell-snapshots/snapshot-zsh-1.sh && eval 'cargo build'";
        let processes = vec![
            p(100, 1, 100, "/Applications/Claude.app/Contents/MacOS/Claude"),
            p(200, 100, 200, "/Users/me/Library/Application Support/Claude/claude-code/2.1/claude --output-format"),
            // An MCP server, and a command, both Claude Code's
            p(300, 200, 200, "node /Users/me/mcp/server.js"),
            p(400, 200, 400, snapshot),
            p(401, 400, 400, "cargo build"),
            // Another session's command
            p(500, 1, 500, "claude"),
            p(600, 500, 600, snapshot),
        ];
        assert_eq!(running_commands(&processes, 200), vec![400]);
        // Reached through a shell that ran the hook
        let mut with_shell = processes.clone();
        with_shell.push(p(700, 200, 200, "/bin/sh -c headroom --headroom-hook"));
        assert_eq!(running_commands(&with_shell, 700), vec![400]);
        assert!(running_commands(&processes, 999).is_empty());
    }

    #[test]
    fn reads_the_process_list() {
        let me = std::process::id() as i32;
        let list = processes();
        let (_, ppid, _, command) = list.iter().find(|p| p.0 == me).expect("this test's own process");
        assert_eq!(*ppid, std::os::unix::process::parent_id() as i32);
        assert!(command.contains("headroom"), "{command}");
    }

    #[test]
    fn a_models_own_weekly_limit_comes_from_the_limits_list() {
        let v = json!({
            "five_hour": { "utilization": 6.0, "resets_at": "2026-09-26T21:30:00+00:00" },
            "seven_day": { "utilization": 91.0, "resets_at": "2026-09-26T20:00:00+00:00" },
            "seven_day_opus": { "utilization": null, "resets_at": null },
            "seven_day_omelette": { "utilization": 3.0, "resets_at": null },
            "limits": [
                { "kind": "session", "percent": 6, "scope": null },
                { "kind": "weekly_all", "percent": 91, "scope": null },
                { "kind": "weekly_scoped", "percent": 16, "resets_at": "2026-09-26T20:00:00+00:00",
                  "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } },
                { "kind": "weekly_scoped", "percent": 40, "scope": { "model": null, "surface": "chat" } }
            ]
        });
        let (usage, models) = parse_usage(&v);
        assert_eq!(usage[1].as_ref().unwrap().pct, 91.0);
        assert!(usage[2].is_none());
        // Only the model's limit: nothing for a surface, and nothing from the fields named after other things
        assert_eq!(models.len(), 1);
        let fable = &models[0];
        assert_eq!((fable.key.as_str(), fable.label.as_str(), fable.window.pct), ("7d_fable", "Fable weekly", 16.0));
        assert!(fable.window.resets_at.is_some());
    }

    #[test]
    fn a_model_in_a_rate_limit_header_is_picked_up_by_its_name() {
        assert_eq!(unlisted_model("7d_fable").as_deref(), Some("fable"));
        assert!(unlisted_model("7d_opus").is_none());
        assert!(unlisted_model("7d").is_none());
        assert!(unlisted_model("overage").is_none());
    }

    #[test]
    fn active_time_leaves_out_breaks() {
        let start = chrono::Local::now();
        let at = |mins: i64| start + chrono::Duration::minutes(mins);
        // Two minutes of work, a half-hour break, then three more minutes
        let mut replies = vec![at(0), at(35), at(1), at(2), at(33), at(32)];
        assert_eq!(active_time(&mut replies), chrono::Duration::minutes(5));
    }

    #[test]
    fn usage_rounds_up_like_claude_ai() {
        assert_eq!(whole_pct(76.3), 77.0);
        assert_eq!(whole_pct(0.76 * 100.0), 76.0);
        assert_eq!(whole_pct(0.0), 0.0);
        assert_eq!(whole_pct(99.2), 100.0);
    }
}
