# Oryn2 runtime completion action plan

**Created:** 2026-08-31
**Historical clean diagnostic source:** `e1b97d7` (`Implement the Image legacy factory`)
**Status reconciled:** 2026-09-06
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
| Upstream conformance | Partial | The historical in-process inventory at clean commit `e1b97d7` records: 2,267/4,022 pass (generated-WebIDL 2,113/3,407, semantics 41/51, Stylo computed values 113/564), with 1,527 assertion, 146 harness, 54 timeout and 28 unclassified failures. Exact root totals are IndexedDB 304/456, WebIDL 26/58, Accessible Name 13/19, CSS 113/564, DOM 269/662, editing 118/704, HTML 30/136, referrer policy 1,366/1,391 and WAI-ARIA 28/32. This is a net gain of 20 over the preceding exact run: 32 new passes and 12 referrer-policy regressions. The runner applies WPT's four-times deadline to tests declared `timeout=long`, so ordinary and long-test failures are not conflated. The result is in-process diagnostic evidence; the release validator rejects that execution mode and includes four probe-inferred passes without verified completion. The explicit v2 inventory and runner have since been verified to contain exactly the same 4,022 IDs; accidental v1 defaults and dynamic fallback are corrected in the development worktree. | Establish a complete production-worker baseline with verified completion, then close the measured DOM/editing, CSS/Stylo, long-tail WebIDL and accessibility differences before publishing a validator-derived production-worker ledger. |
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
  generated/Rust-backed. Agent-critical commands now use bounded undo units,
  operation-aware paragraph-split undo, live-range-preserving text mutation,
  cross-node deletion, selection-preserving formatting and trusted InputEvent
  dispatch. Focused upstream deletion, split, undo and formatting-selection
  cases pass. Long-tail editing conformance remains in the upstream inventory
  rather than being silently inferred from the focused results.
- ~~Implement NodeFilter/`whatToShow` traversal semantics and live mutation
  rules.~~ Generated TreeWalker and NodeIterator now apply callback filters,
  `FILTER_ACCEPT`/`REJECT`/`SKIP`, `whatToShow`, and live Rust node identity.
- ~~Complete same-document history traversal, state cloning and event
  ordering.~~ Generated History owns cloned state, same-document URL updates,
  traversal, `popstate`, `hashchange`, and `scrollRestoration`.
- Validate the implemented referrer-policy calculation across navigation,
  Fetch/XHR, redirects, frames, scripts, stylesheets and form submissions; fix
  only failures exposed by the mapped inventory.
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
3. Run the mapped referrer, IndexedDB transaction/version-change and broader
   editing inventory; focused IndexedDB and editing slices are already closed.
4. Delete the probe-only handwritten interfaces after the compatibility probe
   is no longer required by historical differential tests.

**Exit:** graph/oracle outputs are identical across the mapped corpus, false
reference preservation is zero, generated output is clean, and mapped DOM/API
tests pass without compatibility shims.

### Milestone D — replace the CSS cascade

1. **Implemented for style resolution:** Stylo owns stylesheet/media/declaration
   parsing, style-rule matching, specificity and computed-value resolution. DOM
   query APIs also use Stylo against the canonical tree. A repository-owned Stylo
   patch enables `:has()` and `:nth-child(... of ...)`; the combined CSS-filtered
   signed-worker run adds 82 selector passes without regressions.
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

## 4. Prioritized execution backlog — reviewed 2026-09-05

This section replaces the stale immediate list (45 timeouts and CSS 90/564).
The historical in-process baseline has 54 timeouts and CSS 113/564.
The latest complete signed-worker run is 2,467/4,022, with 2 new passes
and no regressions versus the namespace candidate. The complete root counts
are in [implementation status](IMPLEMENTATION_STATUS.md). All required IDs and
passing harness completion have been verified. The full ledger includes collection tracing, the queue fix, clone ownership and
namespace adapter fixes. New static indexed properties and HTMLCollection await
signed-worker validation. See
[the current review](IMPLEMENTATION-GAP-REVIEW-2026-09-05.md) for scope and checks.
Milestone A's uncommitted-foundation instruction is historical: the review began
at clean `b21c171`, whose changes since diagnostic `e1b97d7` are documentation only.
Do not repeat completed architectural cutovers; use the following order.
Owners below are suggested engineering roles, not assigned people.

