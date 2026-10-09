//! ffmpeg/ffprobe I/O: probing, streaming raw YUV frames, decoding audio.
//!
//! All video is normalised by ffmpeg to 8-bit yuv420p, BT.709, limited range,
//! so the rest of the app only deals with one pixel format.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::Arc;

use crate::paths;

#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub has_audio: bool,
    /// ffmpeg colour-matrix name to assume for the input.
    pub matrix: String,
}

fn parse_rate(s: &str) -> f64 {
    let mut it = s.split('/');
    let n: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let d: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if d == 0.0 {
        0.0
    } else {
        n / d
    }
}

pub fn probe(path: &str) -> Result<VideoInfo> {
    if !std::path::Path::new(path).is_file() {
        bail!("File not found: {path}");
    }
    let out = paths::command("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format", path])
        .output()
        .map_err(|e| anyhow!("Could not run ffprobe ({e}). Put ffmpeg in the 'ffmpeg' folder next to the program."))?;
    if !out.status.success() {
        bail!("Could not read {path}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let mut duration: f64 = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let Some(video) = streams.iter().find(|s| s["codec_type"] == "video") else {
        return Ok(VideoInfo { path: path.into(), width: 0, height: 0, fps: 0.0, duration, has_audio, matrix: "bt709".into() });
    };
    let mut fps = parse_rate(video["avg_frame_rate"].as_str().unwrap_or("0/0"));
    if !(fps > 0.0) {
        fps = parse_rate(video["r_frame_rate"].as_str().unwrap_or("60/1"));
    }
    if !(fps > 0.0) {
        fps = 60.0;
    }
    if duration <= 0.0 {
        duration = video["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    }
    let width = video["width"].as_u64().unwrap_or(0) as u32;
    let height = video["height"].as_u64().unwrap_or(0) as u32;
    // Untagged HD video is BT.709 in practice; ffmpeg would otherwise assume BT.601.
    let matrix = match video["color_space"].as_str().unwrap_or("") {
        "smpte170m" | "bt470bg" => "bt601".to_string(),
        "bt2020nc" | "bt2020c" => "bt2020".to_string(),
        "fcc" => "fcc".to_string(),
        "smpte240m" => "smpte240m".to_string(),
        "bt709" => "bt709".to_string(),
        _ if height < 720 => "bt601".to_string(),
        _ => "bt709".to_string(),
    };
    Ok(VideoInfo { path: path.into(), width, height, fps, duration, has_audio, matrix })
}

/// An 8-bit 4:2:0 frame (BT.709, limited range).
#[derive(Clone, Debug)]
pub struct YuvFrame {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl YuvFrame {
    pub fn byte_size(w: usize, h: usize) -> usize {
        w * h + 2 * (w / 2) * (h / 2)
    }
    pub fn black(w: usize, h: usize) -> YuvFrame {
        let mut data = vec![16u8; w * h];
        data.resize(Self::byte_size(w, h), 128);
        YuvFrame { w, h, data }
    }
    pub fn y(&self) -> &[u8] {
        &self.data[..self.w * self.h]
    }
    pub fn u(&self) -> &[u8] {
        let n = self.w * self.h;
        &self.data[n..n + (self.w / 2) * (self.h / 2)]
    }
    pub fn v(&self) -> &[u8] {
        let n = self.w * self.h + (self.w / 2) * (self.h / 2);
        &self.data[n..]
    }
    pub fn cw(&self) -> usize {
        self.w / 2
    }
}

pub struct ReaderOptions {
    /// Output size (even numbers); the source is scaled if it differs.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub hwaccel: bool,
}

/// Streams frames from `start` seconds at a constant frame rate.
///
/// `start` may be negative: black frames are produced until the source begins.
/// After the source ends the last frame repeats, so a caller can always read
/// exactly as many frames as it needs.
pub struct VideoReader {
    child: Option<Child>,
    stdout: Option<ChildStdout>,
    w: usize,
    h: usize,
    pad: u64,
    last: Option<Arc<YuvFrame>>,
    pub eof: bool,
}

impl VideoReader {
    pub fn open(info: &VideoInfo, start: f64, opt: &ReaderOptions) -> Result<VideoReader> {
        let (w, h) = (opt.width as usize & !1, opt.height as usize & !1);
        let pad = if start < 0.0 { (-start * opt.fps).round() as u64 } else { 0 };
        let seek = start.max(0.0);
        let mut cmd = paths::command("ffmpeg");
        cmd.args(["-v", "error", "-nostdin"]);
        if opt.hwaccel {
            cmd.args(["-hwaccel", "auto"]);
        }
        if seek > 0.0 {
            cmd.args(["-ss", &format!("{seek:.6}")]);
        }
        let vf = format!(
            "fps={fps:.6},scale={w}:{h}:flags=bicubic:in_color_matrix={m}:out_color_matrix=bt709:out_range=tv,format=yuv420p",
            fps = opt.fps,
            m = info.matrix
        );
        cmd.args(["-i", &info.path, "-an", "-sn", "-dn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().context("Could not start ffmpeg")?;
        let stdout = child.stdout.take();
        Ok(VideoReader { child: Some(child), stdout, w, h, pad, last: None, eof: false })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// Next frame; `None` only means "end of stream and nothing was ever decoded".
    pub fn read(&mut self) -> Arc<YuvFrame> {
        if self.pad > 0 {
            self.pad -= 1;
            return Arc::new(YuvFrame::black(self.w, self.h));
        }
        if !self.eof {
            if let Some(out) = self.stdout.as_mut() {
                let mut buf = vec![0u8; YuvFrame::byte_size(self.w, self.h)];
                if out.read_exact(&mut buf).is_ok() {
                    let f = Arc::new(YuvFrame { w: self.w, h: self.h, data: buf });
                    self.last = Some(f.clone());
                    return f;
                }
            }
            self.eof = true;
        }
        self.last.clone().unwrap_or_else(|| Arc::new(YuvFrame::black(self.w, self.h)))
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        self.stdout.take();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Read one frame at time `t` (black before the start).
pub fn read_frame(info: &VideoInfo, t: f64, w: u32, h: u32) -> Result<Arc<YuvFrame>> {
    let mut r = VideoReader::open(info, t, &ReaderOptions { width: w, height: h, fps: info.fps, hwaccel: false })?;
    Ok(r.read())
}

/// Decode audio to interleaved f32 at `sr` Hz with `channels` channels.
pub fn load_audio(path: &str, sr: u32, channels: u16, start: f64, duration: Option<f64>) -> Result<Vec<f32>> {
    let mut cmd = paths::command("ffmpeg");
    cmd.args(["-v", "error", "-nostdin"]);
    if start > 0.0 {
        cmd.args(["-ss", &format!("{start:.6}")]);
    }
    cmd.args(["-i", path]);
    if let Some(d) = duration {
        cmd.args(["-t", &format!("{d:.6}")]);
    }
    cmd.args(["-vn", "-ac", &channels.to_string(), "-ar", &sr.to_string(), "-f", "f32le", "-"]);
    let out = cmd.stdin(Stdio::null()).output().context("Could not start ffmpeg")?;
    if !out.status.success() {
        bail!("Could not decode audio from {path}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}
