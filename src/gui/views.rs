//! Preview (9:16), source monitor (16:9) and the timeline, with their mouse interactions.

use std::sync::Arc;

use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui};

use combo_editor::camera::{clamp_cam, crop_size, Cam, FrameCam};
use combo_editor::compositor::compose;
use combo_editor::imaging::{yuv_thumbnail, Filter};
use combo_editor::media::YuvFrame;
use combo_editor::project::{Keyframe, MarkerKind, Part, Rect as PRect, OUT_H, OUT_W, REF_W};

use super::style::{self, part_color, ACCENT, CYAN, MUTED};
use super::{EditorApp, SourceMode, Tab, TexKey};

pub enum Drag {
    ElDst { idx: usize, handle: bool, start: PRect, from: Pos2 },
    ElSrc { idx: usize, handle: bool, start: PRect, from: Pos2 },
    PanPreview { last: Pos2 },
    PanSource { last: Pos2 },
    Scrub,
    Marker(MarkerKind),
    Key(usize),
}

fn fmt_time(t: f64) -> String {
    let m = (t / 60.0).floor();
    format!("{}:{:06.3}", m as i64, t - m * 60.0)
}

/// Rect of `r` (in a space where `k` maps units to screen points) offset by `origin`.
fn screen_rect(origin: Pos2, r: &PRect, k: f32) -> Rect {
    Rect::from_min_size(origin + vec2(r.x as f32 * k, r.y as f32 * k), vec2(r.w as f32 * k, r.h as f32 * k))
}

fn hit(rect: Rect, p: Pos2, handle: f32) -> Option<bool> {
    let h = Rect::from_min_max(rect.max - vec2(handle, handle), rect.max + vec2(4.0, 4.0));
    if h.contains(p) {
        Some(true)
    } else if rect.contains(p) {
        Some(false)
    } else {
        None
    }
}

fn drag_rect(r: &mut PRect, start: &PRect, delta: egui::Vec2, k: f32, handle: bool, keep_aspect: bool) {
    let (dx, dy) = ((delta.x / k) as f64, (delta.y / k) as f64);
    if handle {
        r.w = (start.w + dx).round().max(4.0);
        r.h = if keep_aspect { (r.w * start.h / start.w).round().max(2.0) } else { (start.h + dy).round().max(2.0) };
    } else {
        r.x = (start.x + dx).round();
        r.y = (start.y + dy).round();
    }
}

impl EditorApp {
    /// Current frame pair (clean, ui) and their indices; falls back to the
    /// nearest decoded frames while seeking.
    fn frames(&mut self) -> Option<((i64, Arc<YuvFrame>), Option<(i64, Arc<YuvFrame>)>)> {
        let dc = self.dec_clean.as_ref()?;
        let i = self.frame_of(self.t);
        let j = i + (self.project.sync.offset * self.fps()).round() as i64;
        let clean = dc.get(i).map(|f| (i, f)).or_else(|| dc.nearest(i))?;
        let ui = self.dec_ui.as_ref().and_then(|d| d.get(j).map(|f| (j, f)).or_else(|| d.nearest(j)));
        if clean.0 != i {
            // Still decoding: come back soon.
            self.shown = (i64::MIN, i64::MIN);
        }
        Some((clean, ui))
    }

    fn frame_cam(&self) -> FrameCam {
        match &self.timeline {
            Some(tl) => tl.sample(self.t),
            None => {
                let (w, h) = self.src_size();
                FrameCam { cam: Cam { cx: w / 2.0, cy: h / 2.0, zoom: 1.0 }, part: Part::Combo, alphas: Default::default() }
            }
        }
    }

