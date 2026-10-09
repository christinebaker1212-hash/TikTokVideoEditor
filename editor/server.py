"""Local web server for the editor UI."""

from __future__ import annotations

import os
import shutil
import threading
import traceback
import uuid
from pathlib import Path

import cv2
from fastapi import Body, FastAPI, File, HTTPException, UploadFile
from fastapi.responses import FileResponse, Response
from fastapi.staticfiles import StaticFiles

from . import presets, render, sync, tracking
from .camera import build_timeline
from .config import DEFAULT_PROJECT, WEB_DIR, WORKSPACE, normalize_project
from .media import MediaError, probe

app = FastAPI(title="TikTok Combo Editor")
JOBS: dict[str, dict] = {}


def _check_file(path: str) -> str:
    if not path or not os.path.isfile(path):
        raise HTTPException(404, f"File not found: {path}")
    return path


def _errors(fn):
    try:
        return fn()
    except HTTPException:
        raise
    except (MediaError, ValueError, FileNotFoundError) as exc:
        raise HTTPException(400, str(exc)) from exc


@app.get("/api/defaults")
def defaults():
    return DEFAULT_PROJECT


@app.get("/api/media")
def media(path: str):
    # Starlette's FileResponse handles Range requests, which <video> seeking needs.
    return FileResponse(_check_file(path))


@app.post("/api/upload")
def upload(file: UploadFile = File(...)):
    dest_dir = WORKSPACE / "uploads"
    dest_dir.mkdir(parents=True, exist_ok=True)
    name = Path(file.filename or "upload").name
    dest = dest_dir / f"{uuid.uuid4().hex[:8]}_{name}"
    with dest.open("wb") as fh:
        shutil.copyfileobj(file.file, fh, length=4 << 20)
    return {"path": str(dest)}


@app.post("/api/probe")
def probe_file(path: str = Body(..., embed=True)):
    return _errors(lambda: probe(_check_file(path)).to_dict())


@app.post("/api/sync")
def sync_sources(ui: str = Body(...), clean: str = Body(...)):
    return _errors(lambda: sync.sync_files(_check_file(ui), _check_file(clean)))


@app.post("/api/waveform")
def waveform(path: str = Body(..., embed=True)):
    return _errors(lambda: sync.waveform(_check_file(path)))


def _track(project: dict) -> dict:
    t = project["tracking"]
    return tracking.track_video(_check_file(project["sources"]["clean"]), t["motionWeight"], t["colorWeight"],
                                t["percentile"])


@app.post("/api/track")
def track(project: dict = Body(...)):
    project = normalize_project(project)
    result = _errors(lambda: _track(project))
    return {k: result[k] for k in ("fps", "width", "height", "frames", "left", "right", "centerX")}


@app.post("/api/timeline")
def timeline(project: dict = Body(...)):
    """Per-frame camera path + overlay alphas. Uses cached tracking if available."""
    project = normalize_project(project)

    def run():
        clean = _check_file(project["sources"]["clean"])
        info = probe(clean)
        t = project["tracking"]
        cache = tracking._cache_path(clean, {"m": t["motionWeight"], "c": t["colorWeight"], "p": t["percentile"]})
        track_data = _track(project) if cache.exists() else None
        result = build_timeline(project, track_data, info.fps, info.width, info.height, info.duration)
        result["tracked"] = track_data is not None
        return result
    return _errors(run)


@app.post("/api/still")
def still(project: dict = Body(...), t: float = Body(...)):
    frame = _errors(lambda: render.render_still(normalize_project(project), t))
    ok, buf = cv2.imencode(".jpg", frame, [cv2.IMWRITE_JPEG_QUALITY, 92])
    return Response(buf.tobytes(), media_type="image/jpeg")


@app.post("/api/export")
def export(project: dict = Body(...)):
    project = normalize_project(project)
    output = project["export"].get("output") or str(WORKSPACE / "exports" / f"{project['name'] or 'export'}.mp4")
    job_id = uuid.uuid4().hex[:10]
    job = {"id": job_id, "progress": 0.0, "message": "Queued", "status": "running", "output": output,
           "cancel": threading.Event()}
    JOBS[job_id] = job

    def progress(p, msg):
        job["progress"], job["message"] = p, msg

    def run():
        try:
            render.export(project, output, progress, job["cancel"])
            job["status"] = "done"
        except render.Cancelled:
            job["status"], job["message"] = "cancelled", "Cancelled"
        except Exception as exc:  # report any failure to the UI
            traceback.print_exc()
            job["status"], job["message"] = "error", str(exc)

    threading.Thread(target=run, daemon=True).start()
    return {"id": job_id, "output": output}


@app.get("/api/jobs/{job_id}")
def job_status(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(404, "Unknown job")
    return {k: v for k, v in job.items() if k != "cancel"}


@app.post("/api/jobs/{job_id}/cancel")
def job_cancel(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(404, "Unknown job")
    job["cancel"].set()
    return {"ok": True}


@app.get("/api/presets/{kind}")
def preset_list(kind: str):
    return _errors(lambda: presets.list_presets(kind))


@app.get("/api/presets/{kind}/{name}")
def preset_get(kind: str, name: str):
    try:
        return presets.load_preset(kind, name)
    except FileNotFoundError as exc:
        raise HTTPException(404, f"No preset {name}") from exc
    except ValueError as exc:
        raise HTTPException(400, str(exc)) from exc


@app.put("/api/presets/{kind}/{name}")
def preset_put(kind: str, name: str, data: dict = Body(...)):
    _errors(lambda: presets.save_preset(kind, name, data))
    return {"ok": True}


@app.delete("/api/presets/{kind}/{name}")
def preset_delete(kind: str, name: str):
    _errors(lambda: presets.delete_preset(kind, name))
    return {"ok": True}


app.mount("/", StaticFiles(directory=WEB_DIR, html=True), name="web")
