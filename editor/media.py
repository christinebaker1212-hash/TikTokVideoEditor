"""ffmpeg/ffprobe helpers: probing, audio extraction and raw frame streaming."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from fractions import Fraction

import numpy as np

from . import tools


class MediaError(RuntimeError):
    pass


@dataclass
class VideoInfo:
    path: str
    width: int
    height: int
    fps: float
    duration: float
    has_audio: bool

    def to_dict(self) -> dict:
        return self.__dict__.copy()


def probe(path: str) -> VideoInfo:
    try:
        out = tools.run(
            ["ffprobe", "-v", "error", "-print_format", "json", "-show_streams", "-show_format", path],
            check=True, capture_output=True, text=True,
        ).stdout
    except FileNotFoundError as exc:
        raise MediaError("ffprobe not found: put ffmpeg in the 'ffmpeg' folder next to the program or on PATH") from exc
    except subprocess.CalledProcessError as exc:
        raise MediaError(f"Could not read {path}: {exc.stderr.strip()}") from exc
    data = json.loads(out)
    streams = data.get("streams", [])
    video = next((s for s in streams if s.get("codec_type") == "video"), None)
    has_audio = any(s.get("codec_type") == "audio" for s in streams)
    duration = float(data.get("format", {}).get("duration") or 0.0)
    if video is None:
        return VideoInfo(path, 0, 0, 0.0, duration, has_audio)
    rate = video.get("avg_frame_rate") or video.get("r_frame_rate") or "0/1"
    try:
        fps = float(Fraction(rate)) if rate != "0/0" else 0.0
    except (ZeroDivisionError, ValueError):
        fps = 0.0
    if not fps:
        fps = float(Fraction(video.get("r_frame_rate", "60/1")))
    if not duration:
        duration = float(video.get("duration") or 0.0)
    return VideoInfo(path, int(video["width"]), int(video["height"]), fps, duration, has_audio)


def load_audio(path: str, sr: int = 16000, start: float = 0.0, duration: float | None = None) -> np.ndarray:
    """Decode an audio track to mono float32 at ``sr``."""
    cmd = ["ffmpeg", "-v", "error", "-nostdin"]
    if start > 0:
        cmd += ["-ss", f"{start:.6f}"]
    cmd += ["-i", path]
    if duration is not None:
        cmd += ["-t", f"{duration:.6f}"]
    cmd += ["-vn", "-ac", "1", "-ar", str(sr), "-f", "f32le", "-"]
    proc = tools.run(cmd, capture_output=True)
    if proc.returncode != 0:
        raise MediaError(f"Could not decode audio from {path}: {proc.stderr.decode(errors='ignore').strip()}")
    return np.frombuffer(proc.stdout, dtype=np.float32).copy()


class FrameReader:
    """Stream BGR frames from ``path`` starting at ``start`` seconds at a fixed fps.

    ``start`` may be negative: black frames are produced until the source begins.
    After the source runs out the last frame is repeated, so callers can always
    read exactly as many frames as they need.
    """

    def __init__(self, path: str, start: float, fps: float, width: int, height: int):
        self.width, self.height, self.fps = width, height, fps
        self.frame_bytes = width * height * 3
        self.pad = max(0, int(round(-start * fps)))
        self.last: np.ndarray | None = None
        seek = max(0.0, start)
        cmd = ["ffmpeg", "-v", "error", "-nostdin"]
        if seek > 0:
            cmd += ["-ss", f"{seek:.6f}"]
        cmd += [
            "-i", path, "-an",
            "-vf", f"fps={fps:.6f},scale={width}:{height}:flags=lanczos",
            "-f", "rawvideo", "-pix_fmt", "bgr24", "-",
        ]
        self.proc = tools.popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)

    def read(self) -> np.ndarray:
        if self.pad > 0:
            self.pad -= 1
            return np.zeros((self.height, self.width, 3), np.uint8)
        buf = self.proc.stdout.read(self.frame_bytes) if self.proc.stdout else b""
        if len(buf) < self.frame_bytes:
            if self.last is None:
                return np.zeros((self.height, self.width, 3), np.uint8)
            return self.last
        self.last = np.frombuffer(buf, np.uint8).reshape(self.height, self.width, 3)
        return self.last

    def close(self) -> None:
        if self.proc.stdout:
            self.proc.stdout.close()
        self.proc.kill()
        self.proc.wait()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def read_frame(path: str, t: float, width: int, height: int) -> np.ndarray:
    """Read a single frame at time ``t`` (black if before the start)."""
    if t < 0:
        return np.zeros((height, width, 3), np.uint8)
    proc = tools.run(
        ["ffmpeg", "-v", "error", "-nostdin", "-ss", f"{t:.6f}", "-i", path, "-frames:v", "1",
         "-vf", f"scale={width}:{height}:flags=lanczos", "-f", "rawvideo", "-pix_fmt", "bgr24", "-"],
        capture_output=True,
    )
    if len(proc.stdout) < width * height * 3:
        return np.zeros((height, width, 3), np.uint8)
    return np.frombuffer(proc.stdout[: width * height * 3], np.uint8).reshape(height, width, 3).copy()
