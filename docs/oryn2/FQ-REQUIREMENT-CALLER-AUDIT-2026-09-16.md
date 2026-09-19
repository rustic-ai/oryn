# F/Q requirement-to-caller reconciliation

Source review only; no builds, tests, fixtures, generators or qualification ran.
This inventory distinguishes generated production paths from missing evidence.
The frozen behavior contract remains unchanged (29 cells, five families).

## Family implementation map

Every row below is consumed by `MixedCellAdapter`; `PreparedCase.seal/outcome`
reopens durable SQLite state, the original family validator checks browser/action
observations, and `workflow_family_recipes.expected/observed` joins the independent
facts. Research recipes instead compare extracted facts and persisted visits.

| Cells | Concrete source recipes | Independent acceptance source |
| --- | --- | --- |
| research.compare | search_paginate_compare | Research validator exact text/table/link facts, canonical URLs and visits |
| research.pagination | dynamic_results, refreshed_results, hydrated_pagination | Research validator refresh/page facts and SSR/module evidence |
| research.frames | nested_cross_origin_extraction | v3 frame fixture and nested same/cross-origin facts |
| research.missing | empty_results, missing_source_recovery, redirected_source | Empty/error facts and redirect visits |
| research.stale | generation_bound_stale_reference | Native selected-generation rejection and reacquisition evidence |
| forms.controls | multistep_custom_role, multistep_authorized_upload | v3 controls plus ordered receipt/Unicode/file metadata |
| forms.validation | async_email_recovery, multistep_cancel, multistep_cancel_recovery | Validation audit, rejected receipt absence, corrected receipt |
| forms.delivery | native_get, native_post, native_multipart | v4 method/body/order/Unicode/hash oracle |
| forms.submitters | native_submitter, native_cancel | Submitter URL/method and no cancelled receipt |
| forms.reentrancy | native_reentrant | One authorized receipt and exact request sequence |
| forms.confinement | native_confinement | Adversarial independent context and authorized byte confinement |
| shopping.discovery/cart | framed_persisted_checkout | Both product observations, filter result, variant/cart/quantity persistence |
| shopping.checkout/payment_frame | framed_persisted_checkout, framed_required_blocks | Persisted quote/order/total/stock and controlled payment events |
| shopping.stale_quote | price/stock change blocks and recovery recipes | Old quote rejected; current price/stock accepted once |
| shopping.duplicate | framed_duplicate_checkout | One durable order/inventory mutation despite repeated request |
| saas.pkce/records | pkce_create_update_reload_logout | OAuthlib 3.3.1 code/S256 exchange, record/version/auth history |
| saas.mfa | pkce_mfa_create_update_reload_logout | Authorized challenge boundary and completed session |
| saas.session | pkce_logged_out_access_denied, pkce_expired_session_save_recovery | Denied expired write, logout/session and relogin state |
| saas.oauth_rejection | oauth_reject_state/redirect/verifier/replay/expiry | Each browser exchange rejected without authorized session |
| saas.save_recovery | pkce_concurrent_edit_recovery | Stale save/version history and duplicate-write protection |
| saas.isolation | pkce_isolated_records | Independent context denied access/write and redaction evidence |
| booking.timezones | client_fold_first/second, client_vancouver/utc, New York/Vancouver gap | Client-selected UTC/zone, fold choice and invalid gap audit |
| booking.reserve | client_phone_option, client_required_validation | Slot ownership/options and durable reservation or no receipt |
| booking.reload | client_fold_first | Same reservation identity after confirmation reload |
| booking.stale_slot | client_stale_recovery | Rejected old slot and accepted replacement request history |
| booking.cancel_retry | client_cancel_retry, client_duplicate_request | No cancelled reservation; one accepted durable reservation |

Sources: `workflow_native_adapter.py::RESEARCH_RECIPES`,
`workflow_family_recipes.py::RECIPES`, the versioned workflow case JSON/fixtures,
and `run-oil-{research,forms,shopping,saas,booking}-outcomes.py` validators.
`workflow_frontend_evidence.validate_claim` separately binds native task steps,
source files and independently validated output to seven frontend dimensions;
its four covering recipes do not substitute for these 29 cells.

## Framework and reliability production paths

- Eight locked framework builds remain a separate gate. The runner executes all
  framework/sample pairs and checks native actions, post-hydration state and
  diagnostics; final qualification reruns the original validator over raw frames.
