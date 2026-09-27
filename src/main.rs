use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use mouseshare_layout::{ControlState, EdgeDetector, LayoutConfig};
use mouseshare_net::{Connection, NetError};
use mouseshare_protocol::Message;
use mouseshare_x11input::LocalCursor;

/// Pixels of hysteresis applied when control might return from a remote
/// screen to local, to avoid flicker right at the seam.
const REENTRY_MARGIN: i32 = 4;

/// Poll interval for the controller's local-position/capture-delta loop.
const TICK: Duration = Duration::from_millis(8);

#[derive(Parser)]
#[command(name = "mouseshare", about = "LAN mouse sharing (MVP: mouse-only, X11)")]
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
        /// Address of the target machine, e.g. 192.168.1.20:7878
        #[arg(long)]
        connect: SocketAddr,
    },
    /// Runs on the machine that receives forwarded mouse movement.
    Target {
        /// Address to listen on, e.g. 0.0.0.0:7878
        #[arg(long)]
        listen: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let layout = LayoutConfig::from_toml_file(&cli.config)?;

    match cli.role {
        Role::Controller { connect } => run_controller(layout, connect).await,
        Role::Target { listen } => run_target(layout, listen).await,
    }
}

async fn run_target(layout: LayoutConfig, listen_addr: SocketAddr) -> anyhow::Result<()> {
    let local = layout.local_screen().clone();
    let listener = mouseshare_net::listen(listen_addr).await?;
    tracing::info!(addr = %listener.local_addr()?, "target listening");

    loop {
        let mut conn = listener.accept().await?;
        let peer = match conn
            .handshake_as_listener(local.id.clone(), local.width, local.height)
            .await
        {
            Ok(peer) => peer,
            Err(e) => {
                tracing::warn!("handshake with incoming controller failed: {e}");
                continue;
            }
        };
        tracing::info!(?peer, "controller connected");

        let cursor = match LocalCursor::connect(None) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to connect to local X display: {e}");
                continue;
            }
        };

        if let Err(e) = handle_target_connection(&mut conn, &cursor).await {
            tracing::warn!("controller connection ended: {e}");
        }
    }
}

async fn handle_target_connection(
    conn: &mut Connection,
    cursor: &LocalCursor,
) -> Result<(), NetError> {
    loop {
        match conn.recv().await {
            Ok(Message::MouseMove { dx, dy }) => {
                if let Err(e) = cursor.warp_relative(dx, dy) {
                    tracing::warn!("warp_relative failed: {e}");
                }
            }
            Ok(Message::Heartbeat) => {}
            Ok(other) => tracing::warn!(?other, "unexpected message from controller"),
            Err(NetError::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

async fn run_controller(layout: LayoutConfig, target_addr: SocketAddr) -> anyhow::Result<()> {
    let local = layout.local_screen().clone();

    let mut conn = mouseshare_net::connect(target_addr).await?;
    let peer = conn
        .handshake_as_dialer(local.id.clone(), local.width, local.height)
        .await?;
    tracing::info!(?peer, "connected to target");

    let mut cursor = LocalCursor::connect(None)?;
    let mut detector = EdgeDetector::new(layout, REENTRY_MARGIN);

    loop {
        match detector.state().clone() {
            ControlState::Local => {
                let (lx, ly) = cursor.query_pointer()?;
                if let Some(transition) = detector.on_local_move(lx, ly) {
                    match transition.new_state {
                        ControlState::Remote(peer_id) => {
                            cursor.begin_capture()?;
                            tracing::info!(target = %peer_id, "control handed off to remote");
                        }
                        ControlState::Local => unreachable!(
                            "on_local_move only returns Some(..) when the state actually changed"
                        ),
                    }
                }
            }
            ControlState::Remote(_) => {
                for (dx, dy) in cursor.poll_capture_delta()? {
                    match detector.on_remote_delta(dx, dy) {
                        None => {
                            conn.send(&Message::MouseMove { dx, dy }).await?;
                        }
                        Some(transition) if transition.new_state == ControlState::Local => {
                            // This delta is the one that crossed back onto
                            // the local screen: it isn't forwarded (control
                            // is ours again), the pointer is placed exactly
                            // at the crossing point, and the OS cursor is
                            // ungrabbed. Any further deltas already queued
                            // in this same batch are relative to the old
                            // (now irrelevant) capture center and are
                            // dropped — at the tick rate used here that's at
                            // most a fraction of a pixel of local movement.
                            cursor.end_capture()?;
                            if let Some((lx, ly)) = transition.local_target {
                                cursor.warp_absolute(lx, ly)?;
                            }
                            tracing::info!("control returned to local");
                            break;
                        }
                        Some(transition) => {
                            tracing::warn!(
                                ?transition,
                                "hand-off to a different remote screen isn't supported yet \
                                 (single-peer MVP); dropping this delta"
                            );
                        }
                    }
                }
            }
        }
        tokio::time::sleep(TICK).await;
    }
}
