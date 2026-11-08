//! Seam 2 tests: the connection layer against fake radio transports.
//!
//! Everything here observes only the supervisor's public surface (state,
//! notifications, registry, settings, session handles) — never internals.
//! Failure injection covers the specimens from the pre-dongle probes:
//! connect error 130, silent blackouts, sleep/wake, standby, contention.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::connection::Supervisor;
use bluepad_core::failure::FailureReason;
use bluepad_core::pad::{ConnectionPath, PadKind, PadPhase};
use bluepad_core::protocol::{PairingProgress, PrivilegeTier};
use bluepad_core::registry::{PadRegistry, PairingProvenance, RegistryEntry};
use bluepad_core::settings::SettingsStore;
use bluepad_radio::fake::FakeRadio;
use std::time::{Duration, Instant};

fn addr(n: u8) -> BdAddr {
    format!("a0:fa:9c:62:7d:{n:02x}").parse().unwrap()
}

/// A supervisor wired to a fake radio the test keeps a handle to.
struct Harness {
    supervisor: Supervisor,
    radio: FakeRadio,
    #[allow(dead_code)]
    dir: std::path::PathBuf,
}

fn harness_with_registry(entries: Vec<RegistryEntry>) -> Harness {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bluepad-sup-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut registry = PadRegistry::load(dir.join("registry.json"));
    for entry in entries {
        registry.upsert(entry).unwrap();
    }
    let settings = SettingsStore::load(dir.join("settings.json"));
    let radio = FakeRadio::new();
    let supervisor = Supervisor::new(
        Box::new(radio.clone()),
        Box::new(radio.clone()),
        registry,
        settings,
        PrivilegeTier::Root,
    );
    Harness {
        supervisor,
        radio,
        dir,
    }
}

fn registry_entry(n: u8) -> RegistryEntry {
    RegistryEntry {
        identity: FakeRadio::dualsense(&format!("a0:fa:9c:62:7d:{n:02x}")),
        kind: PadKind::DualSense,
        proven_path: None,
        provenance: PairingProvenance::Stock,
    }
}

fn stock_entry(n: u8) -> RegistryEntry {
    RegistryEntry {
        identity: FakeRadio::dualsense(&format!("a0:fa:9c:62:7d:{n:02x}")),
        kind: PadKind::DualSense,
        proven_path: Some(ConnectionPath::Stock),
        provenance: PairingProvenance::Stock,
    }
}

/// Poll twice at the same instant: once to kick scheduled work, once to
/// let the health gate settle (the machine's one-tick Connecting phase).
fn settle(sup: &mut Supervisor, now: Instant) {
    sup.poll(now);
    sup.poll(now);
}

fn phase_of(sup: &Supervisor, pad: &BdAddr) -> PadPhase {
    sup.state()
        .pads
        .iter()
        .find(|p| p.address == *pad)
        .unwrap_or_else(|| panic!("pad {pad} missing from state"))
        .phase
}