- Source review found that framework cases created separate process groups but
  normal/error exits did not inspect surviving descendants. Generated
  `finish_case_process` now reaps and checks each owned group; forced cleanup or
  survivors remain failures. Both integrated and final validators require the
  per-case cleanup record. New regression sources cover clean exit, leaked group,
  failed reap and missing/forged cleanup fields. No historical report is upgraded.
- `workflow_fault_recipes.decorate`, actual fixture fault records and native
  executor fault evidence cover delayed response, worker loss, cancellation and
  duplicate delivery. `run_commit_supplement` additionally exercises before/after
  durable commit with exact recovery reads and native conditional retry decisions.
- Serial 3, mixed 3, recovery 30, four faults and 1/5/20-context topology remain
  frozen. `workflow_execution` persists missing/failed/cancelled attempts and
  transient candidate drift. `workflow_qualification` retains the performance,
  process ownership and churn requirements rather than accepting scheduler overlap.
- Six original-run receipts feed `completion_qualification`: frameworks,
  security, architecture, reliability, native prerequisites and fixture topology.
  Final joins rerun validators and source/candidate/provenance checks; component
  output cannot declare combined qualification.

## Exact outstanding work

1. Original booking timeout causality remains unresolved evidence, not an assumed
   fixed defect. `booking_resolution` requires the retained original failure,
   reviewed mechanism, exact named regression failing before and passing on the
   candidate. No diagnosis or result has been fabricated during generation.
2. Root/P/W/S/V runtime implementations remain the dependencies of these generated
   applications. This audit does not mark those packages source-complete.
3. Framework cleanup additions and all existing family/fault/receipt code require
   build/test correction after the global generation gate. Real trust/topology,
   native timing/cleanup, 29-cell outcomes, all eight framework gates, 752 manual
   observations and final qualification are unexecuted.
4. No further missing family recipe or final-join producer was confirmed in this
   pass. This is not a claim that runtime outcomes already pass; the granular
   parent checklist should separate the generated callers above from acceptance.

## Q-05.c official entrypoint reconciliation

Source-only follow-up; no commands under review were executed.

| Entrypoint | Barrier and retained identity | Remaining execution obligation |
|---|---|---|
| `run-official-wpt.py` → `wpt_official_workflow.prepare/execute` → `wpt-upstream-entry.py` | Shared all-51 gate before preparation/launch; original workflow receipt binds gate, contract, lock and producer. Execution freezes the initial declaration and rechecks all inputs after preflight immediately before launch, then again at termination. | Full selected Core and fixture/native observations. |
| `run-oil-full-contract.py` → `execute_contract` | Required gate before adapter construction; original declaration digest checked with each candidate probe, failed attempts retained. | Frozen full 29-cell repetitions/recovery/fault denominator. |
| `run-workflow-qualification-profiles.py` | Required gate before execution; per-attempt source/digest checks and execution-custody record. | Actual 1/5/20-context measurements and controls. |
| `run-completion-component.py` → `completion_execution.produce` | Required shared gate before candidate read/output/process. Freezes the original declaration before candidate work; checks declaration, candidate and source after preflight before launch and after execution. | Six original component receipts and booking regression evidence. |
| Final `completion_qualification.qualify` | Original generation declaration plus candidate manifest/artifact and source identity rechecked before and after every component and external validator. Detected candidate/source drift remains terminal even when later restored; subsequent validators cannot launch. | Actual complete final join and original evidence review. |
| Official framework child → `run-oil-framework-outcomes.py --generation-gate` | Supervisor now propagates the gate. Child validates before output/native runtime/fixture creation, before each child process and after each result/final aggregation. Shared failure latch cancels subsequent children after observed transient drift, even if bytes are restored. Failed result records and original process outcomes remain distinct. | Eight frameworks × three samples; actual per-child cleanup. |
| Native controls/topology, candidate freeze, assessment-packet CLI | Required shared gate at entry, exact input references and terminal checks in their existing producers. | Actual controls, topology, candidate freeze and manual observations. |
| Direct historical framework/security/architecture diagnostic entrypoints | They remain diagnostics. They cannot supply qualifying original-run receipts outside the fixed gated supervisor. Framework's gate option is mandatory in the supervisor's exact expected command; removing it invalidates receipt verification. | No historical report acquires candidate qualification. |

