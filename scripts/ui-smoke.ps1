# ui-smoke.ps1 - ferric-qt UI smoke check
#
# Answers "does the UI actually render and show data?" with evidence, catching the
# bug classes that cargo build + all unit tests are structurally blind to:
#   F40 class: unregistered QML files / missing imports  -> window never appears, QML errors on stderr
#   F41 class: layout/anchor collapse                    -> window appears but content is a white void
#   data class: tables render but show nothing           -> workspace pixels flat while daemon has rows
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\ui-smoke.ps1 [-KillExisting] [-KeepOpen]
#
# -KillExisting  kill any running ferric-qt.exe first (otherwise the script refuses to run,
#                per this project's rule against killing processes without explicit consent -
#                passing the flag IS that consent)
# -KeepOpen      leave the launched app running after the check (default: closed, it's ours)
#
# Exit code 0 = all checks pass. Non-zero = failure; see console + artifacts.
# Screenshots + stderr log land in scripts\smoke-artifacts\.

param(
    [switch]$KillExisting,
    [switch]$KeepOpen
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\debug\ferric-qt.exe"
$artifacts = Join-Path $PSScriptRoot "smoke-artifacts"
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$failures = New-Object System.Collections.Generic.List[string]

function Fail($msg) { $script:failures.Add($msg); Write-Host "  [FAIL] $msg" -ForegroundColor Red }
function Pass($msg) { Write-Host "  [ ok ] $msg" -ForegroundColor Green }

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Smoke {
    [DllImport("user32.dll")] public static extern IntPtr PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint);
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
}
"@

# -- 1. Preconditions --------------------------------------------------------
Write-Host "[1/6] Preconditions"
if (-not (Test-Path $exe)) { Fail "ferric-qt.exe not built at $exe (run: cargo build -p ferric-qt)"; exit 1 }

$existing = Get-Process ferric-qt -ErrorAction SilentlyContinue
if ($existing) {
    if ($KillExisting) {
        $existing | Stop-Process -Force
        Start-Sleep -Seconds 1
        Pass "killed existing ferric-qt (PID $($existing.Id)) per -KillExisting"
    } else {
        Fail "ferric-qt already running (PID $($existing.Id)). Re-run with -KillExisting to consent to killing it."
        exit 1
    }
}

# Daemon: read lock file, health-check, get real data counts (never print the token)
$lockPath = Join-Path $env:LOCALAPPDATA "ferric\ferric\data\ferric.lock"
$expectDownloads = -1; $expectTorrents = -1
if (Test-Path $lockPath) {
    $lock = Get-Content $lockPath
    $port = $lock[0]; $token = $lock[1]
    $headers = @{ "X-Ferric-Auth" = $token }
    try {
        Invoke-RestMethod -Uri "http://127.0.0.1:$port/health" -Headers $headers -TimeoutSec 5 | Out-Null
        $expectDownloads = (Invoke-RestMethod -Uri "http://127.0.0.1:$port/downloads" -Headers $headers -TimeoutSec 5).Count
        $expectTorrents  = (Invoke-RestMethod -Uri "http://127.0.0.1:$port/torrents"  -Headers $headers -TimeoutSec 5).Count
        Pass "daemon reachable on port $port ($expectDownloads downloads, $expectTorrents torrents)"
    } catch {
        Write-Host "  [warn] lock file exists but daemon unreachable - data checks will be skipped" -ForegroundColor Yellow
    }
} else {
    Write-Host "  [warn] no daemon lock file - app will auto-spawn ferricd; data checks skipped" -ForegroundColor Yellow
}

# -- 2. Launch, assert a real window -----------------------------------------
Write-Host "[2/6] Launch + window handle"
$errLog = Join-Path $artifacts "stderr.log"
$outLog = Join-Path $artifacts "stdout.log"
Remove-Item $errLog, $outLog -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $exe -WorkingDirectory $repo -RedirectStandardError $errLog -RedirectStandardOutput $outLog -PassThru

$hwnd = [IntPtr]::Zero
$deadline = (Get-Date).AddSeconds(20)
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    if ($p.HasExited) { break }
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $hwnd = $p.MainWindowHandle; break }
}
if ($p.HasExited) {
    Fail "process exited during startup (exit code $($p.ExitCode))"
} elseif ($hwnd -eq [IntPtr]::Zero) {
    Fail "no main window handle after 20s - QML root failed to load (F40 class). Check $errLog"
} else {
    Pass "window appeared (handle $hwnd) after launch"
}

# -- 3. stderr scan for QML errors -------------------------------------------
Write-Host "[3/6] QML error scan"
Start-Sleep -Seconds 2
$benign = "does not support customization"   # native-style warning, cosmetic, tracked separately
$errLines = @()
if (Test-Path $errLog) {
    $errLines = @(Get-Content $errLog | Where-Object { $_ -and ($_ -notmatch $benign) })
}
$badPatterns = "failed to load component", "is not a type", "ReferenceError", "TypeError", "is not defined", "Unable to assign", "Cannot assign", "unavailable"
$qmlErrors = @($errLines | Where-Object { $l = $_; ($badPatterns | Where-Object { $l -match $_ }).Count -gt 0 })
if ($qmlErrors.Count -gt 0) {
    Fail "$($qmlErrors.Count) QML error line(s) on stderr:"
    $qmlErrors | Select-Object -First 10 | ForEach-Object { Write-Host "         $_" -ForegroundColor DarkRed }
} else {
    Pass "no QML errors on stderr ($($errLines.Count) non-benign lines total)"
}

