//! Colors (the original's four CSS themes) and fonts.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

#[derive(Clone, Copy)]
pub struct Pal {
    pub shadow: Color32,
    pub topbar: Color32,
    pub bg: Color32,
    pub text: Color32,
    pub bg2: Color32,
    pub label: Color32,
    pub accent: Color32,
    pub input_bg: Color32,
    pub input_accent: Color32,
    pub hl: Color32,
    pub hl2: Color32,
    pub hl3: Color32,
    pub hl_text: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const THEMES: [&str; 4] = ["default", "dark", "pink", "green"];

pub fn palette(name: &str) -> Pal {
    match name {
        "dark" => Pal {
            shadow: hex(0x0c0317),
            topbar: hex(0xffffff),
            bg: hex(0x16072b),
            text: hex(0xbbbbbb),
            bg2: hex(0x2a1548),
            label: hex(0x716680),
            accent: hex(0xd3d3d3),
            input_bg: hex(0x38205b),
            input_accent: hex(0x716680),
            hl: hex(0x10ff4b),
            hl2: hex(0xffed23),
            hl3: hex(0xc05257),
            hl_text: hex(0x16072b),
        },
        "pink" => Pal {
            shadow: hex(0xffd9e8),
            topbar: hex(0xffffff),
            bg: hex(0xffffff),
            text: hex(0xedafc6),
            bg2: hex(0xfdf2f8),
            label: hex(0xffbad4),
            accent: hex(0xffbedb),
            input_bg: hex(0xffe9f5),
            input_accent: hex(0xf4a8cc),
            hl: hex(0x7ee988),
            hl2: hex(0x7be5ff),
            hl3: hex(0xfa9052),
            hl_text: hex(0xffffff),
        },
        "green" => Pal {
            shadow: hex(0xbfd7c0),
            topbar: hex(0xffffff),
            bg: hex(0xffffff),
            text: hex(0xd7bcab),
            bg2: hex(0xf0f7ea),
            label: hex(0xd3baa5),
            accent: hex(0xd5c8ac),
            input_bg: hex(0xe8f0e2),
            input_accent: hex(0x8fb87a),
            hl: hex(0xe7c2b0),
            hl2: hex(0x8dd18f),
            hl3: hex(0xff0000),
            hl_text: hex(0xffffff),
        },
        _ => Pal {
            shadow: hex(0xf3dbc2),
            topbar: hex(0x947e6a),
            bg: hex(0xfff7e4),
            text: hex(0xbba280),
            bg2: hex(0xfff3df),
            label: hex(0xdec1a6),
            accent: hex(0xe0c49e),
            input_bg: hex(0xf4e5cc),
            input_accent: hex(0xd1b48b),
            hl: hex(0x7ee988),
            hl2: hex(0x00ccff),
            hl3: hex(0xea5353),
            hl_text: hex(0xffffff),
        },
    }
}

/// Styles egui's stock widgets (used in the settings overlay) to match the palette.
pub fn apply_style(ctx: &egui::Context, p: &Pal) {
    let p = *p;
    ctx.all_styles_mut(move |s| {
        let v = &mut s.visuals;
        v.dark_mode = false;
        v.override_text_color = Some(p.text);
        v.panel_fill = p.bg;
        v.window_fill = p.bg;
        v.window_stroke = egui::Stroke::NONE;
        v.extreme_bg_color = p.input_bg;
        v.faint_bg_color = p.bg2;
        v.selection.bg_fill = p.hl;
        v.selection.stroke = egui::Stroke::new(1.0, p.hl_text);
        v.hyperlink_color = p.hl2;
        for (w, fill) in [
            (&mut v.widgets.noninteractive, p.bg2),
            (&mut v.widgets.inactive, p.input_bg),
            (&mut v.widgets.hovered, p.accent),
            (&mut v.widgets.active, p.input_accent),
            (&mut v.widgets.open, p.input_bg),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = egui::Stroke::NONE;
            w.fg_stroke = egui::Stroke::new(2.0, p.text);
            w.corner_radius = 6.into();
        }
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(2.0, p.bg2);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(2.0, p.hl);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(2.0, p.hl);
        v.widgets.active.fg_stroke = egui::Stroke::new(2.0, p.hl);
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        for (style, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Small, 11.0),
            (egui::TextStyle::Heading, 18.0),
        ] {
            s.text_styles.insert(style, egui::FontId::proportional(size));
        }
    });
}

/// Which system CJK fonts to load as fallbacks (only when the UI needs them; they're large).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Cjk {
    pub korean: bool,
    pub japanese: bool,
    pub chinese: bool,
}

pub fn set_fonts(ctx: &egui::Context, cjk: Cjk) {
    let mut fonts = FontDefinitions::default();
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    let mut chain = Vec::new();
    // The original's font (Arial Rounded MT Bold) isn't redistributable, so use the system copy
    // that ships with Office/Windows when present and fall back to egui's default font.
    if let Ok(bytes) = std::fs::read(dir.join("ARLRDBD.TTF")) {
        fonts.font_data.insert("rounded".into(), FontData::from_owned(bytes).into());
        chain.push("rounded".to_string());
    }
    let wanted: [(bool, &[&str]); 3] = [
        (cjk.korean, &["malgunbd.ttf", "malgun.ttf"]),
        (cjk.japanese, &["YuGothB.ttc", "meiryob.ttc", "msgothic.ttc"]),
        (cjk.chinese, &["msyhbd.ttc", "msyh.ttc", "simhei.ttf"]),
    ];
    for (on, files) in wanted {
        if !on {
            continue;
        }
        if let Some(bytes) = files.iter().find_map(|f| std::fs::read(dir.join(f)).ok()) {
            let name = format!("cjk{}", chain.len());
            fonts.font_data.insert(name.clone(), FontData::from_owned(bytes).into());
            chain.push(name);
        }
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        for (i, name) in chain.iter().enumerate() {
            list.insert(i, name.clone());
        }
    }
    ctx.set_fonts(fonts);
}
