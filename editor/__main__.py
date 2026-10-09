"""Run the editor.

    python -m editor                 desktop window (falls back to the browser)
    python -m editor --browser       open in the default browser instead
    python -m editor --server-only   just serve http://127.0.0.1:PORT (no window)
"""

import argparse
import multiprocessing


def main() -> None:
    parser = argparse.ArgumentParser(description="TikTok combo video editor")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--browser", action="store_true", help="open in the default browser")
    parser.add_argument("--server-only", "--no-browser", dest="server_only", action="store_true",
                        help="run the server without opening a window")
    args = parser.parse_args()

    from editor import desktop

    if args.server_only:
        import uvicorn

        from editor.server import app

        desktop.setup_logging()  # the windowed .exe has no stdout for uvicorn to log to
        print(f"Editor running at http://{args.host}:{args.port}/")
        uvicorn.run(app, host=args.host, port=args.port, log_level="warning", log_config=None)
        return

    desktop.main(port=args.port, browser=args.browser)


if __name__ == "__main__":
    multiprocessing.freeze_support()
    main()
