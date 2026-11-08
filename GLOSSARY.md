# webos-bluepad

A webOS app that lets LG TVs pair, connect, and manage Bluetooth gamepads, and forwards their input to apps on the TV. Primary target: webOS 4.10.

## Language

**Pairing**:
The one-time establishment of Bluetooth trust between a controller and the TV; the controller is remembered afterwards.
_Avoid_: Linking, bonding

**Connecting**:
Bringing an already-paired controller into active use, after which its input begins to flow. The project's core bug is a pairing that never becomes a connection.

**Adapter**:
A Bluetooth radio available to the TV — the onboard radio or a USB Bluetooth dongle. Adapter selection is choosing which one pairs and connects controllers.

**HID dongle**:
A device that pairs controllers itself and presents them to the TV as wired USB input (e.g., DS5Dongle on a Pi Zero), bypassing the TV's Bluetooth stack entirely. Distinct from an adapter.
_Avoid_: Calling an adapter a "dongle" without qualification

**Connection layer**:
The chain from paired controller to usable input: Bluetooth connection, HID setup, input forwarding into webOS.

**App-owned HID session**:
A connection the app establishes itself: it opens the raw Bluetooth L2CAP channels to the controller, parses the HID reports, and creates the input device through the kernel's `uhid` interface — bypassing the TV's built-in HID handling entirely. Revision-agnostic by construction; v2's first-class connection path.

**Stock path**:
Pairing and connecting through the TV's built-in `bluetooth2` HID service (`hid/connect`), where LG's proprietary stack owns the whole connection. Reliable only for the gamepad generations that stack accepts (pre-2023 DualSense, per the revision research).

**Authored descriptor**:
The HID report descriptor BluePad itself writes and feeds `uhid` on the app-owned session, with button ordinals chosen so every control lands on the codes streaming apps auto-map — as opposed to the pad's real descriptor, which the kernel's generic parser scrambles.

**Full mapping**:
Every standard gamepad control a pad exposes in Bluetooth mode — face buttons, D-pad, both sticks, analog triggers, shoulders, stick clicks, system buttons — delivered on codes streaming apps actually read. A property of the authored descriptor, never of the stock path. Output features (rumble, lightbar) are settings, not mapping.
_Avoid_: Reading it as the pad's full feature set (touchpad, gyro, adaptive triggers are out of scope)

**Support tier**:
The expectation a pad kind carries on the pairing sheet before anything is tried: "Supported" (acceptance-gated, full mapping, guaranteed) or "Best effort" (pairing steps and truthful status, no guarantee). Other pads carry no tier — their wording carries the non-claim. Set per device kind, curated per release, and shown on the pairing sheet's device-type rows only.
_Avoid_: Tier words on pad cards or any post-connection surface, where truthful state takes over

**Possible causes**:
The curated list of candidate explanations shown beneath a connect-failure reason — why a paired pad isn't connecting, from pad generation to radio conditions. Keyed by failure reason and pad kind; extended per release from test reports and user feedback, never auto-generated. Deliberately never suggests re-pairing, which happens only on registry defects.

**BluePad**:
The public, user-facing name of this app — the title in the TV launcher and the Homebrew Channel listing. The repo and project name remain "webos-bluepad".
_Avoid_: Mixing the two when naming user-visible surfaces

**Streaming app**:
An app on the TV that renders a desktop or game session streamed from a host (punktfunk, Moonlight) and reads gamepad input.

**Connection daemon**:
The component that owns the connection layer at runtime — pairing, connecting, and keeping pad input flowing whether or not the UI is open. The UI is a view onto it, never the owner of a connection.
_Avoid_: "service" (overloaded on webOS), "background app"

**Radio transport**:
The means by which the connection layer reaches a Bluetooth adapter — through the TV's own stack, or through hardware access of its own. Deliberately unnamed until on-device proof picks one; the connection layer must not assume which.

**Privilege tier**:
The capability level the environment actually grants the connection layer — full (rooted TV) or limited (jailed) — derived by probing what works, never assumed from how the app was installed. The tier bounds what BluePad can honestly promise: background input with all pads, or app-open input with supported pads only.

**Pad registry**:
The connection daemon's persistent record of every paired pad — its address, kind, proven connection path, pairing provenance, and per-pad settings. Outlives any individual connection.
_Avoid_: Device list, paired list

**Health check**:
Active verification that a connected pad's input is genuinely flowing. A pad only counts as connected once it passes; a session the stack reports as up but that streams nothing is recovering, not connected.
_Avoid_: Heartbeat, "connected" as a stack claim

**Update check**:
The UI's courtesy look at the app's release manifest to learn whether a newer version exists. Informational only — it reports and points to the Homebrew Channel; it never installs anything. The connection daemon takes no part in it.
_Avoid_: Auto-update, self-update

**Contending service**:
Another app's userspace service that competes with BluePad for pads or the radio (the original GamePads app's helper is the first known one). Known from a curated list; while one is live, pairing is blocked with an honest reason rather than left to fail mysteriously.

**Tombstone manifest**:
A release manifest whose only job is to announce that the channel it lives on is discontinued and to name the successor — the mechanism by which already-installed versions reroute their update check after the project moves hosts.
_Avoid_: Calling it a release (it installs nothing)

**Acceptance matrix**:
The five end-to-end scenarios (S1–S5) that define "reliable" for the connection layer, every one requiring input flowing in a streaming app: per-device reconnect resilience, truthful connect state, simultaneous pads, and silent-failure auto-recovery, cross-cut by pad sleep, TV standby, and app restart. The outer gate the shipped app must pass — as distinct from substrate probes and baselines recorded before the app exists.
_Avoid_: Calling any pre-app probe run "acceptance"
