# Parallel authority and storage source closure — September 18

Status: source inspection and generation only. No build, tests, formatter, generator, server or conformance execution. This supplements the complete outer-request table in `P03-W05-OPERATION-AUTHORITY-INVENTORY-2026-09-16.md`; it does not supersede its remaining qualification gates.

## New production corrections and regression sources

1. `storage_idb_connections.rs`: a close-pending connection remains a version-change blocker while its transactions exist, but receives no new versionchange notification. Its close-pending state satisfies notification completion, so an upgrade can publish blocked rather than wait forever for a notification the JS producer no longer polls. JS evidence: `BindingState::poll_idb_connections` polls only nonclosed databases and delays native normal close until outstanding transactions finish.
2. `storage.rs::execute_idb_coordinated`: forced `RetireConnection` validates original origin/database first and aborts snapshots matching exactly that connection, origin and database before releasing its lease. Normal close still preserves existing transactions. Retiring one owner cannot abort a sibling, and an uncommitted working copy never becomes committed data.
3. `storage.rs::execute_idb_transactional`: native-scoped script Transaction correlations require a previously admitted connection-backed snapshot. A correlation cannot implicitly create a transaction, bypass live connection admission, or revive a completed snapshot. The native namespace is created by `scope_idb_transaction` only after authenticated Window/Agent ingress. Unscoped trusted profile tooling retains its direct transaction API. This is a denial-only check on the canonical tuple namespace; no transport fields or wire constants changed.
4. `window_idb_custody.rs` regression now admits native connections and BeginTransaction before using scoped requests, retaining its original old-document rollback and replacement-owner isolation assertions.

Generated tests:

- `close_pending_notifications_and_forced_retirement_preserve_exact_connection_ownership`: memory and persistent stores; close-pending notification suppression, acknowledgement progression, foreign retirement rejection, queued sibling admission after forced abort, idempotent old cleanup, no uncommitted publication, and profile reopen with committed schema only.
- `scoped_script_requests_require_admission_and_cannot_revive_committed_transactions`: reject before Begin, accept under live native connection, reject after commit, deny new admission on retired connection.
- Existing `document_replacement_rolls_back_only_its_exact_transaction_owner` source reconciled with explicit Begin/connection requirements.

## Privileged consumer and lifetime matrix

| Family | Native authority and producer | Completion/cancellation ownership | Current disposition |
|---|---|---|---|
| Window IndexedDB | Parent Storage ingress validates command family; `window_idb_custody::Ledger` scopes token to exact AgentOwner with page/target/generation/epoch and revalidates live authority under entry serialization | Window ledger retains transaction/connection/upgrade rollback; retirement retries failed store cleanup | New shared backend admission/forced-close fixes above; complete JS event ordering remains open |
| Agent IndexedDB | `worker_service::agent_indexed_db` uses current worker bootstrap partition/native origin, rejects opaque, validates commands and scopes token to AgentId | `AgentIo` owns operation serialization and connection/transaction ledgers; finish_retirement joins IO first, aborts transactions, retires connections, preserves failed cleanup | Same backend fixes apply; agent live resource cap 64 per connection/transaction registry |
| IDB profile persistence | Persistent profile shares mutation mutex, snapshots and connection directory across contexts; one read/modify/commit sequence | Persistent execution clones metadata; disk commit succeeds before next connection/transaction directory publication | Existing commit-fault sources retained; new restart regression generated |
| Message operations | `MessageRequest::requires_window_document_scope` requires InDocument for channel creation, transfer, post/poll, broadcast and pending checks | ClosePort, DiscardTransfers, BroadcastClose retain token-only cleanup semantics; response custody separately owns transferred capabilities | Current enum classification inspected; existing nested inventory retained; no new blanket retired-route exemption |
| Agent/service platform operations | `AgentServiceRequest` separates live submit/import/create from cancel/retire/release-result; transport is fixed to original principal | AgentIO network scope drains exact operations before retained storage cleanup; WorkerTransport retirement revokes before drain | Full operation-specific receiver source remains in earlier inventory; real signed process qualification open |
| Cookies | `ContextCookieStore` derives schemeful site, top-level partition, ancestor bit, request kind, credentials and redirect-chain site status | Cookie updates persist before publishing memory; replacement preserves creation age; failed persistence leaves original state | Existing sources cover HttpOnly, Secure/prefix, SameSite redirect laundering, partitions/ancestor bit, expiry/eviction and migration failures; full pinned mapping still open |
| Web Storage | Native document owner, nonopaque origin and session scope; native named-property and method routes converge | Native committed write captures source URL and exact recipient Creator; no-op writes suppress events | Existing persistence overlap/quota/receiver tests are generated; no execution claim |
| Service profile lifetime | Durable registrations/graphs and profile coordinator are distinct from active document/agent principal | Active/waiting restart reopening exists; unfinished installer loss cancels its executor-owned job rather than fabricating successful migration | Existing restart requirement reconciliation applies; signed/runtime/fault qualification remains open |

