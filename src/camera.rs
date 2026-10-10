//! Project + tracking -> per-frame camera path and overlay visibility.
//!
//! The camera is a crop window: centre (cx, cy) in source pixels and a zoom.
//! At zoom 1 the crop is the full source height at 9:16 (607.5 x 1080 for
//! 1080p), so gameplay is never rescaled relative to itself.
//!
//!   combo      [in, superStart)          auto-centre on both fighters
//!   startup    [superStart, animStart)   manual keyframes
//!   animation  [animStart, postStart)    manual keyframes
//!   post       [postStart, out]          auto-centre + user offset / zoom
//!
//! Preview and export both sample the same `Timeline`.

use std::collections::HashMap;

use crate::project::{Keyframe, Markers, Part, Project, Skin, OUT_ASPECT};
use crate::tracking::{smooth, smooth_segments, Track};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub t_in: f64,
    pub super_start: f64,
    pub anim_start: f64,
    pub post_start: f64,
    pub out: f64,
}

impl Resolved {
    pub fn part_at(&self, t: f64) -> Part {
        if t >= self.post_start {
            Part::Post
        } else if t >= self.anim_start {
            Part::Animation
        } else if t >= self.super_start {
            Part::Startup
        } else {
            Part::Combo
        }
    }
}

/// Fill in unset markers and force them into order.
pub fn resolve_markers(m: &Markers, duration: f64) -> Resolved {
    let t_in = m.t_in.clamp(0.0, duration.max(0.0));
    let out = m.out.unwrap_or(duration).clamp(t_in, duration.max(t_in));
    let ss = m.super_start.map_or(out, |v| v.clamp(t_in, out));
    let an = m.anim_start.map_or(ss, |v| v.clamp(ss, out));
    let ps = m.post_start.map_or(out, |v| v.clamp(an, out));
    Resolved { t_in, super_start: ss, anim_start: an, post_start: ps, out }
}

pub fn crop_size(zoom: f64, src_h: f64) -> (f64, f64) {
    let ch = src_h / zoom.max(1e-6);
    (ch * OUT_ASPECT, ch)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
}

/// Smallest zoom: the crop is then as wide as the source (zoom < 1 means the
/// picture is shrunk to fit and the space above/below is filled).
pub fn min_zoom(src_w: f64, src_h: f64) -> f64 {
    (src_h * OUT_ASPECT / src_w).min(1.0)
}

pub fn clamp_cam(c: Cam, src_w: f64, src_h: f64) -> Cam {
    let zoom = c.zoom.clamp(min_zoom(src_w, src_h), 8.0);
    let (cw, ch) = crop_size(zoom, src_h);
    let cw = cw.min(src_w);
    // Zoomed out, the whole source height is visible: centre it vertically.
    let cy = if zoom < 1.0 { src_h / 2.0 } else { c.cy.clamp(ch / 2.0, src_h - ch / 2.0) };
    Cam { cx: c.cx.clamp(cw / 2.0, src_w - cw / 2.0), cy, zoom }
}

/// Running min (or max) over a window of +-`r` samples.
fn running(v: &[f64], r: usize, max: bool) -> Vec<f64> {
    (0..v.len())
        .map(|i| {
            let w = &v[i.saturating_sub(r)..(i + r + 1).min(v.len())];
            if max { w.iter().cloned().fold(f64::MIN, f64::max) } else { w.iter().cloned().fold(f64::MAX, f64::min) }
        })
        .collect()
}

/// Run `f` on each shot (split at `cuts`) separately.
fn per_shot(v: &[f64], cuts: &[usize], f: impl Fn(&[f64]) -> Vec<f64>) -> Vec<f64> {
    let mut out = Vec::with_capacity(v.len());
    let mut start = 0;
    for &c in cuts.iter().filter(|&&c| c > 0 && c < v.len()).chain(std::iter::once(&v.len())) {
        if c > start {
            out.extend(f(&v[start..c]));
            start = c;
        }
    }
    out
}

