# webos-bluepad

**BluePad** — a native Rust gamepad manager for webOS 4.10 TVs. Pairs and
connects Bluetooth gamepads of any DualSense revision (including 2023+
retail stock), keeps their input flowing into streaming apps (punktfunk,
Moonlight), and manages them from a TV-native UI.

The inherited Svelte + Node stack (a fork of SuperD22/webos-gamepad) is
kept under `legacy/` as reference material; per
[ADR 0001](docs/adr/0001-full-native-rust-app.md) this is a full native
rebuild — there is no Node stopgap.

## Status

Rust workspace (issue #27, spec: BluePad v2 — 0.1.0):

- **bluepad-core** — the domain: the per-kind pad kit (authored HID
  descriptor, report parsers, output reports), the connection layer
  (per-pad state machines, health-checked Connected, bounded recovery,
  modal pairing), the versioned settings document, the pad registry, and
  the UI↔daemon protocol vocabulary.
- **bluepad-radio** — the transports behind the core traits: the
  app-owned session (raw L2CAP + `uhid`), the stock `bluetooth2` adapter
  over LS2, and the scripted fakes the connection-layer tests drive.
- **bluepad-daemon** — the root connection daemon: Unix-socket seam
  speaking JSON subscribe/notify, poll loop, request dispatcher,
  rotating Debug logs.
- **bluepad-ui** — the jailed, synchronous view: `DaemonClient` facade
  plus a text-mode state view (the GL shell lands with packaging).

Tests observe only the three pre-agreed seams (kit, radio transport,
socket protocol); the S1–S5 acceptance matrix on real hardware is the
outer gate for release.

```
cargo test        # 75 tests across the workspace
cargo clippy      # clean
```

## Gated on hardware (separate issues)

- Raw L2CAP + `uhid` feasibility on the TV's onboard radio — #12
- Bluetooth link-key access for app-owned pairing — #28
- USB Bluetooth dongle bench-test — #6
- Xbox One S whitelist probe — #26
