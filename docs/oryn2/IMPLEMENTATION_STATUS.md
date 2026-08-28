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

## G2R — historical schema-v3 pass; schema-v4 revalidation open

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
- G2R originally enabled `native-v8` in the monolithic production binary.
  G5A supersedes that packaging: the default `oryn` parent is now deliberately
  built without V8, while the required worker is built with `v8-host`.
  Machine-readable runtime metadata and a scripted DOM-mutation preflight still
  reject a no-V8 or debug worker before any hosted request. Resume keys include
  the complete parent, worker, fixture, Chromium, deployment, and local-model
  fingerprint.
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
  evidence recomputation passed at commit a41a231. That result remains
  historical and is not reusable for the G5A worker bundle.

## G3 — complete

- Contract v3 adds page-scoped semantic references, `PageId`, page descriptors,
  lifecycle events, predicates, and compatibility adapters. `BrowserContext`
  owns the PageGroup registry, active page, opener relationships, cookies,
  storage, policy, and page-event history; parent-side OIL can list, wait for,
  activate, and close pages without crossing worker boundaries.
- Top-level pages use independent sandbox workers. Popups, `_blank` links,
  targeted forms, opener state, focus/close, and origin-checked `postMessage`
  are implemented. Foreign-page, closed-page, navigation, frame-replacement,
  and worker-replacement references fail explicitly while unrelated page
  references remain valid.
- Context-owned ephemeral storage and explicit `redb` persistent profiles cover
  partitioned local/session storage, cookies, storage events, quotas, and the
  required IndexedDB transaction, store, index, cursor, and structured-clone
  subset. Persistent restart and context-isolation regressions pass.
- The parent-brokered platform surface now includes asynchronous XHR, bounded
  streams, WebSockets, byte-preserving fetch bodies, File/Blob/FormData,
  aborts, OAuth Web Crypto, dynamic modules, and the task-required DOM,
  selection, traversal, focus, editing, form, custom-element, shadow-root, and
  same-origin-frame behavior. Layout, paint, cross-origin DOM, service workers,
  and media remain explicit G4-or-later capability boundaries.
- All eight pinned applications—React, Next.js, Vue, Nuxt, Angular, Svelte,
  SvelteKit, and Preact—pass their native tasks and Chromium differential
  assertions. All five controlled OAuth/SSO cases pass in both domains.
- The exact clean-tree macOS ARM64 production bundle at source commit
  `8e460a81a6de8c1c2739b1fb5476d323a242c0aa` passed all 43 required manifest
  cases; all five G4 deferrals emitted their declared diagnostics. The G1/G2
  regressions, controlled OIL 12/12, and native MiniWoB 24/24 also passed, with
  zero false semantic-reference preservations.
- The signed parent is 6,933,984 bytes and the worker is 44,792,096 bytes;
  combined installed size is 51,730,100 bytes and compressed size is
  20,452,801 bytes. Startup-to-handshake was 500 ms P50/P95. Eight concurrent
  pages completed brokered streaming and WebSocket work in 664.155 ms.
- The schema-valid compact result is
  `benchmarks/evidence/g3-v1/g3-result-v1.json`; its status record is
  `benchmarks/evidence/g3-v1/STATUS.json`. The validator recomputed the result
  from 48 manifest-bound cases and content-addressed raw artifacts.

## G4 — complete

- Contract v4 adds worker-owned computed style, box geometry, inline text
  fragments, viewport/media state, scrolling, geometry observation, box
  queries, and action interactability while retaining contract-v3 adapters.
- Author CSS declarations use Stylo parsing and a deterministic bounded
  cascade. Taffy supplies the supported block/flex/grid and simple-float box
  model; Parley measures inline text with a frozen embedded Noto Sans font.
  Unavailable system fonts fall back deterministically rather than producing
  zero-width glyph runs.
- Full-document navigation now resets scroll and layout state. Forced OIL
  interaction is explicit in the typed worker protocol, while ordinary clicks
  require a visible pointer-interactable box. Same-document fragment
  navigation preserves the live DOM and document generation.
