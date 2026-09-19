# Historical generation checklist — superseded September 16 source audit

This is the preserved pre-audit checklist, including chronological notes. Its current-status and active-agent statements are historical, not current. Use [the current granular checklist](CORE-AND-OIL-GENERATION-CHECKLIST-2026-09-15.md).

# Full-plan generation checklist

Source: [CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md](CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md).
Updated September 16, 2026. This checklist tracks generation separately from
execution and acceptance. The source plan and its referenced gates remain binding;
this document does not replace or narrow them.

## Current assessment

September 16 source reconciliation: **all nine package generation gates remain
open**. No build, test, generator or conformance command has run in this phase.
The entries below track requirements, not a percentage of working implementation.

Current parallel ownership:

- Root: R/P/L integration and native engine hooks. Persistent dynamic module
  graphs, shared request ownership, typed imports, parser cancellation/source
  custody and bounded module response caches have generated implementation and
  regression sources. Parser/direct-import continuations, combined resource
  accounting and requirement-wide R/P/L reconciliation remain open. Native
  shared-memory factory, growth, Wasm backing/size and wait-admission engine
  patches remain unbuilt and unqualified.
- Family agent: S platform and native input/accessibility. Current work includes
  IME recovery/range geometry, live-region changes, table semantics and opaque
  accessibility text markers. Platform integration and broader S reconciliation
  remain open.
- Layout agent: V layout/render/capture integration, native list markers,
  international counter representations and flattened-tree counter scopes.
  Reversed counters, scoped/layered definitions and broader V reconciliation
  remain open.
- Worker/coverage agent: W/G worker security, native browsing-context groups,
  shared-memory custody and bounded wait transport. Atomic admission/cancellation
  protocol and route retirement/drain integration are being generated; protocol
  source is context40/agent13, with execution and qualification still deferred.

All three generation agents were confirmed running for the user's parallel-work
request. This is source-generation activity, not proof that any package passes.

Source generated since the older assessment includes typed action IPC/HTTP/native
host dispatch, cross-frame drag and form history, production/historical input
isolation with worker feature attestation, native touch/viewport source, worker
response streaming, and realistic family/qualification fixtures. None of these
facts proves package completion or current conformance. R/P/L and final G/F/Q
require further requirement-wide source reconciliation; explicit requirements and
acceptance counts below are unchanged.

## Historical assessment entries

The following entries record earlier generation states and are superseded by the
current assessment and the latest package source notes. Their open/unsupported
statements must not be read as a fresh audit of today's callers.

Typed native action generation now includes session-owned pressed keys/buttons,
pointer coordinates, repeat handling, reverse press-order release planning,
explicit release completion and lifecycle retirement. Source regressions cover
cross-request state, rejected transitions, repeat deduplication and pending
release ownership. Native dispatch/IPC and HTTP endpoint wiring remain open;
these types are not yet evidence of functional `/actions`. No builds/tests ran.

Authenticated-action generation now includes `oryn-native::webdriver_actions`:
typed key/pointer/wheel/pause envelopes, lossless key units, source/action/origin
validation, pointer property bounds, tick duration semantics and atomic persistent
source identity admission. Added negative and cross-request regression source.
The HTTP `/actions` endpoint is still unsupported pending native dispatch, input
state/cancellation ownership and host-scoped clipboard authorization integration.
The parser does not grant privileges or run author scripts. No builds/tests ran.

Contenteditable text-drop source now uses native shaped caret placement, trusted
beforeinput/input events with scoped read-only DataTransfer payloads, editing-host
revalidation and the existing native undo transaction. Added regression source for
payload lifetime, insertion and undo. Rich HTML sanitization/insertion, selected
source deletion, drag images and cross-frame drops remain open. No builds or
tests ran. Clipboard host authorization still requires a typed native input entry;
generic execute/evaluate and author-callable globals are not authorization.

Native text-control drop insertion now resolves the pointer position from shaped
caret geometry (including native transforms and UTF-16 boundaries) before
beforeinput dispatch. Empty controls resolve offset zero; missing geometry does
not manufacture a character offset. Added regression source for ignoring an old
selection and retained coverage for selection changes inside beforeinput.
Contenteditable/source moves, drag-image capture and cross-frame targeting remain
open. Clipboard generation continues in the parallel forms workstream. No builds
or tests ran.

Drag callback-mutation source now preserves source completion when an accepted
drop removes its target, and target removal during dragover clears acceptance
without silently abandoning the source session. Canceled drop completion uses
the final dropEffect constrained by source effectAllowed. Added regression source
for the final-effect matrix, accepted target removal, and Escape cancellation
without a drop or follow-up click. No builds or tests ran; remaining input and
full-plan integration requirements are unchanged.

Native drag defaults now accept text/plain into writable text controls and native
file bytes into file inputs. Text insertion dispatches cancelable trusted
beforeinput, revalidates lifetime/eligibility, edits UTF-16 selection boundaries,
then dispatches input; file assignment precedes trusted input/change events.
Pointer Escape cancels the session while retaining click suppression. Added
pointer regression source for canceled/accepted editing and file payload/event
ordering. Pointer-position caret placement, contenteditable/default source moves,
drag images, cross-frame targeting and platform clipboard remain open. No builds
or tests ran.

Native drag sessions are now connected to `native_actions.js` pointer movement,
release and cancellation through a realm-owned native callback. Source eligibility
and initial link/image URL payloads come from the DOM; session callbacks do not
hold registry/DOM borrows across author code. Drag consumption suppresses the
follow-up click, including canceled sessions. Added pointer-action regression
source for a trusted payload drop and completed operation. Native editable/file
defaults, selected-text source behavior, drag-image capture, cross-frame targeting
and platform clipboard remain open. No builds or tests ran.

History snapshot source now has checked control/payload reservations and native
restoration-boundary validation. Descriptor-only inventory avoids copying saved
files while revalidating callback targets, and indexed matching avoids repeated
pairwise descriptor scans. History entries share snapshot arrays; the per-page
32 MiB form-state cache evicts older payloads while retaining navigation/POST data
and live BFCache documents. Added reservation and payload-accounting regression
source. Producer preflight is being integrated by the forms agent. These changes
remain unbuilt and untested; no package completion is implied.

Root FACE history integration source now captures typed restoration state after
pagehide and retains it on the native history entry. Uncached traversal restores
only when the committed URL still matches the recorded resource, resolves unique
definition/name/form/occurrence descriptors, and rechecks ownership after each
callback. BFCache keeps its live state. Native host/executable APIs and matching
and saved-state-versus-submission regressions are generated. Child-frame history,
late asynchronous custom-element upgrades, snapshot memory bounds and broader
autofill remain open. No execution or completion claim is made.

FACE selector integration source now projects native eligible-control disabled
state into layout (`LayoutNode.disabled`) and Stylo element state. DOM selectors
include registered form-associated custom elements in enabled/disabled matching;
validity continues through native validation-selector eligibility. Added source
regressions for disabled fieldsets, first-legend exemption, non-FACE exclusion,
unregistration, and computed-style changes after validity/disabled mutations.
History restoration and default accessibility integration remain open. No builds
or tests have run; these edits do not establish S/V completion.

Native drag-session source now lives in `v8_bindings/native_drag.rs`: it uses
trusted DataTransfer dispatch, checks document generation/stream epoch after
callbacks, sequences target entry/leave/over/drop/end, rejects detached targets,
and detaches payload stores at completion. Regression source drives the native
session through an accepted move and checks event order and saved-wrapper data
isolation. Pointer-input ownership, default editing/source deletion, image capture
and clipboard integration remain unfinished; this host API is not yet called by
the pointer sequence. Page-close cleanup now clears parent frame-worker policy
tickets as well as retiring agents. No builds or tests ran.

September 16 drag integration source: `oryn-js::data_transfer` now contains
typed operation negotiation, the effectAllowed compatibility matrix, per-target
acceptance reset, native default acceptance, and single-shot completion. Added
regression source for target-generation replacement, rejected operations,
cancellation, missing targets, and completion idempotence. Host input sequencing
in `oryn-native/src/native_actions.js` still needs to call the native drag
session; trusted event delivery already exists, but this negotiation code is
not yet integrated. Drag image capture, default editing and platform clipboard
remain open. No execution evidence is claimed.

**Generation is incomplete. No package is certified generation-complete here.**
That does not mean no code exists: it means the complete package has not yet been
reconciled to current implementation, all production callers, regression sources
and removal of superseded paths. There is no defensible percentage or ETA yet.

The recent source review directly establishes partial work and remaining gaps in
S and V. R/P/L/W/G/F/Q require renewed package-wide source review; historical
completion headings and evidence from older candidates do not close this plan.

All current generation-phase changes remain unbuilt and untested. Build, tests,
repair and full conformance follow only after **all** package generation finishes.

## Parallel generation assignments

The user authorized parallel sub-agent generation on September 15, 2026.
Four concurrent workstreams are assigned:

- Primary agent: native runtime/JS/WebDriver integration, shared interfaces and
  reconciliation against all nine packages. Owns shared status/checklist edits.
- `family_generation`: F application fixtures, existing family orchestration,
  persisted-outcome oracles and regression/negative test sources. Findings go in
  `F-GENERATION-NOTES-2026-09-15.md`.
- `wpt_coverage_generation`: G manifest/evidence/qualification, upstream fixture
  configuration and runner plugin sources with negative controls. Findings go in
  `G-TOOLING-GENERATION-NOTES-2026-09-15.md`.
- `layout_generation`: V algorithms and regression sources within `oryn-layout`
  and `oryn-render`. Shared interface changes require primary-agent coordination.
  Findings go in `V-GENERATION-NOTES-2026-09-15.md`.

These are ownership assignments, not completion claims. Agents preserve existing
changes and do not commit, build, run tests, format, install or deploy. The primary
agent integrates source changes and reviews remaining requirements; validation
starts only after implementation and test generation for the entire plan finishes.

## How to close a generation item

