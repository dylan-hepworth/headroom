// Claude Code hooks: how Headroom finds out what each Claude Code session is doing.
//
// Turning on Hooks in Settings adds a few entries to ~/.claude/settings.json. Each one has Claude Code run this same
// binary with `--headroom-hook` when something happens in a session, and hand it the details on stdin. That run
// appends the few fields we need to a log file and exits; the running app reads the log (see sessions.rs). Going
// through a file means nothing is lost while Headroom isn't running, and there's no socket or port to manage.
//
// The hooks are async, so Claude Code starts them and moves on without waiting. Headroom can never slow a session down
// or get in its way. The exceptions are for "Add a note when I'm near a limit": while it's on, Claude Code waits for
// the hook on each submitted prompt (about 20 ms) so it can add the note, and with "Right away" also for the hook after
// each tool call, so a session that's in the middle of something gets the note too. And for "Approve from the menu
// bar", where the permission hook holds a request until the user answers it in Headroom (see `hold_for_answer`), and
// Claude Code also waits for the hooks on each prompt and at the end of each turn, for "Have Claude ask what's next"
// (see `ask_note` and `nudge`). Those are in place whenever approvals are, so that turning that on or off reaches the
// sessions that are already open: a session keeps the hooks it started with. In hands-free mode, Claude Code also waits
// for the hook after each tool call, which hands Claude any message the user sent from the list (see `interject`).

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};

/// The argument that tells this binary it's running as a hook rather than as the app. It also marks our entries in
/// settings.json, so we can find them again without touching anyone else's.
pub const HOOK_ARG: &str = "--headroom-hook";

/// Given to the hooks Claude Code waits for (the prompt and end-of-turn hooks, and in hands-free the one after each
/// tool call), so they know they can change the turn they're for (see `waited_on`).
const WAIT_ARG: &str = "--wait";

/// Given to the hooks Claude Code waits for (permission requests and questions, while approvals are on), with how long
/// to hold a request, e.g. `--hold=120`. A hook only ever holds when it was started with this. A session keeps the
/// hooks it started with, so one that began while approvals were off runs these hooks without waiting for them, and
/// holding there would answer a request nobody is waiting on. The hold comes with it for the same reason: it matches
/// the timeout Claude Code gave that session's hook.
const HOLD_ARG: &str = "--hold=";

/// How long this run of the hook may hold a request, from its `--hold=` argument.
fn hold_arg() -> Option<std::time::Duration> {
    std::env::args().find_map(|a| a.strip_prefix(HOLD_ARG)?.parse().ok()).map(std::time::Duration::from_secs)
}

/// The events we listen for, and the matcher for any that need one. Claude Code's "waiting for you" notifications
/// aren't here: PermissionRequest and Stop already tell us the moment a session starts waiting, and the notifications
/// come seconds to a minute later, when the user may already have answered.
const EVENTS: [(&str, Option<&str>); 9] = [
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    // When Claude asks the user a multiple-choice question, which needs an answer just like a permission request
    ("PreToolUse", Some("AskUserQuestion")),
    ("PermissionRequest", None),
    ("PostToolUse", None),
    ("PostToolUseFailure", None),
    ("Stop", None),
    ("StopFailure", None),
    ("SessionEnd", None),
];

/// The most we keep of any one piece of text from an event, like a command or the start of Claude's reply.
const MAX_TEXT: usize = 300;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Where the hooks write and the app reads. It's the app's own settings folder, which the hook finds without Tauri.
pub fn log_path() -> Option<PathBuf> {
    Some(home()?.join("Library/Application Support/io.github.dylan-hepworth.headroom/sessions.log"))
}

/// Where the app leaves the latest usage for the prompt hook: see `write_limits`.
pub fn limits_path() -> Option<PathBuf> {
    Some(log_path()?.with_file_name("limits.json"))
}

/// Usage older than this is left out of the note, so a Headroom that quit a while ago can't tell Claude old numbers.
const LIMITS_FRESH_MS: i64 = 15 * 60 * 1000;

/// What the note asks of Claude, unless the user wrote their own in Settings.
pub const DEFAULT_ADVICE: &str = "Prefer small steps, and save progress before starting anything long.";

/// The option Claude is asked to end its "what's next" question with.
const DONE_OPTION: &str = "That's all for now";

/// What "Have Claude ask what's next" adds to each message while it's on.
const ASK_NOTE: &str =
    "Headroom: when you finish, ask the user what to do next with the AskUserQuestion tool. Ask one \
    question, with two to four short options, and make the last one \"That's all for now\". Ask anything else you need \
    from them the same way, rather than in plain text.";

/// Instead, when Claude only asks what's next when there's something to decide: otherwise it ends its turn with the
/// whole of what it has to say, and the user answers that from Headroom.
const ASK_WHEN_NEEDED_NOTE: &str = "Headroom: ask the user with the AskUserQuestion tool only when you need them to \
    decide something: one question, with two to four short options, the last one \"That's all for now\". When you're \
    telling them something, or you're done, end your turn with your full answer rather than a question; they can reply \
    from Headroom. This replaces any earlier Headroom note to always ask.";

/// Added to either in hands-free mode, where the user follows along in the popover rather than the chat.
const HANDS_FREE_NOTE: &str = "The user reads your reply in Headroom's panel rather than the chat, so end with a \
    short markdown summary of what you did and anything they should look over.";

/// What Claude is told when a turn ends without the question, and it goes back to ask.
const ASK_NUDGE: &str = "Headroom: before you stop, ask the user what to do next with the AskUserQuestion tool. Ask \
    one question, with two to four short options, and make the last one \"That's all for now\". If they've just told \
    you they're done, stop without asking.";

/// What a session that was asked to do that hears, once, after it's turned off.
const STOP_ASKING_NOTE: &str = "Headroom: the user turned off asking what's next. Ignore the earlier Headroom notes \
    about asking with AskUserQuestion when you finish, and end your turns the way you normally would.";

/// How a message the user sends a working session from the list reaches Claude.
const INTERJECTED: &str = "The user sent this from Headroom while you were working:";

/// How a report from one of a team's managers reaches the lead.
const TEAM_REPORT: &str = "Headroom has a report from one of your team's managers, which runs as a session of its own:";

/// Where we remember which sessions were asked to ask what's next, one file per session, so the ones that were can be
/// told to stop, and when each was last sent back to ask.
fn asking_dir() -> Option<PathBuf> {
    Some(log_path()?.with_file_name("asking"))
}

/// Where we remember which sessions already got the note after a tool call, one empty file per session and note.
fn sent_dir() -> Option<PathBuf> {
    Some(log_path()?.with_file_name("notes-sent"))
}

/// Where the app says whether approvals are on and how long to hold a request. The app rewrites it every few seconds,
/// so a hook can tell Headroom is still running: see `APP_ALIVE_MS`.
fn approvals_path() -> Option<PathBuf> {
    Some(log_path()?.with_file_name("approvals.json"))
}

/// Where the app leaves the user's answer to a request, one file per request.
fn answers_dir() -> Option<PathBuf> {
    Some(log_path()?.with_file_name("answers"))
}

/// Where messages the user sends a working session from the list wait for it, one folder per session and one file per
/// message (see `interject`).
fn interject_dir(session: &str) -> Option<PathBuf> {
    Some(log_path()?.with_file_name("interject").join(session.replace(['/', '.'], "")))
}

/// The most extra time pausing can add to a request, in total. The hook's timeout in settings.json allows for it.
const MAX_PAUSE: std::time::Duration = std::time::Duration::from_secs(120);

/// If approvals.json is older than this, Headroom isn't running (or has hung), and a held request goes back to Claude
/// Code's own prompt.
const APP_ALIVE_MS: i64 = 15_000;

fn settings_path() -> Option<PathBuf> {
    Some(home()?.join(".claude/settings.json"))
}

/*********************************
 * R U N N I N G  A S  A  H O O K
 ********************************/

/// What this binary does when Claude Code runs it as a hook: read the event from stdin, keep the parts we use, and
/// append them to the log as one line. Any failure is silently dropped, since there's no one to tell.
pub fn run() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(event) = serde_json::from_str::<Value>(&input) else { return };
    let mut kept = keep(&event);
    let name = event["hook_event_name"].as_str().unwrap_or_default();
    let session = event["session_id"].as_str().unwrap_or_default();

    // A message the user sent from the list while the session worked goes to Claude now, if it can. At the end of a
    // turn, it keeps the turn going, ahead of asking what's next or waiting for a reply.
    let interjected = interjections(name, &event);
    // A turn that's sent back to ask what's next isn't over, so that's worked out before the event is logged
    let nudge = (name == "Stop" && interjected.is_none()).then(|| nudge(&event)).flatten();
    // A permission request or question we can hold for an answer gets an ID, so the app can show it and answer it. So
    // does a finished turn in hands-free, held open a while for a reply from the popover.
    let for_reply = name == "Stop" && nudge.is_none() && interjected.is_none() && holds_for_reply(&event);
    let hold =
        hold_arg().filter(|_| (for_reply || holdable(&event)) && team().is_none() && app_says_hold() && !watching());
    if let Some(hold) = hold {
        let id = format!(
            "{}-{}-{}",
            session.replace(['/', '.'], ""),
            chrono::Utc::now().timestamp_millis(),
            std::process::id()
        );
        kept["request_id"] = id.into();
        kept["hold"] = hold.as_secs().into();
        kept["pid"] = std::process::id().into();
        kept["can_session"] = event["permission_suggestions"].as_array().is_some_and(|s| !s.is_empty()).into();
        // The whole command, not the clipped one: this is what Allow approves, so the popover shows all of it
        if let Some(full) = full_detail(event["tool_name"].as_str().unwrap_or_default(), &event["tool_input"]) {
            kept["full_detail"] = full.into();
        }
    }
    if hold.is_some() && for_reply {
        kept["reply"] = true.into();
    }
    // Hands-free, a question comes with the conversation around it, kept beside the request rather than in the log
    if let (Some(id), true) = (kept["request_id"].as_str(), event["tool_name"] == "AskUserQuestion" || for_reply) {
        if hands_free() && save_context(id, &event).is_some() {
            kept["context"] = true.into();
        }
    }
    if nudge.is_some() {
        kept["nudged"] = true.into();
    }
    if interjected.is_some() {
        kept["interjected"] = true.into();
    }
    log(&kept);
    // A message from the user, so being sent back to ask what's next starts over
    if interjected.is_some() {
        fresh_message(session);
    }
    let interjection = interjected.map(|waiting| waiting.say(false));

    if let (Some(hold), Some(id), Some(at)) = (hold, kept["request_id"].as_str(), kept["at"].as_i64()) {
        hold_for_answer(id, at, hold, &event);
        return;
    }
    if let (Some(message), "Stop") = (&interjection, name) {
        println!("{}", json!({ "decision": "block", "reason": with_notes(message) }));
        return;
    }
    if let Some(reason) = nudge {
        println!("{}", json!({ "decision": "block", "reason": reason }));
        return;
    }
    // Messages from the list it never took go with it
    if name == "SessionEnd" {
        let _ = interject_dir(session).map(fs::remove_dir_all);
    }
    // Cleared, the conversation starts over without the note. Ended any other way, it can be resumed with it.
    if name == "SessionEnd" && event["reason"] == "clear" {
        let _ = asking_file(session).map(fs::remove_file);
        return;
    }

    // For a prompt, or after a tool call, anything we print goes to Claude
    let notes = [interjection, ask_note(name, &event), limit_note(name, &event)];
    let notes: Vec<String> = notes.into_iter().flatten().collect();
    if !notes.is_empty() {
        let context = notes.join("\n\n");
        println!("{}", json!({ "hookSpecificOutput": { "hookEventName": name, "additionalContext": context } }));
    }
}

