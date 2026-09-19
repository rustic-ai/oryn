# Parallel family and qualification source review — September 18

Source inspection and manual edits only. No builds, tests, formatters, generators,
fixture services, qualification scripts or applications executed. Checked source
is not acceptance evidence. All five family and all eight framework gates remain
required; the existing 29-cell contract and sampling denominator are unchanged.

## Exact cell-to-production map

`workflow_native_adapter.MixedCellAdapter` consumes every cell below. Research
recipes come from its `RESEARCH_RECIPES`; the other recipes come from
`workflow_family_recipes.RECIPES`. `PreparedCase.outcome` and the mixed-wave path
reopen sealed SQLite snapshots and call the original family validator before
producing the independent oracle projection. Source inspection confirmed those
caller paths; runtime behavior depends on the corresponding native lanes.

| Cell | Production recipe(s) | Independent evidence |
| --- | --- | --- |
| research.compare | search_paginate_compare | exact facts, canonical links, visits |
| research.pagination | dynamic_results; refreshed_results; hydrated_pagination | page/refresh facts and SSR/module evidence |
| research.frames | nested_cross_origin_extraction | nested same/cross-origin facts |
| research.missing | empty_results; missing_source_recovery; redirected_source | empty/error facts and redirect visits |
| research.stale | generation_bound_stale_reference | selected generation rejection and reacquisition |
| forms.controls | multistep_custom_role; multistep_authorized_upload | ordered fields and authorized file metadata |
| forms.validation | async_email_recovery; multistep_cancel; multistep_cancel_recovery | validation audit and receipt absence/recovery |
| forms.delivery | native_get; native_post; native_multipart | method/body/order/Unicode/raw file hash |
| forms.submitters | native_submitter; native_cancel | submitter method/URL and cancellation |
| forms.reentrancy | native_reentrant | one receipt and exact request history |
| forms.confinement | native_confinement | independent context and byte confinement |
| shopping.discovery | framed_persisted_checkout | both product observations and exact filter result |
| shopping.cart | framed_persisted_checkout | persisted variant/quantity and reload |
| shopping.checkout | framed_persisted_checkout; framed_required_blocks | quote/order/totals/inventory or no order |
| shopping.payment_frame | framed_persisted_checkout | quote-owned authorization and receipt |
| shopping.stale_quote | framed_price_change_blocks/recovery; framed_stock_change_blocks/recovery | old quote rejection and accepted current quote |
| shopping.duplicate | framed_duplicate_checkout | one durable order despite replay |
| saas.pkce | pkce_create_update_reload_logout | real code/S256 exchange and auth history |
| saas.mfa | pkce_mfa_create_update_reload_logout | authorized challenge boundary |
| saas.records | pkce_create_update_reload_logout | record/version and reload/logout |
| saas.session | pkce_logged_out_access_denied; pkce_expired_session_save_recovery | denied expired write and reauthentication |
| saas.oauth_rejection | oauth_reject_state/redirect/verifier/replay/expiry | exchange rejected without authorized session |
| saas.save_recovery | pkce_concurrent_edit_recovery | stale version and exact write history |
| saas.isolation | pkce_isolated_records | independent context denial/redaction |
| booking.timezones | client_fold_first/second; client_vancouver/utc; client_new_york_gap; client_vancouver_gap | UTC/zone/fold choice and gap rejection |
| booking.reserve | client_phone_option; client_required_validation | option/slot ownership and receipt absence |
| booking.reload | client_fold_first | same reservation identity after reload |
| booking.stale_slot | client_stale_recovery | old slot rejection and replacement history |
| booking.cancel_retry | client_cancel_retry; client_duplicate_request | cancelled absence and one accepted reservation |

No missing recipe was confirmed. All these native behaviors still require the
shared generated browser implementation and later integrated execution.

## New source corrections

### Shopping durable acceptance

`run-oil-shopping-outcomes.validate_persisted_ownership` now runs before the
existing validator accesses persisted rows. It validates table/row/scalar/JSON
shapes; case ownership in every table; one exact scenario; unique quote and order
identities; the issued-quote counter; quote arithmetic; accepted request recipient
and quote fields; order-to-quote values; and the complete quote/payment inventory.
Payment events must belong to their own quote, and authorized receipt identities
must be unique. Previously matching aggregate payment-event names/counts could
hide attribution to the wrong recovery quote. Repeated accepted requests may
still reference one exact order; the outer recipe retains exact request counts.
Malformed action and observation payloads now return evidence errors before
field access, URL parsing or uniqueness checks.

