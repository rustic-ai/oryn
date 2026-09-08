# Complete conformance execution plan

Current full-run evidence: signed candidate `b40e1d1`, measured on 2026-09-08.
This is the active forward plan. Earlier batch sections remain history.

## Objective and definition of done

Complete every execution in the pinned v2 inventory: **4,022/4,022**, including each
variant and global. This establishes conformance to that inventory, not to every
web standard or every test in upstream WPT. Any broader browser-conformance claim
requires an explicitly expanded inventory and its own complete results.

Completion requires all of the following:

1. One clean source revision, pinned dependencies/fixtures, and an identified signed
   sandboxed worker; exact coverage of all 4,022 IDs, without duplicates or extras.
2. Every required execution reports successful testharness completion, a successful
   harness status, and all its assertions passing. No assertion, harness, runtime,
   incomplete, timeout, crash, or unsupported-capability outcomes remain.
3. A fresh complete rerun of the same candidate also passes. Investigate intermittent
   failures; a selected successful attempt does not erase a failing attempt.
4. Formatting, generated bindings, relevant strict Clippy/unit/integration checks,
   sandbox-denial and existing runtime profile-equivalence gates pass. Preserve the
   distinction between primary-profile conformance and broader release qualification.
5. The evidence validator independently recomputes the result and binds it to final
   source and artifact hashes. Any subsequent runtime change requires new evidence.

The existing 256 MiB V8 heap, 512 MiB worker RSS, 10 s command and 30 s navigation
limits, sandbox boundaries, and inventory remain unchanged. A resource-bound test
requires a runtime/performance fix or remains failing. Do not skip cases, alter
assertions, infer completion from a probe, or supply test-specific API substitutes.

## Evidence and what it tells us

The signed run at `b40e1d1` passes **2,990/4,022 (74.3%)**, with **1,032 remaining**.
It gains 10 whole tests and loses two versus `b074134`, for a net gain of eight.
The losses are `IndexedDB/open-request-queue.any.js` in window and dedicated worker.
No previously passing assertion was observed failing, but missing assertions remain
unresolved and the whole-test losses are regressions. Both committed-upgrade
lifecycle regressions from the previous candidate recover.

The audit validates exact inventory coverage, every passing completion, source and
binary fingerprints, and unchanged limits and sandbox settings. It observes 12
corrected existing assertions and 18 additional passing names. Five server-port
name differences reconcile; 28 newly missing prior passes remain. Thirteen older
gaps recover, 74 remain absent, and the combined tracked missing count is 102.
All 11 historical editing assertion regressions still fail.

[Full evidence](CONFORMANCE-IDB-SCHEDULING-FULL-2026-09-08.json) records the audit.
Stylo remains the CSS engine. IndexedDB passes 399/456; other root totals are unchanged.
Native validation and its narrower scope are recorded in
[native evidence](CONFORMANCE-IDB-SCHEDULING-NATIVE-2026-09-08.json).

Next: resolve the open-request queue regressions and validate the isolated native
focus-visibility draft against DOM/layout/JS/native gates and all 61 inventoried
focus-related selector fixtures. That draft is uncompiled and unmeasured. Cross-page
IndexedDB scheduling and other lifecycle gaps remain open. No next-candidate gain
is established until another signed full run and assertion audit complete.
Every remaining failure stays in scope; complete conformance is not established.

| Root | Passed / required |
|---|---:|
| IndexedDB | 399/456 |
| WebIDL | 30/58 |
| accname | 15/19 |
| css/css-cascade | 54/97 |
| css/cssom | 95/190 |
| css/selectors | 165/277 |
| dom | 400/662 |
| editing | 389/704 |
| html/browsers/history | 32/136 |
| referrer-policy | 1383/1391 |
| wai-aria | 28/32 |

The [failure backlog](CONFORMANCE-FAILURE-BACKLOG-2026-09-06.json) contains exactly 1,032 failed IDs from this run.


