//! End-to-end tests on synthetic recordings: a fake "fight" (two coloured
//! fighters over a scrolling grid, clicks on the audio) rendered twice, with
//! and without UI, the UI recording starting 0.5 s earlier.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

use combo_editor::media::{probe, read_frame};
use combo_editor::project::{Keyframe, Markers, Project};
use combo_editor::{render, sync, tracking};

const W: usize = 640;
const H: usize = 360;
const FPS: f64 = 30.0;
const UI_LEAD: f64 = 0.5;
const SR: usize = 48000;
const CLICKS: [f64; 6] = [1.0, 1.7, 2.2, 3.1, 3.6, 4.4];

fn fighters(t: f64) -> (f64, f64) {
    let mid = 0.5 * W as f64 + 0.18 * W as f64 * (t * 1.3).sin();
    let gap = 0.12 * W as f64 + 0.04 * W as f64 * (t * 3.0).sin();
    (mid - gap / 2.0, mid + gap / 2.0)
}

fn scene(t: f64, with_ui: bool) -> Vec<u8> {
    let mut img = vec![120u8; W * H * 3];
    let pan = ((t * 40.0) as usize) % 32;
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) * 3;
            let mut v = 120u8;
            if (x + pan) % 32 < 2 || y % 32 < 2 {
                v = 90;
            }
            if y >= (H as f64 * 0.72) as usize {
                v += 15;
            }
            img[i..i + 3].fill(v);
        }
    }
    let (a, b) = fighters(t);
    let wobble = (3.0 * (t * 20.0).sin()) as i64;
    for (x, rgb) in [(a, [200u8, 40, 40]), (b, [40u8, 80, 200])] {
        let x0 = (x - 18.0) as usize;
        let y0 = ((H as f64 * 0.40) as i64 + wobble) as usize;
        for y in y0..(H as f64 * 0.75) as usize {
            for xx in x0..x0 + 36 {
                img[(y * W + xx) * 3..(y * W + xx) * 3 + 3].copy_from_slice(&rgb);
            }
        }
    }
    if with_ui {
        let s = W as f64 / 1920.0;
        for y in (52.0 * s) as usize..(98.0 * s) as usize {
            for x in (1040.0 * s) as usize..(1755.0 * s) as usize {
                img[(y * W + x) * 3..(y * W + x) * 3 + 3].copy_from_slice(&[230, 30, 30]);
            }
        }
    }
    img
}

fn audio(world_start: f64, secs: f64, seed: u64) -> Vec<f32> {
    let n = (secs * SR as f64) as usize;
    let mut s = seed;
    let mut rnd = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as f64 / (1u64 << 31) as f64) as f32 - 0.5
    };
    let mut x: Vec<f32> = (0..n).map(|_| 0.02 * rnd()).collect();
    for c in CLICKS {
        let i = ((c - world_start) * SR as f64) as i64;
        if i < 0 {
            continue;
        }
        for k in 0..2000usize {
            let t = k as f32 / SR as f32;
            if let Some(v) = x.get_mut(i as usize + k) {
                *v += 0.8 * (-t * 60.0).exp() * (2.0 * std::f32::consts::PI * 900.0 * t).sin();
            }
        }
    }
    x
}

fn encode(path: &Path, start: f64, with_ui: bool, secs: f64, seed: u64) {
    let wav = path.with_extension("f32");
    let a: Vec<u8> = audio(start, secs, seed).iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(&wav, a).unwrap();
    let mut child = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{W}x{H}"), "-r", "30", "-i", "-"])
        .args(["-f", "f32le", "-ar", &SR.to_string(), "-ac", "1", "-i"])
        .arg(&wav)
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "12", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("ffmpeg on PATH");
    let mut stdin = child.stdin.take().unwrap();
    for i in 0..(secs * FPS) as usize {
        stdin.write_all(&scene(i as f64 / FPS + start, with_ui)).unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    std::fs::remove_file(wav).unwrap();
}

struct Media {
    clean: String,
    ui: String,
    music: String,
    dir: PathBuf,
}

fn media() -> &'static Media {
    static M: OnceLock<Media> = OnceLock::new();
    M.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("combo-editor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Keep caches/presets out of the repo.
        std::env::set_var("COMBO_EDITOR_DATA", dir.join("data"));
        let (clean, ui, music) = (dir.join("clean.mp4"), dir.join("ui.mp4"), dir.join("music.wav"));
        encode(&clean, UI_LEAD, false, 5.0, 1);
        encode(&ui, 0.0, true, 5.0, 2);
        let st = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=220:duration=8"])
            .arg(&music)
            .status()
            .unwrap();
        assert!(st.success());
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        Media { clean: s(clean), ui: s(ui), music: s(music), dir }
    })
}

