# Conformance implementation progress — 2026-09-05

Conformance is the current completion priority. It is **not complete**.

## Complete production-worker baseline

The first development candidate finished all **4,022** frozen v2 executions:
**2,261 passed; 1,761 failed**. Failures comprise 1,371 assertion failures,
241 runtime failures, 129 harness failures, 19 incomplete results, and one
external timeout. Runtime failures include enforced worker wall-time/RSS limits;
these must not be reclassified as successful long tests.

| Root | Passed | Failed | Required |
|---|---:|---:|---:|
| IndexedDB | 299 | 157 | 456 |
| WebIDL | 26 | 32 | 58 |
| accname | 13 | 6 | 19 |
| css | 113 | 451 | 564 |
| dom | 269 | 393 | 662 |
| editing | 110 | 594 | 704 |
| html | 30 | 106 | 136 |
| referrer-policy | 1373 | 18 | 1391 |
| wai-aria | 28 | 4 | 32 |

The worker is signed, App Sandbox enabled, and V8 heap sandbox enabled.
Its SHA-256 is `b5272ce7288a67eb4f74cc5cb191019590546c18946591f522d0a2626b94ce47`.
The local raw ledger is
`artifacts/upstream-conformance/conformance-worker-full-current.json`.
It records the parent executable, worker metadata, source, fixture, and inventory
fingerprints. This is development evidence from a dirty worktree, not a clean
release attestation. The earlier historical 2,267/4,022 result was in-process;
it cannot be used as an equivalent production-worker baseline. Comparing IDs
shows 21 newly passing and 27 newly failing outcomes, which require diagnosis
across the changed execution mode and stricter completion accounting.

## Later focused fixes

A separate later signed worker verified:

- **28/28 IndexedDB bulk-read cases**: generated IDBRecord/getAllRecords,
  options, direction, count, and structured-clone multi-entry index extraction.
- **15/19 Accessible Name cases**: descendant labels, embedded-control values,
  and shadow-slot assignment/fallback. Four generated-content cases remain.
- Two reproduced referrer inheritance crashes were fixed in the earlier
  candidate by limiting the Stylo mirror to style resolution rather than
  running duplicate Blitz layout.

The bulk-read and Accessible Name results are retained in
`conformance-worker-idb-bulk-verified.json` and
`conformance-worker-accname-verified.json` under the same artifact directory.
These focused results do not amend the earlier full run or create a merged
candidate total.

A subsequent scheduler-quantum experiment still failed all four interleaved
cursor cases against worker wall-time limits and was removed from the source.
Its separate diagnostic ledger is
`conformance-worker-idb-scheduler-existing-parent.json`; it used the existing
parent with the experimental signed worker and does not qualify the final
source. A rebuilt parent also experienced a local pre-application startup stall
at `_dyld_start`, independently observed with a process sample.

## What remains, in execution order

1. Diagnose the 19 incomplete results and runtime-limit failures with completion
   and task traces; preserve real completion requirements and existing limits.
2. Run the full IndexedDB and accessibility roots on the fixed candidate to
   detect regressions beyond the focused slices.
3. Connect Stylo pseudo-element computed data and resolved generated content,
   counters, and list markers to accessible names, including mutation
   invalidation. Stylo already owns CSS parsing/cascade/computed values;
   this is an Oryn integration gap, not a reason to build another CSS engine.
4. Cluster DOM/WebIDL failures around realm/ownerDocument and wrapper identity,
   missing APIs, descriptor behavior, and lifecycle ordering. Fix and rerun each
   complete affected root.
5. Close editing command return/state/selection and mutation behavior, with full
   open-ended partition coverage. Triage CSSOM, stylesheet adoption/invalidation,
   and selector/value export failures separately from genuine Stylo defects.
6. Run one unified 4,022-case candidate, then the platform/profile, semantic
   differential, security, performance, and product gates in the action plan.
   Only validator-derived success on exact final artifacts closes conformance.

The detailed dependencies and acceptance criteria remain in
[RUNTIME-COMPLETION-ACTION-PLAN.md](RUNTIME-COMPLETION-ACTION-PLAN.md).

## Validation of this batch

- Runner/validator script tests: **32 passed**.
- Layout unit tests: **22 passed**.
- V8 binding tests: **85 passed**, including the separately enabled pinned
  upstream fixture test.
- `cargo check -p oryn-js --features v8-host`, formatting, WebIDL regeneration
  check, v2 inventory validation, runtime manifest validation, documentation
  truth check, and `git diff --check` passed.
- The final semantic/storage native unit-test run did **not** finish. Both a
  debug attempt and a release-cache attempt encountered local startup stalls;
  a sample of the latter's build script shows `_dyld_start` before application
  code. Those attempts were stopped. The native production code compiled and
  its changed behavior was exercised by the signed-worker upstream slices,
  but the added native unit tests remain to be run successfully.

No conformance or release-completion gate is marked passed by this report.

## Resumed verification

The startup stalls cleared on the resumed run. The native slot test's invalid
DOM mutation API was corrected, and the stale `presentation` expectation was
updated to the existing canonical role `none`. Semantic and storage suites each
pass 14 tests. The full native suite also exposed a stale one-node delta
expectation: embedded input values can change the containing label. Its
replacement checks that reported upserts are actual changed observations.

The rebuilt signed worker (`aee6cae4eafdb575988d2abcf8ca9988ddbe00a4a3f16eb1cc042e35cbeef3a4`)
finished the entire IndexedDB root at **323/456**, with **24 new passes and no
regressions** relative to the 299/456 production baseline. Accessible Name
remains **15/19**. The exact root ledgers are
`conformance-worker-idb-root-current.json` and
`conformance-worker-accname-root-current.json` in the local artifact directory.
Further DOMImplementation and IndexedDB exception/event fixes are under
validation; these are not included in the preceding worker totals.