| Priority / work item | Suggested owner and dependency | Deliverable and acceptance criteria |
|---|---|---|
| P0.1 — execution inventory | Implemented and verified | Runner and validator use the locked v2 inventory of 4,022 IDs, including variants and worker globals. Exact set equality and verified harness completion are checked. Missing, duplicate, unexpected and mixed-fingerprint results and probe-mode release evidence remain rejected. |
| P0.2 — production execution baseline | Implemented and verified | Complete signed App Sandbox worker ledgers cover all 4,022 cases with source, binary, profile, fixture and inventory fingerprints. Failed diagnostic runs remain failed and cannot qualify a release. New candidates receive separate full ledgers. |
| P0.3 — eliminate regression and diagnosis ambiguity | Runtime/tooling; begin with existing diagnostics, confirm against P0.2 | Use the latest signed-worker ledger to classify runtime, assertion, harness, incomplete, and timeout outcomes. Preserve completion/task traces and upstream long-test deadlines. Confirm the header/meta CSP fix in the complete candidate. Exit with no unexplained outcomes, no known referrer regressions, and each remaining failure assigned to a reproducible cause. These categories may change under production execution. |
| P1.1 — DOM, editing and browser API correctness | DOM/WebIDL; after trustworthy triage | Cluster the 365 DOM, 586 editing, 115 IndexedDB, 106 HTML and 29 WebIDL non-passes by first failing assertion/shared algorithm. Prioritize Range/Selection, task/lifecycle ordering, descriptors/collections, history and database transaction scheduling as evidence warrants. Fix each cluster with a focused regression and production-worker confirmation; run the complete affected root and compare newly passing versus regressed IDs. Focused 100/100 IndexedDB and corrected 40/40 editing do not close the broader roots. |
| P1.2 — CSSOM and style correctness | Style/layout; alongside P1.1 after P0.2 | Triage the 338 CSS non-passes. Cover stylesheet ownership, constructed/adopted sheets, nested rules, selector/value behavior and invalidation where failures confirm gaps. Publish pinned native-versus-Chromium computed-value/geometry differentials, including the supported grid bridge. Keep unsupported geometry explicit. Exit with the mapped CSS inventory and required differentials passing. |
| P1.3 — semantics and reference stability | Semantic runtime; coordinate style-dependent cases with P1.2 | Close four Accessible Name and four WAI-ARIA non-passes. Expand graph/oracle equivalence across randomized mutation, frameworks, navigation, shadow/slots, frames and replay. Exercise style visibility and naming dependencies. Exit with 51/51 mapped cases, zero false reference preservation and exact-candidate churn evidence. |
| P1.4 — finish runtime artifact matrix | Platform/build; start alongside P0 | Build Linux x86_64 full/lite in CayVM; refresh macOS full schema-v3 packaging and validate macOS lite. Bind archive, V8, ICU, snapshot, external references and toolchain hashes. Test heap sandbox, pointer compression, locale policy, snapshot loading and fail-closed tamper rejection on all four profiles. Use these profiles in production validation; final attestation waits for source freeze. |
| P2.1 — integrated candidate qualification | Release/test; after P1 correctness and profiles | On one clean candidate, run the full 4,022-case production ledger, four-profile equivalence, framework/controlled OAuth, MiniWoB 24/24, scanner, Python, MCP, churn, replay and platform sandbox-denial suites. Run the canonical formatting, Clippy, generated-binding and Rust checks. Recompute runtime-completion evidence independently. Any runtime fix invalidates affected candidate evidence and requires a new fingerprint. |
| P2.2 — performance qualification | Runtime/performance; measure early after P0.2, final run after P2.1 | Measure shipped-V8 cold startup, warm page, parser/semantic update, RSS, 20-page concurrency, cancellation and crash replacement. Historical no-V8 probe numbers cannot qualify production. Meet the 250 ms cold-start target or document an explicitly reviewed exception; publish reproducible samples and exact artifact hashes. |
| P3 — final attestation | Release; after deterministic gates pass | Freeze and privately sign the exact macOS release bundle, rerun deterministic checks on final bytes, and publish the immutable artifact/evidence record. Only then run the 96-cell Azure/Qwen panel, carrying historical $7.996254 spend within the established $100 ceiling. Private release attestation and paid model calls remain outstanding. |

