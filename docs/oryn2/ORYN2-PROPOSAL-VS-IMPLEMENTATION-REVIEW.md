# Oryn2 v0.2 proposal vs. implementation — combined gap review

**Assessment date:** 2026-08-28  
**Assessed source:** `0b7a38b` (`refactor: replace phase labels with capability names`)  
**Current status update:** 2026-08-30

**Current status source:** `dfd67b0` (`feat: complete native CSP enforcement`)
plus the verified, uncommitted remediation work described below

**Proposal:** *Oryn2 Product Proposal & Technical Architecture v0.2*  
**Scope:** combined product, architecture, code, evidence, security, and release assessment

This is the authoritative combined review. It joins the forensic code and
evidence findings from `ORYN2-PROPOSAL-VS-IMPLEMENTATION-REVIEW.md` with the
broader product-gap assessment formerly maintained as
`PROPOSAL_GAP_ASSESSMENT.md`. It deliberately applies a stricter standard than
the repository's capability milestone pages: passing a frozen controlled
manifest is historical evidence for a bounded feature, not proof that the full
v1 product described by the proposal is complete.

> **Reading note:** Sections 1–7 preserve the forensic assessment of source
> `0b7a38b`. They are historical findings and should not be read as a fresh
> description of current production code. This status section records the
> verified disposition at `dfd67b0` plus the current working tree; later
> remediation notes retain additional
> implementation detail. A finding is closed only when its software defect is
> absent. Release evidence is tracked separately and must bind to the exact
> source and packaged artifacts being claimed.

## Current HEAD status

Oryn has completed the central proposal-v1 runtime cutover: Rust owns the live
DOM; generated WebIDL bindings are installed in V8; same-origin frames use
separate contexts within one page isolate; html5ever supports streaming parser
pauses and document-write re-entry; and Fetch/XHR use the asynchronous parent
broker. The stable `oryn` Rust facade, native-default CLI, full/English-lite
runtime profiles, WebAssembly, HTTP/2, charset decoding, bounded caches,
SameSite cookies, and the `csp3-agent-browser-2` profile also exist.

The implementation is nevertheless not complete against every original design
requirement. The remaining work is concentrated in long-tail layout
compatibility, Linux/release operations, representative compatibility, and
exact-artifact re-attestation.

### Finding disposition at `dfd67b0` plus current verified remediation

| Finding | Status | Current disposition |
|---|---|---|
| F1 — Rust-owned DOM | **Closed in software** | `oryn-dom` is the sole live-DOM authority. The JavaScript-owned graph, full-document export/synchronization, and old frame-document path are absent from production. |
| F2 — semantic KPIs | **Partial; release evidence pending** | Schema v5 implements compact-versus-full failure attribution, labelled causal classes, confirmed-only challenge detection, corrected canary classification, and expanded exact-identity churn. The corrected 96-cell panel and release-candidate churn result have not been produced. |
| F3 — style/layout architecture | **Partial; common-web subset expanded** | The shipped contract uses Stylo declaration parsing, Taffy box algorithms, and Parley measurement. It now covers bounded table/row/cell geometry in addition to block, inline, flex, grid, floats, positioning, overflow and 2D transforms. Fragmentation/multicolumn, vertical writing/ruby, SVG layout, 3D transforms and paint-order hit testing remain explicit alternate-domain boundaries; it is not a claim of general browser layout parity. |
| F4 — act/wait/event loop | **Closed for the declared scheduler profile** | Rust owns timer deadlines and ordering. Each checkpoint runs V8 microtasks, host-phased custom-element reactions, Rust-journal MutationObservers to a stable point, style/semantic publication, then rendering-opportunity animation, ResizeObserver and IntersectionObserver delivery. Focused ordering and self-mutation regressions pass. |
| F5 — evidence integrity | **Partial** | Layout and security evidence builders now bind manifest cases to assertions and recompute their aggregates. Some historical builders/manifests still carry supplied pass states, and the current source tree has no unified exact-artifact attestation. |
| F6 — CORS/SameSite/CSP | **Closed in software; release evidence pending** | Parent CORS/preflight authority, schemeful SameSite cookies, mixed-content policy, CSP reporting, violation events, opaque sandbox origins, and worker/media/object/manifest directives are implemented. The latest CSP change is newer than the committed signed security evidence. |
| F7 — consequential policy | **Closed in software for the native profile** | Concrete form submission and every redirect target are checked at the effect boundary. File upload uses parent-owned opaque handles and sends only explicitly selected payloads to the page worker. Clipboard is text-only and isolated to `BrowserContext`; host system clipboard access remains unsupported. Both paths are policy-controlled and audited. |
| F8 — parser and scripts | **Closed in current software; release evidence pending** | Streaming html5ever, parser-blocking scripts, contextual fragments, serialization, bounded `document.write` re-entry, and dynamic script preparation/execution are implemented. Connected inline classic scripts preserve synchronous insertion ordering; external/module scripts use the parent resource scheduler; fragment-created scripts remain inert. |
| F9 — page-facing capability honesty | **Partial only for declared layout boundaries** | JavaScript geometry distinguishes available, hidden, and unsupported states; ancestor overflow clipping affects interactability. Mutation, resize and intersection observers now consume the authoritative DOM/layout checkpoints. Unsupported fragmentation, vertical/ruby/SVG/3D/paint-order behavior still emits capability diagnostics instead of fabricated geometry. |
| F10 — concurrency and realms | **Closed in software** | Worker exchanges run through `spawn_blocking`; one worker/isolate owns each top-level page; same-origin frames and dedicated workers use separate persistent V8 contexts in that isolate. Dedicated-worker globals and message queues are realm-isolated from their owner page. |
| F11 — retained-state bounds | **Closed in software** | Revision history has count/byte ceilings, DOM slots and wrapper leases are reclaimed, and memory/persistent caches have entry and byte bounds. |
| F12 — untrusted content | **Mostly closed** | Semantic nodes carry `ContentTrust::PageAuthored`, inferred facts use heuristic provenance, and raw inspection is policy-audited without recording page content. Broader prompt-boundary evidence remains narrower than the proposal's full trust model. |
| F13 — secret leakage | **Closed in software** | Secret sensitivity is stored independently on Rust DOM nodes, applies to text inputs, survives page-controlled attribute changes, and redacts observation and HTML extraction. |
| F14 — generated WebIDL | **Closed in software** | Deterministic generated bindings install real prototypes and callbacks with wrapper identity, brand, realm, origin, generation, and weak-root enforcement. |
| F15 — production-path testing | **Closed in software; signed artifact evidence pending** | `production-profile-equivalence-v1` and its validator cover default/full/lite/no-V8 builds, generated bindings, scheduler/observer ordering, uploads, virtual clipboard, accessibility, layout and inherited-FD provisioning. Exact signed worker execution remains a private release-attestation step. |
| F16 — Intent/API/MCP | **Closed in software; release evidence pending** | Agent contract v2 adds upload and context-virtual clipboard intents while preserving the existing resolver/action engine. OIL accepts `$secret:<label>` and `upload <target> $file:<label>`. Standalone Python and MCP accept secrets only through an inherited descriptor consumed by the privileged parent; values never enter argv, environment, OIL, JSON-RPC, traces, observations or worker initialization. |

### Current broader-gap status

- **Canonical product boundary — implemented in software.** `crates/oryn` is a stable Rust
  facade, native is the CLI default, and the shared Rust `Intent`/`Target`
  contract executes through the same semantic resolver and action engine as
  OIL. Python and the first-party MCP stdio server use the same versioned JSON
  envelope; OIL subprocess transport remains available for compatibility.
  Secure secret-label provisioning is available to Rust embedding, Python and
  MCP through parent-owned memory and inherited descriptors.
- **JavaScript/runtime profiles — implemented in software.** V8 WebAssembly,
  `core-full`, and English-lite locale diagnostics are present. Exact packaged
  ICU/profile sizes remain release evidence.
- **Network/decoding profile — implemented in software.** HTTP/2, gzip,
  Brotli, deflate, zstd, charset precedence, cancellation, bounded streaming,
  and context-partitioned caching have production implementations and focused
  tests.
- **Semantic runtime — structured contract implemented; broader conformance remains.**
  Additive contract v5 preserves frozen v2-v4 observations while publishing
  first-class forms, tables, lists, navigation, dialogs, menus, trees, tab sets,
  grids, landmarks, live regions, repeated collections, ownership and ARIA
  relationships. Supported accessible-name/description, role, state and
  tree-exclusion rules carry explicit evidence, provenance and trust. Long-tail
  A pinned agent-critical Accessible Name/ARIA set now covers referenced names,
  hidden descendants, host alternatives, default submit names, role fallback
  and presentational-role conflicts. Long-tail conformance and changing-site
  evidence remain broader compatibility work rather than silently inferred
  support.
- **Compatibility — controlled evidence only.** Eight pinned frameworks and
  five controlled authentication scenarios are useful deterministic coverage,
  not a representative changing-site compatibility program.
- **Linux — implemented but unattested.** Namespace, `no_new_privs`, Landlock,
  and seccomp installation code exists and fails closed. It still requires a
  Linux VM run, policy-parity evidence, reproducible x86_64/ARM64 packages, and
  SBOM/provenance publication.
- **Rendering — bounded subset.** The optional renderer is deterministic for
  its declared effects; browser visual parity, broader SVG/fonts, GPU output,
  and multi-page print layout are not claimed.
- **Performance — current target gap.** Historical signed runs report roughly
  500–580 ms startup-to-handshake, above the proposal's 250 ms cold-start
  target. Current-release startup, warm-page, RSS, concurrency, and sustained
  churn figures must be republished or receive an explicit reviewed exception.

### Evidence and release status

The current source tree is not release-attested as one bundle:

- `validate_runtime_architecture.py --require-cutover` rejects the committed
  shadow evidence because its source-tree hash predates `dfd67b0`.
- Schema-v5 semantic validation cannot complete because the corrected
  `model-panel-v5.json` has not been generated.
- Layout evidence recomputes successfully but is bound to `68e5afe`.
- Security evidence recomputes successfully but is bound to `6eb164d` and
  predates the completed CSP profile.
- The schema-v5 replacement run must include expanded churn plus all 96
  Azure/Qwen × native/Chromium cells. Prior hosted spend remains `$6.362286`
  against the original `$100` ceiling.

The current honest product statement is therefore:

> **The proposal-v1 runtime foundation is substantially implemented, including
> the Rust-owned DOM and secure parent/worker architecture. Oryn is not yet
> complete against the original design or release-ready as one attested
> artifact. Recent working-tree remediation makes unsupported geometry visible,
> adds ancestor clipping, gives production timers real deadlines, and closes
> redirect-time policy bypasses. It also adds one shared typed Rust/OIL/Python/
> MCP intent contract and first-class semantic/accessibility structures. The
> principal remaining software gaps are long-tail layout compatibility and
> broader changing-site coverage;
> platform and model evidence must then be regenerated against one exact
> release candidate.**

### Updated implementation priority

1. Expand pinned accessibility and layout differential coverage while
   preserving explicit evidence, provenance and capability boundaries.
2. Exercise the fail-closed Linux sandbox in CayVM, add platform parity and
   packaging/SBOM evidence, and optimize or explicitly except startup targets.
3. Regenerate shadow evidence and run the corrected churn and 96-cell panel on
   one clean, signed, fingerprinted release candidate.

### 2026-08-30 working-tree remediation status

The following changes are implemented and have focused local verification, but
are not yet represented by a committed source identifier or exact-artifact gate
result:

- Generated V8 geometry bindings preserve `Available`, `Hidden`, and
  `Unsupported` instead of converting every unavailable box into a plausible
  zero rectangle.
