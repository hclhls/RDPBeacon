mod backend;
mod config;
mod scheduler;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

use config::{BackendKind, Config};

impl ValueEnum for BackendKind {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::X11, Self::Wayland, Self::Windows]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::X11 => Some(clap::builder::PossibleValue::new("x11")),
            Self::Wayland => Some(clap::builder::PossibleValue::new("wayland")),
            Self::Windows => Some(clap::builder::PossibleValue::new("windows")),
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "rdpbeacon", version, about = "Keep Omnissa Horizon Client sessions active")]
pub struct Cli {
    #[arg(short, long, global = true, value_name = "PATH", help = "Path to TOML configuration file")]
    pub config: Option<PathBuf>,

    #[arg(short, long, global = true, value_enum, help = "Display backend to use (x11, wayland, windows)")]
    pub backend: Option<BackendKind>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    /// Run the beacon daemon loop
    Run,
    /// Send a single beacon cycle immediately
    Once,
    /// Inspect environment and check Horizon window status
    Check,
}

pub fn resolve_backend_kind(
    cli_backend: Option<BackendKind>,
    cfg_backend: Option<BackendKind>,
) -> anyhow::Result<BackendKind> {
    match cli_backend.or(cfg_backend) {
        Some(kind) => Ok(kind),
        None => backend::detect_backend_kind(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            cfg!(windows),
        ),
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let cfg = Config::load(cli.config.as_deref())?;
    let backend_kind = resolve_backend_kind(cli.backend, cfg.backend)?;

    match cli.command {
        Commands::Run => {
            let stop = Arc::new(AtomicBool::new(false));
            let stop_clone = Arc::clone(&stop);
            ctrlc::set_handler(move || {
                log::info!("Received stop signal, shutting down...");
                stop_clone.store(true, Ordering::Relaxed);
            })
            .context("Error setting Ctrl-C handler")?;

            let backend = backend::make_backend(backend_kind)?;
            scheduler::run_loop(backend.as_ref(), &cfg, &stop)?;
        }
        Commands::Once => {
            let mut once_cfg = cfg.clone();
            once_cfg.idle_threshold = Duration::ZERO;
            let backend = backend::make_backend(backend_kind)?;
            let outcome = scheduler::run_cycle(backend.as_ref(), &once_cfg)?;
            match outcome {
                scheduler::CycleOutcome::Sent => {
                    log::info!("Beacon sent successfully");
                    println!("Beacon sent successfully");
                }
                scheduler::CycleOutcome::SkippedNoWindow => {
                    log::warn!("Horizon window not found");
                    println!("Beacon skipped: Horizon window not found");
                }
                scheduler::CycleOutcome::SkippedActive => {
                    log::debug!("User active; beacon skipped");
                    println!("Beacon skipped: User active");
                }
            }
        }
        Commands::Check => {
            println!("Backend: {:?}", backend_kind);
            let backend = backend::make_backend(backend_kind)?;
            let idle = backend.idle_time()?;
            println!("Idle time: {:?}", idle);
            match backend.find_horizon_window(&cfg.window_match)? {
                Some(id) => println!("Horizon window: found (ID: {id})"),
                None => println!("Horizon window: not found"),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_subcommand() {
        let cli = Cli::try_parse_from(["rdpbeacon", "run"]).unwrap();
        assert_eq!(cli.command, Commands::Run);
        assert_eq!(cli.backend, None);
        assert_eq!(cli.config, None);
    }

    #[test]
    fn parses_once_with_backend_and_config() {
        let cli = Cli::try_parse_from([
            "rdpbeacon",
            "--backend",
            "wayland",
            "--config",
            "/tmp/test.toml",
            "once",
        ])
        .unwrap();
        assert_eq!(cli.command, Commands::Once);
        assert_eq!(cli.backend, Some(BackendKind::Wayland));
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/test.toml")));
    }

    #[test]
    fn parses_check_subcommand() {
        let cli = Cli::try_parse_from(["rdpbeacon", "check"]).unwrap();
        assert_eq!(cli.command, Commands::Check);
    }

    #[test]
    fn resolve_backend_precedence() {
        // CLI flag takes highest priority over config
        let kind = resolve_backend_kind(Some(BackendKind::Windows), Some(BackendKind::X11)).unwrap();
        assert_eq!(kind, BackendKind::Windows);

        // Config takes second priority when CLI flag is None
        let kind = resolve_backend_kind(None, Some(BackendKind::Wayland)).unwrap();
        assert_eq!(kind, BackendKind::Wayland);
    }
}

