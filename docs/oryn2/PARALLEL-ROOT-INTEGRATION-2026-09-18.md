# Primary integration source record — September 18

Source generation only. No builds, tests, formatters, project generators, services or deployment execution.

## CSP hash preload production and consumption

- Parent traced script fetch captures all three digests from original successful response bytes with CORS exposure metadata before bytes are discarded; optional preload proof stores this evidence.
- Standalone ScriptFetchResult retains the same evidence; classic preload hands it to local response storage before terminal publication. Decoded script text is never re-encoded for the digest.
- Parent and standalone successful preload consumers now perform policy/response handling even for an identical request. Consumer receipts prevent duplicate hash enqueue on retry. Parent proof-less script candidates fall back to a fresh load; stylesheet reuse retains its existing fallback.
- Hash emission remains Window-client-only through existing parent owner checks and standalone collector boundary. Original request URL and selected effective policy determine the payload. Module-map reuse is unchanged.
- Regression sources added for legacy-encoded raw-byte evidence, identical-policy receipt consumption, parent opaque redirect evidence and repeated consumer acknowledgement. Existing helper regression sources cover failed enqueue and mismatched consumer identity.
- Source caller searches reconciled both changed native fetch signatures and ScriptFetchResult construction. Broader parent/worker report fault matrix, policy-event deduplication and report queue capacity review remain open. These edits do not close S-01 or L policy parents.

## Parallel integration

The primary granted exact shared-function ownership to avoid serial bottlenecks while preserving disjoint edits: reset/activation and editing snapshots; trusted preparation receipts; observer revisions/provider invalidation; native IDB admission; microphone interface tables; legacy parser-driver removal. Each lane retains its own evidence report. The primary added Microphone to the native capture-delegation revalidation branch; microphone source coverage is owned by the automation lane.

The resumed goal is active and points to the user objective attachment. All-source-before-execution remains binding. The previous goal turn made progress by recording the expanded directive; this turn changed production source and regression sources and continued all available independent lanes.

## Reporting callback realm and explicit DOMString caller integration

ReportingObserver argument arrays and report dictionaries are now created in the callback function’s creation realm while registration/queue ownership remains with the observer’s original document. The new foreign-callback regression source checks array/object/body prototypes, observer identity, failed argument construction retention and exactly one later callback. No runtime result is claimed.

The shared DOMParser HTML branch now uses `parse_inert_dom_string`; its XML branch remains unchanged. The text-control execCommand inserthtml branch now uses `parse_fragment_dom_string`, matching its existing DOMString input conversion. The parser and DOM lanes own the typed inert/live parser contracts and broader surrogate/collision regression matrix.

The exact operation audit discovered missing Storage Access and fetchLater production requirements. These are assigned to authority/storage and navigation, respectively; prior generic helper mapping does not close these source obligations. Generic native insertion post-commit failure custody is assigned to the automation lane. Eight lanes remain assigned; failed agent transport turns were resumed after inspecting partial edits.

## Variadic document.write DOMString integration

The shared Document.write/writeln union conversion now accumulates UTF-16 code units in argument order, serializes once, passes mixed arguments to the Trusted Types default policy using the lossless V8 conversion, and appends the writeln newline as a code unit. Both detached-parser branches use push_dom_string. A direct JS API regression source covers all-trusted and mixed multiargument input, lone high/low surrogates, literal private-storage-prefix text, exactly one default-policy call and the newline text node. Live parser transport is owned by the parser lane. This is source generation, not executed verification.

## Observer registration identity and inventory corrections

ReportingObserver now keeps its registration global as a traced private object edge and validates the original observer object identity when resolving its native ID. This avoids deriving registration ownership from a foreign newTarget/prototype or borrowed method context. The new regression source constructs with a foreign newTarget, invokes another realm’s observer methods, and checks that only the registration document’s report queue is consumed. Existing private callback/weak native object ownership remains intact.

Added explicit G-02 source rows for Storage Access and fetchLater, and recounted every nested granular row. These are newly exposed source obligations, not conformance regressions or execution results. All parent and package generation gates remain open.

## Preload policy publication receipt

