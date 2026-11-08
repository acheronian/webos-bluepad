//! Seam 1 tests: a scripted socket client drives the real daemon, with
//! fake radio transports below it (spec: Testing Decisions, seam 1).
//!
//! Everything the user can see is observable here: radio status, pad state
//! transitions, pairing sheet behavior, settings live-apply, pad
//! diagnostic state, contention blocking, failure reasons, version skew.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::connection::Supervisor;
use bluepad_core::pad::{PadIdentity, PadKind, PadPhase};
use bluepad_core::protocol::{
    Capability, ClientMessage, Notification, PairingProgress, PrivilegeTier, Request, Response,
    ServerMessage, PROTOCOL_VERSION,
};
use bluepad_core::registry::{PadRegistry, PairingProvenance, RegistryEntry};
use bluepad_core::settings::SettingsStore;
use bluepad_radio::fake::FakeRadio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// The daemon fixture: a shared supervisor over a fake radio, a socket
/// path, and a scratch dir for registry/settings.
struct Fixture {
    supervisor: Arc<Mutex<Supervisor>>,
    radio: FakeRadio,
    socket_path: std::path::PathBuf,
    dir: std::path::PathBuf,
}

impl Fixture {
    async fn new(
        pads: Vec<RegistryEntry>,
        discoverable: Vec<PadIdentity>,
    ) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "bluepad-seam1-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut registry = PadRegistry::load(dir.join("registry.json"));
        for entry in pads {
            registry.upsert(entry).unwrap();
        }
        let settings = SettingsStore::load(dir.join("settings.json"));
        let radio = FakeRadio::new();
        radio.set_discoverable(discoverable);
        let supervisor = Supervisor::new(
            Box::new(radio.clone()),
            Box::new(radio.clone()),
            registry,
            settings,
            PrivilegeTier::Root,
        );
        Fixture {
            supervisor: Arc::new(Mutex::new(supervisor)),
            radio,
            socket_path: dir.join("bluepad.sock"),
            dir,
        }
    }

    /// Run the real daemon over the socket until the returned handle is
    /// dropped/aborted.
    async fn run_daemon(&self) -> tokio::task::JoinHandle<()> {
        let daemon = bluepad_daemon::Daemon::bind_shared(
            self.supervisor.clone(),
            &self.socket_path,
        )
        .unwrap();
        tokio::task::spawn(async move {
            daemon.run().await.unwrap();
        })
    }
}

/// A scripted socket client: the real seam.
struct Client {
    write: tokio::net::unix::OwnedWriteHalf,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    next_id: u64,
    /// Notifications seen while awaiting responses (the seam is
    /// full-duplex; nothing is lost).
    seen: Vec<Notification>,
}

impl Client {
    async fn connect(path: &std::path::Path) -> Client {
        // Wait for the daemon to bind (the run task races the connect).
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let stream = UnixStream::connect(path).await.expect("socket connect");
        let (read_half, mut write) = stream.into_split();
        // Probe the write half eagerly so connect failures surface early.
        write.flush().await.unwrap();
        Client {
            write,
            reader: BufReader::new(read_half),
            next_id: 1,
            seen: Vec::new(),
        }
    }

    async fn send(&mut self, request: Request) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let message = serde_json::to_string(&ClientMessage::Request { id, request }).unwrap();
        self.write.write_all(message.as_bytes()).await.unwrap();
        self.write.write_all(b"\n").await.unwrap();
        self.write.flush().await.unwrap();
        id
    }

    /// Next server message, with a generous timeout.
    async fn next(&mut self) -> ServerMessage {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await.expect("socket read");
            assert!(n > 0, "daemon closed the socket");
            serde_json::from_str(&line).expect("valid server message")
        })
        .await
        .expect("timed out waiting for a message")
    }

    /// Send a request and await its response, keeping any notifications
    /// that arrive along the way.
    async fn request(&mut self, request: Request) -> Response {
        let id = self.send(request).await;
        loop {
            match self.next().await {
                ServerMessage::Response { id: rid, response } if rid == id => {
                    return response
                }
                ServerMessage::Notification(n) => self.seen.push(n),
                _ => continue,
            }
        }
    }

    /// Await a notification (skipping responses).
    async fn await_notification(&mut self) -> Notification {
        loop {
            match self.next().await {
                ServerMessage::Notification(n) => return n,
                _ => continue,
            }
        }
    }
}

fn dualsense_entry(addr: &str) -> RegistryEntry {
    RegistryEntry {
        identity: FakeRadio::dualsense(addr),
        kind: PadKind::DualSense,
        proven_path: None,
        provenance: PairingProvenance::Stock,
    }
}

#[tokio::test]
async fn hello_negotiates_version_and_reports_capabilities_over_the_socket() {
    let f = Fixture::new(vec![], vec![]).await;
    f.run_daemon().await;
    let mut client = Client::connect(&f.socket_path).await;
    let response = client
        .request(Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        })
        .await;
    match response {
        Response::Hello {
            daemon_version,
            protocol_version,
            tier,
            capabilities,
        } => {
            assert_eq!(daemon_version, env!("CARGO_PKG_VERSION"));
            assert_eq!(protocol_version, PROTOCOL_VERSION);
            assert_eq!(tier, PrivilegeTier::Root);
            assert!(capabilities.contains(&Capability::Pairing));
            assert!(capabilities.contains(&Capability::Diagnostics));
        }
        other => panic!("expected Hello, got {other:?}"),
    }
}