The next exact worker (`3eb3ba92d366ca9b99519df4cf88a8ee53d103a5e353b34d7fdd2a4089bb545a`)
finished IndexedDB at **329/456**: six further passes, no regressions against
323/456. Both globals now pass failed-open event flags, finished-transaction
index access, and deleteIndex exception ordering. Its complete root ledger is
`conformance-worker-idb-events-current.json`. The same worker passes two of five
DOMImplementation cases (`createDocumentType` and `hasFeature`), retained in
`conformance-worker-domimplementation-current.json`.

The resumed full native suite passed **137/137** before the next style-tree
traversal change. Subsequent HTML document metadata, inherited unforgeable event
accessors, and Stylo mirror fixes are under validation and are not included in
those upstream totals. The style-tree review found that the old snapshot used
arena allocation order and included detached trees/stylesheets; the replacement
walks the current document in tree order and derives child lists in one pass.

## Document, event, and style-tree verification

Worker `6381803d680ca93f6ad5b36b10ecfd505e0ee0a099e8c4d7d8afab746c7290a7`
completed the DOM root at **277/662**, eight new passes with no regressions
against the first production baseline. This verifies inherited unforgeable
`Event.isTrusted` accessors, AbortSignal event trust, document-scoped
implementations, `hasFeature`, doctype creation, and HTML document creation.
The saved-iframe implementation case still fails because its child document
is unavailable; this is distinct from implementation-wrapper lifetime.

The CSS root is **112/564**. Tree-order stylesheet collection fixes
`preferred-stylesheet-reversed-order.html`. Two former passes now fail:
`layer-replaceSync-clears-stale.html` and
`CSSStyleSheet-constructable-cssRules.html`. Previously every detached style
node applied globally, including constructed sheets that were never adopted.
Excluding detached styles exposes the missing `adoptedStyleSheets` integration.
Close this with the specified observable-array adoption/lifetime/invalidation
behavior, not by restoring application of all arena styles.

Root ledgers: `conformance-worker-dom-root-next.json` and
`conformance-worker-css-root-next.json`. The runner's substring filters also
select two shadow-DOM accessibility cases and five HTML CSS cases respectively;
the root totals above are computed by exact path prefix, not the broader filter
aggregate (279/664 and 117/569).

The updated layout suite passes **23/23**, native suite **138/138**, and the
subsequent object-store rename binding suite **86 enabled tests**, with one
fixture-gated test excluded from that invocation. The additional native rename
metadata/rollback test passes separately. Rename changes are not included in
the preceding worker's upstream totals.

### Stylo ownership and remaining selector work

Stylo 0.20.0 remains the CSS parser/cascade/computed-value owner. Inspection of
its pinned Servo selector parser confirms `parse_has()` and
`parse_nth_child_of()` both return false. Enabling these requires a reproducible
dependency change and validation of matching/invalidation support. Oryn DOM
query methods also validate with Stylo but execute a separate handwritten
matcher. Adapt Stylo's DOM-query algorithms to the canonical DOM, preserving
scope, namespaces, detached trees, shadow boundaries, and mutation order.
Do not replace Stylo with another CSS engine or silently patch a registry cache.

## Object-store renaming verified

Worker `d1c8ac959bb5996169fd3c9d288322dc2dc7b5becc1b034734673105ee6809e1`
completed all **456 IndexedDB variants at 337 passes**. Against the preceding
329/456 root, there are eight new passes and no regressions: both globals of
normal store renaming, rename errors, rename aborts, and transaction-abort
object-store metadata restoration. The exact ledger is
`conformance-worker-idb-rename-root.json`. This is 38 more passes than the first
production root (299/456), and still leaves 119 non-passes.

The implementation uses a typed parent storage command, preserves records and
indexes while moving schema names, returns the same store handle within a
transaction, restores existing names on abort, preserves the last name of a
newly created aborted store, restores transaction store lists, and clears
indexes on aborted new stores. No probe shortcuts or relaxed time limits were
used. Signed-worker startup and sandbox-denial checks pass.

Final native unit verification is **139/139** (including all 15 storage tests).
V8 binding verification is **86/86 enabled tests**; layout is **23/23** and
conformance tooling is **32/32**. The 4,022-ID inventory validates, generated
bindings match their IDL, formatting and diff checks pass, and documentation
truth validation passes. Full conformance remains open; no unified new
4,022-case result is inferred from the separate root runs.

Workspace Clippy (`cargo clippy --workspace --locked`) did not finish. Several
compiler processes remained idle without diagnostics; retrying outside the
restricted execution environment did not resolve the stall. Both attempts
were stopped. A diagnostic sample also stalled during symbol processing, so
this report does not infer a specific startup stack or blame runtime code.
Clippy remains an unverified check, independent of the completed test results.

## Stylesheet adoption and Stylo DOM queries verified

Worker `c10aea730e684d7b47200889666652ee680c658e2255575ee4a4ca48db9a2616`
passes **116/564 CSS** (121/569 for the broader substring selection), gaining
four cases without regressions against the preceding 112/564 candidate.
This includes both previously exposed constructed-sheet regressions. Ledger:
`conformance-worker-css-adoption-root.json`.

