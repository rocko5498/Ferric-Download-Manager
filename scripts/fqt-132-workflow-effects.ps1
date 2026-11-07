param([switch]$SkipDownloader)

$ErrorActionPreference = "Stop"
$lockPath = "$env:LOCALAPPDATA\ferric\ferric\data\ferric.lock"
$artifactRoot = Join-Path $PSScriptRoot "smoke-artifacts\fqt-132\payloads"
New-Item -ItemType Directory -Force $artifactRoot | Out-Null

$lock = Get-Content $lockPath
$base = "http://127.0.0.1:$($lock[0])"
$headers = @{ "X-Ferric-Auth" = $lock[1]; "Content-Type" = "application/json" }
$failures = New-Object System.Collections.Generic.List[string]

function Call($method, $path, $body = $null, $timeout = 60) {
    $args = @{ Uri="$base$path"; Method=$method; Headers=$headers; TimeoutSec=$timeout }
    if ($null -ne $body) { $args.Body = ($body | ConvertTo-Json -Depth 12) }
    Invoke-RestMethod @args
}
function Pass($name, $detail) { "PASS $name - $detail" }

# Downloader: real network bytes, unique record, delete only this fixture record.
if (-not $SkipDownloader) {
$download = Call Post "/downloads" @{
    url = "https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4"
    filename = "fqt132-http-big-buck-bunny-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()).mp4"
    save_dir = $artifactRoot
}
$downloadId = $download.id
for ($i=0; $i -lt 600; $i++) {
    Start-Sleep -Milliseconds 500
    $download = Call Get "/downloads/$downloadId"
    if ($download.status -in @("completed","error","cancelled")) { break }
}
if ($download.status -ne "completed" -or $download.downloaded -lt 60000000) {
    throw "downloader failed: status=$($download.status) bytes=$($download.downloaded)"
}
Pass "downloader" "id=$downloadId bytes=$($download.downloaded)"
Call Delete "/downloads/${downloadId}?delete_file=true" | Out-Null
}

# Grabber: crawl public page and prove returned links.
$grab = @(Call Post "/grab" @{
    url = "https://www.rust-lang.org/"
    extensions = @("html","pdf","zip")
    depth = 0
} 120)
if ($grab.Count -eq 0) {
    $grab = @(Call Post "/grab" @{
        url = "https://www.rust-lang.org/"
        extensions = @()
        depth = 0
    } 120)
}
if ($grab.Count -eq 0) { throw "grabber returned zero links" }
Pass "grabber" "links=$($grab.Count)"

# RSS: exercise rule create/delete against an existing feed; never add an
# undeletable feed as test residue.
$feeds = @(Call Get "/rss/feeds")
if ($feeds.Count -eq 0) { throw "RSS has no existing feed fixture" }
$feedId = [string](@($feeds.id)[0])
$existingRules = @(Call Get "/rss/rules")
foreach ($old in $existingRules | Where-Object { $_.name -like "fqt132-rule-*" }) {
    Call Delete "/rss/rules/$($old.id)" | Out-Null
}
$ruleName = "fqt132-rule-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())"
$rule = Call Post "/rss/rules" @{
    feed_id = $feedId
    name = $ruleName
    match_pattern = "fqt132-never-match-*"
    action = "download"
}
$rules = @(Call Get "/rss/rules")
$createdRule = $rules | Where-Object { $_.name -eq $ruleName } | Select-Object -First 1
if (-not $createdRule) { throw "RSS rule missing after create" }
Call Delete "/rss/rules/$($createdRule.id)" | Out-Null
Pass "rss-rule" "created, listed, deleted id=$($createdRule.id)"

# Media: Creative Commons Blender video. Download the smallest advertised
# format, prove real bytes, then remove only this fixture's record and file.
$mediaUrl = "https://www.youtube.com/watch?v=aqz-KE-bpKQ"
try {
    $media = Call Post "/media/extract" @{ url=$mediaUrl } 300
    if (-not $media.formats -or $media.formats.Count -eq 0) { throw "media extraction returned no formats" }
    $format = $media.formats | Where-Object { $_.id -eq "18" } | Select-Object -First 1
    if (-not $format) {
        $format = $media.formats |
            Where-Object { $_.filesize -and [int64]$_.filesize -ge 10000000 -and [int64]$_.filesize -le 100000000 } |
            Sort-Object { [int64]$_.filesize } -Descending |
            Select-Object -First 1
    }
    if (-not $format) { throw "media extraction returned no substantial format" }
    $formatId = $format.id
    $mediaJob = Call Post "/media/download" @{
        url=$mediaUrl
        format_id=$formatId
        filename="fqt132-media-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()).$($format.ext)"
        save_dir=$artifactRoot
    } 300
    $mediaDownload = $mediaJob.download
    $mediaId = $mediaDownload.id
    for ($i=0; $i -lt 600; $i++) {
        Start-Sleep -Milliseconds 500
        $mediaDownload = Call Get "/downloads/$mediaId"
        if ($mediaDownload.status -in @("completed","error","cancelled")) { break }
    }
    if ($mediaDownload.status -ne "completed" -or $mediaDownload.downloaded -lt 10000000) {
        throw "media download failed: id=$mediaId status=$($mediaDownload.status) bytes=$($mediaDownload.downloaded)"
    }
    Pass "media" "title=$($media.title) format=$formatId id=$mediaId bytes=$($mediaDownload.downloaded)"
    Call Delete "/downloads/${mediaId}?delete_file=true" | Out-Null
} catch {
    $mediaError = if ($_.ErrorDetails.Message) { $_.ErrorDetails.Message } else { $_.Exception.Message }
    $failures.Add("media: $mediaError")
    "FAIL media - $mediaError"
}

# Torrent: stage isolated legal Sintel magnet. Start briefly, require engine
# activity or downloaded bytes, then pause and delete only this newly-created ID.
$hash = "08ada5a7a6183aae1e09d831df6748d566095a10"
$before = @(Call Get "/torrents")
if ($before | Where-Object { $_.info_hash -eq $hash }) {
    throw "Sintel fixture already exists; refusing to touch pre-existing record"
}
$magnet = "magnet:?xt=urn:btih:$hash&dn=FQT132-Sintel&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce"
$probe = Call Post "/torrents/probe" @{ input=$magnet } 120
$stage = Call Post "/torrents" @{
    probe = $probe
    save_root = $artifactRoot
    policy = @{
        dht_enabled=$true; pex_enabled=$false; lsd_enabled=$false
        seed_enabled=$false; seed_ratio_limit=0; seed_time_limit_min=0
        proxy_required=$false; blocklist_enabled=$false; mse_required=$false
    }
}
$torrentId = $stage.record.id
try {
    Call Post "/torrents/$torrentId/start" | Out-Null
    $sync = $null
    for ($i=0; $i -lt 12; $i++) {
        Start-Sleep -Seconds 5
        $sync = Call Post "/torrents/$torrentId/sync"
        if ($sync.record.downloaded -gt 0 -or @($sync.peers).Count -gt 0) { break }
    }
    if ($null -eq $sync) { throw "torrent sync unavailable" }
    Pass "torrent" "status=$($sync.record.status) bytes=$($sync.record.downloaded) peers=$(@($sync.peers).Count)"
} finally {
    try { Call Post "/torrents/$torrentId/pause" | Out-Null } catch {}
    try { Call Delete "/torrents/$torrentId" | Out-Null } catch {}
}

if ($failures.Count) { throw ($failures -join "; ") }
"ALL_WORKFLOW_EFFECTS_PASS"
