// Chat icons: an emoji or a picture that each Claude Code chat is shown with, in the list, on its cards, and in its
// notifications, so the one asking for attention is easy to tell apart from the rest at a glance.
//
// A chat without one of its own gets its project's, if one was picked for the whole project, and otherwise one of the
// emoji, the same one every time for the same chat. They're kept in icons.json in Headroom's folder: by chat (the
// Claude app's ID for it, or else the session's), and by project folder.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};

/// The emoji a chat can start with. The picker in the settings window and the popover offers the same ones.
const EMOJI: [&str; 32] = [
    "🦊", "🐙", "🦉", "🐝", "🐢", "🦋", "🐳", "🦕", "🚀", "🛠️", "🧪", "📦", "🧭", "🎨", "📚", "🔭", "🌵", "🍋", "🌶️",
    "🍄", "🌊", "🔥", "⚡", "❄️", "💎", "🎯", "🧩", "🎲", "🪐", "🌙", "⭐", "🍀",
];

/// The most a picture can take, as the data URL the page sends. The page makes it a small square before it's sent.
const MAX_PICTURE: usize = 400 * 1024;

pub struct Icons {
    path: PathBuf,
    chats: Map<String, Value>,
    projects: Map<String, Value>,
}

impl Icons {
    pub fn load(dir: &Path) -> Icons {
        let path = dir.join("icons.json");
        let saved: Value =
            fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let map = |key: &str| saved[key].as_object().cloned().unwrap_or_default();
        Icons { chats: map("chats"), projects: map("projects"), path }
    }

    /// A chat's icon: its own, or its project's, or the emoji it starts with.
    pub fn of(&self, chat: &str, project: &str) -> Value {
        self.chats
            .get(chat)
            .or_else(|| self.projects.get(project))
            .cloned()
            .unwrap_or_else(|| json!({ "emoji": default_emoji(chat) }))
    }

    /// Give a chat an icon, or with `project`, every chat in its project folder, other than any in `others` (that
    /// folder's chats Headroom knows about) that had one of their own: they get it too. `None` goes back to the one it
    /// started with.
    pub fn set(&mut self, chat: &str, project: &str, icon: Option<Value>, whole_project: bool, others: &[String]) {
        match (icon, whole_project) {
            (Some(icon), true) => {
                self.projects.insert(project.into(), icon);
                for other in others.iter().map(String::as_str).chain([chat]) {
                    self.chats.remove(other);
                }
            }
            (Some(icon), false) => {
                self.chats.insert(chat.into(), icon);
            }
            (None, _) => {
                self.chats.remove(chat);
                if whole_project {
                    self.projects.remove(project);
                }
            }
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&self.path, json!({ "chats": self.chats, "projects": self.projects }).to_string());
    }
}

/// Is this an icon the page can send: an emoji, or a picture as a data URL that isn't too big?
pub fn valid(icon: &Value) -> bool {
    match (icon["emoji"].as_str(), icon["image"].as_str()) {
        (Some(emoji), None) => !emoji.is_empty() && emoji.chars().count() <= 8,
        (None, Some(image)) => image.starts_with("data:image/") && image.len() <= MAX_PICTURE,
        _ => false,
    }
}

/// The emoji a chat starts with: one of `EMOJI`, picked by its ID, so it's the same one every time. The hash is
/// written out (FNV-1a) rather than Rust's own, which can change from one version to the next.
fn default_emoji(chat: &str) -> &'static str {
    let hash = chat.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    EMOJI[(hash % EMOJI.len() as u64) as usize]
}

/// The emoji to start a notification's title with, if the icon is one.
pub fn emoji(icon: &Value) -> Option<&str> {
    icon["emoji"].as_str()
}

/// A picture icon as a file of its own, for a notification to show beside its text. macOS takes the file into its own
/// store when the notification goes out, so each one gets a fresh copy, in a folder that's cleared as it goes.
pub fn picture_file(icon: &Value, dir: &Path) -> Option<PathBuf> {
    use base64::Engine;
    let data = icon["image"].as_str()?;
    let (head, body) = data.split_once(',')?;
    let ext = match head.strip_prefix("data:image/")?.strip_suffix(";base64")? {
        "png" => "png",
        "jpeg" | "jpg" => "jpg",
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(body).ok()?;
    let dir = dir.join("notification-pictures");
    fs::create_dir_all(&dir).ok()?;
    // What's left from before (a notification that never went out keeps its copy)
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let old =
            entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a.as_secs() > 3600));
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
    let path = dir.join(format!("{}.{ext}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
    fs::write(&path, bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_has_its_own_icon_or_its_projects_or_one_it_starts_with() {
        let dir = std::env::temp_dir().join(format!("headroom-icons-{}", std::process::id()));
        let mut icons = Icons::load(&dir);
        let start = icons.of("chat-a", "/code/site");
        assert_eq!(start, icons.of("chat-a", "/code/other"), "the same one every time");
        assert!(EMOJI.contains(&start["emoji"].as_str().unwrap()));

        icons.set("chat-a", "/code/site", Some(json!({ "emoji": "🦊" })), false, &[]);
        assert_eq!(icons.of("chat-a", "/code/site")["emoji"], "🦊");
        // For the whole project: the other chats there get it, and so do ones that come later
        icons.set("chat-b", "/code/site", Some(json!({ "emoji": "🐙" })), true, &["chat-a".into()]);
        assert_eq!(icons.of("chat-a", "/code/site")["emoji"], "🐙");
        assert_eq!(icons.of("chat-new", "/code/site")["emoji"], "🐙");
        // Kept across restarts
        assert_eq!(Icons::load(&dir).of("chat-new", "/code/site")["emoji"], "🐙");
        icons.set("chat-a", "/code/site", None, true, &[]);
        assert_eq!(icons.of("chat-a", "/code/site"), start);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_an_emoji_or_a_small_picture_will_do() {
        assert!(valid(&json!({ "emoji": "🦊" })));
        assert!(valid(&json!({ "image": "data:image/png;base64,iVBOR" })));
        assert!(!valid(&json!({ "image": "file:///etc/passwd" })));
        assert!(!valid(&json!({ "emoji": "" })));
        assert!(!valid(&json!({ "emoji": "🦊", "image": "data:image/png;base64,x" })));
        assert!(!valid(&json!({ "image": format!("data:image/png;base64,{}", "A".repeat(MAX_PICTURE)) })));
    }
}
