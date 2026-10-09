Combo Editor (portable)
=======================

Run ComboEditor.exe. Nothing is installed: everything stays in this folder.

  presets\      your UI skin and blend presets
  workspace\    saved projects, exports (workspace\exports) and analysis caches
  ffmpeg\       the bundled ffmpeg used for decoding and export (GPL, see LICENSE.txt)
  combo-editor.log   error log (useful if something goes wrong)

To move or back up the app, copy the whole folder. If the folder is read-only
(for example inside Program Files) your data goes to %LOCALAPPDATA%\ComboEditor.

The window uses Microsoft Edge WebView2, which comes with Windows 10 and 11. If it
is missing, the editor opens in your default browser instead. You can get it from
https://developer.microsoft.com/microsoft-edge/webview2/

Options (from a command prompt):
  ComboEditor.exe --browser        open in the default browser instead of a window
  ComboEditor.exe --port 9000      use a different local port
