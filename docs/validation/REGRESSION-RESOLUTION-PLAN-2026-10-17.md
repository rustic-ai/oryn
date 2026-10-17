# Native regression resolution plan — 2026-10-17

## Baseline and scope

Analyzed candidate: e9551c6413f346807cea44a80e7543067ab9b39a.
Authoritative checkout: oryn-roadmap-next, codex/conformance-next.
Original: oryn-data/development/wpt-final-integration-20261017-owner-dispatch-15-all-libraries.
Raw stdout SHA256: e889c160ff6bd73f86f23281b0796b5a31e92b5c6c03025cb87aec08bf65acfb.
Terminal process receipt SHA256:5448ce6de9994d6dc2e8891ece7244b9f086297a0213f888c0a8a20ad22cdfbe.
Compilation passed; all four library populations completed with cleanup, no timeout.
6491 passed,154 failed,11 ignored: DOM495/0;JS2056/3;layout643/0;native3297/151 (pass/fail).
Compared with integrated-13,321 failed tests recovered and no formerly passing test failed. This is library evidence, not WPT or release qualification.

The October15 WPT plan remains binding. This document is its current native-regression prerequisite, not a replacement for the pinned4022 population or broader delivery requirements. Failed tests are observations, not independent bugs. No projected gains below are guarantees.

## Exhaustive first-level partition

Groups use each failed test's panic block, assigning the first matching signature in this order. No test is counted twice.

| Group | Failed tests | Interpretation |
| --- | ---: | --- |
| Font projection owner retired |24| Standalone popup paths and directly constructed test providers |
| Missing worker executable |17|7 IPC boundaries,10 XSLT discovery; prerequisite unavailable |
| CachedAuthorityExpired |5| Lifetime/admission rejection; cause needs fixture diagnosis |
| Document departure wall time |25| Departure phase failed to settle; deadline is symptom |
| PageTaskYield |33| Terminal/projection cancellation reaches caller; distinguish legitimate yield from stale control |
| Other errors/assertions |50|3 JS plus47 native, itemized by the original failure list |
| Total |154| Original population unchanged |

## Batch A — finish owner publication and restore executable assertions

1. Split24owner failures before editing. Five direct-provider controls in drag_image_capture.rs and production_input_regressions.rs construct LiveLayoutProvider with identity None. Bind explicit owning test Document using the authentic provider contract before querying geometry; retain invalid/foreign-owner negative controls. These are fixture setup corrections, not evidence of browser gains.
2. Remaining19owner failures cluster in standalone_auxiliary_regressions and standalone_root_driver. Trace standalone_auxiliary_root_v8.rs direct initial provider installation and subsequent native adoption/navigation. Unlike admitted auxiliary bootstrap, this path still needs exact owner binding before its first layout/font consumer. Audit successors/provider forks and old-document retirement; never copy parent identity or make None universally acceptable.
3. IndexedDB control still declares caught twice in the same callback scope. Use a local block or distinct options exception variable, preserving every exception/order assertion. Parse the complete embedded script before rerunning; do not repair redeclarations one at a time. Its algorithm has not yet been observed by this control.
4. PI/Range control hides the actual JS exception behind Script.run.unwrap. Use the existing diagnostic evaluation helper or TryCatch to report the primary exception and assertion phase. Inspect XML parser PI creation, entity/UTF16 serialization and Range conversion separately; no inferred shared defect.

Gate: complete standalone auxiliary/input-provider and original JS controls, including saved wrapper/foreign arena/replacement/GC tests. Preserve exact expected assertions. Commit reviewed batch; one cached compile, then affected controls. Reassess residuals rather than claiming24automatic recoveries.

## Batch B — terminal authority and departure progress

This is the largest unresolved symptom group (58); it must not be treated as one history defect.

1. Reproduce one minimal PageTaskYield fixture and a passing sibling. Instrument page_task_terminal_control identity/generation, close-request state, retirement reason, executable/source identity and prepare/commit checkpoints. page_semantic_projection.rs and page_layout_projection.rs deliberately yield on retired terminal controls. Determine whether source retirement accidentally invalidates the continuing page actor, or whether a caller wrongly exposes an internal scheduling yield as terminal failure. Keep real close cancellation intact.
2. Trace25departure deadlines by phase in root_departure.rs: pageswap, pagehide, form capture, unload, semantic sync, then committed phase3. Record first failing operation and retained primary error. complete_root_departure retries some errors until deadline, so deadline text can conceal a persistent semantic/admission failure. Repair retryability/phase progress, not the deadline. Actor and synchronous compatibility paths must use the same receipt and avoid callback replay.
3. Split rollback/persisted pageshow, cross-origin hidden history, no-content/download, eager/complete reply and cancellation chains. Run their complete fixtures, not only the final failing assertion. Include the six residual recovery/event assertions and joint eager reply mismatch even though they are not in the58signature count.
4. Investigate5CachedAuthorityExpired separately using allocation scope, auxiliary closure and replacement provenance; only merge into this batch if traces prove shared custody cause.