Document and ShadowRoot `adoptedStyleSheets` are generated ObservableArray
attributes backed by canonical Rust DOM lists. The binding implements iteration,
indexed mutation, fixed descriptors, no holes, iterator closure, per-document
sheet brands, exception propagation, and a null-prototype proxy handler. Adopted
sheets are retained as native roots, not DOM children or synthetic attributes.
Stylesheet-list changes invalidate style without creating MutationObserver
attribute records. Constructed sheet identity and adoption survive collection
and recreation of a shadow root's JavaScript wrapper. Document style application
uses the canonical list; shadow style isolation/application is still incomplete.

Worker `cce34284e091759ea09becb4b5fe424cc3be100cf8ba7ab39eaf6a6f4fff8aee`
then verifies the Stylo DOM-query adapter:

| Root | Exact passes | Required | Comparison |
|---|---:|---:|---|
| DOM | 283 | 662 | +6, no regressions versus 277 |
| CSS | 135 | 564 | +19, no regressions versus adoption candidate 116 |
| WebIDL | 28 | 58 | +2 ObservableArray cases, no regressions versus 26 |

Ledgers are `conformance-worker-dom-selector-root.json`,
`conformance-worker-css-selector-root.json`, and
`conformance-worker-webidl-selector-root.json`. Substring aggregates for DOM and
CSS are 285/664 and 140/569; the table uses exact root prefixes. An earlier
lowercase `webidl/` invocation selected zero cases and supplies no test evidence.

Production querySelector/querySelectorAll/matches/closest now borrow the
canonical DOM and use Stylo's selector parser and matching algorithms, preserving
tree order, scope, namespace checks, detached trees, and shadow query boundaries.
CSSOM selector validation also parses the selector directly instead of accepting
the first rule from an interpolated stylesheet. The adapter does not enable
`:has()` or `:nth-child(... of ...)` in the pinned Servo parser, and does not claim
complete dynamic pseudo-class or XML-document state support.

Source provenance now includes untracked file paths and contents, so new Rust
modules cannot disappear from a dirty-worktree fingerprint. No required IDs,
completion requirements, or resource limits were relaxed.

Following this candidate, local WebIDL fixes reject Proxy receivers without
unwrapping their targets, preserve exceptions thrown through PutForwards setters,
and generate NodeFilter as a callback interface without a prototype property.
URLSearchParams iterator descriptors were corrected as well. These later changes
are not included in the preceding worker's root results.

Validation after these changes: **90 enabled V8 tests passed**, one fixture-gated
test excluded; **25 layout tests passed**; **33 tooling tests passed**; generated
bindings and diff checks passed. Strict layout Clippy passes. A new workspace
Clippy and native-suite run are in progress and are not yet claimed as passing.
Full conformance remains incomplete.

## Layout flush reuse under verification

Repeated synchronous style and geometry reads rebuilt the complete layout even
when its inputs were unchanged. Providers can now opt into reuse with an explicit
revision. The production provider uses the non-destructive DOM mutation cursor;
its viewport is immutable until the provider is replaced. The binding also
compares custom states exactly, invalidates on explicit layout/provider updates,
and leaves providers without a revision uncached. Failed calculations are not
cached. The 100-read regression test confirms one calculation, with fresh
calculations after style, focus, custom-state, and stylesheet-adoption changes.
This is a candidate optimization for the observed RSS failures, not yet an
upstream memory-limit fix claim.

The updated V8 suite passes **91 enabled tests**, and the pinned range-bootstrap
test passes separately against the same binary: **92 total**. Workspace Clippy
completed successfully with `--workspace --locked --offline -j 1`, retaining two
native dead-code warnings and one storage `ok_or_else` suggestion. Its preceding
parallel run was terminated after an idle layout compiler remained for over
23 minutes. The native suite also completed at **139/139**. These checks do not
replace the signed-worker result required for the cache candidate.

## Complete adoption/selector candidate and memory investigation

`conformance-worker-full-adoption-selectors.json` completed the exact 4,022-ID
inventory at **2,349 passed / 1,673 failed**. Its signed worker SHA-256 is
`a5363d7ccd7430d8423b597fddfdc365a3a9346ab857f9042e6753eb5715bd6c`.
The companion summary verifies no missing/duplicate IDs and real testharness
completion for every pass. This is dirty-tree diagnostic evidence, not release
attestation. Against the initial signed-worker baseline: 89 new passes, one
regression (`keeping-attributes-at-joining-elements`, backspace, RSS limit).

| Root | Passed | Total |
| --- | ---: | ---: |
| IndexedDB | 337 | 456 |
| WebIDL | 29 | 58 |
| Accessible Name | 15 | 19 |
| CSS | 135 | 564 |
| DOM | 283 | 662 |
| Editing | 109 | 704 |
| HTML | 30 | 136 |
| Referrer policy | 1383 | 1391 |
| WAI-ARIA | 28 | 32 |

Failures: 1,291 assertion, 240 runtime, 130 harness, 11 incomplete, one timeout.
Of the runtime failures, 186 report RSS limits. The later cache worker
`d53c2f4c23b93be8d46b24b159af2e617be5bf7060bdcc8ec593bb8e87092bf7`
still fails both joining-attributes variants with RSS limits and passes none
of the 60 joining-whitespace variants; no memory conformance gain is claimed.
A Darwin memory sample of the regression found 186.1 MB allocated and 161.6 MB
allocator fragmentation/retention. Reclaiming unused allocator pages at event
checkpoints is being evaluated without changing the 512 MiB RSS limit.

### Parser allocation fix under verification

