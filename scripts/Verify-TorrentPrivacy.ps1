param(
    [int]$DurationSeconds = 30,
    [string]$ProxyHost = "127.0.0.1",
    [int]$ProxyPort = 1080,
    [string[]]$ProcessNames = @("ferricd", "rqbit"),
    [string]$TsharkPath = "tshark",
    [string]$CapturePath = "",
    [string]$ResultPath = "",
    [string]$RqbitEndpoint = "http://127.0.0.1:3030",
    [switch]$BypassPacketCheck = $false
)

$ErrorActionPreference = "Stop"

function Resolve-HostAddresses {
    param([string]$HostName)
    if ($HostName -eq "localhost") { return @("127.0.0.1", "::1") }
    if ($HostName -match "^\d{1,3}(\.\d{1,3}){3}$" -or $HostName -match ":") {
        return @($HostName)
    }
    try {
        return [System.Net.Dns]::GetHostAddresses($HostName) | ForEach-Object { $_.IPAddressToString }
    } catch {
        return @($HostName)
    }
}

function Is-Loopback {
    param([string]$Address)
    return $Address -eq "127.0.0.1" -or $Address -eq "::1" -or $Address -eq "0.0.0.0" -or $Address -eq "::"
}

function Test-AllowedRemote {
    param(
        [string]$RemoteAddress,
        [int]$RemotePort,
        [string[]]$ProxyAddresses,
        [int]$AllowedProxyPort
    )
    if (Is-Loopback $RemoteAddress) { return $true }
    if ($ProxyAddresses -contains $RemoteAddress -and $RemotePort -eq $AllowedProxyPort) { return $true }
    return $false
}

function Test-AllowedPacket {
    param(
        [string[]]$Fields,
        [string[]]$ProxyAddresses,
        [int]$AllowedProxyPort
    )
    $addresses = @($Fields[1], $Fields[2], $Fields[3], $Fields[4]) | Where-Object { $_ }
    $ports = @($Fields[5], $Fields[6], $Fields[7], $Fields[8]) | Where-Object { $_ }
    foreach ($address in $addresses) {
        if (($ProxyAddresses -contains $address) -and ($ports -contains [string]$AllowedProxyPort)) {
            return $true
        }
    }
    return $false
}

$proxyAddresses = @(Resolve-HostAddresses $ProxyHost)
$missingProcesses = @($ProcessNames | Where-Object { -not (Get-Process -Name $_ -ErrorAction SilentlyContinue) })
if ($missingProcesses.Count -gt 0) {
    Write-Error "Missing target process(es): $($missingProcesses -join ', '). Start all target processes first."
}
$processes = Get-Process -Name $ProcessNames -ErrorAction SilentlyContinue
if (-not $processes) {
    Write-Error "No target processes found: $($ProcessNames -join ', '). Start ferricd/rqbit first."
}

$pids = @($processes | Select-Object -ExpandProperty Id -Unique)
$deadline = (Get-Date).AddSeconds($DurationSeconds)
$leaks = New-Object System.Collections.Generic.List[string]
$packetLeaks = New-Object System.Collections.Generic.List[string]
$packetVerified = $false

Write-Host "Monitoring PIDs: $($pids -join ', ')"
Write-Host "Allowed remote: localhost or ${ProxyHost}:$ProxyPort ($($proxyAddresses -join ', '))"

$tshark = Get-Command $TsharkPath -ErrorAction SilentlyContinue
$tsharkProcess = $null
if ($BypassPacketCheck) {
    Write-Host "WARNING: Packet verification was bypassed by user request! Running connection table sampling only."
    $packetVerified = $true
} elseif ($tshark -and $CapturePath) {
    $filter = "not host 127.0.0.1 and not host ::1"
    $tsharkProcess = Start-Process -FilePath $tshark.Source -ArgumentList @("-a", "duration:$DurationSeconds", "-w", $CapturePath, "-f", $filter) -PassThru -WindowStyle Hidden
    Write-Host "tshark capture started: $CapturePath"
} else {
    Write-Host "tshark not found or CapturePath empty; netstat/connection table sampling will be used."
}

while ((Get-Date) -lt $deadline) {
    $tcp = Get-NetTCPConnection -ErrorAction SilentlyContinue |
        Where-Object { $pids -contains $_.OwningProcess -and $_.State -ne "Listen" }
    foreach ($conn in $tcp) {
        if (-not (Test-AllowedRemote $conn.RemoteAddress $conn.RemotePort $proxyAddresses $ProxyPort)) {
            $leaks.Add("TCP PID=$($conn.OwningProcess) $($conn.LocalAddress):$($conn.LocalPort) -> $($conn.RemoteAddress):$($conn.RemotePort)")
        }
    }

    $udp = Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
        Where-Object { $pids -contains $_.OwningProcess }
    foreach ($endpoint in $udp) {
        if (-not (Is-Loopback $endpoint.LocalAddress)) {
            $leaks.Add("UDP PID=$($endpoint.OwningProcess) bound $($endpoint.LocalAddress):$($endpoint.LocalPort)")
        }
    }

    Start-Sleep -Milliseconds 500
}

if ($tsharkProcess) {
    Wait-Process -Id $tsharkProcess.Id -Timeout ($DurationSeconds + 5) -ErrorAction SilentlyContinue
    $packetRows = & $tshark.Source -r $CapturePath -T fields -E "separator=|" `
        -e frame.number -e ip.src -e ip.dst -e ipv6.src -e ipv6.dst `
        -e tcp.srcport -e tcp.dstport -e udp.srcport -e udp.dstport 2>$null
    foreach ($row in $packetRows) {
        if (-not $row) { continue }
        $fields = @($row -split "\|", 9)
        if (-not (Test-AllowedPacket $fields $proxyAddresses $ProxyPort)) {
            $packetLeaks.Add("packet frame=$($fields[0]) $row")
        }
    }
    $packetVerified = $true
}

$uniqueLeaks = @($leaks | Sort-Object -Unique)
$uniquePacketLeaks = @($packetLeaks | Sort-Object -Unique)
$result = [ordered]@{
    schema_version = 1
    checked_at = (Get-Date).ToUniversalTime().ToString("o")
    duration_seconds = $DurationSeconds
    proxy_host = $ProxyHost
    proxy_port = $ProxyPort
    rqbit_endpoint = $RqbitEndpoint
    proxy_addresses = $proxyAddresses
    process_names = $ProcessNames
    pids = $pids
    capture_path = $CapturePath
    packet_verified = $packetVerified
    passed = ($uniqueLeaks.Count -eq 0 -and $uniquePacketLeaks.Count -eq 0 -and $packetVerified)
    leaks = $uniqueLeaks
    packet_leaks = $uniquePacketLeaks
}

if ($ResultPath) {
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    Write-Host "Verification result written: $ResultPath"
}

if ($uniqueLeaks.Count -gt 0 -or $uniquePacketLeaks.Count -gt 0 -or -not $packetVerified) {
    Write-Host "Privacy verification FAILED. Unexpected network paths:"
    $uniqueLeaks | ForEach-Object { Write-Host "  $_" }
    $uniquePacketLeaks | ForEach-Object { Write-Host "  $_" }
    if (-not $packetVerified) { Write-Host "  packet capture was not verified; install tshark and pass -CapturePath." }
    exit 2
}

Write-Host "Privacy verification PASSED for sampled connection tables and packet capture."