New regression sources: `test_completion_execution.test_valid_replacement_gate_still_fails_original_run_custody`; `FrameworkGenerationCustodyTests` covers rejection before output/server/native calls, declaration drift retaining failed per-case records without native launch, and cancellation after transient drift/restoration. These are generated source, not test results.

Final-join follow-up adds `test_candidate_drift_stops_later_actions_even_if_restored_at_final_check`, covering manifest/source drift during preflight inventory, a failing component, and the first external validator. The source asserts zero subsequent launches and a failed final result despite restoration. This removes reliance on only the initial/final candidate checks; no validation commands or tests were run.

Manual/procedure reconciliation reviewed `wpt_manual_procedures.assessment_errors`, its platform/repair/packet callers, and regression bodies. Existing 752 entries (749 manual, three visual; 392 ATTA) remain exact procedure obligations. macOS selects AXAPI, without demanding mutually alternative platform APIs. The packet/independent observations and matching repair-delivery receipts remain mandatory. No new missing procedure producer was confirmed in this pass.

Operation reconciliation remains bounded: `wpt_operation_contract` maps the saved 1,516 occurrences to 19 required operations plus 19 conditional Bluetooth helper operations. The file-existence mapping is not an algorithm proof or runtime pass. Prior source closures cover selected-frame upload custody, active-root interrupted input, protocol-handler registration/unregistration and trusted prompts, and exact retired-route terminal cancellation. Shared runtime/AX/navigation changes require their owners' integration reconciliation; this tooling pass does not close whole G-02 or any parent acceptance checkbox.

## Original booking evidence binding — September 17

Read-only inspection confirmed the original `required_blocks`, sample 1 result still reports exit 1 at 30.12 seconds, with the runner deadline not firing; stderr contains only `WorkerLimitExceeded { resource: "wall time" }`. It does not identify the command or establish causality. The preserved result SHA-256 is `0843d5c10df4e893c3ff76d40408446043f32b29c19eb0e180d4e35db775da70`.

The final booking-resolution validator now requires those exact original result bytes, allowing a copied artifact but rejecting another failed attempt or the passing recheck. Added regression source for substitution rejection and acceptance of copied original bytes through the next independent source-evidence check. Original files are unchanged. This closes an evidence-substitution gap only; reproduction, confirmed cause, mechanism fix and before/after execution remain required.

Worker-client source review located existing fixed-label correlation diagnostics at IPC write, IPC response and worker-execution errors. Frame-scoped requests previously collapsed to `scoped_frame`; the label resolver now unwraps native scopes iteratively and keeps the underlying fixed operation label without formatting scope or request payloads. Generated nested-scope regression source checks exact operation/phase/correlation output with private URL, token and script values present in the request. This improves reproduction evidence but does not identify the historical cause. No execution.

The generic `query_or_update` diagnostic fallback is also removed: each WorkerRequest variant now has an explicit fixed label, including selector resolution, observations, layout/paint, accessibility, scrolling and tracing. The exhaustive match requires new protocol variants to choose a label. Regression source includes selector secrets and previously generic query operations across all three timeout phases. Historical failure interpretation is unchanged.
# Profile generation barrier integration review — 2026-09-17

The profile supervisor already checks the shared generation barrier at entry,
before each attempt, and after attempts. Source review found that candidate
loading and source inspection occurred between entry admission and the first
evidence-directory write. It now calls the complete candidate/declaration/source
probe immediately before creating that directory. Regression source
`test_profile_declaration_drift_during_setup_precedes_evidence_creation` simulates
a declaration becoming invalid during setup and requires no evidence writes or
native process launch. This regression was generated, not executed. Full shared
entrypoint reconciliation and F/Q qualification remain open.

Full-contract source review also corrected two candidate-custody gaps:

- `run-oil-full-contract` retains the candidate manifest identity used to bind
  the contract and rejects a different manifest at every later probe. Regression
  `test_full_oil_candidate_cannot_change_between_contract_and_execution_binding`
  requires rejection before source-hash execution or adapter/evidence creation.
- `workflow_execution::execute_contract` now latches cancellation when a
  pre-attempt candidate probe reports different identity or throws. Restoring
  valid bytes at the post-attempt probe cannot readmit subsequent attempts.
  `test_transient_preattempt_drift_stops_later_adapters_after_restoration` covers
  both mismatch and exception while retaining every planned attempt record.