### First conformance implementation batch

The worktree now makes v2 the tooling default, uses locked inventory variants,
rejects probe-inferred completion as a pass, and executes open-ended editing
ranges through their actual last assertion. Exact ID equality and failure
classification have regression coverage. The fixture server queue accommodates
parallel resource bursts without extending runtime limits.

Verified through a signed App Sandbox worker (development evidence):

- IndexedDB bulk reads: **28/28** pinned cases pass. Added generated
  `IDBRecord`/`getAllRecords`, options/direction/count handling, and shared
  structured-clone multi-entry key extraction.
- Accessible Name: **15/19** pinned cases pass. Shadow composition, slot
  assignment/fallback, descendant labels, and embedded control values are
  corrected. The remaining four cases require generated CSS content, counter
  resolution/invalidation, and list markers in accessible names.
- Two previously crashing referrer inheritance cases now pass after the
  style-only mirror stopped running Blitz's unrelated layout phase. The wider
  referrer slice still has failures and is not closed.

Raw local ledgers are under `artifacts/upstream-conformance/`:
`conformance-worker-idb-bulk-verified.json`,
`conformance-worker-accname-verified.json`, and
`conformance-worker-referrer-crashes.json`. They bind their own binary hashes;
these focused totals must not be combined into a full-candidate pass count.
The complete 4,022-case run finished at **2,261/4,022**, using the earlier
isolated worker candidate while later fixes were validated separately. Neither
is release attestation. See [the progress report](CONFORMANCE-PROGRESS-2026-09-05.md)
for all root totals and failure classifications.

Stylo already owns parsing, selector matching, cascade, and computed values.
The CSS closure work must distinguish Oryn CSSOM/mirror/invalidation/export
defects from demonstrated Stylo defects. In particular, the current style
snapshot exports primary-element values; semantic name construction does not
yet consume generated pseudo-element content or resolved counters/markers.
Extend that integration using Stylo-computed data rather than adding a second
CSS parser or cascade. Keep Taffy/Parley layout responsibilities distinct.

Next shared-cause work is CSS-generated accessibility content, DOM realm and
wrapper identity, editing command return/state/selection behavior, stylesheet
and selector handling, and remaining IndexedDB transaction scheduling. Each
requires its full affected root rerun before the final unified inventory run.

### Next confirmed shared causes

1. Object-store renaming and abort semantics are verified in both globals. The
   complete signed-worker IndexedDB root is 337/456, with eight new passes and
   no regressions against 329/456. Continue the remaining 119 non-passes around
   transaction scheduling, origins, cloning/blob support, and runtime limits.
   Parser text batching, verified in the full inventory, reduces primary RSS
   failures from 186 to three. Compact Blob serialization passes both targeted
   globals on the later candidate. Preserve limits and verify its full root.
2. Document adoption, ObservableArray mutation/ownership/lifetime, and invalidation
   are implemented; both exposed regressions and the two WebIDL ObservableArray
   tests pass. Complete shadow style isolation and scoped stylesheet application.
   The pinned Blitz adapter leaves shadow-root style hooks unimplemented, so this
   requires an explicit adapter/dependency change, not global sheet injection.
3. Stylo now executes production DOM selector queries against the canonical tree;
   complete roots verify six DOM and nineteen CSS gains without regressions.
   The repository-owned dependency patch enables `:has()` and filtered nth-child
   with matching and recascade tests; its CSS-filtered run adds 82 passes. Complete
   remaining dynamic pseudo-class, shadow scope, and XML-document state integration. Do not edit
   the registry cache or add a second CSS parser.
4. Connect typed Stylo generated content/counters/markers to accessible names,
   including mutation invalidation, then rerun all 19 Accessible Name cases and
   the incremental semantic differential suite.
5. Continue DOM/iframe and editing algorithm clusters, then execute the unified
   inventory and profile/security/product gates on the same final candidate.