Allocator reclamation alone passed one joining variant but left the backspace
regression RSS-limited; that experiment was removed. The parser instead now
batches adjacent html5ever character tokens before publishing text to the DOM.
It flushes before structural changes, script pause/return, and external feed
return, preserving script-visible ordering and observer delivery. Previously
each tiny raw-text append copied the growing script into the mutation journal,
causing quadratic retained text. Streamed appends also now preserve existing
Range offsets instead of replacing the entire text node. All 23 DOM/parser
tests pass, including large raw-script memory growth and Range regressions.
The signed-worker result for this change remains pending.

### Verified parser regression and next CSS candidate

The signed parser-batching worker
`7c7f74b2c5661e68162b06713d3f3d156ec1def45fd4ead1ce291fb89b481b0b`
passes both `keeping-attributes-at-joining-elements` variants under the original
limits (`conformance-worker-parser-joining.json`). All 139 native runtime tests
and strict DOM Clippy pass. The complete parser inventory is running separately
in `conformance-worker-full-parser-batching.json`.

The following changes postdate that candidate and must not be credited to it:

- Generated HTMLElement `offsetWidth`/`offsetHeight` use untransformed layout
  dimensions, integer rounding, and zero for detached/display-none elements.
  Visibility-hidden boxes retain dimensions and take space in flow. Existing
  unsupported geometry remains explicit. The layout's broader fragment/table
  and visibility geometry limitations are not closed by these accessors.
- `vendor/stylo/ORYN-PATCHES.md` documents the exact 0.20.0 source archive and
  two parser feature switches enabling existing Stylo `:has()` and filtered
  `:nth-child()` support. Cargo patches both direct DOM queries and Blitz's
  stylesheet traversal to this same source. The old descendant-state shortcut
  was removed: Stylo now evaluates relational custom-state selectors itself.
- All 28 layout/query tests pass, including relational sibling/child matching,
  invalid nested `:has()`, filtered positions, mutation recascade, and offset
  dimensions. All 91 enabled V8 binding tests pass with the dimension accessors.
  Full signed-worker CSS/DOM confirmation for these changes remains pending.

### Event clock and Blob transfer candidate

The parser run isolates four remaining RSS cases: two composite-Blob executions,
`Event-timestamp-safe-resolution`, and `Range-insertNode`. Two further shared
causes have now been fixed locally:

- Event.timeStamp was the constant `1.0`, causing a test that waits for a new
  timestamp to allocate events indefinitely. Events now retain their creation
  timestamp on a realm-owned monotonic clock, coarsened to 100 microseconds,
  and performance.now uses that same clock and origin.
- IndexedDB Blob/File clone payloads now use base64 strings instead of one JSON
  numeric value per byte. Old persisted byte arrays remain readable. Byte
  accuracy, compact payload size, and File name/type/lastModified round trips
  are covered by a regression test. This does not change IPC or memory limits.

All 93 enabled V8 tests pass after these changes; the one pinned Range fixture
requires its separate environment-enabled invocation. Signed-worker checks are
pending. Strict layout Clippy passed after the offset and Stylo patch changes.

## Complete parser-batching candidate

`conformance-worker-full-parser-batching.json` finishes at **2,363/4,022**,
with every required ID and passing harness completion verified by its summary.
Compared with the preceding adoption/selector full run: 15 new passes, one
regression, net +14. Root totals are IndexedDB 337/456, WebIDL 29/58, Accessible
Name 15/19, CSS 135/564, DOM 289/662, editing 118/704, HTML 30/136, referrer
policy 1,382/1,391, WAI-ARIA 28/32. Failures: 1,460 assertion, 56 runtime,
131 harness, 10 incomplete, two timeout. Three primary runtime failures report
RSS; a fourth execution reports platform-operation wall time and its auxiliary
DOM probe reports RSS. This distinction supersedes the interim four-RSS count.

The one regression, `unsupported-csp-referrer-directive.html`, reports a parent
security snapshot mismatch: the parent has already observed a meta policy,
while the worker is confirming only the response headers. The header/meta
ordering race is under correction; policy enforcement must remain parent-owned.
The original joining-attributes editing regression is restored.

This candidate includes the earlier layout cache and parser batching. It does
not include later offset dimensions, patched Stylo selector switches, event
clock, or compact Blob changes. All raw historical ledgers remain unchanged.

### Combined candidate checks (2026-09-06)

The signed worker `6db53510926174ba441f5d112b9bc63e36d34fc0709a996f51eba4c7c37d24f4`
includes the Stylo feature switches, offset dimensions, monotonic event clock,
and compact IndexedDB Blob encoding. Both globals in
`conformance-worker-blob-compact.json` pass (2/2). The timestamp ledger
`conformance-worker-event-clock.json` passes 2/4 files: cross-realm timestamps
and safe resolution. MouseEvent and WheelEvent high-resolution assertions also
pass; the remaining files fail because KeyboardEvent, FocusEvent, and
GamepadEvent constructors are absent. The formerly memory-exhausting safe
resolution case completes normally. These targeted results are not a new full
inventory total. The pinned ignored V8 bootstrap test was separately run and
passed, in addition to 93 enabled V8 tests.

The response CSP confirmation now compares header policies independently of
meta policies already installed by the parent. It retains reporting endpoint
and origin verification, and leaves parent meta enforcement and ordered worker
meta confirmation intact. Regression tests cover forged, missing, report-only,
and different-origin policies plus continued meta enforcement. Production
verification of this change requires the next signed candidate.

The complete `css/` filtered run passes 222/569 executions, compared with
140/569 on the preceding parser candidate: 82 new selector passes and no
regressions. The filter includes five HTML-root executions; this count must not
be presented as the CSS-root denominator (564). The ledger is
`conformance-worker-stylo-enabled-css.json`. All 11 CSP unit tests pass,
including both new response-confirmation regression tests.