The shared validator reaches both historical shopping runs and mixed-family
`PreparedCase`/wave callers; no fixture behavior, task, denominator or expected
outcome was relaxed. Raw request body hashes are retained; decoded fields are
not re-encoded and misrepresented as a reconstruction of original bytes.

Generated regression sources:

- `test_oil_shopping_outcomes`: foreign rows, duplicate/unbacked quotes/orders,
  wrong accepted recipient, quote counter/arithmetic, malformed table/JSON/frame
  shapes, and accepted replay with one durable order. Existing full positive
  persisted case remains the acceptance control.
- `test_oil_shopping_frontend`: actual v2 SQLite positive authorization/replay,
  two-review positive control, foreign/missing payment evidence, reassigned
  recovery-quote events and duplicate receipt identities.

### Integrated cleanup evidence

`run-oil-integrated-outcomes.validate_summary` no longer accepts dictionary
values through Python's `False == 0` / `0.0 == 0` equality. A structured fixture
cleanup needs integer-zero returncode and explicit boolean-false forced status;
the existing explicit `True` producer form remains valid. Generated regression
sources reject false-like/missing values and retain both valid producer forms.

## Framework and candidate joins

The separate locked framework denominator is React, Preact, Vue, Angular,
Svelte, Next, Nuxt and SvelteKit, each three samples. Final `framework` rebuilds
expected task identity, revalidates original stdout through the framework
validator and checks process cleanup. The candidate's framework build hash is
passed to integrated summary validation. These source paths remain intact.

The full workflow final join independently recreates the candidate-bound frozen
contract, complete attempt list and expected projections. It checks pinned
projection/action/topology/resource/fault/frontend artifacts through
`workflow_evidence.reconcile`. Original durable family validation occurs in the
producer before projection; this review does not equate the final projection
comparison with rerunning every family validator over raw databases. Further
raw-evidence replay/custody reconciliation, if required for whole Q-06 closure,
remains explicit rather than inferred from a matching projection.

Frozen input locks must be deliberately regenerated only for the final candidate
after global generation. These edits intentionally do not modify historical
locks or evidence.

## Original booking failure: evidence limit

Read the original `benchmarks/evidence/booking-signed-2026-09-14/required_blocks-01`
result and final database and the preserved investigation. Result: exit 1,
30.12 seconds, `timed_out: false`; database: zero reviews, offers, requests,
reservations and views, both original slots available at version 1. The original
result remains bound to SHA-256
`0843d5c10df4e893c3ff76d40408446043f32b29c19eb0e180d4e35db775da70`.

The unlogged start redirect and buffered OIL stdout prevent assigning the error
to a command. Historical worker-wall-limit output does not distinguish IPC wait
from worker execution. Concurrent workloads do not establish resource contention
as the cause. Initial 17/18 plus separate recheck 18/18 remains 35/36. No timeout
increase, fixture workaround or fabricated diagnosis was introduced.

Existing fixed command labels, correlation/phase diagnostics and bounded mixed
fault recipes provide source for later reproduction. Confirmed cause, independent
review and same named before/after regression evidence remain required by
`completion_qualification.booking_resolution`. Source-only work cannot supply
those execution facts.

## Remaining dependencies

- Native lifecycle, cross-origin frame/input, submission/file authority,
  storage/network, module/hydration, timezone and rendering source closure in the
  corresponding lanes; no new production hook request arose from the validator
  changes above.
- Whole-Q raw producer/consumer custody reconciliation and final fixture/task/
  adapter/source lock refresh, preserving historical identities.
- Global generation declaration; then build/test/fix, every planned family and
  framework repetition, 1/5/20-context profiles, fault/recovery measurements,
  original booking causality, full pinned Core and final candidate qualification.

No acceptance checkbox should be closed from this source review alone.

## Q-06 complete raw family replay — subsequent source closure

