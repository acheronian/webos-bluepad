# Research: Bluetooth adapter selection on webOS 4.10 (onboard vs USB dongle)

Ticket: acheronian/webos-bluepad#4 — "Research: Bluetooth adapter selection on webOS 4.10 (onboard vs USB dongle)"
Branch: `research/bt-adapter-selection`

## Question

Can a webOS 4.10 app select which Bluetooth adapter (onboard radio vs USB dongle) is used to
pair and connect HID devices? Cover: whether webOS's bluez enumerates USB Bluetooth dongles at
all on 4.10; what the `bluetooth2` LS2 API exposes for listing/selecting adapters; dongle
chipsets known to work with webOS's stack; and how the DS5Dongle approach differs as an
alternative input path.

## TL;DR — feasibility answer

- **Adapter selection is NOT exposed as an app-facing API.** The public `com.webos.service.bluetooth2`
  LS2 surface has methods for *listing* adapters (`adapter/queryAvailable`, `adapter/getStatus`)
  but **no method to select or set the default adapter**. Verified in both the TV-era open-source
  service (`webosose/webos.service.bluetooth2`, the 2014–2019 lineage webOS 4.x derives from) and
  the current OSE service (`webosose/com.webos.service.bluetooth2`). See §2.
- Many adapter/`hid` methods accept an optional `adapterAddress` request parameter, but in the
  TV-era service it is **validated and then ignored** — every operation runs on the single
  `mDefaultAdapter`, and passing any address other than the default's returns
  `BT_ERR_INVALID_ADAPTER_ADDRESS` (source has a literal `//TODO: When we have multiple adapters…`
  comment). Only the newer OSE service (2019–2021) actually routes per `adapterAddress`. Which
  build webOS 4.10 ships is **unverifiable from public sources** — must be tested on-device. See §2.
- At the bluez level, the webOS SIL enumerates **every** `org.bluez.Adapter1` object, but the
  "default adapter" is **hardcoded to whichever adapter has `hci0` in its object path**. So the
  only lever that exists is: *make the adapter you want be `hci0`*. See §3.
- LG's own TV-era package ships **udev rules + scripts whose entire purpose is USB Bluetooth
  dongles**: on dongle insert they tear down the onboard UART radio (`stop hciuart`,
  `rmmod hci_uart`) and restart the BT service; on removal they restore it. That is LG's
  "adapter selection" mechanism — dongle *replaces* onboard, chosen at udev level, not by apps.
  Whether these exact rules ship on the 4.10 TV firmware is **unverified**. See §4.
- **Least-hacky workaround:** plug the dongle in while the TV is off (so it wins `hci0` at boot),
  or with root replicate LG's own script (`stop hciuart; rmmod hci_uart;` replug dongle;
  `systemctl restart webos-bluetooth-service`). The app should always echo the
  `adapterAddress` it read from `queryAvailable`/`getStatus` into its calls — safe on both
  eras and forward-compatible. See §5.
- **Known-working dongle chipsets on webOS: none verifiable.** The webosbrew forum
  (`forum.webosbrew.org`) is dead — the DNS record no longer exists (NXDOMAIN) and there are no
  Wayback Machine captures; no GitHub-side report of a USB BT dongle on a webOS TV was found.
  Recommendation is therefore grounded in the mainline Linux `btusb` driver instead: a genuine
  CSR8510 A10-class (CSR BlueCore, USB `0a12:0001`) dual-mode dongle as primary pick — it is the
  one common chipset handled directly by `btusb` with no firmware-file load. See §6.
