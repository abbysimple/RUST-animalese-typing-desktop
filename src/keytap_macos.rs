//! Global keyboard listener for macOS: a listen-only `CGEventTap` on its own thread and run loop.
//!
//! macOS key codes are translated to the Windows virtual-key codes the rest of the app (key map,
//! remaps, hotkeys, saved config) is built around, so nothing above this layer changes.
//!
//! A listen-only tap needs the "Input Monitoring" permission. Until it's granted the tap can't be
//! created, so the thread keeps retrying and starts working as soon as the user allows it.

use crate::player::Mods;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::Duration;

type CFTypeRef = *const c_void;
type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFStringRef = *const c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventKeyboardGetUnicodeString(event: CGEventRef, max: usize, actual: *mut usize, buf: *mut u16);
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: CFTypeRef);
}

const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const DISABLED_BY_USER: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;

const FLAG_CAPS: u64 = 0x1_0000;
const FLAG_SHIFT: u64 = 0x2_0000;
const FLAG_CTRL: u64 = 0x4_0000;
const FLAG_ALT: u64 = 0x8_0000;
const FLAG_CMD: u64 = 0x10_0000;

pub struct KeyEvent {
    /// Windows virtual-key code.
    pub vk: u32,
    pub down: bool,
    /// `ctrl` is set for Control *or* Command, so Mac shortcuts (⌘C, ⌘V...) get the same
    /// sounds as their Ctrl counterparts on Windows. `caps` is the real Caps Lock state.
    pub mods: Mods,
}

/// True once the user has allowed Input Monitoring for this app.
pub fn has_access() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

/// Shows the system "allow Input Monitoring" prompt (only the first time it's asked).
pub fn request_access() -> bool {
    unsafe { CGRequestListenEventAccess() }
}

/// A running tap. Dropping it does not stop it; call [`Tap::stop`].
pub struct Tap {
    stop: Arc<AtomicBool>,
    run_loop: Arc<AtomicPtr<c_void>>,
}

impl Tap {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let rl = self.run_loop.load(Ordering::SeqCst);
        if !rl.is_null() {
            unsafe { CFRunLoopStop(rl) };
        }
    }
}

struct State {
    tap: CFMachPortRef,
    handler: Box<dyn FnMut(KeyEvent)>,
    /// The vk each physical key went down as, so its key-up matches even if a modifier changed.
    down_vk: [u32; 128],
}

pub fn spawn(handler: impl FnMut(KeyEvent) + Send + 'static) -> Tap {
    let stop = Arc::new(AtomicBool::new(false));
    let run_loop = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
    let tap = Tap { stop: stop.clone(), run_loop: run_loop.clone() };
    let handler: Box<dyn FnMut(KeyEvent) + Send> = Box::new(handler);
    std::thread::Builder::new()
        .name("keytap".into())
        .spawn(move || run(handler, stop, run_loop))
        .expect("spawn keytap thread");
    tap
}

fn run(handler: Box<dyn FnMut(KeyEvent) + Send>, stop: Arc<AtomicBool>, run_loop: Arc<AtomicPtr<c_void>>) {
    let state = Box::into_raw(Box::new(State { tap: std::ptr::null_mut(), handler, down_vk: [0; 128] }));
    let mask = (1u64 << KEY_DOWN) | (1u64 << KEY_UP) | (1u64 << FLAGS_CHANGED);
    let mut logged = false;
    unsafe {
        let tap = loop {
            if stop.load(Ordering::SeqCst) {
                drop(Box::from_raw(state));
                return;
            }
            // A tap created before Input Monitoring is allowed can be handed out but never gets
            // events, so don't create one until the permission is really there.
            if has_access() {
                let tap = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, mask, callback, state.cast());
                if !tap.is_null() {
                    if logged {
                        crate::sys::log("keyboard tap started");
                    }
                    break tap;
                }
            }
            if !logged {
                crate::sys::log("keyboard tap waiting for Input Monitoring permission");
                logged = true;
            }
            std::thread::sleep(Duration::from_secs(2));
        };
        (*state).tap = tap;
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        let rl = CFRunLoopGetCurrent();
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        run_loop.store(rl, Ordering::SeqCst);
        // Short slices so a stop() that lands before the loop is running is still noticed.
        while !stop.load(Ordering::SeqCst) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0, false);
        }
        run_loop.store(std::ptr::null_mut(), Ordering::SeqCst);
        CGEventTapEnable(tap, false);
        CFRunLoopRemoveSource(rl, source, kCFRunLoopCommonModes);
        CFRelease(source);
        CFMachPortInvalidate(tap);
        CFRelease(tap);
        drop(Box::from_raw(state));
    }
}

extern "C" fn callback(_proxy: *mut c_void, ty: u32, event: CGEventRef, user: *mut c_void) -> CGEventRef {
    // Never unwind across the FFI boundary.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let st = &mut *(user as *mut State);
        match ty {
            DISABLED_BY_TIMEOUT | DISABLED_BY_USER => CGEventTapEnable(st.tap, true),
            KEY_DOWN | KEY_UP | FLAGS_CHANGED => handle(st, ty, event),
            _ => {}
        }
    }));
    event
}

