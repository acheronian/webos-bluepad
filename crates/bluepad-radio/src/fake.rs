//! Fake radio transports with controllable failure injection — the test
//! seam for the connection layer (spec: Testing Decisions, seam 2).
//!
//! No real Bluetooth in unit tests: connect error 130, silent blackouts,
//! sleep/wake, and contention are all scripted here. `FakeRadio` is a
//! cloneable handle over shared state, so a test pokes its clone while the
//! supervisor owns another through the transport trait objects.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::failure::FailureReason;
use bluepad_core::kit::PadInput;
use bluepad_core::pad::{ConnectionPath, PadIdentity};
use bluepad_core::transport::{
    AppOwnedTransport, HidSession, StockTransport, TransportError,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

/// A scripted failure to inject on the next connect attempt for a pad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Injected {
    /// The stock stack dies mid-handshake (error 130 specimen).
    StackTimeout,
    /// Auth rejected (error 129 — registry defect).
    AuthRejected,
    /// The pad doesn't answer.
    PadUnreachable,
    /// The session comes up but streams nothing (silent blackout).
    Silent,
}

impl Injected {
    fn transport_error(&self) -> TransportError {
        match self {
            Injected::StackTimeout => {
                TransportError::new(FailureReason::StackTimeout, "error 130 after ~10 s")
            }
            Injected::AuthRejected => {
                TransportError::new(FailureReason::AuthRejected, "error 129")
            }
            Injected::PadUnreachable => {
                TransportError::new(FailureReason::PadUnreachable, "no answer")
            }
            Injected::Silent => TransportError::new(FailureReason::InternalError, "unused"),
        }
    }
}

#[derive(Default)]
struct SessionInner {
    flowing: bool,
    closed: bool,
    outputs: Vec<Vec<u8>>,
    reports_served: usize,
    input: PadInput,
}

/// A fake session: streams inputs (or silence), records outputs and closes.
/// Clones share state, so a test handle sees what the supervisor's session
/// sees.
#[derive(Clone)]
pub struct FakeSession {
    path: ConnectionPath,
    inner: Arc<Mutex<SessionInner>>,
}

impl FakeSession {
    fn flowing(path: ConnectionPath) -> FakeSession {
        FakeSession {
            path,
            inner: Arc::new(Mutex::new(SessionInner {
                flowing: true,
                ..Default::default()
            })),
        }
    }

    fn silent(path: ConnectionPath) -> FakeSession {
        FakeSession {
            path,
            inner: Arc::new(Mutex::new(SessionInner {
                flowing: false,
                ..Default::default()
            })),
        }
    }

    /// Simulate the session going silent (pad asleep, radio degraded).
    pub fn go_silent(&self) {
        self.inner.lock().unwrap().flowing = false;
    }

    /// Simulate input resuming (pad woke).
    pub fn resume(&self) {
        self.inner.lock().unwrap().flowing = true;
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    pub fn outputs(&self) -> Vec<Vec<u8>> {
        self.inner.lock().unwrap().outputs.clone()
    }

    pub fn reports_served(&self) -> usize {
        self.inner.lock().unwrap().reports_served
    }

    /// Script the parsed input this session serves on every poll (the
    /// per-session input log's data source).
    pub fn serves(&self, input: PadInput) {
        self.inner.lock().unwrap().input = input;
    }
}

impl HidSession for FakeSession {
    fn path(&self) -> ConnectionPath {
        self.path
    }

    fn poll_input(&mut self) -> Result<Option<PadInput>, TransportError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.flowing && !inner.closed {
            inner.reports_served += 1;
            Ok(Some(inner.input))
        } else {
            Ok(None)
        }
    }

    fn write_output(&mut self, report: &[u8]) -> Result<(), TransportError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(TransportError::new(
                FailureReason::SessionLost,
                "session closed",
            ));
        }
        inner.outputs.push(report.to_vec());
        Ok(())
    }

    fn input_flowing(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.flowing && !inner.closed
    }

    fn close(&mut self) -> Result<(), TransportError> {
        self.inner.lock().unwrap().closed = true;
        Ok(())
    }
}

/// A stock watch session: health is presence-consistency against the
/// radio's live connected-set — mirroring the ls2 adapter's design so the
/// supervisor's presence refresh is exercised in tests.
#[derive(Clone)]
pub struct FakeStockWatch {
    inner: Arc<Mutex<RadioInner>>,
    address: BdAddr,
}

impl HidSession for FakeStockWatch {
    fn path(&self) -> ConnectionPath {
        ConnectionPath::Stock
    }

    fn poll_input(&mut self) -> Result<Option<PadInput>, TransportError> {
        Ok(None)
    }

