"""Locate and launch the ffmpeg/ffprobe executables.

Search order: $COMBO_EDITOR_FFMPEG_DIR, an ``ffmpeg`` folder next to the program
(the portable Windows build ships one), the PyInstaller bundle, then PATH. On
Windows the child processes are started without flashing a console window.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from functools import lru_cache
from pathlib import Path

from .config import APP_DIR, BUNDLE_DIR

NO_WINDOW = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}


@lru_cache(maxsize=None)
def tool(name: str) -> str:
    exe = name + (".exe" if os.name == "nt" else "")
    dirs = [os.environ.get("COMBO_EDITOR_FFMPEG_DIR"), APP_DIR / "ffmpeg", BUNDLE_DIR / "ffmpeg"]
    for d in dirs:
        if d and (Path(d) / exe).is_file():
            return str(Path(d) / exe)
    return shutil.which(name) or name


def _resolve(cmd: list) -> list:
    return [tool(cmd[0]), *cmd[1:]]


def run(cmd: list, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(_resolve(cmd), **NO_WINDOW, **kwargs)


def popen(cmd: list, **kwargs) -> subprocess.Popen:
    return subprocess.Popen(_resolve(cmd), **NO_WINDOW, **kwargs)
