# Oryn2 runtime completion action plan

**Created:** 2026-08-31
**Committed baseline:** `4b1f287` (`package Oryn for macOS and Linux`)
**Working baseline:** `8a3f2b4` plus the direct-streaming completion changes
**Scope:** remaining work required to close the proposal-v1 runtime and produce
one exact-artifact implementation attestation

This plan supersedes the short priority list in the combined proposal review.
It does not rewrite historical evidence or treat a focused unit test as proof
of a completed production cutover.

## 1. Verified status

| Workstream | Status | Verified implementation | Remaining closure condition |
|---|---|---|---|
| Runtime manifest and evidence contract | Implemented | Immutable `runtime-completion-v1` classification manifest, result schema, cache bootstrapper and fail-closed validators exist. The manifest validator passes. | Produce an exact-binary result whose aggregate is recomputed from complete assertion ledgers. |
| Async transport and streaming navigation | Partial — main-resource boundary complete | `oryn-net` owns the async rustls/HTTP2 pool. Broker v4 emits correlated headers/chunk/completion/failure operations with explicit demand. The executable starts at accepted headers and html5ever incrementally consumes decoded network chunks over the same live `DomRuntime` exposed to V8. Parser scripts, styles/imports, frames and meta policy use typed parent operations. | Migrate remaining image/preflight compatibility collection and page-command scheduling off blocking waits; complete cancellation/concurrency evidence. |
| Incremental semantics | Partial | `DomRuntime` has a bounded sequenced mutation journal with independent cursors. Each document owns a persistent `SemanticGraph` with reverse dependencies, reference generations and incremental delta publication. Focused oracle equivalence and bounded-history tests pass. | Eliminate remaining full projection from normal observation/update paths. Pass randomized mutation, framework churn, navigation, shadow/frame and replay equivalence with zero false reference preservation. |
| Generated WebIDL and compatibility APIs | Partial | Generated V8 prototypes route migrated operations into Rust and enforce identity, brand, realm, origin and generation checks. Range, Selection, NodeIterator, TreeWalker, History, referrer policy and broader IndexedDB behavior are present. | Implement the remaining declared Range/Selection/editing/traversal algorithms, traversal filtering, full history/referrer cases and IndexedDB connection/version/transaction semantics. Remove corresponding handwritten bootstrap interfaces only after mapped tests pass. |
| Stylo cascade | Not started | Stylo parses individual declaration blocks; Taffy and Parley provide bounded layout and text services. | Implement Stylo node/element adapters, stylesheet origins, selector matching, cascade, inheritance, custom properties, media and invalidation. Delete handwritten rule parsing/matching/cascade after mapped CSSOM and differential tests pass. |
| V8 sandbox, ICU and startup snapshots | Scaffolded | Production features request `v8_enable_sandbox`; runtime initialization is fail-closed, metadata fields exist, and build scripts request pointer compression and content-addressed outputs. Lite releases reject missing custom ICU/V8 inputs. | Build reproducible macOS ARM64 and Linux x86_64 full/lite archives; generate a true root/en/en-US ICU payload; install a stable external-reference table and generated-interface snapshot; prove linked heap-sandbox state. |
| Upstream conformance | Partial | A pinned sparse WPT cache exists outside Git. The native harness supports applicable `.any.js`, `.window.js` and testharness HTML cases through controlled loopback and records assertion-level outcomes. | Fix the current Range timeout, then run every required mapped WPT/ARIA/Accessible Name case. Keep deferred, excluded and informational results separate. |
| Product and release validation | Not started for this cutover | Focused Rust/V8 tests pass and historical product evidence remains available for its original commits. | Rerun framework/OAuth, MiniWoB, churn, replay, scanner, Python, MCP, platform security and benchmarks against one clean release candidate; privately sign and re-attest only after deterministic completion. |

## 2. Remaining implementation list

### Runtime correctness

- ~~Replace synchronous broker entry points with operation-based asynchronous
  start, headers, chunk, completion, cancellation and failure messages.~~ The
  operation path is now the production navigation and Fetch-body path; the old
  methods remain compatibility adapters.
- Ensure 32 concurrent operations never hold the page-command mutex while
  waiting for network I/O.
- ~~Feed accepted navigation bytes directly into the live parser rather than
  accumulating a decoded main-resource string.~~ Implemented with explicit
  demand, incremental charset state and parser-boundary resource calls.
- Schedule async/defer/module scripts and lifecycle transitions from real
  prerequisites; reject late completions after navigation or worker replacement.
