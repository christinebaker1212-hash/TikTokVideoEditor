//! Top bar and the tabbed side panel.

use egui::{Color32, ComboBox, DragValue, RichText, Slider, Ui};

use combo_editor::presets::{self, Kind};
use combo_editor::project::{Blend, Element, MarkerKind, Part, Skin};

use super::style::{self, heading, hint, primary, ACCENT, ERROR, MUTED};
use super::{EditorApp, SourceMode, Tab};

const VIDEO_EXT: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "flv", "m4v"];
const AUDIO_EXT: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "mp4", "mkv", "webm"];

/// Text field + native "Browse…" dialog. Returns true when the path changed.
fn path_row(ui: &mut Ui, value: &mut String, hint_text: &str, exts: &[&str], filter: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = ui.available_width() - 80.0;
        changed |= ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).desired_width(w)).changed();
        if ui.button("Browse…").clicked() {
            let mut dlg = rfd::FileDialog::new().add_filter(filter, exts).add_filter("All files", &["*"]);
            if let Some(dir) = std::path::Path::new(value.as_str()).parent().filter(|d| d.is_dir()) {
                dlg = dlg.set_directory(dir);
            }
            if let Some(p) = dlg.pick_file() {
                *value = p.to_string_lossy().into_owned();
                changed = true;
            }
        }
    });
    changed
}

fn fmt_time(t: f64) -> String {
    let m = (t / 60.0).floor();
    format!("{}:{:06.3}", m as i64, t - m * 60.0)
}

fn info_line(info: &Option<combo_editor::media::VideoInfo>) -> String {
    match info {
        Some(i) => format!("{}×{} · {:.2} fps · {:.2} s{}", i.width, i.height, i.fps, i.duration, if i.has_audio { "" } else { " · no audio" }),
        None => String::new(),
    }
}

fn progress_row(ui: &mut Ui, p: (f32, String), on_cancel: impl FnOnce()) {
    ui.horizontal(|ui| {
        ui.add(egui::ProgressBar::new(p.0).desired_width(ui.available_width() - 80.0).text(p.1));
        if ui.button("Cancel").clicked() {
            on_cancel();
        }
    });
}

