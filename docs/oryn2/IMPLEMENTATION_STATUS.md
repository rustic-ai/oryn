# Oryn2 implementation status

> **Current priority: complete conformance. Not a release candidate.**
> Follow the [active execution plan](COMPLETE-CONFORMANCE-PLAN-2026-09-06.md).
> Latest full measurement: **2,906/4,022 (72.3%)**, **1,116 remaining**.

The latest complete signed-worker run at `c43d732` measures **2,906/4,022 (72.3%)**,
with **1,116 failing executions**. Compared with `70352c7`, it gains **27 whole-test
passes** and loses **1**, for a net gain of **26**. CSS gains 15 executions and editing
gains 12; DOM loses one. These are measured full-inventory results, not selected-run overlays.

The full audit finds **3 observed assertion regressions** in the lost DOM execution:
`Node-appendChild-script-and-style.html`. Fragment and multi-node insertion run an
earlier script before a later stylesheet sibling is connected. The new stylesheet
lifetime checks expose this insertion-order defect; the regression is not waived.
An isolated fix attaches and prepares the full insertion batch before executing its
post-connection scripts. It is pending validation and is not part of this measured candidate.

There are **45 newly unobserved prior passing assertions** in incomplete Range
runs (25 dataChange, 9 insertData, 11 replaceData). Five history assertion-name changes
reconcile only after the complete normalized name/status multisets match. All five
previously missing appendData assertions are now observed passing, but that execution
remains incomplete. The older replaceData assertion remains unobserved. The **11
historical editing assertion regressions still fail**. None of these gaps is closed by
selected test results or by the net increase in passing executions.

The audit verifies all 4,022 unique IDs, nonempty successful passing completions,
frozen source/binary fingerprints, and unchanged inventory, resource limits and sandbox
boundaries. [Full evidence](CONFORMANCE-NATIVE-CSSOM-FULL-2026-09-07.json) records gains,
losses, assertion deltas, retained historical gaps and native/selected evidence hashes.
This remains an incomplete conformance result, not a release candidate.

Stylo remains the CSS engine. Native rule projection now retains whole-sheet namespace
and nesting context; style declarations and read-only rule lists are live; stylesheet
identity survives owner replacement and detachment. Both first-line selector spellings
use the native parser. Full-run CSS totals are cascade 49/97, CSSOM 85/190 and selectors
162/277. Editing retains 389/704, including all 180 whitespace-join executions.

The immediate work is to fix the insertion-order regression and validate the shared
native rule-list mutation path. The isolated draft routes stylesheet insert/delete
through Stylo's parser and hierarchy checks, with tests for namespaces, error precedence,
conversion side effects and wrapper identity. Then complete nested rule ownership,
live child collections, grouping mutations and generated typed interfaces. Media lists,
other CSSOM interfaces, Range resource failures and the remaining inventory stay open.

| Root | Passed / required |
|---|---:|
| IndexedDB | 341/456 |
| WebIDL | 30/58 |
| accname | 15/19 |
| css/css-cascade | 49/97 |
| css/cssom | 85/190 |
| css/selectors | 162/277 |
| dom | 392/662 |
| editing | 389/704 |
| html/browsers/history | 32/136 |
| referrer-policy | 1383/1391 |
| wai-aria | 28/32 |

The [failure backlog](CONFORMANCE-FAILURE-BACKLOG-2026-09-06.json) contains exactly 1,116 failed IDs from this run.

This status is intentionally capability-based. It records implemented behavior and
does not imply that later browser-conformance work is complete.

Committed result payloads retain the source fingerprints and versioned field
names from the runs that produced them. The capability-naming refactor does not
rewrite those historical payloads or present them as a new validation run.

## P0 — complete

- Reuse ownership, dual-license policy for new packages, and frozen compatibility
  backend boundaries are documented.
- The Rust, scanner, extension, Python SDK, and IntentGym baselines are captured
  in `benchmarks/baselines/p0-a66a6680.json`.
- IntentGym now resolves the in-repository Python SDK portably; its lockfile no
  longer contains a developer-machine absolute path.
- The versioned result schema and fixed 30-task semantic loop corpus are committed. The
  corpus validator checks counts, unique IDs, read-only canary policy, and all
  required local sources.

## Runtime foundation — implemented; compact-parent re-attestation pending

- Raw V8 and `deno_core` host probes use the same V8 revision. Their historical
  startup/size comparison selected raw V8, but the mandatory concurrent-page
  evidence used a no-V8 probe and must be rerun with the shipped worker bundle.
- `html5ever` builds the generation-safe live `DomRuntime`. Generated V8
  wrappers carry `NodeId`, realm, brand, and document generation; JavaScript
  DOM operations call Rust directly. The former JavaScript-owned graph and
  full-document export/synchronization implementation has been deleted.
- Host-broker network policy, trace spine, and typed page actor are implemented.
- The JSONL page worker supports load, observe, direct semantic actions, OIL,
  and orderly shutdown without exposing filesystem or network commands.
- The historical 20-page benchmark measured a 0.368 ms median load+observe
  batch, 4.59 MB peak RSS, and a 2.38 MB probe, but it did not include V8 and is
  not valid production concurrency/RSS evidence. It must be replaced with a
  shipped-worker measurement.
- The worker is packaged as `OrynPageWorker.app`, signed by the trusted Apple
  Development identity `9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90`, and
  verified with bundle identifier `ai.rustic.oryn.page-worker`, team identifier
  `5HVA9VFF8K`, and `com.apple.security.app-sandbox=true`.
