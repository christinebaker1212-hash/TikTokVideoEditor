//! Find the Super Art in a synced UI/clean pair.
//!
//! * Animation: during a Level 3 / Critical Art cinematic the game hides the
//!   HUD. Inside the UI elements' source boxes, the UI recording then stops
//!   differing from the clean one. The longest such run is the cinematic:
//!   its start is "Animation start", the frame the HUD returns is "Post start".
//!   Both snap to a nearby hard cut.
//! * Startup: the Super Art flash darkens the screen and freezes the game. We
//!   search back from the cinematic for the start of that darkening/freeze.

use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::camera::resolve_markers;
use crate::media::{probe, ReaderOptions, VideoReader, YuvFrame};
use crate::project::{Project, Rect};
use crate::tracking::{is_cut, thumbnail};

const AW: u32 = 480;

#[derive(Clone, Debug, Default)]
pub struct Detection {
    pub fps: f64,
    pub super_start: f64,
    pub anim_start: f64,
    pub post_start: f64,
    pub confidence: f64,
    /// Whether the startup cue (flash/freeze) was found, or estimated.
    pub startup_found: bool,
    /// Per clean frame: is the HUD visible?
    pub hud: Vec<bool>,
    /// Shot cuts (seconds).
    pub cuts: Vec<f64>,
}

/// Mean absolute UI-vs-clean difference inside the given boxes (analysis pixels).
fn hud_difference(ui: &YuvFrame, clean: &YuvFrame, boxes: &[(usize, usize, usize, usize)]) -> f32 {
    let (mut acc, mut n) = (0.0f64, 0usize);
    let cw = ui.w / 2;
    for &(x0, y0, x1, y1) in boxes {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * ui.w + x;
                let c = (y / 2) * cw + x / 2;
                let dy = (ui.y()[i] as f32 - clean.y()[i] as f32).abs();
                let du = (ui.u()[c] as f32 - clean.u()[c] as f32).abs();
                let dv = (ui.v()[c] as f32 - clean.v()[c] as f32).abs();
                acc += (dy + 0.5 * (du + dv)) as f64;
                n += 1;
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        (acc / n as f64) as f32
    }
}

fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[s.len() / 2]
}

/// Contiguous runs of `true` as [start, end) after closing gaps of <= `gap` frames.
fn runs(flags: &[bool], gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    let mut i = 0;
    while i < flags.len() {
        if !flags[i] {
            i += 1;
            continue;
        }
        let s = i;
        while i < flags.len() && flags[i] {
            i += 1;
        }
        match out.last_mut() {
            Some(last) if s - last.1 <= gap => last.1 = i,
            _ => out.push((s, i)),
        }
    }
    out
}

