# Research: Rust design patterns (refactoring.guru) — applicability to the BluePad crate layout

Ticket: acheronian/webos-bluepad#23 · Branch: `research/rust-design-patterns`

**Question.** Which of the design patterns in the Rust editions of the
refactoring.guru articles are feasible and beneficial for the BluePad crate layout
being settled in [Grilling: App architecture](https://github.com/acheronian/webos-bluepad/issues/15)
(`bluepad-core` / `bluepad-radio` / `bluepad-daemon` / `bluepad-ui`; tokio daemon,
synchronous UI), and which would be over-engineering for a four-crate workspace of
this size?

**Method.** All 22 Rust-edition pages at
`https://refactoring.guru/design-patterns/<pattern>/rust/example` were fetched and read
in full during this session (article prose + complete example code; the pattern index
at `https://refactoring.guru/design-patterns/rust` lists exactly these 22 — the GoF
catalog minus Interpreter, which refactoring.guru does not cover). One exception:
**Flyweight**'s Rust edition is a paywalled stub ("Rendering a Forest" example sold
with the eBook; no code on the page), so it is assessed from its description text
only. The concrete mappings below assume the crate layout *proposed* in the
architecture ticket — that ticket is still open, so mapping targets are named by
seam, not by settled module path.

**Verdict legend.** **adopt** = use it, with the concrete home named; **consider** =
right idea, use the lightweight Rust form and only if the named trigger arrives;
**skip** = over-engineering or already built into the language at this scale.

---

## 1. The seams being assessed against

From the architecture ticket and the map's decisions so far, the layout has four
load-bearing seams:

1. **Radio-transport seam** — core defines transport/session traits; `bluepad-radio`
   provides the concrete paths: the stock `bluetooth2` LS2 route and the app-owned
   raw-L2CAP + `uhid` route, with per-device runtime routing (known path sticks, new
   pads try app-owned first — [Lock the connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)).
2. **Session supervision / recovery** — the daemon's crash-restart, backoff, and
   idempotent-reconcile machinery across pad sleep / TV standby / daemon restart.
3. **Privilege-tier capability gating** — privileged daemon vs jailed UI; the UI may
   only reach what the daemon's request surface exposes.
4. **UI↔daemon socket protocol** — subscribe/notify for state, request/response for
   commands, between a synchronous UI process and a tokio daemon.

A fifth, cross-cutting property shapes everything: **the daemon is async (tokio),
the UI is synchronous** — so any pattern crossing the process boundary is a
*serialization* boundary (serde messages), not a shared object graph.

## 2. Rust-specific guidance distilled from the articles

Three lessons recur across the Rust editions and bound how the classic patterns
should be applied here:

- **Static dispatch where the set is closed, `Box<dyn>` where selection is genuinely
  runtime.** The [Abstract Factory](https://refactoring.guru/design-patterns/abstract-factory/rust/example)
  page shows the same factory twice — as a generic trait with associated types
  (compile-time) and as `Box<dyn>` returns (runtime) — and the
  [Factory Method](https://refactoring.guru/design-patterns/factory-method/rust/example)
  page shows both forms too (Dialog vs MazeGame). BluePad's pad-type set is closed
  (DualSense, DS4, Xbox One S, generic HID), but *path* selection (stock vs
  app-owned, per-device, sticky) is runtime state → traits in core, `dyn` only at
  the routing point.
- **Ownership beats global state.** The [Singleton](https://refactoring.guru/design-patterns/singleton/rust/example)
  page's own "Rust specifics" section says the safe singleton is *no global at all*:
  "passing everything around through function arguments," with the oldest living
  variable created at the start of `main()`. That is exactly the daemon-shape the
  architecture ticket proposes — state lives in owned structs, not statics.
- **Top-down ownership for multi-component coordination.** The [Mediator](https://refactoring.guru/design-patterns/mediator/rust/example)
  page's "Top-Down Ownership" approach — the mediator owns all components, and
  components receive the mediator by reference per call rather than holding it —
  is the pattern's answer to Rust's borrow checker, and it is the natural shape for
  the daemon's supervisor.

## 3. Per-pattern verdicts

### Creational

#### Factory Method — **adopt**
The Rust edition shows a creator trait whose factory method returns the product
(`Dialog::create_button() -> Box<dyn Button>`), with a generic associated-type
variant for static dispatch. **Home:** the session-creation point at the
radio-transport seam — `bluepad-core` defines `HidSession`/`RadioTransport` traits
plus a creation entry (e.g. `SessionSpawner` keyed by path and pad kind);
`bluepad-radio` provides `StockSession::spawn` and `AppOwnedSession::spawn`. This
is also where the per-device routing decision from the connection-layer strategy
attaches: the factory consults the routing table (known path sticks) and returns
the concrete session behind the core trait. Use `Box<dyn>` here — the whole point
is runtime selection.

#### Abstract Factory — **consider (lightweight form only)**
The pattern creates *families* of compatible products (the page's `GuiFactory`
returning a Button + a Checkbox of the same family). The genuine family in BluePad
is per-pad-kind bundles: a report parser + output-report writer (lightbar/rumble,
per the [controller-settings research](https://github.com/acheronian/webos-bluepad/issues/17))
+ pairing flow for DualSense vs DS4 vs Xbox. But a full trait-level abstract factory
is heavier than needed: a plain `fn kit(kind: PadKind) -> PadKit` returning a struct
bundle captures the family guarantee with none of the machinery. **Trigger to
upgrade:** if a fourth dimension of variation appears (e.g. per-path × per-kind
kits that must stay compatible), promote the bundle function to the page's
`GuiFactoryDynamic` shape.

#### Builder — **consider (idiomatic form only)**
The page explicitly separates Builder from the Fluent Interface idiom and shows a
Director orchestrating step-setters. Candidate uses: the authored HID report
descriptor for `uhid` (per the [input-delivery research](https://github.com/acheronian/webos-bluepad/issues/13),
the descriptor is fixed 4.4-ordinal data — a `const` table is simpler than any
builder), and `settings.json` (serde struct with `#[serde(default)]` fields beats
step-by-step construction). **Verdict:** no Director, no setter-chain traits; where
a multi-field constructor gets unwieldy, use the Rust idiom the page points at
(`#[derive(Default)]` + field updates / fluent setters). The full pattern is
over-engineering here.

#### Prototype — **skip**
The Rust edition's entire content is that `#[derive(Clone)]` *is* the pattern,
built into the language. Nothing in BluePad needs cloning of opaque objects through
a registry; sessions are stateful tasks, not copyable values. Derive `Clone` on
plain data (settings, protocol messages) and move on.

#### Singleton — **skip (as a pattern); the articles' safe form is the layout's form**
The page's "Rust specifics" marks global mutable singletons as `static mut` +
`unsafe`, and its "Safe Singleton" example is precisely the architecture ticket's
shape: no globals, state created at the top of `main()` and passed down by
argument/mutability. The daemon's supervisor state and the UI's client state should
each live in one owned struct threaded through `main` — which is the *anti*-singleton
the article recommends. The one sanctioned global is process-wide logging via the
`log` crate (the page's own real-world example); use it for exactly that and
nothing else.

### Structural

#### Adapter — **adopt**
The page's shape — a `TargetAdapter` struct wrapping an incompatible `SpecificTarget`
to satisfy a `Target` trait — is precisely how the LS2 seam should look.
**Homes:** (a) `bluepad-radio`'s LS2 client (adapted from punktfunk's
`ls2.rs`/`luna.rs` two-route pattern, per the [feasibility research](https://github.com/acheronian/webos-bluepad/issues/3))
wrapped to satisfy core's `RadioTransport` trait; (b) the LG `bluetooth2` API's
quirks (error-130 connects, per-device status shapes) adapted into core's typed
`PairingError`/`ConnectError` enums at this boundary, so the rest of the codebase
never sees raw LS2 payloads.

#### Bridge — **adopt** (this seam *is* the pattern)
The page's example is literally devices-and-remotes: an abstraction hierarchy
(Remotes) delegating to an implementation hierarchy (Devices) through a common
trait, each developable independently. BluePad's radio-transport seam is the same
diagram rotated: abstraction = core's session/pairing/verification logic (and the
daemon's supervision over it), implementation = the radio transports (stock
`bluetooth2`, app-owned L2CAP+`uhid`, later the dongle path). The trait lives in
`bluepad-core`, the implementations in `bluepad-radio`, and — the pattern's real
payoff — the transports can grow (dongle bench-test fallout) without touching the
supervision logic. A second, smaller bridge: the UI's screens over a
`DaemonClient` trait, so the UI can run against an in-process fake in tests and
the socket client in production.

#### Composite — **skip**
No tree structures anywhere in the domain: pads are a flat list, settings are a
flat versioned file, screens are a flat rail + cards. The pattern earns its keep
on recursive hierarchies; there are none.

#### Decorator — **skip** (the std-lib form is already in everything we use)
The page's example is `BufReader` over `Cursor` — decorator as std-lib idiom.
BluePad's cross-cutting behaviors (retry, backoff, health-check) belong to the
*supervisor's* state machine, not to same-trait wrapper chains around transports:
a retry decorator over `Box<dyn RadioTransport>` would hide from the type system
which layer owns a failure, and the supervision logic needs to *see* those
transitions. If async middleware is ever wanted, that's tower-style layering in the
daemon, not GoF decorators. Nothing to do here.

#### Facade — **adopt**
The page's `WalletFacade` — one struct whose methods orchestrate five collaborators
behind one API — is the right shape for both process boundaries.
**Homes:** (a) the daemon's request dispatcher: each socket request type maps to
one facade method that coordinates supervisor + settings store + radio state and
returns a response — the protocol handler should read like the page's
`add_money_to_wallet`; (b) the UI side: a single `BluePadClient` struct hiding
socket connect/reconnect, subscription bookkeeping, and the state cache behind
methods the screens call. Also the natural place for the **capability gating**: the
facade's method set *is* the jailed UI's capability surface — what's not a method
is not reachable (see Proxy below).

#### Flyweight — **skip**
The Rust edition is a paywalled stub (no code on the page), and the pattern's
problem — supporting vast quantities of objects by sharing state — does not exist
in a domain with single-digit pads, one radio, one settings file. No shared-state
caching is worth its complexity here.

#### Proxy — **consider (the socket protocol already is one; build nothing extra)**
The page's Nginx example is a *protection proxy*: same interface as the service,
but gating access (rate limiting, checks) in front of it. BluePad's privilege
seam has exactly this shape: the jailed UI cannot touch the privileged radio; the
daemon is the substitute it talks to, and the socket protocol's request surface is
the access-control list. The verdict is *consider* only because there is no code to
write in the pattern's name: the seam the architecture ticket already fixes plays
the proxy's role. One genuine proxy-flavored duty *is* worth doing: the daemon
should answer capability introspection (which requests this daemon version serves)
so the UI can degrade gracefully across versions — a proxy's "controlled access"
duty in protocol form.

### Behavioral

#### Chain of Responsibility — **skip**
The page itself shows the cost: the generic (compile-time) form produces
`Reception::<Doctor::<Medical::<Cashier>>>` type soup, so the pattern needs
`Box<dyn>` chaining. BluePad's candidate chains — report parsing → classification →
forwarding, or the connect fallback ladder (onboard → dongle → stock-only) — are
*fixed* pipelines with a known order, which plain functions in a fixed sequence
express better; the fallback ladder is a policy table the supervisor consults,
not a dynamically composable chain. Dynamic handler composition is a need we don't
have.

#### Command — **skip (the protocol's request enum is the data-only form)**
The page's key Rust point — a command should *not* hold a permanent reference to
context; context arrives as a parameter of `execute` — is good hygiene for the
daemon's request handlers (each handler takes `&mut Supervisor`-ish context, owns
no globals). But the pattern's substance (undo stacks, deferred execution queues)
has no home here: UI requests are serde enum values crossing a socket, they need
no `undo`, and queuing is tokio's job. Model requests as a `#[derive(Serialize,
Deserialize)]` enum with a `handle(ctx)` match — the shape the architecture ticket
already assumes — and don't wrap them in trait objects.

#### Iterator — **adopt (idiomatic; costs nothing)**
The page's guidance is to *not* build a custom iterator interface: implement std's
`Iterator` when a custom collection needs traversal, and otherwise use the
language's. BluePad needs nothing beyond that: pad lists, event batches, and
descriptor ordinals all iterate with std iterators. Listed for completeness — the
verdict is "use the language," which the codebase gets for free by being idiomatic.

#### Mediator — **adopt** (top-down ownership)
The page's "Top-Down Ownership" approach — a station that *owns* the trains, hands
them a `&mut dyn Mediator` per interaction, and routes between them — is the
design the daemon's **session supervisor** should take: one struct owning the
transport tasks, per-pad session tasks, settings store, and the event broadcaster;
sessions never hold references to each other or to the radio, they notify the
supervisor (arrival/disconnect/failure) and the supervisor decides (queue, back
off, restart, reconcile). This is the pattern's *and* Rust's answer to the
supervision seam: crash-restart and idempotent reconcile are mediator bookkeeping,
and ownership living in one place is what makes "restart the session" a method
call instead of a reference-graph puzzle.

#### Memento — **consider (serde form only, and only where a snapshot is genuinely needed)**
The page's second example is the honest Rust form: serde serialization as the
snapshot. BluePad already uses it where it counts (one versioned `settings.json`,
per the [controller-settings research](https://github.com/acheronian/webos-bluepad/issues/17)).
The pattern's restore-stack form (history of undoable snapshots) has no use: daemon
crash recovery is *reconcile against the radio's actual state* (ask what's paired,
what's connected, rebuild), not snapshot-restore — a restored snapshot would be a
lie the radio disagrees with. **Verdict:** keep settings-as-serde; resist anything
history-shaped.

#### Observer — **adopt** (it is the socket protocol's subscribe/notify half)
The page's form — a publisher with per-event subscriber lists, subscribers as plain
functions — is the synchronous, in-process shape. BluePad needs the pattern twice,
in two forms: (a) **in the daemon**, the tokio form: session tasks publish
`StateEvent`s (radio status change, pad connected/lost, pairing progress) to a
`tokio::sync::broadcast` channel; the socket layer subscribes and forwards to UI
subscribers — the supervisor-as-mediator and this channel are the same machinery;
(b) **in the UI**, the page's shape almost verbatim: the client holds subscriber
callbacks per event topic and the event loop dispatches to them, driving card
redraws. The subscribe/notify protocol half of the architecture ticket is this
pattern at a process boundary; name it that in the code (subscribe/unsubscribe/
notify) so the protocol vocabulary matches the design vocabulary.

#### State — **adopt (enum-first; the trait form only if states diverge hard)**
The page builds a music player FSM with `self: Box<Self>`-consuming transitions
(`StoppedState → PlayingState`), and notes the pattern is the FSM idea expressed in
types (also covered in The Rust Book's OOP chapter). BluePad's per-pad session is
the natural FSM: Discovered → Pairing → Connecting → Connected → Backoff → Lost,
with standby/sleep transitions — and the supervision machinery (backoff, restart,
reconcile) hangs off these transitions. **Form:** start with a plain
`enum PadState` + `match` in the supervisor (serializable for the UI protocol,
exhaustive under the compiler, no allocation churn at 60-ish events); reach for the
page's `Box<dyn State>` trait form only if individual states grow heavy
per-state data and behavior that the match can't keep readable. The state
*transitions* the acceptance matrix cares about (sleep, standby, app restart) are
protocol-visible and must serialize — another point for the enum.

#### Strategy — **adopt** (closed sets → enums/fn pointers, not trait objects)
The page shows both Rust forms: a generic `Navigator<T: RouteStrategy>` and the
functional form (`type RouteStrategy = fn(...)` — strategies as plain function
pointers). BluePad has two genuine strategies: (a) **route selection** — app-owned
vs stock per device, sticky once known (the [connection-layer strategy](https://github.com/acheronian/webos-bluepad/issues/7)
decision) — an enum `Route` stored per pad in the routing table; (b) **backoff /
retry cadence** — exponential vs fixed vs none per failure class, a small
`BackoffPolicy` enum or fn-pointer set in the supervisor. Both sets are closed and
known at compile time, so the page's functional form fits better than trait
objects: strategies here are *data about how to behave*, persisted in the routing
table and settings, not plug-in objects.

#### Template Method — **consider**
The page's Rust form — a trait with a default `template_method()` calling
`base_operation`s and required `required_operation`s — fits one place: the
**pairing flow skeleton**. All pad types share scan → discover → pair → connect →
verify, but differ in the per-type help steps (the PS5/PS4/Xbox pairing steps in
the UI sheet) and in connect-path specifics. If the shared skeleton is real (the
per-type differences are localized to a couple of hook methods), use the trait form
in `bluepad-core`; if the flows turn out to differ in the middle rather than at the
edges, a per-kind function with a shared helper is simpler. Decide when the pairing
flow is actually written — this is the one pattern whose verdict should be made by
the code that needs it, not in advance.

#### Visitor — **skip**
The page's real-world Rust example is serde's deserialization model — Visitor is
what serde *already does for us* under `#[derive(Deserialize)]`. BluePad has no
heterogeneous object hierarchy to walk with pluggable operations (the protocol enum
gets a `match`; report parsing is per-kind functions). Adding a hand-rolled
visitor would be re-implementing a service the derive macros provide.

## 4. Summary

| Pattern | Verdict | Concrete home |
|---|---|---|
| Factory Method | **adopt** | session creation at the radio seam (`core` trait, `radio` impls, routing table consulted here) |
| Abstract Factory | consider (light) | per-pad-kind kit (parser + output writer + pairing flow) as a bundle fn, not a factory trait |
| Builder | consider (idiom) | `Default` + serde for settings; `const` table for the HID descriptor; no Director |
| Prototype | **skip** | `#[derive(Clone)]` is the whole pattern |
| Singleton | **skip** | the page's safe form = the layout's form: owned state threaded from `main`; `log` crate the one global |
| Adapter | **adopt** | LS2 client + `bluetooth2` quirks → core's `RadioTransport`/typed errors, in `radio` |
| Bridge | **adopt** | the radio-transport seam itself; secondarily UI screens over a `DaemonClient` trait |
| Composite | **skip** | no tree structures in the domain |
| Decorator | **skip** | retry/backoff/health-check belong to the supervisor FSM, not wrapper chains |
| Facade | **adopt** | daemon request dispatcher; UI's `BluePadClient`; method set = capability surface |
| Flyweight | **skip** | paywalled stub page; no vast-object-count problem |
| Proxy | consider | the socket protocol already plays the protection proxy; add capability introspection only |
| Chain of Responsibility | **skip** | fixed pipelines as plain functions; fallback ladder is a policy table |
| Command | **skip** | protocol requests as a serde enum + `handle(ctx)` match; no undo, no trait objects |
| Iterator | **adopt** (idiom) | std iterators everywhere; nothing to build |
| Mediator | **adopt** | the session supervisor: top-down ownership of transports, sessions, settings, broadcaster |
| Memento | consider (serde) | settings-as-serde already decided; crash recovery is reconcile, not restore |
| Observer | **adopt** | daemon: `broadcast` channel of `StateEvent`s; UI: per-topic subscriber callbacks; the subscribe/notify protocol half |
| State | **adopt** (enum-first) | per-pad session FSM as `enum PadState` + match in the supervisor; `Box<dyn State>` only on real divergence |
| Strategy | **adopt** | route selection + backoff cadence as closed enums / fn pointers in the routing table & supervisor |
| Template Method | consider | pairing-flow skeleton, decided when that flow is written |
| Visitor | **skip** | serde's derive macros are the visitor; no hierarchy to walk |

**Balance:** 9 adopt, 5 consider (all with a named
lighter form and an upgrade trigger), 8 skip. The skips are the patterns whose
problem statements BluePad doesn't have (tree structures, vast object counts,
undoable command histories, dynamic handler chains, double dispatch) — skipping
them *is* the anti-over-engineering verdict the ticket asked for.

## 5. What this means for the architecture ticket

For [Grilling: App architecture](https://github.com/acheronian/webos-bluepad/issues/15),
the pattern vocabulary names the seams it is deciding:

- The **radio-transport seam** = Bridge (trait in core, impls in radio) + Adapter
  (LS2/`bluetooth2` wrapped at the boundary) + Factory Method (creation + routing
  point).
- The **supervision machinery** = Mediator (top-down ownership) + State (enum FSM)
  + Strategy (backoff); crash recovery is *reconcile*, never snapshot-restore.
- The **capability gating** = the Facade's method set; nothing needs building beyond
  the protocol surface itself, plus optional capability introspection.
- The **socket protocol** = Observer (subscribe/notify) + data-only Command
  (request/response as a serde enum, handlers taking context as a parameter).
- The **UI** = a Facade client over a Bridge trait, with the page's function-pointer
  observer form for event dispatch.

None of this adds crates, layers, or indirection beyond what the four-seam layout
already fixes — the pattern work is *naming and shaping* the seams the architecture
ticket has to settle, which should make that grilling session shorter, not bigger.
