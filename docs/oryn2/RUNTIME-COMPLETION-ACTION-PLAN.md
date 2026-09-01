# Oryn2 runtime completion action plan

**Created:** 2026-08-31
**Committed baseline:** `4b1f287` (`package Oryn for macOS and Linux`)
**Working baseline:** `a39d7a6` plus the remaining resource-scheduler cutover
**Scope:** remaining work required to close the proposal-v1 runtime and produce
one exact-artifact implementation attestation

This plan supersedes the short priority list in the combined proposal review.
It does not rewrite historical evidence or treat a focused unit test as proof
of a completed production cutover.

## 1. Verified status

| Workstream | Status | Verified implementation | Remaining closure condition |
|---|---|---|---|
| Runtime manifest and evidence contract | Implemented | Immutable `runtime-completion-v1` classification manifest, result schema, cache bootstrapper and fail-closed validators exist. The manifest validator passes. | Produce an exact-binary result whose aggregate is recomputed from complete assertion ledgers. |
| Async transport and streaming navigation | Implemented in software | `oryn-net` owns the async rustls/HTTP2 pool. Broker v4 emits correlated headers/chunk/completion/failure/cancel operations with explicit demand. Navigation incrementally consumes decoded chunks in the shared live `DomRuntime`. Images complete outside the page actor. Async/defer/module scripts, module graphs, stylesheets/imports, frames, and dynamic scripts start concurrently and join only their required parser/lifecycle barrier. All complete-resource operations are correlated, deadline-, size-, and lifetime-bounded. Worker connection loss, replacement, close, navigation supersession, timeout, and stale completion cancel or reject the affected operation. | Exact packaged-binary and broader WPT lifecycle evidence remain part of final validation. |
| Incremental semantics | Implemented in software | Contract v6 uses one persistent graph for semantic nodes, accessibility, structures, reverse dependencies, frame ownership, computed visibility, scoped observations, deltas and graph-native resolution. Equal scores and missing spatial geometry fail explicitly. The full projector is a test/feature-gated oracle only. | Run the pinned ARIA/Accessible Name corpus, framework churn, navigation, shadow/frame and replay equivalence, then publish exact-candidate zero-false-preservation evidence. |
| Generated WebIDL and compatibility APIs | Implemented in software for the declared interface inventory | Generated V8 templates and prototypes are deterministic and executable. Rust owns wrapper identity, brands, realm/origin/document-generation checks and the supported DOM, editing, navigation, Fetch/File/Streams/XHR/WebSocket and IndexedDB algorithms. Network and database work crosses only typed, asynchronous parent-host operations. Production restores the generated interface namespace after bootstrap; handwritten compatibility implementations remain available only to the explicit in-process probe. | Run every mapped API inventory entry and the broader upstream conformance set. Long-tail algorithms outside the declared inventory remain capability-scoped rather than inferred from focused regressions. |
| Stylo cascade | Implemented in software | Production resolves an immutable Rust style snapshot through native Stylo `ComputedValues`, including origins, inheritance, custom properties and supported values. Taffy and Parley remain the box/text consumers; the prior handwritten cascade is test-only. | Pass and publish the mapped CSSOM/native-versus-Chromium differential corpus, including the supported grid bridge and explicit long-tail layout boundaries. |
| V8 sandbox, ICU and startup snapshots | Partial | Production features require `v8_enable_sandbox`; runtime initialization is fail-closed. Exact macOS ARM64 full and external-data lite archives build from pinned source. Lite embeds a physically reduced 2.2 MiB root/en/en-US ICU package. A versioned callback table and content-addressed snapshot are supplied to both snapshot creation and runtime isolates, with hashes in schema-v3 manifests. Heap-sandbox, locale, Segmenter and snapshot-load checks pass. | Build Linux x86_64 full/lite, regenerate the full macOS schema-v3 bundle, then attest all four exact profile artifacts. |
| Upstream conformance | Partial | A pinned sparse WPT cache exists outside Git. The native harness supports applicable `.any.js`, `.window.js` and testharness HTML cases through controlled loopback and records assertion-level outcomes. Four SharedWorker and four ServiceWorker IndexedDB variants pass through a signed production worker and have a compact recomputable ledger. The current local first-100 IndexedDB diagnostic passes 71/100, including factory open, schema rename/abort, multi-entry indexes and unique cursor directions. | Complete transaction rollback, live cursors, connection/versionchange and UTF-16 name preservation; then run every required mapped WPT/ARIA/Accessible Name case. Keep deferred, excluded and informational results separate. |
| Product and release validation | Not started for this cutover | Focused Rust/V8 tests pass and historical product evidence remains available for its original commits. | Rerun framework/OAuth, MiniWoB, churn, replay, scanner, Python, MCP, platform security and benchmarks against one clean release candidate; privately sign and re-attest only after deterministic completion. |

