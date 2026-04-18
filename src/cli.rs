use crate::config::{default_config_path, default_pid_path, GatewayConfig};
use crate::process::{pid_alive, read_pid, send_sighup};
use crate::self_update::{self, SelfUpdateOptions};
use crate::server;
use crate::tui;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "itadori", about = "Local Rust gateway for private APIs")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Serve {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    Validate {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    Init {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Reload,
    Tui {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    SelfUpdate {
        #[arg(long, help = "GitHub repository in owner/repo format")]
        repo: Option<String>,
        #[arg(long, help = "Release asset name to install")]
        asset: Option<String>,
        #[arg(long, env = "GITHUB_TOKEN", help = "GitHub token for private releases")]
        token: Option<String>,
        #[arg(
            long,
            help = "Install even when the latest release matches this version"
        )]
        force: bool,
    },
    ExampleConfig,
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Commands::Serve { config } => server::run(resolve_config_path(config)).await,
            Commands::Validate { config } => {
                let config = resolve_config_path(config);
                GatewayConfig::load(&config)
                    .with_context(|| format!("failed to load config from {}", config.display()))?;
                println!("config OK: {}", config.display());
                Ok(())
            }
            Commands::Init { config, force } => {
                let config = resolve_config_path(config);
                init_config(config, force)
            }
            Commands::Reload => reload_gateway(),
            Commands::Tui { config } => tui::run(resolve_config_path(config)).await,
            Commands::SelfUpdate {
                repo,
                asset,
                token,
                force,
            } => {
                self_update::run(SelfUpdateOptions {
                    repo,
                    asset,
                    token,
                    force,
                })
                .await
            }
            Commands::ExampleConfig => {
                print!("{}", GatewayConfig::example_toml());
                Ok(())
            }
        }
    }
}

fn resolve_config_path(config: Option<PathBuf>) -> PathBuf {
    config.unwrap_or_else(default_config_path)
}

fn init_config(config: PathBuf, force: bool) -> Result<()> {
    if config.exists() && !force {
        anyhow::bail!(
            "config already exists: {} (use --force to overwrite)",
            config.display()
        );
    }

    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::write(&config, GatewayConfig::example_toml())
        .with_context(|| format!("failed to write config to {}", config.display()))?;
    println!("wrote example config to {}", config.display());
    Ok(())
}

fn reload_gateway() -> Result<()> {
    let pid_path = default_pid_path();
    let pid = read_pid(&pid_path)
        .with_context(|| format!("failed to read pid file {}", pid_path.display()))?;

    if !pid_alive(pid) {
        anyhow::bail!("stale pid file: {}", pid_path.display());
    }

    send_sighup(pid)?;
    println!("sent reload signal to pid {}", pid);
    Ok(())
}