The preload consumer receipt now covers report-only CSP event publication together with optional hash reporting, for both parent and standalone owners. Previously a repeated authorization could avoid duplicate hashes yet emit the same policy violation again, including stylesheet or decoded-source responses without hash evidence. Publication failure keeps the receipt retryable; successful publication commits it once. Regression sources cover script/style report-only retries without hash evidence, and parent proof retry/foreign-consumer rejection. Queue/report custody remains source-reviewed only; no tests have run.

## Reporting capacity and deferred-fetch integration review

Added a source regression against the real bounded ScriptCspReports collector: preload policy publication fails with its queue full, leaves all prior reports intact, succeeds after capacity is released, and does not publish again on acknowledgement retry. This exercises consumer receipt placement around policy-event admission, beyond an injected closure-only failure.

Reviewed the generated fetchLater binding and sent two concrete integration concerns to its owner: request/signal graph retention and error-path cleanup for native records created without an author Request wrapper; and bounded cleanup reservation when both prepare and cancel transport fail repeatedly. Neither concern is treated as closed until the owning source and regression fixes are reviewed. No runtime execution occurred.

## CSP request-directive correction

Independent current CSP source review supersedes the earlier worklet/default-worker conclusions: script-src-elem post-request delegates to script-post-request with its selected directive, including audioworklet/paintworklet. Worklet hash reporting now follows element → script → default fallback. Worker entries report only when script-src itself is the selected fallback; child-src/default-src delegate to worker-src’s post-request algorithm, which does not emit hashes. Generated sources cover element-directive precedence/no-hash suppression, script fallback and default-worker suppression. Source reference: https://github.com/w3c/webappsec-csp/blob/main/index.bs (script-src-elem-post-request, script-src-post-request, worker-src-post-request, child-src-post-request, default-src-post-request). Pinned fixture comparison remains part of the source matrix; nothing was executed.

## Retired-page context event targets

Native page retirement now removes queued message/CSP delivery records addressed only to that page and removes that page from shared storage-event recipient lists. Events originating from the closed page remain for live recipients, and surviving cursor IDs are not renumbered. A native source regression covers target/source distinction, idempotent cleanup and later monotonic publication. Diagnostic lifecycle history is preserved. This does not close the broader context-journal capacity/acknowledgement/late-publication obligation, which remains open.

## Newly confirmed pinned source dependencies

The DOM audit confirmed frozen Core membership for template-for-html-setters.html and partial-updates/tentative/fragment/sanitize-nested.html. Required missing HTML setter/streaming/patch semantics are split across DOM (safe/unsafe nonstreaming APIs), parser (PI/template-for fragment patching), and loading (WritableStream sink bridge). They are implementation obligations, not Extended expansion or execution-only gates. Cookie audit likewise established actual Core default third-party-blocking/popup-grant semantics; native cookie policy and trusted popup integration are assigned to authority/storage and navigation.


## September 18 response discriminator and cancellation authority reconciliation

Source inspection found eleven ContextServiceResponse payload fields named `result` colliding with the enum's internally tagged `result` discriminator. Each now uses `#[serde(rename = "outcome")]` on the payload field; Rust variant constructors and matches retain their existing field name. The wire tag remains `result`. This affects DeferredFetchResult, NativePlatformResult, UserActivationRecorded, TransientUserActivation, ClipboardScriptAuthorized, ClipboardRead, ClipboardWrite, MessageWire, WorkerService, NavigationControl and DocumentDomainPrepared. This is an unbuilt collective protocol change, not backwards compatibility with any earlier binary.

Added deferred_fetch_wire_authority_tests source to preserve exact cancellation frame/document-generation/stream-epoch and reject omitted authority fields. Typed ACK source covers success, quota, obsolete-capability and transport failures independently, requiring the outcome field and rejecting null/boolean/untyped terminal payloads. A targeted source search found no hand-written consumers of the changed serialized variant names in native/tools/scripts; production typed consumers retain their Rust API. Broader collective protocol inventory and execution qualification remain open. No builds, tests or generators ran.


## September 18 HTML stream and event-journal independent review