- Define and test backpressure and cancellation across Fetch, XHR, streams,
  dynamic imports, stylesheets, frames and WebSockets.

### Semantic runtime

- Give semantic, style, tracing, custom elements, observers and snapshots
  independent mutation-journal cursors.
- Persist semantic nodes, order, relationships, accessible-name dependencies,
  affordances and reference generations without rebuilding the full snapshot.
- Generate observation deltas from graph revisions only.
- Retain the full projector as a test oracle until equivalence passes, then
  remove it from production features.

### Generated browser interfaces

- Finish Range boundary comparison, content mutation/extraction, insertion and
  surrounding behavior across text and element boundaries.
- Finish Selection mutation, editing, `contenteditable`, contextual fragments,
  focus and adoption behavior.
- Implement NodeFilter/`whatToShow` traversal semantics and live mutation rules.
- Complete same-document history traversal, state cloning and event ordering.
- Apply referrer policy consistently to navigation, Fetch/XHR, redirects,
  frames, scripts, stylesheets and form submissions.
- Complete IndexedDB version changes, blocked connections, transaction lifetime,
  indexes, cursors, compound keys, structured cloning, quota, abort and restart.
- Regenerate bindings and require `oryn-webidl check` to remain clean.

### Style and layout

- Adapt Oryn DOM nodes to Stylo's document, node and element traits without
  creating another canonical DOM.
- Maintain user-agent, author and inline stylesheet origins and revisioned
  invalidation state.
- Convert Stylo computed values into Taffy/Parley inputs and semantic visibility.
- Mark paint-only, fragmentation, unsupported writing-mode and other declared
  boundaries unavailable instead of returning plausible geometry.

### Runtime artifacts and profiles

- Produce custom sandboxed/pointer-compressed rusty_v8 archives for both target
  platforms; the absent official macOS archive is not an acceptable fallback.
- Produce and inspect the actual English-lite ICU data payload.
- Generate profile-specific snapshots containing intrinsic setup and generated
  interfaces with a stable external-reference table.
- Bind toolchain, GN options, V8, archive, ICU, snapshot and external-reference
  hashes into the worker manifest and runtime metadata.
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
2. **Partial:** navigation bytes and Fetch bodies use the operation path;
   subresources and the page-command actor still need scheduler integration.
3. **Implemented:** html5ever and V8 share the live `DomRuntime`; blocking
   scripts checkpoint at retained tokenizer boundaries and parser-inserted
   writes are consumed before the network/source suffix.
4. Remove the remaining production synchronous compatibility call sites after recorded-response shadow
   equivalence passes.

**Exit:** 32-request concurrency, HTTP/2, chunking, backpressure, abort, timeout,
redirect, CORS withholding, decompression and navigation-cancellation tests pass;
no production network path calls the blocking broker facade.

### Milestone C — complete semantic and WebIDL cutovers

1. Remove full semantic reprojection from normal observations and deltas.
2. Run randomized graph-oracle equivalence and expanded reference churn.
3. Complete Range/Selection/traversal/editing, history/referrer and IndexedDB.
4. Remove migrated handwritten JavaScript interfaces.

**Exit:** graph/oracle outputs are identical across the mapped corpus, false
reference preservation is zero, generated output is clean, and mapped DOM/API
tests pass without compatibility shims.

### Milestone D — replace the CSS cascade

1. Implement the Stylo DOM adapter and stylesheet set.
2. Connect mutation-driven invalidation and computed-style caching.
3. Feed computed values to Taffy/Parley and semantic visibility.
4. Remove the handwritten parser, selector matcher, specificity and cascade.

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

1. Finish the async broker interface and migrate scripts, stylesheets, frames
   and dynamic modules away from blocking `send` calls.
2. Fix the upstream Range timeout using an isolated lifecycle/testharness
   regression before broad WPT execution.
3. Expand semantic graph-oracle tests to framework churn and navigation while
   the transport work is under review.
4. Start the Stylo adapter only after the DOM mutation and style-invalidation
   cursor contract is stable.
5. Build custom V8 artifacts after software cutovers stabilize, avoiding costly
   rebuilds for a moving external-reference table.

## 5. Completion rules

- A focused unit test is implementation evidence, not gate completion.
- A compatibility shim cannot silently stand in for a generated or native API.
- A timeout, missing artifact or unsupported behavior is recorded as such.
- Historical evidence remains tied to its original source and schema.
- No live side-effecting shadow request or action is duplicated.
- Release and hosted-model work starts only after deterministic software gates
  pass against the exact candidate.
