# FQT-110: Windows release packaging script
# Packages ferric-qt + Qt DLLs + ferricd into a release directory
# Usage: .\scripts\package-windows.ps1 [-Release] [-OutputDir <path>]

param(
    [switch]$Release,
    [string]$OutputDir = "release"
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { "release" } else { "debug" }
$qtBin = $env:QMAKE -replace '[/\\]qmake\.exe$', ''
# Where Qt is: QMAKE, else whatever is on PATH (CI), else the install root a CI action exports, else the usual local folder.
if (-not $qtBin -and (Get-Command qmake -ErrorAction SilentlyContinue)) { $qtBin = Split-Path (Get-Command qmake).Source }
if (-not $qtBin -and $env:QT_ROOT_DIR) { $qtBin = Join-Path $env:QT_ROOT_DIR "bin" }
if (-not $qtBin) { $qtBin = "C:\Qt\6.8.3\msvc2022_64\bin" }

Write-Host "=== Ferric Release Packaging ===" -ForegroundColor Cyan
Write-Host "Profile: $profile"
Write-Host "Qt bin: $qtBin"

# Build
Write-Host "`nBuilding..." -ForegroundColor Yellow
$env:QMAKE = "$qtBin\qmake.exe"
# The native host only answers extensions it knows: the packed extension (signed with the Ferric
# key) and the id of earlier unpacked builds. Set FERRIC_EXTENSION_ID yourself to override.
if (-not $env:FERRIC_EXTENSION_ID) { $env:FERRIC_EXTENSION_ID = "gepkkfjfgdcgelcnejpikdeanggknbjo,bngipfinnjpcnebfchfbeinfppepibkb" }
# cargo writes progress to stderr; Windows PowerShell 5.1 turns that into an error under "Stop".
$ErrorActionPreference = "Continue"
if ($Release) {
    cargo build --release -p ferric-qt -p ferricd -p ferric-host
} else {
    cargo build -p ferric-qt -p ferricd -p ferric-host
}
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
$ErrorActionPreference = "Stop"

# Create output directory
$outPath = Join-Path $root $outputDir
if (Test-Path $outPath) { Remove-Item $outPath -Recurse -Force }
New-Item -ItemType Directory -Path $outPath -Force | Out-Null

# Copy binaries
$targetDir = Join-Path $root "target\$profile"
Copy-Item "$targetDir\ferric-qt.exe" $outPath
Copy-Item "$targetDir\ferricd.exe" $outPath
Copy-Item "$targetDir\ferric-host.exe" $outPath

# Run windeployqt to copy Qt DLLs
Write-Host "`nRunning windeployqt..." -ForegroundColor Yellow
$windeployqt = "$qtBin\windeployqt.exe"
if (Test-Path $windeployqt) {
    $ErrorActionPreference = "Continue" # windeployqt prints warnings on stderr
    & $windeployqt "$outPath\ferric-qt.exe" --qmldir "$root\crates\ferric-qt\qml" --no-translations
    if ($LASTEXITCODE -ne 0) { throw "windeployqt failed with exit code $LASTEXITCODE" }
    $ErrorActionPreference = "Stop"
} else {
    Write-Host "WARNING: windeployqt not found at $windeployqt" -ForegroundColor Red
    Write-Host "Qt DLLs must be added manually or install Qt 6.8.3" -ForegroundColor Red
}

# Copy QML files
New-Item -ItemType Directory -Path "$outPath\qml" -Force | Out-Null
Copy-Item "$root\crates\ferric-qt\qml\*.qml" "$outPath\qml\"

# Copy README
if (Test-Path "$root\README.md") {
    Copy-Item "$root\README.md" "$outPath\"
}

# Create launch script
@"
@echo off
start "" "%~dp0ferric-qt.exe"
"@ | Set-Content "$outPath\start-ferric.bat"

# Installer scripts (per-user install, uninstall, double-click wrapper)
Copy-Item "$root\scripts\installer\*" $outPath

# The Chrome/Edge/Brave extension, if it has been built (npm run build in extension\)
$extBuild = Join-Path $root "extension\build\chrome-mv3-prod"
if (Test-Path $extBuild) {
    New-Item -ItemType Directory -Path "$outPath\extension" -Force | Out-Null
    Copy-Item "$extBuild\*" "$outPath\extension\" -Recurse
    # Ferric.crx (made by scripts\pack-extension.ps1) is what the installer offers to the browsers
    # through the external-extension registry key, the way IDM does.
    # One package per browser family: each names its own store's update address (see pack-extension.ps1).
    $crxFiles = @{ "chrome-mv3-prod.crx" = "Ferric.crx"; "chrome-mv3-prod-edge.crx" = "Ferric-edge.crx" }
    $anyCrx = $false
    foreach ($name in $crxFiles.Keys) {
        $crx = Join-Path $root "extension\build\$name"
        if (Test-Path $crx) { Copy-Item $crx "$outPath\extension\$($crxFiles[$name])"; $anyCrx = $true }
    }
    if (-not $anyCrx) { Write-Host "NOTE: no packed extension (run scripts\pack-extension.ps1); browsers will not be offered the extension." -ForegroundColor Yellow }
} else {
    Write-Host "NOTE: extension not built (run 'npm run build' in extension\); the package has no extension folder." -ForegroundColor Yellow
}

# One zip that contains everything: unzip, then run Install-Ferric.cmd
$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Path $dist -Force | Out-Null
$zip = Join-Path $dist "Ferric-win64.zip"
Remove-Item $zip -Force -ErrorAction SilentlyContinue
Compress-Archive -Path "$outPath\*" -DestinationPath $zip -CompressionLevel Optimal

# One-click installer (Ferric-Setup-<version>.exe), built with Inno Setup when it is installed
# (winget install JRSoftware.InnoSetup). Without it the zip above still works.
$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    $ver = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    Write-Host "`nBuilding the setup program ($ver)..." -ForegroundColor Yellow
    & $iscc "/DFerricVersion=$ver" "$root\scripts\installer\Ferric.iss"
    if ($LASTEXITCODE -ne 0) { Write-Host "WARNING: the setup program could not be built." -ForegroundColor Red }
} else {
    Write-Host "NOTE: Inno Setup not found; only the zip was made (winget install JRSoftware.InnoSetup)." -ForegroundColor Yellow
}

# Summary
$files = Get-ChildItem $outPath -Recurse -File
$totalSize = ($files | Measure-Object -Property Length -Sum).Sum / 1MB
Write-Host "`n=== Package Complete ===" -ForegroundColor Green
Write-Host "Output: $outPath"
Write-Host "Files: $($files.Count)"
Write-Host "Size: $([math]::Round($totalSize, 1)) MB"