### Execution cadence and scope control

- First implementation batch: P0.1 plus referrer reproduction and failure-ledger
  classification. Run a production smoke slice as soon as IDs reconcile.
- Next batch: complete P0.2 and choose shared-cause P1 fixes from its measured
  failures. Begin platform builds independently; inspect startup early to avoid
  discovering a release-blocking performance problem at the end.
- For every batch, report source fingerprint, execution mode, full required
  count, passed/failed/harness/timeout/unknown counts, newly passing IDs and
  regressions. Never merge totals from different binaries into a candidate run.
- Keep pinned upstream-defect exclusions narrowly source/assertion-bound and
  preserve raw failures. Do not reduce the frozen required scope to improve a
  pass percentage.
- Estimate calendar completion only after the production baseline identifies
  shared causes. The 1,755 non-passes are execution variants, not 1,755 independent
  implementation tasks.

## 5. Completion rules

- A focused unit test is implementation evidence, not gate completion.
- A compatibility shim cannot silently stand in for a generated or native API.
- A timeout, missing artifact or unsupported behavior is recorded as such.
- Historical evidence remains tied to its original source and schema.
- No live side-effecting shadow request or action is duplicated.
- Release and hosted-model work starts only after deterministic software gates
  pass against the exact candidate.

### Measured collection and template follow-up (2026-09-06)

The parser candidate's editing failures include 960 failed subtests reporting
`editor.childNodes.item is not a function`. These are subtests, not 960 inventory
executions. The declared `Node.childNodes` still returns `sequence<Node>`, and
bindings expose an Array snapshot. Replace this with a branded, live NodeList
whose indexed access, `item`, length, iteration, SameObject identity, and native
lifetime remain correct across mutation and wrapper collection. Static selector
results need NodeList branding while preserving snapshot membership. Adding an
`item` function to arrays alone does not close this work. Verify the DOM root
and the full editing whitespace cluster after focused mutation/GC tests.

Several CSS `@scope` fixtures obtain their DOM from parsed templates. The parser
previously kept template-content identity only in a private map that was lost
at binding installation. The development fix moves that identity into the
canonical DOM and has passing parser, cloning, import, serialization, lifetime,
and binding regressions. Confirm this in the next signed worker, then separate
remaining Stylo scope/cascade failures from empty-template setup failures.

### MutationObserver queue ownership follow-up

The native NodeList regression exposed that `takeRecords()` currently sees only
records collected at the host delivery checkpoint, so it misses mutations made
earlier in the same script. Close this at native mutation enqueue time, with
registration-specific options and ordering; do not make takeRecords drain the
shared journal or hide mutations from layout/other realms. Include observe-time
boundaries, multiple observed targets, disconnect/re-observe, subtree removal,
callback-created mutations, and delivery/record lifetime tests. The current
bounded journal alone is insufficient as a lossless observer queue.

### Stylo scope ownership follow-up

Explicit `@scope` now uses Stylo's existing algorithms. The remaining implicit
scope work needs stylesheet owner/tree identity carried through layout input and
the production Blitz/Stylo sheet adapter. Stylo already exposes
`StylesheetInDocument::implicit_scope_root` and `ImplicitScopeRoot`; connect
those hooks to style/link owners, imported sheets, constructed sheets and shadow
hosts instead of rewriting scope selectors. Verify owner moves, removal,
disabling, adoption, nested imports and recascade. Shadow stylesheets need
isolated author cascade integration and native host/slot relationships before
claiming shadow scope conformance.

### Remaining Range RSS investigation

The signed collection/Range worker still exceeds RSS in Range-insertNode.
Its fixture repeatedly clones/replaces two iframe document trees within one
synchronous handler. The shared journal retains removed subtrees until delivery;
old evicted leases now release at that checkpoint, but this does not reduce
within-handler peak retention. Measure native arena/journal/wrapper ownership
through those iterations. Audit all journal consumers before separating observer
record roots from style/semantic mutation history; simply dropping required
leases or increasing the RSS limit is not a valid closure.