Root inspected the generated html_stream::Sink accept/reserve/owner paths. The initial version charged all historical input until close, rather than only retained parser state; owner/closed-parser early errors after reservation bypassed terminal cleanup. Loading now owns the correction and regression sources, coordinated with parser retained-input accounting. Root also requested explicit queued-write suspension/resume behavior; loading reports defer-before-busy and resumed completion polling integration, pending combined review. These findings keep streaming generation open rather than treating initial API presence as completion.

Root inspected Journal::poll and the runtime Compatibility/Storage/Csp routing. Separate cursor records currently acknowledge the same recipient obligation: a compatibility PollEvents acknowledgement can remove an event needed by the native JavaScript channel. Automation owns eliminating that competing-consumer path or preserving distinct delivery custody, with cross-channel regression sources. The compatibility transport has no identified author-delivery producer outside its retained route/tests in the targeted source inventory; full caller disposition is required before removing or changing it. No execution evidence is claimed.

Navigation was reassigned to full R04/R05 caller/checklist reconciliation after its bounded source delivery. All eight lanes remain allocated; individual deliveries do not close package or global generation gates.


### Compatibility inspection correction — parent integration source

Verified Journal::poll now excludes Compatibility from author obligation acknowledgement, matching the actual shim that skips storage and does not dispatch CSP DOM events. Added compatibility_event_delivery_integration_tests in context_service.rs, exercising real parent PollEvents and native CspEvents: inspection ACK does not consume delivery, response retry keeps identity, native ACK releases it, an old cursor cannot resurrect it, and a replacement document cannot inherit the receipt. No tests were executed. Two bounded source-reviewed checklist rows record this integration and the prior eleven response-discriminator fixes; they do not close package/global gates.


## September 18 typed-module destination normative source review

Fetched and inspected the current public HTML specification at https://html.spec.whatwg.org/multipage/webappapis.html#fetch-a-single-module-script and its fetch-destination-from-module-type algorithm. A module request uses the destination derived from module type: css => style, json => json, text => text, otherwise the supplied default. The initiator type remains script. This is not permission to replace request destination with initiator type.

Fetched and inspected https://w3c.github.io/webappsec-csp/#effective-directive-for-a-request: style selects style-src-elem; script/xslt/audioworklet/paintworklet select script-src-elem; json/text/webidentity select connect-src. Style fallback is style-src-elem, style-src, default-src; connect fallback is connect-src, default-src. Loading lane owns correcting both parent LoadTypedModule and standalone ScriptFetch destination/policy/metadata/hash handling, with distinct allowed/blocked policy regression sources. Current normative references supplement exact pinned requirements; no test-dependent switch or denominator change is authorized.

Root also identified cross-document deferred-result eviction before the original JavaScript result is observed. Native lane is generating explicit outcome acknowledgement and protected unacknowledged terminal storage under admission bounds; document retirement and BFCache suspension require distinct treatment. These are confirmed remaining source obligations, not passing execution evidence.


## September 18 remote script-close target history query

Root generated WorkerRequest::ScriptCloseHistory / WorkerResponse::ScriptCloseHistory and the corresponding PageHandle and local PageCommand route. The target actor reads RawV8Host::native_script_close_history through its own active executable root, without evaluating authored JavaScript or reading origin-filtered Navigation.entries. The query rejects selected-frame handles; no-V8 and unavailable/closed targets return explicit errors. Worker diagnostic labels were reconciled. Generated actor regression source checks same-document history entries preserve the Creator and script-closable result, and closed targets reject queries. No execution occurred.

This is infrastructure for the full remote close algorithm, not completed R05 admission. Root still owns retained asynchronous query/admission, exact source/target authority and familiarity/allowed-to-navigate checks, current target-proof validation and cancellation/unload/departure integration before physical close. The source IPC must return without synchronously waiting on a shared worker. Navigation owns self/same-agent proofs, immutable web-created provenance and exact-generation auxiliary rollback cleanup. Existing physical close custody must not substitute for beforeunload/pagehide/unload processing.


### Parent close admission ledger source

