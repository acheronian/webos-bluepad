//! The BluePad UI binary (text-mode view; the GL shell lands with
//! packaging per ADR 0001).

use bluepad_ui::client::DaemonClient;
use bluepad_ui::view;

fn main() {
    let socket = std::env::var("BLUEPAD_SOCKET")
        .unwrap_or_else(|_| "/var/lib/webosbrew/com.acheronian.bluepad/bluepad.sock".into());
    let client = match DaemonClient::connect(&socket) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("BluePad: cannot reach the connection daemon ({e}).");
            eprintln!("Is the daemon running? Socket: {socket}");
            std::process::exit(1);
        }
    };
    let state = match client.state() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("BluePad: state unavailable: {e}");
            std::process::exit(1);
        }
    };
    print!("{}", view::render(&state));
}
