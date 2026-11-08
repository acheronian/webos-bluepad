//! The BluePad connection daemon: owns the connection layer at runtime,
//! whether or not the UI is open (spec: Privilege-tiered architecture).
//!
//! The supervisor (bluepad-core) is pure synchronous logic; this shell
//! owns timers and I/O: a Unix-socket seam speaking JSON subscribe/notify,
//! a poll loop driving the supervisor's time-driven work, and the request
//! dispatcher (the Facade whose method set is the jailed UI's capability
//! surface).

pub mod hosted;
pub mod logging;

use bluepad_core::connection::Supervisor;
use bluepad_core::diagnostics::{DiagnosticsBundle, DiagnosticsFile};
use bluepad_core::protocol::{
    ClientMessage, Notification, PairingProgress, PrivilegeTier, Request, Response,
    ServerMessage, PROTOCOL_VERSION,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, watch};

/// What the request dispatcher needs besides the supervisor: the daemon's
/// own version and the tier-homed dir its rotated logs live in (the
/// diagnostics bundle reads them at export time).
#[derive(Clone, Debug)]
pub struct DaemonInfo {
    pub version: String,
    pub logs_dir: PathBuf,
}

impl DaemonInfo {
    pub fn new(version: impl Into<String>, logs_dir: impl Into<PathBuf>) -> DaemonInfo {
        DaemonInfo {
            version: version.into(),
            logs_dir: logs_dir.into(),
        }
    }
}

/// Assemble the anonymized diagnostics bundle: the supervisor's manifest
/// (environment snapshot, health counters, pseudonymized registry) plus
/// the rotated daemon logs (spec: Diagnostics and observability).
pub fn assemble_bundle(supervisor: &mut Supervisor, info: &DaemonInfo) -> DiagnosticsBundle {
    let generated_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let manifest = supervisor.bundle_manifest(&info.version, generated_unix);
    let known = supervisor.registry_addresses();
    // Rotated logs: bluepad.log, .1, .2 — newest first, absent ones
    // skipped, MACs scrubbed (no address survives in the clear).
    let mut files = Vec::new();
    for (file, bundle_name) in [
        ("bluepad.log", "logs/bluepad-daemon.log"),
        ("bluepad.log.1", "logs/bluepad-daemon.log.1"),
        ("bluepad.log.2", "logs/bluepad-daemon.log.2"),
    ] {
        if let Ok(content) = std::fs::read(info.logs_dir.join(file)) {
            files.push(DiagnosticsFile {
                name: bundle_name.into(),
                content: bluepad_core::diagnostics::scrub_macs(&content, &known),
            });
        }
    }
    DiagnosticsBundle { manifest, files }
}

/// Default poll cadence: drives health checks, backoff, pairing windows.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The shared supervisor behind the seam.
pub type SharedSupervisor = Arc<Mutex<Supervisor>>;

/// Handle one request against the supervisor (the Facade). Public so the
/// seam tests can drive the dispatcher directly as well as through the
/// socket.
pub fn dispatch(supervisor: &SharedSupervisor, info: &DaemonInfo, request: Request) -> Response {
    let mut sup = supervisor.lock().unwrap();
    match request {
        Request::Hello { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                return Response::VersionMismatch {
                    daemon_protocol_version: PROTOCOL_VERSION,
                };
            }
            Response::Hello {
                daemon_version: info.version.clone(),
                protocol_version: PROTOCOL_VERSION,
                tier: sup.tier(),
                capabilities: sup.capabilities(),
            }
        }
        Request::GetState => Response::State(sup.state()),
        Request::StartPairing => match sup.start_pairing(std::time::Instant::now()) {
            Ok(()) => Response::Ok,
            Err(failure) => Response::Error {
                message: format!("{}: {}", failure.reason.label(), failure.detail),
            },
        },
        Request::CancelPairing => {
            sup.cancel_pairing(std::time::Instant::now());
            Response::Ok
        }
        Request::ForgetPad { address } => match sup.forget(&address) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Request::AdoptPaired { address } => {
            match sup.adopt_paired(&address, std::time::Instant::now()) {
                Ok(_identity) => Response::Ok,
                Err(failure) => Response::Error {
                    message: format!("{}: {}", failure.reason.label(), failure.detail),
                },
            }
        }
        Request::SetPadSettings { address, settings } => {
            match sup.set_pad_settings(&address, settings) {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::SetGlobalEnabled { enabled } => match sup.set_global_enabled(enabled) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Request::RumbleTest { address } => match sup.rumble_test(&address) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        }
        Request::GetDiagnostics { address } => match sup.diagnostics(&address) {
            Some(readings) => Response::Diagnostics(readings),
            None => Response::Error {
                message: "pad diagnostic is available for BluePad sessions only".into(),
            },
        },
        Request::GetSessionLog { address } => match sup.session_log(&address) {
            Some(entries) => Response::SessionLog(entries),
            None => Response::Error {
                message: "the session log is available for BluePad sessions only".into(),
            },
        },
        Request::ExportDiagnostics => {
            let bundle = assemble_bundle(&mut sup, info);
            Response::DiagnosticsBundle(bundle)
        }
        Request::Subscribe | Request::Unsubscribe => Response::Ok,
    }
}