With NodeList available, the Chrome-compat whitespace delete fixtures reach
95/584 and 91/602 passing subtests respectively. Remaining failures include
space/non-breaking-space normalization. These files explicitly describe
browser compatibility behavior; retain them in the locked inventory while
tracking them separately from missing API failures. Whole files still fail.

### Native HTMLCollection follow-up

At least 29 failing DOM files cover HTMLCollection, children, or getElementsBy*
behavior. Source now extends the existing native NodeList collection owner with
the generated HTMLCollection interface, live element children, literal
tag/class/namespace filters, indexed and unenumerable named properties, and
namedItem. Local tests cover tree order, live mutation, SameObject children,
wrapper tracing and collection cycles, and separate NodeList/HTMLCollection
receiver brands. All 100 enabled V8 tests and seven WebIDL tests pass; generated
bindings and Clippy checks pass. The signed worker passes all eight HTMLCollection files and six static NodeList
stress files. The complete DOM root is 322/662, with 25 gains and zero
regressions; WebIDL remains 29/58, with zero regressions. Do not implement tag/class lookup by
constructing CSS selectors from literal API arguments. Verify supported-name
visibility against own/prototype properties, numeric-index precedence,
read-only indexed/named descriptors, collection-as-prototype writes, iteration,
GC cycles, and HTML/SVG namespaces. Document mode/quirks-dependent matching and
cross-document adoption need explicit evidence rather than inferred support.
The pinned `Element-getElementsByTagName-change-document-HTMLNess.html` also
requires capturing HTML-document matching mode when the collection is created:
an existing collection keeps its original matching rule after adoption, while a
new query uses the new document's mode. Preserve parser document mode in native
state before implementing this; checking the root element namespace is insufficient.

### Document target state

The iframe clone ownership fix leaves only :target failing in the 669-subtest
Element-webkitMatchesSelector file. The native Stylo query adapter currently
returns false for this pseudo-class. Add document-owned target-element state and
update it through fragment navigation/initial document activation, then expose
the same state to DOM selector matching and stylesheet traversal. Preserve the
HTML indicated-element algorithm's raw-fragment lookup before percent-decoded
lookup, ID-before-legacy-anchor-name precedence, and document-tree order. Do not
replace this state with an on-query ID lookup: target-element identity is set by
navigation and must not silently change when an ID is edited. Test iframe-local
ownership, same-document navigation, removal/identity, empty/top fragments,
percent escapes, and stylesheet invalidation. Reference: HTML scrolling-to-a-
fragment and indicated-part algorithms.

### Remaining collection-adjacent algorithms after signed confirmation

The complete HTMLCollection candidate DOM run gains 25 files without regressions.
The qualified-name creation fix preserves case and splits prefix/local name;
signed evidence now closes both Document/Element tag-name and namespace-query
files plus firstElementChild namespace behavior. The complete DOM root reaches
327/662 (30 gains and zero regressions against the latest full baseline), and
CSS remains 226/564 with no regressions.
`getElementsByClassName-14.htm` requires parser quirks mode to survive into native
document state. Cases 20, 21, 22 and 25 depend on table row collections and
`deleteRow`, rather than the class filter. Namespace-creation coverage also
requires Element.prefix, correct namespaceURI nullability, qualified node names,
namespace validation, and per-document content type/HTMLness. Implement and test
these shared document/element algorithms before labeling their callers complete.

### Document-mode follow-up outcome

Native document metadata and parser quirks mode now survive cloning, import and
replacement; created XML/XHTML/SVG documents report their content types. Element
creation and live collection filters use document mode, with the filter mode
captured on collection creation. Signed DOM evidence is 329/662, two additional
gains and zero regressions versus the qualified-name candidate. All 100 V8,
28 DOM and 141 native tests pass. The quirks-mode class-name case and created
document content-type case are closed.

Next, connect MIME-aware XML parsing and iframe activation to this native
metadata, then verify the cross-document HTMLness case end to end. Complete
namespace validation/reflection (prefix, null namespace, qualified node names)
and DOMParser XML semantics without treating HTML parsing as XML support. Stylo
quirks integration must consume native mode through its adapter and retain
separate production evidence. Table row APIs, Range/editing, CSSOM and lifecycle
work remain open as listed above.
