"""Run the editor: ``python -m editor [--port 8765] [--no-browser]``."""

import argparse
import threading
import webbrowser

import uvicorn


def main() -> None:
    parser = argparse.ArgumentParser(description="TikTok combo video editor")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--no-browser", action="store_true")
    args = parser.parse_args()
    url = f"http://{args.host}:{args.port}/"
    if not args.no_browser:
        threading.Timer(1.0, webbrowser.open, args=(url,)).start()
    print(f"Editor running at {url}")
    uvicorn.run("editor.server:app", host=args.host, port=args.port, log_level="warning")


if __name__ == "__main__":
    main()
