//! Locate the two fighters in the clean (no-UI) recording, frame by frame.
//!
//! The game camera pans, so plain background subtraction fails. Each frame is
//! scored with two cheap foreground cues:
//!
//! * motion: difference to the previous frame after compensating the global
//!   camera shift (phase correlation). Fighters are never still: idle
//!   animations, hit sparks and Drive auras keep them moving.
//! * colour: saturation. Stages, especially training mode, are far less
//!   saturated than the characters and their effects.
//!
//! The column/row profile of that score gives the extent of the action; its
//! robust midpoint is where the camera centres. Results are cached per file.

use anyhow::{Context, Result};
use rustfft::num_complex::Complex32;
use rayon::prelude::*;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::media::{probe, VideoInfo};
use crate::paths;
use crate::project::TrackingCfg;

pub const ANALYSIS_W: usize = 480;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub left: Vec<f64>,
    pub right: Vec<f64>,
    pub top: Vec<f64>,
    pub bottom: Vec<f64>,
    pub center_x: Vec<f64>,
    pub center_y: Vec<f64>,
    pub strength: Vec<f64>,
}

/// 2-D FFT on a fixed-size grid (rows, then columns).
struct Fft2 {
    w: usize,
    h: usize,
    /// Hann window: suppresses the image-edge discontinuity that biases the peak towards 0.
    window: Vec<f32>,
    row_f: Arc<dyn Fft<f32>>,
    col_f: Arc<dyn Fft<f32>>,
    row_i: Arc<dyn Fft<f32>>,
    col_i: Arc<dyn Fft<f32>>,
}

impl Fft2 {
    fn new(w: usize, h: usize) -> Fft2 {
        let mut p = FftPlanner::new();
        let hann = |n: usize| -> Vec<f32> {
            (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n.max(2) - 1) as f32).cos()).collect()
        };
        let (wx, wy) = (hann(w), hann(h));
        let window = (0..w * h).map(|i| wx[i % w] * wy[i / w]).collect();
        Fft2 { w, h, window, row_f: p.plan_fft_forward(w), col_f: p.plan_fft_forward(h), row_i: p.plan_fft_inverse(w), col_i: p.plan_fft_inverse(h) }
    }

    fn run(&self, data: &mut [Complex32], inverse: bool) {
        let (rf, cf) = if inverse { (&self.row_i, &self.col_i) } else { (&self.row_f, &self.col_f) };
        for row in data.chunks_exact_mut(self.w) {
            rf.process(row);
        }
        let mut col = vec![Complex32::default(); self.h];
        for x in 0..self.w {
            for y in 0..self.h {
                col[y] = data[y * self.w + x];
            }
            cf.process(&mut col);
            for y in 0..self.h {
                data[y * self.w + x] = col[y];
            }
        }
    }

    fn forward(&self, img: &[f32]) -> Vec<Complex32> {
        let mean = img.iter().sum::<f32>() / img.len().max(1) as f32;
        let mut d: Vec<Complex32> = img.iter().zip(&self.window).map(|(&v, &w)| Complex32::new((v - mean) * w, 0.0)).collect();
        self.run(&mut d, false);
        d
    }

    /// Translation (dx, dy) such that `b(x) ~= a(x - d)`.
    fn phase_correlate(&self, fa: &[Complex32], fb: &[Complex32]) -> (f32, f32) {
        let mut r: Vec<Complex32> = fa
            .iter()
            .zip(fb)
            .map(|(a, b)| {
                let c = b * a.conj();
                let m = c.norm();
                if m > 1e-9 {
                    c / m
                } else {
                    Complex32::default()
                }
            })
            .collect();
        self.run(&mut r, true);
        let (w, h) = (self.w as i64, self.h as i64);
        let (mut best, mut bx, mut by) = (f32::MIN, 0i64, 0i64);
        for (i, v) in r.iter().enumerate() {
            if v.re > best {
                best = v.re;
                bx = i as i64 % w;
                by = i as i64 / w;
            }
        }
        // Weighted centroid of the 3x3 neighbourhood for sub-pixel precision.
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let x = (bx + dx).rem_euclid(w);
                let y = (by + dy).rem_euclid(h);
                let v = r[(y * w + x) as usize].re.max(0.0);
                sx += v * dx as f32;
                sy += v * dy as f32;
                sw += v;
            }
        }
        let wrap = |p: i64, n: i64| if p > n / 2 { p - n } else { p };
        let (ox, oy) = if sw > 0.0 { (sx / sw, sy / sw) } else { (0.0, 0.0) };
        (wrap(bx, w) as f32 + ox, wrap(by, h) as f32 + oy)
    }
}

