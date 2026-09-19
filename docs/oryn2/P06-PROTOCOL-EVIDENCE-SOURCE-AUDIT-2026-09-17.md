# P-06 protocol evidence source audit

Status: source generation and static caller review only. No builds, validators, tests, suites, signing or evidence generation executed. Historical artifacts were not rewritten.

## Captured identities

Current candidate source now requires three independent native constants: worker 17, context service 50 and agent IPC 19, after the September 18 coordinator-approved collective source cutover. `RuntimeBuildInfo` exposes all three; `native_protocol_versions.runtime_matches` rejects missing, mistyped and mismatched fields. Context/agent absence in historical serialized records stays absence. Later paragraphs referring to the unbuilt 16/49/18 batch are historical generation notes; see the final addendum in `P06-PENDING-CONTRACT-BATCH-2026-09-18.md`. No new runtime artifact or signature was generated.

## Integrated consumers

| Consumer | Generated enforcement |
| --- | --- |
| `build_security_result.py`, `build_layout_result.py`, `build_app_compatibility_result.py` | Validate raw runtime identities before building evidence; copy all three captured fields into compact worker fingerprint |
| `validate_security_result.py`, `validate_layout_result.py`, `validate_app_compatibility_result.py` | Validate current fingerprint even without raw artifacts; raw validation also requires runtime/fingerprint agreement |
| `evidence_protocols.py` | Shared strict current validator and explicit historical handling; no substitution of current constants into historical records |
| Security v3/layout v2/app v1 schemas | Admit captured context/agent fields; remove obsolete worker version constants; current validation requires fields, historical mode can retain absence |
| `emit_suite_evidence_ledgers.py` | Stable historical control ID now evaluates all three actual runtime identities |
| `run-oil-integrated-outcomes.py` | Candidate manifest must match all three versions and source hash |
| Six OIL forms/framework/saas/shopping/booking/research runners | Require runtime and signed worker manifest identity agreement before attempts |
| `run-upstream-wpt.py` | Official upstream profile requires current captured versions; explicit legacy diagnostic profile retains historical identity semantics |
| Eight shell consumers | Build/sign worker; run security/layout/app/execution-replay suites; benchmark worker/layout/app: checks all three runtime identities |
| Native `WorkerManifest` and `verify_worker` | Require typed context/agent fields and current versions, including before verification cache lookup; no production historical bypass |
| Worker Ready startup and attached-environment handshakes | Reject unavailable/stale context/agent build metadata alongside existing worker envelope version checks, including Linux process admission |

The build/sign script validates compiled context/agent metadata before creating a manifest, records source-derived worker/context/agent values, then verifies the signed parent/worker runtime reports all three. The standalone pre-sign worker build metadata has no parent handshake worker-version field; the signed runtime admission verifies that field after the handshake.

Historical validation is explicitly selected through `--allow-historical` or the existing WPT legacy profile. Missing/null identities are unavailable, never inferred. When raw and compact artifacts both claim identities, they must agree. Schemas permit historical omission; current validators do not. App schema retains historical contract 3 alongside current contract 4; raw current validation requires 4.

## Regression sources

- Shared evidence tests: missing/null/string/boolean/stale current fields, compact-only checks, historical absence preservation, mismatched raw/compact identities, upstream rejection of legacy absence.
- Integrated candidate tests: context/agent omissions, wrong values/types and worker source bump.
- Native manifest source: required field deserialization, null/string/boolean rejection, stale per-protocol rejection and current acceptance, alongside cached verification fixture.
- Root-owned source-constant tests cover all three identities and distinct version mapping.

## Remaining qualification

Build and execute generated tests, validate a newly built signed candidate, regenerate suite evidence only from actual runs, then qualify complete WPT and five-family outcomes. This source audit does not mark P-06 runtime/signing evidence or the overall Core package complete.
# Additional unbuilt extraction wire shape

The S-05 metadata implementation adds `ContentKind::Meta` to the existing typed worker Extract request. Runtime and OIL callers use the shared enum; no separate dispatch discriminant is copied. Worker/context/agent constants remain in the same unbuilt generation batch pending final whole-tree protocol reconciliation. Native metadata regression sources are generated, not executed.

## Additional unbuilt accessibility wire shapes

The V-05 native `TextSnapshot` adds optional `control: NodeId` and exact `objects: Vec<TextObject>` positions. Missing historical fields decode as `None` and an empty vector; they do not infer control identity or frame insertion positions. Native boundary journals append `ControlValueReplace` and `ControlValueReset`; ordinary DOM Range rebasing ignores these control-only records. Password edits use reset records without value lengths. These changes remain in the combined unbuilt worker16/context49/agentIPC18 batch for final protocol reconciliation.

Opaque platform marker payloads explicitly use `ORYNAX03`. Version 2 is rejected, not reinterpreted. The control discriminator requires a native boundary identity; protected text never creates a marker. New native regression sources cover absent control metadata, old headers, live edits and frame epoch replacement. No execution qualification is claimed.

### Native counted-item wait predicate

The current unbuilt generation batch adds `WaitPredicate::SelectorCount { selector, minimum }`, carried by existing `WaitFor`/`WaitForUntil` worker messages. OIL validates its count and forwards explicit timeout values; runtime predicate matching is exhaustive and uses the native scoped selector resolver. Generated OIL regression sources cover threshold and zero counts, invalid selectors, detached nodes and asynchronous insertion. Protocol constants remain in the same pending batch; build/fixture/signature qualification has not run.

### Native expression wait predicate

