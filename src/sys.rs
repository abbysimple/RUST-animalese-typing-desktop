//! Small Win32 helpers shared by the tray daemon and the settings window.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const DAEMON_CLASS: &str = "AnimaleseRsDaemon";
pub const SETTINGS_TITLE: &str = "Animalese Typing Settings";
/// Posted to the daemon window to make it re-read the config file.
pub const WM_APP_RELOAD: u32 = WM_APP + 1;
/// Posted to the daemon to toggle "disabled".
pub const WM_APP_TOGGLE_MUTE: u32 = WM_APP + 5;
/// Sent to the daemon; returns 1 when disabled.
pub const WM_APP_GET_MUTED: u32 = WM_APP + 6;

/// Appends a line to `log.txt` next to the config (errors only, so it stays tiny).
pub fn log(msg: &str) {
    use std::io::Write;
    let p = crate::config::path().with_file_name("log.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{msg}");
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Returns false if another process already holds the named mutex.
pub fn single_instance(name: &str) -> bool {
    let n = wide(name);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, n.as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn daemon_hwnd() -> Option<HWND> {
    let class = wide(DAEMON_CLASS);
    let hwnd = unsafe { FindWindowExW(HWND_MESSAGE, null_mut(), class.as_ptr(), std::ptr::null()) };
    (!hwnd.is_null()).then_some(hwnd)
}

/// Tells a running daemon to reload settings.
pub fn notify_daemon() {
    if let Some(hwnd) = daemon_hwnd() {
        unsafe { PostMessageW(hwnd, WM_APP_RELOAD, 0, 0) };
    }
}

pub fn toggle_daemon_mute() {
    if let Some(hwnd) = daemon_hwnd() {
        unsafe { PostMessageW(hwnd, WM_APP_TOGGLE_MUTE, 0, 0) };
    }
}

/// Some(disabled) from the running daemon, None if it isn't running.
pub fn daemon_muted() -> Option<bool> {
    let hwnd = daemon_hwnd()?;
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(hwnd, WM_APP_GET_MUTED, 0, 0, SMTO_ABORTIFHUNG, 200, &mut result)
    };
    (ok != 0).then_some(result == 1)
}

pub fn spawn_daemon() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

pub fn open_url(url: &str) {
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
}

/// Bottom-right corner of the primary work area (above the taskbar), in logical points.
pub fn work_area_corner() -> Option<(f32, f32)> {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut _ as _, 0) };
    if ok == 0 {
        return None;
    }
    let scale = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() } as f32 / 96.0;
    Some((r.right as f32 / scale, r.bottom as f32 / scale))
}

/// Brings an existing settings window to the front. Returns false if none is open.
pub fn focus_settings() -> bool {
    let title = wide(SETTINGS_TITLE);
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        true
    }
}

pub fn spawn_settings() {
    if focus_settings() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}

/// Executable name without extension for the process owning `hwnd`, e.g. "chrome".
pub fn window_app(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        let path = OsString::from_wide(&buf[..len as usize]);
        Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
    }
}

pub fn own_app() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Names of apps that currently have a visible top-level window, sorted.
pub fn running_apps() -> Vec<String> {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        unsafe {
            let list = &mut *(lp as *mut Vec<String>);
            if IsWindowVisible(hwnd) != 0
                && GetWindowTextLengthW(hwnd) > 0
                && GetWindow(hwnd, GW_OWNER).is_null()
            {
                if let Some(name) = window_app(hwnd) {
                    if !list.contains(&name) {
                        list.push(name);
                    }
                }
            }
        }
        1
    }
    let mut list: Vec<String> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut list as *mut _ as LPARAM);
    }
    list.sort_by_key(|s| s.to_lowercase());
    list
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "AnimaleseTypingRs";

pub fn set_autostart(enable: bool) {
    use winreg::RegKey;
    use winreg::enums::*;
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) else {
        return;
    };
    if enable {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()));
        }
    } else {
        let _ = key.delete_value(RUN_VALUE);
    }
}