### CSP confirmation and template contents

Worker `f06a97301ae73c5e0e044e6c447d5874d5fec290408028296a21f4ee9a012a27`
passes signing, startup, sandbox-denial, and parent-broker checks. The specific
CSP/referrer regression passes in `conformance-worker-csp-confirmation.json`.
All 141 native unit tests pass with local fixture-server access; the initial
restricted invocation could not bind four fixture servers. All 27 WPT runner
tests pass. Its full inventory ledger is being collected separately.

A subsequent source change puts template-content identity in the canonical DOM
instead of losing html5ever's private template map before binding installation.
Parsing, arena snapshots, import, deep/shallow cloning, serialization, and native
lifetime roots retain the inert fragment without making it a light-DOM child.
Closing scripts/styles inside inert contents no longer schedules them as document
resources. All 25 DOM/parser tests pass, including nested content, import,
collection, serialization, and inert script/style tests. The extended parsed
and nested template binding test passes with all 93 enabled V8 tests. This later
change is not included in worker `f06a973…` or credited to its full ledger.

After removing both private template maps, all 25 DOM/parser tests and strict
DOM Clippy (`-D warnings`) pass. Workspace Clippy passes with three existing
warnings (unused observable/accessibility helpers and a storage `ok_or_else`).
Generated WebIDL output, formatting, diff whitespace, and documentation truth
checks pass. Template-content owner-document semantics and broader template
conformance remain subject to the next production-worker run.

The next candidate also enables Stylo's existing `@scope` parser in Servo
builds. The pinned static preference disabled this grammar even though upstream
scope matching was present. The repository patch preserves the upstream scope
algorithms and Gecko preference. All 29 layout/query tests pass, including
explicit scope roots, end boundaries, outside elements, and recascading after
root-class mutation. Only `servo/selector_parser.rs` and
`stylesheets/rule_parser.rs` differ from the original Stylo crate source; the
patch provenance and maintenance instructions are in `vendor/stylo/ORYN-PATCHES.md`.
Implicit scope ownership and shadow integration remain separate concerns.

### Complete Stylo/clock/Blob/CSP candidate

`conformance-worker-full-stylo-clock-csp.json` completes all 4,022 required IDs
with 2,451 passes and 1,571 failures. Against the parser candidate it adds 88
passes and has no regressions. Worker SHA-256 is
`f06a97301ae73c5e0e044e6c447d5874d5fec290408028296a21f4ee9a012a27`.
The validator helpers confirmed exact ID equality, uniqueness, and real harness
completion for every reported pass. This remains failed, dirty development
evidence and cannot qualify a release.

| Root | Passed / required |
|---|---:|
| IndexedDB | 341/456 |
| WebIDL | 29/58 |
| accname | 15/19 |
| css | 217/564 |
| dom | 290/662 |
| editing | 118/704 |
| html | 30/136 |
| referrer-policy | 1383/1391 |
| wai-aria | 28/32 |

Failure domains: assertion 1377, harness 128, incomplete 10, runtime 54, timeout 2.
Primary runtime resources: RSS 1, generated platform operation wall time 12, other 21, wall time 20.

Gains comprise 82 CSS selector cases, four IndexedDB cases (composite Blob reads
and nested small cloning in both globals), event timestamp safe resolution, and
the CSP/referrer confirmation regression. The next template/`@scope` source is
not included in this ledger. Latest source verification is 25 DOM/parser,
29 layout/query, 141 native, and 94 V8 tests (including the separately enabled
pinned bootstrap test), with strict DOM/layout Clippy passing.

### Template/scope candidate and Range lifetime follow-up

Signed worker `aa36e58a88ec1d7b378cd56955845147879bbf14637dd2360482e59305cfa0ef`
passes startup and sandbox checks and includes canonical template contents plus
Stylo `@scope`. Its focused `css/css-cascade/scope-` run passes 7/30 cases, all
seven newly passing with no regressions: deep scopes, evaluation, layers, media,
overlapping `:has`, specificity, and supports. The ledger is
`conformance-worker-template-scope-targeted.json`; the full candidate ledger is
being collected separately.

The preceding full candidate has one primary RSS failure,
`dom/ranges/Range-insertNode.html`. A subsequent source change tracks Range and
StaticRange wrappers weakly, preserves the selected Range's wrapper identity,
and releases collected native range records and endpoint roots. StaticRange now
owns frozen endpoint roots. Known native record payload is reported to V8 as
external memory so its GC sees memory retained by JavaScript wrappers; limits
remain unchanged. Cleanup runs at safe checkpoints and periodically during
range allocation. A GC stress regression with 8,192 temporary ranges verifies
that selected and frozen ranges survive and later release; all 94 enabled V8
tests pass before the periodic-scan optimization, which is being rechecked.
This Range change is not included in worker `aa36e58…` and requires its own
signed-worker validation before any memory or conformance gain is claimed.

### Complete template/scope ledger and native collection follow-up

`conformance-worker-full-template-scope.json` completed all 4,022 required
executions: **2,460 passed, 1,562 failed**, nine gains and zero regressions
versus the previous full candidate. Exact inventory IDs and real passing
harness completions were checked. CSS is 224/564 and DOM is 292/662; other
root totals are unchanged. This remains dirty development evidence.

The subsequent NodeList implementation replaces arrays for childNodes,
querySelectorAll, getElementsByName, and mutation record node lists. It uses
native indexed properties, generated WebIDL iterable metadata, live child/name
views, static selector/mutation snapshots, weak collection ownership, and
external-memory accounting. Range cleanup's periodic scan passed the V8 suite.
The combined collection/Range candidate still needs signed-worker measurement.

