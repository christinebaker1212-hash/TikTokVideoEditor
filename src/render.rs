//! Final export: decode both recordings in lockstep, compose every frame,
//! and pipe it to ffmpeg (x264) together with the audio mix.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::camera::{build_timeline, Timeline};
use crate::compositor::compose;
use crate::imaging::{Filter, ImageF};
use crate::media::{probe, read_frame, ReaderOptions, VideoInfo, VideoReader, YuvFrame};
use crate::paths;
use crate::project::{Project, OUT_H, OUT_W};
use crate::tracking::{self, Track};

pub struct Prepared {
    pub clean: VideoInfo,
    pub ui: Option<VideoInfo>,
    pub timeline: Timeline,
}

/// Probe sources and build the timeline, tracking the fighters if needed.
pub fn prepare(project: &Project, track: Option<&Track>, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<Prepared> {
    if project.sources.clean.is_empty() {
        bail!("No clean (no-UI) video selected");
    }
    let clean = probe(&project.sources.clean)?;
    let ui = if project.sources.ui.is_empty() { None } else { Some(probe(&project.sources.ui)?) };
    let owned;
    let track = match track {
        Some(t) => t,
        None => {
            owned = tracking::track_video(&project.sources.clean, &project.tracking, progress, cancel)?;
            &owned
        }
    };
    let timeline = build_timeline(project, Some(track), clean.fps, clean.width as f64, clean.height as f64, clean.duration);
    Ok(Prepared { clean, ui, timeline })
}

/// Export-identical composite of the frame at clean time `t`.
pub fn render_still(project: &Project, p: &Prepared, t: f64) -> Result<ImageF> {
    let clean = read_frame(&p.clean, t, p.clean.width, p.clean.height)?;
    let ui = match &p.ui {
        Some(info) => Some(read_frame(info, t + project.sync.offset, info.width, info.height)?),
        None => None,
    };
    let fc = p.timeline.sample(t);
    Ok(compose(&clean, ui.as_deref(), &fc, &project.overlay, p.clean.height as f64, OUT_W as usize, OUT_H as usize,
        Filter::from_name(&project.export.scaler), project.camera.framing.fill != "black"))
}

/// Write an RGB image as PNG (via ffmpeg, which is bundled anyway).
pub fn save_png(img: &ImageF, path: &str) -> Result<()> {
    let mut child = paths::command("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{}x{}", img.w, img.h), "-i", "-", path])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(&img.to_rgba8())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("Could not write {path}: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// ffmpeg inputs and filter graph for game audio (+ optional music).
fn audio_args(project: &Project, clean: &VideoInfo, ui: Option<&VideoInfo>, start: f64, duration: f64) -> (Vec<String>, Option<String>) {
    let mut inputs: Vec<String> = vec![];
    let mut chains = vec![];
    let mut labels = vec![];
    let mut idx = 1;
    let a = &project.audio;
    let (src, offset) = match (a.primary.as_str(), ui) {
        ("ui", Some(ui)) => (ui, project.sync.offset),
        _ => (clean, 0.0),
    };
    let a_start = start + offset;
    if src.has_audio {
        inputs.extend(["-ss".into(), format!("{:.6}", a_start.max(0.0)), "-t".into(), format!("{duration:.6}"), "-i".into(), src.path.clone()]);
        let mut chain = format!("[{idx}:a]aresample=48000,volume={:.4}", a.game_volume);
        if a_start < 0.0 {
            chain += &format!(",adelay={}:all=1", (-a_start * 1000.0).round() as i64);
        }
        if a.game_fade_out > 0.0 {
            chain += &format!(",afade=t=out:st={:.4}:d={:.4}", (duration - a.game_fade_out).max(0.0), a.game_fade_out);
        }
        chains.push(chain + "[game]");
        labels.push("[game]");
        idx += 1;
    }
    let m = &project.music;
    if !m.path.is_empty() {
        inputs.extend(["-ss".into(), format!("{:.6}", m.start.max(0.0)), "-t".into(), format!("{duration:.6}"), "-i".into(), m.path.clone()]);
        let mut chain = format!("[{idx}:a]aresample=48000,volume={:.4}", m.volume);
        if m.fade_in > 0.0 {
            chain += &format!(",afade=t=in:st=0:d={:.4}", m.fade_in);
        }
        if m.fade_out > 0.0 {
            chain += &format!(",afade=t=out:st={:.4}:d={:.4}", (duration - m.fade_out).max(0.0), m.fade_out);
        }
        chains.push(chain + "[music]");
        labels.push("[music]");
    }
    if labels.is_empty() {
        return (inputs, None);
    }
    let tail = format!("apad,atrim=0:{duration:.6},alimiter=limit=0.97[aout]");
    let graph = if labels.len() == 1 {
        format!("{};{}{}", chains.join(";"), labels[0], tail)
    } else {
        format!("{};{}amix=inputs={}:normalize=0:duration=longest,{}", chains.join(";"), labels.concat(), labels.len(), tail)
    };
    (inputs, Some(graph))
}

pub fn default_output(project: &Project) -> PathBuf {
    let name = if project.name.trim().is_empty() { "export" } else { project.name.trim() };
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).collect();
    paths::workspace().join("exports").join(format!("{safe}.mp4"))
}

