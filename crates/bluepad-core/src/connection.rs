//! The connection layer: one supervisor owning every pad's lifecycle
//! (Mediator, top-down ownership — design-patterns research §3).
//!
//! Pure synchronous logic, driven by `poll` and commands; the daemon shell
//! owns timers and I/O. The per-pad state machine is enum-first
//! (`PadPhase` + match); backoff is a closed Strategy; crash recovery is
//! reconcile-against-the-radio, never snapshot-restore.
//!
//! Invariants enforced here (spec: Per-pad lifecycle):
//! - Connected means health-checked input; silent sessions are Recovering.
//! - Bounded backoff (5 attempts) parks the pad in Paired, visible.
//! - Pairing is modal and bounded: 60 s discovery / 30 s pair.
//! - Forget is the only removal, and it deletes settings too.
//! - The input node is destroyed on leaving Connected.
//! - Four-session floor with connect-order player LEDs.

use crate::backoff::{Backoff, BackoffPolicy};
use crate::bdaddr::BdAddr;
use crate::failure::{Failure, FailureReason};
use crate::kit::{self, Battery, OutputRequest, PadInput, PlayerIndex};
use crate::kit::PadKitFactory;
use crate::pad::{ConnectionPath, PadIdentity, PadKind, PadPhase};
use crate::protocol::Notification;
use crate::protocol::{
    AppState, Capability, DiscoveryState, PairingProgress, PadStatus, PrivilegeTier,
    RadioStatus, AdapterState,
};
use crate::registry::{PadRegistry, PairingProvenance, RegistryEntry};
use crate::settings::SettingsStore;
use crate::transport::{
    AppOwnedTransport, HealthVerdict, HidSession, StockTransport, TransportError,
};
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

/// The 60 s discovery window bound.
pub const DISCOVERY_WINDOW: Duration = Duration::from_secs(60);
/// The 30 s pair window bound.
pub const PAIR_WINDOW: Duration = Duration::from_secs(30);
/// How often connected sessions are health-checked.
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Stagger between pads during a wake reconcile, so multiple pads coming
/// back from standby don't trample each other (user story 17).
pub const STAGGER: Duration = Duration::from_secs(1);

/// Why the supervisor wants to talk to a pad soon (scheduled work).
#[derive(Clone, Copy)]
enum Scheduled {
    /// Try to (re)connect.
    Connect,
    /// Re-check health after this instant.
    HealthCheck,
}

struct PadMachine {
    entry: RegistryEntry,
    phase: PadPhase,
    session: Option<Box<dyn HidSession>>,
    backoff: Backoff,
    failure: Option<Failure>,
    scheduled: Option<(Scheduled, Instant)>,
    player_index: Option<PlayerIndex>,
    battery: Option<Battery>,
    last_input: PadInput,
}

impl PadMachine {
    fn new(entry: RegistryEntry) -> Self {
        PadMachine {
            entry,
            phase: PadPhase::Paired,
            session: None,
            backoff: Backoff::new(BackoffPolicy::default()),
            failure: None,
            scheduled: None,
            player_index: None,
            battery: None,
            last_input: PadInput::default(),
        }
    }

    /// The health verdict for whatever session this pad currently holds.
    fn health(&self) -> HealthVerdict {
        match self.session.as_ref() {
            Some(session) if session.input_flowing() => HealthVerdict::Healthy,
            Some(_) => HealthVerdict::Silent,
            None => HealthVerdict::Lost,
        }
    }

    fn session_is_app_owned(&self) -> bool {
        matches!(&self.session, Some(s) if s.path() == ConnectionPath::AppOwned)
            && self.phase == PadPhase::Connected
    }

    /// Leaving Connected always tears the session down (the input node is
    /// destroyed — no ghost input for streaming apps).
    fn leave_connected(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = session.close();
        }
        self.player_index = None;
    }

    fn status(&self, settings: &SettingsStore) -> PadStatus {
        let stock_only = self
            .entry
            .proven_path
            .map(|p| p == ConnectionPath::Stock)
            .unwrap_or_else(|| {
                // A pad is stock-only when its kind has no app-owned kit.
                kit::factory()
                    .kit(self.entry.kind, ConnectionPath::AppOwned)
                    .is_none()
            });
        PadStatus {
            address: self.entry.identity.address,
            name: self.entry.identity.name.clone(),
            kind: self.entry.kind,
            phase: self.phase,
            path: self.entry.proven_path,
            player_index: self.player_index,
            battery: self.battery,
            failure: self.failure.clone(),
            settings: settings.pad(&self.entry.identity.address),
            stock_only,
            diagnostic_available: self.session_is_app_owned(),
        }
    }
}

