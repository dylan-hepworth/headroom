// What Claude Code's own transcripts say about recent use: every reply, when it came, which session and folder it was
// in, and how many tokens it took. Claude Code keeps these for every session under ~/.claude/projects, whether or not
// Headroom's hooks are on, so this is what "usage by project" and the daily recap are built from.
//
// Transcripts run to megabytes, so we only read ones touched since the time we care about, only parse the lines that
// carry token counts, and keep what we found for each file until the file changes.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Local};
use serde_json::Value;

/// One reply from Claude.
#[derive(Clone)]
pub struct Reply {
    pub session: String,
    pub project: String,
    pub at: DateTime<Local>,
    /// Roughly what the reply cost, in dollars at API prices. It's only used to compare replies with each other, so
    /// that a long Opus reply counts for more than a short Haiku one.
    pub weight: f64,
}

/// Replies found in each transcript, and the file's modified time when we read it.
#[derive(Default)]
pub struct Transcripts {
    files: HashMap<PathBuf, (SystemTime, Vec<Reply>)>,
}

impl Transcripts {
    /// Every reply since `since`, from every Claude Code session on this Mac.
    pub fn since(&mut self, since: DateTime<Local>) -> Vec<Reply> {
        let Some(root) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude/projects")) else {
            return vec![];
        };
        let cutoff = SystemTime::from(since);
        let mut seen = HashSet::new();
        let mut replies = vec![];
        for path in transcript_files(&root) {
            let Some(modified) = fs::metadata(&path).and_then(|m| m.modified()).ok() else { continue };
            if modified < cutoff {
                continue;
            }
            seen.insert(path.clone());
            let cached = self.files.get(&path).filter(|(when, _)| *when == modified);
            if cached.is_none() {
                self.files.insert(path.clone(), (modified, read(&path)));
            }
            replies.extend(self.files[&path].1.iter().filter(|r| r.at >= since).cloned());
        }
        // Files that have gone quiet since last time don't need to stay in memory
        self.files.retain(|path, _| seen.contains(path));
        replies
    }
}

/// Every transcript under ~/.claude/projects: one per session in each project's folder, plus subagents' in the
/// session's own folder.
fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![];
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                files.push(path);
            }
        }
    }
    files
}

/// The replies in one transcript. Claude Code writes a reply's line more than once as it streams, with the same
/// message ID, so each ID is only counted once.
fn read(path: &Path) -> Vec<Reply> {
    let Ok(text) = fs::read_to_string(path) else { return vec![] };
    let mut ids = HashSet::new();
    let mut replies = vec![];
    for line in text.lines().filter(|l| l.contains("\"usage\"") && l.contains("\"assistant\"")) {
        let Ok(line) = serde_json::from_str::<Value>(line) else { continue };
        let message = &line["message"];
        if line["type"] != "assistant" || !message["id"].as_str().is_some_and(|id| ids.insert(id.to_string())) {
            continue;
        }
        let Some(at) = line["timestamp"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { continue };
        replies.push(Reply {
            session: line["sessionId"].as_str().unwrap_or_default().to_string(),
            project: project(line["cwd"].as_str().unwrap_or_default()),
            at: at.with_timezone(&Local),
            weight: weight(message["model"].as_str().unwrap_or_default(), &message["usage"]),
        });
    }
    replies
}

/// A project's name from a session's folder: its last part, except that a worktree Claude Code made
/// (`repo/.claude/worktrees/name`) counts as the repo it belongs to.
pub fn project(cwd: &str) -> String {
    let path = cwd.split("/.claude/worktrees/").next().unwrap_or(cwd);
    Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// A reply's rough cost in dollars, from its token counts and API prices per million tokens. Reading from the cache
/// is a tenth of the input price, and writing to it a quarter more.
fn weight(model: &str, usage: &Value) -> f64 {
    let (input, output) = if model.contains("opus") {
        (5.0, 25.0)
    } else if model.contains("haiku") {
        (1.0, 5.0)
    } else {
        (3.0, 15.0)
    };
    let tokens = |key: &str| usage[key].as_f64().unwrap_or_default();
    (tokens("input_tokens") * input
        + tokens("cache_creation_input_tokens") * input * 1.25
        + tokens("cache_read_input_tokens") * input * 0.1
        + tokens("output_tokens") * output)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktrees_count_as_their_repo() {
        assert_eq!(project("/Users/me/Code/pagewise/.claude/worktrees/translate-cloud"), "pagewise");
        assert_eq!(project("/Users/me/Code/headroom"), "headroom");
    }

    #[test]
    fn a_reply_written_twice_counts_once() {
        let path = std::env::temp_dir().join("headroom-transcript-test.jsonl");
        let line = r#"{"type":"assistant","timestamp":"2026-09-25T23:28:55.931Z","cwd":"/x/site","sessionId":"s","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":1000000,"output_tokens":0}}}"#;
        fs::write(&path, format!("{line}\n{line}\n")).unwrap();
        let replies = read(&path);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].project, "site");
        assert!((replies[0].weight - 5.0).abs() < 1e-9);
        let _ = fs::remove_file(path);
    }
}
