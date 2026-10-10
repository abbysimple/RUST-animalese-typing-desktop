//! The original SVG icons, recolored by substituting their CSS variables and rasterized with
//! resvg into egui textures (cached per color and size).

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;

fn source(name: &str) -> Option<&'static str> {
    Some(match name {
        "accept" => include_str!("../../assets/svg/accept.svg"),
        "close" => include_str!("../../assets/svg/close.svg"),
        "config" => include_str!("../../assets/svg/config.svg"),
        "female" => include_str!("../../assets/svg/female.svg"),
        "keyboard" => include_str!("../../assets/svg/keyboard.svg"),
        "male" => include_str!("../../assets/svg/male.svg"),
        "minimize" => include_str!("../../assets/svg/minimize.svg"),
        "note" => include_str!("../../assets/svg/note.svg"),
        "question" => include_str!("../../assets/svg/question.svg"),
        "reset" => include_str!("../../assets/svg/reset.svg"),
        "sfx" => include_str!("../../assets/svg/sfx.svg"),
        "speech" => include_str!("../../assets/svg/speech.svg"),
        "start" => include_str!("../../assets/svg/start.svg"),
        "stop" => include_str!("../../assets/svg/stop.svg"),
        _ => return None,
    })
}

fn css(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn rasterize(svg: &str, px: u32) -> Option<ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let (dx, dy) = ((px as f32 - size.width() * scale) / 2.0, (px as f32 - size.height() * scale) / 2.0);
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(&tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

#[derive(Default)]
pub struct Icons {
    cache: HashMap<(String, [u8; 8], u32), TextureHandle>,
}

impl Icons {
    /// `alt1`/`alt2` replace `--alt1-color`/`--alt2-color`; icons without variables get `alt1` as fill.
    pub fn get(&mut self, ctx: &egui::Context, name: &str, alt1: Color32, alt2: Color32, points: f32) -> Option<egui::TextureId> {
        let px = (points * ctx.pixels_per_point()).ceil().max(1.0) as u32;
        let key = (
            name.to_string(),
            [alt1.r(), alt1.g(), alt1.b(), alt1.a(), alt2.r(), alt2.g(), alt2.b(), alt2.a()],
            px,
        );
        if let Some(t) = self.cache.get(&key) {
            return Some(t.id());
        }
        // female.svg has `height="64", viewBox=...`, which browsers accept but XML parsers don't.
        let mut svg = source(name)?.replace("\", ", "\" ").replace("var(--alt1-color)", &css(alt1)).replace("var(--alt2-color)", &css(alt2));
        if !svg.contains(&css(alt1)) {
            if let Some(i) = svg.find("<svg") {
                svg.insert_str(i + 4, &format!(" fill=\"{}\"", css(alt1)));
            }
        }
        let image = rasterize(&svg, px)?;
        let tex = ctx.load_texture(format!("icon-{name}-{px}"), image, TextureOptions::LINEAR);
        let id = tex.id();
        self.cache.insert(key, tex);
        Some(id)
    }
}
