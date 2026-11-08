//! The BluePad connection daemon: owns the connection layer at runtime,
//! whether or not the UI is open (spec: Privilege-tiered architecture).
//!
//! The supervisor (bluepad-core) is pure synchronous logic; this shell
//! owns timers and I/O: a Unix-socket seam speaking JSON subscribe/notify,
//! a poll loop driving the supervisor's time-driven work, and the request
//! dispatcher (the Facade whose method set is the jailed UI's capability
//! surface).

pub mod logging;

use bluepad_core::connection::Supervisor;
use bluepad_core::protocol::{
    ClientMessage, Notification, PairingProgress, PrivilegeTier, Request, Response,
    ServerMessage, PROTOCOL_VERSION,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

/// Default poll cadence: drives health checks, backoff, pairing windows.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The shared supervisor behind the seam.
pub type SharedSupervisor = Arc<Mutex<Supervisor>>;

/// Handle one request against the supervisor (the Facade). Public so the
/// seam tests can drive the dispatcher directly as well as through the
/// socket.
pub fn dispatch(supervisor: &SharedSupervisor, request: Request) -> Response {
    dispatch_versioned(supervisor, env!("CARGO_PKG_VERSION"), request)
}

pub fn dispatch_versioned(
    supervisor: &SharedSupervisor,
    daemon_version: &str,
    request: Request,
) -> Response {
    let mut sup = supervisor.lock().unwrap();
    match request {
        Request::Hello { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                return Response::VersionMismatch {
                    daemon_protocol_version: PROTOCOL_VERSION,
                };
            }
            Response::Hello {
                daemon_version: daemon_version.to_string(),
                protocol_version: PROTOCOL_VERSION,
                tier: sup.tier(),
                capabilities: sup.capabilities(),
            }
        }
        Request::GetState => Response::State(sup.state()),
        Request::StartPairing => match sup.start_pairing(std::time::Instant::now()) {
            Ok(()) => Response::Ok,
            Err(failure) => Response::Error {
                message: failure.reason.label().to_string(),
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
        Request::Subscribe | Request::Unsubscribe => Response::Ok,
    }
}

/// The running daemon: the socket seam plus the poll loop.
pub struct Daemon {
    supervisor: SharedSupervisor,
    socket_path: PathBuf,
    notifications: broadcast::Sender<Notification>,
    poll_interval: Duration,
    daemon_version: String,
}

impl Daemon {
    /// Prepare the socket seam. The socket lives in the app dir so the
    /// jailed UI can reach it without privileges; a previous daemon's
    /// socket (postinst restarts, crashes) is replaced.
    pub fn bind(
        supervisor: Supervisor,
        socket_path: impl Into<PathBuf>,
    ) -> std::io::Result<Daemon> {
        let socket_path = socket_path.into();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        if let Some(dir) = socket_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (notifications, _) = broadcast::channel(256);
        Ok(Daemon {
            supervisor: Arc::new(Mutex::new(supervisor)),
            socket_path,
            notifications,
            poll_interval: DEFAULT_POLL_INTERVAL,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    /// Bind with an already-shared supervisor (the seam tests drive the
    /// same supervisor the daemon serves).
    pub fn bind_shared(
        supervisor: SharedSupervisor,
        socket_path: impl Into<PathBuf>,
    ) -> std::io::Result<Daemon> {
        let socket_path = socket_path.into();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        if let Some(dir) = socket_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (notifications, _) = broadcast::channel(256);
        Ok(Daemon {
            supervisor,
            socket_path,
            notifications,
            poll_interval: DEFAULT_POLL_INTERVAL,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
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
                        let _ = tx.send(notification);
                    }
                }
            }
        });

        let supervisor = self.supervisor.clone();
        let notifications = self.notifications.clone();
        let daemon_version = self.daemon_version.clone();
        let server = tokio::task::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(ok) => ok,
                    Err(e) => return Err(e),
                };
                let supervisor = supervisor.clone();
                let notifications = notifications.clone();
                let daemon_version = daemon_version.clone();
                tokio::task::spawn(async move {
                    serve_connection(stream, supervisor, notifications, daemon_version).await;
                });
            }
        });

        let result = server.await;
        poll_loop.abort();
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
    daemon_version: String,
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
                                    dispatch_versioned(&supervisor, &daemon_version, Request::StartPairing);
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
                            other => {
                                let response =
                                    dispatch_versioned(&supervisor, &daemon_version, other);
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
/// can we reach the radio transports at all?
pub fn probe_tier(stock_adapter_reachable: bool) -> PrivilegeTier {
    if stock_adapter_reachable {
        PrivilegeTier::Root
    } else {
        PrivilegeTier::Jailed
    }
}

/// Pairing progress is pushed over the seam as it happens; the helper
/// exists so the poll loop and request path share one vocabulary.
#[allow(dead_code)]
fn pairing_notification(phase: PairingProgress) -> Notification {
    Notification::PairingProgress(phase)
}