For each item record implementation symbols/files, all production callers,
regression sources and negative cases, superseded-path disposition, remaining
subrequirements, and source-review findings. File existence or a test name alone
is insufficient. After generation, record actual build/test/qualification evidence
separately, bound to the final candidate. Do not check an item because a narrower
slice is implemented or a historical test passed.

Statuses used below: **review needed** means package-wide generation coverage is
unreconciled; **partial** means inspected code exists but known requirements remain.
Unchecked items are open; their count is not a measure of effort remaining.

## Inspected current source anchors

- S: `crates/oryn-js/src/v8_bindings.rs` — scroll policy, reconciliation, geometry
  accessors and scroll-into-view; `crates/oryn-native/src/runtime.rs`, `host.rs`
  and `rust_executable.rs` — native semantic and paint integration plus tests.
- V: `crates/oryn-layout/src/lib.rs` and `transforms.rs` — layout inputs, nested
  scroll projection, scroll CSS properties and regression sources;
  `crates/oryn-common/src/v4.rs` — separate text projection and paint visibility;
  `crates/oryn-render/src/lib.rs` — text clipping and renderer regression sources.
- Known open S/V work includes negative scroll origins, viewport reconciliation,
  smooth scrolling, cross-document scrolling, positioning/containing blocks,
  full vertical layout, rotated overflow precision, stacking, SVG and print.
  These anchors establish partial generation, not completion of S or V.

## Work-package requirements

### G — Valid execution and complete coverage

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **G-01** — Adapt existing evidence/qualification tools to the full manifest. Record each entry's owner, prerequisite, observed failure, confirmed cause when known, implementation change and acceptance evidence. Keep raw and qualified outcomes distinct. The old 4,022-test backlog is not the new denominator.

- [ ] **G-02** — Implement the general authenticated automation contract: window handles, navigation, synchronous/asynchronous script, input/actions, testdriver operations, screenshots and lifecycle/error observation. Use the upstream product/executor interface for the required Core executor operations. Full WebDriver/wdspec conformance is a separate scope; none of the current 629 unsupported wdspec entries is Core. Do not serialize browser fixes behind that unrelated expansion.

- [ ] **G-03** — Serve official wrappers and MIME types through upstream fixtures, including HTTPS, aliases, cross-origin hosts, redirects, websocket/HTTP variants, helpers and test CA trust through explicit test configuration.

- [ ] **G-04** — Provision a host/VM topology with genuinely distinct local/public IP spaces for all 17 environment-blocked Core entries. Hostname aliases alone cannot satisfy these local-network-access tests. Assign this environment work immediately.

- [ ] **G-05** — Implement testharness, reftest and crashtest execution; add genuine print and visual execution and a documented manual/assistive-technology assessment path.

- [ ] **G-06** — Reject missing helpers/subtests, unsupported automation, failed reference readiness and malformed results. Validate this with deliberately failing tests and fault injection. A reference browser helps diagnose fixture prerequisites; Oryn renders both sides of its own reftests.

- [ ] **G-07** — Close reference-browser focus and TLS-trust prerequisites. The current reference bypass does not establish normal certificate validation. Qualify reusable prerequisite rules against source hashes, expected subtests and intentional resource failures; do not mass-approve raw passes. The current Core ledger has no qualified results: 11,015 review candidates and 2,715 unresolved mapped results, alongside the unsupported and blocked entries.

- [ ] **G-08** — Keep the current runner for historical reproduction until new-runner parity is established. Preserve old evidence; create a separately attributed baseline when executor semantics change.

### R — Native browsing contexts, documents, navigation and task lifecycle

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **R-01** — Complete synchronous insertion-time initial about:blank realm, Document, inherited origin/base/policy and stable native WindowProxy creation for both parser and script insertion. Reuse the implemented scope-based helpers.

- [ ] **R-02** — Replace provisional/committed JavaScript proxy factories with native routing: self/frames/parent/top/opener, document/location, named/indexed access and cross-origin permitted operations. Internal operations must survive author replacement of public properties and prototypes.

- [ ] **R-03** — Implement initial-empty-document reuse and later navigation rebinding without reinstalling incompatible bindings or changing retained document identity.

- [ ] **R-04** — Unify navigation request ownership, target selection, supersession, redirects, commit, history, reload, srcdoc, fragment traversal and lifecycle events. Implement sandbox/noopener/COOP relationships at their specified boundaries.

- [ ] **R-05** — Complete close eligibility and beforeunload/pagehide/unload ordering. Preserve ordered script-close acknowledgement and explicit host cancellation.

- [ ] **R-06** — Finish per-agent task-source ordering, microtask/custom-element/observer checkpoints, parser pauses and resource/rendering opportunities. Correct work must advance without an unrelated later OIL command.

- [ ] **R-07** — Route callbacks by their relevant/incumbent/current realms and native receiver. Retire execution authority and tasks independently from GC reachability of old Documents. No DOM/broker registry borrow may span arbitrary author script.

### P — Process ownership, environment authority and connected placement

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **P-01** — Make WorkerProcessSession the sole process/client/IPC-dispatch owner. Routed page handles must not independently read the same stream.

- [ ] **P-02** — Complete parent-authoritative context/group/agent/cluster placement and bounded environment registration. Issue per-document/environment grants and rotate them at navigation boundaries; public IDs alone are never credentials.

- [ ] **P-03** — Target every command, completion, cancellation and privileged operation to its authorized environment/generation. Finish admission/revocation/drain for concurrent navigation, close, reset and in-flight operations.

- [ ] **P-04** — Implement connected-window hosting, synchronous same-agent access and remote permitted operations. Enforce origins even for co-located realms; independent root groups remain separated.

- [ ] **P-05** — Bound process/group/environment counts; close retires the correct environment, process failure reaches exactly its hosted domain, and reset/recycling requires complete destruction of all surviving authority, realms and clients.

- [ ] **P-06** — Version wire contracts, tracing, metadata, build validators and signing evidence together. Root-binding v10 is an existing foundation, not the completed route model. Use the next appropriate protocol version for incompatible changes.

### L — Loading, encoding, networking and web security policy

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **L-01** — Finish URL resolution/serialization, MIME sniffing/dispatch, HTML/XML encoding selection and incremental decoding, legacy encoders and TextDecoder/stream interfaces. Profile the encoding/RSS cluster and repair allocation or ownership problems under existing production limits.

- [ ] **L-02** — Complete classic/module script fetch, graph linking/evaluation, dynamic import, top-level-await dependencies, canonical module identity and error/load ordering. Finish dynamic import-map insertion/merging and positive SRI validation, retaining existing exact/prefix/scoped resolution and Nuxt fixes.

- [ ] **L-03** — Use a typed request contract for navigation, fetch, XHR, scripts, styles, fonts/images, frames and workers. Carry initiator, destination, credentials, partition, origin, referrer, body, redirect and cancellation state. Preserve distinct CORS and load-blocking rules for each consumer.

- [ ] **L-04** — Complete Fetch/XHR/WebSocket, request/response bodies, streams/backpressure, abort and error propagation. Share owned body/file mechanisms with submission.

- [ ] **L-05** — Close CSP, Trusted Types, referrer, mixed content, secure contexts, frame embedding, permissions policy and integrity obligations across all resource destinations and redirects. Web policy and parent privilege enforcement are separate controls and both must hold.

### W — Independent workers, messaging and persistent state

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **W-01** — Dedicated workers have independent execution/scheduling and busy-script termination; another context inside the blocked page isolate is insufficient.

- [ ] **W-02** — Complete shared-worker identity, connections/ports, partitioning and client lifetime; service-worker registration, install/activate/control/fetch/update, restart and browser-context-owned persistent authority.

- [ ] **W-03** — Finish structured clone/transfer, MessagePort delivery, BroadcastChannel, close/teardown ordering and shared-memory obligations where required. Existing MessageChannel work is retained. BroadcastChannel is a concrete prerequisite in the historical close-test failures.

- [ ] **W-04** — Close all Core cookies/storage obligations and their dependencies, plus IndexedDB/storage behaviors required by the family contract, with navigation/restart, credentials, origin/partition, transaction ordering and multi-client controls.

- [ ] **W-05** — Keep document-owned and browser-context-owned lifetimes distinct. Route every worker/storage operation through P's authority model.

### S — DOM, WebIDL, components, controls and OIL semantics

Generation: **partial; package review still needed**. Acceptance: **not established for the current candidate**.

- [ ] **S-01** — Close generated WebIDL conversion/exposure/brand/descriptor/realm rules and enforce regeneration consistency. Finish DOM namespaces/tree mutation, collections, traversal, XPath/parsing, Range/Selection, adoption and mutation lifetime obligations, including faithful reproduction of historical Range crash/resource failures.

- [ ] **S-02** — Complete custom-element definition/upgrade/reaction/adoption and shadow/slot ownership across parser, DOM, styles and accessibility.

- [ ] **S-03** — Finish form entry construction, custom-element participation, legacy encoding, image coordinates, target/rel policy, callback reentrancy and cancellation. Preserve native file selection, body fidelity and parent file grants.

- [ ] **S-04** — Finish input/editing, focus, disabled/inert/validation behavior, keyboard/pointer activation, drag/drop and DataTransfer, selection, event order and default actions. Include clipboard integration where these contracts require it. HTML drag/drop alone has 180 current failures and needs an explicit algorithm owner. The passing constraint and numeric slices remain regression controls, not work to redo.

- [ ] **S-05** — Complete OIL target identity, stale-reference recovery, action errors, extraction scopes and observation freshness through dynamic mutation, navigation and frame replacement. Semantic actions call native primitives; application state is never mutated through a test-only JavaScript shortcut.

### V — Style, layout, paint, accessibility and print

Generation: **partial; package review still needed**. Acceptance: **not established for the current candidate**.

- [ ] **V-01** — Retain Stylo; finish namespace/document/state selectors, cascade/CSSOM, stylesheet ownership, shadow scope and mutation invalidation.

