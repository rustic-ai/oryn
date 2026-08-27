# Oryn2 implementation status

This status is intentionally gate-based. It records implemented behavior and
does not imply that later browser-conformance work is complete.

## P0 — complete

- Reuse ownership, dual-license policy for new packages, and frozen compatibility
  backend boundaries are documented.
- The Rust, scanner, extension, Python SDK, and IntentGym baselines are captured
  in `benchmarks/baselines/p0-a66a6680.json`.
- IntentGym now resolves the in-repository Python SDK portably; its lockfile no
  longer contains a developer-machine absolute path.
- The versioned result schema and fixed 30-task G2 corpus are committed. The
  corpus validator checks counts, unique IDs, read-only canary policy, and all
  required local sources.

## G0 — complete

- Raw V8 and `deno_core` probes use the same V8 revision. Raw V8 was selected
  because its startup P50 was 1.117 ms versus 3.747 ms and its probe was
  1.90 MB smaller.
- Oryn-owned generation-safe DOM, `html5ever` tree sink, WebIDL parser,
  host-broker network policy, trace spine, and typed page actor are implemented.
- The JSONL page worker supports load, observe, direct semantic actions, OIL,
  and orderly shutdown without exposing filesystem or network commands.
- The final native 20-page benchmark measured a 0.368 ms median load+observe batch,
  4.59 MB peak RSS, and a 2.38 MB release probe (1.28 MB gzip). Raw artifacts
  live under ignored `artifacts/`.
- The worker is packaged as `OrynPageWorker.app`, signed by the trusted Apple
  Development identity `9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90`, and
  verified with bundle identifier `ai.rustic.oryn.page-worker`, team identifier
  `5HVA9VFF8K`, and `com.apple.security.app-sandbox=true`.
- The sandboxed process launches and completes the versioned JSONL `ping` and
  `exit` exchange. Machine-readable signing, sandbox, protocol, startup, page
  creation, concurrency, RSS, and size evidence is committed at
  `benchmarks/evidence/g0-macos-arm64.json`; raw samples remain ignored.

## G1 — complete

- Classic inline scripts execute in document order in a persistent raw-V8 realm.
- A narrow document realm now provides Oryn-owned element/text nodes,
  `querySelector`/`querySelectorAll`, element creation and mutation, attributes,
  values and boolean form state, event listeners and bubbling, click/input/change
  dispatch, `DOMContentLoaded`, microtasks, and deterministic timers. Mutations
  are synchronized back into Oryn's DOM after each interaction.
- The controlled Vue-style task and Svelte-style counter fixtures execute and
  mutate native DOM state in feature-gated tests. Forms, event ordering, and
  persistent-realm behavior also have executable tests.
- External classic scripts, V8 module graphs, pinned resources, promise-based
  fetch, redirects, request bodies/headers, compression, cookies, URL and text
  encoding primitives all execute through the context-owned rustls broker.
- Executable-DOM synchronization preserves retained generation-stamped node
  identities, retires removed identities, and assigns fresh identities to new
  nodes. A structural-churn regression test prevents false preservation.
- The typed page actor owns V8 on a dedicated page thread. Existing async page
  handles and OIL sessions dispatch actions into registered JavaScript handlers
  and receive versioned semantic deltas without moving an isolate across
  threads or introducing a second runtime API.
- Native navigation/history/refresh, lifecycle checkpoints, form constraints,
  inline and linked stylesheet visibility, custom elements, open/closed shadow
  roots, same-origin frames, cross-origin diagnostics, and pinned real React 18
  plus Babel execution are covered by executable tests.
- The versioned G1 manifest passes all required advertised cases. Pinned WPT
  URL/UTF-8 vectors, pinned html5lib parser vectors, and a supported-scope
  native-versus-Chromium differential run are published in
  `benchmarks/evidence/g1-external-validation.json`. Rendered layout, geometry,
  paint, and Encoding Standard behavior beyond the advertised UTF-8 subset
  remain explicitly unavailable.

## G2R — complete

- Existing OIL syntax drives native observations and click/type/clear/check,
  select, hover, focus, and targeted submit actions.
- Numeric aliases resolve to document-generation-scoped semantic references.
- Action IDs, revisions, deltas, values, effects, provenance, and capability
  diagnostics are versioned in the shared v2 contract.
- Compact, full, scoped-near, and revision-delta projections are wired through
  existing OIL observe flags.
- Native target resolution reuses the existing `oryn-common` resolver through
  a browser-domain adapter. Text, role, numeric, and deterministic
  before/after/near/inside/contains relations are covered using explicit
  semantic tree geometry; ID/test selectors are carried by the v2 projection.
  No second scoring engine is kept in the native crate.
