use std::process::Command;

use super::{Process, APP_ID};

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// The process that started this one.
pub fn parent_id() -> i32 {
    std::os::unix::process::parent_id() as i32
}

/// Watches whether the process that started this one is still there. When it goes, macOS hands this one to another
/// parent, so a different parent ID means it's gone.
pub struct Parent(u32);

impl Parent {
    pub fn new() -> Self {
        Parent(std::os::unix::process::parent_id())
    }

    pub fn gone(&self) -> bool {
        std::os::unix::process::parent_id() != self.0
    }
}

/// Is this process still running? Signal 0 checks without sending anything. A process we're not allowed to signal is
/// still running; only "no such process" means it's gone.
pub fn running(pid: i32) -> bool {
    const ESRCH: i32 = 3;
    // SAFETY: kill with signal 0 only checks that the process exists and sends nothing
    let signalled = unsafe { kill(pid, 0) } == 0;
    signalled || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

/// Ask a process group to stop, or with `force`, make it.
pub fn stop_group(group: i32, force: bool) {
    const SIGTERM: i32 = 15;
    const SIGKILL: i32 = 9;
    // SAFETY: signals a process group Headroom found or started, which is never Headroom's own
    unsafe { kill(-group, if force { SIGKILL } else { SIGTERM }) };
}

/// The bundle ID of the app in front, like "com.mitchellh.ghostty".
pub fn frontmost_app() -> Option<String> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(app.bundleIdentifier()?.to_string())
}

/// The app this process runs for: macOS tells a process which app started it. For a hook, that's the terminal or the
/// Claude app its session runs in.
pub fn host_app() -> Option<String> {
    std::env::var("__CFBundleIdentifier").ok().filter(|id| !id.is_empty())
}

/// Is one of Headroom's own windows the one in front?
pub fn headroom_in_front() -> bool {
    frontmost_app().as_deref() == Some(APP_ID)
}

/// Open a web address, a link another app handles, or a file in the app it belongs to.
pub fn open(target: &str) -> std::io::Result<()> {
    Command::new("open").arg(target).spawn().map(|_| ())
}

/// Bring an app to the front, by its bundle ID.
pub fn open_app(id: &str) {
    let _ = Command::new("open").args(["-b", id]).spawn();
}

/// Show a file in Finder.
pub fn reveal(path: &str) -> std::io::Result<()> {
    Command::new("open").args(["-R", path]).spawn().map(|_| ())
}

/// Open Headroom's own page in System Settings → Notifications, where its alert style is chosen.
pub fn notification_settings() {
    let _ = Command::new("open")
        .arg(format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={APP_ID}"))
        .spawn();
}

/// Every process.
pub fn processes() -> Vec<Process> {
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
            let (pid, ppid, group) = (number()?, number()?, number()?);
            Some(Process { pid, ppid, group, command: rest.to_string() })
        })
        .collect()
}
