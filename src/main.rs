use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use mouseshare_layout::LayoutConfig;

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
        /// Address of the target machine, e.g. 192.168.1.20:7878. If
        /// omitted, the target is auto-discovered on the LAN via mDNS
        /// instead (it must be the other screen configured in --config).
        #[arg(long)]
        connect: Option<SocketAddr>,

        /// How long to wait for mDNS replies when auto-discovering (only
        /// used when --connect is omitted).
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
            let target_addr =
                mouseshare_core::resolve_target_addr(&layout, connect, timeout).await?;
            mouseshare_core::run_controller(layout, target_addr).await
        }
        Role::Target { listen } => mouseshare_core::run_target(layout, listen).await,
    }
}