- [ ] **V-02** — Map actual Core visual tests to required algorithms immediately: inline/block layout, tables/floats, intrinsic sizing, flex/grid dependencies, writing modes, positioning, overflow/scrolling, transforms and fragmentation. Assess existing Taffy/Parley/Blitz integration against these requirements before choosing targeted adapter work or a bounded subsystem replacement.

- [ ] **V-03** — Complete required text shaping, fonts, replaced elements, SVG/paint, clipping/stacking and print pagination. Do not infer these from a screenshot API.

- [ ] **V-04** — Make CSSOM geometry, hit testing, focus visibility and OIL input use the same layout revision. Extend cache invalidation to every relevant input; retain the existing layout-input reuse improvement.

- [ ] **V-05** — Complete accessible-name/ARIA/tree projection and incremental updates, checked against independent full recomputation. Support required OS/assistive-technology procedures for manual entries.

### F — Realistic five-family applications and integrated outcome evidence

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **F-01** — Reuse the six existing family/auth runners, SQLite backends, sealed/reopened databases, artifact locks and validator negative controls. Add one orchestration and reporting layer rather than six new harnesses.

- [ ] **F-02** — Add versioned realistic frontends over those backends. Use a covering matrix for client rendering, SSR/hydration, routing/chunks, portals/custom controls, cross-origin frames, async persistence and client timezones. Keep all eight existing framework gates; avoid unnecessary duplication of five large apps for every framework.

- [ ] **F-03** — Verify whether the frozen September 15 candidate matches the intended runtime source and reuse it when it does; rebuild/sign only when the source differs. Run all existing cases together early, resolving the earlier persisted-SaaS source mismatch through a matching run. Do not wait for all Core to pass before exposing realistic application failures.

- [ ] **F-04** — Capture expected final facts, records, receipts, orders and reservations as oracle contracts; include exact action/authority history where required. Retain every planned failure, retry and cleanup result.

### Q — Reliability, performance, security and final qualification

Generation: **review needed**. Acceptance: **not established for the current candidate**.

- [ ] **Q-01** — Reuse deadline diagnostics to reproduce the booking failure under bounded mixed workload. Add deterministic lifecycle/IPC/I/O fault controls where the existing reproduction lacks precision. Retain the original failure; a green retry is not a fix, and a hypothesized shared WPT cause is not evidence.

- [ ] **Q-02** — Verify delayed/stale responses, worker failure, cancellation, restart and retry cannot duplicate external writes or leak state. Failed workers are killed and reaped; cleanup is part of the result.

- [ ] **Q-03** — Profile end-to-end actions and readiness on integrated candidates. Requalify existing targets: cold P50 <=250 ms, warm >=300 actions/s, binding P50 <=50 ms, worker RSS P50 <=64 MiB/P95 <=160 MiB, and the required 1/5/20-context and churn/no-failure samples. New placement also needs group/aggregate measurements. The historical 124–126 actions/s diagnostic is below the throughput target; current performance needs fresh measurement.

- [ ] **Q-04** — Recommended workflow acceptance sampling: each deterministic case runs three times in serial and three times in mixed-family mode; recovery-sensitive cases additionally get 30 repeated attempts and fault-injection controls. Run the existing 1/5/20-context performance/isolation suite. Freeze these counts before collecting acceptance evidence; they are engineering samples, not a universal statistical reliability guarantee.

- [ ] **Q-05** — Freeze a reproducible source snapshot without discarding current work; bind parent/worker/V8/bindings/ICU/snapshot/signature, adapter, fixtures and OIL tasks to exact identities. Run full Core and all five families on that artifact.

- [ ] **Q-06** — Audit all expected IDs and assertions, manual assessments, source-bound dispositions, security controls, resource limits, results and cleanup. Any implementation change after freezing produces a new candidate and requires the affected checks plus final integrated acceptance again.

## Five-family behavior contracts

Each row below requires both complete behavior and negative/recovery/oracle
coverage. Source generation is unreconciled; no current-candidate acceptance is
claimed. The source plan's explicit PKCE, MFA, real-file and persisted-outcome
constraints apply in full.

- [ ] **Research/extraction** — Search/follow/compare multiple pages; exact text/table/link facts with canonical URLs; pagination and async refresh; dynamic frontend; same/cross-origin and nested frame behavior. Recovery and oracle: Empty/missing sources, redirects, stale references and navigation; exact extracted facts and server request sequence; no detached/foreign-document leakage.

- [ ] **Forms/document submission** — Multi-step native/custom controls; text/options/date/numeric inputs; async validation; submit/cancel; real authorized file upload and GET/POST/multipart delivery. Recovery and oracle: Invalid/required/disabled controls, submitter overrides, cancelled/reentrant submission, navigation and retry; exact receipt fields/order/Unicode/body and uploaded byte hashes; no receipt on rejection; file/secret confinement.

- [ ] **Shopping** — Hydrated/CSR search/filter/compare/variants; persistent cart/quantities; route/reload; quote/review and controlled checkout; declared framed payment-style flow. Recovery and oracle: Async stock/price changes, stale quote/recovery, duplicate/retry; exact persisted cart/order/total/inventory and one accepted mutation; controlled test orders.

- [ ] **Authenticated SaaS** — Authenticated SPA login/SSO; real authorization-code/PKCE exchange with a controlled cross-origin identity provider; MFA handoff/resume; create/read/update/reload/logout. Recovery and oracle: State/redirect/verifier/replay/expiry checks, session expiry/relogin, stale/duplicate saves, denied unauthorized writes, context isolation and redaction; exact persisted record/version/authorization history and session state.

- [ ] **Booking/scheduling** — Client datepicker and timezone conversion; DST fold and gap; availability/options/review; controlled reservation and confirmation reload. Recovery and oracle: Stale/unavailable slot, validation, cancellation/retry/duplicates; exact UTC instant/zone/options, slot ownership/request history and one durable reservation.

## Final acceptance — separate from generation

All seven gates remain unproven for the current candidate.

- [ ] **Coverage:** 16,737/16,737 Core entries reconciled; no missing/duplicate IDs, unimplemented executor obligations or unresolved environment prerequisites.

- [ ] **Conformance:** all automatable Core requirements pass valid execution; applicable manual/visual/platform assessments and any independently reviewed upstream dispositions are complete and preserve raw outcomes.

- [ ] **Architecture:** native initial realms/WindowProxy/navigation, environment grants, connected windows, independent agents and retirement are complete; duplicate/probe-dependent production paths are removed.

- [ ] **Product:** every section 6 behavior and all eight framework gates pass OIL outcomes and independent oracles, with failures/retries retained.

- [ ] **Reliability:** required repetitions, mixed concurrency and fault controls pass; the booking timeout and other unexplained crashes/timeouts are resolved or remain explicit completion blockers. Non-recurrence alone is insufficient.

- [ ] **Security and performance:** applicable signed denial/authority/teardown, resource-limit, isolation and performance gates pass for the claimed policy and platform; no unmeasured profile substitution.

- [ ] **Provenance:** all evidence belongs to the exact frozen candidate and locked inputs; clean reproducible source, candidate integrity and fixture cleanup are verified. Historical mixed-candidate successes are not combined.

## Package G source review — initial findings

- Inspected `scripts/run-upstream-wpt.py`: its module declares a native
  testharness runner, and its path discovery excludes `crashtests`. This is
  historical runner infrastructure, not proof of the full G-05 executor set.
- Inspected `scripts/wpt_session.py`: it implements bounded native JSONL session
  transport and validates result/error/cleanup fields. Preserve this foundation.
- Inspected `scripts/tests/test_wpt_execution.py`: generated regression sources
  cover process/teardown/protocol failures overriding a nominal test pass. This
  supports part of G-06's source inventory, not executed current-candidate proof.
- Searches of repository-visible script/native sources did not locate an
  upstream `__wptrunner__` product registration or executor integration. G-02 and
  the official integration deliverable remain open pending broader source review
  and implementation; naming the legacy runner upstream does not close them.
- Reftest, crashtest, print, visual/manual routes, topology and TLS prerequisite
  coverage still require direct source review. No absence or completion claim is
  inferred merely from these bounded searches.

## Package G generated integration sources

`tools/wptrunner-oryn` now defines an external Product entry point using the pinned
upstream API and its actual testharness, reftest, crashtest and print executors.
Registration and configuration regression sources are included. This supersedes
the initial search finding that no product registration had been located.

**G-02/G-05 remain open:** the native command server does not exist in this
integration yet. Its required launch/protocol contract is documented in the
package README. Existing JSONL session source only accepts run/shutdown and cannot
fulfill that role. Host/CA configuration, authority, lifecycle and native executor
capabilities remain required. Nothing was installed or run, and registration is
not execution or conformance evidence.

### Native command-server generation started

`crates/oryn/src/webdriver/{mod.rs,capabilities.rs}` now contains WebDriver error
response envelopes and capability processing with regression sources. It validates
all firstMatch entries before matching, rejects merge conflicts, preserves native
identity and declines unsupported certificate-bypass/BiDi/extension/proxy requests.
This is source generation only. Complete proxy/prompt capability validation,
HTTP transport, session ownership/authentication, window/frame operations, script
serialization, input and rendering routes are still open. No listening server or
successful executor path is claimed.

### Shared timeout configuration source

Added `crates/oryn/src/webdriver/timeouts.rs` and reused it in capability
negotiation. Source inspection of the pinned upstream `set_timeouts/set.py` and
`new_session/timeouts.py` established that all timeout fields accept null,
integral JSON floats are valid, unknown keys are ignored and defaults must be
merged. Generated atomic-update, type/boundary and session-default regressions.
No upstream or local tests were executed. Command dispatch and actual deadline
integration remain open; this does not establish a working server.

### Native session/window owner source

Added `crates/oryn/src/webdriver/session.rs`: one native context per session,
window-handle enumeration from that context's registry, explicit window selection,
new-window creation without implicit selection, closed-handle rejection,
last-window deletion, timeout updates and revocation before teardown. Added native
regression sources for lifecycle and cross-session isolation. Initial-page failure
also retains cleanup errors. All sources remain unbuilt and untested.

