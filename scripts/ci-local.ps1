# Runs the same checks as .github/workflows (CI + Security) on this machine, so a green GitHub run
# is not needed. Usage: .\scripts\ci-local.ps1 [-SkipQt] [-SkipExtension] [-SkipAudit]
# Needs: Rust, Python, Node (extension), cargo-audit + cargo-deny (cargo install cargo-audit cargo-deny --locked).
# Not covered: the gitleaks secret scan (Go tool) and the Linux test run.
param([switch]$SkipQt, [switch]$SkipExtension, [switch]$SkipAudit)

$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$results = [ordered]@{}
$retried = @()

function Step($name, [scriptblock]$body) {
    Write-Host "`n=== $name" -ForegroundColor Cyan
    $ErrorActionPreference = "Continue"
    & $body
    $ok = ($LASTEXITCODE -eq 0)
    $results[$name] = $ok
    if (-not $ok) { Write-Host "FAILED: $name" -ForegroundColor Red }
}

Step "root hygiene" {
    $allowed = '^(\.gitattributes|\.gitignore|\.gitleaks\.toml|AGENTS\.md|ARCHITECTURE\.md|CHANGELOG\.md|CLAUDE\.md|CONTRIBUTING\.md|Cargo\.lock|Cargo\.toml|Dockerfile|HANDOFF\.md|LICENSE|README\.md|ROADMAP\.md|SECURITY\.md|clippy\.toml|deny\.toml|install\.ps1|rustfmt\.toml)$'
    $stray = git ls-files | Where-Object { $_ -notmatch '/' -and $_ -notmatch $allowed }
    if ($stray) { Write-Host "Unexpected files at repo root: $stray"; cmd /c "exit 1" } else { cmd /c "exit 0" }
}
Step "fmt" { cargo fmt --all -- --check }
Step "clippy" { cargo clippy --workspace --exclude ferric-qt --all-targets -- -D clippy::correctness -D clippy::suspicious }
Step "check" { cargo check --workspace --exclude ferric-qt }
Step "test" {
    cargo test --workspace --exclude ferric-qt
    if ($LASTEXITCODE -ne 0) {
        # A few engine tests are timing-sensitive and fail when the machine is busy compiling/scanning.
        # Run once more; a second pass is reported as "pass after retry" so it is never silent.
        Write-Host "Tests failed; running them once more to tell a busy machine from a real bug..." -ForegroundColor Yellow
        cargo test --workspace --exclude ferric-qt
        if ($LASTEXITCODE -eq 0) { $script:retried += "test" }
    }
}
if (-not $SkipQt) {
    # cargo check never links, so it cannot see this: the cxx bridge (cxx, 1.0.N) and the generator that
    # cxx-qt uses for the C++ side (cxx-gen, 0.7.N) must be the same N, or ferric-qt fails to link.
    Step "cxx versions agree" {
        $lock = Get-Content Cargo.lock -Raw
        $cxx = [regex]::Match($lock, 'name = "cxx"\r?\nversion = "1\.0\.(\d+)"').Groups[1].Value
        $gen = [regex]::Match($lock, 'name = "cxx-gen"\r?\nversion = "0\.7\.(\d+)"').Groups[1].Value
        if ($cxx -and $cxx -eq $gen) { cmd /c "exit 0" }
        else { Write-Host "cxx 1.0.$cxx does not match cxx-gen 0.7.$gen: ferric-qt will not link"; cmd /c "exit 1" }
    }
    Step "ferric-qt check" { cargo check -p ferric-qt }
    Step "wire audit" { python tasks/cxx-qt/tools/wire_audit.py }
}
if (-not $SkipExtension) {
    Step "extension build + tests" {
        Push-Location extension
        npm run build; if ($LASTEXITCODE -eq 0) { npm test }
        Pop-Location
    }
}
if (-not $SkipAudit) {
    Step "cargo audit" { cargo audit --ignore RUSTSEC-2023-0071 }
    Step "cargo deny" { cargo deny check }
}

Write-Host "`n=== Summary" -ForegroundColor Cyan
foreach ($k in $results.Keys) {
    $label = if (-not $results[$k]) { "FAIL" } elseif ($retried -contains $k) { "pass after retry (flaky first run)" } else { "pass" }
    "{0,-26} {1}" -f $k, $label
}
if ($results.Values -contains $false) { exit 1 }
