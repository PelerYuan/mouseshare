use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use mouseshare_config::{PairingStore, Settings};
use mouseshare_core::{Hotkey, PairingCode, TargetSpec, Telemetry};
use mouseshare_layout::LayoutConfig;

/// Parses `--connect id=host:port`.
fn parse_connect(s: &str) -> Result<(String, SocketAddr), String> {
    let (id, addr) = s
        .split_once('=')
        .ok_or_else(|| format!("expected device_id=host:port, got {s:?}"))?;
    let addr: SocketAddr = addr
        .parse()
        .map_err(|e| format!("invalid address {addr:?} for device {id:?}: {e}"))?;
    Ok((id.to_string(), addr))
}

#[derive(Parser)]
#[command(
    name = "mouseshare",
    version,
    about = "Share one mouse, keyboard and clipboard across your computers (Linux/X11)",
    long_about = "Share one mouse, keyboard and clipboard across your computers.\n\n\
        Run `mouseshare target` on every computer you want to control and note its\n\
        pairing code. Run `mouseshare controller` on the computer that has the\n\
        mouse and keyboard, giving it each target's code. Settings made in the\n\
        GUI (mouseshare-gui) are shared with this command line."
)]
struct Cli {
    /// Layout TOML (see layout.example.toml). Without it, the arrangement
    /// saved by the GUI in ~/.config/mouseshare is used.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    role: Role,
}

#[derive(Subcommand)]
enum Role {
    /// Run on the computer that owns the physical mouse and keyboard.
    Controller {
        /// Address of a target, as device_id=host:port. Targets without one
        /// are found on the LAN automatically (mDNS).
        #[arg(long = "connect", value_parser = parse_connect)]
        connect: Vec<(String, SocketAddr)>,

        /// Pairing code of a target: `CODE` for all targets, or
        /// `device_id=CODE` for one. Also read from $MOUSESHARE_PAIR_CODE,
        /// then from codes saved by the GUI.
        #[arg(long = "pair-code")]
        pair_code: Vec<String>,

        /// Emergency-return hotkey, e.g. "Ctrl+Alt+Esc".
        #[arg(long)]
        hotkey: Option<String>,

        /// Do not sync the clipboard.
        #[arg(long)]
        no_clipboard: bool,
    },
    /// Run on a computer that should be controlled by another one.
    Target {
        /// Address to listen on [default: 0.0.0.0:7878].
        #[arg(long)]
        listen: Option<SocketAddr>,

        /// Name of this device [default: from --config, else the saved
        /// device name, else the hostname].
        #[arg(long)]
        name: Option<String>,

        /// Pairing code controllers must present [default: this device's
        /// saved code, generated on first use].
        #[arg(long = "pair-code")]
        pair_code: Option<String>,

        /// Do not sync the clipboard.
        #[arg(long)]
        no_clipboard: bool,
    },
    /// Print this device's pairing code (generating one on first use).
    PairCode {
        /// Replace the saved code with a new one.
        #[arg(long)]
        new: bool,
    },
}

fn parse_code(s: &str) -> anyhow::Result<PairingCode> {
    PairingCode::parse(s).with_context(|| format!("{s:?} is not a valid pairing code"))
}

fn env_code() -> anyhow::Result<Option<PairingCode>> {
    match std::env::var("MOUSESHARE_PAIR_CODE") {
        Ok(v) if !v.trim().is_empty() => Ok(Some(parse_code(&v)?)),
        _ => Ok(None),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let settings = Settings::load();
    let mut opts = settings.session_options();

    match cli.role {
        Role::PairCode { new } => {
            let mut store = PairingStore::load();
            let code = if new {
                store.regenerate_own()
            } else {
                store.own_code()
            };
            println!("{}", code.display());
            Ok(())
        }
        Role::Target {
            listen,
            name,
            pair_code,
            no_clipboard,
        } => {
            let name = match (name, &cli.config) {
                (Some(n), _) => n,
                (None, Some(path)) => LayoutConfig::from_toml_file(path)?.local_id,
                (None, None) => settings.device_name.clone(),
            };
            let code = match pair_code {
                Some(c) => parse_code(&c)?,
                None => match env_code()? {
                    Some(c) => c,
                    None => PairingStore::load().own_code(),
                },
            };
            if no_clipboard {
                opts.clipboard = false;
            }
            let listen =
                listen.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], settings.listen_port)));
            println!(
                "Device {name:?} listening on {listen}\nPairing code: {}",
                code.display()
            );
            mouseshare_core::run_target(name, listen, code, opts, Telemetry::new()).await
        }
        Role::Controller {
            connect,
            pair_code,
            hotkey,
            no_clipboard,
        } => {
            let layout = match &cli.config {
                Some(path) => LayoutConfig::from_toml_file(path)?,
                None => settings.to_layout()?,
            };
            if let Some(hk) = hotkey {
                opts.hotkey = Some(Hotkey::parse(&hk).map_err(anyhow::Error::msg)?);
            }
            if no_clipboard {
                opts.clipboard = false;
            }

            let mut all: Option<PairingCode> = env_code()?;
            let mut per: HashMap<String, PairingCode> = HashMap::new();
            for entry in &pair_code {
                match entry.split_once('=') {
                    Some((id, c)) => {
                        per.insert(id.to_string(), parse_code(c)?);
                    }
                    None => all = Some(parse_code(entry)?),
                }
            }
            let addrs: HashMap<String, SocketAddr> = connect.into_iter().collect();
            let store = PairingStore::load();

            let mut targets = HashMap::new();
            for id in layout.remote_ids() {
                let code = per
                    .get(id)
                    .cloned()
                    .or_else(|| all.clone())
                    .or_else(|| store.peer_code(id));
                let Some(code) = code else {
                    bail!(
                        "no pairing code for {id:?}: pass --pair-code {id}=CODE \
                         (the code is shown when `mouseshare target` starts on that computer)"
                    );
                };
                let addr = addrs.get(id).copied().or_else(|| settings.addr_of(id));
                targets.insert(id.to_string(), TargetSpec { addr, code });
            }
            if targets.is_empty() {
                bail!("the layout has no remote devices to control");
            }
            mouseshare_core::run_controller(layout, targets, opts, Telemetry::new()).await
        }
    }
}
