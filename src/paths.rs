//! Where the app keeps its data, and how it launches ffmpeg.
//!
//! The Windows release is portable: presets, projects, caches and exports live
//! next to the .exe. If that folder is read-only (e.g. Program Files) we fall
//! back to %LOCALAPPDATA%\ComboEditor.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(dir) = std::env::var_os("COMBO_EDITOR_DATA") {
            return PathBuf::from(dir);
        }
        let dir = app_dir();
        let probe = dir.join(".write-test");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return dir;
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("ComboEditor")
    })
}

pub fn workspace() -> PathBuf {
    data_dir().join("workspace")
}

pub fn presets_dir() -> PathBuf {
    data_dir().join("presets")
}

/// Locate ffmpeg/ffprobe: $COMBO_EDITOR_FFMPEG_DIR, an `ffmpeg` folder next to
/// the program (the release ships one), then PATH.
pub fn tool(name: &str) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(d) = std::env::var_os("COMBO_EDITOR_FFMPEG_DIR") {
        dirs.push(PathBuf::from(d));
    }
    dirs.push(app_dir().join("ffmpeg"));
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("ffmpeg"));
    }
    for d in dirs {
        let p = d.join(&exe);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe)
}

/// A `Command` for ffmpeg/ffprobe that never flashes a console window on Windows.
pub fn command(name: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(tool(name));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