HTTP transport/authentication, per-session command serialization, frame selection,
script value handling, input/rendering dispatch, cancellation/deadline ownership
and full teardown/race qualification are still open. This owner alone is not an
operational or qualified WPT command server.

### Session admission and revocation registry source

Added `crates/oryn/src/webdriver/registry.rs` with bounded creation permits,
OS-random opaque session IDs, per-session command leases, revocation before
teardown and rejection of requests queued before deletion. Shutdown drains
in-progress creation permits after revoking published sessions. Added native
regression sources for capacity, closed IDs, cross-session progress and queued
command revocation. The getrandom dependency reuses the workspace's existing
version; lockfile/build validation remains deferred.

Transport authentication, HTTP routing, cancellation-safe disconnect/shutdown,
protocol command coverage and qualification remain open. No server has been
started and no tests have run.

### Authenticated upstream transport source

Generated bearer-header transport and connection-only executor subclasses in
`tools/wptrunner-oryn/oryn_wptrunner/protocol.py`. The browser supervisor generates
a private token file; credentials are not put in the connection URL or argv.
Generated native `webdriver/auth.rs` checks exact loopback authority, absent
browser Origin and credential equality. Added transport/admission regression
sources. HTTP parsing, duplicate-header rejection, token-file consumption and
routing remain open. No listening service or authentication qualification is
claimed; no installation/build/test has run.

### HTTP admission and private token loading source

Generated optional Axum HTTP middleware in `webdriver/http.rs`: exact authority
and bearer checks, duplicate security-header and ambiguous framing rejection,
bounded body collection with a receive deadline, and no-store command responses.
Private token loading checks the opened descriptor, ownership, permission bits,
regular-file type and link count, using no-follow opening. Added header, token
file and in-memory router regression sources. Axum/Tower dependency resolution
and all builds/tests remain deferred.

Endpoint dispatch, command coverage, driver executable startup, connection quotas,
disconnect/cancellation cleanup and end-to-end executor qualification remain open.
The middleware does not start a listener by itself.

### Native HTTP command dispatch source

Added `webdriver/server.rs` connecting authenticated router dispatch to session
creation/deletion, native window operations, timeouts and native URL/title
observations. Method matching precedes session lookup. Missing registered native
routes return unsupported-operation errors, never successful placeholders. Added
native dispatch and malformed-request regression sources. Last-window closure
releases its command lease before registry deletion.

The executable/listener, navigation strategies/deadlines, sync/async script and
value conversion, frame selection, input, screenshot/print commands and remaining
executor routes are still open. This partial dispatch is not a working conformance
runner. No build, test, installation or server run has occurred.

### Native teardown drain correction

Source inspection of `PageHandle::worker_request` found blocking native requests
continue independently of a dropped async future. Session teardown now uses the
existing `BrowserContext::retire_workers` operation after bounded graceful close,
including initial-session failure. It records graceful/retirement errors instead
of treating logical page closure as process cleanup. Added a retained-page-handle
rejection assertion to the session isolation regression source.

This finding prevents implementing navigation timeout by merely dropping `goto`.
WebDriver navigation strategies and cancellation need a native owned-operation
path; they remain open. All changes are unbuilt/untested.

### Driver-owned command tasks

HTTP dispatch now admits commands to a bounded driver-owned task set and waits on
a response channel. Dropping the HTTP waiter does not drop the native command
future. A new session whose response channel is already abandoned is revoked and
drained. Shutdown stops admission, retires sessions and joins tracked commands;
background failures are retained. Added abandoned-creation and admitted-command
disconnect regression sources. Network delivery ambiguity, idle-session cleanup,
aggregate byte quotas and full shutdown/cancellation qualification remain open.

Native navigation still exposes completion rather than a separate interactive
milestone to the driver. Eager/normal/none navigation waiting and cancellation
require additional native lifecycle integration; no timeout-by-future-drop
shortcut was introduced. All source remains unbuilt/untested.

## Next source-review work

1. Reconcile G's official executor integration and every execution-type route to
   current source and canary tests; do not infer upstream integration from names.
2. Reconcile R/P/W together for native lifecycle, actual independent agent
   execution, authority routing and production-path removal.
3. Reconcile L/S/V algorithms and generated regression coverage against mapped
   Core obligations, keeping known gaps explicit.
4. Reconcile F's five complete behavior contracts and eight framework gates,
   then Q's fault, repetition, performance and provenance tooling.
5. Only after every generation item is supported by source evidence, enter the
   build/test/fix phase and then the full qualification phase.

## Native listener lifecycle generation

`crates/oryn/src/webdriver/service.rs` now owns loopback binding, credential
loading against the actual bound address (including ephemeral ports), authenticated
HTTP serving, graceful admission stop and native session/command drain. Combined
transport and cleanup errors are preserved. Regression sources cover public-bind
rejection, actual authenticated/unauthenticated HTTP status requests, listener
release after shutdown and error retention. These sources have not been built or
executed. The separate launcher executable/browser selection, full native command
routes and cancellation-safe lifecycle qualification remain open; this is not a
runnable upstream product claim.

## Shared stacking and hit-test generation integration

The renderer stacking planner is now shared in `oryn-common/src/paint_order.rs`
using borrowed flat-tree style records. `oryn-render/src/stacking.rs` adapts paint
snapshots to that plan; native JS document hit testing uses the same planner,
retains projection/visibility/pointer-event filtering and shadow retargeting, and
refreshes through the scroll-aware layout reconciliation path. Added regression
sources cover descendant z-index promotion and atomic opacity/transform/isolation
contexts. Existing renderer ordering/pixel tests remain source-only. Full CSS
background/content paint phases, inline/float pseudo-contexts and all remaining V
requirements are still open; no conformance or generation completion is claimed.

## Native driver rollback and transport bounds

New-window initialization/publication failure paths now share explicit rollback:
close the unreported page under a deadline, restore the prior current page, retain
cleanup/restore errors, and revoke/drain the session if page rollback fails. Added
native regression sources for successful rollback and a previous window closed
during initialization. HTTP middleware now bounds concurrent body collection and
dispatch to eight admitted requests, reserving capacity before buffering each
16 MiB maximum body; a negative/recovery regression source verifies admission
exhaustion does not dispatch. This is a transport bound, not a browser-process RSS
qualification. All changes remain unbuilt/untested; full G/R/P/Q completion is open.

Driver-owned tasks additionally reserve from a 64 MiB command-body byte budget
before spawning and retain that reservation until completion, including after
HTTP disconnect. The transport admission permit alone cannot bound detached
command memory, so these are distinct, complementary bounds.

## Explicit fixture trust generation

`oryn-net::RustlsTransport::with_trusted_roots` accepts bounded, validated CA PEM
bytes as immutable transport configuration, discards cached clients from earlier
trust, preserves hostname/chain validation and ignores legacy WPT environment
flags (including certificate bypass and HTTP/1 forcing). The native parent
transport exposes the same configuration method and rejects reconfiguration
after its inner transport is shared. Regression sources cover invalid/oversized
CA inputs and client-cache separation. No source was executed. Driver fixture
file/hash loading, context propagation, real host+port destination routing and
TLS/network integration qualification are still open. Legacy reproduction remains
a separate path; no new official execution may rely on its bypass flags.

## Parent endpoint routing and OIL frame parsing

Generated exact DNS-host/scheme/port endpoint routing in native PolicyBroker.
Routes preserve URL identity, reject ambiguous configuration, validate both URL
and actual destination IP policy, and refuse unconfigured ports for routed hosts.
Added regression sources for port-specific IP destinations, unchanged origins,
policy denial and duplicate/invalid configuration. Context/fixture propagation and
WebSocket routing still require integration. Fixed the core OIL frame parser's
unconditional `Main` result to preserve main, parent and parsed target selectors;
added parser regression sources. Native frame dispatch remains separately subject
to source review. All changes remain unbuilt/untested.

## Fixture context propagation and frame-dispatch review

Browser now accepts validated CA bytes and exact endpoint routes as parent-only
immutable fixture configuration, separate from serialized worker BrowserConfig.
Subsequent context creation configures the native transport trust and PolicyBroker
routes; existing contexts retain their transport. Official driver fixture loading
and WebSocket routing remain open. Native OIL review confirms Frame commands have
no dispatch branch and cross-origin semantic observation is opaque; parser support
alone is insufficient for framed shopping and remains an explicit R/S/F dependency.

Shared PaintGlyphRun metadata now declares serde-defaulted per-glyph upright,
clockwise or counterclockwise orientation for the layout agent's vertical shaping
and renderer integration. No vertical conformance claim follows from metadata.

## Native upstream launcher integration

Added feature-gated `oryn-webdriver` binary, shared ServeArgs and runtime
`oryn webdriver` subcommand. The launcher requires an absolute selected runtime,
rejects launching itself and replaces its Unix process with that runtime. Paths
are forwarded as individual argv values; credentials remain file references. The
selected runtime loads/configures verified fixture inputs, binds authenticated
loopback service, prepares workers and drains on SIGINT/SIGTERM. Added argument
forwarding regression sources. The WPT agent is reviewing the fixture loader's
file access and topology validation and generating its tests. No binary was built
or launched. Full native WebDriver routes, frame authority integration, platform
qualification and all other plan gates remain open.

## Closed fixture routing and WebSocket integration

Nonempty EndpointRoutes now reject every unmapped host/scheme/port before DNS,
including caller-supplied socket addresses and subdomains accepted by the general
suffix host policy. Parent contexts retain immutable fixture network configuration
and WebSocket creation resolves through it and constructs TLS trust from explicit
CA bytes, bypassing legacy environment selection. Added sources for unknown-host
rejection, context policy denial, distinct WSS routing and malformed CA rejection.
Original URLs remain unchanged. No tests/builds executed; real TLS handshake,
negative hostname/CA controls and actual endpoint topology still need qualification.

## Native screenshot command generation