fn extent(profile: &[f32], pct: f64) -> (f64, f64, f64) {
    let total: f64 = profile.iter().map(|&v| v as f64).sum();
    if total <= 1e-6 {
        return (f64::NAN, f64::NAN, 0.0);
    }
    let (lo_t, hi_t) = (pct / 100.0, 1.0 - pct / 100.0);
    let (mut acc, mut lo, mut hi) = (0.0, None, None);
    for (i, &v) in profile.iter().enumerate() {
        acc += v as f64 / total;
        if lo.is_none() && acc >= lo_t {
            lo = Some(i);
        }
        if hi.is_none() && acc >= hi_t {
            hi = Some(i);
        }
    }
    let last = profile.len() - 1;
    (lo.unwrap_or(last) as f64, hi.unwrap_or(last) as f64, total)
}

fn gauss5(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    // OpenCV's 5-tap kernel for sigma = 1.1 (its default for ksize 5).
    let k = [0.0702f32, 0.2311, 0.3974, 0.2311, 0.0702];
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let xx = (x as i64 + i as i64 - 2).clamp(0, w as i64 - 1) as usize;
                s += src[y * w + xx] * kv;
            }
            tmp[y * w + x] = s;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let yy = (y as i64 + i as i64 - 2).clamp(0, h as i64 - 1) as usize;
                s += tmp[yy * w + x] * kv;
            }
            out[y * w + x] = s;
        }
    }
    out
}

/// Morphological opening with a 5x5 ellipse (erode, then dilate).
fn open5(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    // OpenCV's 5x5 ellipse: full middle three rows plus the centre of the outer rows.
    const OFFS: [(i64, i64); 17] = [
        (0, -2), (-2, -1), (-1, -1), (0, -1), (1, -1), (2, -1), (-2, 0), (-1, 0), (0, 0), (1, 0), (2, 0),
        (-2, 1), (-1, 1), (0, 1), (1, 1), (2, 1), (0, 2),
    ];
    let op = |src: &[f32], erode: bool| -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let mut v = if erode { f32::MAX } else { f32::MIN };
                for (dx, dy) in OFFS {
                    let xx = (x + dx).clamp(0, w as i64 - 1);
                    let yy = (y + dy).clamp(0, h as i64 - 1);
                    let s = src[(yy * w as i64 + xx) as usize];
                    v = if erode { v.min(s) } else { v.max(s) };
                }
                out[(y * w as i64 + x) as usize] = v;
            }
        }
        out
    };
    op(&op(src, true), false)
}

fn shift_bilinear(src: &[f32], w: usize, h: usize, dx: f32, dy: f32) -> Vec<f32> {
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let fy = (y as f32 - dy).clamp(0.0, (h - 1) as f32);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(h - 1);
        let ay = fy - y0 as f32;
        for x in 0..w {
            let fx = (x as f32 - dx).clamp(0.0, (w - 1) as f32);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let ax = fx - x0 as f32;
            let top = src[y0 * w + x0] * (1.0 - ax) + src[y0 * w + x1] * ax;
            let bot = src[y1 * w + x0] * (1.0 - ax) + src[y1 * w + x1] * ax;
            out[y * w + x] = top * (1.0 - ay) + bot * ay;
        }
    }
    out
}