/// The running daemon: the socket seam plus the poll loop.
pub struct Daemon {
    supervisor: SharedSupervisor,
    socket_path: PathBuf,
    notifications: broadcast::Sender<Notification>,
    poll_interval: Duration,
    info: DaemonInfo,
    /// The graceful-stop trigger: sending `true` stops the socket
    /// server, ends the poll loop, and closes every session before
    /// `run` returns — the jailed tier's while-app-open lifetime
    /// (issue #31).
    shutdown: watch::Sender<bool>,
}

impl Daemon {
    /// Prepare the socket seam. The socket lives in the app dir so the
    /// jailed UI can reach it without privileges; a previous daemon's
    /// socket (postinst restarts, crashes) is replaced. `logs_dir` is the
    /// tier-homed dir the rotated logs live in — the diagnostics bundle
    /// reads them at export time.
    pub fn bind(
        supervisor: Supervisor,
        socket_path: impl Into<PathBuf>,
        logs_dir: impl Into<PathBuf>,
    ) -> std::io::Result<Daemon> {
        Daemon::bind_shared(Arc::new(Mutex::new(supervisor)), socket_path, logs_dir)
    }

    /// Bind with an already-shared supervisor (the seam tests drive the
    /// same supervisor the daemon serves).
    pub fn bind_shared(
        supervisor: SharedSupervisor,
        socket_path: impl Into<PathBuf>,
        logs_dir: impl Into<PathBuf>,
    ) -> std::io::Result<Daemon> {
        let socket_path = socket_path.into();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        if let Some(dir) = socket_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (notifications, _) = broadcast::channel(256);
        let (shutdown, _) = watch::channel(false);
        Ok(Daemon {
            supervisor,
            socket_path,
            notifications,
            poll_interval: DEFAULT_POLL_INTERVAL,
            info: DaemonInfo::new(env!("CARGO_PKG_VERSION"), logs_dir),
            shutdown,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The graceful-stop trigger (see `Daemon::shutdown`): the jailed
    /// tier's UI holds a clone and fires it on app exit.
    pub fn shutdown_trigger(&self) -> watch::Sender<bool> {
        self.shutdown.clone()
    }

    /// Run until the process ends: the poll loop plus the socket server.
    pub async fn run(self) -> std::io::Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;

        // Daemon start is a reconcile: rebuild the pad machines from the
        // registry and schedule staggered reconnects (crash recovery is
        // reconcile-against-the-radio, never snapshot-restore).
        {
            let supervisor = self.supervisor.clone();
            let _ = tokio::task::spawn_blocking(move || {
                supervisor
                    .lock()
                    .unwrap()
                    .reconcile(std::time::Instant::now());
            })
            .await;
        }

        // The poll loop drives the supervisor off the async runtime
        // (transports block on real I/O) and broadcasts what happened.
        let poll_supervisor = self.supervisor.clone();
        let poll_tx = self.notifications.clone();
        let poll_interval = self.poll_interval;
        let poll_loop = tokio::task::spawn(async move {
            let mut tick = tokio::time::interval(poll_interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let supervisor = poll_supervisor.clone();
                let tx = poll_tx.clone();
                let outcome = tokio::task::spawn_blocking(move || {
                    let mut sup = supervisor.lock().unwrap();
                    sup.poll(std::time::Instant::now())
                })
                .await;
                if let Ok(outcome) = outcome {
                    for notification in outcome.notifications {
                        tracing::debug!("{notification:?}");
                        let _ = tx.send(notification);
                    }
                }
            }
        });

        let supervisor = self.supervisor.clone();
        let notifications = self.notifications.clone();
        let info = self.info.clone();
        let mut shutdown = self.shutdown.subscribe();
        let server = tokio::task::spawn(async move {
            // A stop that raced the run's start is still a stop.
            if *shutdown.borrow() {
                return Ok(());
            }
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = match accepted {
                            Ok(ok) => ok,
                            Err(e) => return Err(e),
                        };
                        let supervisor = supervisor.clone();
                        let notifications = notifications.clone();
                        let info = info.clone();
                        tokio::task::spawn(async move {
                            serve_connection(stream, supervisor, notifications, info).await;
                        });
                    }
                    _ = shutdown.changed() => return Ok(()),
                }
            }
        });

