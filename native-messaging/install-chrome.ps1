param(
    [string]$HostPath = (Resolve-Path "$PSScriptRoot\..\target\debug\ferric-host.exe").Path,
    [string]$ManifestPath = "$PSScriptRoot\com.ferric.host.chrome.generated.json"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $HostPath)) {
    throw "ferric-host not found at $HostPath. Run `cargo build -p ferric-host` first."
}

$templatePath = Join-Path $PSScriptRoot "com.ferric.host.chrome.json"
$manifest = Get-Content -LiteralPath $templatePath -Raw
$escapedHostPath = $HostPath.Replace("\", "\\")
$manifest = $manifest.Replace("FERRIC_HOST_PATH", $escapedHostPath)
Set-Content -LiteralPath $ManifestPath -Value $manifest -Encoding UTF8

$keyPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.ferric.host"
New-Item -Path $keyPath -Force | Out-Null
Set-ItemProperty -Path $keyPath -Name "(default)" -Value (Resolve-Path $ManifestPath).Path

Write-Host "Installed com.ferric.host for Chrome"
Write-Host "Manifest: $((Resolve-Path $ManifestPath).Path)"
Write-Host "Host: $HostPath"
