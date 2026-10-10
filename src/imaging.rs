//! Pixel work: YUV <-> RGB (BT.709 limited range) and a separable resampler.
//!
//! Images are interleaved RGB `f32` in 0..255. Crops are converted from YUV
//! only for the source region a resample actually touches, so the cost scales
//! with the output, not with the 1080p source.

use rayon::prelude::*;

use crate::media::YuvFrame;

#[derive(Clone, Debug)]
pub struct ImageF {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl ImageF {
    pub fn new(w: usize, h: usize) -> ImageF {
        ImageF { w, h, data: vec![0.0; w * h * 3] }
    }

    pub fn px(&self, x: usize, y: usize) -> [f32; 3] {
        let i = (y * self.w + x) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    /// RGBA8 for display.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![255u8; self.w * self.h * 4];
        out.par_chunks_mut(4 * self.w).zip(self.data.par_chunks(3 * self.w)).for_each(|(o, i)| {
            for (o, i) in o.chunks_exact_mut(4).zip(i.chunks_exact(3)) {
                o[0] = q(i[0]);
                o[1] = q(i[1]);
                o[2] = q(i[2]);
            }
        });
        out
    }

    /// Planar yuv420p (BT.709, limited range) for the encoder.
    pub fn to_yuv420(&self) -> Vec<u8> {
        let (w, h) = (self.w, self.h);
        let (cw, ch) = (w / 2, h / 2);
        let mut out = vec![0u8; w * h + 2 * cw * ch];
        let (ys, uv) = out.split_at_mut(w * h);
        let (us, vs) = uv.split_at_mut(cw * ch);
        ys.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let src = &self.data[y * w * 3..(y + 1) * w * 3];
            for (x, o) in row.iter_mut().enumerate() {
                let (r, g, b) = (src[x * 3], src[x * 3 + 1], src[x * 3 + 2]);
                *o = q(16.0 + 0.18259 * r + 0.61423 * g + 0.06201 * b);
            }
        });
        us.par_chunks_mut(cw).zip(vs.par_chunks_mut(cw)).enumerate().for_each(|(cy, (urow, vrow))| {
            for cx in 0..cw {
                let mut acc = [0.0f32; 3];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let p = self.px((cx * 2 + dx).min(w - 1), (cy * 2 + dy).min(h - 1));
                        acc[0] += p[0];
                        acc[1] += p[1];
                        acc[2] += p[2];
                    }
                }
                let (r, g, b) = (acc[0] / 4.0, acc[1] / 4.0, acc[2] / 4.0);
                urow[cx] = q(128.0 - 0.10064 * r - 0.33857 * g + 0.43922 * b);
                vrow[cx] = q(128.0 + 0.43922 * r - 0.39894 * g - 0.04027 * b);
            }
        });
        out
    }
}

#[inline]
pub fn q(v: f32) -> u8 {
    (v + 0.5).clamp(0.0, 255.0) as u8
}

#[inline]
fn yuv_to_rgb(y: f32, u: f32, v: f32) -> [f32; 3] {
    let c = 1.164_383_6 * (y - 16.0);
    let (d, e) = (u - 128.0, v - 128.0);
    [c + 1.792_741 * e, c - 0.213_248_6 * d - 0.532_909_3 * e, c + 2.112_402 * d]
}

/// Bilinear chroma sample at luma position (x, y) (centre-sited 4:2:0).
#[inline]
fn chroma(plane: &[u8], cw: usize, chh: usize, x: usize, y: usize) -> f32 {
    let fx = ((x as f32 - 0.5) * 0.5).max(0.0);
    let fy = ((y as f32 - 0.5) * 0.5).max(0.0);
    let x0 = (fx as usize).min(cw - 1);
    let y0 = (fy as usize).min(chh - 1);
    let x1 = (x0 + 1).min(cw - 1);
    let y1 = (y0 + 1).min(chh - 1);
    let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
    let p = |xx: usize, yy: usize| plane[yy * cw + xx] as f32;
    let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
    let bot = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
    top * (1.0 - ay) + bot * ay
}

/// Convert the rectangle [x0, x0+w) x [y0, y0+h) of a YUV frame to RGB.
pub fn yuv_region(frame: &YuvFrame, x0: usize, y0: usize, w: usize, h: usize) -> ImageF {
    let mut out = ImageF::new(w, h);
    let (fy, fu, fv) = (frame.y(), frame.u(), frame.v());
    let (cw, chh) = (frame.w / 2, frame.h / 2);
    out.data.par_chunks_mut(w * 3).enumerate().for_each(|(ry, row)| {
        let y = y0 + ry;
        for rx in 0..w {
            let x = x0 + rx;
            let p = yuv_to_rgb(fy[y * frame.w + x] as f32, chroma(fu, cw, chh, x, y), chroma(fv, cw, chh, x, y));
            row[rx * 3..rx * 3 + 3].copy_from_slice(&p);
        }
    });
    out
}