The same pending batch adds `WaitPredicate::JavaScript(String)` through `WaitFor`/`WaitForUntil`. Both public PageHandle entry points apply the existing raw-inspection policy audit used by evaluate before dispatch. The runtime uses its existing native evaluation/checkpoint path with boolean operators, propagates syntax/runtime errors and checks expiration before subsequent evaluations. Regression sources cover native truthiness, an author-replaced Boolean global, errors, zero-timeout single evaluation and timer completion. No tests/builds ran.

### Action execution failure identity

The pending batch adds `RuntimeError::ActionExecutionFailed { action_id, cause: Box<RuntimeError> }` to native worker error serialization. Native realm action dispatch reserves a checked ID before entry; all subsequent execution/publication/navigation errors carry that ID and indicate possible effects. Pre-dispatch stale-reference errors retain their existing safe retry classification. Generated regression source injects semantic publication failure after a real click, checks a single side effect and error wire round trip, then verifies later ID progression and pre-dispatch exhaustion rejection. No execution.

### Partial action-batch failure receipts

The pending batch adds typed `RuntimeError::BatchExecutionFailed { completed: Vec<ActionResult>, failed_index, cause }`. Local, worker and selected-frame paths stop at the failed action and preserve the successful prefix; the parent updates its known revision from the last completed receipt. The batch length limit is also enforced before local execution. RuntimeError retains PartialEq but no longer derives Eq, because full ActionResult observations carry types that provide PartialEq. Generated regression `failed_action_batch_retains_completed_receipts_and_stops_suffix` covers prefix receipt preservation, stopped suffix, nested error wire round trip and later ID progression. No execution.

### Focused form resolution

Pending batch adds `WorkerRequest::ResolveFocusedForm`, returning the existing SemanticRef response. Local actor and selected-frame dispatch use native focus and form-owner state; the selected-frame request is classified read-only, and the worker diagnostic label contains no page data. OIL targetless submit now resolves this identity then calls the existing policy-checked action path. No new submission authority bypass is introduced. Generated regression covers two forms, external form-associated controls, no focused control and generic focusable elements. No execution.

Nested action/batch failures retain the typed underlying error. Parent page lifecycle classification and worker wall-time diagnostics unwrap these outcome envelopes while preserving action IDs, completed receipts and failed index. Generated `nested_action_deadline_preserves_batch_receipts_and_terminal_cause` covers serialization and wall-time/RSS/crash/protocol causes; not executed.

V05 manual pseudo text separation adds serde-default `GeneratedContent.separates_text` at the end of the record. Native layout sets it from pseudo computed display; semantic ingestion adds speech run boundaries while preserving authored text/alternative payloads. Historical JSON omission is false (legacy joining), not invented authority; binary transport remains gated by the combined protocol candidate. All source literals reconciled. Root owns final version reconciliation; no execution performed.

### OIL partial script errors (source generation)

`RuntimeError::OilExecutionFailed` is appended to the existing error enum. It retains a zero-based failed command index, completed OIL output records as individual JSON strings (avoiding untyped deserialize-any requirements in native codecs), and a boxed typed native cause. Worker error dispatch and nested deadline classification preserve the envelope. Local `NativeOilError` retains typed output records. CLI exposes prefix receipts and MCP supplies structured native errors. This changes the pending error schema; constants stay worker16/context49/agentIPC18 for root's final combined protocol reconciliation. Generated round-trip/deadline regression sources are unexecuted.

Pending context response addition: `ScriptLoadCancellationPending` distinguishes cancellation admission from the actual drop of a claimed script/resource lease. The cleanup executor retains ownership and retries pending responses; `ScriptLoadCancelled` now confirms token release. Generated registry/service regression sources cover retained shutdown scope and terminal acknowledgement. This remains part of the unbuilt combined protocol batch; no execution.

`ScriptCommandFailed { code, message }` is also appended to RuntimeError in this unbuilt batch: native OIL frame selection retains actual WebDriver algorithm codes without converting worker transport errors into text. The external WebDriver API projects this variant back to its existing error schema. The full-contract native executor additionally writes `native_error` alongside existing textual error evidence and retained outputs; this is additive evidence, not a changed success oracle.


Pending L03/L05 private response backing: `ContextServiceResponse::HttpStreamStarted` and `PlatformHostResult::HttpStarted` now carry defaulted `internal_response: Option<InternalResponseMetadata>` (raw status/headers/URL/redirect/range-request provenance). Public status/headers/URL remain filtered; the existing stream carries private bytes. Host mapping and native agent producers feed the binding private record. `CacheResponse` persists the optional private metadata; `AgentFetchResponse.range_requested` and `redirected` retain range and redirect provenance on native service replay. All known producer/literal/consumer sites were source-reconciled. Constants remain worker16/context49/agentIPC18 pending root's combined batch reconciliation; historical missing fields are None/false and do not invent private response authority. No execution.

### Queued navigation execution receipts (source-generated; qualification pending)

`ContextServiceRequest::SettleNavigationExecution { page_id, receipt, outcome }` and `ContextServiceResponse::NavigationExecutionSettled` append typed terminal publication. The service validates socket page ownership and the target page's opaque receipt. `NavigateScriptWindow`/`NavigateSelectedDocumentWindow` carry an optional native execution token; parent callers mint it from the queued ticket before dispatch. Successful admission no longer finishes that ticket. Native execution propagation and retained acknowledgement delivery are source-generated, as detailed below; integrated execution remains pending.

