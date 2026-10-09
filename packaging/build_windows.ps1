# Builds the portable Windows app into dist\ComboEditor and dist\ComboEditor-win64.zip.
# Usage (PowerShell, from anywhere):  .\packaging\build_windows.ps1
param(
    # Static GPL build (includes libx264) from https://github.com/BtbN/FFmpeg-Builds
    [string]$FfmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root

python -m pip install --upgrade pip
python -m pip install -r requirements.txt -r packaging/requirements-build.txt
python -m PyInstaller packaging/combo_editor.spec --noconfirm --clean --distpath dist --workpath build

# ffmpeg / ffprobe go in a visible "ffmpeg" folder next to the .exe so they can be swapped.
$ffCache = Join-Path $root "build\ffmpeg-download"
if (-not (Test-Path (Join-Path $ffCache "bin\ffmpeg.exe"))) {
    New-Item -ItemType Directory -Force $ffCache | Out-Null
    $zip = Join-Path $root "build\ffmpeg.zip"
    Write-Host "Downloading ffmpeg from $FfmpegUrl"
    Invoke-WebRequest -Uri $FfmpegUrl -OutFile $zip
    $tmp = Join-Path $root "build\ffmpeg-extract"
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    Expand-Archive $zip -DestinationPath $tmp
    $inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
    Copy-Item -Recurse -Force (Join-Path $inner.FullName "*") $ffCache
}
$app = Join-Path $root "dist\ComboEditor"
$ffDest = Join-Path $app "ffmpeg"
New-Item -ItemType Directory -Force $ffDest | Out-Null
Copy-Item (Join-Path $ffCache "bin\ffmpeg.exe"), (Join-Path $ffCache "bin\ffprobe.exe") $ffDest
if (Test-Path (Join-Path $ffCache "LICENSE.txt")) { Copy-Item (Join-Path $ffCache "LICENSE.txt") (Join-Path $ffDest "LICENSE.txt") }
Copy-Item (Join-Path $root "packaging\README-portable.txt") (Join-Path $app "README.txt")

$out = Join-Path $root "dist\ComboEditor-win64.zip"
if (Test-Path $out) { Remove-Item $out }
Compress-Archive -Path $app -DestinationPath $out
Write-Host "Built $app and $out"
