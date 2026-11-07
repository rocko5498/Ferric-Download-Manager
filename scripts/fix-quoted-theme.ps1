# fix-quoted-theme.ps1 — Fix "Theme.X" (quoted) → Theme.X (property reference)
# The migration script replaced hex inside quotes, creating invalid QML.

param([switch]$Apply)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$qmlDir = Join-Path $repo "crates\ferric-qt\qml"

$themeProps = @("accent", "accentDark", "bg", "surface", "surfaceAlt", "border", "text", "textDim", "danger", "success", "warning")

$qmlFiles = Get-ChildItem -Path $qmlDir -Filter "*.qml" -Recurse
$totalReplacements = 0
$filesChanged = 0

foreach ($file in $qmlFiles) {
    if ($file.Name -eq "Theme.qml") { continue }

    $content = Get-Content $file.FullName -Raw
    $original = $content
    $fileReplacements = 0

    foreach ($prop in $themeProps) {
        $quoted = "`"Theme.$prop`""
        $unquoted = "Theme.$prop"
        $count = ([regex]::Matches($content, [regex]::Escape($quoted))).Count
        if ($count -gt 0) {
            $content = $content.Replace($quoted, $unquoted)
            $fileReplacements += $count
        }
    }

    if ($fileReplacements -gt 0) {
        $totalReplacements += $fileReplacements
        $filesChanged++
        if ($Apply) {
            Set-Content -Path $file.FullName -Value $content -NoNewline
            Write-Host "  [apply] $($file.Name): $fileReplacements fixes" -ForegroundColor Green
        } else {
            Write-Host "  [dry-run] $($file.Name): $fileReplacements fixes" -ForegroundColor Yellow
        }
    }
}

Write-Host ""
if ($Apply) {
    Write-Host "APPLIED: $totalReplacements fixes across $filesChanged files" -ForegroundColor Green
} else {
    Write-Host "DRY RUN: $totalReplacements fixes across $filesChanged files" -ForegroundColor Yellow
    Write-Host "Run with -Apply to execute." -ForegroundColor Gray
}
