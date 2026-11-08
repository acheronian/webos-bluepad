//! bluepad-core: the BluePad domain core.
//!
//! Everything here is pure logic over pre-agreed seams — no tokio, no I/O —
//! so the connection layer, kit, protocol, settings, and registry are
//! testable without hardware (spec: Testing Decisions).

pub mod backoff;
pub mod bdaddr;
pub mod failure;
pub mod kit;
pub mod pad;

pub use bdaddr::BdAddr;
pub use pad::{ConnectionPath, PadIdentity, PadKind, PadPhase, SupportTier};