This section supersedes the earlier statement that final durable replay remained
producer-side. `workflow_raw_evidence.py` is now required for every manifest-bound
full-contract attempt. Historical diagnostics without a candidate manifest do
not acquire qualification by this change.

### Production and final consumers

`MixedCellAdapter` retains a complete raw manifest: every primary recipe wave,
every primary/companion context, exact native plan/result, sealed SQLite bytes,
sealed JSON snapshot, fixture cleanup and native resource/cleanup artifacts.
The manifest also retains each actual adversarial context and both required
before/after-commit supplemental waves. The fresh workflow input inventory
already includes `workflow*.py`, so the new replay implementation becomes part
of the next candidate's source/input lock. Historical locks remain unchanged.

Final reconciliation derives the wave/context recipe denominator from the same
frozen cell contract, reconstructs source-defined task/options/upload authority,
fault decoration and recovery commands, and checks original attempt/worker
identity and all native terminal/cleanup evidence. It reads the artifact's
verified SQLite bytes into a private temporary file before invoking the fixture
snapshot API; changing an evidence pathname during parsing cannot change those
bytes. No fixture process or browser is launched by this replay.

The shared `replay_case_outcome` runs original family validation and computes the
observed projection/actions. Final acceptance requires both to equal the stored
projection/actions, in addition to matching the independent expected contract.
All 29 cells, all concrete recipes and mixed companion contexts take this path.
The four frontend witnesses continue to provide their separate dimension checks.

### Fault, adversarial, commit and resource custody

- Ordinary fault records are reconstructed from native session evidence and the
  actual sealed response-withholding table, then compared to retained records.
- Confinement replay checks the exact six native denial steps, owner-only secret
  label, empty foreign-case receipts/requests and synthetic-secret hash redaction.
  Only its digest is retained; the original synthetic secret file stays deleted.
  SaaS replay checks the exact independent-context unauthorized read/write steps.
  Missing, extra, reparented or authorized adversarial sessions reject acceptance.
- Both commit phases are required for applicable worker-loss/cancellation cells.
  Final replay checks the source mutation boundary, exact gate token/phase and
  conditional recovery URL, actual inflight retirement, one retained interruption,
  the three original inspect/resolve stages, and sealed commit/read/retry tables.
  A gate may become observable in a later native step while an asynchronous write
  proceeds; that explicit production behavior is retained. No failed operation is
  relabelled as success. Retry token/body ownership and typed timing checks were
  strengthened in the shared producer/final helper.
- Resource accounting is recomputed from raw native evidence and pinned OS
  samples for baseline and supplemental waves. Original parent cleanup and every
  fixture cleanup remain required. Baseline wave resource references and topology
  must match the manifest; supplemental topology/activity are recomputed.
- `completion_qualification.input_inventory` already freezes all original raw
  evidence files before validation and checks the inventory after validation.
  Component receipts independently bind the complete original artifact set;
  existing candidate/source/generation lease checks remain in force.

### Generated regression sources

`tests/test_workflow_raw_evidence.py` contains an actual research SQLite + original
validator positive control; rehashed database/native-output corruption; source
recipe/task/upload/fault/worker identity mutations; omitted/duplicated wave and
context controls; full 29-cell recipe and fault-source reconstruction; malformed
SQLite and immutable-byte snapshot controls; missing commit phases; retry token,
boolean timing/match and committed-retry rejection; independent confinement
mutation/redaction checks; and native cleanup/accounting recomputation controls.
The manifest identity regression in `test_workflow_evidence.py` now explicitly
isolates raw replay, whose full controls live in the new suite.

These are generated source contracts, not executed results. The remaining work
for this scoped F/Q source batch is shared integration review and later validation;
no further confirmed missing family recipe or raw replay producer is recorded.
Original booking causality remains unresolved and cannot be supplied by source
inspection. Global builds/tests, deliberate candidate/lock freeze, all repetitions,
full Core, actual measurements and final qualification remain open.

## Independent malformed-input and final-provenance reconciliation

The outer workflow reconciler already retained most malformed payload errors;
final qualification also rejects exceptions. Direct raw replay helpers could
still raise incidental field/type/index exceptions. The raw boundary now checks
object/list/row/runtime/step/recovery/gate/resource shapes before access, rejects
duplicate JSON keys and nonfinite constants, and converts residual malformed
nested domain-field errors into `ValueError` with a fixed boundary label. It
never skips failed evidence or exposes the malformed payload in diagnostics.
Generated controls cover malformed manifests, wave/context rows, references,
URLs, sessions/runtime/steps and duplicate/nonfinite JSON.

