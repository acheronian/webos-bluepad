//! The jailed tier's app-owned transport: honest refusal (issue #31).
//!
//! App-owned L2CAP sessions need privileges the jailed environment doesn't
//! grant, so the jailed supervisor is constructed with this transport behind
//! the app-owned trait. The supervisor's jailed routing never offers the
//! path — this transport is the backstop that keeps the construction honest
//! if it is ever asked anyway.

use bluepad_core::failure::FailureReason;
use bluepad_core::pad::PadIdentity;
use bluepad_core::transport::{AppOwnedTransport, HidSession, TransportError};

/// An app-owned transport with no app-owned capability behind it.
pub struct UnavailableTransport;

impl AppOwnedTransport for UnavailableTransport {
    fn accept_incoming(&mut self) -> Result<Option<PadIdentity>, TransportError> {
        // Nothing can arrive on a path that was never opened — not an
        // error, just silence, so the poll loop doesn't churn.
        Ok(None)
    }

    fn connect(&mut self, _identity: &PadIdentity) -> Result<Box<dyn HidSession>, TransportError> {
        Err(TransportError::new(
            FailureReason::RadioUnavailable,
            "the app-owned path needs root privileges; this environment is jailed",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_incoming_is_silence_not_an_error() {
        let mut transport = UnavailableTransport;
        assert_eq!(transport.accept_incoming().unwrap(), None);
    }

    #[test]
    fn connect_refuses_honestly() {
        let mut transport = UnavailableTransport;
        let error = match transport.connect(&crate::fake::FakeRadio::dualsense("a0:fa:9c:62:7d:01")) {
            Err(error) => error,
            Ok(_) => panic!("the unavailable transport must never open a session"),
        };
        assert_eq!(error.reason, FailureReason::RadioUnavailable);
        assert!(error.detail.contains("jailed"), "the detail names the tier: {}", error.detail);
    }
}
