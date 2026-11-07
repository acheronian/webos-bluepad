# Research: What separates DualSense revisions AK8CFIZCT1A (connects) from AK8CFIZCT1B (fails) on webOS 4.10

Ticket: acheronian/webos-bluepad#11 — "Research: What separates DualSense revisions AK8CFIZCT1A (connects) from AK8CFIZCT1B (fails) on webOS 4.10"
Branch: `research/dualsense-revisions`

## Question

The maintainer's two older DualSense pads (FCC ID **AK8CFIZCT1A**) pair **and connect** through the
stock `bluetooth2 hid/connect` path on webOS 4.10.2; the pad whose FCC ID ends in **B**
(**AK8CFIZCT1B**, the diagnosis unit `a0:fa:9c:62:7d:1a`) fails with error 130 after a consistent
~10 s inside LG's userspace HID session layer. What do the two FCC grants correspond to — which
DualSense hardware/firmware revisions A and B are (model numbers, production eras, what changed) —
and what differs between them at the Bluetooth/HID level (BT chip or module, firmware, HID report
descriptor, connection-establishment behavior) that could explain LG's proprietary 4.10 stack
accepting one and dying mid-handshake on the other? Also: any community reports of
revision-specific DualSense Bluetooth quirks (revision B failing on other embedded or legacy
stacks)?

## TL;DR — the answer

- **A and B are silent internal revisions of the same retail product.** All three DualSense FCC
  grants certify the identical model number **CFI-ZCT1W** ("Wireless Controller", applicant Sony
  Interactive Entertainment, radio = Bluetooth BR/EDR FHSS 2402–2480 MHz, test firm UL Japan):
  - **AK8CFIZCT1** — granted **2020-04-17** — launch DualSense (2020 production).
  - **AK8CFIZCT1A** — granted **2021-03-08** (tested Feb 2021, UL report 13698456H-A-R1) — the
    2021 hardware revision. **This is the generation that connects on the TV.**
  - **AK8CFIZCT1B** — granted **2023-03-14** (tested Feb 2023, UL report 14617231H-A; antenna
    characterized Dec 2022 at Sony Tokyo) — the 2023 hardware revision. **This is the generation
    that fails.**
  The FCC label on the bottom of the pad is the only external revision tell; a fourth grant,
  **AK8CFIZCT2W** (2025-06-08), covers the newer CFI-ZCT2W model line and is out of scope here.
- **Community teardown taxonomy (real, consistent across channels):** the DualSense line has six
  mainboard generations, **BDM-010 … BDM-060**, distinguished by markings on the PCB. Dating by
  teardown-channel coverage: BDM-010 (2020), BDM-020 (2021), BDM-030 (late 2022 — existed by
  Nov 2022, hence *before* grant B and still under grant A markings), BDM-040 (2023), BDM-050
  (late 2024, the "DualSense V2" battery-refresh era), BDM-060 (2025+, CFI-ZCT2W). Therefore
  **grant A ≈ BDM-020-era production (2021–2022) and grant B ≈ BDM-040-era production (2023+)**.
  The failing pad is 2023+ stock; the working pads are 2021–2022 stock.