/// The near-limit note, for a prompt, or after a tool call with "Right away" (once per session for each note).
fn limit_note(name: &str, event: &Value) -> Option<String> {
    if !matches!(name, "UserPromptSubmit" | "PostToolUse") {
        return None;
    }
    let limits = fs::read_to_string(limits_path()?).ok()?;
    let (note, key) = near_limit_note(&limits)?;
    if name == "PostToolUse" {
        let right_away = serde_json::from_str::<Value>(&limits).is_ok_and(|l| l["right_away"] == true);
        if !right_away || !first_time(&key, event["session_id"].as_str().unwrap_or_default()) {
            return None;
        }
    }
    Some(note)
}

/*************************************************
 * H A V E  C L A U D E  A S K  W H A T ' S  N E X T
 ************************************************/

/// Is "Have Claude ask what's next" on? None when Headroom isn't running and it was: then sessions are left as they
/// are, rather than told to stop every time the Mac sleeps. Off is off, however old the word, so turning Hooks off
/// (which stops the updates) still reaches the sessions that heard about it.
fn asking() -> Option<bool> {
    let config: Value = serde_json::from_str(&fs::read_to_string(approvals_path()?).ok()?).ok()?;
    let alive = config["at"].as_i64().is_some_and(|at| chrono::Utc::now().timestamp_millis() - at < APP_ALIVE_MS);
    match config["ask"] == true {
        true => alive.then_some(true),
        false => Some(false),
    }
}

/// Does Claude always ask what's next, rather than only when there's something to decide? Only then is a turn that
/// ends without the question sent back to ask.
fn asking_always() -> bool {
    let Some(config) = approvals_path().and_then(|p| fs::read_to_string(p).ok()) else { return false };
    serde_json::from_str::<Value>(&config).is_ok_and(|c| c["always"] == true)
}

/// Is Claude Code waiting for this run of the hook? Only then does anything it says change the turn it's for. A session
/// keeps the hooks it started with, so one from before approvals were on runs them without waiting.
fn waited_on() -> bool {
    std::env::args().any(|a| a == WAIT_ARG)
}

/// The last thing the user said in a conversation, from its transcript, for the planner to offer as the work a team's
/// added for (see Planner.tsx). Only ever read for the user, in the planner, and never kept.
pub fn last_prompt(transcript: &Path) -> Option<String> {
    conversation(&tail(transcript)?).0
}

/// Is someone there to answer? Scripts and apps built on the Agent SDK run Claude Code with nobody watching, and so
/// does Headroom, for a team's managers, whatever Claude Code was told about where it was started from.
fn attended() -> bool {
    !std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok_and(|e| e.starts_with("sdk")) && team().is_none()
}

/// The team and agent this session runs for, when Headroom started it as a manager for a team from the planner
/// (`<run>:<agent>`). It runs in the background, so nobody's there to answer, but messages from the user reach it.
fn team() -> Option<String> {
    std::env::var("HEADROOM_TEAM").ok().filter(|t| !t.is_empty())
}

fn asking_file(session: &str) -> Option<PathBuf> {
    Some(asking_dir()?.join(session.replace(['/', '.'], "")))
}

/// The user said something, so a session that was sent back to ask what's next can be again.
fn fresh_message(session: &str) {
    if let Some(file) = asking_file(session).filter(|f| f.exists()) {
        let _ = fs::write(file, "{}");
    }
}

/// With each message: the note while it's on, and once it's off, word to stop for a session that had it.
fn ask_note(name: &str, event: &Value) -> Option<String> {
    let session = event["session_id"].as_str().unwrap_or_default();
    if name != "UserPromptSubmit" || session.is_empty() || !attended() || !waited_on() {
        return None;
    }
    let file = asking_file(session)?;
    match asking()? {
        true => {
            // A new message, so being sent back to ask starts over
            let _ = fs::create_dir_all(file.parent()?);
            fs::write(&file, "{}").ok()?;
            let note = if asking_always() { ASK_NOTE } else { ASK_WHEN_NEEDED_NOTE };
            Some(if hands_free() { format!("{note} {HANDS_FREE_NOTE}") } else { note.into() })
        }
        // Removing the file is the check, so a session is only told once. One that's lost its file (resumed after a
        // long while, or as a fork) goes by whether its own transcript has the note since it was last told.
        false => {
            let removed = fs::remove_file(&file).is_ok();
            let heard = || event["transcript_path"].as_str().is_some_and(|t| still_asking(Path::new(t)));
            (removed || heard()).then(|| STOP_ASKING_NOTE.into())
        }
    }
}

/// Does the end of this transcript have the note to ask what's next, and no word to stop after it?
fn still_asking(transcript: &Path) -> bool {
    let Some(tail) = tail(transcript) else { return false };
    let asked = tail.rfind(&ASK_NOTE[..40]).max(tail.rfind(&ASK_WHEN_NEEDED_NOTE[..40]));
    asked.is_some() && asked > tail.rfind(&STOP_ASKING_NOTE[..40])
}

/// Is hands-free mode on, and Headroom running to answer from?
fn hands_free() -> bool {
    let Some(config) = approvals_path().and_then(|p| fs::read_to_string(p).ok()) else { return false };
    let Ok(config) = serde_json::from_str::<Value>(&config) else { return false };
    let alive = config["at"].as_i64().is_some_and(|at| chrono::Utc::now().timestamp_millis() - at < APP_ALIVE_MS);
    alive && config["hands_free"] == true
}

/// Where a held question's conversation is kept while it waits (see `save_context`).
fn context_path(id: &str) -> Option<PathBuf> {
    Some(answers_dir()?.join(format!("{id}.context.json")))
}

/// Keep what the user last said and what Claude has said since, for the popover to show with the question. It's in a
/// file of its own, only this user can read, which goes when the question does (see `hold_for_answer`): the log never
/// has what the user types.
fn save_context(id: &str, event: &Value) -> Option<()> {
    let lines = tail(Path::new(event["transcript_path"].as_str()?))?;
    let (prompt, said) = conversation(&lines);
    // At the end of a turn, Claude Code hands over the last reply itself
    let said = event["last_assistant_message"].as_str().filter(|s| !s.trim().is_empty()).map_or(said, String::from);
    let path = context_path(id)?;
    fs::create_dir_all(path.parent()?).ok()?;
    let cwd = Path::new(event["cwd"].as_str().unwrap_or_default());
    let text = json!({
        "prompt": prompt.map(|p| clip_to(&p, 8_000, false)),
        "said": clip_to(&said, 40_000, true),
        "recent": recent(&lines, 10),
        "named": crate::made::named(&said, cwd),
    });
    let mut file = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(path).ok()?;
    file.write_all(text.to_string().as_bytes()).ok()
}

/// The user's last message in a transcript, and what Claude has said since: what it wrote after its last tool call
/// (its summary, just before it asks), or failing that, the last thing it wrote.
fn conversation(lines: &str) -> (Option<String>, String) {
    let mut prompt = None;
    let mut fallback_prompt = None;
    // What Claude wrote after its last tool call, and the last thing it wrote at all, since the user's message
    let mut after_tools: Vec<String> = vec![];
    let mut last_text: Option<String> = None;
    let text_of = |items: &[Value]| -> Vec<String> {
        items.iter().filter(|i| i["type"] == "text").filter_map(|i| i["text"].as_str().map(String::from)).collect()
    };
    let new_prompt = |text: &str, prompt: &mut Option<String>, after: &mut Vec<String>, last: &mut Option<String>| {
        // Not the notes Claude Code files under the user's name: a background task finishing, a slash command's
        // output, and the like
        let from_claude_code = ["<task-notification>", "<command-", "<local-command-", "<system-reminder>", "<bash-"]
            .iter()
            .any(|tag| text.trim_start().starts_with(tag));
        if !text.trim().is_empty() && !from_claude_code {
            *prompt = Some(text.trim().to_string());
            after.clear();
            *last = None;
        }
    };
    for line in lines.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if !by_the_user(&entry) {
            continue;
        }
        let content = &entry["message"]["content"];
        match entry["type"].as_str() {
            Some("user") => match content {
                Value::String(text) => new_prompt(text, &mut prompt, &mut after_tools, &mut last_text),
                // Answered a question: what Claude said before it has been read and acted on
                Value::Array(_) if entry["toolUseResult"]["answers"].is_object() => {
                    after_tools.clear();
                    last_text = None;
                }
                Value::Array(items) if items.iter().any(|i| i["type"] == "tool_result") => after_tools.clear(),
                Value::Array(items) => {
                    new_prompt(&text_of(items).join("\n\n"), &mut prompt, &mut after_tools, &mut last_text)
                }
                _ => {}
            },
            // A message sent while Claude was working
            Some("attachment") if entry["attachment"]["type"] == "queued_command" => {
                let text = queued_text(&entry["attachment"]["prompt"]);
                new_prompt(&text, &mut prompt, &mut after_tools, &mut last_text);
            }
            // Claude Code notes the last message it was sent every so often: a stand-in if the message itself is
            // further back than the part of the transcript that's read
            Some("last-prompt") => fallback_prompt = entry["lastPrompt"].as_str().map(|p| p.trim().to_string()),
            Some("assistant") => {
                let texts = text_of(content.as_array().map(Vec::as_slice).unwrap_or_default());
                let texts: Vec<String> = texts.into_iter().filter(|t| !t.trim().is_empty()).collect();
                if let Some(text) = texts.last() {
                    last_text = Some(text.clone());
                }
                after_tools.extend(texts);
            }
            _ => {}
        }
    }
    let said = if after_tools.is_empty() { last_text.unwrap_or_default() } else { after_tools.join("\n\n") };
    (prompt.or(fallback_prompt).filter(|p| !p.is_empty()), said.trim().to_string())
}

