//! The jailed tier's in-process hosting (issue #31): the UI process
//! hosts the supervisor — stock transport only — serves the same
//! Unix-socket seam on a background runtime, and talks to it as a
//! regular client. The layer lives only while the app is open: stopping
//! it closes every session and the socket goes away.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::connection::Supervisor;
use bluepad_core::pad::{ConnectionPath, PadPhase};
use bluepad_core::protocol::{
    ClientMessage, PrivilegeTier, Request, Response, ServerMessage, PROTOCOL_VERSION,
};
use bluepad_core::registry::{PadRegistry, PairingProvenance, RegistryEntry};
use bluepad_core::settings::SettingsStore;
use bluepad_radio::fake::FakeRadio;
use bluepad_radio::unavailable::UnavailableTransport;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn scratch_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bluepad-hosted-{}-{tag}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The jailed supervisor exactly as the jailed UI builds it, but over
/// the fake stock radio: stock transport only, unavailable app-owned.
fn jailed_supervisor_over(dir: &Path, radio: &FakeRadio) -> Supervisor {
    let mut registry = PadRegistry::load(dir.join("registry.json"));
    registry
        .upsert(RegistryEntry {
            identity: FakeRadio::dualsense("a0:fa:9c:62:7d:01"),
            kind: bluepad_core::pad::PadKind::DualSense,
            proven_path: None,
            provenance: PairingProvenance::Stock,
        })
        .unwrap();
    Supervisor::new(
        Box::new(radio.clone()),
        Box::new(UnavailableTransport),
        registry,
        SettingsStore::load(dir.join("settings.json")),
        PrivilegeTier::Jailed,
    )
}

/// A regular client over the seam: JSON lines, request/response by id.
struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    fn connect(path: &Path) -> Client {
        // The hosted layer binds its socket on a background thread.
        for _ in 0..200 {
            if path.exists() {
                if let Ok(stream) = UnixStream::connect(path) {
                    let reader = BufReader::new(stream.try_clone().unwrap());
                    return Client {
                        reader,
                        writer: stream,
                        next_id: 1,
                    };
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the hosted layer never served its socket");
    }

    fn request(&mut self, request: Request) -> Response {
        let id = self.next_id;
        self.next_id += 1;
        let message = serde_json::to_string(&ClientMessage::Request { id, request }).unwrap();
        self.writer.write_all(message.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).unwrap();
            assert!(n > 0, "the hosted layer closed the seam");
            match serde_json::from_str::<ServerMessage>(&line).unwrap() {
                ServerMessage::Response { id: rid, response } if rid == id => return response,
                _ => continue,
            }
        }
    }
}

#[test]
fn the_jailed_ui_hosts_the_layer_in_process_and_stops_cleanly() {
    let dir = scratch_dir("host");
    let radio = FakeRadio::new();
    let address: BdAddr = "a0:fa:9c:62:7d:01".parse().unwrap();
    // The stack already holds the pad: the stock session is adopted.
    radio.stock_reconnects(&address);
    let supervisor = jailed_supervisor_over(&dir, &radio);
    let socket_path = dir.join("bluepad.sock");

    // Host: the supervisor serves the same seam in-process.
    let hosted =
        bluepad_daemon::hosted::host(supervisor, &socket_path, &dir).expect("host");
    let mut client = Client::connect(&socket_path);

    // The UI talks to it as a regular client — same protocol, probed
    // tier over the seam.
    match client.request(Request::Hello {
        protocol_version: PROTOCOL_VERSION,
    }) {
        Response::Hello { tier, .. } => {
            assert_eq!(tier, PrivilegeTier::Jailed, "the probed tier crosses the seam");
        }
        other => panic!("expected Hello, got {other:?}"),
    }

    // Give the poll loop a moment to adopt the stack-connected pad.
    std::thread::sleep(Duration::from_millis(700));
    match client.request(Request::GetState) {
        Response::State(state) => {
            let pad = state.pads.iter().find(|p| p.address == address).unwrap();
            assert_eq!(pad.phase, PadPhase::Connected);
            assert_eq!(pad.path, Some(ConnectionPath::Stock));
        }
        other => panic!("expected State, got {other:?}"),
    }

    // App exit: the layer stops, every session closes, the socket goes.
    hosted.stop();
    assert!(
        radio.stock_watch_closed(&address),
        "no ghost session outlives the app"
    );
    assert!(!socket_path.exists(), "the socket went away with the layer");
}
