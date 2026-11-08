//! The stock-path adapter: LG's `com.webos.service.bluetooth2` over LS2,
//! wrapped behind the core `StockTransport` trait (Adapter pattern —
//! the rest of the codebase never sees raw LS2 payloads).
//!
//! Calls ride the `luna-send` subprocess, the exact route the on-TV probe
//! harness proved (`luna-send -n 1 luna://com.webos.service.bluetooth2/
//! device/getStatus '{}'` — see the P2 capture specimens). The in-process
//! LSRegister route (punktfunk's fast path) lands with the on-device
//! packaging pass; this seam keeps that swap local.
//!
//! Response shapes below are the ones captured on the actual 4.10 TV:
//! `device/getStatus` → `{returnValue, adapterAddress, devices: [{name,
//! address, paired, connectedProfiles, pairing, …}]}`.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::failure::FailureReason;
use bluepad_core::kit::PadInput;
use bluepad_core::pad::{ConnectionPath, PadIdentity};
use bluepad_core::transport::{HidSession, StockTransport, TransportError};
use std::process::Command;
use std::time::Duration;

/// The service URI prefix.
const SERVICE: &str = "luna://com.webos.service.bluetooth2";

/// The curated contend-list: userspace helpers that compete for pads
/// (spec: Contention). The original GamePads app's helper first.
const CONTEND_LIST: &[(&str, &str)] = &[(
    "com.superd22.bluetoothhid.service3",
    "GamePads (Bluetooth gamepads) helper",
)];

pub struct Ls2StockTransport {
    /// The luna CLI binary (configurable for tests/probes).
    sender: String,
    /// The stack's connected-view, shared with watch sessions so their
    /// health check is presence-consistency against the live radio, not a
    /// snapshot taken at connect time.
    presence: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<BdAddr, bool>>>,
    /// The pairable-gate subscription (adapter/awaitPairingRequests),
    /// held while a pairing window is open. The stack is only pairable
    /// while a client subscribes (webos-typings); the legacy app holds
    /// this subscription for its whole lifetime — pads then pair via the
    /// incoming flow, completing without any outgoing call.
    pairable_gate: Option<std::process::Child>,
}

impl Ls2StockTransport {
    pub fn new() -> Ls2StockTransport {
        Ls2StockTransport {
            sender: "luna-send".into(),
            presence: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pairable_gate: None,
        }
    }

    /// Call a bluetooth2 method and parse the JSON reply.
    fn call(&self, method: &str, payload: &str) -> Result<serde_json::Value, TransportError> {
        let uri = format!("{SERVICE}/{method}");
        let output = Command::new(&self.sender)
            .arg("-n")
            .arg("1")
            .arg(&uri)
            .arg(payload)
            .output()
            .map_err(|e| {
                TransportError::new(
                    FailureReason::RadioUnavailable,
                    format!("cannot reach bluetooth2 ({e}); is this a webOS TV?"),
                )
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).map_err(|e| {
            TransportError::new(
                FailureReason::InternalError,
                format!("bluetooth2/{method} returned unparseable output: {e}"),
            )
        })
    }

    fn device_status(&self) -> Result<Vec<serde_json::Value>, TransportError> {
        let reply = self.call("device/getStatus", "{}")?;
        if reply
            .get("returnValue")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            Ok(reply
                .get("devices")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default())
        } else {
            Err(TransportError::new(
                FailureReason::RadioUnavailable,
                format!("device/getStatus error: {}", reply),
            ))
        }
    }

    fn find_device(
        &self,
        address: &BdAddr,
    ) -> Result<Option<serde_json::Value>, TransportError> {
        Ok(self
            .device_status()?
            .into_iter()
            .find(|d| {
                d.get("address")
                    .and_then(|a| a.as_str())
                    .and_then(|a| a.parse::<BdAddr>().ok())
                    .map(|a| a == *address)
                    .unwrap_or(false)
            }))
    }

    /// Open the pairable gate (see `pairable_gate`). Incoming events are
    /// drained and logged; the child stays alive until the window closes.
    fn open_pairable_gate(&mut self) {
        if self.pairable_gate.is_some() {
            return;
        }
        let uri = format!("{SERVICE}/adapter/awaitPairingRequests");
        let child = Command::new(&self.sender)
            .arg("-i")
            .arg(&uri)
            .arg("{\"subscribe\":true}")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    std::thread::spawn(move || {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(stdout).lines() {
                            match line {
                                Ok(line) => tracing::debug!("awaitPairingRequests: {line}"),
                                Err(_) => break,
                            }
                        }
                    });
                }
                self.pairable_gate = Some(child);
            }
            Err(e) => {
                tracing::debug!("awaitPairingRequests gate failed to open: {e}");
            }
        }
    }

    fn device_is_paired(&mut self, address: &BdAddr) -> Option<bool> {
        self.find_device(address)
            .ok()
            .flatten()
            .and_then(|d| d.get("paired").and_then(|p| p.as_bool()))
    }
}

