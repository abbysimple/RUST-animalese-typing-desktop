//! macOS counterparts of the Win32 helpers in `sys.rs`, with the same function names so the
//! editor and the rest of the app don't need to know which platform they run on.
//!
//! - Single instance: an advisory lock on a file next to the config.
//! - Editor → daemon messages: a Unix socket next to the config, served by the daemon.
//! - Run on startup: a LaunchAgent in `~/Library/LaunchAgents`.

use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSScreen, NSWorkspace};
use objc2_foundation::MainThreadMarker;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const SETTINGS_TITLE: &str = "Animalese Typing Settings";
const SETTINGS_LOCK: &str = "animalese-rs-settings";
const LAUNCH_AGENT_LABEL: &str = "com.github.abbysimple.animalese-rs";

/// Appends a line to `log.txt` next to the config (errors only, so it stays tiny).
pub fn log(msg: &str) {
    let p = crate::config::path().with_file_name("log.txt");
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{msg}");
    }
}

fn data_dir() -> PathBuf {
    let p = crate::config::path();
    let dir = p.parent().map(PathBuf::from).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn lock_path(name: &str) -> PathBuf {
    // Names come from the Windows build ("Local\\animalese-rs-daemon"); keep the last part.
    let base = name.rsplit('\\').next().unwrap_or(name);
    data_dir().join(format!("{base}.lock"))
}

pub fn socket_path() -> PathBuf {
    data_dir().join("daemon.sock")
}

/// Returns false if another process already holds this lock. The lock lives as long as the
/// process does, and the OS drops it automatically if the process dies.
pub fn single_instance(name: &str) -> bool {
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path(name))
    else {
        return true;
    };
    if f.try_lock().is_err() {
        return false;
    }
    let _ = f.set_len(0);
    let _ = write!(f, "{}", std::process::id());
    Box::leak(Box::new(f));
    true
}

/// PID written by the process holding `name`, if it's still held.
fn lock_holder(name: &str) -> Option<i32> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(lock_path(name)).ok()?;
    if f.try_lock().is_ok() {
        return None; // nobody holds it
    }
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    s.trim().parse().ok()
}

/// Sends one command line to the daemon and returns its reply.
fn ask_daemon(cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket_path()).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(200))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(200))).ok()?;
    s.write_all(format!("{cmd}\n").as_bytes()).ok()?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    s.read_to_string(&mut reply).ok()?;
    Some(reply.trim().to_string())
}

/// Tells a running daemon to reload settings.
pub fn notify_daemon() {
    let _ = ask_daemon("reload");
}

pub fn toggle_daemon_mute() {
    let _ = ask_daemon("toggle");
}

/// Some(disabled) from the running daemon, None if it isn't running.
pub fn daemon_muted() -> Option<bool> {
    ask_daemon("muted").map(|r| r == "1")
}

pub fn spawn_daemon() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

pub fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// Top-left position for the anchored editor: under the menu bar, at the right edge where the
/// status icon lives.
pub fn anchor_position(w: f32, _h: f32) -> Option<(f32, f32)> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let screen = screens.firstObject()?; // the one with the menu bar
    let full = screen.frame();
    let visible = screen.visibleFrame();
    let right = visible.origin.x + visible.size.width - full.origin.x;
    let top_inset = (full.origin.y + full.size.height) - (visible.origin.y + visible.size.height);
    Some((right as f32 - w - 10.0, top_inset as f32 + 10.0))
}

/// Brings an existing settings window to the front. Returns false if none is open.
#[allow(deprecated)]
pub fn focus_settings() -> bool {
    let Some(pid) = lock_holder(SETTINGS_LOCK) else { return false };
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else { return false };
    app.activateWithOptions(
        NSApplicationActivationOptions::ActivateAllWindows | NSApplicationActivationOptions::ActivateIgnoringOtherApps,
    );
    true
}

pub fn spawn_settings() {
    if focus_settings() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}

/// The frontmost app: (pid, display name, whether it's this program — daemon or editor).
pub fn frontmost_app() -> Option<(i32, String, bool)> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    let name = app_name(&app)?;
    let own = app
        .executableURL()
        .and_then(|u| u.path())
        .map(|p| PathBuf::from(p.to_string()))
        .zip(std::env::current_exe().ok())
        .is_some_and(|(a, b)| same_file(&a, &b));
    Some((pid, name, own))
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn app_name(app: &NSRunningApplication) -> Option<String> {
    app.localizedName()
        .map(|n| n.to_string())
        .or_else(|| app.executableURL().and_then(|u| u.lastPathComponent()).map(|n| n.to_string()))
        .filter(|n| !n.is_empty())
}

/// Names of regular (Dock) apps that are currently running, sorted.
pub fn running_apps() -> Vec<String> {
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut list: Vec<String> = Vec::new();
    for app in apps.iter() {
        if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        if let Some(name) = app_name(&app) {
            if !list.contains(&name) {
                list.push(name);
            }
        }
    }
    list.sort_by_key(|s| s.to_lowercase());
    list
}

// Run on startup uses a per-user LaunchAgent that starts this executable at login.

fn launch_agent() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/LaunchAgents").join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn launch_agent_plist(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCH_AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        xml_escape(exe)
    )
}

/// True when the LaunchAgent exists and launches this executable.
pub fn autostart_is_current() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Ok(text) = std::fs::read_to_string(launch_agent()) else { return false };
    text == launch_agent_plist(&exe.to_string_lossy())
}

pub fn set_autostart(enable: bool) {
    let path = launch_agent();
    let result = (|| {
        if !enable {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        let exe = std::env::current_exe()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written, not loaded: loading it now would start a second copy. It runs at next login.
        std::fs::write(&path, launch_agent_plist(&exe.to_string_lossy()))
    })();
    if let Err(e) = result {
        log(&format!("set_autostart({enable}) failed: {e}"));
    }
}