/// Fighter positions on the timeline: smoothed centre and a conservative
/// left/right envelope of the action (for keeping both fighters in frame).
struct Action {
    cx: Vec<f64>,
    cy: Vec<f64>,
    left: Vec<f64>,
    right: Vec<f64>,
    /// Largest zoom that still fits both fighters plus the margin.
    fit_zoom: Vec<f64>,
}

pub fn ease(name: &str, w: f64) -> f64 {
    match name {
        "linear" => w,
        "easeIn" => w * w,
        "easeOut" => 1.0 - (1.0 - w) * (1.0 - w),
        "hold" => 0.0,
        _ => 0.5 - 0.5 * (std::f64::consts::PI * w).cos(),
    }
}

fn keyframe_path(times: &[f64], t0: f64, start: Cam, keyframes: &[Keyframe], default_ease: &str) -> Vec<Cam> {
    let mut keys: Vec<(f64, Cam, String)> = vec![(t0, start, default_ease.to_string())];
    let mut sorted: Vec<&Keyframe> = keyframes.iter().collect();
    sorted.sort_by(|a, b| a.t.total_cmp(&b.t));
    for k in sorted {
        let cam = Cam { cx: k.cx, cy: k.cy, zoom: k.zoom };
        let e = k.ease.clone().unwrap_or_else(|| default_ease.to_string());
        if (k.t - t0).abs() < 1e-6 {
            keys[0] = (t0, cam, e); // an explicit keyframe at the start replaces the implicit one
        } else if k.t > t0 {
            keys.push((k.t, cam, e));
        }
    }
    times
        .iter()
        .map(|&t| {
            if t < keys[0].0 {
                return keys[0].1;
            }
            for pair in keys.windows(2) {
                let ((ta, a, _), (tb, b, e)) = (&pair[0], &pair[1]);
                if t >= *ta && t < *tb {
                    let w = ease(e, (t - ta) / (tb - ta));
                    return Cam { cx: a.cx + (b.cx - a.cx) * w, cy: a.cy + (b.cy - a.cy) * w, zoom: a.zoom + (b.zoom - a.zoom) * w };
                }
            }
            keys.last().unwrap().1
        })
        .collect()
}

fn blend(a: Cam, b: Cam, t: f64, start: f64, dur: f64) -> Cam {
    if dur <= 0.0 {
        return b;
    }
    let w = ((t - start) / dur).clamp(0.0, 1.0);
    let w = 0.5 - 0.5 * (std::f64::consts::PI * w).cos();
    Cam { cx: a.cx + (b.cx - a.cx) * w, cy: a.cy + (b.cy - a.cy) * w, zoom: a.zoom + (b.zoom - a.zoom) * w }
}

#[derive(Clone, Debug)]
pub struct Timeline {
    pub fps: f64,
    pub markers: Resolved,
    pub src_w: f64,
    pub src_h: f64,
    pub cams: Vec<Cam>,
    pub parts: Vec<Part>,
    pub alphas: HashMap<String, Vec<f32>>,
    pub tracked: bool,
}

#[derive(Clone, Debug)]
pub struct FrameCam {
    pub cam: Cam,
    pub part: Part,
    pub alphas: HashMap<String, f32>,
}

impl Timeline {
    pub fn len(&self) -> usize {
        self.cams.len()
    }

    pub fn time_of(&self, i: usize) -> f64 {
        self.markers.t_in + i as f64 / self.fps
    }

    pub fn index_at(&self, t: f64) -> usize {
        let i = ((t - self.markers.t_in) * self.fps + 1e-6).floor();
        (i.max(0.0) as usize).min(self.cams.len().saturating_sub(1))
    }

    pub fn frame(&self, i: usize) -> FrameCam {
        let i = i.min(self.cams.len().saturating_sub(1));
        FrameCam {
            cam: self.cams[i],
            part: self.parts[i],
            alphas: self.alphas.iter().map(|(k, v)| (k.clone(), v[i])).collect(),
        }
    }

    pub fn sample(&self, t: f64) -> FrameCam {
        self.frame(self.index_at(t))
    }
}

