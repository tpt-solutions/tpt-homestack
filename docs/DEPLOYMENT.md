# Deployment guide

tpt-homestack is a single binary; the only companion you may want is a TLS
edge. Everything below assumes the release build:

```sh
cargo build --release -p tpt-home-cli   # → target/release/tpt-home(.exe)
```

## 1. Initialize

```sh
TPT_HOME_DATA_DIR=/var/lib/tpt-home ./tpt-home init
# or on Windows:
.\tpt-home.exe --data-dir C:\Users\you\AppData\Local\tpt-home init
```

`init` creates `config.toml`, applies migrations and bootstraps the admin
user (password is printed once unless you pass `--admin-password` or set
`TPT_HOME_ADMIN_PASSWORD`).

## 2. Run as a service

### Linux (systemd)

```sh
sudo ./scripts/install.sh                 # installs + enables tpt-home.service
sudo ./scripts/install.sh --uninstall
```

The unit runs the binary as a dedicated `tpt-home` user with
`ProtectSystem=strict`, `NoNewPrivileges` and `MemoryMax=256M`. Override the
listen address with `REMOTE_ADDR=0.0.0.0 sudo -E ./scripts/install.sh`.

### Windows

```powershell
.\scripts\install.ps1                     # copies the exe + registers a logon task
.\scripts\install.ps1 -Uninstall
Start-ScheduledTask tpt-homestack
```

## 3. TLS

The app speaks plain HTTP on localhost; TLS belongs to the edge.

### Caddy (DNS points at this machine)

```sh
TPT_DOMAIN=home.example.com docker compose up -d caddy
```

Caddy obtains/renews certificates automatically and proxies to
`host.docker.internal:8080` (Linux Docker resolves that via `extra_hosts:
host-gateway`, already configured). Set `trust_proxy = true` (otherwise every
visitor shares the proxy's rate-limit bucket) and `secure_cookies = true` in
`data/config.toml` once HTTPS is live.

### Cloudflare Tunnel (no open ports)

```sh
TPT_CLOUDFLARE_TOKEN=<tunnel-token> docker compose up -d cloudflared
```

Create the tunnel in the Cloudflare dashboard, point it at
`http://host.docker.internal:8080`, and put the token in `TPT_CLOUDFLARE_TOKEN`
(a `.env` file next to `docker-compose.yml` works). Set `trust_proxy = true`
here too.

For a plain-language choice between Tailscale and Cloudflare, and the trade-offs,
see [REMOTE_ACCESS.md](REMOTE_ACCESS.md). `tpt-home remote setup` makes the
config changes after you confirm.

## 4. Backups & upgrades

```sh
tpt-home backup                # data/backups/tpt-<ts>.db + uploads/ copy
tpt-home restore <backup.db>   # stop the server first
```

Upgrades: replace the binary, `systemctl restart tpt-home`. Migrations are
forward-only and run automatically at startup (`tpt-home migrate` to apply
them without serving). Always take a `backup` before upgrading.

## 5. Hardening checklist

- `secure_cookies = true` behind HTTPS.
- Bind `host = 127.0.0.1` and let only the edge proxy in.
- `trust_proxy = true` only when a proxy you control is the sole way in
  (default `false`; otherwise `X-Forwarded-For` can be spoofed to dodge the
  rate limit).
- Keep `data/` on a filesystem with sane permissions (700, service user only).
- Rate limiting is built in (auth endpoints: 5/min/IP, API: 240/min/IP).
- Review `SECURITY.md` for the current trust model and known trade-offs
  (e.g. share-link passwords travel as a query parameter).
