# Research: DualSense pairs-but-never-connects on webOS 4.10

Research ticket: acheronian/webos-bluepad#2
Branch: `research/dualsense-webos410`
Date: 2026-11-07

---

## 1. Executive summary

- **LG webOS 4.x TVs do not ship BlueZ at all.** LG's own open-source notices and source drops for webOS 4.0 (GMPP, 2018) and webOS 4.5 (IKLP, 2019, final drop IKLP_4.0 2022) contain no `bluez` component, and the kernel defconfig for the UM7100-series (Realtek rtd287o) enables no kernel Bluetooth subsystem. The Bluetooth stack is LG-proprietary userspace (`webos-bluetooth-service` + `libbluetooth.default.so` + an HID manager), which creates input devices through the kernel's `uhid` interface (confirmed `CONFIG_UHID=y`). So "which bluez version does webOS 4.10 ship" has a surprising answer: none — and every BlueZ-level intuition must be re-checked against LG's proprietary stack instead.
- **LG's HID handling is device-aware, driven by a gamepad whitelist.** On newer webOS (6.5, verified by the chiaki-lg-fork project on a G1) the configd key `com.webos.service.bthidmanager.supportedBtGamepadList` whitelists Xbox One S (045E:02FD), Xbox Series (045E:0B13), DualShock 4 (054C:05C4 and 054C:09CC), NVIDIA controller (0955:7214) and Luna gamepad (0171:0419) — **the DualSense (054C:0CE6) is not in the list**, and LG's `bthidmanager` logs it as `type=not_support_gamepad, vid=0x54c, pid=0xce6`. LG's kernel even has a custom Kconfig (`SONY_DUALSHOCK_4_USER_DRIVER`) that *removes* DS4 from the in-kernel `hid-sony` driver so LG's **user-level** driver can own it — more evidence the HID path is device-specific in LG's userspace.
- **No kernel HID driver knows the DualSense on this era of kernel.** LG's webOS 4.5 kernel is Linux 4.4.84; `hid-playstation` (the DualSense driver) first appeared in Linux v5.12, and LG's 4.4.84 tree's `hid-ids.h` has no `0x0ce6` entry. If an HID connection *were* established, the pad would bind `hid-generic` and produce basic buttons/axes — exactly what happens on webOS 5/6 TVs, where the DualSense "pairs, connects, and works as an input, minus feedback" (punktfunk and chiaki evidence).
- **Ranked root cause (H1):** LG's proprietary HID backend on webOS 4.x refuses or fails the HID profile *connection* for gamepads not on its supported list, so `hid/connect` fails (likely error 130) and no `uhid` device is ever created. Competing hypotheses (input-node creation failure, protocol mismatch with the DualSense's 78-byte CRC32-framed BT reports, pairing/race conditions) are ranked below with discriminating predictions, and the TV-side checklist at the end is designed to separate them in one session with root SSH.

---

## 2. Method and primary sources actually consulted

Every claim below cites a source that was fetched during this research session:

- LG's open-source portal `https://opensource.lge.com` (product search + OSS notice and source downloads for 55UM7100PUA/PVB, webOS 4.5 "IKLP" drops, and webOS 4.0 "GMPP" for UK6400; commands and file names given inline).
- LG's published source archives (downloaded and inspected locally):
  - `webOS 4.5 IKLP_4.0_1.tar.gz` (2,302,844,306 bytes) — Chromium sources only (137,681 files, all under `chromium/`).
  - `webOS 4.5 IKLP_4.0_2.tar.gz` (3,549,003,390 bytes) — `IKLP-r3247/` platform OSS components + toolchain; contains `common/RTK_Kernel_linux.zip` (kernel) and **no bluez tarball**.
  - OSS notices: `OSSNotice-1319_webOS 4.5 IKLP_190304.html` (2019-03-04) and `OSSNotice-4153_webOS 4.5 IKLP_4.0_220114.html` (2022-01-14), plus the webOS 4.0 GMPP notice.
- webOS OSE source code on GitHub: `webosose/webos.service.bluetooth2`, `webosose/com.webos.service.bluetooth2`, `webosose/bluetooth-sil-bluez5`, `webosose/bluetooth-sil-api`, `webosose/meta-webosose` (raw file fetches, cited by path).
- webOS OSE official docs: `https://www.webosose.org/docs/reference/ls2-api/com-webos-service-bluetooth2/` and the LS2 API index.
- Upstream Linux kernel tree at kernel.org: `drivers/hid/hid-playstation.c`, `hid-sony.c` (v3.10), `hid-microsoft.c` (v3.10 and v5.4), `hid-ids.h`, existence checks against release tags v3.10/v5.10/v5.11/v5.12.
- Upstream Superd22/webos-gamepad: issues #2, #5, #6, #7 (via `gh issue view` / `gh api`), plus the app source in this repo (fork).
- webosbrew: `webosbrew/apps-repo` issue #20 (all comments), `https://www.webosbrew.org/` site structure, `https://www.webosbrew.org/pages/luna-service-comwebosservicebluetooth.html`.
- Related TV projects with hardware-verified findings: `Hubert-Rybak/chiaki-lg-fork` issues #2 and #4 and `packaging/root/bluetooth-runtime.sh` + `src/bt_patch.c`; `punktfunk/client-webos` issue #96 and README; `mariotaku/moonlight-tv` issue #362; `chros73/bscpylgtv` settings dumps; `ohmybahgosh/WEBOS-LUNA-SEND-ATLAS` bthidmanager service doc.
- BlueZ upstream: `https://git.kernel.org/pub/scm/bluetooth/bluez.git` log search (for DualSense-specific handling).

Things that could **not** be verified are listed explicitly in §9.

---

## 3. The symptom, precisely

### 3.1 Upstream Superd22/webos-gamepad issue #7 (closed undiagnosed)

> "webos version 4.9.7, TV: 55um7100plb, I am pairing Dualsense gamepad in the app and it's being shown as Wireless Controller (just like on PC), but inputs are not working. IHSPlay (Steamlink for WebOS) shows that there is no gamepad connected and Moonlight doesn't get any input."

