# Oryn2 implementation status

> **Current priority: complete conformance. Not a release candidate.**
> Historical selected-inventory measurement: **3,144/4,022 (78.2%)**, **878 remaining**. Not a current full-WPT baseline.

## Completion implementation in progress — September 15

### Native encoding streams and transform pressure

Native `TextEncoderStream` and `TextDecoderStream` now share the transform queue,
with incremental codecs, split surrogate/multibyte handling, BOM options, fatal
flush errors and original input-conversion errors. The distinct native transform
controller supports enqueue/error/terminate and stable callback identity. Readable
demand controls writes; count-based high-water marks are validated and reflected
in readiness. `pipeThrough` accepts native readable/writable pairs.

Validation so far: **310 JS tests passed, 3 existing ignores**; strict JS Clippy,
generated bindings and formatting checks pass. Signed WPT and native regression
validation are pending. This does not close the stream package: pipe lifecycle and
propagation, custom size functions, bytes/BYOB streams, realm behavior and complete
upstream acceptance still require evidence and implementation.

### Writable and transform stream prerequisite work

The next continuation implements an ordered native write/close queue and tracks
asynchronous sink initialization, writes and transform flush. Sink callbacks keep
their receiver and original thrown/rejected values. Flush finishes before the
readable side closes; transform failures propagate to pending reads. Abort waits
for the in-flight operation and asynchronous cleanup, with explicit close/abort
race handling and the required abort/close/closed promise order.

Writers now retain stable `ready` and `closed` promises, reflect the default queue's
backpressure, reject operations after release, and allow a new writer to observe
existing queued work. Locked-stream close/abort rejects. These changes do not yet
implement custom queuing strategies or transform-readable backpressure.

Validation: **301 JS tests passed, 3 existing ignores**, including nine new
writable/transform regressions; strict JS Clippy passes. Native regressions pass
**583 tests with 6 existing ignores**. Formatting and diff checks pass. Pipe propagation, full queuing strategies, native
transform controller semantics and encoding streams remain open. No signed WPT
or OIL totals are changed by these tests.

### Readable stream prerequisite work

The current continuation implements stored stream errors, stable reader `closed`
promises, queue draining, reader release, cancellation, and asynchronous source
callback sequencing. Pending and future reads preserve the original failure;
network body-read failures enter the same error state. Late network chunks after
cancellation are discarded. Native promise reactions identify the owning stream
through its controller, and source callbacks retain their receiver.

Validation: **292 JS tests passed, 3 existing ignores**, including eight new
stream lifecycle regressions; strict JS Clippy passes. These tests use
`v8-host,v8-sandbox`, matching the supplied sandbox-enabled V8 archive. An earlier
`v8-host`-only run crashed in the pattern-service test and is retained in the
artifacts. The native regression suite passes **583 tests with 6 existing
ignores**. Formatting and diff checks also pass.

This is a prerequisite checkpoint, not encoding-stream or Core qualification.
Writable/transform sequencing and error propagation, strategy/backpressure
conformance, encoding streams, the 378 runtime failures, and the combined plan
acceptance remain open. No WPT totals are changed by these unit-test results.

### Last signed candidate results

**Signed implementation checkpoint: `0dd16976`. The complete plan remains unfinished.**
The final signed candidate passes **904/930 selected encoding executions**
(**823 improvements, zero regressions** against the same IDs), including **76/78**
of the API subset. All 26 remaining selected failures require encoding streams;
378 prior encoding runtime failures were not repeated in this subset and remain
open. This is not a full Core qualification run.

The same signed candidate passes **164/165 controlled OIL attempts**. Research,
forms, shopping, booking, framework and authentication suites pass; SaaS
`invalid_login_recovery` sample 1 times out at IPC-response execute message 39.
Prior timeout evidence remains open. DOM/JS/native/network regression suites total
**996 passes and 9 existing ignores**; strict Clippy, generated bindings and
formatting checks pass. The architecture guard fails on stale shadow evidence.
See the [complete run report](../../artifacts/core-completion-2026-09-15/REPORT.md)
and [final encoding comparison](../../artifacts/core-completion-2026-09-15/encoding-semantic-final-comparison.json).

The pre-implementation working tree was committed as `ba7ba33d`. Native
TextEncoder/TextDecoder and resource-decoding fixes are committed as `aa8bbf2d`;
the complete Core allocation ledger is committed as `d3ac0407`. The overall
completion plan remains open; no package is declared complete by these changes.

- Native codecs replace the page bootstrap substitutes in Window and workers,
  including streaming state, fatal errors, BOM handling, legacy labels, shared
  buffers, scalar-safe `encodeInto`, and collection of unreachable codec records.
- Upstream tests exposed and prompted repairs for detached encoder destinations,
  empty streaming chunks in legacy decoders, double BOM removal in document
  loading, and native data-URL response handling. Text encoding streams remain
  unimplemented.
- The allocation tool reconciles all **16,737 Core obligations** against the
  manifest and evidence fingerprints. It assigns implementation packages while
  preserving raw outcomes, qualification status, and unknown root causes.
- Validation before the following signed run: **579 native tests passed / 6
  ignored**, **281 JS tests passed / 3 ignored**, targeted resource-decoder tests
  passed, and strict Clippy passed. Ignores are existing tests, not new waivers.

The first signed top-level encoding run passed **61/78** executions, versus
**4/78** for the same IDs in the baseline: **57 improvements, no regressions**.
Its 17 failures are preserved in
[encoding API evidence](../../artifacts/core-completion-2026-09-15/encoding-api.json).
The subsequent clean-source signed run completed all **1,308 mapped encoding
executions** in 25m 32s: **81 passed, 1,227 failed** (849 assertion failures,
378 runtime failures; no harness or environment-blocked results). The original
78-case subset now passes **69/78**. The HTML parser still removes an additional
BOM after resource decoding; document encoding metadata, legacy form/URL encoding,
and encoding streams also remain open. See the immutable
[full encoding result](../../artifacts/core-completion-2026-09-15/encoding-all.json).

Shared immutable computed styles now avoid repeated map copies and preserve
independent observable CSS values. Layout tests pass **74/74** and JS tests pass
**281/284**, with three existing ignores. A separate signed candidate still fails
all five representative large-document checks: four RSS limits and one navigation
wall-time limit. This optimization does not close the resource failures.

A sampled native run exposed context ownership cycles: the context registry
retained actors and worker routes that strongly retained the context. These
internal owners now retain weak references, upgrading only during bounded work.
New regressions cover active timers, sibling public owners, and retained broker
routes after context disposal. The full native suite passes **581 tests**, with
**6 existing ignores**. These changes postdate the signed encoding/OIL candidate;
they do not explain or close the shopping timeout without further evidence.

The clean-source signed candidate at `d3ac0407` completed the existing controlled
workflow regression matrix, using three samples each:

| Controlled suite | Passed / planned |
| --- | ---: |
| Research | 18 / 18 |
| Forms | 57 / 57 |
| Shopping | 20 / 21 |
| Persisted SaaS | 12 / 12 |
| Booking | 18 / 18 |
| Eight framework applications | 24 / 24 |
| Five authentication mechanics | 15 / 15 |
| **Total** | **164 / 165** |

All six outcome-runner reports confirm source/build identity. The three auth
batches used the same explicit frozen parent and worker paths. The shopping
`price_change_recovery` third sample failed with **wall time,
phase=ipc_response, command=execute, message_id=35**. Its failed persisted-state
oracle remains recorded; passing retries would not explain or close that defect.
See [batch exits](../../artifacts/core-completion-2026-09-15/oil-batch-exit.json),
[shopping report](../../artifacts/core-completion-2026-09-15/oil-shopping/summary.json),
[framework report](../../artifacts/core-completion-2026-09-15/oil-frameworks/summary.json),
and [auth exits](../../artifacts/core-completion-2026-09-15/oil-auth-exit.json).

Further implementation checkpoint: `9b9cb502` commits context ownership and
shared computed styles; `3e2452d4` commits document encoding propagation and the
HTML parser BOM repair. The latter signed candidate passes **71/78** encoding API
executions. Native tests verify metadata before parser scripts and in static and
dynamic frames, plus preservation of a second UTF-16 BOM.

The candidate at `b09d9429` implements owner-document hyperlink components and legacy
query encoding; form GET and POST encoding (URL-encoded, text and multipart),
`_charset_`, and byte-preserving uploads; bounded HTML charset prescanning and
same-origin parent encoding fallback. Its regression suites pass **116 DOM,
283 JS, 582 native, and 13 network tests**, with only the existing 3 JS and 6
native ignores. Strict Clippy passes across those crates.

The signed candidate at `b09d9429` completed the 930 previously semantic encoding
cases: **903 passed / 27 failed**, an improvement of **822 cases with zero
regressions** against the same IDs. The 378 prior runtime failures are excluded
from this diagnostic subset and remain open. Twenty-six failures require encoding
streams; the other ISO-2022-JP control-character case is repaired locally.
The same candidate completed the controlled workflow matrix at **163/165**:
shopping `required_blocks` sample 2 and SaaS `logged_out_access_denied` sample 1
hit IPC-response execute deadlines (messages 41 and 11). These results do not
qualify Core or the expanded families. See the
[implementation run report](../../artifacts/core-completion-2026-09-15/REPORT.md).
Two pre-existing test-module ordering warnings and an excessive-precision literal
were repaired without changing test values.

The follow-up repairs release temporary context ownership before blocked network
forwarding and escape ISO-2022-JP control-character replacement references. The
backpressure regression was first reproduced failing, then passed with the fix.
Full follow-up suites pass **583 native / 6 existing ignores** and **284 JS / 3
existing ignores**; strict Clippy passes. Final signed verification is recorded at the top of this section.

These are controlled regression outcomes, **not expanded five-family acceptance,
complete Core conformance, or release qualification**. The historical full-WPT
baseline below remains unchanged.

## Complete Core and five-family implementation plan — September 15

The [complete implementation plan](CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md)
defines the combined finish line: all **16,737 Core obligations** and all five
declared OIL workflow families, qualified on the same signed candidate. It covers
the entire remaining implementation, including unsupported runner/global/visual
coverage, native lifecycle and authority, encoding/loading/policy, worker/storage,
DOM/input/rendering, realistic application outcomes and final performance/security
gates. Four parallel work lanes preserve the foundations already implemented.
This is a delivery plan, not additional implementation or qualification evidence.

## Full WPT rerun completed — September 15

The all-tier diagnostic run recorded **35,010/35,010 execution results**:
**7,286 raw passes**, **27,658 failures**,
and **66 environment-blocked results**.
The controller reports completion as **true**; candidate hashes
remain unchanged: **true**. Unsupported coverage remains
**93,544 of 128,554 full-manifest entries**. Unsupported entries are not passes.

See the [full run report](../../artifacts/wpt-full-baseline/full-current-2026-09-15/REPORT.md)
and [controller summary](../../artifacts/wpt-full-baseline/full-current-2026-09-15/summary.json)
for failure domains, qualification, integrity checks and raw evidence links.
This run is diagnostic evidence from an uncommitted source tree, not release
attestation. An incomplete controller result must not be used as a full pass rate.

### Results by implementation priority

Priority follows the existing capability tiers. Pass rates below are raw passes
divided by passes plus failures; environment-blocked and unsupported entries are
excluded from that rate but remain visible in coverage.

| Priority | Tier | Passed | Failed | Environment-blocked | Unsupported | Attempted pass rate |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Core | 4,415 | 9,315 | 17 | 2,990 | 32.2% |
| 2 | Necessary | 2,684 | 12,508 | 0 | 34,330 | 17.7% |
| 3 | Extended | 187 | 5,835 | 49 | 56,224 | 3.1% |

Core covers ordinary loading and agent actions; Necessary covers modern-app
capabilities; Extended includes specialized and not-yet-classified areas.
Unsupported means the inventory has no valid runner mapping, not that the
browser passed or failed that test. This run executed the full supported
inventory, not every entry in the official WPT manifest.

### Overlap with the five OIL workflow families

The table compares exact WPT execution IDs where prior focused evidence exists,
and broader path groups for related browser dependencies. Counts describe whole
executions, not individual assertions. Matching failures across revisions do not
establish a shared root cause. This WPT run did not rerun the five OIL outcome
suites or their independent persisted-state oracles.

| Workflow family | Earlier workflow-driven issues | Current WPT evidence and limits |
| --- | --- | --- |
| Research and extraction | Extraction scope, retained-document URLs, initial iframe readiness | The iframe-element group has **47 passes / 103 failures**. Frame/document behavior remains a relevant dependency, but WPT does not directly validate OIL extraction output or persisted research outcomes. |
| Forms and document submission | Email validation, missing validity APIs, requestSubmit, numeric and text controls | The earlier email, typeMismatch and requestSubmit execution IDs now pass. All **28 selected constraint executions** and **13 selected numeric API executions** pass. The broader forms group still has **241 passes / 204 failures**. |
| Shopping | Framework loading, Nuxt import maps, form validation and controlled checkout | The import-map group remains **19 passes / 38 failures**, matching the preceding focused aggregate. Controlled cart/order success does not establish general storefront or module-loading compatibility. |
| Authenticated SaaS | OAuth popup self-close, callback completion, window lifetime and session recovery | The same **nine window-close execution IDs** used during workflow development still fail. The broader Window group has **20 passes / 74 failures**. This is direct overlap in failing test coverage, not proof that every failure shares the earlier OAuth action error's cause. |
| Booking and scheduling | Date/numeric controls, required fields, stale availability and an intermittent worker timeout | Selected numeric and constraint fixes remain passing. The original booking timeout is still unexplained; WPT timeouts do not prove the same cause. The booking fixture's server-side timezone handling does not qualify browser timezone behavior. |

Focused evidence used for the comparisons:

- Forms: [requestSubmit](../../benchmarks/evidence/forms-outcomes-requestsubmit-wpt-2026-09-14.json), [email](../../benchmarks/evidence/forms-outcomes-email-wpt-2026-09-14.json), [typeMismatch](../../benchmarks/evidence/forms-outcomes-type-mismatch-wpt-2026-09-14.json), [constraints](../../benchmarks/evidence/text-editing-constraints-wpt-2026-09-14.json), and [numeric APIs](../../benchmarks/evidence/numeric-apis-wpt-2026-09-14.json).
- Window lifecycle: [nine-file close slice](../../benchmarks/evidence/research-candidate-close-wpt-2026-09-14.json) and [Window identity slice](../../benchmarks/evidence/native-window-top-wpt-2026-09-14.json).
- Workflow context: [five-family readiness](OIL-AGENTIC-READINESS-2026-09-14.md), [framework/import-map evidence](../../benchmarks/evidence/framework-import-maps-native-2026-09-14.json), and [unresolved booking timeout](../../benchmarks/evidence/booking-timeout-investigation-2026-09-14.md).
- Current outcomes: [full execution ledger](../../artifacts/wpt-full-baseline/full-current-2026-09-15/cohort.json). Broader groups use the `html/semantics/forms/`, `html/semantics/embedded-content/the-iframe-element/`, `html/browsers/the-window-object/`, and `import-maps/` path prefixes.

### Next implementation priorities from the overlap

These priorities describe the workflow overlap. The
[complete plan](CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md) adds encoding/resource
failures, all Core capability groups and the currently unsupported coverage;
the four items below alone cannot complete Core.

1. **Window/iframe creation, navigation and closure:** investigate the shared
   mechanisms needed by SaaS, research, and embedded checkout/booking flows.
   Preserve distinctions between initial realm creation, public WindowProxy
   routing, callback completion and retirement.
2. **Module and import-map loading:** address the remaining module-resolution,
   integrity and lifecycle gaps beyond the controlled framework fixtures.
3. **Forms and input semantics:** retain the confirmed constraint/numeric fixes
   while diagnosing the remaining broader forms failures.
4. **Timeout diagnosis:** correlate transport/execution phase and lifecycle
   evidence before assigning the booking timeout and WPT timeouts a common cause.

Validate each repair against its relevant WPT selection and affected OIL outcomes
on the same frozen candidate. Controlled workflow success and broad WPT failures
are compatible: the fixtures cover specific task paths, while WPT exercises
additional states and boundaries. None of the five families is qualified by
these results alone; full Core coverage and architecture/security gates remain open.

## September 14 native frameElement identity (current development tree)

`window.frameElement` now uses a native readonly getter and a V8-traced private
frame reference rather than a bootstrap data property. Linking installs the
same-origin frame wrapper or explicitly clears the private reference for an
opaque/cross-origin child; retired execution returns null. The regression checks
top-level null, child frame identity, descriptor flags, rejected assignment and
null through a retained getter after removal. All **55 frame tests pass**
(3.40 seconds). The full native suite passes **576 tests**, six ignored
(153.73 seconds), and the full JS suite passes **278 tests**, three ignored
(4.33 seconds), on source `dca60c61…`. Strict Clippy and formatting pass.
Signed/WPT qualification remains pending.
This does not complete initial iframe creation or native public WindowProxy
routing.

## September 14 native Window top identity (preceding signed candidate)

`window.top` is now a native readonly, enumerable, non-configurable getter.
Top-level Windows return themselves. Child linking supplies native ancestry in
a V8-traced private field rather than assigning an author-visible data property;
the bootstrap no longer overwrites `top`. The nested-frame regression verifies
replacement is rejected, the descriptor has no setter, author getters are not
called, and a nested child resolves to the top-level Window.
All **54 frame tests pass** (3.40 seconds) and the full JS suite passes
**278 tests**, three ignored (4.23 seconds). Full native verification passes
**575 tests**, six ignored (156.05 seconds). Strict native Clippy and formatting
pass. The signed worker on source `4aac3b9d…` passes signature verification,
protocol-v10 initialization and sandbox denial probes. The focused signed WPT
run gains **Window unforgeable attribute: top**, losing no passing assertions;
the four-file slice remains **2/4 files passing**. All **24 framework checks
pass**, with matching source/build identity and successful fixture cleanup.
See [WPT evidence](../../benchmarks/evidence/native-window-top-wpt-2026-09-14.json)
and [framework evidence](../../benchmarks/evidence/framework-native-top-2026-09-14/summary.json).
These are dirty-tree diagnostic results, not release or full-family qualification.
This does not complete cross-origin Window callbacks,
initial iframe creation or the remaining JavaScript proxy removal.

## September 14 native committed iframe document lookup (preceding signed candidate)

Iframe `contentDocument` now reads the committed native child's unforgeable
document getter rather than the JavaScript proxy produced by the bootstrap.
The new regression first reproduced a page-modified proxy factory returning
`forged|1` (forged document and one proxy getter invocation); after the change
it returns `native child|0`. All **54 frame tests pass** (3.40 seconds), including
cross-origin document clearing, return navigation and retained-document cases.
The first narrower run had one fixture-server bind denied by the filesystem/
network sandbox; the loopback-enabled rerun passes. Strict native Clippy,
formatting and diff checks pass. Full native verification passes **575 tests**,
six ignored (148.30 seconds), on source `9593f4cd…`; see
[native evidence](../../benchmarks/evidence/native-committed-frame-document-2026-09-14.json).
The signed worker passes signature, protocol-v10 and sandbox denial probes.
Signed research checks pass **18/18** with final persisted oracles verified,
unchanged artifact identities and successful cleanup. Signed forms checks pass
**57/57**. The four-file Window/base WPT slice remains **2/4**, with no passing
assertions gained or lost. See
[research results](../../benchmarks/evidence/research-native-committed-document-2026-09-14/summary.json),
[forms results](../../benchmarks/evidence/forms-native-committed-document-2026-09-14/summary.json)
and [WPT results](../../benchmarks/evidence/native-committed-frame-document-wpt-2026-09-14.json).
These are dirty-tree diagnostics, not full-family or release qualification.
Public WindowProxy factories remain JavaScript-based;
this removes one identity lookup through them, not the entire dependency.

## September 14 child blank-document fallback base (preceding signed candidate)

Native child-document initialization now captures the creator document's base
URL before realm allocation and installs it as the child's inherited fallback.
This uses native document state, including its base element, rather than an
author-controlled Window property. A regression verifies the child remains
`about:blank`, keeps the captured base after the creator's URL changes, and uses
its own URL after a non-blank commit. A second regression verifies a relative
creator base element is resolved and captured, survives later base-element
mutation, and is inherited by a nested blank child independently of subsequent
creator navigation. The full native suite passes **574 tests**, six ignored
(150.12 seconds); strict native Clippy, formatting and diff checks pass.
The signed worker on source `8a9b443f…` passes signature verification,
protocol-v10 initialization and sandbox denial probes. The expanded four-file
Window/base WPT slice finishes **2/4 files passing**, zero environment blocks.
The newly selected upstream `base_about_blank.html` passes its load-time check;
it was not measured on the preceding candidate, so this is new coverage evidence,
not a demonstrated before/after gain. The original three-file slice has no
passing assertions gained or lost. See
[signed WPT evidence](../../benchmarks/evidence/window-blank-base-wpt-2026-09-14.json).
OIL workflows have not been rerun on this candidate. Synchronous iframe
creation and native WindowProxy/navigation integration remain unfinished.

## September 14 document-owned URL lookup (preceding signed candidate)

Rebinding a WindowProxy reproduced a native lifetime bug: a retained document's
base URL changed from `https://example.test/old/document` to the new document's
URL. URL lookup now reads the document binding state's native stored URL rather
than traversing the WindowProxy's current `location` property. Old and new
documents retain their distinct URLs in the regression.

The full JS suite passes **278 tests**, three ignored (4.22 seconds). Full native
verification passes **572 tests**, six ignored (163.92 seconds). The signed
worker build passes protocol-v10 initialization, signature verification and
sandbox denial probes. On source `1284accb…`, signed persisted forms pass
**57/57** with all final oracles verified and framework checks pass **24/24**;
both runners confirm source/build identity and successful fixture cleanup.
The selected three-file Window WPT slice remains **1/3 files**, with no passing
assertions gained or lost versus the previous signed candidate. See
[WPT evidence](../../benchmarks/evidence/document-owned-url-wpt-2026-09-14.json),
[forms evidence](../../benchmarks/evidence/forms-document-owned-url-2026-09-14/summary.json)
and [framework evidence](../../benchmarks/evidence/framework-document-owned-url-2026-09-14/summary.json).
These are dirty-tree diagnostic results, not release or full-family qualification. This
repairs a document-lifetime dependency but does not implement initial iframe
creation or complete Window/document reuse.

## September 14 scope-based child document initialization (preceding development revision)

`create_native_document_context` combines native realm allocation and document
installation without activating the outer driver. Both driver child-document
entrypoints use it and adopt the initialized context afterward. The creator's
native context supplies inherited policy. On installation failure the helper
retires any installed bindings/environment and removes the realm from pending
adoption and identity lookup; identifiers remain monotonic.

All **53 native frame tests** pass. A new invalid-document regression passes,
checking no adoptable failed child, unchanged active parent, and successful later
creation with a fresh identity. The helper now rejects a retired creator before
allocation; that targeted test passes. The existing native callback realm test
also passes after switching to this combined helper, including document access,
origin isolation and caller-context preservation. Strict Clippy passes with the
creator guard. The full guarded revision passes **572 native tests**, six ignored
(150.27 seconds), on source `21aec657…`; see
[native evidence](../../benchmarks/evidence/native-child-document-initialization-2026-09-14.json).
Signed verification remains pending.
This helper still needs insertion-time integration,
initial Window reuse, native proxy linkage and navigation-specific policy rules.

## September 14 scope-based native frame ownership (preceding development revision)

Driver linking now shares `bind_native_frame_lifetime`, which accepts a V8 scope
and parent/child contexts rather than borrowing RawV8Host. It resolves native
environment/binding identities, validates iframe owner-document membership,
registers the lifetime and target name, and rejects retired environments or
discarded-generation rebinding. The new scope-level test verifies invalid-target
rejection, registration from an active JavaScript-to-native callback without
borrowing the outer host, caller-context preservation, removal-driven retirement
and rejection of revival. The strengthened callback test passes; the full native
suite passes **570 tests**, six ignored (159.54 seconds), on the same runtime
implementation compiled before that test extension. Strict Clippy also passes.
See [native evidence](../../benchmarks/evidence/native-scope-frame-ownership-2026-09-14.json).

An initially stricter connected-element check broke two parser-removal tests:
the legacy loader still links some removed frames late before its lifecycle
checkpoint retires them. This ordering remains explicitly preserved until
insertion-time creation replaces it; disconnected targets are not claimed to be
universally rejected. Public JavaScript proxy construction remains separate and
unchanged. All **53 native frame tests pass** (3.37 seconds). This is an
integration prerequisite, not completed synchronous
iframe creation, and has no new signed qualification.

## September 14 child-realm eval isolation diagnostic (preceding development revision)

`same_origin_frame_eval_uses_child_realm` passes for both a srcdoc iframe and
two same-origin external iframes with parser-time removal of the second. It
verifies borrowed eval reads the child's document, writes only child globals,
and observes native `window === self === frames`. The external fixture includes
the blank-page onload handler shape. This narrows, but does not resolve, the
signed WPT failures reporting unavailable iframe eval: the direct in-process
cases do not reproduce them. Broker-backed/frame-origin and public-proxy paths
still require investigation. Only this new targeted test was run; it is not a
new signed qualification or a completed native WindowProxy implementation.

[Signed timing probe](../../benchmarks/evidence/initial-frame-timing-2026-09-14.json)
now confirms the missing phase: a parser-following script sees a null srcdoc
`contentWindow`; post-load evaluation sees its document and eval function. The
post-load regression therefore does not cover initial child creation. This
points back to insertion-time native Window/document creation and navigation
lifetime, not a general missing V8 eval intrinsic. The probe does not yet explain
every external-frame WPT failure or constitute a fix.

## September 14 native Window self-reference properties (preceding signed candidate)

`window`, `self` and `frames` are now native accessor properties rather than
bootstrap data assignments. `window` is readonly and unforgeable; `self` and
`frames` have replaceable setters that create writable own data properties.
Retained getters resolve native Window identity independently of those replaced
values. The bootstrap no longer overwrites these properties.

The full JS suite passes **277 tests**, three ignored, including descriptor,
replacement and retained-getter checks. Extended cross-realm checks also pass:
borrowed getters return the receiver's Window and borrowed replaceable setters
modify only the receiver, without changing the caller's alias. Strict native
Clippy, formatting and diff checks pass. Full native and signed checks are
recorded below. This does not replace the remaining JavaScript
WindowProxy linkage or implement synchronous initial iframe creation.

Window initialization also stops copying EventTarget operations onto the global;
`addEventListener`, `removeEventListener` and `dispatchEvent` now resolve through
the existing native prototype chain. Worker initialization is unchanged. The
full JS suite still passes **277 tests**, three ignored (4.51 seconds), including
method placement and functional listener add/dispatch/remove checks. The earlier
self-reference-only native run completed with **568 passed, six ignored** (157.54
seconds). The fresh full native run for the latest placement change also passes
**568 tests**, six ignored (155.58 seconds), on source `4b41f5e9…`.

[Signed evidence](../../benchmarks/evidence/window-native-properties-signed-2026-09-14.json)
verifies **9 gained WPT assertions, no lost passes**: Window-properties improves
from **61/181 to 70/181 assertions passing**. The three-file slice still has only
**1/3 files passing**, no environmental blocks. Forms/uploads pass **57/57** with
persisted oracles; framework checks pass **24/24**. All fixtures close cleanly,
and post-run source/parent/worker hashes are unchanged. Signing and sandbox
probes pass. Other workflow families were not rerun on this candidate. These
dirty-source results do not establish broad conformance or release qualification.

## September 14 parser-frame load retirement (preceding signed candidate)

The freshly signed accessor candidate reproduced `unknown JavaScript realm 2`
in the Window-properties WPT file. A minimal native regression reproduced the
same error when parent parsing removes the second of two external iframes.
Startup now checks native execution liveness before activating each scheduled
initial frame; pending document loads also reject retired documents.

The regression passes for parser removal of the second frame, removal of both
frames, and removal of the second frame by the first frame's load handler.
Surviving load delivery, closed removed windows and remaining frame counts are
asserted. The full native suite passes **568 tests**, six ignored (158.91 seconds);
strict Clippy, formatting, diff checks and all **37 outcome-validator tests** pass.
See [native evidence](../../benchmarks/evidence/parser-frame-load-retirement-native-2026-09-14.json).
[Signed diagnostic](../../benchmarks/evidence/parser-frame-load-retirement-signed-2026-09-14.json)
now verifies the repair on source `bf2e6c63…`, worker `8fc7aba8…`. The Window-properties
file completes instead of aborting: **61 assertions pass, 120 fail**, including a
passing native document-unforgeability assertion. The three-file slice remains
**1/3 passing**, no environmental blocks. Missing synchronous initial iframe
creation remains independently failing; no full-WPT percentage is claimed.