/// Is this transcript entry part of the conversation itself? Not a subagent's, not one Claude Code adds for itself, and
/// not the summary it writes when it compacts a conversation, which it files under the user's name.
fn by_the_user(entry: &Value) -> bool {
    let origin = entry["origin"]["kind"].as_str().or(entry["attachment"]["origin"]["kind"].as_str());
    entry["isSidechain"] != true
        && entry["isMeta"] != true
        && entry["attachment"]["isMeta"] != true
        && entry["isCompactSummary"] != true
        && entry["isVisibleInTranscriptOnly"] != true
        && origin.is_none_or(|kind| kind == "human")
}

/// The words of a message sent while Claude was working: just text, or text alongside an image.
fn queued_text(prompt: &Value) -> String {
    match prompt {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter(|i| i["type"] == "text")
            .filter_map(|i| i["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// What's been going on lately, for the popover to show without Claude writing any of it: the user's messages and
/// answers, what Claude wrote, and in between, what it did ("Ran 3 commands, edited 2 files"). The last `keep` of them.
fn recent(lines: &str, keep: usize) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    // Tools used since the last thing said, by what they did, in the order first used
    let mut did: Vec<(&'static str, usize)> = vec![];
    let flush = |did: &mut Vec<(&'static str, usize)>, out: &mut Vec<Value>| {
        if did.is_empty() {
            return;
        }
        let parts: Vec<String> = did.drain(..).map(|(what, n)| tool_phrase(what, n)).collect();
        let text = parts.join(", ");
        let mut text: Vec<char> = text.chars().collect();
        text[0] = text[0].to_ascii_uppercase();
        out.push(json!({ "kind": "did", "text": text.into_iter().collect::<String>() }));
    };
    let from_claude_code = |text: &str| {
        ["<task-notification>", "<command-", "<local-command-", "<system-reminder>", "<bash-"]
            .iter()
            .any(|tag| text.trim_start().starts_with(tag))
    };
    let mut asked: std::collections::HashMap<String, String> = Default::default();
    for line in lines.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if !by_the_user(&entry) {
            continue;
        }
        let items = entry["message"]["content"].as_array().map(Vec::as_slice).unwrap_or_default();
        let message = match (entry["type"].as_str(), &entry["message"]["content"]) {
            (Some("user"), Value::String(text)) => Some(text.clone()),
            (Some("user"), _) if !items.iter().any(|i| i["type"] == "tool_result") => Some(
                items
                    .iter()
                    .filter(|i| i["type"] == "text")
                    .filter_map(|i| i["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (Some("attachment"), _) if entry["attachment"]["type"] == "queued_command" => {
                Some(queued_text(&entry["attachment"]["prompt"]))
            }
            _ => None,
        };
        if let Some(text) = message.filter(|t| !t.trim().is_empty() && !from_claude_code(t)) {
            flush(&mut did, &mut out);
            out.push(json!({ "kind": "said", "text": clip_to(text.trim(), 300, false) }));
            continue;
        }
        for item in items {
            match (entry["type"].as_str(), item["type"].as_str()) {
                (Some("assistant"), Some("text")) => {
                    let text = item["text"].as_str().unwrap_or_default().trim();
                    if !text.is_empty() {
                        flush(&mut did, &mut out);
                        out.push(json!({ "kind": "wrote", "text": clip_to(text, 300, false) }));
                    }
                }
                (Some("assistant"), Some("tool_use")) if item["name"] == "AskUserQuestion" => {
                    let question = item["input"]["questions"][0]["question"].as_str().unwrap_or_default();
                    asked.insert(item["id"].as_str().unwrap_or_default().to_string(), question.to_string());
                }
                (Some("assistant"), Some("tool_use")) => {
                    let what = tool_kind(item["name"].as_str().unwrap_or_default());
                    match did.iter_mut().find(|(w, _)| *w == what) {
                        Some((_, n)) => *n += 1,
                        None => did.push((what, 1)),
                    }
                }
                (Some("user"), Some("tool_result")) => {
                    let Some(question) = asked.get(item["tool_use_id"].as_str().unwrap_or_default()) else { continue };
                    let Some(answers) = entry["toolUseResult"]["answers"].as_object() else { continue };
                    let answer = answers.values().filter_map(Value::as_str).collect::<Vec<_>>().join("; ");
                    flush(&mut did, &mut out);
                    out.push(json!({
                        "kind": "answered",
                        "about": clip_to(question, 160, false),
                        "text": clip_to(&answer, 300, false),
                    }));
                }
                _ => {}
            }
        }
    }
    flush(&mut did, &mut out);
    let skip = out.len().saturating_sub(keep);
    out.into_iter().skip(skip).collect()
}

/// What a tool does, in the words `tool_phrase` puts it in.
fn tool_kind(tool: &str) -> &'static str {
    match tool {
        "Bash" => "ran",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "edited",
        "Read" => "read",
        "Grep" | "Glob" => "searched",
        "WebFetch" | "WebSearch" => "looked up",
        "Task" | "Agent" => "started",
        _ => "used",
    }
}

/// "ran 3 commands", "edited a file", and so on.
fn tool_phrase(what: &str, n: usize) -> String {
    let count = |one: &str, many: &str| if n == 1 { one.to_string() } else { format!("{n} {many}") };
    match what {
        "ran" => format!("ran {}", count("a command", "commands")),
        "edited" => format!("edited {}", count("a file", "files")),
        "read" => format!("read {}", count("a file", "files")),
        "searched" => format!("searched {}", if n == 1 { "once".into() } else { format!("{n} times") }),
        "looked up" => format!("looked up {}", count("a page", "pages")),
        "started" => format!("started {}", count("an agent", "agents")),
        _ => format!("used {}", count("another tool", "other tools")),
    }
}

/// At most `max` characters of `text`: its start, or with `from_end`, its end.
fn clip_to(text: &str, max: usize, from_end: bool) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if from_end {
        format!("…{}", text.chars().skip(count - max).collect::<String>())
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// Hands-free: should the end of this turn wait a while for a reply from the popover? Not when there's nobody there,
/// the turn is waiting on work running in the background or a plan, or the user has just said that's all.
fn holds_for_reply(event: &Value) -> bool {
    let waiting_on_work = |key: &str| event[key].as_array().is_some_and(|a| !a.is_empty());
    // Only where going to the session can end the wait: when the hook can tell which app it runs in (not over ssh,
    // say). And not a Claude app chat that's the one open there, whose reply shows the moment the user's back.
    let findable = std::env::var("__CFBundleIdentifier").is_ok();
    let open_there = host_chat().is_some_and(|chat| from_app("open_chat").as_deref() == Some(chat.as_str()));
    findable
        && !open_there
        && hands_free()
        && waited_on()
        && attended()
        && event["permission_mode"] != "plan"
        && !waiting_on_work("background_tasks")
        && !waiting_on_work("session_crons")
        && !event["transcript_path"].as_str().and_then(|t| tail(Path::new(t))).is_some_and(|t| said_thats_all(&t))
}

/// Is the last thing in the conversation the user's answer to Claude's question, and did it end things for now:
/// "That's all for now", or the question closed without an answer?
fn said_thats_all(lines: &str) -> bool {
    let mut done = false;
    for line in lines.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if !by_the_user(&entry) {
            continue;
        }
        let items = entry["message"]["content"].as_array().map(Vec::as_slice).unwrap_or_default();
        match entry["type"].as_str() {
            Some("assistant") if items.iter().any(|i| i["type"] == "tool_use") => done = false,
            Some("user") if entry["toolUseResult"]["answers"].is_object() => {
                let answers = entry["toolUseResult"]["answers"].as_object().into_iter().flat_map(|a| a.values());
                done = answers.filter_map(Value::as_str).any(is_done);
            }
            Some("user") if !items.iter().any(|i| i["type"] == "tool_result") => done = false,
            _ => {}
        }
    }
    done
}

/// Does this answer end things for now? "That's all for now" (Claude sometimes writes it with a curly apostrophe), or
/// the question closed without one, which Claude Code records as an answer in brackets.
fn is_done(answer: &str) -> bool {
    let answer = answer.trim();
    answer.replace('\u{2019}', "'") == DONE_OPTION
        || answer.starts_with("[User dismissed")
        || answer == "[No preference]"
}

/// At the end of a turn: send Claude back to ask what's next, if it should have and didn't. Returns why, for Claude.
fn nudge(event: &Value) -> Option<&'static str> {
    let session = event["session_id"].as_str().filter(|s| !s.is_empty())?;
    // Plans are left to Claude Code, which asks about them itself. And a turn that ends to wait on work running in the
    // background, or on a loop's next run, isn't finished.
    let waiting_on_work = |key: &str| event[key].as_array().is_some_and(|a| !a.is_empty());
    if asking() != Some(true)
        || !asking_always()
        || !attended()
        || !waited_on()
        || event["permission_mode"] == "plan"
        || waiting_on_work("background_tasks")
        || waiting_on_work("session_crons")
    {
        return None;
    }
    let turn = read_turn_end(&tail(Path::new(event["transcript_path"].as_str()?))?);
    let file = asking_file(session)?;
    let nudged = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if !needs_nudge(&turn, nudged.and_then(|n| n["nudged"].as_i64())) {
        return None;
    }
    let _ = fs::create_dir_all(file.parent()?);
    fs::write(&file, json!({ "nudged": chrono::Utc::now().timestamp_millis() }).to_string()).ok()?;
    Some(ASK_NUDGE)
}

/// How a turn ended, from its transcript.
#[derive(Debug, Default, PartialEq)]
struct TurnEnd {
    /// The last thing Claude did was ask with AskUserQuestion, and it got an answer. Whatever the answer was, the user
    /// just had their say: they picked "That's all for now", closed the question, or said something Claude only had to
    /// reply to. Asking again straight away would only be in the way.
    asked_last: bool,
    /// When Claude last asked the user something with AskUserQuestion and got an answer, in milliseconds.
    last_ask: Option<i64>,
}

/// Should the turn go on so Claude can ask what's next? Not when the last thing it did was ask. And not when it was
/// already sent back once and hasn't asked since (it can't, or won't), so it isn't sent round and round.
fn needs_nudge(turn: &TurnEnd, nudged: Option<i64>) -> bool {
    !turn.asked_last && nudged.is_none_or(|at| turn.last_ask.is_some_and(|ask| ask > at))
}

/// The end of a transcript, which Claude Code has written up to this point. Only the end: transcripts run to hundreds
/// of megabytes.
fn tail(transcript: &Path) -> Option<String> {
    const TAIL: u64 = 512 * 1024;
    let mut file = File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    Some(String::from_utf8_lossy(&tail).into_owned())
}

fn read_turn_end(lines: &str) -> TurnEnd {
    let mut turn = TurnEnd::default();
    // The last tool Claude used, by its ID, whether it was AskUserQuestion, and when
    let mut last_tool: Option<(String, bool, Option<i64>)> = None;
    for line in lines.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        // A subagent's work isn't the turn's
        if entry["isSidechain"] == true {
            continue;
        }
        let at = entry["timestamp"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp_millis());
        let content = &entry["message"]["content"];
        let tool_results = content.as_array().is_some_and(|c| c.iter().any(|i| i["type"] == "tool_result"));
        // A message from the user starts a new turn, and whatever they said in the last one is behind it
        if entry["type"] == "user" && entry["isMeta"] != true && !tool_results {
            turn.asked_last = false;
            last_tool = None;
            continue;
        }
        for item in content.as_array().into_iter().flatten() {
            match (entry["type"].as_str(), item["type"].as_str()) {
                (Some("assistant"), Some("tool_use")) => {
                    let ask = item["name"] == "AskUserQuestion";
                    last_tool = Some((item["id"].as_str().unwrap_or_default().to_string(), ask, at));
                    turn.asked_last = false;
                }
                (Some("user"), Some("tool_result")) => {
                    // A question only counts as asked once it's answered. One the user turned down ("Chat about
                    // this"), or that couldn't be asked, doesn't.
                    let Some((_, true, asked_at)) =
                        last_tool.as_ref().filter(|(id, ..)| item["tool_use_id"] == id.as_str())
                    else {
                        continue;
                    };
                    if entry["toolUseResult"]["answers"].as_object().is_none() {
                        continue;
                    }
                    turn.last_ask = asked_at.or(turn.last_ask);
                    turn.asked_last = true;
                }
                _ => {}
            }
        }
    }
    turn
}

/// Append one event to the log. Only this user can read it, and each line is one write, so lines from sessions
/// writing at once can't interleave.
pub fn log(event: &Value) {
    let Some(path) = log_path() else { return };
    let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).mode(0o600).open(path) {
        let _ = file.write_all(format!("{event}\n").as_bytes());
    }
}

/// Would Claude Code ask the user about this permission request itself? Only then is there a prompt for Headroom to
/// stand in for. It doesn't ask in auto mode, "don't ask", or with permissions bypassed, where holding the request
/// would only stall the session. A question is never held here, since it's already held before the tool runs (see
/// `holdable`). Whether to go ahead with a plan is left to Claude Code, since it isn't a yes or no.
fn prompts_user(event: &Value) -> bool {
    match event["tool_name"].as_str() {
        Some("AskUserQuestion" | "ExitPlanMode") => false,
        _ => matches!(event["permission_mode"].as_str(), None | Some("default" | "acceptEdits" | "plan")),
    }
}

/// Is this an event Headroom can hold for the user's answer? A permission request that would prompt the user, or a
/// question Claude is about to ask. Questions are held just before the tool runs rather than at the permission step,
/// since that's where Claude Code lets a hook answer one, and it happens in every mode.
fn holdable(event: &Value) -> bool {
    match event["hook_event_name"].as_str() {
        Some("PermissionRequest") => prompts_user(event),
        Some("PreToolUse") => event["tool_name"] == "AskUserQuestion" && questions(&event["tool_input"]).is_some(),
        _ => false,
    }
}

/// Are approvals on, and is Headroom running to show the request? It rewrites approvals.json every few seconds.
fn app_says_hold() -> bool {
    let Some(config) = approvals_path().and_then(|p| fs::read_to_string(p).ok()) else { return false };
    let Ok(config) = serde_json::from_str::<Value>(&config) else { return false };
    let alive = config["at"].as_i64().is_some_and(|at| chrono::Utc::now().timestamp_millis() - at < APP_ALIVE_MS);
    config["on"] == true && alive
}

/// Is the user looking at this session? Then holding the request would only freeze the prompt they're watching for. A
/// terminal counts when it's the app in front. The Claude app only counts when this session's chat is the one open
/// there: a request from another chat is one they haven't seen.
fn watching() -> bool {
    let front = objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|a| a.bundleIdentifier())
        .map(|id| id.to_string());
    looking(front.as_deref(), from_app("open_chat").as_deref())
}

