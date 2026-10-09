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
use crate::tracking::{smooth, Track};

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

pub fn clamp_cam(c: Cam, src_w: f64, src_h: f64) -> Cam {
    let zoom = c.zoom.clamp(1.0, 8.0);
    let (cw, ch) = crop_size(zoom, src_h);
    let cw = cw.min(src_w);
    Cam { cx: c.cx.clamp(cw / 2.0, src_w - cw / 2.0), cy: c.cy.clamp(ch / 2.0, src_h - ch / 2.0), zoom }
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

    let auto = |smoothing: f64| -> (Vec<f64>, Vec<f64>) {
        match track {
            Some(tr) if tr.frames > 0 => {
                let tfps = if tr.fps > 0.0 { tr.fps } else { fps };
                let tt: Vec<f64> = (0..tr.frames).map(|k| k as f64 / tfps).collect();
                let rx: Vec<f64> = tr.center_x.iter().map(|v| v * src_w / tr.width as f64).collect();
                let ry: Vec<f64> = tr.center_y.iter().map(|v| v * src_h / tr.height as f64).collect();
                let sx = smooth(&rx, smoothing * tfps, 7);
                let sy = smooth(&ry, smoothing * tfps, 7);
                (times.iter().map(|&t| interp(t, &tt, &sx)).collect(), times.iter().map(|&t| interp(t, &tt, &sy)).collect())
            }
            _ => (vec![src_w / 2.0; n], vec![src_h / 2.0; n]),
        }
    };

    let (combo_x, _) = auto(cam.combo.smoothing);
    let combo: Vec<Cam> = combo_x.iter().map(|&x| Cam { cx: x + cam.combo.offset_x, cy: src_h / 2.0, zoom: 1.0 }).collect();

    let post_zoom = cam.post.zoom.max(1.0);
    let (post_x, post_y) = auto(cam.post.smoothing);
    let post: Vec<Cam> = (0..n)
        .map(|i| {
            // At zoom 1 the crop is full height, so vertical tracking only matters when zoomed.
            let cy = if post_zoom <= 1.0 { src_h / 2.0 } else { post_y[i] };
            Cam { cx: post_x[i] + cam.post.offset_x, cy: cy + cam.post.offset_y, zoom: post_zoom }
        })
        .collect();

    // The super path starts where the combo camera is at superStart.
    let ss = markers.super_start;
    let start_idx = (((ss - markers.t_in) * fps).round().max(0.0) as usize).min(n - 1);
    let sup = keyframe_path(&times, ss, combo[start_idx], &cam.super_.keyframes, &cam.super_.easing);

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