Signed controlled outcomes pass: research **18/18**, forms **57/57**, shopping
**21/21**, booking **18/18**, plus framework checks **24/24** and CLI checks **3/3**.
Persisted oracles and cleanup verify; source, parent and worker hashes remain
unchanged. Auth mechanics were not rerun on this candidate. The earlier booking
timeout remains open. This dirty-source diagnostic does not qualify the five
families or the release.

## September 14 native Window.document accessor (preceding signed candidate)

`Window.document` now uses a native readonly, enumerable, nonconfigurable getter
with Window receiver and origin checks. Reinstalling this accessor preserves its
identity; this does not make the complete generated-binding reinstall supported.
The targeted test covers assignment, deletion, redefinition, illegal receivers,
constructor rejection, document identity and accessor reinstallation.

The first full JS run exposed two wrapper-lifetime test failures because document
wrapper creation had become lazy. Restoring eager wrapper-graph creation fixes
both: **276 JS tests pass**, three remain ignored. Formatting and diff checks
pass, as does strict native Clippy. Both targeted accessor tests pass, including
borrowing getters between same-origin realms and resolving the receiver's document.
The full native rerun for the corrected revision completed with **567 passed,
six ignored** (156.19 seconds). The current full JS suite, including the
cross-realm regression, passes **277 tests**, three ignored (4.43 seconds).
Signing and the script's sandbox probes passed for source `a486665f…`, worker
SHA-256 `d814c2be…`, Code Directory hash `a7a65c91…`. The signed three-file
Window/document diagnostic (`window-document-accessor-signed-2026-09-14.json`)
remains **1/3 files passing**, no environmental blocks; it reproduces both the
missing synchronous initial iframe document and the removed-realm startup error.
This is dirty-source diagnostic evidence, not release qualification. The
preserved before-change WPT diagnostic is
`benchmarks/evidence/window-document-before-2026-09-14.json`; synchronous initial
iframe creation and full Window/document reuse remain open.

## September 14 native document policy ownership (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-document-policy-ownership-2026-09-14.json),
source `97f30bc2…`, removes the host-wide Trusted Types/eval policy cache. Policy
now belongs to each native context, and legacy realm creation snapshots its
active creator's policy. The reproduced bug allowed raw HTML and eval in a new
realm from a restricted parent after an unrelated sibling was relaxed. Both
directions now pass without changing existing sibling policies.

**567 native tests pass**, six remain ignored; strict Clippy, formatting and diff
checks pass. No new signed/WPT qualification is claimed. Navigation-response
policy selection and inherited local-document rules remain loader work. A first
probe also exposed that reinstalling generated bindings in one context fails on
`Window.location`; initial Window/document reuse must address that separately.

## September 14 shared native document installation (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-document-installer-2026-09-14.json),
source `58bfd0c1…`, consolidates driver and callback document setup into one
scope-based installer. It attaches platform services, applies supplied Trusted
Types/eval policy, retires old bindings and binds the execution environment.
Invalid realm identities and retired environments are rejected before mutation.
The callback test verifies child policy enforcement without restricting its caller.

**566 native tests pass**, six remain ignored; strict Clippy, formatting and diff
checks pass. This is an iframe-integration prerequisite, not synchronous iframe
creation itself. Initial blank-document reuse, native WindowProxy linkage,
per-document policy inheritance and routing remain open. No new signed/WPT
qualification is claimed; the preceding signed research evidence remains intact.

## September 14 research extraction (latest signed candidate)

[Signed diagnostic](../../benchmarks/evidence/research-extraction-signed-2026-09-14.json),
source `8b5973b1…`, fixes extraction that ignored selector scope and scanned the
whole allocation arena. All content kinds now select from the current Document
tree, excluding detached and other-document nodes. Links/images/tables respect
subtree scope without overlap duplicates, follow live DOM order, and nested-table
rows no longer appear as extra rows in outer tables.

Two regressions failed before the fix; three extraction tests now pass. A new
six-case research corpus checks paginated comparison, redirects, dynamic results,
empty results, missing-source recovery and refreshed results against exact
facts/tables/source links, canonical observation URLs and persisted request logs.
The preceding signed binary passed **1/6** diagnostic attempts; the corrected
candidate passes **18/18**, with original failures retained.

**565 native tests pass**, six remain ignored. Strict Clippy, 37 outcome-validator
tests, three signed CLI tests, fixture locks, formatting and sandbox probes pass.
The same frozen candidate also passes **57/57 forms, 21/21 shopping, 18/18 booking,
24/24 framework and 15/15 auth-mechanics attempts**. Persisted oracles and cleanup
are verified. The nine-file WPT slice is unchanged at **5 passing / 13 failing /
3 timed-out assertions**, zero environment blocks. These are controlled dirty-tree
diagnostics, not full workflow or conformance qualification. Persisted SaaS,
broader research, native iframe/window integration and agent routing remain open;
the original booking timeout is still unresolved.

## September 14 worker deadline diagnostics (preceding signed candidate)

[Signed diagnostic](../../benchmarks/evidence/worker-deadline-diagnostics-2026-09-14.json),
source `f99e6613…`, distinguishes IPC-write, IPC-response and worker-execution
deadlines using fixed command labels and message numbers, never request payloads.
Timeout limits and failed-worker retirement are unchanged. A stalled-peer test
verifies the diagnostic and that the process is killed and reaped.

**562 native tests pass**, six remain ignored; strict Clippy, 33 outcome-validator
tests, six WPT-evidence tests, three signed CLI tests and sandbox probes pass.
A concurrent controlled workload passes **48/48 booking, 57/57 forms, 21/21
shopping and 24/24 framework attempts** with source identities, persisted
oracles where applicable and fixture cleanup verified. Auth was not rerun on
this candidate. The nine-file WPT slice is unchanged at **5 passing / 13 failing /
3 timed-out assertions**. The earlier booking timeout remains **open, not fixed**;
it did not recur. Full five-family readiness and conformance remain incomplete.

## September 14 controlled booking (preceding signed candidate)

The versioned six-case booking corpus covers both occurrences of a DST-fold
appointment, required-field blocking, stale availability rejection/recovery,
and duplicate-request idempotency. Its SQLite oracle independently checks
reservation fields, slot ownership, request outcomes and persistence after
fixture shutdown. Timezone conversion is server-rendered, not browser Intl
qualification. Seven booking validator/fixture tests bring the outcome suite
to 33 tests.

Signed source `cdfdfe1a…` passes **57/57 forms, 21/21 shopping, 24/24 framework
and 15/15 auth** diagnostics. The [initial booking run](../../benchmarks/evidence/booking-signed-2026-09-14/summary.json)
passes **17/18**; one attempt hits a 30-second worker wall-time limit. An unchanged
candidate [separate recheck](../../benchmarks/evidence/booking-signed-isolated-recheck-2026-09-14/summary.json)
passes 18/18. The combined result is **35/36**, not a fixed or failure-free suite.
The [open investigation](../../benchmarks/evidence/booking-timeout-investigation-2026-09-14.md)
preserves the original failure and explains the remaining diagnostic ambiguity.

The [same nine-file WPT selection](../../benchmarks/evidence/booking-candidate-close-wpt-2026-09-14.json)
remains **five passing, thirteen failing and three timed-out assertions**, zero
environment blocks. These are dirty-tree diagnostics, not full-family,
full-WPT, or clean-source release qualification. Research/extraction outcomes,
persisted SaaS workflows, native WindowProxy/initial iframe integration and
connected-agent routing remain open.

## September 14 direct metadata ownership (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-metadata-ownership-2026-09-14.json),
source `ccbff3bb…`, makes direct metadata updates enter their active native owner
document's context. Tests verify that a borrowed metadata setter changes only
the child's referrer policy and schedules refresh navigation only in the child,
using the child-relative URL. The refresh test covers scheduling, not a completed
network navigation. **561 native and 275 JavaScript tests pass**; six native and
three JavaScript tests remain ignored. Strict native Clippy, formatting and diff
checks pass. No new signed/WPT qualification is claimed. Initial iframe creation,
native WindowProxy linking and the complete five-family goal remain open.

## September 14 resource preparation ownership (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-resource-preparation-ownership-2026-09-14.json),
source `14d1b488…`, routes insertion resource preparation through the native owner
document. Image preparation also resolves ownership for direct property setters.
A reproduced cross-realm regression previously resolved a child image against
the parent's URL and sent a borrowed `src` setter request through the parent's
endpoint. Both requests now use the child URL and endpoint, with zero parent
requests. **559 native and 275 JavaScript tests pass**; six native and three
JavaScript tests remain ignored. Strict native Clippy, formatting and diff checks
pass. No new signed/WPT qualification is claimed. Initial iframe creation,
native WindowProxy linking and complete five-family readiness remain open.

## September 14 insertion execution ownership (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-insertion-execution-ownership-2026-09-14.json),
source `7ec8cce6…`, fixes two reproduced insertion-path bugs. Native lifecycle
retirement now precedes newly connected scripts, so replacement scripts cannot
observe a removed frame as open or schedule its timers. Post-connection scripts
execute in their native owner-document realm even when insertion uses a borrowed
method from another realm. Inert and retired documents do not execute them.

Both regressions failed before their fixes and pass afterward. **559 native and
274 JavaScript tests pass**; six native and three JavaScript tests remain ignored.
Strict native Clippy, formatting and diff checks pass. This development revision
has no new signed/WPT qualification. Synchronous initial iframe creation, native
WindowProxy linking and the full five-family goal remain incomplete.

## September 14 native ancestry and signed realm coordinator (preceding signed candidate)

[Signed diagnostic](../../benchmarks/evidence/native-realm-coordinator-signed-2026-09-14.json),
source `381b6971…`, qualifies the accumulated realm factory/coordinator work.
Frame linking now derives top-level ancestry from native frame lifetimes, not
the parent's author-modifiable `top` property. A throwing-getter regression
verifies that linking executes no such getter and gives nested frames the correct
top-level Window. **556 native tests pass**, six remain ignored; strict native
Clippy, formatting, 26 outcome-validator tests, three signed CLI tests, fixture
locks and signed sandbox probes pass.

Repeated signed checks pass **15/15 auth, 24/24 framework, 57/57 form and 21/21
shopping cases**, with persisted oracles, source identities and cleanup verified.
The unchanged nine-file WPT selection remains **five passing, thirteen failing
and three timed-out assertions**, zero environment blocks, with no gained or lost
passes. This is still a dirty-tree diagnostic. Synchronous iframe insertion,
native WindowProxy linking/navigation reuse and full five-family readiness remain
incomplete; no full conformance or clean-source release is claimed.

## September 14 callback-created realm coordinator (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-callback-realm-coordinator-2026-09-14.json),
source `61691865…`, adds isolate-owned retention and checkpoint adoption of
callback-created contexts. Their final binding states enter ordinary enumeration,
activation and retirement. Tests verify idempotent adoption, no revival of retired
environments, timer cancellation, removal before explicit adoption and retained
Document access after GC. **555 native tests pass**, six remain ignored; strict
native Clippy, formatting and diff checks pass.

Synchronous iframe insertion, native WindowProxy linking and initial-document
navigation semantics remain incomplete. This development revision has no new
signed or WPT qualification; prior signed evidence remains historical.

## September 14 scope-based native realm factory (preceding development revision)

[Native diagnostic](../../benchmarks/evidence/native-scope-realm-factory-2026-09-14.json),
source `4cefe541…`, moves realm identities/security tokens into isolate-owned
state and unifies initial/subsequent context creation behind a scope-based native
factory. A regression creates a native Document inside an active JavaScript
callback, verifies distinct intrinsics and opaque-origin denial, and retains the
Document across GC. **555 native tests pass**, six remain ignored; strict native
Clippy, formatting and diff checks pass.

This is groundwork for synchronous iframe creation, **not that integration's
completion**. Callback-created context registration, initial-document linking,
bootstrap/policy and navigation reuse still need to be connected. No new signed
or upstream qualification is claimed for this development source; the following
signed candidate remains the latest signed evidence.

## September 14 native frame disposal and owner-document routing

[Signed diagnostic](../../benchmarks/evidence/native-frame-discard-2026-09-14.json),
source `fd74209a…`, replaces the JavaScript `element.remove()` hook with native
frame lifetime tracking. Ancestor removal, `innerHTML`, ordinary reparenting and
nested foreign-frame disposal retire execution services and close retained window
handles; state-preserving `moveBefore` keeps the context. Dynamic frame routing
uses native owner-document identity, and nested srcdoc inherits native origin
identity without weakening sandbox isolation.

**554 native, 274 JavaScript, 115 DOM, 26 outcome-validator and three signed CLI
tests pass**. Six native and three JavaScript tests remain ignored. Strict native
Clippy, formatting, fixture locks and signed sandbox probes pass. The same signed
candidate passes **15/15 auth, 24/24 framework, 57/57 form and 21/21 shopping
checks**, including the persisted form/shopping oracles and fixture cleanup.

The unchanged nine-file window-close WPT selection gains **two assertion passes**
(cross-origin and cross-site removed frames), with no lost passes. It now has
**five passing, thirteen failing and three timed-out assertions**, zero environment
blocks, and still **0/9 fully passing files**. Synchronous initial iframe creation,
connected popup ownership/access, lifecycle callbacks and BroadcastChannel remain
open. This is a dirty-tree diagnostic, not complete architecture, full-WPT,
five-family or clean release qualification.

## September 14 ordered script closure and truthful auth outcomes (preceding candidate)

The OAuth popup failure below is repaired. Script-requested closure previously
entered the explicit-host cancellation path and killed an in-flight worker before
its click acknowledgement arrived. Script closure now waits behind that command;
host cancellation can still interrupt a stuck worker. Piped CLI commands now fail
nonzero and stop on errors, while interactive/SDK REPL recovery remains available.

[Final signed diagnostic](../../benchmarks/evidence/oauth-ordered-close-native-2026-09-14.json),
source `4d3f8aa1…`: **15/15 controlled auth cases**, **24/24 framework runs**,
**21/21 persisted shopping outcomes**, and **57/57 form outcomes** pass. Each has
three samples; source/signature identities match and fixture cleanup is verified.
**549 native tests, three signed CLI regressions, and 26 outcome-validator/oracle
tests pass**; six native tests remain ignored. Strict Clippy, formatting, fixture
locks, sandbox probes and all six signed-worker substitution rejections pass.

