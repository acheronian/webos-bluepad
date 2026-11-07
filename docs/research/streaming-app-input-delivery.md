# How gamepad input reaches streaming apps on webOS 4.10

Research ticket: [Research: How gamepad input reaches streaming apps on webOS 4.10](https://github.com/acheronian/webos-bluepad/issues/13).
Answers the delivery half of the app-owned HID session ([Grilling: Lock the connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)): once a gamepad input device exists on the TV (`/dev/input/eventN`, stock path or our own `uhid`), what must hold for punktfunk and Moonlight to consume it.

## 0. Answer in brief

- **Neither punktfunk nor Moonlight goes through the webOS input service for gamepads.** Both read `/dev/input/eventN` **directly, through SDL's Linux evdev joystick backend** — and both bundle the same community fork of SDL, [webosbrew/SDL-webOS](https://github.com/webosbrew/SDL-webOS): punktfunk ships SDL3 `release-3.4.16-webos.5` (bundled `lib/libSDL3.so.0`), Moonlight TV builds SDL2 `release-2.30.12-webos.6` (bundled `libSDL2-2.0.so.0`). The webOS compositor path (surface-manager → Wayland → SDL keyboard/pointer events) carries only remote keys and pointer — plus, on webOS 23+, pad presses typed as remote keys. On webOS 4.x **nothing consumes gamepad nodes system-wide** (LG's gamepad support starts at webOS 6).
- The fork exists because stock SDL assumes udev, which webOS lacks: it enumerates `/dev/input` by directory scan plus a netlink uevent monitor plus liveness probes.
- A device is consumable iff three things hold:
  1. a **live, readable** `/dev/input/eventN` node (read permission for the app's jailed uid; node not orphaned — webOS's known stale-node failure);
  2. SDL **classifies it as a joystick**: `EV_ABS` with `ABS_X`+`ABS_Y` plus any of `BTN_TRIGGER`/`BTN_A`/`BTN_1`/`ABS_RX`/`ABS_RY`/`ABS_RZ`/`ABS_THROTTLE`/`ABS_RUDDER`/`ABS_WHEEL`/`ABS_GAS`/`ABS_BRAKE`;
  3. it is **recognized as a gamepad**: built-in DB match on bus+VID+PID (with version/CRC fallbacks), else a capability-driven **auto-mapping** gated on `BTN_GAMEPAD` (`BTN_SOUTH`) being present, else the `default,*` fallback mapping. Both bundled SDL forks implement all three.
- **Decisive for our `uhid` device**: on this TV's kernel (4.4.84, no `hid-playstation`), the evdev codes are produced by **4.4-era `hid-input` parsing the HID descriptor we feed `uhid`**: Gamepad-application button *n* → `BTN_GAMEPAD+(n−1)` (i.e. `BTN_SOUTH`, `BTN_EAST`, `BTN_C`, `BTN_NORTH`, `BTN_WEST`, …), D-pad usages → `ABS_HAT0X`/`ABS_HAT0Y`, sticks per descriptor usage (`ABS_X/Y`, `ABS_RX/RY`). Proxying the DualSense's real descriptor does **not** reproduce `hid-playstation`'s codes (square lands on `BTN_C`, create on `BTN_WEST`, …), and the built-in "PS5 Controller" DB entries encode *other* stacks' button indices — so the robust design is to author our own descriptor whose 4.4 `hid-generic` output feeds SDL's auto-mapping, with a **custom VID/PID** (no DB entry shadows the auto-mapping) and a "DualSense"-bearing name (punktfunk's name heuristic still presents it as a DualSense).

## 1. The two streaming apps' input stacks

### 1.1 punktfunk client-webos (SDL3)

