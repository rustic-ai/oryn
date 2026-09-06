# Complete conformance execution plan

Reviewed against commit `10f9b46` and the signed-worker evidence available on 2026-09-06.
This is the active forward plan. Earlier batch sections in the runtime action plan remain history.

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

The clean **complete full baseline** at `eb54e11` is **2,559/4,022**, with
1,463 failures: 20 gains and 0 lost file passes versus `c12167f` at 2,539/4,022.
DOM passes 375/662 and CSS 240/564.
Exact inventory coverage, reported passing completions, source and signed artifacts
were verified. No file that passed the previous complete baseline became a failure in this run.

The [previous normalization baseline](CONFORMANCE-NORMALIZATION-BASELINE-2026-09-06.json)
retains its four lost passes and isolated history rechecks as historical evidence.
The current full run incorporates the subsequent compound/focus, move/custom-name,
and observer-ordering/scheduling changes; selected results are not added to its score.

The [planning backlog](CONFORMANCE-FAILURE-BACKLOG-2026-09-06.json) now uses that
single full ledger, replacing the original mixed-candidate planning index.
Each open ID retains its source evidence, failure domain, first observed failing
assertion when available, and workstream assignment. Routing remains provisional;
first-error text does not establish a root cause. No ID is silently discarded.

| Root | Latest evidence used for planning | Open executions |
|---|---|---:|
| DOM | Clean full baseline | 287 |
| Editing | Clean full baseline | 586 |
| CSS | Clean full baseline | 324 |
| IndexedDB | Clean full baseline | 115 |
| HTML | Clean full baseline | 106 |
| WebIDL | Clean full baseline | 29 |
| Referrer policy | Clean full baseline | 8 |
| Accessible Name | Clean full baseline | 4 |
| WAI-ARIA | Clean full baseline | 4 |

Later root checks at `a14bc15` reached 372/662 DOM and 240/564 CSS. Assertion-level
regressions found during that work were corrected and verified at `ab88096`;
[batch evidence](CONFORMANCE-COMPOUND-FOCUS-2026-09-06.json) preserves the history.
These historical selections are superseded by the current full ledger for planning counts.
Native observer scheduling/error reporting and the newly retained Range wall-time
diagnostics remain immediate prerequisites for further conformance work.

The next observer prerequisite now tracks native journal sequence, matching ancestor
position, and registration order for ordinary registered observers within a realm.
Updating `observe()` options preserves registration position; disconnect/re-observe
creates a new position. Empty notifications after `takeRecords()` clear pending
ordering state. Native regression tests cover these cases and historical ancestry
when a mutated subtree is subsequently removed.

The next scheduling change uses an agent-wide ordered pending set and a native V8
microtask, populated at generated operation/setter boundaries and native task/parser
entry points. Tests cover earlier/later Promise jobs, callbacks that mutate again,
and observers becoming pending across realms in both orders. Non-observing realms
are excluded from the collection path, and unchanged mutation cursors avoid journal
reconstruction. Host integration also preserves execution deadlines and resets the
queued flag after V8 discards microtasks on termination. Connected reactions from
the existing insertion wrappers run before the method returns.

Transient-registration insertion order, complete callback/parser exception reporting,
worker-agent microtask isolation, and full custom-element reaction-stack semantics
remain open. Targeted native observer integration and the signed 12-case selection are verified: 8/12
passes versus 6/12, six existing assertions corrected, and no lost file passes or
previously passing assertions. The complete `eb54e11` ledger now covers all roots;
these selected results are retained separately and are not added to the full score.

[Native scheduling validation](CONFORMANCE-NATIVE-OBSERVER-SCHEDULING-2026-09-06.json)
records 117 passing V8 binding tests (two existing ignores) and all 265 native test
outcomes: 252 passed, 12 failed, and one navigation-fixture timeout. The normally
ignored pinned-Range diagnostic was run explicitly. All failures and the timeout
reproduce on the clean pre-scheduling source at `96d742f`, with matching failure
reasons. These existing failures remain completion gates, including frame lifecycle,
worker startup, editing state, startup-reference metadata, and runtime assertions.