## Immediate execution correction

The latest batch gained 10 whole tests and lost two open-request queue executions.
Diagnose and recover those queue regressions before the next candidate. Validate
the already isolated focus-state integration draft, targeting the broader CSS
backlog while keeping Stylo. Do not treat missing assertions as non-regressions.

The backlog now joins the durable [diagnosis ledger](CONFORMANCE-DIAGNOSES.json),
keyed by inventory ID. A confirmed cause is bound to the full result hash, source
commit and observed failed assertion names. A later candidate retains the diagnosis
as history requiring revalidation; it does not inherit a confirmation automatically. Subsystem routing
alone is not a confirmed shared cause. Rank subsequent batches by the number of
failures linked to a reproduced common mechanism, then validate that mechanism
against unchanged affected fixtures before paying for another full signed run.
Record whole-test gains, losses, assertion regressions and missing assertions for
every full candidate; retain the completion gates above without exceptions.

## Throughput correction after the IndexedDB batch

The current backlog has 975 failures outside IndexedDB: editing 315, DOM 262,
CSS 250, HTML 104, WebIDL 28, referrer policy 8, and accessibility 8. Repeatedly
extending IndexedDB work leaves most of the overall score unchanged. Native
passes are validation of a candidate, not additions to the full-run pass count.

Close the current lifecycle/scheduling batch before expanding its scope. Preserve
any unresolved cases explicitly, then promote only validated changes and measure
one signed full candidate with the existing regression and missing-assertion audit.
After that measurement, investigate the existing editing regressions and shared
mechanisms in the editing, DOM, and CSS backlog. Choose a reproduced cause with
broad fixture impact; root counts and matching error text alone do not prove one.
Keep Stylo as the CSS engine. Each subsequent batch must have a bounded mechanism,
affected unchanged fixtures, and an explicit full-run measurement step before
starting another batch. The 4,022/4,022 twice requirement remains unchanged.

## W0 — establish the current baseline and keep diagnosis trustworthy

**Measurement completed:** clean source `cdead0a`, signed worker
`2eae1fb3d0874fb9f596afa17cc17ca0a95676457f12bcf0039e64a9f06a764d`,
ledger `conformance-worker-full-dom-boundaries-cdead0a.json`. All 4,022 IDs and passing
completions verified; all 1,294 failures routed. Assertion regressions and missing
coverage remain open as recorded above. The initial resource batch has
reproducible scaling and GC diagnostics recorded in the
[foundation report](CONFORMANCE-FOUNDATION-DIAGNOSTICS-2026-09-06.md).
The procedure below remains required for subsequent complete candidates.

Build the current clean commit and run the complete inventory. Keep the worker and
parent binary fixed for the entire run. Archive source, toolchain, inventory, fixture,
V8, ICU, snapshot, worker and policy fingerprints. Verify exact IDs and every passing
completion using the existing validator. Refresh the planning backlog from that run.

For every remaining ID, assign a confirmed or explicitly unconfirmed cause and an
owner component. Separate runner/fixture defects, missing required APIs, runtime
algorithm failures, and resource/lifetime failures. Preserve exceptions, task traces,
completion state and resource reasons. Keep variants/global environments distinct.
Do not group unrelated failures solely because both mention `TypeError`.

Audit uncaught script and callback exception reporting before declaring completion.
The focus-within regression exposed an earlier one-assertion pass while parser
script exceptions are retained as host diagnostics without Window error delivery.
A successful reported harness completion can therefore miss later registrations.
Implement native exception/error-event reporting and verify that an uncaught
parser exception makes an ordinary testharness run fail unless the test explicitly
permits it. Keep original exception identity and realm behavior.

The native `ErrorEvent` interface prerequisite is implemented with traced payload
storage, receiver brands, WebIDL conversions and inherited `composed` state.
[Validation](CONFORMANCE-NATIVE-ERROR-EVENTS-2026-09-06.json) records 118 passing
V8 binding tests (two existing ignores), strict JS/native Clippy and registry
validation. Parser/callback reporting, Window `onerror`, worker propagation and
initial frame realms remain open; this interface alone does not close W0 or
establish a new signed-worker conformance result.

