// What differs between macOS and Windows: where things are kept, the processes around a hook, the app in front, and
// opening things. The rest of Headroom asks here rather than asking the system itself, so the two stay in one place.

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

/// Headroom's bundle ID, which is also the name of its folder.
pub const APP_ID: &str = "io.github.dylan-hepworth.headroom";

/// The Claude app's bundle ID. On Windows, where apps don't have one, the Claude app is called this too, so the hooks
/// and the app agree on it (see `app_of`).
pub const CLAUDE_APP: &str = "com.anthropic.claudefordesktop";

/// The user's home folder. Windows doesn't set HOME (Git Bash does, which would give a hook a different answer than
/// the app), so there it's the profile folder, which is where Claude Code keeps ~/.claude too.
pub fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// Claude Code's folder, ~/.claude.
pub fn claude_dir() -> Option<PathBuf> {
    Some(home()?.join(".claude"))
}

/// Headroom's own folder, where the settings, the hooks' log, and everything else it keeps go. The same folder Tauri
/// gives the app as its config folder, found here without Tauri, for the hooks.
pub fn app_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return Some(home()?.join("Library/Application Support").join(APP_ID));
    #[cfg(windows)]
    return Some(PathBuf::from(std::env::var_os("APPDATA").filter(|d| !d.is_empty())?).join(APP_ID));
}

/// The Claude app's folder, where it keeps its Code chats.
pub fn claude_app_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return Some(home()?.join("Library/Application Support/Claude"));
    #[cfg(windows)]
    return Some(PathBuf::from(std::env::var_os("APPDATA").filter(|d| !d.is_empty())?).join("Claude"));
}

/// The Claude app's main log, which says which chat is open there.
pub fn claude_app_log() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return Some(home()?.join("Library/Logs/Claude/main.log"));
    #[cfg(windows)]
    return Some(claude_app_dir()?.join("logs/main.log"));
}

/// Keep a folder to the user alone. On Windows a folder under the profile already is.
pub fn private_dir(dir: &Path) {
    #[cfg(unix)]
    let _ = std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    #[cfg(not(unix))]
    let _ = dir;
}

/// Open a file only the user can read.
pub fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(options, 0o600);
    options
}

/// A command that runs without a console window flashing up on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::os::windows::process::CommandExt::creation_flags(command, CREATE_NO_WINDOW);
    }
    command
}

/// A path as people write it. Windows hands back `\\?\C:\…` for a path that's been made canonical, which other apps
/// and the user don't expect.
pub fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// A path for a hook's command line in ~/.claude/settings.json. On Windows Claude Code may run it through Git Bash,
/// which reads backslashes as escapes, or through cmd; forward slashes work in both.
pub fn command_path(path: &Path) -> String {
    let path = path.display().to_string();
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    path.replace('"', "\\\"")
}

/// A process: its ID, its parent's, its group (its own ID on Windows, which has none), and its command line.
#[derive(Clone, Debug)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub group: i32,
    pub command: String,
}

/// Note a problem in Headroom's log, headroom.log in its folder, for the things that have no window to say so in (a
/// notification that couldn't be shown, say). It's kept small: past 256 KB it starts over.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn note(what: &str) {
    use std::io::Write;
    let Some(path) = app_dir().map(|d| d.join("headroom.log")) else { return };
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = private(OpenOptions::new().create(true).append(true)).open(&path) {
        let _ = writeln!(file, "{} {what}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}