- Layout interactability is clipped by viewport and ancestor padding boxes for
  `hidden`, `clip`, `scroll`, and `auto` overflow, including visible-axis
  normalization when the other axis establishes a scroll container.
- Production timers use real deadlines, deterministic ordering, repeating
  intervals, self-cancellation, and a V8 microtask checkpoint after each timer
  task.
- Cross-origin navigation authorization is preserved as a chain grant only
  after the initiating action is approved, and every redirect origin is checked
  again by the parent policy service.
- File upload uses parent-imported opaque handles, policy authorization,
  native `FileList`/`FormData`, agent contract v2 and OIL `$file:` labels.
  Clipboard is a policy-controlled text buffer scoped to `BrowserContext`; host
  system clipboard access remains explicitly unsupported.
- Standalone CLI/Python/MCP secret labels can be provisioned through a bounded
  inherited descriptor that is consumed before page work and never copied into
  argv, environment, OIL, MCP requests, traces or worker initialization.
- The checkpoint pipeline now covers Rust timers, V8 microtasks,
  custom-element reactions, Rust-journal MutationObservers, style/semantic
  publication, animation frames, ResizeObserver and IntersectionObserver.
- Agent-critical Accessible Name/ARIA pins and bounded table geometry extend
  the semantic and common-web layout profiles without claiming long-tail
  browser parity.
- Dynamically connected scripts now have one-shot preparation state. Inline
  classic scripts execute synchronously at insertion; external/module scripts
  run through parent-authorized resource loading; and fragment-created scripts
  remain inert.
- Dedicated workers now execute in independent persistent V8 contexts, with
  parent-brokered `worker-src` loading, realm ownership checks, bounded worker
  counts, and native message queues. The JavaScript compatibility-scope and
  `Function`-constructor execution path have been removed.
- `oryn-common` now owns a versioned serializable `Intent`/`Target` contract.
  `PageHandle::perform` delegates target resolution to the existing native OIL
  resolver and mutations to the existing typed action path, so the new Rust
  entry point does not introduce a competing selector or scoring engine.
- OIL now accepts `type <target> $secret:<label>`. The label resolves in the
  parent-owned, origin-scoped secret store and invokes the existing secret-fill
  path; serialized intents and normalized OIL contain the label only, never a
  plaintext secret value. Compatibility translators reject this native-only
  form rather than treating the label as text.
- Python `perform()` sends the shared versioned intent envelope and returns
  typed observation, action, navigation, delta, extraction, wait and semantic
  structure results. Compatibility modes fail closed instead of translating a
  typed request into a different backend contract.
- `oryn-mcp` implements MCP stdio JSON-RPC and exposes only `oryn_perform` with
  a complete JSON schema for the shared Intent/Target contract. Its secret
  input schema contains a parent-owned label and no value field; it does not
  provide OIL or raw-JavaScript tunnelling.
- Rust embedding can populate the parent-owned origin-scoped store with
  `BrowserContext::import_secret`. The standalone Python subprocess and MCP
  server currently create their own contexts and expose no secure provisioning
  channel. Their secret intents therefore fail closed on an unknown label; the
  adapters must not be described as end-to-end secret-capable until that
  parent-side integration exists.
- Additive semantic contract v5 publishes first-class agent structures for
  forms, tables, lists, navigation, dialogs, menus, trees, tab sets, grids,
  landmarks, live regions and repeated collections. Accessibility nodes carry
  supported name/description evidence, ARIA relationships and ownership,
  properties, tree-exclusion reasons, trust and provenance without modifying
  frozen v2-v4 observations.

Focused tests cover geometry-state distinction, overflow clipping, timer
deadlines/microtask ordering, interval cancellation, redirect-origin denial,
and consequential form effects. Dynamic-script ordering/one-shot/inertness and
dedicated-worker realm isolation are also covered. The no-default-feature
native build also checks successfully. Focused parser/normalizer tests cover
secret-label handling, and typed-intent tests cover shared semantic resolution,
native action execution, and value-free secret serialization. These results
include typed Python SDK tests, MCP protocol/schema tests, first-class semantic
structure/accessibility tests, and worker-protocol serialization. They improve
the software disposition of F4, F7, F8, F9, F10, and F16, but do not
change the release-attestation status above.

The verified working-tree test record is: 178/178 `oryn-native` V8 library
tests; all `oryn-common` and `oryn-core` tests; six `oryn` MCP tests; all-feature
binary checking; strict all-target Clippy with warnings denied; formatting;
26 passing Python SDK tests with 19 environment-dependent skips; Ruff; an MCP
stdio initialize/tools-list smoke test; and `git diff --check`. This is local
software verification, not an exact packaged-worker, signed-release, Linux, or
model-panel attestation.

## 1. Verdict

The implementation delivers the host-layer and isolation half of the thesis convincingly — the raw-V8 decision is settled by measurement, thread affinity is enforced by the type system, the hardened worker profile is real and is the production default, SSRF is closed on resolved-and-pinned IPs, and record/replay plus reference identity are genuinely built and honestly conservative. It inverts the two claims everything else in the document rests on: **Rust owns the DOM** (§8.4/§9.1) and **the browser continuously maintains semantic state** (P4). What ships is a JavaScript-canonical DOM copied into Rust by `JSON.stringify` of the whole document after every action (`executable.rs:608-618`), a full re-projection on every `observe` (`semantic.rs:235`), and a hand-rolled cascade and block-flow layout sitting under the Stylo/Taffy names (`oryn-layout/src/lib.rs:601-638`, `:985-999`).

The single most important thing to fix is neither of those — it is the evidence layer. Three of the four named §5.2 semantic KPIs compute something other than their stated definition, the layout ledger writes `"status": "passed"` into every case unconditionally (`scripts/build_layout_result.py:110-133`), and the runtime-foundation concurrency number that closes the §16.5 mandatory spike was measured on a build with no JavaScript engine (`scripts/benchmark-native-runtime.sh:12`). Until those are corrected the project cannot tell whether any of the rest is working, and each subsequent capability gate inherits a false baseline. After that, decide in writing whether §8.4 is still the target or the JS-canonical DOM is the design — the whole of §9, §12 and §16.4 changes meaning depending on the answer.

---

## 2. Where the build is faithful

Load-bearing only.

- **§8.2 host decision, settled by measurement not convention.** `docs/oryn2/adr/0001-raw-v8-host.md` records a 30-process comparison with both hosts pinned to V8 150.4, and `deno_core` is kept as a feature-isolated probe rather than a runtime dependency. Production `oryn-page-worker` links the raw host.
- **§7.2 thread affinity enforced by the type system.** `pub trait JavaScriptHost` carries no `Send` bound (`host.rs:7-16`); `RawV8Host` owns a `v8::OwnedIsolate` and is therefore un-`Send` by construction. `PageHandle` (`runtime.rs:102-110`) exposes ids and a channel sender only — no V8 handle, no raw pointer.
- **§6.3 hardened profile is real and is the default.** Nonce-bound handshake, protocol/contract version pinning, `env_clear()` plus secret-environment assertion, binary SHA-256 matched against the codesign-verified artifact, release-build requirement, fail-closed (`worker_client.rs:110-262`). `create_page` returns the identical `PageHandle` type either way (`runtime.rs:1917-1975`).
- **§11.4 SSRF closed at the right layer.** `resolve_and_authorize` checks *every* returned address and returns the vetted set (`network.rs:152-193`); `RustlsTransport::send` pins it with `resolve_to_addrs` (`network.rs:246-250`), so rebinding cannot select a different address. Redirects are disabled in the transport (`network.rs:241`) and all three redirect loops re-enter the broker.
- **§12.2 reference identity, conservative in the right direction.** `SemanticRef` is page + document_generation scoped and revalidated on every action (`runtime.rs:4193-4212`); identity survives only by exact JS-side node identity (`executable.rs:695-700` → `dom.rs:55-85`). False preservation is structurally near-impossible, which is the property §12.2 says matters most.
- **§18.2/§18.3 replay is built, and refuses what it cannot do.** Captured responses are substituted with order + request + body-hash verification and still pass through `PolicyBroker`, so replay is not a policy bypass (`replay.rs:139-215`, test at `:483-497`). Determinism seeding really patches Date/Math.random/crypto (`executable.rs:621-640`). Level 3 heap forking is refused rather than faked (`runtime.rs:803-805`).
- **§10.2 "refuse, don't approximate" is implemented as a real mechanism.** `mark_unsupported_subtree` (`oryn-layout/src/lib.rs:849-858`) marks tables `Unsupported` with no box; `box_model()` returns `UnsupportedGeometry` (`runtime.rs:4253-4261`) and `ensure_interactable` refuses the click (`runtime.rs:5113-5127`). This is the best pattern in the codebase and it should be the template for everything in Finding 3 and Finding 9.
- **§21.2 migration boundary genuinely achieved.** `Backend` and `ScannerAction` appear only inside the opt-in `oryn-domain` adapter (`crates/oryn-domain/src/lib.rs:10-12`); `oryn-native` never depends on `oryn-scanner`; the parent↔worker wire format is a typed serde enum (`worker.rs:100-260`), not OIL text.

---

## 3. Findings

> **Superseding implementation update (2026-08-29):** F1 and F14 describe the
> source at the review commit and are retained as historical findings. They no
> longer describe production HEAD. `rust_executable.rs` now uses `oryn-dom` as
> the sole live DOM, installs deterministic generated V8 templates from
> `oryn-js`, preserves realm/brand/generation-checked wrapper identity, and uses
> separate V8 contexts in one page isolate. The JavaScript-owned DOM,
> full-document export/synchronization, handwritten interface graph, and old
> frame-document implementation have been deleted. The frozen shadow ledger
> passes without duplicated live traffic. Runtime cutover remains provisional
> only for the separate private-platform release attestations.

Ranked by severity. Where several reviewers hit the same underlying issue from different sides, they are merged and the merge is stated.

---

### F1 — Critical. The DOM lives in JavaScript; Rust holds a JSON copy re-imported after every action
*Merged: the runtime/host reviewer and the DOM reviewer independently reported this as their top finding; the DOM reviewer's separate O(N)-serialization-cost finding is the same defect and is folded in.*

**Proposal:** §8.4 — "The DOM should be Rust-owned. JavaScript objects are wrappers that carry an internal NodeId… Wrapper stores NodeId, never a raw Rust pointer." §9.1 — "Owning the DOM makes semantic invalidation, action tracing and stable references intrinsic browser operations." P2 — the native path requires "no… JSON serialization."

**Code:** The DOM page JS sees is a 94 KB minified JavaScript object graph, `DOM_BOOTSTRAP` at `crates/oryn-native/src/executable.rs:827-1173`, executed at `:262` and seeded from a JSON dump at `:270`. Rust's arena is rebuilt from JS after every interaction: `sync_dom` runs `evaluate_string("JSON.stringify(__oryn_export())")` → `serde_json` → `into_dom` → `DomArena::prepare_rebuild` (`executable.rs:608-618`, `:695-700`). It terminates `click` (`:496-503`), `type_text` (`:505-513`), `drain_tasks` (`:515-519`), `apply_action` (`:521-575`) and even a pure-read `evaluate` (`:445-449`). No `SetWeak`, no `SnapshotCreator`, no Rust-side wrapper anywhere in `crates/`. `layout_snapshot` (`:527-541`) performs a *second* full export.