- The sandboxed process launches and completes the versioned JSONL `ping` and
  `exit` exchange. Machine-readable signing, sandbox, protocol, startup, page
  creation, concurrency, RSS, and size evidence is committed at
  `benchmarks/evidence/runtime-foundation-macos-arm64.json`; raw samples remain ignored.

## Proposal-v1 runtime completion — in progress

- `oryn-dom`, `oryn-js`, and `oryn-net` provide the Rust-owned live arena,
  generation-checked wrappers, generated interface installation, realm
  infrastructure, resumable html5ever parsing and an async rustls/HTTP2
  transport foundation.
- The production semantic path uses a persistent Rust-owned `SemanticGraph`
  fed by the multi-consumer DOM mutation journal. Contract v6 publishes
  scoped semantic/accessibility/structure observations, field-level trust and
  evidence, frame ownership, exact graph deltas, and graph-native resolution.
  Equal-ranked candidates fail with typed ambiguity, and spatial targets or
  scopes fail explicitly when authoritative geometry is unavailable. The full
  projector is now a test/`semantic-oracle` probe rather than a production
  observation path.
- Navigation now creates the V8 executable immediately after accepted response
  headers and feeds incrementally decoded, demand-driven chunks directly into
  html5ever over the shared live `DomRuntime`; it no longer accumulates the
  main resource first. Navigation consumes correlated broker operations rather than converting
  the live HTTP body back into `Read`. Broker v4 multiplexes header, chunk,
  completion and failure frames across the sandbox boundary; cookie, policy,
  cache and replay adapters preserve those streams. Parent-owned Fetch bodies
  use explicit demand so accepted headers do not pull response bytes until the
  worker reads. The old synchronous broker methods remain compatibility adapters
  only. Production navigation, Fetch/XHR, images, CORS preflights, scripts,
  module graphs, stylesheets/imports, frames, dynamic scripts and other
  subresources use correlated operations. Async scripts execute by completion
  order while defer/modules preserve document order. Abort, timeout, stale
  completion, navigation-body drop, superseding navigation, page close, IPC
  cancellation and worker-service teardown cancel their affected operation.
- html5ever and V8 now share one live `DomRuntime` from the first parsed tag.
- The generated-binding closure now covers generation-checked same-origin
  cross-realm wrappers, canonical security tokens, named window properties,
  DOMImplementation, Attr/DocumentType, cross-container Range algorithms,
  Selection direction and mutation, callback-filtered TreeWalker/NodeIterator,
  CharacterData editing, same-document History and generated Storage. Dynamic
  and parser-created same-origin frames execute in separate V8 contexts in the
  page isolate. The full native V8 library suite passes with these bindings.
  Generated Rust callback dispatch now also owns the declared Blob, File,
  FormData, Headers, Abort, Streams, Request/Response/Fetch, XHR, WebSocket and
  IndexedDB object and algorithm surface. These objects communicate with the
  parent exclusively through typed asynchronous HTTP, WebSocket and database
  operations; promise/event progression, bounded streaming, abort, XHR timeout
  and late-completion rejection occur at V8 scheduler checkpoints. Production
  restores this generated namespace after bootstrap. Handwritten compatibility
  classes remain only for the explicitly named in-process probe and are not the
  secure worker's page-facing implementations.
  Parser-blocking scripts execute at the retained tokenizer boundary, V8
  microtasks/reactions/observers checkpoint before parsing resumes, and
  `document.write` re-enters the tokenizer ahead of the unconsumed suffix.
  Parser-discovered scripts, stylesheets/imports, frames, meta CSP, and meta
  referrer policy cross typed parent operations at their discovery boundary.
  The former post-parse DOM-tail suspension, decoded-main-resource buffer, and
  snapshot handoff are absent from production source loads.
- Generated bindings are authoritative for the declared DOM and platform API
  inventory. Complex editing, history/referrer edge cases, complete IndexedDB
  connection/version-change scheduling and the broader mapped upstream API
  inventory remain conformance work; they are not claimed from focused product
  regressions.