The auth harness now retains all five cases after failures, binds SSO to the
selected server origin, and checks complete action identities and traces. Action
IDs are page-local, not globally unique. A negative control confirms five failures
remain five attempted cases and produce a nonzero exit. Fixture lock revision 2
preserves the original lock byte-for-byte; original application/task sources are
unchanged. Intermediate setup and validator mistakes are preserved and explicitly
adjudicated in the diagnostic rather than counted as browser results.

This is **not full window-close conformance**. A fresh nine-file upstream selection
passes **0/9 files**, with **three passing, fifteen failing and three timed-out
assertions**, zero environment blocks, and 15.079 seconds elapsed. Failures group as:

- Four assertions: missing connected-window `opener`/document access.
- Three assertions: initial iframe and removed-frame closure state.
- Eight assertions: missing `BroadcastChannel`, before intended closability checks.
- Three timeouts: beforeunload/pagehide/unload callback completion.

These prioritize native connected-window/iframe ownership and lifecycle work;
they are not evidence that the repaired OAuth path still crashes. Auth fixtures
test controlled mechanics, not a real token exchange or persisted SaaS operation.
Next work remains native window/agent architecture and persisted authenticated
workflows, followed by research/extraction and booking coverage. No five-family,
clean-source release, full-WPT or performance qualification is claimed.

## September 14 real frameworks and native import maps (preceding candidate)

The actual locked framework corpus now builds through the restored build
entrypoint. Its served output is preserved and hash-checked; the new runner
retains all eight frameworks in the denominator. The initial native run found
Nuxt's `#entry` imports were skipped by the parser-driven import-map path.
Native per-realm registration/resolution now supports exact, prefix and scoped
maps, merge preservation, CSP authorization and canonical module identity.
Event-handler imports use the native document base and reject with native
exception objects, not strings.

[Final signed diagnostic](../../benchmarks/evidence/framework-import-maps-native-2026-09-14.json),
source `596c071e…`: **24/24 framework runs** (Angular, Next, Nuxt, Preact, React,
Svelte, SvelteKit and Vue; three samples each), **21/21 shopping outcomes** and
**57/57 form outcomes** pass, with no timeouts and clean fixture shutdowns.
All persisted outcome oracles and framework artifact identities were verified.
**548 native tests, 274 JavaScript tests and 23 harness/oracle tests pass**;
six native and three JavaScript tests remain ignored. Strict Clippy, formatting,
fixture locks and signed sandbox probes pass. Form constraints remain
**28/28 files, 920/920 assertions**.

The entire inventoried import-map group is **19/57 cases**, with **38 passing,
28 failing, 14 timed-out and six unrun assertions**, plus failed cases without
assertion results. There are zero environment blocks. The final correction gains
five assertion passes and loses one: valid integrity imports from event handlers
are now rejected instead of bypassing the map. Positive SRI support is still
missing; nonempty registered integrity entries fail explicitly. Dynamic map
insertion, script/error lifecycle and frame/helper readiness remain open.
These results do not establish full import-map or full-WPT conformance.

The next authenticated-SaaS probe found a concrete blocker: the OAuth popup's
Approve action reports a closed worker stream even though its callback reaches
the parent. The workflow fails; the final authenticated text is insufficient.
Streaming replay also exposes a CLI accounting gap: it prints the error but
exits zero, whereas file mode correctly fails. The evidence preserves both.
Next: repair and qualify child self-close/in-flight-action handling, then
persisted authenticated workflows. A native fault has not yet been established;
the observed error alone must not be labeled an unexplained process crash.

This remains dirty-tree diagnostic evidence, not release or five-family
qualification. The other workflow families and connected-window/worker,
full Core and clean-source security/performance gates remain open.

## September 14 persisted shopping outcomes

The new versioned shopping suite checks native OIL catalog filtering, product
comparison, variants, quantity updates and cart reloads, followed by controlled
checkout. Negative and recovery cases cover required fields, stale prices and
unavailable stock; duplicate submission must create one order and decrement
inventory only once. No real purchases are made.

[Signed diagnostic evidence](../../benchmarks/evidence/shopping-outcomes-native-2026-09-14.json),
source `b287cdce…`: **21/21 shopping outcomes** (seven cases, three samples,
three concurrent processes) and **57/57 existing form outcomes** pass. All
shopping outcomes were checked through fresh read-only SQLite connections
after fixture shutdown, including exact order fields, totals, inventory,
request sequences and reload evidence. Both fixtures shut down cleanly; no
timeouts occurred. The initial seven-case run also passed and remains preserved.

**19 oracle tests pass**, including concurrent duplicate prevention, invalid
cookie/quote rejection and deliberately corrupted persisted evidence. Ruff,
diff checks and signed worker/security probes pass. There are no Rust changes
in this increment: the parent and unsigned worker hashes match the previous
text-editing candidate; its Rust/WPT results were not rerun or relabeled.

This is server-rendered controlled workflow coverage, not general shopping or
framework qualification. Representative framework behavior, the other workflow
families, connected-window/worker architecture and full Core/clean-source
qualification remain open.

## September 14 native text editing and control events

User-edit provenance, length constraints and textarea defaults now share native
state across editing, validation, CSS, submission and OIL observations. User
editing enforces maxlength in UTF-16 units without splitting surrogate pairs;
script assignment remains distinct. Textarea defaults use direct child text,
with dirty-value/reset behavior and null-to-empty conversion. Select events use
a bounded native task queue, independent of page timers, with realm retirement
cleanup; user changes commit once on blur/focus transition.

[Final signed diagnostic](../../benchmarks/evidence/text-editing-native-2026-09-14.json),
source `ef520aa2…`: **28/28 constraint files, 920/920 assertions**. All five
remaining failures from the previous selection are fixed, with no lost passes.
An additional **8/8 text-control files, 122/122 assertions** pass, including
defaultValue, length reflection, numeric maxlength exclusion and setRangeText.
Numeric API **182/182** and input **150/150** assertions remain green. No selection
has environment blocks. These are selected suites, not a full-WPT baseline.

**539 native, 274 JavaScript, 114 DOM, 80 common, 31 parser, 10 WebIDL and 10
oracle tests pass**; six native and three JavaScript tests remain ignored.
Strict native Clippy, formatting, registry and signed sandbox probes pass.
[Independent OIL outcomes](../../benchmarks/evidence/oil-text-events-signed-2026-09-14/summary.json)
are **57/57** with all final server checks verified, no timeouts and clean fixture
shutdown. Contract v5 adds short-text blocking/recovery and Unicode maxlength
truncation; the server independently checks UTF-16 limits and exact receipt
fields. Existing controlled observation/error smoke is **12/12**.

The first signed candidate already passed all 920 constraints but exposed null
conversion and blur-change failures, plus a select-event timeout that left 22
follow-on assertions unrun. That evidence is preserved; the corrected candidate
retains all 97 intermediate text-control passes and resolves the 25 nonpasses.

This is dirty-tree diagnostic evidence, not release or five-family qualification.
Next coverage must include broader multi-step and persisted workflow outcomes,
representative framework applications and concurrency/isolation checks. The
remaining connected-window/worker architecture, full Core WPT and clean-source
security/performance gates remain open; not all input/editing semantics are done.

## September 14 shared native pattern constraints

Pattern constraints now share one native implementation across validity APIs,
CSS selectors/computed style, submission and OIL observations. It uses Unicode-set
regular expressions in a private context within the existing agent isolate,
not page-modifiable JavaScript regex methods. Bounded caching, backtracking and
input sizes are enforced. Isolate retirement closes the service; retained DOM
references fail explicitly instead of holding dangling V8 pointers.

[Signed diagnostic verification](../../benchmarks/evidence/pattern-native-2026-09-14.json),
source `7dfac26f…`: the constraint selection improves to **26/28 files, 915 passing /
5 failing assertions**, gaining **54** with every previous pass retained.
Remaining failures are textarea default-value/editing behavior (4) and emoji
maxlength (1). Numeric API **182/182** and numeric input **150/150** assertions
remain green. No selection has environment blocks. These are selected suites,
not a full-WPT baseline.

**536 native, 273 JavaScript, 113 DOM, 80 common, 31 parser, 10 WebIDL and 10
oracle tests pass**; six native and three JavaScript tests remain ignored.
Strict native Clippy, formatting, generated registry and signed sandbox probes
pass. [OIL outcomes](../../benchmarks/evidence/oil-pattern-signed-2026-09-14/summary.json)
are **48/48**, all final server oracles verified, with no timeouts and clean
fixture shutdown. Contract v4 adds a patterned reference and negative/recovery
cases; the independent server validates the submitted reference and exact
receipt fields. Existing controlled observation/error smoke is **12/12**.

The regex backtrack ceiling conservatively reports mismatch; resource or retired
engine failures propagate as errors. Private regex state may internally retain a
subject until replaced or the isolate is destroyed; no zero-retention claim is
made. This remains dirty-tree diagnostic evidence, not release or five-family
qualification. Editing/length, broader workflow/framework coverage and the
connected-window/worker architecture and clean-source qualification gates remain.

## September 14 native numeric input APIs

`valueAsNumber`, `valueAsDate`, `stepUp()` and `stepDown()` now use the shared
native control state. They cover UTC calendar/week conversion, time wrapping,
precise decimal stepping, min/max bounds, signed counts and dirty-value/reset
behavior. Date setters read internal Date values without calling authored
methods; nullable conversion and JavaScript-versus-DOM exception types are
tested. Script changes do not synthesize user input/change events.

[Final verification](../../benchmarks/evidence/numeric-apis-native-2026-09-14.json),
source `22f5f7b8…`: **13/13 API files, 182/182 assertions**, and **8/8 input files,
150/150 assertions**. The latter gains 28 assertions over the previous input
selection with no lost or missing passes. The constraint selection retains all
**861 passes**, with **59 failures** in pattern (54), textarea defaultValue/editing
(4) and emoji maxlength (1). All selections have zero environment blocks.
These use persistent parents with isolated prewarmed workers; they are not a
full-WPT baseline or performance qualification.

**534 native, 272 JavaScript, 112 DOM, 80 common, 31 parser, 10 WebIDL and 10
independent oracle tests pass**; six native and three JavaScript tests remain
ignored. Strict native Clippy, formatting, registry and signed sandbox probes
pass. [Signed OIL outcomes](../../benchmarks/evidence/oil-numeric-apis-signed-2026-09-14/summary.json)
are **42/42**, with all final server oracles verified, no timeouts and clean
fixture shutdown. Contract v3 adds an application button that uses the APIs:
OIL clicks it, and the independent receipt verifies seats `5`, date `2026-10-02`
and time `23:01`. Existing observation/error smoke is **12/12**.

The intermediate signed candidate had one empty-value stepping failure; an
added regression also exposed nullable `undefined` conversion. Both were fixed
before final replay, with intermediate evidence preserved. Empty-value stepping,
negative counts and multi-step snapping follow Chromium interoperability where
the literal HTML algorithm wording differs; the evidence records this choice.

This remains dirty-tree diagnostic evidence, not release or five-family
qualification. Pattern/editing work, broader independent workflow outcomes,
representative framework apps and the connected-window/worker architecture
and clean-source qualification gates remain open.

## September 14 native numeric and calendar constraints

Number, range, date, month, week, time and datetime-local inputs now share native
min/max/step validation, sanitization and reflected attributes. The same flags
drive validation APIs, submission, CSS and OIL observations. Calendar parsing,
overnight time bounds, initial range defaults and exact decimal range snapping
are covered; an intermediate floating-point serialization failure was repaired
and replayed against a newly signed worker.

[Verification](../../benchmarks/evidence/numeric-calendar-native-2026-09-14.json),
source `41639015…`: the constraint selection reaches **20/28 files, 861 passing /
59 failing assertions**, gaining **171** with no lost or missing baseline passes.
The broader eight-input selection reaches **5/8 files, 122 passing / 28 failing
assertions**; its remaining failures are missing stepping APIs. Neither run has
environment blocks. These are selected suites, not a full-WPT baseline.

**533 native, 272 JavaScript, 110 DOM, 80 common, 31 parser, 10 WebIDL and 9
oracle tests pass**; six native and three JavaScript tests remain ignored.
Strict native Clippy, formatting, generated registry and signed sandbox probes
pass. Independent signed OIL forms outcomes are **39/39**, with all final server
oracles verified, no timeouts and clean fixture shutdown. Version 2 adds numeric/
calendar fields and six negative/recovery cases: it is not an identical-input
comparison with the historical 21-run suite. Observation/error smoke is **12/12**.

These are dirty-tree signed-worker diagnostics, not release or five-family
qualification. Stepping/value conversion APIs, pattern, editing length,
bad-input and textarea defaultValue remain open, alongside connected-window/
worker architecture and full conformance/security/performance gates.

## September 14 native validation in OIL observations

Native semantic nodes now carry an optional `validation` record with
`will_validate`, `valid`, `value_missing`, `type_mismatch` and `custom_error`.
These flags come directly from the live native DOM, never page getters or
authored attributes. A control blocks validation only when `will_validate` is
true and `valid` is false; disabled radios can still have a missing-value flag.
The record follows semantic node identity through observations and deltas and
is omitted for non-controls and backends without native evidence. Existing
v4/v6 node wrappers retain it through their flattened v3 representation.

Custom error text and control values are deliberately absent from this record,
avoiding a new path for secrets or page-authored instructions. Focus state now
also comes from native focus ownership. Full observations can inspect hidden
invalid controls; compact projections retain their normal visibility filtering.
Unchanged checkpoints reuse the mutation cursor; relevant mutations recompute
control/group-dependent state. Navigation resets the cache with the graph.

The record describes implemented native validity rules, not full HTML constraint
conformance: pattern, range, step, editing length and bad-input work remain.
It does not yet distinguish a canceled validation report from other invalid
state, and is not proof of five-workflow completion.

