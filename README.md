# Combo Editor: automatic TikTok edits for fighting-game combos

This tool turns two 1920×1080 recordings of the same combo into a 1080×1920 TikTok video:

- one recording **with UI** (health bars, damage readout)
- one recording **without UI** (the clean gameplay)

The clean gameplay is cropped to vertical with a camera that follows both fighters. Selected
UI elements (by default the enemy health bar and the combo damage box) are cut out of the UI
recording and laid on top with feathered edges and adjustable opacity. You can add background
music, and the result exports as a single H.264/AAC file.

## Windows: portable app (recommended)

Download `ComboEditor-win64` from the latest
[Windows portable build](../../actions/workflows/windows-build.yml) run (or from a release),
unzip it anywhere (even a USB stick) and run **`ComboEditor.exe`**. There is no installer and
nothing else to download: Python and ffmpeg are bundled, and the editor opens in its own
window with native **Browse…** buttons. Your presets, projects and exports stay inside that
folder, so you can move or back it up by copying the folder. `README.txt` in the folder has
the details.

To build it yourself on Windows (Python 3.10+ needed only for building):

```powershell
.\packaging\build_windows.ps1     # -> dist\ComboEditor\ and dist\ComboEditor-win64.zip
python packaging\smoke_test.py dist\ComboEditor
```

## Running from source

You need Python 3.10+ and **ffmpeg** (with `ffprobe`) on your `PATH`, or in an `ffmpeg`
folder in the repo.

```bash
pip install -r requirements.txt
python -m editor             # native window on Windows; elsewhere opens your browser
python -m editor --browser   # always use the browser (Chrome/Edge play H.264 MP4s)
```

To run the tests: `pip install -r requirements-dev.txt && pytest`.

## Workflow

### 1. Sources & sync
1. Paste the file paths of both recordings, or use **Upload**, then click **Load videos**.
2. Click **Auto-sync (transients)**. Neither recording has music, so their game audio is
   identical. The editor matches the sharp transients (hits, whiffs, announcer) with
   spectral flux cross-correlation, then refines the result to the exact sample with GCC-PHAT.
3. Check the result with the **Difference** source view. When the recordings are in sync,
   everything except the UI turns black. Fine-tune with ±1 frame / ±10 ms, and use **Snap**
   to round to a whole frame.
4. Choose which recording supplies the game audio (the *primary volume*).
5. Mark the clip's **In** and **Out** points under the timeline (keys `I` / `O`).

### 2. Camera
Click **Analyse fighters** once per clip. Then set the part markers under the timeline:

| Part | Markers | Camera |
|---|---|---|
| Combo | In → Super start | Automatic: always centred between both fighters, at native scale (full-height 9:16 crop, no zoom) |
| Super startup | Super start → Animation start | Manual keyframes |
| Super animation | Animation start → Post start | Manual keyframes |
| Post super | Post start → Out | Automatic centring, plus your X/Y offset, zoom and smoothing |

In the super, move the playhead and **drag the preview** to pan or **scroll** to zoom. Each
change creates or updates a keyframe at the playhead (`K` adds one). The super camera starts
exactly where the combo camera stopped, and eases between keyframes. Keyframes can be dragged
along the timeline. **Transitions** set how long the camera takes to blend between parts.

The source view shows the crop window and the tracked edges of the action (cyan dashed lines).
If tracking struggles on a stage, change the Motion / Colour weights. Colour works very well
on the grey training stage. Motion compensates for camera pans and follows the fighters'
animation.

### 3. UI overlay
- Each element has a **source** rectangle (in the 1920×1080 UI recording) and a **placement**
  rectangle (in the 1080×1920 output). Drag them on the UI view and on the preview, or type
  exact values.
- **Blend**: opacity and a per-edge feather (in output pixels). Use **Save blend as…** to keep
  a blend preset and apply it to any element.
- **Visible during**: choose which parts show the element (for example, hide the UI during the
  super animation). Elements fade over the *show/hide fade* time.
- **UI skin presets** store the whole layout plus each element's blend. Save one per in-game
  UI skin. The built-in *Default* matches the example layout.

### 4. Music & export
Add a song, set where in the song to start, and set its volume, fade-in and fade-out (plus an
optional fade-out on the game audio). **Exact frame** renders the current frame through the
real export pipeline. **Export** writes `workspace/exports/<project>.mp4`, or the path you
enter. The default settings are x264 CRF 16, preset `slow`, Lanczos upscaling and 256k AAC.
Bicubic upscaling is about 4× faster for drafts.

Use the header buttons to save projects. Presets live in `presets/` and projects in
`workspace/projects/` (next to `ComboEditor.exe` in the portable app).

## How it works

```
editor/
  sync.py        transient-based audio alignment (onset envelope xcorr + GCC-PHAT refine)
  tracking.py    per-frame fighter extents: camera-shift-compensated motion + saturation
  camera.py      markers + tracking + keyframes -> per-frame camera path & overlay alphas
  compositor.py  sub-pixel crop/upscale (warpAffine) + feathered UI element blending
  render.py      frame-accurate decode of both videos -> compose -> ffmpeg (x264 + audio mix)
  server.py      FastAPI backend for the browser editor
  desktop.py     native window (pywebview / Edge WebView2) + file dialogs
  tools.py       finds the bundled or system ffmpeg
web/             the editor UI (no build step)
packaging/       PyInstaller spec, Windows build script, packaged-app smoke test
```

The camera path is computed in a single place (`camera.py`). The browser preview samples the
same per-frame arrays the exporter uses, so what you see is what you get. The only
approximation is browser canvas scaling compared with Lanczos.