fn project(m: &Media) -> Project {
    let mut p = Project::default();
    p.sources.clean = m.clean.clone();
    p.sources.ui = m.ui.clone();
    p.sync.offset = UI_LEAD;
    p.timeline = Markers { t_in: 0.5, super_start: Some(2.0), anim_start: Some(2.5), post_start: Some(3.0), out: Some(4.0) };
    p.camera.super_.keyframes = vec![Keyframe { t: 2.5, cx: 320.0, cy: 150.0, zoom: 1.4, ease: None }];
    p
}

#[test]
fn sync_finds_offset_both_ways() {
    let m = media();
    let r = sync::sync_files(&m.ui, &m.clean, 60.0).unwrap();
    assert!((r.offset - UI_LEAD).abs() < 0.002, "{}", r.offset);
    assert!(r.confidence > 0.3);
    let r = sync::sync_files(&m.clean, &m.ui, 60.0).unwrap();
    assert!((r.offset + UI_LEAD).abs() < 0.002, "{}", r.offset);
}

#[test]
fn tracking_follows_fighters() {
    let m = media();
    let tr = tracking::track_video(&m.clean, &Default::default(), &|_| {}, &AtomicBool::new(false)).unwrap();
    assert!((tr.frames as i64 - 150).abs() <= 2, "{}", tr.frames);
    let cx = tracking::smooth(&tr.center_x, 2.0, 7);
    let mut errors: Vec<f64> = (5..tr.frames - 5)
        .map(|i| {
            let (a, b) = fighters(i as f64 / FPS + UI_LEAD);
            (cx[i] - (a + b) / 2.0).abs()
        })
        .collect();
    errors.sort_by(f64::total_cmp);
    let median = errors[errors.len() / 2];
    let max = *errors.last().unwrap();
    assert!(median < 0.02 * W as f64, "median {median}");
    assert!(max < 0.06 * W as f64, "max {max}");
}

#[test]
fn decoder_is_frame_accurate_with_negative_start() {
    let m = media();
    let info = probe(&m.ui).unwrap();
    assert_eq!((info.width, info.height), (640, 360));
    assert!((info.fps - FPS).abs() < 1e-6);
    let black = read_frame(&info, -0.2, 640, 360).unwrap();
    assert!(black.y().iter().all(|&v| v == 16));
    // The health bar exists in the UI video: red has low luma but high V.
    let f = read_frame(&info, 1.0, 640, 360).unwrap();
    let v = f.v()[(25 / 2) * 320 + 450 / 2];
    assert!(v > 200, "V = {v}");
}

#[test]
fn export_end_to_end() {
    let m = media();
    let mut p = project(m);
    p.music.path = m.music.clone();
    p.music.fade_in = 0.5;
    p.music.fade_out = 0.5;
    p.export.preset = "ultrafast".into();
    let out = m.dir.join("out.mp4");
    let done = render::export(&p, &out.to_string_lossy(), None, &|_, _| {}, &AtomicBool::new(false)).unwrap();
    let info = probe(&done.to_string_lossy()).unwrap();
    assert_eq!((info.width, info.height), (1080, 1920));
    assert!((info.duration - 3.5).abs() < 0.1, "{}", info.duration);
    assert!(info.has_audio);
    // The health bar from the UI video appears at its destination (y ~165..234).
    let f = read_frame(&info, 0.5, 1080, 1920).unwrap();
    let (y, v) = (f.y()[200 * 1080 + 540], f.v()[100 * 540 + 270]);
    assert!(v > 190 && y < 100, "Y {y} V {v}");
}

#[test]
fn cli_still_matches_layout() {
    let m = media();
    let p = project(m);
    let proj = m.dir.join("still.json");
    p.save(&proj).unwrap();
    let png = m.dir.join("still.png");
    let st = Command::new(env!("CARGO_BIN_EXE_combo-editor"))
        .args(["still", &proj.to_string_lossy(), "1.0", &png.to_string_lossy()])
        .env("COMBO_EDITOR_DATA", m.dir.join("data"))
        .status()
        .unwrap();
    assert!(st.success());
    let info = probe(&png.to_string_lossy()).unwrap();
    assert_eq!((info.width, info.height), (1080, 1920));
}
