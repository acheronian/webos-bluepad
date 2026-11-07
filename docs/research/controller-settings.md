# Controller-specific settings by vendor and type — what belongs in BluePad 0.1.0

Research ticket: [Research: Controller-specific settings by vendor and type — what settings belong in BluePad](https://github.com/acheronian/webos-bluepad/issues/17).
Answers the settings half of the app-owned HID session ([Grilling: Lock the connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)): which per-controller settings (lightbar, player LEDs, rumble, deadzones, remapping) the ecosystems standardize, which of them BluePad can actually reach from its own L2CAP session on webOS 4.10, and how the catalogue should be keyed and stored. Scope: DualSense primary, DualShock 4, Xbox One S, generic HID pads — the types the [UI prototype](https://github.com/acheronian/webos-bluepad/issues/8) already gives pairing help for.

## 0. Answer in brief

- **The ecosystems converge on a small catalogue**: rumble/vibration intensity (PS5, PS4, Xbox, Steam Input), lightbar/indicator brightness (PS5, PS4), player identification (PS4 lightbar color, DualSense player LEDs), stick deadzones (Steam Input; consoles hide them), button remapping (Xbox Accessories, PS5 accessibility, Steam Input), and battery display. Per-type pairing help is already BluePad's own convention from the UI prototype.
- **The map's out-of-scope reason for the DualSense lightbar is factually wrong, and it matters.** `hid-playstation` does not reach the pad through some privileged channel: on Bluetooth it sends one ordinary HID output report (ID `0x31`, 78 bytes, CRC32-seeded) carrying rumble, lightbar RGB, and player LEDs — the very channel BluePad's app-owned session owns. **Rumble, lightbar color, lightbar brightness, and player LEDs are reachable from the app-owned L2CAP session with no kernel driver at all.** What genuinely needs more than that — adaptive triggers, voice/coarse haptics, touchpad, gyro, speakers — is already out of scope and useless to a streaming-forwarder anyway. The stock path on 4.10, by contrast, controls none of it (no Sony HID driver in LG's kernel, no `hidraw` in the app jail).
- **One hard wall**: game-driven rumble is impossible through BluePad's `uhid` device on the TV's 4.4 kernel — `uhid.c` in v4.4 registers no force-feedback capability and 4.4 has no generic HID PID force-feedback driver. Rumble in 0.1.0 can therefore only be **BluePad-initiated** (test pulse, connect/disconnect feedback), scaled by an intensity setting. This is a forwarder, not a game host: nothing would drive FF anyway.
- **Recommended 0.1.0 catalogue** (per pad, any type): stick deadzones, button remap, battery display, player-index assignment; per type where reachable: lightbar color + brightness (DualSense, DS4), player LEDs (DualSense), rumble test + intensity (DualSense, DS4, Xbox One S incl. impulse triggers); global: per-type pairing help (already prototyped). Deliberately out: adaptive triggers, haptics, touchpad/gyro, audio — the map's ruling stands for those.
- **Keying and storage**: key *capabilities* by controller type derived from VID/PID (DualSense `054c:0ce6`, Edge `054c:0df2`, DS4 `054c:05c4`/`054c:09cc`, Xbox One S BT `045e:02fd`, BLE `045e:0b20`, Series BT `045e:0b13`), key *per-pad values* by Bluetooth MAC (stable across re-pairs, known from pairing onward), with name as a fallback hint. Store as one versioned JSON document at `$HOME/settings.json` in the app jail — punktfunk's exact precedent (whole-document load/save, version stamp, unparseable → rename `.bad` + defaults).

## 1. What the ecosystems standardize (primary sources)

### 1.1 PlayStation 5 console (DualSense)

Sony's official "general controller settings" page ([playstation.com](https://www.playstation.com/en-us/support/hardware/adjust-general-controller-settings/), *How to change general controller settings on PS5 consoles*) lists, under **Settings → Accessories → Controller (General)**:

- **Volume (Speaker for Controller)** — pad speaker volume.
- **Vibration Intensity** — "Set the intensity of the DualSense wireless controller vibration", applies to DualShock 4 too.
- **Trigger Effect Intensity** — adaptive-trigger intensity.
- **Brightness of Controller Indicators** — "Set the brightness of the indicators on your wireless controller", applies to DS4.
- **Communication Method** and **Press and Hold Delay** (not settings a forwarder needs).

So the console exposes *intensity sliders*, not just on/off, and treats indicator brightness as a first-class setting. Lightbar *color* is not user-settable on PS5 — the console reserves color for player/system state; the player number shows on the dedicated player LEDs (the kernel mirrors the console's centered mapping, [hid-playstation.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c): player 1 → `--x--`, player 2 → `-xx--`, player 3 → `x-x-x`, player 4 → `xxxxx`).

### 1.2 PlayStation 4 console (DualShock 4)

Sony's official PS4 counterpart ([playstation.com](https://www.playstation.com/en-us/support/hardware/ps4-controller-settings/), *How to change general controller settings on PS4 consoles*), under **Settings → Devices → Controllers**:

- **Volume Control (Speaker for Controller)**
- **Enable Vibration** — on/off only.
- **Brightness of DUALSHOCK 4 Light Bar**
- **Communication Method** (wired/BT, CUH-ZCT2 only).

On PS4 the lightbar *color* is the player indicator: blue, red, green, pink for players 1–4 — the convention the Linux kernel replicates verbatim ([hid-playstation.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c), `dualshock4_set_default_lightbar_colors`: "Use same player colors as PlayStation 4" — Blue `#000040`, Red `#400000`, Green `#004000`, Pink `#200020`).

### 1.3 Xbox (One S / Series)

- **Vibration**: official support article [Change controller vibration on XBOX, cloud gaming, and Windows devices](https://support.xbox.com/en-US/help/hardware-network/accessories/change-controller-vibration-xbox-one-windows-10): "you can adjust their intensity or turn them off completely" — per-controller intensity, adjustable or off.
- **Button remapping**: the Xbox Accessories app's core function; first-party description in [Keyboard Button Mapping for Xbox Controllers](https://news.xbox.com/en-us/2023/08/03/keyboard-button-mapping-for-xbox-controllers/) (Xbox Wire): "Choose any button … and remap it". Remap configs are stored as per-controller **profiles** (assignable per controller; the Elite line additionally exposes stick/trigger sensitivity curves in the same app). The Xbox One S over Bluetooth has no player LEDs and no user-controllable LED — its single "nexus" LED shows connection state on its own.

### 1.4 Steam Input (the PC ecosystem's reference)

Valve's Steamworks documentation describes the full surface ([Steam Input](https://partner.steamgames.com/doc/features/steam_controller), [Input Source Modes](https://partner.steamgames.com/doc/features/steam_controller/input_source_modes), [device pages](https://partner.steamgames.com/doc/features/steam_controller/device)):

- **Per-game and global configurations**, stored per controller, fully **remappable** (the configurator *is* remapping).
- **Deadzone** as a per-input setting: "the radius of the central region to ignore, 0.0–1.0", plus **Anti-Deadzone** and **Anti-Deadzone Buffer** for compensating game-side deadzones.
- **Haptics Intensity**: Off/Low/Medium/High per input.
- The PS4 controller exposes "a programmable LED light bar" ([device page](https://partner.steamgames.com/doc/features/steam_controller/device/ps4_controller)); Steam programs color and brightness.

### 1.5 What this implies for BluePad

Convergent, ecosystem-standard settings: **rumble intensity, lightbar/indicator brightness, player identification, deadzones, button remapping, battery display**. Nintendo's Switch offers the same shape (player LEDs, rumble intensity, button mapping in system settings; its Pro controller is "generic" for us and not worth a catalogue entry of its own). BluePad is a *forwarder*, so its deadzone/remap settings apply to what it forwards — they are app-side transforms, not controller features. Lightbar/player-LED/rumble settings are controller features, and reachability is the next question.

## 2. What's reachable from the app-owned L2CAP session on webOS 4.10

The decisive fact: **on Bluetooth, everything a console does to a DualSense/DS4 — rumble, lightbar, player LEDs — travels as ordinary HID output reports on the L2CAP HID channels.** The kernel's `hid-playstation` driver, when the pad is on Bluetooth, builds a 78-byte output report with report ID `0x31` and sends it via `hid_hw_output_report()`; over BT HID that lands on the interrupt channel BluePad already owns ([hid-playstation.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c), `dualsense_init_output_report` / `dualsense_send_output_report`). No kernel driver is privileged here; the map's out-of-scope line conflated the *stock path* (which on 4.10 indeed has no Sony LED/rumble plumbing) with the app-owned path (which has it all).

### 2.1 DualSense (primary target)

All from [hid-playstation.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c) (structs `dualsense_output_report_bt`/`dualsense_output_report_common`, `dualsense_output_worker`) and SDL's independent implementation ([SDL_hidapi_ps5.c](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c)):

- **Report**: ID `0x31`, 78 bytes total: `report_id(0x31)`, `tag(0x10)`, `seq_tag` (4-bit incrementing sequence in the high nibble), 47-byte common payload, trailing **CRC32** computed as `~crc32(crc32(0xFFFFFFFF, {0xA2}, 1), report[0..len-4])`.
- **Rumble**: `valid_flag0` bit 0 (`COMPATIBLE_VIBRATION`) + bit 1 (`HAPTICS_SELECT`), `motor_left`/`motor_right` bytes at common offsets 2–3. Firmware "update version" ≥ 2.21 instead uses `valid_flag2` bit 2 (`COMPATIBLE_VIBRATION2`) — the "v2" feel that matches PlayStation behavior; both rev-A (2021–22) and rev-B (2023+) pads exceed that (see the [revision research](https://github.com/acheronian/webos-bluepad/issues/11)).
- **Lightbar**: `valid_flag1` bit 2 (`LIGHTBAR_CONTROL_ENABLE`) + RGB bytes near the end of the common payload; `valid_flag2` bit 1 (`LIGHTBAR_SETUP_CONTROL_ENABLE`) + `lightbar_setup`/`led_brightness` for brightness/fade — the kernel explicitly resets the lightbar this way on connect ("On Bluetooth the DualSense outputs an animation on the lightbar during startup and maintains a color afterwards. We need to explicitly reconfigure the lightbar…").
- **Player LEDs**: `valid_flag1` bit 4 (`PLAYER_INDICATOR_CONTROL_ENABLE`) + 5-bit `player_leds` mask; PS5-style centered per-player patterns.
- **Battery**: input report status byte `DS_STATUS0_BATTERY_CAPACITY` (low nibble, 0–10) — BluePad parses input reports anyway.
- **Bluetooth quirk that the implementation must know**: out of the box over BT the pad emits *simple reports*; the host switches it to *enhanced reports* by reading feature reports `0x09` (serial = BT address reversed) and `0x20` (firmware info) — SDL: "This will also enable enhanced reports over Bluetooth" ([SDL_hidapi_ps5.c](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c)). Feature reports travel on the L2CAP control channel, which the app-owned session also owns.
- Also present in the same report (audio routing, mic mute LED, power-save): irrelevant to a forwarder; leave unused.

### 2.2 DualShock 4

From the same driver (structs `dualshock4_output_report_bt`/`_common`, `dualshock4_output_worker`):

- **Report**: ID `0x11`, 78 bytes: `report_id(0x11)`, `hw_control` (bit 7 = HID cmds, bit 6 = CRC present, bits 0–5 = BT poll interval — the kernel keeps the BT link alive with this byte), then common payload with `valid_flag0`/`valid_flag1`, **motor bytes at offsets 3–4**, **lightbar RGB at offsets 5–7**, blink on/off at 8–9, trailing CRC32 (same `0xA2` seed scheme).
- `valid_flag0` bit 0 = motors, bit 1 = lightbar, bit 2 = lightbar blink; the kernel always sends rumble and lightbar together (`valid_flag0 >= 0x03`) for third-party pad compatibility.
- Player identification = lightbar color (PS4 convention, §1.2). Battery in status byte like DualSense.

### 2.3 Xbox One S over Bluetooth

From SDL's hidapi driver ([SDL_hidapi_xboxone.c](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_xboxone.c), `HIDAPI_DriverXboxOne_UpdateRumble`) and xpadneo ([rumble.c](https://github.com/atar-axis/xpadneo/blob/master/hid-xpadneo/src/xpadneo/rumble.c), [xpadneo.h](https://github.com/atar-axis/xpadneo/blob/master/hid-xpadneo/src/xpadneo/xpadneo.h)):

- **Rumble**: HID output report **`0x03`**, 9–10 bytes: `{0x03, 0x0F, 0x00, left_trigger, right_trigger, low_freq, high_freq, 0xFF, 0x00, 0xEB}` — four motors including the impulse triggers. SDL rate-limits to one packet per 50 ms over BT.
- **That's the whole surface.** The One S's own BT HID descriptor ([captured by xpadneo](https://github.com/atar-axis/xpadneo/blob/master/docs/descriptors/xb1s_linux.md)) declares exactly one output report (`0x03`, on the HID **PID usage page** — rumble is a spec-standard physical-interface effect report) and no LED output report; the nexus LED is autonomous. So: rumble yes (intensity + impulse triggers), player/lightbar LEDs no. (Over USB/GIP there is a `0x0A` LED command — SDL's Home-LED hint — but that transport doesn't apply to us.)

### 2.4 Generic HID pads

By definition nothing type-specific is reachable: the catalogue degrades to the app-side settings (deadzones, remap, battery if the input report yields it, player index shown in the UI only). BluePad's own uhid device name carries the "(BluePad)" suffix per the identity decision, so the UI can label pads even when the type is unknown.

### 2.5 The force-feedback wall on the 4.4 kernel

Could games rumble through BluePad's virtual device? No, on this kernel:

- A `uhid`-created device gets its input core from `hid-input` parsing our authored descriptor, but **v4.4's `uhid.c` never registers force feedback** (no `input_ff_create` anywhere in the driver; [uhid.c v4.4](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/uhid.c?h=v4.4)).
- v4.4 has no generic HID PID force-feedback driver either (the `drivers/hid/` directory of that tag has per-vendor FF drivers only; a generic `hid-pidff` is not in v4.4 nor in current mainline) — so even authoring a PID collection into our descriptor would create no FF device on the TV's kernel.
- Consequence: rumble in 0.1.0 is **BluePad-initiated only** (settings test pulse, connect/disconnect feedback). That is honest and still useful; a "rumble intensity" setting scales those. Game-driven rumble would need kernel FF support that 4.4 cannot give `uhid` devices — a documented dead end, not a TODO.

### 2.6 Stock path on 4.10: settings apply only on the app-owned session

On the stock path LG's stack owns the HID session; there is no `hidraw` node in the app jail and no Sony/Microsoft LED-rumble plumbing in LG's 4.4 kernel (the map's diagnosis). punktfunk on *newer* webOS sends DualSense output via `bluetooth2 hid/internal/sendData` — but that was verified on webOS 10.3 ([punktfunk research](https://github.com/acheronian/webos-bluepad/issues/3)); nothing says 4.10's `bluetooth2` exposes it, and the map already treats the app-owned session as the first-class path. **Settings model: lightbar/player-LED/rumble settings apply when the pad is on the app-owned session; on the stock path they show as unavailable.** Per-device runtime routing (strategy decision) already anticipates mixed paths.

## 3. Recommended catalogue for BluePad 0.1.0

Per pad (any type — app-side transforms, always available):

| Setting | Default | Notes |
|---|---|---|
| Stick deadzone (L/R) | 0% | Radial, applied in the input-report → uhid translation; streaming hosts apply their own deadzones, so 0 is the honest default |
| Button remap | identity | Per-pad map applied in the same translation layer; ecosystem-standard (§1.3, §1.4) |
| Battery | display-only | Parsed from input reports where available (DualSense/DS4 status byte, Xbox report `0x04`) |
| Player index | auto = connection order, manual override | Drives the LED/lightbar defaults and the UI label |

Per type (controller features, app-owned session only, §2.6):

| Setting | DualSense | DS4 | Xbox One S | Generic |
|---|---|---|---|---|
| Lightbar color | ✔ (default = player color) | ✔ (default = player color) | — | — |
| Lightbar brightness | ✔ | ✔ | — | — |
| Player LEDs | ✔ (PS5-centered pattern) | — (lightbar *is* the indicator) | — (none on BT) | — |
| Rumble test + intensity | ✔ | ✔ | ✔ (incl. impulse triggers, one intensity scales all four motors) | — |
| Pairing help | PS5 steps | PS4 steps | Xbox steps | generic HID steps |

Global: per-type pairing help (already locked in the [UI prototype](https://github.com/acheronian/webos-bluepad/issues/8) — this ticket adds nothing there), plus a "connection feedback" toggle (lightbar blink + rumble pulse on connect/disconnect) as the one place rumble is *used* rather than tested.

Deliberately **not** in 0.1.0, consistent with the map's scope ruling: adaptive triggers, voice/coarse haptics, touchpad, gyro, speaker/mic (useless to a streaming-forwarder; the lightbar/rumble correction above is the only place the map's stated *reason* was wrong), game-driven rumble (§2.5 — impossible through `uhid` on 4.4).

## 4. Keying per vendor/type, and storage

### 4.1 Identity available to the app

At pairing/connect the app-owned session has: **BDADDR (MAC)** — always, stable across re-pairs; **device name** from SDP/EIR; **VID/PID** from the SDP Device ID record where the pad provides one. Bluetooth product IDs differ from USB ones and are the correct key (from [hid-ids.h](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-ids.h) and [hid-playstation.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c)):

- Sony `054c`: DualSense `0ce6`, DualSense Edge `0df2`, DS4 v1 `05c4`, DS4 v2 `09cc`
- Microsoft `045e`: Xbox One S (1708) BT `02fd`, One S BLE `0b20`, Series X|S BT `0b13`, Elite 2 BLE `0b22`

### 4.2 Recommended keying

Two layers, matching how the settings behave:

1. **Type** — a small enum (`DualSense`, `DualSenseEdge`, `DS4`, `XboxOneS`, `XboxSeries`, `GenericHid`) derived from VID/PID, with name-substring fallback ("Wireless Controller" + VID, "Xbox Wireless Controller") for pads whose SDP lacks Device ID. The type owns the **capability matrix** (§3) and the per-type defaults.
2. **Pad** — keyed by normalized MAC (lowercase hex, no separators). The pad owns the **values**. MAC survives re-pairing and is known before the first connect; VID/PID alone can't distinguish two identical pads (the multi-controller scenario is in scope).

### 4.3 Storage

punktfunk's precedent, which the architecture research already blessed: one **`settings.json` in the app jail's `$HOME`** ([client-webos `store/mod.rs`](https://github.com/punktfunk/client-webos/blob/main/src/services/store/mod.rs), [`paths.rs`](https://github.com/punktfunk/client-webos/blob/main/src/services/paths.rs): `app_dir() = $HOME` under the per-app jail; whole-document load-before-save; version stamp; an unparseable document is renamed `settings.json.bad` and defaults take over — "no migration: a document this build cannot read is renamed first").

Suggested shape:

```json
{
  "schema_version": 1,
  "connection_feedback": true,
  "defaults_by_type": {
    "dualsense": { "rumble_intensity": 0.8, "lightbar_brightness": "medium" }
  },
  "pads": {
    "aabbccddeeff": {
      "type": "dualsense",
      "name": "DualSense Wireless Controller",
      "player_index": 1,
      "deadzones": { "left": 0.0, "right": 0.0 },
      "button_map": {},
      "lightbar": { "mode": "player-color", "color": null, "brightness": "medium" },
      "rumble_intensity": 0.8
    }
  }
}
```

Unknown keys ignored (forward-compat), absent pads fall back to `defaults_by_type[type]` then built-ins. The stock path needs no different storage — it simply renders feature rows as unavailable (§2.6).

## 5. Source index

- Linux `hid-playstation.c` (master): DualSense/DS4 BT output report `0x31`/`0x11` layouts, valid-flag bits, CRC32 scheme (`0xA2` seed), vibration-v2 firmware gate (update version ≥ 2.21), player-LED centered patterns, PS4 player lightbar colors, battery status bits — [git.kernel.org](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-playstation.c).
- Linux `uhid.c` (v4.4): no force-feedback registration — [git.kernel.org](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/uhid.c?h=v4.4); v4.4 `drivers/hid/` tree listing (no generic PID FF driver).
- Linux `hid-ids.h`: Bluetooth VID/PIDs — [git.kernel.org](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/drivers/hid/hid-ids.h).
- SDL3 `SDL_hidapi_ps5.c`: BT enhanced-report activation via feature reports `0x09`/`0x20`, report `0x31`, capability flags — [libsdl-org/SDL](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c).
- SDL3 `SDL_hidapi_xboxone.c`: BT rumble report `0x03` (4 motors, 50 ms pacing) — [libsdl-org/SDL](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_xboxone.c).
- xpadneo: BT rumble report `0x03` (9 bytes) and the captured Xbox One S Bluetooth HID descriptor (single output report, PID usage page, no LED) — [atar-axis/xpadneo](https://github.com/atar-axis/xpadneo/blob/master/hid-xpadneo/src/xpadneo/rumble.c), [descriptor](https://github.com/atar-axis/xpadneo/blob/master/docs/descriptors/xb1s_linux.md).
- Sony Interactive: PS5 controller settings (Vibration Intensity, Trigger Effect Intensity, Brightness of Controller Indicators) — [playstation.com](https://www.playstation.com/en-us/support/hardware/adjust-general-controller-settings/); PS4 controller settings (Enable Vibration, Brightness of DUALSHOCK 4 Light Bar) — [playstation.com](https://www.playstation.com/en-us/support/hardware/ps4-controller-settings/).
- Xbox: vibration intensity adjustable/off — [support.xbox.com](https://support.xbox.com/en-US/help/hardware-network/accessories/change-controller-vibration-xbox-one-windows-10); remapping via Xbox Accessories app — [news.xbox.com](https://news.xbox.com/en-us/2023/08/03/keyboard-button-mapping-for-xbox-controllers/).
- Valve Steamworks: Steam Input handbook, deadzone/anti-deadzone, haptics intensity, programmable PS4 lightbar — [partner.steamgames.com](https://partner.steamgames.com/doc/features/steam_controller), [input source modes](https://partner.steamgames.com/doc/features/steam_controller/input_source_modes), [PS4 device page](https://partner.steamgames.com/doc/features/steam_controller/device/ps4_controller).
- punktfunk client-webos: `settings.json` at jail `$HOME`, whole-document store, `.bad` rename policy — [store/mod.rs](https://github.com/punktfunk/client-webos/blob/main/src/services/store/mod.rs), [paths.rs](https://github.com/punktfunk/client-webos/blob/main/src/services/paths.rs); SettingsStore seam — [pf-console-ui store.rs](https://git.unom.io/unom/punktfunk/src/branch/main/crates/client/pf-console-ui/src/store.rs).
