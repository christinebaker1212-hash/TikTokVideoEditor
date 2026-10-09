"""Entry point for the frozen app (PyInstaller can't use ``python -m editor``)."""

import multiprocessing

from editor.__main__ import main

if __name__ == "__main__":
    multiprocessing.freeze_support()
    main()
