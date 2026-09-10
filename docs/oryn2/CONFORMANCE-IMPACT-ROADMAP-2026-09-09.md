# Conformance impact roadmap — 2026-09-09

Originally assessed at source `9abe008`; updated with the measured implementation
batches below. This roadmap supersedes the immediate queue in
`COMPLETE-CONFORMANCE-PLAN-2026-09-06.md`; its completion criteria still apply.
Full-inventory results, focused results and remaining work are reported separately.

## Latest full signed measurement

The [terminal signed run at `011d470`](CONFORMANCE-ROADMAP-CSS-FULL-2026-09-09.json)
passes **3,144/4,022 (78.2%)**, with **878 remaining**. It confirms all eight CSS
and two history gains below, with zero whole losses, zero matched assertion
regressions, 34 assertion corrections and no new incomplete executions. Exact
coverage, source/artifact identity and unchanged production limits are verified.
The backlog now uses this single ledger. The roadmap's two full measurements
increase the total from 3,109 to 3,144.

Twenty-three older missing observations recover; 119 remain and seven new Range
observations are absent, leaving 126 unobserved assertions. The affected Range
fixture hits the command limit in both versions. Six historical editing regressions,
170 historical ambiguous groups and 12 current duplicate-name groups remain open.
No observation debt is waived. All preceding CSS/history/referrer assertion
name/status multisets match their full-run projections; incomplete/timeout flags
differ in three CSS and ten history executions, all still failing.

The candidate passes all terminal gates: 516 workspace tests (6 ignored),
193 binding tests (2 ignored), 398 native tests (5 ignored), formatting and both
strict Clippy gates. The subsequent signed traversal slice below establishes eleven additional focused
gains; they are not added to this full total.

## Preceding full measurement at `82fe3b4`

The [full signed run at `82fe3b4`](CONFORMANCE-ROADMAP-FULL-2026-09-09.json)
passes **3,134/4,022 (77.9%)**, with **888 remaining**. It gains 25 whole executions
(17 editing, four DOMTokenList, four IndexedDB cursor), with zero whole losses and
zero uniquely matched assertion regressions. Exact inventory coverage and valid
passing completions are verified under unchanged production limits.

The selected 709-execution native audit passes 437 executions. Its exact assertion
names and statuses match the same projection of the signed full run: zero added,
missing, regressed or ambiguous assertions. Seven pre-existing incomplete/harness
executions persist. The previous paragraph batch's 106 uniquely matched assertion
regressions are repaired.

The full baseline comparison records 1,673 corrections, five new missing Range
observations, and 170 legacy duplicate-name groups. Fourteen older missing
observations recover, leaving 137 older plus five new missing observations. Six
historical editing regressions persist. These observations remain open; the batch
is not regression-free release qualification.

Implemented changes:

- Preserve pending typing styles across internal caret movement and retain their
  command order when creating nested markup, including undo/redo.
- Complete DOMTokenList length, indexed access, iteration, stringification,
  replacement, receiver checks and classList value forwarding. Four upstream
  fixtures now pass. The attribute-coverage fixture still fails eight assertions.
- Reject detached semantic targets before layout/action, avoid double activation
  of links, and retain semantic deltas across same-document navigation.
- Retain all 1,898 matrix cases by yielding after each complete case, without
  increasing production command limits. Schedule the timer after observing its
  initial fixture state to remove a race in the revision test.
- Preserve assertion strings through both window and worker result publication.
  The old text extractor normalized whitespace inside serialized JSON.
- Add duplicate-aware assertion reconciliation and preserve failed cargo statuses
  through the canonical test runner's pipelines. Clippy warnings now fail that gate.

Validation: the full native library suite passed **394 tests, with 5 ignored**
(848.31 seconds) before the DOMTokenList/typing-order changes. After those changes,
the binding suite passed **193 tests, with 2 ignored**, followed by the complete
709-execution native slice above. All **45 tooling tests** pass. The canonical
workspace gate passes **516 tests, with 6 ignored**, plus formatting and strict
Clippy. The separate V8-enabled strict Clippy check also passes for all targets
of `oryn-js` and `oryn-native` with all features enabled.

