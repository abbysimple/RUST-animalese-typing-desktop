//! Sound and icon files embedded into the exe.

macro_rules! ogg {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/audio/", $name, ".ogg")) as &[u8])),*]
    };
}

static AUDIO: &[(&str, &[u8])] = ogg![
    "sfx",
    "voice/english/f1", "voice/english/f2", "voice/english/f3", "voice/english/f4",
    "voice/english/m1", "voice/english/m2", "voice/english/m3", "voice/english/m4",
    "voice/korean/f1", "voice/korean/f2", "voice/korean/f3", "voice/korean/f4",
    "voice/korean/m1", "voice/korean/m2", "voice/korean/m3", "voice/korean/m4",
    "instrument/girl", "instrument/boy", "instrument/cranky", "instrument/kk_slider",
    "instrument/organ", "instrument/guitar", "instrument/e_piano", "instrument/synth",
    "instrument/whistle",
];

pub fn audio(name: &str) -> Option<&'static [u8]> {
    AUDIO.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

pub const ICON: &[u8] = include_bytes!("../assets/images/icon.png");
pub const ICON_OFF: &[u8] = include_bytes!("../assets/images/icon_off.png");
pub const ICON_MUTE: &[u8] = include_bytes!("../assets/images/icon_mute.png");
pub const ICON_BIG: &[u8] = include_bytes!("../assets/images/icon_256x256.png");

/// Decodes a PNG into (rgba, width, height).
pub fn png_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, info.width, info.height))
}
