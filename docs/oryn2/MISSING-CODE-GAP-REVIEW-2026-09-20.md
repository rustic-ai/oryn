# Missing-code and integration gap review — September 20, 2026

## Conclusion

**There is real missing code, not just pending testing.** This bounded review identifies 14 concrete API, implementation, cleanup, test-infrastructure or integration work items. Some have an external draft; they are not all new generation. It also separates four accepted fixture corrections, one optional optimization, environment assembly and unresolved diagnostic failures.

The initial 51-parent generation declaration records a completed generation phase, not proof that every required API and caller was implemented. `scripts/generation_gate.py:22–46` validates declarations and source hashes; it does not establish member-level coverage or behavioral correctness. The newly demonstrated API omissions must remain open despite that historical milestone.

Implementation remains paused. Three parallel read-only reviews and root source inspection were used. No product code, builds, tests, deployments or repair integration ran in this review. Existing canonical64 regression evidence and the 80-case WPT diagnostic are the execution basis. This list is a lower bound; no complete current Core or family run exists.

## Priority index

P0 means shared diagnosis/ownership or trustworthy execution must be restored early. P1 items remain mandatory where required by the pinned Core/family contract; priority does not waive them.

| ID | Priority | Work item | State |
| --- | --- | --- | --- |
| G01 | P0 | Error stack accessor breaks WPT AssertionError | confirmed defect |
| G02 | P1 | Expose isSecureContext on supported globals | confirmed missing api |
| G03 | P1 | URL static parse/canParse API coverage | confirmed missing api |
| G04 | P1 | Synchronous XHR implementation remains explicitly rejected | confirmed missing behavior |
| G05 | P1 | Streaming HTML method family only partially implemented | confirmed missing api |
| G06 | P1 | Pinned XSLTProcessor obligation has no implementation | confirmed missing api |
| G07 | P0 | Auxiliary rollback retry and acknowledgement custody | draft exists review pending |
| G08 | P1 | Secondary cleanup failures lack observation and negative coverage | confirmed defect and test gap |
| G09 | P1 | VP9 fallback and its safety regression are unfinished | partial external generation |
| G10 | P0 | Family executor does not establish settlement after survivor kill | confirmed test infrastructure gap |
| G11 | P1 | Family fixture cleanup stops after the first exception | confirmed test infrastructure gap |
| G12 | P1 | PreparedCase can lose a started oracle during construction | confirmed test infrastructure gap |
| G13 | P1 | Generated FACE upload fixture has no full-contract recipe caller | confirmed integration gap |
| G14 | P0 | WPT exporter drops recorded asynchronous errors | confirmed diagnostic infrastructure gap |

## Evidence, remaining code and closure

### G01 — Error stack accessor breaks WPT AssertionError

**Evidence:** `crates/oryn-js/src/v8_bindings/native_errors.rs:60`; `crates/oryn-js/src/v8_bindings/native_errors.rs:159`; `crates/oryn-js/src/v8_bindings.rs:7934`.

Ten of 80 result records contain Error stack receiver required. This masks their underlying causes; fixing reporting does not make those tests pass.

**Remaining code:** Correct stack assignment compatibility for ordinary objects inheriting Error.prototype without weakening genuine Error/DOMException identity checks. The installed setter rejects WPT AssertionError, whose constructor assigns this.stack to an object made with Object.create(Error.prototype).

**Required regression/verification:** Use the pinned AssertionError construction and a deliberately failing assertion; retain original assertion message and writable stack. Retain native Error/DOMException clone, brand, cross-realm and forged-receiver negative coverage.

### G02 — Expose isSecureContext on supported globals

**Evidence:** `crates/oryn-webidl/idl/browser.webidl:51`; `crates/oryn-js/src/v8_bindings/workers.rs:4`; `crates/oryn-native/src/host.rs:5594`.

Both sampled secure-context cases throw isSecureContext is not defined; source search finds no exposed member.

**Remaining code:** Add Window/worker global exposure using existing native secure-context state and correct realm/ancestor inheritance; regenerate canonical bindings. Existing policy state and publication are not the exposed JavaScript property.

**Required regression/verification:** Pinned HTTP/HTTPS popup and iframe cases, inherited about:blank/srcdoc, insecure ancestors, workers, cross-origin and lifecycle updates; verify descriptors.

### G03 — URL static parse/canParse API coverage

**Evidence:** `crates/oryn-webidl/idl/browser.webidl:173`; `crates/oryn-js/src/v8_bindings.rs:7637`.

Sample URL.parse fails as missing; canParse absence is a source finding, not a sampled runtime result.