Added runtime::script_close_admission::Registry, owned by each BrowserContextInner and initialized in all three context constructors. The bounded ledger admits one immutable source/Creator/token per target, joins concurrent close calls without replacing that authority, binds one exact target Creator during preparation, rejects completion before matching departure, and prevents obsolete receipts from touching a subsequent target job. Canceled jobs release admission; terminal evidence retains 64 outcomes with explicit pruning count, and token exhaustion fails rather than wrapping. Regression sources cover concurrent sources, replacement proofs, stale terminal receipts, cancellation churn and identity exhaustion.

This ledger is not yet an executing close runner. Root still must connect asynchronous target history/preparation, frozen native relationship/allowed-to-navigate admission, retained transport retries, context retirement and physical cleanup. Navigation has generated target PrepareScriptClose with retained beforeunload/pagehide/unload and Allowed/Canceled replay. No runtime conformance is claimed for either unbuilt source group.


## September 18 canonical close semantics correction and familiarity source

Read https://html.spec.whatwg.org/multipage/nav-history-apis.html and https://html.spec.whatwg.org/multipage/document-sequences.html. Current script-closable is web-created OR session-history entries size exactly one; prior distinct-Document helper/claims in this audit are superseded. Navigation is renaming the native proof/helper to single_entry and correcting regression expectations for pushState. The close algorithm sets is-closing synchronously before queuing definitely-close; definitely-close checks whether unloading is canceled and then unloads the document/descendants before destruction. Canceling unload does not imply clearing is-closing in that algorithm.

Generated script_close_authority.rs with native strict-origin familiarity, recursive auxiliary opener context (including frame openers), target ancestor origin checks, bounded traversal, stale-source/target and native-group validation. It deliberately reads original Origin rather than relaxed document.domain identity. Regression source covers forward opener familiarity, denied cross-origin reverse relationship, unrelated same-origin group and stale source. This is a source-generated predicate, not complete public close admission.

Synchronous remote close eligibility requires acknowledged parent history projection, not the asynchronous query or stale PageInfo. Navigation owns precommit projection reservation/invalidation plus exact Creator/count/revision publication and retained acknowledgement barrier. Root public admission/closed getter integration remains open until that interface exists; target-owned unload runner and parent ledger are generated independently. No builds/tests/generators ran.


## Parent script-close history projection source integration

Generated `script_close_history_projection.rs` and page-owned ledger, plus typed Begin/Publish/AbortScriptCloseHistory endpoints and ScriptCloseHistoryPublished ACK. Every mutation authenticates the service page and current active root Creator under the native page/authority guard, rejecting frame, retired and private-prerender authority. A pending revision exposes no previous count. Publish retries are idempotent; abort restores the previous count after a lost begin ACK, handles a begin that never arrived, and cannot roll back an already published revision. Revision tombstones reject replay and mismatched payloads. Replacement Documents start unknown with independently scoped revision numbers; no implicit one-entry fallback.

Generated ledger transition and actual native document-replacement regression sources. No builds or tests executed. Public close admission, sticky is-closing visibility, and runner connection remain required integration work; this delivery does not close R-04 or the global generation gate.


## Logical closing-state source integration

Added page-owned `is_closing`, independent of document replacement, physical teardown and bounded admission receipts. Added exact Window Creator + native group authenticated ReadScriptCloseState RPC. Native local and remote root Window.closed getters now consult it; frame getters retain their own lifetime semantics and remote operations continue using physical liveness separately. Generated regression sources for logical closing surviving canceled receipt removal and document replacement, stale caller rejection, and required Creator/stream epoch on projection/state wire requests.

Public close acceptance still must set the flag while binding the target Creator and transition admission directly to Preparing before starting the existing departure runner. Navigation lane is generating local pending-projection flushing and bounded parent wait/notification with explicit transport errors rather than false ineligibility. The old public ClosePage route is not yet replaced. No execution result or parent checklist completion is claimed.


## Public close acceptance and superseded route replacement

