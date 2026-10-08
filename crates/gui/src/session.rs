//! Runs a controller/target session on the background runtime and lets the
//! UI poll it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use mouseshare_core::{PairingCode, SessionOptions, TargetSpec, Telemetry};
use mouseshare_layout::LayoutConfig;
use std::collections::HashMap;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

pub struct Session {
    handle: JoinHandle<()>,
    result: Arc<Mutex<Option<String>>>,
}

impl Session {
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// The error the session ended with, if it ended because of one.
    pub fn take_error(&self) -> Option<String> {
        self.result.lock().ok()?.take()
    }

    pub fn stop(self) {
        self.handle.abort();
    }
}

fn spawn(
    rt: &Runtime,
    fut: impl std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
) -> Session {
    let result = Arc::new(Mutex::new(None));
    let slot = result.clone();
    let handle = rt.spawn(async move {
        if let Err(e) = fut.await {
            tracing::error!("session ended: {e:#}");
            if let Ok(mut s) = slot.lock() {
                *s = Some(format!("{e:#}"));
            }
        }
    });
    Session { handle, result }
}

pub fn start_controller(
    rt: &Runtime,
    layout: LayoutConfig,
    targets: HashMap<String, TargetSpec>,
    opts: SessionOptions,
    tele: Telemetry,
) -> Session {
    spawn(
        rt,
        mouseshare_core::run_controller(layout, targets, opts, tele),
    )
}

pub fn start_target(
    rt: &Runtime,
    name: String,
    listen: SocketAddr,
    code: PairingCode,
    opts: SessionOptions,
    tele: Telemetry,
) -> Session {
    spawn(
        rt,
        mouseshare_core::run_target(name, listen, code, opts, tele),
    )
}
