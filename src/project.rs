//! Project, UI-skin and blend-preset documents. Stored as JSON (camelCase keys).

use serde::{Deserialize, Serialize};

/// Output (TikTok) frame size.
pub const OUT_W: u32 = 1080;
pub const OUT_H: u32 = 1920;
pub const OUT_ASPECT: f64 = OUT_W as f64 / OUT_H as f64;
/// Skin coordinates are authored against a 1920x1080 source.
pub const REF_W: f64 = 1920.0;
pub const REF_H: f64 = 1080.0;

/// The four parts of every video, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Combo = 0,
    Startup = 1,
    Animation = 2,
    Post = 3,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Combo, Part::Startup, Part::Animation, Part::Post];
    pub fn from_index(i: u8) -> Part {
        Part::ALL[(i as usize).min(3)]
    }
    pub fn label(self) -> &'static str {
        match self {
            Part::Combo => "Combo",
            Part::Startup => "Super startup",
            Part::Animation => "Super animation",
            Part::Post => "Post super",
        }
    }
    pub fn is_super(self) -> bool {
        matches!(self, Part::Startup | Part::Animation)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Feather {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Parts {
    pub combo: bool,
    pub startup: bool,
    pub animation: bool,
    pub post: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Parts { combo: true, startup: true, animation: false, post: true }
    }
}

impl Parts {
    pub fn get(&self, p: Part) -> bool {
        match p {
            Part::Combo => self.combo,
            Part::Startup => self.startup,
            Part::Animation => self.animation,
            Part::Post => self.post,
        }
    }
    pub fn get_mut(&mut self, p: Part) -> &mut bool {
        match p {
            Part::Combo => &mut self.combo,
            Part::Startup => &mut self.startup,
            Part::Animation => &mut self.animation,
            Part::Post => &mut self.post,
        }
    }
}

/// A UI element cut from the UI recording (`src`, reference pixels) and placed
/// on the output frame (`dst`, output pixels).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Element {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub src: Rect,
    pub dst: Rect,
    pub opacity: f64,
    pub feather: Feather,
    pub parts: Parts,
}

