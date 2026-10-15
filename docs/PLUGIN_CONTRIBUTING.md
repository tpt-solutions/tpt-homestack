# Contributing a plugin

Plugins extend tpt-homestack without changing the server. This page is for people
who write one, whether for their own household or to share.

## Start from the template

```sh
tpt-home new --plugin my_plugin
```

This creates `plugins/my_plugin` with a manifest, a small WASM SDK crate and a table
with list, add and delete routes. The id may use lowercase letters, digits and
underscores.

Other runtimes are described in [PLUGINS.md](PLUGINS.md). The JavaScript runtime is
the easiest to read; [AI_PLUGIN_GUIDE.md](AI_PLUGIN_GUIDE.md) shows a complete
script.

## Rules a plugin must follow

1. **Ask for the least.** Declare only the permissions the plugin uses in
   `[permissions]`. The admin approves each one, and anything not approved is refused
   at runtime.
2. **Name your own tables.** Database tables must start with the plugin id and an
   underscore (`my_plugin_items`). The host refuses anything else.
3. **Declare outbound hosts.** HTTP is allowed only to the `http_hosts` you list. Do
   not add hosts you do not need.
4. **Make the permissions readable.** The admin sees a plain-language sentence for
   each permission. Write your manifest descriptions so they make sense next to it.
5. **Handle bad input.** Validate every field and return a clear error. Never trust
   request bodies, query strings or webhook data.
6. **Keep secrets out of the package.** Notification channel URLs and hook secrets
   belong to the admin's configuration, not the plugin.
7. **Use the project licence.** Your code is `MIT OR Apache-2.0` like the rest of the
   project, with the SPDX header used in other source files.

## Build, sign and test

```sh
tpt-home plugin build plugins/my_plugin
tpt-home plugin keygen --out publisher.key      # once; keep publisher.key private
tpt-home plugin sign my_plugin-0.1.0.tptp --key publisher.key
```

Signing is optional, but it lets an admin trust your releases. Publish the public key
(the `publisher.key.pub` file, one line) and the `.sig` file next to the package. An
admin adds the public key to `trusted_plugin_keys` in `config.toml`.

Before you share a plugin, test it the way an admin will meet it:

- install it with its permissions approved, and check it works;
- install it with one permission left unapproved, and check that the plugin fails
  safely;
- try bad input on each form and route;
- remove it, once keeping its data and once deleting it.

The repository's own plugins have tests in `crates/tpt-home-plugins/tests/` (for
example `logbook_plugin.rs`, `records_plugin.rs`, `sensor_alerts.rs`). Follow the same
pattern.

## Submitting

1. Open a pull request with the plugin under `plugins/<id>/`, its tests, and a short
   README that says what it does and which permissions it needs.
2. Run `cargo test --workspace` and `cargo fmt --check`.
3. Describe anything the plugin sends off the machine. A reviewer looks for that first.