- Broader release validation now uses the additive, locked
  `runtime-api-style-semantics-v2` manifest. It preserves the pinned WPT commit
  and all 3,184 v1 files, adds every HTML-declared test variant, and therefore
  expands the required execution ledger from 3,462 to 4,022. The runner loads
  transitive `META: script` dependencies, supports bounded parallel execution,
  writes atomic checkpoints, and resumes only on an exact source, binary,
  runtime, inventory, and selection fingerprint. Initial diagnostic shards are
  intentionally not committed as passing release evidence. The two live
  BFCache workflows that kept the clean first-20 diagnostic at 18/20 now pass
  focused release execution and the replacement exact-tree first-100 run. The
  previously failing cross-origin iframe/window origin-
  isolation case now passes through opaque frame realms, brokered popup
  messaging, origin-partitioned storage, and autonomous receiving-page wakeups.
  The latest exact-tree first-100 IndexedDB run passes 100/100 after adding
  atomic read/write transaction rollback, compound key paths,
  transaction-scoped upgrade restoration, live cursor refresh/invalidation,
  opaque-origin rejection, cross-realm receiver branding, stable parser-time
  frame proxies, ordered blocked deletes, generated `IDBKeyRange.includes`,
  and bounded high-volume/multi-entry execution. The complete
  discovered cursor slice passes 11/11, including a 997-record iteration case.
  Cursor task-boundary, connection-close/versionchange, failed-upgrade ordering,
  index-key re-entry, and exact WTF-16 index names now pass. A generated
  MessageChannel transfer regression also passes after the aggregate and makes
  detached binary IndexedDB keys fail with `DataError`. The WPT harness now
  uses a real localhost/127.0.0.1 origin pair, avoids injecting completion code
  into helper documents, retains diagnostics on harness failures, and gives
  long tests a declared bounded wait. The harness also applies its pinned
  wptserve substitutions to `.sub.html` as well as `.sub.js`. Parser-created
  and dynamic cross-origin frames now execute in isolated Rust DOM/V8 realms,
  receive origin-aware parent/opener proxies before their lifecycle events,
  resolve child resources against the child URL, and remain opaque to parent
  DOM and semantic traversal. Focused partitioned-storage basic and persistence
  workflows pass. The broader coverage document now terminates its remote
  enclosing testharness and passes all 31 reported assertions, including
  ordered deletion and its stress loop. BFCache retains the suspended page
  realm and live IndexedDB connection or transaction, dispatches persisted
  `pagehide`/`pageshow`, restores the exact history entry, and rejects stale
  cross-document state. Due timer tasks now preempt later deadlines, and page
  task draining no longer sleeps until a future timer while parent context
  messages are waiting. Those scheduler fixes close the final
  `idb-partitioned-basic.sub.html` and `idbfactory_deleteDatabase.any.js`
  harness-completion failures.
  File selection now exposes real `File` objects through the parent-owned upload path, and the
  focused upload geometry case passes with generated `getClientRects`. A
  comparable non-EditContext editing shard now passes 40/40 after narrowly
  excluding eight invalid assertions from two tentative upstream files through
  a revision-, source-hash-, variant-, assertion-name-, and source-predicate-
  checked defect ledger. The untouched raw result remains 32/40: four
  assertions set `title` but remove `style` before expecting no attribute, and
  four pass numeric bitwise-XOR output to strict `assert_true`. All 564 valid
  internal assertions pass. Focused deletion,
  contenteditable-boundary deletion, split-attribute cloning, moved-node undo,
  definition-list/list flattening, copy/paste across a non-editable boundary,
  End-key behavior, and open/closed shadow-root designMode isolation now pass;
  generated-binding tests cover formatting and selection preservation. The
  CSS-variable clipboard materialization, window-level `DOMContentLoaded`,
  parser-time viewport exposure, Window named-element access, and
  block-boundary deletion around hidden form controls now pass their exact
  upstream cases. Cross-container deletion preserves invisible metadata,
  protects visible title/style/script boundaries, merges normal blocks across
  head/body boundaries, prefers the body-owned destination, and stops ancestor
  resolution at the current frame document element. The WPT automation adapter preserves the constructable
  `test_driver.click` shape used by the pinned legacy cases. The former eight
  tentative head/HTML variant failures contain no remaining valid engine
  assertion failure; their raw failures are the pinned defects above. Generated
  `CSSStyleDeclaration.whiteSpace` behavior
  now routes through Rust and closes the backward preformatted nested-block
  variants. Forward command deletion now removes a final separator, preserves
  the nested block, and moves the caret to its first insertion point. Generated
  immutable forward `beforeinput` target ranges close the keyboard `Delete`
  paths, so all eight nested-block variants pass. Generated
  input/textarea selection APIs plus composed-focus keyboard retargeting close
  the shadow-hosted text-control case, but general editing-engine conformance
  is not claimed.
- Generated IndexedDB now implements the browser key ordering used by
  `IDBFactory.cmp`, including dates, unsigned binary keys and recursive arrays,
  and exposes a real versionchange transaction during upgrade. Upgrade abort
  resets the initial database version, rejects the open request with an
  `AbortError` DOMException, and clears the request transaction. It also exposes
  generated request `source`/`readyState`, task-scoped transaction activity,
  cancelable request/transaction/connection error propagation, listener-object
  dispatch, Blob URL round trips, binary BufferSource keys, persisted index
  definitions, persisted index deletion, parent-owned schema rollback,
  generated `IDBVersionChangeEvent`, `IDBCursorWithValue`, bounded high-volume
  request scheduling, WebIDL count/version range conversion, deleted-store
  invalidation, asynchronous cursor advance/continue state, generated Event
  identity, database-list visibility during upgrades, index rename/rollback,
  multi-entry expansion, unique cursor selection, and update/delete exception
  ordering, atomic transaction commit/abort, compound key paths, and live
  cursor invalidation against the parent-owned transaction view. Focused pinned
  WPT now passes the complete discovered 11-file cursor slice and the full
  `IDBFactory.open` file,
  delete-database version events, historical interface checks, index `getAll`
  range enforcement and cursor source/class identity. The exact-tree first-100
  IndexedDB diagnostic now closes at 100/100; it remains local probe evidence,
  not exact signed release attestation. Generated
  Document editing now supplies `designMode`, safe command queries, and
  selection-backed insert-text/HTML, line/paragraph insertion, directional
  deletion and select-all. Unsupported formatting commands report `false`
  rather than claiming an algorithm that is not installed.
- The style service is now persistently fingerprinted as
  `stylo-stylesheets-selectors-0.20+oryn-cascade-v3`. Stylo parses complete
  stylesheets, media rules, selectors and declarations and supplies selector
  matching, specificity and native computed-value resolution. Those computed
  values feed Taffy/Parley and publish visibility back into the persistent
  semantic graph. The production cutover is complete; mapped CSSOM and
  Chromium-differential conformance remains required before the style gate can
  close.