- Gamepad input = **SDL3 gamepad API** on the bundled fork; `gamepad.rs` maps SDL3 `Button` transitions to punktfunk wire bits ([src/platform/webos/gamepad.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/gamepad.rs)). SDL synthesizes gamepad events from the raw joystick lanes, which must stay enabled ([src/platform/webos/input.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/input.rs), `mute_unused_events`).
- Controller *kind* detection (for host-side glyphs / trigger effects) is SDL's VID/PID-keyed type first, then a **name fallback**: a device whose name contains "dualsense" is presented as a DualSense even when SDL's database has never seen it (`type_for_name`, same file).
- Mouse/keyboard **bypass SDL**: the app reads `/dev/input/event*` directly because SDL pointer events on webOS come from the compositor, smoothed/resampled for the Magic Remote ([src/platform/webos/evdev/mod.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/evdev/mod.rs)). The DualSense's touchpad and motion nodes are also claimed by this reader (they would otherwise become compositor cursors); the **main pad node is left to SDL and read ungrabbed** ([docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md): "Stream reads ungrabbed").
- Remote keys arrive as SDL keyboard events (webOS scancode block 352–375). The **Magic Remote itself enumerates in SDL as a fake gamepad** ("Smart Remote RCU Input") and is filtered by name (`is_tv_remote`).
- DualSense *output* (adaptive triggers, lightbar) goes over `bluetooth2 hid/internal/sendData` because a Bluetooth pad gets no `/dev/hidraw*` in the app jail (verified webOS 10.3, non-rooted). Rumble uses ordinary evdev force feedback via SDL (`EV_FF` on the pad's event node).
- **Sniff keeper**: a thread holds Bluetooth pads out of sniff mode via `bluetooth2 device/internal/stopSniff`, identifying pads by `jsN` handlers in `/proc/bus/input/devices` ([src/platform/webos/pad_link.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/pad_link.rs)). **webOS 4.10 has no `js*` nodes at all** (`CONFIG_INPUT_JOYDEV` unset, per the on-device diagnosis), so this keeper cannot recognize pads on 4.10 — see §5.
- Supported range includes webOS 3.5–4.x (NDL v1 video path), so this stack is designed for 4.10-class TVs ([README](https://github.com/punktfunk/client-webos)).

### 1.2 Moonlight TV (SDL2)

- [mariotaku/moonlight-tv](https://github.com/mariotaku/moonlight-tv) (the deprecated [mariotaku/moonlight-webos](https://github.com/mariotaku/moonlight-webos) is its predecessor; moonlight-stream has no official webOS port).
- Gamepad input = **SDL2 `SDL_GameController` API** behind an `SDL_IsGameController` gate: a pad that fails the gate is logged "Unrecognized game controller" and **dropped entirely** ([src/app/input/input_gamepad.c](https://github.com/mariotaku/moonlight-tv/blob/main/src/app/input/input_gamepad.c)).
- Mappings: builtin `gamecontrollerdb` + a downloaded database (SDL_GameControllerDB project) + an extra-mappings file, all fed via `SDL_GameControllerAddMappingsFromRW` ([src/app/input/input_gamepad_mapping.c](https://github.com/mariotaku/moonlight-tv/blob/main/src/app/input/input_gamepad_mapping.c)). Underneath the DB sits SDL2's own auto-mapping and default fallback (§2.3), so the gate passes for any standard-code gamepad.
- webOS specifics: remote keys via `navkey_from_sdl_webos`, "hardware mouse" (direct evdev) option, a webOS 9 cursor workaround ([src/app/platform/sdl/navkey_sdl.c](https://github.com/mariotaku/moonlight-tv/blob/main/src/app/platform/sdl/navkey_sdl.c), [src/app/input/app_input.c](https://github.com/mariotaku/moonlight-tv/blob/main/src/app/input/app_input.c)).
- The SDL2 it runs on webOS is the **same webosbrew SDL-webOS fork**, release `2.30.12-webos.6`, fetched by `ExternalSDL2BackportForWebOS` ([commons-c/cmake/ExternalSDL2BackportForWebOS.cmake](https://github.com/mariotaku/commons-c/blob/main/cmake/ExternalSDL2BackportForWebOS.cmake); moonlight-tv `CMakeLists.txt` sets `SDL2_BACKPORT_RELEASE "release-2.30.12-webos.6"`).

## 2. SDL's path from `/dev/input` to gamepad events (webosbrew fork)

### 2.1 Enumeration without udev

Stock SDL's Linux joystick backend needs libudev; webOS has no udev. The fork adds, under `SDL_PLATFORM_WEBOS`:

- **netlink uevent monitor** (kernel uevent socket, group 1) for hot-plug (`src/joystick/webos/uevent_monitor.c`, wired into `src/joystick/linux/SDL_sysjoystick.c` as `ENUMERATION_NETLINK`);
- **directory scanning + polling** of `/dev/input` as fallback (`ENUMERATION_POLLING`);
- **presence probing** (`src/joystick/webos/dev_presence.c`): a node counts as live if `/sys/dev/char/<maj>:<min>` exists (devmode jails only), else `/sys/class/input/<name>` exists (mounted in every jail), else `open()` succeeds or fails with `EACCES`/`EPERM`/`EBUSY` (something is behind it); `ENODEV`/`ENXIO`/`ENOENT` mark an **orphan node** as absent. The fork treats stale `/dev/input` nodes as first-class reality — the same failure mode this repo's `init-hids` command works around.

### 2.2 Classification: what makes a device a joystick

`SDL_EVDEV_GuessDeviceClass` ([SDL3 fork src/core/linux/SDL_evdev_capabilities.c](https://github.com/webosbrew/SDL-webOS/blob/release-3.4.16-webos.5/src/core/linux/SDL_evdev_capabilities.c), identical logic in the SDL2 branch) marks `SDL_UDEV_DEVICE_JOYSTICK` when the device has:

- `EV_ABS` **with both `ABS_X` and `ABS_Y`**, **and any of** `BTN_TRIGGER`, `BTN_A` (`BTN_SOUTH`), `BTN_1`, `ABS_RX`, `ABS_RY`, `ABS_RZ`, `ABS_THROTTLE`, `ABS_RUDDER`, `ABS_WHEEL`, `ABS_GAS`, `ABS_BRAKE`.

A device with only buttons (no `ABS_X`/`ABS_Y`) is *not* a joystick to SDL. Devices are opened `O_RDONLY`; multiple readers are fine; neither app issues `EVIOCGRAB` on pad nodes.

### 2.3 Recognition: DB → auto-mapping → default

Both forks resolve a mapping through the same chain (SDL3 `SDL_PrivateGetGamepadMappingForGUID` / `SDL_PrivateGetGamepadMapping` in `src/joystick/SDL_gamepad.c`; SDL2 equivalents in `src/joystick/SDL_gamecontroller.c`):

1. **Built-in DB**, matched on GUID = bus + vendor + product + version (+ CRC of name), with fallbacks dropping CRC then version. The DB contains DualSense evdev entries (`030000004c050000e60c…`/`050000004c050000e60c…`, "PS5 Controller") in both forks.
2. **Driver auto-mapping** — for the Linux evdev driver this is `LINUX_JoystickGetAutoGamepadMapping` (`src/joystick/linux/SDL_sysjoystick.c`, present in both the SDL3 and SDL2 forks): gated on **`BTN_GAMEPAD` (`BTN_SOUTH`) being present** ("Not a gamepad according to specs"), then built from the device's *actual* capabilities:
   - `BTN_A`(SOUTH)→A, `BTN_B`(EAST)→B; X/Y: **Sony VID → `BTN_WEST`/`BTN_NORTH`**, otherwise `BTN_X`/`BTN_Y`;
   - `BTN_SELECT`→back, `BTN_START`→start, `BTN_MODE`→guide, `BTN_THUMBL`/`BTN_THUMBR`→stick clicks;
   - shoulders: `BTN_TL`/`BTN_TR` (else hat 1 / `ABS_HAT1X`/`Y` fallbacks);
   - triggers: `ABS_HAT2Y`/`ABS_HAT2X` (Linux gamepad spec), else `ABS_BRAKE`/`ABS_GAS` (Android convention), else **`ABS_Z`/`ABS_RZ`** (de-facto Xbox/PS convention);
   - dpad: `BTN_DPAD_*` buttons else hat 0 (`ABS_HAT0X`/`Y`);
   - sticks: `ABS_X`/`ABS_Y` and `ABS_RX`/`ABS_RY` (else `ABS_Z`/`ABS_RZ` as right stick).
3. **`default,*` fallback mapping** — present in both forks' built-in DBs (SDL3: `"default,*"`; SDL2: `"default,Standard GamePad"`), so even an unknown VID/PID joystick opens as a gamepad with a standard-layout guess.

SDL2's `SDL_IsGameController` (moonlight-tv's gate) consults this same chain, so a standard-code device passes even with no DB entry. SDL assigns button indices dynamically in evdev code order starting at `BTN_JOYSTICK` (0x120): for standard gamepad codes b0=`BTN_SOUTH`, b1=`BTN_EAST`, b2=`BTN_C`, b3=`BTN_NORTH`, b4=`BTN_WEST`, b5=`BTN_Z`, b6=`BTN_TL`, b7=`BTN_TR`, b8=`BTN_TL2`, b9=`BTN_TR2`, b10=`BTN_SELECT`, b11=`BTN_START`, b12=`BTN_MODE`, b13=`BTN_THUMBL`, b14=`BTN_THUMBR`, b15+=`BTN_DPAD_*`.

## 3. Evidence the delivery model works on old webOS

- **DS4 → Moonlight, end-to-end, on webOS 5.3.1** (65NANO796NE): stock `bluetooth2 hid/connect` created the input device and *Moonlight consumed it in-stream*; only quirk was re-pairing needed after standby ([webosbrew/apps-repo#20 comment](https://github.com/webosbrew/apps-repo/issues/20#issuecomment-1133489660)).
- **DS4 → RetroArch on webOS 3.x**: `/dev/input/js3` + `event6` producing events, RetroArch picking the pad up (Informatic/stephank, same issue thread; documented in the [DualSense 4.10 research](https://github.com/acheronian/webos-bluepad/blob/research/dualsense-webos410/docs/research/dualsense-webos410.md) §3.2).
- **This repo's own Node service** reads `/dev/input/eventN` directly (npm `input-event`) inside the app-service jail on 4.x, and the upstream failure mode is *stale nodes*, not permission denied ([Superd22/webos-gamepad#2](https://github.com/Superd22/webos-gamepad/issues/2)).
- **punktfunk, on-device, non-rooted (webOS 10.3)**: `/dev/input/event*` is `root:compositor 0660` and the app's uid carries **gid 505** in its supplementary groups — evdev readable from the jail; `/dev/hidraw*` absent for Bluetooth pads; `/dev/bus/usb` absent ([client-webos docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md), evdev module doc comment). 4.10's exact owner/group is unverified — spike item (§5).
- **This TV (4.10, from the diagnosis task)**: `CONFIG_UHID=y`, `CONFIG_HID_GENERIC=y`, `CONFIG_HID_SONY=y`, no `CONFIG_HID_PLAYSTATION`, **`CONFIG_INPUT_JOYDEV` not set** (no `/dev/input/js*` can ever exist), kernel 4.4.84, no kernel Bluetooth at all (all BT in LG's userspace, which itself uses `/dev/uhid` for the stock path).

## 4. What the `uhid` device must produce

### 4.1 The knobs `uhid` gives us

`UHID_CREATE2` (kernel 4.4 `include/uapi/linux/uhid.h`) takes: `name[128]`, `phys[64]`, `uniq[64]`, `rd_size`, **`bus` (u16), `vendor`/`product`/`version` (u32)**, `country`, and the report descriptor `rd_data`. These become the evdev node's `input_id` — exactly the fields SDL's GUID/mapping chain matches on — and the descriptor determines the codes. `uhid` also creates a `/dev/hidrawN`; on 4.10 no `jsN` appears (joydev off).

### 4.2 The 4.4 kernel's code assignment (this is the contract)

Kernel 4.4 `hid-input` (`drivers/hid/hid-input.c`) maps, for a **Gamepad application** (`HID_GD_GAMEPAD`):

- **button usage n → `BTN_GAMEPAD + (n−1)`**: 1→`BTN_SOUTH`, 2→`BTN_EAST`, 3→`BTN_C`, 4→`BTN_NORTH`, 5→`BTN_WEST`, 6→`BTN_Z`, 7→`BTN_TL`, 8→`BTN_TR`, 9→`BTN_TL2`, 10→`BTN_TR2`, 11→`BTN_SELECT`, 12→`BTN_START`, 13→`BTN_MODE`, 14→`BTN_THUMBL`, 15→`BTN_THUMBR` (`BTN_GAMEPAD` = `BTN_SOUTH` = 0x130 in 4.4's `input-event-codes.h`);
- **D-pad usages (0x90–0x93) → `ABS_HAT0X`/`ABS_HAT0Y`** (hat switch, not `BTN_DPAD_*`);
- **stick usages → `ABS_X`/`ABS_Y`/`ABS_Z`/`ABS_RX`/`ABS_RY`/`ABS_RZ`** per descriptor usage.

`hid-playstation` (absent here) maps the *same pad* differently — cross→`BTN_SOUTH`, circle→`BTN_EAST`, **square→`BTN_WEST`**, triangle→`BTN_NORTH`, dpad→`BTN_DPAD_*` ([hid-playstation.c](https://github.com/torvalds/linux/blob/v5.12/drivers/hid/hid-playstation.c) button table). So the same DualSense descriptor parsed by 4.4 `hid-generic` yields **different evdev codes** than on a modern PC: the 3rd button (square) lands on `BTN_C` (which no SDL mapping reads), create (5th) on `BTN_WEST`, options (6th) on `BTN_Z`, L1 (7th) on `BTN_TL`, etc. — scrambled under any DB entry written against `hid-playstation` (the built-in "PS5 Controller" entries encode *other* stacks' button indices, e.g. `a:b1,b:b2,x:b0,y:b3`).

### 4.3 Recommended design for the app-owned session

**Author our own HID report descriptor** (we already parse the pad's input reports; re-serializing into our own descriptor is natural), ordered so 4.4 `hid-input` ordinals land exactly on the codes SDL's auto-mapping reads:

| HID (Gamepad app) | becomes | SDL auto-mapping |
|---|---|---|
| button 1 = cross | `BTN_SOUTH` | A |
| button 2 = circle | `BTN_EAST` | B |
| button 3 = *(spare)* | `BTN_C` | — (unmapped by design) |
| button 4 = triangle | `BTN_NORTH` | Y |
| button 5 = square | `BTN_WEST` | X (Sony-VID branch; also X under default mapping) |
| button 6 = *(spare)* | `BTN_Z` | — |
| button 7 = L1 | `BTN_TL` | left shoulder |
| button 8 = R1 | `BTN_TR` | right shoulder |
| buttons 9, 10 = *(spares)* | `BTN_TL2`/`BTN_TR2` | — |
| button 11 = create | `BTN_SELECT` | back |
| button 12 = options | `BTN_START` | start |
| button 13 = PS | `BTN_MODE` | guide |
| button 14 = L3 | `BTN_THUMBL` | left stick click |
| button 15 = R3 | `BTN_THUMBR` | right stick click |
| `GD_X`/`GD_Y` | `ABS_X`/`ABS_Y` | left stick |
| `GD_RX`/`GD_RY` | `ABS_RX`/`ABS_RY` | right stick |
| `GD_Z`/`GD_RZ` (analog) | `ABS_Z`/`ABS_RZ` | L2/R2 triggers |
| D-pad 0x90–0x93 | `ABS_HAT0X`/`ABS_HAT0Y` | dpad |

- **Identity**: a **custom VID/PID** (not Sony's 054c:0ce6) so no built-in DB entry matches and shadows the auto-mapping with wrong indices; `bus` = `BUS_USB` (0x03) or `BUS_BLUETOOTH` (0x05); **name containing "DualSense"** (e.g. "DualSense Wireless Controller (webos-bluepad)") so punktfunk's `type_for_name` fallback still presents it as a DualSense for glyphs/trigger effects. SDL's only VID-specific behavior in the auto-mapper (X=`BTN_WEST`, Y=`BTN_NORTH` for Sony) matches our ordinal layout anyway, so keeping a Sony-ish name is safe either way.
- **Rejected alternative**: proxy the DualSense's real descriptor verbatim — 4.4 `hid-generic` scrambles the ordinals (square→`BTN_C` lost, create→`BTN_WEST` = wrong X, options→`BTN_Z` lost, L1→`BTN_TL2` …) and the built-in DB entries can't be trusted to match, so buttons would mis-map or silently drop in both apps.
- **Not needed**: no webOS registration, no input-service involvement, no udev rules, no app restart — the SDL forks pick hot-plugged nodes up via netlink/polling. Multiple pads are fine (SDL imposes no cap; moonlight-tv forwards up to 4).

### 4.4 The permission bar

The streaming app's jailed uid must be able to `open(path, O_RDONLY)` the node. Known data points: webOS 10.3 gives `root:compositor 0660` with the app uid in gid 505 (punktfunk, on-device); on 4.x, stock-created event nodes are readable by app services (this repo's service, RetroArch, Moonlight-with-DS4 evidence). **Who creates `/dev/input` nodes on 4.10, and with what owner/mode for a `uhid` device created by a third-party app, is unverified** — webOS's `/dev/input` is userspace-managed (the stale-node bug exists precisely because nodes outlive their devices), so this is a spike item, not an assumption.

## 5. What this adds to the on-device spike ([Prototype: On-device feasibility spike](https://github.com/acheronian/webos-bluepad/issues/12))

After the spike's `uhid` create succeeds, verify the delivery half on the 4.10 TV itself:

1. `ls -la /dev/input/` before/after create + `/proc/bus/input/devices` delta — does the node appear at all, and **what owner/group/mode** does a third-party `uhid` device get? Read the node from a jailed (non-root) context, not just root SSH.
2. Consume it the way the apps do: run a small SDL2/SDL3 probe (or punktfunk itself) and press buttons — confirms classification + auto-mapping end-to-end on 4.10.
3. Watch the compositor: any cursor movement or injected key events while stick/dpad events flow (expected: none on 4.x, but unverified for a third-party-created device with absolute axes).
4. Bonus end-to-end proof of the delivery half: with a revision-A DualSense connected via the stock path, check whether its input already flows into punktfunk on this TV (it should — same SDL path; also captures what the stock path's own `uhid` descriptor produces on 4.10 for comparison).
5. Latency: punktfunk's sniff keeper identifies Bluetooth pads by `jsN` handlers, which 4.10 never has — if input arrives in ~77 ms bursts (the measured sniff batching on 10.3), check whether `bluetooth2 device/internal/stopSniff` exists on 4.10 and whether bluepad should call it for its own session's pads.

## 6. Sources

| Claim | Source |
|---|---|
| punktfunk ships SDL3 `3.4.16-webos.5`, bundled `libSDL3.so.0`, "webOS ships no SDL3" | [punktfunk/client-webos](https://github.com/punktfunk/client-webos) `Cargo.toml`, `taskfiles/toolchain.yml`, `build.rs` (per the [punktfunk architecture research](https://github.com/acheronian/webos-bluepad/blob/research/punktfunk-architecture/docs/research/punktfunk-architecture.md)) |
| punktfunk gamepad input via SDL3 gamepad API; kind detection + `type_for_name`; remote-as-fake-pad filter | [client-webos src/platform/webos/gamepad.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/gamepad.rs), [input.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/input.rs) |
| punktfunk evdev bypass (mouse/kb/pad touchpad+motion), grab semantics, jail access facts (gid 505, 0660, no hidraw for BT pads) — on-device, webOS 10.3 | [client-webos src/platform/webos/evdev/mod.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/evdev/mod.rs), [docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md) |
| punktfunk webOS 3.5–4.x support; NDL v1 | [client-webos README](https://github.com/punktfunk/client-webos), [docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md) |
| Sniff keeper; `jsN`-based pad detection; 77.5 ms batching | [client-webos src/platform/webos/pad_link.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/pad_link.rs), [docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md) |
| webOS 23+ types pads as remote keys | client-webos [docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md) |
| moonlight-tv SDL_GameController gate + "Unrecognized" drop; mapping DBs; webOS navkey/hardware-mouse | [mariotaku/moonlight-tv](https://github.com/mariotaku/moonlight-tv) `src/app/input/input_gamepad.c`, `input_gamepad_mapping.c`, `app_input.c`, `src/app/platform/sdl/navkey_sdl.c` |
| moonlight-tv uses webosbrew SDL-webOS as SDL2 `release-2.30.12-webos.6` | [commons-c cmake/ExternalSDL2BackportForWebOS.cmake](https://github.com/mariotaku/commons-c/blob/main/cmake/ExternalSDL2BackportForWebOS.cmake); moonlight-tv `CMakeLists.txt` |
| No-udev enumeration: netlink monitor + polling + presence probes; orphan-node errno logic; jail sysfs mounts | [webosbrew/SDL-webOS](https://github.com/webosbrew/SDL-webOS) `src/joystick/webos/uevent_monitor.c`, `dev_presence.c`, `src/joystick/linux/SDL_sysjoystick.c` (SDL3 `release-3.4.16-webos.5` and SDL2 `release-2.30.12-webos.6` branches, inspected directly) |
| Joystick classification criteria | SDL-webOS `src/core/linux/SDL_evdev_capabilities.c` (`SDL_EVDEV_GuessDeviceClass`) |
| Mapping chain; `default,*` fallback; DualSense DB entries | SDL-webOS `src/joystick/SDL_gamepad.c` + `SDL_gamepad_db.h` (SDL3), `SDL_gamecontroller.c` + `SDL_gamecontrollerdb.h` (SDL2) |
| Linux evdev auto-mapping (`BTN_GAMEPAD` gate, capability table, Sony X/Y branch, trigger conventions) | SDL-webOS `src/joystick/linux/SDL_sysjoystick.c` (`LINUX_JoystickGetAutoGamepadMapping`, both branches) |
| Kernel 4.4 gamepad button ordinal mapping; dpad→`ABS_HAT0X/Y`; `UHID_CREATE2` struct | [linux v4.4](https://github.com/torvalds/linux/tree/v4.4) `drivers/hid/hid-input.c`, `include/uapi/linux/input-event-codes.h`, `include/uapi/linux/uhid.h` |
| `hid-playstation` button codes (cross→SOUTH, square→WEST, dpad→`BTN_DPAD_*`) | [linux v5.12 hid-playstation.c](https://github.com/torvalds/linux/blob/v5.12/drivers/hid/hid-playstation.c) |
| DS4 → Moonlight end-to-end on webOS 5.3.1; standby re-pair quirk | [webosbrew/apps-repo#20 (comment)](https://github.com/webosbrew/apps-repo/issues/20#issuecomment-1133489660) |
| DS4 → RetroArch on webOS 3.x (`js3`/`event6`); LG gamepad support starts webOS 6 | apps-repo#20 thread; [webosbrew Luna API docs](https://www.webosbrew.org/pages/luna-service-comwebosservicebluetooth.html); [DualSense 4.10 research](https://github.com/acheronian/webos-bluepad/blob/research/dualsense-webos410/docs/research/dualsense-webos410.md) §3.2 |
| Upstream service reads `/dev/input/eventN`; stale-node failure mode | this repo `services/web-input/src/services/input.service.ts`; [Superd22/webos-gamepad#2](https://github.com/Superd22/webos-gamepad/issues/2) |
| 4.10 TV kernel facts: 4.4.84, `CONFIG_UHID=y`, `CONFIG_HID_GENERIC`/`HID_SONY=y`, no `hid-playstation`, **`CONFIG_INPUT_JOYDEV` unset**, no kernel BT | [Task: Run the DualSense diagnosis checklist on the TV](https://github.com/acheronian/webos-bluepad/issues/5) resolution |
