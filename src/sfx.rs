//! Procedurally synthesised sound effects.
//!
//! The arcade machine generated all of its audio with discrete analog
//! circuitry, not samples, so shipping .wav assets would be both inauthentic
//! and annoying to distribute. Instead every effect is computed into a float
//! buffer at startup, encoded as a little 16-bit PCM WAV in memory, and handed
//! to macroquad's decoder. Nothing touches the filesystem.

use std::f32::consts::TAU;

use macroquad::audio::{self, PlaySoundParams, Sound};

/// 22 kHz is ample for thumps, bangs and square-wave blips, and keeps each
/// buffer down to a few tens of kilobytes.
const SR: f32 = 22_050.0;

// ---------------------------------------------------------------------------
// WAV container
// ---------------------------------------------------------------------------

fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    let sr = SR as u32;

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels: mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Synthesis primitives
// ---------------------------------------------------------------------------

/// xorshift32 — deterministic white noise, so the sounds are identical on
/// every run and every platform.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    /// Uniform in roughly -1.0..1.0.
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 8_388_607.5 - 1.0
    }
}

fn samples_for(seconds: f32) -> usize {
    (SR * seconds) as usize
}

fn square(phase: f32) -> f32 {
    if phase.sin() >= 0.0 { 1.0 } else { -1.0 }
}

/// Removes the click that an abrupt buffer start or end would produce.
fn fade_edges(buf: &mut [f32], attack: f32, release: f32) {
    let n = buf.len();
    let a = samples_for(attack).max(1).min(n / 2);
    let r = samples_for(release).max(1).min(n / 2);
    for (i, s) in buf.iter_mut().take(a).enumerate() {
        *s *= i as f32 / a as f32;
    }
    for i in 0..r {
        buf[n - 1 - i] *= i as f32 / r as f32;
    }
}

/// Crossfades the tail of a buffer back over its head so the result can be
/// looped without a seam. Returns the shortened, loop-safe buffer.
fn make_loopable(mut buf: Vec<f32>, fade: f32) -> Vec<f32> {
    let n = buf.len();
    let f = samples_for(fade).min(n / 3).max(1);
    for i in 0..f {
        let w = i as f32 / f as f32;
        buf[i] = buf[i] * w + buf[n - f + i] * (1.0 - w);
    }
    buf.truncate(n - f);
    buf
}

/// One-pole low-pass. `cutoff` in Hz; returns the coefficient for `y += a*(x-y)`.
fn lp_coeff(cutoff: f32) -> f32 {
    1.0 - (-TAU * cutoff / SR).exp()
}

// ---------------------------------------------------------------------------
// The effects themselves
// ---------------------------------------------------------------------------

/// One half of the two-note heartbeat that underpins every wave.
fn beat(freq: f32) -> Vec<f32> {
    let dur = 0.17;
    let n = samples_for(dur);
    let mut buf = Vec::with_capacity(n);
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let env = (-t * 13.0).exp();
        // Square fundamental for the body, plus an octave of sine so the thump
        // survives on speakers with no bass response.
        let s = 0.62 * square(phase) + 0.22 * (phase * 2.0).sin();
        buf.push(s * env * 0.9);
        phase += TAU * freq / SR;
    }
    fade_edges(&mut buf, 0.002, 0.02);
    buf
}

/// Descending square-wave blip.
fn fire() -> Vec<f32> {
    let dur = 0.26;
    let n = samples_for(dur);
    let mut buf = Vec::with_capacity(n);
    let mut rng = Rng::new(0x51E7);
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let k = t / dur;
        let freq = 1150.0 * (1.0 - k) + 150.0 * k; // linear sweep down
        let env = (-t * 11.0).exp();
        let s = 0.5 * square(phase) + 0.08 * rng.unit();
        buf.push(s * env * 0.55);
        phase += TAU * freq / SR;
    }
    fade_edges(&mut buf, 0.001, 0.015);
    buf
}

