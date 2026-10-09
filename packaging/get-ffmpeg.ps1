# Downloads a static GPL build of ffmpeg/ffprobe for Windows (includes libx264).
# Usage: ./packaging/get-ffmpeg.ps1 -Dest ffmpeg
param(
    [string]$Dest = "ffmpeg",
    [string]$Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
)
$ErrorActionPreference = "Stop"
if (Test-Path (Join-Path $Dest "ffmpeg.exe")) { Write-Host "ffmpeg already present in $Dest"; exit 0 }
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ffmpeg-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
$zip = Join-Path $tmp "ffmpeg.zip"
Write-Host "Downloading $Url"
Invoke-WebRequest -Uri $Url -OutFile $zip
Expand-Archive $zip -DestinationPath $tmp
$inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
New-Item -ItemType Directory -Force $Dest | Out-Null
Copy-Item (Join-Path $inner.FullName "bin\ffmpeg.exe"), (Join-Path $inner.FullName "bin\ffprobe.exe") $Dest
Copy-Item (Join-Path $inner.FullName "LICENSE.txt") (Join-Path $Dest "LICENSE.txt")
Remove-Item -Recurse -Force $tmp
Write-Host "ffmpeg ready in $Dest"