- All 28 required G4 manifest entries pass; all five declared limitations
  report the expected capability instead of fabricated geometry. G1/G2/G3,
  controlled OIL 12/12, frameworks, OAuth, native MiniWoB 24/24, signed worker
  isolation, and semantic churn all pass with zero false preservation.
- The exact clean-tree macOS ARM64 bundle at source commit
  `68e5afe143490b810301e1ed5d60e6e232209793` passed the complete production
  gate. The parent is 7,033,872 bytes and the worker is 46,921,472 bytes;
  combined installed size is 53,959,743 bytes and compressed size is
  21,691,132 bytes. Startup-to-handshake was 570 ms P50 and 580 ms P95.
- The 400-node layout benchmark measured 947.661 ms cold page-to-geometry,
  328.617 ms reflow P50, and 330.869 ms P95 over 20 samples. The schema-valid
  compact result is `benchmarks/evidence/g4-v1/g4-result-v1.json`, recomputed
  from 33 manifest cases and 114 content-addressed raw artifacts.

## G5A — implemented, release evidence pending

- Public native callers now use a protocol-v2 page worker on macOS. V8, the
  DOM, OIL, semantic state, and traces live in the worker; networking, DNS,
  cookies, policy, limits, signatures, and secrets remain parent-owned.
- Production fails closed for a missing, unsigned, ad-hoc-signed, altered,
  debug, incorrectly entitled, wrong-Team-ID, or wrong-certificate worker.
  There is no macOS in-process fallback. The named in-process-probe feature is
  excluded from production artifacts.
- OrynPageWorker.app is signed with hardened runtime and exactly App Sandbox
  plus allow-jit. It has no direct network, filesystem, DYLD, unsigned-memory,
  or library-validation exception.
- The parent clears the worker environment, verifies a nonce handshake and
  signed manifest, caps IPC at 24 MiB, decoded responses at 16 MiB, pending
  work at 32 commands, V8 heap at 256 MiB, RSS at 512 MiB, commands at 10
  seconds, and navigation at 30 seconds.
- Worker replacement creates a blank high-order document generation. The
  limit/churn suite measures restart latency and proves that all old semantic
  references are invalid.
- Pull-request CI pins Rust 1.98.0 and every GitHub Action to a commit SHA.
  Unsigned production-worker rejection runs without signing secrets. Protected
  g5a-signing and g2r-release environments own the P12 and model secrets.
- Schema v4 binds the source-tree hash, parent and final signed-worker hashes,
  signature/certificate/entitlement/manifest fingerprints, policy, limits,
  fixtures, Chromium, Azure deployment, and Qwen digest. Compact 96-cell and
  24-cell causal ledgers are sufficient to recompute the aggregate; raw turns
  and traces remain uploaded, ignored artifacts.
- The current machine-readable state is
  benchmarks/evidence/g2r-v4/STATUS.json: G2R/G5A remains open until the
  protected signed workflow and manual 96-cell release workflow publish and
  validate schema-v4 evidence. Budget accounting begins at $5.209632.
- The security boundary and deferred items are documented in
  docs/oryn2/G5A_SECURITY.md.

## G5 — complete

- Every native context now owns an explicit policy engine for cross-origin
  navigation, form submission, popups, uploads, downloads, clipboard access,
  credential fills, protected requests, and user-browser delegation. Exact
  request confirmations are random and one-use; all decisions are recorded in
  a structured parent audit log.
- Credentials enter parent-owned memory as opaque, origin-scoped, revocable
  handles. Secret storage is zeroized on removal, handle debug output is
  redacted, password observations never expose values, and the production
  security probe proves secret material is absent from observations and policy
  audit records.
- Downloads enter a bounded parent-owned quarantine with sanitized basenames,
  content hashes, media metadata, and explicit release. Page content never
  chooses a host path or writes a file implicitly.
- Network policy now includes host allow/deny lists and port controls in
  addition to per-hop scheme, DNS/IP, redirect, header, cookie, response, and
  decompression enforcement. Worker-originated popup and cross-origin
  navigation requests are checked by the parent.
- Four immutable libFuzzer entrypoints and curated seeds cover HTML tree
  construction, DOM mutation/stale identity, URL policy, and bounded worker
  protocol parsing. The pinned V8 update policy fixes version/checksum,
  Rust/nightly toolchains, critical/high response SLAs, and the complete signed
  qualification command.
