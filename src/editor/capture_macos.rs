//! Key capture for the remapper on macOS: a keyboard tap that runs only while remapping, so keys
//! arrive as the same virtual-key codes and modifier state the daemon sees.

use crate::keytap_macos::{self, Tap};
use crate::player::Mods;
use eframe::egui;
use std::sync::{Mutex, OnceLock};

static TAP: Mutex<Option<Tap>> = Mutex::new(None);
static EVENTS: Mutex<Vec<(u32, Mods)>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn start(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
    let Ok(mut tap) = TAP.lock() else { return };
    if tap.is_none() {
        *tap = Some(keytap_macos::spawn(|e| {
            if !e.down {
                return;
            }
            let m = Mods { caps: false, ..e.mods };
            if let Ok(mut ev) = EVENTS.lock() {
                ev.push((e.vk, m));
            }
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        }));
    }
}

pub fn stop() {
    if let Some(tap) = TAP.lock().ok().and_then(|mut t| t.take()) {
        tap.stop();
    }
    if let Ok(mut ev) = EVENTS.lock() {
        ev.clear();
    }
}

pub fn drain() -> Vec<(u32, Mods)> {
    EVENTS.lock().map(|mut ev| std::mem::take(&mut *ev)).unwrap_or_default()
}
