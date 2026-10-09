// The app icon, drawn procedurally. Shared by the window (at runtime) and by
// build.rs, which turns it into the .exe's .ico resource.

/// The app icon: a 9:16 frame cut out of a 16:9 one, in the accent colour.
pub fn icon_rgba(size: usize) -> Vec<u8> {
    let mut px = vec![0u8; size * size * 4];
    let s = size as f32;
    let set = |px: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
        let i = (y * size + x) * 4;
        px[i..i + 4].copy_from_slice(&c);
    };
    let r = s * 0.18;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded-square background.
            let dx = (fx - s / 2.0).abs() - (s / 2.0 - r);
            let dy = (fy - s / 2.0).abs() - (s / 2.0 - r);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r;
            if d > 0.5 {
                continue;
            }
            let a = (0.5 - d).clamp(0.0, 1.0);
            let mut c = [0x1b, 0x1d, 0x24, (a * 255.0) as u8];
            // Wide 16:9 outline.
            let (wx0, wx1, wy0, wy1) = (s * 0.12, s * 0.88, s * 0.30, s * 0.70);
            let t = (s * 0.035).max(1.0);
            let on_wide = fx >= wx0 && fx <= wx1 && fy >= wy0 && fy <= wy1
                && (fx < wx0 + t || fx > wx1 - t || fy < wy0 + t || fy > wy1 - t);
            if on_wide {
                c = [0x8d, 0x93, 0xa5, c[3]];
            }
            // Tall 9:16 accent frame.
            let (tx0, tx1, ty0, ty1) = (s * 0.36, s * 0.64, s * 0.14, s * 0.86);
            if fx >= tx0 && fx <= tx1 && fy >= ty0 && fy <= ty1 {
                c = [0xe2, 0x33, 0x6b, c[3]];
                // Play triangle.
                let (cx, cy) = (s * 0.5, s * 0.5);
                let h = s * 0.09;
                if fx >= cx - h * 0.6 && fx <= cx + h * 0.9 && (fy - cy).abs() <= (cx + h * 0.9 - fx) * 0.66 {
                    c = [255, 255, 255, c[3]];
                }
            }
            set(&mut px, x, y, c);
        }
    }
    px
}
