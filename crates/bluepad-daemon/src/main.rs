//! The BluePad connection daemon entry point.
//!
//! Supervised by webosbrew's init.d on the root tier (escalating-backoff
//! startup retries with a toast, standby-surviving); the jailed tier runs
//! it in-process from the UI instead. Paths come from the environment so
//! packaging (deferred per the spec) wires the tiers.

use bluepad_core::connection::Supervisor;
use bluepad_core::protocol::PrivilegeTier;
use bluepad_core::registry::PadRegistry;
use bluepad_core::settings::SettingsStore;
use bluepad_daemon::logging;

/// Data dir for persistent state (registry, settings, logs).
fn data_dir() -> std::path::PathBuf {
    std::env::var("BLUEPAD_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            // Root tier default: webosbrew's data dir (spec: tier-homed
            // logs). The jailed tier overrides via the env var to the app
            // jail.
            std::path::PathBuf::from("/var/lib/webosbrew/com.acheronian.bluepad")
        })
}

fn socket_path() -> std::path::PathBuf {
    std::env::var("BLUEPAD_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| data_dir().join("bluepad.sock"))
}

fn main() -> std::io::Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    logging::install(dir.join("bluepad.log"));

    let stock = bluepad_radio::ls2::Ls2StockTransport::new();
    let app_owned = bluepad_radio::l2cap::L2capTransport::new();
    let registry = PadRegistry::load(dir.join("registry.json"));
    let settings = SettingsStore::load(dir.join("settings.json"));

    // Capability probe, never an install-method assumption: the stock
    // adapter answerable means the privileged tier.
    use bluepad_core::transport::StockTransport;
    let mut probe = bluepad_radio::ls2::Ls2StockTransport::new();
    let tier = if probe.adapter_present() {
        PrivilegeTier::Root
    } else {
        PrivilegeTier::Jailed
    };
    tracing::info!("BluePad daemon starting (tier {tier:?})");

    let supervisor = Supervisor::new(
        Box::new(stock),
        Box::new(app_owned),
        registry,
        settings,
        tier,
    );
    let daemon = bluepad_daemon::Daemon::bind(supervisor, socket_path())?;
    tracing::info!("listening on {}", daemon.socket_path().display());

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async { daemon.run().await })
}