- Production V8 features require the heap sandbox and release builds fail closed
  when profile inputs are absent. A reproducible macOS ARM64 `core-full` archive
  is now built from exact `rusty_v8` 150.4.0 source and git-linked dependencies.
  Its linked runtime assertion passes with the V8 heap sandbox and pointer
  compression enabled. The macOS ARM64 `core-lite` archive now uses an actual
  2,261,856-byte ICU 77.1 package whose locale trees contain only root, `en`
  and `en_US`; V8 installs it before initialization. Both profiles use a
  versioned stable external-reference table and a content-addressed startup
  snapshot, and the exact lite snapshot/worker pass locale and snapshot-load
  regressions. Linux archives and four-profile release attestation remain.
- `runtime-completion-v1` freezes the upstream blocking scope and provides
  cache, harness, schema and validation tooling. Focused Range, IndexedDB and
  editing slices now pass. The first complete current-source in-process
  diagnostic recorded 446/4,022 passes: generated-WebIDL 405/3,407,
  semantic/accessibility 0/51 and Stylo 41/564. It also exposed a WPT server
  alias and sparse-cache dependency defect, both now corrected. The controlled
  runner now delegates unknown responses to the pinned upstream `wptserve`, so
  Python handlers, sidecar headers, redirects and substitutions no longer rely
  on SimpleHTTP approximations. A 20-case history rerun remains 9/20, proving
  that slice is an engine/lifecycle backlog; an IndexedDB dynamic-handler case
  now terminates deterministically rather than timing out. The complete,
  clean-tree `e1b97d7` diagnostic executes all 4,022 variants and passes 2,267.
  Exact root totals are IndexedDB 304/456, generated-WebIDL 26/58, Accessible
  Name 13/19, CSS 113/564, DOM 269/662, editing 118/704, HTML 32/136, referrer
  policy 1,366/1,391 and WAI-ARIA 28/32. The 1,755 failures comprise 1,527
  assertion failures, 146 harness failures, 54 timeouts and 28 unclassified
  results. The tested binary is 53,172,224 bytes with
  SHA-256 `a42f8ebdd905d1fbebd368da7f2920e1803830e51c75cb584d755508c29dae42`;
  its run fingerprint is
  `549b792b132278f60512d4a98383d44dcd290c90aa3a33b0eb6bb3767e377833`.
  This remains in-process diagnostic evidence, not signed production-worker
  attestation. Relative to the preceding exact run, 32 cases became passing and
  12 previously passing referrer-policy cases regressed, for a net gain of 20.
  That churn makes referrer-policy attribution a priority regression slice, not
  a closed subsystem. The release validator correctly rejects this diagnostic
  execution mode; it also currently expands 3,462 static manifest IDs while the
  pinned runner and prior exact ledger contain 4,022, so validator/runner
  inventory equivalence must be repaired before release evidence publication.
  The Observable surface now passes 47/52 exact WPT variants;
  remaining failures isolate iterator-abort exception propagation,
  abort/promise microtask ordering and detached-realm callback suppression.
- Generated click/form default actions now cross a typed binding-to-page queue,
  named iframe targets resolve through the Rust frame registry, and dynamic
  nested frames retain their owning realm through navigation and page-wide
  scheduler checkpoints. Focused top-level and nested named-frame regressions
  pass. The representative referrer-policy case now reaches a deterministic
  assertion instead of its former timeout, but its upstream result still loses
  `sourceContextUrl`. The full recomputation is now complete and exposes 25
  referrer-policy failures, including 12 regressions from the preceding exact
  run; referrer-result attribution therefore remains open.
- Generated CSSOM now includes a Rust-backed constructable `CSSStyleSheet`, a
  live same-object `CSSRuleList`, indexed insert/delete and
  synchronous/asynchronous replacement. Its generated-binding tests pass, and
  the first 30-case CSSOM rerun improves from 0/30 to 4/30. `CSSRuleList`,
  constructable `cssRules`, regular-sheet insertion constraints and replace
  thenable behavior now pass their exact upstream cases. The remaining failures identify concrete
  CSS namespace, broader rule interfaces, adopted stylesheet and
  computed-value work; the Stylo evidence gate remains open.
- The WPT testdriver accessibility hooks now call the same Rust
  accessible-name and computed-role implementation used by the persistent
  semantic graph; no parallel JavaScript naming heuristic is introduced. The
  51-case pinned semantic/ARIA inventory improves from 0/51 to 16/51. Remaining
  failures are now algorithm-level vectors for hidden/reference traversal,
  embedded controls, pseudo content, shadow slots, `aria-owns`/`aria-actions`
  and long-tail whitespace rather than a missing testdriver bridge. Focused
  regressions additionally cover hidden descendants, adjacent inline text,
  authoritative empty `alt`, and dialog-heading naming.
- The V8-host native library regression passes 227 tests, including
  32 simultaneous context streams, demand-driven body reads, production HTTP/2,
  redirects, decompression limits, stale/timeout cancellation, teardown
  cancellation, page-actor responsiveness, semantic graph publication and
  Stylo selector/cascade regressions. The default workspace suite, strict
  default-profile Clippy, formatting, and Python SDK tests/lint pass. The
  strict workspace all-feature Clippy profile now passes using the locally
  built content-addressed sandbox archive. The serialized production-topology
  sandbox suite passes 227/227 tests, including controlled loopback HTTP/2,
  Fetch/XHR, WebSocket, navigation and frame fixtures. These results do not
  close Linux/four-profile attestation or replace signed packaged-artifact validation.

