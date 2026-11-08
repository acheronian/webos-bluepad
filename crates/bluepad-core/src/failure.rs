//! Curated, closed failure reasons and their possible-causes lists.

use crate::pad::PadKind;
use serde::{Deserialize, Serialize};

/// A closed reason enum crossing the UI↔daemon seam, with raw detail riding
/// alongside (spec: Failure reporting). The reason drives the possible-causes
/// list; the detail is shown to the user verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// LG's HID session layer died mid-handshake (the revision-B error-130
    /// specimen: consistent ~10 s, no kernel activity).
    StackTimeout,
    /// Auth rejected / link-key defect (error 129) — the one reason that
    /// legitimately suggests re-pairing.
    AuthRejected,
    /// No usable Bluetooth radio behind the transports.
    RadioUnavailable,
    /// A contending service is live (e.g. the original GamePads helper).
    ContendedRadio,
    /// The pad did not accept the connection or went away mid-attempt.
    PadUnreachable,
    /// An established session dropped (pad sleep, standby, stack restart).
    SessionLost,
    /// The stack reports the session up but no input flows (the silent
    /// `device/getStatus` blackout specimen).
    SilentBlackout,
    /// The four-session floor is taken.
    TooManyPads,
    /// This pad kind cannot use the requested connection path
    /// (kind × path matrix).
    UnsupportedCombination,
    /// The 60 s discovery window closed with no pad to pair.
    DiscoveryTimeout,
    /// The 30 s pair window closed without completing.
    PairingTimeout,
    /// Pairing was cancelled (by the user or contention).
    PairingCancelled,
    /// Anything unexpected — always carries detail.
    InternalError,
}

impl FailureReason {
    /// The human-readable reason shown on the connect-failure card.
    pub fn label(&self) -> &'static str {
        match self {
            FailureReason::StackTimeout => "The TV's Bluetooth stack gave up mid-handshake",
            FailureReason::AuthRejected => "The TV and the pad no longer trust each other",
            FailureReason::RadioUnavailable => "No usable Bluetooth radio",
            FailureReason::ContendedRadio => "Another app's helper is using the radio",
            FailureReason::PadUnreachable => "The pad didn't answer",
            FailureReason::SessionLost => "The connection dropped",
            FailureReason::SilentBlackout => "The connection went silent",
            FailureReason::TooManyPads => "All four pad slots are taken",
            FailureReason::UnsupportedCombination => "This pad can't use that connection path",
            FailureReason::DiscoveryTimeout => "No pad was found in time",
            FailureReason::PairingTimeout => "Pairing didn't finish in time",
            FailureReason::PairingCancelled => "Pairing was cancelled",
            FailureReason::InternalError => "Something unexpected went wrong",
        }
    }

    /// Re-pairing happens only on registry defects (spec: Per-pad lifecycle).
    pub fn is_registry_defect(&self) -> bool {
        matches!(self, FailureReason::AuthRejected)
    }
}

/// A failure as it crosses the seam: curated reason + raw detail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub reason: FailureReason,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Failure {
    pub fn new(reason: FailureReason, detail: impl Into<String>) -> Self {
        Failure {
            reason,
            detail: detail.into(),
        }
    }
}

/// The curated possible-causes list shown beneath a connect-failure reason,
/// keyed by failure reason and pad kind (glossary: Possible causes).
///
/// Deliberately never suggests re-pairing: that suggestion is reserved for
/// registry defects, where `is_registry_defect` already says so.
pub fn possible_causes(reason: FailureReason, kind: PadKind) -> &'static [&'static str] {
    match (reason, kind) {
        (FailureReason::StackTimeout, PadKind::DualSense) => &[
            "This pad's generation may be newer than the TV's Bluetooth stack expects — try again; BluePad will route around the TV stack if it can",
            "The TV's Bluetooth stack may be busy; wait a few seconds and press Connect again",
            "A worn battery can make the pad drop out of the handshake; charge it and try again",
        ],
        (FailureReason::StackTimeout, _) => &[
            "The TV's Bluetooth stack may be busy; wait a few seconds and press Connect again",
            "A worn battery can make the pad drop out of the handshake; charge it and try again",
        ],
        (FailureReason::PadUnreachable, _) => &[
            "The pad may already be connected to something else (a console, another TV); disconnect it there first",
            "The pad may be out of range or behind an obstacle; move closer",
            "The pad's battery may be too low to connect; charge it",
        ],
        (FailureReason::SessionLost, _) => &[
            "The pad went to sleep; press any button to wake it",
            "The TV may have gone through standby; the pad reconnects on its own within a minute",
        ],
        (FailureReason::SilentBlackout, _) => &[
            "The radio link degraded; BluePad is re-establishing the session",
            "Other 2.4 GHz traffic (WiFi, microwaves) can stall Bluetooth; try again in a moment",
        ],
        (FailureReason::RadioUnavailable, _) => &[
            "The TV's Bluetooth radio may be restarting; wait a few seconds",
            "If a USB Bluetooth adapter was in use, it may have come loose; re-plug it",
        ],
        (FailureReason::TooManyPads, _) => &[
            "Forget a pad you no longer use to free a slot",
        ],
        (FailureReason::DiscoveryTimeout, _) => &[
            "Hold the pad's pairing button until its light flashes steadily, then try again",
            "Keep the pad close to the TV while pairing",
        ],
        (FailureReason::PairingTimeout, _) => &[
            "Let go of the pairing buttons once the pad starts flashing",
            "The pad may be paired to another host; reset its pairing memory first",
        ],
        // Registry defect: the curated list here is the one place re-pairing
        // is honest advice.
        (FailureReason::AuthRejected, _) => &[
            "The stored trust between the TV and this pad is damaged; Forget the pad and pair it again",
        ],
        (FailureReason::ContendedRadio, _) => &[
            "Another gamepad app's helper is running; uninstall or stop it, then try again",
        ],
        (FailureReason::UnsupportedCombination, _) => &[
            "This pad type can only connect one way on this TV",
        ],
        (FailureReason::PairingCancelled, _) => &[],
        (FailureReason::InternalError, _) => &[
            "Export diagnostics from Settings → About and attach it to a report",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn possible_causes_never_suggest_repairing_outside_registry_defects() {
        let reasons = [
            FailureReason::StackTimeout,
            FailureReason::RadioUnavailable,
            FailureReason::ContendedRadio,
            FailureReason::PadUnreachable,
            FailureReason::SessionLost,
            FailureReason::SilentBlackout,
            FailureReason::TooManyPads,
            FailureReason::UnsupportedCombination,
            FailureReason::DiscoveryTimeout,
            FailureReason::PairingTimeout,
            FailureReason::PairingCancelled,
            FailureReason::InternalError,
        ];
        for reason in reasons {
            assert!(!reason.is_registry_defect());
            for kind in [PadKind::DualSense, PadKind::Ds4, PadKind::XboxOneS, PadKind::GenericHid] {
                for cause in possible_causes(reason, kind) {
                    let c = cause.to_ascii_lowercase();
                    assert!(
                        !c.contains("re-pair")
                            && !c.contains("repair")
                            && !c.contains("forget the pad")
                            && !c.contains("unpair"),
                        "cause {cause:?} suggests re-pairing for {reason:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn auth_rejection_is_the_only_registry_defect() {
        assert!(FailureReason::AuthRejected.is_registry_defect());
        assert!(FailureReason::AuthRejected.label().contains("trust"));
    }
}