[Verification](../../benchmarks/evidence/validation-observation-native-2026-09-14.json),
source `c7b56b85…`: **531 native and 80 common-contract tests pass**, with six
native tests ignored. Strict Clippy, formatting and signed sandbox probes pass.
[Signed OIL outcomes](../../benchmarks/evidence/oil-validation-observation-signed-2026-09-14/summary.json)
remain **21/21**, all final server oracles verified and clean fixture shutdown.
The rejected required-field case exposes missing flags and native focus in CLI
JSON, while its server oracle records zero requests/receipts. The
[28-file WPT group](../../benchmarks/evidence/validation-observation-constraints-wpt-2026-09-14.json)
remains **11/28 files, 690 passing / 230 failing assertions**, all 690 prior passes
retained; zero environment blocks, runner time 11.564 seconds. Dirty-tree
diagnostics, not release evidence.
Existing OIL observation/error smoke remains **12/12**; it is not independent
task-completion evidence.

The first expanded delta regression failed because it read pending graph deltas
after the runtime's snapshot recorder consumed them. Its corrected checkpoint
assertion and full rerun pass; the intermediate build was stopped before signing.

## September 14 interactive validation events and focus

`reportValidity()` is now exposed on the seven native validation interfaces.
It and `requestSubmit()` share an interactive reporting phase: collect trusted
cancelable invalid events, skip canceled or no-longer-reportable controls, and
focus the first available unhandled control. Static `checkValidity()` does not
focus. Callback repair does not change the original false result, and focus
callbacks retain their own focus decisions.

The [same 28-file selection](../../benchmarks/evidence/interactive-validation-constraints-wpt-2026-09-14.json)
now has **690 passing / 230 failing assertions**, **82 gains with zero lost or
missing previous passes**. File passes remain **11/28**; the reportValidity file
has **82/130** assertions passing, with pattern/range/step failures remaining.
[requestSubmit](../../benchmarks/evidence/interactive-validation-requestsubmit-wpt-2026-09-14.json)
remains **10/10**. Runner elapsed time is **11.074 seconds** (progress interval
9.6 seconds), not a performance qualification.

[Verification](../../benchmarks/evidence/interactive-validation-native-2026-09-14.json):
**530 native, 272 JavaScript, 107 DOM, 31 parser, 10 WebIDL and 8 oracle tests
pass**; strict Clippy, formatting, generated registry and signed sandbox probes
pass. [Independent signed OIL forms](../../benchmarks/evidence/oil-interactive-validation-signed-2026-09-14/summary.json)
remain **21/21**, all final server oracles verified, clean fixture shutdown.
Source `78f4e869…` matches the signed worker manifest. Dirty-tree diagnostic only.
The existing OIL smoke corpus also remains **12/12**; this is observation/error
smoke, not independent end-state qualification.

Headless reporting currently uses focus and the live DOM validation message;
structured native validation-error diagnostics in OIL observations, reporting
unfocusable controls and cross-document qualification remain open. Pattern,
numeric/date/time and editing rules, broader workflow families and architecture/
release gates are not closed by these API gains.

Timing clarification: the previous validation-mutability report's 9.5 seconds
is its progress interval; the retained raw WPT runner records 11.543 seconds.

## September 14 validation eligibility and mutability

Native submit-button classification now serves activation and validation, including
image inputs excluded from `form.elements`. Button, fieldset, output and object
validation interfaces share the same owner-traced state. Required text controls
stop being missing when disabled; checkbox/radio/file flags remain independent of
their eligibility for submission checks. Readonly inputs are barred, and unnamed
radios do not acquire named-group required state.

The [same 28-file WPT group](../../benchmarks/evidence/validation-mutability-constraints-wpt-2026-09-14.json)
improves from **8/28 to 11/28 files**, and **566 to 608 passing assertions**.
**312 assertions still fail**. All **566 prior passes** were compared by file/name
and remain passing: **42 gains, zero lost or missing passes**, zero environment
blocks. Eligibility, datalist eligibility and custom-error files now pass fully.
[requestSubmit](../../benchmarks/evidence/validation-mutability-requestsubmit-wpt-2026-09-14.json)
remains **10/10**, and [email](../../benchmarks/evidence/validation-mutability-email-wpt-2026-09-14.json)
remains **8/8**.

[Verification](../../benchmarks/evidence/validation-mutability-native-2026-09-14.json),
source `79cb72b8…`: **529 native, 272 JavaScript, 107 DOM, 31 parser, 10 WebIDL
and 8 oracle tests pass**, with six native and three JavaScript tests ignored.
Strict native Clippy, formatting, registry and signed sandbox probes pass.
[Independent form outcomes](../../benchmarks/evidence/oil-validation-mutability-signed-2026-09-14/summary.json)
remain **21/21** with final server-oracle verification and clean fixture shutdown;
[smoke](../../benchmarks/evidence/oil-validation-mutability-smoke-2026-09-14/summary.json)
remains **12/12**. These are dirty-tree signed-worker diagnostics, not release or
five-family qualification.

Two intermediate mistakes were caught and repaired: a dropped non-element guard
broke two native focus/pagination tests; overbroad disabled-state suppression lost
seven previously passing WPT assertions. The intermediate evidence remains intact;
the final full native suite and assertion-by-assertion WPT comparison pass those checks.

Next: interactive reporting, pattern/numeric/date/time constraints and editing
semantics, alongside multi-step/framework forms and the remaining workflow families.
The broader architecture and release gates remain open.

## September 14 shared native validation foundation

Form submission, control validity APIs, selector matching and computed style now
share native validation checks. Input and textarea have live owner-traced validity
objects and custom errors. Email/URL values are sanitized natively, with `multiple`
reflection and attribute transitions. A dirty-value flag distinguishes user/script
edits from sanitization, survives cloning, and is cleared by reset.

The three previously failing selections now pass **3/3 files, 29/29 assertions**
(previously 8 passed / 21 failed):
[requestSubmit](../../benchmarks/evidence/shared-validation-requestsubmit-wpt-2026-09-14.json),
[typeMismatch](../../benchmarks/evidence/shared-validation-type-mismatch-wpt-2026-09-14.json),
[email](../../benchmarks/evidence/shared-validation-email-wpt-2026-09-14.json).
[Independent OIL outcomes](../../benchmarks/evidence/oil-shared-validation-signed-2026-09-14/summary.json)
remain **21/21**, with three concurrent parents and final server-oracle verification.
[Existing smoke](../../benchmarks/evidence/oil-shared-validation-smoke-2026-09-14/summary.json)
remains **12/12**.

[Verification](../../benchmarks/evidence/shared-validation-native-2026-09-14.json),
source `35b860f0…`: **528 native, 272 JavaScript, 106 DOM, 31 parser, 4 CLI,
10 WebIDL and 8 oracle tests pass**. Six native and three JavaScript tests remain
ignored. Strict native Clippy, formatting, registry, Ruff and signed sandbox probes
pass. These are source-matched signed-worker diagnostics from a dirty tree, not
release or performance qualification.

A broader [28-file constraint diagnostic](../../benchmarks/evidence/shared-validation-constraints-wpt-2026-09-14.json)
passes **8/28 files, 566 assertions passed / 354 failed**, zero environment blocks.
There is no equivalent pre-change run. Remaining work includes `reportValidity`
(130 failed assertions in its dedicated file), additional control surfaces,
disabled/readonly/type-specific eligibility, patterns, numeric/date/time ranges
and steps, and user-edit/length/default-value behavior. The emoji editing failure
still needs browser-versus-automation-path investigation. Some unimplemented
flags return false: selected files expecting false are not proof of those algorithms.

The validation foundation is shared, **not complete**. Multi-step/framework form
outcomes, the other four workflow families, architecture and release gates remain open.

## September 14 independent form outcomes

[Signed-worker outcome replay](../../benchmarks/evidence/oil-forms-outcomes-signed-2026-09-14/summary.json)
passes **21/21**: seven controlled scenarios, three samples each, three concurrent
parents, default isolated workers. The private-pipe server oracle checks exact
Unicode/repeated fields and submitter, disabled-field exclusion, authorized upload
bytes/hash, invalid and cancelled submission blocking/recovery, and receipt reload.
Final snapshots are taken after HTTP handlers finish; fixture shutdown is clean.
This is independent task-outcome evidence for these fixtures, **not qualification
of the entire forms family or the five-family goal**.

The new harness exposed and drove fixes for non-ASCII OIL strings, a debug CLI
argument-graph panic, invalid email submission, required-file validation reading
text instead of selected files, and same-URL no-fragment links skipping reload.
Historical failed runs remain intact. Form submission now validates these cases,
but the broader validation interfaces are not yet unified.

[Verification](../../benchmarks/evidence/forms-outcomes-native-2026-09-14.json), source
`cb539bbd…`: **527 native, 272 JavaScript, 104 DOM, 31 parser, 4 CLI, 10 WebIDL,
and 8 oracle tests pass**; six native and three JavaScript tests remain ignored.
Strict native/CLI Clippy, formatting, registry, Ruff and signed sandbox probes pass.
[Existing OIL smoke](../../benchmarks/evidence/oil-forms-fixes-smoke-2026-09-14/summary.json)
remains **12/12**. Source matches the worker manifest, but the tree is dirty and
these results are not clean-release or performance qualification.

Three new WPT diagnostics remain failing, with zero environment blocks:

- [requestSubmit](../../benchmarks/evidence/forms-outcomes-requestsubmit-wpt-2026-09-14.json):
  **8/10 assertions**; `input:invalid` and form `:invalid` selector truth fail.
- [typeMismatch](../../benchmarks/evidence/forms-outcomes-type-mismatch-wpt-2026-09-14.json):
  **0/11 assertions**; the input `validity` attribute is missing.
- [email](../../benchmarks/evidence/forms-outcomes-email-wpt-2026-09-14.json):
  **0/8 assertions**; `multiple` reflection, sanitization and validity APIs fail.

Next: one native constraint-validation model shared by submission, validity APIs
and selectors; email sanitization/reflection; multi-step/framework form outcomes.
Other workflow families, architecture work, full Core WPT and release gates remain open.

## September 14 bounded platform request admission

Request bursts now queue at the realm/host boundary while retaining **32 active
operations and 32 live HTTP streams**. Queue bounds are **256 requests and
16 MiB serialized payload**, with 288 total outstanding admission records.
Reads/aborts can progress past stream-blocked starts. Queued cancellation never
dispatches; active cancellation retains its slot until acknowledgment and cleans
up headers that race cancellation. Parent abort is idempotent only after endpoint
authority validation. Retirement revokes the host and drops queued payloads.

[Signed verification](../../benchmarks/evidence/platform-admission-native-2026-09-14.json),
source `0438d0b2…`: **525 native and 271 JavaScript tests passed**, zero failures;
six native and three JavaScript tests remain ignored. Five admission tests include
a fully saturated **288-request** burst, cancellation races, count/byte bounds,
failure delivery and retirement. DOM **103**, WebIDL **10**, strict Clippy,
format/registry/diff checks and signed sandbox probes pass.

The [MIME WPT selection](../../benchmarks/evidence/xhr-admission-mime-wpt-2026-09-14.json)
now passes **9/9 files and 90/90 assertions**: all 42 admission failures are fixed.
[Callback WPT](../../benchmarks/evidence/xhr-admission-callback-wpt-2026-09-14.json)
remains **8/9**, with the cross-realm startup/exception timeout retained.
[OIL smoke](../../benchmarks/evidence/oil-admission-signed-2026-09-14/summary.json)
remains **12/12**, zero timeouts, not independent task-completion evidence.

A new [25-file XHR abort selection](../../benchmarks/evidence/xhr-admission-abort-wpt-2026-09-14.json)
passes **8/25 files**, with **9 assertions passed and 19 failed**. It exposes
statusText, upload/ProgressEvent, abort/re-entrant-open sequencing, window.stop,
navigation cancellation and synchronous-XHR gaps. No same-selection pre-change
run exists; this is not a regression comparison. All three selections have zero
environment blocks. Results remain dirty-tree diagnostics, not general XHR or
full-WPT conformance.

Still open: cross-realm admission/initial iframe routing, retained Fetch bodies
at the stream limit, five-family independent outcome/persistence qualification,
the remaining architecture plan, full Core WPT and clean-release/performance gates.

## September 14 queued upgrades, retained identity and browser MIME parsing

Custom-element upgrades now share the reaction queue with callbacks rather than
constructing the entire subtree first. Upgrade construction preserves the
existing JavaScript wrapper, including private fields, expandos and WeakMap keys.
XHR MIME parsing now uses the browser algorithm instead of strict header parsing.

[Signed verification](../../benchmarks/evidence/queued-upgrades-browser-mime-native-2026-09-14.json),
source `8b8f94bc…`: **525 native and 266 JavaScript tests passed**, zero failures;
six native and three JavaScript tests remain ignored. DOM **103**, WebIDL **10**,
strict Clippy, format/registry/diff checks and signed sandbox probes pass.
[OIL smoke](../../benchmarks/evidence/oil-queued-upgrades-signed-2026-09-14/summary.json)
remains **12/12**, zero timeouts, not independently verified task completion.

The [callback selection](../../benchmarks/evidence/custom-element-queued-upgrades-wpt-2026-09-14.json)
is **8/9 files**, including fragment parsing **8/8**. All 186 reported assertions
pass, but the ninth file still times out with zero assertions; it remains in the
denominator. The [MIME selection](../../benchmarks/evidence/xhr-browser-mime-wpt-2026-09-14.json)
is **8/9 files**, 48 assertions passed and 42 failed at the runtime's existing
32-pending-operation admission limit. Neither run has environment blocks.

**Correction to the earlier MIME summary below:** the 68 failures comprised
**26 parsing mismatches and 42 admission-limit failures**, not 68 parsing
mismatches. The 26 mismatches are repaired. Direct execution of the production
parser passes all **74 hand-written and 881 generated** pinned MIME cases; these
955 parser checks are separate from signed network WPT results.

Next: bounded asynchronous request queuing without raising active-operation or
memory limits; cross-realm startup/exception reporting. Full Core WPT, native
iframe ownership, five-family outcome qualification and release gates remain open.

## September 14 native XHR document responses

Native XHR now supports MIME overrides and cached inert HTML/XML responses,
with document URL, encoding and content-type metadata. Member-level WebIDL
exposure keeps responseXML off worker globals. The signed build also includes
the prior clone/import upgrade repair.

[Verification](../../benchmarks/evidence/xhr-document-native-2026-09-14.json), source
`f24a6027…`: **523 native and 263 JavaScript tests passed**, zero failures;
six native and three JavaScript tests remain ignored. DOM **103**, WebIDL **10**,
strict Clippy, formatting, registry consistency and signed sandbox probes pass.
[OIL smoke](../../benchmarks/evidence/oil-xhr-document-signed-2026-09-14/summary.json)
is **12/12**, zero timeouts, not independent end-state qualification.