struct PairingWindow {
    phase: PairingProgress,
    deadline: Instant,
}

/// The supervisor: owns transports, per-pad machines, registry, settings.
pub struct Supervisor {
    stock: Box<dyn StockTransport>,
    app_owned: Box<dyn AppOwnedTransport>,
    registry: PadRegistry,
    settings: SettingsStore,
    pads: HashMap<BdAddr, PadMachine>,
    pairing: Option<PairingWindow>,
    tier: PrivilegeTier,
    now: Instant,
}

/// What happened during a poll, for the daemon shell to broadcast.
pub struct PollOutcome {
    pub notifications: Vec<Notification>,
}

impl Supervisor {
    pub fn new(
        stock: Box<dyn StockTransport>,
        app_owned: Box<dyn AppOwnedTransport>,
        registry: PadRegistry,
        settings: SettingsStore,
        tier: PrivilegeTier,
    ) -> Supervisor {
        Supervisor {
            stock,
            app_owned,
            registry,
            settings,
            pads: HashMap::new(),
            pairing: None,
            tier,
            now: Instant::now(),
        }
    }

    fn notify_pad(&mut self, address: &BdAddr) -> Notification {
        let status = self
            .pads
            .get(address)
            .expect("pad exists")
            .status(&self.settings);
        Notification::PadChanged(status)
    }

