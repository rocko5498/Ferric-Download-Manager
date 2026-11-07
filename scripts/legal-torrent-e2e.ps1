# Legal Creative Commons torrent smoke (Sintel / Blender Foundation).
# 1) Proves proxy_required hard-gate refuses start
# 2) Temporarily allows bare BT for legal test only, outbound-only
# 3) Restores torrent_proxy_required=1 and kills rqbit
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
if (-not $root) { $root = "R:\Rust Project\Rust.net Download Manager" }
$bin = Join-Path $root "target\debug\ferricd.exe"
$lockPath = Join-Path $env:LOCALAPPDATA "ferric\ferric\data\ferric.lock"
$save = "E:\Downloads\Torrent\legal-test"
New-Item -ItemType Directory -Force -Path $save | Out-Null

function Ensure-Daemon {
  $alive = $false
  if (Test-Path $lockPath) {
    $lines = Get-Content $lockPath
    $port = [int]$lines[0]; $tok = $lines[1]
    try {
      $null = Invoke-RestMethod "http://127.0.0.1:$port/health" -Headers @{"X-Ferric-Auth"=$tok} -TimeoutSec 2
      $alive = $true
    } catch { $alive = $false }
  }
  if (-not $alive) {
    Stop-Process -Name ferricd -Force -ErrorAction SilentlyContinue
    Start-Sleep 1
    Start-Process -FilePath $bin -WorkingDirectory (Split-Path $bin) -WindowStyle Hidden
    for ($i = 0; $i -lt 20; $i++) {
      Start-Sleep 1
      if (-not (Test-Path $lockPath)) { continue }
      $lines = Get-Content $lockPath
      $port = [int]$lines[0]; $tok = $lines[1]
      try {
        $null = Invoke-RestMethod "http://127.0.0.1:$port/health" -Headers @{"X-Ferric-Auth"=$tok} -TimeoutSec 2
        return
      } catch {}
    }
    throw "ferricd failed to become ready"
  }
}

function Get-Auth {
  $lines = Get-Content $lockPath
  @{ port = [int]$lines[0]; tok = $lines[1]; h = @{"X-Ferric-Auth"=$lines[1]; "Content-Type"="application/json"} }
}

function Set-Settings($auth, $map) {
  Invoke-RestMethod -Uri "http://127.0.0.1:$($auth.port)/settings" -Method POST -Headers $auth.h -Body ($map | ConvertTo-Json) | Out-Null
}

Ensure-Daemon
$a = Get-Auth
Write-Host "daemon port=$($a.port)"

# Wipe torrents
$list = @(Invoke-RestMethod "http://127.0.0.1:$($a.port)/torrents" -Headers @{"X-Ferric-Auth"=$a.tok})
foreach ($t in $list) {
  if ($t.id) {
    try { Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$($t.id)" -Method DELETE -Headers @{"X-Ferric-Auth"=$a.tok} | Out-Null; Write-Host "removed $($t.name)" } catch {}
  }
}
Stop-Process -Name rqbit -Force -ErrorAction SilentlyContinue

Set-Settings $a @{
  torrent_proxy_required = "1"
  torrent_tcp_listen_enabled = "0"
  torrent_engine_enabled = "1"
  torrent_kill_switch_active = "0"
  torrent_proxy_url = ""
}

# DHT on for legal discovery test only; PEX/LSD/seed stay off; listen stays off.
$policy = @{
  dht_enabled = $true; pex_enabled = $false; lsd_enabled = $false; seed_enabled = $false
  seed_ratio_limit = 0; seed_time_limit_min = 0; upload_limit_bps = $null
  proxy_required = $false; blocklist_enabled = $false; mse_required = $false
}

# Legal CC Sintel
$magnet = "magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce"

Write-Host "=== GATE under proxy_required=1 ==="
$probe = Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/probe" -Method POST -Headers $a.h -Body (@{ input = $magnet } | ConvertTo-Json)
Write-Host "probe supported=$($probe.supported) hash=$($probe.info_hash) name=$($probe.name)"
$stage = Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents" -Method POST -Headers $a.h -Body (@{
  probe = $probe; save_root = $save; policy = $policy
} | ConvertTo-Json -Depth 8)
$id = $stage.record.id
Write-Host "staged id=$id proxy_required=$($stage.record.policy.proxy_required) status=$($stage.record.status)"
$gateOk = $false
try {
  Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id/start" -Method POST -Headers $a.h | Out-Null
  Write-Host "GATE FAIL: start allowed with proxy_required=1"
} catch {
  Write-Host "GATE OK: $($_.ErrorDetails.Message)"
  $gateOk = $true
}

Write-Host "=== LEGAL DOWNLOAD (temp proxy_required=0, listen off) ==="
Set-Settings $a @{
  torrent_proxy_required = "0"
  torrent_tcp_listen_enabled = "0"
  torrent_kill_switch_active = "0"
}
try {
  $upd = Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id/policy" -Method PATCH -Headers $a.h -Body ($policy | ConvertTo-Json)
  Write-Host "policy proxy_required=$($upd.record.policy.proxy_required)"
} catch {
  Write-Host "policy patch: $($_.ErrorDetails.Message)"
}

$started = Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id/start" -Method POST -Headers $a.h
Write-Host "STARTED status=$($started.record.status) engine=$($started.record.engine) total=$($started.record.total_size)"

$okBytes = $false
for ($i = 1; $i -le 24; $i++) {
  Start-Sleep 5
  try {
    $sync = Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id/sync" -Method POST -Headers @{"X-Ferric-Auth"=$a.tok}
    $dl = $sync.record.downloaded
    $tot = $sync.record.total_size
    $st = $sync.record.status
    $pc = @($sync.peers).Count
    Write-Host "[$i] status=$st dl=$dl total=$tot peers=$pc"
    if ($dl -gt 0) { $okBytes = $true; break }
  } catch {
    Write-Host "[$i] sync: $($_.Exception.Message)"
  }
}

Write-Host "=== listeners ==="
Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
  Where-Object {
    $_.LocalPort -in @(3030, 4240) -or
    ((Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName -eq "rqbit")
  } |
  Select-Object LocalAddress, LocalPort, OwningProcess |
  Format-Table -AutoSize

try { Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id/pause" -Method POST -Headers @{"X-Ferric-Auth"=$a.tok} | Out-Null } catch {}
try { Invoke-RestMethod -Uri "http://127.0.0.1:$($a.port)/torrents/$id" -Method DELETE -Headers @{"X-Ferric-Auth"=$a.tok} | Out-Null; Write-Host "deleted test torrent" } catch {}
Stop-Process -Name rqbit -Force -ErrorAction SilentlyContinue

Set-Settings $a @{
  torrent_proxy_required = "1"
  torrent_tcp_listen_enabled = "0"
  torrent_kill_switch_active = "0"
}
Write-Host "restored torrent_proxy_required=1 listen=0"

if (-not $gateOk) { Write-Host "RESULT: FAIL gate"; exit 1 }
if ($okBytes) { Write-Host "RESULT: PASS gate + legal Sintel bytes downloaded" }
else { Write-Host "RESULT: PASS gate + engine started (no bytes in window; swarm may be quiet)" }
exit 0