        let result = server.await;
        poll_loop.abort();
        // The layer's lifetime end: every session closes — the input node
        // is destroyed on leaving Connected, no ghost input (issue #31).
        {
            let supervisor = self.supervisor.clone();
            let _ = tokio::task::spawn_blocking(move || {
                supervisor.lock().unwrap().close_all_sessions();
            })
            .await;
        }
        // The socket goes away with the layer — a stale socket file would
        // shadow the next daemon's reachability probe.
        let _ = std::fs::remove_file(&self.socket_path);
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(std::io::Error::other(
                format!("server task: {e}"),
            )),
        }
    }
}

/// Serve one UI connection: request/response plus subscribed notifications,
/// framed as JSON lines.
async fn serve_connection(
    stream: UnixStream,
    supervisor: SharedSupervisor,
    notifications: broadcast::Sender<Notification>,
    info: DaemonInfo,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut subscription: Option<broadcast::Receiver<Notification>> = None;

    loop {
        // Read a request or forward a notification — whichever comes first.
        tokio::select! {
            notification = async {
                match subscription.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(notification) = notification {
                    if write_message(&mut write_half, &ServerMessage::Notification(notification))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            read = lines.next_line() => {
                match read {
                    Ok(Some(request_line)) => {
                        if request_line.trim().is_empty() {
                            continue;
                        }
                        let message: ClientMessage = match serde_json::from_str(&request_line) {
                            Ok(m) => m,
                            Err(e) => {
                                let response = ServerMessage::Response {
                                    id: 0,
                                    response: Response::Error {
                                        message: format!("unparseable message: {e}"),
                                    },
                                };
                                if write_message(&mut write_half, &response).await.is_err() {
                                    break;
                                }
                                continue;
                            }
                        };
                        let ClientMessage::Request { id, request } = message;
                        // Subscription bookkeeping (Observer).
                        match request {
                            Request::Subscribe => {
                                subscription = Some(notifications.subscribe());
                                let message = ServerMessage::Response { id, response: Response::Ok };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                            Request::Unsubscribe => {
                                subscription = None;
                                let message = ServerMessage::Response { id, response: Response::Ok };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                            Request::StartPairing => {
                                let response =
                                    dispatch(&supervisor, &info, Request::StartPairing);
                                if matches!(response, Response::Ok) {
                                    // The pairing sheet opens on the Discovering
                                    // phase (modal, bounded).
                                    let _ = notifications.send(Notification::PairingProgress(
                                        PairingProgress::Discovering,
                                    ));
                                }
                                let message = ServerMessage::Response { id, response };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                            Request::ForgetPad { address } => {
                                let response = dispatch(
                                    &supervisor,
                                    &info,
                                    Request::ForgetPad { address },
                                );
                                if matches!(response, Response::Ok) {
                                    // Forget is the only removal — the pad
                                    // disappearing is a notification, not a
                                    // poll.
                                    let _ = notifications
                                        .send(Notification::PadRemoved { address });
                                }
                                let message = ServerMessage::Response { id, response };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                            Request::SetPadSettings { address, settings } => {
                                let response = dispatch(
                                    &supervisor,
                                    &info,
                                    Request::SetPadSettings { address, settings },
                                );
                                if matches!(response, Response::Ok) {
                                    // Live-apply is visible: the pad card
                                    // re-renders from the seam.
                                    let status = supervisor
                                        .lock()
                                        .unwrap()
                                        .state()
                                        .pads
                                        .into_iter()
                                        .find(|p| p.address == address);
                                    if let Some(status) = status {
                                        let _ = notifications
                                            .send(Notification::PadChanged(status));
                                    }
                                }
                                let message = ServerMessage::Response { id, response };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                            other => {
                                let response = dispatch(&supervisor, &info, other);
                                let message = ServerMessage::Response { id, response };
                                if write_message(&mut write_half, &message).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
    }
}

async fn write_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    message: &ServerMessage,
) -> std::io::Result<()> {
    let line = serde_json::to_string(message)?;
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

/// The tier the daemon actually probed — never assumed from the install
/// method (glossary: Privilege tier). The capability probe runs at startup:
/// the stock adapter answering means the privileged tier.
pub fn probe_tier(stock_adapter_reachable: bool) -> PrivilegeTier {
    if stock_adapter_reachable {
        PrivilegeTier::Root
    } else {
        PrivilegeTier::Jailed
    }
}