## Bounds inspected

Native profile IDB connection count and transaction count each cap at 1024. Connection metadata caps at 8 MiB, each name/token at 64 KiB; per-connection notification queues cap at 1024; private upgrades and transaction snapshots cap at 256 MiB. Window custody ledgers cap at 1024. AgentIO resources cap at 64. Worker process environment registration caps at 256. These are individual inspected bounds, not a proof of every process/group fan-out budget.

## Remaining source dependencies and gates

- Root retains shared runtime/host/context-service ownership. This batch needs no root integration hook or wire regeneration.
- Navigation/related-root integration belongs to the navigation lane; service source grants must continue to use exact target snapshots and existing navigation admission.
- Full Core cookie/storage mapping, IDB JS event/transaction ordering matrix and all service restart cases are still open. New tests are assertions authored in source, not evidence of passing behavior.
- P-03/P-05/W-04/W-05 parent rows remain open for exhaustive caller reconciliation and final qualification. Do not convert the bounded fixes above into package completion.

## Agent retirement failure fan-out follow-up

`agent_io.rs::finish_retirement` now attempts the main network scope, every retained preload scope, all transaction aborts and every connection retirement even if an earlier independent owner fails. Only failed preload owners and failed IDB entries remain retained. IDB cleanup snapshots each ledger before persistence, so no ledger lock spans native storage work. Successful cleanup removes only the same token/database entry observed in that snapshot. The caller still joins all IO tasks before this method; this is cleanup fan-out, not new live-operation admission.

Generated source controls `failed_cleanup_retains_only_failed_owner_and_never_holds_ledger_during_release` and `cleanup_ack_does_not_remove_changed_ledger_identity` cover later-owner progress, exact failed-entry retention/retry, absence of persistence under the ledger lock, and rejection of replacement-entry consumption. These helper regressions remain unexecuted; real process teardown fault qualification remains open.

## Same-arena adoption publication custody

Root-assigned cross-package fix in `crates/oryn-js/src/v8_bindings/node_migration.rs`: same-arena adoption now pre-reserves publication queue capacity and tracked node leases before native commit, then immediately retains the post-commit tail in the existing agent queue. Text tracks, audio/video tracks, stylesheet invalidation and animation migration each advance independently. Failure cannot lose the tail merely because nodeDocument already equals the new document on retry.

Parser lane confirmed `resume_pending_mutations` issues ParserAction::Checkpoint and `document_parser_gate` calls `node_migration::resume_publications` before retrying Insert. No new parser hook is required. `same_arena_adoption_retains_each_publication_tail_after_native_commit` injects failures at each of four stages, checks committed parent/child owner identity and retained publication, and checks that retry does not repeat completed stylesheet state reset. Source-only.

## JS IndexedDB request admission custody

`idb_admission.rs` now owns flush logic, called by the existing `V8BindingState::flush_queued_idb_operations`. An original queued request and completion action remain retained until native submit succeeds. Failure no longer deletes retry metadata or loses the request. Only successful connection submissions occupy the database serialization slot, preserving connection request ordering across retry.

`submit_idb` accounts transaction pending requests and ordered request IDs before fallible native flush. An admitted queue item therefore prevents premature auto-commit even if native submission needs another checkpoint; retry does not increment transaction accounting again. Exact shared monolith edits were authorized by root and limited to module declaration, flush method and submit_idb accounting order.

