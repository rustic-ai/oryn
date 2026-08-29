# Oryn2 Native Proof Gates

The committed implementation horizon is P0 through G8 on macOS ARM64. Gates
are evidence-based; they do not carry calendar dates.

## P0 — Reuse and baseline

- Full Rust, scanner, extension, Python, and IntentGym baselines are captured.
- Reuse ownership is complete and enforced during review.
- Benchmark records validate against the versioned schema.
- Existing execution domains accept security and build-maintenance work only.

## G0 — Architecture and measurement proof

- The V8 host decision is resolved by identical correctness and resource tests.
- Minimal DOM, HTML parsing, WebIDL, network broker, semantics, and trace spines run.
- A page worker is confined by the macOS App Sandbox and communicates through
  typed host messages.
- Startup, page creation, RSS, concurrency, and installed/compressed size are recorded.

## G1 — Executable web document

- Native navigation, scripts/modules, event-loop ordering, DOM/events, forms,
  fetch, URL, encoding, compression, cookies, and lifecycle behavior pass the
  advertised conformance manifest.
- Representative SSR and React TodoMVC fixtures operate without scanner, CDP,
  WebDriver, Chromium, or WebKit.
- Unsupported APIs produce truthful capability diagnostics.

## G2 — Agent-native semantic loop

- Existing OIL drives native observations and core actions.
- Numeric OIL aliases resolve to generation-checked semantic references.
- Compact/full/scoped/delta observations and action effects are versioned.
- All controlled tasks pass, the 30-task corpus is published, and zero confirmed
  false reference preservations occur for consequential actions.

The authoritative thresholds and corpus composition are maintained in the
benchmark manifest introduced during P0.

### G2R/G5A recovery closure

- The signed protocol-v4 worker and production parent are bound to the clean
  source tree, policy, limits, entitlements, fixtures, compatibility adapter,
  Chromium version, Azure deployment, and immutable Qwen digest.
- Native and Chromium deterministic matrices pass 24/24 with zero first
  divergences; Azure/native passes 21/24 against the required 15/24 floor.
- The full Azure/Qwen × native/Chromium panel completes 96/96 cells within the
  cumulative hosted-model budget, and schema-v4 aggregates recompute from the
  compact ledgers plus content-addressed raw artifacts.
- Churn records zero false semantic-reference preservation, recovery passes
  2/2, and worker limit replacement invalidates every old reference.
- G2R/G5A is closed by
  `benchmarks/evidence/g2r-v4/g2-model-panel-v4.json` and the signed production
  summary in `benchmarks/evidence/g2r-v4/g5a-production-macos-arm64.json`.

## G3 — Modern application and multi-page compatibility

- Contract-v3 page groups, popup/opener messaging, active-page orchestration,
  page-scoped references, closure, crash replacement, and cross-page rejection
  pass through the production sandbox-worker boundary.
- Context-owned cookies, local/session storage, persistent `redb` profiles, and
  the required IndexedDB subset pass restart, quota, transaction, event, and
  isolation tests.
- Required XHR, Fetch/Streams, WebSocket, File, Web Crypto, dynamic-module,
  DOM, event, Shadow DOM, Custom Elements, and same-origin-frame cases pass.
- React, Next.js, Vue, Nuxt, Angular, Svelte, SvelteKit, and Preact pass native
  task assertions and Chromium differential checks. Five controlled OAuth/SSO
  flows pass in both domains.
- All 43 required manifest entries pass natively, all five G4-deferred entries
  emit their declared capability diagnostics, and reference churn reports zero
  false semantic-reference preservation.
- G1, G2 controlled, native MiniWoB 24/24, strict Rust/Python/JavaScript checks,
  signed worker security tests, and production multi-page benchmarks pass on
  the exact clean source commit.

G3 is closed by the schema-valid, raw-recomputed result at
`benchmarks/evidence/g3-v1/g3-result-v1.json`. Raw traces and application build
artifacts remain ignored.

## G4 — Deterministic style and layout

- Contract v4 exposes deterministic computed style, box geometry, inline text,
  viewport, media state, scrolling, and geometry-aware semantic observations.
- Author declarations are parsed through Stylo, bounded block/flex/grid and
  simple float geometry is computed through Taffy, and inline text is measured
  through Parley using the frozen embedded Noto Sans asset.
- Geometry participates in OIL targeting and action interactability without
  fabricating paint-order hit testing. Full-document navigation resets viewport
  scroll state before new-document interactability checks.
- All 28 required manifest cases pass natively and through the declared
  Chromium differential assertions. The five declared limitations—table
  geometry, paint-order hit testing, 3D transforms, complex float exclusion,
  and paint—emit their expected capability diagnostics.
