# Assembles dist\ComboEditor\ and dist\ComboEditor-<version>-win64.zip from a release build.
# Run after `cargo build --release` and packaging/get-ffmpeg.ps1.
param([string]$Ffmpeg = "ffmpeg")
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
$app = Join-Path $root "dist\ComboEditor"
if (Test-Path $app) { Remove-Item -Recurse -Force $app }
New-Item -ItemType Directory -Force (Join-Path $app "ffmpeg") | Out-Null
Copy-Item "target\release\combo-editor.exe" (Join-Path $app "ComboEditor.exe")
Copy-Item (Join-Path $Ffmpeg "ffmpeg.exe"), (Join-Path $Ffmpeg "ffprobe.exe"), (Join-Path $Ffmpeg "LICENSE.txt") (Join-Path $app "ffmpeg")
Copy-Item "packaging\README.txt" (Join-Path $app "README.txt")
$zip = Join-Path $root "dist\ComboEditor-$version-win64.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path $app -DestinationPath $zip
Write-Host "Packaged $app and $zip"