Original-run receipt review found two concrete truthiness gaps: boolean/float
zero could count as successful returncode, and missing/null receipt errors could
count as no errors. `verify_receipt` now requires integer-zero exit and explicit
empty errors, typed receipt/preflight/execution objects, and a nonempty artifact
list. Artifact path/candidate/component/report shape checks in provenance now
return typed rejections. Framework final replay rejects an already-invalid
summary before dereferencing individual attempt directories. Generated receipt,
candidate/path and existing original artifact-inventory controls cover these
changes. Original artifacts and outcomes are not rewritten.

The candidate-freeze CLI still used only initial/final generation checks around
source inventory and output creation. It now freezes a `GenerationLease` before
setup and checks the original declaration immediately before output and candidate
publication and at completion. Generated replacement-during-inventory control
requires no archive/output/persist work. This does not execute the freeze tool or
regenerate locks; final versioned locks must still be produced deliberately after
the complete generation gate on the integrated source tree.

The final input inventory recursively covers workflow/profile/component raw
files, component receipts require exact original artifact sets, and the new raw
replay file is included by the existing `workflow*.py` input rule. The input-lock
regression source now explicitly covers its inclusion and changed-source
rejection. No frozen denominator, fixture expectation or historical lock changed.

## Native deferred Fetch handoff (source generation, 2026-09-18)

The reassigned native lane generated `crates/oryn-native/src/deferred_fetch.rs`, ContextService admission/activation/cancellation plus container reservation/release, host adapters, frozen standalone authority capture, document/BFCache/page-close flushing, and terminal-context cancellation with bounded cancellation-custody draining. The navigation lane owns the JS binding/provider and the loading lane owns pre-IO parent-driven frame reservation calls. No build, test, formatter, generator, service, or WPT execution occurred in this lane.

Authoritative source checked: WHATWG Fetch PR 1647, head `26b5c4907809719c14965c562370b69157c608ae`, `fetch.bs` algorithms “queue a deferred fetch”, “total request length”, “available deferred-fetch quota”, “reserve deferred-fetch quota”, and “deferred-fetch control document”. This source explicitly sets service-workers mode to `none` and keepalive to true. The direct native HTTP path therefore retains the normal broker/CORS/preflight/CSP/COEP/referrer/cookie/redirect checks while bypassing service-worker interception as specified.

Quota implementation uses a default 512 KiB top-level usable pool (640 KiB when the minimal policy is disabled), 64 KiB per reporting origin within the same control-document group, 64 KiB normal child reservations, and sixteen 8 KiB minimal reservations. Reservations are keyed by the stable `(page, frame token)` browsing context, captured before parent-driven network submission, released on actual same-origin commit/removal, and remain charged across cross-origin redirects. Same-origin ancestors separated by a cross-origin intermediate document do not share the top-level group. Request charging includes the serialized URL excluding its fragment, headers/body, and native effective referrer; explicit empty referrer stays empty.

Twenty-seven native deferred Fetch regression sources now cover:

- Production preparation distinguishes native document URL from the storage partition string.
- Navigation reserves child quota before any fetchLater call; minimal children stop at sixteen and removal restores the slot.
- CORS preflight success precedes the payload, with credentials omitted for preflight.
- Preflight rejection never sends the deferred payload.
- Redirects recheck the frozen CSP and never contact a blocked target.
- Mixed-content rejection and scope cancellation prevent broker entry.
- Per-reporting-origin accounting accepts different reporting origins.
- Same-origin siblings share quotas; independent cross-origin siblings retain their own group.
- Top-level aggregate and delegated reservations constrain admission.
- Cancellation and activation release request quota without replaying IO.
- Unknown-capability cancellation is durable across a lost preparation response and is page-bound.
- Replay rejects changed request bytes, creator, page, and malformed capability length.
- Context shutdown clears pending work and revokes admission.
- Frozen CSP/referrer ownership survives caller mutation.
- 8,192 same-document completed/cancelled operations retain bounded ACK history without exhausting pending capacity.
- 8,192 unknown-cancel attempts retain bounded tombstones and expired old requests cannot be resurrected.
- Foreign Creator high sequences cannot advance a live issuer watermark, and actual document retirement reclaims the issuer.
- Same-origin child admission still respects its own restrictive Permissions-Policy, independently of its quota owner.
- Standalone roots have independent sequence namespaces, and retired provider authority cannot reopen an issuer.
- Native deadlines win races with late abort; canceled deadlines cannot reactivate.
- The native scheduler dispatches an expired request without a JS poll or activation call.
- Integrity verification rejects mismatched final representations and ineligible opaque/manual responses.
- Redirect method/body/authorization rewriting, credential handling, and same-origin mode remain enforced.
- Report-only COEP uses the frozen reporting endpoint without blocking delivery.
- An unobserved activated result survives 8,192 acknowledged requests from another document.
- Unacknowledged terminal outcomes retain bounded custody; explicit acknowledgement restores admission capacity.
- Activated deferred delivery observes the ordinary Fetch keepalive body budget and terminates without IO when that budget is exhausted.

All are **generated, not run**. The scheduler reserves a bounded execution slot before admission and retries failed executor/cancellation-custody admission without dropping activated requests. Permanent executor initialization failure rejects admission. A parent-owned deadline and atomic activation/cancellation winner are observable through the native state query even while JS is blocked. Protocol/frozen-input source reconciliation must include `PrepareDeferredFetch` (including optional duration), `ActivateDeferredFetch`, `QueryDeferredFetch`, `CancelDeferredFetch` (including frozen Creator), `AcknowledgeDeferredFetch`, `ReserveDeferredContainer`, `ReleaseDeferredContainer`, `DeferredFetchResult`, and `DeferredFetchState`. Response payloads use the wire name `outcome` to avoid collision with the enum discriminator `result`. Collective generators remain deferred to the root’s final source barrier.

Frame removal now owns a cleanup slot before initial reservation, stores that lease on the native FrameLifetime, and retries `ReleaseDeferredContainer` until its typed acknowledgement. Two cleanup-custody regression sources reject a wrong response kind and verify terminal client teardown releases still-live frame leases before draining; an ordinary drain does not revoke live leases.

The capability remains 32 bytes: a checked per-Document 64-bit sequence followed by 24 random bytes. Every attempted JS admission advances the sequence. Native exact-Creator watermarks make evicted old tokens explicitly expired. Pending and unacknowledged terminal entries share a 4,096-entry custody limit; unobserved outcomes never enter eviction history. JS first preserves the terminal boolean in its result wrapper, then acknowledges; transport failure retains ACK retry custody. Only acknowledged outcomes enter the bounded 1,024-entry replay history. Unknown cancellation carries the Creator captured by its per-call backend and must authenticate a live issuer; known accepted cancellation remains valid after original-document retirement. BFCache flushing preserves observable outcomes; actual document retirement acknowledges unreachable outcomes and reclaims the issuer watermark. Standalone providers freeze root-page identity and exact document root/generation/authority epoch.

Source reconciliation covers parent-driven pre-IO reservation, initial inherited frame reservation, same-origin commit, frame removal, BFCache, actual document replacement, and terminal context cleanup. Scheduler locks are released before registry dispatch; asynchronous broker work owns frozen plans and cleanup tickets independently. Full integration compilation and execution remain deferred. Shared ordinary/deferred keepalive source generation now uses a native exact-environment 64 KiB body budget. Four budget regression sources cover cross-producer accounting, shared cancellation custody, retirement, overflow, and redirect body removal. Deferred activation reserves the same budget that the loading lane uses for ordinary Fetch; HTTP cleanup tickets retain its lease through cancellation acknowledgement and release after full EOF. `FetchOptions.keepalive` and service-worker `AgentFetchRequest.keepalive` preserve metadata through native producers. The loading lane owns the remaining ordinary Fetch frozen preparation and post-retirement response-draining integration; this audit does not claim that cross-lane implementation has been compiled or verified. No conformance outcome is inferred from generated tests. Historical booking-timeout causality remains unresolved as recorded above.

### Ordinary worker Fetch keepalive source reconciliation