/// Per-frame cues computed independently for each frame.
pub struct Features {
    gray: Vec<f32>,
    spec: Option<Vec<Complex32>>,
    color: Vec<f32>,
}

/// (left, right, top, bottom, strength); NaN extents when nothing was found.
pub type Extent = (f64, f64, f64, f64, f64);

/// Scores frames of a fixed analysis size. Frames are independent apart from
/// pairing each one with its predecessor, so chunks are processed in parallel.
pub struct Analyser {
    w: usize,
    h: usize,
    cfg: TrackingCfg,
    fft: Fft2,
}

impl Analyser {
    pub fn new(w: usize, h: usize, cfg: TrackingCfg) -> Analyser {
        Analyser { w, h, cfg, fft: Fft2::new(w, h) }
    }

    pub fn features(&self, rgb: &[u8]) -> Features {
        let n = self.w * self.h;
        let mut gray = vec![0.0f32; n];
        let mut color = vec![0.0f32; n];
        let cw = self.cfg.color_weight as f32;
        for (i, p) in rgb.chunks_exact(3).enumerate() {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            gray[i] = 0.299 * r + 0.587 * g + 0.114 * b;
            if cw > 0.0 {
                let v = r.max(g).max(b);
                let mn = r.min(g).min(b);
                let sat = if v > 40.0 { (v - mn) / v } else { 0.0 };
                color[i] = cw * ((sat - 0.18) / 0.35).clamp(0.0, 1.0);
            }
        }
        let spec = if self.cfg.motion_weight > 0.0 { Some(self.fft.forward(&gray)) } else { None };
        Features { gray, spec, color }
    }

    pub fn score(&self, prev: Option<&Features>, cur: &Features) -> Extent {
        let (w, h) = (self.w, self.h);
        let mut score = cur.color.clone();
        if let (Some(spec), Some(prev)) = (&cur.spec, prev) {
            if let Some(prev_spec) = &prev.spec {
                let (dx, dy) = self.fft.phase_correlate(prev_spec, spec);
                let warped = shift_bilinear(&prev.gray, w, h, dx, dy);
                let diff: Vec<f32> = cur.gray.iter().zip(&warped).map(|(a, b)| (a - b).abs()).collect();
                let mut mask: Vec<f32> = gauss5(&diff, w, h).iter().map(|d| ((d - 10.0) / 25.0).clamp(0.0, 1.0)).collect();
                // Ignore the strip revealed at the border by the camera shift.
                let bx = (dx.abs() as usize + 2).min(w / 2);
                let by = (dy.abs() as usize + 2).min(h / 2);
                for y in 0..h {
                    for x in 0..w {
                        if y < by || y >= h - by || x < bx || x >= w - bx {
                            mask[y * w + x] = 0.0;
                        }
                    }
                }
                let mw = self.cfg.motion_weight as f32;
                for (s, m) in score.iter_mut().zip(open5(&mask, w, h)) {
                    *s += mw * m;
                }
            }
        }
        let mut cols = vec![0.0f32; w];
        let mut rows = vec![0.0f32; h];
        for y in 0..h {
            for x in 0..w {
                let s = score[y * w + x];
                cols[x] += s;
                rows[y] += s;
            }
        }
        let (x0, x1, total) = extent(&cols, self.cfg.percentile);
        let (y0, y1, _) = extent(&rows, self.cfg.percentile);
        (x0, x1, y0, y1, total / (w * h) as f64)
    }

