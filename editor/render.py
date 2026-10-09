"""Final export: composite every frame in Python and pipe it to ffmpeg with the audio mix."""

from __future__ import annotations

import subprocess
import tempfile
import threading
from pathlib import Path
from typing import Callable

import numpy as np

from .camera import build_timeline, sample
from .compositor import compose
from .config import OUT_H, OUT_W, normalize_project
from .media import FrameReader, MediaError, probe, read_frame
from .tracking import track_video


class Cancelled(Exception):
    pass


def prepare(project: dict, track: dict | None = None):
    project = normalize_project(project)
    clean_path = project["sources"]["clean"]
    if not clean_path:
        raise MediaError("No clean (no-UI) video selected")
    info = probe(clean_path)
    if track is None:
        t = project["tracking"]
        track = track_video(clean_path, t["motionWeight"], t["colorWeight"], t["percentile"])
    timeline = build_timeline(project, track, info.fps, info.width, info.height, info.duration)
    return project, info, timeline


def render_still(project: dict, t: float, track: dict | None = None) -> np.ndarray:
    """Exact (export-identical) composite at clean-timeline time ``t``."""
    project, info, timeline = prepare(project, track)
    clean = read_frame(project["sources"]["clean"], t, info.width, info.height)
    ui = None
    if project["sources"]["ui"]:
        ui_info = probe(project["sources"]["ui"])
        ui = read_frame(project["sources"]["ui"], t + float(project["sync"]["offset"]), ui_info.width, ui_info.height)
    return compose(clean, ui, sample(timeline, t), project["overlay"], str(project["export"].get("scaler", "lanczos")))


def audio_args(project: dict, start: float, duration: float, input_index: int):
    """ffmpeg inputs and filter graph for game audio (+ optional music)."""
    inputs, chains, labels = [], [], []
    audio = project["audio"]
    primary = audio.get("primary", "clean")
    src = project["sources"].get(primary) or project["sources"]["clean"]
    offset = float(project["sync"]["offset"]) if primary == "ui" and project["sources"].get("ui") else 0.0
    a_start = start + offset
    idx = input_index
    if probe(src).has_audio:
        inputs += ["-ss", f"{max(0.0, a_start):.6f}", "-t", f"{duration:.6f}", "-i", src]
        chain = f"[{idx}:a]aresample=48000,volume={float(audio.get('gameVolume', 1.0)):.4f}"
        if a_start < 0:
            delay = int(round(-a_start * 1000))
            chain += f",adelay={delay}:all=1"
        fo = float(audio.get("gameFadeOut", 0.0))
        if fo > 0:
            chain += f",afade=t=out:st={max(0.0, duration - fo):.4f}:d={fo:.4f}"
        chains.append(chain + "[game]")
        labels.append("[game]")
        idx += 1

    music = project["music"]
    if music.get("path"):
        inputs += ["-ss", f"{max(0.0, float(music.get('start', 0.0))):.6f}", "-t", f"{duration:.6f}",
                   "-i", music["path"]]
        chain = f"[{idx}:a]aresample=48000,volume={float(music.get('volume', 0.8)):.4f}"
        fi, fo = float(music.get("fadeIn", 0.0)), float(music.get("fadeOut", 0.0))
        if fi > 0:
            chain += f",afade=t=in:st=0:d={fi:.4f}"
        if fo > 0:
            chain += f",afade=t=out:st={max(0.0, duration - fo):.4f}:d={fo:.4f}"
        chains.append(chain + "[music]")
        labels.append("[music]")

    if not labels:
        return inputs, None
    if len(labels) == 1:
        graph = ";".join(chains) + f";{labels[0]}apad,atrim=0:{duration:.6f},alimiter=limit=0.97[aout]"
    else:
        graph = (";".join(chains) + f";{''.join(labels)}amix=inputs={len(labels)}:normalize=0:duration=longest,"
                 f"apad,atrim=0:{duration:.6f},alimiter=limit=0.97[aout]")
    return inputs, graph


def export(project: dict, output: str, progress: Callable[[float, str], None] | None = None,
           cancel: threading.Event | None = None, track: dict | None = None) -> str:
    progress = progress or (lambda *_: None)
    progress(0.0, "Analysing")
    project, info, timeline = prepare(project, track)
    fps = info.fps
    markers = timeline["markers"]
    start = markers["in"]
    n = len(timeline["times"])
    duration = n / fps

    out_path = Path(output).expanduser()
    out_path.parent.mkdir(parents=True, exist_ok=True)
    a_inputs, graph = audio_args(project, start, duration, 1)
    ex = project["export"]
    cmd = ["ffmpeg", "-y", "-v", "error", "-nostdin",
           "-f", "rawvideo", "-pix_fmt", "bgr24", "-s", f"{OUT_W}x{OUT_H}", "-r", f"{fps:.6f}", "-i", "-"]
    cmd += a_inputs
    if graph:
        cmd += ["-filter_complex", graph, "-map", "0:v", "-map", "[aout]",
                "-c:a", "aac", "-b:a", str(ex.get("audioBitrate", "256k"))]
    cmd += ["-c:v", "libx264", "-preset", str(ex.get("preset", "slow")), "-crf", str(int(ex.get("crf", 16))),
            "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out_path)]

    log = tempfile.TemporaryFile()
    enc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stderr=log)
    clean_r = FrameReader(project["sources"]["clean"], start, fps, info.width, info.height)
    ui_r = None
    if project["sources"]["ui"]:
        ui_info = probe(project["sources"]["ui"])
        ui_r = FrameReader(project["sources"]["ui"], start + float(project["sync"]["offset"]), fps,
                           ui_info.width, ui_info.height)
    skin = project["overlay"]
    scaler = str(ex.get("scaler", "lanczos"))
    try:
        for i in range(n):
            if cancel is not None and cancel.is_set():
                raise Cancelled()
            clean = clean_r.read()
            ui = ui_r.read() if ui_r else None
            cam = {"cx": timeline["cx"][i], "cy": timeline["cy"][i], "zoom": timeline["zoom"][i],
                   "alphas": {k: v[i] for k, v in timeline["alphas"].items()}}
            frame = compose(clean, ui, cam, skin, scaler)
            try:
                enc.stdin.write(frame.tobytes())
            except BrokenPipeError:
                break  # encoder died; its log is reported below
            if i % 10 == 0:
                progress(i / n, f"Rendering frame {i}/{n}")
        try:
            enc.stdin.close()
        except BrokenPipeError:
            pass
        progress(0.99, "Finalising")
        if enc.wait() != 0:
            log.seek(0)
            raise MediaError("ffmpeg failed: " + log.read().decode(errors="ignore")[-2000:])
    except BaseException:
        enc.kill()
        enc.wait()
        if out_path.exists():
            out_path.unlink()
        raise
    finally:
        clean_r.close()
        if ui_r:
            ui_r.close()
        log.close()
    progress(1.0, "Done")
    return str(out_path)