Both corrections and their regression sources remain unbuilt/unrun. They do not
establish completed runtime qualification or close the full Q-05 caller audit.

The shared `completion_provenance::load_candidate` loader now hashes the exact
manifest bytes it parses and verifies that the manifest still has that identity
after artifact/engine/source validation. Previously, hashing only at return
could pair an old parsed manifest with the digest of a replacement file.
`test_candidate_identity_cannot_relabel_previously_parsed_manifest` replaces the
manifest during the engine-validation hook and requires rejection. This is a
generated source regression; no validator or test execution was performed.

Further source review found that profile setup froze the generation declaration
after candidate loading. It now retains the declaration identity immediately
after admission, before setup; a different valid declaration cannot replace it.
`test_profile_valid_replacement_during_candidate_loading_cannot_rebind_admission`
requires rejection before evidence creation and native launch. The shared gate
also hashes the exact declaration bytes it parses and rechecks them after source
validation, rejecting replacement during that validation. Regression source
`test_shared_gate_rejects_replacement_while_sources_are_being_validated` supplies
two independently valid declarations and verifies that only a fresh invocation
can validate the replacement. These regression sources are unexecuted.

Candidate loading also revalidates every artifact record after engine, source
snapshot and workflow-contract validation, before returning candidate identity.
Both bytes and resolved destinations must match the initially observed artifact
map. New regression sources replace worker bytes during engine validation and
retarget a worker symlink to an identical-byte copy; both must reject publication.
This closes an in-validation artifact custody gap without claiming filesystem
locking or runtime qualification. No tests or validators were executed.

## Booking durable ownership join — September 17

Independent source review found that the booking validator checked receipt counts, selected times and final slot availability without fully joining reservation rows to issued offers, accepted request bodies/idempotency keys and slot versions. Added a durable ownership validator used by the historical runner and MixedCellAdapter through their shared original family validator. It rejects foreign-case records, duplicate identities, unbacked accepted requests, mismatched offer time/slot identity, mismatched request bodies/keys, and extra slot-version mutations. Idempotent accepted request replay may still refer to one exact durable reservation. Existing fixture/case versions and expected outcomes are unchanged.

Generated positive replay and nine corrupted-record regression sources. Source inspection confirmed both fixture versions preserve case ids and increment accepted slot versions exactly once; stale offers remain rejected. No tests ran. This strengthens independent acceptance and does not diagnose or resolve the original booking timeout.

### Booking persisted evidence shape and complete inventory

Source review found that malformed reservation JSON could pass through the ownership helper's error collection and then throw in the later full validator, and that unused missing/extra slots were not rejected by the availability comparison. The validator now checks table/row shapes, typed identity/body/status fields, exact integer boolean evidence, and parsed form-body value shapes before joining persisted evidence. Ownership validation requires exactly one case, nonempty unique offer/slot identities, and the fixture's exact slot inventory. Full validation returns evidence errors before unsafe downstream decoding and checks the persisted scenario against the requested case.

Regression sources cover valid persisted evidence followed by sixteen mutations: malformed/missing tables, rows and fields; invalid JSON and form-value shapes; bool-as-version and invalid cookie flags; unhashable/empty identities; missing/duplicate cases; missing/extra slots. They exercise both the ownership helper and full validator error-return path. Fixture schema and case denominators are unchanged. No tests executed; the original booking timeout and full five-family qualification remain unresolved.

### Booking validator caller reconciliation

The v2 fixture extends the legacy tables with string client-timezone fields and integer validity, offer options, and cancellation rows; it uses the same two-slot inventory. Added positive validator calls to v2 regression sources for cancelled-offer recovery, duplicate reservation with a persisted phone option, and correctly shaped rejected client conversions. These are acceptance controls for the stricter persisted-evidence checks, not execution results.

The full validator also previously dereferenced arbitrary action/observation payloads and hashed action IDs before verifying their type. It now rejects malformed result/page/URL shapes and checks positive integer IDs before uniqueness. Seven additional regression mutations cover null/list action results, unhashable and boolean IDs, and malformed observation/page/URL payloads. Valid browser frame validation still retains exact action counts, native execution domain, extracted values and final location requirements.

Re-read the historical signed booking timeout report: original 17/18 plus separate 18/18 remains 35/36. No source-only validator change establishes the missing failing phase or root cause; bounded mixed-run reproduction and original-run closure remain required after global generation.
