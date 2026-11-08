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
//! bluepad-ui check-update    # manual update check (Settings → About row)
//! bluepad-ui update-source [show|set <url>|clear]  # advanced override
//! ```
//!
//! The default `state` view also runs the 24 h-throttled cold-start update
//! check — reporting only, pointing at the Homebrew Channel, silent on
//! failure. The update flow never touches the daemon (spec: Update
//! behavior); its commands run without the seam.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::protocol::{Notification, PairingProgress, Request, Response};
use bluepad_ui::client::DaemonClient;
use bluepad_ui::update::{
    check_update, cold_start_check, render_manual_result, HttpManifestFetcher, UiStateStore,
};
use bluepad_ui::view;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Where the UI's own state lives (update-source override, last-check
/// timestamp, cached result) — beside the socket, overridable for dev.
fn ui_state_path() -> String {
    std::env::var("BLUEPAD_UI_STATE").unwrap_or_else(|_| {
        "/var/lib/webosbrew/com.acheronian.bluepad/ui-state.json".into()
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("state");

    // The update flow lives in the UI process alone — these commands run
    // without the seam (the daemon never phones home).
    match command {
        "check-update" => check_update_command(),
        "update-source" => update_source_command(&args[1..]),
        _ => run_daemon_command(command, &args),
    }
}

fn run_daemon_command(command: &str, args: &[String]) {
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
            let address = expect_address(args);
            let response = client
                .request(Request::ForgetPad { address })
                .expect("request");
            println!("{response:?}");
        }
        "adopt" => {
            let address = expect_address(args);
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
            let address = expect_address(args);
            let response = client
                .request(Request::RumbleTest { address })
                .expect("request");
            println!("{response:?}");
        }
        "diag" => {
            let address = expect_address(args);
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
            eprintln!("commands: state | watch | pair | cancel | adopt <addr> | forget <addr> | rumble <addr> | diag <addr> | check-update | update-source [show|set <url>|clear]");
            std::process::exit(1);
        }
    }
}

/// The manual check row (Settings → About in the GL shell): reports both
/// outcomes, and on failure shows an inline error and asks to retry.
fn check_update_command() {
    let mut store = UiStateStore::load(ui_state_path());
    let fetcher = HttpManifestFetcher::new();
    match check_update(&fetcher, store.effective_manifest_url(), env!("CARGO_PKG_VERSION")) {
        Ok(report) => {
            println!("{}", render_manual_result(&report));
            let _ = store.record_update_check(report, unix_now());
        }
        Err(error) => {
            eprintln!("Update check failed: {error}");
            eprintln!("Check the network and try again.");
            std::process::exit(1);
        }
    }
}

/// The advanced "Update source" setting: the maintainer's fallback when
/// the feed moves, persisted in the UI's own state — never the daemon's
/// settings document.
fn update_source_command(args: &[String]) {
    let mut store = UiStateStore::load(ui_state_path());
    match args.first().map(String::as_str) {
        None | Some("show") => {
            match store.update_source() {
                Some(url) => println!("Update source: {url} (override)"),
                None => println!("Update source: {} (default)", store.effective_manifest_url()),
            }
        }
        Some("set") => {
            let url = args.get(1).unwrap_or_else(|| {
                eprintln!("usage: bluepad-ui update-source set <manifest-url>");
                std::process::exit(1);
            });
            if !url.starts_with("http://") && !url.starts_with("https://") {
                eprintln!("not a manifest URL: {url}");
                std::process::exit(1);
            }
            store
                .set_update_source(Some(url.clone()))
                .unwrap_or_else(|error| {
                    eprintln!("BluePad: cannot save the UI state: {error}");
                    std::process::exit(1);
                });
            println!("Update source set: {url}");
        }
        Some("clear") => {
            store.set_update_source(None).unwrap_or_else(|error| {
                eprintln!("BluePad: cannot save the UI state: {error}");
                std::process::exit(1);
            });
            println!("Update source cleared — back to {}", store.effective_manifest_url());
        }
        Some(other) => {
            eprintln!("BluePad: unknown update-source action {other:?}");
            eprintln!("usage: bluepad-ui update-source [show | set <manifest-url> | clear]");
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
    // The 24 h-throttled cold-start check rides the main view: at most one
    // attempt per day, cached result inside the window, silent on failure.
    let mut ui_state = UiStateStore::load(ui_state_path());
    let banner = cold_start_check(
        &mut ui_state,
        &HttpManifestFetcher::new(),
        unix_now(),
        env!("CARGO_PKG_VERSION"),
    );
    match client.state() {
        Ok(state) => {
            if let Some(banner) = banner {
                println!("{banner}");
            }
            print!("{}", view::render(&state));
        }
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
        Notification::PadChanged(pad) => match &pad.failure {
            Some(failure) => format!(
                "[pad] {} {} — {} ({}: {})",
                pad.name,
                pad.address,
                pad.phase.label(),
                failure.reason.label(),
                failure.detail
            ),
            None => format!("[pad] {} {} — {}", pad.name, pad.address, pad.phase.label()),
        },
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