**Why it matters:** The direction of ownership is reversed, so semantic invalidation, action tracing and stable references are downstream of a serialization step rather than intrinsic operations — the exact "Oryn can only inspect it" position §9.1 argues against. Cost is O(document) encode + parse + arena rebuild per keystroke regardless of mutation size, which directly contradicts §16.4's "incremental style/semantic invalidation rather than global recompute" and "observation deltas rather than full serialization". It is also the root cause of F3's serialization boundary, F11's slot leak, and the loss of namespaces (`from_dom` emits only `name.local`, `executable.rs:668-693`; `insert_into` rebuilds everything as `ns!(html)` at `:743-749`, so SVG/MathML is HTML-namespaced after the first sync). One correction to the reviewers: NodeIds *do* exist (`__id = {slot, generation}`, reachable via `__oryn_get`, `executable.rs:571`) — what is missing is Rust ownership and weak-handle rooting, not the id.

**Recommendation — change the proposal, then the code, in that order.** §8.4/§8.5/§9.1 and P2 currently describe the opposite system; amend them to state that v1 is a JS-side DOM with a Rust mirror, with an explicit P2 carve-out, and correct `IMPLEMENTATION_STATUS.md`'s "Oryn-owned element/text nodes" wording. Then keep the NodeId/weak-wrapper model as named G-gate work rather than implied current state. Cheap wins available immediately: skip `sync_dom` on non-mutating `evaluate`, and carry `ns` through `JsNode` — the loss is unrecoverable once it happens.

---

### F2 — Critical. Three of the §5.2 product KPIs do not measure what they are named for
*Merged: the semantic reviewer and the interfaces/evidence reviewer independently flagged observation sufficiency and causal precision; challenge rate, reference survival and the `deterministic` flag are folded in as the same class.*

This is the finding to act on first, because these numbers are what the proposal makes the product KPIs and what every gate is scored against.

**Observation sufficiency — wrong denominator, inverted polarity, no full-vs-compact comparison.** §5.2 defines it as the percentage of *task failures* where the needed state existed in browser truth but was absent from the compact projection, driven toward zero. `intentgym/scripts/run_semantic_model_panel.py:744-745` takes the numerator over *all completed cells* and `:760` divides by `len(completed)` = 96. The predicate `_checkpoint_sufficient` (`:641-737`) regex-matches the MiniWoB instruction string against `turn["observation_raw"]` and checks for expected roles; nothing ever requests the `full` profile, so the compact-vs-truth comparison §17.5 mandates does not happen. The published ledger shows the consequence directly: `benchmarks/evidence/semantic-sandbox-v4/model-panel-v4.json` cell 0 is `click-button-s17-native-hosted` with `success: true` and `observation_sufficient: false`; 30 of the 36 *successful* cells are labelled insufficient, and all 48 `qwen3:4b` cells (zero successes) are in the denominator. Higher is better here; lower is better in the proposal. `IMPLEMENTATION_STATUS.md:163` publishes "13/96" with no caveat.

**Causal precision/recall — measures effect-kind bookkeeping, not attribution.** §5.2 requires precision and coverage published separately for direct/derived/correlated, against labelled action→JS→network/navigation/DOM datasets, "never count[ing] correlation as deterministic causality." `run_semantic_deterministic.py:351-357` defines ground truth as a single effect *kind* per OIL verb (`{"event"}` for click/type/…), and `:323-327` scores set overlap over kinds. `runtime.rs:5241-5248` unconditionally emits `Effect::DomMutation` when the delta is non-empty and then always `Effect::Event`, so recall is 1.0 by construction (99/99) and a correctly-reported DOM mutation is scored as a causal false positive (99/198). No direct/derived/correlated split is computed anywhere, and `Effect::Correlated` (`oryn-common/src/v3.rs:208`) is never constructed — so §5.2's central prohibition is untestable. `TraceEvent` (`trace.rs:7-18`) carries no causality field at all, and `execute_in_realm` (`runtime.rs:5183-5290`) stamps every drained console/context entry with the current `ActionId`, including work a timer scheduled before the action ran. *(One reviewer claim corrected: not every ledger row is exactly 2×; login-user is 4→7, search-engine 5→11. The aggregate 0.5 is a coincidence of offsets.)*

**Challenge rate — native failures labelled as site challenges.** §5.2 says explicitly this is "not a native-failure KPI". `ExecutionRequirements::site_challenged` (`execution.rs:99`) is set nowhere in production — only in a unit test (`:526`) and the stub probe (`oryn-domain/examples/execution_domain_probe.rs:90`). There is no detector and no `challenge_rate` field in any schema. Meanwhile `benchmarks/evidence/semantic-public-canaries.json` classifies `wikipedia.org` and `crates.io` as `"availability": "challenged"` with diagnostics `["Unexpected token", "empty semantic observation"]` — a native parse failure producing zero semantic nodes. Neither site challenges anything. This inverts the exact distinction the metric exists to protect and makes the native engine look better on the one number designed to prevent that.

**Also in this class, lower weight:** `reference_survival: 1.0` has a denominator of one case that mutates a *sibling* and never touches the target (`examples/semantic_reference_churn.rs:195-213`) — under React/Vue re-render, survival in this design is structurally 0, and no framework-churn case exists. `ReplayRun.deterministic` is `true` whenever two caller-supplied vectors are empty (`runtime.rs:987`), and five of six `NondeterministicRegion` variants are never constructed, so an unseeded trace of a page calling `Date.now()` replays as `deterministic: true` — the precise claim §18.2 forbids.

**Recommendation — change the code (the harnesses), and correct the published numbers today.** (a) Reimplement observation sufficiency per §5.2: denominator = failed tasks; numerator = failures where the node was present in `full` and absent from `compact`, established by capturing both at the failing turn; bucket model-quality failures separately. (b) Build the labelled per-effect causal corpus, classify each effect direct/derived/correlated from the drain path, and publish per class. (c) Either implement a challenge detector or state in the proposal that challenge rate is deferred beyond execution-domain work — and reclassify canary rows whose diagnostics are native errors as native failures. (d) Until these land, relabel the figures in `IMPLEMENTATION_STATUS.md:137,163` as non-conforming heuristics rather than §5.2 metrics.

---

### F3 — Critical. The §10 engines are decorative in the hot path, and wrong geometry is reported as `Available`
*Merged: three separate style/layout findings (cascade, Taffy, inline formatting) — one underlying defect with three mechanisms.*

**Proposal:** §3/§10.1 — "Stylo… Oryn2 should integrate Stylo rather than own CSS parsing/cascade logic"; §10.2 — "Taffy for box-level block/flex/grid/float geometry" and "Geometry cannot be correct without text shaping, line breaking and inline fragment construction."