## 2. Remaining implementation list

### Runtime correctness

- ~~Replace synchronous broker entry points with operation-based asynchronous
  start, headers, chunk, completion, cancellation and failure messages.~~ The
  operation path is now the production navigation and Fetch-body path; the old
  methods remain compatibility adapters.
- ~~Ensure 32 concurrent operations never hold the page-command mutex while
  waiting for network I/O.~~ The multiplexed context service reaches 32 active
  starts, its registry lock is released before per-stream waits, and paint image
  completion runs outside the page actor.
- ~~Feed accepted navigation bytes directly into the live parser rather than
  accumulating a decoded main-resource string.~~ Implemented with explicit
  demand, incremental charset state and parser-boundary resource calls.
- ~~Schedule async/defer/module scripts and lifecycle transitions from real
  prerequisites; reject late completions after navigation or worker
  replacement.~~ Async executes by completion order; defer/modules preserve
  document order; stylesheet/frame/module dependencies join before their
  lifecycle barrier; replacement and connection teardown cancel outstanding
  parent operations.
- Demand-driven Fetch/response streams, navigation, image and CORS-preflight
  scheduling are now bounded and cancellable. Broader per-API cancellation
  matrices for dynamic imports, stylesheets, frames and WebSockets remain part
  of their individual conformance work.

### Semantic runtime

- ~~Persist semantic nodes, order, accessibility relationships, structures,
  accessible-name dependencies, affordances and reference generations without
  rebuilding the full snapshot.~~ Implemented in contract v6.
- ~~Generate observation deltas from graph revisions only and use the same graph
  for Rust intent and OIL resolution.~~ Implemented through worker protocol v8,
  Python and MCP adapters.
- Keep the full projector as a non-production oracle while expanding randomized
  mutation, framework, frame/shadow, navigation and replay comparisons.
- Run pinned ARIA/Accessible Name vectors and exact-candidate churn/semantic KPI
  evidence before release closure.

### Generated browser interfaces

- ~~Finish Range boundary comparison, content mutation/extraction, insertion
  and surrounding behavior across text and element boundaries.~~ Implemented
  against the shared Rust `DomRuntime`, including cross-container operations.
- ~~Finish Selection mutation, editing, `contenteditable`, contextual fragments,
  focus and adoption behavior.~~ The declared Selection mutation surface,
  CharacterData editing, focus/adoption and contextual-fragment paths are now
  generated/Rust-backed. Long-tail editing-command conformance remains in the
  upstream inventory rather than being silently inferred.
- ~~Implement NodeFilter/`whatToShow` traversal semantics and live mutation
  rules.~~ Generated TreeWalker and NodeIterator now apply callback filters,
  `FILTER_ACCEPT`/`REJECT`/`SKIP`, `whatToShow`, and live Rust node identity.
- ~~Complete same-document history traversal, state cloning and event
  ordering.~~ Generated History owns cloned state, same-document URL updates,
  traversal, `popstate`, `hashchange`, and `scrollRestoration`.
- Apply referrer policy consistently to navigation, Fetch/XHR, redirects,
  frames, scripts, stylesheets and form submissions.
- ~~Complete the architectural IndexedDB cutover.~~ Generated Rust callbacks now
  own requests, upgrade delivery, databases, transactions, object stores,
  indexes, cursors, key ranges and structured cloning while the parent remains
  the typed persistent-storage authority. Broader connection/version-change and
  transaction scheduling conformance remains in the mapped upstream inventory.
