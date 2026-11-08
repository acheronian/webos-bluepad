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
//! bluepad-ui session-log <addr> [file]  # per-session input log (story 31)
//! bluepad-ui export          # LAN diagnostics export (token URL, bounded)
//! bluepad-ui export-usb [path]        # USB diagnostics export (root tier)
//! bluepad-ui check-update    # manual update check (Settings → About row)
//! bluepad-ui update-source [show|set <url>|clear]  # advanced override
//! ```
//!
//! The default `state` view also runs the 24 h-throttled cold-start update
//! check — reporting only, pointing at the Homebrew Channel, silent on
//! failure. The update flow never touches the daemon (spec: Update
//! behavior); its commands run without the seam.
//!
//! The seam itself is tier-honest (issue #31): the root daemon's socket
//! when it answers; otherwise the probed tier decides — the jailed
//! tier hosts the connection layer in-process (stock path only) and
//! connects to it as a regular client, and the layer stops with the
//! app.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::protocol::{
    Notification, PairingProgress, PrivilegeTier, Request, Response, SessionLogEntry,
};
use bluepad_ui::client::DaemonClient;
use bluepad_ui::export;
use bluepad_ui::ulog::UiLog;
use bluepad_ui::update::{
    check_update, cold_start_check, render_manual_result, HttpManifestFetcher, UiStateStore,
};
use bluepad_ui::view;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Data dir for persistent state (registry, settings, logs) — the same
/// env-var override the daemon main uses, so packaging wires the tiers:
/// the root tier defaults to webosbrew's data dir, the jailed tier
/// overrides it to the app jail (issue #31: no root assumptions).
fn data_dir() -> std::path::PathBuf {
    std::env::var("BLUEPAD_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from("/var/lib/webosbrew/com.acheronian.bluepad")
        })
}

fn socket_path() -> String {
    std::env::var("BLUEPAD_SOCKET").unwrap_or_else(|_| {
        data_dir()
            .join("bluepad.sock")
            .to_string_lossy()
            .into_owned()
    })
}

/// The hosted connection layer, when this process owns it (the jailed
/// tier). Parked globally so every exit path can stop it — sessions
/// must not outlive the app (issue #31).
static HOSTED: std::sync::Mutex<Option<bluepad_daemon::hosted::HostedDaemon>> =
    std::sync::Mutex::new(None);

/// Stop the hosted connection layer, if this process owns one: the
/// socket server ends, and every session closes (the input node is
/// destroyed — no ghost input).
fn stop_hosted_layer() {
    if let Some(hosted) = HOSTED.lock().unwrap().take() {
        hosted.stop();
    }
}

/// Exit without leaking the hosted connection layer.
fn exit(code: i32) -> ! {
    stop_hosted_layer();
    std::process::exit(code)
}