**Code — cascade:** Stylo is used only to validate one declaration block at a time (`parse_style_attribute` + re-serialize, `oryn-layout/src/lib.rs:601-638`). Everything above is Oryn's: stylesheets tokenized by brace-counting `find('{')` (`:430-476`), so `@supports`/`@layer`/`@container` are silently dropped; `@media` matched by substring (`:478-506`); specificity computed by counting `#`, `.`, `[`, `:` characters (`:697-711`); selector matching splits on whitespace and *trims off* `>`, `+`, `~` (`:713-745`), so combinators degrade to descendant; `:hover`/`:active` never match (`:749-751`); `!important` is stripped from the value before comparison (`:654-670`, `:1662-1665`), so importance is discarded entirely; inheritance is a nine-property allowlist (`:515-529`); `inherit` returns the parent's `color` for every property (`:672-682`); `var()` looks only at self and immediate parent (`:683-693`), so a `:root` design token is invisible at depth 2. The shipped fingerprint is honest — `STYLE_ENGINE_VERSION = "stylo-declarations-0.20+oryn-cascade-v1"` (`:43`) — the docs are not. *(A third hand-rolled CSS engine exists in `semantic.rs:308-350`.)*

**Code — box layout:** Block flow is hand-written (`lib.rs:805-1156`). Taffy is entered only for `display:flex|grid` or a floating child (`:985-999`), only for direct children as leaves, and every auto-height leaf is given the placeholder `font_size * 1.2` (`:1252`). `flex-wrap`, `justify-content`, `align-items`, `flex-basis`, `order`, `grid-template-rows`, `grid-column/row` and `min/max-width` are never passed. If Taffy errors, the fallback divides the container into equal columns (`:987-999`).

**Code — inline:** There is no inline formatting context at all. `layout_input` filters the DOM to elements and concatenates an element's direct text children into one string (`runtime.rs:5481-5535`), so text nodes never reach layout and interleaving is lost. `layout_node` shapes that string as a block and starts *all* element children below it — `let child_y = y + border.top + padding.top + intrinsic_text_height;` (`lib.rs:977`) — then stacks each child with `cursor += child_height` regardless of `display` (`:1019-1041`). `text-align` is inherited but never used; alignment is hardcoded `Alignment::Start` (`:242`); `direction`/`unicode-bidi` appear nowhere.

**Why it matters:** For `<p>Sign in with <a>Google</a> to continue</p>` the engine reports the text as a block and places the anchor on the next line at the paragraph's left edge — with `availability: Available` and a full box model. Any flex column, wrapped row, or multi-row grid derives child y-positions from 19.2px placeholders, also `Available`. `nav > a` matches every descendant anchor; `.btn { color: red !important }` loses to any later rule. All of this feeds `getComputedStyle` and, through `display`/`visibility`/`pointer-events`, the interactability gate that decides whether an agent may click. This is exactly the plausible-but-wrong box the layout gate exists to forbid, in the most common agent-relevant constructs, and the milestone is declared complete.

**Recommendation — change the code, and correct the docs immediately.** Build one Taffy tree for the whole box tree with real measure functions so intrinsic heights come from Parley; build a real inline formatting context and map Parley line fragments back onto per-node boxes; drive Stylo's `Stylist` for matching, cascade, inheritance and custom properties. Until each lands, use the `mark_unsupported_subtree` mechanism you already built rather than emitting `Available`. Correct `IMPLEMENTATION_STATUS.md:220` and `CAPABILITY_MILESTONES.md:86` today: block flow is Oryn-owned, Taffy places direct children of flex/grid/float containers only, and there is no inline formatting context. Delete the equal-column fallback in favour of `Unsupported`. Also note `stylo_taffy` is declared in `Cargo.toml:24` and used nowhere.

---

### F4 — High. The act → wait → observe loop is unsound in three independent places
*Merged: the runtime reviewer's lifecycle finding and event-loop finding, and the semantic reviewer's revision finding. All three break the same agent operation.*

**Lifecycle states are fabricated and `wait_for` ignores which one you asked for.** §7.5: "Expose DOMContentLoaded/load/network-idle as separate wait predicates; do not collapse them into one 'page ready' concept." `runtime.rs:3273-3283` pushes all three into the trace as an unconditional literal array immediately after commit, and `:3454-3459` returns `lifecycle: vec![Commit, DomContentLoaded, Load, NetworkIdle]` as a hardcoded constant on every `NavigationResult`. Nothing observes network quiescence. `wait_satisfied` (`:4000-4002`) is `WaitPredicate::Lifecycle(_) => self.document.is_some()` — the wildcard discards the requested state, so `wait_for(NetworkIdle)` is byte-for-byte `wait_for(Commit)`. The executable-document manifest case `commit-domcontentloaded-load-networkidle` is marked passed on an assertion (`runtime.rs:5968`) that reads `first.lifecycle.last() == NetworkIdle` against the constant vector — a tautology that cannot fail.

**`ObservationRevision` does not advance on async DOM change.** `self.revision` increments at exactly three sites: `runtime.rs:3148` (navigate), `:5032` and `:5199` (actions). `wait_for` calls `drain_tasks()` and replaces `self.document` at `:3991-3993` without bumping the revision or calling `record_snapshot()`; `evaluate` does the same at `:4079`. `delta_from` (`:4905-4936`) then diffs the snapshot recorded *before* the wait, so it returns an empty or stale delta — while `observation()` (`:4860`) projects live state, so a full `observe` and `observe --since <same rev>` disagree about the same page. The §12.3 worked example (`click 3` / `wait` / `observe --since 14`, payment-declined dialog) cannot work: anything a page does asynchronously lands with no revision transition and the agent concludes nothing happened.

**The §7.3 event loop is dead code.** Repository-wide grep for `EventLoop` matches only `event_loop.rs` and its two unit tests. The real scheduler is `globalThis.__oryn_drain` (`executable.rs:1158`), called four times unconditionally by `drain_host` (`:641-646`) with no quiescence check. `globalThis.queueMicrotask = fn => microtasks.push(fn)` (`:1152`) is a plain array, disjoint from V8's queue, which drains only at `perform_microtask_checkpoint()` at the end of each `execute` (`host.rs:1063`) — so `.then` continuations and MutationObserver/custom-element callbacks drain in the wrong relative order versus any real browser. `setInterval` is aliased to a single `setTimeout(fn, 0)` and fires once (`:1155`); `rAF` is `setTimeout(…,0)` and `performance.now()` returns constant 0 (`:1156`); `fetch`/`XHR.send` resolve by calling the *synchronous* `__oryn_fetch_sync` inside a microtask (`:1141`, `:1148`), so "async networking resolves back into the page task queue" is not what happens. Among pending nonzero-delay timers the advance path picks registration order, not earliest deadline. There is no mutation-observer delivery step, no custom-element reaction queue, no style flush and no semantic flush in the loop.

**Recommendation — change the code.** Derive `DomContentLoaded`/`Load` from real milestones and `NetworkIdle` from broker in-flight quiescence, and make `wait_satisfied` compare the requested state; mark the manifest case unmet until then. Bump the revision and `record_snapshot()` whenever `drain_tasks`/`evaluate` replaces the document (or make `delta_from` project live state). Route `queueMicrotask` into V8's real queue, make timers deadline-ordered, make `setInterval` repeat, and either drive the loop from the Rust `EventLoop` with the §7.3 phase order or delete `event_loop.rs` and stop implying the ordering exists.

*Note: the lifecycle and revision findings are the two the adversarial verifier did not reach. The code citations are specific enough to check in minutes; do that before acting.*

---

### F5 — High. Gate evidence records case results that were never executed
*Merged: six findings across the runtime, DOM, layout and interfaces dimensions. One systemic problem in how gates are closed.*

| Artifact | What it claims | What produced it |
|---|---|---|
| `benchmarks/evidence/layout-v1/result-v1.json` | 33 cases, 28 required passing, layout closed | `scripts/build_layout_result.py:110-133` writes `"status": "passed"` into every case unconditionally; `metrics`, `regressions`, all seven `criteria` booleans and `outcome.status` are hardcoded literals (`:189-227`). The manifest carries `"baseline": "missing"` for all 33 cases and no case id appears in any test source. `validate_layout_result.py:95-108` "recomputes" by asserting the ledger says passed. The only real check is `validate-layout-differential.sh:26-33`: seven booleans and one width within 1.0 px — `text_has_fragments` is `getClientRects().length > 0`, which F3's missing inline formatting context passes trivially. |
| `benchmarks/evidence/runtime-foundation-macos-arm64.json` | Runtime foundation passed; `concurrent_pages: 20`, `peak_rss_bytes: 4587520` | `scripts/benchmark-native-runtime.sh:12` builds `--features in-process-probe`, which does not imply `v8-host` (`Cargo.toml:8-11`); `lib.rs:13-14` gates `executable` on `v8-host`. No isolate is ever created. CI even asserts `native_v8 == false` for this build (`.github/workflows/ci-rust.yml:143-146`). The real V8 figures in the same repo: 47 MB worker, 56 MB RSS — for one page (`IMPLEMENTATION_STATUS.md:275-279`). |
| `benchmarks/evidence/executable-document-external-validation.json` | html5lib subset at pinned revision `224991ec…`, `tokenizer/test1.test`, 4 vectors passed | `html.rs:304-329` is four hand-written assertions over one string, self-described as a paraphrase. No html5lib fixture exists in the repo. The file also cites `benchmarks/conformance/g1-manifest-v1.json`, which does not exist. |
| `benchmarks/evidence/execution-domains-v1/result-v1.json` | Execution-domain selections and a Chromium handoff, under a signed-worker fingerprint (`native_v8: true`, `sandbox_state: app_sandbox`) | The probe runs `oryn-domain`'s example, whose dev-dependency unifies `in-process-probe` into the build (`crates/oryn-domain/Cargo.toml:23-24`; confirmed via `cargo tree`), so `uses_worker_process()` is false and there is no V8. The Chromium adapter under test is `ProbeBackend`, an in-memory stub returning `Ok(true)` (`execution_domain_probe.rs:21-60`). The signed `--runtime-info` block is merged in from a separate real invocation (`build_execution_replay_results.py:111-118`). `IMPLEMENTATION_STATUS.md:339-341` describes this as "the signed production probe". |
| Execution-domain case `native-dependency-purity` | The §21.2 boundary is guarded | `validate_execution_replay_architecture.py:60-77` intersects `cargo tree --prefix none` output lines with bare crate names. Lines read `oryn-h v0.1.0 (/path)`, so the intersection is always empty and the check can never fail. The invariant currently holds; the guard is a no-op. |
| Executable-document case `commit-domcontentloaded-load-networkidle` | Lifecycle states distinguished | Tautological assertion against a constant — see F4. |
| Security case `page-content-remains-data` | §14.4 prompt boundary holds | See F12: no implementation exists; evidence is two log files and no assertion in the probe. |

**Why it matters:** These are the gates the proposal uses to authorize the next phase of work. A gate that cannot fail is worse than an open gate, because it removes the signal that would have told you to stop.

**Recommendation — change the code.** Bind each manifest case id to an executed assertion; populate `baseline` with expected values; derive `case_ledger`, `metrics`, `criteria` and `outcome` from raw artifacts rather than literals. Re-run the runtime-foundation spike against the `v8-host` worker path and republish with per-page process count, aggregate RSS and termination isolation, or relabel the existing numbers a no-JS control and mark the §22 N-page criterion open. Fix the tree-line parsing in the purity check and add a negative test. Either vendor the html5lib fixtures or stop citing a pinned revision.

---

### F6 — Resolved in implementation; release evidence pending. Parent-enforced cross-origin, SameSite, mixed-content, and CSP policy

The original finding was valid for the reviewed revision, but the implementation
has since replaced each affected authority boundary:

- Fetch/XHR preflight and CORS response filtering run in the parent context
  service before denied response bytes enter the worker. Worker `Origin`,
  `Referer`, `Cookie`, and forwarding headers are stripped and authoritative
  request provenance is derived from the parent page registry.
- `ContextCookieStore` is an Oryn-owned RFC6265bis store. It accepts an explicit
  site-for-cookies/request context, uses schemeful PSL sites, implements
  Strict/Lax/None plus Lax-by-default and the bounded Lax-allowing-unsafe window,
  taints cross-site redirect chains, enforces Secure/HttpOnly/prefix rules, and
  persists only parent-owned records. reqwest no longer has its cookie feature.
- `PageNetworkBroker` compiles response-header CSP before returning a document,
  derives network destinations, enforces redirects and `frame-ancestors`, and
  validates the worker's policy snapshot. Meta policies are extracted from the
  accepted parent response, consumed in document order, and applied
  conservatively before the next effect if a worker omits its parser checkpoint.
- The native CSP profile implements directive fallback/intersection,
  nonce/hash/strict-dynamic decisions, inline script/style attributes,
  base/form/frame/connect policy, mixed-content rejection,
  `upgrade-insecure-requests`, sandbox action restrictions, report-only trace
  records, and unforgeable core Trusted Types values and sinks.

The `csp3-agent-browser-2` profile closes the former partial boundaries:
violation reports are redacted, bounded, and delivered by the parent through
modern and legacy endpoints; generated `SecurityPolicyViolationEvent` records
reach the affected document; sandboxed documents receive unique parent-owned
opaque origins; and worker, media, object, and manifest requests exercise their
actual CSP directive fallbacks. `security.csp` is therefore supported for the
native platform surface. Dedicated-worker script loading is parent-brokered and
`worker-src` complete; worker code now executes in a persistent independent V8
context with realm-owned message queues rather than a page-realm compatibility
scope. Media decoding, plugin execution, SharedWorker, ServiceWorker, WebRTC, and
ReportingObserver remain separately declared capabilities. Release attestation
still requires a clean signed-worker policy run publishing new compact evidence.

---

### F7 — High. The action policy gates an inert action; the path that actually submits is ungated, and no hook re-runs on redirect

**Proposal:** §14.3 — policy hooks "at the point where actions become consequential: form submission, navigation to sensitive schemes/origins, file upload, download, clipboard access, credential fill". §11.4 — "repeat checks on redirects".

**Code:** `runtime.rs:2722-2740` fires the `FormSubmission` hook only when `action.action == SemanticAction::Submit`. That compiles to `node.dispatchEvent(new Event('submit',{bubbles:true}))` (`executable.rs:568`) — a bare event that never reaches the shim's `submitForm`. The path that actually submits is `Click` → `node.click()` (`executable.rs:558`) → `:871`, which for `<input type=submit>` and `<button>` calls `submitForm(form)` (`:1115`) → `navigateTarget`, with no `FormSubmission` check. `goto_authorized` checks `CrossOriginNavigation` only against the initially requested URL (`runtime.rs:2244-2264`); the redirect loop (`:3361-3420`) re-brokers each hop for *network* policy but never re-runs action policy, so a redirect from an allowed origin to a `denied_origins` entry passes. `ConsequentialAction::FileUpload` and `::Clipboard` have zero call sites outside the security probe. `IMPLEMENTATION_STATUS.md:284-289` lists all nine actions under "security — complete", and the gate case `form-policy-hook` is marked passed citing a probe that contains no form assertion at all.

**Why it matters:** A deployment that sets `FormSubmission: RequireConfirmation` believing it has a human in the loop gets no prompt when the agent clicks "Confirm transfer" on a same-origin form. Also: `submitForm` only encodes fields for `method=get` (`executable.rs:1115`), so a POST form silently becomes a bare GET.

**Recommendation — change the code.** Route every submission through one host-side chokepoint that runs the `FormSubmission` hook regardless of the initiating semantic action; re-run `CrossOriginNavigation` on every redirect hop whose origin changes; wire `FileUpload`/`Clipboard` to real capabilities or remove them from the enum and from the completeness claim.

---

### F8 — High. html5ever is used for exactly one thing — the initial document parse
*Merged: the DOM reviewer's fragment-parser finding and streaming/re-entry finding.*

**Proposal:** §9.3 — "Use html5ever as the tokenizer/tree builder and implement its TreeSink directly over Oryn DOM… Streaming parsing matters because script execution can pause parsing, mutate the document and resume." P6 — "Do not build a… CSS parser… unless a specific gap forces it."

**Code:** `html::parse` is one-shot: `parse_document(OrynTreeSink::new(), ParseOpts::default()).one(input)` (`html.rs:21-23`), with no `TendrilSink::process` and no script hooks on the sink. `ExecutableDocument` parses to completion, then collects every script from the finished tree (`executable.rs:321-337`), sorts classic-before-module (`:339`) and runs them (`:344-424`). `document.write`/`writeln` do not exist. Scripts inserted at runtime are never executed — `appendChild` (`:857`) has no script-insertion path — and no diagnostic is emitted. And `innerHTML` does not use html5ever at all: `parseFragment` (`:990`) is `html.match(/<!--[\s\S]*?-->|<![^>]*>|<\/?[A-Za-z][^>]*>|[^<]+/g)` with a stack that pops on any `</` regardless of tag name, a six-tag void list, no entity decoding, no RCDATA, no foster parenting. The matching `serializeNode` (`:989`) escapes only `"` in attribute values and **nothing** in text.

**Why it matters:** Two concrete failures. (1) Silently inert dynamically-inserted `<script>` is a mainstream pattern — analytics, lazy chunks, consent managers, third-party widgets — and it fails with no capability diagnostic, so the agent sees a page that merely looks incomplete. That is a P7 violation, and the executable-document exit criterion in §22 explicitly names "streaming HTML, scripts/modules" while the evidence declares it passed. (2) A security-relevant round trip: `el.textContent = '<img src=x onerror=…>'` then the ubiquitous `parent.innerHTML = parent.innerHTML` yields, in a real browser, unchanged text — here it yields a live element, and `setAttribute` compiles `on*` attributes eagerly with `new Function` (`:874`). Any sanitizer validated against a real browser can be bypassed under Oryn, which matters because P8 declares every page hostile.

**Recommendation — change the code.** Route fragment parsing through html5ever's fragment mode across the host boundary and fix `serializeNode` to escape `&`, `<`, `>` in text and `&`, `"` in attributes — this is a correctness and safety defect in shipped behavior, not later security work. Emit an `Unsupported` diagnostic when a script element is inserted post-load (the machinery is at `executable.rs:782`). **Change the proposal** to state that streaming and parser re-entry are deferred, rather than citing them as the reason for the dependency.

---

### F9 — High. P7 capability honesty holds only on the typed Rust API
*Merged: the DOM reviewer's fake-APIs finding and three layout findings (zero rects, overflow clipping, aria-hidden).*

**Proposal:** §9.5 — "Oryn2 should… classify each interface as supported, partial, intentionally absent or fallback-required. Partial APIs should… throw an explicit, diagnosable error for unsupported operations." §4.2 non-goal: "Pretending unsupported APIs work through inaccurate stubs."

**Code:** The Rust path is exemplary (see §2). The JS surface is not.
- `getBoundingClientRect()` is `__orynRect(geometry && geometry.box_model && …border_box)` (`executable.rs:926`) and `__orynRect` coerces a missing value to an all-zero rect (`:839`). A table cell — marked `Unsupported` with `box_model: None` by `mark_unsupported_subtree` — and any node created since the last `refresh_layout` both read `{0,0,0,0}` with no error and no diagnostic. No `availability` field is exposed to page JS at all.
- `OrynIntersectionObserver` and `OrynResizeObserver` (`executable.rs:1044-1046`) store the callback and never invoke it — nothing anywhere fires them. `globalThis.confirm = () => true` (`:1033`) auto-accepts confirmation dialogs, which under P8 and the high-consequence-action policy is a silent approval of exactly the interactions that were supposed to be gated. None of these — nor `alert`, `prompt`, `document.write`, or the fragment parser — appears in `capabilities()` (`runtime.rs:4616-4712`), which classifies only layout/paint/frames/service-worker/WebRTC/XHR/IndexedDB.
- **Overflow clipping is ignored entirely.** `overflow-x`/`overflow-y` exist only as UA defaults (`oryn-layout/src/lib.rs:594-595`) and are read nowhere. `in_viewport` is `rect.intersects(viewport_rect)` against the viewport alone (`:1087`), `interactable` follows from it (`:1143-1147`), and `scroll_into_view` only moves document scroll (`runtime.rs:4339-4352`). An option scrolled out of view inside an `overflow:auto` listbox is reported interactable, the click is authorized, and `scroll_into_view` scrolls the wrong thing — hitting exactly the dropdown/combobox/virtualized-list patterns agents use most. A wrongly-interactable element is as harmful as a wrong box.
- `aria-hidden="true"` is treated as `display:none`, suppressing geometry for the node and its whole subtree (`oryn-layout/src/lib.rs:822-848`). ARIA has no effect on layout in any browser; a component library that marks a visible modal wrapper `aria-hidden` while managing focus loses its box.

**Recommendation — change the code.** Distinguish "no box" from "box unavailable" in the installed snapshot and make the JS binding surface the unavailable case observably. Route `confirm()` through host policy or throw. Either drive the observers from the layout snapshot or register them `Unsupported` in `capabilities()`. Clip `in_viewport` against the ancestor clip chain and make `scroll_into_view` scroll the nearest scrollable ancestor. Stop letting `aria-hidden` suppress geometry. **Change the proposal** to add overflow clipping alongside table layout in §10.2 as something that must be implemented or declared, and to require the per-interface classification table §9.5 asks for — it does not exist anywhere.

---

### F10 — High. §7.1/§7.2/§7.4 concurrency and realm model: no pool, blocking async façade, isolate-per-frame
*Merged: the runtime reviewer's worker-pool finding and frame-isolate finding.*

**Proposal:** §7.1 gives `Browser` a "worker pool"; §7.2 — "one active PageRuntime / isolate per top-level Page, scheduled on a worker pool; PageHandle remains Send + Sync and queues typed operations". §7.4/§8.3 — "one V8 isolate per top-level Page and one V8 Context per JavaScript realm/frame."

**Code:** `pub struct Browser { config, worker_process }` (`runtime.rs:66-70`) — no pool, queue or scheduler. In-process: `std::thread::spawn` per page with `blocking_recv` (`:1955-1975`). In production (`uses_worker_process()` true on all non-test unix builds, `:1236`): one child process plus two named parent threads per page (`worker_client.rs:149-167`). The async façade is not async: `load_html`/`goto` call `worker_request` directly in the `async fn` body with no `spawn_blocking` (`runtime.rs:2176-2216`), and `WorkerPage::request` holds a `std::sync::Mutex` guard across the entire blocking exchange (`:430-475`), bottoming out in a `libc::poll`/`libc::read` loop that can run the full 30 s navigation timeout (`worker_client.rs:412-490`). So N "concurrent" pages serialize on a `current_thread` runtime — which is what the runtime-foundation probe itself uses (`bin/native_probe.rs:19`) — and consume N tokio worker threads on a multi-thread one. Separately, each same-origin iframe is a whole separate `OwnedIsolate`, entered via `unsafe isolate.enter()` (`host.rs:152-163`, `executable.rs:547-549`), created with the *same* `max_heap_bytes` the page got (`executable.rs:213`), recursively to depth 4 — so the advertised 256 MiB per-page cap does not bound a page. The in-source comment at `executable.rs:22-23` claims "independent V8 contexts" while creating independent isolates, and `contentDocument` is a JSON mirror re-installed from the parent (`:1163`, fed by `:576-605`), so parent writes into it are discarded at the next sync and `contentWindow` is a stub object.

**Recommendation — change the code for the async façade** (move the blocking exchange behind `spawn_blocking` or an async transport; `PageHandle` is not async today and the §7.2 contract says it is). **Change the proposal for the rest:** §7.1/§7.2 should describe process-per-page and state its concurrency and OS-resource ceiling explicitly, and §7.4/§8.3 should either commit to context-per-frame or record isolate-per-frame with its heap-cap multiplication and loss of cross-frame object identity as accepted limits. Correct the in-source comment either way. §24 still lists both "Page isolate granularity" and "Cross-site iframe isolation" as open — close them deliberately rather than by drift.

---

### F11 — Medium. Retained state grows without bound in three places
*Merged: the runtime reviewer's snapshot finding, the semantic reviewer's snapshot finding (same defect), the DOM reviewer's arena-slot finding, and the network reviewer's resource-cache finding.*

`PageState.snapshots: BTreeMap<u64, Vec<SemanticNodeView>>` (`runtime.rs:3055`) stores a full `project_document` per revision (`:4896-4903`) on every navigation *and* every action (`:3265`, `:3314`, `:5080`, `:5260`). Exhaustive grep returns only the declaration, init, one insert and two gets — no `remove`, `retain`, `clear` or `split_off` anywhere. Each entry is owned `String` role/name/description/value/selector per node. Arena slots are similarly never reclaimed: JS mints ids from a monotonic counter never reset (`executable.rs:1159`), `prepare_rebuild` copies `previous.slots` one-for-one and never shrinks (`dom.rs:55-83`), and `restore` grows the vector pushing intermediates with `generation: 0` without adding them to `free` (`:86-108`) — so the `free` list is effectively never popped and storage grows with every node ever created. `resource_cache` (`runtime.rs:3064`) is an unbounded `BTreeMap<String,String>` with no eviction, size cap or Cache-Control handling — the inverse of §11.6's "memory-bounded HTTP resource cache".

The only backstop is the parent's 512 MiB RSS watchdog, and hitting it is not graceful: `worker_client.rs:433-439` kills the worker, `runtime.rs:2180-2192` marks the page crashed, and the replacement starts blank with a new high-order generation — invalidating every semantic reference the agent holds, mid-task. Against §5's P50 ≤ 64 MiB / P95 ≤ 160 MiB per active page runtime, and against §12.3's long-horizon control loop, this is unmeasured: no benchmark exercises sustained churn.

**Recommendation — change the code and add the bound to the proposal.** Cap retained revisions (a small ring plus any checkpointed revision) and return the `UnknownRevision` error that already exists; reuse or compact arena slots at `prepare_rebuild`; give `resource_cache` a byte cap and a context-policy switch. §12.3/§16.4 should state an explicit retention contract so agents know how far back a delta base may be — the proposal assumes deltas replace retained state rather than requiring it.

---

### F12 — Medium. The §14.4 untrusted-content boundary is not implemented, and the gate case asserting it is marked passed

`grep -rn untrusted crates/oryn-native/src crates/oryn-common/src` returns nothing. The v4 `Observation` (`oryn-common/src/v4.rs:262-279`) carries page-controlled strings inside `PageInfo` and `SemanticNodeView` as unlabeled siblings of browser-owned `revision`/`capabilities`/`diagnostics`/`execution_domain` — a structural separation exists, but nothing marks which side is page-authored. `Provenance` is a derivation axis (`BrowserDeterministic | Heuristic | Model`), not a trust axis, and is a compile-time constant: three construction sites in the workspace, all `BrowserDeterministic` (`semantic.rs:231`, `v3.rs:426`, `v2.rs:303`). Meanwhile `SECURITY_POLICY.md:13` asserts "Web content remains structurally untrusted and never enters the host instruction channel", and `benchmarks/evidence/security-v1/result-v1.json` records `page-content-remains-data` as passed on two log files with no corresponding assertion in `examples/security_policy_probe.rs`.

Relatedly, genuinely approximate values ship labeled deterministic: the projection's `Visible`/`Hidden` comes from the third hand-rolled CSS engine (`semantic.rs:308-355`) and the accessible name falls back to `placeholder` then raw `text_content` (`:67-68`), well past accname. *(The reviewer's "two disagreeing visibility oracles" claim is partly refuted: `observation_v4` at `runtime.rs:4576-4610` reconciles against the Stylo/Taffy layout. The default v3 `observe` path does not.)*