impl EditorApp {
    pub fn top_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Combo Editor").strong().color(ACCENT));
            ui.add(egui::TextEdit::singleline(&mut self.project.name).desired_width(170.0).hint_text("Project name"));
            if ui.button("New").clicked() {
                self.new_project(&ctx);
            }
            if ui.button("Open…").clicked() {
                self.open_project(&ctx);
            }
            if ui.button("Save").clicked() {
                self.save_project();
            }
            if ui.button("Save as…").clicked() {
                self.save_project_as();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.stats.show {
                    let s = &self.stats;
                    ui.label(RichText::new(format!(
                        "UI {:.0} fps · video {:.0} fps · compose {:.1} ms",
                        s.ui_fps, s.video_fps, s.compose_ms
                    )).monospace().color(style::CYAN));
                }
                if let Some((msg, at, err)) = &self.status {
                    let keep = if *err { 10.0 } else { 5.0 };
                    if at.elapsed().as_secs_f32() < keep {
                        ui.label(RichText::new(msg).color(if *err { ERROR } else { MUTED }));
                        ctx.request_repaint_after(std::time::Duration::from_millis(500));
                    }
                }
            });
        });
    }

    pub fn side_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let tabs = [
            (Tab::Sources, "1 · Sources & sync"),
            (Tab::Camera, "2 · Camera"),
            (Tab::Overlay, "3 · UI overlay"),
            (Tab::Export, "4 · Music & export"),
        ];
        for row in tabs.chunks(2) {
            ui.columns(2, |cols| {
                for ((tab, label), ui) in row.iter().zip(cols.iter_mut()) {
                    let active = self.tab == *tab;
                    let text = RichText::new(*label).color(if active { Color32::WHITE } else { style::TEXT });
                    let mut b = egui::Button::new(text).min_size(egui::vec2(ui.available_width(), 26.0));
                    if active {
                        b = b.fill(ACCENT);
                    }
                    if ui.add(b).clicked() {
                        self.tab = *tab;
                        match tab {
                            Tab::Overlay => self.source_mode = SourceMode::Ui,
                            Tab::Camera => self.source_mode = SourceMode::Clean,
                            _ => {}
                        }
                    }
                }
            });
        }
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
            Tab::Sources => self.tab_sources(ui),
            Tab::Camera => self.tab_camera(ui),
            Tab::Overlay => self.tab_overlay(ui),
            Tab::Export => self.tab_export(ui),
        });
    }

    fn tab_sources(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        heading(ui, "Source videos");
        ui.label(format!("With UI   {}", info_line(&self.ui_info)));
        let a = path_row(ui, &mut self.project.sources.ui, "Video with UI", VIDEO_EXT, "Video");
        ui.label(format!("Without UI (clean)   {}", info_line(&self.clean)));
        let b = path_row(ui, &mut self.project.sources.clean, "Video without UI", VIDEO_EXT, "Video");
        if ui.add(primary("Load videos")).clicked() || ((a || b) && !self.project.sources.clean.is_empty() && !self.project.sources.ui.is_empty()) {
            self.load_sources(&ctx);
        }

        heading(ui, "Sync");
        hint(ui, "Both recordings have identical game audio, so they are lined up on their transients. Check with the Difference view: when in sync, everything except the UI turns black.");
        ui.horizontal(|ui| {
            let busy = self.jobs.sync.is_some();
            if ui.add_enabled(!busy, primary(if busy { "Analysing…" } else { "Auto-sync (transients)" })).clicked() {
                self.start_sync(&ctx);
            }
            if let Some(c) = self.project.sync.confidence {
                let txt = format!("confidence {:.0}%{}", c * 100.0, if c < 0.25 { " — check by eye" } else { "" });
                ui.label(RichText::new(txt).color(MUTED));
            }
        });
        ui.label("UI offset (ui time = clean time + offset)");
        let fps = self.fps();
        ui.horizontal(|ui| {
            ui.add(DragValue::new(&mut self.project.sync.offset).speed(0.001).max_decimals(4).suffix(" s"));
            for (label, d) in [("−1f", -1.0 / fps), ("+1f", 1.0 / fps), ("−10ms", -0.01), ("+10ms", 0.01)] {
                if ui.button(label).clicked() {
                    self.project.sync.offset = ((self.project.sync.offset + d) * 10000.0).round() / 10000.0;
                }
            }
            if ui.button("Snap").on_hover_text("Round to a whole number of frames").clicked() {
                self.project.sync.offset = (self.project.sync.offset * fps).round() / fps;
            }
        });
        ui.label(RichText::new(format!("= {:.2} frames", self.project.sync.offset * fps)).color(MUTED).size(12.0));
        self.envelope_plot(ui);

        heading(ui, "Game audio");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.project.audio.primary, "clean".to_string(), "Clean video");
            ui.radio_value(&mut self.project.audio.primary, "ui".to_string(), "UI video");
        });
        ui.add(Slider::new(&mut self.project.audio.game_volume, 0.0..=2.0).text("volume"));
        hint(ui, "Set the clip's start and end with In / Out under the timeline (keys I and O).");
    }

    fn envelope_plot(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 100.0), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 5.0, style::PANEL2);
        let Some((eu, ec)) = &self.envelopes else {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "Run auto-sync to compare transients", egui::FontId::proportional(12.0), MUTED);
            return;
        };
        let hop = combo_editor::sync::HOP as f64 / combo_editor::sync::SR as f64;
        let first = (ec.iter().position(|v| *v > 0.3).unwrap_or(0) as f64 * hop - 0.5).max(0.0);
        let span = 6.0;
        let off = self.project.sync.offset;
        let lane = |vals: &[f32], shift: f64, y: f32, color: Color32| {
            let pts: Vec<egui::Pos2> = (0..rect.width() as usize)
                .map(|px| {
                    let t = first + px as f64 / rect.width() as f64 * span + shift;
                    let v = vals.get((t / hop).round().max(0.0) as usize).copied().unwrap_or(0.0);
                    egui::pos2(rect.left() + px as f32, y - v * (rect.height() / 2.0 - 12.0))
                })
                .collect();
            p.add(egui::Shape::line(pts, egui::Stroke::new(1.0, color)));
        };
        lane(ec, 0.0, rect.top() + rect.height() / 2.0 - 2.0, style::part_color(Part::Post));
        lane(eu, off, rect.bottom() - 4.0, ACCENT);
        p.text(rect.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "clean", egui::FontId::proportional(11.0), MUTED);
        p.text(rect.left_center() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "UI (with offset)", egui::FontId::proportional(11.0), MUTED);
    }

    fn tab_camera(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        heading(ui, "Fighter tracking");
        ui.horizontal(|ui| {
            let t = &mut self.project.tracking;
            ui.label("Motion");
            ui.add(DragValue::new(&mut t.motion_weight).speed(0.05).range(0.0..=5.0));
            ui.label("Colour");
            ui.add(DragValue::new(&mut t.color_weight).speed(0.05).range(0.0..=5.0));
            ui.label("Edge %");
            ui.add(DragValue::new(&mut t.percentile).speed(0.1).range(0.0..=25.0));
        });
        if let Some(job) = &self.jobs.track {
            let p = job.progress();
            let mut cancel = false;
            progress_row(ui, (p.0, "Analysing fighters".into()), || cancel = true);
            if cancel {
                job.cancel();
            }
        } else {
            ui.horizontal(|ui| {
                if ui.add(primary("Analyse fighters")).clicked() {
                    self.start_track(&ctx);
                }
                let txt = if self.track.is_some() { "Tracked (runs automatically when videos load)" } else { "Not analysed: using the frame centre" };
                ui.label(RichText::new(txt).color(MUTED));
            });
        }

        heading(ui, "Parts");
        if let Some(job) = &self.jobs.detect {
            let p = job.progress();
            let mut cancel = false;
            progress_row(ui, (p.0, "Looking for the Super Art".into()), || cancel = true);
            if cancel {
                job.cancel();
            }
        } else {
            ui.horizontal(|ui| {
                if ui.add(primary("Detect Super Art")).on_hover_text("Finds the super flash and the cinematic (where the HUD disappears) and sets the markers").clicked() {
                    self.start_detect(&ctx);
                }
                if self.undo_detect.is_some() && ui.button("Undo").clicked() {
                    self.undo_detection();
                }
            });
        }
        if let Some(d) = &self.detection {
            let txt = format!(
                "Found: startup {}, cinematic {} to {} · {:.0}% confidence{}",
                fmt_time(d.super_start), fmt_time(d.anim_start), fmt_time(d.post_start), d.confidence * 100.0,
                if d.startup_found { "" } else { " · startup estimated" }
            );
            ui.label(RichText::new(txt).size(12.0).color(if d.startup_found { MUTED } else { ERROR }));
        }
        egui::Grid::new("markers").num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
            for k in MarkerKind::ALL {
                let color = match k {
                    MarkerKind::SuperStart => style::part_color(Part::Startup),
                    MarkerKind::AnimStart => style::part_color(Part::Animation),
                    MarkerKind::PostStart => style::part_color(Part::Post),
                    _ => style::TEXT,
                };
                ui.label(RichText::new(k.label()).color(color));
                let v = self.project.timeline.get(k);
                match v {
                    Some(mut x) => {
                        if ui.add(DragValue::new(&mut x).speed(1.0 / self.fps()).max_decimals(3).custom_formatter(|v, _| fmt_time(v))).changed() {
                            self.set_marker(k, Some(x));
                        }
                    }
                    None => {
                        ui.label(RichText::new("unset").color(MUTED));
                    }
                }
                if ui.button("Set").on_hover_text("Set to the playhead").clicked() {
                    self.set_marker(k, Some(self.t));
                }
                if ui.add_enabled(v.is_some(), egui::Button::new("Go")).clicked() {
                    self.seek(v.unwrap_or(0.0));
                }
                if ui.button("Clear").clicked() {
                    self.set_marker(k, if k == MarkerKind::In { Some(0.0) } else { None });
                }
                ui.end_row();
            }
        });

        heading(ui, "Framing (gameplay)");
        {
            let f = &mut self.project.camera.framing;
            ui.checkbox(&mut f.keep_both, "Always keep both fighters in frame")
                .on_hover_text("Native scale normally; zooms out only when the fighters are too far apart for the 9:16 crop");
            if f.keep_both {
                ui.add(Slider::new(&mut f.margin, 0.0..=250.0).text("margin around fighters (px)"));
                ui.horizontal(|ui| {
                    ui.label("When zoomed out, fill above/below with");
                    ui.radio_value(&mut f.fill, "blur".to_string(), "blurred gameplay");
                    ui.radio_value(&mut f.fill, "black".to_string(), "black");
                });
            }
        }

        heading(ui, "Combo — auto centre");
        let c = &mut self.project.camera.combo;
        ui.add(Slider::new(&mut c.offset_x, -400.0..=400.0).text("offset X (px)"));
        ui.add(Slider::new(&mut c.smoothing, 0.0..=2.0).text("smoothing (s)"));

        heading(ui, "Super Art");
        {
            let sc = &mut self.project.camera.super_;
            ui.checkbox(&mut sc.startup_auto, "Auto startup: keep both fighters in frame");
            ui.checkbox(&mut sc.animation_auto, "Auto cinematic: centre on the animation, re-centre at every cut");
            if sc.animation_auto {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut sc.animation_hold, true, "Steady per shot")
                        .on_hover_text("One framing per shot, centred on the whole animation in it");
                    ui.radio_value(&mut sc.animation_hold, false, "Follow the action");
                });
                if !sc.animation_hold {
                    ui.add(Slider::new(&mut sc.smoothing, 0.0..=1.5).text("follow smoothing (s)"));
                }
            }
        }
        let any_auto = self.project.camera.super_.startup_auto || self.project.camera.super_.animation_auto;
        if ui.add_enabled(any_auto, egui::Button::new("Bake to keyframes")).on_hover_text("Turn the auto camera into keyframes you can adjust").clicked() {
            self.bake_keyframes(true, true);
        }
        hint(ui, "Manual parts: move the playhead into the super, then drag the preview to pan and scroll to zoom. A keyframe is created or updated at the playhead. Dragging inside an auto part bakes it first.");
        ui.horizontal(|ui| {
            ui.label("Easing");
            let e = &mut self.project.camera.super_.easing;
            let name = |s: &str| match s {
                "linear" => "Linear",
                "easeIn" => "Ease in",
                "easeOut" => "Ease out",
                "hold" => "Hold (cut)",
                _ => "Ease in/out",
            };
            ComboBox::from_id_salt("easing").selected_text(name(e)).show_ui(ui, |ui| {
                for v in ["easeInOut", "linear", "easeIn", "easeOut", "hold"] {
                    ui.selectable_value(e, v.to_string(), name(v));
                }
            });
            if ui.button("+ Keyframe at playhead").clicked() {
                self.edit_keyframe(|c| c);
            }
        });
        let fps = self.fps();
        let now = self.t;
        let mut go = None;
        let mut remove = None;
        let mut edited = false;
        if self.project.camera.super_.keyframes.is_empty() {
            hint(ui, "No keyframes: the camera holds where the combo ended.");
        } else {
            egui::Grid::new("keys").num_columns(6).spacing([6.0, 3.0]).show(ui, |ui| {
                for h in ["time", "x", "y", "zoom", "", ""] {
                    ui.label(RichText::new(h).color(MUTED).size(12.0));
                }
                ui.end_row();
                for (i, k) in self.project.camera.super_.keyframes.iter_mut().enumerate() {
                    let current = (k.t - now).abs() < 0.5 / fps;
                    ui.label(RichText::new(fmt_time(k.t)).color(if current { ACCENT } else { style::TEXT }));
                    edited |= ui.add(DragValue::new(&mut k.cx).speed(1.0)).changed();
                    edited |= ui.add(DragValue::new(&mut k.cy).speed(1.0)).changed();
                    edited |= ui.add(DragValue::new(&mut k.zoom).speed(0.01).range(1.0..=6.0)).changed();
                    if ui.button("Go").clicked() {
                        go = Some(k.t);
                    }
                    if ui.button("Delete").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        }
        if edited {
            let (w, h) = self.src_size();
            for k in &mut self.project.camera.super_.keyframes {
                let c = combo_editor::camera::clamp_cam(combo_editor::camera::Cam { cx: k.cx, cy: k.cy, zoom: k.zoom }, w, h);
                (k.cx, k.cy, k.zoom) = (c.cx, c.cy, c.zoom);
            }
        }
        if let Some(t) = go {
            self.seek(t);
        }
        if let Some(i) = remove {
            self.project.camera.super_.keyframes.remove(i);
        }

        heading(ui, "Post super — auto centre, adjustable");
        let p = &mut self.project.camera.post;
        ui.add(Slider::new(&mut p.offset_x, -400.0..=400.0).text("offset X"));
        ui.add(Slider::new(&mut p.offset_y, -400.0..=400.0).text("offset Y"));
        ui.add(Slider::new(&mut p.zoom, 1.0..=3.0).text("zoom"));
        ui.add(Slider::new(&mut p.smoothing, 0.0..=2.0).text("smoothing (s)"));

        heading(ui, "Transitions");
        let tr = &mut self.project.camera.transitions;
        ui.horizontal(|ui| {
            ui.label("Combo to super");
            ui.add(DragValue::new(&mut tr.combo_to_super).speed(0.01).range(0.0..=3.0).suffix(" s"));
            ui.label("Super to post");
            ui.add(DragValue::new(&mut tr.super_to_post).speed(0.01).range(0.0..=3.0).suffix(" s"));
        });
    }

    fn tab_overlay(&mut self, ui: &mut Ui) {
        heading(ui, "UI skin preset");
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("skin").selected_text(self.skin_pick.clone()).show_ui(ui, |ui| {
                for n in self.skin_list.clone() {
                    ui.selectable_value(&mut self.skin_pick, n.clone(), n);
                }
            });
            if ui.button("Load").clicked() {
                match presets::load::<Skin>(Kind::Skins, &self.skin_pick) {
                    Ok(s) => {
                        self.selected = s.elements.first().map(|e| e.id.clone());
                        self.project.overlay = s;
                        self.set_status(format!("Loaded skin “{}”", self.skin_pick), false);
                    }
                    Err(e) => self.set_status(format!("{e:#}"), true),
                }
            }
            if ui.button("Save").on_hover_text("Overwrite the selected preset").clicked() {
                let name = self.skin_pick.clone();
                self.save_skin(&name);
            }
            if ui.button("Delete").clicked() {
                let _ = presets::delete(Kind::Skins, &self.skin_pick);
                self.refresh_presets();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("New preset name").desired_width(170.0));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new("Save skin as")).clicked() {
                let name = self.new_name.trim().to_string();
                self.save_skin(&name);
                self.skin_pick = name;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Show/hide fade");
            ui.add(DragValue::new(&mut self.project.overlay.fade).speed(0.01).range(0.0..=2.0).suffix(" s"));
        });

        heading(ui, "Elements");
        let mut select = None;
        for el in &self.project.overlay.elements {
            let active = Some(&el.id) == self.selected.as_ref();
            let label = format!("{}{}", el.label, if el.enabled { "" } else { "  (off)" });
            if ui.add(egui::Button::selectable(active, label).min_size(egui::vec2(ui.available_width(), 24.0))).clicked() {
                select = Some(el.id.clone());
            }
        }
        if select.is_some() {
            self.selected = select;
        }
        ui.horizontal(|ui| {
            if ui.button("+ Add").clicked() {
                let id = self.new_element_id();
                self.project.overlay.elements.push(Element { id: id.clone(), ..Default::default() });
                self.selected = Some(id);
            }
            if ui.button("Duplicate").clicked() {
                if let Some(el) = self.selected_el().cloned() {
                    let id = self.new_element_id();
                    let mut c = el;
                    c.id = id.clone();
                    c.label += " copy";
                    c.dst.y += 40.0;
                    self.project.overlay.elements.push(c);
                    self.selected = Some(id);
                }
            }
            if ui.button("Delete").clicked() {
                if let Some(id) = &self.selected {
                    self.project.overlay.elements.retain(|e| &e.id != id);
                    self.selected = self.project.overlay.elements.first().map(|e| e.id.clone());
                }
            }
        });

        let Some(idx) = self.selected.as_ref().and_then(|id| self.project.overlay.elements.iter().position(|e| &e.id == id)) else {
            return;
        };
        ui.separator();
        hint(ui, "Drag the boxes: source on the UI view, placement on the TikTok preview. Drag a corner to resize (hold Alt to unlock the aspect ratio).");
        let mut apply_blend = None;
        {
            let el = &mut self.project.overlay.elements[idx];
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut el.label).desired_width(200.0));
                ui.checkbox(&mut el.enabled, "Enabled");
            });
            egui::Grid::new("rects").num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
                for h in ["", "x", "y", "w", "h"] {
                    ui.label(RichText::new(h).color(MUTED));
                }
                ui.end_row();
                for (name, r) in [("Source", &mut el.src), ("Placement", &mut el.dst)] {
                    ui.label(name);
                    ui.add(DragValue::new(&mut r.x).speed(1.0).max_decimals(0));
                    ui.add(DragValue::new(&mut r.y).speed(1.0).max_decimals(0));
                    ui.add(DragValue::new(&mut r.w).speed(1.0).max_decimals(0).range(1.0..=4000.0));
                    ui.add(DragValue::new(&mut r.h).speed(1.0).max_decimals(0).range(1.0..=4000.0));
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Match source aspect").clicked() {
                    el.dst.h = (el.dst.w * el.src.h / el.src.w).round();
                }
                if ui.button("Centre horizontally").clicked() {
                    el.dst.x = ((1080.0 - el.dst.w) / 2.0).round();
                }
            });

            ui.label(RichText::new("Blend").strong());
            ui.add(Slider::new(&mut el.opacity, 0.0..=1.0).text("opacity"));
            ui.add(Slider::new(&mut el.feather.left, 0.0..=150.0).text("feather left"));
            ui.add(Slider::new(&mut el.feather.right, 0.0..=150.0).text("feather right"));
            ui.add(Slider::new(&mut el.feather.top, 0.0..=150.0).text("feather top"));
            ui.add(Slider::new(&mut el.feather.bottom, 0.0..=150.0).text("feather bottom"));
        }
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("blend").selected_text(self.blend_pick.clone()).show_ui(ui, |ui| {
                for n in self.blend_list.clone() {
                    ui.selectable_value(&mut self.blend_pick, n.clone(), n);
                }
            });
            if ui.button("Apply").clicked() {
                apply_blend = Some(self.blend_pick.clone());
            }
            if ui.button("Delete").clicked() {
                let _ = presets::delete(Kind::Blends, &self.blend_pick);
                self.refresh_presets();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("New preset name").desired_width(170.0));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new("Save blend as")).clicked() {
                let el = &self.project.overlay.elements[idx];
                let name = self.new_name.trim().to_string();
                let b = Blend { name: name.clone(), opacity: el.opacity, feather: el.feather };
                match presets::save(Kind::Blends, &name, &b) {
                    Ok(()) => {
                        self.blend_pick = name.clone();
                        self.refresh_presets();
                        self.set_status(format!("Saved blend “{name}”"), false);
                    }
                    Err(e) => self.set_status(format!("{e:#}"), true),
                }
            }
        });
        if let Some(name) = apply_blend {
            match presets::load::<Blend>(Kind::Blends, &name) {
                Ok(b) => {
                    let el = &mut self.project.overlay.elements[idx];
                    el.opacity = b.opacity;
                    el.feather = b.feather;
                    self.set_status(format!("Applied blend “{name}”"), false);
                }
                Err(e) => self.set_status(format!("{e:#}"), true),
            }
        }

        ui.label(RichText::new("Visible during").strong());
        let el = &mut self.project.overlay.elements[idx];
        ui.horizontal_wrapped(|ui| {
            for p in Part::ALL {
                ui.checkbox(el.parts.get_mut(p), RichText::new(p.label()).color(style::part_color(p)));
            }
        });
    }

    fn save_skin(&mut self, name: &str) {
        let mut skin = self.project.overlay.clone();
        skin.name = name.to_string();
        match presets::save(Kind::Skins, name, &skin) {
            Ok(()) => {
                self.project.overlay.name = name.to_string();
                self.refresh_presets();
                self.set_status(format!("Saved skin “{name}”"), false);
            }
            Err(e) => self.set_status(format!("{e:#}"), true),
        }
    }

    pub fn selected_el(&self) -> Option<&Element> {
        let id = self.selected.as_ref()?;
        self.project.overlay.elements.iter().find(|e| &e.id == id)
    }

    fn new_element_id(&self) -> String {
        (1..).map(|i| format!("el{i}")).find(|id| !self.project.overlay.elements.iter().any(|e| &e.id == id)).unwrap()
    }

    fn tab_export(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        heading(ui, "Background music");
        path_row(ui, &mut self.project.music.path, "Song file", AUDIO_EXT, "Audio");
        let m = &mut self.project.music;
        ui.horizontal(|ui| {
            ui.label("Song starts at");
            ui.add(DragValue::new(&mut m.start).speed(0.1).range(0.0..=3600.0).suffix(" s"));
        });
        ui.add(Slider::new(&mut m.volume, 0.0..=1.5).text("music volume"));
        ui.horizontal(|ui| {
            ui.label("Fade in");
            ui.add(DragValue::new(&mut m.fade_in).speed(0.05).range(0.0..=20.0).suffix(" s"));
            ui.label("Fade out");
            ui.add(DragValue::new(&mut m.fade_out).speed(0.05).range(0.0..=20.0).suffix(" s"));
        });
        ui.horizontal(|ui| {
            ui.label("Game audio fade out");
            ui.add(DragValue::new(&mut self.project.audio.game_fade_out).speed(0.05).range(0.0..=20.0).suffix(" s"));
        });
        ui.checkbox(&mut self.music_preview, "Play music in preview");

        heading(ui, "Export");
        ui.label("Output file");
        ui.horizontal(|ui| {
            let default = combo_editor::render::default_output(&self.project);
            let w = ui.available_width() - 80.0;
            ui.add(egui::TextEdit::singleline(&mut self.project.export.output).hint_text(default.to_string_lossy()).desired_width(w));
            if ui.button("Browse…").clicked() {
                let mut dlg = rfd::FileDialog::new().add_filter("MP4 video", &["mp4"]).set_file_name(format!("{}.mp4", self.project.name));
                if let Some(dir) = default.parent() {
                    let _ = std::fs::create_dir_all(dir);
                    dlg = dlg.set_directory(dir);
                }
                if let Some(p) = dlg.save_file() {
                    let mut s = p.to_string_lossy().into_owned();
                    if !s.to_lowercase().ends_with(".mp4") {
                        s += ".mp4";
                    }
                    self.project.export.output = s;
                }
            }
        });
        let ex = &mut self.project.export;
        egui::Grid::new("exportopts").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label("Upscaler");
            ComboBox::from_id_salt("scaler")
                .selected_text(match ex.scaler.as_str() {
                    "cubic" => "Bicubic (faster)",
                    _ => "Lanczos (best)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut ex.scaler, "lanczos".to_string(), "Lanczos (best)");
                    ui.selectable_value(&mut ex.scaler, "cubic".to_string(), "Bicubic (faster)");
                });
            ui.end_row();
            ui.label("Quality (CRF)");
            ui.add(DragValue::new(&mut ex.crf).range(0..=35)).on_hover_text("Lower is better quality and a bigger file. 16 is visually lossless.");
            ui.end_row();
            ui.label("Encoder speed");
            ComboBox::from_id_salt("preset").selected_text(ex.preset.clone()).show_ui(ui, |ui| {
                for p in ["veryslow", "slower", "slow", "medium", "fast", "veryfast"] {
                    ui.selectable_value(&mut ex.preset, p.to_string(), p);
                }
            });
            ui.end_row();
            ui.label("Audio bitrate");
            ComboBox::from_id_salt("abr").selected_text(ex.audio_bitrate.clone()).show_ui(ui, |ui| {
                for b in ["320k", "256k", "192k", "128k"] {
                    ui.selectable_value(&mut ex.audio_bitrate, b.to_string(), b);
                }
            });
            ui.end_row();
        });
        ui.add_space(6.0);
        if let Some(job) = &self.jobs.export {
            let p = job.progress();
            let mut cancel = false;
            progress_row(ui, p, || cancel = true);
            if cancel {
                job.cancel();
            }
        } else {
            ui.horizontal(|ui| {
                if ui.add(primary("Export 1080×1920")).clicked() {
                    self.start_export(&ctx);
                }
                let busy = self.jobs.still.is_some();
                if ui.add_enabled(!busy, egui::Button::new("Exact frame")).on_hover_text("Render the current frame exactly as the export will").clicked() {
                    self.start_still(&ctx);
                }
            });
        }
        if let Some(path) = self.last_export.clone() {
            ui.label(RichText::new(format!("Done: {}", path.display())).color(style::part_color(Part::Post)));
            ui.horizontal(|ui| {
                if ui.button("Show in folder").clicked() {
                    reveal(&path);
                }
                if ui.button("Play").clicked() {
                    open(&path);
                }
            });
        }
    }

    pub fn still_window(&mut self, ctx: &egui::Context) {
        let Some(tex) = self.still_tex.clone() else { return };
        let mut open = true;
        egui::Window::new("Exact export frame").open(&mut open).default_height(720.0).resizable(true).show(ctx, |ui| {
            let avail = ui.available_height().max(300.0);
            let size = egui::vec2(avail * 9.0 / 16.0, avail);
            ui.add(egui::Image::new((tex.id(), size)));
        });
        if !open {
            self.still_tex = None;
        }
    }
}

/// Show a file in Explorer / the file manager.
pub fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg("/select,").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// Open a file with its default application.
pub fn open(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
