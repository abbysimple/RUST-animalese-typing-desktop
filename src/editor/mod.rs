//! The editor window (`--settings`), modelled on the original app's editor.html: sound board
//! with tabs, key remapper, voice / instrument editors, master volume and a settings overlay.
//!
//! Runs as its own short-lived process so the GUI toolkit never sits in the background daemon.
//! Every change is saved to the config file and the daemon is told to reload.

mod capture;
mod i18n;
mod icons;
mod theme;

use crate::assets;
use crate::config::{Config, Remap};
use crate::keymap;
use crate::player::{self, Hotkey, INSTRUMENTS, Mods, Opts, Player};
use crate::sys;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Order, Pos2, Rect, Sense, Stroke,
    StrokeKind, UiBuilder, Vec2, pos2, vec2,
};
use serde::Deserialize;
use std::time::{Duration, Instant};
use theme::{Cjk, Pal};

const W: f32 = 720.0;
const H: f32 = 345.0;
const TITLE_H: f32 = 28.0;
const SAVE_DELAY: Duration = Duration::from_millis(250);
const GITHUB: &str = "https://github.com/joshxviii/animalese-typing-desktop";

pub fn run() {
    if !sys::single_instance("Local\\animalese-rs-settings") {
        sys::focus_settings();
        return;
    }
    let cfg = Config::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(sys::SETTINGS_TITLE)
        .with_inner_size([W, H])
        .with_resizable(false)
        .with_decorations(false)
        .with_taskbar(!cfg.anchor_window);
    if cfg.anchor_window {
        if let Some((x, y)) = sys::work_area_corner() {
            viewport = viewport.with_position([x - W - 10.0, y - H - 10.0]);
        }
    }
    if let Some((rgba, width, height)) = assets::png_rgba(assets::ICON_BIG) {
        viewport = viewport.with_icon(egui::IconData { rgba, width, height });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    if let Err(e) = eframe::run_native(
        sys::SETTINGS_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(Editor::new(&cc.egui_ctx, cfg)))),
    ) {
        sys::log(&format!("editor: {e}"));
    }
    capture::stop();
}

#[derive(Deserialize, Clone, Default)]
struct Btn {
    label: Option<String>,
    #[serde(rename = "btnType")]
    btn_type: Option<String>,
    sound: Option<String>,
    icon: Option<String>,
}

fn rows(json: &str) -> Vec<Vec<Btn>> {
    serde_json::from_str(json).unwrap_or_default()
}

struct Layouts {
    special: Vec<Vec<Btn>>,
    english: Vec<Vec<Btn>>,
    korean: Vec<Vec<Btn>>,
    sfx: Vec<Vec<Btn>>,
    piano: Vec<Btn>,
}

impl Layouts {
    fn load() -> Self {
        Self {
            special: rows(include_str!("../../assets/layouts/special.json")),
            english: rows(include_str!("../../assets/layouts/voice/english.json")),
            korean: rows(include_str!("../../assets/layouts/voice/korean.json")),
            sfx: rows(include_str!("../../assets/layouts/sfx.json")),
            piano: serde_json::from_str(include_str!("../../assets/layouts/piano.json")).unwrap_or_default(),
        }
    }
}

/// The key being remapped.
struct KeyInfo {
    vk: u32,
    key: &'static str,
    mods: Mods,
    final_sound: String,
    default_sound: String,
}

impl KeyInfo {
    fn hotkey(&self) -> String {
        Hotkey::from_key(self.vk, self.mods).format()
    }