## executable document — complete

- Classic inline scripts execute in document order in a persistent raw-V8 realm.
- The generated document realm provides Oryn-owned element/text nodes,
  `querySelector`/`querySelectorAll`, element creation and mutation, attributes,
  values and boolean form state, event listeners and bubbling, click/input/change
  dispatch, `DOMContentLoaded`, microtasks, and deterministic timers. Mutations
  occur directly in the Rust DOM and publish one causal revision checkpoint.
- The controlled Vue-style task and Svelte-style counter fixtures execute and
  mutate native DOM state in feature-gated tests. Forms, event ordering, and
  persistent-realm behavior also have executable tests.
- External classic scripts, V8 module graphs, pinned resources, promise-based
  fetch, redirects, request bodies/headers, compression, cookies, URL and text
  encoding primitives all execute through the context-owned rustls broker.
- Generation-stamped Rust wrappers preserve retained node identities, retire
  removed identities, and assign fresh identities to new nodes. Structural
  churn regressions prevent false preservation.
- The typed page actor owns V8 on a dedicated page thread. Existing async page
  handles and OIL sessions dispatch actions into registered JavaScript handlers
  and receive versioned semantic deltas without moving an isolate across
  threads or introducing a second runtime API.
- Native navigation/history/refresh, lifecycle checkpoints, form constraints,
  inline and linked stylesheet visibility, custom elements, open/closed shadow
  roots, same-origin frames, cross-origin diagnostics, and pinned real React 18
  plus Babel execution are covered by executable tests.
- The versioned executable document manifest passes all required advertised cases. Pinned WPT
  URL/UTF-8 vectors, pinned html5lib parser vectors, and a supported-scope
  native-versus-Chromium differential run are published in
  `benchmarks/evidence/executable-document-external-validation.json`. Rendered layout, geometry,
  paint, and Encoding Standard behavior beyond the advertised UTF-8 subset
  remain explicitly unavailable.

## semantic recovery — complete

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
  machine-readable result is `benchmarks/evidence/semantic-loop-controlled-native.json`.
- The 30-entry corpus has historical published results. A versioned,
  confirmed-only challenge detector now requires paired provider evidence and
  reports generic block responses as suspected rather than routing them as
  challenges. Controlled evidence passes six provider-positive and four
  negative/ambiguous cases. The historical Wikipedia and crates.io
  parse/empty-observation rows are corrected to `native_runtime` failures in
  `benchmarks/evidence/semantic-public-canaries-v2.json`; their historical
  challenge disposition is `not_assessed`, because response bodies were not
  retained for a retrospective detector run.
- The earlier 96-cell panel is invalid as native evidence: its production
  `oryn` executable was built without `native-v8`, so the native 0/24 result did
  not exercise the designed runtime. Its `$1.196980` Azure spend remains charged
  to the cumulative $100 ceiling, but its schema-v2 aggregate is being replaced
  rather than retained as a valid result. A first semantic recovery collection attempt then
  spent `$1.993578` before independent validation found that Chromium observation
  byte counts were zero; that attempt is also invalid and the cumulative prior
  spend charged to the replacement run is therefore `$3.190558`.
- semantic recovery originally enabled `native-v8` in the monolithic production binary.
  sandbox worker supersedes that packaging: the default `oryn` parent is now deliberately
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
  `benchmarks/evidence/semantic-reference-churn.json`.
- `benchmarks/schema/result-v3.schema.json` separates `panel_complete` from the
  gate outcome. The final fingerprinted panel completes 96/96 cells: Azure/native
  passes 22/24, Azure/Chromium passes 16/24, and pinned `qwen3:4b` passes 0/48.
  The local-model failures are retained as model-quality results rather than
  reported as runtime failures. Overall model success is 38/96.
- The historical validator recomputed its aggregate from fingerprint-matched
  cells, but `17/96` observation sufficiency and `99/99`/`99/198` causal
  recall/precision are non-conforming heuristics, not the proposal KPIs. The
  schema-v5 runner now compares compact versus full truth at failed-task
  checkpoints, makes failed tasks the denominator, treats projection omission
  as a lower-is-better rate, and separates browser-truth, model/action,
  capability, and runtime failures. Trace events carry explicit direct,
  derived, correlated, or unattributed causality, with controlled per-class
  precision and recall evidence. The validator recomputes these metrics from
  raw cells and rejects unmatched fingerprints or promoted unattributed work.
  The replacement schema-v5 result is now attested against the exact signed
  macOS release candidate. It reports zero compact-projection omissions across
  63 failed cells, separates 57 browser-truth absences from six model/action
  failures, and reports perfect controlled direct/derived/correlated causal
  precision and recall without promoting unattributed work.
- The final Azure run spent `$2.019074`. Including `$3.190558` from the two
  invalid attempts, cumulative hosted spend is `$5.209632`, within the `$100`
  ceiling. The committed schema-v3 summary is
  `benchmarks/evidence/semantic-model-panel.json`; raw cells and traces remain ignored.
