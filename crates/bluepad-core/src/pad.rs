//! Pad domain vocabulary: kinds, identities, phases, connection paths.

use crate::bdaddr::BdAddr;
use serde::{Deserialize, Serialize};

/// A gamepad kind BluePad knows by name.
///
/// The 0.1.0 device matrix (spec): DualSense (both revisions) and Xbox One S
/// are Supported; DS4 is Best effort; everything else is generic — no tier,
/// and its wording carries the non-claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadKind {
    DualSense,
    Ds4,
    XboxOneS,
    GenericHid,
}

impl PadKind {
    /// The support tier shown on the pairing sheet's device-type rows only.
    pub fn support_tier(&self) -> Option<SupportTier> {
        match self {
            PadKind::DualSense | PadKind::XboxOneS => Some(SupportTier::Supported),
            PadKind::Ds4 => Some(SupportTier::BestEffort),
            PadKind::GenericHid => None,
        }
    }

    /// The kind's user-facing name.
    pub fn label(&self) -> &'static str {
        match self {
            PadKind::DualSense => "DualSense",
            PadKind::Ds4 => "DualShock 4",
            PadKind::XboxOneS => "Xbox One S",
            PadKind::GenericHid => "Gamepad",
        }
    }
}

/// Two-word tier vocabulary, pairing sheet only (glossary: Support tier).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportTier {
    Supported,
    BestEffort,
}

impl SupportTier {
    pub fn label(&self) -> &'static str {
        match self {
            SupportTier::Supported => "Supported",
            SupportTier::BestEffort => "Best effort",
        }
    }
}

/// Per-type pairing steps shown on the pairing sheet (user story 14).
impl PadKind {
    pub fn pairing_help(&self) -> &'static [&'static str] {
        match self {
            PadKind::DualSense => &[
                "Hold the PS button and the Create button together",
                "until the lightbar flashes twice, then release",
            ],
            PadKind::Ds4 => &[
                "Hold the Share button and the PS button together",
                "until the lightbar starts double-flashing, then release",
            ],
            PadKind::XboxOneS => &[
                "Hold the Xbox button until it starts flashing",
                "(the pad must not be paired to anything else)",
            ],
            PadKind::GenericHid => &[
                "Put the pad into its pairing mode",
                "(usually holding a dedicated or Home button)",
            ],
        }
    }
}

/// How a pad's connection reaches the TV.
///
/// Routing is per device and invisible to the user (spec: Connection
/// strategy); a known pad's proven path sticks, new pads try app-owned first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPath {
    /// Raw L2CAP + uhid, owned by BluePad — revision-agnostic, first-class.
    AppOwned,
    /// LG's `bluetooth2` HID session — improved, second.
    Stock,
}

impl ConnectionPath {
    pub fn label(&self) -> &'static str {
        match self {
            ConnectionPath::AppOwned => "BluePad session",
            ConnectionPath::Stock => "TV Bluetooth",
        }
    }
}

/// The per-pad lifecycle state machine phases (spec: Per-pad lifecycle).
///
/// `Connected` means health-checked input: a session the stack reports up
/// but that streams nothing is `Recovering`, not `Connected`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadPhase {
    Unpaired,
    Pairing,
    Paired,
    Connecting,
    Connected,
    Recovering,
}

impl PadPhase {
    pub fn label(&self) -> &'static str {
        match self {
            PadPhase::Unpaired => "Not paired",
            PadPhase::Pairing => "Pairing",
            PadPhase::Paired => "Paired",
            PadPhase::Connecting => "Connecting",
            PadPhase::Connected => "Connected",
            PadPhase::Recovering => "Reconnecting",
        }
    }
}

/// A pad's identity as discovered over the radio.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadIdentity {
    pub address: BdAddr,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vid: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u16>,
}

/// VID/PID → kind, the type layer of the settings keying
/// (controller-settings research §4.1–§4.2).
pub fn kind_from_vid_pid(vid: u16, pid: u16) -> Option<PadKind> {
    match (vid, pid) {
        // Sony 054c: DualSense 0ce6, DualSense Edge 0df2, DS4 v1 05c4, DS4 v2 09cc.
        (0x054c, 0x0ce6) | (0x054c, 0x0df2) => Some(PadKind::DualSense),
        (0x054c, 0x05c4) | (0x054c, 0x09cc) => Some(PadKind::Ds4),
        // Microsoft 045e: Xbox One S (1708) BT 02fd, One S BLE 0b20, Series BT 0b13.
        (0x045e, 0x02fd) | (0x045e, 0x0b20) => Some(PadKind::XboxOneS),
        _ => None,
    }
}

/// Name-substring fallback for pads whose SDP lacks a Device ID record.
pub fn kind_from_name(name: &str) -> Option<PadKind> {
    let n = name.to_ascii_lowercase();
    if n.contains("dualsense") || n.contains("wireless controller") && n.contains("5") {
        Some(PadKind::DualSense)
    } else if n.contains("dualshock") {
        Some(PadKind::Ds4)
    } else if n.contains("xbox") && (n.contains("one") || n.contains("series")) {
        Some(PadKind::XboxOneS)
    } else {
        None
    }
}

impl PadIdentity {
    pub fn kind(&self) -> PadKind {
        self.vid
            .zip(self.pid)
            .and_then(|(v, p)| kind_from_vid_pid(v, p))
            .or_else(|| kind_from_name(&self.name))
            .unwrap_or(PadKind::GenericHid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str, vid: Option<u16>, pid: Option<u16>) -> PadIdentity {
        PadIdentity {
            address: "a0:fa:9c:62:7d:1a".parse().unwrap(),
            name: name.into(),
            vid,
            pid,
        }
    }

    #[test]
    fn kind_comes_from_vid_pid_first() {
        assert_eq!(ident("DualSense", Some(0x054c), Some(0x0ce6)).kind(), PadKind::DualSense);
        assert_eq!(ident("Xbox Wireless Controller", Some(0x045e), Some(0x02fd)).kind(), PadKind::XboxOneS);
        assert_eq!(ident("whatever", Some(0x054c), Some(0x09cc)).kind(), PadKind::Ds4);
    }

    #[test]
    fn kind_falls_back_to_the_name() {
        assert_eq!(ident("DualSense Wireless Controller", None, None).kind(), PadKind::DualSense);
        assert_eq!(ident("Xbox One S Controller", None, None).kind(), PadKind::XboxOneS);
        assert_eq!(ident("DUALSHOCK®4", None, None).kind(), PadKind::Ds4);
        assert_eq!(ident("Mystery Pad", None, None).kind(), PadKind::GenericHid);
    }

    #[test]
    fn tiers_follow_the_device_matrix() {
        assert_eq!(PadKind::DualSense.support_tier(), Some(SupportTier::Supported));
        assert_eq!(PadKind::XboxOneS.support_tier(), Some(SupportTier::Supported));
        assert_eq!(PadKind::Ds4.support_tier(), Some(SupportTier::BestEffort));
        assert_eq!(PadKind::GenericHid.support_tier(), None);
    }
}
