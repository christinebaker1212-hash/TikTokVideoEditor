# PyInstaller spec for the portable Windows build (one folder, no console window).
# Build with packaging/build_windows.ps1, which also adds ffmpeg next to the .exe.
import os

from PyInstaller.utils.hooks import collect_submodules

ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))

a = Analysis(
    [os.path.join(ROOT, "packaging", "launcher.py")],
    pathex=[ROOT],
    datas=[(os.path.join(ROOT, "web"), "web")],
    hiddenimports=collect_submodules("uvicorn") + collect_submodules("editor") + ["multipart", "python_multipart"],
    excludes=["tkinter", "matplotlib", "PIL", "pytest", "IPython"],
    noarchive=False,
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="ComboEditor",
    console=False,
    upx=False,
    icon=None,
)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name="ComboEditor")