Native global EventTarget dispatch and observer/global-listener exception reporting
are now integrated after isolated validation: 126 binding tests, three new native
page tests, strict JS/native Clippy, and a 268-test native comparison with no new
nonpasses. Global ErrorEvent handlers preserve payload identity, calling convention,
cancellation, registration order and cross-realm event ownership. The
[hash-bound evidence](CONFORMANCE-NATIVE-GLOBAL-EVENTS-2026-09-06.json) retains all
13 existing native nonpasses. Parser/module and node-listener reporting, remaining
global handler attributes, frame activation and worker propagation still need work.


Classic parser scripts now use native exception reporting for compile/run failures,
including thrown null/undefined, handler cancellation, synchronous reporting before
queued jobs, and continued execution of later scripts. External scripts carry a V8
source URL; cross-origin errors are conservatively muted because the current resource
table lacks CORS response-taint metadata. The ordinary host execute/evaluate API keeps
its existing error propagation. This is a partial W0 implementation: module errors,
inline document line offsets, precise CORS tainting, and callback script muting remain
open. The pinned testharness now proves rejection of unexpected parser exceptions and
acceptance when explicitly allowed. [Evidence](CONFORMANCE-NATIVE-PARSER-ERRORS-2026-09-06.json)
records seven passing regressions, 126 binding passes and all 275 native outcomes
(262 passes; the same 13 prior nonpasses and reasons). No new signed-worker score is claimed.


Document module scripts now retain compile/link exceptions and use native promise
reactions to report evaluation rejection, including rejection after top-level await.
Canceled errors suppress diagnostics; uncanceled delayed errors are recorded once.
Inline roots no longer collide in the module cache, dependencies remain shared, and
module execution exposes null `document.currentScript`. Ordinary host module APIs
continue to return errors without Window reporting. [Validation](CONFORMANCE-NATIVE-MODULE-ERRORS-2026-09-06.json)
records 13 passing parser/module regressions (including both pinned harness proofs),
126 binding passes, strict Clippy, and 281 native outcomes: 268 pass with the same
13 prior nonpasses and reasons. Pending-module load barriers, fetch/CORS semantics,
complete source-coordinate handling, node listeners and worker reporting remain open.


Node listeners and property functions now report exceptions through the native
callback-realm reporter and continue dispatch. Callback-interface objects are
supported; node once listeners are removed before invocation and registration IDs
prevent snapshot revival after remove/re-add. Platform callback exceptions report
while preserving the exception result used by IndexedDB. Host termination is checked
before ordinary checkpoint-error conversion. [Evidence](CONFORMANCE-NATIVE-NODE-ERRORS-2026-09-06.json)
records 16 native reporting regressions, 131 binding passes and strict Clippy.
The 284-test native comparison has 271 passes and the same 13 nonpass IDs. Twelve
reasons match; the Range fixture now supplies canonical resources and reaches a
missing-completion failure instead of aborting at a generic callback error. It is
not counted as fixed. Complete handler slots/options, platform phase semantics,
foreign Event ownership, remaining callback routes and frame activation stay open.
Refresh the clean signed-worker full baseline after this batch; improved error
reporting can reveal previously hidden failures and missing registrations.


Create a reduced regression for each confirmed shared cause. Record its dependency
contracts, affected IDs, expected behavior and decisive check. First failing assertions
are a starting point: inspect later assertions after each fix, and track assertion-level
regressions as well as file-level gains. A previously failing file that newly times out
is an unresolved change in failure mode even if the pass count improves.

**Exit:** complete current-candidate evidence; all failures routed; every active batch
has a reproducible cause or a bounded diagnosis experiment. Resource/incomplete cases
must be explained before their owning cluster can close. Implementations proceed
cluster by cluster; diagnosing every semantic detail is not a prerequisite for all work.

