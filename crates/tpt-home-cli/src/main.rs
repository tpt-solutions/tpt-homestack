// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home` — the one binary that runs and manages a tpt-homestack
//! instance. The official blueprints are compiled in here; a custom build
//! adds more `tpt-home-bp-*` dependencies and they self-register.

mod backup;
mod remote;
mod scaffold;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};

use tpt_home_core::auth::user::{self, Role};
use tpt_home_core::config::Config;

/// Link anchors: the linker only pulls archive members that resolve a
/// referenced symbol, and blueprint registration data lives in statics the
/// rest of the binary never calls. Taking the address of each blueprint's
/// `__link_anchor` keeps its registration object (routes, migrations, UI,
/// startup tasks) in the binary. Add an entry for every compiled-in
/// blueprint; `tpt-home new` does this for the blueprints it creates.
#[allow(dead_code)]
static BLUEPRINT_LINK_ANCHORS: &[fn() -> &'static str] = &[
    tpt_home_bp_notes::__link_anchor,
    tpt_home_bp_drop::__link_anchor,
    tpt_home_bp_track::__link_anchor,
    tpt_home_plugins::__link_anchor,
    tpt_home_mcp::__link_anchor,
];

#[derive(Parser)]
#[command(
    name = "tpt-home",
    version = tpt_home_core::VERSION,
    about = "tpt-homestack: one binary, one file, zero bloat, infinite extensibility",
    after_help = "Blueprints are compiled in. Create new ones with `tpt-home new <name>`."
)]
struct Cli {
    /// Data directory (default `./data`, env `TPT_HOME_DATA_DIR`).
    #[arg(long, global = true)]
    data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a data directory: config, database, migrations, admin user.
    Init {
        /// Admin username (default `admin`).
        #[arg(long)]
        admin_user: Option<String>,
        /// Admin password (default: random, printed once).
        #[arg(long, env = "TPT_HOME_ADMIN_PASSWORD")]
        admin_password: Option<String>,
    },
    /// Apply pending migrations and exit. A copy of the database is kept first.
    Migrate {
        /// Only list the pending migrations; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the server (foreground).
    Run {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Scaffold a new blueprint crate (or, with --plugin, a WASM plugin) and
    /// register it in the workspace.
    New {
        /// Blueprint or plugin id (lowercase letters/digits/underscore).
        name: String,
        /// Starter code to generate.
        #[arg(long, value_enum, default_value_t = scaffold::Template::Basic, conflicts_with = "plugin")]
        template: scaffold::Template,
        /// Scaffold a WASM plugin in `plugins/<name>` instead of a compiled-in blueprint.
        #[arg(long)]
        plugin: bool,
    },
    /// Manage user accounts.
    User {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    /// Serve MCP over stdio for a local AI agent (the HTTP endpoint is POST /api/mcp).
    Mcp {
        /// API token whose scopes decide what the agent can reach.
        #[arg(long, env = "TPT_HOME_MCP_TOKEN")]
        token: String,
    },
    /// Manage API tokens for scripts, agents and external plugins.
    Token {
        #[command(subcommand)]
        cmd: TokenCmd,
    },
    /// Back up the database and uploads to a timestamped folder.
    Backup {
        /// Output directory (default `<data_dir>/backups`).
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
    },
    /// Remote access: check clients and set up Tailscale or Cloudflare.
    Remote {
        #[command(subcommand)]
        cmd: RemoteCmd,
    },
    /// Restore a database backup (server must be stopped).
    Restore {
        /// Path to the `tpt-<timestamp>.db` produced by `backup`.
        backup: std::path::PathBuf,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Generate shell completions.
    Completions { shell: Shell },
    /// Build, install and manage plugins (WASM, JavaScript and external).
    Plugin {
        #[command(subcommand)]
        cmd: PluginCmd,
    },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a token. The secret is printed once; store it then.
    Create {
        /// Owner of the token (its permissions are the owner's).
        username: String,
        /// Label for the token.
        #[arg(long)]
        name: String,
        /// Scope: `api` or `plugin:<id>`. Repeat for several.
        #[arg(long = "scope", required = true)]
        scopes: Vec<String>,
        /// Lifetime in days (default: never expires).
        #[arg(long)]
        days: Option<u64>,
    },
    /// List tokens, optionally for one user.
    List {
        #[arg(long)]
        username: Option<String>,
    },
    /// Revoke a token by id (see `token list`).
    Revoke { id: i64 },
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// Show which remote-access clients are installed and signed in.
    Status,
    /// Explain the steps for a provider and change config.toml after confirmation.
    Setup {
        #[arg(long, value_enum)]
        provider: remote::Provider,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum UserCmd {
    /// Add a user.
    Add {
        username: String,
        /// Password (default: random, printed once).
        #[arg(long, env = "TPT_HOME_USER_PASSWORD")]
        password: Option<String>,
        /// Make this user an admin.
        #[arg(long)]
        admin: bool,
    },
    /// List users.
    List,
    /// Remove a user.
    Remove {
        username: String,
        #[arg(long)]
        yes: bool,
    },
    /// Reset a user's password.
    ResetPassword {
        username: String,
        /// New password (default: random, printed once).
        #[arg(long, env = "TPT_HOME_USER_PASSWORD")]
        password: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginCmd {
    /// List installed plugins (reads `<data_dir>/plugins`).
    List,
    /// Pack a plugin directory (`plugin.toml` + entry + `assets/` + `src/`)
    /// into a `.tptp` package.
    Build {
        /// Plugin source directory.
        dir: std::path::PathBuf,
        /// Output file (default `<id>-<version>.tptp` in the current directory).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Install a `.tptp` package. Same rules as `POST /api/plugins/install`:
    /// only the permissions in `--approve` are granted.
    Install {
        path: std::path::PathBuf,
        /// Comma-separated permissions to approve, e.g. `db,kv`.
        #[arg(long, default_value = "")]
        approve: String,
        /// Enable after install. Without it, an upgrade keeps the current state.
        #[arg(long)]
        enable: bool,
        /// Signature file (`<package>.sig`) from `plugin sign`. Checked against
        /// `trusted_plugin_keys` in config.toml.
        #[arg(long)]
        signature: Option<std::path::PathBuf>,
    },
    /// Create a signing key for plugin packages. Writes the secret to `--out`
    /// (refusing to overwrite) and the public key to `<out>.pub`.
    Keygen {
        /// Where to write the secret key.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Sign a `.tptp` package with a secret key from `plugin keygen`. Writes
    /// `<package>.sig` next to it.
    Sign {
        /// The package to sign.
        package: std::path::PathBuf,
        /// Secret key file from `plugin keygen`.
        #[arg(long)]
        key: std::path::PathBuf,
    },
    /// Enable an installed plugin.
    Enable { id: String },
    /// Disable an installed plugin. Its data is kept.
    Disable { id: String },
    /// Uninstall a plugin. Dropping its data needs `--yes`.
    Remove {
        id: String,
        /// Keep the plugin's data tables and key/value entries.
        #[arg(long)]
        keep_data: bool,
        /// Confirm dropping the plugin's data (not needed with `--keep-data`).
        #[arg(long)]
        yes: bool,
    },
}

fn random_password() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn load_config(data_dir: Option<std::path::PathBuf>) -> anyhow::Result<Config> {
    Config::load(data_dir)
}

fn main() -> anyhow::Result<()> {
    // A script worker runs one JavaScript call and exits. It must not parse
    // CLI arguments or start the async runtime, so check for it first.
    #[cfg(feature = "js")]
    if std::env::args().nth(1).as_deref() == Some(tpt_home_plugins::script_worker::WORKER_ARG) {
        std::process::exit(tpt_home_plugins::script_worker::run_worker());
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(cli_main())
}

async fn cli_main() -> anyhow::Result<()> {
    // JavaScript plugins run in worker processes of this same binary.
    #[cfg(feature = "js")]
    if let Ok(exe) = std::env::current_exe() {
        tpt_home_plugins::script_worker::set_worker_exe(exe);
    }
    let cli = Cli::parse();

    match cli.command {
        Command::Init {
            admin_user,
            admin_password,
        } => init(cli.data_dir, admin_user, admin_password).await,
        Command::Migrate { dry_run } => migrate(cli.data_dir, dry_run).await,
        Command::Run { host, port } => {
            let mut cfg = load_config(cli.data_dir)?;
            if let Some(h) = host {
                cfg.host = h;
            }
            if let Some(p) = port {
                cfg.port = p;
            }
            tpt_home_api::serve(cfg).await
        }
        Command::New {
            name,
            template,
            plugin,
        } => {
            if plugin {
                scaffold::new_plugin(&name)
            } else {
                scaffold::new_blueprint(&name, template)
            }
        }
        Command::User { cmd } => user_cmd(cli.data_dir, cmd).await,
        Command::Token { cmd } => token_cmd(cli.data_dir, cmd).await,
        Command::Mcp { token } => mcp_cmd(cli.data_dir, token).await,
        Command::Backup { out_dir } => backup::backup(&load_config(cli.data_dir)?, out_dir).await,
        Command::Restore { backup, yes } => {
            backup::restore(&load_config(cli.data_dir)?, &backup, yes).await
        }
        Command::Remote { cmd } => {
            let cfg = load_config(cli.data_dir)?;
            match cmd {
                RemoteCmd::Status => remote::status(&cfg),
                RemoteCmd::Setup { provider, yes } => remote::setup(&cfg, provider, yes),
            }
        }
        Command::Completions { shell } => {
            generate(
                shell,
                &mut Cli::command(),
                "tpt-home",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Command::Plugin { cmd } => match cmd {
            PluginCmd::Build { dir, out } => {
                let manifest = tpt_home_plugins::manifest::from_dir(&dir)?;
                let package = tpt_home_plugins::package::pack(&dir)?;
                let out = out.unwrap_or_else(|| {
                    std::path::PathBuf::from(format!(
                        "{}-{}.tptp",
                        manifest.plugin.id, manifest.plugin.version
                    ))
                });
                std::fs::write(&out, &package.bytes)?;
                println!("wrote {} ({} bytes)", out.display(), package.bytes.len());
                Ok(())
            }
            PluginCmd::Keygen { out } => plugin_keygen(&out),
            PluginCmd::Sign { package, key } => plugin_sign(&package, &key),
            cmd => plugin_admin(cli.data_dir, cmd).await,
        },
    }
}

/// `plugin install|enable|disable|remove`. These write to the database, which
/// is the server's source of truth, but the server keeps plugins in memory and
/// loads them at startup. A running server picks up the change on restart; use
/// the admin API for an immediate change.
/// `plugin keygen`: a new Ed25519 key. The secret is written with no
/// overwrite, and on Unix only its owner can read it.
fn plugin_keygen(out: &std::path::Path) -> anyhow::Result<()> {
    use std::io::Write;
    let (secret, public) = tpt_home_plugins::signing::generate_keypair();
    let pub_path = std::path::PathBuf::from(format!("{}.pub", out.display()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(out)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", out.display()))?;
    writeln!(file, "{secret}")?;
    std::fs::write(&pub_path, format!("{public}\n"))?;
    println!("wrote secret key {} (keep it private)", out.display());
    println!("wrote public key {}", pub_path.display());
    println!("add this to trusted_plugin_keys in config.toml: {public}");
    Ok(())
}

/// `plugin sign`: the signature of the package's exact bytes, as hex.
fn plugin_sign(package: &std::path::Path, key: &std::path::Path) -> anyhow::Result<()> {
    let secret = std::fs::read_to_string(key)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", key.display()))?;
    let archive = std::fs::read(package)?;
    let signature = tpt_home_plugins::signing::sign(&archive, &secret)?;
    let sig_path = std::path::PathBuf::from(format!("{}.sig", package.display()));
    std::fs::write(&sig_path, format!("{signature}\n"))?;
    println!("wrote {}", sig_path.display());
    Ok(())
}

async fn plugin_admin(data_dir: Option<std::path::PathBuf>, cmd: PluginCmd) -> anyhow::Result<()> {
    use tpt_home_plugins::{lifecycle, manifest::Permissions};

    let cfg = load_config(data_dir)?;
    let db = tpt_home_api::init_server(cfg.clone()).await?;
    // Enable needs the plugin loaded in this process; the others do not.
    lifecycle::load_installed(&db).await?;

    match cmd {
        PluginCmd::Install {
            path,
            approve,
            enable,
            signature,
        } => {
            let archive = std::fs::read(&path)?;
            let signature = match signature {
                Some(sig_path) => Some(std::fs::read_to_string(&sig_path)?),
                None => None,
            };
            let signed_by =
                tpt_home_plugins::signing::check_install(&archive, signature.as_deref(), &cfg)?;
            let granted = Permissions::parse_list(&approve).map_err(anyhow::Error::msg)?;
            let (staged, manifest) = lifecycle::stage_package(&archive)?;
            // Same default as the admin API: an upgrade keeps its current state.
            let enable = if enable {
                true
            } else {
                lifecycle::is_enabled(&db, &manifest.plugin.id).await?
            };
            let installed = match lifecycle::install_from_dir(&db, &staged, enable, &granted).await
            {
                Ok(installed) => installed,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&staged);
                    return Err(e);
                }
            };
            let effective = granted.intersect(&installed.permissions);
            println!(
                "installed {} {} (enabled: {enable}, approved: {})",
                installed.plugin.id,
                installed.plugin.version,
                effective.to_json()
            );
            match signed_by {
                Some(key) => println!("signature verified (key {key})"),
                None => println!("unsigned package"),
            }
            Ok(())
        }
        PluginCmd::Enable { id } => {
            lifecycle::enable(&db, &id).await?;
            println!("enabled {id}");
            Ok(())
        }
        PluginCmd::Disable { id } => {
            lifecycle::disable(&db, &id).await?;
            println!("disabled {id}");
            Ok(())
        }
        PluginCmd::Remove { id, keep_data, yes } => {
            if !keep_data && !yes {
                anyhow::bail!(
                    "removing {id} drops its data; pass --keep-data to keep it, or --yes to drop it"
                );
            }
            let dropped = lifecycle::uninstall(&db, &id, keep_data).await?;
            if keep_data {
                println!("removed {id} (data kept)");
            } else {
                println!("removed {id} (dropped {dropped} table(s) and its key/value entries)");
            }
            Ok(())
        }
        PluginCmd::List => {
            let rows: Vec<(String, String, String, bool, String)> = sqlx::query_as(
                "SELECT id, name, version, enabled, approved_permissions FROM plugins ORDER BY id",
            )
            .fetch_all(&db)
            .await?;
            if rows.is_empty() {
                println!("no plugins installed");
            }
            for (id, name, version, enabled, approved) in rows {
                let state = if enabled { "enabled" } else { "disabled" };
                println!("{id} {version} ({name}) {state} approved={approved}");
            }
            Ok(())
        }
        PluginCmd::Build { .. } | PluginCmd::Keygen { .. } | PluginCmd::Sign { .. } => {
            unreachable!("handled in main")
        }
    }
}

async fn init(
    data_dir: Option<std::path::PathBuf>,
    admin_user: Option<String>,
    admin_password: Option<String>,
) -> anyhow::Result<()> {
    let cfg = load_config(data_dir)?;
    cfg.ensure_dirs()?;
    let config_file = cfg.data_dir.join("config.toml");
    if !config_file.is_file() {
        std::fs::write(
            &config_file,
            "# tpt-homestack configuration\n\
             # host = \"127.0.0.1\"\n\
             # port = 8080\n\
             # secure_cookies = false   # set true behind HTTPS\n\
             # trust_proxy = false      # set true only behind Caddy/cloudflared\n\
             # session_ttl_hours = 72\n\
             # max_upload_mb = 512\n",
        )?;
        println!("wrote {}", config_file.display());
    }
    let db = tpt_home_api::init_server(cfg).await?;

    if user::count_users(&db).await? == 0 {
        let username = admin_user.unwrap_or_else(|| "admin".to_string());
        let generated = admin_password.is_none();
        let password = admin_password.unwrap_or_else(|| {
            let pw = random_password();
            println!("generated admin password (store it now): {pw}");
            pw
        });
        let admin = user::create_user(&db, &username, &password, Role::Admin).await?;
        if generated {
            // A generated password is only a starting point: change it at the first sign-in.
            tpt_home_core::settings::set_must_change_password(&db, admin.id, true).await?;
        }
        println!("created admin user '{}' (id {})", admin.username, admin.id);
    } else {
        println!("users already exist; skipping admin bootstrap");
    }
    println!("init complete");
    Ok(())
}

async fn migrate(data_dir: Option<std::path::PathBuf>, dry_run: bool) -> anyhow::Result<()> {
    let cfg = load_config(data_dir)?;
    let report = tpt_home_api::migrate_database(&cfg, dry_run).await?;
    if report.pending.is_empty() {
        println!("database is up to date");
    } else if dry_run {
        println!("{} migration(s) would be applied:", report.pending.len());
        for key in &report.pending {
            println!("  {key}");
        }
    } else {
        if let Some(copy) = &report.snapshot {
            println!("copy saved before upgrading: {}", copy.display());
        }
        for key in &report.applied {
            println!("applied {key}");
        }
    }
    Ok(())
}

async fn mcp_cmd(data_dir: Option<std::path::PathBuf>, token: String) -> anyhow::Result<()> {
    let cfg = load_config(data_dir)?;
    let db = tpt_home_api::init_server(cfg).await?;
    // Plugins are loaded by the server at startup; a stdio session does it here.
    tpt_home_plugins::lifecycle::load_installed(&db).await?;
    let caller = tpt_home_mcp::caller_for_token(&db, &token).await?;
    tracing::info!(user = %caller.user.username, "mcp stdio session started");
    tpt_home_mcp::run_stdio(caller).await?;
    Ok(())
}

async fn token_cmd(data_dir: Option<std::path::PathBuf>, cmd: TokenCmd) -> anyhow::Result<()> {
    use tpt_home_core::auth::api_token;
    let cfg = load_config(data_dir)?;
    let db = tpt_home_api::init_server(cfg).await?;
    match cmd {
        TokenCmd::Create {
            username,
            name,
            scopes,
            days,
        } => {
            let owner = user::get_user_by_name(&db, &username).await?;
            let ttl = days.map(|d| std::time::Duration::from_secs(d * 86_400));
            let (id, token) =
                api_token::create_api_token(&db, owner.id, &name, &scopes, ttl).await?;
            println!(
                "created token {id} '{name}' for {} (scopes: {})",
                owner.username,
                scopes.join(", ")
            );
            println!("secret (shown once, store it now): {token}");
        }
        TokenCmd::List { username } => {
            let owner_id = match username {
                Some(name) => Some(user::get_user_by_name(&db, &name).await?.id),
                None => None,
            };
            let tokens = api_token::list_api_tokens(&db, owner_id).await?;
            println!(
                "{:<4} {:<16} {:<20} {:<24} {:<12} last used",
                "id", "owner", "name", "scopes", "expires"
            );
            for t in tokens {
                let expires = t.expires_at.map_or("never".to_string(), |e| e.to_string());
                let last = t
                    .last_used_at
                    .map_or("never".to_string(), |e| e.to_string());
                println!(
                    "{:<4} {:<16} {:<20} {:<24} {:<12} {}",
                    t.id,
                    t.username,
                    t.name,
                    t.scopes.join(" "),
                    expires,
                    last
                );
            }
        }
        TokenCmd::Revoke { id } => {
            if api_token::revoke_api_token(&db, id).await? {
                println!("revoked token {id}");
            } else {
                anyhow::bail!("no token with id {id}");
            }
        }
    }
    Ok(())
}

async fn user_cmd(data_dir: Option<std::path::PathBuf>, cmd: UserCmd) -> anyhow::Result<()> {
    let cfg = load_config(data_dir)?;
    let db = tpt_home_api::init_server(cfg).await?;
    match cmd {
        UserCmd::Add {
            username,
            password,
            admin,
        } => {
            let password = password.unwrap_or_else(|| {
                let pw = random_password();
                println!("generated password (store it now): {pw}");
                pw
            });
            let role = if admin { Role::Admin } else { Role::User };
            let u = user::create_user(&db, &username, &password, role).await?;
            tpt_home_core::settings::set_must_change_password(&db, u.id, true).await?;
            println!("created {} '{}' (id {})", u.role(), u.username, u.id);
        }
        UserCmd::List => {
            let users = user::list_users(&db).await?;
            println!("{:<4} {:<24} {:<7} status", "id", "username", "role");
            for u in users {
                println!(
                    "{:<4} {:<24} {:<7} {}",
                    u.id,
                    u.username,
                    u.role(),
                    if u.disabled { "disabled" } else { "active" }
                );
            }
        }
        UserCmd::Remove { username, yes } => {
            let u = user::get_user_by_name(&db, &username).await?;
            if !yes && !confirm(&format!("delete user '{}' (id {})?", u.username, u.id))? {
                println!("aborted");
                return Ok(());
            }
            user::delete_user(&db, u.id).await?;
            println!("deleted '{}'", u.username);
        }
        UserCmd::ResetPassword { username, password } => {
            let u = user::get_user_by_name(&db, &username).await?;
            let password = password.unwrap_or_else(|| {
                let pw = random_password();
                println!("generated password (store it now): {pw}");
                pw
            });
            user::set_password(&db, u.id, &password).await?;
            // Set by an admin, so the user chooses their own at the next sign-in.
            tpt_home_core::settings::set_must_change_password(&db, u.id, true).await?;
            println!("password reset for '{}'", u.username);
        }
    }
    Ok(())
}

fn confirm(prompt: &str) -> anyhow::Result<bool> {
    println!("{prompt} [y/N]");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
