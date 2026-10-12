//! Settings, stored as JSON in the same shape as the original Electron app's
//! `config.json`, so an existing config can be imported unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct VoiceProfile {
    #[serde(rename = "type")]
    pub kind: String,
    pub pitch: f64,
    pub variation: f64,
    pub intonation: f64,
}

impl Default for VoiceProfile {
    fn default() -> Self {
        Self { kind: "f1".into(), pitch: 0.0, variation: 0.2, intonation: 0.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct NoteProfile {
    pub instrument: String,
    pub transpose: f64,
}

impl Default for NoteProfile {
    fn default() -> Self {
        Self { instrument: "girl".into(), transpose: 0.0 }
    }
}

/// Per-key override of the default sounds. Missing fields fall back to the default map.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Remap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctrl_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_sound: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    /// UI language code (en, es, de, fr, ko, ja, zh_CN).
    pub lang: String,
    /// default, dark, pink, green
    pub theme: String,
    /// Editor sits above the tray and closes when it loses focus.
    pub anchor_window: bool,
    pub volume: f64,
    /// 0 default, 1 voice only, 2 sfx only, 3 randomized
    pub audio_mode: u8,
    pub disable_hotkey: Option<String>,
    pub startup_run: bool,
    pub hold_repeat: bool,
    pub always_active: bool,
    pub selected_apps: Vec<String>,
    /// true: only play in `selected_apps`; false: play everywhere except them
    pub selected_active: bool,
    pub voice_language: String,
    pub voice_profile: VoiceProfile,
    pub note_profile: NoteProfile,
    pub remapped_keys: HashMap<String, Remap>,
    /// Keys we don't use (saved voice profiles, ...) are kept as-is.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang: "en".into(),
            theme: "default".into(),
            anchor_window: true,
            volume: 0.5,
            audio_mode: 0,
            disable_hotkey: None,
            startup_run: false,
            hold_repeat: true,
            always_active: true,
            selected_apps: Vec::new(),
            selected_active: true,
            voice_language: "english".into(),
            voice_profile: VoiceProfile::default(),
            note_profile: NoteProfile::default(),
            remapped_keys: HashMap::new(),
            extra: Map::new(),
        }
    }
}

fn appdata() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn path() -> PathBuf {
    appdata().join("animalese-rs").join("config.json")
}

/// The Electron app's config, used once as a starting point.
fn original_path() -> PathBuf {
    appdata().join("Animalese Typing").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        if let Some(c) = read(&path()) {
            return c;
        }
        let c = read(&original_path()).unwrap_or_default();
        c.save();
        c
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_string_pretty(self) else { return };
        let tmp = p.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &p)) {
            crate::sys::log(&format!("saving {} failed: {e}", p.display()));
        }
    }
}

fn read(p: &PathBuf) -> Option<Config> {
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&text).ok()
}