The legacy transport model accounts for the old audit's missing-name reports:
there are zero missing normalized name groups, but **155 duplicate-name collision
groups remain unresolved**. They are not waived or counted as regression-free proof.
Current lossless publication prevents that corruption in future evidence. The full
run above additionally finds 15 pre-existing duplicate groups outside this native slice.

## Frame history batch: signed focused results

The [complete signed history slice at `8526530`](CONFORMANCE-HISTORY-SIGNED-2026-09-09.json)
passes **47/136**, up from 45. Fixtures 009 and 010 now pass all eight assertions.
There are no whole-execution losses, matched assertion regressions, missing
observations, ambiguous groups or new incomplete executions in this comparison.
The URL-rewriting fixture's seven server-port-dependent names have an explicit,
equal name/status multiset reconciliation. Existing incomplete history executions
remain open. The [complete referrer-policy regression slice](CONFORMANCE-HISTORY-REFERRER-SIGNED-2026-09-09.json)
passes **1,383/1,391** on the same frozen signed candidate, exactly matching the
full baseline's whole-execution statuses and assertion names/statuses. All eight
existing failures remain; no new incomplete executions appear. These focused
results do not replace the full total.

- Register parser-created frame realms in the existing navigation registry and
  initialize their request initiator URLs. Previously only dynamic frames were
  registered, leaving parser-frame Location changes without a loader consumer.
- Consume frame-local history URL updates before detecting cross-document
  navigation. Update the request initiator synchronously in the native History
  operation, so same-task requests observe the new document URL.
- Delegate plain HTML `?pipe=sub` responses to pinned upstream wptserve before
  injecting the result collector. Real loopback requests verify the Referer
  substitution in history fixtures 009 and 010; 46 tooling tests pass.
- The native integration test passes for parser and dynamic frames, both history
  methods and both same-task/later-task child navigation. It verifies retained
  document identity, absence of synthetic-URL fetches, HTTP Referer and
  document.referrer after navigation.
- The generated production Blob URL path passes the binary fetch/XHR, worker
  source sharing and revocation audit. Its native registry is used when generated
  APIs are installed; the bootstrap text registry is a separate diagnostic fallback.

Both new native integration tests pass. The full all-features native library suite
passes **396 tests, with 5 ignored**, in 549.85 seconds. The binding suite passes 193 tests (2 ignored);
the canonical workspace gate passes 516 tests (6 ignored), formatting and strict
Clippy. The separate all-features/all-target V8 Clippy gate also passes.
[Validation evidence](CONFORMANCE-HISTORY-NATIVE-2026-09-09.json) retains log and
source hashes. These changes do not alter the
3,134/4,022 full measurement above.

## CSS integration batch: native fixture results

Stylo already parses and mutates nested declarations. The binding maps native rule
kind 24 to generic CSSRule, omitting CSSNestedDeclarations and its live style
surface. Add the generated interface and route its declaration access/mutations
through the existing native implementation. The full ledger identifies this
blocker in scope-declarations, scope-nesting and cssstyledeclaration-nested.

The remaining scope-nesting assertion reads a custom property after selectorText
mutation. Oryn exports a fixed set of Stylo computed properties but omits custom
values, also blocking selector specificity fixtures. Export Stylo's computed custom
properties and exclude raw inline custom declarations from the fallback, preserving
substitution and guaranteed-invalid values. Validate these bounded integration
changes before attempting the larger import ownership/resource graph work below.

Implemented and checked through the entire 564-execution native CSS inventory:
**368/564**, eight whole-execution gains, 26 corrected assertions, no whole losses,
no uniquely matched regressions and no missing observations. The additional gain
is cssstyledeclaration-registered-custom-properties. All 32 existing incomplete/
harness executions remain; no new incomplete execution appears. Nine duplicate
assertion-name groups persist with unchanged status multisets and remain ambiguous.
The native CSSOM suite passes all 35 tests. The [terminal regression gates](CONFORMANCE-CSS-NATIVE-2026-09-09.json)
pass **516 workspace tests (6 ignored)**, **193 binding tests (2 ignored)** and
**398 native tests (5 ignored)**, plus formatting, strict workspace Clippy and
V8-enabled all-feature/all-target strict Clippy. The signed full-inventory measurement at `011d470` above now confirms these eight
whole-execution gains.

## Live TreeWalker batch: signed focused results

