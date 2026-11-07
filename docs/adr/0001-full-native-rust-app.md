# Full native Rust app, punktfunk-style, over evolving the Svelte + Node stack

The inherited stack is a Svelte web app plus a Node.js webOS service (fork of Superd22/webos-gamepad, dormant since 2023). We decided to rebuild as a **full native Rust app** — the shape proven by punktfunk/client-webos (Skia-on-GL UI over SDL3, NDL for webOS 3.5–4.x, Homebrew Channel packaging) — rather than keep the web UI and port only the service. Reasons: the maintainer wants punktfunk's UI and UX, not just its look; a single native binary removes the fragile web-app/service bridge on a TV; and the Node service's root-elevated, polyfilled runtime is a liability on webOS 4.10's old Chrome. The existing stack is kept as reference and a possible stopgap. Validation gate: passed — the native-Rust feasibility research ([Research: Native Rust app feasibility on webOS 4.10 and punktfunk client-webos reuse](https://github.com/acheronian/webos-bluepad/issues/3)) confirmed viability on webOS 4.x, including a 4.10 board, with a reusable MIT/Apache-licensed toolchain and packaging skeleton.

## Considered Options

- Port only the service to Rust, restyle the Svelte app to imitate punktfunk's UI — rejected: two runtimes and a bridge to maintain forever.
- Full native Rust app — chosen.