    fn label(&self) -> String {
        let k = self.key;
        let s = if ["Ctrl", "Alt", "Shift"].contains(&k) {
            k.to_string()
        } else if self.mods.ctrl {
            format!("Ctrl + {k}")
        } else if self.mods.alt {
            format!("Alt + {k}")
        } else if self.mods.shift {
            format!("Shift + {k}")
        } else {
            k.to_string()
        };
        s.to_uppercase()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Field {
    Pitch,
    Intonation,
    Variation,
    Transpose,
    Volume,
}

impl Field {
    fn range(self) -> (f64, f64) {
        match self {
            Field::Pitch | Field::Transpose => (-12.0, 12.0),
            Field::Intonation => (-1.0, 1.0),
            Field::Variation => (0.0, 12.0),
            Field::Volume => (0.0, 100.0),
        }
    }
    fn step(self) -> f64 {
        match self {
            Field::Intonation => 0.01,
            Field::Pitch | Field::Variation => 0.1,
            Field::Transpose | Field::Volume => 1.0,
        }
    }
    /// Double-click value, as in the original's `defaultValue` attributes.
    fn default(self) -> f64 {
        match self {
            Field::Variation => 0.5,
            Field::Volume => 50.0,
            _ => 0.0,
        }
    }
    fn get(self, c: &Config) -> f64 {
        match self {
            Field::Pitch => c.voice_profile.pitch,
            Field::Intonation => c.voice_profile.intonation,
            Field::Variation => c.voice_profile.variation,
            Field::Transpose => c.note_profile.transpose,
            Field::Volume => (c.volume * 100.0).round(),
        }
    }
    fn set(self, c: &mut Config, v: f64) {
        let (lo, hi) = self.range();
        let v = ((v.clamp(lo, hi) / self.step()).round() * self.step() * 1000.0).round() / 1000.0;
        match self {
            Field::Pitch => c.voice_profile.pitch = v,
            Field::Intonation => c.voice_profile.intonation = v,
            Field::Variation => c.voice_profile.variation = v,
            Field::Transpose => c.note_profile.transpose = v,
            Field::Volume => c.volume = v / 100.0,
        }
    }
    fn format(self, v: f64) -> String {
        let sign = if v > 0.0 { "+" } else { "" };
        match self {
            Field::Volume => format!("{v:.0}%"),
            Field::Transpose => format!("{sign}{}", v as i32),
            _ => format!("{sign}{v:.1}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Settings,
    Bindings,
}

#[derive(Clone, Copy, PartialEq)]
enum Popup {
    None,
    VoiceType,
    Instrument,
}

struct Editor {
    cfg: Config,
    saved: Config,
    changed_at: Option<Instant>,
    player: Option<Player>,
    pal: Pal,
    tr: i18n::Tr,
    tr_lang: String,
    theme_name: String,
    cjk: Cjk,
    icons: icons::Icons,
    layouts: Layouts,
    tab: usize,
    /// Sound id → time its key cap stops looking pressed.
    pressed: Vec<(String, Instant)>,
    last_hover_cap: Option<String>,
    piano_scroll: f32,
    piano_centered: bool,
    piano_held: Option<String>,
    monitoring: bool,
    remap: Option<KeyInfo>,
    highlight: Option<String>,
    overlay: Overlay,
    popup: Popup,
    popup_anchor: Rect,
    editing: Option<Field>,
    edit_text: String,
    edit_focus: bool,
    muted: Option<bool>,
    last_poll: Instant,
    was_focused: bool,
    running_apps: Vec<String>,
    reset_started: Option<Instant>,
    last_volume_blip: Instant,
}

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

/// Mouse wheel movement this frame in points (lines count as 40).
fn wheel(ui: &egui::Ui) -> Vec2 {
    ui.input(|i| {
        i.raw.events.iter().fold(Vec2::ZERO, |acc, e| match e {
            egui::Event::MouseWheel { unit, delta, .. } => {
                acc + *delta * if *unit == egui::MouseWheelUnit::Point { 1.0 } else { 40.0 }
            }
            _ => acc,
        })
    })
}

fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

fn instrument_label(id: &str) -> &str {
    match id {
        "girl" => "Girl",
        "boy" => "Boy",
        "cranky" => "Cranky",
        "kk_slider" => "K.K. Slider",
        "organ" => "Organ",
        "guitar" => "Guitar",
        "e_piano" => "E. Piano",
        "synth" => "Synth",
        "whistle" => "Whistle",
        other => other,
    }
}

/// Which tab shows a given sound id.
fn tab_for(sound: &str) -> usize {
    if sound.is_empty() || sound.starts_with('#') {
        0
    } else if sound.starts_with('&') {
        1
    } else if sound.starts_with('%') {
        2
    } else if sound.starts_with("sfx") {
        3
    } else {
        0
    }
}

impl Editor {
    fn new(ctx: &egui::Context, cfg: Config) -> Self {
        let pal = theme::palette(&cfg.theme);
        theme::apply_style(ctx, &pal);
        let cjk = Self::cjk_for(&cfg);
        theme::set_fonts(ctx, cjk);
        let mut ed = Self {
            tr: i18n::Tr::load(&cfg.lang),
            tr_lang: cfg.lang.clone(),
            theme_name: cfg.theme.clone(),
            saved: cfg.clone(),
            cfg,
            changed_at: None,
            player: None,
            pal,
            cjk,
            icons: icons::Icons::default(),
            layouts: Layouts::load(),
            tab: 1,
            pressed: Vec::new(),
            last_hover_cap: None,
            piano_scroll: 0.0,
            piano_centered: false,
            piano_held: None,
            monitoring: false,
            remap: None,
            highlight: None,
            overlay: Overlay::None,
            popup: Popup::None,
            popup_anchor: Rect::NOTHING,
            editing: None,
            edit_text: String::new(),
            edit_focus: false,
            muted: sys::daemon_muted(),
            last_poll: Instant::now(),
            was_focused: false,
            running_apps: Vec::new(),
            reset_started: None,
            last_volume_blip: Instant::now(),
        };
        // Dev aid: ANIMALESE_UI=special|piano|sfx|settings|bindings opens on that view.
        match std::env::var("ANIMALESE_UI").as_deref() {
            Ok("special") => ed.tab = 0,
            Ok("piano") => ed.tab = 2,
            Ok("sfx") => ed.tab = 3,
            Ok("settings") => {
                ed.running_apps = sys::running_apps();
                ed.overlay = Overlay::Settings;
            }
            Ok("bindings") => ed.overlay = Overlay::Bindings,
            _ => {}
        }
        ed
    }

    fn cjk_for(cfg: &Config) -> Cjk {
        Cjk {
            korean: cfg.voice_language == "korean" || cfg.lang == "ko",
            japanese: cfg.lang == "ja",
            chinese: cfg.lang == "zh_CN",
        }
    }

    fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.tr.t(key)
    }

    // ---------------------------------------------------------------- sound

    fn play(&mut self, sound: &str, o: Opts) {
        let player = self.player.get_or_insert_with(Player::new);
        player.play(&self.cfg, sound, Opts { no_random: true, ..o });
    }

    /// Small UI click, like the original's `{ channel: 2, volume: 0.55 }` sounds.
    fn blip(&mut self, sound: &str) {
        self.play(sound, Opts { channel: Some(2), volume: 0.55, ..Opts::default() });
    }

    fn release_piano(&mut self) {
        if self.piano_held.take().is_some() {
            if let Some(p) = self.player.as_mut() {
                p.release(0, true);
            }
        }
    }

    fn press_sound(&mut self, sound: &str, hold: bool) {
        if hold {
            self.release_piano();
            self.play(sound, Opts { hold: Some(0), ..Opts::default() });
            self.piano_held = Some(sound.to_string());
        } else {
            self.play(sound, Opts::default());
            self.pressed.retain(|(s, _)| s != sound);
            self.pressed.push((sound.to_string(), Instant::now() + Duration::from_millis(100)));
        }
        self.remap_apply(sound);
    }

    fn change_tab(&mut self, tab: usize) {
        if tab == 2 && self.tab != 2 {
            self.piano_centered = false;
        }
        self.tab = tab;
    }

    // ---------------------------------------------------------------- remapper (port of editor.cjs)

    fn remap_start(&mut self, vk: u32, mods: Mods) {
        let Some(def) = keymap::lookup(vk) else { return };
        let info = KeyInfo {
            vk,
            key: def.key,
            mods,
            final_sound: player::final_sound(&self.cfg, vk, mods).unwrap_or_default(),
            default_sound: player::default_sound(vk, mods).unwrap_or_default(),
        };
        let is_hotkey = self.cfg.disable_hotkey.as_deref() == Some(info.hotkey().as_str());
        let shown = if is_hotkey {
            "#disable_toggle".to_string()
        } else if info.final_sound.is_empty() {
            "#no_sound".to_string()
        } else {
            info.final_sound.clone()
        };
        self.change_tab(if is_hotkey { 0 } else { tab_for(&info.final_sound) });
        self.highlight = Some(shown);
        self.remap = Some(info);
        self.piano_centered = false;
    }

    fn remap_apply(&mut self, sound: &str) {
        let Some(info) = self.remap.as_ref() else { return };
        let reset = sound == info.default_sound;
        let hotkey = info.hotkey();
        let is_hotkey = self.cfg.disable_hotkey.as_deref() == Some(hotkey.as_str());
        let (vk, m) = (info.vk, info.mods);

        self.highlight = Some(if sound.is_empty() { "#no_sound".into() } else { sound.to_string() });
        self.change_tab(tab_for(sound));

        if sound == "#disable_toggle" {
            self.cfg.disable_hotkey = Some(hotkey);
            return;
        }
        if sound == "#no_sound" && is_hotkey {
            self.cfg.disable_hotkey = None;
            return;
        }
        let key = vk.to_string();
        let mut mapping = self.cfg.remapped_keys.get(&key).cloned().unwrap_or_default();
        let slot = if m.ctrl {
            &mut mapping.ctrl_sound
        } else if m.alt {
            &mut mapping.alt_sound
        } else if m.shift {
            &mut mapping.shift_sound
        } else {
            &mut mapping.sound
        };
        if reset {
            *slot = None;
            if is_hotkey {
                self.cfg.disable_hotkey = None;
            }
        } else {
            *slot = Some(sound.to_string());
        }
        if mapping == Remap::default() {
            self.cfg.remapped_keys.remove(&key);
        } else {
            self.cfg.remapped_keys.insert(key, mapping);
        }
    }

    fn remap_stop(&mut self) {
        self.remap = None;
        self.monitoring = false;
        self.highlight = None;
        capture::stop();
    }

    // ---------------------------------------------------------------- per-frame housekeeping

    fn housekeeping(&mut self, ctx: &egui::Context) {
        if self.cfg.lang != self.tr_lang {
            self.tr = i18n::Tr::load(&self.cfg.lang);
            self.tr_lang = self.cfg.lang.clone();
        }
        if self.cfg.theme != self.theme_name {
            self.pal = theme::palette(&self.cfg.theme);
            theme::apply_style(ctx, &self.pal);
            self.theme_name = self.cfg.theme.clone();
        }
        let cjk = Self::cjk_for(&self.cfg);
        if cjk != self.cjk {
            theme::set_fonts(ctx, cjk);
            self.cjk = cjk;
        }

        if self.monitoring || self.remap.is_some() {
            for (vk, m) in capture::drain() {
                self.remap_start(vk, m);
            }
        }

        if self.last_poll.elapsed() > Duration::from_millis(500) {
            self.muted = sys::daemon_muted();
            self.last_poll = Instant::now();
        }

        let focused = ctx.input(|i| i.viewport().focused);
        if focused == Some(true) {
            self.was_focused = true;
        }
        if self.cfg.anchor_window && self.was_focused && focused == Some(false) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.cfg != self.saved {
            let at = *self.changed_at.get_or_insert_with(Instant::now);
            if at.elapsed() >= SAVE_DELAY {
                self.save();
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }

        let now = Instant::now();
        self.pressed.retain(|(_, until)| *until > now);
        if let Some(next) = self.pressed.iter().map(|(_, u)| *u).min() {
            ctx.request_repaint_after(next - now);
        }
        if let Some(p) = self.player.as_mut() {
            p.engine.tick();
            if p.engine.is_running() {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn save(&mut self) {
        self.cfg.save();
        sys::notify_daemon();
        self.saved = self.cfg.clone();
        self.changed_at = None;
    }

    // ---------------------------------------------------------------- widgets

    /// An SVG icon that grows on hover and shrinks while pressed, like the original's svg-button.
    fn icon_button(&mut self, ui: &mut egui::Ui, rect: Rect, id: &str, icon: &str, alt1: Color32, alt2: Color32) -> egui::Response {
        let resp = ui.interact(rect, Id::new(("icon", id)), Sense::click());
        let scale = if resp.is_pointer_button_down_on() {
            0.9
        } else if resp.hovered() {
            1.07
        } else {
            1.0
        };
        self.icon(ui, rect.center(), rect.height() * scale, icon, alt1, alt2, Color32::WHITE);
        resp
    }

    #[allow(clippy::too_many_arguments)]
    fn icon(&mut self, ui: &egui::Ui, center: Pos2, size: f32, icon: &str, alt1: Color32, alt2: Color32, tint: Color32) {
        if let Some(tex) = self.icons.get(ui.ctx(), icon, alt1, alt2, size) {
            let rect = Rect::from_center_size(center, Vec2::splat(size));
            ui.painter().image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
    }

    fn slider(&mut self, ui: &mut egui::Ui, field: Field, track: Rect, vertical: bool) {
        let p = self.pal;
        let resp = ui.interact(track, Id::new(("slider", field)), Sense::click_and_drag());
        let (lo, hi) = field.range();
        let mut v = field.get(&self.cfg);
        let before = v;
        if resp.is_pointer_button_down_on() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = if vertical {
                    1.0 - (pos.y - track.top() - 9.0) / (track.height() - 18.0)
                } else {
                    (pos.x - track.left() - 9.0) / (track.width() - 18.0)
                };
                v = lo + t.clamp(0.0, 1.0) as f64 * (hi - lo);
            }
        }
        if resp.hovered() {
            let d = wheel(ui);
            let d = d.y + d.x;
            if d != 0.0 {
                v += d.signum() as f64 * (hi - lo) * 0.05;
            }
        }
        if resp.double_clicked() {
            v = field.default();
        }
        if v != before {
            field.set(&mut self.cfg, v);
            if field == Field::Volume && self.last_volume_blip.elapsed() > Duration::from_millis(80) {
                self.last_volume_blip = Instant::now();
                self.play("sfx.default", Opts { channel: Some(2), ..Opts::default() });
            }
        }
        if (resp.drag_stopped() || resp.clicked()) && matches!(field, Field::Pitch | Field::Intonation | Field::Variation) {
            self.play("&.ok", Opts { channel: Some(2), ..Opts::default() });
        }

        let v = field.get(&self.cfg);
        let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
        let painter = ui.painter();
        painter.rect_filled(track, 9.0, p.input_bg);
        let thumb = if vertical {
            Rect::from_min_size(pos2(track.left(), track.bottom() - 18.0 - t * (track.height() - 18.0)), vec2(track.width(), 18.0))
        } else {
            Rect::from_min_size(pos2(track.left() + t * (track.width() - 18.0), track.top()), vec2(18.0, 18.0))
        };
        if resp.hovered() || resp.dragged() {
            painter.rect_stroke(thumb, 8.0, Stroke::new(3.0, p.hl), StrokeKind::Outside);
        }
        painter.rect_filled(thumb, 8.0, p.accent);
    }

    fn value_box(&mut self, ui: &mut egui::Ui, field: Field, rect: Rect) {
        let p = self.pal;
        ui.painter().rect_filled(rect, 8.0, p.input_bg);
        if self.editing == Some(field) {
            let resp = ui.put(
                rect.shrink2(vec2(2.0, 1.0)),
                egui::TextEdit::singleline(&mut self.edit_text)
                    .horizontal_align(Align::Center)
                    .frame(egui::Frame::NONE)
                    .font(font(14.0))
                    .text_color(p.text),
            );
            if self.edit_focus {
                resp.request_focus();
                self.edit_focus = false;
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if resp.lost_focus() || enter {
                let txt: String = self.edit_text.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
                if let Ok(v) = txt.parse::<f64>() {
                    field.set(&mut self.cfg, v);
                }
                self.editing = None;
            }
            return;
        }
        let resp = ui.interact(rect, Id::new(("value", field)), Sense::click());
        let v = field.get(&self.cfg);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, field.format(v), font(14.0), p.text);
        if resp.double_clicked() {
            field.set(&mut self.cfg, field.default());
            self.editing = None;
        } else if resp.clicked() {
            self.editing = Some(field);
            self.edit_text = field.format(v).trim_end_matches('%').to_string();
            self.edit_focus = true;
        }
    }

    // ---------------------------------------------------------------- sections

    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let bar = r(0.0, 0.0, W, TITLE_H);
        let drag = ui.interact(bar, Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started() && !self.cfg.anchor_window {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.painter().rect_filled(bar, 0.0, p.shadow);
        let title = ui.painter().text(pos2(12.0, TITLE_H / 2.0), Align2::LEFT_CENTER, "animalese typing", font(14.0), p.topbar);
        if self.muted == Some(true) {
            let warning = match &self.cfg.disable_hotkey {
                Some(h) => self.t("warning.disabled").replace("''", &format!("'{}'", h.to_uppercase())),
                None => self.t("warning.disabled_no_hotkey").to_string(),
            };
            ui.painter().text(pos2(title.right() + 12.0, TITLE_H / 2.0), Align2::LEFT_CENTER, warning, font(13.0), p.hl3);
        }

        let mut buttons: Vec<(&str, &str)> = vec![
            ("keyboard", "keyboard"),
            ("config", "config"),
            ("mute", if self.muted != Some(false) { "start" } else { "stop" }),
        ];
        if !self.cfg.anchor_window {
            buttons.push(("minimize", "minimize"));
            buttons.push(("close", "close"));
        }
        let mut x = W - 16.0 - 22.0 * buttons.len() as f32 - 14.0 * (buttons.len() - 1) as f32;
        for (id, icon) in buttons {
            let color = if id == "mute" && self.muted == Some(true) { p.hl3 } else { p.topbar };
            let resp = self.icon_button(ui, r(x, 3.0, 22.0, 22.0), id, icon, color, color);
            let tip = match id {
                "keyboard" => "Key Bindings",
                "config" => "Settings",
                "mute" if self.muted.is_none() => "Start Animalese Typing",
                "mute" if self.muted == Some(true) => "Enable app",
                "mute" => "Disable app",
                "minimize" => "Minimize",
                _ => "Close",
            };
            if resp.on_hover_text(tip).clicked() {
                match id {
                    "keyboard" => self.overlay = if self.overlay == Overlay::Bindings { Overlay::None } else { Overlay::Bindings },
                    "config" => {
                        self.running_apps = sys::running_apps();
                        self.overlay = if self.overlay == Overlay::Settings { Overlay::None } else { Overlay::Settings };
                    }
                    "mute" => {
                        if self.muted.is_none() {
                            sys::spawn_daemon();
                        } else {
                            sys::toggle_daemon_mute();
                        }
                        self.last_poll = Instant::now() - Duration::from_millis(400);
                    }
                    "minimize" => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                    _ => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
            x += 36.0;
        }
    }

    fn key_cap(&mut self, ui: &mut egui::Ui, rect: Rect, btn: &Btn, id: (usize, usize), hovered: &mut Option<String>) {
        let p = self.pal;
        let sound = btn.sound.clone().unwrap_or_else(|| "sfx.default".into());
        let resp = ui.interact(rect, Id::new(("cap", self.tab, id)), Sense::click());
        if resp.contains_pointer() {
            *hovered = Some(sound.clone());
        }
        let pressed = self.pressed.iter().any(|(s, _)| *s == sound);
        let lit = self.highlight.as_deref() == Some(sound.as_str());
        let h = rect.height();
        let radius = h * 0.32;
        let painter = ui.painter();
        painter.rect_filled(Rect::from_min_max(pos2(rect.left(), rect.top() + h * 0.167), rect.max), radius, p.shadow);
        let top = Rect::from_min_size(pos2(rect.left(), rect.top() + if pressed { 4.0 } else { 0.0 }), vec2(rect.width(), h * 0.883));
        let fill = if lit { p.hl2 } else if pressed { p.hl } else { p.bg2 };
        painter.rect_filled(top, radius, fill);
        painter.rect_stroke(top, radius, Stroke::new(1.0, with_alpha(p.shadow, 0.8)), StrokeKind::Inside);
        let fg = if lit || pressed { p.hl_text } else { p.label };
        let center = pos2(top.center().x, top.top() + top.height() * 0.5);
        if let Some(icon) = &btn.icon {
            self.icon(ui, center, h * 0.6, icon, fg, fg, Color32::WHITE);
        } else if let Some(label) = &btn.label {
            ui.painter().text(center, Align2::CENTER_CENTER, label.trim(), font(14.0), fg);
        }
    }

    fn sound_board(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let cont = r(10.0, 36.0, 432.0, 181.0);
        ui.painter().rect_filled(cont.translate(vec2(0.0, 6.0)), 6.0, p.shadow);
        ui.painter().rect_filled(cont, 6.0, p.bg2);
        let inner = cont.shrink(8.0);
        if self.tab == 2 {
            self.piano(ui, inner);
            return;
        }
        let rows = match self.tab {
            0 => self.layouts.special.clone(),
            3 => self.layouts.sfx.clone(),
            _ if self.cfg.voice_language == "korean" => self.layouts.korean.clone(),
            _ => self.layouts.english.clone(),
        };
        let mut hovered = None;
        let (mut x, mut y) = (inner.left(), inner.top());
        for (ri, row) in rows.iter().enumerate() {
            for (ci, btn) in row.iter().enumerate() {
                let w = match btn.btn_type.as_deref() {
                    Some("m") => 64.0,
                    Some("l") => 96.0,
                    _ => 32.0,
                };
                if x + w > inner.right() + 1.0 {
                    x = inner.left();
                    y += 34.0;
                }
                if btn.label.is_some() {
                    self.key_cap(ui, r(x, y, w - 2.0, 32.0), btn, (ri, ci), &mut hovered);
                }
                x += w;
            }
            x = inner.left();
            y += 34.0;
        }
        let (pressed, down) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down()));
        if let Some(sound) = &hovered {
            let entered = self.last_hover_cap.as_ref() != Some(sound);
            if pressed || (down && entered) {
                self.press_sound(sound, false);
            }
        }
        self.last_hover_cap = hovered;
    }

    fn piano(&mut self, ui: &mut egui::Ui, inner: Rect) {
        let p = self.pal;
        const WW: f32 = 30.0;
        let keys = self.layouts.piano.clone();
        let whites = keys.iter().filter(|k| k.btn_type.as_deref() != Some("b")).count();
        let total = whites as f32 * WW;
        let max_scroll = (total - inner.width()).max(0.0);
        if !self.piano_centered {
            self.piano_scroll = max_scroll / 2.0;
            if let Some(hl) = &self.highlight {
                if let Some(i) = keys.iter().position(|k| k.sound.as_deref() == Some(hl.as_str())) {
                    let wi = keys[..i].iter().filter(|k| k.btn_type.as_deref() != Some("b")).count();
                    self.piano_scroll = wi as f32 * WW - inner.width() / 2.0;
                }
            }
            self.piano_centered = true;
        }
        let resp = ui.interact(inner, Id::new("piano"), Sense::click_and_drag());
        if resp.hovered() {
            let d = wheel(ui);
            self.piano_scroll -= d.y + d.x;
        }
        self.piano_scroll = self.piano_scroll.clamp(0.0, max_scroll);

        // Key rects: whites first, blacks drawn (and hit-tested) on top.
        let top = inner.top() + 4.0;
        let mut white_rects = Vec::new();
        let mut black_rects = Vec::new();
        let mut wi = 0;
        for k in &keys {
            let x = inner.left() + wi as f32 * WW - self.piano_scroll;
            if k.btn_type.as_deref() == Some("b") {
                black_rects.push((r(x - 9.0, top, 18.0, 82.0), k));
            } else {
                white_rects.push((r(x + 1.0, top, WW - 2.0, inner.height() - 8.0), k));
                wi += 1;
            }
        }
        let ptr = ui.input(|i| i.pointer.hover_pos()).filter(|p| inner.contains(*p) && resp.contains_pointer());
        let under = ptr.and_then(|pt| {
            black_rects.iter().find(|(rc, _)| rc.contains(pt)).or_else(|| white_rects.iter().find(|(rc, _)| rc.contains(pt)))
        });
        let under_sound = under.and_then(|(_, k)| k.sound.clone());

        let (pressed, down, released) =
            ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released()));
        if released || !down {
            self.release_piano();
        }
        if let Some(s) = &under_sound {
            if pressed || (down && self.piano_held.is_some() && self.piano_held.as_ref() != Some(s)) {
                self.press_sound(s, true);
            }
        } else if down {
            self.release_piano();
        }

        let painter = ui.painter().with_clip_rect(inner);
        let held = self.piano_held.clone();
        let fill_for = |k: &Btn, base: Color32| {
            let s = k.sound.as_deref();
            if s.is_some() && s == held.as_deref() {
                p.hl
            } else if s.is_some() && s == self.highlight.as_deref() {
                p.hl2
            } else {
                base
            }
        };
        for (rc, k) in &white_rects {
            painter.rect_filled(rc.translate(vec2(0.0, 5.0)), 6.0, p.shadow);
            let down_off = if k.sound == held { 3.0 } else { 0.0 };
            painter.rect_filled(rc.translate(vec2(0.0, down_off)), 6.0, fill_for(k, Color32::WHITE));
            if let Some(label) = &k.label {
                painter.text(pos2(rc.center().x, rc.bottom() - 12.0), Align2::CENTER_CENTER, label, font(11.0), Color32::from_black_alpha(40));
            }
        }
        for (rc, k) in &black_rects {
            painter.rect_filled(rc.translate(vec2(0.0, 4.0)), 5.0, Color32::from_gray(40));
            let down_off = if k.sound == held { 3.0 } else { 0.0 };
            painter.rect_filled(rc.translate(vec2(0.0, down_off)), 5.0, fill_for(k, Color32::from_gray(0x44)));
        }
    }

    fn remapper(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let panel = r(452.0, 38.0, 176.0, 149.0);
        ui.painter().rect_filled(panel, CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 }, p.bg2);
        ui.painter().text(pos2(panel.center().x, panel.top() + 20.0), Align2::CENTER_CENTER, self.t("remapper"), font(15.0), p.text);

        let text = match &self.remap {
            Some(info) => info.label(),
            None => self.t("remap").to_string(),
        };
        let galley = ui.painter().layout_no_wrap(text, font(14.0), p.accent);
        let size = vec2((galley.size().x + 32.0).clamp(60.0, panel.width() - 16.0), 40.0);
        let mon = Rect::from_center_size(pos2(panel.center().x, panel.top() + 62.0), size);
        let resp = ui.interact(mon, Id::new("remap_monitor"), Sense::click());
        if resp.clicked() {
            self.monitoring = true;
            capture::start(ui.ctx());
        } else if ui.input(|i| i.pointer.primary_pressed()) && !resp.contains_pointer() && self.remap.is_none() && self.monitoring {
            self.monitoring = false;
            capture::stop();
        }
        ui.painter().rect_filled(mon, 8.0, p.input_bg);
        if self.monitoring || self.remap.is_some() {
            let color = if self.remap.is_some() { p.hl2 } else { with_alpha(p.hl, 0.65) };
            let o = mon.shrink(2.0);
            let pts = [o.left_top(), o.right_top(), o.right_bottom(), o.left_bottom(), o.left_top()];
            ui.painter().extend(egui::Shape::dashed_line(&pts, Stroke::new(4.0, color), 6.0, 4.0));
        }
        let text_color = if self.remap.is_some() { p.text } else { p.accent };
        ui.painter().galley_with_override_text_color(mon.center() - galley.size() / 2.0, galley, text_color);

        let active = self.remap.is_some();
        for (i, (id, icon)) in [("remap_accept", "accept"), ("remap_reset", "reset")].into_iter().enumerate() {
            let base = r(panel.left() + 8.0 + i as f32 * 84.0, panel.top() + 106.0, 76.0, 28.0);
            let resp = ui.interact(base, Id::new(id), if active { Sense::click() } else { Sense::hover() });
            let lifted = active && !resp.is_pointer_button_down_on();
            let rc = if lifted { base.translate(vec2(0.0, -4.0)) } else { base };
            if lifted {
                ui.painter().rect_filled(base, 32.0, p.shadow);
            }
            ui.painter().rect_filled(rc, 32.0, p.bg2);
            let tint = if active { Color32::WHITE } else { Color32::from_white_alpha(100) };
            self.icon(ui, rc.center(), 22.0, icon, p.label, p.label, tint);
            if active && resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
                self.blip("sfx.default");
            }
            if resp.clicked() {
                if i == 0 {
                    self.remap_stop();
                } else if let Some(info) = &self.remap {
                    let d = info.default_sound.clone();
                    self.remap_apply(&d);
                }
            }
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let tabs = [("config", "Special Functions"), ("speech", "Speech"), ("note", "Instruments"), ("sfx", "Sound Effects")];
        for (i, (icon, tip)) in tabs.into_iter().enumerate() {
            let base = r(452.0 + i as f32 * 44.0, 187.0, 44.0, 30.0);
            let pressed = self.tab == i;
            let resp = ui.interact(base, Id::new(("tab", i)), Sense::click()).on_hover_text(tip);
            let radius = CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 };
            let rc = if pressed {
                ui.painter().rect_filled(base.translate(vec2(0.0, 6.0)), radius, p.shadow);
                base
            } else {
                let rc = base.translate(vec2(0.0, 6.0));
                ui.painter().rect_filled(rc.translate(vec2(0.0, -6.0)), radius, p.shadow);
                rc
            };
            ui.painter().rect_filled(rc, radius, if pressed { p.bg2 } else { p.input_bg });
            let color = if pressed && self.remap.is_some() { p.hl2 } else { p.label };
            self.icon(ui, rc.center(), 22.0, icon, color, color, Color32::WHITE);
            if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
                self.change_tab(i);
                self.blip("sfx.default");
            }
        }
    }

    fn voice_editor(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let boxr = r(22.0, 238.0, 420.0, 96.0);
        ui.painter().rect_stroke(boxr, 9.0, Stroke::new(2.0, p.bg2), StrokeKind::Inside);
        for (i, (field, key)) in
            [(Field::Pitch, "voice.pitch"), (Field::Intonation, "voice.intonation"), (Field::Variation, "voice.variation")]
                .into_iter()
                .enumerate()
        {
            let y = 248.0 + i as f32 * 28.0;
            self.slider(ui, field, r(32.0, y, 170.0, 18.0), false);
            self.value_box(ui, field, r(210.0, y - 1.0, 52.0, 20.0));
            ui.painter().text(pos2(272.0, y + 9.0), Align2::LEFT_CENTER, self.t(key), font(14.0), p.text);
        }

        // voice type: male / female faces and the personality select
        let kind = self.cfg.voice_profile.kind.clone();
        let is_male = kind.starts_with('m');
        for (gender, cx) in [("male", 512.0), ("female", 572.0)] {
            let selected = (gender == "male") == is_male;
            let rect = Rect::from_center_size(pos2(cx, 266.0), Vec2::splat(52.0));
            let resp = ui.interact(rect, Id::new(("gender", gender)), Sense::click());
            let scale = if resp.is_pointer_button_down_on() { 0.9 } else if resp.hovered() { 1.07 } else { 1.0 };
            let (a1, a2, glow, lip) = match (gender, selected) {
                ("male", true) => (0xf0feff, 0x52e1fa, Some(0x00aaff), 0xace3ff),
                ("female", true) => (0xfff1ff, 0xff5ce9, Some(0xf71bff), 0xffc3f7),
                _ => (0xf3f3f3, 0x4b4b4b, None, 0xdadada),
            };
            let c = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            if let Some(g) = glow {
                for (rad, a) in [(30.0, 0.05), (26.0, 0.08), (22.0, 0.12)] {
                    ui.painter().circle_filled(rect.center(), rad * scale, with_alpha(c(g), a));
                }
            }
            let size = 64.0 * scale;
            self.icon(ui, rect.center() + vec2(0.0, 2.0), size, gender, c(lip), c(lip), Color32::WHITE);
            self.icon(ui, rect.center(), size, gender, c(a1), c(a2), Color32::WHITE);
            if resp.clicked() {
                if !selected {
                    self.cfg.voice_profile.kind = if gender == "male" { "m1" } else { "f1" }.into();
                }
                self.play("&.ok", Opts { channel: Some(2), volume: 0.55, ..Opts::default() });
            }
        }
        let pill = Rect::from_center_size(pos2(542.0, 309.0), vec2(100.0, 20.0));
        let resp = ui.interact(pill, Id::new("voice_type"), Sense::click());
        let (bg, fg) = if is_male {
            (Color32::from_rgb(0xd2, 0xee, 0xff), Color32::from_rgb(0x00, 0xaa, 0xff))
        } else {
            (Color32::from_rgb(0xff, 0xe9, 0xff), Color32::from_rgb(0xff, 0x8b, 0xcb))
        };
        let a = if resp.hovered() || self.popup == Popup::VoiceType { 1.0 } else { 0.45 };
        ui.painter().rect_filled(pill, 6.0, with_alpha(bg, a));
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, self.t(&kind), font(14.0), with_alpha(fg, a.max(0.7)));
        if resp.clicked() {
            self.popup = if self.popup == Popup::VoiceType { Popup::None } else { Popup::VoiceType };
            self.popup_anchor = pill;
        }
    }

    fn note_editor(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let boxr = r(22.0, 238.0, 420.0, 70.0);
        ui.painter().rect_stroke(boxr, 9.0, Stroke::new(2.0, p.bg2), StrokeKind::Inside);
        let pill = r(32.0, 248.0, 230.0, 20.0);
        let resp = ui.interact(pill, Id::new("instrument"), Sense::click());
        let a = if resp.hovered() || self.popup == Popup::Instrument { 1.0 } else { 0.6 };
        ui.painter().rect_filled(pill, 6.0, with_alpha(p.input_bg, a));
        let inst = self.cfg.note_profile.instrument.clone();
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, instrument_label(&inst), font(14.0), p.text);
        ui.painter().text(pos2(272.0, 258.0), Align2::LEFT_CENTER, self.t("note.instrument"), font(14.0), p.text);
        if resp.clicked() {
            self.popup = if self.popup == Popup::Instrument { Popup::None } else { Popup::Instrument };
            self.popup_anchor = pill;
        }
        self.slider(ui, Field::Transpose, r(32.0, 278.0, 170.0, 18.0), false);
        self.value_box(ui, Field::Transpose, r(210.0, 277.0, 52.0, 20.0));
        ui.painter().text(pos2(272.0, 287.0), Align2::LEFT_CENTER, self.t("note.transpose"), font(14.0), p.text);
    }

    fn volume(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.painter().rect_filled(r(641.0, TITLE_H, 46.0, H - TITLE_H), 0.0, p.bg2);
        self.slider(ui, Field::Volume, r(649.0, 44.0, 30.0, 240.0), true);
        self.value_box(ui, Field::Volume, r(645.0, 294.0, 38.0, 20.0));
    }

    fn popups(&mut self, ctx: &egui::Context) {
        if self.popup == Popup::None {
            return;
        }
        let p = self.pal;
        let options: Vec<(String, String)> = match self.popup {
            Popup::VoiceType => {
                let male = self.cfg.voice_profile.kind.starts_with('m');
                player::VOICE_TYPES
                    .iter()
                    .filter(|t| t.starts_with('m') == male)
                    .map(|t| (t.to_string(), self.t(t).to_string()))
                    .collect()
            }
            _ => INSTRUMENTS.iter().map(|i| (i.to_string(), instrument_label(i).to_string())).collect(),
        };
        let current = match self.popup {
            Popup::VoiceType => self.cfg.voice_profile.kind.clone(),
            _ => self.cfg.note_profile.instrument.clone(),
        };
        let row_h = 22.0;
        let width = self.popup_anchor.width();
        let height = row_h * options.len() as f32 + 8.0;
        let mut pos = self.popup_anchor.left_bottom() + vec2(0.0, 2.0);
        if pos.y + height > H {
            pos.y = self.popup_anchor.top() - height - 2.0;
        }
        let area_rect = Rect::from_min_size(pos, vec2(width, height));
        let mut chosen = None;
        egui::Area::new(Id::new("popup")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(area_rect.size(), Sense::click());
            ui.painter().rect_filled(rect.translate(vec2(0.0, 3.0)), 8.0, with_alpha(Color32::BLACK, 0.12));
            ui.painter().rect_filled(rect, 8.0, p.bg);
            ui.painter().rect_stroke(rect, 8.0, Stroke::new(2.0, p.input_bg), StrokeKind::Inside);
            for (i, (id, label)) in options.iter().enumerate() {
                let rc = r(rect.left() + 4.0, rect.top() + 4.0 + i as f32 * row_h, width - 8.0, row_h);
                let resp = ui.interact(rc, Id::new(("popup_opt", i)), Sense::click());
                if *id == current {
                    ui.painter().rect_filled(rc, 6.0, p.input_accent);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rc, 6.0, p.input_bg);
                }
                let fg = if *id == current { p.hl_text } else { p.text };
                ui.painter().text(rc.center(), Align2::CENTER_CENTER, label, font(14.0), fg);
                if resp.clicked() {
                    chosen = Some(id.clone());
                }
            }
        });
        if let Some(id) = chosen {
            match self.popup {
                Popup::VoiceType => {
                    self.cfg.voice_profile.kind = id;
                    self.play("&.ok", Opts { channel: Some(2), ..Opts::default() });
                }
                _ => {
                    self.cfg.note_profile.instrument = id;
                    self.play("%.60", Opts { channel: Some(2), ..Opts::default() });
                }
            }
            self.popup = Popup::None;
        } else if ctx.input(|i| i.pointer.primary_pressed()) {
            let pt = ctx.input(|i| i.pointer.interact_pos());
            if pt.is_some_and(|pt| !area_rect.contains(pt) && !self.popup_anchor.contains(pt)) {
                self.popup = Popup::None;
            }
        }
    }

    /// Dimmed backdrop + centered panel in a foreground layer; returns the panel's inner ui.
    fn overlay_frame(&mut self, ctx: &egui::Context, size: Vec2, content: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let p = self.pal;
        let full = r(0.0, TITLE_H, W, H - TITLE_H);
        let panel = Rect::from_min_size(pos2((W - size.x) / 2.0, TITLE_H + 4.0), size);
        let mut close = false;
        egui::Area::new(Id::new("overlay")).order(Order::Middle).fixed_pos(full.min).show(ctx, |ui| {
            let backdrop = ui.interact(full, Id::new("backdrop"), Sense::click());
            ui.painter().rect_filled(full, 0.0, Color32::from_black_alpha(0x2f));
            for (grow, a) in [(10.0, 0.05), (6.0, 0.08), (3.0, 0.12)] {
                ui.painter().rect_filled(panel.expand(grow).translate(vec2(0.0, 6.0)), 8.0 + grow, with_alpha(Color32::BLACK, a));
            }
            ui.painter().rect_filled(panel, 8.0, p.bg);
            ui.interact(panel, Id::new("panel_bg"), Sense::click());
            if backdrop.clicked() {
                close = true;
            }
            ui.scope_builder(
                UiBuilder::new().max_rect(panel.shrink(16.0)).layout(Layout::top_down(Align::Center)),
                |ui| {
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| content(self, ui));
                },
            );
        });
        if close {
            self.overlay = Overlay::None;
        }
    }

    fn settings_overlay(&mut self, ctx: &egui::Context) {
        self.overlay_frame(ctx, vec2(328.0, 305.0), |ed, ui| {
            let p = ed.pal;
            let sep = |ui: &mut egui::Ui| {
                let (rc, _) = ui.allocate_exact_size(vec2(ui.available_width() * 0.95, 4.0), Sense::hover());
                ui.painter().rect_filled(rc.shrink2(vec2(0.0, 1.0)), 16.0, p.bg2);
            };

            ui.heading(ed.t("settings.general"));
            sep(ui);
            ui.label(ed.t("settings.lang"));
            egui::ComboBox::from_id_salt("lang").width(150.0).selected_text(ed.t(&ed.cfg.lang).to_string()).show_ui(ui, |ui| {
                for code in i18n::LANGS {
                    let label = ed.tr.t(code).to_string();
                    ui.selectable_value(&mut ed.cfg.lang, code.to_string(), label);
                }
            });
            ui.label(ed.t("settings.theme"));
            let theme_label = |ed: &Editor, t: &str| ed.t(&format!("settings.theme.{t}")).to_string();
            egui::ComboBox::from_id_salt("theme").width(150.0).selected_text(theme_label(ed, &ed.cfg.theme)).show_ui(ui, |ui| {
                for t in theme::THEMES {
                    let label = theme_label(ed, t);
                    ui.selectable_value(&mut ed.cfg.theme, t.to_string(), label);
                }
            });
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                if ui.checkbox(&mut ed.cfg.anchor_window, ed.tr.t("settings.anchor_window")).changed() && ed.cfg.anchor_window {
                    if let Some((x, y)) = sys::work_area_corner() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(x - W - 10.0, y - H - 10.0)));
                    }
                }
                if ui.checkbox(&mut ed.cfg.startup_run, ed.tr.t("settings.startup")).changed() {
                    sys::set_autostart(ed.cfg.startup_run);
                }
                ui.checkbox(&mut ed.cfg.always_active, ed.tr.t("settings.always_active"));
                ui.add_enabled_ui(!ed.cfg.always_active, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(ed.tr.t("settings.apps.mode"));
                        toggle(ui, &mut ed.cfg.selected_active, &p);
                    });
                    let header = if ed.cfg.selected_active { "settings.apps.active" } else { "settings.apps.inactive" };
                    ui.small(ed.tr.t(header));
                    egui::Frame::new().fill(p.bg2).corner_radius(8.0).inner_margin(6.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut names = ed.cfg.selected_apps.clone();
                        for a in &ed.running_apps {
                            if !names.iter().any(|n| n.eq_ignore_ascii_case(a)) {
                                names.push(a.clone());
                            }
                        }
                        egui::ScrollArea::vertical().id_salt("apps").max_height(140.0).show(ui, |ui| {
                            for name in names {
                                let mut on = ed.cfg.selected_apps.contains(&name);
                                if ui.checkbox(&mut on, egui::RichText::new(&name).size(12.0)).changed() {
                                    if on {
                                        ed.cfg.selected_apps.push(name);
                                    } else {
                                        ed.cfg.selected_apps.retain(|n| *n != name);
                                    }
                                }
                            }
                        });
                    });
                });
            });

            ui.add_space(10.0);
            ui.heading(ed.t("settings.audio"));
            sep(ui);
            ui.label(ed.t("settings.voice_language"));
            let lang_label = |ed: &Editor, l: &str| ed.t(&format!("voice.language.{l}")).to_string();
            let before = ed.cfg.voice_language.clone();
            egui::ComboBox::from_id_salt("voice_lang").width(150.0).selected_text(lang_label(ed, &before)).show_ui(ui, |ui| {
                for l in ["english", "korean"] {
                    let label = lang_label(ed, l);
                    ui.selectable_value(&mut ed.cfg.voice_language, l.to_string(), label);
                }
            });
            if ed.cfg.voice_language != before {
                ed.change_tab(1);
                ed.play("&.ok", Opts::default());
            }
            ui.label(ed.t("settings.audio.mode"));
            egui::Frame::new().fill(p.bg2).corner_radius(8.0).inner_margin(8.0).show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    for m in 0..4u8 {
                        let label = ed.tr.t(&format!("settings.audio.mode.{}", m + 1)).to_string();
                        ui.radio_value(&mut ed.cfg.audio_mode, m, label);
                    }
                });
            });
            ui.checkbox(&mut ed.cfg.hold_repeat, ed.tr.t("settings.hold_repeat"));

            ui.add_space(10.0);
            ui.heading(ed.t("settings.reset"));
            sep(ui);
            ui.label(egui::RichText::new(ed.t("settings.reset.description")).size(12.0).color(with_alpha(p.text, 0.6)));
            let (rc, resp) = ui.allocate_exact_size(vec2(90.0, 32.0), Sense::click_and_drag());
            ui.painter().rect_filled(rc, 8.0, if resp.hovered() { p.accent } else { p.input_bg });
            if resp.is_pointer_button_down_on() {
                let start = *ed.reset_started.get_or_insert_with(Instant::now);
                let t = (start.elapsed().as_secs_f32() / 1.5).min(1.0);
                let fill = Rect::from_min_size(rc.min, vec2(rc.width() * t, rc.height()));
                ui.painter().rect_filled(fill, 8.0, Color32::from_rgba_unmultiplied(255, 0, 0, 0x87));
                ui.ctx().request_repaint();
                if t >= 1.0 {
                    let keep = (ed.cfg.lang.clone(), ed.cfg.theme.clone(), ed.cfg.startup_run);
                    ed.cfg = Config { lang: keep.0, theme: keep.1, startup_run: keep.2, ..Config::default() };
                    ed.reset_started = None;
                    ed.blip("sfx.backspace");
                }
            } else {
                ed.reset_started = None;
            }
            ui.painter().text(rc.center(), Align2::CENTER_CENTER, ed.t("settings.reset.button"), font(14.0), p.text);

            ui.add_space(12.0);
            let footer = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Animalese Typing (Rust) v{}", env!("CARGO_PKG_VERSION")))
                        .size(12.0)
                        .color(with_alpha(p.text, 0.35)),
                )
                .sense(Sense::click()),
            );
            if footer.on_hover_text("GitHub").clicked() {
                sys::open_url(GITHUB);
            }
        });
    }

    fn bindings_overlay(&mut self, ctx: &egui::Context) {
        let (rows, none) = bindings(&self.cfg);
        self.overlay_frame(ctx, vec2(460.0, 305.0), |ed, ui| {
            let p = ed.pal;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.heading(ed.t("bindings.title"));
            ui.add_space(6.0);
            egui::Grid::new("bindings").striped(true).num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                ui.label(egui::RichText::new(ed.t("bindings.c1")).size(15.0).color(p.text));
                ui.label(egui::RichText::new(ed.t("bindings.c2")).size(15.0).color(p.text));
                ui.end_row();
                for (combos, sound) in &rows {
                    ui.vertical(|ui| {
                        for (combo, custom) in combos {
                            let t = egui::RichText::new(combo).size(12.0);
                            ui.label(if *custom { t.color(p.hl2) } else { t });
                        }
                    });
                    ui.label(egui::RichText::new(sound).size(12.0));
                    ui.end_row();
                }
                if !none.is_empty() {
                    ui.label("NONE");
                    ui.vertical(|ui| {
                        for combo in &none {
                            ui.label(egui::RichText::new(combo).size(12.0).color(p.hl2));
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
}

/// A small pill switch like the original's `.toggle` checkbox.
fn toggle(ui: &mut egui::Ui, on: &mut bool, p: &Pal) {
    let (rc, resp) = ui.allocate_exact_size(vec2(28.0, 16.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    ui.painter().rect_filled(rc, 8.0, p.input_bg);
    ui.painter().rect_stroke(rc, 8.0, Stroke::new(2.0, p.input_accent), StrokeKind::Inside);
    let x = if *on { rc.right() - 8.0 } else { rc.left() + 8.0 };
    ui.painter().circle_filled(pos2(x, rc.center().y), 5.0, p.hl);
}

/// Rows for the key bindings table: (combos with "is custom", sound id), plus combos set to none.
/// Mirrors remapper.cjs.
fn bindings(cfg: &Config) -> (Vec<(Vec<(String, bool)>, String)>, Vec<String>) {
    let mut vks: Vec<u32> = keymap::all().map(|(c, _)| c).collect();
    vks.sort_unstable();
    vks.dedup();
    let mut sounds: Vec<(String, Vec<(String, bool)>)> = Vec::new();
    let mut none = Vec::new();
    for vk in vks {
        let Some(def) = keymap::lookup(vk) else { continue };
        let custom = cfg.remapped_keys.get(&vk.to_string());
        let slots: [(&str, &str, Option<&Option<String>>); 4] = [
            ("", def.sound, custom.map(|c| &c.sound)),
            ("Shift + ", def.shift_sound(), custom.map(|c| &c.shift_sound)),
            ("Ctrl + ", def.ctrl, custom.map(|c| &c.ctrl_sound)),
            ("Alt + ", "", custom.map(|c| &c.alt_sound)),
        ];
        for (prefix, default, over) in slots {
            let combo = format!("{prefix}{}", def.key).to_uppercase();
            let (sound, is_custom) = match over.and_then(|o| o.as_ref()) {
                Some(s) => (s.clone(), true),
                None => (default.to_string(), false),
            };
            if is_custom && sound.is_empty() {
                none.push(combo);
                continue;
            }
            if sound.trim().is_empty() {
                continue;
            }
            match sounds.iter_mut().find(|(s, _)| *s == sound) {
                Some((_, list)) => list.push((combo, is_custom)),
                None => sounds.push((sound, vec![(combo, is_custom)])),
            }
        }
    }
    let rows = sounds
        .into_iter()
        .map(|(s, combos)| (combos, s.replacen('%', "note", 1).replacen('&', "voice", 1)))
        .collect();
    (rows, none)
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.housekeeping(&ctx);
        ui.painter().rect_filled(r(0.0, 0.0, W, H), 0.0, self.pal.bg);
        self.title_bar(ui);
        self.sound_board(ui);
        self.remapper(ui);
        self.tabs(ui);
        match self.tab {
            1 => self.voice_editor(ui),
            2 => self.note_editor(ui),
            _ => {}
        }
        self.volume(ui);
        self.popups(&ctx);
        match self.overlay {
            Overlay::Settings => self.settings_overlay(&ctx),
            Overlay::Bindings => self.bindings_overlay(&ctx),
            Overlay::None => {}
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.release_piano();
        if self.cfg != self.saved {
            self.save();
        }
    }
}
