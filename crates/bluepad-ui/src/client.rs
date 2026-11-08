//! The UI's client facade: one struct hiding socket connect/reconnect,
//! subscription bookkeeping, and the state cache behind the methods the
//! screens call (design-patterns research: Facade, second home).
//!
//! The UI is synchronous by decision (spec: Four-crate workspace); the
//! client runs a background reader thread that fills the response map
//! and dispatches notifications to subscriber callbacks (Observer,
//! function-pointer form).

use bluepad_core::protocol::{
    AppState, ClientMessage, Notification, Request, Response, ServerMessage, PROTOCOL_VERSION,
};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Errors the client can report to the screens.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("not connected to the daemon")]
    NotConnected,
    #[error("daemon speaks an incompatible protocol version")]
    VersionMismatch,
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encoding: {0}")]
    Encoding(#[from] serde_json::Error),
}

type Pending = Arc<Mutex<HashMap<u64, Response>>>;
type Subscribers = Arc<Mutex<Vec<Box<dyn Fn(&Notification) + Send>>>>;

/// The synchronous UI client over the daemon's Unix-socket seam.
pub struct DaemonClient {
    writer: Mutex<UnixStream>,
    pending: Pending,
    subscribers: Subscribers,
    next_id: AtomicU64,
}

impl DaemonClient {
    /// Connect, start the reader, and negotiate the protocol version
    /// (the version-skew gate: Hello is the first thing every client sends).
    pub fn connect(socket_path: &str) -> Result<DaemonClient, ClientError> {
        let stream = UnixStream::connect(socket_path)?;
        let reader = stream.try_clone()?;
        let client = DaemonClient {
            writer: Mutex::new(stream),
            pending: Arc::new(Mutex::new(HashMap::new())),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            next_id: AtomicU64::new(1),
        };
        client.start_reader(reader);
        match client.request(Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        })? {
            Response::Hello { .. } => Ok(client),
            Response::VersionMismatch { .. } => Err(ClientError::VersionMismatch),
            other => Err(ClientError::Daemon(format!("unexpected hello reply: {other:?}"))),
        }
    }

    fn start_reader(&self, reader: UnixStream) {
        let pending = self.pending.clone();
        let subscribers = self.subscribers.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<ServerMessage>(&line) else {
                    continue;
                };
                match message {
                    ServerMessage::Response { id, response } => {
                        pending.lock().unwrap().insert(id, response);
                    }
                    ServerMessage::Notification(notification) => {
                        for subscriber in subscribers.lock().unwrap().iter() {
                            subscriber(&notification);
                        }
                    }
                }
            }
        });
    }

    /// Subscribe to notifications; `on_event` runs on the reader thread.
    pub fn subscribe(
        &self,
        on_event: impl Fn(&Notification) + Send + 'static,
    ) -> Result<(), ClientError> {
        self.subscribers.lock().unwrap().push(Box::new(on_event));
        self.request(Request::Subscribe)?;
        Ok(())
    }

    /// Send a request and await its response (bounded wait).
    pub fn request(&self, request: Request) -> Result<Response, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let message = serde_json::to_string(&ClientMessage::Request { id, request })?;
        {
            let mut writer = self.writer.lock().unwrap();
            writer.write_all(message.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(response) = self.pending.lock().unwrap().remove(&id) {
                return Ok(response);
            }
            if Instant::now() >= deadline {
                return Err(ClientError::NotConnected);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Convenience: the full state snapshot.
    pub fn state(&self) -> Result<AppState, ClientError> {
        match self.request(Request::GetState)? {
            Response::State(state) => Ok(state),
            other => Err(ClientError::Daemon(format!("expected state: {other:?}"))),
        }
    }
}