GET session screenshot now captures the current native paint snapshot and renders
PNG on a blocking worker task, returning WebDriver base64 data. Empty capture area,
renderer errors/incomplete-render diagnostics and oversized encoded output return
explicit command errors; builds without render report unsupported operation. Added
native regression source for PNG signature/viewport dimensions and retired-page
rejection. Renderer algorithms and screenshot readiness still need complete-plan
review and later qualification; no screenshot test or renderer was executed.

## Headless window sizing generation

Window rectangle GET/POST now uses native viewport state. The complete request is
validated before mutation, omitted/null fields are preserved, paired dimensions
are applied through native viewport updates, and device scale is retained. The
headless window origin stays (0,0), since there is no desktop window manager.
Screenshot rendering now uses native device pixel ratio, with 1x/2x PNG dimension
regression sources. Added rectangle validation, partial-size and invalid-request
atomicity regression sources. No tests/builds run; desktop window placement is not
claimed and native configured size limits still apply.

## Explicit native readiness deadlines

Added PageHandle.wait_for_timeout and a distinct WaitForUntil worker command so
older workers reject an unsupported operation rather than silently ignoring a
timeout field. The native event-loop-driving wait accepts finite or unlimited
predicate deadlines while production worker resource limits remain in force;
legacy waits retain their command-timeout default. Network-idle quiet-period waits
respect finite deadline budgets. Generated sources check ready-at-zero, timeout
without losing page ownership and wire round-trip identity. No tests/builds run.
This supplies a readiness primitive; initial navigation interruption, full
page-load-strategy semantics and final protocol/build identity qualification remain
open before complete WebDriver navigation can be claimed.

## Navigation deadline ownership

Source review found that redirect header waits and final parser body streaming
each renewed navigation_timeout. They now share one checked deadline captured at
navigation entry. Expiry is checked before starting another hop, before accepting
already-queued headers and before requesting/accepting body chunks; active network
operations are cancelled on expiry. Added redirect-chain regression source with
queued headers proving the total budget cannot renew per hop. No execution occurred.
This repairs native network budget ownership; WebDriver per-command navigation
configuration, eager/none readiness and prompt/cancellation semantics remain open.

## Native paginated print capture generation

Generated PrintSnapshot worker request/response and PageHandle capture integration
against the layout agent's agreed print::compute interface. Native capture uses
fresh current layout input, print-specific layout output and page fragment maps;
fonts/images are shared once and image collection includes offscreen content.
Screen layout/viewport is not replaced by print geometry. Added source regression
for print media, pagination, document identity and unchanged screen viewport. The
layout/PDF implementation is still being generated in parallel; no build/test has
run. Native beforeprint/afterprint lifecycle, full iframe print composition and
remaining fragmentation/qualification requirements remain open.

## WebDriver print request integration

Generated POST print route with WebDriver option conversion matching the pinned
executor's centimeter page/margin fields, background and shrinkToFit flags.
Scale/orientation/type/range validation occurs before native capture; page ranges
retain open bounds and overlaps for print-layout selection. Native paginated PDF
rendering runs off the async executor and rejects incomplete-render diagnostics or
oversized output. Sources include pinned invalid-argument examples and upstream
reftest option regression controls. No builds/tests run. Print lifecycle events,
iframe composition and full fragmentation remain unfinished; the presence of the
route does not establish qualified print execution.

## Print assembly identity/resource checks

Native print assembly now checks aggregate page-fragment counts against the
existing configured layout-node budget before cloning node/style/text templates,
and rejects fragments lacking a captured document node rather than omitting them.
The layout agent confirmed page geometry excludes anonymous text/cluster IDs.
Added regression sources for both rejection paths. Source review also corrected
the WSS fixture test to explicitly allow private networks: loopback_test alone
does not grant that permission. No test/build was run.

## Native script routes and trusted print lifecycle

Generated typed WebdriverScript worker/actor integration and synchronous/asynchronous
HTTP routes using native script primitives. The actor owns task draining, checks
document generation during pending execution and processes native task checkpoints
before returning complete results. Script errors retain WebDriver codes. Added
request conversion and end-to-end structured value/callback/exception regression
sources. Native print now brackets capture with trusted beforeprint/afterprint
dispatch, validates options before events and retains both capture and cleanup
failures. Regression source replaces public dispatchEvent to verify native dispatch
and tests before/after mutations around immutable capture.

Agreed generated print-margin node support counts against fragment budgets and
rejects DOM ID collisions/missing resources. Layout agent owns generation of those
nodes. No builds/tests run. Window-reference mapping, lone UTF-16 surrogate fidelity,
full frame selection/authority, print media-event semantics and other plan gaps
remain open.

## Parallel generation ownership

Three subagents are assigned concurrent source and regression-test generation:

- Native WebDriver/WPT: lossless UTF-16 script values and bodies, actual window
  handle bindings, and migration through native actor, worker and HTTP boundaries.
- Selection/use-case support: cross-node segmentation and text-control selection
  geometry, coordinated with the native host owner for live control-value layout.
- Layout/printing: page rules, margin content, fragmentation and PDF fidelity.

The primary agent coordinates shared contracts and integrates remaining plan work.
Native runtime/worker and script HTTP files belong to the WebDriver agent during
the lossless wire migration to avoid concurrent edits. Generated print-node
resource checks require source review against the final layout contract. These
assignments do not mark any work package complete. All code and all regression
tests across the full plan must be generated before any build or test execution.

## User-prompt policy generation

Pinned upstream new-session sources require object-valued unhandledPromptBehavior
as well as legacy strings. Generated typed prompt-kind/action/notification policy,
strict object validation, per-type/default resolution and Classic beforeunload
automatic acceptance. Negotiation preserves the requested representation; session
creation validates/stores policy before native context allocation. Regression
sources cover all legacy handlers, overrides, empty objects and invalid later
firstMatch entries. Native pending-prompt state and command integration remain
under generation with the JS agent. No build or test execution occurred.

Native generated print-node font checks now cover projection text as well as the
node text list, since rendering can consume either representation.

## Parent-owned prompt transport generation

Added per-page parent prompt controllers, actor-independent PageHandle access,
multiplexed context-service request/cancel operations and worker backend forwarding.
Requests validate the source page; cancellation compares the exact prompt token.
Page closure/crash and route shutdown retire controllers to wake suspended calls.
Shared controller types must remain V8-free for the privileged parent; the JS agent
is moving them into its always-built prompts module. Generated parent-dispatch
regressions cover wrong-page rejection, stale cancellation, independent resolution
and retirement. WebDriver prompt text/accept/dismiss/input and policy application
helpers now use this parent path. HTTP ownership of scripts suspended by prompts,
watchdog accounting, complete alert response data and route integration remain open.

Pinned new-session invalid-capability sources also show that timeout capabilities
must reject unknown fields, unlike the Set Timeouts command. Capability validation
and regression sources now enforce that distinction. No builds/tests executed.

## Suspended WebDriver script ownership generation (2026-09-16)

Generated a driver-owned script task set with bounded task and retained-body byte
admission. When native execution publishes a modal prompt, the HTTP command returns
null and releases its session lease while the native future and admission permits
remain owned. Alert text/input/accept/dismiss routes operate through parent prompt
controllers. Commands that handle pending prompts apply negotiated policy before
entering the actor; unexpected-alert responses include the prompt text data.

Session revocation now signals independently of the command mutex. Script tasks
release waiting responses, close the native page and drain execution on revocation.
Driver shutdown stops script admission, revokes/cleans sessions, then drains script
tasks. Regression sources cover retained modal ownership, resolution into the same
script, default dismiss-and-notify, admission closure, and deletion during a null-
timeout asynchronous script. Host owner generated pending-loop retirement checks.

These are source changes only. Full prompt lifecycle, watchdog/worker time budget
alignment, prompts arising during other commands, sandbox policy, UTF-16 prompt
text fidelity, and all required qualification remain unverified or unfinished.
No build, test, WPT, or use-case execution has occurred in this generation phase.

## Unicode prompt wire and native session response generation

Prompt text/default/result storage already uses the native lossless UTF-16 adapter.
Alert HTTP text input/output now converts through ScriptValue directly, preserving
unpaired surrogates and escaping author strings resembling the internal storage
prefix. Error wire serialization also preserves exact unexpected-alert data. Added
Unicode prompt roundtrip and error-envelope regression sources.

Pinned new-session sources require UUID session IDs, default proxy/setWindowRect
fields and userAgent, and acceptance of unknown namespaced capabilities. Generated
UUIDv4 identifiers from system entropy, completed those response defaults using
the current native navigator identity, and ignore unrecognized namespaced values
after validating all merge conflicts. Added validation/identity regression sources.

## Scoped native document snapshots

Generated DomArena::snapshot_document_scope for the OIL frame action integration.
It retains selected-document native node identities and inert template ownership,
while excluding embedding/nested documents and pruning cross-document indexes.
Snapshot arena authority is fresh; removed slots cannot revive omitted NodeIds on
reuse. Regression sources cover shared-arena frames, attributes, shadow/template
metadata, stylesheets and stale identity behavior. Native scoped action integration
remains owned by the frame agent and is not yet complete. No builds/tests ran.

## Native navigation lifecycle control generation

Generated V8-free per-page navigation controls with monotonic operation IDs,
explicit queued/started/committed/interactive/complete milestones, exact document
generation checks, independent readiness waiting and cooperative stop reasons.
One page-load deadline is shared by parent waiters and native checkpoints; eager
readiness releases that deadline without aborting remaining resources, and none
does not apply a page-load timer. Supersession and page retirement wake waiters
without taking the actor lock. Added per-page parent storage and close/crash hooks.

Generated WebDriver navigation URL/strategy/timeout conversion and regression
sources for milestones, generation mismatch, deadline expiry, supersession,
retirement and native no-op history completion. These controls are not yet wired
through worker navigation requests, loader milestone publication/checkpoints or
HTTP navigation tasks; existing navigation routes remain incomplete. Replacing
the existing busy-worker interruption behavior requires that integration. All
changes remain source-only, with no builds or tests executed.

