# migrate-theme.ps1 — Replace hardcoded hex colors with Theme tokens in QML files
#
# Run from repo root: powershell -ExecutionPolicy Bypass -File scripts\migrate-theme.ps1
# Dry-run by default. Pass -Apply to actually edit files.

param([switch]$Apply)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$qmlDir = Join-Path $repo "crates\ferric-qt\qml"

# Mapping: hex → Theme property (case-insensitive, exact matches only)
$map = [ordered]@{
    "#2a2a35" = "Theme.surfaceAlt"
    "#b45309" = "Theme.accent"
    "#e0e0e0" = "Theme.text"
    "#a1a1aa" = "Theme.textDim"
    "#0d0d16" = "Theme.bg"
    "#0c0c0d" = "Theme.bg"
    "#e8e8ea" = "Theme.text"
    "#f87171" = "Theme.danger"
    "#6b6b73" = "Theme.textDim"
    "#34d399" = "Theme.success"
    "#3a3a4a" = "Theme.border"
    "#1e1e2e" = "Theme.surface"
    "#CC0000" = "Theme.danger"
    "#1a1a24" = "Theme.surface"
    "#2a2a3a" = "Theme.surfaceAlt"
    "#131316" = "Theme.bg"
    "#fbbf24" = "Theme.warning"
    "#52525b" = "Theme.textDim"
    "#1a1a1e" = "Theme.surface"
    "#2a2a40" = "Theme.surfaceAlt"
    "#fca5a5" = "Theme.danger"
    "#fb923c" = "Theme.warning"
    "#4b4b53" = "Theme.textDim"
    "#22c55e" = "Theme.success"
    "#0f0f12" = "Theme.bg"
    "#131320" = "Theme.bg"
    "#e0a020" = "Theme.accent"
    "#0a0a0c" = "Theme.bg"
    "#1a1a22" = "Theme.surface"
    "#14141a" = "Theme.bg"
    "#d97706" = "Theme.warning"
    "#9ca3af" = "Theme.textDim"
    "#4ade80" = "Theme.success"
    "#6ee7b7" = "Theme.success"
    "#a3e635" = "Theme.success"
    "#7f1d1d" = "Theme.danger"
    "#3a1a1a" = "Theme.danger"
    "#1a3a1a" = "Theme.success"
    "#1a2a1a" = "Theme.surface"
    "#3b82f6" = "Theme.accent"
    "#60a5fa" = "Theme.accent"
}

$qmlFiles = Get-ChildItem -Path $qmlDir -Filter "*.qml" -Recurse
$totalReplacements = 0
$filesChanged = 0

foreach ($file in $qmlFiles) {
    # Skip Theme.qml itself
    if ($file.Name -eq "Theme.qml") { continue }

    $content = Get-Content $file.FullName -Raw
    $original = $content
    $fileReplacements = 0

    foreach ($hex in $map.Keys) {
        $prop = $map[$hex]
        # Case-insensitive replace, but only exact 7-char hex (#RRGGBB)
        # Use word-boundary-ish: hex must not be followed by hex char
        $pattern = "(?i)" + [regex]::Escape($hex) + "(?![0-9a-f])"
        $matches = [regex]::Matches($content, $pattern)
        if ($matches.Count -gt 0) {
            $content = [regex]::Replace($content, $pattern, $prop)
            $fileReplacements += $matches.Count
        }
    }

    if ($fileReplacements -gt 0) {
        $totalReplacements += $fileReplacements
        $filesChanged++
        if ($Apply) {
            Set-Content -Path $file.FullName -Value $content -NoNewline
            Write-Host "  [apply] $($file.Name): $fileReplacements replacements" -ForegroundColor Green
        } else {
            Write-Host "  [dry-run] $($file.Name): $fileReplacements replacements" -ForegroundColor Yellow
        }
    }
}

Write-Host ""
if ($Apply) {
    Write-Host "APPLIED: $totalReplacements replacements across $filesChanged files" -ForegroundColor Green
} else {
    Write-Host "DRY RUN: $totalReplacements replacements across $filesChanged files" -ForegroundColor Yellow
    Write-Host "Run with -Apply to execute." -ForegroundColor Gray
}