The [complete signed traversal slice at `3eee013`](CONFORMANCE-TRAVERSAL-SIGNED-2026-09-09.json)
passes **13/18**, up from 2/18, with **eleven whole-execution gains and 793 corrected
assertions**. There are zero whole losses, matched assertion regressions, missing
or added assertions, ambiguous groups and incomplete executions. Every assertion
name/status and completion flag matches the native run. The 761-assertion
TreeWalker matrix passes in full. Production limits are unchanged.

The implementation traverses current light-tree links, separates FILTER_SKIP from
FILTER_REJECT, permits currentNode outside the root, and invokes filters only when
traversing. It retains callback identity, guards reentrancy, preserves thrown values,
converts callback returns using WebIDL rules and produces callback errors in the
filter realm. Wrapper tracing retains the root/current node/filter and weak
finalizers release native records.

The [terminal regression gates](CONFORMANCE-TRAVERSAL-NATIVE-2026-09-09.json)
pass 401 native tests (5 ignored), 193 binding tests (2 ignored), 516 workspace
tests (6 ignored), generated bindings, formatting and both strict Clippy gates.
The production worker builds, signs, verifies and passes its mutation smoke test.
These focused gains await a later full-inventory measurement.

The five remaining traversal executions are explicit follow-ups:

- NodeIterator.html and both removal fixtures: implement live reference/candidate
  positions and native pre-removal retargeting, including removal during filtering.
- The detached-iframe callback fixture (despite its TreeWalker filename) creates a
  NodeIterator: verify lazy callback invocation and detached-realm behavior.
- TreeWalker-realm.html reads contentDocument immediately after dynamic insertion:
  implement the initial iframe document lifecycle, then verify foreign-node
  identity for both callback arguments and returned nodes.

Keep these separate from the larger Stylo resource/ownership work and the existing
Range command-limit observation gaps. None of the five is included among the
proven eleven gains, and the full signed baseline remains 3,144/4,022.

## Original assessment at `9abe008` (historical)

The last complete signed inventory measurement is **3,109/4,022 (77.3%)** at
`9faec9a`, with **913 failing executions**. That assessment predated the full run above. A signed Git commit is not a signed-worker
conformance result.

| Area | Passing / required | Remaining |
|---|---:|---:|
| Editing | 416 / 704 | 288 |
| DOM | 415 / 662 | 247 |
| CSS, all three suites | 360 / 564 | 204 |
| History / Location | 45 / 136 | 91 |
| IndexedDB | 416 / 456 | 40 |
| WebIDL | 31 / 58 | 27 |
| Referrer policy | 1,383 / 1,391 | 8 |
| Accessible Name and ARIA | 43 / 51 | 8 |
| Total | 3,109 / 4,022 | 913 |

DOM, editing and CSS contain **739 failures, 80.9% of the remaining inventory**.
These are opportunity sizes, not forecasts of gains from a single implementation.
The backlog has 749 assertion, 136 harness, 25 runtime and three incomplete
outcomes. None of its 913 entries has a root cause confirmed for this ledger;
older diagnoses exist but have not been revalidated against it.

Since the full run:

- Cursor commit `9517a08` has four exact signed execution gains, repeated with
  one and four jobs under unchanged limits. They still need a full-run promotion.
- Paragraph commit `e9d3fe0` has a native 704-case audit with 429 passes, 13 gains
  and no complete-execution losses. The same audit reports **102 regressed
  assertions and 455 missing assertion names**. These are unresolved audit
  findings, not acceptable regression-free progress. Reconcile renamed/dynamic
  names and absent observations before interpreting the raw missing count.
- Validation commit `1ce90c0` has focused blob-worker, timer and XHR test passes.
  CSP commit `9abe008` was committed without a post-change focused test result.
  No terminal, successful all-features workspace gate establishes HEAD as green.
- The full baseline separately retains six historical editing regressions and
  151 unobserved assertions. Do not add these to the paragraph counts: their
  overlap and identities require reconciliation.

## Why progress has felt slow

1. Work is selected from first error messages and broad subsystem sizes without
   proving what remains after that first blocker. Fixing many assertions inside
   an execution that still fails does not improve whole-execution conformance.