- The release V8-enabled production binary is 46,556,752 bytes (17,837,843
  bytes gzip), starts and completes the mutation protocol at 10 ms P50/P95 over
  30 samples, and peaks at 23,805,952 bytes RSS. Its SHA-256 matches the model
  panel fingerprint; evidence is published in
  `benchmarks/evidence/semantic-production-macos-arm64.json`.
- Formatting, strict all-feature Clippy, all-feature Rust workspace tests,
  Python SDK tests, IntentGym tests, executable document/semantic loop/corpus validators, and schema-v3 raw
  evidence recomputation passed at commit a41a231. That result remains
  historical and is not reusable for the sandbox worker worker bundle.
- The replacement schema-v4 run is bound to clean source commit
  `cbfee34143af257bc43430d23fa842f4bad12448`, the signed protocol-v4 worker,
  the separate opt-in Chromium adapter, pinned MiniWoB, Chrome 152, Azure
  deployment `gpt-5.6-terra`, and the immutable `qwen3:4b` digest.
- Native and Chromium deterministic matrices both pass 24/24 with zero first
  divergences. The full model panel completes 96/96 cells: Azure/native passes
  21/24, Azure/Chromium passes 15/24, and pinned Qwen completes 48/48 cells with
  0 task successes. Qwen's measured model-quality failures do not masquerade as
  runtime failures and were not a semantic recovery success threshold.
- Schema-v4 historically reported 36/96 overall task success, `13/96`
  observation sufficiency, and `99/99`/`99/198` causal recall/precision. The
  latter three figures use the superseded heuristic definitions and do not
  close the current semantic evidence criteria. Recovery 2/2, reference
  survival 1/1, and false preservation 0/5 remain bounded historical results.
  The run's Azure spend was `$1.152654`; cumulative hosted spend was
  `$6.362286`, within the `$100` ceiling.
- Schema v5 replaces the semantic KPI definitions without rewriting schema-v4
  history. It expands exact-NodeId churn to eight survival and eight
  invalidation cases, including attribute/text/value changes, sibling and
  ancestor churn, same-document reparenting, shadow changes, DOM recreation,
  framework-style keyed replacement, frame/navigation changes, and worker
  replacement. The exact signed candidate passes 8/8 survival and 8/8
  invalidation cases with 0/8 false preservation and 2/2 recovery.
- `benchmarks/evidence/semantic-validation-v5/STATUS.json` records the current
  completed attestation: native and Chromium deterministic matrices each pass
  24/24; the model panel completes 96/96 cells; Azure/native passes 18/24; and
  false reference preservation is zero. New hosted spend is `$1.633968`;
  including `$6.362286` prior spend, cumulative spend is `$7.996254` against
  the `$100` ceiling.
- Current compact evidence is published under
  `benchmarks/evidence/semantic-validation-v5/`, with the exact release bill of
  materials at `benchmarks/evidence/release-attestation-v5/attestation.json`.
  Raw observations, turns, traces and deterministic cells remain ignored.
- Formatting, strict default/all-feature Clippy, Rust default/all-feature,
  V8-host and no-default tests, Python SDK, IntentGym, scanner, extension, and
  corpus/executable document/semantic loop/schema-v4 validators pass against the closure tree.

## Agent interface and semantic convergence — implemented in working tree; adapter secret provisioning pending

- `oryn-common` defines a versioned, serializable `Intent`/`Target` contract
  covering navigation, observation and deltas, semantic actions, extraction,
  and revision waits. `oryn` re-exports the contract as part of its stable Rust
  facade.
- `PageHandle::perform` executes the shared IR through the existing native OIL
  target resolver and typed action engine. It does not add a second selector,
  scoring, or mutation implementation.
- The canonical OIL grammar accepts `type <target> $secret:<label>`. The label
  resolves through the parent-owned origin-scoped secret store and the existing
  secret-fill policy path. Normalization and serialization retain only the
  label; compatibility backends reject the native-only form rather than typing
  it as plaintext.
- Focused parser, normalizer, native action, and secret-serialization tests
  cover these additions. They are working-tree software evidence and are not
  part of a signed release attestation yet.
- Python `perform()` sends the same versioned JSON envelope and returns typed
  results; legacy `execute()` remains an OIL compatibility path. Typed requests
  fail closed outside native mode instead of being reinterpreted by a different
  backend.
- `oryn-mcp` is a first-party MCP stdio server. Its sole browser tool,
  `oryn_perform`, publishes the complete shared Intent/Target JSON schema and
  delegates directly to the same context session. It exposes neither an OIL
  tunnel nor raw JavaScript. Secret intent schema contains a label only.
- Rust embedding can provision origin-scoped labels with
  `BrowserContext::import_secret`. Standalone CLI/Python and MCP sessions use a
  bounded inherited descriptor consumed by the parent before page work.
  Plaintext secret values remain forbidden in argv, environment variables, the
  typed envelope, OIL, JSON-RPC, traces, observations, and worker initialization.
- Additive contract v5 preserves frozen v2-v4 observations and publishes
  first-class forms, tables, lists, navigation, dialogs, menus, trees, tab sets,
  grids, landmarks, live regions and repeated collections. Accessibility nodes
  carry supported name/description evidence, role/state properties, IDREF
  relationships, conservative `aria-owns`, exclusion reasons, trust and
  provenance. Closed-shadow tree scopes remain isolated and missing/cyclic
  relationships produce diagnostics instead of fabricated links.
