//! Sound selection logic, ported from audio-manager.cjs / animalese.cjs / preload.cjs.

use crate::audio::{Engine, PlayCmd};
use crate::config::Config;
use crate::keymap;
use std::collections::HashSet;

const LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t",
    "u", "v", "w", "x", "y", "z",
];
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];

/// Sprite names in sfx.ogg, 600ms each, in file order.
pub const SFX: [&str; 26] = [
    "backspace", "enter", "tab", "question", "exclamation", "at", "pound", "dollar", "caret",
    "ampersand", "asterisk", "parenthesis_open", "parenthesis_closed", "bracket_open",
    "bracket_closed", "brace_open", "brace_closed", "tilde", "default", "arrow_left", "arrow_up",
    "arrow_right", "arrow_down", "slash_forward", "slash_back", "percent",
];

/// Sung syllables in the voiced instruments, 2000ms each.
const SING: [&str; 7] = ["nah", "me", "now", "way", "oh", "oh2", "me2"];
pub const SINGING_INSTRUMENTS: [&str; 4] = ["girl", "boy", "cranky", "kk_slider"];
pub const INSTRUMENTS: [&str; 9] =
    ["girl", "boy", "cranky", "kk_slider", "organ", "guitar", "e_piano", "synth", "whistle"];
pub const VOICE_TYPES: [&str; 8] = ["f1", "f2", "f3", "f4", "m1", "m2", "m3", "m4"];

/// Region of a voice clip in ms: letters, digits (200ms), then ok/gwah/deska (600ms).
fn voice_sprite(name: &str) -> Option<(f64, f64)> {
    if let Some(i) = LETTERS.iter().chain(DIGITS.iter()).position(|s| *s == name) {
        return Some((200.0 * i as f64, 200.0));
    }
    let i = ["ok", "gwah", "deska"].iter().position(|s| *s == name)?;
    Some((7200.0 + 600.0 * i as f64, 600.0))
}

#[derive(Clone, Copy)]
pub struct Opts {
    pub volume: f32,
    pub pitch_shift: f64,
    pub variation: f64,
    pub intonation: f64,
    pub note: i32,
    pub channel: Option<u32>,
    pub hold: Option<u32>,
    pub no_random: bool,
    pub yelling: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch_shift: 0.0,
            variation: 0.0,
            intonation: 0.0,
            note: 60,
            channel: None,
            hold: None,
            no_random: false,
            yelling: false,
        }
    }
}

/// What a key press should trigger besides sound.
pub enum Action {
    None,
    ShowSettings,
}

pub struct Player {
    pub engine: Engine,
    /// Keys whose sound is waiting for key-up (blocks auto-repeat).
    held: HashSet<u32>,
}

impl Player {
    pub fn new() -> Self {
        Self::with_engine(Engine::new())
    }

    pub fn with_engine(engine: Engine) -> Self {
        Self { engine, held: HashSet::new() }
    }

    /// Pre-decodes the clips the current settings use and frees the rest.
    pub fn prepare(&mut self, cfg: &Config) {
        let names = [
            "sfx".to_string(),
            format!("voice/{}/{}", cfg.voice_language, cfg.voice_profile.kind),
            format!("instrument/{}", cfg.note_profile.instrument),
        ];
        self.engine.set_working_set(&names);
    }

    pub fn play(&mut self, cfg: &Config, path: &str, mut o: Opts) {
        if path.is_empty() {
            return;
        }
        if let Some(h) = o.hold {
            if self.held.contains(&h) {
                return;
            }
        }
        let mode = cfg.audio_mode;
        if path == "&.gwah" && mode != 3 {
            self.play(cfg, "sfx.exclamation", Opts::default());
        }
        if path == "&.deska" && mode != 3 {
            self.play(cfg, "sfx.question", Opts::default());
        }

        let is_voice = path.starts_with('&');
        let is_instrument = path.starts_with('%');
        let is_sfx = path.starts_with("sfx");
        if path.starts_with('#') {
            return;
        }

        let mut path = path.to_string();
        if mode == 1 && is_sfx {
            path = "sfx.default".into();
        }
        if mode == 2 && is_voice {
            path = "sfx.default".into();
        }
        if mode == 3 && !o.no_random {
            if is_voice {
                path = format!("&.{}", LETTERS[fastrand::usize(..26)]);
            } else if is_instrument {
                path = format!("%.{}", fastrand::i32(36..72));
            } else if is_sfx {
                path = format!("sfx.{}", SFX[fastrand::usize(..SFX.len())]);
            }
        }

        let vp = &cfg.voice_profile;
        let np = &cfg.note_profile;
        if is_instrument {
            if let Some(n) = path.strip_prefix("%.").and_then(|n| n.parse().ok()) {
                o.note = n;
            }
            path = format!("inst.{}", np.instrument);
            o.pitch_shift += np.transpose;
        }
        if is_voice {
            o.volume = if o.yelling { 0.75 } else { 0.65 };
            o.pitch_shift = if o.yelling { 1.5 } else { 0.0 } + vp.pitch;
            o.variation = if o.yelling { 1.0 } else { 0.0 } + vp.variation;
            o.intonation = vp.intonation;
            o.channel = o.channel.or(Some(1));
            path = path.replacen('&', &vp.kind, 1);
        }

        // Resolve "bank.sprite" to a clip and a region in ms.
        let Some((bank, sprite)) = path.split_once('.') else { return };
        let (clip_name, region) = match bank {
            "sfx" => {
                let Some(i) = SFX.iter().position(|s| *s == sprite) else { return };
                ("sfx".to_string(), Some((600.0 * i as f64, 600.0)))
            }
            "inst" => {
                let region = SINGING_INSTRUMENTS
                    .contains(&sprite)
                    .then(|| (2000.0 * fastrand::usize(..SING.len()) as f64, 2000.0));
                (format!("instrument/{sprite}"), region)
            }
            v if VOICE_TYPES.contains(&v) => {
                let Some(r) = voice_sprite(sprite) else { return };
                (format!("voice/{}/{v}", cfg.voice_language), Some(r))
            }
            _ => return,
        };
        let Some(clip) = self.engine.clip(&clip_name) else { return };
        let ms = clip.rate as f64 / 1000.0;
        let (start, end) = match region {
            Some((s, d)) => ((s * ms) as usize, ((s + d) * ms) as usize),
            None => (0, clip.frames()),
        };

        let pitch = (o.note - 60) as f64 + o.pitch_shift + (fastrand::f64() * 2.0 - 1.0) * o.variation;
        let rate = 2f64.powf(pitch / 12.0);
        let i = o.intonation;
        let end_rate = rate
            * if i >= 0.0 { 1.0 + i * 3.0 } else { 1.0 - (((1.0 - i * 3.0).abs().sqrt() - 1.0) * 0.75) };

        self.engine.play(PlayCmd {
            clip,
            start,
            end,
            rate,
            end_rate,
            volume: (cfg.volume as f32) * o.volume,
            channel: o.channel,
            hold: o.hold,
        });
        if let Some(h) = o.hold {
            self.held.insert(h);
        }
    }

