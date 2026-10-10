// A GUI program on Windows (no console window); CLI commands re-attach to the
// console they were started from.
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{bail, Result};
use std::sync::atomic::AtomicBool;

use combo_editor::{detect, paths, project::Project, render, sync, tracking};

mod gui;
mod icon;

const USAGE: &str = "Combo Editor

USAGE:
  combo-editor [PROJECT.json]                       open the editor
  combo-editor export PROJECT.json [-o OUT.mp4]     render a project without the GUI
  combo-editor still PROJECT.json SECONDS OUT.png   render one frame exactly as exported
  combo-editor detect PROJECT.json [--write]        find the Super Art; --write saves the markers
  combo-editor sync UI_VIDEO CLEAN_VIDEO            print the audio offset (ui = clean + offset)
  combo-editor track CLEAN_VIDEO                    analyse fighter positions (cached)
";

fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn cli(args: &[String]) -> Result<()> {
    let cancel = AtomicBool::new(false);
    match args[0].as_str() {
        "export" => {
            let Some(path) = args.get(1) else { bail!(USAGE) };
            let project = Project::load(path.as_ref())?;
            let out = match args.iter().position(|a| a == "-o" || a == "--output") {
                Some(i) => args.get(i + 1).cloned().unwrap_or_default(),
                None => project.export.output.clone(),
            };
            let last = std::cell::Cell::new(-1);
            let done = render::export(&project, &out, None, &|f, msg| {
                let pct = (f * 100.0) as i32;
                if pct != last.get() {
                    last.set(pct);
                    eprint!("\r{pct:3}%  {msg:40}");
                }
            }, &cancel)?;
            eprintln!();
            println!("{}", done.display());
        }
        "still" => {
            let (Some(path), Some(t), Some(out)) = (args.get(1), args.get(2), args.get(3)) else { bail!(USAGE) };
            let project = Project::load(path.as_ref())?;
            let prepared = render::prepare(&project, None, &|_| {}, &cancel)?;
            let img = render::render_still(&project, &prepared, t.parse()?)?;
            render::save_png(&img, out)?;
        }
        "detect" => {
            let Some(path) = args.get(1) else { bail!(USAGE) };
            let mut project = Project::load(path.as_ref())?;
            let d = detect::detect(&project, &|f| eprint!("\r{:3}%", (f * 100.0) as i32), &cancel)?;
            eprintln!();
            println!(
                "super start {:.3} s\nanimation start {:.3} s\npost start {:.3} s\nconfidence {:.0}%{}",
                d.super_start, d.anim_start, d.post_start, d.confidence * 100.0,
                if d.startup_found { "" } else { " (startup estimated)" }
            );
            if args.iter().any(|a| a == "--write") {
                project.timeline.super_start = Some(d.super_start);
                project.timeline.anim_start = Some(d.anim_start);
                project.timeline.post_start = Some(d.post_start);
                project.camera.super_.startup_auto = true;
                project.camera.super_.animation_auto = true;
                project.save(path.as_ref())?;
                println!("markers written to {path}");
            }
        }
        "sync" => {
            let (Some(ui), Some(clean)) = (args.get(1), args.get(2)) else { bail!(USAGE) };
            let r = sync::sync_files(ui, clean, 60.0)?;
            println!("offset {:.4} s (confidence {:.0}%)", r.offset, r.confidence * 100.0);
        }
        "track" => {
            let Some(clean) = args.get(1) else { bail!(USAGE) };
            let t = tracking::track_video(clean, &Default::default(), &|f| eprint!("\r{:3}%", (f * 100.0) as i32), &cancel)?;
            eprintln!();
            println!("{} frames tracked; first centre x = {:.1}", t.frames, t.center_x.first().copied().unwrap_or(0.0));
        }
        "-h" | "--help" | "help" => print!("{USAGE}"),
        _ => bail!("Unknown command {}\n\n{USAGE}", args[0]),
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let is_cli = args.first().is_some_and(|a| ["export", "still", "detect", "sync", "track", "-h", "--help", "help"].contains(&a.as_str()));
    if is_cli {
        attach_console();
        if let Err(e) = cli(&args) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        paths::log(&format!("panic: {info}"));
        eprintln!("{info}");
    }));
    let flags: Vec<&str> = args.iter().map(String::as_str).filter(|a| a.starts_with("--")).collect();
    let project = args.iter().find(|a| !a.starts_with("--")).cloned();
    if let Err(e) = gui::run(project, flags.contains(&"--opengl")) {
        attach_console();
        eprintln!("error: {e:#}");
        paths::log(&format!("could not start: {e:#}"));
        if std::env::var_os("COMBO_EDITOR_SMOKE").is_some() {
            std::process::exit(1);
        }
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Combo Editor")
            .set_description(format!("Combo Editor could not start:\n\n{e:#}"))
            .show();
        std::process::exit(1);
    }
}