/// Fast whole-frame RGBA thumbnail (bilinear on the luma grid), for monitors.
pub fn yuv_thumbnail(frame: &YuvFrame, out_w: usize, out_h: usize) -> Vec<u8> {
    let mut out = vec![255u8; out_w * out_h * 4];
    let (fy, fu, fv) = (frame.y(), frame.u(), frame.v());
    let (cw, chh) = (frame.w / 2, frame.h / 2);
    let sx = frame.w as f32 / out_w as f32;
    let sy = frame.h as f32 / out_h as f32;
    out.par_chunks_mut(out_w * 4).enumerate().for_each(|(oy, row)| {
        let y = (((oy as f32 + 0.5) * sy) as usize).min(frame.h - 1);
        for ox in 0..out_w {
            let x = (((ox as f32 + 0.5) * sx) as usize).min(frame.w - 1);
            let yy = fy[y * frame.w + x] as f32;
            let u = fu[(y / 2).min(chh - 1) * cw + (x / 2).min(cw - 1)] as f32;
            let v = fv[(y / 2).min(chh - 1) * cw + (x / 2).min(cw - 1)] as f32;
            let p = yuv_to_rgb(yy, u, v);
            row[ox * 4] = q(p[0]);
            row[ox * 4 + 1] = q(p[1]);
            row[ox * 4 + 2] = q(p[2]);
        }
    });
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Linear,
    Cubic,
    Lanczos3,
}

impl Filter {
    pub fn from_name(s: &str) -> Filter {
        match s {
            "linear" | "bilinear" => Filter::Linear,
            "cubic" | "bicubic" => Filter::Cubic,
            _ => Filter::Lanczos3,
        }
    }
    fn support(self) -> f32 {
        match self {
            Filter::Linear => 1.0,
            Filter::Cubic => 2.0,
            Filter::Lanczos3 => 3.0,
        }
    }
    fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        match self {
            Filter::Linear => (1.0 - x).max(0.0),
            Filter::Cubic => {
                // Catmull-Rom (a = -0.5): sharp without much ringing.
                let a = -0.5;
                if x < 1.0 {
                    (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
                } else if x < 2.0 {
                    a * x * x * x - 5.0 * a * x * x + 8.0 * a * x - 4.0 * a
                } else {
                    0.0
                }
            }
            Filter::Lanczos3 => {
                if x < 1e-6 {
                    1.0
                } else if x < 3.0 {
                    let px = std::f32::consts::PI * x;
                    3.0 * px.sin() * (px / 3.0).sin() / (px * px)
                } else {
                    0.0
                }
            }
        }
    }
}

/// Per-output-sample source indices and weights along one axis.
pub struct Axis {
    pub taps: usize,
    pub idx: Vec<usize>,
    pub wts: Vec<f32>,
    pub min: usize,
    pub max: usize,
}

/// Output sample `i` covers source coordinate `origin + (i + 0.5) / scale`
/// (in pixel-edge units, i.e. pixel k spans [k, k+1)).
pub fn axis(out_n: usize, src_n: usize, scale: f64, origin: f64, filter: Filter) -> Axis {
    let fscale = scale.min(1.0) as f32; // widen the kernel when downscaling
    let support = filter.support() / fscale;
    let taps = (support.ceil() as usize) * 2 + 1;
    let mut idx = Vec::with_capacity(out_n * taps);
    let mut wts = Vec::with_capacity(out_n * taps);
    let (mut min, mut max) = (usize::MAX, 0usize);
    for i in 0..out_n {
        let centre = (origin + (i as f64 + 0.5) / scale - 0.5) as f32; // pixel-centre coordinate
        let first = (centre - support).floor() as i64 + 1;
        let mut sum = 0.0;
        let start = wts.len();
        for k in 0..taps as i64 {
            let s = first + k;
            let w = filter.weight((s as f32 - centre) * fscale);
            let si = s.clamp(0, src_n as i64 - 1) as usize;
            min = min.min(si);
            max = max.max(si);
            idx.push(si);
            wts.push(w);
            sum += w;
        }
        if sum.abs() > 1e-6 {
            for w in &mut wts[start..] {
                *w /= sum;
            }
        }
    }
    Axis { taps, idx, wts, min, max }
}