The parent receipt registry retains active custody without an Arc cycle. Exact replay is idempotent while its bounded tombstone is retained; conflicting, foreign and unknown receipts fail. Full-result SHA-256 fingerprints preserve replay identity even when bounded diagnostic text is truncated. Retirement publishes a failed execution result for outstanding receipts. Snapshot outcomes add `execution` to distinguish committed generations, completed no-op operations, cancellation and failure while preserving legacy result fields and pruning/truncation markers. Generated registry regressions cover ownership through dispatch, retirement, cross-owner rejection and truncated conflicting replay. This is unbuilt source; constants remain worker16/context49/agentIPC18 for final combined reconciliation.

`navigation_execution_settlement_requires_exact_page_and_receipt` adds parent-service regression source for a valid receipt on the wrong page, an unknown receipt on the correct page, native-only JSON request round trip, duplicate acknowledgement and conflicting terminal rejection. It checks that rejected publications leave active custody untouched and valid duplicate publication produces exactly one terminal outcome. Execution-side observer/outbox integration is source-generated as recorded below; no test has run.

The pending batch also adds `ClaimNavigationExecution { page_id, receipt }` with `NavigationExecutionClaimed` and `NavigationExecutionClaimRejected`. Receiver claim and parent dispatch-failure settlement share the custody lock: a successful claim gives the receiver exclusive terminal publication ownership; an earlier parent failure causes subsequent claim rejection before effects. Repeated live claims are idempotent, and terminal rejection lets a never-admitted publisher release its slot. Settlement rejects unclaimed receipts. `dispatch_failure_and_receiver_claim_have_one_terminal_owner` covers both orderings; transferred receipt capacity and replay-history pruning also have generated regression sources. Constants remain unchanged pending the combined protocol pass.

Claim now rechecks target supersession before transferring execution ownership. Unclaimed superseded receipts publish Canceled and reject effects; already claimed receivers keep their terminal custody. `supersession_before_claim_rejects_effects_but_claimed_execution_retains_ownership` covers both cases and preservation of the newer target.

`NavigationExecutionRetired` is a distinct disposal acknowledgement for an exact receipt settled by parent retirement. It never overwrites the parent's failed outcome with a sender success. Retirement provenance is stored explicitly in bounded tombstones; unknown receipts and conflicting ordinary completions still reject. Parent service shutdown retires receipt custody, rejects subsequent claims and continues serving terminal settlement cleanup after its normal admission gate closes. The parent-service regression now exercises this shutdown path. All source and regression changes remain unexecuted.

Receiver receipt integration is now source-generated: optional opaque tokens on
both remote navigation FrameCommand variants become native-only observers and
are never decoded from author JSON. The observer stays outside serialization;
only `NavigationExecutionOutcome` crosses SettleNavigationExecution. Root/frame
completion waits native parser/load publication and exact stream custody.
Publication retries retain the identical token/outcome; ClaimRejected and the
explicit retirement acknowledgement dispose custody without accepting an
unexecuted result. Qualification consumes `Outcome.stage` and `execution` and
checks consistency with the retained legacy `result`. Tests are generated only;
worker16/context49/agentIPC18 remain for root's final combined bump assessment.

SWR ownership is process-private: lexical BackgroundOwner spans and Arc native leases add no wire fields. New background requests use a distinct IdentityBroker namespace while retaining the originating document scopes until terminal/cancellation ACK. The pending private response wire additions above remain in the same unbuilt generation batch.

G-02.c window-position integration appends platform `Request::WindowPosition`
and `Response::WindowPosition { x:i32, y:i32 }`. Native platform state supplies
these only after the existing creator/owner authorization; setters remain
native BrowserContext methods under live-page retirement admission. The Window
move event and coordinate accessors use the same state as WebDriver rectangles.
Window/worker platform transports retain typed propagation. Matching native,
HTTP and callback-retry regression sources are generated. Constants remain
worker16/context49/agentIPC18 for the final combined protocol assessment.


#### Pending G-02.g.4 media transport extension (source batch)

`PlatformHostRequest::MediaStart` and `ContextServiceRequest::MediaStreamStart`
carry the native bodyless media request, CORS state and validated audio/video
destination. Window host admission/cancellation and parent media policy/SW routing
consume them; worker routes reject DOM media production. They share the existing
HttpStarted/HttpRead/HttpAbort response and retained cancellation contracts.
These additions belong to the same unbuilt protocol batch (worker16/context49/
agentIPC18); final protocol reconciliation/signing remains required. No historical
artifact was rewritten and no protocol compatibility qualification is claimed.

## Frame resource ownership source integration

The hierarchy in `resource_job_scope::Registry`, realm endpoint ancestry, scoped loader client selection, and prepared broker/client transfer into frame commit are process-private. They add no serialized fields or envelope variants. Source inspection rechecked worker **16** (`worker.rs`), context service **49** (`context_service.rs`), and agent IPC **18** (`agent_ipc.rs`); no version was bumped during this review. The pending media, window-position, receipt and earlier extraction/response wire changes still require one final combined protocol assessment after all generation. No compiled candidate or compatibility result is implied.

## Image preload and privileged media Range source additions

The pending batch also adds `ImagePreloadStart` / `ImagePreloadStreamStart` and the defaulted private `privileged_range` field in FetchOptions/AgentFetchRequest. Native media sets that provenance; Request clone/reset and no-CORS header mutation preserve or remove it through native records. Parent and agent consumers only preserve a validated Range header for GET. Image preload responses retain private URL/type/redirect metadata through existing stream envelopes. The bounded parent preload cache is process-private. Versions remain worker16/context49/agentIPC18 pending final combined assessment; no binary was built or historical evidence altered.

### Native timing visibility metadata