Replaced ClosePage's optional self-supplied boolean proof with required incumbent Creator and expected native group. Local borrowed calls now derive the incumbent endpoint/Creator; remote calls carry their native proxy group. The parent authenticates familiarity, the dedicated sandbox snapshot algorithm, parent history projection, and both current documents under one final page/authority guard. It starts runnable custody before mutation, binds registry state directly to Preparing, and synchronously sets traversable is-closing. Departure remains asynchronous; it no longer physically closes from the public service handler or re-evaluates eligibility after acceptance. Explicit projection/transport failures remain errors rather than false eligibility.

Removed the obsolete direct-self/opener-only may_script_close helper; its old relation assertions are superseded by strict-origin familiar authority and public admission regression sources. Retained related-origin liveness tests and auxiliary rollback's separate initial-document custody. Migrated ClosePage wire/source callers and old authority tests to required Creator and parent projection. Navigation lane owns actual admission/runner integration regression additions; loading lane owns the close-specific target-document sandbox snapshot implementation. No build/test execution or global generation completion claimed.


## Queued-close active-document correction and checklist reconciliation

Canonical definitely-close selects the traversable's active Document when its queued task runs, then selects the active Document for the traversal unload step. The source review found that immutable admission-Creator checks in the target preparation/runner incorrectly canceled an accepted close after an intervening navigation. Navigation lane is correcting execution-time document selection while retaining immutable admission-token custody and terminal replay. It is also adding retained destructive-write guards across beforeunload and pagehide/unload, replacing the earlier test that expected document.open to replace during unloading.

Reconciled three bounded R-05 source-reviewed rows (history projection, logical closing, close-specific sandbox snapshot), and added an explicit open public-departure integration row. Current granular inventory: 330 source-reviewed / 93 open. All nine package and 51 parent gates remain open; counts are not an effort percentage. Remote public close now checks typed acknowledgement shape consistently with the local caller. No builds/tests executed.


## Hidden prerender terminal visibility and teardown

Integrated hidden PageRecord provenance with group close/crash. Hidden candidates still retire native message, prompt, navigation and drag state and retain ordered physical teardown, but do not publish public Closed/Crashed events. Active-page fallback skips hidden candidates. Context lifecycle diagnostics apply the same visibility filter while Closed still retires journal delivery custody before suppression. Generated source regressions exercise visible-page fallback, idempotent hidden publication, physical close completion, sibling preservation and crash diagnostics. Authority lane owns remaining navigation/startup/focus/promotion visibility branches. No execution occurred.


## Hidden public-close admission boundary

Public closing-state reads now reject a hidden source or destination even when its native initial root authority exists. The final serialized close-acceptance guard repeats the visibility check before creating a job or setting is-closing. Generated a source regression for rejection with current root authority, absence of admission/state mutation, and readability after publication of the same native page. Native hidden cancellation and ordered physical teardown remain available through internal custody; they do not use public script close. No execution.

Parallel reconciliation identified independent Core link-prefetch production as missing: the legacy scan conflates it with module preparation. Parser lane is assigned the dedicated producer after its XML/stop fixes; loading and family lanes supply native policy and pinned caller evidence. This is distinct from Extended speculation document-rule/prefetch support, for which the frozen Core caller audit established no dependency. No manifest or denominator changes were made.


## HTML stream terminal custody and in-flight suspension review

Inspected the delivered sink, PreparedInsertion guard attachment and actual native scheduler regression source. Root review found that owner validation still classified in-flight author-triggered suspension as an error and aborted the stream; automation lane corrected this to retain the write's converted input, parser, event/preparation frontier and once-only final checkpoint. Source regression now covers suspension during conversion and inline execution, in addition to parked blocking completion. Added one bounded source-reviewed S-01 row; inventory is 331 source-reviewed / 93 open, with all parent/package gates open. This is source evidence only.


## Native link-prefetch CSP source integration

Added CspDestination::Prefetch as a resource-hint policy category, separate from the empty Fetch/ServiceWorker destination. Authorization treats supported fetch-source lists as a union within each policy, intersects enforced policies, permits a policy without default-src, and reports a denied hint under default-src. Unlike ordinary script fallback, a restrictive script-src-elem does not hide an independently permissive script-src. Unsupported prefetch-src, base-uri/form-action/frame-ancestors and attribute-only directives do not grant resource-hint access. Inline nonce/hash/trust metadata is not interpreted as permission to prefetch a URL. Existing redirect path matching and original request report-URL handling remain in use.