- Source: https://github.com/Superd22/webos-gamepad/issues/7 (author `tomalek0`, fetched via `gh issue view 7 -R Superd22/webos-gamepad`).
- The issue was closed on 2023-08-28T21:16:03Z with `state_reason: "completed"` but has **0 comments** and no diagnosis (`gh api repos/Superd22/webos-gamepad/issues/7` → `{"closed_at": "2023-08-28T21:16:03Z", "comments": 0, "state_reason": "completed"}`; the only issue event is `closed`). The reporter's TV (55UM7100PLB) is the same Realtek-rtd287o platform whose source LG published as "webOS 4.5 IKLP" (see §4.3), and "webOS 4.9.7" is a late firmware of the same webOS 4.x line — the same family as the maintainer's webOS 4.10 TV.
- Note the phrasing: pairing succeeded and the device name ("Wireless Controller") was read — so inquiry, SDP name lookup and pairing all worked. What is unknown from this report is whether the app's `hid/connect` step returned success or an error, because the reporter only observed "no input anywhere".

### 3.2 webosbrew community reports

- **There is no public webosbrew forum.** `https://www.webosbrew.org/` links only to GitHub (`webosbrew/` org) and a Discord invite (`https://discord.gg/xWqRVEm`) (fetched from the site's link list). Discord is not publicly archivable, so any DualSense reports there are outside this research's reach.
- The closest public equivalent is **webosbrew/apps-repo issue #20 "[IDEA] Bluetooth HID manager"** (https://github.com/webosbrew/apps-repo/issues/20), filed by mariotaku (moonlight-tv author), which is the direct ancestor of webos-gamepad. Verified facts from its comments:
  - Issue body: "webOS has `com.webos.service.bluetooth2` service that seems to have capability to pair & connect to HID devices, down to webOS 4. While they only provide pairing to their official keyboards, and only supports game controllers on webOS 6."
  - Informatic (webosbrew core) reverse-engineered the **webOS 3.x** service `com.webos.service.bluetooth` (daemon `/usr/sbin/btmngr`) and found its HID support is Sony-specific: a `service/enable {"service":"hid_sony"}` call exists, and the community-documented API page states "'HID' here generally means Sony DualShock4 controllers" (https://www.webosbrew.org/pages/luna-service-comwebosservicebluetooth.html, "Reversed on webOS 3.8").
  - DS4 (DualShock 4) **does** pair and connect on old webOS via these services: Informatic and stephank got a DS4 working on webOS 3.x (`/dev/input/js3` and `event6` producing events; RetroArch picking it up), czak reproduced on OLED55B7V (core_os_release 3.9.0-62709), and manny8975 on webOS 5.3.1 (65NANO796NE) using bluetooth2's `adapter/pair` + `hid/connect`.
  - Recurring webOS HID quirks reported there: events don't arrive right after the *first* pairing (fixed by a disconnect/reconnect or controller power cycle), and after TV standby the pad reconnects but input is not registered until re-pairing (manny8975, webOS 5.3.1).
- No webosbrew-space report of a DualSense *connecting for input* on any webOS < 5 was found (searched GitHub issues repo-wide for `dualsense webos`).

### 3.3 This repo's context

- The fork's app pairs via `com.webos.service.bluetooth2` `adapter/pair` (subscription) and then calls `hid/connect` on `endPairing` (`app/src/lib/DeviceFinder.svelte` lines 54–97; service wrapper `services/web-input/src/hid-bluetooth.service.ts`).
- Upstream webos-gamepad issue #2 ("Gamepads sometimes gets input sometimes not") documents the generic 4.x/5.x failure mode: devices "always register in `/proc/bus/input/devices` but not in their `/dev/input/eventN`", and the fork carries a `init-hids` command that deletes stale `/dev/input` nodes for that reason (`services/web-input/src/commands/init-hids.command.ts`). This is the known *connects-but-silent* failure mode, distinct from the DualSense *never-connects* symptom.

---

## 4. What LG actually ships on webOS 4.x (from LG's own source drops)

### 4.1 No BlueZ on the TV

- The webOS 4.5 IKLP OSS notice (2022-01-14, fetched from opensource.lge.com, osSeq 510775) lists 583 OSS components — including `sbc` and `bluez-alsa` (MIT parts) — but **no `bluez` core** (`grep -ic bluez` → only the `bluez-alsa` entry and its URL). The initial 2019-03-04 IKLP notice has **zero** `bluez`-substring matches. The webOS 4.0 GMPP notice (`OSSNotice-748_webOS 4.0 GMPP_1.1_180220`, fetched via opensource.lge.com, osSeq 508446, 469 table rows) likewise has zero `bluez`-substring matches.
- The IKLP_4.0 source archive part 2 (`IKLP-r3247/`) contains 131 entries: OSS tarballs (glib 2.48.2, dbus 1.10.10, eudev 3.2, SDL 1.2.15, connman, ffmpeg 3.3, …) plus the kernel zip and toolchain — **no bluez source tarball**. Part 1 is Chromium only.
- Since BlueZ is GPL-2.0 (and `bluez-alsa`'s presence shows LG does list BT components when shipped), the absence from both the notice and the drop is strong evidence the TV's Bluetooth daemon is **not** BlueZ. This is corroborated downstream: the chiaki-lg-fork project found the TV's Bluetooth daemon is `/usr/sbin/webos-bluetooth-service` using `/usr/lib/libbluetooth.default.so` (an LG-named library, not bluez's soname pattern), and that it implements HID transport itself (see §6).
- Contrast — webOS **OSE** (the open-source sibling LG publishes on GitHub) *does* use BlueZ: `meta-webosose` carries `meta-webos/recipes-connectivity/bluez5/bluez5_%.bbappend` with 29 LG patches plus `main.conf`, and the OSE bluetooth2 docs describe the stack abstraction as "(bluez5/bsa)" — i.e., OSE's SIL layer can sit on BlueZ **or** on Broadcom's proprietary "bsa" stack (https://www.webosose.org/docs/reference/ls2-api/com-webos-service-bluetooth2/, adapter/setState error 113). The exact bluez5 version in OSE was not pinned (it comes from the OE/poky layer, not meta-webosose) — unverified, and not relevant to the TV, which ships neither.

**Consequence for this ticket:** "the bluez version and config LG ships on webOS 4.10" is answerable only as *none*: there is no bluez version, no `/etc/bluetooth/main.conf`, no bluez input plugin. The relevant "config" is LG's proprietary daemon + configd keys (§6).

### 4.2 The kernel: Linux 4.4.84, uhid-based HID, no DualSense driver

From `IKLP-r3247/common/RTK_Kernel_linux.zip` → `RTK_Kernel_linux.tgz` → `linux_pop/` (Realtek rtd287o, per `README_Make_Kernel.txt`: `make realtek/config.develop.rtd287o.tv006.emmc.old_defconfig`):

- Kernel version: **4.4.84** (`linux_pop/Makefile`: VERSION=4, PATCHLEVEL=4, SUBLEVEL=84). (The chiaki-lg-fork project independently reports a G1 on webOS 6.5 running `4.4.84-229.kcl4tv.6` — LG reused 4..84 across generations.)
- Defconfig `arch/arm/configs/realtek/config.develop.rtd287o.tv006.emmc.old_defconfig` (401-line savedefconfig) contains **no `CONFIG_BT*` lines at all** — the kernel Bluetooth subsystem (and `hidp`) is not enabled in this config; Bluetooth runs in LG's userspace. Confirmed relevant lines that *are* set:
  - `CONFIG_UHID=y` — userspace HID: how LG's daemon creates input devices.
  - `CONFIG_HIDRAW=y`, `CONFIG_INPUT_EVDEV=y`, `CONFIG_INPUT_JOYDEV=y`, `CONFIG_INPUT_JOYSTICK=y`, `CONFIG_INPUT_UINPUT=y`.
  - `CONFIG_HID_SONY=y` **plus LG's custom `CONFIG_SONY_DUALSHOCK_4_USER_DRIVER=y`**.
  - `CONFIG_HID_LOGITECH=y`, `CONFIG_HID_PID=y`. (`HID_GENERIC` defaults to `y` with `HID`; its source is present in the tree.)
- The LG patch behind that custom config — `drivers/hid/hid-sony.c` lines 2457–2466:

  ```c
  #ifndef CONFIG_SONY_DUALSHOCK_4_USER_DRIVER
      /* Sony Dualshock 4 controllers for PS4 */
      { HID_USB_DEVICE(USB_VENDOR_ID_SONY, USB_DEVICE_ID_SONY_PS4_CONTROLLER), ... },
      { HID_BLUETOOTH_DEVICE(USB_VENDOR_ID_SONY, USB_DEVICE_ID_SONY_PS4_CONTROLLER), ... },
  #endif
  ```

  with `drivers/hid/Kconfig` `SONY_DUALSHOCK_4_USER_DRIVER`: "Say y here if you want to use user-layer driver for sony dualshock 4 instead of the driver in kernel-layer." **LG deliberately keeps the kernel's DS4 driver from binding so LG's user-level driver owns DS4** — direct evidence that gamepad HID on webOS is handled by device-specific userspace code, not by the kernel.
- `drivers/hid/hid-ids.h` in this tree knows Sony PIDs up to `USB_DEVICE_ID_SONY_PS4_CONTROLLER 0x05c4` — **no `0x0ce6` (DualSense), no `0x09cc` (DS4 v2)**. There is no `hid-playstation` file (it first appears in mainline v5.12; verified 404 on kernel.org for `h=v5.10`, `h=v5.11`, 200 for `h=v5.12`).
- Upstream context (kernel.org): stock hid-sony in v3.10 only handled PS3 controllers; hid-microsoft in v3.10 has no Xbox One S entry (Xbox One S BT support `HID_BLUETOOTH_DEVICE(..., USB_DEVICE_ID_MS_XBOX_ONE_S_CONTROLLER)` exists by v5.4). So on a 4.4 kernel, **both** Xbox One S and DualSense would bind `hid-generic` — kernel driver presence does not differentiate the two pads; whatever differs is in LG's userspace (§6).

Caveat: the defconfig above is for the Realtek rtd287o family (UM7100 etc.). webOS 4.x shipped on several SoC families (MStar "IMP" for SM9000, "GMPP/GLMA" for 2018 UK models — per the opensource.lge.com drop names), and the maintainer's exact model is unknown to this research. The shipping kernel's real config is readable on the TV (`CONFIG_IKCONFIG_PROC=y` is set in this defconfig → `/proc/config.gz`), which is checklist item A3.

### 4.3 webOS 4.10 ↔ IKLP mapping

opensource.lge.com lists four source drops for 55UM7100: "webOS 4.5 IKLP" (2019-03-04), "IKLP_2.0" (2019-03-19), "IKLP_3.0" (2019-07-26), "IKLP_4.0" (2022-01-14). The 2022 drop corresponds to the final firmware generation of these 2019 sets (the line that reaches "webOS 4.9.x/4.10" as reported by users — issue #7's reporter was on 4.9.7 on this exact model). Treating IKLP_4.0 as representative of webOS 4.10 is an inference from the drop dates; it is the best open-source evidence available and the checklist verifies the specifics on the actual TV.

---

## 5. The `bluetooth2` LS2 API connect flow and its failure modes

Primary source: webOS OSE `webosose/webos.service.bluetooth2` (and its newer sibling `webosose/com.webos.service.bluetooth2`, same code). **Caveat:** the TV ships LG's proprietary build of this service plus a proprietary SIL (stack interface layer); the OSE code defines the LS2 surface and the failure-mode vocabulary, but the TV's internals may differ. The SIL is explicitly a dlopen'ed plugin: `src/bluetoothsilfactory.cpp` loads `$WEBOS_BLUETOOTH_SIL_BASE_PATH/<WEBOS_BLUETOOTH_SIL>.so` — on OSE that's `bluetooth-sil-bluez5`; on the TV it is LG's own (name unknown, closed source).

### 5.1 Flow as the app uses it (verified in this repo's fork)

1. `device/getStatus` (subscribe) → list of devices (`app/src/lib/DeviceFinder.svelte` line 17–29).
2. `adapter/startDiscovery` / `adapter/cancelDiscovery`.
3. `adapter/pair` with `{"address":..., "subscribe":true}` — a subscription that emits `displayPinCode` / `displayPasskey` / `confirmPassKey` and finally `endPairing` with `returnValue` (`DeviceFinder.svelte` lines 54–97).
4. On `endPairing` with `returnValue: true` the app immediately calls `hid/connect` with `{"address":...}` (subscription form).

### 5.2 What the service does on `adapter/pair` (OSE source)

`src/bluetoothmanagerservice.cpp` `BluetoothManagerService::pair` (line 2221): requires default adapter (else error 101), validates schema (`address`,`subscribe` required; else 105/143/144), rejects if another pairing subscription is active (104) or pairing already in progress (118), requires the device to be known from discovery (106) and not already paired. It then cancels discovery and calls the SIL `pair`; on SIL completion it posts the `endPairing` message with `returnValue: true/false` to the subscriber (`cancelDiscoveryCallback`, lines ~3739–3776). Pairing itself is thus a stack-level operation; HID is not involved yet.

### 5.3 What the service does on `hid/connect` (OSE source)

`src/bluetoothhidprofileservice.cpp` registers the category `/hid` with methods `connect`, `disconnect`, `getStatus` (constructor, lines 25–36; profile UUID string in the constructor is `"00000011-0000-1000-8000-00805f9b34fb"` — note this is *not* the standard HID profile UUID 0x1124; in OSE it is dead code because the OSE SIL has no HID profile at all, see below). The shared `BluetoothProfileService::connect` (`src/bluetoothprofileservice.cpp` line 300) and `connectToStack` (line 210):

1. `mImpl = defaultAdapter->getProfile("HID")` at initialize — if the SIL provides no HID profile, **every** `/hid/*` call fails with error **127 "Profile backend is not available"** (`bluetootherrors.h` line 52). In OSE this is the *permanent* state: `bluetooth-sil-bluez5` implements a2dp/avrcp/ftp/gatt/hfp/map/opp/pbap/spp/mesh but **no HID** (verified: no HID source file and no `hid` string in `src/bluez5sil.*`, `bluez5device.cpp`, `bluez5profilebase.cpp`; `BluetoothHidProfile` exists only as an interface in `bluetooth-sil-api/include/public/bluetooth-sil-api/hid.h`). The OSE docs correspondingly do not document any `/hid/*` methods (fetched API page lists adapter/device/a2dp/avrcp/gatt/hfp/le/map/mesh/opp/pbap/spp only). **On the TV, LG's SIL does implement HID** — otherwise webos-gamepad could never connect an Xbox pad at all (it can, per apps-repo #20 and webos-gamepad #5).
2. Device availability check → error **106** if the address isn't a known device.
3. `isDevicePaired` → error **129 "Device is not paired"**.
4. `isDeviceConnecting` → error **128**; then a `getProperty(CONNECTED)` round-trip: if already connected → error **131 "Already connected"**.
5. SIL `connect(address, cb)` — the OSE bluez5 SIL would call `org.bluez.Device1.ConnectProfile(uuid)` (`bluetooth-sil-bluez5/src/bluez5device.cpp` line 624); on any SIL error the client gets error **130 "Failed to connect with remote device"**. **Error 130 is the generic "the stack refused or failed the HID connection"** — the single most diagnostic value for this ticket.
6. On success the first response carries `returnValue: true`; **if the caller subscribed, the service registers a ClientWatch and disconnects the device when the calling client disappears** (`handleConnectClientDisappeared`, line ~381) — a real failure mode for GUI apps that exit after connecting.

Error codes and texts: `webosose/webos.service.bluetooth2/src/bluetootherrors.h` (101, 106, 107, 127, 128, 129, 130, 131 …) and `bluetootherrors.cpp`; the OSE docs' per-method error tables match (e.g., 128–131 for profile connect).

### 5.4 Failure-mode map for "pairs but never connects" on the TV

| Observation at `hid/connect` | Meaning per OSE semantics |
|---|---|
| error 127 | TV's SIL has no HID profile (unlikely — Xbox works) |
| error 106 | Device dropped out of the service's device list after pairing |
| error 129 | Pairing didn't actually store (link-key/pairing state machine issue) |
| error 131 | Stack thinks it's already connected (matches webos-gamepad #5's "shows connected even when off" symptom) |
| error 130, fast | Stack refused the HID connect — policy/whitelist or immediate protocol refusal (H1/H3) |
| error 130, after ~10–30 s | Stack attempted and timed out — protocol-level failure with the pad (H3) |
| `returnValue: true` but no input | HID session up, `uhid`/input-node problem downstream (H2) |

---

## 6. LG's HID manager is device-aware: the gamepad whitelist

Hardware-verified on webOS 6.5 (LG OLED G1, webOS 6.5.3, kernel 4.4.84-229.kcl4tv.6) by the chiaki-lg-fork project (issues #2, #4, merged with hardware validation):

- LG's `bthidmanager` classifies a connected DualSense as `type=not_support_gamepad, vid=0x54c, pid=0xce6` (issue #4, "Native G1 Bluetooth finding").
- The classification comes from a configd key: `com.webos.service.bthidmanager.supportedBtGamepadList`, base layer at `/etc/configd/layers/base/com.webos.service.bthidmanager.json`; the daemon is `/usr/sbin/webos-bluetooth-service`, library `/usr/lib/libbluetooth.default.so` (all named in `packaging/root/bluetooth-runtime.sh`, which manipulates exactly these).
- The base whitelist (from that script, vid/pid decoded against kernel `drivers/hid/hid-ids.h`): Xbox Wireless Controller **045E:02FD** (model 1708 = Xbox One S), Xbox **045E:0B13** (model 1914 = Series X|S), "Wireless Controller" **054C:05C4** (DualShock 4 v1), **054C:09CC** (DualShock 4 v2), NVIDIA Controller **0955:7214**, Luna Gamepad **0171:0419**. **No DualSense (054C:0CE6).**
- Adding `{"name":"DualSense Wireless Controller","vid":1356,"pid":3302}` to that list via `luna://com.webos.service.config/setConfigs` (`volatile:true`) is how chiaki-lg-fork makes the DualSense a "supported" pad on that firmware.
- On webOS 6.5 the unsupported classification did **not** block basic connection: the pad connected, bound `hid-generic`, and buttons/sticks/triggers worked; what broke was *output* — LG's daemon labels HID interrupt-channel output as FEATURE traffic (0xA3) instead of OUTPUT (0xA2), so rumble/LED reports made the pad physically disconnect after ~30 s (issue #4; the one-byte in-memory patch is `src/bt_patch.c`, signature `bb680322...` → `bb680222...`).
- `com.webos.service.bthidmanager` also exists as its own LS2 service (16 private methods incl. `getSupportedGamePadList`, `requestConnect`, `requestDiscovery`), documented from on-device introspection of webOS TV 7.0 in `ohmybahgosh/WEBOS-LUNA-SEND-ATLAS/docs/services/com.webos.service.bthidmanager.md`; `chros73/bscpylgtv` settings dumps show `com.webos.service.bthidmanager.*` keys present on CX (webOS 5) through G6 (webOS 13). Whether this service/key exists on webOS **4.10** is unverified — checklist item A5/E1.

Interpretation for webOS 4.10: the same architecture (proprietary daemon, device whitelist, uhid) very likely applies, but the webOS 4.x generation may be *stricter* than 6.5 (where unsupported pads still connect). If on 4.x the HID connect path gates on the whitelist — or if LG's 4.x HID backend simply has no code path for an unknown Sony pad — that precisely produces "pairs but never connects" while Xbox One S (whitelisted) works.

---

## 7. How the DualSense differs from controllers that do work

| Aspect | Xbox One S (works on webOS 4.x) | DualShock 4 (works on webOS 3.x/5 per community) | DualSense (never connects on 4.x) | Source |
|---|---|---|---|---|
| VID:PID (BT) | 045E:02FD (also 045E:0B13 newer fw) | 054C:05C4 / 054C:09CC | **054C:0CE6** | kernel `hid-ids.h` lines 1065/1365-1369 |
| In LG `supportedBtGamepadList` | yes (both) | yes (both) | **no** | chiaki-lg-fork `bluetooth-runtime.sh` base arrays |
| Kernel driver on 4.4 | none specific → `hid-generic` | hid-sony entry **compiled out** by LG (`SONY_DUALSHOCK_4_USER_DRIVER`) → LG user-level driver | none — `hid-playstation` needs ≥ v5.12; no `0x0ce6` in LG's `hid-ids.h` → `hid-generic` if connected | LG kernel tree (§4.2); kernel.org tag checks |
| BT HID protocol | standard HID reports | standard HID reports; poll interval set via `hw_control` | input **report 0x31, 78 bytes, CRC32 (seed 0xA1)**; output report 0x31 with tag 0x10 and CRC32; USB uses report 0x01/64 bytes | `drivers/hid/hid-playstation.c` lines 76–88, 302–309 |
| DID/SDP | standard | standard | standard; LG classifies by DID vid/pid (whitelist) | chiaki issue #4 |
| Community evidence on webOS 4.x | connects via webos-gamepad on this era (webos-gamepad #5: an Xbox One S connected fine until a Microsoft controller-firmware update broke it — TV model unstated; apps-repo #20: bluetooth2 HID pair/connect works "down to webOS 4"; the maintainer's map issue names Xbox One S as a currently-working reference pad on the 4.10 TV alongside the broken DualSense) | none found specifically on 4.x (works 3.x and 5.x) | pairs, **never connects** (webos-gamepad #7, webOS 4.9.7) | Superd22/webos-gamepad#5; webosbrew/apps-repo#20; acheronian/webos-bluepad#1 |
| On webOS ≥ 5 | works incl. rumble (moonlight-tv #362: Xbox fine on C3) | works | connects as generic input; rumble only after LG update / on newer webOS; enhanced reports can **kill the link** on old stacks (chiaki #4) | moonlight-tv #362; chiaki #2/#4; punktfunk #96 |

Kernel-level conclusion (all primary-sourced): on a 4.4-era kernel both pads are `hid-generic` clients, so the kernel cannot be where Xbox succeeds and DualSense fails. The differentiators are (a) LG's **userspace whitelist/classification** and (b) the DualSense's **non-standard BT report framing (0x31/78B/CRC32)**, which a hand-rolled userspace HIDP implementation could plausibly mishandle (LG's already mislabels output vs feature on 6.5 — chiaki).

---

## 8. Ranked root-cause hypotheses

**H1 — LG's HID backend refuses unknown gamepads at connect time (whitelist/policy gate).**
The 4.x daemon's HID connect path fails fast for pads absent from its supported-gamepad list (DualSense 054C:0CE6), returning error 130 without ever opening HID channels or creating a `uhid` device. Supported by: whitelist evidence (§6), LG's device-specific userspace HID design (DS4 user-driver config, §4.2), the old webOS 3.x `hid_sony`-only service (§3.2), and the fact that exactly the whitelisted pads work on 4.x. Against: on 6.5 unsupported pads still connect — but that is a newer, softer generation of the same policy.
*Predicts:* `hid/connect` → 130 quickly; no `uhid`/input/dmesg activity; adding the pad to `supportedBtGamepadList` (if the key exists on 4.x) changes the outcome.

**H2 — Connect succeeds, but no input node is created / events don't flow.**
The HID session comes up (`returnValue: true`) but LG's backend never creates the `uhid` device for an unsupported pad (or the known stale-`/dev/input`-handler bug bites: webos-gamepad #2). This matches the literal reading of issue #7 ("shown as Wireless Controller … but inputs are not working") if the app displayed success.
*Predicts:* `hid/connect` returns true; `/proc/bus/input/devices` may or may not gain an entry; `/dev/input/eventN` missing or silent; dmesg may show a `uhid` create that fails or never appears.

**H3 — Protocol-level failure against DualSense-specific framing.**
LG's userspace HIDP implementation fails the L2CAP/HID handshake with the DualSense's 78-byte CRC32-framed 0x31 reports (or its SDP/DID parsing), so the connect attempt times out → 130 after a delay. Supported by: the DualSense's non-standard BT framing (§7) and LG's demonstrated HIDP implementation bugs (0xA2/0xA3, §6).
*Predicts:* 130 after a visible timeout; controller lightbar shows connection attempts; identical steps with a DS4 succeed.

**H4 — Sequencing/race between pairing and connect.**
`hid/connect` fired immediately after `endPairing` hits a stack not yet ready for the new link key (community pattern: first-pairing events not arriving until a disconnect/reconnect or power cycle; webos-gamepad #2/#9 flakiness).
*Predicts:* a delayed second `hid/connect`, or a controller power cycle + retry, succeeds; error is 130 fast on the first attempt but the pad's LEDs show repeated attempts.

**H5 — Pairing-state defect (link key type / security mode).**
Pairing "succeeds" at the UI level but stores a link key the DualSense rejects for HID-channel authentication, so every connect is refused by the pad. Less likely (the pad reports as paired and the same flow works for other SSP gamepads), but cheap to test.
*Predicts:* 130 fast on every attempt; deleting the pairing and re-pairing changes nothing; the pad's lightbar flashes on attempts (pad side sees and refuses); a2dp-style profiles also fail.

The checklist below orders the TV session so that each phase's observations either confirm or eliminate these in sequence (error-code capture → input-layer inspection → whitelist experiment → differential control → sequencing tests).

---

## 9. Explicitly unverified / open questions

- Whether `com.webos.service.bthidmanager` (service or config key `supportedBtGamepadList`) exists on **webOS 4.10** — all direct evidence is webOS 5–13 (bscpylgtv dumps, ATLAS introspection) and 6.5 (chiaki, hardware-verified). Checklist A5/E1 answers this on the TV.
- The exact WebOS 4.10 binary inventory (`webos-bluetooth-service` vs `btmngr` naming, SIL .so name) — chiaki's evidence is webOS 6.5; webOS 3.x used `btmngr` (webosbrew doc). Checklist A4 answers this.
- Which SoC/kernel the maintainer's TV uses (IKLP evidence is Realtek rtd287o / 4.4.84; other webOS 4.x platforms are MStar etc.). Checklist A2/A3 answers this.
- The exact error code the maintainer's app surfaces on `hid/connect` today (the app UI shows an error string built from `errorCode/errorText` — `DeviceFinder.svelte` lines 69–81 — but the value was not recorded in any ticket).
- Whether the OSE `hid/connect` semantics (error codes 127/106/129/128/131/130, auto-disconnect on subscriber drop) match the TV's proprietary build exactly — OSE is the closest published relative, not the TV's source.
- Any webosbrew Discord reports — not publicly archivable.
- The OSE bluez5 version number (poky-layer-derived; not fetched — and moot for the TV, which ships no bluez).

---

## 10. Appendix: source index

| Claim | Source |
|---|---|
| webos-gamepad #7 symptom, closed undiagnosed | https://github.com/Superd22/webos-gamepad/issues/7 (+ `gh api .../issues/7`, `.../issues/7/events`) |
| App pair→connect flow | this repo `app/src/lib/DeviceFinder.svelte` (L17–29, 54–97); `services/web-input/src/hid-bluetooth.service.ts` |
| Stale /dev/input failure mode | Superd22/webos-gamepad issue #2; `services/web-input/src/commands/init-hids.command.ts` |
| webosbrew has no forum; apps-repo #20 reports | https://www.webosbrew.org/ link list; https://github.com/webosbrew/apps-repo/issues/20 (all comments) |
| webOS 3.x hid_sony/btmngr | https://www.webosbrew.org/pages/luna-service-comwebosservicebluetooth.html |
| bluetooth2 LS2 flow + error codes | webosose/webos.service.bluetooth2 `src/bluetoothhidprofileservice.cpp`, `src/bluetoothprofileservice.cpp` (L210, L300, L381), `src/bluetoothmanagerservice.cpp` (L2221, ~L3739), `src/bluetootherrors.h` (L52–56), `src/bluetoothsilfactory.cpp` |
| OSE SIL has no HID; OSE docs omit /hid | webosose/bluetooth-sil-bluez5 tree (no hid impl); webosose/bluetooth-sil-api `include/public/bluetooth-sil-api/hid.h`; https://www.webosose.org/docs/reference/ls2-api/com-webos-service-bluetooth2/ |
| OSE uses bluez5 (TV does not) | webosose/meta-webosose `meta-webos/recipes-connectivity/bluez5/bluez5_%.bbappend`; LG OSS notices + archives (below) |
| LG drops, no bluez, kernel 4.4.84, defconfig, hid-sony patch | opensource.lge.com search `keyword=55UM7100` / `keyword=UK6400`; downloads osSeq 510775 (`webOS 4.5 IKLP_4.0_1/2.tar.gz`), 508989 (IKLP 2019 notice), 508446 (GMPP notice); `IKLP-r3247/common/RTK_Kernel_linux.zip` → `linux_pop/Makefile`, `arch/arm/configs/realtek/config.develop.rtd287o.tv006.emmc.old_defconfig`, `drivers/hid/hid-sony.c` L2457–2466, `drivers/hid/Kconfig` `SONY_DUALSHOCK_4_USER_DRIVER`, `drivers/hid/hid-ids.h` |
| hid-playstation ≥ v5.12; no DualSense in 4.4 | kernel.org `drivers/hid/hid-playstation.c` (404 at h=v5.10, v5.11; 200 at h=v5.12); LG tree `hid-ids.h`; hid-sony v3.10 id table; hid-microsoft v3.10/v5.4 id tables |
| DualSense BT framing (0x31/78B/CRC32) | kernel.org `drivers/hid/hid-playstation.c` L76–88, L302–309 |
| VID/PID meanings | kernel.org `drivers/hid/hid-ids.h` L1065–1067, L1365–1369 |
| bthidmanager whitelist, not_support_gamepad, 0xA2/0xA3 bug, webOS 6.5 hardware tests | Hubert-Rybak/chiaki-lg-fork issues #2, #4; `packaging/root/bluetooth-runtime.sh`; `src/bt_patch.c` |
| bthidmanager LS2 service (webOS 7 introspection) | ohmybahgosh/WEBOS-LUNA-SEND-ATLAS `docs/services/com.webos.service.bthidmanager.md` |
| bthidmanager config keys on webOS 5–13 | chros73/bscpylgtv `docs/available_settings_{CX,C1,G4}.md` |
| DualSense works (input) on webOS ≥ 5 without hid-playstation; hid-playstation only webOS 10+ | punktfunk/client-webos issue #96 + README; mariotaku/moonlight-tv issue #362 |

---

## 11. TV-side diagnostic checklist

For a rooted webOS 4.10 TV over SSH, with a DualSense (and ideally one known-good pad: Xbox One S or DualShock 4) at hand. Run phases in order; record every output verbatim. `luna-send` is `/usr/bin/luna-send`; if permission errors occur, try `luna-send-pub` (per webosbrew/apps-repo#20 guidance from Informatic).

### Phase A — inventory (before touching the pad)

```sh
# A1. Firmware identity (command from webosbrew/apps-repo#20, Informatic)
grep -h -E '"(hardware_id|core_os_release|product_id|webos_manufacturing_version|board_type)"' /var/run/nyx/*

# A2. Kernel
uname -a

# A3. Shipping kernel config (CONFIG_IKCONFIG_PROC is set in LG's defconfig)
zcat /proc/config.gz > /tmp/k.cfg
grep -E "CONFIG_BT|CONFIG_BT_HIDP|CONFIG_UHID|CONFIG_HID=|CONFIG_HID_GENERIC|CONFIG_HID_SONY|CONFIG_SONY_DUALSHOCK_4_USER_DRIVER|CONFIG_HID_MICROSOFT|CONFIG_HID_PLAYSTATION|CONFIG_INPUT_EVDEV|CONFIG_INPUT_JOYDEV" /tmp/k.cfg

# A4. Bluetooth daemon/binary inventory
ps aux | grep -iE "bluetooth|btmgr|bthid" | grep -v grep
pidof webos-bluetooth-service
ls -la /usr/sbin/webos-bluetooth-service /usr/lib/libbluetooth* 2>/dev/null
ls /etc/bluetooth 2>/dev/null || echo "no /etc/bluetooth (expected: no bluez)"
which bluetoothd btmon hcitool hcidump 2>/dev/null || echo "no bluez tools"

# A5. bthidmanager presence + gamepad whitelist
ls /etc/configd/layers/base/ | grep -i bthid
luna-send -n 1 -f luna://com.webos.service.config/getConfigs '{"configNames":["com.webos.service.bthidmanager.supportedBtGamepadList"]}'
luna-send -n 1 -f luna://com.webos.service.bthidmanager/getSupportedGamePadList '{}' 2>&1 | head -c 2000

# A6. Baseline input state
cat /proc/bus/input/devices > /tmp/input.before
ls -la /dev/input/ > /tmp/devinput.before

# A7. Start capturing the kernel log
dmesg -c > /tmp/dmesg.baseline
```

**Expected under §4's findings:** A3 shows `CONFIG_BT` absent (or not set), `CONFIG_UHID=y`, `CONFIG_HID_SONY=y` (+ `CONFIG_SONY_DUALSHOCK_4_USER_DRIVER=y` on the LG 4.4 kernel), no `CONFIG_HID_PLAYSTATION`. A4 shows no bluez artifacts. A5 is the key unknown: if the bthidmanager config key exists, H1 becomes testable directly (Phase E).

### Phase B — pair the DualSense

```sh
# B1. Start discovery
luna-send -n 1 -f luna://com.webos.service.bluetooth2/adapter/startDiscovery '{}'
# B2. On the pad: hold PS + Create until the lightbar flashes rapidly (pairing mode)
# B3. Find the address
luna-send -n 1 -f luna://com.webos.service.bluetooth2/device/getStatus '{}' | grep -i -B2 -A8 "Wireless Controller"
# B4. Pair (subscription; keep it running until endPairing)
luna-send -i -f luna://com.webos.service.bluetooth2/adapter/pair '{"address":"AA:BB:CC:DD:EE:FF","subscribe":true}'
# B5. Device state after pairing
luna-send -n 1 -f luna://com.webos.service.bluetooth2/device/getStatus '{}' | grep -i -B2 -A8 "AA:BB:CC:DD:EE:FF"
```

Record: every `adapter/pair` subscription message (`displayPinCode`/`displayPasskey`/`confirmPassKey`/`endPairing` and `returnValue`), and the device entry in B5 (`paired`, `profiles`, `connectedProfiles` fields if present).

### Phase C — the connect attempt (key discriminator)

```sh
# C1. One-shot connect with timing; a hang is itself data (H3)
date; timeout 40 luna-send -n 1 -f luna://com.webos.service.bluetooth2/hid/connect '{"address":"AA:BB:CC:DD:EE:FF"}'; date
# If the method insists on a subscription:
luna-send -i -f luna://com.webos.service.bluetooth2/hid/connect '{"address":"AA:BB:CC:DD:EE:FF","subscribe":true}'
# C2. Controller LEDs during C1: flashing = connection attempts; steady = connected
# C3. Immediately after:
dmesg > /tmp/dmesg.afterC1; diff /tmp/dmesg.baseline /tmp/dmesg.afterC1 | head -50
cat /proc/bus/input/devices | diff /tmp/input.before - | head -40
ls -la /dev/input/ | diff /tmp/devinput.before - 
```

Interpret against §5.4's table. Fast 130 → H1/H3; slow 130 → H3; true → Phase D (H2); 131 → the stack thinks it's connected (matches webos-gamepad #5's phantom-connected symptom); 129 → pairing didn't stick (H5).

### Phase D — if connect returned success (H2 path)

```sh
# D1. Locate the pad and its handlers
grep -A6 -i "Wireless Controller" /proc/bus/input/devices
# D2. Node present? permissions? (webos-gamepad's ensure step chowns root:compositor 0660)
ls -la /dev/input/event*
# D3. Which driver owns it (expect hid-generic)
ls -l /sys/class/input/event*/device/driver 2>/dev/null
# D4. Raw event read while pressing buttons/sticks (Ctrl-C after 10 s)
timeout 10 cat /dev/input/eventNN | od -An -tx1 | head -20
# D5. Stale-handler cleanup per webos-gamepad init-hids, then re-check
#     (compare /proc/bus/input/devices handlers vs /dev/input contents; remove stale nodes)
```

Events flow but apps don't see them → app-side (permission/permission-group) problem, not Bluetooth. No node at all despite `returnValue: true` → LG backend skipped `uhid` creation for an unsupported pad (H1/H2 hybrid).

### Phase E — whitelist experiment (H1)

```sh
# E1. Read the current list (raw)
luna-send -n 1 -f luna://com.webos.service.config/getConfigs '{"configNames":["com.webos.service.bthidmanager.supportedBtGamepadList"]}'
```

If (and only if) the key exists: append the DualSense entry (vid 1356, pid 3302) to the *current* array value and set it volatile — mirroring chiaki-lg-fork's verified G1 procedure (`packaging/root/bluetooth-runtime.sh`, `set_runtime_config`):

```sh
# E2. Volatile set (value = current array + {"name":"DualSense Wireless Controller","vid":1356,"pid":3302})
luna-send -n 1 -f luna://com.webos.service.config/setConfigs '{"configs":{"com.webos.service.bthidmanager.supportedBtGamepadList":[ ...current entries..., {"name":"DualSense Wireless Controller","vid":1356,"pid":3302}]},"volatile":true}'
# E3. Retry Phase C1 exactly; record errorCode and dmesg delta
# E4. Cleanup (reboot also clears volatile configd state)
```

If the key does not exist on 4.x, H1's *mechanism* on this firmware is something else — capture that fact and fall through to Phase F/G.

### Phase F — differential control (known-good pad)

Repeat Phases B and C with the Xbox One S / DS4. Record the error codes, dmesg signature, and `/dev/input` node creation of the *healthy* path on this exact TV. Any step where Xbox succeeds and DualSense fails is the precise location of the defect; if Xbox also fails on this TV, the problem is TV-wide (bluetooth2 HID backend/firmware state), not DualSense-specific.

### Phase G — sequencing and controller-side tests (H4/H5)

```sh
# G1. Power-cycle the pad fully (hold PS ~10 s), press PS once, wait 10 s, then:
luna-send -n 1 -f luna://com.webos.service.bluetooth2/device/getStatus '{}' | grep -i -A8 "AA:BB:CC:DD:EE:FF"   # any incoming connection?
# G2. Retry connect after the wait
timeout 40 luna-send -n 1 -f luna://com.webos.service.bluetooth2/hid/connect '{"address":"AA:BB:CC:DD:EE:FF"}'
# G3. Unpair, re-pair, wait 5 s before connecting (vs immediately)
luna-send -n 1 -f luna://com.webos.service.bluetooth2/adapter/unpair '{"address":"AA:BB:CC:DD:EE:FF"}'
# G4. Double connect (128 "already connecting" then?)
timeout 40 luna-send -n 1 -f luna://com.webos.service.bluetooth2/hid/connect '{"address":"AA:BB:CC:DD:EE:FF"}' ; timeout 40 luna-send -n 1 -f luna://com.webos.service.bluetooth2/hid/connect '{"address":"AA:BB:CC:DD:EE:FF"}'
```

G1–G2 reproduces the community "power-cycle revives it" pattern (apps-repo #20: DS4 events flow only after a controller power cycle on first pairing). G3 tests H4. G4 distinguishes 128/130 sequencing.

### Phase H — daemon-side view (optional, root)

```sh
# H1. Does the daemon hold /dev/uhid open, and does anything change during connect?
ls -l /proc/$(pidof webos-bluetooth-service)/fd 2>/dev/null | grep -iE "uhid|input|hci"
# H2. Optional syscall trace during a connect attempt
strace -f -p $(pidof webos-bluetooth-service) -e trace=open,openat,ioctl,read,write -s 256 -o /tmp/bt.strace &
#   ... run Phase C1 ... then: kill %1; grep -iE "uhid|hid|error|ENODEV|EPERM" /tmp/bt.strace | tail -40
# H3. PMLog tail
grep -iE "bluetooth|bthid|hid" /var/log/messages 2>/dev/null | tail -50
```

`/dev/uhid` never opened during a "successful" connect → LG backend skipped device creation (H2). Opened then closed with errors → protocol failure (H3).

### Expected outcome matrix

| Hypothesis | C1 | C3 dmesg | D4 events | E3 after whitelist | F Xbox |
|---|---|---|---|---|---|
| H1 whitelist/policy | 130 fast | silent | n/a | **changes** (130→true or different failure) | works |
| H2 no input node | true | maybe uhid create | none | same | works |
| H3 protocol | 130 slow | maybe hidp/L2CAP errors | n/a | same | works |
| H4 race | 130 fast first, retry OK | activity on retry | yes after retry | same | works |
| H5 pairing/key | 130 fast always | silent | n/a | same | works |

Whichever row matches, capture the exact `errorCode`/`errorText`, dmesg lines, and `/proc/bus/input/devices` diffs — those artifacts are the input to the follow-up connection-layer strategy ticket (map issue #1's "Not yet specified" section).

---

