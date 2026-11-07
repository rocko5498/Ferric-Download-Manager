# Per-user installer for Ferric (no admin rights, no extra tooling).
# Run from the unpacked package folder:  powershell -ExecutionPolicy Bypass -File .\Install-Ferric.ps1
# Options: -InstallDir <path>  -NoShortcut  -NoHostRegistration  -Quiet
param(
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\Ferric",
    [switch]$NoShortcut,
    [switch]$NoHostRegistration,
    [switch]$Quiet
)
$ErrorActionPreference = "Stop"
$src = $PSScriptRoot
foreach ($f in "ferric-qt.exe", "ferricd.exe", "ferric-host.exe") {
    if (-not (Test-Path (Join-Path $src $f))) { throw "$f is missing next to the installer; run it from the unpacked package folder." }
}
function Say($m) { if (-not $Quiet) { Write-Host $m } }

Say "Installing Ferric to $InstallDir"
# A running copy would lock its own files.
Get-Process ferric-qt, ferricd, ferric-host -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 800

New-Item -ItemType Directory -Force $InstallDir | Out-Null
# robocopy exit codes below 8 mean "copied something / nothing to do".
robocopy $src $InstallDir /E /NFL /NDL /NJH /NJS /NP /XF Install-Ferric.cmd | Out-Null
if ($LASTEXITCODE -ge 8) { throw "copying the files failed (robocopy exit code $LASTEXITCODE)" }

if (-not $NoHostRegistration) {
    Say "Registering the browser native-messaging host"
    # ferric-host reports on stderr even on success; Windows PowerShell 5.1 would treat that as a failure.
    $ErrorActionPreference = "Continue"
    & (Join-Path $InstallDir "ferric-host.exe") --install 2>&1 | ForEach-Object { Say "  $_" }
    $hostExit = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($hostExit -ne 0) { Write-Warning "Host registration failed (this build may not carry an extension id)." }
}

if (-not $NoShortcut) {
    $shell = New-Object -ComObject WScript.Shell
    $places = @{
        "Start Menu" = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
        "Desktop"    = [Environment]::GetFolderPath("Desktop")
    }
    foreach ($name in $places.Keys) {
        $lnk = $shell.CreateShortcut((Join-Path $places[$name] "Ferric.lnk"))
        $lnk.TargetPath = Join-Path $InstallDir "ferric-qt.exe"
        $lnk.WorkingDirectory = $InstallDir
        $lnk.Description = "Ferric Download Manager"
        $lnk.Save()
        Say "$name shortcut created"
    }
}

# Add/Remove Programs entry (per user).
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferric"
New-Item -Path $key -Force | Out-Null
$version = (Get-Item (Join-Path $InstallDir "ferric-qt.exe")).VersionInfo.ProductVersion
if (-not $version) { $version = "1.0.0-rc.3" }
$uninstall = "powershell.exe -ExecutionPolicy Bypass -File `"$InstallDir\Uninstall-Ferric.ps1`""
@{
    DisplayName     = "Ferric Download Manager"
    DisplayVersion  = $version
    Publisher       = "Ferric"
    InstallLocation = $InstallDir
    DisplayIcon     = Join-Path $InstallDir "ferric-qt.exe"
    UninstallString = $uninstall
    NoModify        = 1
    NoRepair        = 1
}.GetEnumerator() | ForEach-Object {
    $type = if ($_.Value -is [int]) { "DWord" } else { "String" }
    New-ItemProperty -Path $key -Name $_.Key -Value $_.Value -PropertyType $type -Force | Out-Null
}
Say "Done. Start Ferric from the Start Menu, then load the browser extension from $InstallDir\extension."