- ~~Move Fetch/File/Streams/XHR/WebSocket compatibility classes onto generated
  callback dispatch.~~ Generated Rust state and callbacks now own these objects,
  promise/event progression, backpressure, abort/timeout and late-completion
  rejection over typed parent-host operations. The compatibility classes are
  restored only for the explicitly named in-process probe.
- ~~Regenerate bindings and require `oryn-webidl check` to remain clean.~~ The
  deterministic checked output and clean-regeneration test are enforced.

### Style and layout

- ~~Resolve an immutable Rust DOM mirror through native Stylo document/node/
  element traversal without creating another canonical live DOM.~~ Implemented.
- ~~Maintain user-agent, author and inline stylesheet origins and revisioned
  invalidation state.~~ Implemented by the Stylo-backed snapshot service.
- ~~Convert Stylo computed values into Taffy/Parley inputs and semantic
  visibility.~~ Implemented, including the standalone grid-value bridge.
- Continue marking paint-only, fragmentation, unsupported writing-mode and
  other declared boundaries unavailable instead of returning plausible geometry.

### Runtime artifacts and profiles

- Produce custom sandboxed/pointer-compressed rusty_v8 archives for both target
  platforms; the absent official macOS archive is not an acceptable fallback.
- ~~Produce and inspect the actual English-lite ICU data payload.~~ Completed
  on macOS ARM64 with exact locale-resource inspection and Intl regressions.
- ~~Generate profile-specific snapshots with a stable external-reference
  table.~~ Implemented and verified for the macOS profile builder.
- ~~Bind V8, archive, generated binding, ICU, snapshot and external-reference
  hashes into the profile manifest and runtime metadata.~~ Implemented in
  manifest schema 3; toolchain fingerprinting remains part of final evidence.
- Refuse page creation when a production artifact is unsandboxed, mismatched or
  missing required profile data.

### Conformance and evidence

- Repair the WPT completion/lifecycle path and make timeouts explicit failures.
- Run all required manifest mappings, not a hand-selected success subset.
- Add broad pinned Accessible Name and ARIA comparisons against Chromium.
- Run public-site canaries read-only; classify challenges separately from native
  runtime, capability and network failures.
- Record exact binary, fixture and tool hashes in compact recomputable evidence.
- Do not update the release-complete status until the validator independently
  derives every required result.

## 3. Dependency-ordered execution plan

### Milestone A — stabilize the current foundation

1. Review the current uncommitted diff for historical-schema compatibility.
2. Run formatting, strict default/V8 Clippy, default/no-V8/V8 Rust tests,
   WebIDL regeneration, Python syntax checks and manifest validators.
3. Commit one foundation changeset before deeper cutovers.

**Exit:** clean working tree, reproducible generated bindings, all focused tests
green, and no implementation-status claim beyond the evidence.

### Milestone B — complete transport and parser scheduling

1. **Implemented:** operation-based broker interface, demand-driven bodies,
   streaming cache/cookie/policy/replay adapters and multiplexed broker v4 IPC.
2. **Implemented:** navigation and Fetch bodies use the operation path; image
   work is detached from the page actor; preflight and all complete script,
   module, style, frame, font, media, worker and reporting resources use bounded
   correlated operations.
3. **Implemented:** html5ever and V8 share the live `DomRuntime`; blocking
   scripts checkpoint at retained tokenizer boundaries and parser-inserted
   writes are consumed before the network/source suffix.
4. **Implemented:** the legacy unbounded collector and direct broker `send`
   calls are absent from production resource scheduling. Complete-byte resources use a correlated collector
   with explicit deadline, decoded-size limit and cancellation on every
   unsuccessful terminal path.

**Exit achieved for the requested transport-scheduling boundary:** 32-request
concurrency, HTTP/2, demand/backpressure, abort/teardown, timeout, redirect,
CORS withholding, decompression and stale-completion tests pass. Compatibility
`send` remains an adapter for complete-resource callers and is not used by
navigation, Fetch bodies, image loading, or CORS preflight.

### Milestone C — complete semantic and WebIDL cutovers