- The v5 surface includes a pinned agent-critical Accessible Name/ARIA corpus
  for referenced names, hidden descendants, host alternatives, role fallback
  and presentation conflicts. It does not claim complete long-tail Accessible
  Name, ARIA or changing-site conformance.
  Exact packaged-worker evidence now passes on macOS; Linux and broader
  changing-site evidence remain pending.
- Local verification before attestation passed 178/178 native V8
  library tests, all common/core tests, six MCP tests, all-feature binary
  checking, strict all-target Clippy, formatting, 26 Python tests (19 skipped
  for unavailable optional environments), Ruff, MCP stdio smoke testing, and
  `git diff --check`. These results do not replace signed-package, Linux, or
  corrected model-panel evidence. The exact candidate subsequently passed the
  signed package suites and corrected panel recorded above.

## application compatibility — complete

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
  and media remain explicit layout-or-later capability boundaries.
- All eight pinned applications—React, Next.js, Vue, Nuxt, Angular, Svelte,
  SvelteKit, and Preact—pass their native tasks and Chromium differential
  assertions. All five controlled OAuth/SSO cases pass in both domains.
- The exact clean-tree macOS ARM64 production bundle at source commit
  `8e460a81a6de8c1c2739b1fb5476d323a242c0aa` passed all 43 required manifest
  cases; all five layout deferrals emitted their declared diagnostics. The executable document/semantic loop
  regressions, controlled OIL 12/12, and native MiniWoB 24/24 also passed, with
  zero false semantic-reference preservations.
- The signed parent is 6,933,984 bytes and the worker is 44,792,096 bytes;
  combined installed size is 51,730,100 bytes and compressed size is
  20,452,801 bytes. Startup-to-handshake was 500 ms P50/P95. Eight concurrent
  pages completed brokered streaming and WebSocket work in 664.155 ms.
- The schema-valid compact result is
  `benchmarks/evidence/app-compatibility-v1/result-v1.json`; its status record is
  `benchmarks/evidence/app-compatibility-v1/STATUS.json`. The validator recomputed the result
  from 48 manifest-bound cases and content-addressed raw artifacts.

## layout — complete

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
- All 28 required layout manifest entries pass; all five declared limitations
  report the expected capability instead of fabricated geometry. executable document/semantic loop/application compatibility,
  controlled OIL 12/12, frameworks, OAuth, native MiniWoB 24/24, signed worker
  isolation, and semantic churn all pass with zero false preservation.
- The exact clean-tree macOS ARM64 bundle at source commit
  `68e5afe143490b810301e1ed5d60e6e232209793` passed the complete production
  gate. The parent is 7,033,872 bytes and the worker is 46,921,472 bytes;
  combined installed size is 53,959,743 bytes and compressed size is
  21,691,132 bytes. Startup-to-handshake was 570 ms P50 and 580 ms P95.
- The 400-node layout benchmark measured 947.661 ms cold page-to-geometry,
  328.617 ms reflow P50, and 330.869 ms P95 over 20 samples. The schema-valid
  compact result is `benchmarks/evidence/layout-v1/result-v1.json`, recomputed
  from 33 manifest cases and 114 content-addressed raw artifacts.

## sandbox worker — complete

- Public native callers now use a protocol-v4 page worker on macOS. V8, the
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
  signing and model workflow definitions remain available for a future private
  repository; no Apple key or model credential is stored in this public repo.
- Schema v4 binds the source-tree hash, parent and final signed-worker hashes,
  signature/certificate/entitlement/manifest fingerprints, policy, limits,
  fixtures, Chromium, Azure deployment, and Qwen digest. Compact 96-cell and
  24-cell causal ledgers are sufficient to recompute the aggregate; raw turns
  and traces remain uploaded, ignored artifacts.
- The exact clean-tree local macOS ARM64 release run verified the trusted Apple
  identity, App Sandbox plus JIT-only entitlements, runtime handshake, mutation,
  broker isolation, worker replacement, deterministic parity, and the complete
  96-cell panel. `benchmarks/evidence/semantic-sandbox-v4/STATUS.json` records
  semantic recovery and the sandbox worker as closed.
- Production measurements publish a 4,929,744-byte parent, 47,004,112-byte
  worker, 51,938,255-byte combined installation, and 20,641,316-byte compressed
  bundle. Handshake and mutation P50 are 560 ms and 580 ms; parent and worker
  RSS are 56,098,816 and 56,377,344 bytes. Crash replacement passes and restores
  a blank generation in 489.739 ms after termination.
- The security boundary and deferred items are documented in
  docs/oryn2/SANDBOX_WORKER_SECURITY.md.

## security — complete

- Every native context now owns an explicit policy engine for cross-origin
  navigation, form submission, popups, downloads, credential fills, protected
  requests, and user-browser delegation. Exact
  request confirmations are random and one-use; all decisions are recorded in
  a structured parent audit log. File-input upload is implemented through
  parent-owned opaque handles and explicit selection; the worker never receives
  host paths. Clipboard access is a text-only `BrowserContext` virtual buffer.
  Host system clipboard access remains explicitly unsupported.
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
- Cookies are now owned by an Oryn `ContextCookieStore`, not reqwest. It uses
  schemeful registrable sites from an embedded PSL, Strict/Lax/None and
  Lax-by-default selection, the bounded 120-second compatibility exception,
  redirect-chain taint, Secure/HttpOnly/prefix rules, parent-owned
  `document.cookie`, and a persistent-record migration that cannot invent a
  cookie creation time.