`native_admission_failure_preserves_request_action_and_database_order` is a generated regression source for native failure followed by successful retry, retained original action/request metadata, same-database open ordering and transaction accounting exactly once.

### Current JS event/retirement source map

- `submit_idb`: exact receiver origin, transaction token wrapping, private upgrade connection wrapping, bounded queue (16,384), request accounting and native admission.
- `idb_transaction_ready` / `idb_operation_ready`: earlier overlapping writes/versionchange block later transactions; native Begin still records creation order.
- `finalize_idb_open` / blocked-completion queue: open remains active through upgrade/blocking tasks rather than just the first host response.
- Native connection poll: `last_versionchange_event` is advanced before callback; failed ACK submission retries without repeating callback. Callback checkpoint runs before ACK.
- Successful request completion: result publication, transaction activation through event/microtasks, thrown-listener abort unless explicit commit, request completion accounting.
- Deferred abort/open failure tasks: request error state is published before error event and request accounting; transaction complete/abort tasks are separate tasks.
- Retirement: host revoke/cancel precedes clearing pending, queued, retry, blocked and deferred delivery registries; native Window/Agent IDB ledgers separately retain rollback custody.

This map is not yet exhaustive fault qualification of every callback/allocation boundary. In particular, generic successful/error event dispatch and result publication have additional fallible V8 edges that need retained-progress review before broad event-order closure. No runtime qualification claimed.

## Staged IDB delivery continuation — supersedes the event-custody gap above

Generated `idb_delivery.rs` and native `idb_deliveries` state. The completion handler transfers the accepted IndexedDB response/action into a retained task before author callbacks; host acceptance is independent of later V8 publication failure. Superseded direct IndexedDb success/error branches have been removed.

Each task retains result construction (including stable database/transaction/cursor IDs and partial batch values), result publication, transaction activation, event delivery, abort/upgrade suffixes, request accounting, connection flush and polling as separate steps. Request accounting records completion before a fallible suffix, so retry cannot decrement another request. Deferred request/open errors, blocked notifications and deferred transaction terminal events use the same continuation. Native failed-upgrade submission queues once before its error-event suffix; cancellation preflights all request records and queues retained abort deliveries rather than losing a taken list at the first fallible internal finish.

`native_platform/publication.rs` has an explicit IDB path constructor for request → transaction → database capture/bubble dispatch, plus a retained thrown-listener flag. It reuses existing per-listener progress: callback effects commit once, and allocation/fault retry resumes remaining listeners. Event cancellation is read from the native event record, never a script-replaced defaultPrevented getter. The task queue has a reentrancy guard, exact document/generation/authority identity, retirement cancellation and explicit listener-graph active roots.

Generated regression sources:

- `result_event_and_accounting_failures_resume_without_repeating_author_prefix`
- `error_path_preserves_parent_capture_bubbling_and_prevent_default_after_retry`
- `thrown_success_callback_and_reentrant_abort_keep_remaining_listener_suffix`
- `deferred_error_and_document_retirement_never_replay_old_callbacks`

These cover failures before result preparation, before event preparation, after the first author callback and after accounting; parent path/default prevention; reentrant abort and thrown callback; and retirement of a partly delivered old-document event. Source generation only, no execution. Platform-wide non-IDB completion behavior was not rewritten.

## Storage Access API and initial srcdoc lifetime dependency

Pinned source dependency: `html/semantics/embedded-content/the-iframe-element/srcdoc-lifecycle-crash-crbug-1472607.https.html`; pinned `interfaces/storage-access.idl` defines the two no-argument Promise methods.

Generated `storage_access.rs`, explicit native Document prototype installation and authored WebIDL for `hasStorageAccess` / `requestStorageAccess`. Receiver resolution finds the exact active Document; inert, detached, retired or suspended documents cannot borrow replacement authority. Promise settlement invokes no frame navigation or parser progress, so an initial inherited about:blank Document remains the same Document while its request promise settles. Decisions use native secure origin, active frame-parent generation, Permissions Policy, committed sandbox snapshot, schemeful registrable-site comparison, exact native permission state and native activation.