/// The same, while holding a turn open: this process's view of the app in front doesn't change without a run loop, so
/// it goes by the app's, which it writes into approvals.json.
fn looked_at() -> bool {
    looking(from_app("front").as_deref(), from_app("open_chat").as_deref())
}

/// Is `front` the app this session runs in, and in the Claude app, is `open` (empty when none is) its chat? Not knowing
/// which chat is open, it's taken to be this one.
fn looking(front: Option<&str>, open: Option<&str>) -> bool {
    let Ok(app) = std::env::var("__CFBundleIdentifier") else { return false };
    front == Some(app.as_str()) && host_chat().is_none_or(|chat| open.is_none_or(|open| open == chat))
}

/// Something the app says in approvals.json.
fn from_app(key: &str) -> Option<String> {
    let config: Value = serde_json::from_str(&fs::read_to_string(approvals_path()?).ok()?).ok()?;
    config[key].as_str().map(String::from)
}

/// The Claude app's own ID for the chat this session is ("local_…"), when it runs in the Claude app.
fn host_chat() -> Option<String> {
    std::env::var("CLAUDE_CODE_HOST_SESSION_ID").ok().filter(|id| id.starts_with("local_"))
}

/// Wait for the user's answer to a held request, and hand it to Claude Code. Give up without an answer, so Claude Code
/// asks the user itself, when: the hold runs out (plus any time the user added by pausing, up to `MAX_PAUSE`),
/// approvals are turned off, Headroom stops running, Claude Code stops waiting on this hook, or the user hands the
/// request back ("Answer in Terminal") or sends an answer that doesn't fit it.
///
/// The time is kept by the clock on the wall, from `at` (when the request was logged, in milliseconds), the same way
/// the app keeps it, so the two agree even after the Mac sleeps through part of a hold.
fn hold_for_answer(id: &str, at: i64, hold: std::time::Duration, event: &Value) {
    let Some(dir) = answers_dir() else { return };
    let answer_path = dir.join(format!("{id}.json"));
    let more_path = dir.join(format!("{id}.more"));
    // The conversation kept for the popover (see `save_context`) goes with the question
    let _context = Remove(context_path(id));
    let parent = std::os::unix::process::parent_id();
    let give_up = || {
        let _ = fs::remove_file(&more_path);
        log(&json!({
            "hook_event_name": "PermissionFallback",
            "session_id": event["session_id"],
            "request_id": id,
            "at": chrono::Utc::now().timestamp_millis(),
        }));
    };
    // A message sent from the list for while the session worked, which it finished before it could see, is a reply to
    // the finished turn now
    let session = event["session_id"].as_str().filter(|_| event["hook_event_name"] == "Stop");
    let interjected = || {
        let waiting = take_interjections(session?)?;
        log(&json!({
            "hook_event_name": "HeadroomInterjected",
            "session_id": session,
            "at": chrono::Utc::now().timestamp_millis(),
        }));
        Some(waiting.say(true))
    };
    let mut polls = 0u32;
    loop {
        let answer = match fs::read_to_string(&answer_path) {
            Ok(text) => {
                let _ = fs::remove_file(&answer_path);
                Some(text)
            }
            Err(_) => None,
        };
        // Taken as it is: it's already said as a reply
        if let Some(said) = answer.is_none().then(interjected).flatten() {
            let _ = fs::remove_file(&more_path);
            println!("{}", json!({ "decision": "block", "reason": with_notes(&said) }));
            fresh_message(event["session_id"].as_str().unwrap_or_default());
            return;
        }
        if let Some(text) = answer {
            let _ = fs::remove_file(&more_path);
            match reply(text.trim(), event) {
                Some(output) => {
                    println!("{output}");
                    // A reply is a new message, so being sent back to ask what's next starts over
                    fresh_message(event["session_id"].as_str().unwrap_or_default());
                }
                None => give_up(),
            }
            return;
        }
        let more = fs::read_to_string(&more_path).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
        let more = more.min(MAX_PAUSE.as_millis() as u64);
        let out_of_time = chrono::Utc::now().timestamp_millis() >= at + (hold.as_millis() + more as u128) as i64;
        // Claude Code gone (the session was interrupted or closed) leaves this hook with a new parent
        let orphaned = std::os::unix::process::parent_id() != parent;
        // approvals.json is only rechecked about once a second; it's rewritten every few
        let app_gone = polls % 7 == 0 && !app_says_hold();
        // A turn held for a reply lets go once the user's looking at the chat, to type in it, or hands-free goes off
        let over = polls % 7 == 0 && event["hook_event_name"] == "Stop" && (!hands_free() || looked_at());
        if out_of_time || orphaned || app_gone || over {
            give_up();
            return;
        }
        polls = polls.wrapping_add(1);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

/// Removes a file when it goes out of scope, however the function it's in returns.
struct Remove(Option<PathBuf>);

impl Drop for Remove {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

/// A message from the user that keeps a turn going, with the notes a message typed in the chat would have carried.
fn with_notes(message: &str) -> String {
    let limits = limits_path().and_then(|p| fs::read_to_string(p).ok());
    let ask = match hands_free() {
        true => Some(format!("{ASK_NOTE} {HANDS_FREE_NOTE}")),
        false => (asking() == Some(true)).then(|| ASK_NOTE.to_string()),
    };
    let notes = [ask, limits.and_then(|l| near_limit_note(&l)).map(|(note, _)| note)];
    std::iter::once(message.to_string()).chain(notes.into_iter().flatten()).collect::<Vec<_>>().join("\n\n")
}

/// The hook's output for the user's answer, in the shape each event takes: a decision for a permission request, or for
/// questions, their input with the answers added, which Claude Code then shows Claude as the user's reply. The answers
/// come as "answers:" and a list with one for each question, in order ("answer:" and the answer, for just one).
fn reply(answer: &str, event: &Value) -> Option<Value> {
    // A reply from the popover to a finished turn: it goes on, with the reply as what the user said
    if event["hook_event_name"] == "Stop" {
        let text = answer.strip_prefix("reply:")?.trim();
        if text.is_empty() {
            return None;
        }
        let reason = with_notes(&format!("The user replied from Headroom:\n\n{text}"));
        return Some(json!({ "decision": "block", "reason": reason }));
    }
    if event["hook_event_name"] == "PreToolUse" {
        let picked: Vec<String> = match answer.strip_prefix("answers:") {
            Some(list) => serde_json::from_str(list).ok()?,
            None => vec![answer.strip_prefix("answer:")?.to_string()],
        };
        let input = &event["tool_input"];
        let asked = input["questions"].as_array()?;
        if picked.len() != asked.len() {
            return None;
        }
        let mut answers = Map::new();
        for (question, picked) in asked.iter().zip(picked) {
            answers.insert(question["question"].as_str()?.to_string(), picked.into());
        }
        let mut updated = input.clone();
        updated["answers"] = Value::Object(answers);
        return Some(json!({
            "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "allow", "updatedInput": updated }
        }));
    }
    let decision = decision(answer, event)?;
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }))
}

