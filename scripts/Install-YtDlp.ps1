# Install official yt-dlp release for Ferric (Windows).
# Does NOT vendor yt-dlp into the repo — downloads the GitHub release binary.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\Install-YtDlp.ps1 [-DestDir path]

param(
    [string]$DestDir = "$env:LOCALAPPDATA\Ferric\bin"
)

$ErrorActionPreference = "Stop"
$asset = "yt-dlp.exe"
$url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$asset"

New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
$out = Join-Path $DestDir $asset

Write-Host "Downloading official yt-dlp → $out"
Invoke-WebRequest -Uri $url -OutFile $out -UseBasicParsing

Write-Host "OK: $out"
Write-Host "Point Ferric Settings → Media → yt-dlp path to this file,"
Write-Host "or set user env FERRIC_YTDLP=$out and restart ferricd."
Write-Host "Also add $DestDir to PATH if you want bare 'yt-dlp' discovery."