Gate: all history/navigation/semantic-sync/terminal-control tests, closed-worker controls, event order and exact-once side effects. No new yield leakage, stale revival or weakened retirement rules. Production command/navigation deadlines remain unchanged.

## Batch C — native CSSOM and font completion

1. CSSOM failures cover imports, nested declarations, MediaList, CSSPageRule/declarations, readonly rule lists and subtype accessors. Trace generated declaring interface versus native concrete record/handle, wrapper identity and collection lifetime. WrapperId(MAX), incompatible receiver and CSSRule.media errors suggest dispatch/identity gaps, but each complete fixture must prove its own cause. Preserve proxy/forged receiver TypeErrors and detached native rule identity.
2. Import observation3|true|true|false|true|true and srcdoc SecurityError must be mapped to their exact assertion/owner origin. Audit original baseURL, import conditions/order, original rule ownership and replacement. Do not allow cross-origin CSSOM merely to recover a fixture.
3. Font404 currently aborts document/stylesheet flows in standalone and brokered tests. Separate optional font fetch/decode failure from infrastructure/capacity/cancellation; settle FontFace promises/events correctly without converting failed fonts to loaded. Diagnose font_broker_failure_provenance, optional deadline pending binary retention, FontFaceSet brand assertion and native font termination recovery independently.
4. Keep providers without the native font backend explicit and compatible: scheduler custom-provider test fails activation. Determine capability deferral versus required loading; do not fabricate readiness.

Gate: full native CSSOM/font/import controls, cancellation/deadline/owner replacement and semantic/layout consumers. Then complete564CSS WPT slice on the resulting signed runtime.

## Batch D — real sandbox worker prerequisites and XSLT

Build/assemble the worker with canonical scripts and existing prepared V8 inputs/cache; verify its signature, profile and source identity. Configure authenticated worker discovery (ORYN_PAGE_WORKER where supported) rather than falling back to an unrelated historical executable or copying an unsigned helper beside test binaries. Verify exact bundle path before launch. Signing/other external prerequisites require approval when unavailable.

Run all17missing-worker originals; only then diagnose their semantics. Three additional XSLT NetworkError observations may be related but are not included in17 and must be retested independently. Preserve sandbox restrictions, same-origin resource admission and process cleanup. No skips or fake transformations.

## Batch E — residual semantic/algorithm groups

- Editing: paragraph_owner_withdrawal failure occurs before the author's subsequent insert, after injected undo preparation failure. Record committed DOM/journal prefix, history identity and native continuation state at the actual failure boundary. Repair prefix/history custody, not an assumed all-or-nothing rollback. Historical inline-DIV LF versus BR original12observations remain separate; investigate computed whitespace and oracle disagreement without style-presence heuristics or weakened assertions. Gate full704editing slice after coherent editing fixes.
- IndexedDB: parent typed-key test reports missing connection/transaction authority; two other storage tests remain pending. Trace authentic upgrade/connection/cursor authority and result task delivery, separating security rejection from scheduler progress. Fix the JS control first; never grant blanket storage authority. Gate complete455IDB slice.
- Resource task completion: body streaming and credential tests remain pending. Trace original broker completion, decoded body and queued JS promise tasks; do not assume IDB and fetch pending share a defect.
- Semantic/OIL: no-document snapshots, native form observation None, stale modal semantic references and retained allocation expiry during assigned-slot scrolling. Trace document installation, projection publication and mutation revision. Churn test's IDB allocation limit requires measured retained-memory diagnosis; do not raise budget.
- Lifetime/input: frame retirement suspended, pending abort notification, ancestor snapshot and saved proxy registration. Preserve retained callback/cursor identity through suspend/close without retargeting or reviving retired owners.
- Animation allocation failure during history reload: profile receipt/retained allocation at native animation creation; do not reinterpret as unsupported animation or disable it.

Gate: original residual tests plus nearest passing controls; full relevant WPT families only after library observation stability.

## Integration and verification protocol

One coordinator owns shared edits/cache/signing; disjoint staging lanes may diagnose concurrently when authorized. All drafts, manifests, hashes, dirty-input snapshots and review notes live under workspace .local-patches, never tmp. Keep source commits authoritative and recovery checkout untouched.

Integrate coherent A/B/C batches before expensive broad verification; perform cheap script parsing and targeted compiler checks where available. One compile per joined source batch, then reuse its binaries for focused controls and full four-library comparison. Record test names, assertions, ignored count, runtime, terminal cleanup and exact source identity. After two unsuccessful repairs revisit diagnosis.

Each promotion requires no formerly passing test failures, no missing original observations and preserved raw evidence. Do not clear remaining failures by changing limits, assertions, inventory, security or generated binding authority. A group closes only after its complete original tests pass, not after source implementation.

After library closure: canonical generated-binding/workspace/Clippy gates, signed4022WPT original and independent ledger reconciliation; diagnose any WPT-only regressions by complete fixtures. Final qualification still requires two fully passing4022originals on identical signed artifacts, all required observations and existing platform/security/recovery/performance/differential gates. No overall completion percentage follows from this154test backlog.