/// Filtered noise burst. Lower `cutoff` and longer `dur` read as a bigger rock.
fn bang(dur: f32, cutoff: f32, thump: Option<f32>, seed: u32) -> Vec<f32> {
    let n = samples_for(dur);
    let mut buf = Vec::with_capacity(n);
    let mut rng = Rng::new(seed);
    let a = lp_coeff(cutoff);
    let mut y = 0.0;
    let mut phase: f32 = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        y += a * (rng.unit() - y);
        let env = (-t / (dur * 0.33)).exp();
        let mut s = y * 1.9;
        if let Some(f) = thump {
            s += 0.5 * (phase).sin() * (-t * 9.0).exp();
            phase += TAU * f / SR;
        }
        buf.push(s * env * 0.7);
    }
    fade_edges(&mut buf, 0.001, 0.03);
    buf
}

/// The ship coming apart: deeper and longer than any rock.
fn ship_explode() -> Vec<f32> {
    let dur = 1.0;
    let n = samples_for(dur);
    let mut buf = Vec::with_capacity(n);
    let mut rng = Rng::new(0xBEEF);
    let mut y = 0.0;
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let k = t / dur;
        y += lp_coeff(1400.0 * (1.0 - k) + 180.0) * (rng.unit() - y);
        let env = (-t * 3.2).exp();
        let freq = 220.0 * (1.0 - k) + 45.0 * k;
        let s = y * 2.0 + 0.45 * square(phase);
        phase += TAU * freq / SR;
        buf.push(s * env * 0.75);
    }
    fade_edges(&mut buf, 0.002, 0.08);
    buf
}

/// Steady rumble, looped for as long as the thrust key is held.
fn thrust() -> Vec<f32> {
    let n = samples_for(0.3);
    let mut buf = Vec::with_capacity(n);
    let mut rng = Rng::new(0x7A51);
    let mut y = 0.0;
    let a = lp_coeff(780.0);
    for _ in 0..n {
        y += a * (rng.unit() - y);
        buf.push(y * 2.2 * 0.5);
    }
    make_loopable(buf, 0.03)
}

/// Looped warble for a flying saucer. `base` sets which one.
fn saucer(base: f32) -> Vec<f32> {
    // 0.5 s holds exactly four cycles of the 8 Hz warble, so the modulation
    // lines up with the loop point.
    let n = samples_for(0.5);
    let mut buf = Vec::with_capacity(n);
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let freq = base * (1.0 + 0.3 * (TAU * 8.0 * t).sin());
        buf.push(square(phase) * 0.3);
        phase += TAU * freq / SR;
    }
    make_loopable(buf, 0.02)
}

/// Rising arpeggio awarded with every extra ship.
fn extra_life() -> Vec<f32> {
    let notes = [523.25, 659.25, 783.99, 1046.5];
    let each = 0.1;
    let n = samples_for(each * notes.len() as f32);
    let mut buf = Vec::with_capacity(n);
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let idx = ((t / each) as usize).min(notes.len() - 1);
        let local = t - idx as f32 * each;
        let env = (-local * 9.0).exp();
        buf.push(square(phase) * env * 0.3);
        phase += TAU * notes[idx] / SR;
    }
    fade_edges(&mut buf, 0.002, 0.03);
    buf
}

/// Filter sweep for the hyperspace jump.
fn hyperspace() -> Vec<f32> {
    let dur = 0.45;
    let n = samples_for(dur);
    let mut buf = Vec::with_capacity(n);
    let mut rng = Rng::new(0x4D21);
    let mut y = 0.0;
    let mut phase: f32 = 0.0;
    for i in 0..n {
        let t = i as f32 / SR;
        let k = t / dur;
        y += lp_coeff(5000.0 * (1.0 - k) + 150.0) * (rng.unit() - y);
        let freq = 900.0 * (1.0 - k) + 1800.0 * k;
        let env = (1.0 - k).powf(0.7);
        buf.push((y * 1.6 + 0.25 * phase.sin()) * env * 0.6);
        phase += TAU * freq / SR;
    }
    fade_edges(&mut buf, 0.003, 0.04);
    buf
}

// ---------------------------------------------------------------------------
// Sound bank
// ---------------------------------------------------------------------------

/// Which rock blew up, for picking the right bang.
#[derive(Copy, Clone)]
pub enum BangSize {
    Large,
    Medium,
    Small,
}