**Remaining code:** Add URL.parse and URL.canParse with WebIDL conversion, optional-base semantics and correct return types; reuse native URL parsing and fresh wrapper creation. Current custom statics only install blob URL operations.

**Required regression/verification:** Pinned URL.parse case: invalid inputs return null, valid inputs match URL constructor, each parse returns a distinct URL; canParse conversion/base/error cases and Window/worker exposure.

### G04 — Synchronous XHR implementation remains explicitly rejected

**Evidence:** `crates/oryn-js/src/v8_bindings.rs:27420`; `crates/oryn-native/src/rust_executable.rs:6795`; `crates/oryn-native/src/runtime.rs:39192`.

Two sampled cases directly hit the explicit unsupported branch. This is not a fixture-only failure.

**Remaining code:** Implement the required synchronous XHR path, authority/policy/response handling and correct event behavior in supported execution routes. Review worker versus Window applicability; do not remove the rejection without implementing scheduling and response semantics. Update the current regression that expects unsupported behavior.

**Required regression/verification:** Pinned cors/client-hint-request-headers-2.tentative.htm and xhr/send-sync-no-response-event-load.htm; allowed/disallowed sync responseType/timeout behavior, request policy, same-origin/CORS and cancellation/deadline custody.

### G05 — Streaming HTML method family only partially implemented

**Evidence:** `crates/oryn-webidl/idl/browser.webidl:906`; `crates/oryn-webidl/idl/browser.webidl:2650`; `crates/oryn-js/src/v8_bindings.rs:35468`; `crates/oryn-js/src/v8_bindings/html_stream.rs:375`.

Nine of the ten tested method names lack declarations; an append-unsafe streaming implementation and tests already exist.

**Remaining code:** Extend the existing streamAppendHTMLUnsafe machinery to the missing tested methods: streamHTML/Unsafe, streamAppendHTML, streamPrependHTML/Unsafe, streamBeforeHTML/Unsafe and streamAfterHTML/Unsafe. Implement the required receiver-specific semantics, sanitization, Trusted Types and insertion positions; do not alias every method to append.

**Required regression/verification:** Pinned ten-method Trusted Types test, correct insertion/replacement positions, split tokens, script scheduling, receiver disconnection, cancellation/backpressure, policy rejection and mutation/reentrancy.

### G06 — Pinned XSLTProcessor obligation has no implementation

**Evidence:** `crates/oryn-webidl/idl/browser.webidl`; `crates/oryn-js/src/v8_bindings.rs`.

Sample throws XSLTProcessor is not defined, and no declaration/implementation was found in crates. The pinned Core scope retains this obligation despite changing browser support elsewhere.

**Remaining code:** Implement the pinned XSLTProcessor surface and transformation/integration needed by Core, including stylesheet import and transformToFragment with proper ownership and script insertion behavior. Determine dependency/resource-policy design before generation; this is larger than adding a constructor stub.

**Required regression/verification:** Pinned xml/xslt/transformToFragment.tentative.window.js plus XML namespaces, parameters, malformed stylesheets, resource restrictions and fragment/document ownership. Retain executable inserted internal/external script assertions.

### G07 — Auxiliary rollback retry and acknowledgement custody

**Evidence:** `crates/oryn-native/src/auxiliary_root_v8.rs:28`; `crates/oryn-native/src/auxiliary_root_v8.rs:315`; `crates/oryn-native/src/context_service.rs:7467`.

Confirmed control-flow/ownership omission and existing failing regression. The external draft is generated, not accepted or integrated.

**Remaining code:** Review the existing eight-file rollback proposal and integrate its pre-admission ticket, target binding, revocation-before-retirement and retry/ACK ownership after resolving review findings. Current one-shot Drop discards errors; do not create a competing draft.

**Required regression/verification:** Lost rollback ACK retained until retry; retired identity not rematerialized; stale generation cannot close a replacement; original bootstrap error retained alongside cleanup failure.

### G08 — Secondary cleanup failures lack observation and negative coverage

**Evidence:** `crates/oryn-native/src/selected_paint_image_load.rs:233`; `crates/oryn-native/src/selected_paint_image_load.rs:299`; `crates/oryn-native/src/webdriver_fairness.rs:49`.

Abort results and cancelled-worker join results are discarded. A leak has not been demonstrated.

**Remaining code:** Validate ordinary abort acknowledgements and report secondary abort/transport/join failures while retaining the primary error. Keep destructor fallback and owned joins; do not detach cleanup to manufacture timely completion.

**Required regression/verification:** Successful/failed read combined with abort error/wrong response; cancelled cleanup worker panic; delayed replies and settled resource ownership.

### G09 — VP9 fallback and its safety regression are unfinished

