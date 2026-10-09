"""Synthetic fixtures: a fake 'fight' rendered twice (with and without UI), offset in time."""

from __future__ import annotations

import subprocess
import wave
from pathlib import Path

import numpy as np
import pytest

W, H, FPS = 640, 360, 30
UI_LEAD = 0.5          # the UI recording starts 0.5 s before the clean one -> offset = +0.5
CLICKS = [1.0, 1.7, 2.2, 3.1, 3.6, 4.4]
SR = 48000


def fighter_positions(world_t: float) -> tuple[float, float]:
    """Screen x of the two fighters (source pixels) at world time t."""
    mid = 0.5 * W + 0.18 * W * np.sin(world_t * 1.3)
    gap = 0.12 * W + 0.04 * W * np.sin(world_t * 3.0)
    return mid - gap / 2, mid + gap / 2


def scene(world_t: float, with_ui: bool) -> np.ndarray:
    img = np.full((H, W, 3), 120, np.uint8)
    pan = int(world_t * 40) % 32  # camera pan: background grid scrolls
    img[:, (np.arange(W) + pan) % 32 < 2] = 90
    img[(np.arange(H) % 32) < 2, :] = 90
    img[int(H * 0.72):] += 15
    a, b = fighter_positions(world_t)
    wobble = int(3 * np.sin(world_t * 20))  # idle animation
    for x, color in ((a, (40, 40, 200)), (b, (200, 80, 40))):
        x0 = int(x - 18)
        img[int(H * 0.40) + wobble:int(H * 0.75), x0:x0 + 36] = color
    if with_ui:
        # Enemy health bar at the reference position (scaled to 640x360).
        s = W / 1920
        img[int(52 * s):int(98 * s), int(1040 * s):int(1755 * s)] = (30, 30, 230)
    return img


def _encode(path: Path, frames, audio: np.ndarray) -> None:
    wav = path.with_suffix(".wav")
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((np.clip(audio, -1, 1) * 32767).astype(np.int16).tobytes())
    proc = subprocess.Popen(
        ["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "bgr24", "-s", f"{W}x{H}", "-r", str(FPS),
         "-i", "-", "-i", str(wav), "-c:v", "libx264", "-preset", "ultrafast", "-crf", "12", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-shortest", str(path)],
        stdin=subprocess.PIPE,
    )
    for f in frames:
        proc.stdin.write(f.tobytes())
    proc.stdin.close()
    assert proc.wait() == 0
    wav.unlink()


def _audio(world_start: float, duration: float, seed: int) -> np.ndarray:
    n = int(duration * SR)
    rng = np.random.default_rng(seed)
    x = 0.01 * rng.standard_normal(n)
    for c in CLICKS:
        i = int((c - world_start) * SR)
        if 0 <= i < n - 2000:
            t = np.arange(2000) / SR
            x[i:i + 2000] += 0.8 * np.exp(-t * 60) * np.sin(2 * np.pi * 900 * t) * rng.uniform(0.7, 1.0)
    return x


@pytest.fixture(scope="session")
def media(tmp_path_factory):
    d = tmp_path_factory.mktemp("media")
    dur = 5.0
    n = int(dur * FPS)
    clean, ui, music = d / "clean.mp4", d / "ui.mp4", d / "music.wav"
    _encode(clean, (scene(i / FPS + UI_LEAD, False) for i in range(n)), _audio(UI_LEAD, dur, 1))
    _encode(ui, (scene(i / FPS, True) for i in range(n)), _audio(0.0, dur, 2))
    t = np.arange(int(8 * SR)) / SR
    with wave.open(str(music), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((0.3 * np.sin(2 * np.pi * 220 * t) * 32767).astype(np.int16).tobytes())
    return {"clean": str(clean), "ui": str(ui), "music": str(music), "dir": d}


@pytest.fixture(autouse=True)
def isolated_workspace(tmp_path, monkeypatch):
    """Keep caches and presets out of the repository during tests."""
    from editor import config, presets, tracking

    monkeypatch.setattr(tracking, "WORKSPACE", tmp_path / "ws")
    monkeypatch.setattr(config, "WORKSPACE", tmp_path / "ws")
    monkeypatch.setitem(presets.KINDS, "skins", (tmp_path / "skins", config.DEFAULT_SKIN))
    monkeypatch.setitem(presets.KINDS, "blends", (tmp_path / "blends", config.DEFAULT_BLEND))
    yield
