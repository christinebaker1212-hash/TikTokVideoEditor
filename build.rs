//! Embeds the icon and version information into the Windows executable.

use std::io::Write;

include!("src/icon.rs");

/// Write a .ico with 32-bit BMP entries (no image codec needed).
fn write_ico(path: &std::path::Path, sizes: &[usize]) -> std::io::Result<()> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&n| {
            let rgba = icon_rgba(n);
            let mut d = Vec::new();
            // BITMAPINFOHEADER; height is doubled to include the AND mask.
            for v in [40u32, n as u32, (n * 2) as u32] {
                d.extend(v.to_le_bytes());
            }
            d.extend(1u16.to_le_bytes());
            d.extend(32u16.to_le_bytes());
            d.extend([0u8; 24]);
            for y in (0..n).rev() {
                for x in 0..n {
                    let p = &rgba[(y * n + x) * 4..(y * n + x) * 4 + 4];
                    d.extend([p[2], p[1], p[0], p[3]]);
                }
            }
            let mask_row = (n.div_ceil(32)) * 4;
            d.extend(vec![0u8; mask_row * n]);
            d
        })
        .collect();
    let mut f = std::fs::File::create(path)?;
    f.write_all(&[0, 0, 1, 0])?;
    f.write_all(&(sizes.len() as u16).to_le_bytes())?;
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (n, img) in sizes.iter().zip(&images) {
        let dim = if *n >= 256 { 0 } else { *n as u8 };
        f.write_all(&[dim, dim, 0, 0])?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&32u16.to_le_bytes())?;
        f.write_all(&(img.len() as u32).to_le_bytes())?;
        f.write_all(&offset.to_le_bytes())?;
        offset += img.len() as u32;
    }
    for img in &images {
        f.write_all(img)?;
    }
    Ok(())
}

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("icon.ico");
    write_ico(&out, &[16, 24, 32, 48, 64, 256]).expect("write icon");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(out.to_str().unwrap());
    res.set("ProductName", "Combo Editor");
    res.set("FileDescription", "Combo Editor: automatic TikTok edits for fighting-game combos");
    res.set("OriginalFilename", "ComboEditor.exe");
    if let Err(e) = res.compile() {
        // Missing resource compiler: still build, just without the icon.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
