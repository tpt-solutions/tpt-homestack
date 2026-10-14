# Plugin index format (proposed)

Status: **the reader is built.** Set `catalog_url` in config.toml and the admin
**Admin · Plugins** page lists the entries, with a plain-language line for each
ability. Installing one downloads it over HTTPS, checks the SHA-256 (and the
signature when the entry names one), and then uses the normal install with the
approvals the admin ticks. Publishing an index is outside the current scope; this
document is the format that a publisher writes against.

## Why an index

An admin should be able to see what a plugin does, who published it and whether it is
signed, before installing it. An index is a plain JSON file that lists packages. The
server never runs code from an index: it downloads a package, checks its checksum and
signature, and then uses the normal install path with the admin's approvals.

## Shape

```json
{
  "format": 1,
  "name": "Example index",
  "publisher": "Example Publisher",
  "updated": "2026-10-14",
  "plugins": [
    {
      "id": "logbook",
      "name": "Logbook",
      "version": "0.1.0",
      "description": "Keep a dated log of entries with labels.",
      "package": "https://example.org/packages/logbook-0.1.0.tptp",
      "sha256": "<64 hex characters: the package file's SHA-256>",
      "signature": "https://example.org/packages/logbook-0.1.0.tptp.sig",
      "signed_by": "<64 hex characters: the publisher's public key>",
      "permissions": ["db", "kv"],
      "min_server": "0.1.0"
    }
  ]
}
```

## Rules

- `format` is `1`. A reader refuses other values.
- `id` follows the plugin id rules; `version` is the package's own version.
- `permissions` lists what the package requests, so the admin sees it before the
  download. The package's manifest is the authority: a mismatch refuses the install.
- `sha256` must match the downloaded bytes. `signature` and `signed_by` are optional;
  a server that requires signatures (`require_signed_plugins`) refuses entries without
  them.
- Entries are never run. Installing one takes the same steps as the **Admin · Plugins**
  page: approvals, then install.
- Descriptions are text and are shown escaped.