Added committed `storage_access_allowed` snapshot to binding state. Host realm inheritance, frame binding, HTTP frame commit, standalone resource policy and root CSP setup conjoin the new `SandboxFlags::allow_storage_access_by_user_activation`; author attribute changes cannot retroactively relax committed policy. No protocol version was regenerated.

The subsequent native-profile batch below adds explicit Block/Allow context policies. Both Storage Access methods now query the same native cookie decision after active/nonopaque/secure checks. Existing-cookie-access resolves after document/security/sandbox/Permissions-Policy gates; a denied permission does not revoke access already permitted by the native cookie policy. There is no JS-accessible authority setter and no WPT URL-specific override.

Generated controls: `inherited_blank_storage_access_is_native_promise_and_does_not_force_navigation`, `storage_access_checks_secure_origin_activity_and_native_policy`, `storage_access_sandbox_flag_is_conjoined_and_round_trips`; existing host modal sandbox test now also asserts StorageAccess snapshot immutability. No builds/tests/conformance executed.


## Continued caller reconciliation: poll, enumeration, finalization

Source generated (not executed):
- `IdbConnectionPoll` transfers accepted native response to `idb_delivery` before callbacks. Versionchange event preparation/listener continuation, microtask checkpoint, and native ACK submission have independent retained stages. Duplicate native notifications ACK without replay; failure after first listener resumes the remaining listener, not a freshly dispatched event.
- `IdbDatabases` transfers response/error + PromiseResolver into retained delivery. Array/property/exception preparation and promise settlement failures retain custody; replacement generation cancels old-realm publication.
- Deferred `FinalizeIdbOpen` and `FinalizeIdbTransaction` transfer into retained delivery before fallible native queuing. Hidden request identity survives retries; successful queue owns command before independent native flush. Active roots include hidden cleanup request and all pending open-finalization IDs.
- Regression sources cover listener-prefix failure plus ACK rejection, duplicate notification, enumeration settlement/replacement, and finalization retry preserving hidden request/one accepted commit.
- `fully_active` uses native live/suspended identity and exact connected ancestor generations. Unloading/task admission closure is not by itself active-document replacement, so it is not an extra Storage Access fully-active rejection.

## Exact pinned Core cookie-profile dependency (OPEN implementation)