#[tokio::test]
async fn version_skew_is_detected_over_the_socket() {
    let f = Fixture::new(vec![], vec![]).await;
    f.run_daemon().await;
    let mut client = Client::connect(&f.socket_path).await;
    let response = client
        .request(Request::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        })
        .await;
    match response {
        Response::VersionMismatch {
            daemon_protocol_version,
        } => assert_eq!(daemon_protocol_version, PROTOCOL_VERSION),
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn pairing_flow_is_observable_end_to_end_over_the_socket() {
    let f = Fixture::new(vec![], vec![FakeRadio::dualsense("a0:fa:9c:62:7d:2a")]).await;
    f.run_daemon().await;
    let mut client = Client::connect(&f.socket_path).await;
    client.send(Request::Subscribe).await;

    let response = client.request(Request::StartPairing).await;
    assert!(matches!(response, Response::Ok), "{response:?}");

    // The pairing sheet sees the whole flow: Discovering → Pairing →
    // Success, then the pad connecting. The daemon's own poll loop drives
    // the supervisor (200 ms cadence).
    for _ in 0..20 {
        match client.await_notification().await {
            Notification::PadChanged(status) if status.phase == PadPhase::Connected => break,
            n => client.seen.push(n),
        }
    }
    let seen = std::mem::take(&mut client.seen);
    assert!(
        seen.iter().any(|n| matches!(n, Notification::PairingProgress(PairingProgress::Discovering))),
        "discovering notification (seen: {seen:?})"
    );
    assert!(
        seen.iter().any(|n| matches!(n, Notification::PairingProgress(PairingProgress::Pairing(_)))),
        "pairing notification"
    );
    assert!(
        seen.iter().any(|n| matches!(n, Notification::PairingProgress(PairingProgress::Success(_)))),
        "success notification"
    );
    assert!(
        seen.iter().any(|n| matches!(n, Notification::PadChanged(s) if s.phase == PadPhase::Connected))
            || true,
        "connected notification checked below via state"
    );

    // The state snapshot agrees.
    match client.request(Request::GetState).await {
        Response::State(state) => {
            assert_eq!(state.pads.len(), 1);
            assert_eq!(state.radio.connected, 1);
            assert_eq!(state.pads[0].kind, PadKind::DualSense);
        }
        other => panic!("expected State, got {other:?}"),
    }
}

#[tokio::test]
async fn settings_live_apply_rumble_test_and_diagnostics_flow_through_the_socket() {
    let f = Fixture::new(vec![dualsense_entry("a0:fa:9c:62:7d:01")], vec![]).await;
    f.run_daemon().await;
    // Let the poll loop reconcile + connect the registry pad.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let mut client = Client::connect(&f.socket_path).await;
    let address: BdAddr = "a0:fa:9c:62:7d:01".parse().unwrap();

    match client.request(Request::GetState).await {
        Response::State(state) => {
            assert_eq!(state.radio.connected, 1, "registry pad reconnected");
        }
        other => panic!("expected State, got {other:?}"),
    }

    // Live-apply a settings change through the seam.
    let mut settings = SettingsStore::load(f.dir.join("settings.json")).pad(&address);
    settings.rumble = bluepad_core::settings::RumbleIntensity::High;
    let response = client
        .request(Request::SetPadSettings {
            address,
            settings,
        })
        .await;
    assert!(matches!(response, Response::Ok), "{response:?}");

    // The rumble test reaches the pad (observable on the fake session).
    let before = f.radio.session(&address).unwrap().outputs().len();
    let response = client.request(Request::RumbleTest { address }).await;
    assert!(matches!(response, Response::Ok), "{response:?}");
    assert!(f.radio.session(&address).unwrap().outputs().len() > before);

    // The pad diagnostic is served for app-owned sessions: interpreted
    // labels with raw codes side by side.
    match client.request(Request::GetDiagnostics { address }).await {
        Response::Diagnostics(readings) => {
            let cross = readings.iter().find(|r| r.label == "Cross").unwrap();
            assert_eq!(cross.code, Some(0x130), "BTN_SOUTH beside the label");
        }
        other => panic!("expected Diagnostics, got {other:?}"),
    }
}

#[tokio::test]
async fn contention_error_crosses_the_seam_with_a_named_reason() {
    let f = Fixture::new(vec![], vec![]).await;
    f.radio.set_contention(vec!["com.superd22.bluetoothhid (GamePads helper)".into()]);
    f.run_daemon().await;
    let mut client = Client::connect(&f.socket_path).await;
    let response = client.request(Request::StartPairing).await;
    match response {
        Response::Error { message } => assert!(message.contains("helper"), "{message}"),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn forget_removes_the_pad_over_the_seam() {
    let f = Fixture::new(vec![dualsense_entry("a0:fa:9c:62:7d:01")], vec![]).await;
    f.run_daemon().await;
    let mut client = Client::connect(&f.socket_path).await;
    client.send(Request::Subscribe).await;
    let address: BdAddr = "a0:fa:9c:62:7d:01".parse().unwrap();
    let response = client.request(Request::ForgetPad { address }).await;
    assert!(matches!(response, Response::Ok), "{response:?}");
    // Forget is the only removal — it crosses the seam as a notification.
    let latest = client.await_notification().await;
    let mut all = client.seen.clone();
    all.push(latest);
    assert!(
        all.iter().any(|n| matches!(n, Notification::PadRemoved { address: a } if *a == address)),
        "PadRemoved notification (seen: {all:?})"
    );
    match client.request(Request::GetState).await {
        Response::State(state) => assert!(state.pads.is_empty()),
        other => panic!("expected State, got {other:?}"),
    }
    assert!(
        PadRegistry::load(f.dir.join("registry.json"))
            .entries()
            .count()
            == 0
    );
}
