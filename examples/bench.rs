use combo_editor::camera::{Cam, FrameCam};
use combo_editor::compositor::compose;
use combo_editor::imaging::{crop_scale, yuv_region, Filter};
use combo_editor::media::YuvFrame;
use combo_editor::project::{Part, Skin};
use std::time::Instant;

fn main() {
    let mut f = YuvFrame::black(1920, 1080);
    for (i, v) in f.data.iter_mut().enumerate() { *v = (i * 7 % 251) as u8; }
    let fc = FrameCam { cam: Cam { cx: 900.0, cy: 540.0, zoom: 1.0 }, part: Part::Combo,
        alphas: [("health".to_string(), 1.0f32), ("damage".to_string(), 1.0)].into_iter().collect() };
    let skin = Skin::default();
    let time = |name: &str, n: u32, mut g: Box<dyn FnMut()>| {
        g();
        let t = Instant::now();
        for _ in 0..n { g(); }
        println!("{name:28} {:7.2} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    };
    time("yuv_region 608x1080", 30, Box::new(|| { yuv_region(&f, 656, 0, 608, 1080); }));
    time("crop_scale -> 414x736 lin", 30, Box::new(|| { crop_scale(&f, 656.0, 0.0, 736.0/1080.0, 736.0/1080.0, 414, 736, Filter::Linear); }));
    time("compose preview 414x736", 30, Box::new(|| { compose(&f, Some(&f), &fc, &skin, 1080.0, 414, 736, Filter::Linear, true).to_rgba8(); }));
    time("compose export lanczos", 10, Box::new(|| { compose(&f, Some(&f), &fc, &skin, 1080.0, 1080, 1920, Filter::Lanczos3, true).to_yuv420(); }));
}
