# Removes what Install-Ferric.ps1 added. Your downloads and Ferric's own data (list, settings) are kept.
param([switch]$Quiet, [switch]$RemoveData)
$ErrorActionPreference = "Continue"
$dir = $PSScriptRoot
function Say($m) { if (-not $Quiet) { Write-Host $m } }

Get-Process ferric-qt, ferricd, ferric-host -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 800
$host_ = Join-Path $dir "ferric-host.exe"
# The browser registration is shared by every Ferric copy on the machine: only remove it when it points at this one.
$manifest = Join-Path $env:APPDATA "Ferric\com.ferric.host.json"
$ours = $true
if (Test-Path $manifest) {
    try { $ours = ((Get-Content $manifest -Raw | ConvertFrom-Json).path -ieq $host_) } catch { $ours = $true }
}
if ((Test-Path $host_) -and $ours) { & $host_ --uninstall 2>&1 | ForEach-Object { Say "  $_" } }
elseif (-not $ours) { Say "The browser registration belongs to another Ferric install; left in place." }

Remove-Item (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Ferric.lnk") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path ([Environment]::GetFolderPath("Desktop")) "Ferric.lnk") -Force -ErrorAction SilentlyContinue
Remove-Item "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferric" -Recurse -Force -ErrorAction SilentlyContinue

if ($RemoveData) {
    Remove-Item (Join-Path $env:LOCALAPPDATA "ferric") -Recurse -Force -ErrorAction SilentlyContinue
    Say "Ferric's data folder removed"
}
# The script lives in the folder being deleted, so hand the delete to a detached cmd after this exits.
Start-Process cmd.exe -ArgumentList "/c ping 127.0.0.1 -n 3 >nul & rmdir /s /q `"$dir`"" -WindowStyle Hidden
Say "Ferric uninstalled."
