use std::{
    collections::{HashMap, HashSet},
    os::windows::ffi::OsStrExt,
    process::Command,
};

use windows::{
    core::{BOOL, PCWSTR, PWSTR},
    Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation},
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, UNICODE_STRING, WAIT_OBJECT_0},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
                PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                PROCESS_VM_READ,
            },
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                EnumWindows, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
                SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
            },
        },
    },
};

use super::{Process, CLAUDE_APP};

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// A process handle that's closed when it goes.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle OpenProcess gave us, closed once
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open_process(pid: u32, access: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS) -> Option<Handle> {
    // SAFETY: OpenProcess with a process ID; the handle is closed by `Handle`
    unsafe { OpenProcess(access, false, pid).ok().map(Handle) }
}

/// Each process's parent and file name, from a snapshot of them all.
fn table() -> HashMap<u32, (u32, String)> {
    let mut out = HashMap::new();
    // SAFETY: a snapshot walked with a PROCESSENTRY32W sized as the API wants, and closed after
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let snapshot = Handle(snapshot);
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot.0, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.insert(entry.th32ProcessID, (entry.th32ParentProcessID, name));
            ok = Process32NextW(snapshot.0, &mut entry).is_ok();
        }
    }
    out
}

/// The process that started this one.
pub fn parent_id() -> i32 {
    table().get(&std::process::id()).map_or(0, |(ppid, _)| *ppid as i32)
}

/// Watches whether the process that started this one is still there. Windows doesn't hand an orphan to another parent,
/// so this holds the parent open and asks whether it has ended.
pub struct Parent(Option<Handle>);

impl Parent {
    pub fn new() -> Self {
        let ppid = parent_id() as u32;
        Parent((ppid != 0).then(|| open_process(ppid, PROCESS_SYNCHRONIZE)).flatten())
    }

    pub fn gone(&self) -> bool {
        let Some(handle) = &self.0 else { return false };
        // SAFETY: waits no time at all on a handle we hold
        unsafe { WaitForSingleObject(handle.0, 0) == WAIT_OBJECT_0 }
    }
}

/// Is this process still running?
pub fn running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let Some(handle) = open_process(pid as u32, PROCESS_SYNCHRONIZE) else {
        // Not allowed to look is still running; gone is gone
        return table().contains_key(&(pid as u32));
    };
    // SAFETY: as in `Parent::gone`
    unsafe { WaitForSingleObject(handle.0, 0) != WAIT_OBJECT_0 }
}

/// Stop a process and everything it started. Windows has no polite signal for a console program, so both ways end it.
pub fn stop_group(group: i32, _force: bool) {
    let mut taskkill = Command::new("taskkill");
    taskkill.args(["/T", "/F", "/PID", &group.to_string()]);
    let _ = super::quiet(&mut taskkill).output();
}