- **What changed in B at the radio level (primary evidence):** a redesigned **PWB printed
  inverted-F antenna with 3.8 dBi peak gain vs 2.6 dBi in A** (a +1.2 dB jump — RF front end and
  antenna layout redesigned, measured in Sony's Tokyo anechoic chamber Dec 2022), and the
  certification test firmware generation jumped from **1.2.10 (A) to 3.77 (B)** (launch: 0.1.85).
  A new FCC grant was only needed because the radio implementation changed.
- **Bluetooth module vendor switch (strongest hardware signal):** the failing pad's MAC
  `a0:fa:9c:62:7d:1a` falls in OUI **A0:FA:9C = Weifang Goertek Electronics Co., Ltd** (IEEE
  registry via Wireshark `manuf`), not a Sony OUI. Goertek is a Chinese RF module manufacturer:
  the B generation very likely sources its Bluetooth module (or at least its MAC block) from
  Goertek rather than the earlier generation's supplier. A real-world DS4Windows log (2026) shows
  another Goertek-OUI DualSense (`A0:FA:9C:1D:42:01`) connecting fine on Windows — the pad is not
  defective; the host stack is the variable. (Verification: read the two rev-A pads' MACs off the
  TV via `device/getStatus` — if they are Sony OUIs, the vendor switch is confirmed on-device.)
- **Firmware generations change HID-level behavior (first-party, documented):** Sony's own
  hid-playstation driver maintainer states the pad reports an "update version" because "there can
  be many different variations of a controller over time with the same physical shell, but with
  different PCBs and other internal changes", and the kernel changed output-report *format*
  (rumble v2 moved from `valid_flag0` bit 0 to `valid_flag2` bit 2) based on that version — feature
  version 2.21 (2021-era firmware) already changed HID output report formatting. The B generation
  ships a firmware two major generations newer than A's certification baseline.
- **Documented precedent of BT HID descriptor drift breaking hosts:** the 2023 DualSense **Edge**
  ships a Bluetooth HID report descriptor with a **trailing null byte and a significantly
  different structure than its USB descriptor**, which mis-registers on Linux hid-playstation and
  triggers `link tx timeout` / connection drops (linux-input, May 2026). This is the same failure
  *shape* as our TV: a newer Sony pad generation presenting changed BT HID session material that
  a host's HID layer mishandles. It makes "rev B's newer firmware presents something in the HID
  handshake that LG's 2019-era stack can't parse" the leading explanation.
- **Ranked hypotheses for the error-130-after-10 s failure** (to feed the connection-layer
  strategy):
  1. **H-B1 (leading): HID session-material drift.** Rev B's 3.x-era firmware presents a changed
     SDP record / HID report descriptor / feature set during L2CAP HID session establishment;
     LG's 2019-era userspace HID layer stalls on it and times out (~10 s consistent, zero kernel
     activity — the stack dies before ever creating a uhid device). Precedents: Edge descriptor
     bug; rumble-v2 output format change.
  2. **H-B2: RF module behavior.** The Goertek module's EDR/L2CAP timing or negotiation behavior
     trips LG's session layer; the consistent ~10 s smells like a link/session supervision
     timeout inside the proprietary stack.
  3. **H-B3 (weak): descriptor-shape-specific acceptance.** A compiled-in device filter keyed on
     HID/SDP specifics rather than VID/PID — the A/B differential rules out a simple
     "DualSense whitelist" (rev A connects), but a shape-keyed acceptance remains conceivable.