Native collection verification passes 96 enabled V8 tests, plus the separately
enabled pinned WPT bootstrap. The live name list handles literal names (including
quotes, backslashes, and empty names); mutation record added/removed lists retain
static membership. A separate same-script MutationObserver `takeRecords()` gap
was identified and recorded in the completion plan; collection tests do not
claim that queue scheduling is fixed.

The signed collection/Range worker is
`1befc1abab8fc3a0f8bf3f40f4dc9a78012e0656d5ddbfe96c50cb1633482c5f`.
`conformance-worker-nodelist-targeted.json` passes 2/8 files: live mutations
and iterable behavior. Six static length-tampering stress files hit the
unchanged wall-time limit; these are not passes. Its full ledger is running.

Later source changes remove whole-vector copying from static indexed/length
reads, add the standard webkitMatchesSelector alias through the existing
matcher, and replace collection-owned native Global handles with weak handles
plus V8-traced internal references. The latter prevents detached node/list
cycles from becoming permanent native GC roots. The cycle regression and
length-shadowing checks pass with all 96 enabled V8 tests. These follow-ups
are not included in worker `1befc1…`.

The collection/Range full run still reports RSS failure for
`dom/ranges/Range-insertNode.html`; Range cleanup alone does not close it.
A separate native leak was found during follow-up: removal records evicted
from the bounded mutation journal lost their retained subtree leases. The
journal now records those leases for release at the normal checkpoint and
distinguishes entries already transferred into a drained batch. A regression
checks both paths; all 26 DOM/parser tests pass. This is a post-`1befc1…` source
fix, not a measured reduction in the failing test's peak RSS.

A subsequent CSS bridge fix preserves native element namespaces in LayoutNode
and the Blitz/Stylo mirror. Previously every mirrored element was constructed
in the HTML namespace, including SVG. The new test distinguishes HTML, SVG
and no-namespace elements with the same local name through Stylo stylesheet
namespace selectors; legacy serialized inputs still default to HTML. All
30 layout/query tests pass. Attribute namespace metadata and shadow cascade
ownership remain separate gaps. This change is not in worker `1befc1…`.

### Complete collection/Range candidate ledger

`conformance-worker-full-collections-ranges.json` completed 4,022 executions:
**2,464 passed, 1,558 failed**, 4 new passes and
0 regressions versus the template/scope full candidate.
The exact inventory and all passing harness completions were checked.
This remains dirty development evidence, not a release-qualified pass.


### Complete collection/namespace candidate ledger

Worker `4adfc937a4aa54212004df40d49cb0eaf3c6c778444badfe218c9ee589279990`
completed `conformance-worker-full-collections-namespace.json`: **2,465/4,022**,
with two gains and one regression versus the collection/Range candidate. The
canonical validator verified the exact unique execution set and every passing
harness completion. Gains are `css/selectors/nth-of-type-namespace.html` and
`dom/nodes/Element-matches-namespaced-elements.html`. The regression is
`IndexedDB/open-request-queue.any.js` (duplicate blocked events and out-of-order
completion). Root totals and failure domains are in the accompanying summary.
The failed aggregate and dirty source remain development-only evidence.

Focused checks of this worker confirmed the namespace type pass. The selector
alias file remains failing (634/669 subtests pass), primarily detached-document
DOMException access. SVG attribute-value matching remains failing (1/3 subtests);
the pinned Blitz selector adapter unconditionally reports HTML-element status,
a separate integration defect. Six static NodeList stress files still hit the
wall-time limit. Range-insertNode still reaches the RSS limit.

### Detached tree and wrapper lifetime follow-up (source only)

A new native regression showed that retaining a detached child did not retain
its reachable parent and siblings. Native reclamation now frees a detached
ordinary tree only when none of its nodes has a wrapper/native root. All 27
DOM/parser tests pass. A V8 regression then demonstrated loss of parent/sibling
JavaScript properties after GC. Shared, V8-traced arrays now preserve existing
detached-tree wrapper identities and explicitly assigned owner-document wrappers
without native Global cycles. The test also covers a live NodeList created
before a child is appended. All 97 enabled V8 tests and 141 native tests pass;
DOM strict Clippy, V8 Clippy, and formatting pass. Attribute, template-owned,
associated-object, and Range endpoint JavaScript tracing remain separate work.

The queue regression exposed same-database open/delete host submissions running
ahead of a blocked request. A per-database connection queue is being validated;
its gate spans blocked/upgrade tasks, while independent databases and active
upgrade operations can continue. These changes are not in worker `4adfc9…`.

The connection-queue regression and all 98 enabled V8 tests now pass, as does
the separately enabled pinned Range bootstrap test. V8 Clippy and all 27 WPT
runner tests pass. Signed-worker build/validation is pending.


### Signed GC/connection-queue candidate and CSS adapter follow-up

Worker `2784ae42c07a6f2859093528dcb996248f729a44315429880497d069081cceba`
passed signing, startup, broker, and sandbox checks with unchanged limits.
The complete IndexedDB and DOM filtered inventories are running in
`conformance-worker-gc-queue-indexeddb.json` and
`conformance-worker-gc-queue-dom.json`. Do not combine partial counts into a
full-inventory result.