pub fn build_timeline(project: &Project, track: Option<&Track>, fps: f64, src_w: f64, src_h: f64, duration: f64) -> Timeline {
    let markers = resolve_markers(&project.timeline, duration);
    let cam = &project.camera;
    let n = (((markers.out - markers.t_in) * fps).round() as usize).max(1);
    let times: Vec<f64> = (0..n).map(|i| markers.t_in + i as f64 / fps).collect();

    let framing = &cam.framing;
    let margin = framing.margin.max(0.0);
    let zmin = min_zoom(src_w, src_h);
    let full_w = src_h * OUT_ASPECT; // crop width at zoom 1
    let action = |smoothing: f64| -> Action {
        match track {
            Some(tr) if tr.frames > 0 => {
                let tfps = if tr.fps > 0.0 { tr.fps } else { fps };
                let tt: Vec<f64> = (0..tr.frames).map(|k| k as f64 / tfps).collect();
                let (kx, ky) = (src_w / tr.width as f64, src_h / tr.height as f64);
                // Missing series (hand-built tracks) fall back to the centre.
                let series = |v: &[f64], k: f64| {
                    let v = if v.len() == tr.frames { v } else { &tr.center_x };
                    v.iter().map(|x| x * k).collect::<Vec<f64>>()
                };
                let scale = |v: &[f64], k: f64| series(v, k);
                let (rx, ry) = (scale(&tr.center_x, kx), if tr.center_y.len() == tr.frames { scale(&tr.center_y, ky) } else { vec![src_h / 2.0; tr.frames] });
                let sigma = smoothing * tfps;
                let sx = smooth_segments(&rx, sigma, 7, &tr.cuts);
                let sy = smooth_segments(&ry, sigma, 7, &tr.cuts);
                // Envelope: widen first (running min/max), then smooth, so the
                // frame opens up a little before a fighter would leave it.
                let r = (0.4 * tfps) as usize;
                let left = per_shot(&scale(&tr.left, kx), &tr.cuts, |s| smooth(&running(s, r, false), 0.25 * tfps, 1));
                let right = per_shot(&scale(&tr.right, kx), &tr.cuts, |s| smooth(&running(s, r, true), 0.25 * tfps, 1));
                let fit: Vec<f64> = left.iter().zip(&right).map(|(l, r)| (full_w / (r - l + 2.0 * margin).max(1.0)).max(zmin)).collect();
                // Zoom changes are slow and start early: running min, then smooth.
                let fit = per_shot(&fit, &tr.cuts, |s| smooth(&running(s, (0.5 * tfps) as usize, false), 0.3 * tfps, 1));
                let at = |v: &[f64]| times.iter().map(|&t| interp(t, &tt, v)).collect::<Vec<f64>>();
                Action { cx: at(&sx), cy: at(&sy), left: at(&left), right: at(&right), fit_zoom: at(&fit) }
            }
            _ => Action {
                cx: vec![src_w / 2.0; n],
                cy: vec![src_h / 2.0; n],
                left: vec![src_w / 2.0; n],
                right: vec![src_w / 2.0; n],
                fit_zoom: vec![f64::MAX; n],
            },
        }
    };
    // Gameplay camera: centre on the fighters and, if enabled, zoom out just
    // enough to keep both in frame; offsets can never push one out.
    let gameplay = |a: &Action, i: usize, base_zoom: f64, offset_x: f64, cy: f64| -> Cam {
        let zoom = if framing.keep_both { base_zoom.min(a.fit_zoom[i]) } else { base_zoom };
        let half = crop_size(zoom, src_h).0.min(src_w) / 2.0;
        let mut cx = a.cx[i] + offset_x;
        if framing.keep_both && track.is_some() {
            let (lo, hi) = (a.right[i] + margin - half, a.left[i] - margin + half);
            cx = if lo <= hi { cx.clamp(lo, hi) } else { (a.left[i] + a.right[i]) / 2.0 };
        }
        Cam { cx, cy: if zoom <= 1.0 { src_h / 2.0 } else { cy }, zoom }
    };

    let combo_a = action(cam.combo.smoothing);
    let combo: Vec<Cam> = (0..n).map(|i| gameplay(&combo_a, i, 1.0, cam.combo.offset_x, src_h / 2.0)).collect();

    let post_zoom = cam.post.zoom.max(1.0);
    let post_a = action(cam.post.smoothing);
    let post: Vec<Cam> = (0..n)
        .map(|i| gameplay(&post_a, i, post_zoom, cam.post.offset_x, post_a.cy[i] + cam.post.offset_y))
        .collect();

    let idx_of = |t: f64| (((t - markers.t_in) * fps).round().max(0.0) as usize).min(n - 1);
    let sc = &cam.super_;
    let ss = markers.super_start;
    // Auto startup: the gameplay camera keeps following both fighters.
    // Auto cinematic: one framing per shot, centred on the whole animation
    // (or, optionally, following it), re-centring at each cut.
    let anim_x: Vec<f64> = match track {
        Some(tr) if tr.frames > 0 && sc.animation_hold => {
            let tfps = if tr.fps > 0.0 { tr.fps } else { fps };
            let tt: Vec<f64> = (0..tr.frames).map(|k| k as f64 / tfps).collect();
            let rx: Vec<f64> = tr.center_x.iter().map(|v| v * src_w / tr.width as f64).collect();
            // Shots inside the cinematic: split at cuts and at its start/end.
            let (a0, a1) = ((markers.anim_start * tfps).round() as usize, (markers.post_start * tfps).round() as usize);
            let mut bounds: Vec<usize> = tr.cuts.iter().copied().chain([a0, a1]).filter(|&c| c > 0 && c < tr.frames).collect();
            bounds.sort_unstable();
            bounds.dedup();
            let held = per_shot(&rx, &bounds, |shot| {
                let mut s = shot.to_vec();
                s.sort_by(f64::total_cmp);
                vec![s[s.len() / 2]; shot.len()]
            });
            times.iter().map(|&t| interp(t, &tt, &held)).collect()
        }
        _ => action(sc.smoothing).cx,
    };
    let anim_auto: Vec<Cam> = anim_x.iter().map(|&x| Cam { cx: x, cy: src_h / 2.0, zoom: 1.0 }).collect();
    // Manual keyframes start from wherever the camera is when the first manual part begins.
    let (manual_t0, manual_start) = if sc.startup_auto {
        (markers.anim_start, combo[idx_of(markers.anim_start)])
    } else {
        (ss, combo[idx_of(ss)])
    };
    let manual = keyframe_path(&times, manual_t0, manual_start, &sc.keyframes, &sc.easing);
    let sup: Vec<Cam> = (0..n)
        .map(|i| {
            let in_anim = times[i] >= markers.anim_start;
            match (in_anim, sc.startup_auto, sc.animation_auto) {
                (false, true, _) => combo[i],
                (true, _, true) => anim_auto[i],
                _ => manual[i],
            }
        })
        .collect();

    let tr = &cam.transitions;
    let cams: Vec<Cam> = (0..n)
        .map(|i| {
            let t = times[i];
            let c = if t >= markers.post_start {
                blend(sup[i], post[i], t, markers.post_start, tr.super_to_post)
            } else if t >= ss {
                blend(combo[i], sup[i], t, ss, tr.combo_to_super)
            } else {
                combo[i]
            };
            clamp_cam(c, src_w, src_h)
        })
        .collect();
    let parts: Vec<Part> = times.iter().map(|&t| markers.part_at(t)).collect();
    let alphas = overlay_alphas(&project.overlay, &parts, fps);
    Timeline { fps, markers, src_w, src_h, cams, parts, alphas, tracked: track.is_some() }
}

