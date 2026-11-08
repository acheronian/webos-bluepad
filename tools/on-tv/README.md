# On-TV smoke test (SSH, no ipk)

A minimal way to run the 0.1.0 connection layer on the real TV **today**,
before the packaging pipeline (#34) exists: two statically-linked armv7
binaries pushed over SSH, no installer involved.

## What this validates (and what it can't)

**Validates:**
- The binaries run on the TV (musl static — no dependency on LG's glibc;
  kernel 4.4 / armv7).
- The tier probe: `luna-send` reachable → Root tier.
- The stock `bluetooth2` adapter against the live stack: `device/getStatus`
  parsing, the pairing flow (`adapter/pair` subscription — evidence-locked via
  the legacy file-by-file pass, see #32), `hid/connect` with real error codes,
  presence-consistency health.
- The socket seam end-to-end: daemon → Unix socket → `bluepad-ui` state view.
- Rotating logs land in the data dir.

**Cannot validate (hardware/packaging-gated):**
- The app-owned L2CAP path — the TV's kernel has no Bluetooth; that is
  spike #12 (and the dongle bench #6).
- The GL UI, HBC install, launcher card — ticket #34.
- S1–S5 — ticket #35, after #34.

## Build

```
rustup target add armv7-unknown-linux-musleabi   # once
cross build --release --target armv7-unknown-linux-musleabi \
    -p bluepad-daemon -p bluepad-ui
```

(`cross` builds in Docker with the armv7 toolchain. The musl target is
deliberate: a static binary sidesteps LG's old glibc entirely — punktfunk
needed shims on the gnu target; we don't.)

## Run on the TV (rooted, via SSH)

```
# from the repo root, with TV_SSH set to your ssh target (e.g. root@tv.ip)
tools/on-tv/deploy.sh
```

The script pushes both binaries to `/tmp/bluepad/` on the TV, starts the
daemon as root, prints the log tail and the UI state view, and leaves the
daemon running for interactive poking.

### The UI binary's smoke-test commands

```
# on the TV (BLUEPAD_SOCKET=/tmp/bluepad/bluepad.sock is set by deploy.sh):
bluepad-ui state             # the state view (default)
bluepad-ui watch            # subscribe; print seam notifications live
bluepad-ui pair             # open the pairing window; print progress
bluepad-ui cancel            # cancel the pairing window
bluepad-ui forget <addr>    # remove a pad (registry + settings)
bluepad-ui rumble <addr>    # rumble test pulse
bluepad-ui diag <addr>       # pad diagnostic readings (app-owned pads)
```

## Pairing probe (raw surface, before the daemon)

`tools/on-tv/pair-probe.sh` exercises the exact bluetooth2 call sequence the
daemon's stock path uses (`ls2.rs`), with every reply logged as a specimen.
Run it before the daemon flow to settle the raw surface in isolation:

```
TV_SSH=root@tv.local tools/on-tv/pair-probe.sh        # auto-picks the first candidate
TV_SSH=root@tv.local tools/on-tv/pair-probe.sh <MAC>  # or a known address
```

Put the pad in pairing mode first. One run settles, in order:

1. **`luna-send -i` subscribe mode** — the one call the original probe harness
   never proved (only `-n 1` one-shots); the daemon's pair subscription *and*
   pairable gate both depend on it. The probe aborts here if it doesn't stream.
2. The **pairable gate** (`adapter/awaitPairingRequests`).
3. **Discovery candidates** via `device/getStatus` (unpaired, named,
   peripheral-class — the daemon's candidate filter).
4. The **`adapter/pair` subscription event stream** (`displayPasskey` /
   `endPairing` …), 30 s bound, incoming-flow completion included.
5. **`hid/connect`** with real error codes; on success, the teardown
   specimens **`hid/disconnect`** and **`adapter/unpair`** (#36's calls).
6. **`luna-send-pub` presence** (the last open #32 item).

Notes:

- **`luna-send` stdin quirk (captured on the TV):** one-shots exit silently
  (zero bytes) when stdin is an open pipe — the probe pins stdin to
  `/dev/null` on every call for this reason. The daemon pins its spawned
  subscriptions' stdin too (`ls2.rs`); its one-shots were already safe.
- Stops a running `bluepad-daemon` first (the probe needs exclusive pairing;
  it also kills leftover `luna-send -i` subscribers that would hold gate
  slots) — restart with `deploy.sh` afterwards.
- Leaves the TV clean: the pad is disconnected and unpaired at the end. If
  `hid/connect` fails, it stays paired for inspection (cleanup command printed).
- All output is teed to `/tmp/bluepad-probes/pair-probe-<epoch>.log` on the TV —
  copy it back into `probe-logs/` as a specimen.

### What to look for, per test

1. **Tier probe**: `state` shows `tier Root` — the capability probe
   reached `luna-send` and the stock adapter answered.
2. **Stock adapter parsing**: with previously-stock-paired pads on the TV,
   `pair` a pad and watch `[pad]` notifications; `state` then lists it
   with its name, kind (`DualSense`), and truthful phase.
3. **Pairing flow bounds**: `pair` with no pad in pairing mode must end
   in a visible `DiscoveryTimeout` failure after 60 s — never churn.
4. **Truthful connect**: after a successful `hid/connect`, the pad shows
   `Connected` only while the stack's presence view says so; pull the
   pad's battery and watch it flip to `Reconnecting` (stock health is
   presence-consistency).
5. **Forget**: `forget <addr>` removes the pad; `state` no longer lists
   it, and `/tmp/bluepad/registry.json` loses the entry.
6. **Honest refusals**: `diag <addr>` on a stock-path pad refuses with
   the BluePad-sessions-only message; `rumble <addr>` likewise reports
   that the stock path cannot send outputs.

## Safety notes

- The daemon only talks to the radio through LG's own stack
  (`luna-send` → `bluetooth2`); it never touches the radio at kernel
  level, so it cannot fight the TV's Bluetooth more than any client of
  the stock API can.
- Pairing during the smoke test really pairs a pad to the TV (that is
  the test). `forget` removes it again.
- Everything is scoped to `/tmp/bluepad/` — deleting the directory
  removes all traces.