After that worker was built, a stylesheet regression reproduced incorrect SVG
attribute-value case folding. A local copy of the pinned Blitz DOM package now
checks the HTML namespace in `is_html_element_in_html_document`. This is the
only upstream Rust-source change; manifest workspace entries are normalized,
sibling dependencies remain pinned, and upstream licenses are retained. All 31
layout/query tests and strict layout Clippy pass, including HTML/SVG/empty
namespace defaults and explicit `i`/`s` flags. See
`vendor/blitz-dom/ORYN-PATCHES.md` for provenance. XML-document mode and attribute
namespace metadata remain separate gaps.

The subsequent V8 optimization avoids rebuilding detached wrapper graphs merely
because another already-rooted connected node was wrapped. Existing mutation
revision invalidation and detached-node graph updates remain. All 98 enabled
V8 tests pass. Neither this optimization nor the Blitz correction is in worker
`2784ae…`; signed-worker confirmation is pending.


The signed GC/connection-queue IndexedDB root completed **341/456**, recovering
`open-request-queue.any.js` with zero regressions versus the namespace full run.
The exact filtered inventory and all passing harness completions were validated.
The DOM root is still running; this is not a new full-inventory aggregate.

A further source regression isolates iframe clone ownership in the shared native
arena. `cloneNode` had used the arena's main document rather than the source
node's owner document, causing `defaultView` to be null on the incorrectly
wrapped owner document. Owner lookup now follows native ancestry/explicit
ownership and falls back to the binding realm's document. Clones record ownership
for ordinary descendants, including children later detached from the clone;
cloned documents own their cloned descendants. All 99 enabled V8 tests pass.
This follow-up is not in worker `2784ae…`.


The signed GC/queue DOM filter finished **299/664**, comprising **297/662 DOM**
and two passing Accessible Name files selected by the substring. It has zero
gains and zero regressions versus the namespace full run. Exact filtered IDs
and passing completions were validated. The lifetime fixes have targeted unit
evidence but are not credited with an upstream file gain.

A further static NodeList optimization reads immutable length from its private
traced array before realm/registry resolution. Proxy and ordinary receivers
continue through the full brand check, and public length shadowing is unchanged.
All 99 enabled V8 tests pass. Production stress-file confirmation is pending.


### Signed CSS/clone/static-length focused results

Worker `e9a42dffcab8d647b57b22620f51e0cf8431a086016eb802e586448702fd9ad6`
passed signing, startup, broker, and sandbox checks. The SVG attribute-value
file now passes all 3 subtests. Element-webkitMatchesSelector improves from
634/669 to **668/669**, but remains a file failure: `:target` is not matched by
the native Stylo query adapter. Static NodeList stress results are **2/6 files**:
getter-tampered-1 and -3 pass; -2 has only an unverified partial completion;
all three indexOf variants hit runtime limits. Exact focused IDs and every
credited harness completion were checked. These are focused development
results, not a full aggregate. Latest validation before this build: 99 enabled
V8 tests plus the pinned Range bootstrap, 141 native tests, 31 layout/query
tests, relevant Clippy, generated binding check, formatting, and docs checks.

## Complete GC, connection queue, CSS and clone candidate

The signed worker `e9a42dffcab8d647b57b22620f51e0cf8431a086016eb802e586448702fd9ad6`
finished all 4,022 required executions: **2,467 passed, 1,555 failed**, with two
new passes and zero regressions against the namespace candidate. Exact inventory
coverage and every passing harness completion were verified. The ledger is
`artifacts/upstream-conformance/conformance-worker-full-gc-queue-css-clones.json`;
its adjacent summary records the root counts and comparison. Failure domains are
1,355 assertion, 1 timeout, 53 runtime, 133 harness and 13 incomplete. Security and
resource limits are unchanged; this is dirty-source development evidence.

New source after that worker was built adds static NodeList indexed data properties
and native HTMLCollection views, with live literal filters, named properties and
WebIDL named-getter metadata. All six pinned static NodeList stress scripts pass
in separate diagnostic realms (4.9–5.5 seconds per script); these diagnostics do
not constitute upstream harness completion. HTMLCollection's local regression
covers descriptors, prototype visibility and inherited writes, brands, mutations,
namespaces and literal class matching. Signed-worker confirmation remains pending.

## Signed HTMLCollection and static NodeList confirmation

Worker `00d58fd2e2b0cc3ece145491ac83ea20d1f6bfb4d6e61e1445e09c82dbcb7d81`
passes all eight pinned HTMLCollection files and all six static NodeList
length/indexOf stress files. Exact filtered inventory and every passing harness
completion were verified. All 14 files failed in the preceding complete full
ledger. Results are `conformance-worker-html-collection-collections.json` and
`conformance-worker-html-collection-static.json` under the upstream-conformance
artifact directory, with adjacent summaries. Production security, memory and
time limits are unchanged. The complete DOM and WebIDL roots are being checked;
these focused results do not replace the 2,467/4,022 full aggregate.

Local validation is 100 enabled V8 tests, including unreachable HTMLCollection
cycles, and seven WebIDL tests, with generated bindings, formatting and Clippy
checks passing. Two fixture-dependent diagnostic tests are excluded from the
default V8 test run; the six-script NodeList diagnostic was also run explicitly.

The complete collection-candidate DOM run finished at 324/664 for the substring
filter (322/662 DOM plus two accname files): **25 gains, zero regressions**.
WebIDL finished at 29/58 with no gains or regressions. Exact selection and passing
harness completion were checked for both. Summaries are adjacent to
`conformance-worker-html-collection-dom.json` and
`conformance-worker-html-collection-webidl.json`. These root results do not
replace the preceding complete full-inventory score.

