//! The text-mode state view: the honest degraded rendering while the GL
//! shell is pending. Every line it prints is data from the seam — nothing
//! is scraped from logs.

use bluepad_core::pad::PadPhase;
use bluepad_core::protocol::{AppState, PairingProgress};

/// Render the state snapshot as the card texts the real UI would show.
pub fn render(state: &AppState) -> String {
    let mut out = String::new();
    let radio = &state.radio;
    out.push_str(&format!(
        "Radio: {} pads ({} connected, {} reconnecting), discovery {}, tier {:?}\n",
        radio.paired,
        radio.connected,
        radio.recovering,
        match radio.discovery {
            bluepad_core::protocol::DiscoveryState::Idle => "off",
            bluepad_core::protocol::DiscoveryState::Discovering => "ON",
        },
        radio.tier,
    ));
    if !radio.contention.is_empty() {
        out.push_str(&format!(
            "Contended: {}\n",
            radio.contention.join(", ")
        ));
    }
    if let Some(pairing) = &state.pairing {
        let text = match pairing {
            PairingProgress::Discovering => "looking for pads…".into(),
            PairingProgress::Pairing(identity) => {
                format!("pairing with {}…", identity.name)
            }
            PairingProgress::Success(address) => format!("paired {address}"),
            PairingProgress::Failed(failure) => {
                format!("failed: {} — try: {}", failure.reason.label(), {
                    let causes = bluepad_core::failure::possible_causes(
                        failure.reason,
                        bluepad_core::pad::PadKind::DualSense,
                    );
                    causes.first().copied().unwrap_or("")
                })
            }
            PairingProgress::Cancelled => "cancelled".into(),
        };
        out.push_str(&format!("Pairing: {text}\n"));
    }
    for pad in &state.pads {
        out.push_str(&format!(
            "{} [{}] {} — {}{}\n",
            pad.name,
            pad.kind.label(),
            pad.address,
            phase_label(pad.phase),
            pad.battery
                .map(|b| format!(" (battery {}%)", b.percent))
                .unwrap_or_default(),
        ));
        if let Some(player) = pad.player_index {
            out.push_str(&format!("  {}\n", player.label()));
        }
        if let Some(failure) = &pad.failure {
            out.push_str(&format!(
                "  last failure: {}\n",
                failure.reason.label()
            ));
        }
    }
    out
}

fn phase_label(phase: PadPhase) -> &'static str {
    phase.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluepad_core::bdaddr::BdAddr;
    use bluepad_core::kit::Battery;
    use bluepad_core::pad::{PadKind, PadPhase};
    use bluepad_core::protocol::{
        AdapterState, DiscoveryState, PadStatus, PrivilegeTier, RadioStatus,
    };
    use bluepad_core::settings::PadSettings;

    #[test]
    fn renders_pads_and_radio_honestly() {
        let address: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let state = AppState {
            radio: RadioStatus {
                tier: PrivilegeTier::Root,
                adapter: AdapterState::Active,
                discovery: DiscoveryState::Idle,
                contention: vec![],
                paired: 1,
                connected: 1,
                recovering: 0,
            },
            pads: vec![PadStatus {
                address,
                name: "DualSense Wireless Controller".into(),
                kind: PadKind::DualSense,
                phase: PadPhase::Connected,
                path: None,
                player_index: Some(bluepad_core::kit::PlayerIndex::P1),
                battery: Some(Battery {
                    percent: 80,
                    charging: false,
                }),
                failure: None,
                settings: PadSettings::default(),
                stock_only: false,
                diagnostic_available: true,
            }],
            pairing: None,
        };
        let text = render(&state);
        assert!(text.contains("1 connected"));
        assert!(text.contains("DualSense Wireless Controller"));
        assert!(text.contains("Connected"));
        assert!(text.contains("battery 80%"));
        assert!(text.contains("Player 1"));
    }
}
