//! Decimation from the simulation rate to 48 kHz, and WAV output.

use std::io::Write;

pub struct Decim {
    taps: Vec<f64>,
    buf: Vec<f64>,
    pos: usize,
    k: usize,
    phase: usize,
}

impl Decim {
    pub fn new(k: usize) -> Self {
        let m = 24 * k;
        let n = 2 * m + 1;
        let fc = 0.45 / k as f64; // cycles per input sample
        let mut taps = vec![0.0; n];
        let mut sum = 0.0;
        for i in 0..n {
            let x = i as f64 - m as f64;
            let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x) };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos() + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
            taps[i] = sinc * w;
            sum += taps[i];
        }
        for t in taps.iter_mut() {
            *t /= sum;
        }
        Decim { taps, buf: vec![0.0; n], pos: 0, k, phase: 0 }
    }

    #[inline]
    pub fn push(&mut self, x: f64) -> Option<f64> {
        let n = self.buf.len();
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % n;
        self.phase += 1;
        if self.phase < self.k {
            return None;
        }
        self.phase = 0;
        let mut acc = 0.0;
        let (a, b) = self.buf.split_at(self.pos);
        let mut ti = 0;
        for v in b.iter().chain(a.iter()) {
            acc += v * self.taps[ti];
            ti += 1;
        }
        Some(acc)
    }
}

fn header(w: &mut impl Write, channels: u16, rate: u32, bits: u16, float: bool, frames: usize) -> std::io::Result<()> {
    let block = channels * bits / 8;
    let data = (frames * block as usize) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&(if float { 3u16 } else { 1u16 }).to_le_bytes())?;
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * block as u32).to_le_bytes())?;
    w.write_all(&block.to_le_bytes())?;
    w.write_all(&bits.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())?;
    Ok(())
}

/// Interleaved 16-bit PCM.
pub fn write_wav16(path: &std::path::Path, channels: u16, rate: u32, data: &[f32]) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    header(&mut w, channels, rate, 16, false, data.len() / channels as usize)?;
    for &s in data {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

/// Interleaved 32-bit float (pascals at the microphone, for the stems).
pub fn write_wav_f32(path: &std::path::Path, channels: u16, rate: u32, data: &[f32]) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    header(&mut w, channels, rate, 32, true, data.len() / channels as usize)?;
    for &s in data {
        w.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}
