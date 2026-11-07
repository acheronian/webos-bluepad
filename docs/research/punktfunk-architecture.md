# Research: Native Rust app feasibility on webOS 4.10 and punktfunk/client-webos reuse

Ticket: acheronian/webos-bluepad#3 · Branch: `research/punktfunk-architecture`

Question: is the decided full-native-Rust shape viable on webOS 4.10, and what of
`punktfunk/client-webos` can be reused?

**Method.** Everything below was read from primary sources during this session: the
`punktfunk/client-webos` repository files (fetched via `gh api repos/punktfunk/client-webos/contents/...`
at the default branch `main`), the upstream punktfunk monorepo at `git.unom.io/unom/punktfunk`
(fetched via its Gitea API), the repo's GitHub issues, and `mariotaku/ss4s` (the video-backend
library Moonlight-webOS uses). Each claim names the file it came from. Where a claim could not
be verified from a fetched primary source it is marked **unverified**.

---

## 1. What punktfunk/client-webos is

A native LG webOS TV client for the punktfunk game-streaming host, written in Rust, packaged as a
homebrew `.ipk`. Per its README: targets webOS 5 and later (verified live on an LG CX webOS 5.6 and
LG G5 webOS 10), with **webOS 3.5–4.x served by the older NDL interface (H.264 SDR)**
([README.md](https://github.com/punktfunk/client-webos/blob/main/README.md)). The repo is
dual-licensed MIT/Apache-2.0 and is a host for punktfunk's *shared controller shell*: "a host for
Punktfunk's shared controller shell (Skia on GL over an SDL3 window), NDL DirectMedia hardware
video decode, and webOS packaging" (same README).

### 1.1 Layering

From [AGENTS.md](https://github.com/punktfunk/client-webos/blob/main/AGENTS.md) (titled
`CLAUDE.md` internally):

```
core (pure domain: Settings, caps, media traits) ← services (portable I/O) ←
session (streaming on punktfunk-core, no sdl3) and platform/webos (SDL3 + hardware boundary) ←
console (the shared shell's GL host and service) ← runtime (menu and stream loops)
```

- **The only UI is punktfunk's shared controller shell** (`pf-console-ui`). "Screens, rows and
  navigation live in the kit; this client adds none." ([AGENTS.md](https://github.com/punktfunk/client-webos/blob/main/AGENTS.md))
- `src/console/gl.rs` hosts the shell: creates an SDL3 `GLContext` on the app's window, builds a
  Skia `DirectContext` over GLES **through SDL's `gl_get_proc_address` resolver** ("the native
  assembler has no reason to find webOS's loader"), wraps the window framebuffer as a Skia
  `Surface`, and sets the Skia GPU cache to `pf_console_ui::MIN_GPU_CACHE_BYTES`
  ([src/console/gl.rs](https://github.com/punktfunk/client-webos/blob/main/src/console/gl.rs)).
- `src/main.rs` shows the top level: `core`, `logger`, `services`, plus Linux-gated `console`,
  `platform`, `session`, `runtime` modules
  ([src/main.rs](https://github.com/punktfunk/client-webos/blob/main/src/main.rs)).

### 1.2 The controller UI shell (pf-console-ui)

`pf-console-ui` lives in the **upstream monorepo**, not in client-webos: the client depends on it
as a pinned git dependency — `pf-console-ui = { git = "https://git.unom.io/unom/punktfunk", rev = "b58dc3cf...", features = ["gl"] }`
([Cargo.toml](https://github.com/punktfunk/client-webos/blob/main/Cargo.toml)). Upstream it is
described as "The Skia console UI … stats OSD, capture HUD, and … the gamepad library; renders on
the presenter's device" ([crates/client/pf-console-ui/Cargo.toml](https://git.unom.io/unom/punktfunk/src/branch/main/crates/client/pf-console-ui/Cargo.toml)).
Its `src/lib.rs` states the design: "one shell — home, library, settings, add-host, pairing —
drawn onto whatever `skia_safe::Canvas` a host hands it", with hosts on Vulkan (desktop) and
Android GL; "Everything but `skia_overlay.rs` is platform-free: screens draw to `&Canvas`,
settings through `store::SettingsStore`, keys as `input::Key`, platform rows as `platform::Platform`"
([crates/client/pf-console-ui/src/lib.rs](https://git.unom.io/unom/punktfunk/src/branch/main/crates/client/pf-console-ui/src/lib.rs)).

The `gl` feature exists specifically for this webOS client: "Skia's GL backend for a Linux
consumer that has no Vulkan presenter and no SDL3 — a TV app drawing on an EGL surface it owns
(design/webos-skia-console-port.md §2.1)" (same Cargo.toml). Note: that referenced design doc
`design/webos-skia-console-port.md` was **not found** at any path I tried in the upstream repo
(404 on `design/…`, `docs/…`, and under the crate); the upstream repo has no top-level `design/`
directory (verified via the Gitea contents API). Unverified where it lives.

The drawing kit (`theme`, `widgets`, `icons`, `glyphs`, `anim`, `pointer`, mark tables) is `pub`
"for one consumer: the webOS pointer UI … No stability promise; a kit change there is a re-pin
plus a compile fix, by design" (same `src/lib.rs`).

### 1.3 Video: NDL DirectMedia, two generations

From [src/platform/webos/ndl/mod.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/ndl/mod.rs)
and [docs/NOTES.md](https://github.com/punktfunk/client-webos/blob/main/docs/NOTES.md):

- **v2** (webOS 5+): `NDL_DirectMediaLoad` + `NDL_DirectVideoPlay(buffer, size, pts)`, render-buffer
  query, flush, HDR metadata, Opus audio plane.
- **v1** (webOS 3.5–4.x): `NDL_DirectVideoOpen/SetCallback/SetArea/PlayWithCallback/Close` —
  "H.264 only, SDR, and **no PTS input**", no buffer query, no flush, no HDR call; fixed
  1920×1080 display rect via `SetArea`; audio stays software Opus → SDL because v1's
  `NDL_DirectAudio*` is PCM/AAC/AC3 with no Opus
  ([src/platform/webos/ndl/v1.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/ndl/v1.rs)).
- Generation is picked from Luna `getSystemInfo`'s `sdkVersion` (major < 5 → v1), with `dlsym` as
  the final authority ([src/platform/webos/device.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/device.rs)).
- **NDL is `dlopen`'d, never linked**: the binary links with `DT_BIND_NOW`, so a `DT_NEEDED` on
  `libNDL_directmedia.so.1` makes the loader resolve v2 symbols at exec time and "refuse to start
  the process at all on webOS 4 — before `main()`, with nothing logged" (ndl/mod.rs; also AGENTS.md
  "Invariants"). This was a real bug: the app could not start on webOS 4 until NDL was dlopen'd
  ([issue #104](https://github.com/punktfunk/client-webos/issues/104)).
- An SMP (Starfish Media Pipeline) backend for 3.5–4.x was built and then **removed**: "SMP was
  never verified on real 3.5–4.x hardware, yet cost a bundled C++ shim `.so`, a version-specific
  ACB sink … webOS 3.5–4.x TVs get NDL v1 (H.264/SDR) instead of an unverified SMP path"
  ([issue #167](https://github.com/punktfunk/client-webos/issues/167)).

### 1.4 Cross-compilation toolchain

From [.cargo/config.toml](https://github.com/punktfunk/client-webos/blob/main/.cargo/config.toml),
[taskfiles/toolchain.yml](https://github.com/punktfunk/client-webos/blob/main/taskfiles/toolchain.yml),
[scripts/cc-shim.sh](https://github.com/punktfunk/client-webos/blob/main/scripts/cc-shim.sh),
[build.rs](https://github.com/punktfunk/client-webos/blob/main/build.rs) and docs/NOTES.md:

- Target: **`armv7-unknown-linux-gnueabi`** (tier-2), webosbrew `native-toolchain` buildroot SDK
  (`arm-webos-linux-gnueabi_sdk-buildroot`), fetched from
  `github.com/webosbrew/native-toolchain/releases` — **Linux-aarch64 host only** (CI runs on
  `ubuntu-24.04-arm`; local dev wraps everything in Docker/QEMU; [.github/workflows/build.yml](https://github.com/punktfunk/client-webos/blob/main/.github/workflows/build.yml)).
- Linker/CC/CXX wired to `scripts/cc-shim.sh` / `cxx-shim.sh` because the SDK's baked-in sysroot
  path breaks post-relocate, so every invocation passes `--sysroot` explicitly (cc-shim.sh header).
- **Soft-float fix** (single biggest perf fix, ~300 ms → ~30 ms per render): the non-`hf` target
  disables hardware FP codegen; fixed with `target-feature=+neon,+vfp3,-soft-float` and
  `target-cpu=cortex-a73` — "binary still runs on cortex-a53" (.cargo/config.toml;
  docs/NOTES.md § Toolchain).
- **glibc compat shim** (`src/platform/webos/glibc_compat_shim.c`): webOS glibc ~2.12 lacks
  `getauxval`/`gettid`/`sendmmsg`; compiled by build.rs and placed **after** libstdc++ in link
  order via `cargo:rustc-link-arg` (build.rs; docs/NOTES.md).
- **getrandom `linux_raw` backend**: webOS 4.x's old glibc resolves `getrandom(2)` through
  `dlsym` (finds nothing) then falls back to `/dev/urandom`, "which the app sandbox does not
  expose — so every RNG user (rand, quinn) panicked" (.cargo/config.toml comment;
  [issue #114](https://github.com/punktfunk/client-webos/issues/114)). `linux_raw` issues the
  syscall by inline asm. webOS 4.5 is kernel 4.9, so the syscall exists (#114).
- **libstdc++ linked statically, never bundled** — a bundled `libstdc++.so.6` via `DT_RPATH`
  outranks the jail's `LD_LIBRARY_PATH` and breaks the TV's own libraries (cc-shim.sh;
  docs/NOTES.md).
- **SDL3 is the webosbrew fork** (`webosbrew/SDL-webOS`, release `3.4.16-webos.5`), fetched to its
  own prefix, linked by path from build.rs, bundled as `lib/libSDL3.so.0` with
  `$ORIGIN/../lib` rpath — "webOS ships no SDL3 at all" (taskfiles/toolchain.yml; build.rs;
  docs/NOTES.md).
- **FreeType bundled** in the ipk because "Skia calls entry points the on-device copy lacks below
  webOS 9" (build.rs `BUNDLED_COMPONENTS`; taskfiles/toolchain.yml `stage-and-package`).
- **Skia**: `skia-safe 0.153` with `gl`+`textlayout`; rust-skia publishes **no armv7 archive** for
  the `gl` key, so punktfunk hosts its own prebuilt at
  `git.unom.io/unom/skia-binaries` ("Prebuilt Skia archives for punktfunk's console UI … the
  armv7-linux-androideabi key rust-skia doesn't publish" — repo description via Gitea API;
  `SKIA_BINARIES_URL` in [Taskfile.yml](https://github.com/punktfunk/client-webos/blob/main/Taskfile.yml)).
- Release profile: fat LTO, one codegen unit, `panic = "abort"` ([Cargo.toml](https://github.com/punktfunk/client-webos/blob/main/Cargo.toml)).

### 1.5 Packaging & distribution

- `packaging/appinfo.json`: `"type": "native"`, `main: bin/punktfunk-webos`, app id
  `io.unom.punktfunk.client-webos` ([packaging/appinfo.json](https://github.com/punktfunk/client-webos/blob/main/packaging/appinfo.json)).
- Staging: binary + `libSDL3.so.0` + `libfreetype.so.6` into `bin/`+`lib/`, stripped with the NDK
  strip, then `ares-package "$STAGE_DIR" -o dist --force-arch arm` (webosbrew's Rust
  `ares-cli-rs`) (taskfiles/toolchain.yml `stage-and-package`).
- CI ([.github/workflows/webosbrew.yml](https://github.com/punktfunk/client-webos/blob/main/.github/workflows/webosbrew.yml))
  verifies the ipk against firmware symbol data with webosbrew's `dev-toolbox-cli`:
  `webosbrew-ipk-verify dist/*.ipk --fw-releases '>=4'` — i.e. the shipped binary is CI-checked
  against **webOS 4-and-later firmware symbols**. It then generates a Homebrew manifest
  (`webosbrew-gen-manifest --root false`), validates it against the webosbrew schema, and
  attaches ipk+manifest to a rolling `beta` release.
- Distribution: a [repo.json](https://github.com/punktfunk/client-webos/blob/main/repo.json)
  pointing at the beta release's manifest — "That repository serves a build of every commit on
  `main`" (README.md). Tagged releases reach the central webosbrew repo instead (README.md).
- Deploy to a TV: `task deploy TV_HOST=root@<tv-ip>` via ares-cli-rs install/launch (README.md;
  Taskfile.yml).

---

## 2. Feasibility verdict for webOS 4.10

**Verdict: viable, with one device-specific blocker and a known ceiling.** The punktfunk client —
which is exactly the "full-native-Rust on webOS" shape webos-bluepad v2 decided on (Skia-on-GL UI
over SDL3, Rust armv7 cross-build, homebrew .ipk) — demonstrably runs on webOS 4.x hardware,
including a 4.10 set, once one jailer quirk is fixed. For webos-bluepad the risk profile is *better*
than for punktfunk, because bluepad is a gamepad manager, not a streaming client: it needs no
hardware video decode at all, so the entire NDL/SMP/escreen risk area falls away.

### 2.1 Direct hardware evidence (webOS 4.x, including 4.10)

- webOS 4.9: "I've tested last version on my webOS 4.9.0 and it rocks solid!"
  ([issue #174 comment](https://github.com/punktfunk/client-webos/issues/174)). Earlier testing on
  the same TV: "FHD/QHD/4K @30/60/120 OK", "Only NDL works (HDR not available)" (#174 comments).
- **webOS 4.10 (machine `k5lp`)**: after a dependency fix, streaming failed with
  `ndlv1 load: this tv jailer config leaves dev/rtkmem unreadable (machine k5lp) so ndlv1 cannot
  reach the decoder`; the reporter fixed it "by copying manually the jail files in the punktfunk
  folder, same fix as the one documented for moonlight & kodi. now it seems to work perfectly."
  ([issue #274 comments](https://github.com/punktfunk/client-webos/issues/274)).
- The client now detects this itself: `device::jail_config_broken()` checks machines
  `k5lp`/`k3lp` for an unreadable `/dev/rtkmem` and refuses with a named error instead of a black
  screen ([src/platform/webos/device.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/device.rs)).
  The same check exists in Moonlight's video library: "Only k3lp/k5lp needs to be checked …
  Make sure /dev/rtkmem is readable, otherwise we have a broken jailer config"
  ([mariotaku/ss4s `modules/webos/utils/jail_check.c`](https://github.com/mariotaku/ss4s/blob/main/modules/webos/utils/jail_check.c)).

**Risk (blocker-class for affected boards):** if the maintainer's webOS 4.10 TV is a `k5lp`/`k3lp`
machine, the out-of-the-box jailer config blocks direct-media access and requires the manual
jail-file fix (documented for moonlight/kodi; the map issue notes root SSH is available, so this
is fixable, but it must be part of the install story). Which machine the maintainer's TV is, is
**not determinable from these sources** — `k5lp` is only known to be *a* webOS 4.10 board (issue #274).

### 2.2 NDL vs the legacy (escreen/LGNC) route

- On webOS 4.x, punktfunk uses **NDL v1**: H.264 only, SDR/BT.709 only, no PTS input, no HDR, no
  audio plane (v1.rs; docs/NOTES.md "v1 limits"). `core::caps::VideoCaps::H264_SDR` caps the whole
  app accordingly — stereo audio, single codec row
  ([src/core/caps.rs](https://github.com/punktfunk/client-webos/blob/main/src/core/caps.rs)).
- The alternative on 4.x is Moonlight's "legacy (lgnc)" route (the only client the 4.10 reporter
  had working before punktfunk's fix — issue #274). In `mariotaku/ss4s` this is the `lgnc` module:
  H.264 only, max 40 Mbps, max 60 fps, BT.709, "UI compositing" transform
  ([modules/webos/lgnc/lgnc_video.c](https://github.com/mariotaku/ss4s/blob/main/modules/webos/lgnc/lgnc_video.c)).
  ss4s also carries an `m3_kadp_fix` and the same k3lp/k5lp jail check in that module
  ([modules/webos/lgnc/lgnc_module.c](https://github.com/mariotaku/ss4s/blob/main/modules/webos/lgnc/lgnc_module.c)).
- **Terminology note:** the ticket's phrase "LSM escreen" does not appear in any primary source I
  fetched. The primary sources name the legacy surface **LGNC** (`liblgnc`, `LGNC_DIRECTVIDEO_*`)
  and the modern one **NDL DirectMedia**; ss4s additionally has an SMP module gated
  `OS_VERSION ">=5"` ([modules/webos/smp/CMakeLists.txt](https://github.com/mariotaku/ss4s/blob/main/modules/webos/smp/CMakeLists.txt)).
  If "LSM escreen" refers to the LGNC/escreen legacy video API, the above is the comparison;
  otherwise the term is **unverified**.
- **Relevance to bluepad:** none of this is bluepad's problem. Bluepad delivers input *into*
  streaming apps; the video path belongs to punktfunk/Moonlight, which already work on the TV
  (map issue: "Streaming apps already stream fine on the TV; only controller input is broken").
  The one transferable lesson is punktfunk's discipline: dlopen vendor libs, detect rather than
  assume, refuse with named errors (ndl/mod.rs, device.rs).

### 2.3 Does the Skia/SDL3 UI stack run on 4.10?

Yes — with the bundling punktfunk already does. Evidence and requirements:

- The whole app (shell included) is what the webOS 4.9/4.10 reporters above ran; "it rocks solid"
  / "works perfectly" are reports of the Skia-UI app, not just decode (#174, #274).
- Requirements the client already satisfies for old firmware: bundle SDL3 (webOS ships none),
  bundle FreeType (on-device copy lacks entry points below webOS 9), static libstdc++, glibc
  shims, `getrandom linux_raw` (§1.4). CI's `webosbrew-ipk-verify --fw-releases '>=4'` checks the
  binary's dynamic symbols against webOS-4-era firmware (§1.5).
- **CPU/GPU constraints of 4.x-era TVs.** The codegen tunes for `cortex-a73` (the CX's α9-gen3)
  but "binary still runs on cortex-a53" (.cargo/config.toml). The shell's frame cost is treated as
  a first-class concern — `src/core/perf.rs` exists to log per-frame CPU time and cover-art decode
  cost ("'it lags a bit on my CX' is the only signal a 2020 set has ever produced",
  [src/core/perf.rs](https://github.com/punktfunk/client-webos/blob/main/src/core/perf.rs)) — and
  the shell gained a **"Reduce interface resolution"** mode, **defaulted on for webOS**, which on
  a G5 cut Home-navigation GPU busy from 37% to 16% by dropping live blur and rendering the
  backdrop through a ≤512px offscreen buffer
  ([issue #271](https://github.com/punktfunk/client-webos/issues/271)). A 4.x-era SoC is weaker
  than the G5/CX these numbers come from; bluepad's UI (a pairing/list/settings app, no cover-art
  carousel, no video) sits far below this envelope, but the reduce-resolution knob and the
  "measure on device, don't theorize" rule (AGENTS.md: "code theories about this hardware are
  usually wrong") should carry over.
- **Memory.** The Skia GPU cache is set to the kit's `MIN_GPU_CACHE_BYTES` explicitly because "the
  G5's GPU memory is the app's own RAM, and the stream's decoder needs what is left"
  (src/console/gl.rs); #271 raised it to a 96 MB shared floor. 4.x-era sets have less RAM headroom
  (no specific figure is given in any source I fetched — **unverified**); bluepad's UI budget is
  small regardless.

### 2.4 Input/gamepad-relevant platform facts on 4.x

- webOS 4.x era: **no `hid-playstation` kernel driver** — LG ships it only on webOS 10+, so a
  DualSense works as plain input but has no haptics/triggers/lightbar/speakers (README.md note;
  matches the map's out-of-scope decision).
- The client's input stack reads `/dev/input/event*` directly (evdev bypass of the compositor),
  with per-node `EVIOCGRAB`, and uses SDL3 for gamepad events
  ([src/platform/webos/evdev/mod.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/evdev/mod.rs),
  src/platform/webos/gamepad.rs). evdev nodes are `root:compositor 0660` and the app uid carries
  the compositor gid (evdev/mod.rs "Access" note — verified by them on webOS 10.3; **not
  independently verified on 4.10**).
- Luna/LS2 access: in-process `LSRegister` works **only from the app's own binary path** ("the hub
  keys permissions on the executable"), with `luna-send-pub` subprocess fallback; `bluetooth2`
  methods reply in 1–3 ms in-process
  ([src/platform/webos/ls2.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/ls2.rs),
  [src/platform/webos/luna.rs](https://github.com/punktfunk/client-webos/blob/main/src/platform/webos/luna.rs)).
  These were verified on webOS 5.6/10.3 hardware (docs/NOTES.md); behavior on 4.10 is **likely
  similar but unverified** — bluepad's whole domain is the `bluetooth2` API, so this is the first
  thing to probe on the actual TV.
- webOS-version quirks the client encodes per version: webOS 9+ cursor-hide behavior, webOS 23+
  pads typing as remote keys, webOS 24+ never hiding the cursor (docs/NOTES.md "Known platform
  limitations"). None of these are 4.x-specific; the 4.x-specific items in the codebase are the
  NDL v1 path, the glibc/getrandom shims, and the `k5lp`/`k3lp` jail check.

### 2.5 Risk summary for webOS 4.10

| # | Risk | Severity | Mitigation (proven in client-webos) |
|---|------|----------|-------------------------------------|
| 1 | `k5lp`/`k3lp` jailer config blocks device access (`/dev/rtkmem` unreadable) | Blocker on affected boards | Detect + named error (device.rs); manual jail-file fix documented for moonlight/kodi (#274); root SSH available per map issue |
| 2 | Old glibc (~2.12): missing `getauxval`/`gettid`/`sendmmsg`; no `getrandom` symbol; no `/dev/urandom` in sandbox | Certain on 4.x, solved | glibc shim C file + link order (build.rs); `getrandom_backend="linux_raw"` (.cargo/config.toml; #114) |
| 3 | `DT_BIND_NOW` + `DT_NEEDED` on vendor libs kills startup pre-`main` | Certain if violated | dlopen all vendor libs (ndl/mod.rs; #104) |
| 4 | No SDL3 on device; on-device FreeType too old for Skia below webOS 9 | Certain, solved | Bundle both in ipk lib/ with `$ORIGIN` rpath (toolchain.yml, build.rs) |
| 5 | Weaker 4.x-era CPU/GPU for the UI | Moderate | cortex-a53-compatible codegen; reduce-UI-resolution pattern; measure on device (perf.rs, #271) |
| 6 | `bluetooth2`/LS2 permission model on 4.10 unverified for our use | Unknown | ls2.rs/luna.rs two-route pattern (in-process + `luna-send-pub`); probe first |
| 7 | DualSense advanced features impossible (kernel driver only webOS 10+) | Accepted | Already out of scope in the map |

---

## 3. Reuse inventory

Legend: **take** = copy/wire directly (license permits); **adapt** = reuse with modification;
**imitate** = pattern only; **n/a** = not applicable to bluepad's domain.

### Take directly

| Asset | Source | Notes |
|---|---|---|
| Cross-toolchain skeleton: `.cargo/config.toml` (target, softfp flags, `linux_raw`), `scripts/cc-shim.sh`/`cxx-shim.sh`, `taskfiles/toolchain.yml` (NDK/SDL3 fetch, staging, `ares-package --force-arch arm`) | client-webos root | This is the entire "Rust on webOS 4.x" problem, already solved and CI-proven against `--fw-releases '>=4'` (webosbrew.yml) |
| `src/platform/webos/glibc_compat_shim.c` + its build.rs wiring | client-webos | Needed for any Rust binary on 4.x-era glibc |
| `device.rs` detection pattern: `sdkVersion`/`otaId`/`machineName` via Luna `getSystemInfo`, `jail_config_broken()` | client-webos | Bluepad needs machine-name quirk handling too |
| Packaging: `appinfo.json` shape (`"type": "native"`), staging layout (`bin/`, `lib/`, `$ORIGIN` rpath), ares-cli-rs usage | client-webos | |
| Distribution pipeline: `webosbrew.yml` workflow (ipk-verify → `webosbrew-gen-manifest --root false` → schema validation → beta release), `repo.json` | client-webos | The map already names this as the pattern for Homebrew Channel publishing |
| `ls2.rs` + `luna.rs` (in-process LS2 with dlopen + `luna-send-pub` fallback, call deadlines) | client-webos | Bluepad's core domain is `bluetooth2` over LS2; this is the fastest proven route (1–3 ms/call) |
| evdev reader skeleton (`evdev/mod.rs`: node probing, `EVIOCGRAB` scoping, `SYN_DROPPED` handling) | client-webos | Bluepad must read/relay pad nodes; the same discipline applies if it must keep the compositor quiet |

### Adapt

| Asset | Source | Notes |
|---|---|---|
| `console/gl.rs` GL-host pattern (SDL3 GL context → Skia DirectContext via SDL's proc-address resolver, surface re-wrap on resize, vsync/immediate swap policy) | client-webos | If bluepad's UI is Skia-on-GL (the decided shape), this is the working webOS recipe; swap in our own screens |
| Skia armv7 prebuilt (`SKIA_BINARIES_URL` → `git.unom.io/unom/skia-binaries`) | Taskfile.yml + unom/skia-binaries | rust-skia publishes no armv7 `gl` archive; either reuse punktfunk's mirror (check its license terms — **unverified**, see §4) or build our own |
| `logger` (ring buffer, launch/telemetry, on-device files) and `pfprobe` diagnostics-bin pattern | client-webos tree | Debugging on a TV without a PTY is the documented pain (docs/NOTES.md "Luna replies unreachable from ssh jail") |
| `build.rs` third-party-notices generation + bundled-component licensing discipline | client-webos | Homebrew distribution hygiene; NDL headers copied from mariotaku/ss4s (build.rs note) |

### Imitate (pattern, not code)

| Pattern | Source | Notes |
|---|---|---|
| The controller shell concept: gamepad-driven full-screen Skia UI, one settings document, platform rows via a trait | pf-console-ui | The kit's *screens* are streaming-specific (hosts, library, pairing to a punktfunk host); bluepad needs pairing-to-TV screens instead. The kit's `Platform` trait + `ConsoleOptions` shape is the pattern to copy |
| "Detect, don't assume; refuse with named errors" (NDL generations, optional symbols, jail check) | ndl/mod.rs, device.rs | |
| "Measure on device; code theories about this hardware are usually wrong" | AGENTS.md / docs/NOTES.md | |

### Not applicable / must not take

| Asset | Why |
|---|---|
| `punktfunk-core`, `pf-client-core` session/streaming stack | Bluepad doesn't stream; also pulls QUIC/crypto/codec weight irrelevant to a pad manager |
| NDL video pipeline (`ndl/`), SMP remnants, HDR calibration, ABR probing | Not bluepad's domain (§2.2) |
| DualSense feedback paths (`dualsense.rs`, `pad_link.rs`, `hidraw.rs`) | Depend on webOS 10+ `hid-playstation` / `bluetooth2` internals verified on newer sets; 4.10 lacks the kernel driver (README.md). The *bluetooth2 sniff-mode findings* (docs/NOTES.md) are webOS-10-era measurements and explicitly not transferable assumptions for 4.10 |

---

## 4. Licensing

- **client-webos**: dual-licensed **MIT OR Apache-2.0** — `license = "MIT OR Apache-2.0"` in
  [Cargo.toml](https://github.com/punktfunk/client-webos/blob/main/Cargo.toml); full texts in
  [LICENSE-MIT](https://github.com/punktfunk/client-webos/blob/main/LICENSE-MIT) ("Copyright (c)
  2026 dyptan-io") and [LICENSE-APACHE](https://github.com/punktfunk/client-webos/blob/main/LICENSE-APACHE)
  (standard Apache 2.0; both fetched and read). README: "Dual-licensed under MIT or Apache 2.0,
  matching upstream Punktfunk, at your option."
- **pf-console-ui (the controller UI shell)**: inherits the upstream workspace license —
  `license.workspace = true` in its Cargo.toml, and the upstream workspace root declares
  `license = "MIT OR Apache-2.0"`
  ([unom/punktfunk Cargo.toml §workspace.package](https://git.unom.io/unom/punktfunk/src/branch/main/Cargo.toml));
  upstream [LICENSE-MIT](https://git.unom.io/unom/punktfunk/src/branch/main/LICENSE-MIT) reads
  "Copyright (c) 2026 unom - Enrico Bühler", and a LICENSE-APACHE exists at the repo root
  (verified via the Gitea contents API).
- **Conclusion: the shell can be reused, not only imitated.** Both the webOS client and the shell
  are permissively dual-licensed (MIT or Apache-2.0, at the reuser's option). Nothing in the
  fetched sources restricts reuse — no CLA/CONTRIBUTING restrictions were seen in client-webos
  (upstream has a CONTRIBUTING.md and SECURITY.md that I did not read — **unverified** whether
  they add terms; Apache-2.0 §5 contribution terms would in any case grant the same license).
  Practical caveats:
  - The shell embeds **Punktfunk branding** (Geist font, brand palette, launcher marks; build.rs
    `BUNDLED_COMPONENTS` lists Geist under SIL-OFL-1.1 and Lucide under ISC). Reusing the shell
    as-is ships punktfunk's look; trademark/brand questions around that are **unverified** — no
    trademark policy was found in the fetched sources.
  - The kit's public drawing modules carry "No stability promise" (pf-console-ui src/lib.rs) —
    reuse means pinning a rev and accepting re-pin compile fixes, exactly as client-webos does
    (one pinned rev across `punktfunk-core`/`pf-client-core`/`pf-console-ui`, per Cargo.toml).
  - The **skia-binaries mirror** (`git.unom.io/unom/skia-binaries`) has no license statement in
    anything I fetched — **unverified**; the safe path is asking upstream or building our own
    armv7 Skia archive.
  - Bundled third-party components' licenses are catalogued in build.rs `BUNDLED_COMPONENTS`:
    SDL3 (webosbrew/SDL-webOS) Zlib; FreeType FTL; libstdc++ GPL-3.0 **with the GCC Runtime
    Library Exception** (statically linked); NDL DirectMedia is a system library, dlopen'd at
    runtime and **not redistributed** ("Header signatures were taken from mariotaku/ss4s").

---

## 5. Native Rust vs keeping a web UI — pro/con

Context from this repo: the current webos-bluepad is a **Svelte web app + Node.js
`webos-service`** (fork of Superd22/webos-gamepad; `package.json`, `app/`, `services/`), and its
own docs.md already lists the pain: "webos-service only for nodejs (and its api sucks)",
"webosTV is in window for web apps (and its API sucks)", Chrome remote debugging only works
same-version, services can't be debugged, and services don't run as root in dev mode without the
hbchannel elevate wrapper. The map issue records the decided direction: full native Rust,
punktfunk-style, with the existing stack as "reference material and a possible stopgap."

### Pro (native Rust, punktfunk shape)

1. **Proven on the exact target.** The identical shape runs on webOS 4.9/4.10 hardware today
   (#174, #274), with a CI gate (`webosbrew-ipk-verify --fw-releases '>=4'`) that checks binary
   compatibility against webOS-4-era firmware symbols (webosbrew.yml). A web app's runtime on
   4.10 is an old, fixed Chromium that the project already fights (docs.md).
2. **One artifact, one process.** A single native binary + bundled `.so`s in one `.ipk`
   (`"type": "native"` appinfo.json). No app+service split, no Node runtime on the TV, no
   IPC between a sandboxed web app and a root-hungry service. The current stack's core problem —
   input plumbing across the app/service boundary — disappears when everything is one process.
3. **Direct platform access.** In-process LS2 (`LSRegister` from the app binary, 1–3 ms
   `bluetooth2` calls; ls2.rs), raw evdev with `EVIOCGRAB` (evdev/mod.rs), Luna calls with
   deadlines (luna.rs). A web app cannot do any of this; a Node service can only via
   `luna-send`-style indirection, which the punktfunk notes show is the *slow fallback*, not the
   fast path.
4. **Performance and predictability.** Fat LTO, no GC, no JIT warm-up on a weak 4.x-era CPU;
   the soft-float and codegen flags are already solved (.cargo/config.toml). The UI cost is
   measured, not guessed (perf.rs).
5. **Toolchain reuse.** Everything in §3 "Take directly" — toolchain, shims, packaging, CI,
   Homebrew distribution — is MIT/Apache-licensed and copyable, cutting the largest risk of the
   v2 plan.
6. **Distribution parity with the ecosystem.** Native `.ipk` + `repo.json` is exactly what the
   Homebrew Channel consumes; the map's publishing plan (punktfunk's webosbrew.yml as pattern)
   presumes this shape.

### Con (native Rust)

1. **UI development velocity.** HTML/CSS/Svelte iteration (hot reload, DevTools) is faster than
   Skia widget code; the punktfunk client mitigates with a VNC container preview
   (`task docker:deploy`, scripts/preview.sh) but that is still compile-then-look. Bluepad's UI
   (pairing flows, device lists, settings) is modest, which bounds this cost.
2. **The UI must be built.** pf-console-ui's screens are streaming-specific; bluepad can reuse the
   kit's drawing modules and the gl.rs host, but its screens are new work (§3 "Adapt"/"Imitate").
   With a web UI, the existing Svelte screens carry over.
3. **armv7-specific friction.** No rust-skia armv7 prebuilt (punktfunk maintains its own mirror),
   Linux-aarch64-only toolchain (Docker/QEMU on other hosts), glibc/getrandom shims to keep
   correct (§1.4). All solved once, but they are ongoing constraints (e.g., every Skia bump needs
   a new armv7 archive — Taskfile.yml `SKIA_BINARIES_URL`).
4. **Smaller ecosystem/team familiarity.** Node/web tooling is what the current repo knows; Rust
   on webOS has punktfunk and moonlight-adjacent projects but a thin community. Debugging on-device
   is SSH+logs (punktfunk built a telemetry listener and a pfprobe CLI for exactly this reason —
   logger/, src/bin/pfprobe.rs).
5. **A web UI is not actually free on 4.10.** The counter-argument cuts the other way: the
   current stack needs a Node service anyway (web apps can't touch LS2/evdev), the 4.10 web
   runtime is an old fixed Chromium (docs.md's polyfill/debugging complaints), and the
   app↔service split is where the current input bugs live ("Gamepads sometimes work sometimes
   don't", README of this repo).

### Bottom line

The native-Rust decision is sound for webOS 4.10: the shape is proven on that exact firmware by
punktfunk/client-webos, the hardest infrastructure (toolchain, shims, packaging, CI, LS2 access)
is permissively licensed and directly reusable, and the alternative (web UI) still requires a
native service for everything bluepad actually does — pairing and input — while adding an old
Chromium and an IPC boundary to the critical path. The Svelte app remains useful as a stopgap and
as reference material for flows, per the map. The one open feasibility item no research can close
is the maintainer's specific 4.10 board (k5lp/k3lp jail quirk) and the `bluetooth2` permission
surface on 4.10 — both need a probe on the actual TV, and the punktfunk repo's probe-first
discipline (device.rs startup log line, pfprobe) is the template for doing that.

---

## Source index (all fetched this session)

- punktfunk/client-webos@main via GitHub API: README.md, Cargo.toml, build.rs, .cargo/config.toml,
  Taskfile.yml, taskfiles/toolchain.yml, AGENTS.md, docs/NOTES.md, repo.json, .env.example,
  LICENSE-MIT, LICENSE-APACHE, packaging/{appinfo.json, description.md},
  scripts/{cc-shim.sh, cxx-shim.sh, host.sh, preview.sh},
  .github/workflows/{build.yml, webosbrew.yml}, src/main.rs, src/core/{mod,caps,perf}.rs,
  src/console/{mod,gl}.rs, src/platform/webos/{mod,device,luna,ls2,gamepad,sdl_webos}.rs,
  src/platform/webos/ndl/{mod,v1}.rs, src/platform/webos/evdev/{mod,pad}.rs, src/services/mod.rs
- git.unom.io/unom/punktfunk via Gitea API: root Cargo.toml (workspace.package), README.md,
  LICENSE-MIT, LICENSE-APACHE (existence), crates/client/pf-console-ui/{Cargo.toml, src/lib.rs},
  web/README.md; repo metadata and contents listings
- git.unom.io/unom/skia-binaries: repo metadata + release asset listing
- punktfunk/client-webos issues (via gh api): #88, #104, #112 (title), #114, #167, #174 (+comments),
  #177, #271, #274 (+comments)
- mariotaku/ss4s@main: modules/webos/{CMakeLists.txt, utils/jail_check.c, lgnc/lgnc_module.c,
  lgnc/lgnc_common.h, lgnc/lgnc_video.c, smp/CMakeLists.txt, ndl/webos4/ndl_common.h}
- acheronian/webos-bluepad: map issue #1 (read-only), this repo's README.md, docs.md, package.json