**Recommendation — change the code.** Add a real trust axis (`TrustLabel` on string-bearing fields, or a `page_provided { … }` envelope) — this is the single control §14.4 names against prompt injection and it is currently convention. Label CSS-derived visibility and non-accname fallbacks `Heuristic`. Mark `page-content-remains-data` not-implemented until an executable assertion exists; certifying an unimplemented control is worse than leaving the gate open.

---

### F13 — Medium. `fill_secret` reflects the plaintext credential into a content attribute, and observation redaction is keyed on a page-controlled attribute
*Merged: two network/security findings.*

`runtime.rs:2820-2832` materializes the secret and passes it as `NativeAction.value`; `executable.rs:559` compiles that to `node.value = <plaintext>`; the shim's setter at `:952` is `set value(v){ … this.setAttribute('value', v); }` — reflecting into the *content* attribute, which no real browser does — and `setAttribute` (`:874`) fires a MutationObserver record. The secret therefore lands in `__attrs`, in `outerHTML`, in the Rust arena after the next sync, and in any attribute-carrying serialization. Separately, `semantic.rs:217-231` redacts `value` only when `local_name == "input"` and `type == "password"`; a secret filled into a `type="text"` control (API key, TOTP) is emitted verbatim into every observation the agent and its model see. The security probe's fixture is exactly the one shape that works (`security_policy_probe.rs:110`), asserting only `!serialized.contains("never-observe-this")`, while `SECURITY_POLICY.md:33-34` and `IMPLEMENTATION_STATUS.md:290-294` make the broad claim.