    pub fn monitors(&mut self, ui: &mut Ui) {
        let full = ui.available_rect_before_wrap().shrink(14.0);
        let bar = 30.0;
        let ph = (full.height() - bar).max(100.0);
        let pw = ph * 9.0 / 16.0;
        let preview_rect = Rect::from_min_size(full.min, vec2(pw, ph));
        let sx = preview_rect.right() + 14.0;
        let sw = (full.right() - sx).clamp(100.0, 1100.0);
        let sh = (sw * 9.0 / 16.0).min(ph);
        let sw = sh * 16.0 / 9.0;
        let source_rect = Rect::from_min_size(pos2(sx, full.top()), vec2(sw, sh));

        let frames = self.frames();
        self.preview(ui, preview_rect, frames.as_ref());
        self.source_monitor(ui, source_rect, frames.as_ref());

        // Bars under the monitors.
        let fc = self.frame_cam();
        let part = self.part_at(self.t);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(preview_rect.left(), preview_rect.bottom() + 6.0), vec2(pw, 24.0))));
        child.horizontal(|ui| {
            egui::Frame::new().fill(part_color(part)).corner_radius(10.0).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.label(RichText::new(part.label()).color(Color32::WHITE).strong());
            });
            ui.checkbox(&mut self.safe_zone, "TikTok safe zone");
        });
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(source_rect.left(), source_rect.bottom() + 6.0), vec2(sw, 60.0))));
        child.horizontal_wrapped(|ui| {
            for (mode, label) in [(SourceMode::Clean, "Clean + crop"), (SourceMode::Ui, "UI"), (SourceMode::Diff, "Difference")] {
                let active = self.source_mode == mode;
                let mut b = egui::Button::new(RichText::new(label).color(if active { Color32::WHITE } else { style::TEXT }));
                if active {
                    b = b.fill(style::BLUE);
                }
                if ui.add(b).clicked() {
                    self.source_mode = mode;
                }
            }
            ui.label(RichText::new(format!("cam x {:.0} y {:.0} · zoom {:.2}×", fc.cam.cx, fc.cam.cy, fc.cam.zoom)).color(MUTED));
            let ctx = ui.ctx().clone();
            if ui.checkbox(&mut self.fast_preview, "Fast preview").on_hover_text("Decode at half resolution (for slower PCs)").changed() {
                self.reload_decoders(&ctx);
            }
            if ui.checkbox(&mut self.hw_decode, "GPU decoding").changed() {
                self.reload_decoders(&ctx);
            }
        });
    }

    fn preview(&mut self, ui: &mut Ui, rect: Rect, frames: Option<&((i64, Arc<YuvFrame>), Option<(i64, Arc<YuvFrame>)>)>) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::BLACK);
        let ppp = ctx.pixels_per_point();
        let w = ((rect.width() * ppp) as usize).clamp(90, OUT_W as usize);
        let h = w * 16 / 9;
        if let Some(((ci, clean), ui_f)) = frames {
            let key = TexKey { clean: *ci, ui: ui_f.as_ref().map_or(-1, |u| u.0), version: self.version, w, h, mode: 0 };
            if self.preview_tex.as_ref().is_none_or(|(_, k)| *k != key) {
                let fc = self.frame_cam();
                let started = std::time::Instant::now();
                let blur = self.project.camera.framing.fill != "black";
                let img = compose(clean, ui_f.as_ref().map(|u| &*u.1), &fc, &self.project.overlay, self.src_size().1, w, h, Filter::Linear, blur);
                let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], &img.to_rgba8());
                self.stats.compose_ms = started.elapsed().as_secs_f32() * 1000.0;
                self.stats.composed += 1;
                match &mut self.preview_tex {
                    Some((tex, k)) => {
                        tex.set(ci, egui::TextureOptions::LINEAR);
                        *k = key;
                    }
                    None => self.preview_tex = Some((ctx.load_texture("preview", ci, egui::TextureOptions::LINEAR), key)),
                }
            }
            if let Some((tex, _)) = &self.preview_tex {
                painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        } else {
            let msg = if self.clean.is_some() { "Decoding…" } else { "Load videos to begin" };
            painter.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(18.0), MUTED);
        }

        let k = rect.width() / OUT_W as f32;
        let part = self.part_at(self.t);
        if self.tab == Tab::Overlay {
            for el in &self.project.overlay.elements {
                let sel = Some(&el.id) == self.selected.as_ref();
                let r = screen_rect(rect.min, &el.dst, k);
                let stroke = if sel { Stroke::new(2.0, ACCENT) } else { Stroke::new(1.0, Color32::from_white_alpha(150)) };
                painter.rect_stroke(r, 0.0, stroke, StrokeKind::Middle);
                if sel {
                    painter.rect_filled(Rect::from_min_max(r.max - vec2(9.0, 9.0), r.max), 0.0, ACCENT);
                }
            }
        }
        if self.safe_zone {
            let z = Color32::from_rgba_unmultiplied(255, 40, 80, 46);
            let sz = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(rect.min + vec2(x * k, y * k), vec2(w * k, h * k));
            painter.rect_filled(sz(0.0, 0.0, 1080.0, 130.0), 0.0, z);
            painter.rect_filled(sz(0.0, 1500.0, 1080.0, 420.0), 0.0, z);
            painter.rect_filled(sz(930.0, 130.0, 150.0, 1370.0), 0.0, z);
        }
        if self.tab == Tab::Camera && part.is_super() {
            painter.rect_stroke(rect.shrink(2.0), 6.0, Stroke::new(4.0, part_color(part)), StrokeKind::Inside);
        }

        // Interaction.
        let resp = ui.interact(rect, ui.id().with("preview"), Sense::click_and_drag());
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.drag = None;
                if self.tab == Tab::Overlay {
                    if let Some((idx, handle)) = self.pick_element(p, |el| screen_rect(rect.min, &el.dst, k), 14.0) {
                        self.selected = Some(self.project.overlay.elements[idx].id.clone());
                        self.drag = Some(Drag::ElDst { idx, handle, start: self.project.overlay.elements[idx].dst, from: p });
                    }
                } else if self.tab == Tab::Camera && part.is_super() {
                    self.pause();
                    self.drag = Some(Drag::PanPreview { last: p });
                }
            }
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let alt = ui.input(|i| i.modifiers.alt);
                match &mut self.drag {
                    Some(Drag::ElDst { idx, handle, start, from }) => {
                        let (idx, handle, start, delta) = (*idx, *handle, *start, p - *from);
                        if let Some(el) = self.project.overlay.elements.get_mut(idx) {
                            drag_rect(&mut el.dst, &start, delta, k, handle, !alt);
                        }
                    }
                    Some(Drag::PanPreview { last }) => {
                        let d = p - *last;
                        *last = p;
                        let (_, src_h) = self.src_size();
                        self.edit_keyframe(|c| {
                            // Source pixels per screen point at this zoom.
                            let s = (src_h / c.zoom) / rect.height() as f64;
                            Cam { cx: c.cx - d.x as f64 * s, cy: c.cy - d.y as f64 * s, ..c }
                        });
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.hovered() && self.tab == Tab::Camera && part.is_super() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.pause();
                self.edit_keyframe(|c| Cam { zoom: c.zoom * 1.0006f64.powf(scroll as f64), ..c });
            }
        }
    }

    fn pick_element(&self, p: Pos2, rect_of: impl Fn(&combo_editor::project::Element) -> Rect, handle: f32) -> Option<(usize, bool)> {
        let els = &self.project.overlay.elements;
        let sel = self.selected.as_ref().and_then(|id| els.iter().position(|e| &e.id == id));
        let order = sel.into_iter().chain((0..els.len()).filter(|i| Some(*i) != sel));
        for i in order {
            if let Some(h) = hit(rect_of(&els[i]), p, handle) {
                return Some((i, h));
            }
        }
        None
    }

    fn source_monitor(&mut self, ui: &mut Ui, rect: Rect, frames: Option<&((i64, Arc<YuvFrame>), Option<(i64, Arc<YuvFrame>)>)>) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::BLACK);
        let ppp = ctx.pixels_per_point();
        let w = ((rect.width() * ppp) as usize).clamp(64, 1280) & !1;
        let h = (w * 9 / 16) & !1;
        let mode = self.source_mode;
        if let Some(((ci, clean), ui_f)) = frames {
            let key = TexKey { clean: *ci, ui: ui_f.as_ref().map_or(-1, |u| u.0), version: 0, w, h, mode: mode as u8 + 1 };
            if self.source_tex.as_ref().is_none_or(|(_, k)| *k != key) {
                let rgba = match (mode, ui_f) {
                    (SourceMode::Clean, _) | (_, None) => yuv_thumbnail(clean, w, h),
                    (SourceMode::Ui, Some((_, u))) => yuv_thumbnail(u, w, h),
                    (SourceMode::Diff, Some((_, u))) => {
                        let a = yuv_thumbnail(clean, w, h);
                        let b = yuv_thumbnail(u, w, h);
                        a.chunks(4).zip(b.chunks(4)).flat_map(|(a, b)| [a[0].abs_diff(b[0]), a[1].abs_diff(b[1]), a[2].abs_diff(b[2]), 255]).collect()
                    }
                };
                let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                match &mut self.source_tex {
                    Some((tex, k)) => {
                        tex.set(ci, egui::TextureOptions::LINEAR);
                        *k = key;
                    }
                    None => self.source_tex = Some((ctx.load_texture("source", ci, egui::TextureOptions::LINEAR), key)),
                }
            }
            if let Some((tex, _)) = &self.source_tex {
                painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }

        let (src_w, src_h) = self.src_size();
        let ks = rect.width() / src_w as f32;
        let fc = self.frame_cam();
        let part = self.part_at(self.t);
        let show_src_rects = mode != SourceMode::Clean || self.tab == Tab::Overlay;
        let kr = rect.width() / self.project.overlay.reference_size[0].max(1.0) as f32;
        if mode == SourceMode::Clean {
            if let Some(tr) = &self.track {
                let i = ((self.t * tr.fps).round().max(0.0) as usize).min(tr.frames.saturating_sub(1));
                let kt = rect.width() / tr.width as f32;
                for x in [tr.left.get(i), tr.right.get(i)].into_iter().flatten() {
                    let x = rect.left() + *x as f32 * kt;
                    painter.add(egui::Shape::dashed_line(&[pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, CYAN), 4.0, 4.0));
                }
            }
            let (cw, ch) = crop_size(fc.cam.zoom, src_h);
            let r = Rect::from_min_size(
                rect.min + vec2(((fc.cam.cx - cw / 2.0) as f32) * ks, ((fc.cam.cy - ch / 2.0) as f32) * ks),
                vec2(cw as f32 * ks, ch as f32 * ks),
            );
            let dim = Color32::from_black_alpha(115);
            painter.rect_filled(Rect::from_min_max(rect.min, pos2(r.left(), rect.bottom())), 0.0, dim);
            painter.rect_filled(Rect::from_min_max(pos2(r.right(), rect.top()), rect.max), 0.0, dim);
            painter.rect_filled(Rect::from_min_max(pos2(r.left(), rect.top()), pos2(r.right(), r.top())), 0.0, dim);
            painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom()), pos2(r.right(), rect.bottom())), 0.0, dim);
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, part_color(part)), StrokeKind::Middle);
            painter.vline(r.center().x, r.y_range(), Stroke::new(1.0, part_color(part).gamma_multiply(0.4)));
        }
        if show_src_rects {
            for el in &self.project.overlay.elements {
                let sel = Some(&el.id) == self.selected.as_ref();
                let r = screen_rect(rect.min, &el.src, kr);
                let stroke = if sel { Stroke::new(2.0, ACCENT) } else { Stroke::new(1.0, Color32::from_white_alpha(170)) };
                painter.rect_stroke(r, 0.0, stroke, StrokeKind::Middle);
                if sel {
                    painter.rect_filled(Rect::from_min_max(r.max - vec2(7.0, 7.0), r.max), 0.0, ACCENT);
                }
            }
        }

        let resp = ui.interact(rect, ui.id().with("source"), Sense::click_and_drag());
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.drag = None;
                if show_src_rects {
                    if let Some((idx, handle)) = self.pick_element(p, |el| screen_rect(rect.min, &el.src, kr), 10.0) {
                        self.selected = Some(self.project.overlay.elements[idx].id.clone());
                        self.drag = Some(Drag::ElSrc { idx, handle, start: self.project.overlay.elements[idx].src, from: p });
                    }
                } else if self.tab == Tab::Camera && part.is_super() {
                    self.pause();
                    self.drag = Some(Drag::PanSource { last: p });
                }
            }
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let alt = ui.input(|i| i.modifiers.alt);
                match &mut self.drag {
                    Some(Drag::ElSrc { idx, handle, start, from }) => {
                        let (idx, handle, start, delta) = (*idx, *handle, *start, p - *from);
                        if let Some(el) = self.project.overlay.elements.get_mut(idx) {
                            // Source boxes resize freely; hold Alt to keep their aspect.
                            drag_rect(&mut el.src, &start, delta, kr, handle, alt);
                        }
                    }
                    Some(Drag::PanSource { last }) => {
                        let d = p - *last;
                        *last = p;
                        self.edit_keyframe(|c| Cam { cx: c.cx + (d.x / ks) as f64, cy: c.cy + (d.y / ks) as f64, ..c });
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.hovered() && self.tab == Tab::Camera && part.is_super() && mode == SourceMode::Clean {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.pause();
                self.edit_keyframe(|c| Cam { zoom: c.zoom * 1.0006f64.powf(scroll as f64), ..c });
            }
        }
        let _ = REF_W;
    }

    fn pause(&mut self) {
        if self.playing {
            self.toggle_play();
        }
    }

    /// Create or update the keyframe at the playhead (only inside the super).
    pub fn edit_keyframe(&mut self, f: impl FnOnce(Cam) -> Cam) {
        let t = self.snap(self.t);
        let part = self.part_at(t);
        if !part.is_super() {
            self.set_status("Move the playhead into the Super Art section (set its markers first)", true);
            return;
        }
        // Editing an auto-centred part turns its path into keyframes first.
        let sc = &self.project.camera.super_;
        if (part == Part::Startup && sc.startup_auto) || (part == Part::Animation && sc.animation_auto) {
            self.bake_keyframes(part == Part::Startup, part == Part::Animation);
            self.detect_changes();
        }
        let fps = self.fps();
        let current = self.frame_cam().cam;
        let (w, h) = self.src_size();
        let keys = &mut self.project.camera.super_.keyframes;
        let idx = match keys.iter().position(|k| (k.t - t).abs() < 0.5 / fps) {
            Some(i) => i,
            None => {
                keys.push(Keyframe { t: (t * 10000.0).round() / 10000.0, cx: current.cx, cy: current.cy, zoom: current.zoom, ease: None });
                keys.sort_by(|a, b| a.t.total_cmp(&b.t));
                keys.iter().position(|k| (k.t - t).abs() < 0.5 / fps).unwrap()
            }
        };
        let k = &keys[idx];
        let c = clamp_cam(f(Cam { cx: k.cx, cy: k.cy, zoom: k.zoom }), w, h);
        let k = &mut self.project.camera.super_.keyframes[idx];
        k.cx = (c.cx * 10.0).round() / 10.0;
        k.cy = (c.cy * 10.0).round() / 10.0;
        k.zoom = (c.zoom * 1000.0).round() / 1000.0;
    }

    // ------------------------------------------------------------ timeline

    fn tl_range(&self) -> (f64, f64) {
        if self.zoom_to_clip && self.clean.is_some() {
            let m = self.markers();
            let pad = ((m.out - m.t_in) * 0.05).max(0.5);
            ((m.t_in - pad).max(0.0), (m.out + pad).min(self.duration()))
        } else {
            (0.0, self.duration().max(1.0))
        }
    }

    pub fn timeline_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).on_hover_text("Play / pause (Space)").clicked() {
                self.toggle_play();
            }
            if ui.button("⏮").on_hover_text("Previous frame (Left arrow, Shift = 1 s)").clicked() {
                self.step(-1.0);
            }
            if ui.button("⏭").on_hover_text("Next frame (Right arrow, Shift = 1 s)").clicked() {
                self.step(1.0);
            }
            ui.label(RichText::new(format!("{}  f{}", fmt_time(self.t), self.frame_of(self.t))).monospace());
            ui.checkbox(&mut self.loop_play, "Loop clip");
            ui.checkbox(&mut self.zoom_to_clip, "Zoom to clip");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for k in MarkerKind::ALL.iter().rev() {
                    let color = match k {
                        MarkerKind::SuperStart => part_color(Part::Startup),
                        MarkerKind::AnimStart => part_color(Part::Animation),
                        MarkerKind::PostStart => part_color(Part::Post),
                        _ => style::LINE,
                    };
                    let tip = match k {
                        MarkerKind::In => "Set the clip start here (I)",
                        MarkerKind::Out => "Set the clip end here (O)",
                        _ => "Set this marker at the playhead",
                    };
                    if ui.add(egui::Button::new(k.label()).stroke(Stroke::new(1.0, color))).on_hover_text(tip).clicked() {
                        self.set_marker(*k, Some(self.t));
                    }
                }
            });
        });
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 86.0), Sense::click_and_drag());
        self.paint_timeline(ui, rect);
        self.timeline_input(rect, &resp);
    }

    fn paint_timeline(&self, ui: &Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 5.0, style::PANEL2);
        let (a, b) = self.tl_range();
        let x_of = |t: f64| rect.left() + ((t - a) / (b - a)) as f32 * rect.width();
        if let Some(peaks) = &self.waveform {
            let rate = 50.0;
            let mid = rect.center().y + 6.0;
            let half = (rect.height() - 30.0) / 2.0;
            for px in 0..rect.width() as usize {
                let t0 = a + px as f64 / rect.width() as f64 * (b - a);
                let t1 = a + (px + 1) as f64 / rect.width() as f64 * (b - a);
                let (i0, i1) = ((t0 * rate) as usize, ((t1 * rate) as usize).max((t0 * rate) as usize + 1));
                let v = peaks.get(i0..i1.min(peaks.len())).map_or(0.0, |s| s.iter().fold(0f32, |m, v| m.max(*v)));
                let hh = v.min(1.0) * half;
                let x = rect.left() + px as f32;
                p.line_segment([pos2(x, mid - hh), pos2(x, mid + hh)], Stroke::new(1.0, Color32::from_white_alpha(34)));
            }
        }
        if self.clean.is_none() {
            return;
        }
        let m = self.markers();
        for (part, s, e) in [(Part::Combo, m.t_in, m.super_start), (Part::Startup, m.super_start, m.anim_start), (Part::Animation, m.anim_start, m.post_start), (Part::Post, m.post_start, m.out)] {
            if e <= s {
                continue;
            }
            let r = Rect::from_min_max(pos2(x_of(s), rect.top() + 16.0), pos2(x_of(e), rect.bottom()));
            p.rect_filled(r, 0.0, part_color(part).gamma_multiply(0.22));
            p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 4.0)), 0.0, part_color(part));
        }
        let shade = Color32::from_black_alpha(115);
        p.rect_filled(Rect::from_min_max(pos2(rect.left(), rect.top() + 16.0), pos2(x_of(m.t_in), rect.bottom())), 0.0, shade);
        p.rect_filled(Rect::from_min_max(pos2(x_of(m.out), rect.top() + 16.0), rect.max), 0.0, shade);

        // Ruler.
        let span = b - a;
        let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0].into_iter().find(|s| span / s < rect.width() as f64 / 70.0).unwrap_or(120.0);
        let mut t = (a / step).ceil() * step;
        while t <= b {
            let x = x_of(t);
            p.vline(x, rect.top()..=rect.top() + 5.0, Stroke::new(1.0, MUTED));
            p.text(pos2(x + 3.0, rect.top() + 1.0), Align2::LEFT_TOP, format!("{t:.2}").trim_end_matches('0').trim_end_matches('.'), FontId::proportional(10.0), MUTED);
            t += step;
        }
        // Markers.
        for k in MarkerKind::ALL {
            if self.project.timeline.get(k).is_none() {
                continue;
            }
            let t = match k {
                MarkerKind::In => m.t_in,
                MarkerKind::SuperStart => m.super_start,
                MarkerKind::AnimStart => m.anim_start,
                MarkerKind::PostStart => m.post_start,
                MarkerKind::Out => m.out,
            };
            let color = match k {
                MarkerKind::SuperStart => part_color(Part::Startup),
                MarkerKind::AnimStart => part_color(Part::Animation),
                MarkerKind::PostStart => part_color(Part::Post),
                _ => Color32::WHITE,
            };
            let x = x_of(t);
            p.vline(x, rect.top() + 16.0..=rect.bottom(), Stroke::new(2.0, color));
            p.text(pos2(x + 4.0, rect.bottom() - 3.0), Align2::LEFT_BOTTOM, k.label(), FontId::proportional(10.0), color);
        }
        // HUD presence (from the Super Art detection) and shot cuts.
        if let Some(d) = &self.detection {
            let n = d.hud.len();
            let mut i = 0;
            while i < n {
                let on = d.hud[i];
                let s = i;
                while i < n && d.hud[i] == on {
                    i += 1;
                }
                if on {
                    let (x0, x1) = (x_of(s as f64 / d.fps), x_of(i as f64 / d.fps));
                    p.rect_filled(Rect::from_min_max(pos2(x0, rect.top() + 21.0), pos2(x1, rect.top() + 25.0)), 0.0, Color32::from_white_alpha(210));
                }
            }
        }
        for c in self.cut_times() {
            if c >= a && c <= b {
                let x = x_of(c);
                p.vline(x, rect.top() + 40.0..=rect.top() + 58.0, Stroke::new(1.5, Color32::from_rgb(0xff, 0xd8, 0x4d)));
            }
        }
        // Keyframes.
        for kf in &self.project.camera.super_.keyframes {
            let c = pos2(x_of(kf.t), rect.top() + 32.0);
            let pts = vec![c + vec2(0.0, -6.0), c + vec2(6.0, 0.0), c + vec2(0.0, 6.0), c + vec2(-6.0, 0.0)];
            p.add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
        }
        // Playhead.
        let x = x_of(self.t);
        p.vline(x, rect.y_range(), Stroke::new(2.0, ACCENT));
        p.add(egui::Shape::convex_polygon(vec![pos2(x - 6.0, rect.top()), pos2(x + 6.0, rect.top()), pos2(x, rect.top() + 8.0)], ACCENT, Stroke::NONE));
    }

    fn timeline_input(&mut self, rect: Rect, resp: &egui::Response) {
        if self.clean.is_none() {
            return;
        }
        let (a, b) = self.tl_range();
        let t_of = |x: f32| a + ((x - rect.left()) / rect.width()) as f64 * (b - a);
        let x_of = |t: f64| rect.left() + ((t - a) / (b - a)) as f32 * rect.width();
        let pressed = resp.drag_started() || resp.clicked();
        if pressed {
            if let Some(p) = resp.interact_pointer_pos() {
                self.pause();
                let m = self.markers();
                let key = (p.y > rect.top() + 22.0 && p.y < rect.top() + 44.0)
                    .then(|| self.project.camera.super_.keyframes.iter().position(|k| (x_of(k.t) - p.x).abs() < 7.0))
                    .flatten();
                let marker = MarkerKind::ALL.into_iter().find(|k| {
                    let t = match k {
                        MarkerKind::In => Some(m.t_in),
                        _ => self.project.timeline.get(*k).map(|_| match k {
                            MarkerKind::SuperStart => m.super_start,
                            MarkerKind::AnimStart => m.anim_start,
                            MarkerKind::PostStart => m.post_start,
                            _ => m.out,
                        }),
                    };
                    p.y > rect.top() + 16.0 && t.is_some_and(|t| (x_of(t) - p.x).abs() < 5.0)
                });
                self.drag = Some(match (key, marker) {
                    (Some(i), _) => Drag::Key(i),
                    (None, Some(k)) => Drag::Marker(k),
                    _ => Drag::Scrub,
                });
                let t = match self.drag {
                    Some(Drag::Key(i)) => self.project.camera.super_.keyframes[i].t,
                    Some(Drag::Marker(k)) => match k {
                        MarkerKind::In => m.t_in,
                        MarkerKind::SuperStart => m.super_start,
                        MarkerKind::AnimStart => m.anim_start,
                        MarkerKind::PostStart => m.post_start,
                        MarkerKind::Out => m.out,
                    },
                    _ => self.snap(t_of(p.x)),
                };
                self.seek(t);
            }
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let t = self.snap(t_of(p.x).clamp(0.0, self.duration()));
                match self.drag {
                    Some(Drag::Scrub) => self.seek(t),
                    Some(Drag::Marker(k)) => {
                        self.set_marker(k, Some(t));
                        self.seek(t);
                    }
                    Some(Drag::Key(i)) => {
                        let m = self.markers();
                        let fps = self.fps();
                        let t = t.clamp(m.super_start + 1.0 / fps, m.post_start.max(m.super_start + 1.0 / fps));
                        if let Some(k) = self.project.camera.super_.keyframes.get_mut(i) {
                            k.t = (t * 10000.0).round() / 10000.0;
                        }
                        self.seek(t);
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() || resp.clicked() {
            if let Some(Drag::Key(_)) = self.drag {
                self.project.camera.super_.keyframes.sort_by(|a, b| a.t.total_cmp(&b.t));
            }
            self.drag = None;
        }
        let _ = (OUT_H, MUTED);
    }
}