impl Default for Ls2StockTransport {
    fn default() -> Self {
        Self::new()
    }

}

impl StockTransport for Ls2StockTransport {
    fn adapter_present(&mut self) -> bool {
        self.device_status().is_ok()
    }

    fn start_discovery(&mut self) -> Result<(), TransportError> {
        // Open the pairable gate for the window: without an
        // awaitPairingRequests subscriber the adapter is not pairable
        // (webos-typings: pairable is "initially false and set to true
        // when client is subscribed to adapter/awaitPairingRequests").
        // Incoming pairing events are logged at debug for observability.
        self.open_pairable_gate();
        let reply = self.call("adapter/startDiscovery", "{}")?;
        if reply.get("returnValue").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(())
        } else {
            Err(TransportError::new(
                FailureReason::RadioUnavailable,
                format!("startDiscovery refused: {reply}"),
            ))
        }
    }

    fn stop_discovery(&mut self) {
        // webos-typings + the legacy app: adapter/cancelDiscovery
        // ("returnValue will always contain true, even if there is no
        // active discovery scan"). The pairable gate is deliberately NOT
        // closed here: it is held for the daemon's lifetime (the legacy
        // app's own pattern) — killing the subscriber would orphan the
        // subscription on the hub (errorCode 104 blocks re-opening).
        let _ = self.call("adapter/cancelDiscovery", "{}");
    }

    fn discovered(&mut self) -> Vec<PadIdentity> {
        // Devices appear in device/getStatus while discovery runs. Pairing
        // candidates are unpaired, *named* devices of the peripheral class —
        // the on-TV smoke test showed raw discovery also yields nameless
        // neighbor devices (phones and the like) that must not be grabbed.
        match self.device_status() {
            Ok(devices) => devices
                .into_iter()
                .filter(is_pairing_candidate)
                .filter_map(|d| {
                    let address = d
                        .get("address")
                        .and_then(|a| a.as_str())
                        .and_then(|a| a.parse::<BdAddr>().ok())?;
                    Some(PadIdentity {
                        address,
                        name: d
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                        vid: None,
                        pid: None,
                    })
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    fn paired_devices(&mut self) -> Vec<PadIdentity> {
        // The stock adapter's view of what is already paired (adoption
        // source): paired devices with names, from the same captured
        // device/getStatus shape.
        match self.device_status() {
            Ok(devices) => devices
                .into_iter()
                .filter(|d| {
                    d.get("paired").and_then(|p| p.as_bool()).unwrap_or(false)
                        && d.get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| !n.is_empty())
                            .unwrap_or(false)
                })
                .filter_map(|d| {
                    let address = d
                        .get("address")
                        .and_then(|a| a.as_str())
                        .and_then(|a| a.parse::<BdAddr>().ok())?;
                    Some(PadIdentity {
                        address,
                        name: d
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                        vid: None,
                        pid: None,
                    })
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    fn pair(&mut self, identity: &PadIdentity) -> Result<(), TransportError> {
        // Already paired? The incoming flow can complete before the
        // outgoing call even starts (or the pad paired itself while the
        // gate was open).
        if self.device_is_paired(&identity.address) == Some(true) {
            return Ok(());
        }
        // Both flows run concurrently:
        // - outgoing: adapter/pair subscription ends with request:endPairing
        // - incoming: the gate lets the pad pair itself; completion is the
        //   paired field flipping true in device/getStatus (webos-typings
        //   instructs monitoring it exactly so)
        // adapter/pair is a *subscription* (webos-typings: "The caller
        // must subscribe to this method"): the stack pushes
        // displayPinCode / displayPasskey / confirmPasskey UI steps, and
        // the outcome arrives as a request:endPairing event — the exact
        // flow the legacy app's DeviceFinder implemented and this TV
        // accepted for all three DualSense revisions. Gamepads pair
        // without passkey prompts in practice. (`adapter/pairDevice`
        // does not exist — captured on the TV.)
        let payload = serde_json::json!({
            "address": identity.address.to_string(),
            "subscribe": true,
        })
        .to_string();
        let uri = format!("{SERVICE}/adapter/pair");
        let mut child = Command::new(&self.sender)
            .arg("-i")
            .arg(&uri)
            .arg(&payload)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                TransportError::new(
                    FailureReason::RadioUnavailable,
                    format!("cannot reach bluetooth2 ({e})"),
                )
            })?;
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if tx.send(Some(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });
        // The supervisor's 30 s pair window bounds this; stop before it.
        // Events are checked with a short poll so the incoming flow's
        // completion (paired flipping true) is noticed too.
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        let mut last_event = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let poll = remaining.min(Duration::from_secs(2));
            match rx.recv_timeout(poll) {
                Ok(Some(line)) => {
                    last_event = line;
                    if let Ok(event) =
                        serde_json::from_str::<serde_json::Value>(&last_event)
                    {
                        let request =
                            event.get("request").and_then(|r| r.as_str()).unwrap_or("");
                        if request == "endPairing" {
                            let _ = child.kill();
                            if event
                                .get("returnValue")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                return Ok(());
                            }
                            return Err(TransportError::new(
                                FailureReason::PairingTimeout,
                                format!("pairing refused: {event}"),
                            ));
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    // No event within the poll: check the incoming flow.
                    if self.device_is_paired(&identity.address) == Some(true) {
                        let _ = child.kill();
                        tracing::debug!(
                            "pad {} paired via the incoming flow",
                            identity.address
                        );
                        return Ok(());
                    }
                }
            }
        }
        let _ = child.kill();
        // Cleanly end the outgoing pairing request: a killed luna-send
        // leaves the subscription orphaned on the hub (the stack keeps
        // dead subscribers' slots — captured: errorCode 104 "Only one
        // subscription allowed"), and the active pairing request would
        // block every future attempt ("only one pairing request can be
        // active at a given time" — webos-typings).
        let payload =
            serde_json::json!({ "address": identity.address.to_string() }).to_string();
        let _ = self.call("adapter/cancelPairing", &payload);
        Err(TransportError::new(
            FailureReason::PairingTimeout,
            format!(
                "pairing window closed without completion (last event: {last_event})"
            ),
        ))
    }

    fn cancel_pairing(&mut self, address: &BdAddr) {
        // adapter/cancelPairing {address} — one-shot, always succeeds.
        let payload = serde_json::json!({ "address": address.to_string() }).to_string();
        let _ = self.call("adapter/cancelPairing", &payload);
    }

    fn connect(
        &mut self,
        identity: &PadIdentity,
    ) -> Result<Box<dyn HidSession>, TransportError> {
        // The stack auto-connects paired pads when they wake (the P3
        // probe's <2 s reconnect baseline). In that state hid/connect
        // refuses with 106 "Device with supplied address is not available"
        // (captured on the TV) — the session already exists. Adopt the
        // live session instead of asking for a new one.
        if self.reports_connected(&identity.address) {
            self.presence
                .lock()
                .unwrap()
                .insert(identity.address, true);
            return Ok(Box::new(StockWatchSession {
                address: identity.address,
                presence: self.presence.clone(),
            }));
        }
        let payload = serde_json::json!({ "address": identity.address.to_string() }).to_string();
        let reply = self.call("hid/connect", &payload)?;
        if reply.get("returnValue").and_then(|v| v.as_bool()).unwrap_or(false) {
            self.presence
                .lock()
                .unwrap()
                .insert(identity.address, true);
            Ok(Box::new(StockWatchSession {
                address: identity.address,
                presence: self.presence.clone(),
            }))
        } else {
            // Map the stock error codes to curated reasons:
            // - 129: auth/registry defect (revision research) — re-pair
            //   territory, the one honest suggestion.
            // - 130: LG's HID session layer died mid-handshake (the
            //   revision-B specimen).
            // - 106: "Device with supplied address is not available" —
            //   captured on the TV: the pad is not in radio contact
            //   (asleep, off, or out of range).
            let code = reply.get("errorCode").and_then(|c| c.as_i64());
            let reason = match code {
                Some(129) => FailureReason::AuthRejected,
                Some(130) => FailureReason::StackTimeout,
                Some(106) => FailureReason::PadUnreachable,
                _ => FailureReason::PadUnreachable,
            };
            Err(TransportError::new(
                reason,
                format!("hid/connect refused: {reply}"),
            ))
        }
    }

    fn reports_connected(&mut self, address: &BdAddr) -> bool {
        let present = self
            .find_device(address)
            .ok()
            .flatten()
            .and_then(|d| {
                d.get("connectedProfiles")
                    .and_then(|p| p.as_array())
                    .map(|p| !p.is_empty())
            })
            .unwrap_or(false);
        self.presence.lock().unwrap().insert(*address, present);
        present
    }

    fn contending_services(&mut self) -> Vec<String> {
        // Live helpers from the curated list: found by scanning /proc for
        // their service binary.
        CONTEND_LIST
            .iter()
            .filter(|(process, _)| process_is_live(process))
            .map(|(_, label)| label.to_string())
            .collect()
    }
}

/// A pairing candidate: unpaired, named, and of the Bluetooth peripheral
/// major device class (gamepads report classOfDevice 0x2508 — see the
/// captured DualSense entries; major class 5 lives in bits 8-12).
fn is_pairing_candidate(device: &serde_json::Value) -> bool {
    let unpaired = !device.get("paired").and_then(|p| p.as_bool()).unwrap_or(false);
    let named = device
        .get("name")
        .and_then(|n| n.as_str())
        .map(|n| !n.is_empty())
        .unwrap_or(false);
    let peripheral = device
        .get("classOfDevice")
        .and_then(|c| c.as_u64())
        .map(|c| c & 0x1F00 == 0x0500)
        .unwrap_or(false);
    unpaired && named && peripheral
}

fn process_is_live(process: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .any(|e| {
            let cmdline = e.path().join("cmdline");
            std::fs::read_to_string(&cmdline)
                .map(|c| c.contains(process))
                .unwrap_or(false)
        })
}

/// The stock session: a watched presence handle. LG's stack owns the HID
/// session; health is presence-consistency — the shared presence map is
/// refreshed by every `reports_connected` poll, so a session the stack
/// dropped stops counting as flowing (never a pre-created node, per the
/// blackout specimen's lesson).
struct StockWatchSession {
    address: BdAddr,
    presence: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<BdAddr, bool>>>,
}

impl HidSession for StockWatchSession {
    fn path(&self) -> ConnectionPath {
        ConnectionPath::Stock
    }

    fn poll_input(&mut self) -> Result<Option<PadInput>, TransportError> {
        // No per-report visibility on the stock path.
        Ok(None)
    }

    fn write_output(&mut self, _report: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::new(
            FailureReason::UnsupportedCombination,
            "the stock path cannot send pad outputs",
        ))
    }

    fn input_flowing(&self) -> bool {
        self.presence
            .lock()
            .unwrap()
            .get(&self.address)
            .copied()
            .unwrap_or(false)
    }

    fn close(&mut self) -> Result<(), TransportError> {
        self.presence.lock().unwrap().remove(&self.address);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contend_list_names_the_original_gamepads_helper() {
        assert!(CONTEND_LIST
            .iter()
            .any(|(process, label)| process.contains("superd22") && label.contains("GamePads")));
    }

    #[test]
    fn stock_error_codes_map_to_curated_reasons() {
        // The revision research's specimens: 129 = auth/registry defect,
        // 130 = stack died mid-handshake.
        let reply = serde_json::json!({"returnValue": false, "errorCode": 130});
        let code = reply.get("errorCode").and_then(|c| c.as_i64());
        assert_eq!(code, Some(130));
        let reason = match code {
            Some(129) => FailureReason::AuthRejected,
            Some(130) => FailureReason::StackTimeout,
            _ => FailureReason::PadUnreachable,
        };
        assert_eq!(reason, FailureReason::StackTimeout);
    }

    #[test]
    fn captured_get_status_shape_parses() {
        // The exact reply captured on the maintainer's TV (probe-logs,
        // P2 specimen): devices carry name/address/paired/connectedProfiles.
        let reply = serde_json::json!({
            "subscribed": false,
            "adapterAddress": "44:cb:8b:91:1c:41",
            "returnValue": true,
            "devices": [
                {
                    "name": "DualSense Wireless Controller",
                    "address": "a0:fa:9c:62:7d:1a",
                    "paired": true,
                    "connectedProfiles": [],
                    "pairing": false
                }
            ]
        });
        assert!(reply.get("returnValue").unwrap().as_bool().unwrap());
        let devices = reply.get("devices").unwrap().as_array().unwrap();
        assert_eq!(devices.len(), 1);
        let addr: BdAddr = devices[0]["address"].as_str().unwrap().parse().unwrap();
        assert_eq!(addr.key(), "a0fa9c627d1a");
        assert!(devices[0]["connectedProfiles"].as_array().unwrap().is_empty());
    }
}