## Workstreams and acceptance contracts

Counts below are routing counts from the planning index, not guaranteed gains from a
single patch. Shared prerequisites may resolve failures in several workstreams.

| Workstream | Routed open IDs | Primary owner | Required result |
|---|---:|---|---|
| W1: native documents, DOM, Range and traversal | 190 | `oryn-dom`, native parser; `oryn-js` bindings | Correct document/tree ownership, node APIs and live boundary behavior, with bounded lifetimes |
| W2: bindings, interfaces and realms | 38 | `oryn-webidl`, `oryn-js` generated registry | Real required interfaces with correct conversions, brands, descriptors and realm behavior |
| W3: events, input and task scheduling | 80 | `oryn-js`, native scheduler/default actions | Correct event paths, ordering, activation and cancellation across required environments |
| W4: browsing contexts, history and referrer | 116 | Native runtime/context service and bindings | Document activation and session history with correct origin, realm and policy behavior |
| W5: CSSOM | 123 | Native stylesheet owner, generated CSSOM bindings, Stylo adapter | Live rule objects, editing, serialization, ownership and exceptions |
| W6: selectors and cascade | 168 | `oryn-layout`, Stylo/Blitz adapters | Correct tree/state/mode inputs, scope/cascade semantics and mutation invalidation |
| W7: editing | 586 | Native editing algorithms, Range/Selection and input | Correct DOM transformations, selection and input-event behavior across every variant |
| W8: IndexedDB | 115 | Storage host, structured clone, JS transaction/task bridge | Correct keys, transactions, cursor scheduling, abort/upgrade and origin isolation |
| W9: accessibility semantics | 8 | Native semantic projection with DOM/style inputs | Correct generated-content names, role resolution, reflection and invalidation |

### W1: close document and tree foundations first

1. Route response MIME type from the broker into initial/dynamic iframe activation
   and DOMParser. Reuse a suitable XML parser with namespace and well-formedness
   behavior; verify library capabilities before integration. Error-recovering XML5
   behavior alone is not proof of browser XML conformance. Keep HTML parsing on
   html5ever. Test declarations, namespaces, case, CDATA, processing instructions,
   malformed XML, entities, whitespace outside the document element, and MIME metadata.
2. Make document ownership reliable across append/insert/adopt/import/clone, including
   descendants, Attr nodes, template content, shadow trees and detached subtrees.
   Avoid conflicting realm-local ownership records. Test both existing and newly
   created collections after adoption, plus name reflection and document identity.
3. Complete attribute and doctype validation/reflection, NamedNodeMap and DOMStringMap,
   Text/other required constructors, table row collections and row mutation APIs,
   and remaining node/collection algorithms identified by the backlog.
4. Correct Range boundary adjustment and clone/delete/extract/insert algorithms, then
   NodeIterator/TreeWalker filtering, traversal mutation and exception behavior.
   Preserve UTF-16 offsets, node identity, retained endpoints and mutation delivery.
5. Profile Range RSS and name-validation wall time. Fix unnecessary allocation,
   full-tree scans, wrapper retention or repeated native crossings. Use large
   adversarial fixtures to show bounded retention and unchanged synchronous semantics.
   Do not introduce observable yields into synchronous DOM operations to evade limits.

**Dependency gates:** XML document creation/activation and ownership are ready before
cross-document W2/W4/W6 closure; Range mutation/selection contracts are ready before
W7 deletion, joining and paragraph algorithms. Remaining independent W1 APIs can
continue alongside dependent work after those contracts pass.

### W2: close interface and realm prerequisites

Audit generated IDL against interfaces actually required by the inventory, including
HTMLSpanElement/HTMLUnknownElement, ErrorEvent and constructors encountered by tests.
Implement required behavior, not constructor-shaped stubs. Check current/incumbent/
relevant realm, cross-realm exceptions and return objects, global prototype invariants,
implicit receivers, callback conversion, buffer/shared/resizable rules and legacy
platform-object behavior. Extend the existing generator and native owners.

