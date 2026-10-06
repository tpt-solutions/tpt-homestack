// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home` — the one binary that runs and manages a tpt-homestack
//! instance. The official blueprints are compiled in here; a custom build
//! adds more `tpt-home-bp-*` dependencies and they self-register.

mod backup;
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
/// blueprint.
#[allow(dead_code)]
static BLUEPRINT_LINK_ANCHORS: [fn() -> &'static str; 3] = [
    tpt_home_bp_notes::__link_anchor,
    tpt_home_bp_drop::__link_anchor,
    tpt_home_bp_track::__link_anchor,
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
    /// Apply pending migrations and exit.
    Migrate,
    /// Run the server (foreground).
    Run {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Scaffold a new blueprint crate and register it in the workspace.
    New {
        /// Blueprint id (lowercase letters/digits/underscore).
        name: String,
    },
    /// Manage user accounts.
    User {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    /// Back up the database and uploads to a timestamped folder.
    Backup {
        /// Output directory (default `<data_dir>/backups`).
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
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
    /// Manage WASM plugins (Phase 4 — not yet implemented).
    Plugin {
        #[command(subcommand)]
        cmd: PluginCmd,
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
    /// Install a `.tptp` package (Phase 4 — not yet implemented).
    Install { path: std::path::PathBuf },
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init {
            admin_user,
            admin_password,
        } => init(cli.data_dir, admin_user, admin_password).await,
        Command::Migrate => migrate(cli.data_dir).await,
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
        Command::New { name } => scaffold::new_blueprint(&name),
        Command::User { cmd } => user_cmd(cli.data_dir, cmd).await,
        Command::Backup { out_dir } => backup::backup(&load_config(cli.data_dir)?, out_dir).await,
        Command::Restore { backup, yes } => {
            backup::restore(&load_config(cli.data_dir)?, &backup, yes).await
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
            PluginCmd::List => {
                let cfg = load_config(cli.data_dir)?;
                let dir = cfg.data_dir.join("plugins");
                if dir.is_dir() {
                    let mut found = false;
                    for entry in std::fs::read_dir(&dir)? {
                        let p = entry?.path();
                        if p.extension().is_some_and(|e| e == "tptp") {
                            println!("{}", p.display());
                            found = true;
                        }
                    }
                    if !found {
                        println!("no plugins installed in {}", dir.display());
                    }
                } else {
                    println!("no plugins directory at {}", dir.display());
                }
                Ok(())
            }
            PluginCmd::Install { .. } => {
                anyhow::bail!("the WASM plugin system is Phase 4 — not implemented yet")
            }
        },
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
             # session_ttl_hours = 72\n\
             # max_upload_mb = 512\n",
        )?;
        println!("wrote {}", config_file.display());
    }
    let db = tpt_home_api::init_server(cfg).await?;

    if user::count_users(&db).await? == 0 {
        let username = admin_user.unwrap_or_else(|| "admin".to_string());
        let password = admin_password.unwrap_or_else(|| {
            let pw = random_password();
            println!("generated admin password (store it now): {pw}");
            pw
        });
        let admin = user::create_user(&db, &username, &password, Role::Admin).await?;
        println!("created admin user '{}' (id {})", admin.username, admin.id);
    } else {
        println!("users already exist; skipping admin bootstrap");
    }
    println!("init complete");
    Ok(())
}

async fn migrate(data_dir: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let cfg = load_config(data_dir)?;
    let db = tpt_home_api::init_server(cfg).await?;
    let applied = tpt_home_core::migrate::run_all(&db).await?;
    if applied.is_empty() {
        println!("database is up to date");
    } else {
        for key in applied {
            println!("applied {key}");
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