    fn write_output(&mut self, _report: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::new(
            FailureReason::UnsupportedCombination,
            "stock sessions cannot send outputs",
        ))
    }

    fn input_flowing(&self) -> bool {
        self.inner.lock().unwrap().stock_connected.contains(&self.address)
    }

    fn close(&mut self) -> Result<(), TransportError> {
        // Mirrors the real adapter's watch teardown: the stack's
        // connected-view stops being watched for this pad.
        self.inner.lock().unwrap().stock_watches_closed.insert(self.address);
        Ok(())
    }
}

#[derive(Default)]
struct RadioInner {
    adapter_present: bool,
    discoverable: Vec<PadIdentity>,
    discovered_queue: Vec<PadIdentity>,
    paired: HashSet<BdAddr>,
    stock_connect_script: HashMap<BdAddr, Vec<Injected>>,
    app_owned_connect_script: HashMap<BdAddr, Vec<Injected>>,
    incoming: VecDeque<PadIdentity>,
    contention: Vec<String>,
    discovery_calls: usize,
    sessions: HashMap<BdAddr, FakeSession>,
    stock_connected: HashSet<BdAddr>,
    stock_watches_closed: HashSet<BdAddr>,
    unpair_fails: bool,
}

/// The fake radio: one shared state behind both transport traits, with
/// scripted per-pad behavior. Clone freely — clones share state.
#[derive(Clone, Default)]
pub struct FakeRadio {
    inner: Arc<Mutex<RadioInner>>,
}

impl FakeRadio {
    pub fn new() -> FakeRadio {
        FakeRadio {
            inner: Arc::new(Mutex::new(RadioInner {
                adapter_present: true,
                ..Default::default()
            })),
        }
    }

    /// A DualSense identity for tests.
    pub fn dualsense(addr: &str) -> PadIdentity {
        PadIdentity {
            address: addr.parse().unwrap(),
            name: "DualSense Wireless Controller".into(),
            vid: Some(0x054c),
            pid: Some(0x0ce6),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut RadioInner) -> T) -> T {
        let mut guard = self.inner.lock().unwrap();
        f(&mut guard)
    }

    // -- Scripting knobs ------------------------------------------------

    pub fn set_discoverable(&self, pads: Vec<PadIdentity>) {
        self.with(|r| r.discoverable = pads);
    }

    pub fn inject_stock(&self, address: &BdAddr, failures: Vec<Injected>) {
        self.with(|r| r.stock_connect_script.insert(*address, failures));
    }

    pub fn inject_app_owned(&self, address: &BdAddr, failures: Vec<Injected>) {
        self.with(|r| r.app_owned_connect_script.insert(*address, failures));
    }

    pub fn set_contention(&self, services: Vec<String>) {
        self.with(|r| r.contention = services);
    }

    pub fn set_adapter_present(&self, present: bool) {
        self.with(|r| r.adapter_present = present);
    }

    /// Simulate the pad waking and re-advertising (pad-initiated accept).
    pub fn pad_wakes(&self, identity: PadIdentity) {
        self.with(|r| r.incoming.push_back(identity));
    }

    /// Mark a pad as already stock-paired (adoption source).
    pub fn stock_paired(&self, identity: PadIdentity) {
        self.with(|r| {
            r.paired.insert(identity.address);
        });
    }

    /// Script the parsed input a pad's live session serves on every poll
    /// (the per-session input log's data source).
    pub fn session_serves(&self, address: &BdAddr, input: PadInput) {
        self.with(|r| {
            if let Some(s) = r.sessions.get(address) {
                s.serves(input);
            }
        });
    }

    /// Simulate the session going silent (pad sleep, radio degradation).
    pub fn session_goes_silent(&self, address: &BdAddr) {
        self.with(|r| {
            if let Some(s) = r.sessions.get(address) {
                s.go_silent();
            }
        });
    }

    /// Simulate the pad waking on an existing session (input resumes).
    pub fn session_resumes(&self, address: &BdAddr) {
        self.with(|r| {
            if let Some(s) = r.sessions.get(address) {
                s.resume();
            }
        });
    }

    /// Simulate the stock stack dropping a pad (standby): the stack's
    /// connected-view flips; the watch session follows by itself.
    pub fn stock_drops(&self, address: &BdAddr) {
        self.with(|r| {
            r.stock_connected.remove(address);
        });
    }

    /// Simulate the stack reconnecting a pad on its own (adopt-by-stack).
    pub fn stock_reconnects(&self, address: &BdAddr) {
        self.with(|r| {
            r.stock_connected.insert(*address);
        });
    }

    /// Script the stack refusing adapter/unpair (radio down, hub wedged).
    pub fn make_unpair_fail(&self) {
        self.with(|r| r.unpair_fails = true);
    }

    /// Whether the stack currently holds the pad's HID session.
    pub fn stock_connected(&self, address: &BdAddr) -> bool {
        self.with(|r| r.stock_connected.contains(address))
    }

    /// Whether the stock watch session for this pad was closed (the
    // session teardown the layer's lifetime end performs).
    pub fn stock_watch_closed(&self, address: &BdAddr) -> bool {
        self.with(|r| r.stock_watches_closed.contains(address))
    }

