param(
    [switch]$KeepOpen,
    [double]$Scale = 1.0,
    [switch]$DpiOnly
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\debug\ferric-qt.exe"
$art = Join-Path $PSScriptRoot "smoke-artifacts\fqt-132"
$shotPrefix = if ($Scale -eq 1.0) { "" } else { "dpi-$([math]::Round($Scale * 100))-" }
$env:QT_SCALE_FACTOR = [string]$Scale
$trace = Join-Path $art "dpi-trace.log"
if ($DpiOnly) { Set-Content $trace "start scale=$Scale" }
function Trace($message) { if ($DpiOnly) { Add-Content $trace $message } }
New-Item -ItemType Directory -Force $art | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Fqt132 {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr e);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out RECT r);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int hgt,bool repaint);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public struct RECT { public int Left,Top,Right,Bottom; }
}
"@

function WindowRect {
    $r = New-Object Fqt132+RECT
    [Fqt132]::GetWindowRect($script:hwnd,[ref]$r) | Out-Null
    $r
}
function Click($x,$y) {
    $r = WindowRect
    [Fqt132]::SetForegroundWindow($script:hwnd) | Out-Null
    Start-Sleep -Milliseconds 100
    $px = 8 + (($x - 8) * $Scale)
    $py = 31 + (($y - 31) * $Scale)
    [Fqt132]::SetCursorPos($r.Left+[int]$px,$r.Top+[int]$py) | Out-Null
    [Fqt132]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [Fqt132]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
}
function ResizeWindow($w,$h) {
    Trace "move-start ${w}x${h}"
    [Fqt132]::MoveWindow($script:hwnd,20,20,[int]($w*$Scale),[int]($h*$Scale),$true) | Out-Null
    Trace "move-end ${w}x${h}"
}
function Key($vk) {
    [Fqt132]::keybd_event($vk,0,0,[UIntPtr]::Zero)
    [Fqt132]::keybd_event($vk,0,2,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
}
function ToggleDark {
    [Fqt132]::SetForegroundWindow($script:hwnd) | Out-Null
    [Fqt132]::keybd_event(0x12,0,0,[UIntPtr]::Zero)
    Key 0x56
    [Fqt132]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    Key 0x44
    Start-Sleep -Milliseconds 700
}
function Shot($name) {
    $r = WindowRect
    $bmp = New-Object System.Drawing.Bitmap ($r.Right-$r.Left),($r.Bottom-$r.Top)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [Fqt132]::PrintWindow($script:hwnd,$dc,2) | Out-Null
    $g.ReleaseHdc($dc)
    $g.Dispose()
    $bmp.Save((Join-Path $art "$shotPrefix$name.png"),[System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

if (Get-Process ferric-qt -ErrorAction SilentlyContinue) {
    throw "ferric-qt already running; close it before certification"
}
$err = Join-Path $art "stderr.log"
Remove-Item $err -ErrorAction SilentlyContinue
$p = Start-Process $exe -WorkingDirectory $repo -RedirectStandardError $err -PassThru
Trace "process-started id=$($p.Id)"
$script:hwnd = [IntPtr]::Zero
$until = (Get-Date).AddSeconds(25)
while ((Get-Date) -lt $until) {
    Start-Sleep -Milliseconds 500
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $script:hwnd=$p.MainWindowHandle; break }
}
if ($script:hwnd -eq [IntPtr]::Zero) { throw "window did not appear" }
Trace "window-ready"
Trace "configure-sizes"

if ($DpiOnly) {
    $sizes = @()
    $sizes += ,@(700,480)
    $sizes += ,@(1016,689)
} else {
    $sizes = @(@(700,480),@(800,600),@(900,650),@(1016,689),@(1280,720),@(1600,900))
}
Trace "sizes-ready count=$($sizes.Count)"
$tabs = [ordered]@{ overview=60; downloads=150; torrents=250; grabber=350; media=440; rss=530 }
foreach ($s in $sizes) {
    Trace "light-resize-loop"
    ResizeWindow $s[0] $s[1]
    Start-Sleep -Milliseconds 700
    foreach ($tab in $tabs.GetEnumerator()) {
        Trace "light-click-$($tab.Key)"
        Click $tab.Value 136
        Shot "light-$($s[0])x$($s[1])-$($tab.Key)"
    }
}
ToggleDark
foreach ($s in $sizes) {
    Trace "dark-resize-loop"
    ResizeWindow $s[0] $s[1]
    Start-Sleep -Milliseconds 700
    foreach ($tab in $tabs.GetEnumerator()) {
        Trace "dark-click-$($tab.Key)"
        Click $tab.Value 136
        Shot "dark-$($s[0])x$($s[1])-$($tab.Key)"
    }
}

# Dialog/menu control pass at stable geometry, using real input.
if (-not $DpiOnly) {
ResizeWindow 1016 689
Start-Sleep -Milliseconds 700
Click 60 136
Click 44 90
Shot "control-add-url"
Key 0x1B
[Fqt132]::keybd_event(0x11,0,0,[UIntPtr]::Zero); Key 0x4F
[Fqt132]::keybd_event(0x11,0,2,[UIntPtr]::Zero); Start-Sleep -Milliseconds 500
Shot "control-add-torrent"
Key 0x1B
Click 28 46; Shot "control-menu-file"; Click 70 137
Shot "control-add-batch"; Key 0x1B
Click 284 46; Shot "control-menu-options"; Click 320 76
Shot "control-preferences"; Key 0x1B
Click 220 46; Shot "control-menu-tools"; Click 255 76
Shot "control-proxy"; Key 0x1B
Click 344 46; Shot "control-menu-help"; Click 375 76
Shot "control-about"; Key 0x1B
Click 165 46; Shot "control-menu-view"; Click 220 177
Shot "control-statistics"; Key 0x1B
Click 165 46; Click 230 207
Shot "control-shortcuts"; Key 0x1B
Click 806 136
Shot "control-columns"; Key 0x1B

# Read-only detail-panel pass against existing rows.
Click 150 136
Click 400 180
Shot "control-download-selected"
$downloadDetailTabs = [ordered]@{ general=230; chunks=320; audit=410; diagnostic=500 }
foreach ($tab in $downloadDetailTabs.GetEnumerator()) {
    Click $tab.Value 451
    Shot "control-download-detail-$($tab.Key)"
}
Click 250 136
Click 400 180
Shot "control-torrent-selected"
$torrentDetailTabs = [ordered]@{
    general=230; files=320; trackers=410; peers=500; pieces=595; http=695; speed=790
}
foreach ($tab in $torrentDetailTabs.GetEnumerator()) {
    Click $tab.Value 451
    Shot "control-torrent-detail-$($tab.Key)"
}
}

$bad = @("ReferenceError","TypeError","is not defined","Unable to assign","Cannot assign",
    "polish\(\) loop","Binding loop","Unable to handle")
$lines = if (Test-Path $err) { @(Get-Content $err) } else { @() }
$qml = @($lines | Where-Object { $line=$_; ($bad | Where-Object { $line -match $_ }).Count })
if (-not $KeepOpen) { Stop-Process -Id $p.Id -Force }
if ($qml.Count) { $qml | Select-Object -First 20; exit 1 }
"PASS: $($sizes.Count * $tabs.Count * 2) real-click resize/theme captures; 0 QML errors"
