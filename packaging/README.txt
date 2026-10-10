Combo Editor
============

Turns two recordings of a fighting-game combo (one with the game UI, one
without) into a 1080x1920 TikTok video.

Run ComboEditor.exe. Nothing is installed: everything stays in this folder.

  ComboEditor.exe      the editor (a native Windows program)
  ffmpeg\              ffmpeg, used to read and write video (GPL, see LICENSE.txt)
  presets\             your UI skin and blend presets (created on first save)
  workspace\           projects, exports (workspace\exports) and analysis caches
  combo-editor.log     problems are written here

To move or back up the app, copy the whole folder. If the folder is read-only
(for example inside Program Files) your data goes to %LOCALAPPDATA%\ComboEditor.

Shortcuts: Space play/pause, Left/Right one frame (Shift = 1 second),
I / O set clip start / end, K add a camera keyframe, F3 performance overlay.

Command line (automation):
  ComboEditor.exe export project.json [-o out.mp4]    render without opening the editor
  ComboEditor.exe still project.json 12.5 frame.png   render one frame exactly as exported
  ComboEditor.exe detect project.json --write         find the Super Art and save the markers
  ComboEditor.exe sync with_ui.mp4 clean.mp4          print the sync offset
  ComboEditor.exe --opengl                            use OpenGL instead of DirectX 12

Windows may show a SmartScreen warning the first time because the program is
not code-signed: choose "More info" then "Run anyway".
