# P-06 pending collective protocol batch

Source audit and regression generation only. Initial review retained worker **16**, context service **49**, agent IPC **18**; the final September 18 coordinator-approved source cutover below now sets **17/50/19** after wire-owner stability confirmation. These are unbuilt source labels, not a compatibility promise. No tests, generators, signing or runtime artifact generation ran.

There is no frozen source snapshot for the instant these three labels were assigned. Git HEAD has worker 10; treating the entire working-tree diff as changes *since worker 16* would be incorrect. This inventory reconciles the existing P06 ledger with current typed definitions and real producers/consumers. The accompanying source snapshot records file hashes at review time; active lanes can still change them.

## Current pending contract families

| Wire scope | Current contract and exact caller chain |
| --- | --- |
| Worker + context close | `WorkerRequest::{ScriptCloseHistory,PrepareScriptClose{token,expected}}` → worker dispatch → `PageHandle`/PageCommand → native active root. Replies `ScriptCloseHistory{proof}` and `ScriptClosePreparation{outcome}` carry Pending/Allowed/Canceled. `ScriptCloseHistoryProof` is **creator + single_entry**, required and denies unknown fields. |
| Context close authority | `ClosePage{source_page_id,target_page_id,creator,expected}` and `ReadScriptCloseState` carry the current initiating Creator and optional browsing-group expectation. Producers are `host.rs` self-close and `remote_window_v8.rs` remote-close; parent validates exact source/target authority and retained close preparation. There is **no current ClosePage proof field**. Old ledger prose describing optional `single_document` is superseded. `RollbackAuxiliaryRoot{page_id,expected_generation}` is separate construction cleanup. |
| Context history projection | `BeginScriptCloseHistory`, `PublishScriptCloseHistory`, `AbortScriptCloseHistory`, each page/Creator/revision/count, return `ScriptCloseHistoryPublished`. `NativeHistoryProjectionBackend` implements JS native history publisher; parent `script_close_history_projection` owns transactional counts. Do not replace required revision/Creator with defaults. |
| Context frame opener and related Window | `NativeWindowOpener`, `DisownNativeOpener`, `DiscardNativeFrameOpener`, native name/inventory/read-origin/navigation routes; `WindowOpener` preserves `(PageId,WindowTarget)`, `NativeWindowGroup` has `opener_target`, and `RelatedWindowSnapshot` has creator/parent/opener/children/length/origin-domain/url. Producers and consumers are native host/remote Window, parent browsing groups and typed context service. Frame opener identity must survive instead of collapsing to root PageId. |
| Worker configuration + context profile | `ContextOptions.web_features:NativeFeatureProfile{usermedia_element:Standard|Legacy}` passes through Worker Initialize context and policy digest. Context constructor omission deliberately means Standard; explicit profile rejects unknown/URL-selected modes. `WebFeatureProfile{page_id}`/typed profile reply → `usermedia_profile.rs` freezes bindings before construction. Worker limits also carry native scrollbar width; existing config serialization/validation applies. |
| Context prerender | Reserve/ReserveBlank, CommitResponse, ReadAuthority, MarkReady/MarkLoaded, RegisterFrame, Activate/ActivateBlank, PublishBlank, Cancel/CancelBlank requests and typed replies. Capability, generation and epoch are required; blank activation also carries destination page and URL. Readiness differs from loaded and published; constructor readiness cannot imply activation. Private frame registration carries its initiating parent Creator. |
| Context deferred Fetch | Prepare includes Creator/token/PlatformHttpRequest/activate_after; Query/Activate/Cancel/Acknowledge return typed state or admission outcomes. Embedded container reservation/release is separate. Pending/Activated/Canceled are not booleans; Cancel retains initiating Creator. Parent autonomous deadline and terminal ACK survive source-document retirement. |
| Context + agent keepalive | `FetchOptions.keepalive` crosses PlatformHttpRequest → Context HttpStreamRequest.fetch or AgentServiceRequest::PlatformSubmit. `AgentFetchRequest.keepalive` crosses service-worker Fetch commands. Parent/agent prelaunch admission and retired draining inspect this bit. Serde omission deliberately means false, but an old implementation ignoring true is semantically incompatible. `PlatformRetire`/`PlatformReleaseResult`/typed completions carry result cleanup. |
| Context resource loaders | TemplateStreamStart carries native HttpStreamRequest and nonce, typed Script destination/CORS/SRI routing. Frame/stylesheet/module response streams, release/ACK/cancellation tickets, prepared policy and source Creator remain mandatory where defined. Link-prefetch producer work is still active and must join the final freeze inventory. |
| Worker automation and OIL | Scoped WebDriver realm snapshots, frame/script/actions requests, action feedback and failure receipts; focused-form/target resolution; counted-selector and expression waits, WaitForUntil; native accessibility text/snapshot and print snapshot. Worker discriminants and payloads follow typed host/runtime producers; do not reuse a previously signed worker binary for these shapes. |
| Context navigation | Controlled navigation, queued execution token/outcome, form/script/service authority, source metadata/provenance, initial/local/blob/data preparation and commit/discard, related target navigation; full joint history, scroll/encoding/request-state snapshots and script-close history all feed retained navigation custody. |
| Context results | **12** current `ContextServiceResponse` variants have Rust field `result` serialized as **outcome**, distinct from the enum's `result` discriminator: DeferredFetchState, DeferredFetchResult, NativePlatformResult, UserActivationRecorded, TransientUserActivation, ClipboardScriptAuthorized, ClipboardRead, ClipboardWrite, MessageWire, WorkerService, NavigationControl, DocumentDomainPrepared. Typed producers/consumers already agree. The earlier count eleven predates DeferredFetchState. |
| Context event journal | PollEvents optional Creator/cursor/limit plus native Storage/CSP recipients, ACK and bounded diagnostic history semantics. Compatibility inspection cannot consume native event delivery. Cursor/recipient authority and monotonic IDs are semantic protocol changes even where JSON keys persist. |
| Shared platform/media | Native permission choices/replay, microphone prompt kind, StorageAccess, media/PiP sessions and generation, privileged Range and preload destinations, shared memory and native prompt/result publication. See existing P06 ledger for exact nested request/response source list. |
| Shared timing | ResourceTimingMetadata producer clock, navigation phases, response end, server timing; TransportTiming measured-response/delivery/worker-start; response permission provenance. Defaults represent absent measurements, not invented values. All context/agent/platform producers must serialize the same nested types. |
| Agent bootstrap/module policy | AgentPolicy original self URL(s), hide_nonces/policy_sources, AgentBootstrap secure-context/resource digests/import metadata/service registration/controller/top-level/cookie ancestors; typed JS/JSON/Text import kinds, script UTF-16 response, credentials/referrer provenance and native CSP/shared-memory/service requests. |
| Agent service lifecycle | Service Snapshot take_messages and Acknowledge, fetch pull/start/chunk/end/fallback, native stream transfer descriptors, suspension epochs and result receipts. ServiceReceipt denies unknown fields; frame size guards and correlation IDs are independent of the bootstrap version. |
| Message/transfer | CachedCompletionExpired and current cached/private-document capabilities, stream/shared/Wasm transfer hosts, media frame/audio records, client/source identities. These nested MessageWire/AgentService payload additions require the same collective cutover. |

