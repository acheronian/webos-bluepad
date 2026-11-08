# Bluetooth link-key access on the webOS 4.10 TV — app-owned pairing gate probe

Resolves [Task: Probe Bluetooth link-key access on the TV for the app-owned pairing gate](https://github.com/acheronian/webos-bluepad/issues/28).

- **Date:** 2025-11-08
- **TV:** LG OLED65C97LA, webOS 4.10, rooted (webosbrew), root SSH. Adapter `44:cb:8b:91:1c:41`, BT firmware `23:15:04:19` (`/tmp/bt_local_info.txt`).
- **Probe device:** DualSense rev A (FCC grant AK8CFIZCT1A, `d0:bc:c1:67:74:92`), stock-paired via the BluePad smoke-test rig (`tools/on-tv/deploy.sh` + `bluepad-ui pair`, which drives the stock LS2 `bluetooth2 adapter/pair` subscription).
- **Method:** root-shell inspection of the running stack, the on-disk key store, and three controlled write tests (in-place field edit, hand-authored new entry, link-key corrupt/restore), each verified across real power cycles.
- All key material in this doc is **redacted**; pristine and post-test copies of the store are held locally, untracked, and deliberately kept out of the repo.

## Verdict

**Key read+write: GO — with one carve-out.** LG's "proprietary" stack is Android **bluedroid** on a MediaTek combo chip; link keys live in a plain-text, root-writable `bt_config.conf` in the documented bluedroid format. A root process can **read** keys and **modify the key value of an existing bond**, and the stack demonstrably loads and authenticates with the written value (corrupt → power cycle → connect fails on auth; restore → reconnects). **Forging a new device entry is not tolerated** — the stack drops hand-authored sections at the next flush and never enumerates them.

Combined with the headline finding that the TV has **no kernel Bluetooth at all**, the app-owned pairing architecture is forced onto the dongle radio with BluePad's own key store; LG's store is bypassed — the alternative outcome the ticket pre-named. The spec's fallback (stock pairing as 0.1.0 trust establishment) is unaffected.

## 1. The stack is Android bluedroid on a MediaTek combo chip

- Daemons: `/usr/sbin/webos-bluetooth-service` (LS2 `com.webos.service.bluetooth2`) and `/usr/sbin/bluetooth-dot-service`. `connmand` runs with `--noplugin=bluetooth,bluetooth_legacy`.
- Mapped libraries: `libbluetooth.default.so` (bluedroid's canonical name), `/usr/lib/bluetooth-sils/libmtk.so.1.0.0`, `libbt-vendor_mtk.so`; Apache-2.0 `android-external-bluetooth-bluedroid` license file shipped in `/usr/palm/plugins/license/opensource/`.
- **No kernel Bluetooth**: `lsmod` empty for hci/blue, `/sys/class/bluetooth/` does not exist. The entire stack is userspace over a vendor HAL (`/dev/stpbt`).
- `webos-bluetooth-service` holds **`/dev/uhid` open** (fd 160): the stock stack itself injects HID devices through uhid — directly relevant to the gate's exclusivity point and to BluePad's own uhid plans.
- An HCI snoop log exists at `/var/log/bt_snoop.cfa` (btsnoop format) — a fallback read path and pairing-trace evidence source.

**Gate implication:** raw L2CAP on the *onboard* radio is impossible without kernel HCI sockets. App-owned transport requires the dongle ([Prototype: On-device feasibility spike](https://github.com/acheronian/webos-bluepad/issues/12), dongle bench [#6](https://github.com/acheronian/webos-bluepad/issues/6)).

## 2. The key store

- **Path:** `/mnt/lg/cmn_data/btsvc/bluedroid-mtk/rec/bluedroid/bt_config.conf` (twin `bt_config.bak`), on the persistent `/mnt/lg` partition.
- **Format:** plain-text INI, the standard bluedroid config store. Sections: `[Info]`, `[Adapter]`, one per bonded device keyed by MAC. No obfuscation, no checksums, no encryption.
- **Permissions:** `-rw-rw---- root root` — readable and writable by root.
- **Lifecycle:** the stack loads it at boot and flushes its in-memory database back to disk at boot/shutdown (rotating the previous file into `.bak`). A power cycle with hand-edited content confirmed both directions: the stack read our edits, then later rewrote the file from memory (normalizing away fields it didn't keep).
- Located via `strings` on `webos-bluetooth-service` (the store path is compiled in), confirmed by mtime matching pairing time. A filesystem-wide MAC grep is a poor locator here: `/mnt` is large enough that a naive sweep doesn't complete, and (worse) the stack rewrites the file, so stale copies mislead.

## 3. Read (probe point 2)

The pad's section (`[d0:bc:c1:67:74:92]`) holds, in the clear:

- `LinkKey = <REDACTED 16-byte BR/EDR link key, 32 hex chars>` — standard link key, no LG-proprietary wrapping.
- `LinkKeyType = 4` — bluedroid's *unauthenticated combination key*: SSP without passkey/numeric comparison. Stock pairing does Just-Works-style SSP; BluePad's own pairing would produce the same key type. No exotic trust format to replicate.
- `DevType = 1` (BR/EDR), `AddrType = 0` (public), Sony `HidVendorId/HidProductId = 1356/3302`, cached `HidDescriptor`, `Service` UUIDs (HID).
- `[Adapter]` holds the TV's own LE identity keys (`LE_LOCAL_KEY_IRK/IR/DHK/ER` — redacted); the paired magic remote's section holds standard BLE LTK material (`LE_KEY_PENC/LENC/LID/PID`).

Everything matches the open-source bluedroid format, so upstream documentation applies.

## 4. Write tests (probe point 3)

Three tests, each across real power cycles (mains removed):

1. **In-place field edit (pad's `Name`)** — survived on disk across a power cycle; the pad then reconnected using the `LinkKey` from our *edited* file (proof the stack reads what we write). A later stack flush rewrote the name from the live advertisement. Conclusion: the stack rewrites from memory; on-disk edits are loaded at boot, not ignored.
2. **Hand-authored new entry ("phantom")** — a full section mirroring the pad's fields (later completed with its `Service` line) under a spare MAC. Never enumerated by `device/getStatus`, the TV's Bluetooth settings UI (audio-only on this webOS version), or the legacy GamePads app; **dropped by the stack's own rewrite at the next boot**. Conclusion: **forging new bonds in LG's store is not tolerated.**
3. **Link-key corrupt/restore (decisive)** — corrupted one hex digit of the pad's `LinkKey`, power-cycled: the pad **failed to connect** (auth failure; bond intact — `paired: true`, `connectedProfiles: []`), and the file still held the corrupted value after boot (**no second authoritative store**). Restored the correct key, power-cycled: the pad reconnected. Conclusion: **the stack loads and authenticates with the key value a root process writes.**

## 5. Cross-check against the four-point gate

| Gate point | Status after this probe |
|---|---|
| **Transport** | Onboard radio: impossible (no kernel Bluetooth). Dongle-gated → spike [#12](https://github.com/acheronian/webos-bluepad/issues/12) / [#6](https://github.com/acheronian/webos-bluepad/issues/6). |
| **Key read+write** | **Settled here.** Read: GO. In-place key write: GO (authoritative, verified end-to-end). New-entry forging: NO. |
| **Exclusivity** | Open — note the stock stack itself owns `/dev/uhid`; on-device spike. |
| **Incoming accept** | Open — on-device spike. |

## 6. Architectural conclusion

A Bluetooth link key is a shared secret bound to the **pair of addresses** that created it. A key the pad established with the onboard adapter cannot authenticate a dongle with a different MAC, and dongle-paired keys are useless to the onboard stack. Therefore:

- **App-owned pairing rides the dongle, and BluePad keeps its own key store entirely, ignoring LG's** — exactly the alternative the ticket named. This is not merely allowed; it is forced by the no-kernel-BT finding and by address-pair binding.
- Writing into LG's store remains available as a *hybrid-path tool for existing bonds only* (e.g., key rotation for a stock-paired pad), but it is not a trust-establishment route: new entries get dropped.
- **Fallback unchanged:** stock pairing remains 0.1.0's trust establishment (proven for rev A on the onboard radio; the spec already allows app-owned pairing to slip past 0.1.0).

## Evidence inventory

- Pristine and post-test copies of `bt_config.conf`/`.bak` (real key material — held locally, untracked, never committed).
- Pairing LS2 call sequence from the smoke-test daemon: `pair-probe-1794163252.log` (repo root at probe time).
- Terminal transcripts of every test (this doc's chronological record); key values redacted.
