//! The always-running part: low-level keyboard hook, foreground-app tracking, tray icon and
//! the audio engine, all driven by one Win32 message loop on the main thread.
//!
//! The hook callback only packs the key into a message and posts it; the sound is chosen and
//! queued when the loop handles that message, so the hook never blocks typing.

use crate::assets;
use crate::config::Config;
use crate::player::{Action, Hotkey, Mods, Player};
use crate::sys::{self, WM_APP_GET_MUTED, WM_APP_RELOAD, WM_APP_TOGGLE_MUTE, wide};
use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_APP_KEY: u32 = WM_APP + 2;
const WM_APP_FOCUS: u32 = WM_APP + 3;
const WM_APP_MENU: u32 = WM_APP + 4;
const TIMER_AUDIO: usize = 1;

const MENU_MUTE: usize = 1;
const MENU_SETTINGS: usize = 2;
const MENU_STARTUP: usize = 3;
const MENU_QUIT: usize = 4;

static HWND_MAIN: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());

struct Daemon {
    cfg: Config,
    player: Player,
    hotkey: Option<Hotkey>,
    /// Disabled from the tray / hotkey.
    muted: bool,
    /// Disabled because of the app filter.
    filtered: bool,
    caps: bool,
    caps_down: bool,
    own_app: String,
    tray: TrayIcon,
    mute_item: CheckMenuItem,
    startup_item: CheckMenuItem,
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

impl Daemon {
    fn apply_config(&mut self, cfg: Config) {
        self.hotkey = cfg.disable_hotkey.as_deref().and_then(Hotkey::parse);
        self.player.prepare(&cfg);
        self.startup_item.set_checked(cfg.startup_run);
        self.cfg = cfg;
        self.update_filter(unsafe { GetForegroundWindow() });
    }