    /// Analyse a chunk of consecutive frames in parallel. `carry` is the last
    /// frame of the previous chunk; returns the extents and the new carry.
    pub fn chunk(&self, carry: Option<Features>, frames: &[Vec<u8>]) -> (Vec<Extent>, Option<Features>) {
        let feats: Vec<Features> = frames.par_iter().map(|f| self.features(f)).collect();
        let extents = (0..feats.len())
            .into_par_iter()
            .map(|i| self.score(if i == 0 { carry.as_ref() } else { Some(&feats[i - 1]) }, &feats[i]))
            .collect();
        (extents, feats.into_iter().last().or(carry))
    }
}

fn cache_path(info: &VideoInfo, cfg: &TrackingCfg) -> std::path::PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let meta = std::fs::metadata(&info.path).ok();
    let abs = std::fs::canonicalize(&info.path).unwrap_or_else(|_| info.path.clone().into());
    abs.hash(&mut h);
    meta.as_ref().map(|m| m.len()).hash(&mut h);
    meta.and_then(|m| m.modified().ok()).hash(&mut h);
    format!("{:?}|{:?}|{:?}|v3", cfg.motion_weight, cfg.color_weight, cfg.percentile).hash(&mut h);
    paths::workspace().join("cache").join(format!("track_{:016x}.json", h.finish()))
}

pub fn cached(path: &str, cfg: &TrackingCfg) -> Option<Track> {
    let info = probe(path).ok()?;
    let text = std::fs::read_to_string(cache_path(&info, cfg)).ok()?;
    serde_json::from_str(&text).ok()
}

fn fill_gaps(v: &[f64], fallback: f64) -> Vec<f64> {
    let good: Vec<usize> = (0..v.len()).filter(|&i| v[i].is_finite()).collect();
    if good.is_empty() {
        return vec![fallback; v.len()];
    }
    (0..v.len())
        .map(|i| {
            if v[i].is_finite() {
                return v[i];
            }
            let k = good.partition_point(|&g| g < i);
            match (k.checked_sub(1).map(|j| good[j]), good.get(k)) {
                (Some(a), Some(&b)) => v[a] + (v[b] - v[a]) * (i - a) as f64 / (b - a) as f64,
                (Some(a), None) => v[a],
                (None, Some(&b)) => v[b],
                (None, None) => fallback,
            }
        })
        .collect()
}

/// Track the action in `path`. `progress` gets 0..1; set `cancel` to stop.
pub fn track_video(path: &str, cfg: &TrackingCfg, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<Track> {
    let info = probe(path)?;
    let cache = cache_path(&info, cfg);
    if let Ok(text) = std::fs::read_to_string(&cache) {
        if let Ok(t) = serde_json::from_str(&text) {
            return Ok(t);
        }
    }
    let aw = ANALYSIS_W;
    let ah = ((info.height as f64 * aw as f64 / info.width.max(1) as f64 / 2.0).round() as usize * 2).max(2);
    let mut child = paths::command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i", path, "-an", "-sn", "-vf"])
        .arg(format!("scale={aw}:{ah}:flags=area"))
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start ffmpeg")?;
    let mut out = child.stdout.take().unwrap();
    let analyser = Analyser::new(aw, ah, cfg.clone());
    let mut rows: Vec<Extent> = vec![];
    let mut carry = None;
    let expected = (info.duration * info.fps).max(1.0);
    let mut eof = false;
    while !eof {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            anyhow::bail!("Cancelled");
        }
        let mut frames = Vec::with_capacity(64);
        while frames.len() < 64 {
            let mut buf = vec![0u8; aw * ah * 3];
            if out.read_exact(&mut buf).is_err() {
                eof = true;
                break;
            }
            frames.push(buf);
        }
        let (ext, c) = analyser.chunk(carry.take(), &frames);
        carry = c;
        rows.extend(ext);
        progress((rows.len() as f64 / expected).min(0.99) as f32);
    }
    let _ = child.wait();
    let (sx, sy) = (info.width as f64 / aw as f64, info.height as f64 / ah as f64);
    let col = |i: usize| rows.iter().map(|r| [r.0, r.1, r.2, r.3][i]).collect::<Vec<f64>>();
    let left: Vec<f64> = fill_gaps(&col(0), aw as f64 / 2.0).iter().map(|v| v * sx).collect();
    let right: Vec<f64> = fill_gaps(&col(1), aw as f64 / 2.0).iter().map(|v| v * sx).collect();
    let top: Vec<f64> = fill_gaps(&col(2), ah as f64 / 2.0).iter().map(|v| v * sy).collect();
    let bottom: Vec<f64> = fill_gaps(&col(3), ah as f64 / 2.0).iter().map(|v| v * sy).collect();
    let track = Track {
        fps: info.fps,
        width: info.width,
        height: info.height,
        frames: rows.len(),
        center_x: left.iter().zip(&right).map(|(a, b)| (a + b) / 2.0).collect(),
        center_y: top.iter().zip(&bottom).map(|(a, b)| (a + b) / 2.0).collect(),
        strength: rows.iter().map(|r| r.4).collect(),
        left,
        right,
        top,
        bottom,
    };
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&cache, serde_json::to_string(&track)?);
    }
    progress(1.0);
    Ok(track)
}