2. Native probes, unit tests, signed focused runs and full inventory results have
   been discussed together. Each is useful, but only the last measures the total.
3. Regression reconciliation has lagged implementation. Thirteen editing gains
   cannot be treated as an accepted batch while 102 assertion regressions remain.
4. Validation orchestration caused unnecessary restarts. Previous tool calls
   printed only command output and discarded returned process/session IDs. A
   completed orchestration cell does not establish that its shell process ended.
   Focused-pass/full-run-fail evidence alone also does not prove V8 state leakage;
   timing, concurrent runs, feature combinations and resource pressure need controls.
5. Bootstrap and generated implementations can diverge. The recent blob-worker
   fix adds a JavaScript text registry alongside Rust-owned blob URLs. Worker
   success alone does not establish shared fetch/XHR/revocation/binary semantics.
   This is a specific integration risk to test, not a reason for a wholesale rewrite.

## Prioritized execution queue

### 0. Establish trustworthy current evidence and repair regressions

Keep one build/test lane, capture the full tool result (including session ID and
exit code), and poll the same process to terminal completion. Save logs and
source/artifact identity. Do not restart a command because its output yielded.

The foundation checks, full regression gates, lossless reporting and paragraph
regression repair have completed as recorded above. The production Blob URL audit
covers workers, fetch/XHR, revocation and binary data. All 1,898 native editing
matrix cases remain; signed inventory execution uses unchanged production limits.

Keep the remaining observation debt explicit: 126 currently unobserved assertions,
six historical editing regressions and 170 historical ambiguous groups. Profile
the Range fixtures that exhaust the command budget, then repair the measured
runtime bottleneck. Preserve fixtures, assertions and limits.

The latest complete ledger is `011d470`; the `3eee013` traversal batch has signed
focused proof. A subsequent full run must measure its overall impact and regenerate
the backlog from that single ledger. Do not add focused gains to the total.

**Exit:** terminal validation results and a complete current ledger; no unresolved
new regressions accepted as progress. Investigate a reproducible validation blocker
directly rather than spending repeated cycles on unfounded process-leak hypotheses.

### 1. Close bounded DOM API gaps with complete-fixture evidence

Four DOMTokenList executions now pass through signed full measurement. The
attribute-coverage fixture retains eight failures across related element token-list
properties; treat those as a separate bounded follow-up. Live TreeWalker traversal
now has eleven signed focused gains. Close NodeIterator's live reference/candidate
positions and removal retargeting next, then verify the remaining iframe-document
fixture through the shared frame lifecycle. Screen WebIDL and IndexedDB
constructor/default-value failures with the same complete-fixture discipline.

**Exit:** selected executions complete with all assertions observed, plus passing
DOM mutation and selector-invalidation coverage. Rerank if later blockers prevent
complete fixtures.

Traversal screening identified 16 failing dom/traversal executions in the current
full ledger. The signed TreeWalker batch above now completes eleven of them;
NodeIterator and initial-frame document behavior are the remaining follow-ups.
The previous implementation eagerly filtered snapshots, omitted filter getters and restricted
TreeWalker currentNode/child/sibling movement incorrectly. The TreeWalker portion
is repaired and measured. Use all traversal fixtures, including large matrices,
to validate subsequent NodeIterator and frame changes; preserve every assertion
and callback exception before promoting any further whole-execution gains.

### 2. Investigate frame, popup and history lifecycle as the main shared candidate

The current history pool has 89 failures. Related symptoms occur in DOM frame tests,
IndexedDB origin isolation and referrer inheritance. Reproduce complete fixtures
for popup creation, same-document navigation, frame replacement, retained Window
identity, posted messages and history traversal. Trace each event/default action
from producer through broker to consumer before changing dispatch.

Official wptserve substitution and parser-frame history registration now close
fixtures 009 and 010, with full signed confirmation and unchanged referrer results.
The next lifecycle screening should start with immediate initial iframe documents,
popup/context creation and traversal, while preserving stable Window identity.
No common cause is established for all remaining history failures.

**Exit:** a recorded dependency map linking exact IDs to reproduced causes and
complete-fixture gains across the affected families. If representative fixtures
reveal unrelated causes, split the work instead of treating all 89 as one project.

### 3. Finish CSS integration around Stylo

