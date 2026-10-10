//! Key capture for the remapper: a low-level keyboard hook installed only while remapping, so
//! keys arrive as the same virtual-key codes and modifier state the daemon sees.

use crate::player::Mods;
use eframe::egui;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static HOOK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
static EVENTS: Mutex<Vec<(u32, Mods)>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let msg = wp as u32;
        if code == HC_ACTION as i32 && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) {
            let k = &*(lp as *const KBDLLHOOKSTRUCT);
            let pressed = |vk: VIRTUAL_KEY| GetAsyncKeyState(vk as i32) < 0;
            let m = Mods { shift: pressed(VK_SHIFT), ctrl: pressed(VK_CONTROL), alt: pressed(VK_MENU), caps: false };
            if let Ok(mut ev) = EVENTS.lock() {
                ev.push((k.vkCode, m));
            }
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}

pub fn start(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
    if HOOK.load(Ordering::Relaxed).is_null() {
        let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(std::ptr::null()), 0) };
        HOOK.store(h, Ordering::Relaxed);
    }
}

pub fn stop() {
    let h = HOOK.swap(null_mut(), Ordering::Relaxed);
    if !h.is_null() {
        unsafe { UnhookWindowsHookEx(h) };
    }
    if let Ok(mut ev) = EVENTS.lock() {
        ev.clear();
    }
}

pub fn drain() -> Vec<(u32, Mods)> {
    EVENTS.lock().map(|mut ev| std::mem::take(&mut *ev)).unwrap_or_default()
}