Evidence inspected: all nine pinned content-security-policy/resource-hints/prefetch-* fixtures, including generated directive permutations, conflicting permissive policies, absent/default rules and unsupported prefetch-src; CSP resource-hint algorithm and effective-directive text. Generated dedicated regression sources for supported directive permutations, excluded directives, per-policy intersection, report-only outcomes, redirects and non-URL trust tokens. Parser lane integrates this policy destination in native parent and portable producers. No tests/builds executed.


## Final qualification native-component source review

Reviewed completion_qualification's required native-media inclusion, receipt-gated native domain consumers, final exact-component equality and cycle-free booking diagnosis path/original before-after source custody. Added root regression sources that independently reject an invalid original receipt and invalid domain evidence for media, prerequisites and topology, alongside the existing full positive component fixture. These source tests do not substitute for actual platform evidence or prove booking causality. Granular inventory now 332 source-reviewed / 93 open; all whole-package and parent gates remain open. No tests, validators or builds executed.


## Portable frame resource result boundary

Generated unconditional frame_resource_response.rs with native PageResource, PageBinaryResource and PageResourceNoCommit outcomes, preserving every existing page-resource field. The Unix-only TryFrom adapter consumes exact ContextServiceResponse variants and rejects unrelated ACKs without exposing payload text. Extracted the shared embedded-document MIME classifier into the portable module and reexported it from the old parent location, preserving callers. The existing mixed_content::enforce is already portable; no duplicate policy algorithm was introduced.

Generated source regressions for MIME essence/classification and native result adaptation, including original opaque initiator/referrer provenance, POST bytes, policy token, stream identifier, response/header/encoding metadata and explicit no-commit/binary outcomes. Navigation lane owns portable loader/completion/caller conversion. No execution or full portable frame completion claimed.

## Portable frame response retained allocation accounting

Source inspection found the frame preparation charge only covered document payloads and omitted binary/no-commit responses and several metadata fields. `FrameResourceResponse::retained_bytes` now counts inline storage plus owned string/vector capacities across all variants, frozen request provenance, POST data, native origin, headers, policy/referrer fields, timing metrics and both redirect exposure chains. Checked arithmetic rejects overflow; the preparation queue delegates to this portable accounting and charges retained error strings. No serialization or duplicate body allocation is needed to measure the response.

Generated regressions cover binary reserved capacity, no-commit reasons, nested timing exposure and arithmetic overflow. These are source changes only; no build or test executed. Navigation owns the remaining portable frame consumer integration.

## Global source inventory reconciliation

Reviewed final loading consumer table alongside native cache-only linking and shared parser/page graph-limit setup. L-02.g/h now record bounded source completion; a separate continuation execution row preserves full platform and lifecycle qualification. Added the frame-response accounting source row. Recounted only the nine granular package sections: 335 source-reviewed rows, 92 open rows. No parent acceptance gate closed.

Confirmed new source work remains assigned: canonical object/embed declared-type selection and request semantics (loading), portable auxiliary/blob/COOP integration (rendering), and exact blank-prerender destination-destroyed receipt custody (DOM). Automation is checking completion policy/body accounting beyond response payloads. The standalone SW audit traced Unix BrowserContext actors and in-process pages to genuine context-backed service registration/control; raw ownerless convenience APIs are distinct. Plan section 2 requires signed macOS and evidence for each claimed policy, so additional backend work must be tied to an actual candidate/qualification claim rather than inferred from an optional low-level constructor. No source-only result establishes conformance.

## Native close visibility integration regression source

Reviewed target-owned script-close preparation: admitted Creator remains replay authentication while queued close selects the current document; earlier traversal departure/publication retains ownership; the native close receipt retains selected-tree unload guards through hide/visibility/unload. Added `close_visibility_is_ordered_and_cannot_replace_selected_parent_document`, asserting root/child beforeunload, pagehide, hidden visibility and unload ordering, exact terminal replay, and ignored parent document.open/write/close from the selected child's visibility listener. This complements the dynamic-frame selected-parent versus child-only guard matrix. Regression source only; no build or test execution.

