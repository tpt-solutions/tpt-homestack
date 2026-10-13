# tpt-homestack guide

Two parts. Part 1 is for people who sign in and use the apps. Part 2 is for the
person who runs the server. The commands in Part 2 are typed in a terminal; the
rest is done in the browser.

Written for: people who run a home server and the family who use it. No
programming is needed for either part.

---

## Part 1: using the apps

### Signing in

Open the address your administrator gave you (on the same machine it is
<http://127.0.0.1:8080>). Enter your username and password and select
**Sign in**. You stay signed in for 72 hours by default, then you sign in again.

If you enter the wrong password too many times, the account is locked for
15 minutes. Wait and try again. The lock also applies if someone else is
guessing your password, so it protects you.

To change your password, open **Settings**, enter the new password (at least
8 characters, at most 1024), and save. Changing it signs you out everywhere
else.

### Notes

**Notes** keeps short text notes with tags.

- Select **New note**, give it a title and text, and add tags separated by commas.
- Use the search box to find notes by words in the title or text, or pick a tag.
- Text can use simple formatting: `**bold**`, `` `code` ``, `# headings`,
  `- lists`, and `[links](https://example.com)`. Only web links are shown as
  links; anything else is shown as plain text.

### Drop

**Drop** holds files you want to share or move between devices.

- Select **Choose file** and then **Upload**. There is a size limit set by the
  administrator (`max_upload_mb`, 512 MB by default).
- **Share** creates a link. You can add a password and an expiry date to a share.
- Files with an expiry date are removed automatically within a minute of
  that time.

### Track

**Track** keeps your own tables of records, such as expenses or meter readings.

- Create a table and give each column a name and a kind: text, integer, real
  (a number with decimals), or bool (yes/no).
- Add rows in the table. A value of the wrong kind is refused with a message.
- **Export CSV** downloads the table so it opens in a spreadsheet. **Import
  CSV** adds rows from a file with the same column names. Headers are not
  case-sensitive. If any row in the file is bad, nothing is added and the
  first problems are listed by line number.

### Apps added by the administrator

Apps (plugins) appear in the sidebar once an administrator has installed and
enabled them. They look like the built-in pages. An app can only use the
permissions the administrator approved for it.

### When something looks wrong

| You see | What it means | What to do |
|---|---|---|
| "invalid username or password" | The details did not match. | Check the spelling and the caps lock key. |
| "too many requests" on sign-in | Too many attempts from your address, or the account is locked. | Wait a minute (address limit) or 15 minutes (account lock). |
| "Content-Type must be ..." | An upload or import was sent in a form the server does not accept. | Use the page's own upload and import buttons. |
| "internal server error" | Something went wrong on the server. | Tell the administrator; the details are in the server log. |

---

## Part 2: running the server

### Install and first start

Build or download `tpt-home` (see [DEPLOYMENT.md](DEPLOYMENT.md) for the release
binary, the systemd service on Linux, and the Windows scheduled task).

```sh
tpt-home init --admin-password 'a-long-password'   # creates data/ and the first admin
tpt-home run                                        # serves on http://127.0.0.1:8080
```

`init` creates the data directory, the database, and the `admin` account. The
browser setup page at `/setup` only works until the first admin exists. Keep
the password somewhere safe; you can reset it with `tpt-home user reset-password`.

The first time the admin opens the Dashboard, the **Welcome** page
(`/welcome`) walks through three things: naming the server, adding a family
account for each person, and finishing. Remote access is not set up on that
page; run `tpt-home remote status` for what is ready. Until the admin clicks
**Finish setup**, the Dashboard keeps redirecting to this page. The page stays
at `/welcome` afterwards, for changing the name or adding accounts.

The server listens on `127.0.0.1` only by default. To reach it from other
devices on your home network, change `host` in `config.toml`, and read
[DEPLOYMENT.md](DEPLOYMENT.md) first. Remote access from outside the home is
**not yet recommended**: the security review in
[SECURITY_REVIEW.md](SECURITY_REVIEW.md) lists open items.

### Settings

Settings are read from `data/config.toml`, then from `TPT_HOME_*` environment
variables, then from command-line flags. The most useful keys:

| Key | Default | Meaning |
|---|---|---|
| `host` | `127.0.0.1` | Address to listen on. |
| `port` | `8080` | Port to listen on. |
| `session_ttl_hours` | `72` | How long a sign-in lasts. |
| `secure_cookies` | `false` | Send session cookies only over HTTPS. Turn on behind HTTPS. |
| `max_upload_mb` | `100` | Largest Drop upload. |
| `trust_proxy` | `false` | Use the `X-Forwarded-For` address for rate limits. Set only behind a proxy you control. |
| `log_level` | `info` | Logging detail. |

`tpt-home run --host ... --port ...` overrides both for one run.

### Users

Users are managed at **Admin · Users** in the browser, or from the terminal:

```sh
tpt-home user add alice --admin         # random password printed once, or --password
tpt-home user list
tpt-home user reset-password alice      # new random password, printed once
tpt-home user remove alice --yes
```

There are two roles. An **admin** can manage users, install apps, and see the
job status page. A **user** can use the apps. The server refuses to remove or
demote the last admin.

**Passwords you set for someone else.** When an admin creates an account, resets
a password, or the first start generates the admin password, that person must
choose their own password at their next sign-in. Until they do, pages redirect
to Settings and API calls return 403 with `password change required`. A
password you choose for yourself (the setup wizard's admin, or
`tpt-home init --admin-password`) is not affected.

### API tokens

Scripts, AI agents and external apps sign in with a token instead of a password.

```sh
tpt-home token create alice --name backup-script --scope api --days 90
tpt-home token list --username alice
tpt-home token revoke 3
```

The secret is printed once when created; store it then. A token with scope
`api` can use the HTTP API; a scope such as `plugin:records` limits it to one
app. A token that is out of scope gets `403`; a missing or bad token gets `401`.

### Apps (plugins)

```sh
tpt-home plugin build ./my-app           # writes my-app-<version>.tptp
tpt-home plugin install my-app-1.0.0.tptp --approve db --enable
tpt-home plugin list
tpt-home plugin enable my-app
tpt-home plugin disable my-app
tpt-home plugin remove my-app --keep-data   # or --yes to delete its data too
```

Installing shows which permissions the app asks for. An app can only use the
permissions named in `--approve`; anything else is refused at run time. The
browser install page does the same. A disabled app keeps its data.

### Background jobs

**Admin · Jobs** lists the scheduled jobs of each built-in app and plugin: when
each last ran, how long it took, whether it succeeded, and the next run. The
status is kept in memory, so it starts again after a restart.

### Backups

Back up while the server runs. The backup holds the database and the uploads.

```sh
tpt-home backup                         # writes data/backups/<time>/
```

Each backup is a folder named by the time it was made, with the database
(`tpt-<time>.db`) and a copy of the uploads inside it. Older backups are never
overwritten. Use `--out-dir` to write somewhere else.

To restore, stop the server first:

```sh
tpt-home restore data/backups/<time>/tpt-<time>.db --yes
```

Test a restore on a copy of the data before you need one. Keep backups on
another disk or machine.

**From the browser.** Admins can open **Admin · Status** to see the server's
version, uptime, database size, the last backup and any failing jobs. The
same page takes a backup now, lists backups with their restore paths, and
downloads a backup's database. The download holds the database only, not the
uploads. Uploading a database file adds it as a new backup, after checking that
it is a tpt-home database. The browser cannot restore: that still needs the
server stopped, so run `tpt-home restore` with the path the page shows.

**Scheduled backups.** Set how often to back up and how many to keep in
`config.toml` (or with environment variables):

```toml
backup_every_hours = 24   # 0 (default) turns scheduled backups off
backup_keep = 7           # the oldest are deleted; the newest is always kept
```

The first scheduled backup runs one interval after the server starts. Keep a
copy somewhere else as well: scheduled backups sit on the same disk.

### Sign-in protection

- Each address may try to sign in 20 times a minute. Setup is limited the same way.
- An account locks for 15 minutes after 10 failed attempts in a row, whichever
  address they come from. A successful sign-in clears the count.
- These counts are kept in memory, so a restart clears them.

### Troubleshooting

- **"address already in use"**: another program uses the port. Change `port`.
- **Everyone is rate limited**: the server is behind a reverse proxy and
  `trust_proxy` is off, so all visitors share one address. Set `trust_proxy = true`
  only if the proxy is the only way in.
- **An upload is refused with 415**: a script is uploading without the
  `X-TPT-Request: 1` header. Add it; the browser pages already send it.
- **The server will not start after an update**: check the log. Migrations
  only move forward. A copy of the database from before the upgrade is in
  `data/backups/upgrade/`; restore it (or a `backup`) and report
  the error.
- The log is written to standard error. Raise `log_level` to `debug` only
  while investigating.

### Where things are

| Path | Holds |
|---|---|
| `data/config.toml` | Settings |
| `data/tpt.db` | Everything except uploaded files |
| `data/uploads/` | Drop files |
| `data/plugins/` | Installed apps |
| `data/backups/<time>/` | One backup: the database and its uploads |

Back up the whole `data/` directory when the server is stopped, or use
`tpt-home backup` while it runs.
