$ErrorActionPreference = "Stop"
$repo = "R:\Rust Project\Rust.net Download Manager"
$exe  = Join-Path $repo "target\debug\ferric-qt.exe"
$dexe = Join-Path $repo "target\debug\ferricd.exe"
$art  = Join-Path $repo "scripts\smoke-artifacts\e2e3"
$lockPath = "$env:LOCALAPPDATA\ferric\ferric\data\ferric.lock"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class F1 {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int X,int Y,int W,int H,bool r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
}
"@
function S($n){ $r=New-Object F1+RECT; [F1]::GetWindowRect($hwnd,[ref]$r)|Out-Null
  $b=New-Object System.Drawing.Bitmap ($r.Right-$r.Left),($r.Bottom-$r.Top)
  $g=[System.Drawing.Graphics]::FromImage($b); $hdc=$g.GetHdc(); [F1]::PrintWindow($hwnd,$hdc,2)|Out-Null
  $g.ReleaseHdc($hdc); $g.Dispose(); $b.Save((Join-Path $art "$n.png"),[System.Drawing.Imaging.ImageFormat]::Png); $b.Dispose(); "shot: $n" }

$portBefore = (Get-Content $lockPath)[0]
"daemon port BEFORE: $portBefore"

# launch UI against the current daemon
$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru
$hwnd=[IntPtr]::Zero; $dl=(Get-Date).AddSeconds(25)
while ((Get-Date) -lt $dl) { Start-Sleep -Milliseconds 500; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $hwnd=$p.MainWindowHandle; break } }
if ($hwnd -eq [IntPtr]::Zero) { "NO WINDOW"; exit 1 }
[F1]::MoveWindow($hwnd,50,50,1016,689,$true)|Out-Null; Start-Sleep -Seconds 3
[F1]::SetForegroundWindow($hwnd)|Out-Null
S "20-before-daemon-restart"

# kill the daemon; UI is now pointing at a dead port
Stop-Process -Name ferricd -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 4
S "21-daemon-down"
[F1]::SetForegroundWindow($hwnd)|Out-Null
[F1]::keybd_event(0x0D,0,0,[UIntPtr]::Zero)
[F1]::keybd_event(0x0D,0,2,[UIntPtr]::Zero)

# start a NEW daemon (it will pick a fresh port + token)
Remove-Item $lockPath -ErrorAction SilentlyContinue
Start-Process -FilePath $dexe -WorkingDirectory $repo -WindowStyle Hidden
Start-Sleep -Seconds 8
$portAfter = (Get-Content $lockPath)[0]
"daemon port AFTER:  $portAfter"

# give the hub's recovery poll time to notice the new lock and re-attach
Start-Sleep -Seconds 20
S "22-after-restart-should-reconnect"

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
if ($portBefore -ne $portAfter) { "PORT CHANGED $portBefore -> $portAfter (real F-1 scenario)" }
else { "port unchanged - test inconclusive" }