The same callback selection improves to **7/9 files**: adoption **71/71**,
connection/disconnection **40/40 each**, attributes **13/13**. Fragment parsing
and cross-realm exception reporting remain. MIME override selection is **8/9**;
the Blob file exposes **68 MIME parsing/serialization failures**. Neither run
has environment blocks. These are selected dirty-tree diagnostics, not a full
WPT baseline or release qualification. Browser MIME parsing is the next repair.

## September 14 agent-wide custom-element checkpoints

Reactions now checkpoint across defining realms sharing one native DOM. Adopted
elements keep their definitions; destination registries cannot reconstruct them.
Allocation notifications are distinct from insertion. Automatic upgrades use
the document registry, while explicit registry.upgrade can upgrade inert roots.

[Verification](../../benchmarks/evidence/custom-element-agent-native-2026-09-14.json)
preserves two source identities. Signed source `a1b866f9…` passes sandbox checks
and [12/12 OIL smoke tasks](../../benchmarks/evidence/oil-custom-agent-signed-2026-09-14/summary.json),
zero timeouts. All eight React observations have zero `:host` CSS leakage, while
add/filter/search/delete counts remain 5/6/4/2/6/1/6/5.

The [same callback WPT selection](../../benchmarks/evidence/custom-element-agent-callback-wpt-2026-09-14.json)
remains **3/9 files**, zero environment blocks: adoption **61/71**, connection and
disconnection **35/40 each**, nested callbacks **8/8**. Attribute callbacks
regressed to **12/13** because detached clones had relied on allocation records
to trigger upgrade. Current source explicitly upgrades clone/import results and
passes the new regression; that final repair has **not yet been signed/replayed**.

Current native source `137b1c980d10460ff38cb614ccc8e6bac38f55dabca87dd6ddda1ec461f4227b`:
**523 passed, zero failed, six ignored (141.56 seconds)**; **12 focused custom
element tests**, strict Clippy, format and diff checks pass. Remaining callback
failures include XHR document variants, fragment timing and cross-realm exception
reporting. Five-family independent outcome qualification and release gates remain
open; OIL smoke is not proof of persistence or general website conformance.

## September 14 semantic source-text filtering

Accessible text traversal now excludes script/style/template source, including
inside shadow roots; live-region compatibility names and caption/legend labels
use the same traversal instead of raw DOM textContent. Slotted rendered content
remains, and DOM source text is preserved rather than sanitized away.

[Native verification](../../benchmarks/evidence/semantic-source-text-2026-09-14.json):
**520 passed, zero failed, six ignored (141.62 seconds)**; all **17 semantic
regressions** and strict Clippy/format/diff checks pass. Source
`33b7bb48bf823a5c1273a3086f937626ea7fdcf71dc1ae8404b5eb9b9674aa34`.
This change is **not yet signed/replayed**. The prior 12/12 OIL smoke result below
belongs to source `d195ffcc…`, not this source. Full accessibility conformance,
cross-realm work and independent workflow outcome qualification remain open.

## September 14 adoption and nested reaction queues

Ownership notifications now cover implicit insertion adoption and affected
descendants, including shadow trees, attributes and template contents. Reactions
use per-element queues with nested invocation and bounded callback/depth budgets.
Observed attributes accept iterables via shared native DOMString conversion.

[Verification](../../benchmarks/evidence/custom-element-adoption-native-2026-09-14.json):
**519 native tests passed, zero failed, six ignored (151.63 seconds)**; **103 DOM
tests passed**, strict Clippy, formatting, registry checks and signed sandbox
probes pass. Source `d195ffcc809df3b0945089687ad938abc22c6dc1cb09c3665a83a2146dc27a86`.

[Signed OIL smoke](../../benchmarks/evidence/oil-custom-adoption-signed-2026-09-14/summary.json)
is **12/12, zero timeouts**. The React task now explicitly targets the new record's
Delete button; ambiguity protection is unchanged. Its eight observations show
5/6/4/2/6/1/6/5 Delete buttons through add/filter/search/delete, with the new task
absent initially and after deletion. This is browser-observation evidence, not an
independent persistence oracle or qualification of the five workflow families.

The [same nine callback WPT files](../../benchmarks/evidence/custom-element-callback-adoption-wpt-2026-09-14.json)
now pass **3/9**, zero environment blocks: attribute callbacks **13/13** (was12),
adoption **51/71** (was2). Cross-realm callbacks, a stale-node failure in the
upstream nested-callback case, fragment upgrade timing and XHR variants remain.
Shadow style text leaking into generic semantic names is also visible in the
React observations and requires repair. This remains dirty-tree diagnostic
evidence, not a release candidate or full conformance baseline.

## September 14 native custom-element reaction foundation

Native definitions, construction targets and captured lifecycle callbacks now
replace the JavaScript custom-element registry and DOM insertion overrides.
Detached subtree connection/reconnection and explicit adoption regressions pass;
inert-document connectivity is separate from active rendering/script eligibility.
Host reaction checkpoints are native and deadline-bounded.

[Exact-source native verification](../../benchmarks/evidence/custom-elements-native-2026-09-14.json):
**517 passed, zero failed, six ignored (148.74 seconds)**; DOM **103 passed**,
strict Clippy, formatting and generated registry checks pass. Signed build and
sandbox probes pass. Source `051cec7db015bed8a8889f1fae25b6ecde8ba7456f52ae53db7355dc2b6f5e7f`.

[Signed OIL smoke](../../benchmarks/evidence/oil-custom-elements-signed-2026-09-14/summary.json)
remains **11/12, zero timeouts**, but React shadow controls now initialize and the
case reaches an ambiguous `Delete` action with six candidates. Correct the task
target explicitly; do not weaken ambiguity protection. These are smoke results,
not independent end-state verification.

[Selected callback WPT](../../benchmarks/evidence/custom-element-callback-wpt-2026-09-14.json):
**2/9 files pass, zero environment-blocked (12.5 seconds)**. Constructor/callback
retention and template connection pass. Basic connection and disconnection each
pass 31/40 assertions, attribute changes 12/13. Implicit adoption, fragment upgrade
timing, nested reaction lifetime/order, cross-realm behavior and XHR document
variants remain unresolved. Native registry implementation is a foundation, not
complete custom-element conformance. All five workflow families remain unqualified.

## September 14 native MessageChannel delivery and transfer

Native channel endpoints now enqueue serialized messages onto platform tasks,
with a microtask checkpoint between deliveries. MessageEvents have native brands,
trusted delivery and frozen transferred-port arrays. Buffer/port transfer validates
before detachment, moves queued messages, revokes sender wrappers and deserializes
in the destination realm. Start/onmessage enable delivery; close disconnects while
preserving already queued messages; execution retirement cancels delivery. Queues
are capped at 1024 messages and 32 MiB of accounted payload per endpoint.

[Native verification](../../benchmarks/evidence/message-channel-native-2026-09-14.json):
**514 passed, zero failed, six ignored (153.84 seconds)**, including eight channel
regressions and destination-realm/navigation retirement. Existing structured-clone
and MessageEvent regression groups also pass (two each), as do strict Clippy,
registry consistency, formatting and diff checks. Source:
`8c72b171d35667b0ef38f6aa942da8ff55c43e81535651a50ec1b08f7751a4c4`.

The signed build passes signature and sandbox probes.
[Controlled OIL replay](../../benchmarks/evidence/oil-message-channel-signed-2026-09-14/summary.json)
is **11/12 passing, zero timeouts**, up from 10/12: React navigation/interactivity
now passes. The shadow-DOM React case still fails with StaleSemanticRef. Its
[startup inspection](../../benchmarks/evidence/oil-message-channel-signed-2026-09-14/react-startup-diagnosis.json)
shows a rendered React light DOM, while [component inspection](../../benchmarks/evidence/oil-message-channel-signed-2026-09-14/react-shadow-diagnosis.json)
shows upgraded custom-element instances without shadow roots. The current JS
custom-element bridge only queues descendant connection when upgrade returns a
newly upgraded node; descendants upgraded while detached are skipped when their
containing subtree is later connected. Native custom-element connection/reaction
ownership is the next workflow implementation target, not forced React rendering.

Pinned signed WPT results (persistent isolated-prewarmed parents, four jobs):

- [Message-channel window inventory](../../benchmarks/evidence/message-channel-wpt-2026-09-14.json): **5/14 files pass**, zero environment-blocked; includes all 14 selected cases, not only the passing basics.
- [Channel cloning/transfer window inventory](../../benchmarks/evidence/message-channel-transfer-wpt-2026-09-14.json): **10/11 files pass**, zero environment-blocked. Cross-site transfer fails on a null postMessage target.

The failing channel cases include cross-document delivery, frame lifetime,
GC/BFCache, missing userActivation/service-worker surfaces and dedicated-worker
plumbing. Two remote-context cases produce invalid duplicate completion counts
after a `reject is not defined` error; their partial assertions are not credited
as passing files. A runner `harness` failure-domain tag on timeout is not proof
that the browser harness caused the underlying feature failure. Cross-process
port transport and complete GC/lifetime semantics remain unqualified. These are
dirty-source diagnostics, not clean release acceptance or five-family completion.

## September 14 native script text and React scheduling diagnosis

Implemented HTMLScriptElement.text in the WebIDL registry and native bindings:
the getter reads direct text children; the setter replaces children and uses
existing script preparation and Trusted Types enforcement. Existing generated
member IDs are preserved. [Native verification](../../benchmarks/evidence/script-text-binding-2026-09-14.json):
**506 passed, zero failed, six ignored (149.21 seconds)**; both new regression
tests, all 10 WebIDL tests, registry consistency, strict Clippy and formatting pass.
Source: `7395b34f73d3d3e96665b1aac0e50067c2ba7e1dfbc718701e06ebb92f50a80c`.

The signed build passes signature and sandbox probes, but its
[controlled replay](../../benchmarks/evidence/oil-script-text-signed-2026-09-14/summary.json)
remains **10/12, zero timeouts**. Shopping passes; both React cases still fail.
The [signed inspection](../../benchmarks/evidence/oil-script-text-signed-2026-09-14/react-diagnosis.json)
now confirms script.text updates native textContent and generated JavaScript is
inserted. Manually transforming again is not a valid startup test: it attempts to
redefine custom elements and can mask the original state. The simple React fixture
can [synchronously render a diagnostic button](../../benchmarks/evidence/oil-script-text-signed-2026-09-14/react-flush-exception.txt),
but this is diagnostic JavaScript, not OIL task completion.

Next concrete implementation barrier: MessageChannel creates platform ports,
but MessagePort.postMessage currently delivers no message and start/close are
no-ops. React's channel-based scheduler therefore cannot make progress through
that path. Implement native entanglement, structured serialization, queued task
delivery, start/close and realm retirement; do not substitute synchronous calls
or framework-specific forced rendering. Dynamic-script exceptions also currently
collapse into misleading allocation errors and need proper reporting. Full WPT,
independent workflow outcome oracles and the remaining architecture gates stay open.

## September 14 signed script loading, raw-text serialization and spatial freshness

Signed source `aa51dc65...` passes signature and sandbox probes. Its
[controlled replay](../../benchmarks/evidence/oil-script-budget-signed-2026-09-14/summary.json)
is **9/12 passing, zero timeouts**, not the preceding 10/12: shopping failed with
missing authoritative spatial geometry, including on an isolated retry. React
still fails, but Babel now loads. [Diagnostic inspection](../../benchmarks/evidence/oil-script-budget-signed-2026-09-14/react-diagnosis.json)
found an empty root and unequal script `innerHTML`/`textContent`. Manually asking
the author's Babel library to transform script tags reports a syntax error at
an arrow escaped as `=&gt;`. That diagnostic is not an OIL completion action.