Missing helper APIs used by event or WebIDL tests—animation, messaging, decoding or
worker support—remain real capability work. Identify their required standards contract
and add focused tests before crediting the dependent inventory case. IDL harness success
alone does not establish semantic implementation.

### W3 and W4: model browser lifecycle coherently

Build on native task queues and context records. Verify event path construction,
capture/target/bubble phases, listener snapshots/removal, passive/once/signal behavior,
retargeting, default actions, disabled controls, focus, microtasks and timer ordering.
Include required messaging, abort and animation lifecycle prerequisites.

Then close Window/WindowProxy relationships, same-origin access checks, frame creation,
removal and activation, initial about:blank/srcdoc, popup and navigation lifetimes.
Implement session-history entries and traversal, state cloning, push/replaceState,
fragment navigation, Location reflection and fully-active-document checks, followed
by referrer inheritance across those transitions.

Document `:target` must be navigation-owned state shared with Stylo. Test fragment
lookup/decoding, iframe-local targets, ID edits, removal and subsequent navigation;
on-query ID lookup is not equivalent. Never weaken same-origin or sandbox checks to
make cross-frame tests reach their assertions.

### W5 and W6: complete the Stylo integration and CSSOM surface

Audit the existing Stylo/Blitz ownership bridge. Map each failure to a native adapter,
CSSOM operation, invalidation path, or demonstrated upstream engine limitation.
Keep parsing and cascade in Stylo; use narrow, provenance-documented upstream patches
only when a reduced case establishes an engine gap.

For CSSOM, cover stylesheet/rule identity and lifetime, rule insertion/deletion and
exceptions, live lists, declaration mutation and serialization, imports, nested rules,
constructed/adopted sheet ownership, cross-document restrictions and shadow isolation.

For selectors/cascade, supply correct namespace, HTMLness/quirks, flat/shadow tree,
scoping roots and dynamic element/document state. Close implicit/shadow @scope,
layers/revert behavior, pseudo-state/content, media/import conditions and invalidation.
Use pinned native-versus-browser differentials for computed values and geometry where
required. A querySelector pass does not establish stylesheet matching or recascade.

**Exit:** all 564 CSS executions pass, with ownership/invalidation regressions covering
DOM/style mutations and the required differential checks. W9 consumes the resulting
generated-content and counter semantics.

### W7: attack editing by shared algorithms and variant families

After the relevant Range, input and computed-white-space contracts are stable:

1. Normalize boundary points and selection updates; fix character deletion, splitting,
   joining and preservation of white-space semantics. Start with the three joining
   families (180 open variants) and verify their full Cartesian variant sets.
2. Implement paragraph insertion and host restrictions, including inline and restricted
   hosts (88 variants in the two largest families), placeholder breaks, nested blocks
   and selection placement. Cover plaintext-only and preformatted content explicitly.
3. Complete insertHTML/paste style handling, inline/block formatting and command state/
   value behavior, then the shared command corpus under `editing/run`.
4. Close input/beforeinput cancellation, typing/composition and the required EditContext
   surface. Diagnose testdriver setup separately from editing algorithm output.

Test both resulting DOM and selection/event effects. Correct serialization alone is
insufficient. Compare all relevant variants after each algorithm change, including
previously passing ones. **Exit:** 704/704 editing executions, plus the supporting
Range/Selection/input regressions.

### W8: close database semantics on the real scheduler

Preserve the per-database connection queue already implemented. Complete key and
key-path conversion/exception precedence, structured-clone coverage, request/event
ordering, transaction active/inactive windows, commit/abort behavior, versionchange
and blocked upgrades/deletes, cursor ordering/interleaving, indexes and origin
isolation. Test asynchronous broker acknowledgements and window/worker globals.
Successful requests after abort and inconsistent host-versus-JS transaction state
must be eliminated. **Exit:** all 456 IndexedDB executions with complete harness
results, deterministic scheduling regressions and no resource failures.