impl Default for Element {
    fn default() -> Self {
        Element {
            id: "el1".into(),
            label: "New element".into(),
            enabled: true,
            src: Rect::new(760.0, 400.0, 400.0, 120.0),
            dst: Rect::new(240.0, 500.0, 600.0, 180.0),
            opacity: 1.0,
            feather: Feather::default(),
            parts: Parts::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Skin {
    pub name: String,
    pub reference_size: [f64; 2],
    /// Seconds over which an element fades when shown/hidden between parts.
    pub fade: f64,
    pub elements: Vec<Element>,
}

impl Default for Skin {
    fn default() -> Self {
        Skin {
            name: "Default".into(),
            reference_size: [REF_W, REF_H],
            fade: 0.2,
            elements: vec![
                Element {
                    id: "health".into(),
                    label: "Enemy health bar".into(),
                    enabled: true,
                    src: Rect::new(1040.0, 52.0, 715.0, 46.0),
                    dst: Rect::new(0.0, 165.0, 1080.0, 69.0),
                    opacity: 1.0,
                    feather: Feather { left: 0.0, right: 0.0, top: 4.0, bottom: 4.0 },
                    parts: Parts::default(),
                },
                Element {
                    id: "damage".into(),
                    label: "Combo damage / attack type".into(),
                    enabled: true,
                    src: Rect::new(623.0, 173.0, 244.0, 90.0),
                    dst: Rect::new(348.0, 252.0, 390.0, 144.0),
                    opacity: 0.92,
                    feather: Feather { left: 6.0, right: 6.0, top: 4.0, bottom: 4.0 },
                    parts: Parts::default(),
                },
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Blend {
    pub name: String,
    pub opacity: f64,
    pub feather: Feather,
}

impl Default for Blend {
    fn default() -> Self {
        Blend { name: "Default".into(), opacity: 1.0, feather: Feather { left: 0.0, right: 0.0, top: 4.0, bottom: 4.0 } }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sources {
    pub ui: String,
    pub clean: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncCfg {
    /// ui_time = clean_time + offset. The clean video is the master timeline.
    pub offset: f64,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioCfg {
    /// "clean" or "ui": which recording supplies the game audio.
    pub primary: String,
    pub game_volume: f64,
    pub game_fade_out: f64,
}

impl Default for AudioCfg {
    fn default() -> Self {
        AudioCfg { primary: "clean".into(), game_volume: 1.0, game_fade_out: 0.0 }
    }
}

/// Part markers, seconds on the clean video's timeline. `None` = not set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Markers {
    #[serde(rename = "in")]
    pub t_in: f64,
    pub super_start: Option<f64>,
    pub anim_start: Option<f64>,
    pub post_start: Option<f64>,
    pub out: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    In,
    SuperStart,
    AnimStart,
    PostStart,
    Out,
}

impl MarkerKind {
    pub const ALL: [MarkerKind; 5] =
        [MarkerKind::In, MarkerKind::SuperStart, MarkerKind::AnimStart, MarkerKind::PostStart, MarkerKind::Out];
    pub fn label(self) -> &'static str {
        match self {
            MarkerKind::In => "In",
            MarkerKind::SuperStart => "Super start",
            MarkerKind::AnimStart => "Animation start",
            MarkerKind::PostStart => "Post start",
            MarkerKind::Out => "Out",
        }
    }
}

impl Markers {
    pub fn get(&self, k: MarkerKind) -> Option<f64> {
        match k {
            MarkerKind::In => Some(self.t_in),
            MarkerKind::SuperStart => self.super_start,
            MarkerKind::AnimStart => self.anim_start,
            MarkerKind::PostStart => self.post_start,
            MarkerKind::Out => self.out,
        }
    }

    /// Set a marker and push the others so they stay in order.
    pub fn set(&mut self, k: MarkerKind, v: Option<f64>) {
        match k {
            MarkerKind::In => self.t_in = v.unwrap_or(0.0).max(0.0),
            MarkerKind::SuperStart => self.super_start = v,
            MarkerKind::AnimStart => self.anim_start = v,
            MarkerKind::PostStart => self.post_start = v,
            MarkerKind::Out => self.out = v,
        }
        let Some(v) = v else { return };
        let idx = MarkerKind::ALL.iter().position(|m| *m == k).unwrap();
        for (i, other) in MarkerKind::ALL.iter().enumerate() {
            let Some(ov) = self.get(*other) else { continue };
            let fixed = if i > idx && ov < v {
                v
            } else if i < idx && ov > v {
                v
            } else {
                continue;
            };
            match other {
                MarkerKind::In => self.t_in = fixed,
                MarkerKind::SuperStart => self.super_start = Some(fixed),
                MarkerKind::AnimStart => self.anim_start = Some(fixed),
                MarkerKind::PostStart => self.post_start = Some(fixed),
                MarkerKind::Out => self.out = Some(fixed),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrackingCfg {
    pub motion_weight: f64,
    pub color_weight: f64,
    /// Percent of the action trimmed at each edge when measuring its extent.
    pub percentile: f64,
}

impl Default for TrackingCfg {
    fn default() -> Self {
        TrackingCfg { motion_weight: 1.0, color_weight: 1.0, percentile: 4.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ComboCam {
    pub offset_x: f64,
    pub smoothing: f64,
}

impl Default for ComboCam {
    fn default() -> Self {
        ComboCam { offset_x: 0.0, smoothing: 0.35 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyframe {
    pub t: f64,
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ease: Option<String>,
}

impl Default for Keyframe {
    fn default() -> Self {
        Keyframe { t: 0.0, cx: 960.0, cy: 540.0, zoom: 1.0, ease: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuperCam {
    pub easing: String,
    pub keyframes: Vec<Keyframe>,
}

impl Default for SuperCam {
    fn default() -> Self {
        SuperCam { easing: "easeInOut".into(), keyframes: vec![] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PostCam {
    pub offset_x: f64,
    pub offset_y: f64,
    pub zoom: f64,
    pub smoothing: f64,
}

impl Default for PostCam {
    fn default() -> Self {
        PostCam { offset_x: 0.0, offset_y: 0.0, zoom: 1.0, smoothing: 0.5 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Transitions {
    pub combo_to_super: f64,
    pub super_to_post: f64,
}

impl Default for Transitions {
    fn default() -> Self {
        Transitions { combo_to_super: 0.0, super_to_post: 0.5 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraCfg {
    pub combo: ComboCam,
    #[serde(rename = "super")]
    pub super_: SuperCam,
    pub post: PostCam,
    pub transitions: Transitions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Music {
    pub path: String,
    /// Where in the song playback starts (seconds).
    pub start: f64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl Default for Music {
    fn default() -> Self {
        Music { path: String::new(), start: 0.0, volume: 0.8, fade_in: 1.0, fade_out: 2.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportCfg {
    pub output: String,
    pub crf: u32,
    pub preset: String,
    pub audio_bitrate: String,
    /// "lanczos" (best), "cubic" or "linear".
    pub scaler: String,
}

impl Default for ExportCfg {
    fn default() -> Self {
        ExportCfg {
            output: String::new(),
            crf: 16,
            preset: "slow".into(),
            audio_bitrate: "256k".into(),
            scaler: "lanczos".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub sources: Sources,
    pub sync: SyncCfg,
    pub audio: AudioCfg,
    pub timeline: Markers,
    pub tracking: TrackingCfg,
    pub camera: CameraCfg,
    pub overlay: Skin,
    pub music: Music,
    pub export: ExportCfg,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            version: 1,
            name: "untitled".into(),
            sources: Sources::default(),
            sync: SyncCfg::default(),
            audio: AudioCfg::default(),
            timeline: Markers::default(),
            tracking: TrackingCfg::default(),
            camera: CameraCfg::default(),
            overlay: Skin::default(),
            music: Music::default(),
            export: ExportCfg::default(),
        }
    }
}

impl Project {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Project> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_python_era_json() {
        let json = r#"{"name":"x","timeline":{"in":1.5,"superStart":3.0,"out":null},
            "camera":{"super":{"keyframes":[{"t":3.5,"cx":800,"cy":540,"zoom":1.4}]}},
            "overlay":{"name":"S","referenceSize":[1920,1080],"elements":[{"id":"a","src":{"x":1,"y":2,"w":3,"h":4}}]}}"#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.timeline.t_in, 1.5);
        assert_eq!(p.timeline.super_start, Some(3.0));
        assert_eq!(p.camera.super_.keyframes[0].zoom, 1.4);
        assert_eq!(p.overlay.elements[0].src.h, 4.0);
        assert_eq!(p.export.crf, 16);
        let back: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn markers_stay_ordered() {
        let mut m = Markers { t_in: 0.0, super_start: Some(5.0), anim_start: Some(6.0), post_start: Some(7.0), out: Some(9.0) };
        m.set(MarkerKind::SuperStart, Some(8.0));
        assert_eq!((m.anim_start, m.post_start, m.out), (Some(8.0), Some(8.0), Some(9.0)));
        m.set(MarkerKind::In, Some(8.5));
        assert_eq!(m.super_start, Some(8.5));
    }
}