    fn update_filter(&mut self, hwnd: HWND) {
        let app = sys::window_app(hwnd).unwrap_or_default();
        self.filtered = if self.cfg.always_active || app.is_empty() || app == self.own_app {
            false
        } else {
            let listed = self.cfg.selected_apps.iter().any(|a| a.eq_ignore_ascii_case(&app));
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
        self.update_tray();
    }

    fn on_key(&mut self, hwnd: HWND, packed: usize) {
        let vk = (packed & 0xFFFF) as u32;
        let down = packed & (1 << 16) != 0;
        let mut m = Mods {
            shift: packed & (1 << 17) != 0,
            ctrl: packed & (1 << 18) != 0,
            alt: packed & (1 << 19) != 0,
            caps: false,
        };
        if vk == VK_CAPITAL as u32 {
            if down && !self.caps_down {
                self.caps = !self.caps;
            }
            self.caps_down = down;
        }
        m.caps = self.caps;

        if down && self.hotkey.is_some_and(|h| h.matches(vk, m)) {
            self.toggle_muted();
            return;
        }
        if self.muted || self.filtered {
            return;
        }
        if down {
            if let Action::ShowSettings = self.player.key_down(&self.cfg, vk, m) {
                sys::spawn_settings();
            }
            if self.player.engine.is_running() {
                unsafe { SetTimer(hwnd, TIMER_AUDIO, 1000, None) };
            }
        } else {
            self.player.key_up(&self.cfg, vk, m);
        }
    }

    fn on_menu(&mut self, id: usize) {
        match id {
            MENU_MUTE => self.toggle_muted(),
            MENU_SETTINGS => sys::spawn_settings(),
            MENU_STARTUP => {
                self.cfg.startup_run = !self.cfg.startup_run;
                sys::set_autostart(self.cfg.startup_run);
                self.cfg.save();
                self.startup_item.set_checked(self.cfg.startup_run);
            }
            MENU_QUIT => unsafe { PostQuitMessage(0) },
            _ => {}
        }
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let k = &*(lp as *const KBDLLHOOKSTRUCT);
            let msg = wp as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            if down || msg == WM_KEYUP || msg == WM_SYSKEYUP {
                let pressed = |vk: VIRTUAL_KEY| GetAsyncKeyState(vk as i32) < 0;
                let packed = (k.vkCode as usize & 0xFFFF)
                    | (down as usize) << 16
                    | (pressed(VK_SHIFT) as usize) << 17
                    | (pressed(VK_CONTROL) as usize) << 18
                    | (pressed(VK_MENU) as usize) << 19;
                PostMessageW(HWND_MAIN.load(Ordering::Relaxed), WM_APP_KEY, packed, 0);
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}

unsafe extern "system" fn foreground_hook(
    _: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    unsafe { PostMessageW(HWND_MAIN.load(Ordering::Relaxed), WM_APP_FOCUS, 0, hwnd as LPARAM) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_KEY => with_daemon(|d| d.on_key(hwnd, wp)),
        WM_APP_FOCUS => with_daemon(|d| d.update_filter(lp as HWND)),
        WM_APP_MENU => with_daemon(|d| d.on_menu(wp)),
        WM_APP_RELOAD => with_daemon(|d| d.apply_config(Config::load())),
        WM_APP_TOGGLE_MUTE => with_daemon(|d| d.toggle_muted()),
        WM_APP_GET_MUTED => {
            let mut muted = 0;
            with_daemon(|d| muted = d.muted as LRESULT);
            return muted;
        }
        WM_TIMER if wp == TIMER_AUDIO => with_daemon(|d| {
            d.player.engine.tick();
            if !d.player.engine.is_running() {
                unsafe { KillTimer(hwnd, TIMER_AUDIO) };
            }
        }),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    0
}

fn post_menu(id: usize) {
    unsafe { PostMessageW(HWND_MAIN.load(Ordering::Relaxed), WM_APP_MENU, id, 0) };
}

pub fn run() {
    let cfg = Config::load();
    if cfg.startup_run {
        sys::set_autostart(true); // keep the registry path current if the exe moved
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide(sys::DAEMON_CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            sys::log(&format!("RegisterClassW failed: {}", GetLastError()));
        }
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0,
            HWND_MESSAGE, null_mut(), hinst, std::ptr::null(),
        );
        if hwnd.is_null() {
            sys::log(&format!("CreateWindowExW failed: {}", GetLastError()));
            return;
        }
        HWND_MAIN.store(hwnd, Ordering::Relaxed);

        let mute_item = CheckMenuItem::with_id("mute", "Disable Animalese Typing", true, false, None);
        let startup_item = CheckMenuItem::with_id("startup", "Run on startup", true, cfg.startup_run, None);
        let menu = Menu::new();
        let _ = menu.append(&mute_item);
        let _ = menu.append(&MenuItem::with_id("settings", "Settings...", true, None));
        let _ = menu.append(&startup_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, None));
        MenuEvent::set_event_handler(Some(|e: MenuEvent| {
            post_menu(match e.id.0.as_str() {
                "mute" => MENU_MUTE,
                "settings" => MENU_SETTINGS,
                "startup" => MENU_STARTUP,
                "quit" => MENU_QUIT,
                _ => 0,
            })
        }));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                post_menu(MENU_SETTINGS);
            }
        }));
        let Ok(tray) = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Animalese Typing")
            .with_icon(icon(assets::ICON).expect("tray icon"))
            .build()
        else {
            return;
        };

        let mut d = Daemon {
            cfg: cfg.clone(),
            player: Player::new(),
            hotkey: None,
            muted: false,
            filtered: false,
            caps: GetKeyState(VK_CAPITAL as i32) & 1 != 0,
            caps_down: false,
            own_app: sys::own_app(),
            tray,
            mute_item,
            startup_item,
        };
        d.apply_config(cfg);
        DAEMON.with(|s| *s.borrow_mut() = Some(d));

        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0);
        let fg = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, null_mut(),
            Some(foreground_hook), 0, 0, WINEVENT_OUTOFCONTEXT,
        );

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnhookWindowsHookEx(kb);
        windows_sys::Win32::UI::Accessibility::UnhookWinEvent(fg);
        DAEMON.with(|s| s.borrow_mut().take());
    }
}