/// What to tell Claude Code for the user's answer to a permission request: "allow", "session" (allow, and don't ask
/// again this session), or "deny". "Session" passes on the rules Claude Code suggested, but only for this session: a
/// suggestion that would be saved to the user's settings files is kept to the session instead. Anything else, like
/// "terminal", gets no decision, and Claude Code asks the user itself.
fn decision(answer: &str, event: &Value) -> Option<Value> {
    let suggestions = &event["permission_suggestions"];
    match answer {
        "allow" => Some(json!({ "behavior": "allow" })),
        "deny" => Some(json!({ "behavior": "deny", "message": "Denied from Headroom." })),
        "session" => {
            let rules: Vec<Value> = suggestions
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|rule| {
                            let mut rule = rule.clone();
                            rule["destination"] = "session".into();
                            rule
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(json!({ "behavior": "allow", "updatedPermissions": rules }))
        }
        _ => None,
    }
}

/// What the hooks need to know from the app: whether approvals are on and how long to hold a request, whether to have
/// Claude ask what's next, and which chat is open in the Claude app. Written every few seconds while Headroom runs; the
/// time in it is how the hooks know it's still running.
pub struct ForHooks {
    pub on: bool,
    pub hold: u64,
    pub ask: bool,
    /// Claude always asks what's next, rather than only when there's something to decide
    pub always: bool,
    /// Hands-free: a held question comes with what Claude said that turn, and what the user said last.
    pub hands_free: bool,
    pub open_chat: Option<String>,
    /// The app in front, for a turn held open for a reply to let go when the user goes to it (see `looked_at`).
    pub front: Option<String>,
}

pub fn write_approvals(app: &ForHooks) {
    let Some(path) = approvals_path() else { return };
    let at = chrono::Utc::now().timestamp_millis();
    let out = json!({
        "on": app.on,
        "hold": app.hold,
        "ask": app.ask,
        "always": app.always,
        "hands_free": app.hands_free,
        "open_chat": app.open_chat,
        "front": app.front,
        "at": at,
    });
    let _ = write_whole(&path, &out.to_string());
}

/// Write a file the hooks read while it may be changing: under another name first, then renamed over it, so a reader
/// always sees all of it, never an empty or half-written file.
fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

/// The conversation kept with a held question in hands-free mode: what the user last said, and what Claude has said
/// since. Read by the app when the request comes in.
pub fn context(id: &str) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(context_path(&id.replace(['/', '.'], ""))?).ok()?).ok()
}

/// How much time the user has added to each held request by pausing it, in milliseconds, by request ID. The hooks
/// read the same files, so this survives Headroom restarting.
pub fn extra_time() -> std::collections::HashMap<String, i64> {
    let Some(entries) = answers_dir().and_then(|d| fs::read_dir(d).ok()) else { return Default::default() };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".more")?.to_string();
            let ms = fs::read_to_string(entry.path()).ok()?.trim().parse().ok()?;
            Some((id, ms))
        })
        .collect()
}

/// Leave the user's answer to a request for the hook that's holding it. It's written under another name and renamed,
/// so the hook never reads half of it.
pub fn answer(id: &str, choice: &str) -> Result<(), String> {
    let dir = answers_dir().ok_or("Can't find Headroom's folder")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let name = id.replace(['/', '.'], "");
    let temp = dir.join(format!(".{name}.tmp"));
    fs::write(&temp, choice)
        .and_then(|_| fs::rename(&temp, dir.join(format!("{name}.json"))))
        .map_err(|e| e.to_string())
}

/// Give a held request more time: the total milliseconds the user has paused it for.
pub fn extend(id: &str, ms: u64) -> Result<(), String> {
    let dir = answers_dir().ok_or("Can't find Headroom's folder")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    write_whole(&dir.join(format!("{}.more", id.replace(['/', '.'], ""))), &ms.to_string()).map_err(|e| e.to_string())
}

/// Leave a message for a working session, from the list, for its hooks to hand Claude (see `take_interjections`). Each
/// message is a file of its own, written under another name and renamed into place, so a hook taking the messages never
/// reads half of one, and one sent while a hook is taking them waits for the next.
pub fn interject(session: &str, text: &str) -> Result<(), String> {
    queue(session, text, "txt")
}

/// Leave a report from one of a team's managers for the lead's chat (see team.rs), to go in the same way as a message
/// from the user, but said as what it is.
pub fn report(session: &str, text: &str) -> Result<(), String> {
    queue(session, text, "report.txt")
}

fn queue(session: &str, text: &str, kind: &str) -> Result<(), String> {
    let dir = interject_dir(session).ok_or("Can't find Headroom's folder")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for dir in [dir.parent(), Some(dir.as_path())].into_iter().flatten() {
        let _ = fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    }
    static SENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Named so they sort in the order they were sent
    let name = format!("{}-{n:06}", chrono::Utc::now().timestamp_millis());
    let temp = dir.join(format!(".{name}.tmp"));
    let mut file = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&temp);
    file.as_mut().map_err(|e| e.to_string())?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&temp, dir.join(format!("{name}.{kind}"))).map_err(|e| e.to_string())
}

/// Messages the user sent this session from the list while it worked, for Claude to see now: after a tool call, at the
/// end of the turn, or failing both, with the user's next message. Only when Claude Code waits for this hook, since
/// otherwise nothing it prints reaches Claude, and not after a subagent's tool call, which may reach only the
/// subagent.
fn interjections(name: &str, event: &Value) -> Option<Waiting> {
    let session = event["session_id"].as_str().filter(|s| !s.is_empty())?;
    let subagent = event["agent_id"].as_str().is_some_and(|id| !id.is_empty());
    let delivers = matches!(name, "PostToolUse" | "PostToolUseFailure" | "Stop" | "UserPromptSubmit");
    (delivers && waited_on() && (attended() || team().is_some()) && !subagent)
        .then(|| take_interjections(session))
        .flatten()
}

/// Is a message from the list waiting for this session to take it?
pub fn queued(session: &str) -> bool {
    let Some(entries) = interject_dir(session).and_then(|d| fs::read_dir(d).ok()) else { return false };
    entries.flatten().any(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".txt") && !n.starts_with('.')))
}

/// Take the messages waiting for a session, oldest first, as one. Each is renamed before it's read, so two hooks
/// running at once can't both hand the same one to Claude.
fn take_interjections(session: &str) -> Option<Waiting> {
    let dir = interject_dir(session)?;
    let mut names: Vec<String> = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".txt") && !name.starts_with('.'))
        .collect();
    names.sort();
    let mut waiting = Waiting::default();
    for name in names {
        let taken = dir.join(format!(".{name}.{}", std::process::id()));
        if fs::rename(dir.join(&name), &taken).is_err() {
            continue;
        }
        let text = fs::read_to_string(&taken).unwrap_or_default().trim().to_string();
        match (text.is_empty(), name.ends_with(".report.txt")) {
            (true, _) => {}
            (false, true) => waiting.reports.push(text),
            (false, false) => waiting.messages.push(text),
        }
        let _ = fs::remove_file(&taken);
    }
    (!waiting.messages.is_empty() || !waiting.reports.is_empty()).then_some(waiting)
}

/// What was waiting for a session: messages from the user, and reports from a team's managers.
#[derive(Default)]
struct Waiting {
    messages: Vec<String>,
    reports: Vec<String>,
}

impl Waiting {
    /// All of it, for Claude: the user's messages as sent while it worked, or as the reply to its finished turn, and
    /// each report as coming from the team.
    fn say(&self, as_reply: bool) -> String {
        let mut parts = vec![];
        if !self.messages.is_empty() {
            let from = if as_reply { "The user replied from Headroom:" } else { INTERJECTED };
            parts.push(format!("{from}\n\n{}", self.messages.join("\n\n")));
        }
        for report in &self.reports {
            parts.push(format!("{TEAM_REPORT}\n\n{report}"));
        }
        parts.join("\n\n")
    }
}

/// Has this session not had this note yet? Marks it as had, so it's only true once. Creating the file is the check,
/// so two tool calls finishing at the same moment can't both get it.
fn first_time(key: &str, session: &str) -> bool {
    let Some(dir) = sent_dir() else { return false };
    let _ = fs::create_dir_all(&dir);
    let name = format!("{:x}-{}", short_hash(key), session.replace(['/', '.'], ""));
    OpenOptions::new().write(true).create_new(true).open(dir.join(name)).is_ok()
}

fn short_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The note for Claude when any limit is past the threshold picked in Settings, from what `write_limits` saved, and
/// a key that stays the same for as long as it's the same note (the same limits past the threshold, in the same
/// windows), for telling whether a session already had it.
fn near_limit_note(limits: &str) -> Option<(String, String)> {
    let limits: Value = serde_json::from_str(limits).ok()?;
    let threshold = limits["near"].as_f64()?;
    let now = chrono::Utc::now().timestamp_millis();
    if now - limits["at"].as_i64()? > LIMITS_FRESH_MS {
        return None;
    }
    let past: Vec<&Value> =
        limits["limits"].as_array()?.iter().filter(|l| l["pct"].as_f64().is_some_and(|p| p >= threshold)).collect();
    let key = past.iter().map(|l| format!("{}@{}", l["label"], l["resets_at"])).collect::<Vec<_>>().join(",");
    let key = format!("{threshold}:{key}");
    let near: Vec<String> = past
        .iter()
        .map(|l| {
            let pct = l["pct"].as_f64().unwrap_or_default();
            let label = l["label"].as_str().unwrap_or("usage");
            match (l["resets"].as_str(), l["resets_at"].as_i64()) {
                (Some(when), Some(at)) => {
                    format!("{pct:.0}% of their {label} Claude limit, which resets at {when} ({})", time_left(at - now))
                }
                _ => format!("{pct:.0}% of their {label} Claude limit"),
            }
        })
        .collect();
    if near.is_empty() {
        return None;
    }
    let advice = limits["message"].as_str().map(str::trim).filter(|m| !m.is_empty()).unwrap_or(DEFAULT_ADVICE);
    Some((format!("Headroom: the user is at {}. {advice}", near.join(", and ")), key))
}