`HttpStreamStarted` and `PlatformHostResult::HttpStarted` now carry defaulted optional `ResourceTimingMetadata` outside authored response headers: all-hop TAO decision, raw status, header-arrival offset, encoded Content-Length when available, and content type. Parent and agent admission produce it; host forwards it; native Performance filtering consumes it independently of opaque Response filtering. Unknown transport phases remain absent/zero rather than inferred. External resource timing queues are process-private and document-owned. These fields remain part of the pending combined protocol batch; no execution occurred.

### Complete measured timing propagation batch

Additional optional fields now carry measured `TransportTiming` in NetworkResponse/NetworkStreamResponse/NetworkEvent::Headers and encoded broker headers, and detailed `ResourceTimingMetadata` in ScriptSource/PageResource/PageBinaryResource, AgentFetchResponse and CacheResponse. Response clone/cache/respondWith retain native TAO-hop provenance; replay strips old phase timestamps. DNS timing seeds on HttpRequest/CookieRequestContext are serde-skipped, process-private. Pinned reqwest 0.12.28 has a local connector instrumentation patch; no lock regeneration occurred. The source audit is `RESOURCE-TIMING-SOURCE-AUDIT-2026-09-17.md`. All fields remain in the same pending protocol generation batch; root owns final version/manifest reconciliation.


## Remote Window dynamic projection source batch

`ReadRelatedWindow` now carries a native local page/frame inventory to prevent synchronous self-queries; `RelatedWindowSnapshot` adds explicit `same_origin_domain` and ordered `(name, WindowTarget)` children. Parent validates target creator, child native identity/ancestry, bounded inventory and post-query current authority. The parent owns published native tree inventories; reads never synchronously query another actor. `PublishNativeWindowInventory` / `NativeWindowInventoryPublished` publish bounded exact-root/frame snapshots at native frame registration, task checkpoints and outgoing Window/port/Broadcast messages. Locally owned actors also supply the current inventory on reads, including negative entries. These are typed protocol fields, not author page-ID/name authority. Historical missing fields are not synthesized into native identities. Current worker16/context49/agentIPC18 constants are preserved for coordinator final combined version reconciliation. No protocol/build execution.

## URL media generation wire additions (pending combined reconciliation)

Media Request appends UrlOpen (bounded base64 admitted bytes), UrlSample (private asset identity, time/pause/volume/seek), UrlRelease and UrlCanPlay. Response appends UrlAsset/UrlFrame/UrlSupport. Existing capture discriminants retain ordering. LayoutSelectorState adds serde-defaulted native media_state bits. Worker16/context49/agentIPC18 constants remain unchanged pending root's combined final reconciliation. Native Symphonia is pinned to 0.5.5 with cached FLAC/MP3/PCM/Vorbis/Ogg/WAV features; Cargo.lock generation and all execution remain deferred.

Resource lane supplemental pending batch: media capture Request appends
`UrlPictureInPicture { id, enabled }` and `UrlPictureInPictureState { id }`;
Response appends `UrlPictureInPicture(Option<(u32,u32)>)`. Producers are native PiP
DOM session management; parent consumers authenticate the current window owner,
permissions policy, activation and decoded asset ownership. Common TopLayerKind
appends Fullscreen, and LayoutSelectorState includes defaulted fullscreen/media
state metadata. Existing worker16/context49/agentIPC18 values remain unchanged
pending the root-owned final combined wire-version reconciliation. No historical
evidence has been rewritten and no wire/runtime checks have been executed.

Resource dependency follow-on: PlatformControl Request appends
`PermissionsPolicySnapshot`; Response appends
`PermissionsPolicySnapshot(oryn_js::permissions_policy::Snapshot)`. The private
snapshot contains the strict document origin, a container-inspection discriminator,
and bounded header/container/inherited-admission constraints. Parent producer
rechecks current native owner authority; standalone producer uses native stored
response policies and live embedding DOM declarations. No strict origin, storage,
message or navigation authority field is replaced by this inspection record.
Final combined protocol versions and qualification remain root-owned and deferred.

## Native permission choice replay (unbuilt)

The pending platform request enum appends `PromptResolve { id, choice, receipt }`. Retained pointer popup input allocates one checked process-native receipt and reuses it after transport failure. Parent state stores up to 1,024 exact owner/receipt/prompt/choice result tombstones, rejects conflicts and foreign or pruned consumed prompts, and clears records on owner/page retirement. Replay returns the original acknowledgement without reapplying grants or registrations; deliberate repeated legacy `PromptAnswer` still fails. Runtime resolves replay before activation/policy preflight because that path performs no permission mutation. New source regressions cover changed grants, wrong choice/owner/prompt, dismissal, precommit failure, pruning and retirement. Worker16/context49/agentIPC18 are unchanged pending the parent’s final combined version assessment. No execution.


Native media qualification now joins all three compiled protocol constants from
the frozen `native_media_tests` executable. The test output separates compiled
protocol identities from `runtime_build_info()`'s unattested worker state; it does
not replace a missing live worker attestation with an inferred identity.
`native_media_qualification.py` checks exact source constants and compiled native
media source/Cargo lock hashes, pinned decoder fixtures and fixed original-run
command. Current completion candidates require the new artifact and native-media
receipt. Historical evidence is unchanged. No protocol field/constant change or
execution was introduced by this qualification addition.

## Fetch Metadata request provenance (source generated, unbuilt)

`CookieRequestContext` now carries defaulted native `fetch_metadata` (mode,
destination, opaque initiator, frozen navigation activation and monotonic redirect
site taint). Both PageNetworkBroker context reconstructions preserve it while
replacing untrusted origin/ancestry with native authority. `FormNavigationSource`
adds defaulted `user_activation`, included inside native signed form/script source
payloads. Document provenance capture itself does not freeze activation: accepted
navigation captures the then-current exact owner's activation; later grant,
consumption or retirement cannot change the sealed source. Standalone frame
capture freezes its local native binding activation before resource dispatch.

