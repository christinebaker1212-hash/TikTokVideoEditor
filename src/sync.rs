//! Align the UI and clean recordings on their audio transients.
//!
//! Both carry the same game audio (no music), so hits, whiffs and announcer
//! lines line up once the clips are offset correctly. An onset-strength
//! envelope (spectral flux) of each is cross-correlated for a coarse offset,
//! then GCC-PHAT on the raw audio refines it to the sample.
//!
//! Convention: `ui_time = clean_time + offset`.

use anyhow::{bail, Result};
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use crate::media::load_audio;

pub const SR: u32 = 16000;
pub const HOP: usize = 160; // 10 ms
const N_FFT: usize = 1024;

pub fn onset_envelope(x: &[f32]) -> Vec<f32> {
    let mut x = x.to_vec();
    if x.len() < N_FFT {
        x.resize(N_FFT, 0.0);
    }
    let n_frames = 1 + (x.len() - N_FFT) / HOP;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let window: Vec<f32> =
        (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N_FFT - 1) as f32).cos()).collect();
    let bins = N_FFT / 2 + 1;
    let mut prev = vec![0.0f32; bins];
    let mut flux = Vec::with_capacity(n_frames);
    let mut buf = vec![Complex32::default(); N_FFT];
    for f in 0..n_frames {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = Complex32::new(x[f * HOP + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        let mut s = 0.0;
        for k in 0..bins {
            let m = (1.0 + 100.0 * buf[k].norm()).ln();
            if f > 0 {
                s += (m - prev[k]).max(0.0);
            }
            prev[k] = m;
        }
        flux.push(s);
    }
    // Remove the slowly varying floor so only sharp transients remain.
    let k = ((0.25 * SR as f32 / HOP as f32) as usize).max(1);
    let n = flux.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + flux[i] as f64;
    }
    let env: Vec<f32> = (0..n)
        .map(|i| {
            let lo = (i as i64 - (k as i64 - 1) / 2).max(0) as usize;
            let hi = (i + k / 2 + 1).min(n);
            let floor = (prefix[hi] - prefix[lo]) / k as f64;
            (flux[i] - floor as f32).max(0.0)
        })
        .collect();
    let peak = env.iter().cloned().fold(0.0f32, f32::max);
    if peak > 0.0 {
        env.iter().map(|v| v / peak).collect()
    } else {
        env
    }
}

/// c[lag] = sum a[n + lag] * b[n] for lag in -(len b - 1)..len a.
fn xcorr(a: &[f32], b: &[f32]) -> (Vec<i64>, Vec<f32>) {
    let n = a.len() + b.len() - 1;
    let size = n.next_power_of_two();
    let mut p = FftPlanner::<f32>::new();
    let (f, inv) = (p.plan_fft_forward(size), p.plan_fft_inverse(size));
    let mut fa: Vec<Complex32> = a.iter().map(|&v| Complex32::new(v, 0.0)).collect();
    fa.resize(size, Complex32::default());
    let mut fb: Vec<Complex32> = b.iter().map(|&v| Complex32::new(v, 0.0)).collect();
    fb.resize(size, Complex32::default());
    f.process(&mut fa);
    f.process(&mut fb);
    let mut c: Vec<Complex32> = fa.iter().zip(&fb).map(|(x, y)| x * y.conj()).collect();
    inv.process(&mut c);
    let mut lags = vec![];
    let mut vals = vec![];
    for lag in -(b.len() as i64 - 1)..a.len() as i64 {
        lags.push(lag);
        vals.push(c[lag.rem_euclid(size as i64) as usize].re / size as f32);
    }
    (lags, vals)
}

#[derive(Clone, Debug)]
pub struct SyncResult {
    pub offset: f64,
    pub coarse: f64,
    pub confidence: f64,
    /// Onset envelopes (10 ms resolution) for the visual check.
    pub env_ui: Vec<f32>,
    pub env_clean: Vec<f32>,
}