### W9: close semantic projection after its DOM/style dependencies

Implement Accessible Name contributions from pseudo-content, list markers, counters
and alternate counter text, including multiple instances and mutation invalidation.
Close role-none conflict resolution, ARIA reflection and required interface checks.
Verify naming and role effects under focusability, hidden content, shadow composition
and style changes. **Exit:** 19/19 Accessible Name and 32/32 WAI-ARIA executions;
retain the existing semantic identity/churn gates.

## Milestone order

| Milestone | Work and dependencies | Completion evidence |
|---|---|---|
| M0 | W0: fresh clean baseline and failure routing | One complete current-candidate ledger and refreshed per-ID backlog |
| M1 | W1 document/ownership/Range foundations; W2 required bindings; W3 scheduler contracts; resource profiles | Targeted contract tests and signed affected-root runs, with unexplained failures resolved in active clusters |
| M2 | W4 lifecycle/history; W5 CSSOM; W6 state/scope/invalidation; W8 transactions, using M1 contracts | Complete affected roots after each cluster and full-inventory checkpoints |
| M3 | W7 whitespace/join/paragraph families as soon as their M1/style dependencies pass; remaining editing and W9 generated semantics | Every routed ID closed; complete affected roots, not selected-file totals |
| M4 | Residual audit of every remaining failure, including capability prerequisites, unsupported cases and resource failures | Zero open required IDs; no waiver or exclusion substituted for a fix |
| M5 | Freeze candidate, run complete inventory twice, validate final hashes and existing profile/security gates | 4,022/4,022 verified on both runs; independently validated evidence and accurate scope statement |

M2 and M3 are dependency gates, not an instruction to delay all editing until every
CSS/history test passes. Start the high-multiplicity editing families once their exact
prerequisites are ready. Likewise, independent storage work need not wait for XML.

## Execution cadence and decisions

Use a shared algorithm cluster as the unit of work and commit. For each cluster:
record failing IDs and the causal hypothesis; add a meaningful regression; implement
in the owning component; run unit/strict/generated checks; build/sign once; run the
focused tests and complete affected roots; compare file and assertion outcomes; then
update the backlog and commit. Keep source/artifacts fixed during each measurement.

Run a complete inventory at milestone boundaries and after cross-cutting changes to
realms, task scheduling, parsing, lifetime management or the generator. Do not rebuild
the signed worker while a run is using it. Preserve prior ledgers and failure-mode
changes even when the total pass count improves.

Priority within a ready milestone is dependency reach, number of distinct failures
explained, confidence in the cause, and implementation risk. Variant multiplicity
helps identify reusable algorithms but must not hide unimplemented capabilities.
If a patch closes one symptom while leaving the shared algorithm undefined, return
to diagnosis before stacking further special cases.

Do not promise a calendar completion date from pass counts. After M0 and the first
representative clusters, estimate remaining work per confirmed algorithm and capability,
including validation cost and uncertainty. Report completed gates, unresolved causes,
new gains/losses and resource trends. Full conformance remains the objective until
M5 passes; a smaller achievable batch is progress, not completion.

## Risks that need explicit resolution

| Risk | Required response |
|---|---|
| A test fails before exercising its advertised feature | Identify the missing API, fixture or realm prerequisite; implement its actual contract and rerun the dependent case |
| First-error fixes reveal new failures or resource limits | Track assertion outcomes and failure domains, retain the new evidence, and profile before closing the cluster |
| XML parser library lacks required well-formedness or namespace behavior | Validate reduced normative cases before adoption; select or adapt a suitable existing parser rather than accepting an incompatible mode |
| Cross-document fixes expose conflicting ownership models | Consolidate the owning state and test all mutation/adoption paths, including detached descendants and wrappers |
| Tentative tests, oracle disagreements or fixture defects prevent a valid implementation from passing | Reproduce with the pinned reference browser and a reduced case; distinguish implementation defects from upstream defects. Keep the current ID failing until resolved. Any corrected inventory is a separately versioned, explicitly agreed scope—not a silent change to this gate |
| A required capability is larger than its apparent test count | Estimate the real contract and dependencies; retain it in M4 rather than replacing it with a shallow stub |
| Pass counts improve while stability or memory worsens | Treat new crashes, timeouts, retention growth and assertion regressions as unresolved work even if no passing file was lost |