/// Owns every decoded sound plus the state of the two looping channels.
///
/// Audio is strictly optional: if a device is missing or a decode fails, the
/// fields stay `None` and every method becomes a no-op rather than a crash.
pub struct SoundBank {
    beat_lo: Option<Sound>,
    beat_hi: Option<Sound>,
    fire: Option<Sound>,
    bang_large: Option<Sound>,
    bang_medium: Option<Sound>,
    bang_small: Option<Sound>,
    ship_explode: Option<Sound>,
    thrust: Option<Sound>,
    saucer_big: Option<Sound>,
    saucer_small: Option<Sound>,
    extra_life: Option<Sound>,
    hyperspace: Option<Sound>,

    muted: bool,
    thrust_on: bool,
    /// `Some(true)` = small saucer looping, `Some(false)` = large.
    saucer_on: Option<bool>,
}

async fn load(samples: Vec<f32>) -> Option<Sound> {
    audio::load_sound_from_bytes(&encode_wav(&samples))
        .await
        .ok()
}

impl SoundBank {
    pub async fn load_all() -> Self {
        Self {
            beat_lo: load(beat(50.0)).await,
            beat_hi: load(beat(62.0)).await,
            fire: load(fire()).await,
            bang_large: load(bang(0.6, 520.0, Some(70.0), 0x11)).await,
            bang_medium: load(bang(0.45, 950.0, None, 0x22)).await,
            bang_small: load(bang(0.3, 1700.0, None, 0x33)).await,
            ship_explode: load(ship_explode()).await,
            thrust: load(thrust()).await,
            saucer_big: load(saucer(190.0)).await,
            saucer_small: load(saucer(330.0)).await,
            extra_life: load(extra_life()).await,
            hyperspace: load(hyperspace()).await,
            muted: false,
            thrust_on: false,
            saucer_on: None,
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            self.silence_loops();
        }
    }

    fn one_shot(&self, slot: &Option<Sound>, volume: f32) {
        if self.muted {
            return;
        }
        if let Some(s) = slot {
            audio::play_sound(
                s,
                PlaySoundParams {
                    looped: false,
                    volume,
                },
            );
        }
    }

    pub fn fire(&self) {
        self.one_shot(&self.fire, 0.6);
    }

    pub fn bang(&self, size: BangSize) {
        match size {
            BangSize::Large => self.one_shot(&self.bang_large, 0.8),
            BangSize::Medium => self.one_shot(&self.bang_medium, 0.7),
            BangSize::Small => self.one_shot(&self.bang_small, 0.6),
        }
    }

    pub fn ship_explode(&self) {
        self.one_shot(&self.ship_explode, 0.9);
    }

    pub fn extra_life(&self) {
        self.one_shot(&self.extra_life, 0.7);
    }

    pub fn hyperspace(&self) {
        self.one_shot(&self.hyperspace, 0.7);
    }

    /// `high` alternates the two notes of the heartbeat.
    pub fn beat(&self, high: bool) {
        let slot = if high { &self.beat_hi } else { &self.beat_lo };
        self.one_shot(slot, 0.85);
    }

    /// Idempotent: safe to call every frame with the current thrust state.
    pub fn set_thrust(&mut self, on: bool) {
        let on = on && !self.muted;
        if on == self.thrust_on {
            return;
        }
        self.thrust_on = on;
        if let Some(s) = &self.thrust {
            if on {
                audio::play_sound(
                    s,
                    PlaySoundParams {
                        looped: true,
                        volume: 0.45,
                    },
                );
            } else {
                audio::stop_sound(s);
            }
        }
    }

    /// `None` stops the saucer loop; `Some(small)` starts the matching one.
    pub fn set_saucer(&mut self, kind: Option<bool>) {
        let kind = if self.muted { None } else { kind };
        if kind == self.saucer_on {
            return;
        }
        if let Some(prev) = self.saucer_on
            && let Some(s) = self.saucer_slot(prev)
        {
            audio::stop_sound(s);
        }
        if let Some(next) = kind
            && let Some(s) = self.saucer_slot(next)
        {
            audio::play_sound(
                s,
                PlaySoundParams {
                    looped: true,
                    volume: 0.4,
                },
            );
        }
        self.saucer_on = kind;
    }

    fn saucer_slot(&self, small: bool) -> Option<&Sound> {
        if small {
            self.saucer_small.as_ref()
        } else {
            self.saucer_big.as_ref()
        }
    }

    /// Stops the looping channels, e.g. on death, pause or game over.
    pub fn silence_loops(&mut self) {
        self.set_thrust(false);
        self.set_saucer(None);
    }
}