*Two reviewer claims corrected: page script could read a filled credential in a real browser too, and the origin scoping / confirmation token / zeroization are **not** bypassed — `require_policy` and `materialize(handle, &origin)` still run (`runtime.rs:2810-2821`). The show/hide-toggle bypass also does not work as described, since the shim defines no `type` IDL accessor.* The genuine delta over a real browser is the attribute reflection, the mutation record, DOM serialization, and the arena/observation exposure.

**Recommendation — change the code.** Stop reflecting the `value` IDL setter to the content attribute (correct web-platform behavior anyway), suppress the mutation record for secret fills, and track secret-ness on the node from the `fill_secret` operation rather than re-deriving it from a page attribute at projection time. Extend the probe to a `type="text"` fill.

---

### F14 — Medium. `oryn-webidl` is a parser with three string constants and zero consumers; the DOM hierarchy is flat

§8.5 makes the generator "a named architecture workstream, not a convenience script", to be prototyped "before broad DOM API implementation so hundreds of interfaces do not accumulate bespoke glue", and the §17 risk register names it a project-scale bottleneck. `crates/oryn-webidl/src/lib.rs` is 519 lines of string-splitting whose only emitter, `emit_rust_metadata` (`:175-192`), produces `INTERFACE_NAME`, `PARENT_INTERFACE` and a `MEMBERS: &[&str]`. Workspace-wide grep finds it only as a member line in `Cargo.toml:16` and a README mention — no dependency edge, no build script. The ordering §8.5 exists to enforce has already been violated: a large handwritten JS surface exists and the generator does not. And `executable.rs:1034-1035` sets `globalThis.HTMLElement = Node = Element = DocumentFragment = HTMLIFrameElement = HTMLInputElement = … = OrynNode` — no inheritance at all, which breaks prototype patching, `instanceof` narrowing and `Object.getPrototypeOf` chains in framework and polyfill code.

**Recommendation — change the docs now, the code at a named gate.** `README.md:153` ("WebIDL parser/generator spine") and `REUSE_OWNERSHIP.md:23` should say "WebIDL parser only, not wired to any binding output" — `IMPLEMENTATION_STATUS.md:27` already does, standardize on it. **Change the proposal** to record that the generator slice slipped behind the API surface it was meant to precede, and track the risk as materialized. The flat prototype chain should be fixed regardless of the generator's schedule.

---

### F15 — Medium. The production execution path has no unit-test coverage

`uses_worker_process()` includes `!cfg!(test)` (`runtime.rs:1236`), so every one of the ~50 tests in `runtime.rs` and 17 in `executable.rs` takes the in-process branch — the profile `IMPLEMENTATION_STATUS.md:249-250` says is "excluded from production artifacts". `crates/oryn-native` has no `tests/` directory, so nothing compiles the crate with `cfg(test)` unset. CI does cover signing, launch and handshake rejection on macOS (`.github/workflows/ci-rust.yml:116-148`), but `WorkerRequest`/`WorkerResponse` mapping, correlation handling, frame limits, the RSS watchdog and crash-and-replace generation renumbering — all the behavior most likely to diverge — have none. §6.3's "same PageHandle API, different isolation profile" assumes the two profiles are interchangeable and says nothing about proving it.

**Recommendation — change the code, and add one line to §6.3.** Add an integration-test target (which compiles the lib without `cfg(test)`) running a representative slice against the worker path on macOS and Linux, plus a differential test asserting both profiles produce identical observations and revisions for the same script. §6.3 should state that profile equivalence is a tested contract.

---

### F16 — Medium. No typed Intent IR; the SDK ships OIL text; two named §13 OIL features missing
*Merged: two interfaces findings.*

> **Superseding implementation update (2026-08-30):** This finding describes
> the assessed historical commit, not the current working tree. The shared
> typed `Intent`/`Target` IR, `PageHandle::perform`, `observe --since`, and
> `$secret:<label>` OIL path now exist and reuse the native resolver/action
> engine. Typed Python transport and the first-party `oryn-mcp` adapter now use
> that same envelope, closing the software defect. Exact release attestation
> remains separate.

§13.1 specifies `enum Intent`/`enum Target` and Appendix A specifies `PageHandle::perform(intent)`. Neither exists. The typed surface is `NativeAction { target: SemanticRef, action, value }` (`runtime.rs:3035-3039`) — target is a resolved ref only, with no `Semantic(SemanticQuery)` or `Css` arm; CSS is a separate method (`:2389`). Semantic resolution lives in the OIL session (`oil.rs:954-1043`), so any non-OIL frontend must reproduce that glue or shell out to OIL text — which is what `oryn-python` does (`client.py:100-122` over stdin). No MCP adapter exists, and no gate owns one. Separately, §13.3 makes `observe --since <rev>` first-class and the §13.3 worked example uses `$secret:` — `ObserveCmd` (`oryn-core/src/ast.rs:131-140`) has no `since` (only an implicit session cursor, `oil.rs:24,427-432`) and the canonical grammar has no `$secret:` token, so `fill_secret` is Rust-only and the printed OIL example does not parse. `IMPLEMENTATION_STATUS.md:82` claims revision-delta projections are wired through OIL flags.

**Recommendation — pick one and write it down.** Either amend §13.1/Appendix A to the method-per-verb API that shipped (defensible — the parent↔worker wire format is already typed, which is the important half) and move `Target::Semantic` resolution into a shared crate both OIL and future adapters call; or introduce the IR *before* an MCP adapter is written so the adapter is not built on the OIL text path. Independently, **change the code**: add `--since <rev>` routed to the existing `observe_delta`, and a `$secret:<label>` form dispatching to `fill_secret` — without it the §13.4 redaction guarantees are unavailable to exactly the callers most likely to put a password in a prompt.

---

### Low severity

| # | Finding | Evidence | Action |
|---|---|---|---|
| L1 | No V8 startup snapshot; the 94 KB bootstrap is recompiled per realm per navigation | No `SnapshotCreator`/`snapshot_blob` under `crates/`; `host.rs:228-246`, `executable.rs:262` | Change code — §16.4 commitment whose payoff grows with the isolate-per-frame model |
| L2 | V8 heap sandbox not compiled in; the security narrative doesn't say so | `Cargo.toml:44` has no `features` key; `v8_enable_sandbox` is a real opt-in feature | Keep as deferred security work, but record it in `SANDBOX_WORKER_SECURITY.md` — "App Sandbox + allow-jit" reads stronger than it is |
| L3 | Termination is per-`execute()` wall clock; no cancellation token exists; `terminate()` is never called; heap cap yields a V8 OOM abort not `LimitExceeded`; RSS watchdog forks `/bin/ps` ~120×/navigation | `host.rs:82-99`, `:283` (hardcoded 9.5 s), `:1099`/`:1198` uncalled; `worker_client.rs:832-840` | Change code — Appendix A's "cancellation tokens are accepted by long-running operations" is a concrete unmet rule |
| L4 | ADR 0001's "stripped probe size" compares different feature sets; neither probe carries the §16.5 payload; a cited manifest path doesn't exist; ADR and evidence JSON disagree on two numbers | `bin/host_probe.rs:20-31` vs `bin/deno_host_probe.rs:20-31`; `Cargo.toml:10`; `executable-document-external-validation.json:8` | Change docs — relabel the size row and fix the path |
| L5 | Renderer re-shapes text with fontdue per-character advances instead of consuming Parley's shaping; web-font text is measured with the brokered font and painted with Noto Sans | `oryn-common/src/v4.rs:29-35`, `oryn-render/src/lib.rs:326`, `:476-518` | Change proposal — §10.3 should say the renderer re-shapes, and record the divergence; §24 still has renderer tech open |
| L6 | Three unpkg.com URLs are silently rewritten to pinned versions in the shipping fetch path | `runtime.rs:5845-5861`, called unconditionally from `:3824-3844` | Change code — move behind an explicit `ContextOptions` field and record substitutions in the trace; the React-heavy corpus partly measures three hardcoded deps |
| L7 | Raw `evaluate`/`resolve_selector`/`extract` have no policy gate or audit record | `runtime.rs:2354-2371`, `:2389`, `:2417`; `ConsequentialAction` (`security.rs:115-125`) has no script variant | Change code — Appendix A's "raw CSS/JS/debug access is explicit and separately permissioned" is the one design rule inverted rather than reshaped |
| L8 | IPv6-literal URLs can never be fetched: `host_str()` returns the bracketed form, which neither `IpAddr::parse` nor `getaddrinfo` accepts | `network.rs:167-190` | Change code — fails closed, but the carefully-built IPv6 branch of `is_private` (`security.rs:418-427`) is dead for literals and never exercised by the gate |
| L9 | Interactability rejections and layout events carry `action_id: None` | `ensure_interactable` runs before `next_action_id` increments; `runtime.rs:5119-5125`, `:5147-5155`, `:4157-4180` | Change code — "why did click 17 not fire" is the query the trace exists for |
| L10 | Doctype public/system ids dropped; attribute prefixes collapsed; template contents orphaned; no `ShadowRoot` node kind | `html.rs:145-152`, `:184-198`, `:110-118`; `dom.rs:13-21` | Change proposal to record the flattening boundary; change code to carry `ns` (see F1) |

---

## 4. Proposal amendments for v0.3

Where the implementation's choice was the better one, or where reality should simply be written down.

1. **§6.3 / §7.1 / §7.2 — describe process-per-page.** The hardened worker (nonce handshake, codesign + SHA verification, `env_clear`, exact-entitlement check, fail-closed kill) is stronger than the proposal's "same API, different profile" sketch and is the shipped default. Replace "worker pool" with the real model, state the concurrency ceiling and the 1 process + 3 threads per page cost, and add pooling as named future work rather than implied present state. Add: *profile equivalence is a tested contract, not an assumption* (F15).
2. **§10.2 — promote "refuse, don't approximate" to a named mechanism.** `mark_unsupported_subtree` + `UnsupportedGeometry` + `ensure_interactable` refusal is the best pattern in the codebase. Name it, and require it for every unsupported construct — inline formatting, overflow clipping, `Unsupported` boxes reaching page JS. Add overflow clipping and scroll containers alongside table layout as "implement or declare".
3. **§11.3 — rename the seam.** `NetworkBroker` (`network.rs:44-46`) already is the swappable transport abstraction with four implementations; `HttpTransport` is a naming difference, not a missing layer. Amend §11.3 to name `NetworkBroker`, and keep only the real requirement: Oryn-owned cookie types so `reqwest` stops appearing in public signatures (`network.rs:48`, `:212`) — which is also the prerequisite for SameSite (F6).
4. **§18.3 — record that cookies are deliberately excluded and restore is archive-backed.** `replay.rs:251/259` scrub cookie headers by design and `REPLAY_AND_CHECKPOINTS.md:5-6` documents it; restored pages are served from the recorded archive, so the "logged-out restore" failure mode does not occur. Amend §18.3 to state the policy explicitly, and add one thing the code should gain: an explicit loss list on `CheckpointRestore` (dropped form fields, dropped secrets, excluded cookies) so a lossy restore is visible rather than inferred.
5. **§12.2 — make identity-only the v1 contract.** No `SemanticFingerprint` exists and no matcher is planned soon; the conservative posture (false preservation structurally impossible, 0/5) is the right call and §24 still lists the matcher as open. State that v1 is exact-identity-only, that survival across framework re-render is expected to be near zero, and that the token cost of re-observation is the accepted trade. Then report the honest survival rate against a real churn corpus (F2).
6. **§12.3 / §16.4 — add a retention contract.** State how far back a delta base may be and what happens beyond it (`UnknownRevision`). The proposal assumes deltas *replace* retained state; the implementation needs them to *bound* it (F11).
7. **Appendix B — promote what shipped better.** `CapabilityDiagnostic { capability, support, alternatives, handoff_lossy }` with real named alternatives (`runtime.rs:4616-4730`), the `DomainReasonCode` set (`execution.rs:160-270`), and the `HandoffLoss` list (`oryn-domain/src/lib.rs:200-250`) are richer and better specified in code than in the appendix. Bring the appendix up to the code.
8. **§8.2 — name the feature-isolated-probe pattern.** Keeping `deno_core` behind a `deno-host` feature rather than as a runtime dependency, with both hosts pinned to the same V8 revision and an in-source comment explaining why (`oryn-native/Cargo.toml:42-43`), is a reusable method for the remaining §24 open decisions. Write it down as the standard for resolving them.
9. **§13.1 / Appendix A — decide.** Either adopt the shipped method-per-verb API as the contract (and move `Target::Semantic` resolution into a shared crate), or keep the Intent IR and build it before the MCP adapter. Do not leave both descriptions standing (F16).
10. **§7.4 / §8.3 — close the realm decision deliberately.** If isolate-per-frame stays, say so and record the heap-cap multiplication and the loss of cross-frame object identity as accepted limits; §24 currently lists both isolate granularity and cross-site iframe isolation as open, and the code has already chosen (F10).