Native context primitives are now validated separately in the
[context lifecycle evidence](CONFORMANCE-NATIVE-CONTEXT-LIFECYCLE-2026-09-06.json).
They preserve native proxy identity across navigation and expose the incumbent
realm through public V8 APIs. The page frame-loading path has not yet adopted them.
The next integration must cover actual global identity, early initial documents,
ordinary navigation versus initial same-origin Window reuse, incumbent-origin
message delivery, cross-origin access and inactive document behavior together.
The four frame regressions remain open until the pinned tests verify that path.

Native Window receiver access checks are also validated in the
[Window access evidence](CONFORMANCE-NATIVE-WINDOW-ACCESS-2026-09-06.json).
Borrowed EventTarget methods and onerror accessors reject foreign navigated
Window receivers before argument conversion; DOM errors use the original realm
interface even when page code replaces the global constructor. The native matrix
is 277/290 with the same 13 nonpasses, and bindings are 131 passed, two ignored.
This prerequisite does not close the four page frame regressions or change the
2,561/4,022 complete-run baseline.

The page loader now retains the old frame realm and document until a replacement
is ready to commit. The [frame commit evidence](CONFORMANCE-NATIVE-FRAME-COMMIT-2026-09-06.json)
records the reproduced failure and passing regression, including a subsequent valid
navigation. Native validation is 278/291 with unchanged 13 nonpasses. This corrects
retirement ordering; native proxy adoption and the four WPT frame regressions remain open.

Frame commits now also clear the previous accessible document mapping when the
replacement is foreign-origin or opaque. The [frame visibility evidence](CONFORMANCE-NATIVE-FRAME-VISIBILITY-2026-09-06.json)
verifies origin transitions and returning to a same-origin document. Native validation
is 279/292 with unchanged 13 nonpasses. This does not close native proxy integration
or the four WPT frame regressions.

The public V8 bridge now exposes native cross-context property handlers behind
security-token checks. The [handler evidence](CONFORMANCE-NATIVE-CROSS-CONTEXT-HANDLERS-2026-09-06.json)
verifies foreign named/indexed dispatch, denied writes and normal same-origin behavior.
All 45 public headers match the pinned V8 git blobs. Native validation is 280/293
with unchanged 13 nonpasses. This hook is not yet installed on production frame
contexts; the Window whitelist and native frame integration remain open.

Native cross-origin Window descriptor machinery now includes callable/non-callable
method handling, descriptor attributes, ordered keys, error branding and GC-traced
WeakMap memoization. The [descriptor evidence](CONFORMANCE-NATIVE-WINDOW-DESCRIPTORS-2026-09-06.json)
verifies stable values through GC and collection of retired contexts after retained
callables are released. Native validation is 281/294 with unchanged 13 nonpasses.
Prototype/extensibility and same-origin indexed behavior still require verification;
real Window operations and production frame integration remain open.

Same-origin indexed behavior and immutable prototypes are now implemented. An
isolated opt-in V8 extension also verifies cross-origin structural completions:
preventing extensions returns false; setting the observable null prototype succeeds;
other prototype changes fail. Protected property reads still throw SecurityError.
The [structural evidence](CONFORMANCE-NATIVE-WINDOW-STRUCTURAL-2026-09-06.json)
records nine context passes, 131 binding passes (two existing ignores), and 283/296
native outcomes with all 13 prior nonpass IDs and reasons retained. Navigation
checks preserve global-proxy identity and expose fresh Window state. That isolated
stage did not install the patch in production profiles; the upstream unpatched
engine fails the normative structural test. Its qualification alone did not
establish reproducible profiles, signed-worker or sandbox/profile gates.
The full baseline and four production frame regressions remain unchanged.