unsafe fn handle(st: &mut State, ty: u32, event: CGEventRef) {
    let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) } as usize;
    let flags = unsafe { CGEventGetFlags(event) };
    let mods = Mods {
        shift: flags & FLAG_SHIFT != 0,
        ctrl: flags & (FLAG_CTRL | FLAG_CMD) != 0,
        alt: flags & FLAG_ALT != 0,
        caps: flags & FLAG_CAPS != 0,
    };
    let slot = code & 0x7F;
    match ty {
        KEY_DOWN => {
            let vk = letter_vk(event).or_else(|| mac_to_vk(code)).unwrap_or(0);
            if vk != 0 {
                st.down_vk[slot] = vk;
                (st.handler)(KeyEvent { vk, down: true, mods });
            }
        }
        KEY_UP => {
            let vk = match std::mem::take(&mut st.down_vk[slot]) {
                0 => mac_to_vk(code).unwrap_or(0),
                vk => vk,
            };
            if vk != 0 {
                (st.handler)(KeyEvent { vk, down: false, mods });
            }
        }
        _ => {
            // Modifier keys only report "flags changed"; work out press vs release from the
            // device-specific bit for that exact key.
            let Some(vk) = mac_to_vk(code) else { return };
            if code == 0x39 {
                // Caps Lock reports once per toggle: treat it as a tap.
                (st.handler)(KeyEvent { vk, down: true, mods });
                (st.handler)(KeyEvent { vk, down: false, mods });
                return;
            }
            let bit = match code {
                0x38 => 0x2,    // left shift
                0x3C => 0x4,    // right shift
                0x3B => 0x1,    // left control
                0x3E => 0x2000, // right control
                0x3A => 0x20,   // left option
                0x3D => 0x40,   // right option
                0x37 => 0x8,    // left command
                0x36 => 0x10,   // right command
                _ => return,
            };
            (st.handler)(KeyEvent { vk, down: flags & bit != 0, mods });
        }
    }
}

/// Letters follow the active keyboard layout (AZERTY, Dvorak...) like Windows VK codes do.
fn letter_vk(event: CGEventRef) -> Option<u32> {
    let mut buf = [0u16; 4];
    let mut len = 0usize;
    unsafe { CGEventKeyboardGetUnicodeString(event, buf.len(), &mut len, buf.as_mut_ptr()) };
    if len != 1 {
        return None;
    }
    let c = char::from_u32(buf[0] as u32)?;
    c.is_ascii_alphabetic().then(|| c.to_ascii_uppercase() as u32)
}

/// Physical macOS key code (`kVK_*`) → Windows virtual-key code.
fn mac_to_vk(code: usize) -> Option<u32> {
    let vk = match code {
        0x00 => 65, 0x01 => 83, 0x02 => 68, 0x03 => 70, 0x04 => 72, 0x05 => 71, 0x06 => 90,
        0x07 => 88, 0x08 => 67, 0x09 => 86, 0x0A => 192, 0x0B => 66, 0x0C => 81, 0x0D => 87,
        0x0E => 69, 0x0F => 82, 0x10 => 89, 0x11 => 84, 0x12 => 49, 0x13 => 50, 0x14 => 51,
        0x15 => 52, 0x16 => 54, 0x17 => 53, 0x18 => 187, 0x19 => 57, 0x1A => 55, 0x1B => 189,
        0x1C => 56, 0x1D => 48, 0x1E => 221, 0x1F => 79, 0x20 => 85, 0x21 => 219, 0x22 => 73,
        0x23 => 80, 0x24 => 13, 0x25 => 76, 0x26 => 74, 0x27 => 222, 0x28 => 75, 0x29 => 186,
        0x2A => 220, 0x2B => 188, 0x2C => 191, 0x2D => 78, 0x2E => 77, 0x2F => 190,
        0x30 => 9,   // tab
        0x31 => 32,  // space
        0x32 => 192, // `
        0x33 => 8,   // delete (backspace)
        0x35 => 27,  // escape
        0x36 => 92,  // right command
        0x37 => 91,  // command
        0x38 => 160, // shift
        0x39 => 20,  // caps lock
        0x3A => 164, // option
        0x3B => 162, // control
        0x3C => 161, // right shift
        0x3D => 165, // right option
        0x3E => 163, // right control
        0x41 => 110, // keypad .
        0x43 => 106, // keypad *
        0x45 => 107, // keypad +
        0x47 => 144, // keypad clear (num lock position)
        0x48 => 175, // volume up
        0x49 => 174, // volume down
        0x4A => 173, // mute
        0x4B => 111, // keypad /
        0x4C => 13,  // keypad enter
        0x4E => 109, // keypad -
        0x51 => 187, // keypad =
        0x52 => 96, 0x53 => 97, 0x54 => 98, 0x55 => 99, 0x56 => 100, 0x57 => 101,
        0x58 => 102, 0x59 => 103, 0x5B => 104, 0x5C => 105,
        0x60 => 116, 0x61 => 117, 0x62 => 118, 0x63 => 114, 0x64 => 119, 0x65 => 120,
        0x67 => 122, 0x69 => 124, 0x6A => 127, 0x6B => 125, 0x6D => 121,
        0x6E => 93,  // context menu
        0x6F => 123, 0x71 => 126,
        0x72 => 45,  // help / insert
        0x73 => 36,  // home
        0x74 => 33,  // page up
        0x75 => 46,  // forward delete
        0x76 => 115, // F4
        0x77 => 35,  // end
        0x78 => 113, // F2
        0x79 => 34,  // page down
        0x7A => 112, // F1
        0x7B => 37, 0x7C => 39, 0x7D => 40, 0x7E => 38, // arrows
        _ => return None,
    };
    Some(vk)
}
