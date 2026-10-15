# Reaching your server from outside the house

homestack runs on your own machine and listens only on that machine
(`127.0.0.1`). To open it from your phone or another place, you need a private
path to it. This guide covers two options, one for private family use and one
for a public web address, then the advanced choices and their trade-offs.

Run `tpt-home remote status` to see what is installed and signed in on the
server, and `tpt-home remote setup --provider tailscale|cloudflare` to take the
steps below. The setup command explains each change and makes it only after you
confirm.

## Option 1: Tailscale (recommended for family)

Tailscale creates a private network ("tailnet") between your devices. Only
devices you sign in to it can reach the server. You need no domain, no router
changes and no open ports, and it works on phones.

Cost: free for personal use, with device and user limits on the free plan.
Check the current limits at tailscale.com.

1. Install the official Tailscale client on the server and on each device.
   Download it from tailscale.com/download.
2. Sign in on every device with the same account, or invite family members.
3. In the Tailscale admin console, turn on MagicDNS. This gives the server a
   name such as `homestack.tail1234.ts.net`.
4. On the server, run:

   ```sh
   tpt-home remote setup --provider tailscale
   ```

   It runs `tailscale serve`, which gives the server an HTTPS address on your
   tailnet, and sets `secure_cookies = true` in `config.toml`. The server keeps
   listening on loopback only.
5. Restart homestack, then open the address the command printed on a device
   that is signed in to your tailnet.

Devices outside your tailnet cannot reach the address at all. That is the
point: nothing is exposed to the internet.

## Option 2: Cloudflare Tunnel (a public web address)

Use this when you want a normal web address, or when a device cannot install a
client (for example a shared computer). A tunnel from Cloudflare sends traffic
to the server without opening a port on your router.

This option exposes the app to the internet. Anyone who finds the address can
try to sign in. Put a login in front of it before you rely on it.

Cost: the tunnel is free. You need a domain, which usually costs a few dollars
a year, and it must be added to Cloudflare.

1. Add your domain to Cloudflare.
2. In Cloudflare Zero Trust, create a tunnel and copy its token.
3. Add a public hostname to the tunnel that points at `http://127.0.0.1:8080`
   (use your port if you changed it).
4. Put Cloudflare Access in front of that hostname, so visitors must sign in
   before they reach homestack. Do not skip this step.
5. Run `tpt-home remote setup --provider cloudflare`. It checks that
   `cloudflared` is installed, lists the steps above, and sets
   `secure_cookies = true` and `trust_proxy = true` after you confirm.
6. Run `cloudflared` with the token as a service. The Docker setup in
   [DEPLOYMENT.md](DEPLOYMENT.md#cloudflare-tunnel-no-open-ports) does this
   for you.

`trust_proxy = true` tells the server to believe the `X-Forwarded-For` header
for rate limiting. Set it only when the tunnel is the only way in. If other
routes reach the server, a client can send a fake address and get around the
login rate limit.

## Advanced options

These need more know-how. Their steps are not automated. The step-by-step
instructions are in [REMOTE_ADVANCED.md](REMOTE_ADVANCED.md).

- **Caddy with your own domain.** Caddy gets HTTPS certificates for a domain
  you point at your home address. You must open ports 80 and 443 on your router
  and accept that the server is reachable from the internet. See the Caddy
  section in [DEPLOYMENT.md](DEPLOYMENT.md#3-tls).
- **Netbird.** An open-source alternative to Tailscale with a free hosted tier.
  It is similar in use, with its own coordination service.
- **Headscale.** A self-hosted coordination server for Tailscale clients. It
  needs a small server you control, such as a cheap virtual machine, which
  itself must be reachable from the internet.
- **Vercel and similar hosts** cannot be used. They run short-lived functions
  and cannot accept incoming connections to a home server.

## Trade-offs

- **Carrier-grade NAT.** Some home internet connections share one public
  address between many customers. Options that rely on your router accepting
  connections (Caddy with your own domain) fail on these. Tailscale and
  Cloudflare Tunnel do not need inbound connections, so they keep working.
- **Free plans have limits.** Tailscale and Netbird limit devices and users on
  free plans. Check current limits before you rely on them.
- **Domains cost money.** Cloudflare Tunnel is free, but a domain is not.
- **Public means public.** Only a public address needs a login in front of
  it. A tailnet address is private to the devices you sign in.
- **Who can read the traffic.** Tailscale traffic is encrypted between your
  devices and the server. Cloudflare Tunnel ends HTTPS at Cloudflare's edge, so
  Cloudflare can read the requests and responses to your app. If that matters
  to you, use Tailscale.

## Checklist before you share an address

- [ ] The server has an admin account with a strong, non-default password.
- [ ] `secure_cookies = true` is set, and you open the app over HTTPS.
- [ ] For Cloudflare: Access is in front of the hostname, and `trust_proxy` is
      set only because the tunnel is the only route in.
- [ ] You have tested a backup and restore (see the admin guide).
- [ ] You know how to revoke a device (Tailscale) or a user (homestack's
      Users page) if one is lost.
