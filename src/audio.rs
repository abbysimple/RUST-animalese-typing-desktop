//! Small software mixer on top of cpal.
//!
//! Clips are decoded once into i16 PCM. Each playing sound reads a region of a clip at a
//! variable rate (pitch = playback speed, like the original Howler.js version) with linear
//! interpolation. The output stream is paused after a few seconds of silence so the app uses
//! no CPU and doesn't hold the audio device while you're not typing.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Clip {
    data: Vec<i16>,
    channels: usize,
    pub rate: u32,
}

impl Clip {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }

    fn decode(bytes: &[u8]) -> Option<Clip> {
        let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).ok()?;
        let channels = r.ident_hdr.audio_channels as usize;
        let rate = r.ident_hdr.audio_sample_rate;
        let mut data = Vec::new();
        while let Some(pck) = r.read_dec_packet_itl().ok()? {
            data.extend_from_slice(&pck);
        }
        data.shrink_to_fit();
        (channels > 0).then_some(Clip { data, channels, rate })
    }
}

pub struct PlayCmd {
    pub clip: Arc<Clip>,
    /// Region of the clip in frames.
    pub start: usize,
    pub end: usize,
    /// Playback rate (1.0 = original pitch).
    pub rate: f64,
    /// Rate at the end of the 3.2s intonation ramp; equal to `rate` for none.
    pub end_rate: f64,
    pub volume: f32,
    /// Starting a sound on a channel quickly fades out the previous one on it.
    pub channel: Option<u32>,
    /// Key that can later release (fade out) this sound.
    pub hold: Option<u32>,
}

enum Cmd {
    Play(PlayCmd),
    Release { hold: u32, secs: f32 },
}

const CUTOFF_SECS: f32 = 0.025;
const INTONATION_SECS: f64 = 3.2;
const IDLE_PAUSE: Duration = Duration::from_secs(3);

struct Voice {
    clip: Arc<Clip>,
    pos: f64,
    end: f64,
    /// Source frames per output frame at rate 1.
    step: f64,
    rate: f64,
    start_rate: f64,
    end_rate: f64,
    elapsed: f64,
    vol: f32,
    fade: f32,
    channel: Option<u32>,
    hold: Option<u32>,
}

impl Voice {
    fn fade_out(&mut self, secs: f32, out_rate: f32) {
        let step = self.vol / (secs * out_rate).max(1.0);
        if self.fade == 0.0 || step > self.fade {
            self.fade = step;
        }
    }

    /// Ease-out curve from the original applyIntonation (ramp = 2).
    fn update_rate(&mut self, out_rate: f64) {
        if self.end_rate == self.start_rate {
            return;
        }
        let t = (self.elapsed / (INTONATION_SECS * out_rate)).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.rate = self.start_rate * (self.end_rate / self.start_rate).powf(eased);
    }
}

struct Mixer {
    rx: Receiver<Cmd>,
    voices: Vec<Voice>,
    out_rate: f32,
    out_ch: usize,
    active: Arc<AtomicUsize>,
}

impl Mixer {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play(p) => {
                let out_rate = self.out_rate;
                if let Some(ch) = p.channel {
                    for v in self.voices.iter_mut().filter(|v| v.channel == Some(ch)) {
                        v.fade_out(CUTOFF_SECS, out_rate);
                    }
                }
                if let Some(h) = p.hold {
                    for v in self.voices.iter_mut().filter(|v| v.hold == Some(h)) {
                        v.hold = None;
                    }
                }
                let start_rate = p.rate.max(0.01);
                self.voices.push(Voice {
                    step: p.clip.rate as f64 / out_rate as f64,
                    pos: p.start as f64,
                    end: p.end.min(p.clip.frames()) as f64,
                    clip: p.clip,
                    rate: start_rate,
                    start_rate,
                    end_rate: p.end_rate.max(0.01),
                    elapsed: 0.0,
                    vol: p.volume,
                    fade: 0.0,
                    channel: p.channel,
                    hold: p.hold,
                });
            }
            Cmd::Release { hold, secs } => {
                let out_rate = self.out_rate;
                for v in self.voices.iter_mut().filter(|v| v.hold == Some(hold)) {
                    v.fade_out(secs, out_rate);
                }
            }
        }
    }

    fn render(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.handle(cmd);
        }
        out.fill(0.0);
        let out_ch = self.out_ch;
        let frames = out.len() / out_ch;
        let out_rate = self.out_rate as f64;
        const BLOCK: usize = 32;

        for v in &mut self.voices {
            let clip = v.clip.clone();
            let src_ch = clip.channels;
            let data = &clip.data;
            let last = v.end - 1.0;
            let lerp = |a: usize, b: usize, t: f32| {
                let x0 = data[a] as f32;
                x0 + (data[b] as f32 - x0) * t
            };
            for f in 0..frames {
                if f % BLOCK == 0 {
                    v.update_rate(out_rate);
                }
                if v.pos >= last || v.vol <= 0.0 {
                    v.vol = 0.0;
                    break;
                }
                let i = v.pos as usize;
                let t = (v.pos - i as f64) as f32;
                let a = i * src_ch;
                let b = a + src_ch;
                let l = lerp(a, b, t);
                let r = if src_ch > 1 { lerp(a + 1, b + 1, t) } else { l };
                let g = v.vol / 32768.0;
                let o = &mut out[f * out_ch..];
                if out_ch == 1 {
                    o[0] += (l + r) * 0.5 * g;
                } else {
                    o[0] += l * g;
                    o[1] += r * g;
                }
                v.pos += v.step * v.rate;
                v.vol -= v.fade;
                v.elapsed += 1.0;
            }
        }
        self.voices.retain(|v| v.vol > 0.0);
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        self.active.store(self.voices.len(), Ordering::Relaxed);
    }
}