Two subsequent fixes are under verification: HTML text serialization preserves
literal data under the seven unconditional raw-text parent elements, with an
HTML namespace guard; spatial target resolution flushes layout before matching,
using the unchanged-input cache. This follows the
[HTML fragment serialization rule](https://html.spec.whatwg.org/multipage/parsing.html#serialising-html-fragments)
for those parents; scripting-dependent noscript behavior is not qualified here.
Author script preparation and post-callback spatial resolution regressions pass,
along with **103 DOM tests** and strict Clippy. Full native verification passed:
**504 passed, zero failed, six ignored (151.22 seconds)**.
[Native diagnostic evidence](../../benchmarks/evidence/raw-script-spatial-freshness-2026-09-14.json).
New source: `8f69007ca226beb2ec4e4809b544c3389b58569b76c54d765c893bd6743e841b`.
The signed build subsequently passed signature and sandbox probes. Its
[full controlled replay](../../benchmarks/evidence/oil-raw-script-spatial-signed-2026-09-14/summary.json)
passes **10/12, zero timeouts**: shopping now passes; interactivity and React
still fail. These are smoke outcomes, not independently verified task end states.
[Signed diagnosis](../../benchmarks/evidence/oil-raw-script-spatial-signed-2026-09-14/react-diagnosis.json)
confirms equal script innerHTML/textContent but an empty React root. Babel's
generated `script.text` assignment only creates an expando: native textContent
stays empty. A native HTMLScriptElement.text binding fix is now under verification.

## September 14 bounded context responses and page-script budgets

Parent context replies now serialize into one bounded buffer under the channel
writer lock before any socket write. JSON escape expansion and the final LF
count toward the 24 MiB frame limit. An oversized result is replaced by a small
error preserving route and correlation; it does not emit partial JSON or corrupt
the next response. If even that error cannot be encoded, the channel is closed.

Page-script loading now permits up to 8 MiB of received bytes and independently
up to 8 MiB of decoded source, aligning with the default resource-cache entry
ceiling. The transport frame budget remains independent. Dedicated-worker script
limits are unchanged. CSP, origin, redirect, cancellation and broker policy
checks remain in place; no native Babel transpilation shortcut was reintroduced.

All 19 context-service tests pass, including exact frame bounds, escape
expansion, route preservation, a valid reply following an oversized response,
decoded-text expansion, 2 MiB bundle loading and over-budget rejection. Strict
V8 library/test Clippy, formatting and diff checks pass. Full native verification:
**502 passed, zero failed, six ignored (142.59 seconds)**.
[Diagnostic evidence](../../benchmarks/evidence/context-response-script-budgets-2026-09-14.json).
Source: `aa51dc65b993e9a85442c3ebba4f4947d937c3986ecc0aa460a5307dbfc51855`.
Signed React replay is still required; successful author-script execution and
workflow completion are not implied by lifting the earlier loading barrier.

## September 14 layout-input reuse

Pointer actions still flush layout at the action boundary, but a page now reuses
its output when the complete produced `LayoutInput` and document generation are
unchanged. One input is retained per page. Changes in focus, validation, styles,
viewport, media, fonts or DOM data represented by that input invalidate reuse.
Empty-geometry panic fallbacks are not cached. This does not extend the set of
CSS/layout features currently implemented.

The timer/ancestor recovery regression remains passing. A new test verifies
unchanged-input reuse, focus/style invalidation and rejection after disabling
pointer events. **498 native tests passed, zero failed, six ignored (143.91
seconds)** on source `85a15f5aa2c394e01534b0218957499ed5f4c4a20df6bcb175d18ec14f5163a8`.
Subsequent test-only removal of redundant `clone()` calls produced source
`66317df66fc4963fd1e69ff4f00c2d4272185cafb818e04b7974d9eafc014745`;
strict Clippy, formatting and diff checks pass. Signed probes and the controlled
replay pass at the same **10/12 workflows, zero timeouts**. Three signed-worker
diagnostics measured **124.47–126.31 warm actions/sec**, compared with the prior
74.86/sec single run, using the identical benchmark executable. First readiness
was **98.64–103.24 ms**. [Qualification and raw artifacts](../../benchmarks/evidence/oil-layout-reuse-signed-2026-09-14/qualification.json)
preserve separate test/source/parent/worker/harness identities. This improvement
does not meet the 300/sec target or full performance qualification. Recovery
remains correct, but action-boundary overhead requires further profiling;
React loading, independent task oracles, WPT and architecture gates remain open.

## September 14 signed recovery qualification and performance regression

Signed source `aec0e819cb0a19a2570f94e1b5b4d60bb69ad467922ca55462350c2ab2fcdb1f`
passes the build's signature/designated-requirement, sandbox-denial,
DOM-mutation and authorized-network probes. The controlled replay now passes
**10/12 workflows, zero timeouts**, including the full targeting/recovery
script (10.821 seconds). Both remaining failures are React fixtures; the
previously diagnosed parent script resource cap still applies.
[Raw replay and signed identities](../../benchmarks/evidence/oil-controlled-recovery-signed-2026-09-14/summary.json)
include per-task hashes, outputs, errors, runtime metadata and worker manifest.
This is dirty-source smoke evidence, not independent end-state qualification.

The action-boundary layout flush has a substantial performance regression:
[three-page/1,000-action diagnostic](../../benchmarks/evidence/oil-controlled-recovery-signed-2026-09-14/isolated-diagnostic.json)
measured **74.86 actions/sec**, first readiness **127.54 ms**, and cached isolated
readiness **41.36 / 35.12 ms**. These are single-run diagnostics, not P50/P95
acceptance statistics. The next repair must retain correct hit testing while
avoiding recomputation when layout inputs are unchanged. Benchmark executable
SHA-256: `2d7351f49eac4bf82d9fcfd35a0555fbee8729df37f2349291aacf875c781b81`.

Before increasing the React script budget, also bound parent context response
serialization: its current `serde_json::to_writer` path writes directly to the
socket, while the peer's reader enforces 24 MiB. Loading, decoded text and
serialized-frame budgets need coordinated tests. No limit was increased here.

## September 14 recovery wait and action-boundary repair

A native OIL regression reproduced the intermittent-button failure. Three
issues were separated: the normalizer treated a whitespace-prefixed CSS ID as
an inline comment inside `css(...)`; runtime selector waits discarded ancestor
conditions; and action hit testing could retain pre-callback layout after the
DOM changed. The normalizer now tracks parentheses, runtime matching shares
the DOM arena's bounded selector implementation, and pointer actions flush
layout at their execution boundary. DOM matching now includes compound negation
and substring attributes while retaining whitespace inside attribute values.
This is not a claim of complete CSS selector conformance.

The OIL ancestor-class/timer recovery regression passes (0.20 seconds), along
with 29 core and 102 DOM unit tests. Broader verification passed: **497 native
tests, zero failed, six ignored (151.79 seconds)** and strict native Clippy.
[Diagnostic evidence](../../benchmarks/evidence/oil-recovery-selectors-2026-09-14.json)
records source and test-binary hashes. Formatting and diff checks pass.
Signed/WPT replay and throughput measurement are still required; the extra
action-boundary layout flush may affect warm performance. The signed 9/12
workflow result below predates these browser fixes.

## September 14 corrected workflow replay and React load diagnosis

The same signed `a2117aab...` binaries with corrected task scripts now pass
**9/12 controlled smoke workflows, zero timeouts**. The shipping workflow targets
the actual express radio instead of its equally named clickable label. The tree
workflow no longer collapses the initially expanded Documents group before
selecting report.pdf, but still fails later with a non-interactable target.
[Full raw replay](../../benchmarks/evidence/oil-controlled-task-corrections-2026-09-14/summary.json)
records binary and individual task hashes. These are post-build task changes,
not a newly signed source snapshot or independently verified task completion.

[React observation](../../benchmarks/evidence/oil-controlled-task-corrections-2026-09-14/react-load-diagnostic.jsonl)
reports the immediate load failure: the parent context script loader rejects
the Babel resource at its hard-coded 1,048,576-byte response cap. Direct evaluation
confirms React and ReactDOM exist while Babel does not. This is an enforced
resource budget, not evidence that Babel executed and failed. Script budget,
decoding expansion and IPC frame bounds need joint review before changing this
limit; no security limit has been relaxed. React runtime compatibility remains
unproven beyond that loading barrier.

The remaining 09 failure was isolated to its second intermittent-button click.
Its readiness selector incorrectly checked `.disabled` on the button while the
fixture toggles that class on the wrapping container. Correcting that selector
still produces `NotInteractable` for node 111, so this is not declared resolved;
the next check must distinguish wait/selector behavior from stale inherited
`pointer-events` geometry after container-class removal.

## September 14 signed workflow replay and task disambiguation

Signed source `a2117aab396d5b39a338e75bf56bfab737c02a4db58cddd6bdb63084dac33022`
replayed all 12 controlled OIL scripts: **6 passed, 6 failed, zero timeouts**.
[Raw diagnostic results](../../benchmarks/evidence/oil-controlled-script-data-2026-09-14/summary.json)
preserve all outputs and errors. Strict bundle signature verification passed.
This is diagnostic smoke, not end-state or clean-source release qualification.
React controls remain unavailable; removal of the native Babel shortcut is not
a React compatibility fix. Prefix replay isolated further failures to the
partial search placeholder (07 line 57), Express Shipping ambiguity (08 line 49),
and report.pdf targeting (09 line 73).

Subsequent task-only corrections scope Add to Cart to Laptop and use the full
search placeholder. Both corrected scripts (03 and 07) exit successfully against
the same signed binaries; the server cart independently contains Laptop,
quantity 1. This does not prove persisted checkout/order completion. The task
edits postdate the signed source snapshot and are not included in the 6/12 count.
Shipping label/control ambiguity and file-link targeting remain under investigation.

## September 14 author-script data blocks and fixture pinning

The React failure investigation found a nonstandard browser shortcut: native
execution of `text/babel` through `Babel.transform`. It is removed from parser
and dynamic-script paths. Data blocks remain inert until author code prepares
JavaScript. An unrecognized dynamic type no longer marks the element already
started; after author transformation/reinsertion a classic script executes once.
This follows the [HTML script model](https://html.spec.whatwg.org/multipage/scripting.html),
not a React-specific bypass. The two React fixtures now explicitly request
React 18.3.1 and Babel 7.28.3, consistent with the existing resource baseline;
their unversioned Babel URL had drifted to 8.0.5.

[Diagnostic evidence](../../benchmarks/evidence/script-data-blocks-2026-09-14.json):
**496 native V8 tests passed, zero failed, six ignored (144.96 seconds)** and
**101 DOM tests passed**. The new regression covers inert parser/dynamic data
blocks, no native transpiler invocation, and author preparation/exactly-once
execution. Source is `a2117aab396d5b39a338e75bf56bfab737c02a4db58cddd6bdb63084dac33022`.
Signed workflow replay is still required; React boot and five-use-case readiness
are not claimed from this script-processing regression. The next signed build
must include both targeting alternatives and these changes before comparing
against the preserved 6/12 controlled smoke result.

## September 14 OIL targeting alternatives

Declarative text targeting now admits exact content labels, IDs and placeholders
below accessibility-name matches, without replacing accessibility names. Hints
exclude hidden content and form-control values, follow mutation/generation
cleanup, and retain ambiguity rejection. Missing declarative graph targets no
longer masquerade as stale references; explicit references/aliases retain stale
rejection. CSS selector error mapping is still a separate follow-up.

The cart regression uses the existing fixture through OIL: `+` near Wireless
Mouse increments that product to 2; `coupon-code` receives SAVE10. Name integrity,
hidden/password exclusion, empty-query rejection, mutation and duplicate-target
controls pass. [Evidence](../../benchmarks/evidence/oil-target-hints-2026-09-14.json):
**495 native V8 tests passed, zero failed, six ignored (145.91 seconds)**, source
`8c95010518f6f0b5a1091c711d1dae244be7f5d5987a675af7229b80b2c3ca93`.
This source has not yet been signed or replayed across the full OIL corpus. The
signed 6/12 diagnostic below remains attributed to its earlier source. The
five-use-case objective and remaining architecture gates remain open.

## September 14 OIL workflow-driven visibility repair

The active objective is functional native OIL coverage for five main agentic
use-case families. [Acceptance requirements](OIL-AGENTIC-READINESS-2026-09-14.md)
define provisional research, forms/uploads, shopping, authenticated SaaS and
booking targets without treating narrow fixture success as family completion.

After restoring the fixture server's lockfile dependencies, the signed controlled
corpus passed the first static smoke and failed forms at modal dismissal. Native
regression analysis proved that submission revealed the modal but its controls
remained hidden in the semantic projection. Ancestor visibility-related attribute
mutations now refresh descendants, layout visibility updates compact membership,
and dismissal excludes hidden fallback controls. The regression verifies normal
OIL input, submission and dismissal without a JavaScript action bypass.

[Diagnostic evidence](../../benchmarks/evidence/oil-modal-visibility-2026-09-14.json):
**493 V8-native tests passed, zero failed, six ignored (142.11 seconds)** on source
`2032dd61a9ebe21de8ec43f2a250b82847c7d530b5ddf0e45a8c2a9ee447ef52`.
The same source now passes the signed build/security probes, and the forms OIL
script exits successfully. The full twelve-script diagnostic replay has **6/12
smoke passes**, zero outer timeouts. Raw outputs, task hashes and stderr are in
[the preserved report](../../benchmarks/evidence/oil-controlled-2026-09-14/summary.json).
Failures: ambiguous Add to Cart (03); Babel loading (04, 10); reported stale
references (07, 08, 09). For 08, the first failure is missing text resolution:
the script requests `+` while the accessible name is `Increase quantity`; the
resolver currently maps NotFound to StaleSemanticRef. Do not bypass genuine stale
reference rejection or silently choose ambiguous targets to make these pass.
Next gates: repair target resolution/loading, add independent persisted-state
oracles, and rerun. Smoke command success does not establish all application
outcomes; checkout currently logs success without persisting an order. No
five-use-case, full-WPT or P0 completion is claimed.

## September 13 response-frame limits and fail-closed retirement

The parent checks the size of each response before draining it, including a
newline-terminated response. Previously finding the delimiter returned the frame
before checking its size. The delimiter now counts consistently against incoming
and outgoing wire limits; coalesced frames remain independently bounded.
WorkerClient itself kills/reaps protocol-failing, crashed and limit-breaching
peers, including malformed JSON and unexpected EOF, rather than depending on
WorkerPage to discard them later.

[Exact-source diagnostic evidence](../../benchmarks/evidence/browser-v10-frame-retirement-2026-09-13.json):
**196 non-V8 native tests passed**, including 19 worker-client tests and three new
regressions. Strict non-V8 library/test Clippy, formatting and signed v10 build
probes pass. Source snapshot:
`f23777798c3578b44813c8ca28805afd5d26ef2ae219cd2ce6b6c01c8058a391`.
The full V8 suite and WPT were not rerun for this snapshot; their preceding results
below remain separately attributed. This does not close P0/R6 or the initial
iframe-realm compatibility gap.

## September 13 authenticated root-binding routing (preceding worker v10 snapshot)

Worker, context-service and network IPC now carry parent-issued root-binding
credentials. Scoped clients retain immutable grants; reset/rebind replaces them.
Parent dispatch rejects missing, forged and stale authority, response correlation
checks the expected authority, and cancellation is scoped by route plus request ID.
Wire versions are worker **10**, context service **11**, and network broker **5**.
Current build/validation entrypoints require v10; historical evidence is unchanged.

[Source-bound diagnostic evidence](../../benchmarks/evidence/browser-v10-routing-2026-09-13.json):
**489 native tests passed, zero failed, six ignored (141.12 seconds)**. Strict V8
library Clippy, formatting, 140 harness/tooling tests (22 skipped), and ARM64 Linux
isolated release compilation pass. The signed v10 bundle passes signature,
designated-requirement, sandbox-denial, DOM-mutation and authorized-network probes.
A three-lease recycled smoke confirms JavaScript-marker erasure. This is not an
exhaustive erasure proof or performance acceptance run.

Signed diagnostic WPT results: timers **10/13**, adoption **3/3**, templates
**8/67**, with zero environment-blocked files. These family totals are unchanged;
they do not constitute a full-WPT baseline. Timer failures remain cross-realm
error-reporting timeouts and the missing initial iframe window. The initial
native-suite failure is preserved separately: its image fixture read load-handler
state before asynchronous image completion. It now awaits the existing Load
lifecycle before making unchanged assertions; the full corrected suite passes.

**P0/R6 remain incomplete.** This credential covers a root-page binding, not each
navigation or child environment. Remaining work includes per-document grant
rotation, multi-environment routes and WorkerProcessSession ownership, initial
iframe realms/custom-element lifecycle, W0 upstream product integration, and
clean-source full qualification. Connected-window placement is not activated.
See the [routing threat model](BROKER-ROUTING-THREAT-MODEL-2026-09-13.md) for the
implemented boundary and residual gates. The signed source snapshot is
`09bcaab42b3a465f6cad0a3b923b81b2e99ac6be9e5048dbce0ea9b4bddfc8a4`;
this remains dirty-tree diagnostic evidence, not a release candidate.

## September 13 timer-task lifecycle follow-up (historical v9 snapshot)

Native timer-task nesting now applies the nested timeout clamp without carrying
that nesting into microtasks. Intervals reinitialize after callback/microtask
completion, preserving their public handle while cancellation targets the current
scheduler handle. Cancellation from a microtask prevents reinitialization.

[Current diagnostic evidence](../../benchmarks/evidence/browser-timer-task-lifecycle-2026-09-13.json):
**483 native tests passed, zero failed, six ignored (142.52 seconds)**. Strict V8
library Clippy passes. The signed bundle passes signature/designated-requirement,
sandbox-denial and authorized broker probes. A serial persistent-parent timer WPT
run passes **10/13 files** in 39.241 seconds, with zero environment-blocked files.
The microtask timing case passes; the remaining three failures are the two
cross-realm callback error-reporting timeouts and null initial contentWindow in
the detached-iframe test. No tests or timing thresholds were weakened.

This remains dirty-tree diagnostic evidence, not release qualification. **The
remaining P0 plan is not complete:** R6 authenticated environment routing/process
ownership, R4-R5 initial-window/custom-element lifecycle work, W0 upstream product
integration and the remaining qualification gates are still open. Protocol and
process placement have not changed.

## September 13 P0 IPC audit follow-up

The parent now independently bounds context dispatch at 64 operations, including
response writing, and disconnects over-budget peers without spawning more work.
The client retains its 32-operation limit; the extra parent cohort prevents a
reply/guard-release race from rejecting legitimate replacement requests. Network
IPC allocates identifiers under the frame writer lock, fixing concurrent
start/cancel reordering. Context, network and worker message/session identifiers
now reject exhaustion rather than reusing authority.

[Local evidence](../../benchmarks/evidence/browser-p0-ipc-bounds-2026-09-13.json):
**480 native tests passed, zero failed, six ignored in 139.52 seconds**, including
six new admission/concurrency/exhaustion regressions. Strict V8 library Clippy,
formatting and diff checks pass. No new signed or WPT qualification was run.
These are current-protocol hardening fixes, **not completion of P0 or R6**:
authenticated environment routing, lifecycle gaps, official wptrunner integration
and remaining package qualification gates are still open. Protocol remains v9.

## September 13 native ownership, lifecycle and broker-drain follow-up

The accumulated implementation and historical diagnostics were committed as
`d0f2143`. A subsequent actor scheduling change bounds idle polling by the next
timer deadline and dispatches already-due timers without a zero-duration Tokio
sleep, preserving command priority. Local validation: default-feature deadline
test passes; V8 native timer-filtered suite passes **10/10**; strict V8 native
library Clippy and formatting pass. This follow-up has **not** yet been signed or
rerun against WPT; the signed measurements below belong to the preceding source.

Native document/adoption ownership now includes inert template documents,
host-including cycle rejection and GC retention without permanent template roots.
The document driver owns AgentRuntime; native environment retirement cancels
timers, fetch resolutions and platform endpoints while retaining valid detached
DOM wrappers. Close acknowledges actor destruction. Reset unbinds the broker
before teardown; the parent admission barrier rejects new work, cancels owned
operations and requires bounded drain before reuse. Failed drain retires the
worker/router permanently. Timer conversion, cancellation and callback-realm
error reporting have also been repaired.

The subsequent [isolate-watchdog snapshot](../../benchmarks/evidence/browser-isolate-watchdog-2026-09-13.json),
source `cfcbb43d3861d7a43964ed9b19dbd4dafe94d6059b89e2b85e2e9903f180c4b1`,
passes **473 native tests, zero failed, six ignored (141.85 s)**. Its ten focused
image-responsiveness controls pass without changing the 250 ms deadline. It uses
one reusable watchdog thread per isolate, with serialized expiry/cancellation,
bounded non-reused registrations, nested-deadline enforcement and joined shutdown.
The slow-image fixture now releases blocked I/O on assertion failure. Its signed
bundle and sandbox probes pass. Signed WPT: adoption **3/3 files (13/13 subtests)**,
templates **8/67**, timers **9/13**, with zero environment-blocked files. The timer
family still fails the sub-4ms scheduling assertion (9.2ms measured), alongside
three iframe/window cases. The actor's fixed polling quantum requires further
deadline-aware scheduling work; this timing result is not waived.

Three alternating signed throughput samples using the same parent executable
improve the median from **194.7 to 245.4 actions/sec (+26%)**. Recycled state
erasure passes in all samples. This remains below the **300 actions/sec** target
and is diagnostic evidence, not clean-source performance acceptance.

Preceding signed source: `354c50f9c5bc09f691eda09ed12bad7a0bbad554b48a568619bf5dd26b25972a`.
The [hashed diagnostic](../../benchmarks/evidence/browser-parent-drain-barrier-2026-09-13.json)
records passing signature/sandbox probes and a three-lease recycled state-erasure
smoke. Thirteen worker-client tests pass in both default and V8 configurations;
strict library Clippy passes. The full native run finished **468 passed, one
failed, six ignored (596.82 s)**. The unchanged 250 ms image-responsiveness test
`slow_paint_images_do_not_hold_the_page_actor` failed; this is not waived or
classified as an environment block. Full native qualification has not passed.

Signed WPT: adoption **3/3 files (13/13 subtests)**; templates **8/67 files**;
timers **9/13 with concurrent family runs, 10/13 in a serial repeat**. The timing
failure remains recorded, not converted to a pass or an environment block. The
previous template hierarchy RSS breach now passes. These family diagnostics are
not a full-WPT baseline or clean-source release qualification.

**Architecture is still incomplete:** parent-issued per-environment IPC
credentials, protocol-v10 process-session routing, native custom-element adoption
reactions, synchronous initial iframe environments and connected-window placement
remain open. Agent-local environment IDs and the drain barrier do not replace
wire authentication. Connected-window placement remains inactive; protocol is still v9. The earlier
Range.insertNode crash remains unresolved/deferred. Historical sections below
describe their own snapshots, not this source.

## September 13 preceding document-boundary and write follow-up (local validation)

That [local diagnostic](../../benchmarks/evidence/browser-document-writes-local-2026-09-13.json)
passes **455 native tests, 0 failed, 6 ignored (589.47 s)**, **96 DOM tests**,
**255 V8-binding tests (3 ignored)** and **73 layout tests**. Strict library
Clippy, formatting and diff checks pass. The full native run used local-server
permissions and ran without competing builds or WPT. Earlier restricted and
interrupted attempts are preserved in the preceding local diagnostic.

This follow-up removes arena-wide ID lookup, rejects native embedding cycles and
Document child edges before mutation, preserves live Range endpoints on rejected
insertions, and retargets focus through explicit frame hosts. It also repairs
document.open/write/close replacing the primary document when invoked on a frame
document: replacement and buffered sessions now target the receiver. Interleaved
frame writes preserve parent/sibling identities and execute in the correct frame
realm. Aggregate write limits remain unchanged; inert-document scripts do not
execute in the caller's realm.

**No fresh signed, WPT or Linux qualification was run for this follow-up.** The
signed results below belong to their older source. Full incremental dynamic-markup
semantics, native ownerDocument/adoptNode, AgentRuntime, routing and subsequent
R/W packages remain incomplete. The Range.insertNode crash is still unresolved.

## September 13 preceding signed architecture checkpoint

The corrected frame-association cutover separates hosted iframe documents from
DOM child edges. Native layout, retention and semantic action validation follow
explicit embedding associations; detached-frame actions are rejected. This is a
partial R4 implementation, not completion of document ownership or agent routing.

The [corrected diagnostic evidence](../../benchmarks/evidence/browser-frame-association-corrected-2026-09-13.json)
records **453 native tests passed, 0 failed, 6 ignored (652.16 s)** in a serial
run without competing builds or WPT. An earlier run failed the unchanged 250 ms
image-responsiveness deadline; that failure remains recorded alongside ten passing
individual repeats. Its cause is not established. DOM: **91 passed**; layout:
**73 passed**; V8 bindings: **254 passed, 3 ignored**. Library Clippy with warnings
denied, formatting, signed sandbox denial probes, and Linux aarch64 no-default-feature
release compilation passed. Linux runtime/V8 behavior was not qualified.

Signed targeted WPT results remain **FormData 18/18** and **submission 18/35**,
with no file-level status changes from the previous form-reflection snapshot and
no environmental blocks. These are selected executions, not full-family or full
WPT results. The Range.insertNode stack-overflow crash remains unresolved; its
latest reproduction belongs to the separately preserved initial cutover snapshot.

This is dirty-source diagnostic evidence, not a release qualification. Native
document ownership/adoption, agent lifecycle/scheduling, protocol-v10 routing,
connected-window activation and broader worker/WPT integration remain unfinished.
Related-window sharing is not activated; isolated execution remains the default.

## Historical conformance measurements

**September 10 scope update:** this is a historical selected-inventory result.
The [full WPT capability roadmap](WPT-CAPABILITY-ROADMAP-2026-09-10.md) supersedes
the old 4,022-test completion gate with Core, Necessary, Extended and
Platform-dependent tiers. The source catalog maps 271 capability roots; official
full-manifest enumeration and tier baseline runs remain pending.

An additive agent-browser inventory now maps the published Obscura Core/Relevant
and Kitesurf feature footprints onto the same pinned WPT revision. It contains
12,708 files and 16,842 execution variants while preserving the 4,022-execution
ledger unchanged. This is inventory coverage, not a measured pass result; see
[the footprint review](AGENT-BROWSER-WPT-FOOTPRINT-2026-09-10.md).

The complete signed run at `011d470` gains **ten whole executions** over `82fe3b4`:
eight CSS fixtures and two history fixtures. There are **zero whole-execution
losses**, **zero uniquely matched assertion regressions**, and no new incomplete
executions. Across the roadmap's two full measurements, the total has increased
from 3,109 to 3,144. Stylo remains the CSS engine; the inventory and production
command, memory and transport limits are unchanged.

[Full signed evidence](CONFORMANCE-ROADMAP-CSS-FULL-2026-09-09.json) binds the terminal
4,022-execution ledger to clean source, signed binaries and runtime identity.
The canonical validator verifies exact coverage and every passing completion.
The [failure backlog](CONFORMANCE-FAILURE-BACKLOG-2026-09-06.json) contains 878
failures from this single ledger, without focused-run overlays.

The latest comparison records 34 assertion corrections and 12 duplicate-name
groups. Twenty-three older missing observations recover; 119 remain missing,
plus seven newly unobserved assertions in Range-mutations-dataChange, for
**126 unobserved assertions**. Both versions of that fixture hit the unchanged
command limit. Six historical editing regressions and 170 historical ambiguous
groups remain unresolved. Missing observations and ambiguous identities are not
waived or treated as passing evidence.

The complete native CSS slice and signed history/referrer slices have exactly
the same assertion name/status multisets as their projections in this full run.
Three CSS and ten history executions differ in incomplete/timeout result flags;
all remain failing. Referrer completion flags match exactly.

The CSS changes expose native CSSNestedDeclarations and Stylo's computed custom
properties, including registration, inheritance, substitution and invalid values.
Frame history changes register parser-created realms, consume history URL commits,
and update request referrers synchronously. Official wptserve performs HTML
substitution before instrumentation. Earlier roadmap work repaired typing-style
preservation, DOMTokenList liveness, cursor retrieval, semantic actions and
assertion transport. See the [preceding full result](CONFORMANCE-ROADMAP-FULL-2026-09-09.json).

[Validation at this candidate](CONFORMANCE-CSS-NATIVE-2026-09-09.json):
**516 workspace tests passed (6 ignored)**, **193 binding tests (2 ignored)**,
and **398 native tests (5 ignored)**. Formatting, strict workspace Clippy and
V8-enabled all-feature/all-target strict Clippy pass. The CSSOM suite passes
35 tests, and the complete native CSS audit passes 368/564. The unchanged runner's
46 tooling tests passed in the preceding history batch.

The subsequent TreeWalker batch at `3eee013` replaces eager snapshots with live
traversal, lazy filters, reentrancy protection and callback-realm exception handling.
Its [complete signed traversal slice](CONFORMANCE-TRAVERSAL-SIGNED-2026-09-09.json)
passes **13/18**, gaining **eleven whole executions and 793 assertions**. Every
assertion and completion flag matches the native run. There are no whole losses,
matched regressions, missing/added assertions, ambiguous names or incomplete
executions in this comparison. The five remaining executions concern NodeIterator
and immediate iframe document/foreign-node behavior. These focused gains are not
added to the full total above.

[Traversal regression gates](CONFORMANCE-TRAVERSAL-NATIVE-2026-09-09.json) pass
**401 native tests (5 ignored)**, **193 binding tests (2 ignored)** and
**516 workspace tests (6 ignored)**, plus generated bindings, formatting and both
strict Clippy gates. The production worker build, signature verification and
DOM-mutation smoke test pass.

Follow the [impact roadmap](CONFORMANCE-IMPACT-ROADMAP-2026-09-09.md). Live
NodeIterator mutation retargeting, initial iframe documents, Stylo stylesheet
ownership and Range performance remain open. Qualification now follows the full
capability roadmap's declared tiers and environments, with exact coverage and
regression observations resolved.

| Root | Passed / required |
|---|---:|
| IndexedDB | 420/456 |
| WebIDL | 31/58 |
| accname | 15/19 |
| css/css-cascade | 61/97 |
| css/cssom | 107/190 |
| css/selectors | 200/277 |
| dom | 419/662 |
| editing | 433/704 |
| html/browsers/history | 47/136 |
| referrer-policy | 1383/1391 |
| wai-aria | 28/32 |

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

- Close the remaining 913 cases in the last measured exact 4,022-variant inventory,
  prioritizing shared harness/lifecycle causes over isolated assertions. The
  measured deficits are editing 288, CSS 204, DOM 247, IndexedDB 40, referrer
  policy 8, HTML 91, generated-WebIDL 27, WAI-ARIA 4 and Accessible Name 4.
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
