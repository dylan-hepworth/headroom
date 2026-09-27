// How full each Claude Code conversation's context window is, from its transcript, for the "Context is filling up"
// alert.
//
// The tokens in context are what the last reply was sent: its input tokens, cached or not. The window is 1M for the
// models that have one natively, or when the session asked for it with a "[1m]" model, which Claude Code writes into
// the transcript as a model attachment; otherwise 200k. That's the same share of the window the Claude app shows.
//
// Transcripts only grow, so each is read once in full and then from where we left off. One wrinkle: around some
// compactions Claude Code writes thousands of earlier lines into the transcript again, with their original times.
// Replies and compactions whose time goes backwards are those copies, and are skipped.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::Value;

/// Models with a 1M-token window without asking for one.
const NATIVE_1M: [&str; 10] = [
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-opus-5-5",
    "claude-fable-5",
    "claude-fable-5-1",
    "claude-mythos-5",
    "claude-mythos-5-1",
    "claude-mythos-preview",
    "claude-sonnet-5",
];

/// A conversation's context passed the alert's threshold.
pub struct Crossing {
    pub session: String,
    pub pct: u32,
    pub tokens: u64,
    pub window: u64,
}

/// Where one transcript's reading is up to.
#[derive(Default)]
struct Transcript {
    path: Option<PathBuf>,
    offset: u64,
    /// The newest reply or compaction time taken so far. Anything older is a copy.
    latest: String,
    /// The model the session was started with or switched to, as written in the transcript ("claude-opus-5[1m]").
    model_id: Option<String>,
    /// The last reply's tokens in context, and its model, since the last compaction.
    tokens: u64,
    model: String,
    /// The share of the window last time we looked, so the alert goes off when it passes the threshold, not while
    /// it's past it.
    pct: u32,
}

#[derive(Default)]
pub struct Contexts {
    transcripts: HashMap<String, Transcript>,
}

impl Contexts {
    /// Read what's new in each session's transcript. Returns the sessions whose context has just passed `threshold`
    /// percent of its window. Sessions not listed are forgotten.
    pub fn update(&mut self, sessions: &[String], threshold: u32) -> Vec<Crossing> {
        self.transcripts.retain(|id, _| sessions.contains(id));
        let mut crossings = vec![];
        for id in sessions {
            let t = self.transcripts.entry(id.clone()).or_default();
            // The first reading is only where things stand, not news
            let first = t.offset == 0;
            if t.path.is_none() {
                t.path = find_transcript(id);
            }
            let Some(path) = t.path.clone() else { continue };
            let before = t.pct;
            if !t.read(&path) {
                continue;
            }
            let (pct, window) = t.share();
            t.pct = pct;
            if !first && before < threshold && pct >= threshold {
                crossings.push(Crossing { session: id.clone(), pct, tokens: t.tokens, window });
            }
        }
        crossings
    }
}

impl Transcript {
    /// Read the lines added since last time. Returns whether there were any.
    fn read(&mut self, path: &Path) -> bool {
        let Ok(mut file) = File::open(path) else { return false };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            *self = Transcript { path: self.path.take(), ..Default::default() };
        }
        if len == self.offset || file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(n) = reader.read_line(&mut line) else { break };
            // A line that's still being written waits for next time
            if n == 0 || !line.ends_with('\n') {
                break;
            }
            self.offset += n as u64;
            self.take(&line);
        }
        true
    }

    /// Take what counts from one line: the model the session asked for, a compaction, or a reply's tokens.
    fn take(&mut self, line: &str) {
        // Most lines are none of these, and aren't worth parsing
        let model = line.contains("\"identity\"") && line.contains("\"model\"");
        let boundary = line.contains("\"compact_boundary\"");
        let reply = line.contains("\"assistant\"") && line.contains("\"usage\"");
        if !model && !boundary && !reply {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        if v["type"] == "attachment" && v["attachment"]["type"] == "model" {
            if let Some(id) = v["attachment"]["identity"]["modelId"].as_str() {
                self.model_id = Some(id.to_string());
            }
            return;
        }
        let is_boundary = v["type"] == "system" && v["subtype"] == "compact_boundary";
        let is_reply = v["type"] == "assistant" && v["isSidechain"] != true && v["message"]["model"] != "<synthetic>";
        if !is_boundary && !is_reply {
            return;
        }
        let at = v["timestamp"].as_str().unwrap_or_default();
        if at < self.latest.as_str() {
            return;
        }
        self.latest = at.to_string();
        if is_boundary {
            self.tokens = 0;
            return;
        }
        let tokens = context_tokens(&v["message"]["usage"]);
        if tokens > 0 {
            self.tokens = tokens;
            self.model = v["message"]["model"].as_str().unwrap_or_default().to_string();
        }
    }

    /// The share of the window in use, as a whole percent, and the window.
    fn share(&self) -> (u32, u64) {
        let window = window(self.model_id.as_deref(), &self.model, self.tokens);
        ((self.tokens * 100 / window).min(100) as u32, window)
    }
}

/// A session's transcript: ~/.claude/projects/<folder>/<session id>.jsonl, in whichever project folder it's in.
fn find_transcript(session: &str) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("HOME")?).join(".claude/projects");
    let name = format!("{session}.jsonl");
    std::fs::read_dir(root).ok()?.flatten().map(|dir| dir.path().join(&name)).find(|p| p.is_file())
}