    // -- Observation ----------------------------------------------------

    pub fn session(&self, address: &BdAddr) -> Option<FakeSession> {
        self.with(|r| r.sessions.get(address).cloned())
    }

    pub fn is_paired(&self, address: &BdAddr) -> bool {
        self.with(|r| r.paired.contains(address))
    }

    pub fn discovery_calls(&self) -> usize {
        self.with(|r| r.discovery_calls)
    }
}

fn script_next(script: &mut HashMap<BdAddr, Vec<Injected>>, address: &BdAddr) -> Option<Injected> {
    script.get_mut(address).and_then(|v| {
        if v.is_empty() {
            None
        } else {
            Some(v.remove(0))
        }
    })
}

impl StockTransport for FakeRadio {
    fn adapter_present(&mut self) -> bool {
        self.with(|r| r.adapter_present)
    }

    fn start_discovery(&mut self) -> Result<(), TransportError> {
        self.with(|r| {
            r.discovery_calls += 1;
            r.discovered_queue = r.discoverable.clone();
            Ok(())
        })
    }

    fn stop_discovery(&mut self) {
        self.with(|r| r.discovered_queue.clear());
    }

    fn discovered(&mut self) -> Vec<PadIdentity> {
        self.with(|r| std::mem::take(&mut r.discovered_queue))
    }

    fn paired_devices(&mut self) -> Vec<PadIdentity> {
        self.with(|r| r.paired.iter().map(|addr| FakeRadio::dualsense(&addr.to_string())).collect())
    }

    fn pair(&mut self, identity: &PadIdentity) -> Result<(), TransportError> {
        self.with(|r| {
            r.paired.insert(identity.address);
            Ok(())
        })
    }

    fn cancel_pairing(&mut self, _address: &BdAddr) {}

    fn disconnect(&mut self, address: &BdAddr) {
        // hid/disconnect: the stack drops the pad's session.
        self.with(|r| {
            r.stock_connected.remove(address);
        });
    }

    fn unpair(&mut self, address: &BdAddr) -> Result<(), TransportError> {
        self.with(|r| {
            if r.unpair_fails {
                return Err(TransportError::new(
                    FailureReason::RadioUnavailable,
                    "scripted unpair refusal",
                ));
            }
            r.paired.remove(address);
            Ok(())
        })
    }

    fn connect(
        &mut self,
        identity: &PadIdentity,
    ) -> Result<Box<dyn HidSession>, TransportError> {
        // Mirror the real adapter: a pad the stack already connected is
        // adopted, not re-connected.
        if self.with(|r| r.stock_connected.contains(&identity.address)) {
            return Ok(Box::new(FakeStockWatch {
                inner: self.inner.clone(),
                address: identity.address,
            }));
        }
        let injected = self.with(|r| script_next(&mut r.stock_connect_script, &identity.address));
        if let Some(injected) = injected {
            match injected {
                Injected::Silent => {
                    // hid/connect "succeeded" but the stack never reports
                    // the pad connected: presence stays false.
                    let session = FakeStockWatch {
                        inner: self.inner.clone(),
                        address: identity.address,
                    };
                    self.with(|r| {
                        r.sessions.insert(identity.address, FakeSession::silent(ConnectionPath::Stock));
                    });
                    return Ok(Box::new(session));
                }
                other => return Err(other.transport_error()),
            }
        }
        self.with(|r| {
            r.stock_connected.insert(identity.address);
        });
        Ok(Box::new(FakeStockWatch {
            inner: self.inner.clone(),
            address: identity.address,
        }))
    }

    fn reports_connected(&mut self, address: &BdAddr) -> bool {
        self.with(|r| r.stock_connected.contains(address))
    }

    fn contending_services(&mut self) -> Vec<String> {
        self.with(|r| r.contention.clone())
    }
}

impl AppOwnedTransport for FakeRadio {
    fn accept_incoming(&mut self) -> Result<Option<PadIdentity>, TransportError> {
        Ok(self.with(|r| r.incoming.pop_front()))
    }

    fn connect(
        &mut self,
        identity: &PadIdentity,
    ) -> Result<Box<dyn HidSession>, TransportError> {
        let injected =
            self.with(|r| script_next(&mut r.app_owned_connect_script, &identity.address));
        if let Some(injected) = injected {
            match injected {
                Injected::Silent => {
                    let session = FakeSession::silent(ConnectionPath::AppOwned);
                    self.with(|r| {
                        r.sessions.insert(identity.address, session.clone());
                    });
                    return Ok(Box::new(session));
                }
                other => return Err(other.transport_error()),
            }
        }
        let session = FakeSession::flowing(ConnectionPath::AppOwned);
        self.with(|r| {
            r.sessions.insert(identity.address, session.clone());
        });
        Ok(Box::new(session))
    }
}