/// How long until a reset, for the note: "in 29 minutes", "in 2 hours", "in 3 days".
fn time_left(ms: i64) -> String {
    let mins = (ms / 60_000).max(0);
    match mins {
        0 => "any moment now".into(),
        1 => "in 1 minute".into(),
        2..=89 => format!("in {mins} minutes"),
        90..=2159 => format!("in {} hours", (mins + 30) / 60),
        _ => format!("in {} days", (mins + 720) / 1440),
    }
}

/// How the near-limit note is set up in Settings.
pub struct NoteSettings {
    /// The threshold, or `None` when the note is off.
    pub near: Option<u32>,
    /// Also send it after a session's next tool call, not only with the user's next message.
    pub right_away: bool,
    /// What to ask of Claude, in place of `DEFAULT_ADVICE`. Empty for the default.
    pub message: String,
}

/// Save the latest usage for the hooks, along with how the note is set up. The hooks run as their own processes, so
/// they can't ask the app; this file is how they find out.
pub fn write_limits(limits: Vec<Value>, note: &NoteSettings) {
    let Some(path) = limits_path() else { return };
    let out = json!({
        "at": chrono::Utc::now().timestamp_millis(),
        "near": note.near,
        "right_away": note.right_away,
        "message": note.message,
        "limits": limits,
    });
    let _ = fs::write(path, out.to_string());
    forget_old_notes();
}

/// Clear out the record of who got which note once it's more than a week old, which is past any limit's window, and
/// of sessions asked to ask what's next that haven't had a message in that long either. Also images attached to answers
/// once they're a week old, and a held question's conversation that outlived its hook (a hook that was killed never
/// gets to take it away).
fn forget_old_notes() {
    let hour = std::time::Duration::from_secs(60 * 60);
    let old = |entry: &fs::DirEntry, age: std::time::Duration| {
        entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a > age))
    };
    for entry in answers_dir().and_then(|d| fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().ends_with(".context.json") && old(&entry, hour) {
            let _ = fs::remove_file(entry.path());
        }
    }
    let attachments = log_path().map(|p| p.with_file_name("attachments"));
    for entry in attachments.and_then(|d| fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        if old(&entry, 7 * 24 * hour) {
            let _ = fs::remove_file(entry.path());
        }
    }
    // Messages from the list that no hook took in half a day, from a session that went without saying it ended (a
    // subagent can keep a session at work for a long while, and its tool calls don't take them)
    let interject = log_path().map(|p| p.with_file_name("interject"));
    for dir in interject.and_then(|d| fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        // A session's folder that's had nothing come or go in that time, and is empty, goes too
        if old(&dir, 12 * hour) {
            let _ = fs::remove_dir(dir.path());
        }
        for entry in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            if old(&entry, 12 * hour) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let dirs = [sent_dir(), asking_dir()];
    let entries = dirs.into_iter().flatten().filter_map(|d| fs::read_dir(d).ok()).flatten();
    let week = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age > week));
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The fields of an event that Headroom uses. Everything else, like the prompts you type and what tools send back,
/// never leaves Claude Code.
fn keep(event: &Value) -> Value {
    let mut out = Map::new();
    for key in ["session_id", "hook_event_name", "cwd", "source", "tool_name", "error", "transcript_path"] {
        if let Some(v) = event.get(key).filter(|v| v.is_string()) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(text) = event["last_assistant_message"].as_str() {
        out.insert("last_assistant_message".into(), clip(text).into());
        // The documents, pictures, and links it names, to look at from the panel (see made.rs). Only here is the
        // whole reply to hand, and the session's folder to find them in.
        let cwd = Path::new(event["cwd"].as_str().unwrap_or_default());
        let named = crate::made::named(text, cwd);
        if !named.is_empty() {
            out.insert("named".into(), named.into());
        }
    }
    // Files Claude sent the user, as the Claude app shows them
    if event["tool_name"] == "SendUserFile" {
        if let Some(files) = event["tool_input"]["files"].as_array() {
            out.insert("files".into(), files.iter().filter(|f| f.is_string()).cloned().collect::<Vec<_>>().into());
        }
    }
    if let Some(detail) = tool_detail(&event["tool_input"]) {
        out.insert("detail".into(), detail.into());
    }
    if let Some(questions) = questions(&event["tool_input"]) {
        out.insert("questions".into(), questions);
    }
    // The chat's title, as the Claude app shows it in its list of chats. It can change during a session, so we look
    // again at the start and end of each turn.
    let name = event["hook_event_name"].as_str().unwrap_or_default();
    if matches!(name, "SessionStart" | "UserPromptSubmit" | "Stop") {
        let title = event["session_title"]
            .as_str()
            .map(String::from)
            .or_else(|| event["transcript_path"].as_str().and_then(|path| chat_title(Path::new(path))));
        if let Some(title) = title {
            out.insert("title".into(), clip(&title).into());
        }
    }

    // macOS tells a process which app started it. For a session, that's the terminal or the Claude app it runs in,
    // which lets Headroom skip a notification when you're already looking at that app. In the Claude app, it's also
    // told which chat, which Headroom can open straight to.
    if let Ok(app) = std::env::var("__CFBundleIdentifier") {
        out.insert("app".into(), app.into());
    }
    if let Some(chat) = host_chat() {
        out.insert("chat".into(), chat.into());
    }
    // The process that ran the hook: the session's Claude Code, or a shell it started for it. The app looks there for
    // the command Claude's running, to stop it when the user wants a message to go in right away.
    out.insert("ppid".into(), std::os::unix::process::parent_id().into());
    // Nobody to send it anything from the list
    if !attended() {
        out.insert("unattended".into(), true.into());
    }
    if let Some(team) = team() {
        out.insert("team".into(), team.into());
        // Which of its workers did this, and which one it handed work to and what came back, for the planner to
        // follow each of them (a worker's `agent_type` is its name in the team, as the planner gave it)
        if event["agent_id"].as_str().is_some_and(|id| !id.is_empty()) {
            if let Some(worker) = event["agent_type"].as_str() {
                out.insert("worker".into(), worker.into());
            }
        } else if matches!(event["tool_name"].as_str(), Some("Agent" | "Task")) {
            // Handed to one of its workers, or, with none named, a general-purpose subagent (like a worker the user
            // added while it worked)
            let worker = event["tool_input"]["subagent_type"].as_str().unwrap_or("general-purpose");
            out.insert("handed".into(), worker.into());
            // The model it was handed to, when the manager picked one: how a change to a worker's model mid-run shows
            if let Some(model) = event["tool_input"]["model"].as_str() {
                out.insert("model".into(), model.into());
            }
            if let Some(said) = subagent_result(&event["tool_response"]) {
                out.insert("said".into(), clip(&said).into());
            }
        }
    }
    out.insert("at".into(), chrono::Utc::now().timestamp_millis().into());
    Value::Object(out)
}

/// The chat's title from its transcript. Claude Code writes the title into the transcript again every turn, so the
/// latest one is near the end, and we only read the end: transcripts run to megabytes.
fn chat_title(transcript: &Path) -> Option<String> {
    const TAIL: u64 = 256 * 1024;
    let mut file = File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .filter(|line| line.contains("\"custom-title\""))
        .find_map(|line| serde_json::from_str::<Value>(line).ok()?["customTitle"].as_str().map(String::from))
        .filter(|title| !title.trim().is_empty())
}

/// The questions Claude is asking, each with its options, for Headroom to show and answer. Claude Code asks up to four
/// at once; the popover steps through them.
fn questions(input: &Value) -> Option<Value> {
    let asked = input["questions"].as_array().filter(|q| !q.is_empty())?;
    let list = asked.iter().map(|q| {
        let options: Vec<Value> = q["options"]
            .as_array()?
            .iter()
            .filter_map(|o| {
                // The label as it is, not clipped: the one picked goes back to Claude Code as the answer
                let label = o["label"].as_str()?;
                Some(json!({ "label": label, "description": o["description"].as_str().map(clip) }))
            })
            .collect();
        Some(json!({
            "text": clip(q["question"].as_str()?),
            "multiple": q["multiSelect"].as_bool().unwrap_or(false),
            "options": options,
        }))
    });
    list.collect::<Option<Vec<Value>>>().map(Value::Array)
}

/// The one thing about a tool call worth showing: the command it runs, the file it touches, and so on.
fn tool_detail(input: &Value) -> Option<String> {
    if let Some(question) = input["questions"][0]["question"].as_str() {
        return Some(clip(question));
    }
    ["command", "file_path", "notebook_path", "path", "url", "query", "pattern", "description"]
        .iter()
        .find_map(|key| input[*key].as_str())
        .map(clip)
}

/// What a subagent said back when it was done: Claude Code's tool result for it, as text or as content blocks.
fn subagent_result(response: &Value) -> Option<String> {
    if let Some(text) = response.as_str() {
        return Some(text.to_string());
    }
    let blocks = response["content"].as_array()?;
    let text: Vec<&str> = blocks.iter().filter_map(|b| b["text"].as_str()).collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// For a held request, the whole of what it would do, since that's what the user approves. For the built-in tools
/// that's one field (a command, a file, a URL), shown the way Claude Code shows it. For anything else, an MCP tool say,
/// every field could matter, so it's the whole input. Kept to a size that still fits a line in the log.
fn full_detail(tool: &str, input: &Value) -> Option<String> {
    const MAX: usize = 8000;
    let field = match tool {
        "Bash" => Some("command"),
        "Read" | "Edit" | "MultiEdit" | "Write" => Some("file_path"),
        "NotebookEdit" => Some("notebook_path"),
        "WebFetch" => Some("url"),
        "WebSearch" => Some("query"),
        "Glob" | "Grep" => Some("pattern"),
        _ => None,
    };
    let text = match field.and_then(|f| input[f].as_str()) {
        Some(text) => text.to_string(),
        None => input.as_object().filter(|o| !o.is_empty()).map(|_| input.to_string())?,
    };
    Some(match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}… (cut off: too long to show in full)", &text[..cut]),
        None => text,
    })
}

