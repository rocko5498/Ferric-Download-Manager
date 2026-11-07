# Packs extension\build\chrome-mv3-prod into extension\build\chrome-mv3-prod.crx, signed with the Ferric
# extension key (its public half is the "key" in extension\package.json, so the id is always the same).
# Usage: .\scripts\pack-extension.ps1 -Key <path to the private .pem>  [-Browser <brave.exe|chrome.exe|msedge.exe>]
param(
    [Parameter(Mandatory = $true)][string]$Key,
    [string]$Browser
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$dir = Join-Path $root "extension\build\chrome-mv3-prod"
if (-not (Test-Path (Join-Path $dir "manifest.json"))) { throw "build the extension first (npm run build in extension\)" }
if (-not $Browser) {
    $Browser = @(
        "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
        "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\Application\brave.exe",
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Browser) { throw "no Chromium browser found; pass -Browser" }
$crx = "$dir.crx"
Remove-Item $crx -Force -ErrorAction SilentlyContinue
# A separate profile so a running browser does not swallow the command.
$profile = Join-Path $env:TEMP "ferric-pack-profile"
Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
& $Browser "--user-data-dir=$profile" --headless=new --no-first-run "--pack-extension=$dir" "--pack-extension-key=$Key"
$deadline = (Get-Date).AddSeconds(30)
while (-not (Test-Path $crx) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
if (-not (Test-Path $crx)) { throw "the browser did not produce $crx" }
Write-Host "Packed $crx"
