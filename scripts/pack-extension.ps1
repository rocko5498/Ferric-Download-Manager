# Packs the built extension into a .crx signed with the Ferric extension key (its public half is the "key" in
# extension\package.json, so the id is always the same).
# Usage: .\scripts\pack-extension.ps1 -Key <path to the private .pem> [-Out <file.crx>] [-UpdateUrl <address>] [-Browser <exe>]
#
# -UpdateUrl is what makes a browser offer the extension to the user: with the store's update address in the
# manifest, a package registered through the external-extension registry key is installed and the user is asked
# to enable it (measured on Chrome and Edge with scripts\ci\extension-offer-test.ps1, disable reason 8192);
# without it the browser installs the package but never lets the user switch it on (reason 256).
#   Chrome and Brave: https://clients2.google.com/service/update2/crx
#   Edge:             https://edge.microsoft.com/extensionwebstorebase/v1/crx
param(
    [Parameter(Mandatory = $true)][string]$Key,
    [string]$Out,
    [string]$UpdateUrl,
    [string]$Browser
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$built = Join-Path $root "extension\build\chrome-mv3-prod"
if (-not (Test-Path (Join-Path $built "manifest.json"))) { throw "build the extension first (npm run build in extension\)" }
if (-not $Out) { $Out = "$built.crx" }
if (-not $Browser) {
    $Browser = @(
        "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
        "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\Application\brave.exe",
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Browser) { throw "no Chromium browser found; pass -Browser" }

# Pack a copy, so the build folder (also shipped unpacked) is never changed.
$work = Join-Path ([IO.Path]::GetTempPath()) ("ferric-pack-" + [Guid]::NewGuid().ToString("N").Substring(0, 8))
$dir = Join-Path $work "extension"
New-Item -ItemType Directory -Force $work | Out-Null
Copy-Item $built $dir -Recurse
if ($UpdateUrl) {
    $m = Get-Content "$dir\manifest.json" -Raw | ConvertFrom-Json
    $m | Add-Member -NotePropertyName update_url -NotePropertyValue $UpdateUrl -Force
    [IO.File]::WriteAllText("$dir\manifest.json", ($m | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding($false)))
}
try {
    # A separate profile so a running browser does not swallow the command.
    $profile = Join-Path $work "profile"
    & $Browser "--user-data-dir=$profile" --headless=new --no-first-run "--pack-extension=$dir" "--pack-extension-key=$Key" | Out-Null
    $crx = "$dir.crx"
    $deadline = (Get-Date).AddSeconds(40)
    while (-not (Test-Path $crx) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (-not (Test-Path $crx)) { throw "the browser did not produce $crx" }
    Copy-Item $crx $Out -Force
    Write-Host "Packed $Out"
}
finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
