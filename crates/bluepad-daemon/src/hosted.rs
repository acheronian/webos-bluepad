//! The jailed tier's in-process connection layer (issue #31).
//!
//! When no root daemon answers the socket, the UI process hosts the
//! supervisor itself — the stock transport only, because app-owned
//! L2CAP is unavailable without privileges — and serves the same
//! Unix-socket seam the root daemon serves. The UI then talks to it as a
//! regular client, so no screen code branches on tier.
//!
//! The layer's lifetime is the app's: [`HostedDaemon::stop`] (app exit)
//! ends the socket server and the poll loop and closes every session —
//! the input node is destroyed, no ghost input.

use crate::{Daemon, SharedSupervisor};
use bluepad_core::connection::Supervisor;
use bluepad_core::protocol::PrivilegeTier;
use bluepad_core::registry::PadRegistry;
use bluepad_core::settings::SettingsStore;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A connection layer hosted inside the UI process.
pub struct HostedDaemon {
    shutdown: tokio::sync::watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
    socket_path: PathBuf,
}

impl HostedDaemon {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// App exit: stop the connection layer and close every session —
    /// input flows only while the app is open (issue #31). Blocks until
    /// the layer has stopped.
    pub fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // The thread's end dropped the daemon (and the supervisor with
        // it) — the transports, the pairable-gate child included, went
        // with it.
    }
}

/// The jailed tier's supervisor: the stock transport only, over the
/// app-jail data dir (the env-var override wiring is the daemon main's;
/// the caller picks the dir). The tier is probed, never assumed from the
/// install method — the caller derives it from the capability probe.
pub fn jailed_supervisor(data_dir: &Path) -> Supervisor {
    let registry = PadRegistry::load(data_dir.join("registry.json"));
    let settings = SettingsStore::load(data_dir.join("settings.json"));
    Supervisor::new(
        Box::new(bluepad_radio::ls2::Ls2StockTransport::new()),
        Box::new(bluepad_radio::unavailable::UnavailableTransport),
        registry,
        settings,
        PrivilegeTier::Jailed,
    )
}

/// Host a supervisor in-process: the same Unix-socket seam, served on a
/// background single-threaded runtime. Returns once the socket is about
/// to be bound; connect with a short retry, the bind races the return.
pub fn host(
    supervisor: Supervisor,
    socket_path: impl Into<PathBuf>,
    logs_dir: impl Into<PathBuf>,
) -> std::io::Result<HostedDaemon> {
    let supervisor: SharedSupervisor = Arc::new(Mutex::new(supervisor));
    let daemon = Daemon::bind_shared(supervisor, socket_path, logs_dir)?;
    let shutdown = daemon.shutdown_trigger();
    let socket_path = daemon.socket_path().to_path_buf();
    let thread = std::thread::Builder::new()
        .name("bluepad-connection-layer".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    tracing::error!("cannot start the hosted connection layer: {e}");
                    return;
                }
            };
            if let Err(e) = runtime.block_on(async { daemon.run().await }) {
                tracing::error!("the hosted connection layer stopped: {e}");
            }
        })?;
    Ok(HostedDaemon {
        shutdown,
        thread: Some(thread),
        socket_path,
    })
}
