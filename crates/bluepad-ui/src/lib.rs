//! bluepad-ui: the jailed, synchronous view onto the connection daemon.
//!
//! The full GL shell (punktfunk's design language — rail + cards, hero
//! radio-status card, pairing sheet, pad detail) lands with the packaging
//! pass per ADR 0001; what ships here is its foundation: the
//! `DaemonClient` facade over the Unix-socket seam.

pub mod client;
pub mod export;
pub mod ulog;
pub mod update;
pub mod view;

pub use client::{ClientError, DaemonClient};