struct Output {
    stream: cpal::Stream,
    tx: Sender<Cmd>,
    broken: Arc<AtomicBool>,
    playing: bool,
}

fn open_output(active: Arc<AtomicUsize>) -> Option<Output> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (tx, rx) = channel();
    let broken = Arc::new(AtomicBool::new(false));
    let mut mixer = Mixer {
        rx,
        voices: Vec::with_capacity(64),
        out_rate: config.sample_rate as f32,
        out_ch: config.channels as usize,
        active,
    };
    let b = broken.clone();
    let err = move |_e: cpal::Error| b.store(true, Ordering::Relaxed);
    let stream = match format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(config, move |d: &mut [f32], _: &_| mixer.render(d), err, None)
            .ok()?,
        cpal::SampleFormat::I16 => {
            let mut buf = Vec::new();
            device
                .build_output_stream(
                    config,
                    move |d: &mut [i16], _: &_| {
                        buf.resize(d.len(), 0.0);
                        mixer.render(&mut buf);
                        for (o, s) in d.iter_mut().zip(&buf) {
                            *o = (s * 32767.0) as i16;
                        }
                    },
                    err,
                    None,
                )
                .ok()?
        }
        cpal::SampleFormat::I32 => {
            let mut buf = Vec::new();
            device
                .build_output_stream(
                    config,
                    move |d: &mut [i32], _: &_| {
                        buf.resize(d.len(), 0.0);
                        mixer.render(&mut buf);
                        for (o, s) in d.iter_mut().zip(&buf) {
                            *o = (*s as f64 * i32::MAX as f64) as i32;
                        }
                    },
                    err,
                    None,
                )
                .ok()?
        }
        _ => return None,
    };
    Some(Output { stream, tx, broken, playing: false })
}

/// Owns the decoded clips and the output stream. Lives on one thread (cpal streams aren't Send).
pub struct Engine {
    out: Option<Output>,
    active: Arc<AtomicUsize>,
    last_play: Instant,
    clips: HashMap<String, Arc<Clip>>,
    /// Renders into memory instead of a device (for `--render` / `--bench`).
    offline: Option<(Sender<Cmd>, Mixer)>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            out: None,
            active: Arc::new(AtomicUsize::new(0)),
            last_play: Instant::now(),
            clips: HashMap::new(),
            offline: None,
        }
    }

    /// An engine with no device; call `render_offline` to pull stereo samples.
    pub fn new_offline(rate: u32) -> Self {
        let mut e = Self::new();
        let (tx, rx) = channel();
        let mixer = Mixer { rx, voices: Vec::new(), out_rate: rate as f32, out_ch: 2, active: e.active.clone() };
        e.offline = Some((tx, mixer));
        e
    }

    pub fn render_offline(&mut self, frames: usize) -> Vec<f32> {
        let mut buf = vec![0.0; frames * 2];
        if let Some((_, m)) = self.offline.as_mut() {
            m.render(&mut buf);
        }
        buf
    }

    /// Returns a decoded clip, decoding and caching it on first use.
    pub fn clip(&mut self, name: &str) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(name) {
            return Some(c.clone());
        }
        let clip = Arc::new(Clip::decode(crate::assets::audio(name)?)?);
        self.clips.insert(name.to_string(), clip.clone());
        Some(clip)
    }

    /// Decodes `names` ahead of time and drops every other cached clip.
    pub fn set_working_set(&mut self, names: &[String]) {
        self.clips.retain(|k, _| names.contains(k));
        for n in names {
            self.clip(n);
        }
    }

    fn output(&mut self) -> Option<&mut Output> {
        if self.out.as_ref().is_some_and(|o| o.broken.load(Ordering::Relaxed)) {
            self.out = None; // device removed or default changed: reopen on the new default
        }
        if self.out.is_none() {
            self.out = open_output(self.active.clone());
        }
        let out = self.out.as_mut()?;
        if !out.playing {
            out.playing = out.stream.play().is_ok();
        }
        Some(out)
    }

    pub fn play(&mut self, cmd: PlayCmd) {
        self.last_play = Instant::now();
        if let Some((tx, _)) = &self.offline {
            let _ = tx.send(Cmd::Play(cmd));
        } else if let Some(out) = self.output() {
            let _ = out.tx.send(Cmd::Play(cmd));
        }
    }

    pub fn release(&mut self, hold: u32, secs: f32) {
        if let Some((tx, _)) = &self.offline {
            let _ = tx.send(Cmd::Release { hold, secs });
        } else if let Some(out) = self.out.as_ref() {
            let _ = out.tx.send(Cmd::Release { hold, secs });
        }
    }

    pub fn is_running(&self) -> bool {
        self.out.as_ref().is_some_and(|o| o.playing)
    }

    /// Call periodically while running: pauses the stream once everything has been silent a while.
    pub fn tick(&mut self) {
        let idle = self.active.load(Ordering::Relaxed) == 0 && self.last_play.elapsed() > IDLE_PAUSE;
        if let Some(out) = self.out.as_mut() {
            if out.playing && idle {
                out.playing = out.stream.pause().is_err();
            }
        }
    }
}