pub fn estimate_offset(ui: &[f32], clean: &[f32], max_lag: f64) -> SyncResult {
    let (eu, ec) = (onset_envelope(ui), onset_envelope(clean));
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (mu, mc) = (mean(&eu), mean(&ec));
    let a: Vec<f32> = eu.iter().map(|v| v - mu).collect();
    let b: Vec<f32> = ec.iter().map(|v| v - mc).collect();
    let (lags, mut c) = xcorr(&a, &b);
    let max_k = (max_lag * SR as f64 / HOP as f64) as i64;
    for (l, v) in lags.iter().zip(c.iter_mut()) {
        if l.abs() > max_k {
            *v = f32::MIN;
        }
    }
    let best = (0..c.len()).max_by(|&i, &j| c[i].total_cmp(&c[j])).unwrap_or(0);
    let coarse = lags[best] as f64 * HOP as f64 / SR as f64;
    // Confidence: how far the winning peak stands out from the runner-up.
    let guard = (0.05 * SR as f64 / HOP as f64) as usize + 1;
    let second = c
        .iter()
        .enumerate()
        .filter(|(i, _)| i.abs_diff(best) > guard)
        .map(|(_, v)| *v)
        .fold(f32::MIN, f32::max);
    let confidence = if c[best] > 0.0 { (1.0 - second.max(0.0) / c[best]).clamp(0.0, 1.0) as f64 } else { 0.0 };
    let offset = coarse + refine(ui, clean, coarse, 0.03);
    SyncResult { offset, coarse, confidence, env_ui: eu, env_clean: ec }
}

/// GCC-PHAT on the overlapping audio, searched within +-window of `coarse`.
fn refine(ui: &[f32], clean: &[f32], coarse: f64, window: f64) -> f64 {
    let sr = SR as i64;
    let shift = (coarse * SR as f64).round() as i64;
    let start_c = (-shift).max(0);
    let mut end_c = (clean.len() as i64).min(ui.len() as i64 - shift);
    if end_c - start_c < sr / 2 {
        return 0.0;
    }
    end_c = end_c.min(start_c + 60 * sr);
    let a = &ui[(start_c + shift) as usize..(end_c + shift) as usize];
    let b = &clean[start_c as usize..end_c as usize];
    let size = (2 * a.len()).next_power_of_two();
    let mut p = FftPlanner::<f32>::new();
    let (f, inv) = (p.plan_fft_forward(size), p.plan_fft_inverse(size));
    let mut fa: Vec<Complex32> = a.iter().map(|&v| Complex32::new(v, 0.0)).collect();
    fa.resize(size, Complex32::default());
    let mut fb: Vec<Complex32> = b.iter().map(|&v| Complex32::new(v, 0.0)).collect();
    fb.resize(size, Complex32::default());
    f.process(&mut fa);
    f.process(&mut fb);
    let mut r: Vec<Complex32> = fa
        .iter()
        .zip(&fb)
        .map(|(x, y)| {
            let c = x * y.conj();
            c / (c.norm() + 1e-12)
        })
        .collect();
    inv.process(&mut r);
    let w = (window * SR as f64) as i64;
    let best = (-w..=w).max_by(|&i, &j| {
        r[i.rem_euclid(size as i64) as usize].re.total_cmp(&r[j.rem_euclid(size as i64) as usize].re)
    });
    best.unwrap_or(0) as f64 / SR as f64
}

/// Estimate the offset between two recordings from their first `analyse` seconds.
pub fn sync_files(ui_path: &str, clean_path: &str, analyse: f64) -> Result<SyncResult> {
    let ui = load_audio(ui_path, SR, 1, 0.0, Some(analyse))?;
    let clean = load_audio(clean_path, SR, 1, 0.0, Some(analyse))?;
    if ui.is_empty() || clean.is_empty() {
        bail!("Both videos need an audio track to auto-sync");
    }
    Ok(estimate_offset(&ui, &clean, 20.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clicks(lead: f64, secs: f64, seed: u64) -> Vec<f32> {
        let n = (secs * SR as f64) as usize;
        let mut s = seed;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64 / (1u64 << 31) as f64) as f32 - 0.5
        };
        let mut x: Vec<f32> = (0..n).map(|_| 0.02 * rnd()).collect();
        for c in [1.0, 1.7, 2.2, 3.1, 3.6, 4.4] {
            let i = ((c - lead) * SR as f64) as i64;
            if i < 0 {
                continue;
            }
            for k in 0..800usize {
                let t = k as f32 / SR as f32;
                if let Some(v) = x.get_mut(i as usize + k) {
                    *v += 0.8 * (-t * 60.0).exp() * (2.0 * std::f32::consts::PI * 900.0 * t).sin();
                }
            }
        }
        x
    }

    #[test]
    fn finds_positive_and_negative_offsets() {
        let ui = clicks(0.0, 5.0, 1);
        let clean = clicks(0.5, 5.0, 2); // clean starts 0.5 s later
        let r = estimate_offset(&ui, &clean, 20.0);
        assert!((r.offset - 0.5).abs() < 0.002, "{r:?}");
        assert!(r.confidence > 0.3);
        let r = estimate_offset(&clean, &ui, 20.0);
        assert!((r.offset + 0.5).abs() < 0.002, "{}", r.offset);
    }
}