1. **Implemented:** remove full semantic reprojection from normal observations
   and deltas; publish additive contract v6 through every public adapter.
2. Run randomized graph-oracle equivalence, pinned accessibility conformance and
   expanded reference churn.
3. Complete the mapped referrer, IndexedDB transaction/version-change and
   broader editing conformance inventory.
4. Delete the probe-only handwritten interfaces after the compatibility probe
   is no longer required by historical differential tests.

**Exit:** graph/oracle outputs are identical across the mapped corpus, false
reference preservation is zero, generated output is clean, and mapped DOM/API
tests pass without compatibility shims.

### Milestone D — replace the CSS cascade

1. **Implemented:** Stylo owns stylesheet/media/declaration parsing, selector
   matching, specificity and computed-value resolution.
2. **Implemented:** connect style caching/invalidation and computed visibility
   to the semantic graph.
3. **Implemented:** feed native computed values to Taffy/Parley and semantic
   visibility, including the grid compatibility bridge.
4. **Implemented:** the bounded winner/inheritance/value projection is no
   longer in production and remains only as a test oracle.

**Exit:** runtime metadata identifies a real Stylo cascade and every required
mapped CSSOM/cascade/selector assertion plus native-versus-Chromium differential
passes.

### Milestone E — produce hardened runtime profiles

1. Build full and English-lite sandboxed V8 archives on macOS ARM64 and Linux
   x86_64.
2. Generate reduced ICU and profile-specific snapshots.
3. Verify archive and runtime heap-sandbox state, pointer compression, locale
   contents, WebAssembly and unavailable-locale diagnostics.
4. Package and run the fail-closed macOS and CayVM Linux workers.

**Exit:** verified manifests exist for all four platform/profile combinations;
an altered or unsandboxed artifact fails before page creation.

### Milestone F — conformance and deterministic product validation

1. Run all required pinned WPT/ARIA/Accessible Name mappings.
2. Run framework, controlled OAuth, MiniWoB 24/24, churn, replay, scanner,
   Python and MCP suites.
3. Run platform sandbox-denial and profile-equivalence tests.
4. Republish startup, warm-page, RSS, parser, semantic-update, concurrency,
   cancellation and crash-replacement benchmarks.

**Exit:** every required deterministic assertion passes, deferred behavior has
the declared diagnostic, cold startup meets 250 ms or has an explicit reviewed
exception, and the runtime-completion validator recomputes a passing result.

### Milestone G — exact release-candidate attestation

1. Freeze a clean source commit and exact parent/worker/profile artifacts.
2. Privately sign the macOS bundle and record signatures, entitlements, policy,
   limits and hashes.
3. Repeat deterministic validation against those exact bytes.
4. Only then run the deferred 96-cell hosted/local model panel, carrying forward
   prior hosted spend under the established US$100 ceiling.

**Exit:** one immutable release-candidate record binds source, binaries,
toolchain, V8/ICU/snapshot, fixtures, deterministic results, security results,
performance and model evidence. Only at this point may proposal-v1 software and
release status be marked complete.

## 4. Immediate next actions

1. Diagnose the remaining broader Range/API WPT inventory; the eight requested
   SharedWorker/ServiceWorker IndexedDB variants now pass production and have a
   recomputable compact ledger.
2. Expand the now-production contract-v6 graph/oracle tests to framework churn,
   navigation, frames and shadow DOM, then run pinned ARIA/AccName mappings.
3. Complete the remaining generated Selection/traversal/editing,
   history/referrer and IndexedDB connection/version/transaction algorithms.
4. Publish the Stylo CSSOM and native-versus-Chromium differential corpus.
5. Build Linux custom V8 artifacts after software cutovers stabilize, avoiding costly
   rebuilds for a moving external-reference table.

## 5. Completion rules

- A focused unit test is implementation evidence, not gate completion.
- A compatibility shim cannot silently stand in for a generated or native API.
- A timeout, missing artifact or unsupported behavior is recorded as such.
- Historical evidence remains tied to its original source and schema.
- No live side-effecting shadow request or action is duplicated.
- Release and hosted-model work starts only after deterministic software gates
  pass against the exact candidate.
