use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand};
use mouseshare_layout::LayoutConfig;

/// Parses a single `--connect` value of the form `screen_id=host:port`.
fn parse_connect_override(s: &str) -> Result<(String, SocketAddr), String> {
    let (id, addr) = s
        .split_once('=')
        .ok_or_else(|| format!("expected screen_id=host:port, got {s:?}"))?;
    let addr: SocketAddr = addr
        .parse()
        .map_err(|e| format!("invalid address {addr:?} for screen {id:?}: {e}"))?;
    Ok((id.to_string(), addr))
}

#[derive(Parser)]
#[command(
    name = "mouseshare",
    about = "LAN mouse sharing (MVP: mouse-only, X11)"
)]
struct Cli {
    /// Path to the layout TOML config (see layout.example.toml).
    #[arg(long)]
    config: PathBuf,

    #[command(subcommand)]
    role: Role,
}

#[derive(Subcommand)]
enum Role {
    /// Runs on the machine that owns the physical mouse.
    Controller {
        /// Explicit address for one remote screen, as screen_id=host:port
        /// (e.g. --connect office-pc=192.168.1.20:7878). Repeat for every
        /// remote screen that shouldn't rely on mDNS auto-discovery; any
        /// remote screen in --config without a matching --connect is
        /// auto-discovered on the LAN instead.
        #[arg(long = "connect", value_parser = parse_connect_override)]
        connect: Vec<(String, SocketAddr)>,

        /// How long to wait for mDNS replies when auto-discovering (only
        /// relevant for remote screens without a --connect override).
        #[arg(long, default_value_t = mouseshare_core::DEFAULT_DISCOVER_TIMEOUT.as_secs())]
        discover_timeout_secs: u64,
    },
    /// Runs on the machine that receives forwarded mouse movement.
    Target {
        /// Address to listen on, e.g. 0.0.0.0:7878
        #[arg(long)]
        listen: SocketAddr,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let layout = LayoutConfig::from_toml_file(&cli.config)?;

    match cli.role {
        Role::Controller {
            connect,
            discover_timeout_secs,
        } => {
            let timeout = Duration::from_secs(discover_timeout_secs);
            let overrides: HashMap<String, SocketAddr> = connect.into_iter().collect();
            let targets =
                mouseshare_core::resolve_target_addrs(&layout, &overrides, timeout).await?;
            let peer_status = Arc::new(Mutex::new(HashMap::new()));
            mouseshare_core::run_controller(layout, targets, peer_status).await
        }
        Role::Target { listen } => {
            let controller_status = Arc::new(Mutex::new(None));
            mouseshare_core::run_target(layout, listen, controller_status).await
        }
    }
}
