# Oryn2 Native Proof Gates

The committed implementation horizon is P0 through G5 on macOS ARM64. Gates
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