/// Open the seam to the connection layer. The root daemon's socket when
/// it answers; otherwise the probed tier decides — never the install
/// method (glossary: Privilege tier). The jailed tier hosts the layer
/// in-process — stock path only — and connects to it as a regular
/// client, so no screen code branches on tier (issue #31).
fn connect() -> DaemonClient {
    let socket = socket_path();
    let daemon_unreachable = |e: bluepad_ui::client::ClientError| -> ! {
        UiLog::open(ui_log_path()).error(&format!(
            "cannot reach the connection daemon: {e}"
        ));
        eprintln!("BluePad: cannot reach the connection daemon ({e}).");
        eprintln!("Is the daemon running? Socket: {socket}");
        exit(1);
    };
    match DaemonClient::connect(&socket) {
        Ok(client) => client,
        Err(e) => {
            // No daemon answered: probe what this environment actually
            // grants (the stock adapter answering means the privileged
            // tier — see `bluepad_daemon::probe_tier`).
            use bluepad_core::transport::StockTransport;
            let mut probe = bluepad_radio::ls2::Ls2StockTransport::new();
            match bluepad_daemon::probe_tier(probe.adapter_present()) {
                PrivilegeTier::Jailed => {
                    let dir = data_dir();
                    if let Err(error) = std::fs::create_dir_all(&dir) {
                        UiLog::open(ui_log_path()).error(&format!(
                            "cannot create the data dir {}: {error}",
                            dir.display()
                        ));
                        eprintln!(
                            "BluePad: cannot create the data dir {}: {error}",
                            dir.display()
                        );
                        exit(1);
                    }
                    // Tier-homed logs, like the daemon main: the rotated
                    // log lives in the (jail-homed) data dir — file only,
                    // never stderr, so the layer's Debug logs never
                    // garble this UI's own output.
                    bluepad_daemon::logging::install_file_only(dir.join("bluepad.log"));
                    let supervisor = bluepad_daemon::hosted::jailed_supervisor(&dir);
                    match bluepad_daemon::hosted::host(supervisor, &socket, &dir) {
                        Ok(hosted) => {
                            *HOSTED.lock().unwrap() = Some(hosted);
                            // A regular client against the in-process
                            // seam; the bind races the host's return.
                            for _ in 0..250 {
                                if let Ok(client) = DaemonClient::connect(&socket) {
                                    return client;
                                }
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            daemon_unreachable(e);
                        }
                        Err(error) => {
                            UiLog::open(ui_log_path()).error(&format!(
                                "cannot host the connection layer: {error}"
                            ));
                            eprintln!(
                                "BluePad: cannot host the connection layer in-process \
                                 ({error})."
                            );
                            exit(1);
                        }
                    }
                }
                // The root tier's layer is the daemon's, never the UI's:
                // hosting in-process would mask a down daemon and put a
                // second owner on the radio. Honest error instead.
                PrivilegeTier::Root => daemon_unreachable(e),
            }
        }
    }
}

/// Where the UI's minimal warn/error log lives — beside the UI state,
/// overridable for dev (spec: Diagnostics and observability).
fn ui_log_path() -> String {
    std::env::var("BLUEPAD_UI_LOG").unwrap_or_else(|_| {
        data_dir().join("ui.log").to_string_lossy().into_owned()
    })
}

/// Where the UI's own state lives (update-source override, last-check
/// timestamp, cached result) — beside the socket, overridable for dev.
fn ui_state_path() -> String {
    std::env::var("BLUEPAD_UI_STATE").unwrap_or_else(|_| {
        data_dir()
            .join("ui-state.json")
            .to_string_lossy()
            .into_owned()
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
                    exit(1);
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
        "session-log" => session_log_command(&client, args),
        "export" => export_command(&client),
        "export-usb" => export_usb_command(&client, args),
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
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("BluePad: unknown command {other:?}");
            eprintln!(
                "commands: state | watch | pair | cancel | adopt <addr> | forget <addr> | \
                 rumble <addr> | diag <addr> | session-log <addr> [file] | export | export-usb [path] | \
                 check-update | update-source [show|set <url>|clear]"
            );
            exit(1);
        }
    }
    // The command is done — the app is closing. If this process hosted
    // the connection layer (jailed tier), its lifetime ends here:
    // every session closes, no ghost input (issue #31).
    drop(client);
    stop_hosted_layer();
}

/// The per-session input log, downloadable from the pad diagnostic page
/// (user story 31): control → code → value events, printed or written to
/// a file.
fn session_log_command(client: &DaemonClient, args: &[String]) {
    let address = expect_address(args);
    let response = client.request(Request::GetSessionLog { address });
    let entries = match response {
        Ok(Response::SessionLog(entries)) => entries,
        Ok(other) => {
            println!("{other:?}");
            return;
        }
        Err(e) => {
            eprintln!("BluePad: {e}");
            exit(1);
        }
    };
    let text = render_session_log(&address, &entries);
    match args.get(2) {
        Some(path) => match std::fs::write(path, &text) {
            Ok(()) => println!("Session log written to {path} ({} events)", entries.len()),
            Err(e) => {
                eprintln!("BluePad: cannot write {path}: {e}");
                exit(1);
            }
        },
        None => print!("{text}"),
    }
}

fn render_session_log(address: &BdAddr, entries: &[SessionLogEntry]) -> String {
    let mut out = format!("Session log for {address} ({} events)\n", entries.len());
    for entry in entries {
        match entry.code {
            Some(code) => out.push_str(&format!(
                "#{:<4} +{:>6} ms  {:<14} code {code:#06x}  value {}\n",
                entry.seq, entry.elapsed_ms, entry.label, entry.value
            )),
            None => out.push_str(&format!(
                "#{:<4} +{:>6} ms  {:<14} value {}\n",
                entry.seq, entry.elapsed_ms, entry.label, entry.value
            )),
        }
    }
    out
}

/// The LAN diagnostics export (user story 36, primary on both tiers): a
/// screen-bounded token-URL HTTP server on the TV's LAN address. The URL
/// is shown while the export is live; Enter (or the timer) stops it.
fn export_command(client: &DaemonClient) {
    let tar = match request_bundle_tar(client) {
        Ok(tar) => tar,
        Err(e) => {
            eprintln!("BluePad: {e}");
            exit(1);
        }
    };
    let exporter = match export::LanExporter::start(
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        export::local_lan_ip(),
        0,
        tar,
        export::EXPORT_LIFETIME,
    ) {
        Ok(exporter) => exporter,
        Err(e) => {
            UiLog::open(ui_log_path())
                .error(&format!("cannot start the LAN export: {e}"));
            eprintln!("BluePad: cannot start the LAN export: {e}");
            exit(1);
        }
    };
    println!("Diagnostics export is live:");
    println!("  {}", exporter.url());
    println!(
        "Open the URL on a device on the same network and download {}.",
        export::BUNDLE_FILENAME
    );
    println!(
        "The export stops in {} minutes — or press Enter to stop it now.",
        export::EXPORT_LIFETIME.as_secs() / 60
    );
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(exporter);
    println!("Export stopped.");
}

/// The USB diagnostics export (user story 36, root tier): write the
/// bundle to a mounted USB drive, or to an explicit path.
fn export_usb_command(client: &DaemonClient, args: &[String]) {
    // USB write is the root tier's delivery path — honest refusal instead
    // of a mystery on the jailed tier.
    let tier = match client.state() {
        Ok(state) => state.radio.tier,
        Err(e) => {
            eprintln!("BluePad: state unavailable: {e}");
            exit(1);
        }
    };
    if tier != PrivilegeTier::Root {
        eprintln!(
            "BluePad: USB export needs the root tier; this daemon runs jailed. \
             Use the LAN export instead (`bluepad-ui export`)."
        );
        exit(1);
    }
    let tar = match request_bundle_tar(client) {
        Ok(tar) => tar,
        Err(e) => {
            eprintln!("BluePad: {e}");
            exit(1);
        }
    };
    let written = match args.get(1) {
        Some(path) => match std::fs::write(path, &tar) {
            Ok(()) => std::path::PathBuf::from(path),
            Err(e) => {
                eprintln!("BluePad: cannot write {path}: {e}");
                exit(1);
            }
        },
        None => {
            let root = export::usb_mount_root();
            let mount = export::find_usb_mounts(&root).into_iter().next();
            match mount {
                Some(mount) => match export::write_bundle_to_usb(&tar, &mount, unix_now()) {
                    Ok(path) => path,
                    Err(e) => {
                        eprintln!("BluePad: cannot write to {}: {e}", mount.display());
                        exit(1);
                    }
                },
                None => {
                    eprintln!(
                        "BluePad: no USB drive found (looked under {}).",
                        root.display()
                    );
                    exit(1);
                }
            }
        }
    };
    println!("Diagnostics bundle written to {}", written.display());
}

/// Request the bundle over the seam and finalize it with the UI's own
/// log — the shared half of both delivery paths.
fn request_bundle_tar(client: &DaemonClient) -> Result<Vec<u8>, String> {
    match client.request(Request::ExportDiagnostics) {
        Ok(Response::DiagnosticsBundle(bundle)) => {
            let ui_log = UiLog::open(ui_log_path());
            Ok(export::finalize_bundle(&bundle, &ui_log.read()))
        }
        Ok(other) => Err(format!("unexpected daemon reply: {other:?}")),
        Err(e) => Err(e.to_string()),
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
            exit(1);
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
                exit(1);
            });
            if !url.starts_with("http://") && !url.starts_with("https://") {
                eprintln!("not a manifest URL: {url}");
                exit(1);
            }
            store
                .set_update_source(Some(url.clone()))
                .unwrap_or_else(|error| {
                    eprintln!("BluePad: cannot save the UI state: {error}");
                    exit(1);
                });
            println!("Update source set: {url}");
        }
        Some("clear") => {
            store.set_update_source(None).unwrap_or_else(|error| {
                eprintln!("BluePad: cannot save the UI state: {error}");
                exit(1);
            });
            println!("Update source cleared — back to {}", store.effective_manifest_url());
        }
        Some(other) => {
            eprintln!("BluePad: unknown update-source action {other:?}");
            eprintln!("usage: bluepad-ui update-source [show | set <manifest-url> | clear]");
            exit(1);
        }
    }
}

fn expect_address(args: &[String]) -> BdAddr {
    let raw = args.get(1).unwrap_or_else(|| {
        eprintln!("usage: bluepad-ui <command> <bluetooth-address>");
        exit(1);
    });
    raw.parse().unwrap_or_else(|_| {
        eprintln!("not a Bluetooth address: {raw}");
        exit(1);
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
            exit(1);
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
            exit(1);
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
