"""Locate the two fighters in the clean (no-UI) recording, frame by frame.

The game camera pans and zooms, so plain background subtraction does not work.
Instead each frame is scored with two cheap foreground cues:

* motion: difference against the previous frame after compensating the global
  camera shift (phase correlation). Fighters are never perfectly still -- idle
  animations, hit sparks and Drive auras keep them moving.
* colour: saturation. Stages (especially training mode) are much less saturated
  than the characters and their effects.

The column/row profile of that score gives the horizontal/vertical extent of
"the action"; its robust midpoint is where the camera should centre so both
characters stay in frame. Results are cached per file.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path

import cv2
import numpy as np

from . import tools
from .config import WORKSPACE
from .media import probe

ANALYSIS_W = 480


def _cache_path(path: str, params: dict) -> Path:
    st = os.stat(path)
    key = json.dumps([os.path.abspath(path), st.st_size, st.st_mtime, params, 2], sort_keys=True)
    return WORKSPACE / "cache" / f"track_{hashlib.sha1(key.encode()).hexdigest()[:16]}.json"


def _extent(profile: np.ndarray, pct: float) -> tuple[float, float, float]:
    total = profile.sum()
    if total <= 1e-6:
        return np.nan, np.nan, 0.0
    cdf = np.cumsum(profile) / total
    lo = float(np.searchsorted(cdf, pct / 100.0))
    hi = float(np.searchsorted(cdf, 1.0 - pct / 100.0))
    return lo, hi, float(total)


def analyse_frames(frames, motion_weight: float = 1.0, color_weight: float = 1.0, percentile: float = 4.0):
    """Analyse an iterable of small BGR frames; returns per-frame extents in analysis pixels."""
    prev = None
    rows = []
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))
    for frame in frames:
        h, w = frame.shape[:2]
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY).astype(np.float32)
        score = np.zeros((h, w), np.float32)

        if color_weight > 0:
            hsv = cv2.cvtColor(frame, cv2.COLOR_BGR2HSV)
            sat = hsv[..., 1].astype(np.float32) / 255.0
            sat *= hsv[..., 2] > 40
            sat = np.clip((sat - 0.18) / 0.35, 0.0, 1.0)
            score += color_weight * sat

        if motion_weight > 0 and prev is not None:
            (dx, dy), _ = cv2.phaseCorrelate(prev, gray)
            m = np.float32([[1, 0, dx], [0, 1, dy]])
            warped = cv2.warpAffine(prev, m, (w, h), borderMode=cv2.BORDER_REPLICATE)
            diff = cv2.GaussianBlur(np.abs(gray - warped), (5, 5), 0)
            mask = np.clip((diff - 10.0) / 25.0, 0.0, 1.0)
            # Ignore the strip revealed at the border by the camera shift.
            bx, by = int(abs(dx)) + 2, int(abs(dy)) + 2
            mask[:by] = mask[h - by:] = 0
            mask[:, :bx] = mask[:, w - bx:] = 0
            mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel)
            score += motion_weight * mask
        prev = gray

        x0, x1, wx = _extent(score.sum(axis=0), percentile)
        y0, y1, _ = _extent(score.sum(axis=1), percentile)
        rows.append((x0, x1, y0, y1, wx / (w * h)))
    return np.array(rows, dtype=np.float64).reshape(-1, 5)


def _iter_frames(path: str, width: int, height: int):
    proc = tools.popen(
        ["ffmpeg", "-v", "error", "-nostdin", "-i", path, "-an", "-vf", f"scale={width}:{height}:flags=area",
         "-f", "rawvideo", "-pix_fmt", "bgr24", "-"],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    )
    size = width * height * 3
    try:
        while True:
            buf = proc.stdout.read(size)
            if len(buf) < size:
                break
            yield np.frombuffer(buf, np.uint8).reshape(height, width, 3)
    finally:
        proc.stdout.close()
        proc.kill()
        proc.wait()


def _fill_gaps(values: np.ndarray, fallback: float) -> np.ndarray:
    good = np.isfinite(values)
    if not good.any():
        return np.full_like(values, fallback)
    idx = np.arange(len(values))
    return np.interp(idx, idx[good], values[good])


def track_video(path: str, motion_weight: float = 1.0, color_weight: float = 1.0, percentile: float = 4.0,
                use_cache: bool = True) -> dict:
    """Track the action in ``path``. Returns per-frame arrays in source pixels."""
    params = {"m": motion_weight, "c": color_weight, "p": percentile}
    cache = _cache_path(path, params)
    if use_cache and cache.exists():
        return json.loads(cache.read_text())

    info = probe(path)
    aw = ANALYSIS_W
    ah = int(round(info.height * aw / info.width / 2)) * 2
    data = analyse_frames(_iter_frames(path, aw, ah), motion_weight, color_weight, percentile)
    sx, sy = info.width / aw, info.height / ah

    left = _fill_gaps(data[:, 0], aw / 2) * sx
    right = _fill_gaps(data[:, 1], aw / 2) * sx
    top = _fill_gaps(data[:, 2], ah / 2) * sy
    bottom = _fill_gaps(data[:, 3], ah / 2) * sy
    result = {
        "fps": info.fps,
        "width": info.width,
        "height": info.height,
        "frames": int(len(data)),
        "left": np.round(left, 2).tolist(),
        "right": np.round(right, 2).tolist(),
        "top": np.round(top, 2).tolist(),
        "bottom": np.round(bottom, 2).tolist(),
        "centerX": np.round((left + right) / 2, 2).tolist(),
        "centerY": np.round((top + bottom) / 2, 2).tolist(),
        "strength": np.round(data[:, 4], 5).tolist(),
    }
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(result))
    return result


def smooth(values: np.ndarray, sigma_frames: float, median: int = 7) -> np.ndarray:
    """Outlier-robust, zero-phase smoothing (median prefilter + Gaussian)."""
    v = np.asarray(values, dtype=np.float64)
    if len(v) == 0:
        return v
    if median > 1 and len(v) >= median:
        pad = median // 2
        padded = np.pad(v, pad, mode="edge")
        v = np.median(np.lib.stride_tricks.sliding_window_view(padded, median), axis=1)
    if sigma_frames <= 0:
        return v
    radius = int(3 * sigma_frames) + 1
    k = np.exp(-0.5 * (np.arange(-radius, radius + 1) / sigma_frames) ** 2)
    k /= k.sum()
    padded = np.pad(v, radius, mode="edge")
    return np.convolve(padded, k, mode="valid")
