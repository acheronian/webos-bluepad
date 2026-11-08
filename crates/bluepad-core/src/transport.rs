//! The radio-transport seam (Bridge): traits in core, implementations in
//! `bluepad-radio`.
//!
//! The connection layer must not assume which transport serves it
//! (glossary: Radio transport). Two paths exist: the app-owned session
//! (raw L2CAP + uhid) and the stock `bluetooth2` path. Pairing in 0.1.0
//! rides the stock path — it is the proven trust establishment for every
//! revision including B.

use crate::bdaddr::BdAddr;
use crate::failure::FailureReason;
use crate::kit::PadInput;
use crate::pad::{ConnectionPath, PadIdentity};
use serde::{Deserialize, Serialize};

/// Why a transport operation failed, in curated-reason terms plus raw detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError {
    pub reason: FailureReason,
    pub detail: String,
}

impl TransportError {
    pub fn new(reason: FailureReason, detail: impl Into<String>) -> Self {
        TransportError {
            reason,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.reason.label(), self.detail)
    }
}

impl std::error::Error for TransportError {}

/// A live, health-checkable session with a pad.
///
/// App-owned sessions own the raw report stream (parse + forward through
/// `uhid`); stock sessions are a watched presence-consistency handle on the
/// TV's own session — the stock stack owns parsing there.
pub trait HidSession: Send {
    /// The path this session serves.
    fn path(&self) -> ConnectionPath;

    /// Read the next parsed input state. For app-owned sessions this is the
    /// parsed HID report; for stock sessions it is whatever presence or
    /// input evidence the health check can observe.
    fn poll_input(&mut self) -> Result<Option<PadInput>, TransportError>;

    /// Send a pad-side output report (lightbar, player LEDs, rumble).
    /// Stock sessions cannot honor outputs.
    fn write_output(&mut self, report: &[u8]) -> Result<(), TransportError>;

    /// The health-check's transport half: is input genuinely flowing?
    /// (glossary: Health check — a stack-up session that streams nothing is
    /// recovering, not connected).
    fn input_flowing(&self) -> bool;

    /// Tear the session down; the input node is destroyed on leaving
    /// Connected (user story 19).
    fn close(&mut self) -> Result<(), TransportError>;
}

/// The stock-path transport: LG's `bluetooth2` over LS2, adapted (Adapter
/// pattern) behind this trait. Also carries 0.1.0's pairing flow — stock
/// pairing is the proven trust establishment.
pub trait StockTransport: Send {
    /// Whether the adapter is present at all.
    fn adapter_present(&mut self) -> bool;

    /// Open the bounded discovery window.
    fn start_discovery(&mut self) -> Result<(), TransportError>;

    fn stop_discovery(&mut self);

    /// Pads discovered since the last poll.
    fn discovered(&mut self) -> Vec<PadIdentity>;

    /// Pads the stock stack already considers paired (adoption source).
    fn paired_devices(&mut self) -> Vec<PadIdentity>;

    /// Establish trust (the one-time pairing).
    fn pair(&mut self, identity: &PadIdentity) -> Result<(), TransportError>;

    /// Connect through LG's stack. May return a session whose health must
    /// be verified before the pad counts as Connected.
    fn connect(&mut self, identity: &PadIdentity) -> Result<Box<dyn HidSession>, TransportError>;

    /// Whether the stack currently reports this pad connected (presence
    /// consistency for the stock health check).
    fn reports_connected(&mut self, address: &BdAddr) -> bool;

    /// Names of live contending services from the curated list.
    fn contending_services(&mut self) -> Vec<String>;
}

/// The app-owned-path transport: raw L2CAP + `uhid`, owned by BluePad.
pub trait AppOwnedTransport: Send {
    /// Accept a pad-initiated connection (wake, re-advertise) — user story 16.
    fn accept_incoming(&mut self) -> Result<Option<PadIdentity>, TransportError>;

    /// Open a session to a paired pad over the app-owned path.
    fn connect(&mut self, identity: &PadIdentity) -> Result<Box<dyn HidSession>, TransportError>;
}

/// The health-check verdict for a session the stack reports as up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthVerdict {
    /// Input is genuinely flowing: the pad counts as Connected.
    Healthy,
    /// Reported up but streaming nothing: Recovering, not Connected.
    Silent,
    /// The session is gone.
    Lost,
}
