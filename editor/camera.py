"""Turn a project + tracking data into a per-frame camera path and overlay alphas.

The camera is a crop window described by its centre (cx, cy) in source pixels
and a zoom factor. At zoom 1 the crop is the full source height with a 9:16
width (607.5 x 1080 for 1080p), i.e. the source is never rescaled relative to
itself -- it is only cut out and upscaled to 1080x1920 at the end.

Parts:
  combo      [in, superStart)          auto-centre on both fighters
  startup    [superStart, animStart)   manual keyframes
  animation  [animStart, postStart)    manual keyframes
  post       [postStart, out]          auto-centre + user offset / zoom

This module is the single source of truth: the browser preview samples the
arrays returned by :func:`build_timeline` rather than reimplementing them.
"""

from __future__ import annotations

import math

import numpy as np

from .config import OUT_ASPECT, PARTS
from .tracking import smooth

EASINGS = {
    "linear": lambda w: w,
    "easeInOut": lambda w: 0.5 - 0.5 * np.cos(np.pi * w),
    "easeIn": lambda w: w * w,
    "easeOut": lambda w: 1 - (1 - w) * (1 - w),
    "hold": lambda w: np.zeros_like(w),
}


def resolve_markers(timeline: dict, duration: float) -> dict:
    """Fill in unset markers and force them into order."""
    t_in = float(timeline.get("in") or 0.0)
    t_out = timeline.get("out")
    t_out = float(duration if t_out is None else t_out)
    t_in = min(max(0.0, t_in), duration)
    t_out = min(max(t_in, t_out), duration)
    ss = timeline.get("superStart")
    ss = t_out if ss is None else min(max(float(ss), t_in), t_out)
    an = timeline.get("animStart")
    an = ss if an is None else min(max(float(an), ss), t_out)
    ps = timeline.get("postStart")
    ps = t_out if ps is None else min(max(float(ps), an), t_out)
    return {"in": t_in, "superStart": ss, "animStart": an, "postStart": ps, "out": t_out}


def crop_size(zoom: float, src_h: float) -> tuple[float, float]:
    ch = src_h / np.maximum(zoom, 1e-6)
    return ch * OUT_ASPECT, ch


def clamp_camera(cx, cy, zoom, src_w: float, src_h: float):
    zoom = np.maximum(np.asarray(zoom, dtype=np.float64), 1.0)
    cw, ch = crop_size(zoom, src_h)
    cw = np.minimum(cw, src_w)
    cx = np.clip(cx, cw / 2, src_w - cw / 2)
    cy = np.clip(cy, ch / 2, src_h - ch / 2)
    return cx, cy, zoom


def _keyframe_path(times, start_value, keyframes, default_ease: str):
    """Interpolate (cx, cy, zoom) through keyframes starting from ``start_value``."""
    t0, start = start_value
    keys = [(t0, start, default_ease)]
    for k in sorted(keyframes, key=lambda k: float(k["t"])):
        kt = float(k["t"])
        val = (float(k["cx"]), float(k["cy"]), float(k.get("zoom", 1.0)))
        ease = k.get("ease") or default_ease
        if abs(kt - t0) < 1e-6:
            keys[0] = (t0, val, ease)  # explicit keyframe at the start replaces the implicit one
        elif kt > t0:
            keys.append((kt, val, ease))
    out = np.empty((len(times), 3))
    out[:] = keys[-1][1]
    out[times < keys[0][0]] = keys[0][1]
    for (ta, va, _), (tb, vb, ease) in zip(keys, keys[1:]):
        sel = (times >= ta) & (times < tb)
        if not sel.any():
            continue
        w = EASINGS.get(ease, EASINGS["linear"])((times[sel] - ta) / (tb - ta))
        out[sel] = np.asarray(va)[None, :] * (1 - w)[:, None] + np.asarray(vb)[None, :] * w[:, None]
    return out


def _blend(a, b, times, start, duration):
    """Crossfade from path ``a`` to ``b`` over [start, start + duration)."""
    if duration <= 0:
        return b
    w = np.clip((times - start) / duration, 0.0, 1.0)
    w = (0.5 - 0.5 * np.cos(np.pi * w))[:, None]
    return a * (1 - w) + b * w