/// Resample `src` (which starts at source pixel (ox, oy) of the full image the
/// axes were computed against) to `ax.len x ay.len`.
pub fn resample(src: &ImageF, ox: usize, oy: usize, ax: &Axis, ay: &Axis) -> ImageF {
    let out_w = ax.idx.len() / ax.taps;
    let out_h = ay.idx.len() / ay.taps;
    let rows = ay.max - ay.min + 1;
    // Horizontal pass over the rows the vertical pass will read.
    let mut tmp = vec![0.0f32; out_w * rows * 3];
    tmp.par_chunks_mut(out_w * 3).enumerate().for_each(|(r, trow)| {
        let sy = ay.min + r - oy;
        let srow = &src.data[sy * src.w * 3..(sy + 1) * src.w * 3];
        for x in 0..out_w {
            let mut acc = [0.0f32; 3];
            for t in 0..ax.taps {
                let k = x * ax.taps + t;
                let w = ax.wts[k];
                if w == 0.0 {
                    continue;
                }
                let sx = (ax.idx[k] - ox) * 3;
                acc[0] += srow[sx] * w;
                acc[1] += srow[sx + 1] * w;
                acc[2] += srow[sx + 2] * w;
            }
            trow[x * 3..x * 3 + 3].copy_from_slice(&acc);
        }
    });
    let mut out = ImageF::new(out_w, out_h);
    out.data.par_chunks_mut(out_w * 3).enumerate().for_each(|(y, orow)| {
        for t in 0..ay.taps {
            let k = y * ay.taps + t;
            let w = ay.wts[k];
            if w == 0.0 {
                continue;
            }
            let r = ay.idx[k] - ay.min;
            let trow = &tmp[r * out_w * 3..(r + 1) * out_w * 3];
            for (o, s) in orow.iter_mut().zip(trow) {
                *o += s * w;
            }
        }
    });
    out
}

/// Bilinear sample straight from the YUV planes: what a GPU would do. Used for
/// the live preview, where speed matters more than ideal downscaling.
fn crop_scale_bilinear(frame: &YuvFrame, x0: f64, y0: f64, scale_x: f64, scale_y: f64, out_w: usize, out_h: usize) -> ImageF {
    let mut out = ImageF::new(out_w, out_h);
    let (fy, fu, fv) = (frame.y(), frame.u(), frame.v());
    let (w, h) = (frame.w, frame.h);
    let (cw, chh) = (w / 2, h / 2);
    // Precompute horizontal sample positions for luma and chroma.
    let col = |n: usize, origin: f64, scale: f64, sub: f64, max: usize| -> Vec<(usize, usize, f32)> {
        (0..out_w)
            .map(|u| {
                let x = ((origin + (u as f64 + 0.5) / scale) / sub - 0.5).clamp(0.0, (max - 1) as f64);
                let i = (x as usize).min(max - 1);
                (i, (i + 1).min(max - 1), (x - i as f64) as f32)
            })
            .take(n)
            .collect()
    };
    let lx = col(out_w, x0, scale_x, 1.0, w);
    let cx = col(out_w, x0, scale_x, 2.0, cw);
    out.data.par_chunks_mut(out_w * 3).enumerate().for_each(|(v, row)| {
        let samp = |sub: f64, max: usize| {
            let y = ((y0 + (v as f64 + 0.5) / scale_y) / sub - 0.5).clamp(0.0, (max - 1) as f64);
            let i = (y as usize).min(max - 1);
            (i, (i + 1).min(max - 1), (y - i as f64) as f32)
        };
        let (ly0, ly1, lay) = samp(1.0, h);
        let (cy0, cy1, cay) = samp(2.0, chh);
        let bil = |p: &[u8], stride: usize, (x0, x1, ax): (usize, usize, f32), y0: usize, y1: usize, ay: f32| {
            let a = p[y0 * stride + x0] as f32 * (1.0 - ax) + p[y0 * stride + x1] as f32 * ax;
            let b = p[y1 * stride + x0] as f32 * (1.0 - ax) + p[y1 * stride + x1] as f32 * ax;
            a + (b - a) * ay
        };
        for u in 0..out_w {
            let yy = bil(fy, w, lx[u], ly0, ly1, lay);
            let uu = bil(fu, cw, cx[u], cy0, cy1, cay);
            let vv = bil(fv, cw, cx[u], cy0, cy1, cay);
            row[u * 3..u * 3 + 3].copy_from_slice(&yuv_to_rgb(yy, uu, vv));
        }
    });
    out
}