The opt-in is now integrated into the reproducible profile builder and native
bridge, with hash-checked synchronized public headers. The canonical core-full
build generated a fresh snapshot and worker; all artifact hashes and reported
fingerprints match. [Profile evidence](CONFORMANCE-NATIVE-WINDOW-PROFILE-2026-09-06.json)
records nine context passes, 131 binding passes (two existing ignores), and 284/296
native outcomes. The missing importScripts entry in the external-reference
manifest is corrected; all 12 remaining native nonpass IDs and reasons match the
prior candidate. The signed candidate at `b8e5dae` also passes exact-entitlement,
filesystem/network-denial, handshake, DOM-mutation and parent-broker smoke checks;
heap, RSS, command and navigation limits are unchanged. Core-lite/profile
equivalence and the security update policy's remaining release gates are pending. Production
frame adoption and the four WPT regressions remain open; this is not a new full
conformance result or a release qualification.

Native `MessageEvent` is now implemented as a prerequisite for posted-message
integration: traced payloads, native source/port branding, frozen port arrays,
constructor conversions, legacy initialization and native global dispatch.
[MessageEvent evidence](CONFORMANCE-NATIVE-MESSAGE-EVENTS-2026-09-06.json) records
133 binding passes (two existing ignores), eight WebIDL unit passes, and 285/297
native outcomes with the same 12 nonpass IDs and reasons. Frame delivery still
uses facades; worker startup still supplies a JavaScript constructor, and native
ServiceWorker source objects remain unsupported. The signed candidate at `53ce8b8` passes exact-entitlement, sandbox-denial,
handshake, DOM-mutation and parent-broker checks. No WPT execution has been run
for this change, so the full baseline remains 2,561/4,022.

Native structured serialization now replaces the page's JavaScript clone implementation.
It preserves cyclic graphs and buffer relationships, transfers ArrayBuffers, and
serializes native Blob/File and DOMException data into the destination realm.
[Serialization evidence](CONFORMANCE-NATIVE-STRUCTURED-CLONE-2026-09-06.json) records
135 binding passes (two existing ignores), three selected native passes and 286/298
native outcomes with the same 12 nonpass IDs and reasons. Native MessagePort
transfer/delivery, Window operations and posted-message scheduling remain open.
The fresh signed inventory run at `19feaac` is complete: 2,562/4,022, one file gain,
no lost file passes, and 1,460 remaining failures. The baseline evidence above
records unresolved Range observations and all remaining conformance gates.

## Immediate next execution sequence

1. Recover the eight SVG filter/marker resource regressions through native Stylo
   support. Validate the isolated stylesheet source/rule batch against pinned CSSOM
   failures, including live mutations, identity, ownership and parser behavior. Prove selected WPT gains before integrating. Keep MessagePort work parked until a pinned failure needs it.
2. Prioritize shared causes in the current failing ledger. Each batch must name
   affected pinned IDs, preserve existing passes, and deliver signed-worker
   evidence; native unit tests alone do not increase overall conformance.
3. Correct the remaining frame regressions and missing Range assertions while
   preserving limits and requiring actual testharness completion. Continue all
   document, CSS, event, editing, history and IndexedDB workstreams.
4. Integrate validated batches and rerun the full inventory on a frozen candidate.
   Refresh the backlog from that complete ledger. Require all 4,022 executions
   to pass twice on one candidate plus the native/profile/security gates before
   declaring completion.

The existing release/product/platform action plan still applies independently.
A paid evaluation, deployment, or publication is not necessary to claim that this
4,022-ID conformance gate passes and is not authorized by this planning document.
