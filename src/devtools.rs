//! Hidden test modes: `--render out.wav "text"` types text offline into a WAV file, and
//! `--bench` measures how much CPU the mixer needs for fast typing.

use crate::audio::Engine;
use crate::config::Config;
use crate::player::{Mods, Player};
use std::time::Instant;

const RATE: u32 = 48000;

/// Virtual key and shift state for a character.
fn key_for(c: char) -> Option<(u32, bool)> {
    Some(match c {
        'a'..='z' => (c as u32 - 'a' as u32 + 65, false),
        'A'..='Z' => (c as u32 - 'A' as u32 + 65, true),
        '0'..='9' => (c as u32, false),
        ' ' => (32, false),
        '\n' => (13, false),
        '.' => (190, false),
        ',' => (188, false),
        '\'' => (222, false),
        '-' => (189, false),
        '!' => (49, true),
        '?' => (191, true),
        _ => return None,
    })
}

/// Plays `text` at `interval` seconds per key and returns interleaved stereo samples.
fn type_text(cfg: &Config, text: &str, interval: f64) -> Vec<f32> {
    let mut player = Player::with_engine(Engine::new_offline(RATE));
    player.prepare(cfg);
    let mut events = Vec::new();
    for (i, c) in text.chars().enumerate() {
        if let Some((vk, shift)) = key_for(c) {
            let t = i as f64 * interval;
            events.push((t, vk, shift, true));
            events.push((t + interval * 0.6, vk, shift, false));
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut now = 0usize;
    for (t, vk, shift, down) in events {
        let at = (t * RATE as f64) as usize;
        if at > now {
            out.extend(player.engine.render_offline(at - now));
            now = at;
        }
        let m = Mods { shift, ..Mods::default() };
        if down {
            player.key_down(cfg, vk, m);
        } else {
            player.key_up(cfg, vk, m);
        }
    }
    out.extend(player.engine.render_offline(RATE as usize * 2));
    out
}

pub fn render(path: &str, text: &str) {
    let samples = type_text(&Config::load(), text, 0.09);
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    let _ = std::fs::write(path, wav);
}

pub fn bench(report: &str) {
    let text = "The quick brown fox jumps over the lazy dog, 1234567890! ".repeat(80);
    let cfg = Config::load();
    let start = Instant::now();
    let samples = type_text(&cfg, &text, 1.0 / 12.0); // 12 keys per second, ~140 wpm
    let cpu = start.elapsed().as_secs_f64();
    let audio = samples.len() as f64 / 2.0 / RATE as f64;
    let _ = std::fs::write(
        report,
        format!(
            "audio rendered: {audio:.1}s\nwall time: {:.1}ms\none core busy: {:.4}%\n",
            cpu * 1000.0,
            cpu / audio * 100.0
        ),
    );
}