    fn notify_pad_if_present(&mut self, address: &BdAddr) -> Option<Notification> {
        if self.pads.contains_key(address) {
            Some(self.notify_pad(address))
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // Polling: the heartbeat of every bounded thing.
    // ------------------------------------------------------------------

    /// Advance time-driven work. `now` comes from the shell so tests can
    /// move time freely.
    pub fn poll(&mut self, now: Instant) -> PollOutcome {
        self.now = now;
        let mut notifications = Vec::new();

        // Pairing window: drive the flow and enforce the bounds.
        if self.pairing.is_some() {
            self.drive_pairing(now, &mut notifications);
        }

        // Pad-initiated accepts (wake, re-advertise).
        if let Ok(Some(identity)) = self.app_owned.accept_incoming() {
            self.adopt_or_connect(&identity, &mut notifications);
        }

        // Per-pad scheduled work and health.
        let addresses: Vec<BdAddr> = self.pads.keys().copied().collect();
        for address in addresses {
            self.drive_pad(&address, &mut notifications);
        }

        PollOutcome { notifications }
    }

    /// The pairing flow: discovery → candidate → pair → registry.
    /// Stock pairing is the 0.1.0 trust establishment; the app-owned
    /// pairing flow is gated behind the key-access spike (issue #28) and
    /// gets its own skeleton when un-gated.
    fn drive_pairing(&mut self, now: Instant, notifications: &mut Vec<Notification>) {
        enum Step {
            Discovering,
            Pair(PadIdentity),
            Terminal,
        }
        let step = match self.pairing.as_ref().map(|w| &w.phase) {
            Some(PairingProgress::Discovering) => Step::Discovering,
            Some(PairingProgress::Pairing(identity)) => Step::Pair(identity.clone()),
            Some(_) => Step::Terminal,
            None => return,
        };
        match step {
            Step::Terminal => (),
            Step::Discovering => {
                let expired = self
                    .pairing
                    .as_ref()
                    .map(|w| now >= w.deadline)
                    .unwrap_or(false);
                if let Some(identity) = self.stock.discovered().into_iter().next() {
                    self.pairing_candidate(identity, now);
                    self.notify_pairing(notifications);
                } else if expired {
                    self.stock.stop_discovery();
                    if let Some(window) = &mut self.pairing {
                        window.phase = PairingProgress::Failed(Failure::new(
                            FailureReason::DiscoveryTimeout,
                            "no pad appeared in the discovery window",
                        ));
                    }
                    self.notify_pairing(notifications);
                }
            }
            Step::Pair(identity) => {
                let expired = self
                    .pairing
                    .as_ref()
                    .map(|w| now >= w.deadline)
                    .unwrap_or(false);
                if expired {
                    self.stock.stop_discovery();
                    if let Some(window) = &mut self.pairing {
                        window.phase = PairingProgress::Failed(Failure::new(
                            FailureReason::PairingTimeout,
                            format!("pairing with {} did not complete", identity.name),
                        ));
                    }
                    self.notify_pairing(notifications);
                    return;
                }
                match self.stock.pair(&identity) {
                    Ok(()) => {
                        let _ = self.pairing_succeeded(identity.clone());
                        // Connect the fresh pad right away.
                        let address = identity.address;
                        if let Some(machine) = self.pads.get_mut(&address) {
                            machine.scheduled = Some((Scheduled::Connect, now));
                        }
                        self.notify_pairing(notifications);
                    }
                    Err(e) => {
                        self.stock.stop_discovery();
                        if let Some(window) = &mut self.pairing {
                            window.phase = PairingProgress::Failed(Failure::new(
                                e.reason,
                                e.detail,
                            ));
                        }
                        self.notify_pairing(notifications);
                    }
                }
            }
        }
    }

    fn notify_pairing(&mut self, notifications: &mut Vec<Notification>) {
        if let Some(window) = self.pairing.as_ref() {
            notifications.push(Notification::PairingProgress(window.phase.clone()));
        }
    }

    /// A pad-initiated connection: accept it if it's in the registry.
    fn adopt_or_connect(
        &mut self,
        identity: &PadIdentity,
        notifications: &mut Vec<Notification>,
    ) {
        let address = identity.address;
        if self.registry.get(&address).is_none() {
            return; // unknown pad: not ours to accept
        }
        if let Some(machine) = self.pads.get(&address) {
            if matches!(
                machine.phase,
                PadPhase::Connected | PadPhase::Connecting
            ) {
                return; // already ours
            }
        }
        self.begin_connect(&address, notifications);
    }

    fn drive_pad(&mut self, address: &BdAddr, notifications: &mut Vec<Notification>) {
        let now = self.now;

        // Scheduled work first.
        let due = match self.pads.get(address).map(|m| m.scheduled) {
            Some(Some((Scheduled::Connect, at))) if now >= at => Some(Scheduled::Connect),
            Some(Some((Scheduled::HealthCheck, at))) if now >= at => {
                Some(Scheduled::HealthCheck)
            }
            _ => None,
        };
        match due {
            Some(Scheduled::Connect) => {
                self.begin_connect(address, notifications);
                return;
            }
            Some(Scheduled::HealthCheck) => {
                if let Some(machine) = self.pads.get_mut(address) {
                    machine.scheduled = None;
                }
            }
            None => {}
        }

        // Phase transitions.
        let phase = match self.pads.get(address) {
            Some(machine) => machine.phase,
            None => return,
        };
        match phase {
            PadPhase::Connecting => {
                let verdict = self.pads[address].health();
                match verdict {
                    HealthVerdict::Healthy => {
                        if let Some(machine) = self.pads.get_mut(address) {
                            machine.phase = PadPhase::Connected;
                            machine.failure = None;
                            machine.backoff.reset();
                        }
                        self.assign_player_index(address);
                        let _ = self.send_feedback_pulse(address);
                        self.record_proven_path(address);
                        notifications.push(self.notify_pad(address));
                    }
                    HealthVerdict::Lost | HealthVerdict::Silent => {
                        if let Some(machine) = self.pads.get_mut(address) {
                            machine.leave_connected();
                            machine.phase = PadPhase::Recovering;
                            machine.failure = Some(Failure::new(
                                FailureReason::SilentBlackout,
                                "session reported up but no input flowed",
                            ));
                        }
                        self.schedule_retry(address);
                        notifications.push(self.notify_pad(address));
                    }
                }
            }
            PadPhase::Connected => {
                let verdict = self.pads[address].health();
                match verdict {
                    HealthVerdict::Healthy => {
                        if let Some(machine) = self.pads.get_mut(address) {
                            machine.scheduled =
                                Some((Scheduled::HealthCheck, now + HEALTH_CHECK_INTERVAL));
                        }
                    }
                    HealthVerdict::Silent => {
                        if let Some(machine) = self.pads.get_mut(address) {
                            machine.phase = PadPhase::Recovering;
                            machine.failure = Some(Failure::new(
                                FailureReason::SilentBlackout,
                                "input stopped flowing",
                            ));
                            machine.leave_connected();
                        }
                        self.schedule_retry(address);
                        notifications.push(self.notify_pad(address));
                    }
                    HealthVerdict::Lost => {
                        if let Some(machine) = self.pads.get_mut(address) {
                            machine.phase = PadPhase::Recovering;
                            machine.failure = Some(Failure::new(
                                FailureReason::SessionLost,
                                "session dropped",
                            ));
                            machine.leave_connected();
                        }
                        self.schedule_retry(address);
                        notifications.push(self.notify_pad(address));
                    }
                }
            }
            _ => {}
        }

        // Drain input from live sessions (diagnostics + battery + liveness).
        let mut update = None;
        let mut failed = false;
        if let Some(machine) = self.pads.get_mut(address) {
            if let Some(session) = machine.session.as_mut() {
                match session.poll_input() {
                    Ok(Some(input)) => update = Some(input),
                    Ok(None) => {}
                    Err(_) => failed = true,
                }
            }
        }
        if let Some(input) = update {
            if let Some(machine) = self.pads.get_mut(address) {
                machine.last_input = input;
                if let Some(battery) = input.battery {
                    machine.battery = Some(battery);
                }
            }
        }
        if failed {
            if let Some(machine) = self.pads.get_mut(address) {
                if machine.phase == PadPhase::Connected {
                    machine.phase = PadPhase::Recovering;
                    machine.failure = Some(Failure::new(
                        FailureReason::SessionLost,
                        "session read failed",
                    ));
                    machine.leave_connected();
                    self.schedule_retry(address);
                    notifications.push(self.notify_pad(address));
                }
            }
        }
    }

    fn schedule_retry(&mut self, address: &BdAddr) {
        let now = self.now;
        if let Some(machine) = self.pads.get_mut(address) {
            match machine.backoff.next_delay() {
                Some(delay) => {
                    machine.scheduled = Some((Scheduled::Connect, now + delay));
                }
                None => {
                    // Budget spent: park in Paired with the failure visible.
                    machine.phase = PadPhase::Paired;
                    machine.scheduled = None;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Connection establishment and routing.
    // ------------------------------------------------------------------

    /// Begin a connect attempt on the routed path. A known pad's proven
    /// path sticks; new pads try app-owned first; unsupported combinations
    /// fall back to the other path when the kit matrix allows it.
    fn begin_connect(
        &mut self,
        address: &BdAddr,
        notifications: &mut Vec<Notification>,
    ) {
        if !self.settings.enabled() {
            return;
        }
        let Some(entry) = self.registry.get(address).cloned() else {
            return;
        };
        if !self.pads.contains_key(address) {
            self.pads.insert(*address, PadMachine::new(entry.clone()));
        }
        if self.connected_count() >= 4 {
            if let Some(machine) = self.pads.get_mut(address) {
                machine.phase = PadPhase::Paired;
                machine.failure = Some(Failure::new(
                    FailureReason::TooManyPads,
                    "all four pad slots are taken",
                ));
            }
            notifications.push(self.notify_pad(address));
            return;
        }
        if let Some(machine) = self.pads.get_mut(address) {
            machine.phase = PadPhase::Connecting;
            machine.failure = None;
        }
        notifications.push(self.notify_pad(address));

        let kind = entry.kind;
        let identity = entry.identity.clone();
        let preferred = self.registry.route_for(address);
        // A known pad's proven path sticks; only unproven pads fall back to
        // the other path when the kit matrix allows it.
        let allow_fallback = entry.proven_path.is_none();
        let mut outcome = self.open_session(&identity, preferred, kind);
        if outcome.is_err() && allow_fallback {
            let fallback = match preferred {
                ConnectionPath::AppOwned => ConnectionPath::Stock,
                ConnectionPath::Stock => ConnectionPath::AppOwned,
            };
            if kit::factory().kit(kind, fallback).is_some() {
                outcome = self.open_session(&identity, fallback, kind);
            }
        }

        match outcome {
            Ok(session) => {
                let path = session.path();
                if let Some(machine) = self.pads.get_mut(address) {
                    machine.session = Some(session);
                    machine.entry.proven_path = Some(path);
                    machine.scheduled = None;
                }
            }
            Err(error) => {
                if let Some(machine) = self.pads.get_mut(address) {
                    machine.phase = PadPhase::Recovering;
                    machine.failure = Some(Failure::new(error.reason, error.detail));
                }
                self.schedule_retry(address);
                if let Some(n) = self.notify_pad_if_present(address) {
                    notifications.push(n);
                }
            }
        }
    }

    fn open_session(
        &mut self,
        identity: &PadIdentity,
        path: ConnectionPath,
        kind: PadKind,
    ) -> Result<Box<dyn HidSession>, TransportError> {
        if kit::factory().kit(kind, path).is_none() {
            return Err(TransportError::new(
                FailureReason::UnsupportedCombination,
                format!("{kind:?} has no kit for {path:?}"),
            ));
        }
        match path {
            ConnectionPath::AppOwned => self.app_owned.connect(identity),
            ConnectionPath::Stock => self.stock.connect(identity),
        }
    }

    fn record_proven_path(&mut self, address: &BdAddr) {
        let path = self
            .pads
            .get(address)
            .and_then(|m| m.entry.proven_path);
        if let Some(path) = path {
            let _ = self.registry.record_proven_path(address, path);
        }
    }

    fn connected_count(&self) -> usize {
        self.pads
            .values()
            .filter(|m| m.phase == PadPhase::Connected)
            .count()
    }

    fn assign_player_index(&mut self, address: &BdAddr) {
        let taken: Vec<PlayerIndex> = self
            .pads
            .values()
            .filter_map(|m| m.player_index)
            .collect();
        let index = [PlayerIndex::P1, PlayerIndex::P2, PlayerIndex::P3, PlayerIndex::P4]
            .into_iter()
            .find(|i| !taken.contains(i))
            .expect("connected_count gate keeps a slot free");
        if let Some(machine) = self.pads.get_mut(address) {
            machine.player_index = Some(index);
        }
    }

    /// Connection feedback: rumble pulse + lightbar flash on a true
    /// connection, scaled by the pad's settings (user story 27).
    fn send_feedback_pulse(&mut self, address: &BdAddr) -> Result<(), TransportError> {
        let settings = self.settings.pad(address);
        if !settings.feedback_enabled() {
            return Ok(());
        }
        let player = self
            .pads
            .get(address)
            .and_then(|m| m.player_index)
            .unwrap_or(PlayerIndex::P1);
        let request = OutputRequest {
            rumble: Some(settings.pulse_rumble()),
            lightbar: Some(settings.effective_lightbar(player)),
            player_leds: Some(settings.effective_player_leds(player)),
        };
        self.send_output(address, &request)
    }

    fn send_output(
        &mut self,
        address: &BdAddr,
        request: &OutputRequest,
    ) -> Result<(), TransportError> {
        let (kind, path) = {
            let Some(machine) = self.pads.get(address) else {
                return Ok(());
            };
            let Some(session) = machine.session.as_ref() else {
                return Ok(());
            };
            (machine.entry.kind, session.path())
        };
        let Some(kit) = kit::factory().kit(kind, path) else {
            return Ok(());
        };
        let Some(report) = kit.build_output(request, 0) else {
            return Ok(());
        };
        if let Some(machine) = self.pads.get_mut(address) {
            if let Some(session) = machine.session.as_mut() {
                return session.write_output(&report);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Commands (the Facade's method set — the jailed UI's capability surface).
    // ------------------------------------------------------------------

    /// Start the modal pairing window. Blocked while contending services are
    /// live, with a named reason (user story 38). A window whose outcome is
    /// terminal (success, failure, cancelled) may be replaced.
    pub fn start_pairing(&mut self, now: Instant) -> Result<(), Failure> {
        self.now = now;
        if let Some(window) = &self.pairing {
            if !window.phase.is_terminal() {
                return Err(Failure::new(
                    FailureReason::InternalError,
                    "a pairing window is already open",
                ));
            }
        }
        let contention = self.stock.contending_services();
        if !contention.is_empty() {
            return Err(Failure::new(
                FailureReason::ContendedRadio,
                format!("another app's helper is running: {}", contention.join(", ")),
            ));
        }
        if let Err(e) = self.stock.start_discovery() {
            return Err(Failure::new(e.reason, e.detail));
        }
        self.pairing = Some(PairingWindow {
            phase: PairingProgress::Discovering,
            deadline: now + DISCOVERY_WINDOW,
        });
        Ok(())
    }

    pub fn cancel_pairing(&mut self, now: Instant) {
        self.now = now;
        if let Some(window) = &mut self.pairing {
            self.stock.stop_discovery();
            window.phase = PairingProgress::Cancelled;
        }
    }

    /// Forget a pad: farewell pulse, teardown, registry + settings removal —
    /// the only removal (user story 18).
    pub fn forget(&mut self, address: &BdAddr) -> std::io::Result<()> {
        if let Some(mut machine) = self.pads.remove(address) {
            // Farewell pulse, if the pad can feel it.
            if let Some(session) = machine.session.as_mut() {
                let settings = self.settings.pad(address);
                if settings.feedback_enabled() {
                    let request = OutputRequest {
                        rumble: Some(settings.pulse_rumble()),
                        lightbar: None,
                        player_leds: None,
                    };
                    if let Some(report) = kit::factory()
                        .kit(machine.entry.kind, session.path())
                        .and_then(|k| k.build_output(&request, 0))
                    {
                        let _ = session.write_output(&report);
                    }
                }
            }
            machine.leave_connected();
        }
        self.registry.forget(address)?;
        self.settings.forget(address)?;
        Ok(())
    }

    /// Live-apply a per-pad settings change; re-applied on every session
    /// establishment (user story 28).
    pub fn set_pad_settings(
        &mut self,
        address: &BdAddr,
        settings: crate::settings::PadSettings,
    ) -> std::io::Result<()> {
        self.settings.set_pad(address, settings)?;
        // Live-apply: if the pad is connected on the app-owned path, push the
        // LED state now.
        let app_owned_connected = self
            .pads
            .get(address)
            .map(|m| m.session_is_app_owned())
            .unwrap_or(false);
        if app_owned_connected {
            let player = self
                .pads
                .get(address)
                .and_then(|m| m.player_index)
                .unwrap_or(PlayerIndex::P1);
            let settings = self.settings.pad(address);
            let request = OutputRequest {
                rumble: None,
                lightbar: Some(settings.effective_lightbar(player)),
                player_leds: Some(settings.effective_player_leds(player)),
            };
            let _ = self.send_output(address, &request);
        }
        Ok(())
    }

    pub fn set_global_enabled(&mut self, enabled: bool) -> std::io::Result<()> {
        self.settings.set_enabled(enabled);
        self.settings.save()?;
        if !enabled {
            // The daemon keeps no sessions while off.
            let addresses: Vec<BdAddr> = self.pads.keys().copied().collect();
            for address in addresses {
                if let Some(machine) = self.pads.get_mut(&address) {
                    if machine.phase != PadPhase::Paired {
                        machine.leave_connected();
                        machine.phase = PadPhase::Paired;
                        machine.scheduled = None;
                        machine.backoff.reset();
                    }
                }
            }
        }
        Ok(())
    }

    /// The rumble test button (user story 25).
    pub fn rumble_test(&mut self, address: &BdAddr) -> Result<(), TransportError> {
        let settings = self.settings.pad(address);
        let request = OutputRequest {
            rumble: Some(settings.pulse_rumble()),
            lightbar: None,
            player_leds: None,
        };
        self.send_output(address, &request)
    }

    /// The live pad diagnostic: the daemon's parsed HID state
    /// (user story 30). App-owned sessions only.
    pub fn diagnostics(&self, address: &BdAddr) -> Option<Vec<kit::Reading>> {
        let machine = self.pads.get(address)?;
        if !machine.session_is_app_owned() {
            return None;
        }
        let k = kit::factory().kit(machine.entry.kind, ConnectionPath::AppOwned)?;
        Some(k.diagnostic_readings(&machine.last_input))
    }

    // ------------------------------------------------------------------
    // Reconcile (crash/standby/wake recovery — reconcile, not restore).
    // ------------------------------------------------------------------

    /// Load the registry into machines and schedule staggered reconnects.
    /// Called on daemon start and after standby wake (user stories 9, 10,
    /// 17).
    pub fn reconcile(&mut self, now: Instant) {
        self.now = now;
        let entries: Vec<RegistryEntry> = self.registry.entries().cloned().collect();
        let mut slot = 0;
        for entry in entries {
            let address = entry.identity.address;
            if self.pads.contains_key(&address) {
                continue;
            }
            let mut machine = PadMachine::new(entry);
            machine.scheduled = Some((Scheduled::Connect, now + STAGGER * slot));
            self.pads.insert(address, machine);
            slot += 1;
        }
    }

    /// Adopt a pad the stock stack already paired into BluePad's registry,
    /// then connect it. The migration path for stock-paired pads — no
    /// pairing call needed, the trust already exists.
    pub fn adopt_paired(&mut self, address: &BdAddr, now: Instant) -> Result<PadIdentity, Failure> {
        self.now = now;
        let identity = self
            .stock
            .paired_devices()
            .into_iter()
            .find(|d| d.address == *address)
            .ok_or_else(|| {
                Failure::new(
                    FailureReason::PadUnreachable,
                    "no stock-paired pad with that address",
                )
            })?;
        let entry = RegistryEntry {
            kind: identity.kind(),
            identity: identity.clone(),
            proven_path: None,
            provenance: PairingProvenance::Stock,
        };
        if !self.pads.contains_key(address) {
            self.pads.insert(*address, PadMachine::new(entry.clone()));
        }
        if let Err(e) = self.registry.upsert(entry) {
            return Err(Failure::new(FailureReason::InternalError, e.to_string()));
        }
        if let Some(machine) = self.pads.get_mut(address) {
            machine.scheduled = Some((Scheduled::Connect, now));
        }
        Ok(identity)
    }

    /// A pad completed pairing (called by the shell when the stock transport
    /// reports trust established).
    pub fn pairing_succeeded(&mut self, identity: PadIdentity) -> std::io::Result<()> {
        let entry = RegistryEntry {
            kind: identity.kind(),
            identity,
            proven_path: None,
            provenance: PairingProvenance::Stock,
        };
        let address = entry.identity.address;
        self.registry.upsert(entry.clone())?;
        self.pads.insert(address, PadMachine::new(entry));
        if let Some(window) = &mut self.pairing {
            window.phase = PairingProgress::Success(address);
        }
        Ok(())
    }

    /// Called by the shell when the pairing flow found a candidate.
    pub fn pairing_candidate(&mut self, identity: PadIdentity, now: Instant) {
        self.now = now;
        if let Some(window) = &mut self.pairing {
            window.phase = PairingProgress::Pairing(identity);
            window.deadline = now + PAIR_WINDOW;
        }
    }

    // ------------------------------------------------------------------
    // Views.
    // ------------------------------------------------------------------

    pub fn state(&self) -> AppState {
        let mut pads: Vec<PadStatus> = self
            .pads
            .values()
            .map(|m| m.status(&self.settings))
            .collect();
        pads.sort_by_key(|p| p.address.key());
        let pairing = self.pairing.as_ref().map(|w| w.phase.clone());
        AppState {
            radio: self.radio_status(),
            pads,
            pairing,
        }
    }

    pub fn radio_status(&self) -> RadioStatus {
        RadioStatus {
            tier: self.tier,
            adapter: AdapterState::Idle, // the shell refreshes from transports
            discovery: if self
                .pairing
                .as_ref()
                .map(|w| !w.phase.is_terminal())
                .unwrap_or(false)
            {
                DiscoveryState::Discovering
            } else {
                DiscoveryState::Idle
            },
            contention: Vec::new(), // the shell refreshes from transports
            paired: self.pads.len() as u8,
            connected: self.connected_count() as u8,
            recovering: self
                .pads
                .values()
                .filter(|m| m.phase == PadPhase::Recovering)
                .count() as u8,
        }
    }

    pub fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability::Pairing,
            Capability::Forget,
            Capability::PadSettings,
            Capability::RumbleTest,
            Capability::Diagnostics,
            Capability::GlobalToggle,
        ]
    }

    pub fn tier(&self) -> PrivilegeTier {
        self.tier
    }

    pub fn settings_store(&self) -> &SettingsStore {
        &self.settings
    }

    pub fn settings_store_mut(&mut self) -> &mut SettingsStore {
        &mut self.settings
    }
}