`AgentOperationExecutor::prepare` now freezes an accepted AgentIo request before its asynchronous launch. The retained receipt holds the immutable native worker bootstrap, exact selected service-worker controller/version, and a bounded orphan-response slot. Ordinary agent retirement preserves these accepted requests; explicit cancellation removes preparation and cancels returned streams. A separate cancellation-disposal method distinguishes AbortSignal cancellation from loss of a retired JS response consumer.

Agent payload IO reserves the shared exact-AgentId body budget after preflight, carries its lease in cancellation custody, shrinks the charge when redirects remove the body, and releases it at full response EOF. Retired consumers hand response streams to the bounded native drain without blocking on a concurrent reader; transport failure, deadline, or actual browser-context shutdown ends that drain through retained cancellation custody. Browser shutdown force-cancels and drains the dedicated keepalive cleanup scope. Service-worker interception uses the frozen controller on every redirect rather than querying a removed worker client.

Generated source controls cover synchronous preparation before asynchronous launch, retirement without canceling the accepted operation, preparation canceled before retirement, dispatch after the original native worker authority retires, and orphan EOF restoring the original worker's body budget. The shared Budget has four independent regression sources; the HTTP cleanup regression retains the byte charge through a failed cancellation acknowledgement. These sources have not been built or run. The loading/authority lanes own ordinary Window receipt and response-lifetime integration; the renderer lane owns the standalone Window adapter.

### F/Q explicit feature-profile reconciliation

The pinned Core entry `core-html-0014`, `html/semantics/permission-element/usermedia/legacy-mode/legacy-mode.https.html` (source SHA-256 `98ab277067734df102eceb63c47656880249cac386a072cd5566ea4dfb313702`) explicitly requires the `UserMediaElementLegacy` virtual configuration. A URL-triggered behavior switch would not satisfy that requirement.

Generated native configuration now uses immutable `ContextOptions.web_features`, with `usermedia_element` explicitly `standard` or `legacy`. Normal contexts default to standard; official fixture contracts require an explicit selection. The active, socket-page-bound `WebFeatureProfile` query supplies the context profile before binding installation. The automation lane owns the frozen binding slot and actual legacy behavior, including constructor members, invalid type handling, and no stream/event production on granted legacy activation. It also owns the standalone setter, which rejects profile changes after binding installation.

The official producer binds the feature profile and its digest into the candidate fixture, run_info, prepared lock and execution record. Original-run replay checks both feature and cookie profiles against the exact candidate fixture and preserved raw report, so relabeling a report and updating only its raw hash cannot qualify it. Profile comparison also binds every unchanged fixture setting; separate standard/legacy Core runs retain the entire denominator and cannot be combined by selecting each URL's better outcome. `compare-wpt-feature-profiles.py` verifies original execution custody before emitting that explicitly non-conformance comparison. The existing malformed indentation in `wpt_official_workflow.verify_inputs` was repaired during this source reconciliation.

Generated regression sources cover native serialization/defaults/invalid and URL-shaped profiles; fixture application before context creation; profile mutation requiring a new fixture/candidate identity; strict comparison with unchanged coverage and environment; and report relabeling rejection. No generators, native builds, tests, fixtures, or qualification runs were executed. Final frozen locks and accepted execution evidence remain pending the complete generation barrier. Existing 29 family cells, eight framework gates, reliability sampling, original booking failure and full Core IDs are unchanged.

### Subsequent complete F/Q acceptance-gate audit

See [FQ-ACCEPTANCE-GATE-AUDIT-2026-09-18.md](FQ-ACCEPTANCE-GATE-AUDIT-2026-09-18.md) for every F-01–F-04 and Q-01–Q-06 parent row, exact source owners and deferred execution. The fixed population remains 29 cells / 688 original attempts, eight frameworks / 24 original samples, and 210 performance/churn profiles. Confirmed consumer repairs bind all benchmark and raw worker identities, reject missing profile error custody, require original failing booking regression custody and actual before/after source-archive differences, preserve the complete original booking trace, remove the diagnosis/candidate hash cycle using a frozen output path, and include native media in the exact final component set. Positive/negative regression sources are generated; none have been run. Original booking causality remains unresolved and all acceptance parents remain open.
