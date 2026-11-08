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
  parsing, the pairing flow (`adapter/pairDevice` — unverified, see #32),
  `hid/connect` with real error codes, presence-consistency health.
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
daemon running for interactive poking:

```
# on the TV:
BLUEPAD_SOCKET=/tmp/bluepad/bluepad.sock /tmp/bluepad/bluepad-ui
```

Pairing can be driven by hand over the seam (JSON lines on the socket):

```
{"type":"request","id":1,"payload":{"type":"start_pairing"}}
```

## Safety notes

- The daemon only talks to the radio through LG's own stack
  (`luna-send` → `bluetooth2`); it never touches the radio at kernel
  level, so it cannot fight the TV's Bluetooth more than any client of
  the stock API can.
- Pairing during the smoke test really pairs a pad to the TV (that is
  the test). `forget` removes it again.
- Everything is scoped to `/tmp/bluepad/` — deleting the directory
  removes all traces.