No further missing production discriminator or typed caller was confirmed in this scoped review. The added tests close serialization/required-field coverage; they do not establish runtime authority enforcement, which remains covered by each owning lane's source tests.

## Generated regression source

`crates/oryn-native/src/protocol_pending_contract_tests.rs` is enabled only for Unix V8 tests. It covers close/history field removal, worker preparation/target proof, frame opener identity, prerender generation/epoch and blank activation field removal, all 12 result/outcome roundtrips and required outcome rejection, explicit context/profile defaults, and keepalive through Agent Fetch plus actual framed platform service serialization. Unknown discriminants reject. Tests read current types rather than duplicating reserved version numbers. No tests executed.

## Final propagation patch plan — root-owned

1. Finish all active producers, then freeze/hash the collective source. Reconcile this snapshot with changes made after review, especially link-prefetch, prerender/close and agent platform lifecycle. Reconcile generated binding/schema consumers only after the global source gate.
2. If no intervening assignment changes the labels, bump **worker 16→17**, **context 49→50**, **agent IPC 18→19** together in `worker.rs`, `context_service.rs`, `agent_ipc.rs`. These are proposed next integers, not reserved or applied values. Do not bump only one layer while nested requests/config are incompatible.
3. Source-derived consumers already propagate these constants: `RuntimeBuildInfo`, worker process metadata, Worker Ready initial/attached handshakes and request envelopes, context client/server envelopes/receipts, agent bootstrap/Prepared admission. Preserve exact version rejection; no permissive fallback for old binaries.
4. `worker_client.rs` WorkerManifest signing/verification checks all three versions before verification-cache return. Rebuild both parent/worker, create fresh signed manifest and bootstrap digest after final sources, and verify real admission. Do not relabel an old manifest or cached binary.
5. `scripts/native_protocol_versions.py` and `evidence_protocols.py` read authoritative constants. Build/sign shell consumers, integrated/family OIL runners, WPT official profile, result builders/validators and schemas/ledgers must consume freshly captured identities. Update only assertions that deliberately pin the current candidate; retain historical fixtures and unavailable historical identities as historical. Recheck malformed/missing/stale protocol rejection in both startup paths.
6. After source generation completes, build, run protocol and all source regression suites, fix, sign/verify and regenerate full runtime/WPT/OIL evidence with the actual candidate. Historical ledgers are not overwritten. P06 artifact qualification and conformance remain open until that execution succeeds.

## Final source cutover — coordinator approved September 18

Loading explicitly confirmed its wire generation stable; authority confirmed no
further planned wire changes. The coordinator then approved and this lane applied
worker **16 → 17**, context **49 → 50**, agent **18 → 19** in the three
authoritative constants. The proposal above is now applied **to source only**.
Source-derived consumers need no duplicated number edits. Historical fixtures,
signed manifests, compiled metadata, original reports and prior source-review
snapshot remain unchanged.