## Controlled navigation transport and route generation

Added authenticated per-page context-service navigation events/checkpoints and a
ControlledNavigate worker/actor command. Its parent path preserves cross-origin
policy authorization without invoking the legacy busy-worker kill operation.
Native redirect/header/body loops now consult the shared control; normal legacy
navigation keeps its existing bounded deadline. Native document completion can
publish load after the actor command has returned. New document commits clear
stale scoped frame layout/action state. Route retirement stops navigation waiters.

HTTP navigate/back/forward/refresh now use owned tasks that distinguish native
command completion from readiness. None/eager can return while retaining the native
future, normal can await load after parsing, and beforeunload prompts are accepted
through parent control. Added route, ownership and control-authority regression
sources. Generation remains incomplete: worker outer wall budgets, command
interleaving during streaming parse, full stop-loading after timeout, scripted
renavigation, beforeunload loader invocation and local/data URL navigation still
need source integration. The new data-URL history regression intentionally exercises
a required path that is not yet generated. No builds or tests were run.

## Parallel generation checkpoint — September 16

Parent source integration now validates both live source and target for native
automation Window references. Script-close admission is revoked synchronously,
with retained handles used for ordered teardown. Regression source covers the
interval before teardown and rejects a retained closed source route.

Structured-message generation now has a context-owned MessageHub, privileged
MessageWire request/response routing, native backend, canonical origin lookup,
and page close/crash retirement. Queue admission is serialized with retirement;
successful posts wake destination actors after unlocking. Polls do not recursively
wake themselves. Source regressions cover forged and retired routes. Native clone
integration, realm/document lifetime authority and peer-close wakeups remain under
generation; this is not a claim of complete messaging support.

The family agent is generating a shared-parent native qualification binary and
operation audit rather than treating independent CLI parents as concurrent
contexts. An explicit idle-worker fault hook now checks the owned Child's SIGKILL
exit and reaping and returns actual PID evidence. It rejects foreign, unstarted,
non-process and busy targets; rejection regression source is present. Mid-operation
worker failure still requires separate implementation/evidence.

Beforeunload dispatch is now called before cross-document replacement and BFCache
restoration; veto maps to controlled-navigation no-op completion. JavaScript URL
handling and local/data navigation remain open. All packages remain uncertified
for generation completion. No builds, tests or conformance runs were executed.

## Data navigation and frame reference integration — September 16

Data URLs now decode through the existing bounded Fetch data processor and native
HTML/XML/text MIME selection, then enter the native document loader. Parent-only
preparation derives security and HTML meta expectations from the URL; plain text
does not become executable policy markup. Source regressions cover native script
execution, opaque origin, history, text/XML MIME, foreign preparation and malformed
data. File/JavaScript URL handling, complete inherited policy-container behavior,
and broader navigation cancellation/interleaving remain open.

Frame references now use native opaque connection tokens in session mapping.
Local inventory and remote WindowRealm results share one handle allocation path;
the previous parallel structural-key frame-handle map was removed. Parent hub
liveness verifies tokens against the owning context before serialization and
argument authorization. Regression sources cover aliases, detach invalidation,
foreign tokens and author attempts to supply internal identities. Native realm
registration and clone integration continue in the messaging agent. All changes
remain source-only and no package is certified generation-complete.

## Navigation deadline and milestone generation — September 16

Controlled navigation and native WebDriver scripts now wait for responses under
their own controllers rather than an unrelated generic IPC response deadline.
IPC writes stay bounded, and reads retain process-exit, framing and RSS checks.
Legacy commands retain their transport deadlines. Regression source covers the
deadline-owner distinction, including null and finite native deadlines.

New-document commit/readiness now publish from the loader; BFCache restoration
publishes the restored generation. The command wrapper avoids duplicate commit
publication after loader completion. A scripted replacement before load changes
the outstanding job's document identity, rejecting old-generation load events.
Eager readiness is latched so a later replacement cannot reinstate the already
satisfied page-load deadline. Source regressions cover replacement identity,
latched readiness and controlled data navigation milestone completion.

Full timeout stop-loading cleanup, cooperative command interleaving during
streaming parsing, JavaScript/file navigation and inherited policy containers
remain open. These source changes do not establish runtime conformance. No builds
or tests have run.

## Same-context cancellation recovery generation — September 16

Added an explicit page-local qualification cancellation API. It requires an
existing native worker, retires only the target page's prompt/navigation control,
interrupts in-flight execution and waits for native cleanup. Worker error cleanup
retains checked Child exit evidence before disposal; cancellation verifies that
evidence against the captured PID. Idle fallback uses checked kill/wait and is
reported separately through `interrupted_inflight: false`. Pool disposal adjusts
ownership instead of recycling the failed process.

The context's worker admission remains open so the family harness can create a
replacement page and exercise real session/cookie recovery. Source regressions
cover foreign/unstarted target rejection and preservation of context admission.
The family agent is integrating recovery recipes; mid-operation qualification
must require actual interrupted-inflight evidence. This does not establish full
navigation stop-loading behavior or runtime recovery acceptance. Source-only;
no builds, tests or conformance runs were executed.

## Load cancellation and origin authority generation — September 16

Native task checkpoints now detect a stopped navigation and stop queued document
load work without retiring the execution realm. Binding cleanup cancels image
operations/streams, invalidates delayed image events and clears queued dynamic
script/style/frame loaders. It preserves image source keys against accidental
restart by discovery checkpoints. Executable cleanup removes pending document
load/frame navigation queues and marks the load stopped, so removing a pending
load does not fabricate successful completion. A later new document stream can
establish its own load. Regression source checks image cancellation, suppressed
rediscovery and continued DOM execution/new authored image loading.

This is partial stop-loading integration: currently executing synchronous
subresource fetches, parser interleaving, Fetch/XHR cancellation semantics and
all deferred/admission edge cases still need generation. No runtime behavior is
verified. In parallel, parent messaging now passes canonical security-origin
identity keys to the hub, preserving distinctions between opaque origins which
all serialize as `null`. All changes remain unbuilt and untested.

## Worker registry and cancellation error-path generation — September 16

Navigation loader failures now run the same cleanup as checkpoint-detected
cancellation, preserving the controller's timeout/supersession reason and
clearing late milestone admission. Added executable regression source showing
that stopping a pending load preserves DOM execution, does not dispatch success,
and permits a later document stream to establish its own lifecycle.

Added/exported the parent AgentRegistry source with trusted bootstrap admission,
dedicated/shared/service identity records, origin/partition separation, random
client capabilities, per-client event mailboxes, ownership retirement and an
explicit process transport interface. Named shared workers reject URL/type/
credentials mismatches instead of creating another agent. Regression sources
cover shared reuse, mismatches, client ownership/capability forgery, partition and
opaque-origin separation, last-client teardown and mailbox isolation.

This registry is not yet connected to context service or standalone process
launch. Shared connection ports, service lifecycle/fetch interception, streaming
responses, queue failure handling and complete process qualification remain
open generation work. Module existence is not evidence of operational worker
support. No builds or tests have run.

## Worker event routing and message partition generation — September 16

Worker registry event ingestion now reserves capacity for the whole batch before
publishing any client event. Overflow/protocol/clone failures become terminal
agent failures with explicit process teardown; subsequent client polls cannot
silently resume after discarded events. Successful teardown is not repeated.
Lifecycle and fetch-control results go to a parent-only queue, not page-facing
client queues. Added regression source for that separation, atomic overflow,
principal revocation and one-time successful teardown.

Message routing now installs canonical top-level worker/message partition keys
in the parent hub and wakes BroadcastChannel destinations after posts. Keys
distinguish context, schemeful site and opaque origin identity; inherited tuple
origins preserve the creator's site. Regression source covers these distinctions.
The helper is available for parent-authorized agent bootstrap. Reconciliation
with all existing storage backends and complete agent process/context-service
integration remain open. All work remains source-only; no tests or builds ran.

## Context-owned agents and outbound transfers — September 16

All native context constructors now own an AgentRegistry. Explicit context worker
retirement drains it and still attempts page-worker cleanup when agent teardown
returns an error. The current internal registry bound is 64 agents; configurable
resource-budget reconciliation remains open.

Worker-bound registry messages now require parent MessageHub transfer validation
and ownership rotation from the actual Window/Agent principal into the destination
agent. Process-queue rejection closes remapped port capabilities. Regression
source covers foreign-owner rejection before transfer and disposal after queue
rejection. Incoming worker message transfers and SharedWorker connection ports
still need the corresponding integration, along with process launch and context
service routes. These source changes remain unbuilt and untested.

## Incoming worker transfer generation — September 16

Worker registry client/control polling now requires the parent MessageHub.
Incoming transfer tickets rotate from the worker principal into the actual
receiving Window/Agent principal after batch capacity checks. Port fan-out to
multiple clients is rejected, no-recipient transfers are disposed, and a failed
batch releases already remapped capabilities and retires sender endpoint
authority. Added regression source for receiving-window ownership and rejection
of subsequent access through the sending agent.

Client-release/registry-teardown disposal of already queued transferred ports
still needs integration. Shared connection-port setup and standalone agent IPC
remain open. These are source-generation changes, not tested worker support.
No builds or tests have run.

## Queued worker-port teardown generation — September 16

Client release, owner retirement, agent termination and context retirement now
take the parent message hub and dispose transferred ports still held in discarded
client queues. Teardown revokes client admission even when process termination
fails, retaining failed process ownership for retry. Shared workers remain alive
while another client owns a connection. Added regression source for releasing
an undelivered transferred port without terminating a surviving shared client.

Context cleanup callers were updated for the new registry/hub contract. Window
worker binding dispatch, trusted creation validation and concrete worker-global
runtime/process launch remain open integration. The standalone thread transport
being generated by the messaging agent does not by itself establish worker
execution. No builds or tests ran.

## Native inline SVG capture generation — September 16