fn clip(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/***************************
 * S E T T I N G S . J S O N
 **************************/

/// Add our hooks to ~/.claude/settings.json, pointing at `exe`. Any of ours that are already there are replaced, so
/// this also fixes them up after Headroom moves (say, from a build folder into Applications). With the near-limit note
/// on, Claude Code waits for the prompt hook so it can add the note, and with "Right away", for the tool hook too. With
/// approvals on (`hold` is how long to hold a request), it waits for the permission hook, a little longer than the
/// hold, so the hook always gives up on its own first, and for the prompt and end-of-turn hooks, for "Have Claude ask
/// what's next". With `hands_free` too, it waits for the tool hook, which hands Claude messages sent from the list.
pub fn install(exe: &Path, note: &NoteSettings, hold: Option<u64>, hands_free: bool) -> Result<(), String> {
    let exe = exe.display().to_string().replace('"', "\\\"");
    let command = format!("\"{exe}\" {HOOK_ARG}");
    edit_settings(|settings| {
        remove_ours(settings);
        let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
        let hooks = hooks.as_object_mut().ok_or("\"hooks\" in ~/.claude/settings.json isn't an object")?;
        for (event, matcher) in EVENTS {
            let wait = (note.near.is_some()
                && (event == "UserPromptSubmit" || (event == "PostToolUse" && note.right_away)))
                || (hold.is_some() && matches!(event, "UserPromptSubmit" | "Stop"))
                // Hands-free, a message sent from the list to a working session goes in after its next tool call
                // (or failed, as a command does when the user stops it to send one right away)
                || (hold.is_some() && hands_free && matches!(event, "PostToolUse" | "PostToolUseFailure"));
            let hook = if let (Some(hold), "PermissionRequest" | "PreToolUse" | "Stop") = (hold, event) {
                // The end of a turn can wait for a reply, in hands-free, and is waited on for asking what's next
                let waited = if event == "Stop" { format!(" {WAIT_ARG}") } else { String::new() };
                let command = format!("\"{exe}\" {HOLD_ARG}{hold}{waited} {HOOK_ARG}");
                json!([{ "type": "command", "command": command, "timeout": hold + MAX_PAUSE.as_secs() + 10 }])
            } else if wait {
                json!([{ "type": "command", "command": format!("\"{exe}\" {WAIT_ARG} {HOOK_ARG}"), "timeout": 5 }])
            } else {
                json!([{ "type": "command", "command": command, "async": true }])
            };
            let group = match matcher {
                Some(m) => json!({ "matcher": m, "hooks": hook }),
                None => json!({ "hooks": hook }),
            };
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            groups
                .as_array_mut()
                .ok_or(format!("\"{event}\" hooks in ~/.claude/settings.json aren't a list"))?
                .push(group);
        }
        Ok(())
    })
}

/// Take our hooks back out of ~/.claude/settings.json, leaving everything else as it was.
pub fn uninstall() -> Result<(), String> {
    edit_settings(|settings| {
        remove_ours(settings);
        Ok(())
    })
}

/// Remove every hook of ours, then any groups, events, and the "hooks" key itself that we left empty. Anything the
/// user or another tool put there stays exactly as it was.
fn remove_ours(settings: &mut Map<String, Value>) {
    let ours = |hook: &Value| hook["command"].as_str().is_some_and(|c| c.contains(HOOK_ARG));
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        for group in groups.iter_mut() {
            if let Some(list) = group["hooks"].as_array_mut() {
                list.retain(|hook| !ours(hook));
            }
        }
        groups.retain(|group| !group["hooks"].as_array().is_some_and(|list| list.is_empty()));
    }
    hooks.retain(|_, groups| !groups.as_array().is_some_and(|g| g.is_empty()));
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}

