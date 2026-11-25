//! The always-running part on macOS: keyboard tap, frontmost-app filter, menu-bar icon and the
//! audio engine. Everything is handled on the main thread (the AppKit run loop); the keyboard
//! tap and the editor socket run on their own threads and hand work over with
//! `dispatch_async`, the macOS equivalent of the Windows build's `PostMessage`.

use crate::assets;
use crate::config::Config;
use crate::keytap_macos::{self, KeyEvent};
use crate::player::{Action, Hotkey, Player};
use crate::sys;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::MainThreadMarker;
use std::cell::RefCell;
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(queue: *const DispatchQueue, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_after_f(when: u64, queue: *const DispatchQueue, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

enum Msg {
    Init,
    Key(KeyEvent),
    Mute,
    Settings,
    Startup,
    Quit,
    Reload,
    AudioTick,
}

/// Queues `msg` for the main thread.
fn post(msg: Msg) {
    let ctx = Box::into_raw(Box::new(msg)).cast();
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, ctx, run_msg) };
}

fn post_after(secs: f64, msg: Msg) {
    let ctx = Box::into_raw(Box::new(msg)).cast();
    unsafe {
        let when = dispatch_time(0, (secs * 1e9) as i64);
        dispatch_after_f(when, &raw const _dispatch_main_q, ctx, run_msg);
    }
}

extern "C" fn run_msg(ctx: *mut c_void) {
    let msg = unsafe { *Box::from_raw(ctx.cast::<Msg>()) };
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(msg))) {
        sys::log(&format!("daemon message panicked: {e:?}"));
    }
}

/// Mirrors `Daemon::muted` so the socket thread can answer without the main thread.
static MUTED: AtomicBool = AtomicBool::new(false);

struct Daemon {
    cfg: Config,
    player: Player,
    hotkey: Option<Hotkey>,
    /// Disabled from the menu / hotkey.
    muted: bool,
    /// Disabled because of the app filter.
    filtered: bool,
    /// Frontmost app the filter was last computed for.
    front_pid: i32,
    tray: TrayIcon,
    mute_item: CheckMenuItem,
    startup_item: CheckMenuItem,
    audio_timer: bool,
}

thread_local! {
    static DAEMON: RefCell<Option<Daemon>> = const { RefCell::new(None) };
}

fn with_daemon(f: impl FnOnce(&mut Daemon)) {
    DAEMON.with(|d| {
        if let Ok(mut d) = d.try_borrow_mut() {
            if let Some(d) = d.as_mut() {
                f(d);
            }
        }
    });
}

fn icon(bytes: &[u8]) -> Option<Icon> {
    let (rgba, w, h) = assets::png_rgba(bytes)?;
    Icon::from_rgba(rgba, w, h).ok()
}

fn handle(msg: Msg) {
    match msg {
        Msg::Init => init(),
        Msg::Key(e) => with_daemon(|d| d.on_key(e)),
        Msg::Mute => with_daemon(|d| d.toggle_muted()),
        Msg::Settings => sys::spawn_settings(),
        Msg::Startup => with_daemon(|d| d.toggle_startup()),
        Msg::Quit => {
            DAEMON.with(|s| s.borrow_mut().take());
            std::process::exit(0);
        }
        Msg::Reload => with_daemon(|d| d.apply_config(Config::load())),
        Msg::AudioTick => with_daemon(|d| {
            d.player.engine.tick();
            d.audio_timer = d.player.engine.is_running();
            if d.audio_timer {
                post_after(1.0, Msg::AudioTick);
            }
        }),
    }
}

impl Daemon {
    fn apply_config(&mut self, cfg: Config) {
        self.hotkey = cfg.disable_hotkey.as_deref().and_then(Hotkey::parse);
        self.player.prepare(&cfg);
        // Self-heal the LaunchAgent if it's missing or points at another copy of the app.
        if cfg.startup_run && !sys::autostart_is_current() {
            sys::set_autostart(true);
        }
        self.startup_item.set_checked(cfg.startup_run);
        self.cfg = cfg;
        self.front_pid = -1;
        self.update_filter();
    }

    /// Recomputes the app filter when the frontmost app changed. Called on every key press,
    /// which costs one Objective-C call and avoids any polling.
    fn update_filter(&mut self) {
        let (pid, name, own) = sys::frontmost_app().unwrap_or((0, String::new(), false));
        if pid == self.front_pid {
            return;
        }
        self.front_pid = pid;
        self.filtered = if self.cfg.always_active || name.is_empty() || own {
            false
        } else {
            let listed = self.cfg.selected_apps.iter().any(|a| a.eq_ignore_ascii_case(&name));
            listed != self.cfg.selected_active
        };
        self.update_tray();
    }