Native screen/print paint capture now serializes an inline SVG tree from a copy
of the authoritative DOM, projects computed SVG presentation styles, applies the
local content-box viewport, and supplies the renderer's SVG image resource.
Captured descendants leave the outer paint list; root layout text is suppressed
to avoid duplicate glyph painting. Root opacity and transform remain owned by
the outer paint projection and are reset inside the captured XML. Nested image
references pass native URL and image-policy checks and use snapshot resources.

Regression source covers namespace/escaping, unchanged original DOM, inherited
style capture, root versus nested effects, resource collection, byte/viewport
bounds, and native snapshot descendant/text suppression. It has not run.
Native foreignObject subtree layout/composition, external SVG document capture,
CSS resource references and custom font alias integration remain open. This
does not certify package V or close any full-plan generation requirement.
No builds, tests, dependency installation or conformance runs were performed.

## SVG foreignObject native producer generation — September 16

The native producer now supplies the shared foreignObject snapshot contract:
capture assigns private XML tokens without replacing author IDs, resolves local
SVG positions and extents, and independently formats HTML using the document's
computed styles and the foreignObject viewport. Local paint nodes and captured
fonts enter the owner snapshot. Image completion now addresses either page nodes
or foreignObject nodes, so both use the same bounded resource collection.

Added regression source for local coordinates under an offset outer SVG,
percentage extents, captured HTML images, author-ID preservation, forged capture
token replacement, and suppression of descendants from the outer paint list.
The layout/render agent supplied the independent subtree layout and composition
consumer. All of this remains unbuilt and untested.

Remaining SVG integration includes nested SVG native capture inside HTML
foreignObject content, external SVG HTML parsing/layout, overflow-visible paint
bounds, target-density rendering, and semantic PDF text for composed HTML.
Full generation and all final qualification requirements remain open.

## Nested SVG/HTML capture ownership — September 16

Native SVG capture now recursively handles SVG trees inside foreignObject HTML.
Owner image/object indexes are reserved before recursion; each local paint tree
suppresses only the descendants replaced by its captured nested SVG image.
Outer SVG serialization no longer registers nested HTML resources or nested
foreignObject records against the wrong image owner. Recursion and object counts
are bounded, while image bytes retain the shared snapshot budget.

Regression source covers distinct nested owner indexes, HTML versus SVG text
ownership, and nested resource collection. No build or test was run. External
SVG HTML document capture and the other previously recorded rendering and
qualification requirements remain incomplete.

## Paint edge cases and service event routing — September 16

Zero-area SVG roots now suppress their owned descendants without requesting an
invalid image surface or aborting the containing page snapshot. Added native
regression source for zero width/height with nested text and HTML content.

Worker registry routing recognizes the new service-worker FetchFallback event
as parent-only control, alongside response and lifetime results. Regression
source verifies that fallback is absent from client events and preserves its
order relative to lifetime completion. The service-worker controller and
authorized process/loader integration remain unfinished. No builds or tests ran.

## SharedWorker parent connection-port generation — September 16

The native registry now has complete connection-port admission for shared
workers: the parent message hub creates entangled endpoints directly for the
authenticated client and agent principals, sends the agent's endpoint in its
Connect command, and returns the client endpoint. Both capability secrets are
allocated before either endpoint is published. Ordinary MessageChannel creation
uses the same allocator.

Rejected worker Connect commands close both new endpoints and roll back the
new registry client, preserving existing shared clients. Regression source
covers actual port delivery, cross-principal rejection, reused shared workers,
and connection rollback. Native context-service creation/loader/process routing
and complete document-retirement integration still remain open. No builds or
tests ran; this does not establish verified SharedWorker support.

## Worker document/page retirement generation — September 16

Added registry retirement operations for an exact document lifetime (page,
frame/root target, document generation, and stream epoch) and for an entire
page including hosted frames. Matching client removal reuses the existing
transferred-port disposal and recursive dedicated-agent teardown. Shared
workers remain owned while another page still has a client.

Regression source covers stale and repeated old-document retirement, replacement
document survival, whole-page frame cleanup, sibling shared-client survival,
and recursive dedicated-worker termination. Parent close/crash/document-notify
call sites still need integration with safe process-join/lock ordering; these
new registry operations alone do not establish lifecycle completion. No builds
or tests ran.

## Native worker process lifecycle evidence — September 16

Added an opt-in parent-local lifecycle audit with native launch/session identity,
PID, parent PID, binary identity, roles, wall-clock spawn/reap timestamps, checked
exit status, and context/page/generation lease intervals. Hooks cover process
spawn, verification classification, prewarm preparation, successful environment
attachment/reset, related-page release, checked qualification termination, and
session teardown. Bounded history reports overflow; missing checked reap evidence
is retained as a gap rather than inferred success.

Added source regression for checked exit, missing reap evidence, and selective
lease retirement. The qualification agent is integrating snapshot export and OS
sample reconciliation. Startup failure paths, process reuse accounting and all
qualification acceptance still require the full source audit and later execution.
No builds or tests ran.

## Native action admission source follow-up — September 16

Typed action requests now revalidate source/action compatibility, duplicate
identities, limits, origins, key code points, finite pointer properties and safe
integer bounds before persistent source admission. This also covers requests
constructed directly or decoded from native IPC, which otherwise bypass JSON
parsing. Integer-valued decimal/exponent JSON numbers now follow the same rules
as integer spellings. Added unexecuted regression source for numeric spellings,
typed invalid bounds and atomic rejection of a malformed late source.

The authenticated HTTP `/actions` route still reports unsupported operation;
native dispatch and direct clipboard grant integration remain required. This
source change does not establish action conformance. No builds or tests ran.

Persistent input state also exposes the union of modifiers across keyboard
sources, preserving left/right and independently held modifiers. Pointer button
bits use DOM middle/right ordering with checked shifts. Pointer cancellation
clears only its source's held buttons and pending releases, preserving its
position, other pointer sources and keyboard state. Added regression source
covering those transitions and final release cleanup. These dispatch inputs
are generated; authenticated endpoint/native event integration remains open.

Native host integration now resolves action element capabilities only within
the selected live realm and checks its exact document root/generation/stream
epoch. A crate-private clipboard dispatch path enters that realm, mints the
native single-use grant, applies the execution deadline, and rechecks retirement
after callbacks. Author script globals do not expose this path. Added regression
source for forged references, stale selected lifetimes and detached targets.
HTTP action dispatch is still not connected; no runtime success is claimed.

Action coordinate resolution now reads a fresh native layout viewport and first
element border fragment, computes the clipped in-view center, adds viewport or
pointer-relative offsets and rejects nonfinite/out-of-bounds targets without
mutating input state. Host resolution validates document lifetime around the
layout read and uses native element capabilities. Added unexecuted regression
source for fragment clipping, signed extents, bounds, missing rectangles and
author-overridden DOM geometry methods. Full event dispatch remains open.

Trusted native input event construction now uses captured instance templates and
typed boolean/number/UTF-16 payloads. Private payload objects and slot arrays
have null prototypes, preventing author constructors or inherited setters from
running during construction. Native event records carry trust/composed flags;
nonfinite numeric inputs are rejected. Added unexecuted regression source for
constructor/prototype tampering and exact surrogate preservation. The keyboard
subagent is integrating this primitive with keyboard events and default actions.

Native wheel events now use the shared native paint-order hit tester (also used
by Document point queries), retaining transforms, fragment bounds, inertness,
pointer-events and shadow retargeting semantics. Native wheel delivery targets
the actual shadow descendant and applies pixel scrolling only after uncanceled
dispatch in the same document lifetime. Added source regression for author hit
test/constructor overrides and canceled versus accepted scrolling. Host keyboard
and wheel dispatch methods now enforce lifetime and execution deadlines. The
HTTP action scheduler/transport remains open; no builds or tests ran.

Native pointer event source now carries device identity, coordinates, buttons,
pressure, contact size, tilt/twist and modifiers. Capture state is document
lifetime scoped with pending/effective targets, native Element capture methods,
touch implicit capture and lost/got notifications. Up/cancel releases capture;
host dispatch checks document lifetime and execution deadlines. Added unexecuted
regression source for pen capture across hit-target movement and release ordering.
Compatibility mouse/boundary/drag sequencing and the HTTP scheduler are still
required; this primitive is not full pointer action conformance.

Pointer generation now includes boundary enter/leave/over/out sequences from
native ancestry, compatibility mouse delivery, suppression after canceled
pointerdown, chorded button transitions, native down/up click targeting and
focus defaults shared with the existing input adapter. Added regression source
for suppression, subsequent normal mouse delivery and native focus. Enter/leave
events are noncomposed. Drag consumption, touch gesture behavior, capture/click
ordering refinement and HTTP action scheduling remain open; nothing has run.

Keyboard host routing now maps native focused-document identity to a live
automation realm and verifies every embedding lifetime back to the selected
document. It rejects sibling or stale authority instead of dispatching to the
parent body. Added unexecuted regression source for trusted child-frame key
delivery, canceled defaults, absence of parent duplicate delivery, sibling
rejection and removed/stale documents. This closes the source routing connection;
runtime behavior and full action scheduling remain unverified.

Generated a thread-affine action execution layer that admits persistent source
identities, dispatches native keys/buttons/cancellation, resolves motion origins,
interpolates pointer/wheel ticks while the caller services page tasks, and releases
held inputs in reverse press order. Pointer IDs remain stable across subsequent
source admission. Added unexecuted regression source for longest-source tick
timing, regressed polling time, keyboard repeat/release and pointer ID stability.
Page runtime/IPC/HTTP integration is still required, as are remaining pointer
gesture/drag and multi-pointer semantics; no builds or tests ran.

The preceding runtime/IPC/HTTP connection gap now has generated implementation:
PageState retains input ownership, polls action ticks while servicing page tasks,
checks cancellation/lifetime changes, and retains partially dispatched state for
release. Typed worker requests/responses and scoped-frame routing carry perform
and release operations (worker protocol 12). Authenticated HTTP POST/DELETE
`/actions` uses lossless request parsing and the existing admission/cancellation
scheduler. The unconditional unsupported route was removed. Added HTTP regression
source for cross-request held keys, repeat, release and malformed input. None of
this has been executed; remaining pointer/drag/touch/multi-source semantics and
full integration review are still required before qualification.