/// The tokens a reply was sent, which is what's in context. With an advisor, the reply is several requests, and the
/// last one that was a message is the one that counts.
fn context_tokens(usage: &Value) -> u64 {
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
    let sum = |v: &Value| n(v, "input_tokens") + n(v, "cache_creation_input_tokens") + n(v, "cache_read_input_tokens");
    let top = sum(usage);
    if top == 0 {
        return top;
    }
    let last = usage["iterations"].as_array().and_then(|its| {
        its.iter().rev().find(|i| !matches!(i["type"].as_str(), Some("advisor_message" | "compaction")))
    });
    let Some(last) = last.filter(|i| matches!(i["type"].as_str(), Some("message" | "fallback_message"))) else {
        return top;
    };
    let complete = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .all(|k| last[*k].is_u64());
    match sum(last) {
        s if complete && s > 0 => s,
        _ => top,
    }
}

/// A model's name without what doesn't change its window: "[1m]", a cloud provider's prefix, a version or date suffix.
fn plain_model(model: &str) -> String {
    let mut m = model.to_lowercase().replace("[1m]", "").replace("[2m]", "");
    for prefix in ["us.", "eu.", "apac.", "global.", "anthropic."] {
        if let Some(rest) = m.strip_prefix(prefix) {
            m = rest.to_string();
        }
    }
    if let Some(at) = m.find('@') {
        m.truncate(at);
    }
    // A version ("-v1:0") and a date ("-20251001") on the end
    while let Some((rest, end)) = m.rsplit_once('-') {
        let date = end.len() == 8 && end.chars().all(|c| c.is_ascii_digit());
        let version = end.len() > 1 && end.starts_with('v') && end[1..].chars().all(|c| c.is_ascii_digit() || c == ':');
        if !date && !version {
            break;
        }
        m = rest.to_string();
    }
    m
}

/// How big a session's window is, in tokens. More than 200k in context can only fit the 1M window, whatever the model.
fn window(model_id: Option<&str>, model: &str, tokens: u64) -> u64 {
    let asked = model_id.is_some_and(|id| id.to_lowercase().contains("[1m]") && plain_model(id) == plain_model(model));
    if asked || NATIVE_1M.contains(&plain_model(model).as_str()) || tokens > 200_000 {
        1_000_000
    } else {
        200_000
    }
}

/// Where Claude Code compacts a conversation on its own, as a rough share of the window: about 33k short of the end.
pub fn compacts_at(window: u64) -> u32 {
    if window >= 1_000_000 {
        97
    } else {
        83
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(at: &str, model: &str, input: u64, cached: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{at}","message":{{"model":"{model}","usage":{{"input_tokens":{input},"cache_read_input_tokens":{cached},"cache_creation_input_tokens":0,"output_tokens":5}}}}}}"#
        ) + "\n"
    }

    #[test]
    fn the_context_is_the_last_replys_input_and_copies_are_skipped() {
        let mut t = Transcript::default();
        t.take(&reply("2026-09-26T10:00:00Z", "claude-sonnet-4-6", 1000, 99_000));
        assert_eq!(t.share(), (50, 200_000));
        // A line written again during a compaction, with its old time, doesn't count
        t.take(&reply("2026-09-26T09:00:00Z", "claude-sonnet-4-6", 10, 10));
        assert_eq!(t.tokens, 100_000);
        t.take(r#"{"type":"system","subtype":"compact_boundary","timestamp":"2026-09-26T10:05:00Z"}"#);
        assert_eq!(t.share().0, 0);
    }

    #[test]
    fn a_one_million_window_comes_from_the_model() {
        assert_eq!(window(None, "claude-opus-5-5", 10), 1_000_000);
        assert_eq!(window(None, "claude-sonnet-4-6", 10), 200_000);
        assert_eq!(window(Some("claude-sonnet-4-6[1m]"), "claude-sonnet-4-6", 10), 1_000_000);
        assert_eq!(window(Some("claude-sonnet-4-6[1m]"), "claude-haiku-4-5-20251001", 10), 200_000);
        assert_eq!(plain_model("us.anthropic.claude-haiku-4-5-20251001-v1:0"), "claude-haiku-4-5");
        assert_eq!(compacts_at(1_000_000), 97);
        assert_eq!(compacts_at(200_000), 83);
    }

    #[test]
    fn an_advisor_reply_counts_its_last_message() {
        let usage = serde_json::json!({
            "input_tokens": 500_000, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0,
            "iterations": [
                { "type": "message", "input_tokens": 250_000, "output_tokens": 1, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 },
                { "type": "advisor_message", "input_tokens": 250_000 }
            ]
        });
        assert_eq!(context_tokens(&usage), 250_000);
    }

    #[test]
    fn the_alert_goes_off_once_when_the_threshold_is_passed() {
        let dir = std::env::temp_dir().join("headroom-context-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s1.jsonl");
        std::fs::write(&path, reply("2026-09-26T10:00:00Z", "claude-sonnet-4-6", 0, 100_000)).unwrap();
        let mut contexts = Contexts::default();
        contexts.transcripts.insert("s1".into(), Transcript { path: Some(path.clone()), ..Default::default() });
        let sessions = vec!["s1".to_string()];
        // Past the threshold already when first read: that's where it stands, not news
        assert!(contexts.update(&sessions, 40).is_empty());
        let append = |line: String| {
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(line.as_bytes()).unwrap();
        };
        append(reply("2026-09-26T10:01:00Z", "claude-sonnet-4-6", 0, 150_000));
        let crossed = contexts.update(&sessions, 70);
        assert_eq!(crossed.len(), 1);
        assert_eq!((crossed[0].pct, crossed[0].window), (75, 200_000));
        append(reply("2026-09-26T10:02:00Z", "claude-sonnet-4-6", 0, 160_000));
        assert!(contexts.update(&sessions, 70).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