The full assertion audit found 485 corrected existing assertions and no observed
pass-to-fail assertion changes. Twelve previously passing assertions are absent
from two partial Range results whose diagnostics report worker wall-time limits.
Five further name mismatches were verified as passing tests with changed fixture
ports. These missing Range results and changes in failure domain remain unresolved;
zero lost file passes does not establish freedom from all regressions.

Important concentration and dependencies:

- Editing's 586 failing executions cover 133 source files. Three whitespace-joining
  files account for 180 variants; two paragraph-insertion files account for another
  88. These are high-priority shared algorithms, not hundreds of independent fixes.
- CSS splits into 158 CSSOM, 113 selector and 53 cascade failures. Stylo owns
  the rendering style pipeline and selector matching. CSSOM declaration mutation
  still uses a separate binding helper; integrate it with Stylo's declaration API.
  Also investigate adapter state, stylesheet ownership and invalidation.
- All 106 HTML failures are history/browsing-context tests. Treat them together with
  frame activation, navigation tasks and the eight referrer-inheritance failures.
- The XML/XHTML iframe parser and missing interfaces exposed by createElementNS
  are fixed; that file now passes all 596 assertions. Broader XML navigation,
  fragment parsing, serialization and template ownership remain open.
- DOM failures include 91 event files, 35 Range files and 16 traversal files. Several event
  cases cannot reach their target assertion because required APIs, such as animation,
  are absent. A "harness" classification is not automatically a runner defect.
- `Range-insertNode` still needs resource diagnosis. The expanded `name-validation`
  loops now finish and pass after detached-wrapper performance and native name fixes.
  Profile native work, V8 callbacks and retained objects for remaining resource failures.

## W0 — establish the current baseline and keep diagnosis trustworthy

**Baseline completed:** clean source `eb54e11`, signed worker
`58bc9a61b046222447b6e82bbb1e5a74bed0a58f9d137f594c6c747a8d8f8406`,
ledger `conformance-worker-full-observer-microtasks-eb54e11.json`. All 4,022 IDs and passing
completions verified; all 1,463 failures routed. The initial resource batch has
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
| W2: bindings, interfaces and realms | 40 | `oryn-webidl`, `oryn-js` generated registry | Real required interfaces with correct conversions, brands, descriptors and realm behavior |
| W3: events, input and task scheduling | 86 | `oryn-js`, native scheduler/default actions | Correct event paths, ordering, activation and cancellation across required environments |
| W4: browsing contexts, history and referrer | 114 | Native runtime/context service and bindings | Document activation and session history with correct origin, realm and policy behavior |
| W5: CSSOM | 158 | Native stylesheet owner, generated CSSOM bindings, Stylo adapter | Live rule objects, editing, serialization, ownership and exceptions |
| W6: selectors and cascade | 166 | `oryn-layout`, Stylo/Blitz adapters | Correct tree/state/mode inputs, scope/cascade semantics and mutation invalidation |
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

## Immediate next execution sequence

1. Build clean `10f9b46` (or a later explicitly recorded candidate) and run all 4,022
   IDs; refresh the backlog and inspect every runtime/harness/incomplete outcome.
2. In the same diagnosis cycle, profile `name-validation` wall time and Range RSS;
   inspect XML frame MIME routing/parser suitability, and reduce one whitespace-join
   family to its shared Range/style contract.
3. Implement the confirmed document/ownership and binding prerequisites. Validate
   creation/adoption/realm tests and the entire DOM root; retain the new namespace passes.
4. Begin the event/lifecycle and CSSOM adapter contracts and the high-multiplicity
   editing families as their dependencies become testable; run transaction work on the
   same well-defined scheduling contracts.
5. Continue through the remaining workstream gates, then execute M5 without exemptions.

The existing release/product/platform action plan still applies independently.
A paid evaluation, deployment, or publication is not necessary to claim that this
4,022-ID conformance gate passes and is not authorized by this planning document.