/// Per-element visibility (0..1) per frame, ramping at part changes.
pub fn overlay_alphas(skin: &Skin, parts: &[Part], fps: f64) -> HashMap<String, Vec<f32>> {
    let fade_frames = (skin.fade * fps).round().max(0.0) as usize;
    let mut out = HashMap::new();
    for el in &skin.elements {
        let target: Vec<f32> =
            parts.iter().map(|p| if el.enabled && el.parts.get(*p) { 1.0 } else { 0.0 }).collect();
        let values = if fade_frames > 0 && !target.is_empty() {
            let step = 1.0 / fade_frames as f32;
            let mut cur = target[0];
            target
                .iter()
                .map(|&tv| {
                    cur = if tv > cur { (cur + step).min(tv) } else { (cur - step).max(tv) };
                    cur
                })
                .collect()
        } else {
            target
        };
        out.insert(el.id.clone(), values);
    }
    out
}

fn interp(t: f64, xs: &[f64], ys: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    if t <= xs[0] {
        return ys[0];
    }
    if t >= xs[xs.len() - 1] {
        return ys[ys.len() - 1];
    }
    let i = xs.partition_point(|&x| x <= t);
    let (x0, x1) = (xs[i - 1], xs[i]);
    let w = (t - x0) / (x1 - x0);
    ys[i - 1] + (ys[i] - ys[i - 1]) * w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Keyframe;

    fn project() -> Project {
        let mut p = Project::default();
        p.timeline = Markers { t_in: 0.5, super_start: Some(1.5), anim_start: Some(2.0), post_start: Some(3.0), out: Some(4.0) };
        p.camera.super_.keyframes = vec![Keyframe { t: 2.5, cx: 300.0, cy: 180.0, zoom: 1.5, ease: None }];
        p.camera.transitions.super_to_post = 0.0;
        p
    }

    #[test]
    fn parts_keyframes_and_alphas() {
        let tl = build_timeline(&project(), None, 30.0, 640.0, 360.0, 5.0);
        assert_eq!(tl.len(), 105);
        assert_eq!(tl.sample(1.0).part, Part::Combo);
        assert_eq!(tl.sample(1.7).part, Part::Startup);
        assert_eq!(tl.sample(2.2).part, Part::Animation);
        assert_eq!(tl.sample(3.9).part, Part::Post);
        let s = tl.sample(2.5);
        assert!((s.cam.zoom - 1.5).abs() < 1e-9 && (s.cam.cx - 300.0).abs() < 1e-9);
        assert!((tl.sample(2.9).cam.zoom - 1.5).abs() < 1e-9, "holds the last keyframe");
        assert!((tl.sample(3.1).cam.zoom - 1.0).abs() < 1e-9);
        assert_eq!(tl.sample(2.6).alphas["health"], 0.0, "UI hidden during animation");
        assert_eq!(tl.sample(1.0).alphas["health"], 1.0);
    }

    #[test]
    fn super_starts_from_combo_camera_and_eases() {
        let mut p = project();
        p.camera.combo.offset_x = 50.0;
        let tl = build_timeline(&p, None, 30.0, 1920.0, 1080.0, 5.0);
        let at_start = tl.sample(1.5).cam;
        assert!((at_start.cx - 1010.0).abs() < 1e-6, "{at_start:?}");
        let mid = tl.sample(2.0).cam;
        assert!(mid.zoom > 1.0 && mid.zoom < 1.5);
    }

    #[test]
    fn auto_super_follows_track_and_snaps_at_cuts() {
        let mut p = project();
        p.camera.super_.startup_auto = true;
        p.camera.super_.animation_auto = true;
        p.camera.transitions.super_to_post = 0.0;
        // Action at x=600 until a cut at 2.3 s, then at x=1300.
        let frames = 150;
        let cut = (2.3 * 30.0) as usize;
        let cx: Vec<f64> = (0..frames).map(|i| if i < cut { 600.0 } else { 1300.0 }).collect();
        let tr = Track { fps: 30.0, width: 1920, height: 1080, frames, center_x: cx, center_y: vec![540.0; frames], cuts: vec![cut], ..Default::default() };
        let tl = build_timeline(&p, Some(&tr), 30.0, 1920.0, 1080.0, 5.0);
        assert!((tl.sample(1.7).cam.cx - 600.0).abs() < 1.0, "startup follows the fighters");
        assert!((tl.sample(2.25).cam.cx - 600.0).abs() < 1.0, "no glide before the cut");
        assert!((tl.sample(2.31).cam.cx - 1300.0).abs() < 1.0, "snaps at the cut");
        // Manual keyframes in an auto part are ignored.
        assert!((tl.sample(2.5).cam.zoom - 1.0).abs() < 1e-9);
    }

    fn wide_track(frames: usize, left: f64, right: f64) -> Track {
        Track {
            fps: 30.0,
            width: 1920,
            height: 1080,
            frames,
            left: vec![left; frames],
            right: vec![right; frames],
            center_x: vec![(left + right) / 2.0; frames],
            center_y: vec![540.0; frames],
            ..Default::default()
        }
    }

    #[test]
    fn far_apart_fighters_both_stay_in_frame() {
        let mut p = Project::default();
        p.camera.combo.offset_x = 300.0; // must not push a fighter out
        let tr = wide_track(90, 500.0, 1500.0); // 1000 px apart, crop at zoom 1 is 607.5
        let tl = build_timeline(&p, Some(&tr), 30.0, 1920.0, 1080.0, 3.0);
        let c = tl.sample(1.5).cam;
        let (cw, _) = crop_size(c.zoom, 1080.0);
        assert!(c.zoom < 1.0, "zooms out: {c:?}");
        assert!(c.cx - cw / 2.0 <= 500.0 - 59.0 && c.cx + cw / 2.0 >= 1500.0 + 59.0, "both inside with margin: {c:?} {cw}");
        // Close together: native scale (zoom 1), centred on them.
        let tr = wide_track(90, 900.0, 1100.0);
        p.camera.combo.offset_x = 0.0;
        let c = build_timeline(&p, Some(&tr), 30.0, 1920.0, 1080.0, 3.0).sample(1.5).cam;
        assert!((c.zoom - 1.0).abs() < 1e-9 && (c.cx - 1000.0).abs() < 1e-6, "{c:?}");
        // Turning the rule off keeps native scale.
        p.camera.framing.keep_both = false;
        let tr = wide_track(90, 500.0, 1500.0);
        let c = build_timeline(&p, Some(&tr), 30.0, 1920.0, 1080.0, 3.0).sample(1.5).cam;
        assert!((c.zoom - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cinematic_holds_one_framing_per_shot() {
        let mut p = project();
        p.camera.super_.animation_auto = true;
        p.camera.transitions.super_to_post = 0.0;
        // Wobbling action, cut at 2.5 s (frame 75) from x~600 to x~1300.
        let frames = 150;
        let cx: Vec<f64> = (0..frames).map(|i| if i < 75 { 600.0 } else { 1300.0 } + 80.0 * (i as f64 * 0.7).sin()).collect();
        let tr = Track { fps: 30.0, width: 1920, height: 1080, frames, left: cx.clone(), right: cx.clone(), center_x: cx, center_y: vec![540.0; frames], cuts: vec![75], ..Default::default() };
        let tl = build_timeline(&p, Some(&tr), 30.0, 1920.0, 1080.0, 5.0);
        let (a, b) = (tl.sample(2.1).cam.cx, tl.sample(2.4).cam.cx);
        assert!((a - b).abs() < 1e-9 && (a - 600.0).abs() < 60.0, "steady within a shot: {a} {b}");
        let c = tl.sample(2.6).cam.cx;
        assert!((c - 1300.0).abs() < 60.0 && (c - tl.sample(2.9).cam.cx).abs() < 1e-9, "re-centred after the cut: {c}");
    }

    #[test]
    fn camera_is_clamped_inside_source() {
        let mut p = Project::default();
        p.camera.combo.offset_x = -5000.0;
        let tl = build_timeline(&p, None, 30.0, 1920.0, 1080.0, 2.0);
        let (cw, _) = crop_size(1.0, 1080.0);
        assert!((tl.cams[0].cx - cw / 2.0).abs() < 1e-9);
    }

    #[test]
    fn unset_markers_mean_all_combo() {
        let r = resolve_markers(&Markers::default(), 10.0);
        assert_eq!((r.super_start, r.post_start, r.out), (10.0, 10.0, 10.0));
    }
}