Release now resets native pointer hover/capture records across page realms after
held-input release succeeds; a reused pointer ID cannot inherit old hover state.
Added regression source for that reuse. Native pointer dispatch now directly
shares the existing drag-session core with the legacy adapter, starts eligible
mouse drags after movement, cancels pointer delivery/capture, advances/drop/cancels
native payload sessions and suppresses release clicks once drag consumed input.
Added source regression for exact payload delivery and no follow-up click. These
paths remain unexecuted; selected-text/cross-frame drag and broader gesture
semantics still require completion.

ServiceWorker message delivery now has a native registry route that validates
recipient/source kinds, native agent storage authority, payload/metadata bounds,
and remaps port capabilities to the service process. Failed sends dispose routed
ports; generic service control cannot bypass that route. Added unexecuted source
regressions for transfer ownership, rejection cleanup and cross-partition denial.
Native pointer dispatch also suppresses pre-contact touch hover and nonprimary
touch click compatibility, with regression source; multi-pointer source-state
normalization is being integrated in parallel. No builds or tests ran.

Uncanceled native Escape now cancels the active drag using the latest native
pointer coordinates and current keyboard modifiers. Consumed pointer state
remains until release, preventing a canceled drag from generating a mouseup/click
sequence. Expanded native drag regression source to cover both successful drop
and Escape cancellation with no payload delivery or click after cancellation.
Selected-text and rich-content drag defaults are assigned for further generation;
these source changes have not been built or tested.

Pointer capture now releases before click delivery while preserving the
pointerup capture target for click/auxclick targeting, including capture by an
element different from the original press target. Added unexecuted regression
source asserting pointerup/lostcapture/click order and false hasPointerCapture
inside the click listener. Native mouse press and non-mouse release also notify
user activation. Full interaction conformance remains unverified.

## Natural worker closure and terminal delivery — September 16

Service registrations now have a native registry admission API that allocates
an AgentId and owns the process with zero window/client capabilities. Admission
publishes the agent before starting it; failed startup retains the process and
capacity until detached teardown. Native service commands require a live
Service agent and restrict control to lifecycle, fetch and cancellation.
Explicit service retirement also stages descendant cleanup. Added unexecuted
regression source for kind rejection, client-free admission, control restriction,
startup failure ownership and capacity recovery. Service registration/controller
and interception integration is being generated by the worker subagent; this
API alone is not evidence of ServiceWorker conformance.

Source generation now separates a closed worker's execution authority from its
existing clients' authority to drain queued events. Registry polling commits
messages preceding Closed before retiring agent endpoints, rejects events after
Closed in the same batch, and stops polling the closed process. Both client and
native control polling retire descendant ownership. Closed shared workers can
be replaced while their old queues drain; old cleanup cannot erase the new
shared-worker identity. Unowned closed services remain retained until pending
control results drain, then enter detached process teardown.

Added regression source for queued message ordering, revoked bootstrap/post
authority, retained drain capability, single process poll, shared replacement,
invalid terminal batches, and service control/descendant cleanup. These are
unexecuted sources, not conformance evidence. All package completion gates stay
open; no builds or tests ran.

## Bounded lifecycle ownership history — September 16

Lifecycle lease history now has a shared 65,536-record cap across all processes,
in addition to the process cap. Overflow retains incomplete evidence rather than
allowing per-process histories to multiply without a global bound. Overlapping
page leases, leases admitted after reap, invalid roles and malformed binary
digests become explicit gaps. Added regression source for shared-budget
exhaustion and invalid lifetime transitions. No builds or tests ran.

## Frame worker creator authority — September 16

Added a parent-only creator snapshot that verifies a live source page and the
exact registered frame document generation/stream epoch, derives storage
partition from the parent page's native security origin, and preserves the
frame's native origin authority (including inherited opaque origins). Constructor
options do not supply origin or partition authority.

Regression source covers stale epochs, replacement generations, foreign-page
frame tokens, inherited opaque origin keys, and exact retirement. This snapshot
is a validation primitive: the pending worker loader must revalidate after async
fetch before admitting an agent. Root-document epoch registration, loader,
context-service dispatch and process launch remain incomplete. No builds or tests ran.

## Root worker creator lifetime registration — September 16

Added native root document registration/retirement messages and exact
generation/stream-epoch lookup. Retired lifetime tombstones prevent stale
re-registration, page retirement removes root authority, and agent principals
cannot perform Window lifetime registration. Parent window-worker validation
now supports root and frame creators and derives root origin authority directly
from native security state.

Added regression source for document.open-style epoch changes, stale retirement,
retired-lifetime resurrection, agent rejection, and page removal. Native binding
registration hooks and complete WorkerService/loader/factory wiring are assigned
to the worker agent. BFCache reactivation requires an explicit trusted path for
restoring an older generation; it remains open. No builds or tests ran.

## Worker admission gate and deferred failure cleanup — September 16

Registry connection admission now starts a prepared worker only after its
principal/client exist; SharedWorker Connect is enqueued before start. Release,
termination, failed creation and terminal poll failure no longer join processes
inside registry/message operations. Revoked processes remain owned and count
against capacity until a detached pending batch terminates outside broker locks.
Context/selective teardown also collects pending processes.

Regression source now distinguishes revoked authority from completed process
cleanup, and covers Connect-before-start plus deferred join/capacity accounting.
The worker-service agent is wiring mandatory post-dispatch pending-batch cleanup.
All code and tests remain unbuilt and unrun.

## Page close agent cleanup and idempotent queued disposal — September 16

Teardown now catches an unwinding process-termination panic per agent, retains
that process in quarantine for retry, and continues terminating peers. New
regression source checks retained ownership, peer termination, guard release,
and a successful retry. Closed page handles and `BrowserContext::close_page`
can retry retained agent cleanup without sending another command to a dead
actor. Ordinary page lookup still excludes closed pages. The close regression
now injects a first teardown failure and checks retry plus idempotent completion.
These are source changes only; no build or test has run.

Additional integration regression source now exercises `PageHandle::close` with
two pages attached to one shared agent. It asserts that the first close revokes
only its client capability, the sibling remains usable, and the last close
acknowledges agent termination. The mock termination attempts to acquire the
registry, page, and message locks to detect locks held across a join. This is
source coverage, not execution evidence; no build or test has run.

Explicit, ordered script, and interrupted page-close paths now retire the page's
agents after page admission is revoked, terminating detached processes outside
registry/message locks. Queued transfer disposal accepts endpoints already
removed by page retirement while still rejecting live foreign capabilities.
New source regression covers atomic authority checks and repeated disposal.

Empty pending-cleanup batches no longer acquire or clear the global retirement
guard; regression source verifies they cannot release another in-flight batch's
guard. Crash/implicit-drop and exact document-notification lifecycle integration
remain part of the ongoing parent worker-service work. No builds or tests ran.

## Agent context teardown without broker-lock joins — September 16

Context worker retirement now detaches a revoked batch under registry/message
locks, releases both locks, then terminates the owned agent processes. Queued
client transfers are disposed before agent endpoint authority is retired.
Failed terminations return to the retired registry for retry without restoring
client admission. A concurrent retirement attempt cannot report completion while
another batch remains in flight.

Added source regressions for broker-lock access during termination, retained
failed-process ownership, retry, and concurrent-retirement rejection. Exact
document/page teardown callers and agent broker-wait cancellation still need
integration. No builds or tests ran.

## Selective detached agent retirement — September 16

Added lock-safe detached batches for exact document lifetimes and whole pages.
Selection recursively includes dedicated descendants, disposes queued transfers
before endpoint retirement, and preserves shared agents with surviving clients.
Finishing a selective batch no longer retires the whole registry; unrelated
admission resumes afterward. Concurrent admission is rejected during a batch.

Regression source covers replacement-document survival, shared sibling survival,
dedicated descendant termination, and unrelated admission after selective cleanup.
The parent close/crash/document-notification call sites still require ordered
admission revocation and integration; no lifecycle-completion claim is made.
No builds or tests ran.

## Worker startup ownership cleanup — September 16

Added an OS-child launch guard covering every fallible step between spawn and
session ownership, including running-code verification, RSS monitoring, broker
startup and pipe setup. Early return now kills and waits for the owned child,
records checked reap evidence, stops the monitor and revokes broker routes.
Successful handoff transfers the child and audit observation to the session.
Verification role is assigned at spawn so failed verification launches retain
their actual purpose in the lifecycle journal.

Generated regressions for early-launch reap/monitor cleanup and successful
ownership handoff without premature termination. These tests have not run.
Full worker creation/service integration and final qualification remain open.

## Lifecycle evidence integrity — September 16

IPC read/write polling and qualification precondition checks now retain checked
exit status at the first native observation. Lifecycle timestamps preserve raw
wall-clock values while recording clock rollback/unavailability as gaps; poisoned
audit locks cannot produce complete evidence. Added regression source for clock
rollback and poisoned-history rejection.

The qualification source audit identified auxiliary signing-verification helper
processes as another ownership requirement. They remain explicitly unattributed
until native helper spawn/wait evidence is integrated. No builds or tests ran.

## Direct Linux RSS and helper-attribution correction — September 16

Fresh source inspection supersedes the preceding signing-helper hypothesis:
macOS static/running-code validation uses Security.framework in-process and
RSS uses proc_pidinfo. No signing helper subprocess exists in those paths.

The actual Linux RSS helper was `/bin/ps`. Linux measurement now reads the
kernel's `/proc/<pid>/status` VmRSS counter directly, with strict unit validation
and checked byte conversion. This removes measurement subprocess overhead and
measurement descendants from native process-tree accounting. Other Unix
fallback behavior remains separate. Added parser regression source covering
resident versus virtual memory, zero, missing/duplicate/malformed fields and
overflow. Unknown observed descendants still fail qualification attribution.
No builds or tests ran.