The remaining tag-name failures exposed lowercasing in createElementNS and
missing qualified-name decomposition in the native creation path. New source
preserves case and stores prefix separately from local name; its regression
checks HTML-namespace uppercase names and prefixed non-ASCII names through both
collection APIs. All 100 V8 and 27 DOM tests and Clippy pass. This follow-up is
not included in the collection candidate's signed ledgers.

## Qualified element names: signed focused result

Worker `25dc37e09d6cb6103135f97e460162bd1ff281f591ceb66dd5e7b9ed37b79bf5`
passes four of five pinned tag-name executions: both Document and Element
getElementsByTagName/getElementsByTagNameNS files now pass. The cross-document
HTMLness case still fails. Exact filtered inventory and passing completions are
verified in `conformance-worker-qualified-elements-tags.json` and its summary.
The complete affected DOM root is running. No production limits were changed.

The qualified-name candidate's complete DOM filter finished at 329/664
(327/662 DOM plus two accname files): **30 gains and zero regressions** versus
the latest complete full baseline, five additional gains beyond the collection
candidate. CSS finished at 231/569 for its substring filter (226/564 CSS plus
five referrer-policy files), unchanged with zero regressions. Exact selections
and all passing completions were verified; ledgers and summaries are
`conformance-worker-qualified-elements-dom.json` and
`conformance-worker-qualified-elements-css.json`. All 31 layout tests also pass.
The full aggregate remains the last actually completed full run, 2,467/4,022;
root results are not combined into a claimed full-inventory result.

## Native document mode and metadata follow-up

New source preserves per-document content type, HTMLness and parser quirks mode
in the native DOM, including document clones, imports and replacement parsing.
DOMImplementation-created XML/XHTML/SVG documents receive their correct content
type; createHTMLDocument remains HTML in standards mode. Document.createElement
uses document mode for case and namespace and preserves literal colons in local
names. Tag/class collections capture matching mode when created; quirks class
matching folds ASCII only. Native metadata is allocated only for documents.

All 100 enabled V8 tests and 27 DOM tests pass, with clean V8 production Clippy.
The regression covers parser quirks, document metadata, XML name case, colon
handling, cloning and a collection retained across adoption into an XML document.
Signed-worker confirmation is pending. XML parsing/iframe MIME handling, Stylo
quirks integration and remaining namespace validation are separate open work.

Signed worker `a041419dc7480a29c7eaf8af6dbb030efeaa1e9a648ebe65c002f942eddef824`
passes both focused executions: `getElementsByClassName-14.htm` and
`Document-contentType/contentType/createDocument.html`. Both were failing in
the qualified-name candidate. Exact selections and harness completions are
verified in the `conformance-worker-document-mode-classes` and `-content`
ledgers and summaries. The complete DOM root is being validated with unchanged
security and resource limits.

A subsequent test-only addition explicitly verifies metadata preservation through
deep cloning and native import, and refresh on document replacement without
changing existing clones. All 28 DOM tests and strict DOM Clippy pass. This test
was added after the signed worker above was built; production source is unchanged.

The complete document-mode DOM filter finishes at 331/664 (329/662 DOM plus
two accname files), with **two gains and zero regressions** versus the
qualified-name candidate. Exact filtered inventory and all passing completions
are verified in `conformance-worker-document-mode-dom.json` and its summary.
Failure domains: 283 assertion, 10 runtime, 9 incomplete and 31 harness. All
141 native runtime tests also pass. These root results do not replace the last
complete full-inventory ledger (2,467/4,022).

## Committed baseline and namespace follow-up

Commit `53914a5` records the accumulated conformance implementation, vendored
Stylo/Blitz adapters, strict runner tooling and evidence notes. The working tree
was clean immediately after that commit. Historical ledgers retain their original
source fingerprints; committing source does not turn them into release evidence.

New source applies current DOM element-local-name and namespace validation to
createElement/createElementNS and DOMImplementation.createDocument. It exposes
Element.prefix, returns null for absent namespaces, and reflects qualified node
names with HTML-document casing. Undefined qualified names convert to the string
"undefined" while legacy null-to-empty handling remains. Regression tests cover
reserved namespaces, unusual valid names, exception precedence and reflection in
HTML and XML documents. All 100 enabled V8 tests, production Clippy, generated
bindings and formatting checks pass; signed validation is pending. The name rules
were checked against https://dom.spec.whatwg.org/#validate-and-extract and the
pinned Document-createElementNS fixture.

Signed worker `819c2dd270a09ec8610eda1fd44c4df47b7bf43d5746c966f99a62ca77e5c1ca`
passes DOMImplementation.createDocument with **434/434 assertions** and retains
the content-type pass (two of two filtered files). The createElementNS file is
195/596 assertions, still failing: 390 failures originate in XML/XHTML iframe
fixtures parsed as HTML, and 11 involve missing HTMLSpanElement/HTMLUnknownElement
interfaces. Exact selected IDs and every passing completion are verified in
`conformance-worker-namespace-validation-creation.json` and `-element.json`.
The complete DOM inventory is running; security and resource limits are unchanged.

The namespace candidate completes the DOM filter at 333/664 (331/662 DOM plus
two accname files), with two gains (`DOMImplementation-createDocument` and
`Node-nodeName`) and zero previously passing files lost. Exact inventory and
passing harness completion are verified in `conformance-worker-namespace-validation-dom.json`
and its summary. Failure domains are 280 assertion, 11 runtime, nine incomplete,
30 harness and one timeout. `name-validation.html` changes from assertion failure
to the existing worker wall-time limit as valid-name loops progress further;
`Element-classlist.html` changes from harness failure to timeout. Neither was a
previously passing file. These remaining failures are not treated as conformance
completion, and limits remain unchanged.
