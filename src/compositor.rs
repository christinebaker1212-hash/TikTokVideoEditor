//! One output frame = camera crop of the clean video + feathered UI elements.
//! The preview uses this same code at screen size, the export at 1080x1920.

use crate::camera::{crop_size, FrameCam};
use crate::imaging::{box_blur, crop_scale, resize_bilinear, Filter, ImageF};
use crate::media::YuvFrame;
use crate::project::{Element, Feather, Skin, OUT_W};

/// Alpha ramp that fades each edge to 0 over the given pixel widths.
pub fn feather_mask(w: usize, h: usize, f: &Feather, scale: f64) -> (Vec<f32>, Vec<f32>) {
    let ramp = |n: usize, a: f64, b: f64| -> Vec<f32> {
        (0..n)
            .map(|i| {
                let x = i as f64 + 0.5;
                let mut r = 1.0;
                if a > 0.0 {
                    r *= (x / a).clamp(0.0, 1.0);
                }
                if b > 0.0 {
                    r *= ((n as f64 - x) / b).clamp(0.0, 1.0);
                }
                r as f32
            })
            .collect()
    };
    (ramp(w, f.left * scale, f.right * scale), ramp(h, f.top * scale, f.bottom * scale))
}

/// Integer placement of an element on an output of width `out_w`.
pub fn dst_rect(el: &Element, out_w: usize) -> (i64, i64, usize, usize) {
    let k = out_w as f64 / OUT_W as f64;
    let d = &el.dst;
    ((d.x * k).round() as i64, (d.y * k).round() as i64, ((d.w * k).round() as usize).max(1), ((d.h * k).round() as usize).max(1))
}

pub fn draw_element(out: &mut ImageF, ui: &YuvFrame, el: &Element, alpha: f32, ref_size: [f64; 2], filter: Filter) {
    let opacity = (el.opacity as f32 * alpha).clamp(0.0, 1.0);
    if opacity <= 0.001 || el.src.w <= 0.0 || el.src.h <= 0.0 {
        return;
    }
    let (kx, ky) = (ui.w as f64 / ref_size[0], ui.h as f64 / ref_size[1]);
    let (dx, dy, dw, dh) = dst_rect(el, out.w);
    // Skip work for elements entirely off-frame.
    if dx >= out.w as i64 || dy >= out.h as i64 || dx + dw as i64 <= 0 || dy + dh as i64 <= 0 {
        return;
    }
    let (sw, sh) = (el.src.w * kx, el.src.h * ky);
    let patch = crop_scale(ui, el.src.x * kx, el.src.y * ky, dw as f64 / sw, dh as f64 / sh, dw, dh, filter);
    let (mx, my) = feather_mask(dw, dh, &el.feather, out.w as f64 / OUT_W as f64);
    for py in 0..dh {
        let oy = dy + py as i64;
        if oy < 0 || oy >= out.h as i64 {
            continue;
        }
        for px in 0..dw {
            let ox = dx + px as i64;
            if ox < 0 || ox >= out.w as i64 {
                continue;
            }
            let a = mx[px] * my[py] * opacity;
            if a <= 0.0 {
                continue;
            }
            let o = (oy as usize * out.w + ox as usize) * 3;
            let p = (py * dw + px) * 3;
            for c in 0..3 {
                out.data[o + c] += (patch.data[p + c] - out.data[o + c]) * a;
            }
        }
    }
}

/// Soft, darkened, full-height crop around `cx` (frame pixels): the fill
/// behind a zoomed-out picture.
fn blurred_background(clean: &YuvFrame, cx: f64, out_w: usize, out_h: usize) -> ImageF {
    let (sw, sh) = ((out_w / 24).max(4), (out_h / 24).max(4));
    let cw = (clean.h as f64 * sw as f64 / sh as f64).min(clean.w as f64);
    let x0 = (cx - cw / 2.0).clamp(0.0, clean.w as f64 - cw);
    let s = sh as f64 / clean.h as f64;
    let mut small = crop_scale(clean, x0, 0.0, s, s, sw, sh, Filter::Cubic);
    box_blur(&mut small, 2);
    for v in &mut small.data {
        *v *= 0.45;
    }
    resize_bilinear(&small, out_w, out_h)
}