    fn update_tray(&self) {
        let (img, tip) = if self.muted {
            (assets::ICON_MUTE, "Animalese Typing: Disabled")
        } else if self.filtered {
            (assets::ICON_OFF, "Animalese Typing: Off in this app")
        } else {
            (assets::ICON, "Animalese Typing")
        };
        let _ = self.tray.set_icon(icon(img));
        let _ = self.tray.set_tooltip(Some(tip));
        self.mute_item.set_checked(self.muted);
    }

    fn toggle_muted(&mut self) {
        self.muted = !self.muted;
        MUTED.store(self.muted, Ordering::Relaxed);
        self.update_tray();
    }

    fn toggle_startup(&mut self) {
        self.cfg.startup_run = !self.cfg.startup_run;
        sys::set_autostart(self.cfg.startup_run);
        self.cfg.save();
        self.startup_item.set_checked(self.cfg.startup_run);
    }

    fn on_key(&mut self, e: KeyEvent) {
        let m = e.mods;
        if e.down && self.hotkey.is_some_and(|h| h.matches(e.vk, m)) {
            self.toggle_muted();
            return;
        }
        if e.down {
            self.update_filter();
        }
        if self.muted || self.filtered {
            return;
        }
        if e.down {
            if let Action::ShowSettings = self.player.key_down(&self.cfg, e.vk, m) {
                sys::spawn_settings();
            }
            if self.player.engine.is_running() && !self.audio_timer {
                self.audio_timer = true;
                post_after(1.0, Msg::AudioTick);
            }
        } else {
            self.player.key_up(&self.cfg, e.vk, m);
        }
    }
}

/// Serves the editor's one-line commands: `reload`, `toggle`, `muted`.
fn serve_socket() {
    let path = sys::socket_path();
    let _ = std::fs::remove_file(&path); // stale socket from a previous run; we hold the lock
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            sys::log(&format!("binding {} failed: {e}", path.display()));
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        let mut line = String::new();
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let reply = match line.trim() {
            "reload" => {
                post(Msg::Reload);
                "ok"
            }
            "toggle" => {
                post(Msg::Mute);
                "ok"
            }
            "muted" => {
                if MUTED.load(Ordering::Relaxed) {
                    "1"
                } else {
                    "0"
                }
            }
            _ => "?",
        };
        let _ = (&stream).write_all(reply.as_bytes());
    }
}

fn init() {
    let cfg = Config::load();
    let mute_item = CheckMenuItem::with_id("mute", "Disable Animalese Typing", true, false, None);
    let startup_item = CheckMenuItem::with_id("startup", "Run on startup", true, cfg.startup_run, None);
    let menu = Menu::new();
    let _ = menu.append(&mute_item);
    let _ = menu.append(&MenuItem::with_id("settings", "Settings...", true, None));
    let _ = menu.append(&startup_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, None));
    MenuEvent::set_event_handler(Some(|e: MenuEvent| match e.id.0.as_str() {
        "mute" => post(Msg::Mute),
        "settings" => post(Msg::Settings),
        "startup" => post(Msg::Startup),
        "quit" => post(Msg::Quit),
        _ => {}
    }));
    TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            post(Msg::Settings);
        }
    }));
    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Animalese Typing")
        .with_icon(icon(assets::ICON).expect("tray icon"))
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            sys::log(&format!("creating the menu bar icon failed: {e}"));
            std::process::exit(1);
        }
    };

    let mut d = Daemon {
        cfg: cfg.clone(),
        player: Player::new(),
        hotkey: None,
        muted: false,
        filtered: false,
        front_pid: -1,
        tray,
        mute_item,
        startup_item,
        audio_timer: false,
    };
    d.apply_config(cfg);
    DAEMON.with(|s| *s.borrow_mut() = Some(d));

    std::thread::Builder::new().name("ipc".into()).spawn(serve_socket).ok();

    if !keytap_macos::has_access() {
        // Opens the "Input Monitoring" prompt; the tap starts by itself once it's allowed.
        keytap_macos::request_access();
    }
    // The tap lives for the whole process.
    std::mem::forget(keytap_macos::spawn(|e| {
        static FIRST: std::sync::Once = std::sync::Once::new();
        FIRST.call_once(|| sys::log("keyboard: first key received"));
        post(Msg::Key(e))
    }));
}

pub fn run() {
    let Some(mtm) = MainThreadMarker::new() else {
        sys::log("daemon must run on the main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // Menu-bar only: no Dock icon, no app menu.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Build the status item once the run loop is going.
    post(Msg::Init);
    app.run();
}