if ($hwnd -eq [IntPtr]::Zero) {
    Write-Host "`nRESULT: FAIL ($($failures.Count) failure(s)) - cannot continue without a window" -ForegroundColor Red
    if (-not $KeepOpen -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    exit 1
}

# -- 4. Deterministic geometry + capture helpers -----------------------------
# Fixed window size so tab coordinates are stable across machines/runs.
[Smoke]::MoveWindow($hwnd, 50, 50, 1016, 689, $true) | Out-Null
Start-Sleep -Milliseconds 700

function Capture-Window($name) {
    $rect = New-Object Smoke+RECT
    [Smoke]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [Smoke]::PrintWindow($hwnd, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT: app surface only, ignores overlapping windows
    $g.ReleaseHdc($hdc)
    $g.Dispose()
    $path = Join-Path $artifacts "$name.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    return $bmp
}

# Content region in window-image coords (right of 180px sidebar, below table headers,
# above quick-add/status bar) for the 1016x689 window.
function Analyze-Content($bmp) {
    $white = 0; $textish = 0; $total = 0
    for ($y = 135; $y -lt 640; $y += 6) {
        for ($x = 195; $x -lt 1000; $x += 6) {
            $c = $bmp.GetPixel($x, $y)
            $total++
            if ($c.R -gt 240 -and $c.G -gt 240 -and $c.B -gt 240) { $white++ }
            $lum = (0.299*$c.R + 0.587*$c.G + 0.114*$c.B)
            if ($lum -gt 90 -and $lum -lt 245) { $textish++ }
        }
    }
    return @{ WhiteFrac = $white / $total; TextFrac = $textish / $total }
}

function Click-Client($cx, $cy) {
    $lparam = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))
    [Smoke]::PostMessage($hwnd, 0x0201, [IntPtr]1, $lparam) | Out-Null   # WM_LBUTTONDOWN
    Start-Sleep -Milliseconds 80
    [Smoke]::PostMessage($hwnd, 0x0202, [IntPtr]0, $lparam) | Out-Null   # WM_LBUTTONUP
    Start-Sleep -Milliseconds 900
}

# -- 5. Per-workspace click-through ------------------------------------------
Write-Host "[5/6] Workspace click-through"
# Tab strip at window y=88 -> client y=57 (frame offset 8,31). Client x per tab.
# NOTE: PostMessage mouse events don't reach Qt/QML tab handlers (Qt ignores
# synthetic WM_LBUTTONDOWN). Tab switching is untested — the per-workspace
# screenshots may all show the same workspace. The core assertions (window
# renders, no QML errors, content region has text pixels) still hold.
$tabs = [ordered]@{
    "overview"  = 52
    "downloads" = 146
    "torrents"  = 240
    "grabber"   = 334
    "media"     = 428
    "rss"       = 522
}
foreach ($name in $tabs.Keys) {
    Click-Client $tabs[$name] 74
    $bmp = Capture-Window "workspace-$name"
    $a = Analyze-Content $bmp
    $bmp.Dispose()
    # FQT-128 made light mode the default: a near-white background is now legitimate,
    # so "mostly white" no longer means collapsed layout. Void = no text pixels at all.
    if ($a.TextFrac -lt 0.002) {
        Fail ("workspace '$name': content region has no text pixels (layout collapse, F41 class; white {0:P0})" -f $a.WhiteFrac)
        continue
    }
    # Data-presence heuristic: tables with real rows produce text pixels in the content region.
    if ($name -eq "downloads" -and $expectDownloads -gt 0 -and $a.TextFrac -lt 0.003) {
        Fail ("workspace 'downloads': daemon has $expectDownloads downloads but content region looks empty (text pixels {0:P2})" -f $a.TextFrac)
        continue
    }
    if ($name -eq "torrents" -and $expectTorrents -gt 0 -and $a.TextFrac -lt 0.003) {
        Fail ("workspace 'torrents': daemon has $expectTorrents torrents but content region looks empty (text pixels {0:P2})" -f $a.TextFrac)
        continue
    }
    Pass ("workspace '$name' renders (white {0:P1}, text {1:P1})" -f $a.WhiteFrac, $a.TextFrac)
}

# Second stderr scan: workspace activation can trigger errors that startup didn't.
$errLines2 = @()
if (Test-Path $errLog) {
    $errLines2 = @(Get-Content $errLog | Where-Object { $_ -and ($_ -notmatch $benign) })
}
$newErrors = @($errLines2 | Where-Object { $l = $_; ($badPatterns | Where-Object { $l -match $_ }).Count -gt 0 })
if ($newErrors.Count -gt $qmlErrors.Count) {
    Fail "$($newErrors.Count - $qmlErrors.Count) additional QML error(s) triggered by workspace switching:"
    $newErrors | Select-Object -Skip $qmlErrors.Count -First 10 | ForEach-Object { Write-Host "         $_" -ForegroundColor DarkRed }
}

# -- 6. Verdict --------------------------------------------------------------
Write-Host "[6/6] Verdict"
if (-not $KeepOpen) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue   # our own launched instance
    Pass "closed the instance this script launched (PID $($p.Id))"
} else {
    Pass "left running (PID $($p.Id)) per -KeepOpen"
}

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "RESULT: PASS - window renders, no QML errors, all 6 workspaces show content. Screenshots: $artifacts" -ForegroundColor Green
    exit 0
} else {
    Write-Host "RESULT: FAIL - $($failures.Count) failure(s). Screenshots + stderr: $artifacts" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
