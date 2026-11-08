//! The UI↔daemon socket protocol: data-only Command requests plus Observer
//! subscribe/notify, as serde messages (spec: Privilege-tiered architecture;
//! design-patterns research: Command as a serde enum, Observer over the
//! seam).
//!
//! Framing is JSON lines over a Unix socket in the app dir. Version skew is
//! detected at Hello; capability introspection is the proxy's controlled-
//! access duty in protocol form.

use crate::bdaddr::BdAddr;
use crate::failure::Failure;
use crate::kit::{Battery, PlayerIndex, Reading};
use crate::pad::{ConnectionPath, PadIdentity, PadKind, PadPhase};
use crate::settings::PadSettings;
use serde::{Deserialize, Serialize};

/// The protocol version of this build. Bump on any breaking change to the
/// message vocabulary.
pub const PROTOCOL_VERSION: u32 = 1;

/// The privilege tier the daemon actually probed — never assumed from the
/// install method (glossary: Privilege tier).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeTier {
    /// Root daemon: background sessions, all pads, S1–S5 bar.
    Root,
    /// Jailed: degraded stock-path-only, while-app-open mode.
    Jailed,
}

/// Adapter presence and activity for the radio-status card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterState {
    Absent,
    Idle,
    Active,
}

/// Discovery state for the radio-status card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryState {
    Idle,
    /// A pairing window is open (bounded, modal).
    Discovering,
}

/// The closed signal set the radio-status card renders — pushed over the
/// seam, never scraped from logs (glossary: Radio status).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioStatus {
    pub tier: PrivilegeTier,
    pub adapter: AdapterState,
    pub discovery: DiscoveryState,
    /// Names of live contending services (contention blocks pairing).
    pub contention: Vec<String>,
    /// Aggregate pad summary.
    pub paired: u8,
    pub connected: u8,
    pub recovering: u8,
}

/// A pad as the UI sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PadStatus {
    pub address: BdAddr,
    pub name: String,
    pub kind: PadKind,
    pub phase: PadPhase,
    /// The path serving this pad, once known (routing is invisible to the
    /// user; this exists for diagnostics and honest stock-only rendering).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<ConnectionPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_index: Option<PlayerIndex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<Battery>,
    /// The last connect failure, if the pad isn't connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    /// Effective settings (defaults where no override exists).
    pub settings: PadSettings,
    /// True when this pad can only use the stock path: settings rows are
    /// disabled with an honest reason, never hidden (user story 21).
    pub stock_only: bool,
    /// True when the live pad diagnostic is available (app-owned sessions).
    pub diagnostic_available: bool,
}

/// Pairing sheet state (modal, bounded).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingProgress {
    /// The 60 s discovery window is open.
    Discovering,
    /// A candidate was found; the 30 s pair window is running.
    Pairing(PadIdentity),
    /// Trust established; the pad enters the registry.
    Success(BdAddr),
    /// The window closed or the attempt failed — always human-visible.
    Failed(Failure),
    /// Cancelled by the user.
    Cancelled,
}

impl PairingProgress {
    /// Terminal outcomes end the modal window — a new one may open.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PairingProgress::Success(_) | PairingProgress::Failed(_) | PairingProgress::Cancelled
        )
    }
}

/// The full state snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub radio: RadioStatus,
    pub pads: Vec<PadStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingProgress>,
}

/// A capability the daemon serves — the proxy's controlled-access surface,
/// reflected so a newer UI can degrade gracefully against an older daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Pairing,
    Forget,
    PadSettings,
    RumbleTest,
    Diagnostics,
    GlobalToggle,
}

/// Requests: data-only Commands over the seam.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum Request {
    /// Version-skew gate; every client sends this first.
    Hello { protocol_version: u32 },
    GetState,
    StartPairing,
    CancelPairing,
    /// Adopt a pad already paired to the TV by the stock stack into
    /// BluePad's registry (pairing provenance: stock) and connect it.
    /// The migration path for users upgrading from stock pairing.
    AdoptPaired { address: BdAddr },
    ForgetPad { address: BdAddr },
    SetPadSettings { address: BdAddr, settings: PadSettings },
    SetGlobalEnabled { enabled: bool },
    RumbleTest { address: BdAddr },
    /// Current pad diagnostic readings (app-owned sessions only).
    GetDiagnostics { address: BdAddr },
    /// Begin/extend the notification subscription (Observer).
    Subscribe,
    Unsubscribe,
}

/// Responses, paired to requests by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum Response {
    Hello {
        daemon_version: String,
        protocol_version: u32,
        tier: PrivilegeTier,
        capabilities: Vec<Capability>,
    },
    State(AppState),
    Ok,
    PadSettings(PadSettings),
    Diagnostics(Vec<Reading>),
    Error { message: String },
    /// The client speaks an incompatible protocol version.
    VersionMismatch { daemon_protocol_version: u32 },
}

/// Notifications pushed to subscribers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum Notification {
    RadioStatus(RadioStatus),
    PadChanged(PadStatus),
    PadRemoved { address: BdAddr },
    PairingProgress(PairingProgress),
}

/// One client → daemon line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum ClientMessage {
    Request { id: u64, request: Request },
}

/// One daemon → client line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum ServerMessage {
    Response { id: u64, response: Response },
    Notification(Notification),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_json_lines() {
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let msg = ClientMessage::Request {
            id: 7,
            request: Request::SetPadSettings {
                address: addr,
                settings: PadSettings::default(),
            },
        };
        let line = serde_json::to_string(&msg).unwrap();
        let back: ClientMessage = serde_json::from_str(&line).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn hello_is_the_version_skew_gate() {
        let msg = ClientMessage::Request {
            id: 1,
            request: Request::Hello {
                protocol_version: PROTOCOL_VERSION,
            },
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(line.contains("\"hello\""));
        let back: ClientMessage = serde_json::from_str(&line).unwrap();
        match back {
            ClientMessage::Request {
                request: Request::Hello { protocol_version },
                ..
            } => assert_eq!(protocol_version, PROTOCOL_VERSION),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn pad_status_omits_unset_optional_fields() {
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let status = PadStatus {
            address: addr,
            name: "DualSense".into(),
            kind: PadKind::DualSense,
            phase: PadPhase::Paired,
            path: None,
            player_index: None,
            battery: None,
            failure: None,
            settings: PadSettings::default(),
            stock_only: false,
            diagnostic_available: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("path"));
        assert!(!json.contains("failure"));
    }
}