Pinned checkout revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`; Core manifest hash remains `566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101`.

| Core root | Transitive source | Native obligation |
| --- | --- | --- |
| `cookies/third-party-cookies/third-party-cookies.tentative.https.html` | `resources/third-party-cookies-cross-site-embedder-opener.html` → `third-party-cookies-cross-site-embedder.html` and `third-party-cookies-cross-site-embed.html` | Default cross-site HTTP read/write and embedded DOM read/write blocked; first-party cookies retained and readable. |
| `cookies/third-party-cookies/third-party-cookie-heuristics.tentative.https.html` | `resources/third-party-cookies-cross-site-popup-opener.html` → popup → `third-party-cookies-cross-site-popup-verify.html` | Default cross-site block, then actual popup heuristic grant enables origin cookies in cross-site requests. |
| `html/semantics/embedded-content/the-iframe-element/srcdoc-lifecycle-crash-crbug-1472607.https.html` | Native permission grant + initial about:blank Document API | Existing same-site access Promise does not force srcdoc navigation or substitute a replacement Document. |

The pinned `cookies`, `webstorage`, and `html` source search found no `set_storage_access` calls. Adding the optional testdriver override alone cannot satisfy these Core assertions. The outstanding gap is a real native third-party cookie policy with exact top-site/embedded principal grant, popup heuristic/lifetime integration, shared HTTP+DOM policy and Storage Access decisions. No URL-specific exception or denominator change is appropriate. Existing SameSite/CHIPS coverage is independent of this policy. Production default/profile behavior needs consistent implementation across these consumers before this row is source-complete.

No build, test, formatter, generator, service or WPT execution was performed for this continuation.


## Native third-party cookie profile and Storage Access integration generated

The preceding open implementation row is now superseded for source integration (execution remains open):
- `ContextOptions.third_party_cookies` selects `Block` (default) or explicit `Allow`. Both are real browser-context policies, with no WPT URL dispatch or optional testdriver override. `ThirdPartyCookiePolicy` is publicly exported.
- `ContextCookieStore::for_context` shares the serialized cookie map/disk transaction owner with the persistent profile, but creates a fresh context-local policy/grant map. Consent never leaks between two contexts sharing persisted cookies and is not written into the cookie database.
- Unpartitioned HTTP send/receive and DOM get/set use the same policy. Matching partitioned cookies remain governed by CHIPS partition keys. Credentials, Secure, HttpOnly and SameSite remain independent filters and cannot be bypassed by a grant.
- Native grants are bounded to 1,024 entries, keyed by schemeful top registrable site plus exact embedded origin, expire after 30 days under native time and are context-lifetime only. Opaque/insecure grant principals fail. Navigation lane owns the actual trusted popup-input hook and its native opener/identity regressions.
- Native `StorageAccess` control request resolves the receiver's current Window authority and native top principal; checks secure context, permission/container policy and committed CSP sandbox; existing cookie access resolves without a new grant. For blocked access, explicit native denied permission rejects, granted permission or consumed native activation permits a bounded grant. Grant mutation runs inside exact Window-authority custody. JS `hasStorageAccess` and `requestStorageAccess` both use this same native decision.
- `document_cookie_for_creator` now holds exact native authority through read/write publication, rather than only rechecking after a mutation.
- Source regressions cover first-party cookie preservation, third-party HTTP+DOM block, blocked writes, CHIPS, SameSite and credentials after grant, exact partition, expiry/capacity, shared profile without consent leakage, native permission denied/granted/existing-access, and stale generation rejection.

### Qualification profile conflict in the unchanged pinned denominator

`cookies/samesite/iframe.document.https.html:35` explicitly expects a cross-site frame's `dc_samesite_none` cookie to be set/read. `cookies/samesite/setcookie-navigation.https.html:78–80` likewise expects cross-site iframe SameSite=None creation. The tentative third-party tests above explicitly require blocking those operations by default. Neither directory supplies a `__dir__.ini` privacy preference nor calls `set_storage_access`.

These assertions cannot all pass under one unchanged cookie profile. Qualification must preserve the manifest/hash and report the profile and resulting assertions honestly; selecting behavior by fixture URL or silently changing the denominator would conceal this incompatibility. Both native profiles and the popup grant semantics are generated, but no profile-specific execution result is claimed.


## Reproducible full-run cookie profile custody

Frozen baseline metadata independently confirms both incompatible families are required Core entries with no environment requirements: `cookies/samesite/iframe.document.https.html` (source SHA-256 `58084f349f395d0d1ee71feb7e3a4c15ee5bab2de743fe006d2c8129bf5263f6`), privacy default (`0a1faa42549d82bfc163a8157b0c9f46bc09e63a7dd6db931b95e353c19f8e83`) and popup heuristic (`876cc964dd1946f710038f2cd7709cd278633c86254e77f86ad5301e47e64fc6`). Baseline coverage hash is `685884081872242895990657052f3e412faaec207f16854cd303350792c999ed`; policy hash `a655bd38bbd3cc90e49344f7ceae1db1a3bfb09c8b84419b336ebf72f81eb84f`. The baseline rows remain unresolved historical observations; no expected-pass statuses were substituted.

Generated integration:
- Hash-bound fixture JSON now requires explicit `third_party_cookies: "block"` or `"allow"`. Native `webdriver/fixture.rs` deserializes the enum and configures `ContextOptions` before creating each session. Omission, booleans, unknown policy and per-URL maps reject. Example fixture declares `block`.
- `oryn_wptrunner/cookie_profile.py` validates one immutable entire-run profile. `provenance.py` derives profile/hash from the verified fixture and publishes both in upstream `run_info`; changing fixture policy invalidates its candidate hash until a new candidate is frozen.
- `wpt_official_workflow.py` binds the profile into prepare lock and rechecks at execution. Execution records carry profile/hash, full manifest/coverage/policy/Core inclusion hashes and total, runtime/driver/source/worker-manifest identities and fixture-environment hash excluding only the cookie policy.
- `scripts/compare-wpt-cookie-profiles.py` reads two completed Core execution receipts, requires equal frozen denominator/binaries/source/worker/environment and opposite policies, and emits a comparison retaining two separate executions. It explicitly reports `single_profile_conformance: false` and never merges passing assertions across profiles. This comparison is configuration evidence, not independent validation of raw-run custody or a replacement for the existing qualification process.
- Regression sources exercise native enum/application, missing/malformed/path-map rejection, candidate invalidation and rebind, changed denominator/runtime/environment rejection, and separate-profile result preservation.

After the global generation/build gate, root must prepare **two new hash-bound fixture/candidate/run-contract sets** using the same built/signed binaries and source snapshot: one with `third_party_cookies: "block"`, one with `"allow"`; all other fixture data must match. Use the existing freeze workflow and official `prepare`/`core` operations in distinct fresh output directories with the same pinned inventory, checkout, manifest and tier policy. Existing frozen evidence is not rewritten. Feed each `execution.json` into the comparison command only after both full Core executions and normal raw-evidence qualification. Do not use per-test arguments, subsuite configuration, metadata changes or narrowed include lists to select cookie behavior.

All hooks are integrated; there is no outstanding code hook for root. Remaining work is the globally deferred build, tests, fresh freeze and actual qualification. No tools implementing those operations were executed in this lane.


## P06 shared protocol/schema reconciliation

### Concrete corrections generated

| Source / caller inventory | Confirmed issue | Final source behavior |
| --- | --- | --- |
| `oryn-common/protocol.rs`: BrowserAction Tab/Frame/Dialog; SessionAction Cookie/Storage/Headers/Proxy; MetaAction Pack/Intent/Learn | Internally tagged `action` collided with each newtype payload's `action` field. Serialization could overwrite or duplicate the discriminator and could not round-trip. | Rust payload member remains `action`; wire suboperation is `operation`. All 10 variants round-trip; ambiguous legacy forms reject. `oryn-core/translator.rs` Tabs path has a source regression. |
| ScannerProtocolResponse → `ScannerData::Value`; `oryn-r/backend.rs` screenshot, `oryn-domain` boolean probe, remote server object response; engine adapter/formatter | `flatten` cannot represent scalar/null and arbitrary object `status`/`warnings` could collide with response authority. | Typed Scan/Action keep established flat fields. Arbitrary Value serializes under `data`. Scalar, array and reserved-key object round-trip. Malformed typed values and mixed `data`/typed fields reject instead of untagged fallback. Legacy unambiguous object responses still read. |
| `worker_wire.rs` WorkerOptions | `Eq` derive included `Option<f64>`, which cannot implement Eq. | Retains PartialEq and fractional sample-rate round-trip; no float policy change. |
| AgentBootstrap → parent `agent_process::prepare` / agent_ipc Bootstrap → child admission | Missing native policy silently defaulted to empty policy. | `policy` required during deserialization. All typed producers already supply it. Native creator target/generation/authority epoch and AgentOrigin authority are demonstrated required by regression sources. Safe false/None defaults for secure-context/controller/optional metadata remain. |
| MessageHub RegisterWindow → runtime Window routing / history lifecycle | Frame capability bounded, but origin String accepted without canonical/bounded native metadata admission. | Rejects oversized/noncanonical/non-tuple origin except exact `null` before registry mutation. Native producer canonical origins remain valid. |
| MessageHub BroadcastOpen → Window/Agent BroadcastChannel subscriptions | Individual 65,536 UTF-16 names × 4,096 subscriptions could retain roughly 512 MiB independently of message payload quota. | Separate aggregate 32 MiB metadata budget includes names, partition, origin and origin key. Rejection precedes ID/secret allocation; closing a channel releases admission capacity. |

### Inspected existing boundaries retained

Worker/service/message/platform-control enums are externally tagged, so inner `request`/`result` members do not collide with an internal discriminator. Common v2/v3/v4 effects/provenance/paint enums, agent target/input/intent and JS platform request/result, IndexedDB command, WebSocket frame and Trusted Types internal tags have no direct tag-name field collisions after the shared-command repair. Root owns ContextServiceResponse's eleven `result`→wire `outcome` field fixes and its end-to-end response regressions.

Agent IPC already measures outbound serialized bytes against 32 MiB before materializing frames and rejects inbound zero/overlimit length before allocating/reading payload. Worker connection/operation ownership and structured clone port/shared-memory validation remain native admission responsibilities; no permissive authority default or schema version was added. Parent policy bootstrap no longer falls back open. Broadcast metadata is separately bounded from payload queue accounting to preserve existing queue measurement semantics.

### Collective candidate compatibility

This source batch intentionally cannot mix old and new candidate halves: the 10 action suboperations change wire key, arbitrary scanner values gain the explicit data envelope, and AgentBootstrap policy becomes required. Earlier native StorageAccess control variants/fixture cookie-profile field and root ContextService response outcome fields also require matching source-built peers. The parent must reconcile the whole candidate protocol identity/version/signature once **after all source generation**; this lane changed no protocol version or generated bindings and preserved every frozen manifest/denominator.

Regression sources were generated only. No build, test, formatter, service, serializer test execution or generator ran.

## P03/W05 clipboard and focus caller reconciliation (source only)

- ClipboardAuthorizeScript, ClipboardRead and ClipboardWrite now require Window Creator; socket page identity is independently checked. The native backend refuses unscoped read/write. JS async clipboard and native default action jobs stamp their retained receiver; writeText additionally compares document/root/generation/stream/authority across author string conversion.
- Dedicated clipboard_authority.rs checks native secure ancestry and Permissions Policy, consequential-action policy, exact live creator and active page, then holds the document and active-page guards through clipboard effects. Script-supplied origin strings are no longer a native authority input. Trusted parent automation remains a separate native entry point.
- ActivatePage now requires source Creator and accepts the retained native group token. Dedicated focus_authority.rs checks the source document, live target and native group under the same locks as active-descriptor/pointer mutation. Host focus uses the actual incumbent caller endpoint; retained remote focus sends its group token. ClosePage cleanup/provenance is owned by navigation/root, not coupled to this operation.
- Native creator lookup supports private prerender documents for legitimate loading. Clipboard and focus explicitly reject private scopes inside the locked admission. Existing generic creator consumers preserve private-scope behavior.
- Regression sources cover missing wire Creator, stale root, nonexistent frame, replacement during writeText conversion, stale group identity, private-prerender clipboard/focus denial and continued active-document access. Nothing in this section has been executed.
- Legacy LoadWorkerScript/GetReferrerPolicy/InstallMetaSecurity/InstallReferrerPolicy/AuthorizeNavigation reconciliation remains assigned to parser: runtime document mutation handlers, executable referrer lookup and prerender navigation policy are actual callers and must not be removed speculatively.
- Deferred fetch inventory includes terminal AcknowledgeDeferredFetch (page/token capability after JS caches result), bounded protected unobserved outcomes, acknowledged replay history, exact original-creator Cancel, and pre-reserved frame cleanup Release. These are cleanup authority paths that intentionally survive originating document retirement; they must not be converted into live-creator-only operations.
- Collective wire change: required clipboard/focus Creator makes old page-only requests incompatible. Candidate source/schema/worker binaries must be rebuilt together after the global generation gate; no protocol version or historical conformance evidence was rewritten.

## Ordinary keepalive acceptance closure (source only)

`PrepareOrdinaryKeepalive` validates exact native Window Creator and captures the complete immutable request, native origin/partition/CSP/COEP/referrer/top URL, retained blob resource if applicable, and the selected service-worker controller. It queues a bounded parent-owned dispatch before returning its opaque receipt. `StartOrdinaryKeepalive` joins that already accepted operation exactly once; it cannot replace request fields or become a second network dispatch. `ReleaseOrdinaryKeepalive` abandons response delivery while accepted dispatch continues, including when the originating process never sends Start.

The parent service uses an initialized weak self reference (`into_shared`) at direct, standalone socket, and sandbox route creation/reattachment sites. Only an active dispatch holds the strong service reference. Retained response jobs hold the native lifecycle, not the service. Receipt metadata is bounded (1024 entries, 32 MiB including retained blobs, 60-second response retention), and bounded IO/cleanup admission precedes acceptance. Failed orphan handoff retains the exact stream and creator in a pre-reserved retry ticket. It never refetches.

The host freezes before its completion-wait task launches. Retirement skips automatic cancellation of accepted keepalive starts; explicit cancellation remains operative. HTTP redirect and cookie/referrer policy reads use the captured source. Data/blob remain supported, with blob lookup performed before revocation/retirement. Loading owns network quota, per-hop cleanup, retained service-controller event leases and orphan body draining; the shared 64 KiB charge is taken at actual network dispatch, not receipt creation.

Regression sources cover paused prelaunch followed by document replacement; exact request/body/top-site preservation; page mismatch and replay denial; receipt release before dispatch; transport teardown without Start; local blob revocation/data responses; and malformed/nonkeepalive/oversized preparation. These sources have not been run. Fresh candidate protocol/runtime/worker binaries must be generated together after global source completion.

## Blank-target prerender transfer source reconciliation

Generated hidden final-destination admission for `_blank` candidates. The native
PageId, storage/worker/message principal and related-actor route are allocated
before execution. `Reservation` retains the original source cleanup capability
while the candidate uses the hidden destination endpoint for construction,
resource loading and readiness/loaded acknowledgements. Parser roster selection
separates self and blank targets and transfers an accepted job out of source
cancellation ownership. Loading's explicit `NavigationPolicySource::Prerender`
binds requests to the retained destination scope; no general author authority
check is disabled.

`prerender_blank_actor.rs` owns the handoff continuation: retain pending roster
load; validate exact destination/token/generation/epoch; await native activation
ACK; publish the existing root; reserve/install the existing executable in the
related actor; await public-page publication ACK; resume retained activation
events. The existing parser driver, pending response capture, native globals,
history and resource continuations move intact. There is no replacement load.
Native activation/publication ACK replay and initial-document parking custody
are in the DOM lane's dedicated blank activation ledger. The actor retains its
reservation until publication and event completion, with delayed infrastructure
retry and terminal detached-root retirement.

Hidden pages are omitted from descriptors, named targets, ordinary focus and
cross-page relationships; nonterminal group navigation/load/start events are
suppressed. Root owns close/crash diagnostics suppression and physical teardown.
Private self-authority remains usable for candidate loading. Noopener activation
publishes no opener. DOM producer integration selects only final noopener blank
GET URLs; form body construction retains `SubmissionTarget::New`, so final query
serialization precedes candidate selection and POST follows normal navigation.

Regression sources: `blank_prerender_host_tests` checks distinct destination,
original globals/document identity, unchanged source lifecycle, repeated native
and local publication ACKs, exactly-once activation events and harmless cleanup
after promotion. `blank_transfer_tests` moves an existing driver and response
capture, checks one broker request, and injects pauses after promotion and after
actor installation. Loading and DOM lanes own destination-policy and native
ACK-loss/parking regression sources respectively. All are generated source;
none has been built or executed and no WPT result is claimed.

## Remaining P/W row reconciliation and page cleanup correction

[Per-row disposition](PW-REMAINING-ROW-SOURCE-DISPOSITION-2026-09-18.md) now
separates every open granular P/W source dependency from actual execution-only
qualification. The operation inventory names every current top-level request
(including final blank activation ACK); this is source coverage, not an assertion
that every nested platform/runtime path has executed.

Confirmed `retire_page_agents` correction preserves deferred flush errors while
continuing independent cleanup, and contains agent-detach errors so Window IDB
and socket cleanup still run. Regression source injects deferred registry failure
and verifies selective transaction teardown plus retained failure on retry.

Blank producer follow-through now reserves queue capacity before OpenPage claim.
The actor sends final AcknowledgeBlankPrerender after publication/event completion
so original source-route replay custody can retire without waiting for the new
page to close. Native receipt/retired-route side is owned by the DOM lane.
