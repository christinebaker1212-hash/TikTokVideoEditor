"""Compose one output frame: camera crop of the clean video + UI elements."""

from __future__ import annotations

from functools import lru_cache

import cv2
import numpy as np

from .camera import crop_size
from .config import OUT_H, OUT_W, REF_H, REF_W


SCALERS = {"lanczos": cv2.INTER_LANCZOS4, "cubic": cv2.INTER_CUBIC, "linear": cv2.INTER_LINEAR}


def crop_frame(frame: np.ndarray, cx: float, cy: float, zoom: float, scaler: str = "lanczos") -> np.ndarray:
    """Sub-pixel accurate crop + upscale of the source to 1080x1920."""
    src_h = frame.shape[0]
    cw, ch = crop_size(zoom, src_h)
    s = OUT_H / ch
    x0, y0 = cx - cw / 2, cy - ch / 2
    # Pixel-centre correct mapping: u = s * (x - x0 + 0.5) - 0.5
    m = np.float32([[s, 0, s * (0.5 - x0) - 0.5], [0, s, s * (0.5 - y0) - 0.5]])
    return cv2.warpAffine(frame, m, (OUT_W, OUT_H), flags=SCALERS.get(scaler, cv2.INTER_LANCZOS4), borderMode=cv2.BORDER_REPLICATE)


@lru_cache(maxsize=64)
def feather_mask(w: int, h: int, left: float, right: float, top: float, bottom: float) -> np.ndarray:
    """Alpha ramp (h, w, 1) that fades each edge to 0 over the given pixel widths."""
    def ramp(n, a, b):
        x = np.arange(n, dtype=np.float32) + 0.5
        r = np.ones(n, np.float32)
        if a > 0:
            r *= np.clip(x / a, 0, 1)
        if b > 0:
            r *= np.clip((n - x) / b, 0, 1)
        return r
    return (ramp(h, top, bottom)[:, None] * ramp(w, left, right)[None, :])[..., None]


def scale_rect(rect: dict, sx: float, sy: float) -> tuple[int, int, int, int]:
    return (int(round(rect["x"] * sx)), int(round(rect["y"] * sy)),
            max(1, int(round(rect["w"] * sx))), max(1, int(round(rect["h"] * sy))))


def draw_element(out: np.ndarray, ui: np.ndarray, el: dict, alpha: float, ref_size=(REF_W, REF_H)) -> None:
    opacity = float(el.get("opacity", 1.0)) * alpha
    if opacity <= 0.001:
        return
    uh, uw = ui.shape[:2]
    sx, sy, sw, sh = scale_rect(el["src"], uw / ref_size[0], uh / ref_size[1])
    x0, y0 = max(0, sx), max(0, sy)
    x1, y1 = min(uw, sx + sw), min(uh, sy + sh)
    if x1 <= x0 or y1 <= y0:
        return
    patch = np.zeros((sh, sw, 3), np.uint8)
    patch[y0 - sy:y1 - sy, x0 - sx:x1 - sx] = ui[y0:y1, x0:x1]

    dx, dy, dw, dh = scale_rect(el["dst"], 1, 1)
    interp = cv2.INTER_LANCZOS4 if dw >= sw else cv2.INTER_AREA
    patch = cv2.resize(patch, (dw, dh), interpolation=interp)
    f = el.get("feather", {})
    mask = feather_mask(dw, dh, float(f.get("left", 0)), float(f.get("right", 0)),
                        float(f.get("top", 0)), float(f.get("bottom", 0))) * opacity

    # Clip destination to the output frame.
    ox0, oy0 = max(0, dx), max(0, dy)
    ox1, oy1 = min(out.shape[1], dx + dw), min(out.shape[0], dy + dh)
    if ox1 <= ox0 or oy1 <= oy0:
        return
    p = patch[oy0 - dy:oy1 - dy, ox0 - dx:ox1 - dx].astype(np.float32)
    a = mask[oy0 - dy:oy1 - dy, ox0 - dx:ox1 - dx]
    region = out[oy0:oy1, ox0:ox1].astype(np.float32)
    out[oy0:oy1, ox0:ox1] = np.clip(region + (p - region) * a + 0.5, 0, 255).astype(np.uint8)


def compose(clean: np.ndarray, ui: np.ndarray | None, cam: dict, skin: dict, scaler: str = "lanczos") -> np.ndarray:
    out = crop_frame(clean, cam["cx"], cam["cy"], cam["zoom"], scaler)
    if ui is not None:
        ref = tuple(skin.get("referenceSize") or (REF_W, REF_H))
        for el in skin.get("elements", []):
            if el.get("enabled", True):
                draw_element(out, ui, el, float(cam["alphas"].get(el["id"], 0.0)), ref)
    return out
