# Contributing to Ferric (Task 174)

## Setup
```bash
cargo check --workspace
cargo test -p ferric-extract --lib
cargo test -p ferricd --lib
```

## Rules
- Pure Rust for extract/download paths; no Python runtime in product
- `security::validate_url` before network on user URLs
- No cookie logging
- Platform code behind `cfg(windows)` / `cfg(target_os = "linux")`
- Prefer smallest change (ponytail)

## Tasks
See `docs/superpowers/specs/2026-07-11-ferric-master-roadmap.md` and `HANDOFF.md`.

## Checks run on your own machine

GitHub Actions is switched to manual start (the account has no paid runner minutes). Before you push or open a pull
request, run every check CI would run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\ci-local.ps1
```

It runs the root-file check, `cargo fmt`, clippy, the workspace check and tests, the Qt build check and QML wire audit,
the extension build and tests, `cargo audit` and `cargo deny` (install the last two with
`cargo install cargo-audit cargo-deny --locked`), and prints one pass/FAIL line per step. Not covered: the gitleaks
secret scan and the Linux test run. On Windows, Smart App Control can block a freshly built helper program
(`os error 4551`); that is the machine, not the code.
