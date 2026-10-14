# Advanced remote access: step by step

[REMOTE_ACCESS.md](REMOTE_ACCESS.md) explains the options and their trade-offs.
This page has the steps for the three advanced ones. Read the trade-offs first:
each of these either opens your router or depends on a server you run.

Before you start, finish the checklist at the end of REMOTE_ACCESS.md: a strong
admin password, and a backup you have tried.

## 1. Caddy with your own domain

Caddy gets HTTPS certificates for a domain you own and forwards requests to the
server. Your router must forward ports 80 and 443 to this computer, so the
server becomes reachable from the internet. Your internet provider must give you
a public address (not carrier-grade NAT).

1. **Point a domain at your home address.** Create an `A` record for, say,
   `home.example.com`, with your public IP address. If the address changes,
   update the record or use a dynamic DNS service.
2. **Forward ports.** On your router, forward TCP 80 and 443 to this computer's
   local address. Caddy needs port 80 to get the certificate.
3. **Set the server's config** in `config.toml` in the data directory:

   ```toml
   trust_proxy = true     # Caddy is the only route in
   secure_cookies = true  # cookies only over HTTPS
   ```

   Restart the server after changing the file.
4. **Start Caddy** with the domain, as [DEPLOYMENT.md](DEPLOYMENT.md#3-tls)
   describes:

   ```sh
   TPT_DOMAIN=home.example.com docker compose up -d caddy
   ```

5. **Check it.** Open `https://home.example.com` from a phone on mobile data (not
   your Wi-Fi). You should see the sign-in page with a padlock.
6. **Keep it safe.** Keep the admin password strong, enable backups, and watch
   the Admin · Status page for failing jobs.

## 2. Netbird

Netbird is an open-source private network, like Tailscale, with its own
coordination service. It has a free hosted tier. Your devices join a private
network, and the server listens only on that network's address.

1. **Create a Netbird account** and sign in to its dashboard.
2. **Install the Netbird client** on the server computer and on each family
   device, and run `netbird up` on each one. Sign in when asked.
3. **Find the server's Netbird address** with `netbird status`. It is an IP
   address on the Netbird network, not the Wi-Fi address.
4. **Listen on that address only.** In `config.toml`:

   ```toml
   host = "100.x.y.z"   # the Netbird address from step 3
   port = 8080
   ```

   Do not use `0.0.0.0` here: that would also listen on your home network and any
   other interface. Restart the server.
5. **Open the app** from a family device at `http://100.x.y.z:8080`. The address
   works only on devices in the same Netbird network.
6. **Optional: a name.** If your Netbird setup gives devices DNS names, use that
   name in place of the address.

## 3. Headscale

Headscale is a self-hosted replacement for Tailscale's coordination server. The
Tailscale client works with it unchanged. You run Headscale on a small server you
control, such as a cheap virtual machine. That server must be reachable from the
internet, and keeping it secure is your responsibility.

1. **Install Headscale** on the virtual machine, following the Headscale project
   documentation. Put it behind HTTPS (Caddy works well) at a name such as
   `https://headscale.example.com`.
2. **Create a user** in Headscale for your family's network, and a pre-auth key
   for each device, as the Headscale docs describe.
3. **Install the Tailscale client** on the server computer and on each family
   device. Join with your Headscale address:

   ```sh
   tailscale up --login-server https://headscale.example.com --authkey <key>
   ```

4. **Listen on the tailnet address only.** Run `tailscale ip -4` on the server and
   set `host` in `config.toml` to that address, as in the Netbird section. Restart
   the server.
5. **Open the app** from a family device at `http://<tailnet address>:8080`.

## If something does not work

- **The page does not load from another device:** check `host` (it must be an
  address that device can reach), that the server was restarted, and that the
  computer's firewall allows the port on that network.
- **Certificates fail with Caddy:** ports 80 and 443 must reach this computer from
  the internet, and the DNS record must point at the current public IP.
- **Sign-in works but cookies are not kept:** `secure_cookies = true` needs HTTPS.
  Use the HTTPS address, or set it back to `false` on a private network.
