# Experiment, run by .github/workflows/extension-offer-test.yml on a GitHub Windows machine:
# does a browser accept Ferric's extension from the external-extension registry key (the way IDM offers its
# own), and does it matter whether the extension's manifest names the store's update address?
# A throwaway key and throwaway profiles are used; nothing is kept.
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$built = Join-Path $root "extension\build\chrome-mv3-prod"
$work = Join-Path $env:RUNNER_TEMP "offer-test"
New-Item -ItemType Directory -Force $work | Out-Null

$chrome = "C:\Program Files\Google\Chrome\Application\chrome.exe"
# Brave's install folder and registry key are not documented anywhere I could find: look for the program,
# and try each key a Chromium browser might read.
$brave = Get-ChildItem "C:\Program Files", "C:\Program Files (x86)", $env:LOCALAPPDATA -Recurse -Filter brave.exe -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
$browsers = [ordered]@{
    chrome = @{ exe = $chrome; regs = @('HKCU:\Software\Google\Chrome\Extensions'); store = "https://clients2.google.com/service/update2/crx" }
    edge   = @{ exe = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"; regs = @('HKCU:\Software\Microsoft\Edge\Extensions'); store = "https://edge.microsoft.com/extensionwebstorebase/v1/crx" }
    brave  = @{ exe = $brave; regs = @('HKCU:\Software\BraveSoftware\Brave-Browser\Extensions', 'HKCU:\Software\Google\Chrome\Extensions', 'HKCU:\Software\Chromium\Extensions'); store = "https://clients2.google.com/service/update2/crx" }
}
foreach ($n in $browsers.Keys) { Write-Host ("{0,-7} installed: {1}  {2}" -f $n, ($browsers[$n].exe -and (Test-Path $browsers[$n].exe)), $browsers[$n].exe) }

# A throwaway signing key and the extension id that comes from it.
$rsa = [Security.Cryptography.RSA]::Create(2048)
$pem = Join-Path $work "key.pem"
Set-Content $pem $rsa.ExportPkcs8PrivateKeyPem()
$hash = [Security.Cryptography.SHA256]::HashData($rsa.ExportSubjectPublicKeyInfo())
$id = -join ($hash[0..15] | ForEach-Object { [char](97 + ($_ -shr 4)); [char](97 + ($_ -band 15)) })
$version = (Get-Content "$built\manifest.json" -Raw | ConvertFrom-Json).version
Write-Host "test extension id $id, version $version"

function New-Crx([string]$variant, [string]$updateUrl) {
    $dir = Join-Path $work "ext-$variant"
    Remove-Item $dir, "$dir.crx" -Recurse -Force -ErrorAction SilentlyContinue
    Copy-Item $built $dir -Recurse
    $m = Get-Content "$dir\manifest.json" -Raw | ConvertFrom-Json
    $m.PSObject.Properties.Remove("key")
    if ($updateUrl) { $m | Add-Member -NotePropertyName update_url -NotePropertyValue $updateUrl -Force }
    [IO.File]::WriteAllText("$dir\manifest.json", ($m | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding($false)))
    $prof = Join-Path $work "pack-profile"
    Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue
    & $chrome "--user-data-dir=$prof" --headless=new --no-first-run "--pack-extension=$dir" "--pack-extension-key=$pem" | Out-Null
    $deadline = (Get-Date).AddSeconds(40)
    while (-not (Test-Path "$dir.crx") -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (-not (Test-Path "$dir.crx")) { throw "could not pack $variant" }
    "$dir.crx"
}

$crx = @{
    plain  = New-Crx "plain" $null
    chrome = New-Crx "chromeurl" $browsers.chrome.store
    edge   = New-Crx "edgeurl" $browsers.edge.store
}

function Stop-Browsers { Get-Process -Name chrome, msedge, brave, crashpad_handler -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue; Start-Sleep 2 }

$rows = @()
foreach ($name in $browsers.Keys) {
    $b = $browsers[$name]
    if (-not $b.exe -or -not (Test-Path $b.exe)) { $rows += [pscustomobject]@{ browser = $name; manifest = "-"; result = "browser not installed" }; continue }
    $variants = if ($name -eq "edge") { @("plain", "edge") } elseif ($name -eq "brave") { @("chrome") } else { @("plain", "chrome") }
    foreach ($variant in $variants) {
        foreach ($reg in $b.regs) {
            Stop-Browsers
            $regPath = "$reg\$id"
            New-Item $regPath -Force | Out-Null
            Set-ItemProperty $regPath -Name path -Value $crx[$variant]
            Set-ItemProperty $regPath -Name version -Value $version
            $prof = Join-Path $work ("profile-{0}-{1}-{2}" -f $name, $variant, ($reg -replace '[^A-Za-z]', ''))
            Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue
            $p = Start-Process $b.exe -ArgumentList "--user-data-dir=$prof", "--no-first-run", "--no-default-browser-check", "--headless=new", "about:blank" -PassThru
            Start-Sleep 45
            Stop-Browsers
            Remove-Item $regPath -Recurse -Force -ErrorAction SilentlyContinue

            $found = $null
            foreach ($f in "Secure Preferences", "Preferences") {
                $path = Join-Path $prof "Default\$f"
                if (-not (Test-Path $path)) { continue }
                $prefs = Get-Content $path -Raw | ConvertFrom-Json
                $e = $prefs.extensions.settings.$id
                if ($e) { $found = $e; break }
            }
            $installed = Test-Path (Join-Path $prof "Default\Extensions\$id")
            $label = if ($variant -eq "plain") { "no update_url" } else { "update_url = store" }
            $where = $reg.Replace('HKCU:\Software\', '')
            if ($found) {
                $rows += [pscustomobject]@{ browser = $name; key = $where; manifest = $label; result = "KNOWN to the browser"; disable_reasons = ($found.disable_reasons -join ","); location = $found.location; files_installed = $installed }
            } else {
                $rows += [pscustomobject]@{ browser = $name; key = $where; manifest = $label; result = "not picked up"; files_installed = $installed }
            }
        }
    }
}
Write-Host "`n=== RESULTS ==="
$rows | Format-List | Out-String | Write-Host