HTTP brokers regenerate headers before cache/transport; native WebSocket
handshakes generate equivalent metadata without authored header inputs. Producers
include script/module/style/font/image/media/frame, agent entry/import/fetch,
legacy runtime/fetch and report delivery. These request/source shapes join the
pending combined protocol bump; worker16/context49/agentIPC18 remain unchanged in
this source batch. Final wire version/lock reconciliation and execution are still
required. Existing historical evidence is not rewritten.

Native DOM snapshots additionally retain private video poster bytes, SVG/raster
kind, intrinsic dimensions and showing-poster state alongside existing media
frame pixels. Authored clone/import constructs fresh state. The runtime consumes
these fields for layout and paint; they are not HTML attributes. This native
snapshot shape must join final combined protocol/schema reconciliation.

## AudioWorklet generation in progress

Appended `AgentKind::AudioWorklet`, `WorkletCommand` (module load/processor
construction), `AgentCommand::Worklet`, exact-owner `AgentServiceRequest::Worklet`,
and `AgentEvent::WorkletFinished`. WorkerOptions additionally carries a defaulted
native audio sample rate, admitted only against the worklet's native context
configuration. Processor construction reuses signed client authority and existing
MessageHub transferred-value ownership/rollback. Public callers and independent
agent/global/module paths are being generated; this inventory is not a source
closure or execution claim. Final combined worker/context/agent protocol version
and schema reconciliation remains mandatory and is owned by root.

AudioWorklet continuation adds `ImportWorkletModule` with explicit credentials
and importing URL/referrer policy, and bounded `WorkletDefinitions(Vec<String>)`
metadata before readiness. The private worker/worklet prepared module link now
carries the shared original parse-error receipt before graph compile/link. These
are part of the same unbuilt protocol generation batch; final versions, generated
bindings, build identity and signed worker manifest reconciliation remain required.


### Pending history scroll snapshot shape — September 17

`NavigationHistorySnapshot` adds `scroll: HistoryScrollState { mode: HistoryScrollMode, position: Option<(f64,f64)> }`. Modes serialize as `auto`/`manual`; the missing whole field defaults to auto with an unknown (`None`) position, never a fabricated saved coordinate. Native history/session/frame-descriptor admission rejects non-finite coordinates. The shape propagates through serialized native history initialization, frame descriptors and all requests/responses carrying history snapshots. Existing fixed-size snapshot accounting includes the new field via `size_of`; owned entry copies retain the data. JSON historical default behavior is covered by source regression; it does not claim binary compatibility. Worker16/context49/agentIPC18 remain the pending combined-generation constants; root owns final schema/version reconciliation before build/sign/qualification.


### Pending combined batch: HTML encoding replay

`ContextServiceRequest::ParserEncodingPolicy { page_id, creator, token, action }` carries typed Prepare/Restore/Release. Prepare allocates its known opaque token and cleanup ticket before IPC; Release is admitted through retained navigation cleanup for retired owners. Restore validates current exact ownership and is single-application/idempotent ACK. `PageResource.raw_body_base64: Option<String>` (serde default) preserves buffered frame response bytes into the native decoder/replay owner; native streamed frames keep their existing stream route. Response queues account the retained encoded payload and the input owner enforces the decoded raw-byte bound. The final combined protocol/manifest/generated-source identity reconciliation remains required and root-owned; no version was independently bumped for these additions.

### Joint history native metadata — September 17

Native `oryn:joint-history` resource metadata carries the bounded Timeline with routes, steps, cursor and opaque pending completion correlation. `SavedFrameNavigation` adds serde-default `joint_route: Option<String>`; absent older metadata does not invent route identity. Pending route completion distinguishes entry commitment from cross-document load completion. The weak process registry correlates receipts across native isolate replacement, grants no Document/resource authority, and holds no page strongly. Native initial-frame owner claims remain mandatory before restoration. Worker16/context49/agentIPC18 remain the current shared unbuilt batch; final protocol reconciliation remains open.

SavedFrameNavigation additionally carries serde-default `document_controls: Vec<SavedFrameControls>` for archived network-document controls. Each descriptor must reference a known document-state id, cannot overlap inline-source control custody, and counts toward aggregate bytes/control caps. Missing old metadata means no retained network controls, not fabricated state. All native producers/restore consumers and source regressions were updated; execution remains deferred.

Joint Timeline pending route metadata now adds serde-default `interactive: false`; historical snapshots cannot invent a delivered DOMContentLoaded milestone. Native completion receipt milestones are process-local and rebind only through exact opaque receipt identity. Combined protocol versions remain unchanged pending root reconciliation.

SavedFrameNavigation adds `saved_names: Vec<(document_id, name)>`, serde-default empty meaning unknown. Decoder rejects unknown/duplicate document identities, more than 4096 records, names above64KiB and aggregate snapshot budget overflow. Private history metadata only; no name data grants native authority. Root final protocol reconciliation retains ownership.


### Pending combined batch: inherited frame charset provenance

`ContextServiceResponse::PageResource` adds serde-default `inherited_encoding: Option<String>`. Absence means no eligible inherited fallback, not the selected default charset. Parent/standalone producers authenticate embedding-origin eligibility; response accounting and LoadedFrameResource propagate the field to buffered/streamed native decoders. Existing selected `encoding` remains metadata. Historical absence does not invent inheritance. Final combined protocol/source/manifest identity reconciliation remains root-owned; no independent version bump or execution occurred.