---

## 5. Open risks

Tied to the proposal's own gates and risk register.

- **Runtime foundation is not closed against the proposal.** §7.2 calls N-isolate concurrency "an Oryn2 hypothesis, not a borrowed proof" that "must be validated in an early load spike". The spike has not been run with a JavaScript engine. The only real datapoint in the repo is 47 MB installed / 56 MB RSS for a single page. Throughput under JS-heavy and network-heavy load, and termination isolation across concurrent isolates, are unmeasured; no Lightpanda/Obscura comparison exists. Until this is re-run, every downstream capacity and packaging claim is unfounded.
- **Executable document is marked passed on unmet criteria.** §22's exit explicitly names "streaming HTML, scripts/modules" — parsing is one-shot, `document.write` is absent and dynamically inserted scripts are silently inert (F8); lifecycle predicates are constants (F4); the html5lib subset is a paraphrase (F5).
- **Application compatibility is at material risk** from the microtask-queue split, `setInterval` firing once, `performance.now()` returning 0, inert dynamic scripts, measure-after-mutate returning pre-script geometry (`refresh_layout` runs *after* the script returns, `runtime.rs:4097`), `Node === Element === HTMLElement`, and the regex `innerHTML` parser. These are not independent bugs; they are the same corpus failing for six reasons.
- **Layout evidence cannot detect an untruthful box.** The gate that certifies geometry truthfulness compares seven booleans and one width, and its ledger is written by a script that always says "passed" (F5). No absolute coordinate, line count, wrapped-line rect or flex/grid child offset is ever compared to Chromium.
- **The security narrative is ahead of the code.** SameSite, CSP, mixed-content and preflight are absent (F6); the §14.4 prompt boundary is unimplemented while its security case is passed (F12); the V8 heap sandbox is not compiled in and no doc records it (L2). `SANDBOX_WORKER_SECURITY.md`'s deferred list is incomplete on all four counts.
- **The execution-domain challenge-rate criterion has no implementation.** No detector, no schema field, no baseline — and the canary evidence labels native parse failures as challenges, which is the specific inversion §5.2 forbids (F2).
- **The §17 named risk has already materialized untracked.** WebIDL generation was called a project-scale bottleneck and required to precede broad DOM work; a large handwritten JS surface exists and the generator has three string constants and no consumers (F14).
- **The §5 memory targets (P50 ≤ 64 MiB, P95 ≤ 160 MiB per active page runtime) are unmeasured under the conditions that would break them.** Semantic snapshots, arena slots and the resource cache all grow monotonically for a page's lifetime, and the only backstop is a kill that invalidates every semantic reference the agent holds (F11). No benchmark exercises sustained DOM churn — the §14 `dom-churn` suite is the right place and does not exist in executable form.
- **Profile equivalence is unproven.** The production worker path — protocol mapping, correlation, limits, crash-and-replace generation renumbering — has no unit-test coverage at all (F15). A regression there passes the entire suite.

---

## 6. Broader product gaps not captured by individual findings

The findings above concentrate on defects and architectural mismatches that can
be tied to specific code. The following gaps are broader product-readiness
differences between the implementation and the proposal.

### 6.1 Canonical product boundary is incomplete

The proposal describes `oryn` as the canonical embeddable Rust library, with the
CLI, OIL, Python, and MCP as adapters over one stable native API. The current
workspace does not yet have that shape:

- `crates/oryn` is a binary package and has no `src/lib.rs`;
- the typed browser surface lives primarily in `oryn-native`, exposing an
  implementation crate as the de facto public API;
- the CLI still requires the explicit `native` subcommand;
- the Python client sends OIL text through a subprocess rather than consuming a
  shared typed Intent/Target contract;
- no first-party MCP adapter was found;
- the README still calls native execution a gated implementation and a proof.

This is a release blocker even if the controlled runtime behavior is repaired.
Create a stable `oryn` library facade, keep worker internals private, make native
the default CLI path, decide the typed Intent IR question in F16, implement MCP
over that same contract, and rewrite the public documentation around the product
that actually ships.

### 6.2 Required JavaScript and internationalization profiles are absent

The proposal explicitly includes modern ECMAScript plus WebAssembly and an Intl
profile. V8 runs substantial classic/module JavaScript and real framework
fixtures, but no production WebAssembly surface, WebAssembly conformance set, or
declared Intl/ICU profile was found. A v1 release must either implement these
requirements or narrow the proposal explicitly.

Required decisions and work:

- define `full-icu` and, if valuable, `lite-icu` profiles;
- pin ICU data and test locale, time zone, collation, number, plural, and date
  behavior;
- add bounded WebAssembly compile, instantiate, imports, memory, exception, CPU,
  and heap behavior;
- include WebAssembly and ICU assets in size, supply-chain, and update evidence.

### 6.3 Network protocol and decoding profile remains narrower than proposed

F6 covers the most serious cross-origin and cookie defects. Separately, the
proposal's HTTP/2, zstd, and charset-decoding commitments do not have clear
production implementation and conformance evidence. The current dependency
configuration explicitly enables gzip, Brotli, and deflate; it does not establish
the complete proposal profile.

The supported network contract must name and test:

- HTTP/1.1 and HTTP/2 negotiation and connection behavior;
- gzip, Brotli, deflate, and zstd bounds;
- the exact Encoding Standard subset rather than a generic “encoding” claim;
- streaming cancellation/backpressure after the synchronous bridge in F4/F10 is
  removed;
- HTTP caching behavior, or an explicit statement that only pinned-resource
  caching is supported.

### 6.4 Semantic state is a flat projection, not the proposed semantic runtime

The current `Observation` contains page data, a revision, projection metadata,
nodes, capabilities, diagnostics, and execution domain. It has no first-class
semantic structure collection for forms, tables, lists, navigation, dialogs,
menus, trees, tabs, grids, or repeated collections. Native nodes are emitted
with `BrowserDeterministic` provenance even where accessible-name and visibility
logic are heuristic. There is no confidence-bearing relationship inference.

Reference safety is conservative and valuable, but it should not be confused
with durable logical identity. The published survival denominator is one
unrelated-mutation case. Benign React/Vue replacement invalidates the reference,
and no high-confidence matcher exists. The proposal must either adopt F2/F4's
recommended identity-only v1 contract or implement conservative logical
rematching with ambiguity rejection and a much larger churn corpus.

To deliver the original semantic thesis rather than only a flattened DOM view:

- implement the supported Accessible Name and Description Computation rules;
- expand ARIA role, state, relationship, ownership, landmark, live-region, and
  tree-exclusion behavior;
- add versioned semantic structures and evidence for each;
- separate deterministic facts, inferred facts, trust labels, confidence, and
  evidence;
- improve measured causal precision and observation sufficiency using corrected
  metrics;
- test identity survival and false preservation across real framework churn.

### 6.5 Controlled compatibility is not broad compatibility

Eight pinned framework applications and five controlled authentication scenarios
are valuable reproducible fixtures. They do not establish browser-scale
compatibility, especially given F1, F3, F4, F8, F9, and F14. The earlier four
public canaries are both stale and misclassified as described in F2.

A trustworthy compatibility program needs two separate tracks:

1. immutable local conformance and framework regressions that can gate a release;
2. a changing, legally safe, read-only site corpus that measures compatibility
   without making live-site instability a deterministic gate.

The latter should cover static pages, SSR, SPAs, commerce, documentation,
dashboards, authentication shells, large tables, content-heavy pages, nested
frames, and shadow-root applications. Record challenge/availability separately
from correctness, publish the first divergence and missing-capability taxonomy,
and turn every stable real-site failure into a local immutable regression.

### 6.6 Linux production isolation and packaging are missing

The macOS worker is one of the implementation's strongest components. It does
not close the proposal's Linux deployment requirement. Non-macOS execution is
truthfully reported as `process_isolated_unsandboxed`; that is not an acceptable
hostile-page security boundary.

Production closure requires:

- Linux namespaces, seccomp, filesystem isolation, process/resource controls,
  and broker-only networking;
- policy-parity tests against the macOS sandbox;
- reproducible x86_64 and ARM64 packages;
- SBOM and dependency provenance;
- continuous fuzzing with corpus retention, sanitizers, crash triage, and
  coverage tracking;
- automatic V8/ICU update qualification and rollback;
- a clear distinction between Apple development signing evidence and notarized
  distribution artifacts.

### 6.7 Rendering is a deterministic subset, not browser visual parity

The optional renderer is correctly separated from the default parent/worker and
receives a bounded paint snapshot. It can produce deterministic PNG and PDF
output for supported backgrounds, borders, embedded-font text, and images.
However, F3 and L5 mean that the input geometry and text shaping do not yet match
the proposal's richer layout model, and the output does not claim Chromium pixel
parity.

The product contract should explicitly choose between:

- deterministic native output for supported effects; or
- visual compatibility sufficient for customer screenshot, PDF, and
  verification workflows.

The second option is a materially larger program. Until chosen, rendering must
remain optional and every unsupported effect must remain observable.

### 6.8 Current release evidence is not current

The committed evidence sets bind to several ancestor commits. Historical-mode
validators recompute their compact ledgers, which preserves their value, but the
current source tree has not been rebuilt, signed, and validated as a single
production bundle after the naming refactor. At least one validator rejects the
current fixture hash without a historical override, as it should. The semantic
sandbox validator also has a macOS Bash empty-array failure under `set -u`.

A release claim needs one exact source tree, one parent/worker bundle, one set of
optional packages, and one carried-forward validation run. Historical results
must remain immutable and labeled historical rather than being treated as
current by prose.

---

## 7. Integrated priority and remediation plan

This ordering fixes the project's ability to measure truth before expanding its
surface. It intentionally uses capability names rather than numbered stage
labels.

### Priority 0 — stop relying on invalid evidence and close security exposures

1. Rebuild observation sufficiency, causal precision/recall, challenge-rate, and
   reference-survival metrics to match the proposal definitions (F2).