pub fn detect(project: &Project, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<Detection> {
    if project.sources.ui.is_empty() || project.sources.clean.is_empty() {
        bail!("Super Art detection needs both videos (it looks for the HUD disappearing in the UI recording)");
    }
    let clean_info = probe(&project.sources.clean)?;
    let ui_info = probe(&project.sources.ui)?;
    let fps = clean_info.fps;
    let ah = ((clean_info.height as f64 * AW as f64 / clean_info.width.max(1) as f64 / 2.0).round() as u32 * 2).max(2);
    let opt = ReaderOptions { width: AW, height: ah, fps, hwaccel: false };
    let mut rc = VideoReader::open(&clean_info, 0.0, &opt)?;
    let mut ru = VideoReader::open(&ui_info, project.sync.offset, &opt)?;
    let n = (clean_info.duration * fps).floor().max(1.0) as usize;

    // HUD boxes: elements that are shown during gameplay, in analysis pixels.
    let skin = &project.overlay;
    let k = AW as f64 / skin.reference_size[0].max(1.0);
    let to_box = |r: &Rect| {
        let x0 = ((r.x * k).floor().max(0.0) as usize).min(AW as usize);
        let y0 = ((r.y * k).floor().max(0.0) as usize).min(ah as usize);
        let x1 = (((r.x + r.w) * k).ceil().max(0.0) as usize).min(AW as usize);
        let y1 = (((r.y + r.h) * k).ceil().max(0.0) as usize).min(ah as usize);
        (x0, y0, x1, y1)
    };
    let mut boxes: Vec<_> = skin.elements.iter().filter(|e| e.enabled && e.parts.combo).map(|e| to_box(&e.src)).collect();
    if boxes.is_empty() {
        boxes = skin.elements.iter().map(|e| to_box(&e.src)).collect();
    }
    boxes.retain(|b| b.2 > b.0 && b.3 > b.1);
    if boxes.is_empty() {
        bail!("Add at least one UI element (e.g. the health bar) so the HUD can be found");
    }

    let mut hud = Vec::with_capacity(n);
    let mut luma = Vec::with_capacity(n);
    let mut motion = Vec::with_capacity(n);
    let mut cut_frames = vec![];
    let mut prev_thumb: Option<Vec<f32>> = None;
    for i in 0..n {
        if cancel.load(Ordering::Relaxed) {
            bail!("Cancelled");
        }
        let c = rc.read();
        let u = ru.read();
        if rc.eof {
            break;
        }
        hud.push(hud_difference(&u, &c, &boxes));
        let gray: Vec<f32> = c.y().iter().map(|&v| v as f32).collect();
        luma.push(gray.iter().sum::<f32>() / gray.len() as f32);
        let thumb = thumbnail(&gray, c.w, c.h);
        let m = match &prev_thumb {
            Some(p) => {
                if is_cut(p, &thumb) {
                    cut_frames.push(i);
                }
                p.iter().zip(&thumb).map(|(a, b)| (a - b).abs()).sum::<f32>() / thumb.len() as f32
            }
            None => 0.0,
        };
        motion.push(m);
        prev_thumb = Some(thumb);
        if i % 30 == 0 {
            progress(i as f32 / n as f32);
        }
    }
    let n = hud.len();
    if n < 4 {
        bail!("The video is too short to analyse");
    }

    // HUD on/off: the signal is bimodal (bars drawn vs. identical pixels).
    let mut sorted = hud.clone();
    sorted.sort_by(f32::total_cmp);
    let p90 = sorted[(n * 9 / 10).min(n - 1)];
    let thr = (0.35 * p90).max(6.0);
    let raw: Vec<bool> = hud.iter().map(|&d| d > thr).collect();
    // Median filter (5) to remove single-frame glitches.
    let on: Vec<bool> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(2);
            let hi = (i + 3).min(n);
            raw[lo..hi].iter().filter(|&&b| b).count() * 2 > hi - lo
        })
        .collect();

    let m = resolve_markers(&project.timeline, clean_info.duration);
    let (lo, hi) = (((m.t_in * fps) as usize).min(n), ((m.out * fps).ceil() as usize).min(n));
    let off: Vec<bool> = (0..n).map(|i| i >= lo && i < hi && !on[i]).collect();
    let min_len = (0.4 * fps) as usize;
    let Some(&(a0, a1)) = runs(&off, 2).iter().filter(|r| r.1 - r.0 >= min_len).max_by_key(|r| r.1 - r.0) else {
        bail!("Could not find the Super Art: the HUD never disappears in this clip. Set the markers by hand.");
    };
    if !on[lo.min(n - 1)..a0].iter().any(|&b| b) {
        bail!("The HUD is never visible before the cinematic; check the UI elements' source boxes and the sync");
    }
    let snap = |f: usize| cut_frames.iter().copied().filter(|c| c.abs_diff(f) <= 6).min_by_key(|c| c.abs_diff(f)).unwrap_or(f);
    let anim = snap(a0);
    let post = snap(a1).max(anim + 1);

    // Startup: darkening and/or freeze shortly before the cinematic.
    let win_lo = anim.saturating_sub((4.0 * fps) as usize).max(lo);
    let base_lo = win_lo.saturating_sub((2.0 * fps) as usize).max(lo);
    let baseline_luma = median(&luma[base_lo..win_lo.max(base_lo + 1).min(n)]);
    let baseline_motion = median(&motion[base_lo..win_lo.max(base_lo + 1).min(n)]).max(0.5);
    let dark: Vec<bool> = (0..n).map(|i| i >= win_lo && i < anim && luma[i] < 0.82 * baseline_luma).collect();
    let still: Vec<bool> = (0..n).map(|i| i >= win_lo && i < anim && motion[i] < 0.25 * baseline_motion).collect();
    let near = (1.5 * fps) as usize;
    let candidate = |flags: &[bool], min: usize| {
        runs(flags, 2).into_iter().filter(|r| r.1 - r.0 >= min && r.1 + near >= anim).map(|r| r.0).max()
    };
    let cue = [candidate(&dark, 3), candidate(&still, 5)].into_iter().flatten().min();
    let (startup, startup_found) = match cue {
        Some(f) => (f, true),
        None => (anim.saturating_sub(fps as usize).max(lo), false),
    };

    // Confidence: how cleanly the HUD switches, and whether the startup cue exists.
    let on_mean = median(&hud[lo..a0.max(lo + 1)].iter().zip(&on[lo..a0.max(lo + 1)]).filter(|(_, &o)| o).map(|(d, _)| *d).collect::<Vec<_>>());
    let off_mean = median(&hud[a0..a1]);
    let contrast = if on_mean > 0.0 { ((on_mean - off_mean) / on_mean).clamp(0.0, 1.0) } else { 0.0 };
    let confidence = (0.6 * contrast as f64 + if startup_found { 0.4 } else { 0.1 }).clamp(0.0, 1.0);

    progress(1.0);
    let t = |f: usize| f as f64 / fps;
    Ok(Detection {
        fps,
        super_start: t(startup),
        anim_start: t(anim),
        post_start: t(post),
        confidence,
        startup_found,
        hud: on,
        cuts: cut_frames.iter().map(|&f| t(f)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::runs;

    #[test]
    fn runs_merge_small_gaps() {
        let f = [false, true, true, false, true, true, false, false, false, true];
        assert_eq!(runs(&f, 1), vec![(1, 6), (9, 10)]);
        assert_eq!(runs(&f, 0), vec![(1, 3), (4, 6), (9, 10)]);
    }
}