/// Render the project to `output`. `progress(fraction, message)`.
pub fn export(project: &Project, output: &str, track: Option<&Track>, progress: &dyn Fn(f32, &str), cancel: &AtomicBool) -> Result<PathBuf> {
    progress(0.0, "Analysing fighters");
    let p = prepare(project, track, &|f| progress(f * 0.05, "Analysing fighters"), cancel)?;
    let fps = p.clean.fps;
    let tl = Arc::new(p.timeline);
    let n = tl.len();
    let start = tl.markers.t_in;
    let duration = n as f64 / fps;
    let out_path = if output.is_empty() { default_output(project) } else { PathBuf::from(output) };
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let (a_inputs, graph) = audio_args(project, &p.clean, p.ui.as_ref(), start, duration);
    let ex = &project.export;
    let mut cmd = paths::command("ffmpeg");
    cmd.args(["-y", "-v", "error", "-nostdin", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s"])
        .arg(format!("{OUT_W}x{OUT_H}"))
        .args(["-r", &format!("{fps:.6}"), "-color_range", "tv", "-colorspace", "bt709", "-i", "-"])
        .args(&a_inputs);
    if let Some(g) = &graph {
        cmd.args(["-filter_complex", g, "-map", "0:v", "-map", "[aout]", "-c:a", "aac", "-b:a", &ex.audio_bitrate]);
    }
    cmd.args(["-c:v", "libx264", "-preset", &ex.preset, "-crf", &ex.crf.to_string(), "-pix_fmt", "yuv420p",
        "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv",
        "-movflags", "+faststart"])
        .arg(&out_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut enc = cmd.spawn().context("Could not start ffmpeg")?;
    let mut stdin = enc.stdin.take().unwrap();
    let stderr = enc.stderr.take().unwrap();
    let err_thread = thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(stderr), &mut s);
        s
    });

    // Decoders run ahead on their own threads; frames arrive in order.
    let spawn_reader = |info: VideoInfo, t0: f64| -> Result<mpsc::Receiver<Arc<YuvFrame>>> {
        let opt = ReaderOptions { width: info.width, height: info.height, fps, hwaccel: false };
        let mut r = VideoReader::open(&info, t0, &opt)?;
        let (tx, rx) = mpsc::sync_channel(6);
        thread::spawn(move || {
            for _ in 0..n {
                if tx.send(r.read()).is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    };
    let clean_rx = spawn_reader(p.clean.clone(), start)?;
    let ui_rx = match &p.ui {
        Some(info) => Some(spawn_reader(info.clone(), start + project.sync.offset)?),
        None => None,
    };
    let (wtx, wrx) = mpsc::sync_channel::<Vec<u8>>(4);
    let writer = thread::spawn(move || -> std::io::Result<()> {
        for buf in wrx {
            stdin.write_all(&buf)?;
        }
        Ok(())
    });

    let filter = Filter::from_name(&ex.scaler);
    let blur_fill = project.camera.framing.fill != "black";
    let skin = &project.overlay;
    let src_h = p.clean.height as f64;
    let mut result: Result<()> = Ok(());
    for i in 0..n {
        if cancel.load(Ordering::Relaxed) {
            result = Err(anyhow!("Cancelled"));
            break;
        }
        let clean = clean_rx.recv().map_err(|_| anyhow!("decoder stopped"))?;
        let ui = match &ui_rx {
            Some(rx) => Some(rx.recv().map_err(|_| anyhow!("decoder stopped"))?),
            None => None,
        };
        let img = compose(&clean, ui.as_deref(), &tl.frame(i), skin, src_h, OUT_W as usize, OUT_H as usize, filter, blur_fill);
        if wtx.send(img.to_yuv420()).is_err() {
            break; // encoder died; its error is reported below
        }
        if i % 5 == 0 {
            progress(0.05 + 0.94 * i as f32 / n as f32, &format!("Rendering frame {i} / {n}"));
        }
    }
    drop(wtx);
    let _ = writer.join();
    if result.is_err() {
        let _ = enc.kill();
    }
    let status = enc.wait()?;
    let err = err_thread.join().unwrap_or_default();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&out_path);
        return Err(e);
    }
    if !status.success() {
        let _ = std::fs::remove_file(&out_path);
        bail!("ffmpeg failed: {}", err.trim());
    }
    progress(1.0, "Done");
    Ok(out_path)
}
