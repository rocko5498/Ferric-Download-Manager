# Ferric-Qt Security Review

Date: 2026-07-14
Reviewer: MiMoCode (automated)
Scope: ferric-qt client (QML + Rust CommandBus)

## Checklist

### 1. Network binding
- [x] Daemon binds to `127.0.0.1` only (random port)
- [x] Auth token required on every request (`X-Ferric-Auth` header)
- [x] Token stored in lock file with restricted permissions
- [ ] WebUI remote admin: NOT IMPLEMENTED (settings exist but are inert; no server code was written)

### 2. Cookie/credential handling
- [x] Never reads Chrome/Firefox cookie databases from disk
- [x] Cookie paste is user-initiated only (clipboard)
- [x] Proxy credentials encrypted at rest (AES-256-GCM)
- [x] Audit payloads never include cookies, passkeys, or proxy credentials

### 3. Privilege escalation
- [x] No UAC/elevation prompts
- [x] No DLL injection
- [x] No Chrome patching or MITM
- [x] No kernel drivers or system service installation

### 4. Input validation
- [x] `validate_url()` on all URLs from extension before network requests
- [x] `safe_filename()` before any file creation
- [x] SSRF prevention: URL validation blocks internal IPs
- [x] DNS rebind defense: secondary URL validation

### 5. Token handling
- [x] Token generated randomly per daemon start
- [x] Token never logged or persisted beyond lock file lifetime
- [x] Lock file cleaned up on daemon exit

### 6. Log redaction
- [x] No secrets in tracing output
- [x] Proxy passwords encrypted, never logged in plaintext
- [x] Download URLs logged at info level only (no cookies in URL)

### 7. Torrent privacy
- [x] Kill switch: refuses torrent start when proxy_required=1 and no proxy set
- [x] TCP listen disabled by default (outbound-only)
- [x] DHT/PEX/LSD off by default
- [x] Seeding off by default
- [x] Proxy test endpoint does not leak credentials in logs

### 8. QML/Qt security
- [x] No `QtWebEngine` usage (no browser engine attack surface)
- [x] No remote code loading
- [x] All API calls go through CommandBus (Rust bridge), not direct HTTP from QML
- [x] No `eval()` or dynamic code execution in QML

### 9. File system
- [x] Data directory resolved via `resolve_data_dir()` (no hardcoded paths)
- [x] Portable mode supported (data dir next to binary)
- [x] No writes to system directories
- [x] Download paths validated before write

## Known limitations
- Proxy test uses httpbin.org (external) — could be replaced with self-hosted endpoint
- No code signing yet — users must trust the binary来源
- WebUI remote admin: not implemented (settings exist but are inert); do not enable, has no effect

## Recommendations
1. Add rate limiting to auth failures (brute force protection)
2. Rotate auth token on daemon restart (already done)
3. If WebUI is ever built: add mTLS, CSP headers, and rate limiting before shipping