/// A process's program, with its whole path.
fn exe_of(pid: u32) -> Option<String> {
    let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer's size goes with it, and the call writes no more than that
    unsafe { QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()? };
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// What an app is called, for telling whether two are the same: its program's path, in lowercase. The Claude app is
/// called by its Mac bundle ID, as the hooks call it there, so the rest of Headroom can tell it apart either way.
fn app_of(pid: u32) -> Option<String> {
    let exe = exe_of(pid)?.to_lowercase();
    let name = exe.rsplit('\\').next().unwrap_or(&exe);
    // Claude Code's own program is claude.exe too, but it has no window, and only apps with one are asked about
    Some(if name == "claude.exe" { CLAUDE_APP.to_string() } else { exe })
}

/// The process that owns the window in front.
fn foreground_pid() -> Option<u32> {
    // SAFETY: plain window queries
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

/// The app with the window in front: its program's path, like "c:\program files\windowsapps\…\windowsterminal.exe".
pub fn frontmost_app() -> Option<String> {
    app_of(foreground_pid()?)
}

/// The processes that have a window showing, of their own rather than one another window owns.
fn window_owners() -> HashSet<u32> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the set passed in below, alive for the whole of EnumWindows
        unsafe {
            let owned = GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
            if IsWindowVisible(window).as_bool() && !owned {
                let mut pid = 0u32;
                GetWindowThreadProcessId(window, Some(&mut pid));
                (*(found.0 as *mut HashSet<u32>)).insert(pid);
            }
        }
        true.into()
    }
    let mut found: HashSet<u32> = HashSet::new();
    // SAFETY: the callback only touches `found`, which outlives the call
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// The app this process runs for: the nearest process above it with a window, which for a hook is the terminal, the
/// editor, or the Claude app its session runs in. Windows doesn't say, as macOS does, so it's found by walking up.
pub fn host_app() -> Option<String> {
    let table = table();
    let owners = window_owners();
    let mut pid = std::process::id();
    for _ in 0..24 {
        let (ppid, _) = table.get(&pid)?;
        if *ppid == 0 || *ppid == pid {
            return None;
        }
        pid = *ppid;
        if owners.contains(&pid) {
            // A team's manager runs under Headroom itself (see team.rs), which isn't an app its session runs in
            let headroom = exe_of(pid).is_some_and(|e| e.rsplit('\\').next().is_some_and(|n| n.to_lowercase().starts_with("headroom")));
            return (!headroom).then(|| app_of(pid)).flatten();
        }
    }
    None
}

/// Is one of Headroom's own windows the one in front?
pub fn headroom_in_front() -> bool {
    foreground_pid() == Some(std::process::id())
}

fn shell_open(target: &str, verb: Option<&str>, args: Option<&str>) -> std::io::Result<()> {
    let (target, verb, args) = (wide(target), verb.map(wide), args.map(wide));
    // SAFETY: every string is a NUL-terminated wide string that outlives the call
    let done = unsafe {
        ShellExecuteW(
            None,
            verb.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            PCWSTR(target.as_ptr()),
            args.as_ref().map_or(PCWSTR::null(), |a| PCWSTR(a.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Anything above 32 worked
    match done.0 as isize {
        n if n > 32 => Ok(()),
        n => Err(std::io::Error::other(format!("Windows couldn't open it ({n})"))),
    }
}

/// Open a web address, a link another app handles, or a file in the app it belongs to.
pub fn open(target: &str) -> std::io::Result<()> {
    shell_open(target, None, None)
}

/// Bring an app to the front: the first window of a process running its program.
pub fn open_app(id: &str) {
    let mut windows: Vec<(HWND, u32)> = vec![];
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the list passed in below, alive for the whole of EnumWindows
        unsafe {
            let owned = GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
            if IsWindowVisible(window).as_bool() && !owned {
                let mut pid = 0u32;
                GetWindowThreadProcessId(window, Some(&mut pid));
                (*(found.0 as *mut Vec<(HWND, u32)>)).push((window, pid));
            }
        }
        true.into()
    }
    // SAFETY: the callback only touches `windows`, which outlives the call
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut windows as *mut _ as isize));
    }
    // Windows are listed front to back, so the first is the one last used
    let Some((window, _)) = windows.into_iter().find(|(_, pid)| app_of(*pid).as_deref() == Some(id)) else { return };
    // SAFETY: a window that was just listed; if it's gone since, these calls fail harmlessly
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);
    }
}

/// Show a file in File Explorer, selected.
pub fn reveal(path: &str) -> std::io::Result<()> {
    shell_open("explorer.exe", None, Some(&format!("/select,\"{path}\"")))
}

/// Open Windows' notification settings.
pub fn notification_settings() {
    let _ = open("ms-settings:notifications");
}

/// A process's command line, which Windows only gives to a process allowed to read the other's memory.
pub fn command_line(pid: u32) -> Option<String> {
    let handle = open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
        .or_else(|| open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION))?;
    let mut buf = vec![0u8; 8192];
    let mut len = 0u32;
    // SAFETY: the buffer's size goes with it; on success it starts with a UNICODE_STRING pointing into itself
    unsafe {
        let status = NtQueryInformationProcess(
            handle.0,
            ProcessCommandLineInformation,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut len,
        );
        if status.is_err() {
            return None;
        }
        let text = &*(buf.as_ptr() as *const UNICODE_STRING);
        if text.Buffer.is_null() {
            return None;
        }
        let chars = std::slice::from_raw_parts(text.Buffer.0, text.Length as usize / 2);
        Some(String::from_utf16_lossy(chars))
    }
}

/// Every process, with its program's name for its command: reading every command line would take a while, so
/// `command_line` is for the few that matter. Each is its own group, since Windows has none.
pub fn processes() -> Vec<Process> {
    table()
        .into_iter()
        .map(|(pid, (ppid, name))| Process { pid: pid as i32, ppid: ppid as i32, group: pid as i32, command: name })
        .collect()
}

/// Is the taskbar light? Windows keeps the taskbar's theme apart from apps' theme, as "SystemUsesLightTheme".
pub fn light_taskbar() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    // SAFETY: a DWORD-sized buffer with its size, and NUL-terminated names
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    read.is_ok() && value == 1
}
