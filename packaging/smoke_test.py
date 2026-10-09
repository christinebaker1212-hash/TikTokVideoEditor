"""End-to-end check of the packaged Windows app, using only the bundled ffmpeg.

    python packaging/smoke_test.py dist/ComboEditor

Generates a with-UI / clean pair with a known 0.3 s offset, then checks the
command-line sync and export, and that the editor window opens and renders.
"""

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path


def run(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True, timeout=600, **kw)


def main(app_dir: str) -> None:
    app = Path(app_dir).resolve()
    exe, ffmpeg, ffprobe = app / "ComboEditor.exe", app / "ffmpeg" / "ffmpeg.exe", app / "ffmpeg" / "ffprobe.exe"
    for f in (exe, ffmpeg, ffprobe, app / "ffmpeg" / "LICENSE.txt", app / "README.txt"):
        assert f.is_file(), f"missing {f}"

    tmp = Path(tempfile.mkdtemp())
    env = dict(os.environ, COMBO_EDITOR_DATA=str(tmp / "data"))
    master, ui, clean = tmp / "master.mp4", tmp / "ui.mp4", tmp / "clean.mp4"
    # A moving test pattern with irregular beeps (the transients used for syncing).
    beeps = "+".join(f"between(t,{t},{t + 0.05})" for t in (0.7, 1.3, 1.6, 2.4, 3.1, 3.3, 4.2, 5.0))
    r = run([ffmpeg, "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30:duration=6",
             "-f", "lavfi", "-i", f"aevalsrc='0.8*sin(2*PI*900*t)*({beeps})':s=48000:d=6",
             "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", master])
    assert r.returncode == 0, r.stderr
    r = run([ffmpeg, "-v", "error", "-y", "-i", master, "-vf", "drawbox=x=1040:y=52:w=715:h=46:color=red:t=fill",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "copy", ui])
    assert r.returncode == 0, r.stderr
    r = run([ffmpeg, "-v", "error", "-y", "-ss", "0.3", "-i", master, "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", clean])
    assert r.returncode == 0, r.stderr

    r = run([exe, "sync", ui, clean], env=env)
    print(r.stdout, r.stderr)
    assert r.returncode == 0, r.stderr
    m = re.search(r"offset (-?[\d.]+)", r.stdout)
    assert m, f"no offset printed: {r.stdout!r}"
    offset = float(m.group(1))
    assert abs(offset - 0.3) < 0.02, offset
    print("sync ok", offset)

    out = tmp / "out.mp4"
    project = {
        "name": "smoke",
        "sources": {"ui": str(ui), "clean": str(clean)},
        "sync": {"offset": offset},
        "timeline": {"in": 0.5, "superStart": 2.0, "animStart": 2.5, "postStart": 3.5, "out": 4.5},
        "camera": {"super": {"keyframes": [{"t": 2.5, "cx": 900, "cy": 500, "zoom": 1.5}]}},
        "export": {"output": str(out), "preset": "veryfast"},
    }
    proj = tmp / "smoke.json"
    proj.write_text(json.dumps(project))
    r = run([exe, "export", proj], env=env)
    print(r.stdout, r.stderr)
    assert r.returncode == 0, r.stderr
    r = run([ffprobe, "-v", "error", "-show_entries", "stream=codec_type,width,height", "-show_entries", "format=duration",
             "-of", "json", out])
    info = json.loads(r.stdout)
    video = next(s for s in info["streams"] if s["codec_type"] == "video")
    assert (video["width"], video["height"]) == (1080, 1920), video
    assert any(s["codec_type"] == "audio" for s in info["streams"]), info
    assert abs(float(info["format"]["duration"]) - 4.0) < 0.15, info
    print("export ok", info)

    png = tmp / "still.png"
    r = run([exe, "still", proj, "1.0", png], env=env)
    assert r.returncode == 0 and png.is_file(), r.stderr
    print("still ok")

    # The editor window: render frames from the project, then quit by itself.
    marker = tmp / "gui.txt"
    r = run([exe, proj], env=dict(env, COMBO_EDITOR_SMOKE=str(marker)))
    log = tmp / "data" / "combo-editor.log"
    if log.exists():
        print("log:", log.read_text())
    assert r.returncode == 0, f"GUI exited with {r.returncode}: {r.stderr}"
    assert marker.is_file() and marker.read_text() == "ok", marker.read_text() if marker.is_file() else "no marker"
    print("gui ok")
    print("SMOKE TEST PASSED")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "dist/ComboEditor")
