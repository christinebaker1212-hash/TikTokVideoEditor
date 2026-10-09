"""End-to-end check of a packaged build: start it, sync + track + export a generated clip.

    python packaging/smoke_test.py dist/ComboEditor

Uses only the standard library and the ffmpeg bundled with the build.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

PORT = 8799
BASE = f"http://127.0.0.1:{PORT}"


def call(path: str, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as res:
        return json.loads(res.read())


def main(app_dir: str) -> None:
    app = Path(app_dir).resolve()
    exe = app / ("ComboEditor.exe" if os.name == "nt" else "ComboEditor")
    ffmpeg = app / "ffmpeg" / ("ffmpeg.exe" if os.name == "nt" else "ffmpeg")
    assert exe.is_file(), f"missing {exe}"
    assert ffmpeg.is_file(), f"missing bundled {ffmpeg}"
    assert (app / "README.txt").is_file()

    tmp = Path(tempfile.mkdtemp())
    clip = tmp / "clip.mp4"
    subprocess.run([str(ffmpeg), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30",
                    "-f", "lavfi", "-i", "sine=frequency=600:beep_factor=6", "-t", "4", "-c:v", "libx264",
                    "-pix_fmt", "yuv420p", "-c:a", "aac", str(clip)], check=True)

    env = dict(os.environ, COMBO_EDITOR_DATA=str(tmp / "data"))
    proc = subprocess.Popen([str(exe), "--server-only", "--port", str(PORT)], env=env)
    try:
        for _ in range(120):
            try:
                project = call("/api/defaults")
                break
            except OSError:
                if proc.poll() is not None:
                    raise SystemExit(f"app exited with code {proc.returncode}")
                time.sleep(0.5)
        else:
            raise SystemExit("app did not start")
        print("server up")

        info = call("/api/probe", {"path": str(clip)})
        assert (info["width"], info["height"]) == (1920, 1080), info
        sync = call("/api/sync", {"ui": str(clip), "clean": str(clip)})
        assert abs(sync["offset"]) < 0.005, sync
        print("sync ok", sync["offset"])

        project["name"] = "smoke"
        project["sources"] = {"ui": str(clip), "clean": str(clip)}
        project["timeline"].update({"in": 0.5, "superStart": 1.5, "animStart": 2.0, "postStart": 2.5, "out": 3.5})
        project["export"].update({"output": str(tmp / "out.mp4"), "preset": "veryfast"})
        call("/api/track", project)
        assert call("/api/timeline", project)["tracked"]
        print("tracking ok")

        job = call("/api/export", project)
        while True:
            status = call(f"/api/jobs/{job['id']}")
            if status["status"] != "running":
                break
            time.sleep(1)
        assert status["status"] == "done", status
        out = call("/api/probe", {"path": job["output"]})
        assert (out["width"], out["height"]) == (1080, 1920) and out["has_audio"], out
        assert abs(out["duration"] - 3.0) < 0.15, out
        print("export ok", out)
        assert (tmp / "data" / "workspace").is_dir(), "data folder not used"
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
    print("SMOKE TEST PASSED")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "dist/ComboEditor")