## Shared initializer configuration repair

Source review confirmed `PageState::initial_document_cleanup`, its type and implementation are all gated by `all(unix, feature="v8-host")`, but the initializer had lost that gate when `script_close_preparation` was inserted. Restored the cleanup field's matching gate and removed the duplicate gate from the close initializer. Reviewed included cleanup type/impl declarations and the corresponding field declarations; no compiler or test was run.

Portable auxiliary driver review also identified a possible lost wakeup when initial root adoption fails before the driver table's next deadline is assigned. Renderer is checking retained unclaimed roots and host extraction recovery. The remaining source audits have found concrete issues now assigned: DataTransfer constructor effectAllowed and interactive drag-source exclusion (navigation), committed synchronous DOM mutation-tail recovery (DOM), and embedded failure-event semantics/cancellation acknowledgment (loading). These remain source-generation obligations, not validated fixes.

## Evidence-map source-path reconciliation

Inspected the draft 51-parent evidence map and verified that every listed implementation/test path exists. Enriched R-04/R-05 with retained blank activation, root/frame departure, synchronous close admission, target preparation, logical closing and history projection; added direct regression owners rather than relying on broad runtime files. P-06 now cites the pending contract regression matrix and context wire definitions. L-04 includes the portable response/accounting consumer and its regression sources.

This remains a draft, not execution admission: no completion flags or frozen source hashes were asserted. Final wire reconciliation is assigned to the qualification lane while loading finishes embedded-type and explicit unavailable-result changes. Current constants remain worker 16/context 49/agent 18. All original qualification requirements remain open until source closure and actual execution.

## L-01 source inventory closure and remaining integration

Reviewed the pinned helper production map, preserving its distinction between native TextEncoder, legacy HTML URL/form serialization, actual frame decode consumers, XML shared dependencies and platform media decode. L-01.b now records bounded source inventory completion; original RSS/timeouts and codec/global behavior remain a separate unchecked execution obligation. Recount: 337 source-reviewed granular rows and 92 open rows. No parent acceptance gate was closed.

Auxiliary adoption retry and shared-host recovery source now address root review findings. Remaining active implementation is tracked in the global barrier checkpoint instead of treating completed audits as absent algorithms. Decoded fixture-string maps cannot assert HTTP MIME/status; real embedded response semantics stay covered through actual broker response sources.

## Frame completion accounting integration review

Reviewed the delivered producer reservation paths and `reserve_frame_response`: response and standalone policy are admitted together; failure replaces both with the bounded terminal error covered by owner metadata. Standalone error completion does not retain the broker, and metadata/ancestor/context transfer avoids cloning allocations after admission. Expanded the existing checked frame-accounting row to cite the five completion/cancellation regression sources. This does not assert allocator-exact or whole-document memory accounting; shared NavigationTiming ownership is separately being reconciled against real admission limits.

Approved the final collective wire source cutover to worker 17/context 50/agent 19 after wire owners confirmed the embedded type/unavailable response and blank completion contracts stable. Build/sign/runtime compatibility evidence remains deferred. Embedded navigate requests must preserve include credentials under COEP credentialless; that policy applies to no-CORS subresources and must not silently alter navigation credentials. Loading owns the scoped correction and regression sources.

## Prerender feature-detection switch

Final source reconciliation found `script_supports_static` still advertising only classic/module/importmap despite the integrated native speculation-rule parser/actor/activation paths. Added exact `speculationrules` recognition and a feature-gated native candidate registration regression with conversion/case/whitespace controls. This removes the obsolete pre-integration false feature result; no tests or conformance executed.

## Final source-evidence status reconciliation

The draft parent map now distinguishes delivered queue/auxiliary/embedded/S inventory work from still-active final review. NavigationTiming metadata admission remains the concrete active implementation; final S/V package reviews reconcile historical rows against their current producer/caller maps. Added native speculation feature-detection sources to R-04 evidence. The declaration remains deliberately incomplete and unhashed while those edits continue. No execution occurred, and no original runtime failure was upgraded.