2. Bind every manifest case to an executed assertion and eliminate hardcoded
   passing ledgers (F5).
3. Correct status documentation immediately where evidence is non-conforming.
4. Fix CORS/preflight, Origin/Referer authority, SameSite cookies, CSP visibility,
   mixed-content checks, and consequential form/navigation policy (F6, F7).
5. Stop reflecting secret values into content attributes and track secret-taint
   independently of page-controlled input types (F13).
6. Gate or audit raw evaluate/selector/extraction access (L7).

**Exit condition:** A failed or unsupported control cannot be serialized as
passed, cross-origin page code cannot forge privileged request context, and
secret fills cannot leak through observation or DOM serialization.

### Priority 1 — repair the execution model

1. Decide explicitly between the proposal's Rust-owned DOM and the shipped
   JavaScript-canonical DOM with Rust mirror (F1).
2. Make lifecycle states factual, revision changes complete, and task/microtask/
   timer ordering browser-like (F4).
3. Replace the synchronous fetch/XHR bridge and blocking async facade with a
   genuinely asynchronous, correlation-based event-loop integration (F4, F10).
4. Decide and enforce the page-isolate/frame-context topology and total heap
   accounting (F10).
5. Bound snapshots, arena slots, resource caches, queues, and history (F11).
6. Add real production-worker integration and profile-equivalence tests (F15).

**Exit condition:** `act → wait → observe` has correct lifecycle and revision
semantics, concurrent pages do not block the async runtime, and sustained churn
does not grow state without bound.

### Priority 2 — make capability truthfulness systemic

1. Replace plausible-but-wrong geometry with `Unsupported` until the full box
   tree, inline formatting, clipping, and required cascade behavior exist (F3,
   F9).
2. Drive observer APIs or report them unavailable; never install inert stubs
   (F9).
3. Add an explicit trust axis for page-provided content and mark heuristic
   semantic values honestly (F12).
4. Replace flat DOM prototypes and begin generated V8 bindings (F14).
5. Add an executable per-interface capability classification table.

**Exit condition:** No typed or page-facing API silently returns fabricated
success, geometry, lifecycle, confirmation, or observer behavior.

### Priority 3 — complete the product and semantic contract

1. Add the canonical `oryn` Rust library and settle the typed Intent/Target
   design (F16, §6.1).
2. Make native the default CLI path and implement MCP over the same contracts.
3. Add supported semantic structures, accessibility semantics, confidence,
   trust labels, and conservative logical identity (§6.4).
4. Implement WebAssembly and the declared Intl/ICU profiles (§6.2).
5. Complete HTTP/2, content decoding, charset, and cache profile decisions
   (§6.3).

**Exit condition:** Rust, CLI, Python, OIL, and MCP expose one coherent product,
and the semantic layer provides more than flat node projection.

### Priority 4 — broaden compatibility and platform readiness

1. Replace paraphrased WPT/html5lib claims with vendored pinned cases and publish
   a coverage growth ledger.
2. Establish the deterministic and current-site compatibility tracks (§6.5).
3. Implement Linux sandboxing and reproducible packages (§6.6).
4. Operationalize continuous fuzzing and engine/data update qualification.
5. Decide the rendering product target (§6.7).

**Exit condition:** Compatibility claims are representative, hostile content is
isolated on every supported platform, and unsupported features remain explicit.

### Priority 5 — re-attest one release candidate

1. Fix validator portability and negative-test every evidence builder.
2. Build one clean source commit with exact parent, worker, optional domain, and
   renderer artifacts.
3. Run all corrected carried-forward manifests against those exact bits.
4. Publish a release bill of materials containing source, toolchain, V8/ICU,
   binary, entitlement, policy, fixture, and result hashes.
5. Publish size, startup, RSS, concurrency, sustained churn, and crash-recovery
   metrics for the shipped bundle.

**Exit condition:** Every public release claim resolves to recomputable evidence
for the exact packaged artifacts without a historical override.

---

## 8. Proposed v1 acceptance criteria

### Product and interfaces

- A documented `oryn` Rust library is the stable public facade.
- Native is the default product path; compatibility domains are explicit and
  opt-in.
- Rust, CLI, OIL, Python, and MCP use the same typed contracts and resolver.
- The README describes supported behavior and limitations rather than milestone
  history.

### Runtime correctness

- The chosen DOM ownership model is documented and its performance/correctness
  implications are tested.
- Same-origin realms follow the documented isolate/context topology.
- Lifecycle predicates correspond to real milestones.
- Every mutation visible to observation advances or is represented by a valid
  revision/delta transition.
- Tasks, microtasks, timers, mutation observers, custom-element reactions,
  network completions, style, and semantic flushes have a tested ordering.
- Page APIs do not block the asynchronous executor for the duration of worker
  I/O.

### Web platform

- WebAssembly and the declared Intl/ICU profile pass frozen conformance sets.
- Fetch, XHR, and streams are truly asynchronous and pass concurrency,
  cancellation, backpressure, and timeout tests.
- HTTP/1.1 and HTTP/2, redirects, cookies, SameSite, CORS/preflight, CSP,
  mixed-content, charset decoding, gzip, Brotli, deflate, and zstd have explicit
  implementation/evidence or explicit unsupported diagnostics.
- DOM bindings have correct prototypes, brand/realm checks, conversions,
  exceptions, and GC rooting for the supported surface.

### Semantics and actions

- Supported accessible-name, role, state, relationship, visibility, and trust
  rules pass an immutable manifest.
- Semantic structures cover the v1 task profile.
- Consequential actions are policy-checked at the actual point of effect and on
  every redirect/handoff.
- Logical-reference survival is measured across substantial framework churn,
  with zero false preservation for consequential actions.
- Corrected observation-sufficiency and causal metrics meet declared thresholds
  on a model-independent oracle corpus.
- Runtime, semantic, execution-domain, and model-quality outcomes remain
  separate.

### Style, layout, and rendering

- The supported CSS/layout profile is exact and versioned.
- Full box-tree and inline geometry are either correct for the profile or marked
  unavailable; equal-column and zero-rectangle approximations cannot be
  `Available`.
- Ancestor clipping, scroll containers, visibility, and interactability agree.
- Optional rendering consumes the same shaped fragments as layout or reports the
  known divergence.

### Security and operations

- macOS and Linux sandboxes pass equivalent authority, escape, resource, direct
  network/filesystem, secret-environment, and crash-recovery suites.
- Cross-origin bytes do not enter the worker before parent-side response policy.
- Secret-tainted controls remain redacted regardless of page-controlled type or
  later DOM mutation.
- Page-authored content is structurally labeled untrusted.
- Continuous fuzzing, dependency response, and V8/ICU qualification are
  operational.

### Evidence and release integrity

- Each manifest case maps to an executed assertion and a content-addressed raw
  artifact.
- Builders derive metrics, criteria, and outcome from raw evidence; negative
  tests prove that each gate can fail.
- All release criteria pass on one clean source commit and the exact packaged
  artifacts.
- Validators recompute without historical overrides.
- Parent, worker, optional packages, policies, entitlements, fixtures,
  toolchains, and model identifiers are fingerprinted.

---

## 9. Decisions that must be explicit before more implementation

1. **DOM ownership:** Restore the proposal's Rust-canonical NodeId/wrapper model,
   or amend the design around a JavaScript-canonical DOM and accept its mirror,
   serialization, namespace, and invalidation costs?
2. **Compatibility target:** Is v1 a documented agent-browser profile or a broad
   browser-compatibility promise?
3. **Style engine:** Is a full Stylo cascade/computed-value integration required,
   or is the Oryn cascade the intended contract?
4. **Realm topology:** Must same-origin frames share a top-level isolate through
   separate contexts, or is isolate-per-frame an accepted security/compatibility
   tradeoff?
5. **Semantic identity:** Is exact identity the v1 safety contract, or must benign
   replacement preserve references at a declared confidence threshold?
6. **Intent surface:** Does v1 keep method-per-verb APIs, or implement the
   proposal's typed Intent/Target IR before MCP?
7. **Local model:** Is a useful local-model experience a release requirement, or
   is completing the panel sufficient even when the pinned model succeeds on no
   tasks?
8. **Rendering:** Is deterministic supported-subset output sufficient, or do
   customers require browser visual parity?
9. **Platforms:** Is macOS the first supported production target, or does Linux
   remain required for v1 as the proposal implies?

These are product decisions, not test fixes. Leaving them implicit encourages
the project to optimize increasingly detailed fixtures without converging on a
supportable v1 contract.

---

## 10. Final combined assessment

Oryn2 has crossed the line from experiment to serious systems prototype. Its
strongest achievements are the raw-V8 host choice, thread-affine isolation,
macOS fail-closed worker, parent-owned SSRF controls, conservative semantic
references, replay, typed worker protocol, compatibility-backend separation, and
the explicit refusal mechanism used for unsupported table geometry.

At the assessed commit, the milestone narrative was nevertheless materially ahead of the product
in several places. The most urgent problem is epistemic: some evidence builders
and KPIs cannot establish the claims attached to them. The most consequential
runtime problems are the reversed DOM ownership model, fabricated lifecycle
states, incomplete revisions, blocking network/worker facades, permissive
cross-origin behavior, and plausible-but-wrong geometry reported as available.
The most important product gaps are the missing canonical library/MCP boundary,
thin semantic structures, absent WebAssembly/Intl profiles, limited real-site
evidence, and missing Linux sandbox.

Accordingly, the honest status is:

> **Oryn2 is a working and ambitious native-browser prototype with strong host
> isolation and historically passing controlled manifests. It is not yet
> complete against the original v1 design, and the current evidence is not
> sufficient to call it generally release-ready.**

The path forward is not to add another milestone name. It is to make the
measurement layer capable of detecting failure, repair the execution and
security foundations, decide the remaining architectural divergences explicitly,
finish the public and semantic product contracts, broaden representative
compatibility, secure Linux, and then re-attest one exact release candidate.

The 2026-08-29 runtime cutover closes the review's central DOM-ownership,
generated-binding, parser visibility, frame-realm, response-streaming, charset,
compression, and HTTP-cache implementation gaps. It does not retroactively
alter the audit evidence above. Current proposal-v1 software status is tracked
by `runtime-architecture-v1.json`; its local software cases pass. Platform
signing/sandbox execution and exact-artifact performance/model evidence remain
release attestations.

### 2026-08-29 F2 remediation update

The software defects identified by F2 are now addressed without rewriting the
historical schema-v3/v4 records:

- Schema v5 computes observation insufficiency only over failed tasks and only
  when a captured full projection contains task-required state omitted by the
  compact projection. Browser-truth absence, model/action failure, capability
  failure, and runtime failure are separate attribution buckets.
- Runtime trace records carry direct, derived, correlated, or unattributed
  causality. A labelled controlled manifest publishes independent precision and
  recall for the first three classes and rejects promotion of unattributed work.
- Navigation records a typed challenge assessment. Only paired, provider-
  specific evidence is confirmed; generic 401/403/429 or challenge language is
  suspected and does not set execution-domain `site_challenged`.
- Public-canary v2 corrects the Wikipedia and crates.io rows to native-runtime
  failures. Because the old response bodies were not retained, all historical
  challenge dispositions are explicitly `not_assessed`, not fabricated
  negative detector results.
- Exact Rust NodeId behavior is covered by eight preservation and eight
  invalidation cases. DOM recreation and framework-style keyed/conditional
  replacement invalidate old references; benign mutation and same-document
  reparenting preserve them.

Controlled causal and challenge evidence is reproducible locally. F2 evidence
closure remains pending until the expanded churn suite and corrected 96-cell
panel run against the final signed release candidate. That future run starts
with `$6.362286` already spent and remains bounded by the original `$100` cap.