/// Read settings.json, change it, and write it back. If the file can't be read as JSON, we leave it alone rather than
/// risk losing someone's settings. The new file is written next to the old one and renamed over it, so a crash
/// halfway can't leave it cut off.
fn edit_settings(change: impl FnOnce(&mut Map<String, Value>) -> Result<(), String>) -> Result<(), String> {
    let path = settings_path().ok_or("Can't find your home folder")?;
    // If it's a link (say, into a dotfiles repo), write through it rather than replacing the link with a file
    let path = fs::canonicalize(&path).unwrap_or(path);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => return Err(format!("Couldn't read ~/.claude/settings.json: {e}")),
    };
    let mut settings = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => map,
        _ => return Err("~/.claude/settings.json isn't valid JSON, so Headroom left it alone".into()),
    };
    change(&mut settings)?;

    let out = serde_json::to_string_pretty(&Value::Object(settings)).map_err(|e| e.to_string())? + "\n";
    let temp = path.with_extension("json.headroom");
    fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(|e| e.to_string())?;
    fs::write(&temp, out)
        .and_then(|_| fs::rename(&temp, &path))
        .map_err(|e| format!("Couldn't save ~/.claude/settings.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_ours_leaves_everything_else() {
        let before = json!({
            "theme": "dark",
            "hooks": {
                "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo hi" }] }],
                "Stop": [
                    { "hooks": [{ "type": "command", "command": "\"/Applications/Headroom.app/Contents/MacOS/headroom\" --headroom-hook" }] },
                    { "matcher": "", "hooks": [{ "type": "command", "command": "say done" }] }
                ],
                "SessionEnd": [{ "hooks": [{ "type": "command", "command": "\"/x/headroom\" --headroom-hook" }] }],
                "PermissionRequest": [{ "hooks": [{ "type": "command", "command": "\"/x/headroom\" --hold=120 --headroom-hook", "timeout": 250 }] }]
            }
        });
        let mut settings = before.as_object().unwrap().clone();
        remove_ours(&mut settings);
        assert_eq!(
            Value::Object(settings),
            json!({
                "theme": "dark",
                "hooks": {
                    "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo hi" }] }],
                    "Stop": [{ "matcher": "", "hooks": [{ "type": "command", "command": "say done" }] }]
                }
            })
        );
    }

    #[test]
    fn removing_ours_drops_the_hooks_key_we_emptied() {
        let mut settings = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "\"/x/headroom\" --headroom-hook" }] }] }
        })
        .as_object()
        .unwrap()
        .clone();
        remove_ours(&mut settings);
        assert!(settings.is_empty());
    }

    #[test]
    fn the_title_is_the_last_one_in_the_transcript() {
        let path = std::env::temp_dir().join("headroom-title-test.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"type\":\"custom-title\",\"customTitle\":\"First idea\"}\n",
                "{\"type\":\"user\",\"message\":\"hi\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"Fix the login bug\"}\n",
                "{\"type\":\"assistant\",\"message\":\"done\"}\n",
            ),
        )
        .unwrap();
        assert_eq!(chat_title(&path).as_deref(), Some("Fix the login bug"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn the_note_only_mentions_limits_past_the_threshold() {
        let now = chrono::Utc::now().timestamp_millis();
        let limits = json!({
            "at": now,
            "near": 85,
            "limits": [
                { "label": "5-hour", "pct": 91.0, "resets": "3:10 PM", "resets_at": now + 29 * 60_000 + 30_000 },
                { "label": "weekly", "pct": 60.0, "resets": "Tue 9:00 AM", "resets_at": now + 4 * 86_400_000 }
            ]
        });
        let (note, _) = near_limit_note(&limits.to_string()).unwrap();
        assert!(note.contains("91% of their 5-hour Claude limit, which resets at 3:10 PM (in 29 minutes)"));
        assert!(note.ends_with(DEFAULT_ADVICE));
        assert!(!note.contains("weekly"));
    }

    #[test]
    fn the_users_own_message_replaces_the_advice() {
        let now = chrono::Utc::now().timestamp_millis();
        let limits = json!({
            "at": now, "near": 80, "message": "Commit what you have and stop.",
            "limits": [{ "label": "weekly", "pct": 85.0 }]
        });
        let (note, _) = near_limit_note(&limits.to_string()).unwrap();
        assert_eq!(note, "Headroom: the user is at 85% of their weekly Claude limit. Commit what you have and stop.");
    }

    #[test]
    fn no_note_when_its_off_or_the_numbers_are_old() {
        let now = chrono::Utc::now().timestamp_millis();
        let limits =
            |at: i64, near: Value| json!({ "at": at, "near": near, "limits": [{ "label": "5-hour", "pct": 99.0 }] });
        assert!(near_limit_note(&limits(now, Value::Null).to_string()).is_none());
        assert!(near_limit_note(&limits(now - 3_600_000, json!(85)).to_string()).is_none());
        assert!(near_limit_note(&limits(now, json!(85)).to_string()).is_some());
    }

    #[test]
    fn requests_are_only_held_where_the_user_would_be_asked() {
        let mode = |m: &str| json!({ "permission_mode": m });
        assert!(prompts_user(&mode("default")));
        assert!(prompts_user(&mode("acceptEdits")));
        assert!(prompts_user(&mode("plan")));
        assert!(!prompts_user(&mode("auto")));
        assert!(!prompts_user(&mode("bypassPermissions")));
        assert!(!prompts_user(&mode("dontAsk")));
        assert!(!prompts_user(&json!({ "permission_mode": "default", "tool_name": "ExitPlanMode" })));
        // Questions are held before the tool runs, in any mode, one at a time or several together
        let one = json!({ "questions": [{ "question": "Which?", "options": [{ "label": "A" }] }] });
        let two =
            json!({ "questions": [{ "question": "Which?", "options": [] }, { "question": "And?", "options": [] }] });
        let none = json!({ "questions": [] });
        let asking = |input: &Value| json!({ "hook_event_name": "PreToolUse", "permission_mode": "auto", "tool_name": "AskUserQuestion", "tool_input": input });
        assert!(holdable(&asking(&one)));
        assert!(holdable(&asking(&two)));
        assert!(!holdable(&asking(&none)));
        assert!(!holdable(
            &json!({ "hook_event_name": "PermissionRequest", "tool_name": "AskUserQuestion", "tool_input": one })
        ));
    }

    #[test]
    fn answers_become_decisions() {
        let event = json!({
            "permission_suggestions": [{ "type": "addRules", "rules": [{ "toolName": "Bash" }], "destination": "localSettings" }]
        });
        assert_eq!(decision("allow", &event), Some(json!({ "behavior": "allow" })));
        assert_eq!(decision("deny", &event).unwrap()["behavior"], "deny");
        let session = decision("session", &event).unwrap();
        assert_eq!(session["behavior"], "allow");
        // A rule Claude Code would have saved to the user's settings stays with the session instead
        assert_eq!(session["updatedPermissions"][0]["destination"], "session");
        assert_eq!(decision("terminal", &event), None);
    }

    #[test]
    fn an_answer_to_a_question_goes_back_with_the_question() {
        let event = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "AskUserQuestion",
            "tool_input": { "questions": [{ "question": "Which photos?", "options": [{ "label": "Larger" }] }] }
        });
        let out = reply("answer:Larger", &event).unwrap()["hookSpecificOutput"].clone();
        assert_eq!(out["permissionDecision"], "allow");
        assert_eq!(out["updatedInput"]["answers"]["Which photos?"], "Larger");
        assert_eq!(out["updatedInput"]["questions"][0]["question"], "Which photos?");
        assert!(reply("terminal", &event).is_none());

        // Several at once: each answer goes with its own question, and a list that doesn't fit them is no answer
        let event = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "AskUserQuestion",
            "tool_input": { "questions": [{ "question": "Paper?", "options": [] }, { "question": "Size?", "options": [] }] }
        });
        let out = reply(r#"answers:["Cream","6 x 9, Big"]"#, &event).unwrap()["hookSpecificOutput"].clone();
        assert_eq!(out["updatedInput"]["answers"], json!({ "Paper?": "Cream", "Size?": "6 x 9, Big" }));
        assert!(reply(r#"answers:["Cream"]"#, &event).is_none());
    }

    #[test]
    fn a_reply_from_the_popover_carries_on_the_turn() {
        let stop = json!({ "hook_event_name": "Stop" });
        let out = reply("reply:Do the archive too", &stop).unwrap();
        assert_eq!(out["decision"], "block");
        // Whatever notes follow it depend on the Mac's own settings
        assert!(out["reason"].as_str().unwrap().starts_with("The user replied from Headroom:\n\nDo the archive too"));
        // Anything else lets the turn end as usual
        assert_eq!(reply("terminal", &stop), None);
        assert_eq!(reply("reply:  ", &stop), None);
    }

    #[test]
    fn a_turn_the_user_ended_isnt_held_open() {
        let answered = |picked: &str| {
            json!({ "type": "user", "toolUseResult": { "answers": { "What next?": picked } },
                "message": { "content": [{ "type": "tool_result", "tool_use_id": "q" }] } })
            .to_string()
        };
        assert!(said_thats_all(&answered(DONE_OPTION)));
        assert!(said_thats_all(&answered("[User dismissed — do not proceed, wait for next instruction]")));
        assert!(!said_thats_all(&answered("Build the docs page")));
        // More work after it: the answer wasn't the last word
        let tool = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "b", "name": "Bash" }] } });
        assert!(!said_thats_all(&[answered(DONE_OPTION), tool.to_string()].join("\n")));
    }

    #[test]
    fn a_held_request_shows_all_of_what_it_would_do() {
        let long = "x".repeat(500);
        assert_eq!(full_detail("Bash", &json!({ "command": long })).unwrap().len(), 500);
        // Every field of a tool Headroom doesn't know, even one that happens to have a "query"
        let mcp = json!({ "query": "issues", "delete": true });
        assert_eq!(full_detail("mcp__linear__run", &mcp).unwrap(), r#"{"query":"issues","delete":true}"#);
        assert!(full_detail("mcp__x", &json!({})).is_none());
        let huge = json!({ "command": "y".repeat(9000) });
        assert!(full_detail("Bash", &huge).unwrap().ends_with("(cut off: too long to show in full)"));
    }

    #[test]
    fn a_turn_can_end_once_claude_has_asked() {
        let ask = |id: &str, at: &str| {
            json!({ "type": "assistant", "timestamp": at,
                "message": { "content": [{ "type": "tool_use", "id": id, "name": "AskUserQuestion", "input": {} }] } })
            .to_string()
        };
        let answer = |id: &str, picked: &str| {
            json!({ "type": "user", "toolUseResult": { "answers": { "What next?": picked } },
                "message": { "content": [{ "type": "tool_result", "tool_use_id": id }] } })
            .to_string()
        };
        let tool = json!({ "type": "assistant", "timestamp": "2026-09-26T10:05:00Z",
            "message": { "content": [{ "type": "tool_use", "id": "b", "name": "Bash", "input": {} }] } })
        .to_string();
        let asked_at = chrono::DateTime::parse_from_rfc3339("2026-09-26T10:00:00Z").unwrap().timestamp_millis();

        // Answered, whatever the answer, and nothing done since
        for picked in [DONE_OPTION, "Keep testing", "[User dismissed — do not proceed, wait for next instruction]"] {
            let asked = [ask("a", "2026-09-26T10:00:00Z"), answer("a", picked)].join("\n");
            assert_eq!(read_turn_end(&asked), TurnEnd { asked_last: true, last_ask: Some(asked_at) });
        }
        // Asked, but the answer was more work, and Claude did it without asking again
        let more = [ask("a", "2026-09-26T10:00:00Z"), answer("a", "Run the tests"), tool].join("\n");
        assert_eq!(read_turn_end(&more), TurnEnd { asked_last: false, last_ask: Some(asked_at) });
        assert_eq!(read_turn_end(""), TurnEnd::default());
        // Turned down ("Chat about this"): not asked
        let declined =
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "a" }] } });
        let declined = [ask("a", "2026-09-26T10:00:00Z"), declined.to_string()].join("\n");
        assert_eq!(read_turn_end(&declined), TurnEnd::default());
        // The next message starts a new turn
        let prompt = json!({ "type": "user", "message": { "content": "One more thing" } }).to_string();
        let next = [ask("a", "2026-09-26T10:00:00Z"), answer("a", DONE_OPTION), prompt].join("\n");
        assert_eq!(read_turn_end(&next), TurnEnd { asked_last: false, last_ask: Some(asked_at) });
    }

    #[test]
    fn claude_is_only_sent_back_while_it_keeps_asking() {
        let turn = |asked_last, last_ask| TurnEnd { asked_last, last_ask };
        assert!(needs_nudge(&turn(false, None), None));
        assert!(!needs_nudge(&turn(true, Some(5)), None));
        // Sent back at 10, asked at 20, and now the turn is ending again without a question: it's sent back again
        assert!(needs_nudge(&turn(false, Some(20)), Some(10)));
        // Sent back at 10 and hasn't asked since: it's let go
        assert!(!needs_nudge(&turn(false, Some(5)), Some(10)));
        assert!(!needs_nudge(&turn(false, None), Some(10)));
    }

    #[test]
    fn the_notes_ask_for_one_question_ending_in_the_done_option() {
        // One question for what's next, ending with the option that lets the turn end
        for note in [ASK_NOTE, ASK_NUDGE] {
            assert!(note.contains("one question"));
            assert!(note.contains(&format!("\"{DONE_OPTION}\"")));
            assert!(!note.contains("  "));
        }
    }

    #[test]
    fn hands_free_shows_the_last_message_and_what_claude_said_since() {
        let lines = |lines: Vec<Value>| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        let text = |t: &str| json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": t }] } });
        let tool = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t", "name": "Bash" }] } });
        let result =
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t" }] } });
        let turn = lines(vec![
            json!({ "type": "user", "message": { "content": "Fix the login bug" } }),
            text("Looking."),
            json!({ "type": "user", "message": { "content": "Use the new endpoint" } }),
            text("Checking the routes first."),
            tool.clone(),
            result.clone(),
            json!({ "type": "system", "subtype": "stop_hook_summary" }),
            text("**Summary**: it works."),
            json!({ "type": "user", "isSidechain": true, "message": { "content": "A subagent's task" } }),
            json!({ "type": "user", "message": { "content": "<task-notification>\n<task-id>b1</task-id>" } }),
        ]);
        // The summary after the last tool call, not the progress notes before it
        assert_eq!(conversation(&turn), (Some("Use the new endpoint".into()), "**Summary**: it works.".into()));
        // Nothing written since the last tool call: the last thing it did write
        let asked_straight_away =
            lines(vec![json!({ "type": "user", "message": { "content": "Go" } }), text("On it."), tool, result]);
        assert_eq!(conversation(&asked_straight_away), (Some("Go".into()), "On it.".into()));
        // A message sent while it worked, and the stand-in for one that's further back
        let queued = lines(vec![
            json!({ "type": "last-prompt", "lastPrompt": "An older message" }),
            json!({ "type": "attachment", "attachment": { "type": "queued_command", "prompt": "Also add dark mode" } }),
        ]);
        assert_eq!(conversation(&queued).0.as_deref(), Some("Also add dark mode"));
        // Compacting files a summary under the user's name; it isn't their message
        let compacted = lines(vec![
            json!({ "type": "user", "message": { "content": "Keep going" } }),
            json!({ "type": "user", "isCompactSummary": true, "message": { "content": "This session is being continued…" } }),
        ]);
        assert_eq!(conversation(&compacted).0.as_deref(), Some("Keep going"));
        // Sent with an image while Claude worked
        let with_image = lines(vec![json!({ "type": "attachment", "attachment": { "type": "queued_command",
            "prompt": [{ "type": "image" }, { "type": "text", "text": "Like this one" }] } })]);
        assert_eq!(conversation(&with_image).0.as_deref(), Some("Like this one"));
        // An answered question: the summary before it has been read, so it isn't shown again
        let answered_q = lines(vec![
            json!({ "type": "user", "message": { "content": "Go" } }),
            text("Summary one."),
            json!({ "type": "user", "toolUseResult": { "answers": { "Q": "Yes" } },
                "message": { "content": [{ "type": "tool_result", "tool_use_id": "q" }] } }),
        ]);
        assert_eq!(conversation(&answered_q).1, "");
        let far_back = lines(vec![json!({ "type": "last-prompt", "lastPrompt": "An older message" }), text("Done.")]);
        assert_eq!(conversation(&far_back), (Some("An older message".into()), "Done.".into()));
        // Lately: messages, answers, what Claude wrote, and what it did in between
        let ask = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "q", "name": "AskUserQuestion",
            "input": { "questions": [{ "question": "Which layout?" }] } }] } });
        let answered = json!({ "type": "user", "toolUseResult": { "answers": { "Which layout?": "The wide one" } },
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "q" }] } });
        let edit = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "e", "name": "Edit" }] } });
        let lately = lines(vec![
            json!({ "type": "user", "message": { "content": "Redo the settings page" } }),
            json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "b", "name": "Bash" }] } }),
            json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "b2", "name": "Bash" }] } }),
            edit,
            text("Two layouts to pick from."),
            ask,
            answered,
        ]);
        assert_eq!(
            Value::Array(recent(&lately, 10)),
            json!([
                { "kind": "said", "text": "Redo the settings page" },
                { "kind": "did", "text": "Ran 2 commands, edited a file" },
                { "kind": "wrote", "text": "Two layouts to pick from." },
                { "kind": "answered", "about": "Which layout?", "text": "The wide one" },
            ])
        );
        assert_eq!(recent(&lately, 2).len(), 2);
        assert_eq!(clip_to("abcdef", 3, true), "…def");
        assert_eq!(clip_to("abcdef", 3, false), "abc…");
    }

    #[test]
    fn the_log_keeps_only_what_we_use() {
        let event = json!({
            "session_id": "abc",
            "hook_event_name": "UserPromptSubmit",
            "cwd": "/Users/me/Code/site",
            "prompt": "my secret plans",
            "transcript_path": "/Users/me/.claude/projects/x.jsonl"
        });
        let kept = keep(&event);
        assert_eq!(kept["session_id"], "abc");
        assert!(kept.get("prompt").is_none());
        // Where the transcript is, for the planner to read the last message from when asked, but nothing that's in it
        assert_eq!(kept["transcript_path"], "/Users/me/.claude/projects/x.jsonl");
    }
}