- **DS5Dongle is not a Bluetooth dongle at all.** It is a Raspberry Pi **Pico 2 W** firmware
  (not a Pi Zero — the ticket's description is inaccurate) that *itself* pairs the DualSense
  over Bluetooth (BTstack, HID host over L2CAP) and re-exposes the controller to the host as a
  **wired USB DualSense** (USB VID `0x054C`, PID `0x0CE6`, HID gamepad/touchpad + USB audio
  interfaces). The host's Bluetooth stack is bypassed entirely. See §8.

---

## 1. Sources actually consulted (and how)

Fetched and read in full or in relevant part:

| Source | What was fetched |
|---|---|
| `github.com/webosose/webos.service.bluetooth2` (TV-era bluetooth2 service, copyright 2014–2019 LG) | `src/bluetoothmanagerservice.{h,cpp}`, `src/bluetoothprofileservice.cpp`, `src/bluetoothsilfactory.cpp`, `src/bluetoothhidprofileservice.cpp`, `files/rules/99-bluetooth.rules`, `files/scripts/bluetooth_usb_inserted.sh`, `files/scripts/bluetooth_usb_removed.sh`, `files/sysbus/com.webos.service.bluetooth2.api.json`, `CMakeLists.txt` |
| `github.com/webosose/com.webos.service.bluetooth2` (current OSE service, copyright 2019–2021 LG) | `src/bluetoothmanagerservice.{h,cpp}`, `src/bluetoothmanageradapter.h`, `files/sysbus/com.webos.service.bluetooth2.api.json` |
| `github.com/webosose/bluetooth-sil-api` | `include/public/bluetooth-sil-api.h` |
| `github.com/webosose/bluetooth-sil-bluez5` (the bluez5 SIL plugin the service dlopen()s) | `src/bluez5sil.cpp`, `src/bluez5adapter.cpp` |
| `github.com/torvalds/linux` | `drivers/bluetooth/btusb_main.c` (USB ID table, fake-CSR detection, firmware paths) |
| `github.com/OpenLGTV/lgmod_bcm` | `addons/sources/kernel/.config` (LG TV kernel config, NetCast-era — indirect evidence only) |
| `github.com/webosbrew/pages` (the webosbrew community wiki, GitHub Pages content) | `content/pages/luna/bluetooth.md`, `content/pages/hacking/glossary.md`, `content/pages/luna.md` |
| `github.com/webosbrew/apps-repo` | issue #20 "[IDEA] Bluetooth HID manager", PR #284 "Add BT Unlock" |
| `github.com/lucaasdias/bt-unlock` (BT Unlock — real webOS-4.x Bluetooth manager app) | `README.md`, `js/luna.js`, `scripts/bt-common.sh`, `scripts/bt-power.sh`, `scripts/bt-paired.sh`, `scripts/bt-discover.sh`, `scripts/bt-connect.sh`, `scripts/bt-pair.sh` |
| `github.com/Superd22/webos-gamepad` (upstream of this repo) | issues #6, #7 (webOS 4.9.7 reports) |
| `github.com/awalol/DS5Dongle` | `README.md`, `src/bt.cpp`, `src/usb_descriptors.cpp` |
| This repo | `app/src/App.svelte`, `app/src/lib/DeviceFinder.svelte`, `app/src/lib/DeviceCard.svelte`, `app/src/lib/interfaces/bluetooth-device.interface.ts`, `services/web-input/src/*` |
| `web.archive.org` | CDX/availability queries for `webostv.developer.lge.com` API pages and `forum.webosbrew.org`; one archived LG developer-forum thread |
| `opensource.lge.com` | product/list endpoints (reachable; per-model source disclosures exist but are multi-GB tarballs — not downloaded) |

Note: the ticket said "webOS/OSX source tree"; the webOS open-source org on GitHub is
`webosose` (Open webOS / webOS OSE lineage), and that is what was used. webOS **TV** firmware
itself is closed-source; LG publishes per-model source disclosures on `opensource.lge.com`
(verified reachable), but the 4.10-era bluetooth2/bluez tarballs were not downloaded, so all
TV-firmware-specific claims below are flagged as unverified.

Explicitly unavailable:
- **webosbrew forum** (`forum.webosbrew.org`): DNS returns NXDOMAIN
  (`curl "https://dns.google/resolve?name=forum.webosbrew.org&type=A"` → `"Status":3`), and the
  Wayback Machine has **no captures** of the domain (CDX query returned empty). The community has
  moved to Discord (linked from `www.webosbrew.org`), which is not fetchable. So forum reports of
  dongle compatibility could **not** be consulted.
- **LG TV developer API reference** for `com.webos.service.bluetooth2`: the old
  `webostv.developer.lge.com/api/...` pages are gone from the live site (404) and were never
  captured by the Wayback Machine (CDX query for the path prefix returned no matches). LG does
  not officially document bluetooth2 for TV apps (an archived LG developer-forum thread,
  "Compatibility of com.webos.service.bluetooth2 on webOS 24 LG TVs", shows LG staff declining
  to guarantee anything beyond the official TV docs).

## 2. What the bluetooth2 LS2 API exposes for listing/selecting adapters

### 2.1 The public method list has no "select adapter" method

The public LS2 API definition shipped by the TV-era service
(`webosose/webos.service.bluetooth2` → `files/sysbus/com.webos.service.bluetooth2.api.json`)
lists exactly these public `adapter` methods:

```
adapter/awaitPairingRequests, adapter/cancelDiscovery, adapter/cancelPairing,
adapter/getStatus, adapter/pair, adapter/queryAvailable, adapter/setState,
adapter/startDiscovery, adapter/supplyPasskey, adapter/supplyPasskeyConfirmation,
adapter/supplyPinCode, adapter/unpair
```

The current OSE repo's `files/sysbus/com.webos.service.bluetooth2.api.json` lists the **same**
twelve public adapter methods (plus `internal/*` ones). **There is no
`adapter/select`, `adapter/setDefault`, or equivalent anywhere in either API definition.**
Listing is exposed; selecting is not.

### 2.2 `adapter/queryAvailable` and `adapter/getStatus` — the listing surface

- `adapter/queryAvailable` (subscribable) returns an `adapters` array where each entry has
  `adapterAddress`, `default` (bool), `classOfDevice`, `stackName`, `stackVersion`,
  `firmwareVersion`, and `supportedServiceClasses`
  (`webosose/webos.service.bluetooth2` → `src/bluetoothmanagerservice.cpp`,
  `BluetoothManagerService::queryAvailable` ~line 1545 and `appendAvailableStatus` ~line 1226).
- **In the TV-era service, `queryAvailable` can only ever list one adapter**: the only member
  is `mDefaultAdapter` (`src/bluetoothmanagerservice.h` line 238:
  `BluetoothAdapter *mDefaultAdapter;` — there is no adapter map), and `appendAvailableStatus`
  only appends an entry `if (mDefaultAdapter)`.
- `adapter/getStatus` similarly returns an `adapters` array, again with a single entry in the
  TV-era build (`appendCurrentStatus`, ~line 1202).
- The current OSE service instead iterates `mAdaptersInfo` (a map of *all* adapters) and marks
  the default with `"default": true` (`webosose/com.webos.service.bluetooth2` →
  `src/bluetoothmanagerservice.cpp`, `appendCurrentStatus`/`appendAvailableStatus` ~lines
  830–905) — i.e. multi-adapter *visibility* exists there, but still no way to *choose* one.

Practical consequence: `adapter/queryAvailable` is the app's way to learn which adapter is in
use (its address, stack name/version — handy on-device diagnostics), not to pick one.

### 2.3 The `adapterAddress` request parameter — present, but decorative on the TV-era build

Nearly every adapter/device/profile method in the TV-era service accepts an optional
`adapterAddress` in the request payload, e.g.:

- `adapter/pair` schema: `STRICT_SCHEMA(PROPS_3(PROP(address, string),
  PROP_WITH_VAL_1(subscribe, boolean, true), PROP(adapterAddress, string)) …)`
  (`src/bluetoothmanagerservice.cpp` ~line 2232).
- `adapter/startDiscovery` schema: `SCHEMA_1(PROP(adapterAddress, string))` (~line 1711).
- `hid/connect` (via `BluetoothProfileService::connect`): schema includes
  `PROP(adapterAddress, string)` (`src/bluetoothprofileservice.cpp` ~line 192).

But the semantics in the TV-era build are:

1. `isRequestedAdapterAvailable()` (~line 221) defaults `adapterAddress` to the default
   adapter's address when omitted, then calls `isAdapterAvailable()` (~line 213), which
   contains the literal comment:

   ```cpp
   //TODO: When we have multiple adapters, look for address==one of the available adapterAddresses
   //      currently only one default adapter is supported
   std::string convertedAddress = convertToLower(address);
   return (mAddress.compare(convertedAddress) == 0);
   ```

   Any address other than the default adapter's → `BT_ERR_INVALID_ADAPTER_ADDRESS` (~line 230).

2. Even when validation passes, the operation **runs on `mDefaultAdapter` regardless**. In
   `startDiscovery` (~line 1723): `error = mDefaultAdapter->startDiscovery();` — the validated
   `adapterAddress` is never used to pick an adapter. In `pair` (~line 2270+) the device is
   found via `findDevice(address)` on the default adapter and `startPairing(device)` is invoked
   on the service's single-adapter state; `adapterAddress` is merely echoed back in the
   response.

So on the TV-era build, `adapterAddress` is a compatibility/validation parameter, **not a
selector**. The current OSE service, by contrast, genuinely routes: e.g. `pair` does
`auto adapter = findAdapterInfo(adapterAddress); return adapter->pair(request, requestObj);`
(`webosose/com.webos.service.bluetooth2` → `src/bluetoothmanagerservice.cpp` ~lines
1505–1514), backed by `mAdaptersInfo` populated from `mSil->getAdapters()` in
`adaptersChanged()` (~lines 568–606).

**Unverified:** which of the two behaviors webOS 4.10's closed-source binary implements. The
TV-era repo (2014–2019, with the USB udev rules copyrighted 2018) is the closest public
ancestor of the 4.x TV firmware; the OSE multi-adapter routing postdates the 4.x TV branch
point and is gated on OSE-era features (`MULTI_SESSION_SUPPORT` for multi-display). The
on-device test in §5.1 settles it in one `luna-send` call.

### 2.4 Real-world corroboration on webOS 4.x TVs

- **BT Unlock** (`lucaasdias/bt-unlock`, submitted in `webosbrew/apps-repo` PR #284) is a
  Bluetooth manager app tested on a real **webOS 4.4.3** TV (LG 43LK5750PSA, "goldilocks-
  gorongosa"). Its scripts drive the service with plain `luna-send` and never pass
  `adapterAddress` — e.g. `scripts/bt-power.sh`:
  `luna://com.webos.service.bluetooth2/adapter/setState '{"powered":true}'` and
  `adapter/getStatus '{}'`; `scripts/bt-pair.sh`:
  `adapter/pair '{"address":"…","subscribe":true}'`; `scripts/bt-connect.sh`:
  `…/hid/connect '{"address":"…"}'` — and it works (DualShock 4 and an Xbox controller tested
  per the PR description). Consistent with a single-default-adapter service.
- This repo's own app (`app/src/lib/DeviceFinder.svelte`) uses the same calls without
  `adapterAddress`: `adapter/startDiscovery`, `adapter/cancelDiscovery`, `adapter/pair`
  (subscription), `adapter/cancelPairing`, then `hid/connect`;
  `app/src/App.svelte` subscribes `adapter/awaitPairingRequests` and `device/getStatus`;
  `app/src/lib/DeviceCard.svelte` calls `adapter/unpair`. The `Device` interface
  (`app/src/lib/interfaces/bluetooth-device.interface.ts`) already carries an `adapterAddress`
  field — the API reports which adapter owns each device, it just can't be told to use another
  one.
- The webosbrew community wiki (`webosbrew/pages` → `content/pages/luna/bluetooth.md`)
  documents only the *older* `com.webos.service.bluetooth` service (reversed on webOS 3.8) and
  notes: "For equivalent service on webOS 4.x and newer see `com.webos.service.bluetooth2`".
  `webosbrew/apps-repo` issue #20 adds that bluetooth2 "seems to have capability to pair &
  connect to HID devices, down to webOS 4", while LG's own UI only supports game controllers
  from webOS 6.

## 3. The bluez level: every adapter is enumerated, but the default is hardcoded to `hci0`

The bluetooth2 service does not talk to bluez directly; it `dlopen()`s a SIL plugin
(`src/bluetoothsilfactory.cpp`; SIL name from `WEBOS_BLUETOOTH_SIL`, overridable via the
`WEBOS_BLUETOOTH_SIL` env var; the Yocto recipe sets `BLUETOOTH_SIL_NAME = "bluez5"` —
`webosose/meta-webosose` → `meta-webos/recipes-webos/bluetooth/com.webos.service.bluetooth2.bb`).
The bluez5 SIL (`webosose/bluetooth-sil-bluez5`):

- Enumerates **all** bluez adapters: on `org.bluez` name ownership it creates a
  `GDBusObjectManager` and calls `createAdapter()` for every object exposing
  `org.bluez.Adapter1` (`src/bluez5sil.cpp`, `handleBluezServiceStarted` ~lines 60–120,
  `createAdapter` ~lines 275–295). So any adapter bluez sees — onboard UART radio or USB
  dongle — is enumerated by the webOS stack. There is no allowlist.
- Picks the default by **hardcoded string match on `hci0`** (`assignNewDefaultAdapter`,
  ~lines 262–272):

  ```cpp
  std::size_t found = adapter->getObjectPath().find("hci0");
  if (found != std::string::npos)
      mDefaultAdapter = adapter;
  ```

  `getDefaultAdapter()` just returns that member (~line 462). If no adapter is named `hci0`,
  there is **no default adapter** and every LS2 adapter method fails with
  `BT_ERR_ADAPTER_NOT_AVAILABLE`.
- The SIL API itself is multi-adapter-capable (`webosose/bluetooth-sil-api` →
  `include/public/bluetooth-sil-api.h` lines 99–106: `getDefaultAdapter()` *and*
  `getAdapters()`), but the TV-era service consumes only `getDefaultAdapter()`.

**Consequence:** whether by app or by OS policy, the only thing that determines which radio
the TV's Bluetooth stack uses is **which HCI device got index 0**. An app cannot influence
that through any LS2 method; it can only be influenced by kernel-level adapter
registration order (i.e. which hardware is present at boot, and in what order it registered).

## 4. Does webOS enumerate USB Bluetooth dongles at all (on 4.10)?

Evidence that the TV-era platform was *designed* for USB BT dongles — all from the TV-era
bluetooth2 package (`webosose/webos.service.bluetooth2`):

- `files/rules/99-bluetooth.rules` (copyright 2018 LG):

  ```
  ACTION=="add", SUBSYSTEM=="bluetooth",DEVPATH=="*/usb*", ENV{DEVTYPE}=="host", RUN+="/etc/udev/scripts/bluetooth_usb_inserted.sh"
  ACTION=="remove", SUBSYSTEM=="bluetooth", DEVPATH=="*/usb*", ENV{DEVTYPE}=="host", RUN+="/etc/udev/scripts/bluetooth_usb_removed.sh"
  ```

  These rules only fire if a USB-attached Bluetooth controller has created an `hciN` device —
  i.e. LG anticipated the kernel `btusb` path on TV hardware.
- `files/scripts/bluetooth_usb_inserted.sh`:

  ```sh
  systemctl stop hciuart
  rmmod hci_uart
  systemctl restart webos-bluetooth-service
  ```

- `files/scripts/bluetooth_usb_removed.sh` re-inserts `/lib/modules/$(uname -r)/kernel/drivers/bluetooth/hci_uart.ko`,
  restarts `hciuart`, and restarts the BT service.

So LG's own design: **a USB dongle replaces the onboard (UART) radio**, the BT service is
restarted, and the dongle becomes the radio the whole stack uses. "Adapter selection" exists
in webOS only at this udev/system level — never as an app API.

Two caveats, both derived from the code above rather than a 4.10 firmware image:

1. **The `hci0` index matters.** If the dongle is hot-plugged while the onboard radio is up,
   the kernel gives the dongle `hci1` (indices don't renumber). Removing `hci_uart` afterwards
   leaves no `hci0`, and per §3 the bluez5 SIL then has *no* default adapter at all — worse
   than before. The designed flow therefore only works cleanly when the dongle is present at
   boot (dongle registers first → `hci0`; udev script keeps `hci_uart` from taking an index).
   This is code-derived reasoning, **not a 4.10 observation**.
2. **Unverified for 4.10 specifically:** (a) that the stock 4.10 image ships these udev
   rules/scripts; (b) that the 4.10 TV kernel includes `btusb`/`hci_uart` modules. Indirect
   evidence only: LG TV kernels have shipped `CONFIG_BT_HCIBTUSB=m` since the NetCast era
   (`OpenLGTV/lgmod_bcm` → `addons/sources/kernel/.config`, kernel 2.6.31, 2010/2011 Broadcom
   TVs — *not* webOS), and the 2018-era service package's scripts assume both modules exist
   on-device. The stock 4.10 kernel config is not publicly fetchable without pulling a
   per-model kernel source tarball from `opensource.lge.com`. **This must be settled by the
   on-device bench test (issue #6)** — checklist in §5.2.

## 5. If selection isn't exposed on 4.10 — the least-hacky workaround

### 5.1 Rank-ordered workaround

1. **Boot-order selection (zero code, LG's own mechanism).** Plug the dongle into the TV
   while it is powered off. At boot the dongle registers as `hci0` (and if the 4.10 image
   ships LG's udev rules, `hci_uart` gets torn down automatically). The dongle then *is* the
   default adapter for every LS2 call — no app changes needed. Verify with
   `luna-send -n 1 luna://com.webos.service.bluetooth2/adapter/queryAvailable '{}'` and
   compare `adapterAddress` before/after.
2. **Manual replication of LG's script (root, hot-plug case).** The maintainer's TV has root
   SSH (issue #1). If the dongle came up as `hci1` while onboard was `hci0`:
   `systemctl stop hciuart; rmmod hci_uart;` unplug/replug the dongle (or `rmmod btusb;
   modprobe btusb`) so it re-registers as `hci0`, then
   `systemctl restart webos-bluetooth-service`. This is byte-for-byte what
   `bluetooth_usb_inserted.sh` does — using LG's own script semantics is the definition of
   least-hacky.
3. **Always echo `adapterAddress` from the app.** Read the adapter address from
   `adapter/getStatus`/`adapter/queryAvailable` and pass it in `adapter/pair`,
   `adapter/startDiscovery`, `hid/connect`, etc. On the TV-era build this is a no-op that
   happens to be safe (the value equals the default adapter's address, so validation passes);
   if 4.10 turns out to be an OSE-style multi-adapter build, it *correctly routes*. Passing
   a foreign address is the only unsafe variant (→ `BT_ERR_INVALID_ADAPTER_ADDRESS`).
4. **Not recommended / speculative:** talking to bluez directly over the system D-Bus from a
   native service, bypassing bluetooth2. Unverified whether the TV's D-Bus policy allows a
   service to own the bluez agent role alongside `bluetoothd`'s own LS2 service, and pairing
   state would be invisible to the TV's settings UI. Treat as a last resort; needs a
   feasibility spike (out of scope here).

### 5.2 On-device verification checklist (feeds issues #5/#6)

Run with root SSH and record outputs:

```sh
lsusb                                  # dongle visible on USB? note VID:PID
lsmod | grep -E "btusb|hci_uart"       # which BT kernel modules are loaded
hciconfig -a                           # how many hciN, which is which (Address/manufacturer)
luna-send -n 1 luna://com.webos.service.bluetooth2/adapter/queryAvailable '{}'
                                       # adapters[] — count, adapterAddress, stackName/stackVersion
luna-send -n 1 luna://com.webos.service.bluetooth2/adapter/getStatus '{}'
ls /etc/udev/rules.d/ | grep -i blue   # do LG's 99-bluetooth.rules ship on 4.10?
cat /etc/udev/scripts/bluetooth_usb_inserted.sh 2>/dev/null
```

Then: dongle inserted while TV off → boot → re-run `queryAvailable` and compare
`adapterAddress`/`stackVersion` (the onboard radio vs a dongle will have different
firmware/stack strings in `queryAvailable`'s response — that's the cleanest proof of which
radio is in use). Finally test `adapter/pair` with `adapterAddress` set to a *deliberately
wrong* address: if the reply is `BT_ERR_INVALID_ADAPTER_ADDRESS`, the 4.10 build is the
single-default-adapter kind (§2.3 case 1); if it pairs anyway, it routes per-address (§2.3
case 2).

## 6. Known-working dongle chipsets — and what to recommend

**No chipset is verifiable as "known to work with webOS's stack" from the sources reachable
here.** The webosbrew forum is dead (NXDOMAIN, no archive — §1), the webosbrew wiki has no
dongle content (`grep dongle` over `webosbrew/pages` returns nothing), and no GitHub issue or
repo documents a USB BT dongle attached to a webOS TV. Anything claiming "chipset X works on
webOS 4.10" would be invented; this section is therefore a **rationale-based recommendation**
grounded in the kernel driver the TV would use, plus the constraints established above.

Grounding (from `torvalds/linux` → `drivers/bluetooth/btusb_main.c`, fetched at master):

- **CSR BlueCore** dongles (`USB_DEVICE(0x0a12, 0x0001)`, `BTUSB_CSR`, ~line 195) are handled
  directly by `btusb` — the generic HID-capable BR/EDR path, **no firmware file loaded from
  `/lib/firmware`** (the file has dedicated firmware-loading helpers only for
  Intel/Broadcom/Realtek/MediaTek — `#include "btintel.h" "btbcm.h" "btrtl.h" "btmtk.h"`
  ~lines 27–30; `request_firmware` calls exist on those paths, ~lines 3784/3872). On an
  appliance with unknown `/lib/firmware` contents, that matters: an Intel/Realtek/Broadcom
  dongle whose firmware `.bin`/`.hcd` file is missing from the TV simply won't come up.
- Caveat: most cheap "CSR 4.0" dongles are clones with fake CSR IDs; mainline `btusb` contains
  extensive fake-CSR detection (~lines 2735–2775) and workarounds, but a 2019-era TV kernel
  (4.4/4.9 vintage) handles them less gracefully than master. A genuine/named-brand dongle is
  worth the premium.
- **Broadcom BCM20702B0** (~line 141), **Intel** (~lines 507–520), **Realtek RTL8821CE/8822CU/
  8851BU** (~lines 538–570) all have in-kernel support *but* need per-chip firmware files, and
  `btusb` on the TV may not ship them (unverified).

**Recommended specs for the acquisition task (issue #6):**

- Class-compliant, dual-mode (BR/EDR + BLE), HID-capable USB dongle — DualSense pairs over
  classic BR/EDR HID, so any BT 4.0-or-later dual-mode dongle suffices protocol-wise; avoid
  BLE-only or audio-gateway-only devices.
- **Primary pick: genuine CSR8510 A10-class (CSR BlueCore, USB VID `0a12:0001`)** — the one
  mainstream chipset with no firmware-file dependency in `btusb`. Buy a named brand to dodge
  fake-CSR clones.
- **Secondary pick (to test the firmware-file hypothesis): a Realtek RTL8761B-based BT 5.0
  dongle** (e.g. the common TP-Link UB400 class) — if it enumerates (`hciconfig`) but never
  powers up, the missing `rtl_bt/rtl8761b_fw.bin` hypothesis for the TV's `/lib/firmware` is
  confirmed. Two dongles also give the bench test a comparison point.
- USB-A connector (TV ports), and remember §3/§4: whatever dongle is bought must end up as
  `hci0` (plug before boot) to be usable by the TV-era stack at all.

## 7. How this interacts with the repo's current code

- The current app never sends `adapterAddress` (§2.4) — fine today, but per §5.1(3) the v2
  app should read the adapter address once and echo it in every call.
- The Node service (`services/web-input/src`) does not touch Bluetooth at all: it reads
  `/proc/bus/input/devices` and `/dev/input/eventN`
  (`src/services/input.service.ts`, `src/commands/init-hids.command.ts`). That means the
  service side is adapter-agnostic already — input arrives through the kernel HID path no
  matter which radio paired the device. Adapter selection is purely a pairing/connect-time
  concern (bluetooth2 calls from the app side).

## 8. DS5Dongle vs a generic BT dongle — clarification

The ticket describes DS5Dongle as "Pi Zero pairing the controller itself and presenting it
over USB HID". The actual project (`github.com/awalol/DS5Dongle`, fetched: `README.md`,
`src/bt.cpp`, `src/usb_descriptors.cpp`) is:

- A **Raspberry Pi Pico 2 W** (RP2350 + CYW43 Wi-Fi/BT silicon; also Waveshare RP2350B-Plus-W)
  firmware — not a Pi Zero.
- The Pico acts as the **Bluetooth HID host**: it runs BTstack (`src/bt.cpp` —
  `gap_inquiry_start`, L2CAP HID control/interrupt channels with 672-byte MTUs, link-key
  storage/iterating in flash via BTstack TLV, pairing inquiry triggered by the BOOTSEL
  button), pairing the DualSense itself. The controller never knows the TV exists.
- To the host it presents a **wired DualSense over USB**: `src/usb_descriptors.cpp` sets
  `idVendor = 0x054C` (Sony) and `idProduct = 0x0CE6` (DualSense) or `0x0DF2` (DualSense
  Edge), as a USB composite device with a HID interface ("DualSense 5 Gamepad + Touchpad",
  HID 1.11, IN/OUT endpoints) plus USB audio interfaces for headset speaker/mic. Per the
  README: "The Pico device will only be visible to the system after the controller is
  connected."

So the two paths are fundamentally different:

| | Generic USB BT dongle | DS5Dongle |
|---|---|---|
| What the TV's Bluetooth stack sees | The dongle as `hci0`; pairing/connect still done **by webOS** via bluez + `bluetooth2` | Nothing — Bluetooth is bypassed entirely |
| What pairs the controller | webOS (bluetooth2 `adapter/pair`, `hid/connect`) | The Pico (BTstack) |
| What the TV sees | A Bluetooth HID device | A **wired USB DualSense** (HID + audio) |
| Input path | kernel `hid` via the BT stack → `/dev/input` | kernel USB HID → `/dev/input` |
| Dependency on issue #2 outcome | Full — if webOS 4.10's pairing/connect of DualSense is broken at the profile level, a new radio may not fix it (the failure may be in bluez config / HID profile handling, not the onboard radio) | None for pairing; instead depends on webOS 4.10 accepting a **wired USB gamepad** (unverified on 4.10 — the DualSense diagnosis checklist, issue #5, covers this) |
| Multi-controller | Yes (same radio, multiple HID devices) | One dongle per controller (single-connection firmware) |
| Extra hardware | One dongle, any TV | One Pico 2 W per controller, flashed firmware |

In short: a generic BT dongle changes **which radio** runs the (possibly broken) webOS
pairing path; DS5Dongle removes the webOS pairing path from the equation entirely and bets
on the USB HID input path instead. They are complements, not substitutes: the dongle is the
in-stack fix this ticket researched; DS5Dongle is the out-of-band fallback if the in-stack
path proves unfixable.

## 9. Explicitly unverified / open items

1. Whether webOS 4.10's stock image ships the 2018-era udev rules and
   `bluetooth_usb_inserted.sh` scripts (§4) — on-device check §5.2.
2. Whether the 4.10 TV kernel includes `btusb` (and as module or built-in) — no public 4.10
   kernel config was fetchable; on-device `lsmod`/`lsusb` check §5.2.
3. Whether 4.10's bluetooth2 binary is single-default-adapter (TV-era) or per-`adapterAddress`
   routing (OSE-style) — on-device wrong-address `adapter/pair` probe §5.2.
4. The bluez version LG ships on 4.10 — out of scope for this ticket (owned by the DualSense
   research, issue #2); note `adapter/queryAvailable`'s `stackName`/`stackVersion` fields
   expose it on-device for free.
5. Any specific dongle chipset confirmed working on a webOS TV — no reachable report exists
   (§6); the recommendation is driver-derived, to be confirmed by the acquisition bench test
   (issue #6).
6. Whether webOS 4.10 binds a wired USB gamepad (DS5Dongle's input path) — issue #5's
   checklist.
7. LG TV source disclosures for 4.x exist on `opensource.lge.com` (per-model, verified
   reachable) but were not downloaded (multi-GB tarballs); they remain the definitive way to
   confirm items 1–3 without hardware, if ever needed.

## 10. Source index

- webosose/webos.service.bluetooth2 (TV-era service):
  https://github.com/webosose/webos.service.bluetooth2 — `files/rules/99-bluetooth.rules`,
  `files/scripts/bluetooth_usb_inserted.sh`, `files/scripts/bluetooth_usb_removed.sh`,
  `files/sysbus/com.webos.service.bluetooth2.api.json`, `src/bluetoothmanagerservice.cpp`
  (lines ~213–233, ~1202–1249, ~1545–1578, ~1685–1730, ~2221–2295), `src/bluetoothmanagerservice.h`
  (line 238), `src/bluetoothprofileservice.cpp` (line ~192, ~317), `src/bluetoothsilfactory.cpp`,
  `CMakeLists.txt`.
- webosose/com.webos.service.bluetooth2 (OSE service):
  https://github.com/webosose/com.webos.service.bluetooth2 — `src/bluetoothmanagerservice.cpp`
  (lines ~237–315, ~568–606, ~830–905, ~1479–1514), `src/bluetoothmanageradapter.h`,
  `files/sysbus/com.webos.service.bluetooth2.api.json`.
- webosose/bluetooth-sil-bluez5: https://github.com/webosose/bluetooth-sil-bluez5 —
  `src/bluez5sil.cpp` (lines ~60–120, ~262–295, ~462–474).
- webosose/bluetooth-sil-api: https://github.com/webosose/bluetooth-sil-api —
  `include/public/bluetooth-sil-api.h` (lines ~89–106).
- webosose/meta-webosose:
  `meta-webos/recipes-webos/bluetooth/com.webos.service.bluetooth2.bb` (BLUETOOTH_SIL_NAME),
  `meta-webos/recipes-webos/bluetooth/bluetooth-sil-bluez5.bb`.
- Linux mainline: https://github.com/torvalds/linux — `drivers/bluetooth/btusb_main.c`
  (lines ~27–30, ~141, ~194–195, ~2735–2775, ~3784/3872, ~507–570).
- OpenLGTV/lgmod_bcm: https://github.com/OpenLGTV/lgmod_bcm —
  `addons/sources/kernel/.config` (NetCast-era `CONFIG_BT_HCIBTUSB=m`).
- webosbrew community wiki (GitHub): https://github.com/webosbrew/pages —
  `content/pages/luna/bluetooth.md`, `content/pages/hacking/glossary.md`.
- webosbrew/apps-repo: issue #20, PR #284 (BT Unlock).
- lucaasdias/bt-unlock: https://github.com/lucaasdias/bt-unlock — `scripts/bt-*.sh`, `js/luna.js`.
- Superd22/webos-gamepad: issues #6, #7 (webOS 4.9.7 reports).
- awalol/DS5Dongle: https://github.com/awalol/DS5Dongle — `README.md`, `src/bt.cpp`,
  `src/usb_descriptors.cpp`.
- This repo: `app/src/App.svelte`, `app/src/lib/DeviceFinder.svelte`,
  `app/src/lib/DeviceCard.svelte`, `app/src/lib/interfaces/bluetooth-device.interface.ts`,
  `services/web-input/src/services/input.service.ts`,
  `services/web-input/src/commands/init-hids.command.ts`.
- DNS: `https://dns.google/resolve?name=forum.webosbrew.org&type=A` → NXDOMAIN (Status 3).
- Wayback Machine CDX: no captures for `forum.webosbrew.org`; no captures of
  `webostv.developer.lge.com/api/com.webos.service.bluetooth2*`; one capture of LG developer
  forum thread "Compatibility of com.webos.service.bluetooth2 on webOS 24 LG TVs"
  (web.archive.org/web/20250423142542/https://forum.webostv.developer.lge.com/t/…/24280).
- LG open-source disclosure portal: https://opensource.lge.com (product list verified
  reachable; webOS TV per-model source tarballs not downloaded).