/// Outlier-robust, zero-phase smoothing: median prefilter + Gaussian.
pub fn smooth(values: &[f64], sigma_frames: f64, median: usize) -> Vec<f64> {
    let n = values.len();
    if n == 0 {
        return vec![];
    }
    let mut v = values.to_vec();
    if median > 1 && n >= median {
        let pad = median / 2;
        v = (0..n)
            .map(|i| {
                let mut win: Vec<f64> =
                    (0..median).map(|k| values[(i + k).saturating_sub(pad).min(n - 1)]).collect();
                win.sort_by(f64::total_cmp);
                win[median / 2]
            })
            .collect();
    }
    if sigma_frames <= 0.0 {
        return v;
    }
    let radius = (3.0 * sigma_frames) as usize + 1;
    let kernel: Vec<f64> = (0..=2 * radius)
        .map(|k| {
            let d = k as f64 - radius as f64;
            (-0.5 * (d / sigma_frames).powi(2)).exp()
        })
        .collect();
    let ksum: f64 = kernel.iter().sum();
    (0..n)
        .map(|i| {
            let mut acc = 0.0;
            for (k, w) in kernel.iter().enumerate() {
                let j = (i as i64 + k as i64 - radius as i64).clamp(0, n as i64 - 1) as usize;
                acc += v[j] * w;
            }
            acc / ksum
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textured(w: usize, h: usize, shift: f32) -> Vec<f32> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 - shift, (i / w) as f32);
                100.0 + 40.0 * (x * 0.31).sin() * (y * 0.17).cos() + 30.0 * ((x * 0.05 + y * 0.11).sin())
            })
            .collect()
    }

    #[test]
    fn phase_correlation_sign_and_subpixel() {
        let (w, h) = (96, 64);
        let fft = Fft2::new(w, h);
        let a = fft.forward(&textured(w, h, 0.0));
        let b = fft.forward(&textured(w, h, 3.0));
        let (dx, dy) = fft.phase_correlate(&a, &b);
        assert!((dx - 3.0).abs() < 0.3 && dy.abs() < 0.3, "{dx} {dy}");
    }

    #[test]
    fn smoothing_removes_spikes() {
        let mut v = vec![100.0; 50];
        v[20] = 900.0;
        let s = smooth(&v, 2.0, 7);
        assert!(s.iter().all(|x| (x - 100.0).abs() < 1e-6));
    }

    #[test]
    fn extent_percentiles() {
        let mut p = vec![0.0f32; 100];
        for v in &mut p[30..60] {
            *v = 1.0;
        }
        let (lo, hi, _) = extent(&p, 4.0);
        assert!((30.0..=32.0).contains(&lo) && (57.0..=59.0).contains(&hi), "{lo} {hi}");
    }
}