def build_timeline(project: dict, track: dict | None, fps: float, src_w: int, src_h: int, duration: float) -> dict:
    markers = resolve_markers(project["timeline"], duration)
    cam = project["camera"]
    n = max(1, int(round((markers["out"] - markers["in"]) * fps)))
    times = markers["in"] + np.arange(n) / fps

    # Auto-centre signal, smoothed separately for the combo and post parts.
    if track and track.get("frames"):
        tfps = float(track["fps"]) or fps
        tt = np.arange(track["frames"]) / tfps
        raw_x = np.asarray(track["centerX"], dtype=np.float64) * (src_w / track["width"])
        raw_y = np.asarray(track["centerY"], dtype=np.float64) * (src_h / track["height"])

        def auto(smoothing_s):
            sx = smooth(raw_x, smoothing_s * tfps)
            sy = smooth(raw_y, smoothing_s * tfps)
            return np.interp(times, tt, sx), np.interp(times, tt, sy)
    else:
        def auto(_smoothing_s):
            return np.full(n, src_w / 2), np.full(n, src_h / 2)

    combo_x, _ = auto(float(cam["combo"].get("smoothing", 0.35)))
    combo = np.stack([combo_x + float(cam["combo"].get("offsetX", 0.0)), np.full(n, src_h / 2), np.ones(n)], 1)

    post_cfg = cam["post"]
    post_x, post_y = auto(float(post_cfg.get("smoothing", 0.5)))
    post_zoom = max(1.0, float(post_cfg.get("zoom", 1.0)))
    # At zoom 1 the crop is full height, so vertical tracking only matters when zoomed.
    post_cy = np.full(n, src_h / 2) if post_zoom <= 1.0 else post_y
    post = np.stack([post_x + float(post_cfg.get("offsetX", 0.0)),
                     post_cy + float(post_cfg.get("offsetY", 0.0)), np.full(n, post_zoom)], 1)

    # The super path starts where the combo camera is at superStart.
    ss = markers["superStart"]
    start_idx = int(np.clip(round((ss - markers["in"]) * fps), 0, n - 1))
    start_val = tuple(combo[start_idx])
    sup = _keyframe_path(times, (ss, start_val), cam["super"].get("keyframes", []),
                         cam["super"].get("easing", "easeInOut"))

    trans = cam.get("transitions", {})
    path = combo.copy()
    in_super = (times >= ss) & (times < markers["postStart"])
    blended = _blend(combo, sup, times, ss, float(trans.get("comboToSuper", 0.0)))
    path[in_super] = blended[in_super]
    in_post = times >= markers["postStart"]
    blended = _blend(sup, post, times, markers["postStart"], float(trans.get("superToPost", 0.5)))
    path[in_post] = blended[in_post]

    cx, cy, zoom = clamp_camera(path[:, 0], path[:, 1], path[:, 2], src_w, src_h)

    part_idx = np.zeros(n, dtype=np.int64)
    part_idx[times >= markers["superStart"]] = 1
    part_idx[times >= markers["animStart"]] = 2
    part_idx[times >= markers["postStart"]] = 3

    alphas = overlay_alphas(project["overlay"], part_idx, fps)
    return {
        "fps": fps,
        "markers": markers,
        "sourceSize": [src_w, src_h],
        "times": np.round(times, 5).tolist(),
        "cx": np.round(cx, 3).tolist(),
        "cy": np.round(cy, 3).tolist(),
        "zoom": np.round(zoom, 5).tolist(),
        "part": part_idx.tolist(),
        "parts": list(PARTS),
        "alphas": {k: np.round(v, 4).tolist() for k, v in alphas.items()},
    }


def overlay_alphas(skin: dict, part_idx: np.ndarray, fps: float) -> dict:
    """Per-element visibility factor (0..1) for each frame, with fades at part changes."""
    fade_frames = max(0, int(round(float(skin.get("fade", 0.2)) * fps)))
    result = {}
    for el in skin.get("elements", []):
        if not el.get("enabled", True):
            result[el["id"]] = np.zeros(len(part_idx))
            continue
        flags = np.array([1.0 if el.get("parts", {}).get(p, True) else 0.0 for p in PARTS])
        target = flags[part_idx]
        if fade_frames > 0 and len(target) > 1:
            # Linear ramp towards the target, one step per frame.
            step = 1.0 / fade_frames
            out = np.empty_like(target)
            cur = target[0]
            for i, tv in enumerate(target):
                cur = min(tv, cur + step) if tv > cur else max(tv, cur - step)
                out[i] = cur
            target = out
        result[el["id"]] = target
    return result


def sample(timeline: dict, t: float) -> dict:
    """Camera + alphas at time ``t`` (nearest frame)."""
    times = timeline["times"]
    i = int(np.clip(math.floor((t - times[0]) * timeline["fps"] + 1e-6), 0, len(times) - 1))
    return {
        "cx": timeline["cx"][i], "cy": timeline["cy"][i], "zoom": timeline["zoom"][i],
        "part": timeline["part"][i], "alphas": {k: v[i] for k, v in timeline["alphas"].items()},
    }