- URL and local-HTML modes now execute navigation/history, observe/delta,
  click/type/clear/check/select/submit, waits, scroll checkpoints, popup
  dismissal, URL/title, text/HTML, and structured links/images/tables.
- The Python subprocess transport supports native mode and now decodes contract
  version, revision, document generation, capabilities, diagnostics, exact
  UTF-8 bytes, action effects, deltas, revision transitions, and execution-domain
  classifications while preserving the raw `execute()` API.
- All 12 harness OIL files and all six required framework fixtures pass natively.
  Structural churn reports zero false semantic-reference preservations. The
  machine-readable result is `benchmarks/evidence/g2-controlled-native.json`.
- The 30-entry corpus has published results. The four read-only public canaries
  separately report one available, two challenged, and one unavailable result
  without conflating availability with correctness.
- The earlier 96-cell panel is invalid as native evidence: its production
  `oryn` executable was built without `native-v8`, so the native 0/24 result did
  not exercise the designed runtime. Its `$1.196980` Azure spend remains charged
  to the cumulative $100 ceiling, but its schema-v2 aggregate is being replaced
  rather than retained as a valid result. A first G2R collection attempt then
  spent `$1.993578` before independent validation found that Chromium observation
  byte counts were zero; that attempt is also invalid and the cumulative prior
  spend charged to the replacement run is therefore `$3.190558`.
- Production `oryn` now enables `native-v8` by default. Machine-readable runtime
  metadata and a scripted DOM-mutation preflight reject no-V8 or debug binaries
  before any hosted request. Resume keys include the complete binary, fixture,
  Chromium, deployment, and local-model fingerprint.
- A benchmark-only server injects deterministic `Math.seedrandom` initialization
  after pinned `core.js` without changing MiniWoB files on disk. All eight native
  fixtures now bootstrap without unexpected script diagnostics, including D3,
  jQuery, jQuery UI datepicker, and pagination code.
- The OIL-only deterministic oracle records observations, commands, resolution,
  effects, trace slices, reward, causal counts, exact observation bytes, and
  failure-domain reasons. Native and Chromium both pass 24/24 deterministic
  cells with zero first divergences.
- Semantic-reference churn is measured independently for removal, replacement,
  closed-shadow isolation, frame replacement, navigation, and unrelated-mutation
  survival. The current suite reports 4/4 required invalidations, 1/1 survival,
  and 0/5 false preservations. Its controlled stale-reference and event-only
  no-progress recovery cases pass 2/2 in
  `benchmarks/evidence/g2r-churn.json`.
- `benchmarks/schema/result-v3.schema.json` separates `panel_complete` from the
  gate outcome. The final fingerprinted panel completes 96/96 cells: Azure/native
  passes 22/24, Azure/Chromium passes 16/24, and pinned `qwen3:4b` passes 0/48.
  The local-model failures are retained as model-quality results rather than
  reported as runtime failures. Overall model success is 38/96.
- The validator recomputes every aggregate from fingerprint-matched cell and
  per-turn JSONL artifacts. It reports measured observation sufficiency 17/96,
  recovery 2/2, reference survival 1/1, false preservation 0/5, causal recall
  99/99, and causal precision 99/198. All action classifications are explicit.
- The final Azure run spent `$2.019074`. Including `$3.190558` from the two
  invalid attempts, cumulative hosted spend is `$5.209632`, within the `$100`
  ceiling. The committed schema-v3 summary is
  `benchmarks/evidence/g2-model-panel.json`; raw cells and traces remain ignored.
- The release V8-enabled production binary is 46,556,752 bytes (17,837,843
  bytes gzip), starts and completes the mutation protocol at 10 ms P50/P95 over
  30 samples, and peaks at 23,805,952 bytes RSS. Its SHA-256 matches the model
  panel fingerprint; evidence is published in
  `benchmarks/evidence/g2r-production-macos-arm64.json`.
- Formatting, strict all-feature Clippy, all-feature Rust workspace tests,
  Python SDK tests, IntentGym tests, G1/G2/corpus validators, and schema-v3 raw
  evidence recomputation pass. G2R is closed.

## Next tracks

- G3 compatibility and G5 security now proceed in parallel. G3 owns
  context-scoped storage, XHR/WebSocket/streams, the complete framework corpus,
  page groups/popups/opener, and controlled OAuth/SSO. G5 owns the V8-enabled
  sandbox worker, threat model, policy engine, secret handles, network
  isolation, fuzzing, and the pinned V8 update process.
- G4 begins after G3's DOM/application requirements stabilize. G6 and G7 follow
  convergence of G3-G5. G8 remains optional and non-blocking unless a workflow
  requires screenshot, PDF, or visual verification.