/// Crop the region of `frame` covering `[x0, x0 + out_w/scale) x [y0, ...)`
/// and scale it to `out_w x out_h`.
pub fn crop_scale(frame: &YuvFrame, x0: f64, y0: f64, scale_x: f64, scale_y: f64, out_w: usize, out_h: usize, filter: Filter) -> ImageF {
    if filter == Filter::Linear {
        return crop_scale_bilinear(frame, x0, y0, scale_x, scale_y, out_w, out_h);
    }
    let ax = axis(out_w, frame.w, scale_x, x0, filter);
    let ay = axis(out_h, frame.h, scale_y, y0, filter);
    let region = yuv_region(frame, ax.min, ay.min, ax.max - ax.min + 1, ay.max - ay.min + 1);
    resample(&region, ax.min, ay.min, &ax, &ay)
}

/// Bilinear resize of an RGB image (used to enlarge a tiny, blurred background).
pub fn resize_bilinear(src: &ImageF, w: usize, h: usize) -> ImageF {
    let mut out = ImageF::new(w, h);
    let (sx, sy) = (src.w as f32 / w as f32, src.h as f32 / h as f32);
    out.data.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (src.h - 1) as f32);
        let (y0, ay) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(src.h - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (src.w - 1) as f32);
            let (x0, ax) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(src.w - 1);
            for c in 0..3 {
                let p = |xx: usize, yy: usize| src.data[(yy * src.w + xx) * 3 + c];
                let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * ax;
                let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * ax;
                row[x * 3 + c] = top + (bot - top) * ay;
            }
        }
    });
    out
}

/// In-place box blur (radius `r`, two passes approximate a Gaussian).
pub fn box_blur(img: &mut ImageF, r: usize) {
    let (w, h) = (img.w, img.h);
    for _ in 0..2 {
        let src = img.data.clone();
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 3];
                let mut n = 0.0;
                for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                    for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                        let i = (yy * w + xx) * 3;
                        acc[0] += src[i];
                        acc[1] += src[i + 1];
                        acc[2] += src[i + 2];
                        n += 1.0;
                    }
                }
                let i = (y * w + x) * 3;
                img.data[i] = acc[0] / n;
                img.data[i + 1] = acc[1] / n;
                img.data[i + 2] = acc[2] / n;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, y: u8, u: u8, v: u8) -> YuvFrame {
        let mut f = YuvFrame::black(w, h);
        let n = w * h;
        f.data[..n].fill(y);
        f.data[n..n + n / 4].fill(u);
        f.data[n + n / 4..].fill(v);
        f
    }

    #[test]
    fn yuv_round_trip() {
        let mut img = ImageF::new(4, 4);
        for p in img.data.chunks_mut(3) {
            p.copy_from_slice(&[200.0, 40.0, 30.0]);
        }
        let yuv = img.to_yuv420();
        let f = YuvFrame { w: 4, h: 4, data: yuv };
        let back = yuv_region(&f, 0, 0, 4, 4);
        let p = back.px(1, 1);
        assert!((p[0] - 200.0).abs() < 2.0 && (p[1] - 40.0).abs() < 2.0 && (p[2] - 30.0).abs() < 2.0, "{p:?}");
    }

    #[test]
    fn black_and_white_levels() {
        assert_eq!(yuv_region(&solid(2, 2, 16, 128, 128), 0, 0, 2, 2).px(0, 0).map(q), [0, 0, 0]);
        assert_eq!(yuv_region(&solid(2, 2, 235, 128, 128), 0, 0, 2, 2).px(0, 0).map(q), [255, 255, 255]);
    }

    #[test]
    fn resample_preserves_flat_and_maps_position() {
        for filter in [Filter::Linear, Filter::Cubic, Filter::Lanczos3] {
            let f = solid(64, 32, 120, 100, 160);
            let out = crop_scale(&f, 10.0, 4.0, 1.7, 1.7, 30, 20, filter);
            let flat = yuv_region(&f, 0, 0, 1, 1).px(0, 0);
            for p in out.data.chunks(3) {
                assert!((p[0] - flat[0]).abs() < 0.5);
            }
        }
        // A bright column at source x=20 should land at output x = (20.5 - 10) * 2 - 0.5 = 20.5.
        let mut f = solid(64, 8, 16, 128, 128);
        for y in 0..8 {
            f.data[y * 64 + 20] = 235;
        }
        let out = crop_scale(&f, 10.0, 0.0, 2.0, 2.0, 40, 8, Filter::Linear);
        let row: Vec<f32> = (0..40).map(|x| out.px(x, 3)[0]).collect();
        let peak = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!(peak == 20 || peak == 21, "{peak}");
    }
}
