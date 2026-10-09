"""Desktop launcher: runs the server in-process and shows the editor in a native window.

The window is pywebview, which on Windows uses Edge WebView2 (preinstalled on
Windows 10/11, plays H.264). If no webview is available the editor opens in the
default browser instead.
"""

from __future__ import annotations

import logging
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.request
import webbrowser
from pathlib import Path

from .config import DATA_DIR, FROZEN

log = logging.getLogger("combo_editor")

VIDEO_TYPES = ("Video files (*.mp4;*.mkv;*.mov;*.webm;*.avi;*.flv)", "All files (*.*)")
AUDIO_TYPES = ("Audio files (*.mp3;*.wav;*.m4a;*.aac;*.flac;*.ogg;*.opus)", "Video files (*.mp4;*.mkv;*.mov;*.webm)",
               "All files (*.*)")


def setup_logging() -> Path:
    """A windowed .exe has no console (sys.stdout is None), so log to a file next to the data."""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    log_path = DATA_DIR / "combo-editor.log"
    if sys.stdout is None or sys.stderr is None or FROZEN:
        stream = open(log_path, "a", encoding="utf-8", buffering=1)  # noqa: SIM115 - lives for the process
        sys.stdout = sys.stdout or stream
        sys.stderr = sys.stderr or stream
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s",
                        handlers=[logging.FileHandler(log_path, encoding="utf-8")])
    return log_path


def free_port(preferred: int) -> int:
    for port in (preferred, 0):
        with socket.socket() as s:
            try:
                s.bind(("127.0.0.1", port))
                return s.getsockname()[1]
            except OSError:
                continue
    raise RuntimeError("No free port")


def start_server(host: str, port: int):
    import uvicorn

    from .server import app

    config = uvicorn.Config(app, host=host, port=port, log_level="warning", log_config=None)
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    url = f"http://{host}:{port}/"
    for _ in range(200):
        try:
            urllib.request.urlopen(url + "api/defaults", timeout=0.5).close()
            return server, thread, url
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("Editor server did not start")


class Api:
    """Functions exposed to the page as ``window.pywebview.api``."""

    def __init__(self):
        self.window = None

    def _dialog(self, kind: str, **kwargs):
        import webview

        dialog = getattr(getattr(webview, "FileDialog", None), kind, None)
        if dialog is None:  # pywebview < 5
            dialog = getattr(webview, f"{kind}_DIALOG")
        return self.window.create_file_dialog(dialog, **kwargs)

    def pick_file(self, kind: str = "video"):
        result = self._dialog("OPEN", file_types=AUDIO_TYPES if kind == "audio" else VIDEO_TYPES)
        return result[0] if result else None

    def pick_save(self, suggested: str = "export.mp4"):
        result = self._dialog("SAVE", save_filename=Path(suggested).name or "export.mp4",
                              file_types=("MP4 video (*.mp4)",))
        if not result:
            return None
        path = result if isinstance(result, str) else result[0]
        return path if path.lower().endswith(".mp4") else path + ".mp4"

    def reveal(self, path: str):
        """Show a file in Explorer / the file manager."""
        if not path or not os.path.exists(path):
            return False
        if os.name == "nt":
            subprocess.Popen(["explorer", "/select,", os.path.normpath(path)])
        elif sys.platform == "darwin":
            subprocess.Popen(["open", "-R", path])
        else:
            subprocess.Popen(["xdg-open", os.path.dirname(path)])
        return True


def run_window(url: str) -> bool:
    """Show the editor in a native window; returns False if no webview backend is available."""
    try:
        import webview
    except ImportError:
        return False
    api = Api()
    try:
        api.window = webview.create_window("Combo Editor", url, js_api=api, width=1600, height=960,
                                           min_size=(1100, 700), background_color="#121318")
        storage = DATA_DIR / "webview"
        webview.start(private_mode=False, storage_path=str(storage))
        return True
    except Exception:  # missing WebView2 runtime / GUI toolkit
        log.exception("Native window unavailable, falling back to the browser")
        return False


def main(port: int = 8765, browser: bool = False) -> None:
    log_path = setup_logging()
    log.info("Starting Combo Editor, data folder %s", DATA_DIR)
    server, thread, url = start_server("127.0.0.1", free_port(port))
    if not browser and run_window(url):
        server.should_exit = True
        thread.join(timeout=5)
        return
    webbrowser.open(url)
    print(f"Editor running at {url} (log: {log_path}). Close this window to quit.")
    try:
        while thread.is_alive():
            thread.join(0.5)
    except KeyboardInterrupt:
        server.should_exit = True