### Final additions missing from the earlier inventory

| Contract | Exact current shape and production/custody chain |
| --- | --- |
| Embedded request selection | `ResourceInitiator.embedded_type: Option<String>` has an explicit serde default of `None`. Actual object/embed capture passes the declared type; non-embedded producers pass None. Parent typed loading and portable frame selection keep declared type separate from received Content-Type. Its optional decode does not make old code that ignores selection semantically compatible. `frame_resource_preparation` accounts for its metadata bytes. |
| Embedded no-handler outcome | `ContextServiceResponse::EmbeddedResourceUnavailable` serializes as the unit `result: embedded_resource_unavailable`. It means a completed object/embed fetch with no native handler, not a network error or a fake HTML response. The context completed-response path recognizes it, `FrameResourceResponse::from_parent` preserves it, and `retained_bytes` charges the enum with no invented stream/body handle. |
| Accepted prerender policy | `NavigationPolicySource::Prerender { token, generation, epoch }` lets AuthorizeNavigation validate retained prerender authority before any current-Document lookup. Missing capability/lifetime fields cannot degrade into InitialAboutBlank or successor authority. |
| Link-prefetch | Shared `PlatformHostRequest::PrefetchStart { request }` routes through native host to context `PrefetchStreamStart { page_id, request }`; standalone native transport uses the same category, workers reject it. CSP Prefetch is distinct, while network metadata and SW Request.destination remain empty. No legacy module-style prefetch classification is retained. |
| Final blank ACK | `AcknowledgeBlankPrerender { page_id, activation }` carries full destination page/token/generation/epoch/URL and returns `PrerenderAcknowledged`. Parent direct and worker routed completion dispatch run through the retained navigation cleanup owner before ordinary new-admission checks. Final ACK releases route retention only after actor publication. |
| Destroyed blank receipt | `prerender_blank_activation::Record` retains authenticated destroyed/acknowledged tombstones. Full source and receipt equality are checked before a legitimate destroyed destination yields typed PageNotFound; changed URL/source/capability remains ProtocolViolation. Actor transfer interprets the typed terminal result, rather than retrying a generic protocol error forever. This is semantic completion behavior, not a new public boolean authority field. |
| Shared command envelopes | `oryn-common/src/protocol.rs` separates ten nested action payloads using `operation` and arbitrary ScannerData uses the `data` envelope. These shared wire changes must be included in the collective source snapshot even though they do not introduce a fourth independent native version constant. |

`protocol_pending_contract_tests.rs` now covers required fields for Activate,
Publish and Acknowledge blank receipts, embedded optional/default/type rejection,
the no-handler unit reply and retained Prerender policy field rejection. Native
`embedded_type_request_tests`, `prerender_blank_activation::tests` and retired
worker route regressions cover actual consumer/authority behavior. These are
unexecuted regression sources.

### Snapshot and artifact propagation

The prior `wpt-procedures/p06-pending-contract-source-snapshot-2026-09-18.json`
remains a historical point-in-time review of the unbuilt 16/49/18 batch. The new
`wpt-procedures/p06-final-contract-source-snapshot-2026-09-18.json` records reviewed
17/50/19 source bytes and expands custody to common protocol, blank actor/receipt,
embedded selection/frame response and native Prefetch/CSP producers. Neither
file is the final reproducible candidate archive or a generated runtime result;
later source edits require a new final freeze.

After the global source barrier: regenerate required bindings/protocol evidence,
build matching parent/worker/agent code, sign a fresh worker, record all three
actual runtime identities, create the reproducible source/candidate/input locks,
and execute admission, protocol, full Core and integrated workflow qualification.
Old 16/49/18 peers must reject at the appropriate handshake/envelope boundary;
they must not be relabeled 17/50/19. Missing historical version fields remain
unavailable in explicit historical interpretation. No build, signing, generator,
validator, service or test execution occurred in this cutover.


## Post-test03 LoadHtml custody revision

Worker protocol 18 and context-service protocol 51 replace 17/50 for the repaired candidate; agent IPC remains 19. Historical snapshots and execution evidence retain their actual versions. This revision is source integration pending execution, not a signing or qualification result.

`WorkerRequest::LoadHtml` and the internal page command now carry `policy_token`. The trusted parent stages exact source authority, page, URL, empty response-header list and HTML SHA-256 before dispatch, without replacing the incumbent policy. A parent-owned RAII lease revokes an unused token on completion or cancellation. Parent-backed workers forward this token; standalone native sessions stage their own parent-owned grant.

`ContextServiceRequest::CommitPreparedLoadHtml { page_id, token, document_url, headers, body_sha256 }` returns the existing `DocumentSecurityInstalled` reply. After incumbent parking, commit validates the exact one-use grant and active or retained source authority, then installs the policy. Payload substitution, wrong page, ordinary-local-grant confusion and replay cannot consume a valid unrelated grant. Old pending grants use existing document retirement cleanup; late abandoned commits stay rejected. Current runtime/profile evidence must capture the bumped authoritative constants.
