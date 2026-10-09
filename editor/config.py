"""Shared constants and default documents (project, skin, blend preset)."""

from __future__ import annotations

import copy
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB_DIR = ROOT / "web"
PRESET_DIR = ROOT / "presets"
WORKSPACE = ROOT / "workspace"

# Output (TikTok) frame size.
OUT_W = 1080
OUT_H = 1920
OUT_ASPECT = OUT_W / OUT_H  # 9:16

# Coordinates in skins are authored against a 1920x1080 source; they are scaled
# if the actual source has a different resolution.
REF_W = 1920
REF_H = 1080

# Parts of a video, in order. "startup" and "animation" together form the
# Super Art section whose camera is keyframed by hand.
PARTS = ("combo", "startup", "animation", "post")

DEFAULT_SKIN = {
    "name": "Default",
    "referenceSize": [REF_W, REF_H],
    # Seconds over which an element fades when it is shown/hidden between parts.
    "fade": 0.2,
    "elements": [
        {
            "id": "health",
            "label": "Enemy health bar",
            "enabled": True,
            "src": {"x": 1040, "y": 52, "w": 715, "h": 46},
            "dst": {"x": 0, "y": 165, "w": 1080, "h": 69},
            "opacity": 1.0,
            "feather": {"left": 0, "right": 0, "top": 4, "bottom": 4},
            "parts": {"combo": True, "startup": True, "animation": False, "post": True},
        },
        {
            "id": "damage",
            "label": "Combo damage / attack type",
            "enabled": True,
            "src": {"x": 623, "y": 173, "w": 244, "h": 90},
            "dst": {"x": 348, "y": 252, "w": 390, "h": 144},
            "opacity": 0.92,
            "feather": {"left": 6, "right": 6, "top": 4, "bottom": 4},
            "parts": {"combo": True, "startup": True, "animation": False, "post": True},
        },
    ],
}

DEFAULT_BLEND = {"name": "Default", "opacity": 1.0, "feather": {"left": 0, "right": 0, "top": 4, "bottom": 4}}

DEFAULT_PROJECT = {
    "version": 1,
    "name": "untitled",
    "sources": {"ui": "", "clean": ""},
    # ui_time = clean_time + offset. The clean video is the master timeline.
    "sync": {"offset": 0.0, "confidence": None},
    "audio": {"primary": "clean", "gameVolume": 1.0, "gameFadeOut": 0.0},
    # All times are seconds on the clean video's timeline. None = not set.
    "timeline": {"in": 0.0, "superStart": None, "animStart": None, "postStart": None, "out": None},
    "tracking": {"motionWeight": 1.0, "colorWeight": 1.0, "percentile": 4.0},
    "camera": {
        "combo": {"offsetX": 0.0, "smoothing": 0.35},
        "super": {"easing": "easeInOut", "keyframes": []},
        "post": {"offsetX": 0.0, "offsetY": 0.0, "zoom": 1.0, "smoothing": 0.5},
        "transitions": {"comboToSuper": 0.0, "superToPost": 0.5},
    },
    "overlay": copy.deepcopy(DEFAULT_SKIN),
    "music": {"path": "", "start": 0.0, "volume": 0.8, "fadeIn": 1.0, "fadeOut": 2.0},
    "export": {"output": "", "crf": 16, "preset": "slow", "audioBitrate": "256k", "scaler": "lanczos"},
}


def deep_merge(base: dict, override: dict | None) -> dict:
    """Return ``base`` with ``override`` merged in recursively (lists replace)."""
    out = copy.deepcopy(base)
    for key, value in (override or {}).items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = deep_merge(out[key], value)
        else:
            out[key] = copy.deepcopy(value)
    return out


def normalize_project(project: dict | None) -> dict:
    return deep_merge(DEFAULT_PROJECT, project)