/// Compose a frame. Camera coordinates are in clean-source pixels (`src_w` x `src_h`);
/// the decoded frames may be smaller (half-resolution preview). Below zoom 1
/// the whole source height is shown, shrunk, over a blurred (or black) fill.
#[allow(clippy::too_many_arguments)]
pub fn compose(
    clean: &YuvFrame,
    ui: Option<&YuvFrame>,
    fc: &FrameCam,
    skin: &Skin,
    src_h: f64,
    out_w: usize,
    out_h: usize,
    filter: Filter,
    blur_fill: bool,
) -> ImageF {
    let k = clean.h as f64 / src_h;
    let (cw, ch) = crop_size(fc.cam.zoom, src_h);
    let x0 = (fc.cam.cx - cw / 2.0) * k;
    let scale = out_h as f64 / (ch * k);
    let mut out = if fc.cam.zoom < 1.0 - 1e-9 {
        let mut out = if blur_fill { blurred_background(clean, fc.cam.cx * k, out_w, out_h) } else { ImageF::new(out_w, out_h) };
        let fg_h = ((clean.h as f64 * scale).round() as usize).clamp(1, out_h);
        let fg = crop_scale(clean, x0, 0.0, scale, scale, out_w, fg_h, filter);
        let top = (out_h - fg_h) / 2;
        out.data[top * out_w * 3..(top + fg_h) * out_w * 3].copy_from_slice(&fg.data);
        out
    } else {
        let y0 = (fc.cam.cy - ch / 2.0) * k;
        crop_scale(clean, x0, y0, scale, scale, out_w, out_h, filter)
    };
    if let Some(ui) = ui {
        for el in &skin.elements {
            if el.enabled {
                let a = fc.alphas.get(&el.id).copied().unwrap_or(0.0);
                draw_element(&mut out, ui, el, a, skin.reference_size, filter);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Cam;
    use crate::project::{Part, Rect};
    use std::collections::HashMap;

    fn frame_with_bar() -> YuvFrame {
        // Black 1920x1080 with a pure red bar where the health bar lives.
        let mut img = ImageF::new(1920, 1080);
        for y in 52..98 {
            for x in 1040..1755 {
                let i = (y * 1920 + x) * 3;
                img.data[i] = 255.0;
            }
        }
        YuvFrame { w: 1920, h: 1080, data: img.to_yuv420() }
    }

    #[test]
    fn feather_ramps() {
        let (mx, my) = feather_mask(100, 20, &Feather { left: 10.0, ..Default::default() }, 1.0);
        assert!(mx[0] < 0.1 && mx[50] == 1.0 && mx[99] == 1.0 && my[10] == 1.0);
    }

    #[test]
    fn overlay_lands_at_destination_with_opacity() {
        let ui = frame_with_bar();
        let mut out = ImageF::new(1080, 1920);
        let el = Element {
            src: Rect::new(1040.0, 52.0, 715.0, 46.0),
            dst: Rect::new(0.0, 165.0, 1080.0, 69.0),
            opacity: 0.5,
            feather: Feather::default(),
            ..Default::default()
        };
        draw_element(&mut out, &ui, &el, 1.0, [1920.0, 1080.0], Filter::Lanczos3);
        let p = out.px(540, 200);
        assert!((p[0] - 127.5).abs() < 6.0 && p[1] < 10.0 && p[2] < 10.0, "{p:?}");
        assert_eq!(out.px(540, 100), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn compose_is_full_height_crop() {
        let ui = frame_with_bar();
        let fc = FrameCam { cam: Cam { cx: 1400.0, cy: 540.0, zoom: 1.0 }, part: Part::Combo, alphas: HashMap::new() };
        // Camera centred on the bar: the bar (rows 52..98) appears at the top of the output.
        let out = compose(&ui, None, &fc, &Skin::default(), 1080.0, 1080, 1920, Filter::Cubic, true);
        let row = (75.0 * 1920.0 / 1080.0) as usize;
        assert!(out.px(540, row)[0] > 200.0);
        assert!(out.px(540, 1000)[0] < 5.0);
    }

    #[test]
    fn zoomed_out_shows_full_height_over_fill() {
        let frame = frame_with_bar();
        // Crop as wide as the source: the 1920x1080 picture becomes 1080x607.5 in the middle.
        let fc = FrameCam { cam: Cam { cx: 960.0, cy: 540.0, zoom: 1080.0 * 0.5625 / 1920.0 }, part: Part::Combo, alphas: HashMap::new() };
        let out = compose(&frame, None, &fc, &Skin::default(), 1080.0, 1080, 1920, Filter::Cubic, false);
        let top = (1920 - 608) / 2;
        // The bar (source rows 52..98, x 1040..1755) lands at the matching place in the band.
        let y = top + (75.0 * 1080.0 / 1920.0) as usize;
        let x = (1400.0 * 1080.0 / 1920.0) as usize;
        assert!(out.px(x, y)[0] > 200.0, "{:?}", out.px(x, y));
        assert_eq!(out.px(540, 100), [0.0, 0.0, 0.0], "black fill above");
        assert_eq!(out.px(540, 1850), [0.0, 0.0, 0.0], "black fill below");
    }
}