### Pending binding/layout source identities: document URL reflection

IDL adds HTMLObjectElement.data, HTMLInputElement.src, anchor/area ping and HTMLEmbedElement/HTMLTrackElement source-reflection interfaces. Native prototype routing/getter/setter implementation and regression sources are present; the final shared binding generator/ABI/source identity reconciliation remains required. `LayoutInput.legacy_background_urls` is serde-default native resolved presentational-hint metadata, separate from raw selector attributes. Historical absence means no precomputed hint map. No generator or protocol bump ran in this source batch.


### Pending combined batch: automatic refresh policy

`FrameWorkerPolicyBound` adds serde-default `automatic_features_allowed: bool`, false for absent historical evidence. Parent derives it from the authenticated bound worker policy's CSP sandbox, host conjoins inherited/iframe automatic-feature permission, and standalone/root response policy constrains the same native bit. CSP script-src alone does not disable automatic refresh. Original response headers drive native Refresh parsing; legacy PageResource.refresh_url is no longer produced as an immediate redirect. Final combined versions/manifests/generated identities remain root-owned and unexecuted.

### Hyperlink audit admission transport (generation, unrun)

`AcceptNavigationExecution` and `NavigationExecutionAccepted` distinguish actual native destination admission from queued dispatch and terminal settlement. The exact claimed receipt admits once; repeats acknowledge without another side effect. Off-actor publication retains acceptance ahead of terminal settlement across uncertain acknowledgements. This is part of the pending combined context/agent protocol reconciliation; no constants or historical evidence were rewritten. `PrepareHyperlinkAudit`, `FinishHyperlinkAudit`, prepared/finished acknowledgements and defaulted `OpenPage.hyperlink_audit` now carry the frozen one-shot producer. The author JSON bridge clears the native field. Final combined wire identity validation remains deferred until all generation is complete.

R-04 initial cleanup adds the appended `ContextServiceResponse::InitialDocumentRestorationObsolete` variant. It acknowledges disposal of an exact initial restore completion whose original generation/epoch is no longer current; the child releases cleanup custody without applying the old runtime snapshot. `InitialDocumentRestored` continues to require matching current authority. No version constants were changed in this source batch; final combined reconciliation remains coordinator-owned.

- Cached history completion expiry: appended `MessageResponse::CachedCompletionExpired` distinguishes disposed/pruned native capability custody from successful restore. `ParkedDocumentToken` keeps its opaque 32-byte wire shape, now authenticated with exact page ID, checked native issuance sequence and a full 128-bit AES tag under a separate per-hub random key. Old-process/random/foreign capabilities reject. Combined batch protocol constants remain unchanged pending final parent reconciliation.


Prerender source batch adds native-only ContextService ReservePrerender/CommitPrerenderResponse/CancelPrerender requests and PrerenderReserved/PrerenderResponseCommitted/PrerenderCancelled responses. Reservation transfers an opaque authenticated capability plus generation/epoch; dropped response custody cancels exact native scope. No authored compatibility dispatcher admits these routes. AuthorizeInlineScript adds serde-default `speculation_rules:false`; only native rule preparation selects true. The semantic discriminator does not grant executable script authority. Constants remain worker16/context49/agentIPC18 pending final combined schema reconciliation. Native candidate actor loading/activation remain open and this schema alone is not a capability-completion claim.

### Pending embedded-content generation shape

Removed superseded native ObjectImageStart/ObjectImageStreamStart variants; object/embed now use the existing page-resource destination discriminator. LayoutNode adds serde-default document_replaced to distinguish native child Document replacement from intrinsic raster size. Include these enum/layout changes in the final combined version, producer/consumer and literal reconciliation; no historical evidence was rewritten and no generator/build was run.

LayoutNode also carries serde-default optional ImageIntrinsics (native finite bit-encoded optional width/height/ratio). Reconcile this additional layout payload field in the same final schema/version batch.

Prerender source batch additionally adds native ReadPrerenderAuthority and MarkPrerenderLoaded requests, with PrerenderAuthority/PrerenderLoaded typed responses. Loaded ACK follows native parser/load and retained pageshow tree completion, retains exact reservation custody and never means activation. These join the pending combined protocol reconciliation; constants are unchanged.

Native ActivatePrerender additionally carries exact candidate lifetime/capability and already-parked predecessor token. Typed PrerenderActivated versus PrerenderActivationObsolete distinguishes current completion from historical/expired disposal. Parent completion records retain exact replay provenance, stay bounded per page, and never repeat group/authority publication. Actor activation caller integration remains open.

Pending batch: LayoutNode adds serde-default image_document_root (native hosted SVG identity); WebVTT adds source IDL interfaces and track destination policy handling. Existing protocol constants are intentionally unchanged until the root-owned combined wire/IDL/schema reconciliation.

Pending source IDL batch also adds AudioTrack/VideoTrack/List interfaces and HTMLMediaElement audioTracks/videoTracks; TrackEvent union includes all native track brands. Native platform record variants and GC graph hooks are in-process only; no separate protocol bump was made.


Prerender private descendants add native-only `RegisterPrerenderFrame { page_id, parent: Creator, frame, generation, epoch, origin, inherit_origin }` and `PrerenderFrameRegistered`. The parent creator is resolved against the exact private tree; this request cannot register a private child in the visible page frame table. Frame response policy tickets now carry the actual initiating private root epoch. Constants remain worker16/context49/agentIPC18 pending combined protocol reconciliation. Sources only; no execution.