- The parent broker compiles response-header CSP, verifies the worker snapshot,
  validates ordered meta policies against the accepted response body, and
  fails closed on an omitted meta checkpoint before the next network effect.
  The `csp3-agent-browser-2` profile adds parent-delivered `report-uri`,
  `Reporting-Endpoints`, `report-to`, and legacy `Report-To` reports; generated
  `SecurityPolicyViolationEvent`; unique opaque sandbox origins across cookies,
  storage, CORS, frames, opener messaging, and navigation policy; and active
  `worker-src`, `media-src`, `object-src`, and `manifest-src` resource paths.
  `security.csp` is supported for Oryn's exposed native platform surface.
  Dedicated-worker script loading is policy-complete and each worker executes
  in an independent persistent V8 context with realm-owned message queues and
  no page-global or DOM access. Media decoding/playback, SharedWorker, ServiceWorker,
  WebRTC/media capture, and ReportingObserver remain separate explicit
  capabilities rather than CSP omissions.
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
  `6eb164df0e0bd50aa02aae969b6112ff117cdd9c` passed all 51 required security cases,
  all executable document-layout regressions, native MiniWoB 24/24, four fuzz targets with zero
  crashes, limit/crash recovery, and reference churn with zero false
  preservation.
- The parent is 7,449,040 bytes and the worker is 46,970,896 bytes; combined
  installed size is 54,424,335 bytes and compressed size is 21,890,870 bytes.
  Startup-to-handshake was 580 ms P50/P95, startup-to-mutation was 590 ms P50
  and 600 ms P95, parent RSS was 56,492,032 bytes, worker RSS was 56,410,112
  bytes, and crash restart-to-blank was 570.780 ms.
- The schema-valid compact result is
  `benchmarks/evidence/security-v1/result-v1.json`; its status record is
  `benchmarks/evidence/security-v1/STATUS.json`. The validator recomputed the result
  from 51 manifest cases and 147 content-addressed raw artifacts.

## execution domains — complete

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
- All 25 frozen cases and the complete security production regression passed on clean
  commit `d8930e3c87da0bdaee94699aaebf190b9b092c71`. The schema-valid result is
  `benchmarks/evidence/execution-domains-v1/result-v1.json`.

## replay — complete

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
  divergences, verified checkpoint/failure seals, and all executable document-execution domains regressions on
  clean commit `d8930e3c87da0bdaee94699aaebf190b9b092c71`. The schema-valid result is
  `benchmarks/evidence/replay-v1/result-v1.json`.

## rendering — complete

- The optional `oryn-render` package consumes a neutral contract-v4 paint
  snapshot and builds a bounded display list. Page state, JavaScript, storage,
  policy, and network access remain in the signed page worker and privileged
  parent; the default `oryn` dependency graph does not include the renderer.
- The deterministic CPU reference renderer paints supported backgrounds,
  borders, embedded-font text, and PNG/JPEG images. It emits PNG screenshots
  and single-page PDF output through both the Rust API and dedicated CLI.
- Rendering limits bound viewport dimensions, pixels, commands, image bytes,
  and output size. Unsupported SVG images, gradients, GPU acceleration, and
  unrestricted page fonts produce explicit diagnostics instead of fabricated
  output. Chromium pixel parity is not claimed.
- All 24 frozen rendering manifest cases passed. The signed worker produced a six-node,
  eight-command paint snapshot with text and one broker-fetched image; its PNG
  and PDF outputs were deterministic and valid. The visual differential matched
  Chromium's 320×200 viewport and 22,748 painted pixels, with mean absolute RGB
  delta 7.730 over the declared visible-content sanity scope.
- The exact clean-tree macOS ARM64 run at source commit
  `d14b73314f748e9b35c5020ef86c175aa1050563` passed the complete executable document-replay
  production regression, signed-worker boundary, formatting, strict Clippy,
  renderer tests, architecture boundary, and raw evidence recomputation.
- The parent is 4,929,744 bytes, the worker is 47,004,112 bytes, and the optional
  renderer is 4,443,584 bytes. Startup-to-screenshot was 622.285 ms P50 and
  623.220 ms P95 over ten samples. The schema-valid compact result is
  `benchmarks/evidence/rendering-v1/result-v1.json`; raw images, PDFs, logs, and
  benchmark samples remain ignored.

## Next tracks

- Close the remaining 1,659 cases in the exact 4,022-variant inventory,
  prioritizing shared harness/lifecycle causes over isolated assertions. The
  current deficits are editing 586, CSS 429, DOM 373, IndexedDB 119, referrer
  policy 9, HTML 106, generated-WebIDL 29, WAI-ARIA 4 and Accessible Name 4.
- Complete semantic-graph/oracle, ARIA/Accessible Name and Stylo/Chromium
  differential evidence, then build and validate Linux full/lite profiles in
  CayVM alongside the macOS full/lite profiles.
- Freeze one exact candidate, rerun deterministic product, platform-security
  and performance validation, and produce a recomputable
  `runtime-completion-v1` result. Historical schema-v4/v5 evidence does not
  close this result.
- Privately sign the exact macOS bundle and only then re-attest the 96-cell
  Azure/Qwen panel with `$7.996254` cumulative hosted spend carried forward
  under the `$100` ceiling.
- Rendering's optional CPU reference package is complete. Broader long-running fuzzing,
  V8 update automation, GPU acceleration, multi-page print layout, broader SVG
  and font support remain operational or product-driven tracks rather than
  unimplemented execution domains-rendering gates.
