//! Colours, frames and the window icon.

use egui::{Color32, CornerRadius, Frame, Margin, Stroke};

use combo_editor::project::Part;

pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x24);
pub const PANEL2: Color32 = Color32::from_rgb(0x23, 0x26, 0x31);
pub const LINE: Color32 = Color32::from_rgb(0x31, 0x35, 0x43);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xee);
pub const MUTED: Color32 = Color32::from_rgb(0x8d, 0x93, 0xa5);
pub const ACCENT: Color32 = Color32::from_rgb(0xe2, 0x33, 0x6b);
pub const BLUE: Color32 = Color32::from_rgb(0x4e, 0x7c, 0xff);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x8f, 0x8a);
pub const CYAN: Color32 = Color32::from_rgb(0x50, 0xdc, 0xff);

pub fn part_color(p: Part) -> Color32 {
    match p {
        Part::Combo => BLUE,
        Part::Startup => Color32::from_rgb(0xf0, 0xa0, 0x32),
        Part::Animation => ACCENT,
        Part::Post => Color32::from_rgb(0x3c, 0xc4, 0x8a),
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x15, 0x16, 0x1c);
    v.faint_bg_color = PANEL2;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = BLUE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_fill = PANEL2;
    v.widgets.inactive.weak_bg_fill = PANEL2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2c, 0x30, 0x3d);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x2c, 0x30, 0x3d);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4a, 0x50, 0x63));
    v.widgets.active.weak_bg_fill = ACCENT;
    v.widgets.active.bg_fill = ACCENT;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(5);
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.slider_width = 200.0;
    });
}

pub fn bar_frame() -> Frame {
    Frame::new().fill(PANEL).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, LINE))
}

pub fn side_frame() -> Frame {
    Frame::new().fill(PANEL).inner_margin(Margin::symmetric(12, 8))
}

pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(text.to_uppercase()).size(12.0).strong().color(MUTED));
    ui.add_space(2.0);
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(MUTED));
}

pub fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE)).fill(ACCENT)
}

pub fn icon() -> egui::IconData {
    egui::IconData { rgba: crate::icon::icon_rgba(64), width: 64, height: 64 }
}
