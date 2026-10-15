//! Built-in generated sound clips (so samples work without audio files) and a WAV encoder.

use std::f32::consts::TAU;

/// 16-bit mono PCM WAV bytes.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

const RATE: u32 = 44100;

/// Seamless one-second engine loop at ~30 Hz firing frequency (pitch it with rpm).
pub fn engine_loop() -> Vec<u8> {
    let n = RATE as usize;
    let mut rng = Rng(0x1234_5678);
    let base = 30.0; // whole number of cycles per second keeps the loop seamless
    let s: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let p = base * t * TAU;
            let fire = (p.sin() * 0.55
                + (2.0 * p).sin() * 0.3
                + (3.0 * p + 0.5).sin() * 0.2
                + (5.0 * p).sin() * 0.1)
                * (0.8 + 0.2 * (0.5 * p).sin());
            (fire * 0.6 + rng.next() * 0.04).tanh() * 0.7
        })
        .collect();
    wav(&s, RATE)
}

/// Short gunshot: noise burst with a fast decay and a low thump.
pub fn gunshot() -> Vec<u8> {
    let n = (RATE as f32 * 0.6) as usize;
    let mut rng = Rng(0x9e37_79b9);
    let mut lp = 0.0f32;
    let s: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let noise = rng.next();
            lp += (noise - lp) * (0.6 - t).clamp(0.08, 0.6);
            let env = (-t * 18.0).exp();
            let thump = (TAU * 70.0 * t).sin() * (-t * 25.0).exp();
            (lp * env * 1.4 + thump * 0.8).clamp(-1.0, 1.0)
        })
        .collect();
    wav(&s, RATE)
}

/// UI beep (880 Hz, 0.12 s).
pub fn beep() -> Vec<u8> {
    let n = (RATE as f32 * 0.12) as usize;
    let s: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            (TAU * 880.0 * t).sin() * 0.4 * (1.0 - t / 0.12).min(t * 200.0).min(1.0)
        })
        .collect();
    wav(&s, RATE)
}

/// Two-second seamless wind / ambience loop (filtered noise with slow gusts).
pub fn wind_loop() -> Vec<u8> {
    let n = RATE as usize * 2;
    let mut rng = Rng(0x2545_f491);
    let mut lp = 0.0f32;
    let mut s: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            lp += (rng.next() - lp) * 0.02;
            lp * (0.6 + 0.4 * (TAU * 0.5 * t).sin()) * 2.5
        })
        .collect();
    // Crossfade the ends so the loop has no click.
    let fade = 2000;
    for i in 0..fade {
        let a = i as f32 / fade as f32;
        let tail = s[n - fade + i];
        s[i] = s[i] * a + tail * (1.0 - a);
    }
    s.truncate(n - fade);
    wav(&s, RATE)
}

/// Tire skid: band-limited noise loop.
pub fn skid_loop() -> Vec<u8> {
    let n = RATE as usize;
    let mut rng = Rng(0xdead_beef);
    let mut a = 0.0f32;
    let mut b = 0.0f32;
    let s: Vec<f32> = (0..n)
        .map(|_| {
            a += (rng.next() - a) * 0.35;
            b += (a - b) * 0.2;
            (a - b) * 1.8
        })
        .collect();
    wav(&s, RATE)
}