Native prerender readiness now adds `MarkPrerenderReady { page_id, token, generation, epoch }` / `PrerenderReady`, separate from `MarkPrerenderLoaded`. Only the native actor sends it after candidate Document construction with committed response policy; socket page, exact candidate lifetime and original source authority are checked before ready publication. Promotion accepts the created Document with its retained parser/body, without claiming load completion. Replayed readiness is idempotent only for the live exact candidate; foreign/stale/retired authority fails. Authored messages do not expose this variant. Combined source batch constants remain worker16/context49/agentIPC18 pending root's final reconciliation.

## Navigation timing native metadata (generation batch)

`ResourceTimingMetadata` now carries optional serde-default `producer_clock_micros`, an authenticated same-host monotonic anchor used to align parent response timing with a child Document admission clock. Historical absent metadata remains unavailable; decoding does not synthesize timing or native authority. The anchor is never exposed in author JSON/getters. Platform clock failure and future/invalid anchors produce no aligned timing. This extends the current combined wire batch; worker16/context49/agentIPC18 constants remain reserved for the root's final schema reconciliation. No handshake/build execution is claimed.

The same metadata now also has optional `navigation_phases`, containing final fetch start and an optional complete redirect interval in microseconds relative to the producer anchor. Native validation rejects partial/inverted intervals or final fetch after response start. Parent Document resource loading and standalone frame loading measure their own chain; cached timing provenance does not copy old phase clocks. The public navigation entry applies destination-origin/TAO privacy separately. Other callers explicitly leave these optional navigation phases absent when unmeasured.

Native response timing now carries optional `response_end_micros`, measured at complete buffered-response production independently of transport instrumentation. Streaming header producers use `None`; the schema rejects an end before response start. Buffered frame installation projects the original monotonic completion, while service response reuse takes a fresh completion measurement. Generated regression sources cover uninstrumented completion, pre-completion zero, invalid intervals and no old timestamp reuse. Historical/default absence remains unknown. Protocol versions remain reserved for final combined reconciliation. No execution.

Native `TransportTiming.delivery` now distinguishes unknown, network, local cache and validated cache. Reqwest records network provenance; memory/persistent cache hits record local cache; all buffered/streaming/asynchronous 304 merge routes retain transport phases and mark validated cache. Navigation/resource exposure derives transfer size using standardized fixed 300-byte header accounting (network encoded bytes + 300, validated cache 300, local cache 0) and cache delivery type. Missing historical metadata remains unknown; service-response reuse carries permission provenance without asserting an old delivery or replaying timestamps. Generated model/navigation/native-cache regression sources cover this distinction. No execution.

Current native service-worker navigation interception now records a private monotonic worker-start stamp after actual worker selection and before executor startup. Both in-process and parent worker brokers preserve it through service response or network fallback; an owned header overlay retains demand/backpressure/terminal cleanup. Native root/frame timing projects the stamp against the existing Document clock; unknown/future stamps remain absent and reused Response permission provenance never carries an old worker stamp. TransportTiming adds optional worker_start_clock_micros; current protocol constants stay reserved. Native clock/fallback/stream regression sources generated, unexecuted. Synchronous HTTP/data/blob/prepared-local/about:blank clock propagation is now reconciled; broader generic resource workerStart exposure remains separate.

TransportTiming also distinguishes `measured_response` from provenance-only records. Reqwest sets it; service interception measures current header completion when no response phase exists. Root HTTP/buffered projection uses actual header/complete-response observations for provenance-only records instead of interpreting default zero as a measured responseStart. No execution.

Navigation timing caller reconciliation adds serde-default `ResourceTimingMetadata.server_timing: Vec<Metric>` (name, finite nonnegative duration, description). Native parsing/validation caps 256 metrics and 64 KiB name/description data; historical absence means no metrics. Parent/agent/network timing producers preserve native timing permission masking, while same-document navigation reads final response headers. Current worker16/context49/agentIPC18 constants remain reserved for final combined schema reconciliation; default fields do not claim binary backwards compatibility. Added metadata parsing/privacy/initial-author regression sources; no execution.

Terminal timing provenance extends NetworkEvent/EncodedNetworkEvent::Complete, ContextServiceResponse::HttpStreamChunk and PlatformHostResult::HttpChunk with optional encoded_body_size. TransportTiming and persistent CacheEntry preserve the same optional completed representation size; absence means unmeasured and never a decoded-byte guess. Native raw readers/counters publish only at accepted EOF; parent/agent/frame/resource consumers retain exact operation/stream IDs and existing retirement gates. Encoded JSON completion and historical cache metadata accept absent optional fields; no binary-version compatibility is inferred. worker16/context49/agentIPC18 remain reserved for parent final reconciliation. Regression sources generated, none executed.

## September 18 deferred Fetch and rendering provenance source additions

Current source adds ContextServiceRequest PrepareDeferredFetch, ActivateDeferredFetch and CancelDeferredFetch, with ContextServiceResponse DeferredFetchResult carrying the typed DeferredFetchError result. DeferredFetchToken now carries a 32-byte capability: an eight-byte checked per-Document sequence followed by 24 random bytes. Prepare authenticates the live Window creator; accepted capability authority survives source retirement. Native bounded terminal replay/high-water handling is still being integrated. CancelDeferredFetch now also transports Creator attribution for unknown-token cancellation. ReserveDeferredContainer transports page, creator, frame token, source and allow metadata; ReleaseDeferredContainer transports page and frame token. Pre-navigation reservation and terminal cleanup must be reconciled across parent and standalone callers in the final combined variant inventory.

