//! The editor window.

mod jobs;
mod panels;
mod player;
mod style;
mod views;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use egui::{Context, Key};

use combo_editor::camera::{build_timeline, Timeline};
use combo_editor::imaging::ImageF;
use combo_editor::media::{load_audio, probe, VideoInfo};
use combo_editor::project::{MarkerKind, Part, Project};
use combo_editor::sync::SyncResult;
use combo_editor::tracking::{self, Track};
use combo_editor::{paths, presets, render, sync};

use jobs::Job;
use player::{Audio, Decoder, MusicParams};

/// Start the editor window. On Windows the default renderer is wgpu (DirectX 12);
/// if it cannot start, the program relaunches itself with OpenGL.
pub fn run(project: Option<String>, opengl: bool) -> Result<()> {
    let result = run_with(project.clone(), opengl);
    if result.is_err() && !opengl && cfg!(windows) {
        paths::log(&format!("wgpu renderer failed ({:#}); retrying with OpenGL", result.as_ref().unwrap_err()));
        let exe = std::env::current_exe()?;
        let mut cmd = std::process::Command::new(exe);
        cmd.args(std::env::args().skip(1)).arg("--opengl");
        let status = cmd.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    result
}

fn run_with(project: Option<String>, opengl: bool) -> Result<()> {
    let renderer = if opengl || !cfg!(windows) { eframe::Renderer::Glow } else { eframe::Renderer::Wgpu };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Combo Editor")
            .with_inner_size([1600.0, 960.0])
            .with_min_inner_size([1100.0, 680.0])
            .with_icon(Arc::new(style::icon())),
        renderer,
        ..Default::default()
    };
    eframe::run_native("Combo Editor", options, Box::new(move |cc| Ok(Box::new(EditorApp::new(&cc.egui_ctx, project)))))
        .map_err(|e| anyhow!("{e}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sources,
    Camera,
    Overlay,
    Export,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    Clean,
    Ui,
    Diff,
}

#[derive(Default)]
struct Jobs {
    sync: Option<Job<SyncResult>>,
    track: Option<Job<Track>>,
    export: Option<Job<PathBuf>>,
    still: Option<Job<ImageF>>,
    game_audio: Option<Job<Vec<f32>>>,
    music_audio: Option<Job<Vec<f32>>>,
    waveform: Option<Job<Vec<f32>>>,
}

/// What a texture was last computed from, so it is only rebuilt on change.
#[derive(Clone, Copy, PartialEq)]
struct TexKey {
    clean: i64,
    ui: i64,
    version: u64,
    w: usize,
    h: usize,
    mode: u8,
}

pub struct EditorApp {
    pub project: Project,
    snapshot: Project,
    version: u64,
    project_path: Option<PathBuf>,
    changed_at: Option<Instant>,

    clean: Option<VideoInfo>,
    ui_info: Option<VideoInfo>,
    timeline: Option<Arc<Timeline>>,
    track: Option<Arc<Track>>,
    waveform: Option<Vec<f32>>,
    envelopes: Option<(Vec<f32>, Vec<f32>)>,

    dec_clean: Option<Decoder>,
    dec_ui: Option<Decoder>,
    audio: Audio,
    audio_key: (String, String),

    playing: bool,
    t: f64,
    shown: (i64, i64),

    tab: Tab,
    source_mode: SourceMode,
    selected: Option<String>,
    preview_tex: Option<(egui::TextureHandle, TexKey)>,
    source_tex: Option<(egui::TextureHandle, TexKey)>,
    still_tex: Option<egui::TextureHandle>,
    last_export: Option<PathBuf>,

    jobs: Jobs,
    status: Option<(String, Instant, bool)>,

    // Presets.
    skin_list: Vec<String>,
    blend_list: Vec<String>,
    skin_pick: String,
    blend_pick: String,
    new_name: String,

    // View options.
    pub loop_play: bool,
    pub zoom_to_clip: bool,
    pub safe_zone: bool,
    pub music_preview: bool,
    pub fast_preview: bool,
    pub hw_decode: bool,
    drag: Option<views::Drag>,
    pub stats: Stats,
    /// $COMBO_EDITOR_SMOKE: render some frames, write "ok" to this file and quit (used by CI).
    smoke: Option<(PathBuf, u32)>,
}

/// Performance counters shown with F3.
#[derive(Default)]
pub struct Stats {
    pub show: bool,
    pub compose_ms: f32,
    pub composed: u64,
    frames: u64,
    window_start: Option<Instant>,
    composed_at_start: u64,
    pub ui_fps: f32,
    pub video_fps: f32,
}

impl Stats {
    fn tick(&mut self) {
        self.frames += 1;
        let start = *self.window_start.get_or_insert_with(Instant::now);
        let dt = start.elapsed().as_secs_f32();
        if dt >= 1.0 {
            self.ui_fps = self.frames as f32 / dt;
            self.video_fps = (self.composed - self.composed_at_start) as f32 / dt;
            self.frames = 0;
            self.composed_at_start = self.composed;
            self.window_start = Some(Instant::now());
        }
    }
}

fn autosave_path() -> PathBuf {
    paths::workspace().join("autosave.json")
}

impl EditorApp {
    fn new(ctx: &Context, project_arg: Option<String>) -> EditorApp {
        style::apply(ctx);
        let (project, project_path) = match project_arg.as_deref().map(|p| (Project::load(p.as_ref()), p)) {
            Some((Ok(p), path)) => (p, Some(PathBuf::from(path))),
            _ => (Project::load(&autosave_path()).unwrap_or_default(), None),
        };
        let mut app = EditorApp {
            snapshot: project.clone(),
            project,
            version: 0,
            project_path,
            changed_at: None,
            clean: None,
            ui_info: None,
            timeline: None,
            track: None,
            waveform: None,
            envelopes: None,
            dec_clean: None,
            dec_ui: None,
            audio: Audio::new(),
            audio_key: Default::default(),
            playing: false,
            t: 0.0,
            shown: (i64::MIN, i64::MIN),
            tab: Tab::Sources,
            source_mode: SourceMode::Clean,
            selected: None,
            preview_tex: None,
            source_tex: None,
            still_tex: None,
            last_export: None,
            jobs: Jobs::default(),
            status: None,
            skin_list: vec![],
            blend_list: vec![],
            skin_pick: "Default".into(),
            blend_pick: "Default".into(),
            new_name: String::new(),
            loop_play: true,
            zoom_to_clip: false,
            safe_zone: false,
            music_preview: true,
            fast_preview: false,
            hw_decode: true,
            drag: None,
            stats: Stats::default(),
            smoke: std::env::var_os("COMBO_EDITOR_SMOKE").map(|p| (PathBuf::from(p), 0)),
        };
        app.refresh_presets();
        app.selected = app.project.overlay.elements.first().map(|e| e.id.clone());
        if !app.project.sources.clean.is_empty() {
            app.load_sources(ctx);
        }
        if !app.audio.available {
            app.set_status("No audio output device found: previews will be silent", true);
        }
        app
    }

    pub fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), Instant::now(), error));
    }

    pub fn fps(&self) -> f64 {
        self.clean.as_ref().map_or(60.0, |c| c.fps)
    }

    pub fn duration(&self) -> f64 {
        self.clean.as_ref().map_or(0.0, |c| c.duration)
    }

    pub fn src_size(&self) -> (f64, f64) {
        self.clean.as_ref().map_or((1920.0, 1080.0), |c| (c.width as f64, c.height as f64))
    }

    pub fn frame_of(&self, t: f64) -> i64 {
        (t * self.fps() + 1e-6).floor() as i64
    }

    pub fn snap(&self, t: f64) -> f64 {
        (t * self.fps()).round() / self.fps()
    }

    pub fn markers(&self) -> combo_editor::camera::Resolved {
        match &self.timeline {
            Some(tl) => tl.markers,
            None => combo_editor::camera::resolve_markers(&self.project.timeline, self.duration()),
        }
    }

    pub fn part_at(&self, t: f64) -> Part {
        self.markers().part_at(t)
    }

    pub fn refresh_presets(&mut self) {
        self.skin_list = presets::list(presets::Kind::Skins);
        self.blend_list = presets::list(presets::Kind::Blends);
        if !self.skin_list.contains(&self.skin_pick) {
            self.skin_pick = self.project.overlay.name.clone();
        }
    }

    // ------------------------------------------------------------ loading

    pub fn load_sources(&mut self, ctx: &Context) {
        self.playing = false;
        self.audio.pause();
        self.dec_clean = None;
        self.dec_ui = None;
        self.clean = None;
        self.ui_info = None;
        self.track = None;
        self.waveform = None;
        self.preview_tex = None;
        self.source_tex = None;
        let s = self.project.sources.clone();
        if s.clean.is_empty() {
            self.set_status("Choose at least the clean (no-UI) video", true);
            return;
        }
        let clean = match probe(&s.clean) {
            Ok(i) => i,
            Err(e) => return self.set_status(format!("{e:#}"), true),
        };
        let ui = if s.ui.is_empty() {
            None
        } else {
            match probe(&s.ui) {
                Ok(i) => Some(i),
                Err(e) => return self.set_status(format!("{e:#}"), true),
            }
        };
        if let Some(u) = &ui {
            if (u.fps - clean.fps).abs() > 0.01 {
                self.set_status("The two videos have different frame rates; the UI video is resampled to match", true);
            }
        }
        if self.project.timeline.out.is_none() {
            self.project.timeline.out = Some(clean.duration);
        }
        self.track = tracking::cached(&s.clean, &self.project.tracking).map(Arc::new);
        self.clean = Some(clean);
        self.ui_info = ui;
        self.create_decoders(ctx);
        self.audio_key = Default::default();
        let path = s.clean.clone();
        self.jobs.waveform = Some(Job::spawn(ctx, move |_, _| {
            let x = load_audio(&path, 4000, 1, 0.0, None)?;
            Ok(x.chunks(80).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect())
        }));
        self.rebuild_timeline();
        self.t = self.t.clamp(0.0, self.duration());
        if self.status.as_ref().is_none_or(|s| !s.2) {
            self.set_status("Videos loaded", false);
        }
    }

    fn create_decoders(&mut self, ctx: &Context) {
        let fps = self.fps();
        let size = |i: &VideoInfo, fast: bool| {
            let d = if fast { 2 } else { 1 };
            ((i.width / d) & !1, (i.height / d) & !1)
        };
        self.dec_clean = self.clean.clone().map(|i| Decoder::new(i.clone(), fps, size(&i, self.fast_preview), self.hw_decode, ctx.clone()));
        self.dec_ui = self.ui_info.clone().map(|i| Decoder::new(i.clone(), fps, size(&i, self.fast_preview), self.hw_decode, ctx.clone()));
        self.shown = (i64::MIN, i64::MIN);
    }

    pub fn reload_decoders(&mut self, ctx: &Context) {
        if self.clean.is_some() {
            self.create_decoders(ctx);
        }
    }

    fn rebuild_timeline(&mut self) {
        if let Some(c) = &self.clean {
            let tl = build_timeline(&self.project, self.track.as_deref(), c.fps, c.width as f64, c.height as f64, c.duration);
            self.timeline = Some(Arc::new(tl));
        }
    }

    /// Detect edits by comparing with the last snapshot.
    fn detect_changes(&mut self) {
        if self.project == self.snapshot {
            if let Some(at) = self.changed_at {
                if at.elapsed().as_secs_f32() > 1.0 {
                    let _ = self.project.save(&autosave_path());
                    self.changed_at = None;
                }
            }
            return;
        }
        if self.project.tracking != self.snapshot.tracking {
            self.track = tracking::cached(&self.project.sources.clean, &self.project.tracking).map(Arc::new);
        }
        self.snapshot = self.project.clone();
        self.version += 1;
        self.changed_at = Some(Instant::now());
        self.rebuild_timeline();
    }

    fn sync_audio(&mut self, ctx: &Context) {
        let p = &self.project;
        let primary_ui = p.audio.primary == "ui" && self.ui_info.is_some();
        let game_path = if primary_ui { p.sources.ui.clone() } else { p.sources.clean.clone() };
        let game_path = if self.clean.is_some() { game_path } else { String::new() };
        let key = (game_path.clone(), p.music.path.clone());
        if key != self.audio_key {
            let rate = self.audio.rate;
            if key.0 != self.audio_key.0 {
                self.audio.set_game(None, 0.0, 1.0);
                if !key.0.is_empty() {
                    let path = key.0.clone();
                    self.jobs.game_audio = Some(Job::spawn(ctx, move |_, _| load_audio(&path, rate, 2, 0.0, None)));
                }
            }
            if key.1 != self.audio_key.1 {
                self.audio.set_music(None);
                if !key.1.is_empty() {
                    let path = key.1.clone();
                    self.jobs.music_audio = Some(Job::spawn(ctx, move |_, _| load_audio(&path, rate, 2, 0.0, None)));
                }
            }
            self.audio_key = key;
        }
        let offset = if primary_ui { p.sync.offset } else { 0.0 };
        self.audio.set_game_params(offset, p.audio.game_volume as f32);
        let m = self.markers();
        self.audio.set_music_params(MusicParams {
            start: p.music.start,
            volume: p.music.volume as f32,
            fade_in: p.music.fade_in,
            fade_out: p.music.fade_out,
            t_in: m.t_in,
            t_out: m.out,
            enabled: self.music_preview,
        });
    }

    fn poll_jobs(&mut self, ctx: &Context) {
        if let Some(r) = jobs::take(&mut self.jobs.waveform) {
            self.waveform = r.ok();
        }
        if let Some(r) = jobs::take(&mut self.jobs.game_audio) {
            match r {
                Ok(s) => self.audio.set_game(Some(Arc::new(s)), 0.0, 1.0),
                Err(e) => self.set_status(format!("Game audio: {e:#}"), true),
            }
        }
        if let Some(r) = jobs::take(&mut self.jobs.music_audio) {
            match r {
                Ok(s) => self.audio.set_music(Some(Arc::new(s))),
                Err(e) => self.set_status(format!("Music: {e:#}"), true),
            }
        }
        if let Some(r) = jobs::take(&mut self.jobs.sync) {
            match r {
                Ok(s) => {
                    self.project.sync.offset = (s.offset * 10000.0).round() / 10000.0;
                    self.project.sync.confidence = Some(s.confidence);
                    self.set_status(format!("Synced: offset {:.3} s", s.offset), false);
                    self.envelopes = Some((s.env_ui, s.env_clean));
                }
                Err(e) => self.set_status(format!("Sync failed: {e:#}"), true),
            }
        }
        if let Some(r) = jobs::take(&mut self.jobs.track) {
            match r {
                Ok(t) => {
                    self.track = Some(Arc::new(t));
                    self.rebuild_timeline();
                    self.version += 1;
                    self.set_status("Fighters tracked", false);
                }
                Err(e) => self.set_status(format!("Tracking: {e:#}"), true),
            }
        }
        if let Some(r) = jobs::take(&mut self.jobs.export) {
            match r {
                Ok(p) => {
                    self.set_status(format!("Exported {}", p.display()), false);
                    self.last_export = Some(p);
                }
                Err(e) => self.set_status(format!("Export: {e:#}"), true),
            }
        }
        if let Some(r) = jobs::take(&mut self.jobs.still) {
            match r {
                Ok(img) => {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.to_rgba8());
                    self.still_tex = Some(ctx.load_texture("still", ci, egui::TextureOptions::LINEAR));
                    self.set_status("Exact frame rendered", false);
                }
                Err(e) => self.set_status(format!("Exact frame: {e:#}"), true),
            }
        }
    }

    // ------------------------------------------------------------ actions

    pub fn start_sync(&mut self, ctx: &Context) {
        let (ui, clean) = (self.project.sources.ui.clone(), self.project.sources.clean.clone());
        if ui.is_empty() || clean.is_empty() {
            return self.set_status("Load both videos first", true);
        }
        self.jobs.sync = Some(Job::spawn(ctx, move |_, _| sync::sync_files(&ui, &clean, 60.0)));
    }

    pub fn start_track(&mut self, ctx: &Context) {
        let clean = self.project.sources.clean.clone();
        if clean.is_empty() {
            return self.set_status("Load the clean video first", true);
        }
        let cfg = self.project.tracking.clone();
        self.jobs.track = Some(Job::spawn(ctx, move |p, cancel| tracking::track_video(&clean, &cfg, &|f| p(f, "Analysing"), cancel)));
    }

    pub fn start_export(&mut self, ctx: &Context) {
        if self.project.sources.clean.is_empty() {
            return self.set_status("Load videos first", true);
        }
        let project = self.project.clone();
        let track = self.track.clone();
        self.last_export = None;
        self.jobs.export = Some(Job::spawn(ctx, move |p, cancel| {
            render::export(&project, &project.export.output, track.as_deref(), p, cancel)
        }));
    }

    pub fn start_still(&mut self, ctx: &Context) {
        if self.project.sources.clean.is_empty() {
            return self.set_status("Load videos first", true);
        }
        let project = self.project.clone();
        let track = self.track.clone();
        let t = self.t;
        self.jobs.still = Some(Job::spawn(ctx, move |_, cancel| {
            let prepared = render::prepare(&project, track.as_deref(), &|_| {}, cancel)?;
            render::render_still(&project, &prepared, t)
        }));
    }

    pub fn seek(&mut self, t: f64) {
        self.t = t.clamp(0.0, self.duration().max(0.0));
        if self.playing {
            self.audio.play(self.t);
        }
    }

    pub fn toggle_play(&mut self) {
        if self.clean.is_none() {
            return;
        }
        if self.playing {
            self.playing = false;
            self.audio.pause();
            self.t = self.snap(self.t);
        } else {
            let m = self.markers();
            if self.t < m.t_in || self.t >= m.out - 1.0 / self.fps() {
                self.t = m.t_in;
            }
            self.playing = true;
            self.audio.play(self.t);
        }
    }

    pub fn step(&mut self, frames: f64) {
        self.playing = false;
        self.audio.pause();
        let t = self.snap(self.t) + frames / self.fps();
        self.seek(t);
    }

    pub fn set_marker(&mut self, k: MarkerKind, t: Option<f64>) {
        let t = t.map(|t| (self.snap(t) * 10000.0).round() / 10000.0);
        self.project.timeline.set(k, t);
    }

    pub fn save_project_as(&mut self) {
        let dir = presets::dir(presets::Kind::Projects);
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(&dir)
            .set_file_name(format!("{}.json", self.project.name))
            .add_filter("Combo Editor project", &["json"])
            .save_file()
        {
            self.project_path = Some(path);
            self.save_project();
        }
    }

    pub fn save_project(&mut self) {
        let Some(path) = self.project_path.clone() else { return self.save_project_as() };
        match self.project.save(&path) {
            Ok(()) => self.set_status(format!("Saved {}", path.display()), false),
            Err(e) => self.set_status(format!("Save failed: {e:#}"), true),
        }
    }

    pub fn open_project(&mut self, ctx: &Context) {
        let dir = presets::dir(presets::Kind::Projects);
        if let Some(path) = rfd::FileDialog::new().set_directory(&dir).add_filter("Combo Editor project", &["json"]).pick_file() {
            match Project::load(&path) {
                Ok(p) => {
                    self.project = p;
                    self.project_path = Some(path);
                    self.selected = self.project.overlay.elements.first().map(|e| e.id.clone());
                    self.envelopes = None;
                    self.load_sources(ctx);
                }
                Err(e) => self.set_status(format!("Could not open project: {e:#}"), true),
            }
        }
    }

    pub fn new_project(&mut self, ctx: &Context) {
        self.project = Project::default();
        self.project_path = None;
        self.envelopes = None;
        self.timeline = None;
        self.load_sources(ctx);
        self.status = None;
    }

    // ------------------------------------------------------------ frame loop

    fn handle_keys(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::F3)) {
            self.stats.show = !self.stats.show;
        }
        let (space, left, right, shift, i, o, k) = ctx.input(|inp| {
            (
                inp.key_pressed(Key::Space),
                inp.key_pressed(Key::ArrowLeft),
                inp.key_pressed(Key::ArrowRight),
                inp.modifiers.shift,
                inp.key_pressed(Key::I),
                inp.key_pressed(Key::O),
                inp.key_pressed(Key::K),
            )
        });
        let step = if shift { self.fps() } else { 1.0 };
        if space {
            self.toggle_play();
        }
        if left {
            self.step(-step);
        }
        if right {
            self.step(step);
        }
        if i {
            self.set_marker(MarkerKind::In, Some(self.t));
        }
        if o {
            self.set_marker(MarkerKind::Out, Some(self.t));
        }
        if k {
            self.edit_keyframe(|c| c);
        }
    }

    fn advance_playback(&mut self, ctx: &Context) {
        if !self.playing {
            return;
        }
        let m = self.markers();
        let t = self.audio.time();
        if t >= m.out {
            if self.loop_play {
                self.t = m.t_in;
                self.audio.play(m.t_in);
            } else {
                self.playing = false;
                self.audio.pause();
                self.t = m.out;
            }
        } else {
            self.t = t;
        }
        ctx.request_repaint();
    }

    fn request_frames(&mut self) {
        let i = self.frame_of(self.t);
        let ahead = if self.playing { (self.fps() / 3.0) as i64 } else { 3 };
        let offset_frames = (self.project.sync.offset * self.fps()).round() as i64;
        if let Some(d) = &self.dec_clean {
            d.request(i, ahead);
        }
        if let Some(d) = &self.dec_ui {
            d.request(i + offset_frames, ahead);
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.stats.tick();
        if self.stats.show {
            ctx.request_repaint();
        }
        self.poll_jobs(&ctx);
        self.detect_changes();
        self.sync_audio(&ctx);
        self.advance_playback(&ctx);
        self.handle_keys(&ctx);
        self.request_frames();
        // Keep the progress bars moving.
        if self.jobs.export.is_some() || self.jobs.track.is_some() || self.jobs.sync.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::Panel::top("top").frame(style::bar_frame()).show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("timeline").frame(style::bar_frame()).resizable(false).show(ui, |ui| self.timeline_panel(ui));
        egui::Panel::left("panel").frame(style::side_frame()).resizable(true).default_size(380.0).show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.monitors(ui));
        self.still_window(&ctx);

        if let Some((path, frames)) = &mut self.smoke {
            *frames += 1;
            ctx.request_repaint();
            // Wait until the first decoded frame has been composed (or give up after a while).
            if (self.stats.composed > 0 || self.clean.is_none()) && *frames > 30 || *frames > 600 {
                let ok = self.stats.composed > 0 || self.clean.is_none();
                let _ = std::fs::write(&*path, if ok { "ok" } else { "no frame decoded" });
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.project.save(&autosave_path());
    }
}