- **Community reports of rev-B failing on other legacy stacks: none found via accessible
  channels** (search engines bot-blocked from this research environment; Reddit/pullpush
  unreachable). Closest evidence: the Edge descriptor bug (above), the failing-generation pad
  working fine on modern Windows, and DualSense pads connecting natively on webOS 6.5 TVs
  (moonlight-tv #576 — revision of those pads unknown). Residual gap; the PC-side descriptor
  differential below settles it better than any forum post could.
- **Implication for the strategy:** the stock `hid/connect` path on this TV is reliable only for
  **2020–2022 production** pads (grant A and earlier). Everything sold since ~mid-2023 (BDM-040,
  BDM-050, and the CFI-ZCT2W line) is grant-B-or-later and, on the evidence, will fail on the
  stock path. An app-owned HID path (raw L2CAP + uhid) is revision-agnostic by construction.
  Deciding the rev-B stance *is* the connection-layer strategy ticket's job; this research is its
  input.
- **Cheap verification steps for the strategy session** (no new ticket needed — fold into
  [Grilling: Lock the connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)):
  1. On the TV: read the two rev-A pads' MAC addresses (`device/getStatus`) and check their OUIs
     (Sony vs Goertek).
  2. On any PC over USB: read the failing pad's `hw_version` / `fw_version` / `update_version`
     (Linux dmesg prints "Registered DualSense controller hw_version=… fw_version=…" — pins the
     exact generation; `dualsensectl` shows the same).
  3. Optional, definitive: on a Linux PC with a BT dongle, `btmon`/`hcidump` a rev-A and a rev-B
     connect and diff the L2CAP/HID handshake (SDP records, HID report descriptor bytes).

## 1. The three FCC grants (primary: FCC ULS records via fccid.io mirror + UL Japan test reports)

The DualSense line sold as **CFI-ZCT1W** carries exactly three FCC equipment-authorization grants
under FCC ID AK8CFIZCT1, each a "DSS — Part 15 Spread Spectrum Transmitter" (the 2.4 GHz Bluetooth
radio), applicant Sony (Interactive Entertainment / Sony Group Corporation), tested by UL Japan,
Ise EMC Lab:

| Grant | Final action | Test dates | UL report | Model | Antenna | Antenna gain | Certification test FW |
|---|---|---|---|---|---|---|---|
| AK8CFIZCT1 | 2020-04-17 | — | (report 4691261) | CFI-ZCT1W | IFA | 2.7 dBi max | 0.1.85 |
| **AK8CFIZCT1A** | **2021-03-08** | Feb 9–21, 2021 | 13698456H-A-R1 | CFI-ZCT1W | IFA | 2.6 dBi max | 1.2.10 |
| **AK8CFIZCT1B** | **2023-03-14** | Feb 2023 | 14617231H-A | CFI-ZCT1W | Inverted F (PWB printed) | **3.8 dBi** | **3.77** |

Sources: fccid.io grant pages ([AK8CFIZCT1](https://fccid.io/AK8CFIZCT1),
[AK8CFIZCT1A](https://fccid.io/AK8CFIZCT1A), [AK8CFIZCT1B](https://fccid.io/AK8CFIZCT1B)); UL
Japan radio test reports (A: FCC exhibit 5151488; B: FCC exhibit 6414722 "FCC Test Report_DSS",
report 14617231H-A, issued 2023-02-16; original: exhibit 4691261).

Key facts from the test reports:

- All three certify the **same model number CFI-ZCT1W** — these are silent internal revisions, not
  new retail products. (User manuals differ only in internal part number: 3501167 (A era) vs
  3501321 (B era); the B label exhibit shows the FCC ID laser-engraved on the bottom of the pad.)
- Radio spec is constant on paper: Bluetooth BR/EDR, FHSS (GFSK, π/4DQPSK, 8DPSK), 2402–2480 MHz,
  79 channels/1 MHz. What changed is the implementation behind it.
- **Antenna redesign in B:** A used an IFA at 2.6 dBi; B uses a *PWB printed* inverted-F antenna at
  **3.8 dBi**, characterized in Sony Interactive Entertainment's Tokyo anechoic chamber in
  **Dec 2022** (B's "AUT report", FCC exhibit 6414720). A +1.2 dB antenna-gain jump plus a new
  grant means the RF front end was redesigned, not tweaked.
- **Firmware generation gap:** the certification test firmware ("PlayStation Wireless Controller
  FW for function control") is 0.1.85 (2020) → 1.2.10 (2021) → **3.77 (2023)**. These are
  test-mode builds, but they track the pad's embedded firmware generation: B-generation pads ship
  a firmware two major generations newer than A-generation pads.

A fourth grant, **AK8CFIZCT2W** (latest application 2025-06-08), covers the newer CFI-ZCT2W
model line (post-BDM-050 generations); no AK8CFIZCT1C/D or AK8CFIZCT2 exists. So the standard
DualSense has exactly three radio generations: 2020, 2021 (A), 2023 (B).

## 2. Mapping the grants to the community's BDM board revisions

The repair/teardown community identifies DualSense generations by the **BDM-xxx** mainboard
part number, with six generations documented:

- **BDM-010** — launch board (2020). Revision-specific aftermarket kits (e.g. eXtremerate RISE4
  back-button kit, sold ~2022) are labeled "BDM-010 BDM-020", i.e. two generations existed by
  then.
- **BDM-020** — 2021 revision. A Spanish teardown channel published "HOW TO DISTINGUISH THE
  DUALSENSE MODELS // BDM-010, BDM-020 AND BDM-030" on **Nov 20, 2022** — so BDM-020 was mature
  and **BDM-030 existed by late 2022**. A viewer comment distinguishes "BDM-020 Big IC and
  BDM-020 Small IC" sub-variants (the main IC changed within the A era).
- **BDM-030** — late 2022. Because it predates grant B (2023-03-14) and a device must bear its
  FCC ID at sale, **BDM-030 still carries grant-A markings**.
- **BDM-040** — 2023. "DualSense Revision Comparison: BDM-010 vs 020 vs 030 vs 040" (How To X,
  May 24, 2024) documents four generations by then. **Grant B (certified Feb–Mar 2023, antenna
  measured Dec 2022) maps to the 2023 production generation — BDM-040.**
- **BDM-050** — late 2024 ("The 5th Revision of the DualSense Has Arrived | BDM-050 vs 040",
  How To X, ~Nov 2024; the DualSense V2 battery-refresh era). Presumably still under grant B
  (no new CFIZCT1 grant exists after 2023).
- **BDM-060** — 2025+ ("The 6th Revision … BDM-060 vs 050", ~early 2026), the CFI-ZCT2W line
  under grant AK8CFIZCT2W.

Sources: YouTube (real channels, consistent across independent creators): How To X
"Revision Comparison" videos, La mazmorra de Isaac (Nov 20, 2022), eXtremerate kit listings.

Conclusion for our pads: the two working pads (AK8CFIZCT1A) are **2021–2022 production
(BDM-020, possibly early BDM-030)**; the failing pad (AK8CFIZCT1B) is **2023+ production
(BDM-040 or BDM-050)**. The external generation tell is the FCC label; the exact BDM number is
printed on the mainboard (only visible after opening the pad — not required for the strategy
decision).

## 3. The Bluetooth module vendor switch (the Goertek OUI)

The failing pad's MAC address `a0:fa:9c:62:7d:1a` (recorded during the diagnosis) falls in IEEE
OUI **A0:FA:9C = "Weifang Goertek Electronics Co., Ltd"** (verified against the IEEE-derived
Wireshark `manuf` database; macvendors.com agrees). Goertek is a major Chinese acoustics/RF
module manufacturer. Sony OUIs (30:F9:ED, 5C:C6:AB, …) are what earlier Sony pads typically show.

This is the strongest hardware-level signal in the document record: the B generation's Bluetooth
radio is very likely a **Goertek-sourced module (or a Goertek-built MAC block)**, replacing
whatever the A generation used — consistent with the redesigned PWB antenna and the new FCC
grant.

Corroboration that B-generation pads are healthy: a DS4Windows issue log (hbashton/DS4Windows#122,
Sept 2026) shows a DualSense with MAC `A0:FA:9C:1D:42:01` (same Goertek OUI) connecting over
Bluetooth to Windows 11 and enumerating normally (VID 054C PID 0CE6). The pad is not broken —
modern stacks handle it; LG's 2019-era stack does not.

Open verification (cheap, on the TV): the two rev-A pads' MAC addresses are readable via
`device/getStatus`. If they are Sony OUIs, the vendor switch A→B is confirmed on-device.

## 4. Sony pads change HID behavior across firmware/hardware generations (first-party)

Sony's own kernel driver (hid-playstation, authored by Sony Interactive Entertainment) encodes
the fact that revisions differ at the HID level:

- The driver reads a vendor feature report exposing `hw_version`, `fw_version`, and an
  `update_version`. Sony's maintainer on the "update version" (linux-input, Oct 2022, commit
  9fecab247e):
  > "It is distinct from the firmware version as there can be many different variations of a
  > controller over time with the same physical shell, but with different PCBs and other internal
  > changes. The update version (internal name) is used as a means to detect what features are
  > available and change behavior."
- **Feature version 2.21 introduced a new vibration method** (2021-era firmware): the classic
  rumble output moved from `DS_OUTPUT_VALID_FLAG0_COMPATIBLE_VIBRATION` (valid_flag0 bit 0) to
  `DS_OUTPUT_VALID_FLAG2_COMPATIBLE_VIBRATION2` (valid_flag2 bit 2) — i.e. the **HID output
  report format itself changed across firmware generations**, and hosts must gate on the
  version.
- The driver's device probe logs "there can be significant differences between versions"
  (commit 0b25b55d34, "report DualSense hardware and firmware version").

So: revision A (2021, FW 1.x-era certification) and revision B (2023, FW 3.x-era certification)
are separated not just by radio hardware but by **two firmware generations of HID-level
behavior changes** — exactly the layer where the TV's stack dies.

## 5. Documented precedent: newer Sony pad generations break hosts at the BT HID layer

**DualSense Edge (2023) Bluetooth HID report descriptor bug** (linux-input mailing list, May
2026, "hid-playstation: DualSense Edge Bluetooth HID report descriptor trailing null byte…"):

- Over Bluetooth, the Edge sends a HID report descriptor **ending in an invalid trailing null
  byte** (`… c0 00` after End Collection) and structurally different from its USB descriptor
  (BT: 4 axes/14 buttons/different vendor report IDs; USB: 6 axes/15 buttons).
- Linux's HID layer logs `unknown main item tag 0x0`, mis-registers the device as a generic
  DualSense, formats output reports with the wrong layout — and **the controller then rejects
  output reports, triggering `link tx timeout` and connection drops**. Over USB the same pad
  works perfectly.
- The reporter's pad reports `hw_version=0x01000223 fw_version=0x0100008b` — showing the
  version-report machinery works over USB even when BT is broken.

This is the same failure *shape* as our TV: a newer Sony pad generation presents changed
Bluetooth HID session material; a host whose HID layer can't tolerate the change stalls or
mis-handles the session. LG's proprietary 4.10 stack (a 2019-era codebase) is exactly the kind
of rigid, never-updated consumer the Edge bug would predict. Whether rev B's standard-DualSense
descriptor differs from rev A's in the same way is **not yet documented** — the btmon
differential (TL;DR item 3) would capture it directly.

## 6. Community reports of revision-specific Bluetooth quirks

Searched: GitHub issues (bluez-adjacent, RetroArch, moonlight/Lakka, DS4Windows, webOS projects),
the linux-input kernel lists (lore.kernel.org), and general web (heavily bot-blocked from this
research environment). Findings:

- **Direct "rev B fails on stack X" reports: none found.** The revision-specific failure space is
  thinly documented in public; most BT-failure reports predate or don't identify revisions.
  Residual gap — worth a human-assisted pass on Reddit (r/webos, r/LGwebOSTV, r/Playstation) and
  RetroPie/Lakka forums with normal browser access, but the on-device/PC differentials are
  higher-value.
- **Positive controls for the failing generation:** Goertek-OUI DualSense works on Windows 11
  (DS4Windows#122); DualSense pads connect over Bluetooth on webOS 6.5 LG TVs
  (mariotaku/moonlight-tv#576 — DualShock and DualSense both pair and connect natively on
  webOS 6.5.3; revision unknown). So the failure class is "old/quirky stacks", not the pad.
- **The Edge descriptor bug** (§5) is the closest documented analogue: newer Sony pad hardware +
  changed BT HID descriptor + host that can't parse it = session failure.

## 7. What this means for the connection-layer strategy (issue #7)

- **The stock `bluetooth2 hid/connect` path on this TV works for 2020–2022 DualSense stock
  only.** Grant-A and earlier pads connect end-to-end (proven on the TV). Grant-B pads (2023+
  production — BDM-040/050) fail inside LG's stack at HID session establishment. The CFI-ZCT2W
  line (2025+) carries a different FCC ID again and is untested — assume failing until proven
  otherwise.
- **Retail reality:** in 2026, essentially all new DualSense stock on shelves is grant-B-or-later.
  A v2 that relies solely on the stock connect path supports only used/older pads. That is a
  product decision the strategy grilling must make explicitly.
- **An app-owned HID session (raw L2CAP + uhid) is revision-agnostic by construction:** the
  kernel config on the TV supports `CONFIG_UHID=y` and `hid-generic` (per the diagnosis); the
  DualSense BT input report format (report ID 0x31, 78 bytes, CRC32-checked) is stable across
  generations for *input*; version differences matter mainly for *output/feature* reports, which
  the app can gate on the pad's reported update version exactly as Sony's own driver does.
- The rev-B stance (support via app-owned session vs stock-path-only vs both) is the decision
  [Grilling: Lock the connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)
  exists to make; the ranked hypotheses above (H-B1 descriptor/session-material drift leading)
  are its input.

## Sources

Primary:

- FCC ULS equipment-authorization records and exhibits (via fccid.io mirror; document IDs are
  FCC EAS attachment IDs, also reachable at
  `https://apps.fcc.gov/eas/GetApplicationAttachment.html?id=<id>`):
  - AK8CFIZCT1A: grant page <https://fccid.io/AK8CFIZCT1A>; UL Japan test report 13698456H-A-R1
    (exhibit 5151488, issued 2021-03-05); user manual (5151499); internal photos (5151498).
  - AK8CFIZCT1B: grant page <https://fccid.io/AK8CFIZCT1B>; UL Japan test report 14617231H-A
    (exhibit 6414722, issued 2023-02-16); antenna AUT report (exhibit 6414720 — PWB printed IFA,
    3.8 dBi, measured 2022-12); label exhibit (6414723); user manual (6414749); internal photos
    (6414748).
  - AK8CFIZCT1 (launch): grant page <https://fccid.io/AK8CFIZCT1>; test report (exhibit 4691261;
    antenna IFA 2.7 dBi, test FW 0.1.85).
  - AK8CFIZCT2W: grant page <https://fccid.io/AK8CFIZCT2W> (latest application 2025-06-08).
- IEEE OUI registry for A0:FA:9C (Weifang Goertek Electronics Co., Ltd) — via Wireshark `manuf`
  database (<https://www.wireshark.org/download/automated/data/manuf>) and macvendors.com.
- Linux kernel hid-playstation driver and its git history
  (<https://git.kernel.org/…/drivers/hid/hid-playstation.c>): update-version/feature-version
  machinery; commit 9fecab247e "support updated DualSense rumble mode" (feature version 2.21,
  2022) and its lore thread
  (<https://lore.kernel.org/all/20221010212313.78275-4-roderick.colenbrander@sony.com/>);
  commit 0b25b55d34 "report DualSense hardware and firmware version".
- linux-input mailing list: DualSense Edge BT descriptor bug report, May 2026
  (<https://lore.kernel.org/all/CA+s8=1JtqpxoVzaSht0xR-tSLy-MRedEef3uWFwzuPMpGhvQrg@mail.gmail.com/>).

Community (real, corroborated across independent channels):

- How To X (YouTube): "DualSense Revision Comparison: BDM-010 vs 020 vs 030 vs 040" (May 24,
  2024, <https://www.youtube.com/watch?v=M-WR2Ans1so>); "How To Tell What Generation Of DualSense
  You Have" (Nov 28, 2024, <https://www.youtube.com/watch?v=AyTx6H9OzP8>); "The 5th Revision of
  the DualSense Has Arrived | BDM-050 vs 040" (<https://www.youtube.com/watch?v=QJmtdcK-KqI>);
  "The 6th Revision … BDM-060 vs 050" (<https://www.youtube.com/watch?v=LZw_OegD3PE>).
- La mazmorra de Isaac (YouTube): "HOW TO DISTINGUISH THE DUALSENSE MODELS // BDM-010, BDM-020
  AND BDM-030" (Nov 20, 2022, <https://www.youtube.com/watch?v=WoRnfagmP7E>).
- hbashton/DS4Windows#122 (Sept 2026): Goertek-OUI DualSense `A0:FA:9C:1D:42:01` connecting
  normally over Bluetooth on Windows 11.
- mariotaku/moonlight-tv#576: DualSense/DualShock connecting natively over Bluetooth on an LG
  webOS 6.5.3 TV.

Not found / residual gaps (honest ledger):

- No public report of a grant-B DualSense failing on another specific legacy stack was found —
  search engines were bot-blocked from this environment and Reddit archives were unreachable;
  worth one human-assisted search pass, though the PC-side btmon differential (TL;DR item 3)
  would settle the mechanism question more definitively.
- The exact Bluetooth silicon in each generation is visible only in the FCC internal-photo
  exhibits (image PDFs; not text-extractable in this session) and community teardown footage;
  the Goertek OUI is the strongest module-vendor evidence available in text form.
- The exact BDM number of the maintainer's three pads is unknown (requires opening the pads or
  the USB version read); the grant-based era mapping above does not depend on it.
