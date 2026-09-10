# Conformance impact roadmap — 2026-09-09

Assessment of source `9abe008`. This roadmap supersedes the immediate queue in
`COMPLETE-CONFORMANCE-PLAN-2026-09-06.md`; its completion criteria still apply.
This is a planning assessment, not a fresh test or conformance qualification.

## Implementation in progress

The [completed native slice rerun](CONFORMANCE-ROADMAP-NATIVE-2026-09-09.json) passes
**437/709 executions**: 433/704 editing and 4/5 DOMTokenList. Relative to the last
signed baseline this is 21 whole-execution gains and zero losses; it includes gains
from the previous paragraph work. All 106 uniquely matched paragraph assertion
regressions are repaired. Against the prior raw native paragraph audit there are
152 corrected assertions, zero regressions, zero missing names and no duplicate-name
ambiguities. Seven existing incomplete/harness observations persist. Signed
qualification is pending; the full signed inventory total remains 3,109/4,022.

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
Current lossless publication prevents that corruption in future evidence.

## What is established

The last complete signed inventory measurement is **3,109/4,022 (77.3%)** at
`9faec9a`, with **913 failing executions**. Current HEAD has not been measured
against the complete inventory. A signed Git commit is not a signed-worker
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

Validate CSP once, audit the changed blob registry across worker/fetch/XHR,
revocation and binary data, and check timer ordering through the public page path.
Test-local 60-second in-process editing matrices do not demonstrate compliance
with the production 10-second command limit. Keep a production-path counterpart.
Reproduce remaining navigation, stale-reference and duplicate-popup failures.
Finish the default/all-feature workspace gates with the canonical pinned runtime.

Reconcile the paragraph batch's 102 regressions and 455 missing names against
exact fixture/assertion identities. Fix or revise the offending implementation;
preserve the inventory, assertions and production limits. Recheck the six older
editing regressions and 151 missing observations separately.

Then freeze one candidate and run all 4,022 signed executions, updating the backlog
and status from that single ledger. Publish gains, losses, assertion regressions,
missing observations and failure domains together. Do not claim 3,126 passes by
adding four cursor gains and thirteen native editing gains to the old total.

**Exit:** terminal validation results and a complete current ledger; no unresolved
new regressions accepted as progress. Investigate a reproducible validation blocker
directly rather than spending repeated cycles on unfounded process-leak hypotheses.

### 1. Close bounded DOM API gaps with complete-fixture evidence

Start with the five failing `dom/lists/DOMTokenList-*` executions: iterable behavior,
attribute coverage, iteration, stringification and value/length reflection. These
share a concrete API and are a better bounded candidate than broad editing work.
Verify native identity, liveness, token validation and mutation-driven style updates.
Screen remaining WebIDL and IndexedDB constructor/default-value failures similarly.

**Exit:** selected executions complete with all assertions observed, plus passing
DOM mutation and selector-invalidation coverage. Five is the DOMTokenList screening
pool, not a promised gain; rerank if later blockers prevent complete fixtures.

### 2. Investigate frame, popup and history lifecycle as the main shared candidate

The history pool has 91 failures. Related symptoms occur in DOM frame tests,
IndexedDB origin isolation and referrer inheritance. Reproduce complete fixtures
for popup creation, same-document navigation, frame replacement, retained Window
identity, posted messages and history traversal. Trace each event/default action
from producer through broker to consumer before changing dispatch.

Check test infrastructure first where the ledger reports literal
`{{headers[referer]}}` responses in history fixtures 009 and 010: verify canonical
WPT server template processing and resource routing. Do not implement browser
workarounds for malformed fixture responses. No common cause is yet established.

**Exit:** a recorded dependency map linking exact IDs to reproduced causes and
complete-fixture gains across the affected families. If representative fixtures
reveal unrelated causes, split the work instead of treating all 91 as one project.

### 3. Finish CSS integration around Stylo

Stylo already supplies CSS parsing and cascade. Prioritize missing integration:
stylesheet import loading and ownership, load completion, CSSOM liveness, selector
invalidation, then computed/used values and geometry. There are 85 CSSOM, 80
selector and 39 cascade failures. Twenty-seven selector failures are in the
invalidation directory; that count does not establish a shared cause.

Use full fixtures for import/layer/scope behavior to validate resource loading
through Stylo and native CSSOM. Implement font readiness only when a reproduced
fixture proves its completion path; a permanently resolved promise is not loading.
Avoid adding independent JavaScript CSS behavior or replacing Stylo.

**Exit:** complete affected CSS fixtures plus full CSS regression coverage,
including movement/reconnection, live rule identity and layout consumers.

### 4. Resume editing in small, measurable semantic groups

Editing has the largest pool (288), but its latest batch has substantial regression
debt. After phase 0, split work into paragraph/line-break behavior, plaintext-only
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