#[test]
fn daemon_restart_reconciles_registry_pads_staggered() {
    // Pads reconnect automatically after the app or daemon restarts, in a
    // staggered wake so they don't trample each other (stories 10, 17).
    let mut h = harness_with_registry(vec![registry_entry(1), registry_entry(2)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    assert_eq!(h.supervisor.state().pads.len(), 2);

    // Pad 1 connects at +0; pad 2 one second later.
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
    assert_eq!(phase_of(&h.supervisor, &addr(2)), PadPhase::Paired);
    settle(&mut h.supervisor, start + Duration::from_secs(1));
    assert_eq!(phase_of(&h.supervisor, &addr(2)), PadPhase::Connected);
}

#[test]
fn connected_requires_health_checked_input() {
    // A session that comes up but streams nothing is Recovering, never
    // Connected (story 3; the silent-blackout specimen).
    let h = harness_with_registry(vec![stock_entry(1)]);
    h.radio.inject_app_owned(&addr(1), vec![bluepad_radio::fake::Injected::Silent]);
    // Force the app-owned path by clearing the proven path in the registry
    // the supervisor loaded: instead, just use an unproven entry.
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    h.radio.inject_app_owned(&addr(1), vec![bluepad_radio::fake::Injected::Silent]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    let state = h.supervisor.state();
    assert_eq!(
        phase_of(&h.supervisor, &addr(1)),
        PadPhase::Recovering,
        "a silent session must not count as connected"
    );
    let status = state.pads.iter().find(|p| p.address == addr(1)).unwrap();
    assert_eq!(
        status.failure.as_ref().unwrap().reason,
        FailureReason::SilentBlackout
    );
}

#[test]
fn connect_failure_error130_backs_off_and_parks_in_paired() {
    // Bounded backoff: 1–30 s, 5 attempts, then the pad parks in Paired
    // with the failure visible — never retrying forever (story 11).
    let mut h = harness_with_registry(vec![stock_entry(1)]);
    h.radio
        .inject_stock(&addr(1), vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    let start = Instant::now();
    h.supervisor.reconcile(start);

    // Attempts at +0, +1, +3, +7, +15 s (delays 1, 2, 4, 8 s) — each
    // failure leaves the pad Recovering until the budget is spent.
    for second in [0u64, 1, 3, 7] {
        h.supervisor.poll(start + Duration::from_secs(second));
        h.supervisor.poll(start + Duration::from_secs(second));
        assert_eq!(
            phase_of(&h.supervisor, &addr(1)),
            PadPhase::Recovering,
            "after the +{second}s attempt the pad is recovering"
        );
    }
    // The 5th attempt fails at +15 s: budget spent, parked in Paired.
    settle(&mut h.supervisor, start + Duration::from_secs(15));
    let state = h.supervisor.state();
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Paired, "parked after 5 attempts");
    let status = state.pads.iter().find(|p| p.address == addr(1)).unwrap();
    assert_eq!(
        status.failure.as_ref().unwrap().reason,
        FailureReason::StackTimeout
    );
}

#[test]
fn pad_sleep_then_wake_reconnects() {
    // A mid-game nap doesn't end the session (story 8): the pad goes silent,
    // the supervisor recovers; the pad wakes and re-advertises, and is
    // accepted (story 16).
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);

    // Pad sleeps: session goes silent → Recovering, session closed
    // (no ghost input).
    h.radio.session_goes_silent(&addr(1));
    settle(&mut h.supervisor, start + Duration::from_secs(10));
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Recovering);
    assert!(
        h.radio.session(&addr(1)).unwrap().is_closed(),
        "input node destroyed on leaving Connected"
    );

    // The pad wakes on its own initiative and is accepted.
    h.radio.pad_wakes(FakeRadio::dualsense("a0:fa:9c:62:7d:01"));
    settle(&mut h.supervisor, start + Duration::from_secs(11));
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
}

#[test]
fn four_session_floor_and_player_index_order() {
    // Up to four pads simultaneously; player LEDs indicate connect order
    // (stories 6, 7).
    let mut h = harness_with_registry(vec![
        registry_entry(1),
        registry_entry(2),
        registry_entry(3),
        registry_entry(4),
        registry_entry(5),
    ]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    for second in 0..5u64 {
        settle(&mut h.supervisor, start + Duration::from_secs(second));
    }
    let state = h.supervisor.state();
    assert_eq!(state.radio.connected, 4, "four-session floor");
    let fifth = state.pads.iter().find(|p| p.address == addr(5)).unwrap();
    assert_eq!(fifth.phase, PadPhase::Paired);
    assert_eq!(
        fifth.failure.as_ref().unwrap().reason,
        FailureReason::TooManyPads
    );
    // Connect order 1-4 → player indices 1-4.
    for (n, expected) in [(1u8, "P1"), (2, "P2"), (3, "P3"), (4, "P4")] {
        let status = state.pads.iter().find(|p| p.address == addr(n)).unwrap();
        assert_eq!(format!("{:?}", status.player_index.unwrap()), expected);
    }
}

#[test]
fn forget_is_the_only_removal_and_deletes_settings() {
    // Forget removes the pad from the registry AND its settings entry
    // (story 18).
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let a = addr(1);
    h.supervisor.settings_store_mut().set_pad(&a, Default::default()).unwrap();
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);

    h.supervisor.forget(&a).unwrap();
    let state = h.supervisor.state();
    assert!(state.pads.iter().all(|p| p.address != a));
    assert!(
        h.radio.session(&a).unwrap().is_closed(),
        "session torn down on Forget"
    );
    assert!(
        PadRegistry::load(h.dir.join("registry.json"))
            .entries()
            .count()
            == 0,
        "registry entry deleted"
    );
    assert!(
        SettingsStore::load(h.dir.join("settings.json"))
            .document()
            .pads
            .is_empty(),
        "settings entry deleted"
    );
}

#[test]
fn contention_blocks_pairing_with_a_named_reason() {
    // While a contending service is live, pairing is blocked with a named
    // reason (story 38).
    let mut h = harness_with_registry(vec![]);
    h.radio.set_contention(vec!["com.superd22.bluetoothhid (GamePads helper)".into()]);
    let err = h.supervisor.start_pairing(Instant::now()).unwrap_err();
    assert_eq!(err.reason, FailureReason::ContendedRadio);
    assert!(err.detail.contains("GamePads helper"), "named reason");
    assert!(h.supervisor.state().pairing.is_none(), "no window opened");
}

#[test]
fn pairing_is_modal_and_bounded() {
    // 60 s discovery, 30 s pair, always human-visible (stories 14, 15).
    let mut h = harness_with_registry(vec![]);
    h.radio.set_discoverable(vec![FakeRadio::dualsense("a0:fa:9c:62:7d:2a")]);
    let start = Instant::now();
    h.supervisor.start_pairing(start).unwrap();
    assert!(matches!(
        h.supervisor.state().pairing,
        Some(PairingProgress::Discovering)
    ));

    // Discovery finds the candidate; the 30 s pair window starts.
    h.supervisor.poll(start + Duration::from_secs(5));
    assert!(matches!(
        h.supervisor.state().pairing,
        Some(PairingProgress::Pairing(_))
    ));
    // Pairing completes; the pad lands in the registry and connects.
    settle(&mut h.supervisor, start + Duration::from_secs(6));
    let state = h.supervisor.state();
    assert!(matches!(state.pairing, Some(PairingProgress::Success(_))));
    assert_eq!(state.pads.len(), 1);
    assert_eq!(
        phase_of(&h.supervisor, &"a0:fa:9c:62:7d:2a".parse().unwrap()),
        PadPhase::Connected
    );
    assert!(h.radio.is_paired(&"a0:fa:9c:62:7d:2a".parse().unwrap()));
}

#[test]
fn discovery_window_times_out_visibly() {
    let mut h = harness_with_registry(vec![]);
    let start = Instant::now();
    h.supervisor.start_pairing(start).unwrap();
    // Just inside the window: still discovering.
    h.supervisor.poll(start + Duration::from_secs(59));
    assert!(matches!(
        h.supervisor.state().pairing,
        Some(PairingProgress::Discovering)
    ));
    // Past the 60 s bound: failed, human-visible.
    h.supervisor.poll(start + Duration::from_secs(61));
    match h.supervisor.state().pairing {
        Some(PairingProgress::Failed(f)) => {
            assert_eq!(f.reason, FailureReason::DiscoveryTimeout)
        }
        other => panic!("expected visible failure, got {other:?}"),
    }
}

#[test]
fn proven_path_sticks_and_stock_pads_render_honestly() {
    // A pad that has proven the stock path reconnects over stock; its
    // status renders connect-only: settings rows disabled with an honest
    // reason, no pad diagnostic (stories 21, 22).
    let mut h = harness_with_registry(vec![stock_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    let status = h
        .supervisor
        .state()
        .pads
        .iter()
        .find(|p| p.address == addr(1))
        .unwrap()
        .clone();
    assert_eq!(status.path, Some(ConnectionPath::Stock));
    assert!(status.stock_only, "stock-only rendering");
    assert!(!status.diagnostic_available, "no pad diagnostic on stock path");
    assert!(
        h.supervisor.diagnostics(&addr(1)).is_none(),
        "diagnostics refused for stock sessions"
    );
}

#[test]
fn feedback_pulse_fires_on_true_connection() {
    // A rumble pulse + lightbar flash when a connection is truly
    // established (story 27).
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
    let outputs = h.radio.session(&addr(1)).unwrap().outputs();
    assert!(
        !outputs.is_empty(),
        "connection feedback output sent on entering Connected"
    );
    // It is a DualSense 0x31 report.
    assert_eq!(outputs[0][0], 0x31);
}

#[test]
fn settings_live_apply_pushes_output() {
    // Per-pad settings apply live (story 28).
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    let before = h.radio.session(&addr(1)).unwrap().outputs().len();
    let mut settings = h.supervisor.settings_store().pad(&addr(1));
    settings.lightbar_brightness = bluepad_core::settings::Brightness::High;
    h.supervisor.set_pad_settings(&addr(1), settings).unwrap();
    let after = h.radio.session(&addr(1)).unwrap().outputs().len();
    assert!(after > before, "lightbar pushed on live-apply");
}

#[test]
fn rumble_test_reaches_the_pad() {
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    let before = h.radio.session(&addr(1)).unwrap().outputs().len();
    h.supervisor.rumble_test(&addr(1)).unwrap();
    let outputs = h.radio.session(&addr(1)).unwrap().outputs();
    assert_eq!(outputs.len(), before + 1);
    assert_eq!(outputs[outputs.len() - 1][0], 0x31, "DualSense rumble report");
}

#[test]
fn global_toggle_off_closes_sessions_and_stays_down() {
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
    h.supervisor.set_global_enabled(false).unwrap();
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Paired);
    assert_eq!(h.supervisor.state().radio.connected, 0);
    // And it stays down: later polls don't reconnect.
    h.supervisor.poll(start + Duration::from_secs(60));
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Paired);
}

#[test]
fn standby_drop_recovers_the_session() {
    // Pads reconnect automatically after TV standby (story 9).
    let mut h = harness_with_registry(vec![registry_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);

    // Standby: the session drops.
    h.radio.session_goes_silent(&addr(1));
    settle(&mut h.supervisor, start + Duration::from_secs(30));
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Recovering);
    // Backoff reconnects within a few seconds.
    settle(&mut h.supervisor, start + Duration::from_secs(31));
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
}

#[test]
fn auth_rejected_is_curated_as_a_registry_defect() {
    let mut h = harness_with_registry(vec![stock_entry(1)]);
    h.radio
        .inject_stock(&addr(1), vec![bluepad_radio::fake::Injected::AuthRejected; 5]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    let status = h
        .supervisor
        .state()
        .pads
        .iter()
        .find(|p| p.address == addr(1))
        .unwrap()
        .clone();
    assert_eq!(
        status.failure.as_ref().unwrap().reason,
        FailureReason::AuthRejected
    );
    assert!(status.failure.unwrap().reason.is_registry_defect());
}

#[test]
fn adopt_paired_migrates_a_stock_paired_pad() {
    // A pad the TV already paired (stock trust) is adopted into BluePad's
    // registry and connects without any pairing call.
    let mut h = harness_with_registry(vec![]);
    let identity = FakeRadio::dualsense("a0:fa:9c:62:7d:01");
    h.radio.stock_paired(identity);
    let a = addr(1);
    let adopted = h
        .supervisor
        .adopt_paired(&a, Instant::now())
        .expect("adoption");
    assert_eq!(adopted.address, a);
    // Registry entry exists with stock provenance.
    let registry = PadRegistry::load(h.dir.join("registry.json"));
    let entry = registry.get(&a).expect("registry entry");
    assert_eq!(entry.provenance, PairingProvenance::Stock);
    // And it connects on the next polls.
    let start = Instant::now();
    h.supervisor.poll(start);
    h.supervisor.poll(start);
    assert_eq!(phase_of(&h.supervisor, &a), PadPhase::Connected);
}

#[test]
fn adopt_unknown_address_fails_honestly() {
    let mut h = harness_with_registry(vec![]);
    let err = h
        .supervisor
        .adopt_paired(&addr(9), Instant::now())
        .unwrap_err();
    assert_eq!(err.reason, FailureReason::PadUnreachable);
}

#[test]
fn pairing_window_reopens_after_failure() {
    // The on-TV smoke test found this: a failed window blocked retries
    // with "a pairing window is already open". Terminal outcomes must
    // allow a new window.
    let mut h = harness_with_registry(vec![]);
    let start = Instant::now();
    h.supervisor.start_pairing(start).unwrap();
    // Nothing discoverable → the window times out (terminal Failed).
    h.supervisor.poll(start + bluepad_core::connection::DISCOVERY_WINDOW + Duration::from_secs(1));
    assert!(matches!(
        h.supervisor.state().pairing,
        Some(PairingProgress::Failed(_))
    ));
    // A new window must open without an error.
    h.supervisor
        .start_pairing(start + Duration::from_secs(120))
        .expect("pairing window reopens after a terminal outcome");
}

#[test]
fn stock_health_is_presence_consistency_not_a_snapshot() {
    // The stack's live view decides: a stock pad the stack drops goes
    // Recovering on the health-check cadence — the session is never a
    // connect-time snapshot (the on-TV smoke test surfaced this).
    let mut h = harness_with_registry(vec![stock_entry(1)]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);

    // The stack drops the pad (standby). The watch session's presence is
    // now false; the supervisor notices on the health-check cadence.
    h.radio.stock_drops(&addr(1));
    h.supervisor.poll(start + Duration::from_secs(3));
    h.supervisor.poll(start + Duration::from_secs(3));
    assert_eq!(
        phase_of(&h.supervisor, &addr(1)),
        PadPhase::Recovering,
        "dropped presence must not stay Connected"
    );
    let status = h
        .supervisor
        .state()
        .pads
        .iter()
        .find(|p| p.address == addr(1))
        .unwrap()
        .clone();
    assert_eq!(status.failure.as_ref().unwrap().reason, FailureReason::SessionLost);
}

#[test]
fn adopt_resets_the_recovery_budget() {
    // The on-TV run showed a second adopt getting a single attempt (the
    // budget was spent). A user-initiated adoption deserves a fresh one.
    let mut h = harness_with_registry(vec![]);
    let identity = FakeRadio::dualsense("a0:fa:9c:62:7d:01");
    h.radio.stock_paired(identity);
    let a = addr(1);
    h.supervisor.adopt_paired(&a, Instant::now()).unwrap();
    let start = Instant::now();
    // Spend the whole budget on failures. The entry is unproven, so the
    // app-owned path is tried first and falls back to stock — both must
    // fail for the pad to park.
    h.radio
        .inject_app_owned(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    h.radio
        .inject_stock(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    for second in [0u64, 1, 3, 7, 15] {
        h.supervisor.poll(start + Duration::from_secs(second));
        h.supervisor.poll(start + Duration::from_secs(second));
    }
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Paired, "parked");
    // Adopt again: fresh budget → a full new round of attempts.
    h.radio
        .inject_stock(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    h.radio
        .inject_app_owned(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    h.radio
        .inject_stock(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    h.supervisor.adopt_paired(&a, start + Duration::from_secs(60)).unwrap();
    h.supervisor.poll(start + Duration::from_secs(60));
    h.supervisor.poll(start + Duration::from_secs(60));
    assert_eq!(
        phase_of(&h.supervisor, &addr(1)),
        PadPhase::Recovering,
        "fresh budget after user-initiated adopt"
    );
}

#[test]
fn stock_pad_already_connected_at_stack_level_is_adopted() {
    // The stack auto-connects paired pads on wake; hid/connect refuses in
    // that state (error 106, captured on the TV). The stock transport
    // adopts the live session instead — no hid/connect call, no failure
    // script consumed.
    let mut h = harness_with_registry(vec![stock_entry(1)]);
    let a = addr(1);
    // The stack already has the pad connected.
    h.radio.stock_reconnects(&a);
    // A failure script that must NOT be consumed (adopting skips
    // hid/connect entirely).
    h.radio
        .inject_stock(&a, vec![bluepad_radio::fake::Injected::StackTimeout; 5]);
    let start = Instant::now();
    h.supervisor.reconcile(start);
    settle(&mut h.supervisor, start);
    assert_eq!(phase_of(&h.supervisor, &addr(1)), PadPhase::Connected);
}