**Evidence:** `crates/oryn-native/src/media_capture/vpx_build.rs:18`; `crates/oryn-native/src/media_capture/webm_macos.inc:34`.

Integrated VP9 relies on failing native setup. Initial fallback was rejected; partial corrected files and stale four-file manifest must not be integrated.

**Remaining code:** Finish safe software fallback and per-actually-decoded-frame allocation bounds. External proposal declares/calls oryn_webm_unindexed_bounds_check without its native implementation; complete it and the concatenated-frame/mid-GOP tests, then refresh all seven source files and vendor provenance for review.

**Required regression/verification:** Valid small first frame plus oversized subsequent unindexed frame must reject before allocation; native failure mid-GOP replay, cross-platform compilation and original pixels/PCM/seek/PiP/cleanup assertions.

### G10 — Family executor does not establish settlement after survivor kill

**Evidence:** `scripts/workflow_native_adapter.py:123`; `scripts/workflow_native_adapter.py:142`; `scripts/workflow_execution.py:119`.

Existing code rejects success but does not prove final group disappearance or latch subsequent attempts after lost custody.

**Remaining code:** After killing surviving owned descendants, perform bounded group-settlement checks and persist terminal custody. Prevent subsequent dispatch when cleanup remains unsettled; keep ordinary clean business failures runnable and preserve all planned attempt records.

**Required regression/verification:** Exited parent with surviving descendant, failed settlement stops later dispatch, clean nonzero exit permits continuation, timeout/exit races retain actual wait result.

### G11 — Family fixture cleanup stops after the first exception

**Evidence:** `scripts/workflow_native_adapter.py:487`; `scripts/workflow_native_adapter.py:644`; `scripts/run-oil-forms-outcomes.py:150`.

Sequential finally loops can skip later cleanup. This is a proved exceptional path, not an observed current process leak.

**Remaining code:** Use exception-isolated cleanup for every oracle/probe/secret and pipe; retain primary failure plus all cleanup failures and incomplete ownership. Apply the same collector to mixed attempts and commit supplements.

**Required regression/verification:** First oracle close throws but later owners close; secret unlink throws while all oracles close; kill/wait failure still closes pipes and remains recorded.

### G12 — PreparedCase can lose a started oracle during construction

**Evidence:** `scripts/workflow_native_adapter.py:354`; `scripts/workflow_native_adapter.py:360`; `scripts/run-oil-forms-outcomes.py:107`.

The outer prepared list cannot clean an object whose constructor never returns.

**Remaining code:** Retain constructor-owned resources through task-line construction and transfer ownership only after successful preparation. Close preopened stderr if generation validation or Popen fails. Preserve both construction and cleanup errors.

**Required regression/verification:** task_lines failure after successful oracle create closes exactly once; create plus close failure retains both; generation/Popen failure closes the preopened handle.

### G13 — Generated FACE upload fixture has no full-contract recipe caller

**Evidence:** `test-harness/workflows/forms-cases-v4.json:394`; `test-harness/workflows/forms_fixture_v4.py:96`; `scripts/workflow_family_recipes.py:11`; `scripts/run-oil-full-contract.py:53`.

Fixture and native implementation exist; the relevant custom-control/upload intersection is not exercised by full-contract dispatch.

**Remaining code:** Connect native_face_authorized_multipart to the appropriate existing forms recipe and regenerate its expected-oracle/witness binding. Preserve the 29 cells and 688 original attempt IDs; change the internal recipe only through a reviewed new candidate.

**Required regression/verification:** Require recipe reachability; compare ordered duplicate multipart fields and authorized upload hashes; FormData mutation after setFormValue and denied-upload negative checks.

### G14 — WPT exporter drops recorded asynchronous errors

**Evidence:** `crates/oryn/src/wpt_session.rs:129` exports only four resource/policy event kinds. Scheduler errors and unclaimed navigation errors already exist at `page_actor_turn.rs:263` and `navigation_actor.rs:3118`.

**Remaining code:** export bounded diagnostic events with page/sequence identity and original error intact. Preserve the existing policy/resource fields. This repairs diagnostic visibility, not the unresolved popup transaction itself.

**Required regression:** failed scheduled child task and unclaimed navigation preserve errors through export/cleanup; multiple pages, overflow and successful cases.

## Work that must not be counted as absent implementation