PlatformControl PromptKind adds Microphone, restricting its native permission names to microphone rather than the combined camera/microphone set. PaintImageResource adds serde-default svg_document_snapshot:false and optional content_type; absent historical provenance selects secure fetched-resource mode and does not infer embedding-document authority. Original bytes, Content-Type charset, normalized media type and final URL/aliases remain distinct. New common regression sources cover historical omission, exact raw-byte roundtrip and rejection of null/string/numeric snapshot discriminators.

Worker16/context49/agentIPC18 remain the unbuilt source batch identifiers. This inventory does not claim wire compatibility with old binaries. Final collective schema/version/artifact reconciliation and all execution remain pending; no generator or build ran.


### September 18 native Storage Access wire additions

Platform control Request::StorageAccess { request: bool } and Response::StorageAccess { allowed: bool } connect the JS API to the native context policy/grant decision. Source review confirms both typed variants exist; exact-authority admission and all exhaustive-match/roundtrip regression coverage remain part of final collective reconciliation. ContextOptions.third_party_cookies selects an explicit native context policy; qualification must record the selected policy and cannot infer cross-context grant sharing from persistent cookie storage. No protocol generator, build or test ran.


## September 18 response discriminator and cancellation authority reconciliation

Source inspection found eleven ContextServiceResponse payload fields named `result` colliding with the enum's internally tagged `result` discriminator. Each now uses `#[serde(rename = "outcome")]` on the payload field; Rust variant constructors and matches retain their existing field name. The wire tag remains `result`. This affects DeferredFetchResult, NativePlatformResult, UserActivationRecorded, TransientUserActivation, ClipboardScriptAuthorized, ClipboardRead, ClipboardWrite, MessageWire, WorkerService, NavigationControl and DocumentDomainPrepared. This is an unbuilt collective protocol change, not backwards compatibility with any earlier binary.

Added deferred_fetch_wire_authority_tests source to preserve exact cancellation frame/document-generation/stream-epoch and reject omitted authority fields. Typed ACK source covers success, quota, obsolete-capability and transport failures independently, requiring the outcome field and rejecting null/boolean/untyped terminal payloads. A targeted source search found no hand-written consumers of the changed serialized variant names in native/tools/scripts; production typed consumers retain their Rust API. Broader collective protocol inventory and execution qualification remain open. No builds, tests or generators ran.


### Parent-owned deferred deadline and state contract

Current source adds PrepareDeferredFetch.activate_after:Option<Duration> and QueryDeferredFetch. Activate/Cancel/Query use DeferredFetchState with an `outcome` payload carrying Pending/Activated/Canceled or typed errors; preparation/container reservation/release retain DeferredFetchResult. The native scheduler now owns the deadline independently of source-document polling. Source review confirms request/response variants; root added typed state regression sources rejecting admission-only/null/boolean/unknown-state and untyped error replies. Parent timer/abort races, cancellation ownership and full transport qualification remain in the combined generation/validation audit. No execution or protocol version compatibility is claimed.

Navigation source additions (September 18): ClosePage now carries optional typed
ScriptCloseHistoryProof {creator,single_document}, with authenticated current-root
validation. RollbackAuxiliaryRoot {page_id,expected_generation} separates failed
native auxiliary construction cleanup from author script-close eligibility.
Request inventory/version/prerequisite generation must include these variants
and field shape after the global source gate; no generator has been run.


## September 18 remote script-close target history query

Root generated WorkerRequest::ScriptCloseHistory / WorkerResponse::ScriptCloseHistory and the corresponding PageHandle and local PageCommand route. The target actor reads RawV8Host::native_script_close_history through its own active executable root, without evaluating authored JavaScript or reading origin-filtered Navigation.entries. The query rejects selected-frame handles; no-V8 and unavailable/closed targets return explicit errors. Worker diagnostic labels were reconciled. Generated actor regression source checks same-document history entries preserve the Creator and script-closable result, and closed targets reject queries. No execution occurred.

This is infrastructure for the full remote close algorithm, not completed R05 admission. Root still owns retained asynchronous query/admission, exact source/target authority and familiarity/allowed-to-navigate checks, current target-proof validation and cancellation/unload/departure integration before physical close. The source IPC must return without synchronously waiting on a shared worker. Navigation owns self/same-agent proofs, immutable web-created provenance and exact-generation auxiliary rollback cleanup. Existing physical close custody must not substitute for beforeunload/pagehide/unload processing.

Target close preparation adds WorkerRequest::PrepareScriptClose {token,expected}
and WorkerResponse::ScriptClosePreparation {outcome}, where the typed outcome is
Pending, Allowed(ScriptCloseHistoryProof), or Canceled. Native fetch transport
adds defaulted keepalive to FetchOptions and AgentFetchRequest. Required protocol
inventory/schema/hash generation remains deferred with all execution.


## September 18 collective current-shape reconciliation

See [pending contract batch](P06-PENDING-CONTRACT-BATCH-2026-09-18.md) and its source snapshot for the current complete propagation plan. Earlier close prose is superseded: current ClosePage carries source_page_id, target_page_id, creator and optional expected group, with no proof field; ScriptCloseHistoryProof is required creator + single_entry. Begin/Publish/Abort history projections carry required creator/revision/count. DeferredFetchState increases the internally tagged response outcome inventory from eleven to twelve. Explicit web-feature config, exact frame opener targets, blank prerender scopes and keepalive nested payloads join the same unbuilt batch. Dedicated protocol_pending_contract_tests.rs adds current typed serialization/required-field/discriminator source coverage. Worker16/context49/agent18 constants, signatures and artifacts remain unchanged; no tests or generators executed.
