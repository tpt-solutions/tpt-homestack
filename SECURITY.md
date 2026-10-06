# Security Policy

## Supported versions

Only the latest tagged release of `tpt-homestack` receives security fixes.

## Reporting a vulnerability

**Do not open a public issue for security reports.**

Email **security@tpt-solutions.example** with:

- A description of the vulnerability and its impact
- Steps to reproduce or a proof of concept
- Affected versions / commit hash

You will receive an acknowledgement within 72 hours. We will coordinate a fix
and disclosure timeline with you; credit is given unless you prefer otherwise.

## Security model notes

- Sessions use random 256-bit tokens; only the SHA-256 hash of the token is
  stored server-side. Cookies are `HttpOnly` and `SameSite=Lax` (`Secure` when
  `secure_cookies = true` in `data/config.toml` — enable behind HTTPS).
- State-changing JSON API requests must carry `Content-Type:
  application/json`, which cross-site HTML forms cannot send without a CORS
  preflight (not granted by default).
- Uploaded files are stored under a randomized name inside `data/uploads/`;
  paths are validated against traversal (`..`, absolute paths, control
  characters).
- Blueprint plugins compiled into the binary are fully trusted code. Community
  WASM plugins run sandboxed (Phase 4) with capability-based permissions.
