# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private
vulnerability reporting (Security tab -> "Report a vulnerability"). Include the
version, steps to reproduce and the impact. Expect an acknowledgement within a few days.

## Scope and design notes

- The daemon listens on `127.0.0.1` only and every API call needs the per-session token
  (`X-Ferric-Auth`) stored in a file readable by the current user.
- Download URLs are validated against private/loopback/metadata addresses (SSRF) and
  saved paths and file names are sanitised.
- Release builds ignore development-only overrides (for example `FERRIC_ALLOW_PRIVATE_IPS`).

Reports about these boundaries (token handling, SSRF/redirect bypasses, path traversal,
native-messaging host abuse, unsafe file handling, plugin sandbox escapes) are especially welcome.