- The dependency audit reports zero vulnerabilities. Three informational
  transitive warnings are explicitly scoped, remediated, and time-bounded in
  `security/dependency-exceptions.json`; unreviewed or expired warnings fail the
  gate.
- The exact clean-tree macOS ARM64 bundle at source commit
  `6eb164df0e0bd50aa02aae969b6112ff117cdd9c` passed all 51 required G5 cases,
  all G1-G4 regressions, native MiniWoB 24/24, four fuzz targets with zero
  crashes, limit/crash recovery, and reference churn with zero false
  preservation.
- The parent is 7,449,040 bytes and the worker is 46,970,896 bytes; combined
  installed size is 54,424,335 bytes and compressed size is 21,890,870 bytes.
  Startup-to-handshake was 580 ms P50/P95, startup-to-mutation was 590 ms P50
  and 600 ms P95, parent RSS was 56,492,032 bytes, worker RSS was 56,410,112
  bytes, and crash restart-to-blank was 570.780 ms.
- The schema-valid compact result is
  `benchmarks/evidence/g5-v1/g5-result-v1.json`; its status record is
  `benchmarks/evidence/g5-v1/STATUS.json`. The validator recomputed the result
  from 51 manifest cases and 147 content-addressed raw artifacts.

## G6 — complete

- Native is the canonical default dependency graph. Chromium and user-browser
  compatibility live in the opt-in `oryn-domain` crate and compile behind
  explicit features.
- A versioned capability ledger and deterministic policy select domains for
  missing capabilities, authenticated sessions, user presence, consequential
  work, known-bad origins, explicit user choice, and detected site challenges.
  Decisions retain stable reason codes and rejected candidates.
- Handoff is a controlled reload carrying only policy-approved URL, storage,
  selected non-secret form state, cookies, task context, and checkpoint
  lineage. Heap, promises, sockets, IndexedDB, host objects, and secrets remain
  behind with explicit loss diagnostics and a fresh destination revision.
- Domain/reason mix and failures are measured. The signed production probe
  selected native, Chromium for paint and challenge cases, and the user browser
  for authenticated consequential work; its controlled Chromium handoff passed.
- All 25 frozen cases and the complete G5 production regression passed on clean
  commit `d8930e3c87da0bdaee94699aaebf190b9b092c71`. The schema-valid result is
  `benchmarks/evidence/g6-v1/g6-result-v1.json`.

## G7 — complete

- Versioned task traces record page/lifecycle events, scrubbed OIL intents and
  outcomes, revisions, capability/domain decisions, deterministic clock/random
  inputs, network exchanges, and explicit nondeterministic regions.
- Network replay is exact and policy preserving: method, URL, scrubbed headers,
  request body, order, response body, and hashes are checked while the current
  parent network policy still executes before replay.
- Sealed L1 checkpoints restore page groups, storage, form state, opener and
  active-page relationships with fresh identities. L2 replays post-checkpoint
  OIL and network input; branches retain parent hashes. L3 live V8 heap forking
  remains explicitly unsupported.
- Failed tasks become tamper-evident checkpoint+trace regression artifacts.
  CLI recording, restoration, replay, failure capture, and failure replay are
  available without changing existing OIL syntax.
- All 26 frozen cases passed with 1/1 deterministic production replay, zero
  divergences, verified checkpoint/failure seals, and all G1-G6 regressions on
  clean commit `d8930e3c87da0bdaee94699aaebf190b9b092c71`. The schema-valid result is
  `benchmarks/evidence/g7-v1/g7-result-v1.json`.

## Next tracks

- G2R/G5A schema-v4 release evidence remains open pending the private protected
  signing/model workflows. G5's local signed security gate is complete; this
  does not substitute for the separate 96-cell Azure/Qwen release evidence.
- G8 remains optional and non-blocking unless a workflow requires screenshot,
  PDF, or visual verification. Broader long-running fuzzing, V8 update
  automation, and private protected G2R/G5A release evidence remain operational
  tracks rather than unimplemented G6/G7 behavior.
