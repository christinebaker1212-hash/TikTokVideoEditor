# Combo Editor: automatic TikTok edits for fighting-game combos

A native Windows program that turns two 1920×1080 recordings of the same combo into a
1080×1920 TikTok video:

- one recording **with UI** (health bars, damage readout);
- one recording **without UI** (the clean gameplay).

The clean gameplay is cropped to vertical with a camera that follows both fighters. Chosen
UI elements are cut from the UI recording and laid on top with feathered edges and
adjustable opacity. You can add background music with fades, and the result exports as one
H.264/AAC file.

## Download

Get **`ComboEditor-<version>-win64.zip`** from the repository's
[Releases](../../releases) page. You can also download the `ComboEditor-win64` artifact from
the latest [Build](../../actions/workflows/release.yml) run. Unzip it anywhere and run
**`ComboEditor.exe`**.

There is no installer and nothing else to download, because ffmpeg is bundled. Presets,
projects and exports stay in that folder, so you can move or back up the app by copying it.

## Workflow

### 1. Sources & sync
1. **Browse…** for both recordings. They load as soon as both are chosen.
2. Click **Auto-sync (transients)**. Neither recording has music, so their game audio is
   identical. The editor matches the sharp transients (hits, whiffs, announcer) and refines
   the result to the exact sample.
3. Check the result with the **Difference** view: when the recordings are in sync,
   everything except the UI turns black. Fine-tune with ±1 frame or ±10 ms.
4. Choose which recording supplies the game audio, and mark the clip's **In** and **Out**
   points under the timeline (keys `I` / `O`).

### 2. Camera
Click **Analyse fighters** once per clip, then set the part markers under the timeline:

| Part | Markers | Camera |
|---|---|---|
| Combo | In → Super start | Automatic: always centred on both fighters, at native scale (full-height 9:16 crop) |
| Super startup | Super start → Animation start | Manual keyframes |
| Super animation | Animation start → Post start | Manual keyframes |
| Post super | Post start → Out | Automatic centring, plus your offset, zoom and smoothing |

In the super, **drag the preview** to pan or **scroll** to zoom. Each change creates or
updates a keyframe at the playhead (`K` adds one). Keyframes can be dragged along the
timeline.

### 3. UI overlay
Each element has a **source** box (on the UI recording) and a **placement** box (on the
TikTok frame). You can drag both or type exact values.
- **Opacity and edge feathering** can be saved as blend presets.
- **Visible during** chooses which parts show each element.
- **UI skin presets** store the whole layout; save one per in-game UI skin.

### 4. Music & export
- Pick a song, where in the song to start, its volume, and its fade-in and fade-out.
- **Exact frame** renders the current frame through the real export pipeline.
- **Export** writes `workspace\exports\<project>.mp4`, or the file you choose. The default
  settings are x264 CRF 16 and Lanczos upscaling.

### Keyboard
`Space` play/pause · `←`/`→` one frame (with `Shift`: 1 s) · `I`/`O` clip start/end ·
`K` keyframe · `F3` performance overlay.

### Automation
```
ComboEditor.exe export project.json [-o out.mp4]   # render a saved project without the GUI
ComboEditor.exe still project.json 12.5 frame.png  # one export-identical frame
ComboEditor.exe sync with_ui.mp4 clean.mp4         # print the sync offset
```

## How it works

| Module | Role |
|---|---|
| `src/media.rs` | ffmpeg/ffprobe I/O; all video is normalised to 8-bit BT.709 yuv420p |
| `src/sync.rs` | Onset-envelope cross-correlation, then GCC-PHAT refinement |
| `src/tracking.rs` | Per-frame fighter extents from camera-shift-compensated motion (phase correlation) plus saturation; parallel and cached |
| `src/camera.rs` | Markers + tracking + keyframes → per-frame camera path and overlay visibility |
| `src/imaging.rs` | YUV↔RGB, separable Lanczos/bicubic resampler, fast bilinear preview path |
| `src/compositor.rs` | Crop + feathered UI elements, shared by the preview and the export |
| `src/render.rs` | Export: decode threads → parallel compose → x264 encoder thread, plus the audio mix |
| `src/gui/` | Native egui window. The decoder threads keep frame caches indexed by the same master frame number, so the two recordings play in lockstep, and the audio output is the playback clock |

## Building

You need Rust (stable), plus ffmpeg on `PATH` or in an `ffmpeg` folder next to the program.

```powershell
cargo test --release
cargo build --release                      # target\release\combo-editor.exe
.\packaging\get-ffmpeg.ps1 -Dest ffmpeg
.\packaging\package-windows.ps1            # dist\ComboEditor\ + zip
python packaging\smoke_test.py dist\ComboEditor
```

Pushing a tag like `v1.0.0` makes CI build, test and publish the zip as a GitHub Release.

The bundled ffmpeg is a GPL build ([BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds));
its licence is included in the `ffmpeg` folder of the release.