Stylo already supplies CSS parsing and cascade. Prioritize missing integration:
stylesheet import loading and ownership, load completion, CSSOM liveness, selector
invalidation, then computed/used values and geometry. There are 83 CSSOM, 77
selector and 36 cascade failures. Twenty-seven selector failures are in the
invalidation directory; that count does not establish a shared cause.

Source inspection narrows the first integration work: `load_parser_stylesheet`
currently flattens imports without retaining their conditions, while parser and
dynamic stylesheet completion append synthetic style elements instead of publishing
into the original owner. The next CSS batch should preserve the import resource
graph, resolved URLs, owner identity and load completion through the existing
Stylo/CSSOM path, then measure full import/layer/scope fixtures. Do not infer a
whole-fixture gain from resolving only a missing load callback.

Use full fixtures for import/layer/scope behavior to validate resource loading
through Stylo and native CSSOM. Implement font readiness only when a reproduced
fixture proves its completion path; a permanently resolved promise is not loading.
Avoid adding independent JavaScript CSS behavior or replacing Stylo.

**Exit:** complete affected CSS fixtures plus full CSS regression coverage,
including movement/reconnection, live rule identity and layout consumers.

### 4. Resume editing in small, measurable semantic groups

Editing has the largest remaining pool (271). The earlier paragraph assertion
regressions are repaired; six historical editing regressions remain. Split further
work into paragraph/line-break behavior, plaintext-only
whitespace, selection/range preservation, formatting commands, and undo/input
events. Finish one group at a time under production limits. A large matrix is a
regression tool, not a reason to change resource limits or accept absent assertions.

**Exit:** full 704-case audit per editing batch, positive whole-execution gains,
zero new assertion regressions and reconciled observation coverage. Use shared
range/selection fixes only where the affected DOM fixtures also validate them.

### 5. Implement expensive missing capabilities and close the remainder

Screen scrolling/animation, state-preserving moves, geometry, EditContext and the
remaining accessibility/IDL/storage cases. The baseline includes 40 DOM scrolling
failures and 32 moveBefore failures; fourteen DOM first failures mention missing
`document.body.animate`. Animation, rendering and lifecycle prerequisites mean
these cannot be counted as cheap API-stub wins. Implement the required end-to-end
capability when the diagnosis shows it has better completion yield than alternatives.

Finish with 4,022/4,022 twice on the same frozen signed candidate, independently
recomputed evidence, no absent required observations, and all specified validation,
profile and security gates. Broader web-platform conformance needs a larger inventory.

## Rules for selecting and accepting each batch

- Maintain one implementation batch and the next diagnosed candidate. Rank by
  demonstrated complete-execution opportunities, confidence, effort, regression
  exposure and required validation cost. Do not assign numeric gain forecasts
  before examining every blocker in representative fixtures.
- Before coding, record exact IDs, the shared cause, production code path, later
  blockers, baseline assertions, passing neighbors and complete-fixture acceptance.
- After two unsuccessful implementation/validation cycles, revisit the hypothesis
  and compare the next candidate. Do not continue an open-ended assertion campaign.
- Track net whole-execution gains per engineering day alongside assertion losses,
  missing observations, crashes and resource failures. Build health is a separate
  mandatory gate, not a conformance-score increment.
- Promote compatible, regression-clean batches into a full signed run. Report the
  result before starting another broad subsystem change. Preserve all failing
  attempts when investigating intermittency.

## Evidence used

- `CONFORMANCE-IDB-FULL-2026-09-09.json` and its complete raw ledger at
  `artifacts/upstream-conformance/conformance-worker-full-dom-boundaries-9faec9a.json`.
- `CONFORMANCE-FAILURE-BACKLOG-2026-09-06.json` (regenerated from the September 9
  ledger despite its filename), and `CONFORMANCE-DIAGNOSES.json`.
- `CONFORMANCE-CURSOR-STEP-SIGNED-2026-09-09.json`.
- Commit `e9d3fe0` and local `/private/tmp/oryn-paragraph-v5-editing.audit.json`.
  Preserve this temporary audit and its raw evidence durably during phase 0.
- Commits `1ce90c0`, `9abe008`, the focused test results in this task, and current
  `crates/oryn-native/src/rust_executable.rs` / generated V8 bindings.