    pub fn release(&mut self, hold: u32, cut: bool) {
        if cut {
            self.engine.release(hold, 0.15);
        }
        self.held.remove(&hold);
    }

    pub fn key_down(&mut self, cfg: &Config, vk: u32, m: Mods) -> Action {
        let Some(sound) = final_sound(cfg, vk, m) else { return Action::None };
        if sound.is_empty() {
            return Action::None;
        }
        let mut o = Opts::default();
        if !cfg.hold_repeat {
            o.hold = Some(vk);
        }
        if sound.starts_with('#') {
            return if sound == "#show_window" { Action::ShowSettings } else { Action::None };
        } else if sound.starts_with('&') {
            o.yelling = m.caps != m.shift;
        } else if sound.starts_with('%') {
            o.hold = Some(vk);
            o.pitch_shift = if m.caps { -12.0 } else { 0.0 };
        }
        self.play(cfg, &sound, o);
        Action::None
    }

    pub fn key_up(&mut self, cfg: &Config, vk: u32, m: Mods) {
        if let Some(sound) = final_sound(cfg, vk, m) {
            self.release(vk, sound.starts_with('%'));
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps: bool,
}

/// Sound for a key with the given modifiers, honoring user remaps. None for unknown keys.
pub fn final_sound(cfg: &Config, vk: u32, m: Mods) -> Option<String> {
    let def = keymap::lookup(vk)?;
    let remap = cfg.remapped_keys.get(&vk.to_string());
    let pick = |r: Option<&Option<String>>, d: &str| r.and_then(|s| s.clone()).unwrap_or_else(|| d.to_string());
    Some(if m.ctrl {
        pick(remap.map(|r| &r.ctrl_sound), def.ctrl)
    } else if m.alt {
        pick(remap.map(|r| &r.alt_sound), "")
    } else if m.shift {
        pick(remap.map(|r| &r.shift_sound), def.shift_sound())
    } else {
        pick(remap.map(|r| &r.sound), def.sound)
    })
}

/// Sound for a key with the given modifiers from the built-in map, ignoring remaps.
pub fn default_sound(vk: u32, m: Mods) -> Option<String> {
    let def = keymap::lookup(vk)?;
    Some(if m.ctrl { def.ctrl } else if m.alt { "" } else if m.shift { def.shift_sound() } else { def.sound }.to_string())
}

/// A parsed disable hotkey such as "CommandOrControl+Alt+M".
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hotkey {
    pub vk: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey { vk: 0, ctrl: false, alt: false, shift: false };
        for part in s.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "commandorcontrol" | "control" | "ctrl" | "cmdorctrl" => hk.ctrl = true,
                "alt" | "option" => hk.alt = true,
                "shift" => hk.shift = true,
                _ => hk.vk = keymap::vk_for_name(part)?,
            }
        }
        (hk.vk != 0).then_some(hk)
    }

    pub fn from_key(vk: u32, m: Mods) -> Hotkey {
        Hotkey { vk, ctrl: m.ctrl, alt: m.alt, shift: m.shift }
    }

    pub fn format(&self) -> String {
        let name = keymap::lookup(self.vk).map_or("?", |d| d.key);
        format!(
            "{}{}{}{}",
            if self.ctrl { "CommandOrControl+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            name
        )
    }

    pub fn matches(&self, vk: u32, m: Mods) -> bool {
        self.vk == vk && self.ctrl == m.ctrl && self.alt == m.alt && self.shift == m.shift
    }
}