- G1/G2/G3 regressions, controlled OIL 12/12, framework and OAuth coverage,
  native MiniWoB 24/24, signed sandbox checks, and reference churn with zero
  false preservation pass on the exact clean source commit.
- G4 is closed by the schema-valid, raw-recomputed result at
  `benchmarks/evidence/g4-v1/g4-result-v1.json`. Raw observations, traces, and
  benchmark samples remain ignored.

## G5 — Autonomous-runtime security

- The trusted V8 worker is confined by the macOS App Sandbox with only the JIT
  entitlement. Direct filesystem, network, listener, and inherited-secret
  access fail; all network and persistent authority remains in the parent.
- Context policy covers cross-origin navigation, submissions, popups, uploads,
  downloads, clipboard use, credential fills, protected requests, and
  user-browser delegation with allow, deny, or exact one-use confirmation.
- Origin-scoped opaque secret handles, observation redaction, structured audit
  records, and bounded parent-owned download quarantine prevent ambient secret
  and filesystem authority.
- Scheme, host, port, resolved-address, redirect, header, body, decompression,
  IPC, queue, CPU, heap, and RSS controls fail closed. Worker replacement
  starts a blank generation and invalidates every old semantic reference.
- Four pinned libFuzzer targets cover HTML construction, DOM mutation, URL
  policy, and the worker protocol. Rust/V8 versions, checksums, response SLAs,
  dependency exceptions, and full update qualification are versioned.
- All 51 required threat-model cases, G1-G4 regressions, native MiniWoB 24/24,
  strict software checks, signed-worker rejection cases, and production
  benchmarks pass on the exact clean source commit with zero false semantic
  reference preservation and zero dependency vulnerabilities.
- G5 is closed by the schema-valid, raw-recomputed result at
  `benchmarks/evidence/g5-v1/g5-result-v1.json`. Raw traces, fuzz logs, and
  benchmark samples remain ignored.

## G6 — Capability-driven execution domains

- Native remains canonical and compatibility backends remain opt-in packages.
- A versioned capability ledger and policy select native, Chromium, or a
  user-controlled browser with stable, agent-visible reason diagnostics.
- Domain movement is an explicit reload handoff with policy-controlled state,
  fresh revisions, complete loss reporting, and no secret migration.
- Domain/reason mix and handoff outcomes are measurable through the unchanged
  native page and OIL APIs.
- G6 closes only when all 25 frozen cases and the carried-forward G5 production
  gate pass on the same signed clean-tree bundle.
- G6 is closed by `benchmarks/evidence/g6-v1/g6-result-v1.json`, recomputed from
  the clean source commit and content-addressed raw production artifacts.

## G7 — Deterministic replay and checkpoints

- Traces bind lifecycle inputs, scrubbed intents, revisions, deterministic
  time/random state, causal events, and captured network exchanges.
- Sealed L1 checkpoints restore context state with fresh identities. L2 adds
  exact network and post-checkpoint intent replay; L3 live-heap forking remains
  unsupported.
- Divergences and nondeterministic regions are explicit. Failed tasks can be
  sealed as regression artifacts, replayed, and branched from the CLI.
- G7 closes only when all 26 frozen cases, G6, and G1-G5 production regressions
  pass on the same signed clean-tree bundle.
- G7 is closed by `benchmarks/evidence/g7-v1/g7-result-v1.json`, recomputed from
  the same signed production run as G6.

## G8 — Optional rendering and visual verification

- Rendering remains an opt-in package. The default parent and signed page
  worker do not depend on the renderer, and the renderer receives only a
  neutral, bounded paint snapshot rather than live page state or network access.
- A deterministic CPU display list covers supported backgrounds, borders,
  embedded-font text, and PNG/JPEG images. PNG screenshot and single-page PDF
  outputs are available through the Rust API and dedicated CLI.
- Viewport, pixel, command, image, and output budgets fail explicitly. SVG,
  gradients, GPU paint, unrestricted fonts, and other unsupported effects emit
  truthful diagnostics; Chromium pixel parity is outside the gate.
- The native-versus-Chromium differential asserts identical viewport dimensions
  and visible-content sanity. It does not redefine the deterministic native
  output by comparing pixels against Chromium.
- G8 closes only when all 24 frozen cases, the signed worker boundary, the
  default dependency boundary, strict software validation, and the complete
  G1-G7 production regression pass on one clean source commit.
- G8 is closed by `benchmarks/evidence/g8-v1/g8-result-v1.json`, recomputed from
  204 content-addressed raw artifacts. Raw images, PDFs, logs, and benchmark
  samples remain ignored.