- **Accepted but unintegrated:** wait-fairness, import-map completion, WebDriver shared-actor and srcdoc-load fixture patches. Reuse their accepted source; reconcile overlapping files, then validate. Four fixture proposals account for six existing failing executions.
- **Optional accepted optimization:** wrapper-graph proposal. The original capacity test passed; optimize only when required by a measured cost/performance problem.
- **Mechanical fix:** default-configuration Clippy return at `page_actor_turn.rs:727`, followed by the two unrun configurations. This is hygiene work, not a missing subsystem.
- **Existing infrastructure:** official WPT preparation/execution/qualification, profile contracts, topology/native controls, release/prebuilt producers, 29-cell mixed/fault family orchestration, persisted oracles and PKCE/MFA provider all have source. Missing runtime results do not justify rewriting them.
- **Setup/assembly:** actual private/public WPT route and trust/control configuration; matching current release/driver/workflow binaries and security/architecture maps; real profile registry/contracts; local family dependencies and fixture ownership. These are not proved missing software.
- **Execution:** three complete Core profiles, 688 family/reliability originals, 24 framework originals and remaining final gates remain unverified. Local family diagnostics do not inherently wait for the off-host WPT topology.

## Failures requiring diagnosis before a production patch

1. **Popup/network progress:** policy submission is observed but fixture receipt is absent; correlate actor, IO admission, broker entry/return and cancellation. Do not infer the cause from passing short prefixes. Retain autonomous navigation and original signed popup regressions.
2. **Resource/time limits:** Big5 worker RSS, import-map evaluation, integrity-policy navigation, X-Frame-Options navigation and outer timeout; plus the larger ignored Range population. Profile actual work and ownership before changing algorithms or limits. Raising limits is not a demonstrated fix.
3. **Worker mailbox overflow:** `agent_process.rs:556–573` enforces message/byte capacity and the DOMException worker case hits it. Determine message production, delivery/drain and payload retention before changing caps or adding backpressure. No particular missing flow-control implementation is proved yet.
4. **Masked assertions:** ten raw case records contain `Error stack receiver required`. Repair G01 to recover original diagnostics; do not count each masked case as a distinct proven browser algorithm defect.
5. **Historical testdriver limitation:** three cases record unsupported action_sequence in the diagnostic adapter; `scripts/wpt-testdriver-oryn.js:16–21` rejects mixed-frame origins or missing versioned action support. This does not prove the official WebDriver action endpoint is absent. Keep the original failures and use the official admitted route for acceptance.
6. **Fixture error attribution:** zero environment-blocked cases is a classifier result, not proof all HTTP handlers succeeded. Inspect original response/fixture evidence before classifying a timeout or reporting error as a browser defect.

**Optional historical diagnostic maintenance:** `scripts/wpt_fixture.py:60,99–108` closes temporary server stderr without retaining it. A bounded stderr artifact would help explain the observed HTTP 500, but is not a prerequisite to the official runner, which already retains its logs.

## Recommended complete next repair allocation

- **Diagnostics and shared lifecycle:** G01, G07, G08, G14; diagnose the original popup transition and add only the supported causal repair.
- **API completion:** G02–G06 with shared IDL/generated-binding/caller ownership; explicitly plan XSLT dependency/resource semantics rather than assuming it is a small binding addition.
- **Family infrastructure and coverage:** G10–G13 plus the four accepted fixture patches; preserve denominator and independent oracles.
- **Media and resource behavior:** finish/review G09; investigate measured RSS/time/mailbox failures before selecting remaining code changes.
- **Environment/release lane:** complete real fixture and artifact assembly in parallel; do not let unrelated qualification result receipts delay a valid first official Core or local family measurement.
- **One integration owner:** reconcile shared files, new tests, include/cfg callers and review hashes for the entire repair batch before candidate freeze and validation. No subsystem-by-subsystem mutation while testing.

An estimate of total remaining implementation is still premature: the 80-case cross-section finds missing features but cannot bound gaps across all 16,737 Core obligations. Complete current Core and family measurements remain necessary after admission.

## References

- [Machine-readable gap list](../../artifacts/completion-validation-2026-09-18/gap-review-2026-09-20/gaps.json)
- [Family infrastructure and media review](../../artifacts/completion-validation-2026-09-18/gap-review-2026-09-20/families.md)
- [Official fixture/release infrastructure review](../../artifacts/completion-validation-2026-09-18/gap-review-2026-09-20/infrastructure.md)
- [Integration and cleanup review](../../artifacts/completion-validation-2026-09-18/gap-review-2026-09-20/integration.md)
- [WPT diagnostic report](../../artifacts/completion-validation-2026-09-18/canonical64-wpt-diagnostic-2026-09-20-01/REPORT.md)
- [Current status](CURRENT_STATUS.md)
- [Governing directive](POST-STOCKTAKE-DELIVERY-DIRECTIVE-2026-09-20.md)
- [Full scope](CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md)
