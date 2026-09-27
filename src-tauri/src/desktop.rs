// What the Claude app knows about its Code chats that Claude Code's hooks don't tell us, so the menu bar's dots can
// follow the app's own:
//
// - After each turn, Claude Code sorts it: finished, or waiting on the user ("blocked", "need_input") because Claude
//   ended by asking something in plain text. The app keeps that for each chat in a file of its own, under
//   ~/Library/Application Support/Claude/claude-code-sessions, along with whether the chat is archived. A chat waiting
//   on the user gets the yellow dot there, not the blue one.
// - A finished chat only gets a dot if it wasn't open when the turn ended, and opening it clears the dot. The app logs
//   which chat is open whenever that changes ("LocalSessions.setFocusedSession" in ~/Library/Logs/Claude/main.log).
//
// None of this is documented, so anything that isn't there as expected is taken as "no word from the app", and the
// session is shown the way the hooks alone would have it.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    time::{Instant, SystemTime},
};

use serde_json::Value;

/// How the Claude app sees a chat's last turn.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Chat {
    /// Claude ended the turn waiting on the user, asking something in plain text.
    pub needs_you: bool,
    pub archived: bool,
}

#[derive(Default)]
pub struct Desktop {
    /// How far into the app's log we've read, and the chat that's open, by the app's own ID ("local_…"), once the log
    /// has said (`focus_known`): it can say none is.
    log_offset: u64,
    open: Option<String>,
    focus_known: bool,
    /// Each Claude Code session's chat file, by session ID, and what it said when it last changed.
    files: HashMap<String, (PathBuf, SystemTime, Chat)>,
    /// The app's chat files, by the Claude Code session in each, and when they were last listed.
    index: HashMap<String, PathBuf>,
    indexed: Option<Instant>,
}

impl Desktop {
    /// Catch up on the app's log. Returns whether a different chat is open now.
    pub fn follow_log(&mut self) -> bool {
        let Some(path) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/Claude/main.log")) else {
            return false;
        };
        let Ok(mut file) = File::open(path) else { return false };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        // A new log (it's rotated at about 10MB), or the first look: only the end matters
        if len < self.log_offset || self.log_offset == 0 {
            self.log_offset = len.saturating_sub(512 * 1024);
        }
        if len == self.log_offset || file.seek(SeekFrom::Start(self.log_offset)).is_err() {
            return false;
        }
        let mut bytes = vec![];
        if file.read_to_end(&mut bytes).is_err() {
            return false;
        }
        // Up to the last full line; a line being written waits for next time
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { return false };
        self.log_offset += end as u64 + 1;
        let text = String::from_utf8_lossy(&bytes[..=end]);
        let before = self.open.clone();
        for line in text.lines() {
            if let Some(id) = line.split("LocalSessions.setFocusedSession: sessionId=").nth(1) {
                let id = id.split_whitespace().next().unwrap_or_default();
                self.open = id.starts_with("local_").then(|| id.to_string());
                self.focus_known = true;
            }
        }
        self.open != before
    }

    /// The app's ID for the chat that's open in it ("local_…"), as of the last look at its log: empty when none is,
    /// and None when the log hasn't said.
    pub fn open_chat(&self) -> Option<String> {
        self.focus_known.then(|| self.open.clone().unwrap_or_default())
    }

    /// Is this Claude Code session's chat the one open in the app?
    pub fn is_open(&mut self, session: &str) -> bool {
        let Some(open) = self.open.clone() else { return false };
        self.path(session).is_some_and(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == open))
    }

    /// The app's ID for a Claude Code session's chat ("local_…"), if it's one of the app's chats.
    pub fn chat_id(&mut self, session: &str) -> Option<String> {
        Some(self.path(session)?.file_stem()?.to_string_lossy().into_owned())
    }

    /// What the app says about a Claude Code session's last turn, if the session is one of its chats. The file is read
    /// again only when it changes.
    pub fn chat(&mut self, session: &str) -> Option<Chat> {
        let path = self.path(session)?;
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        if let Some((_, _, chat)) = self.files.get(session).filter(|(p, when, _)| *p == path && *when == modified) {
            return Some(*chat);
        }
        let v: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
        let chat = read_chat(&v);
        self.files.insert(session.to_string(), (path, modified, chat));
        Some(chat)
    }

    /// The app's file for a Claude Code session's chat. The files are listed again, at most every half minute, when a
    /// session isn't in the list yet.
    fn path(&mut self, session: &str) -> Option<PathBuf> {
        if let Some(path) = self.index.get(session) {
            return Some(path.clone());
        }
        if self.indexed.is_some_and(|t| t.elapsed().as_secs() < 30) {
            return None;
        }
        self.indexed = Some(Instant::now());
        let root =
            PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Claude/claude-code-sessions");
        for dir in std::fs::read_dir(root).ok()?.flatten() {
            for sub in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
                for file in std::fs::read_dir(sub.path()).into_iter().flatten().flatten() {
                    let path = file.path();
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !(name.starts_with("local_") && name.ends_with(".json")) {
                        continue;
                    }
                    let Some(v) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    else {
                        continue;
                    };
                    let ids = v["cliSessionId"]
                        .as_str()
                        .into_iter()
                        .chain(v["priorCliSessionIds"].as_array().into_iter().flatten().filter_map(Value::as_str));
                    for id in ids {
                        self.index.insert(id.to_string(), path.clone());
                    }
                }
            }
        }
        self.index.get(session).cloned()
    }
}

/// How the app sorted a chat's last turn. The sorting only counts if it's about the chat's latest reply; a turn wrap-up
/// can say the user is needed, or that they aren't after all.
fn read_chat(v: &Value) -> Chat {
    let summary = &v["postTurnSummary"];
    let current = summary["summarizes_uuid"].as_str().is_some() && summary["summarizes_uuid"] == v["lastAssistantUuid"];
    let mut category = if current { summary["status_category"].as_str().unwrap_or_default() } else { "" };
    let wrap_up = &v["turnWrapUp"];
    if wrap_up.is_object() && category != "failed" {
        if wrap_up["needsUser"].as_bool() == Some(true) {
            category = "blocked";
        } else if matches!(category, "blocked" | "need_input") {
            category = "review_ready";
        }
    }
    Chat { needs_you: matches!(category, "blocked" | "need_input"), archived: v["isArchived"] == true }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_turn_that_ends_waiting_on_the_user_needs_them() {
        let blocked = json!({ "lastAssistantUuid": "a", "postTurnSummary": { "status_category": "blocked", "summarizes_uuid": "a" } });
        assert_eq!(read_chat(&blocked), Chat { needs_you: true, archived: false });
        // About an older reply: it doesn't count
        let stale = json!({ "lastAssistantUuid": "b", "postTurnSummary": { "status_category": "blocked", "summarizes_uuid": "a" } });
        assert!(!read_chat(&stale).needs_you);
        let wrapped = json!({ "lastAssistantUuid": "a", "turnWrapUp": { "needsUser": false },
            "postTurnSummary": { "status_category": "need_input", "summarizes_uuid": "a" } });
        assert!(!read_chat(&wrapped).needs_you);
        assert!(read_chat(&json!({ "isArchived": true })).archived);
    }
}
