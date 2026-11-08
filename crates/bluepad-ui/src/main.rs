//! The BluePad UI binary (text mode; the GL shell lands with packaging
//! per ADR 0001).
//!
//! Subcommands drive the daemon over the seam so the on-TV smoke test
//! (`tools/on-tv/`) needs no extra tooling on the TV:
//!
//! ```text
//! bluepad-ui                 # state view (same as `state`)
//! bluepad-ui state
//! bluepad-ui watch           # subscribe; print notifications live
//! bluepad-ui pair            # open the pairing window; print progress
//! bluepad-ui cancel          # cancel the pairing window
//! bluepad-ui forget <addr>   # remove a pad (registry + settings)
//! bluepad-ui rumble <addr>   # rumble test pulse
//! bluepad-ui diag <addr>     # pad diagnostic readings (app-owned pads)
//! ```

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::protocol::{Notification, PairingProgress, Request, Response};
use bluepad_ui::client::DaemonClient;
use bluepad_ui::view;
use std::time::Duration;

fn socket_path() -> String {
    std::env::var("BLUEPAD_SOCKET").unwrap_or_else(|_| {
        "/var/lib/webosbrew/com.acheronian.bluepad/bluepad.sock".into()
    })
}

fn connect() -> DaemonClient {
    let socket = socket_path();
    match DaemonClient::connect(&socket) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("BluePad: cannot reach the connection daemon ({e}).");
            eprintln!("Is the daemon running? Socket: {socket}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("state");
    let client = connect();

    match command {
        "state" => show_state(&client),
        "watch" => watch(&client),
        "pair" => pair(&client),
        "cancel" => {
            let response = client.request(Request::CancelPairing).expect("request");
            println!("{response:?}");
        }
        "forget" => {
            let address = expect_address(&args);
            let response = client
                .request(Request::ForgetPad { address })
                .expect("request");
            println!("{response:?}");
        }
        "adopt" => {
            let address = expect_address(&args);
            match client.request(Request::AdoptPaired { address }) {
                Ok(Response::Ok) => println!("adopted {address} — connecting"),
                Ok(other) => println!("{other:?}"),
                Err(e) => {
                    eprintln!("BluePad: {e}");
                    std::process::exit(1);
                }
            }
        }
        "rumble" => {
            let address = expect_address(&args);
            let response = client
                .request(Request::RumbleTest { address })
                .expect("request");
            println!("{response:?}");
        }
        "diag" => {
            let address = expect_address(&args);
            match client.request(Request::GetDiagnostics { address }) {
                Ok(Response::Diagnostics(readings)) => {
                    for reading in readings {
                        match reading.code {
                            Some(code) => println!(
                                "{:<14} code {:#06x}  value {}",
                                reading.label, code, reading.value
                            ),
                            None => println!("{:<14} value {}", reading.label, reading.value),
                        }
                    }
                }
                Ok(other) => println!("{other:?}"),
                Err(e) => {
                    eprintln!("BluePad: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("BluePad: unknown command {other:?}");
            eprintln!("commands: state | watch | pair | cancel | adopt <addr> | forget <addr> | rumble <addr> | diag <addr>");
            std::process::exit(1);
        }
    }
}

fn expect_address(args: &[String]) -> BdAddr {
    let raw = args.get(1).unwrap_or_else(|| {
        eprintln!("usage: bluepad-ui <command> <bluetooth-address>");
        std::process::exit(1);
    });
    raw.parse().unwrap_or_else(|_| {
        eprintln!("not a Bluetooth address: {raw}");
        std::process::exit(1);
    })
}

fn show_state(client: &DaemonClient) {
    match client.state() {
        Ok(state) => print!("{}", view::render(&state)),
        Err(e) => {
            eprintln!("BluePad: state unavailable: {e}");
            std::process::exit(1);
        }
    }
}

fn watch(client: &DaemonClient) {
    client
        .subscribe(|notification| println!("{}", render_notification(notification)))
        .expect("subscribe");
    println!("watching the seam — Ctrl-C to stop");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn pair(client: &DaemonClient) {
    // Subscribe first so no progress notification is missed; a terminal
    // phase ends the wait early instead of sleeping the whole window.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let done = done.clone();
        client
            .subscribe(move |notification| {
                println!("{}", render_notification(notification));
                if let Notification::PairingProgress(phase) = notification {
                    if phase.is_terminal() {
                        done.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            })
            .expect("subscribe");
    }
    match client.request(Request::StartPairing) {
        Ok(Response::Ok) => {}
        Ok(other) => {
            println!("{other:?}");
            return;
        }
        Err(e) => {
            eprintln!("BluePad: {e}");
            std::process::exit(1);
        }
    }
    // The window is bounded: 60 s discovery + 30 s pair.
    for _ in 0..95 {
        if done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    show_state(client);
}

fn render_notification(notification: &Notification) -> String {
    match notification {
        Notification::RadioStatus(radio) => {
            format!("[radio] {} connected, {} reconnecting", radio.connected, radio.recovering)
        }
        Notification::PadChanged(pad) => {
            format!("[pad] {} {} — {}", pad.name, pad.address, pad.phase.label())
        }
        Notification::PadRemoved { address } => format!("[pad] {address} forgotten"),
        Notification::PairingProgress(phase) => match phase {
            PairingProgress::Discovering => "[pairing] looking for pads…".into(),
            PairingProgress::Pairing(identity) => {
                format!("[pairing] pairing with {} ({})…", identity.name, identity.address)
            }
            PairingProgress::Success(address) => format!("[pairing] paired {address}"),
            PairingProgress::Failed(failure) => format!(
                "[pairing] failed: {} ({})",
                failure.reason.label(),
                failure.detail
            ),
            PairingProgress::Cancelled => "[pairing] cancelled".into(),
        },
    }
}
