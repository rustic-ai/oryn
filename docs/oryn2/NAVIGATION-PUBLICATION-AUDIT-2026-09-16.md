# Navigation publication generation audit — 2026-09-16

Status: source generated; coordinator source review pending. No build, formatter, test, service, or WPT execution performed in this generation lane.

## Generated custody and callers

- `navigation_api.rs::sync_history_inner` stages missing entry wrappers and their registration rollback, a complete replacement order/session record, committed resolver settlement, current-entry change, and dispose events before metadata publication.
- Native history and event publication return explicit acceptance receipts. A post-acceptance callback failure retains the receipt and event continuation; retry acknowledges completion without applying history metadata again.
- `HistoryPublication` retains per-callback event progress, completion stages, precommit/intercept handler values and promise machinery, and commit-plan progress. Promise fulfillment/rejection callbacks transfer their continuation requests into native state before fallible completion work. Nested publication is inserted ahead of the waiting outer tail; queued plus active batches are bounded.
- `Completion` retains committed/finished resolver stages, exact entry/reason, native focus transition, AbortSignal propagation, scroll, and success/error event publication.
- `native_platform/publication.rs` supports retained native DOM paths in addition to platform/global events: assigned slots, shadow retargeting, related-target pruning, closed-path target clearing, live wrapper ownership, per-phase listener snapshots, once-listener custody, property-handler retirement guards, and callback identity.
- `native_platform/focus_publication.rs` retains dirty-control change, blur, focusout, focus, and focusin phases. Author focus revisions suppress the superseded automatic target.
- `native_platform/abort_publication.rs` prepares the complete dependent set before marking it aborted, retains the first reason and strong wrappers, and resumes Observable algorithms, fetch cancellation, and abort events without repeating consumed callbacks. Admission and retry deadlines are bounded. Fetch-body cancellation restores pending custody on fallible cancellation preparation.
- `history_events.rs` exposes native event construction separately from dispatch. Same-document history transition callers now use retained popstate/hashchange publication.
- `host.rs` / `rust_executable.rs` expose native receipt operations and a retained descendant page-transition cursor. Each recipient retains its receipt; root, recipient, and ancestor frame identities are revalidated before resumed delivery.
- `runtime.rs::PendingHistoryPublication` retains an atomic visible/full session history and activation transaction, receipt acknowledgment, history transition, and pageshow stages. Both cached restore and same-document native navigation use it. Pending queues move with the document during BFCache suspension and restoration, including rollback restoration.
- `sync_activation` stages entry and native activation registrations before updating the activation map.

## Regression sources generated

- Entry staging failure leaves history, entry maps, and native registrations unchanged.
- A currententrychange callback failure resumes the same event and disposes a removed entry once; restored selection reuses the original entry wrapper.
- Navigation finish retains navigatesuccess after pending navigation ownership transfers.
- Native receipts distinguish an accepted callback failure from a precommit staging failure.
- An intercept handler's returned value survives failure without invoking that handler again.
- Abort dependency marking, reason identity, internal algorithm order, same-event retry, and pre-mark failure.
- Focus callback retry without duplicate blur, plus author focus supersession.
- Native BFCache park/rollback preserves queued history publication and delivers its history event once.

## Review boundary

This audit records generated source, not passing conformance. Coordinator review must inspect queue reentrancy, V8 lifetime/borrow signatures, promise microtask ordering, bounded admission, and native BFCache/tree caller integration before the generation row is accepted. Initial `navigate` admission now uses the retained engine with the fail-closed cancellation semantics below. This change does not replace every generic DOM dispatch entry point.

## Initial cancelable admission closure

- `NavigateRecord` records admission completion/failure separately from history commit. Native operation tokens require a completed, nonfailed, noncanceled admission.
- Infrastructure dispatch/preparation failure and V8 termination cannot return `Proceed`. The original synchronous error is returned; the exact remaining event/phase/listener snapshot is retained and the admission is marked non-proceeding immediately.
- Recovery finishes that event tail and rejects only the captured navigation. A reentrant replacement navigation is never rejected through the old global pending slot. Ordinary author listener exceptions remain reported exceptions and do not turn into cancellation.
- A failed native request is not retried automatically: caller-owned POST/referrer/download data is never reconstructed by the event queue. An explicit new caller request receives a new admission identity; recovery of the old event never issues a second request.
- Regression sources cover failed native admission without a request/token/history commit, exact event tail and later cancellation, explicit new-request admission, termination versus ordinary author exceptions.

## Coordinator review corrections generated

- `PageState::next_timer_delay` and `has_runnable_page_work` include the native history queue deadline; suspended documents are excluded and keep their queues in BFCache. Added due/not-due/parked/restored scheduler source regression.
- Full native session history and activation now commit in the staged JS history transaction before currententrychange. The old post-callback session/activation stages were removed, preventing their stale snapshots from overwriting author pushState/replaceState/navigation. Added reentrant replaceState source regression checking activation/session visibility and surviving author state.

- Cached restore now preflights the **destination cached document's** publication queue before incumbent suspension or authority restoration. That allocated queue moves intact into the restored page; there is no fallible postcommit queue admission. The prior incumbent reservation and late destination reservation were removed. Added full-destination-queue source regression asserting unchanged active/cached document generations, realm roots, cache reuse, and no incumbent pagehide.

## 2026-09-17: local target inventory and BFCache suspension

Independent source review found that `AgentEnvironments::is_live` does not exclude parked Documents. The native local target inventory therefore exposed parked root/frame snapshots despite its negative-entry comment. Generated correction: isolate-owned `NativeRealmIdentities::parked_environments` records exact environment IDs after successful parent park admission, remains set through partial restoration, clears only after complete restoration, and remains set after discard until execution retirement. Failed park admission does not publish suspension; failed restore/discard retains custody. Native local inventory retains empty page entries to avoid synchronous calls back to the blocked actor. Root/frame/parent-receipt submission consumers reject parked destinations.

The source regression `native_popup_preserves_synchronous_global_dom_and_root_retirement` now creates a real native iframe forest alongside an independent auxiliary root and checks park/partial restore/restore/discard/retirement against the actual inventory producer and selected-target consumer. It asserts restoration preserves the same frame connection and that the other root remains selectable. The existing manually constructed empty-inventory receipt regression remains a resolver control, not evidence about inventory production.

No build, test, formatter, server or WPT execution was performed. Coordinator review and execution remain pending; this bounded correction does not close R-04/R-05 or full target selection.


## Beforeunload native owner and ancestor reconciliation — September 17

Source review found beforeunload prompt admission checked generation alone after callbacks. It now snapshots native document, generation, parser stream and authority, and rejects suspended/retired or changed owners before showing a prompt and after receiving its result. Host tree traversal keeps the exact binding-state owner and every original ancestor link/identity for each recipient; an unchanged immediate parent link no longer permits delivery through a parked/replaced outer ancestor.

Generated regression sources: beforeunload_cannot_prompt_after_callback_replaces_stream_or_authority and beforeunload_skips_descendants_of_an_ancestor_parked_by_root_callback. They cover suppressed prompt requests, released dispatch guard, descendant exclusion and restored delivery. This is a bounded identity correction; complete retained lifecycle/event continuation, native navigation caller matrix, and execution remain open. No tests or builds ran.


## September 17 — retained beforeunload event and return conversion

Generated `prompts::BeforeUnloadReceipt` retains the original native event, callback cursor, and admission identity. Registered global handlers now retain their returned value separately from the remaining listener queue, so suspension after the handler cannot lose its DOMString conversion or replay the handler. Property-handler fallback has equivalent retained return conversion. The host retries source-safe publication failures against that same receipt; synchronous callers deny admission and cancel when they cannot retain a parked operation. Prompt backend errors are terminal to avoid duplicate prompts after a lost response.

A native ownership token and generation/stream/authority snapshot prevent an obsolete receipt from releasing a replacement stream's dispatch guard. Retirement releases the native owner. Generated regression `retained_beforeunload_preserves_property_result_and_dispatch_owner` covers suspension, conversion before the listener suffix, one prompt, repeated poll, nested dispatch, stream replacement, cancellation and retirement. Existing stream/authority/suspension rejection sources remain applicable.

Source inspection only: no build or tests run. Full asynchronous navigation admission, all close/unload callers, prompt failure injection and the complete R-05 lifecycle matrix remain open; this bounded change does not close R-05.

### History request document-state generation — September 17 (in progress)

Pinned `html/browsers/browsing-the-web/history-traversal/document-state.https.html` requires original initiator origin and referrer on uncached traversal and reload, even after the current preceding entry changes to the destination origin. Source inspection found only current-document request initialization. `NavigationHistoryEntry` and commit resources now retain `NavigationRequestState`; the HTTP continuation publishes its original initiator/opaque bit and final redirected referrer/policy. Both actor and serialized HTTP replay read this document state while preserving current source generation, creator and admission checks. Existing HTTP redirect regression source now asserts persisted original initiator and no-referrer suppression.

Still open in this generation group: root `document.referrer` is currently read from an authored global compatibility slot rather than native document metadata; full public A/B traversal/reload regression and same-document entry clone/persistence audit; local/frame request-state paths. Do not mark this group or broad history complete. No execution.

### Native document referrer and request replay regression — September 17

`DocumentMetadata.referrer` now owns the creation request referrer; the Document getter reads the receiver's native metadata. Removed the `__oryn_document_referrer` bootstrap/getter/setter compatibility route. Native host setters initialize frame referrers, and root construction receives the final HTTP request referrer before authored parsing. Same-document history updates clone their existing document payload, including request state; replacement allocates a fresh entry and request state.

Generated `reload_and_uncached_history_keep_original_request_document_state` follows A→B, then either reloads B or returns to A, replaces it with B-origin, and traverses forward. It asserts original initiator, cross-site Fetch Metadata, original Referer, initial-script/document referrer and resistance to mutation of the former global slot. It explicitly evicts cached documents to isolate the repopulation path; this does not qualify no-store cache-admission semantics. HTTP redirect regression also checks retained no-referrer suppression. No execution. Remaining source reconciliation includes local/auxiliary/frame document-state replay and handoff persistence; this is not broad history closure.

### Shared request-state serialization preparation — September 17

Moved `NavigationRequestState` into `oryn-common::navigation_request` so native root/frame/history transport can share one record. It now serializes the explicit absent referrer and opaque-initiator bit, uses a canonical policy string, validates URL/policy shape and bounds each URL to64KiB before retained history admission. Initial navigations normalize an absent/invalid source URL to None rather than persisting an empty URL. Root HTTP commit runs validation before replacing the incumbent. Generated round-trip, opaque/no-referrer, malformed metadata and size-bound regression sources; not executed.

Current frame source inspection confirms `FrameRequest`/`SavedFrameRequest` only retain POST/form authority; frame original request provenance is not yet integrated. Auxiliary adoption also reconstructs root entries from navigation-only snapshots. Those are confirmed remaining caller gaps, not completed by introducing the shared record.

### Frame request descriptor custody — September 17 (source integration ongoing)

`FrameRequest` and serde-default `SavedFrameRequest.request_state` now retain the shared provenance record. Frame snapshot capture, restored initial frame descriptors and later frame navigation descriptor restoration preserve it; both serialized descriptor accounting and live Arc-owned reservations include validated provenance bytes, even for GET requests with no POST body or form authority. Generated `frame_history_request_provenance_round_trip_and_budget` and `frame_history_get_request_provenance_owns_and_releases_its_bytes` cover round-trip, absent legacy fields, invalid policy, exact accounting and last-owner release.

Fresh frame response capture and replay consumption remain pending; the current live frame commit still passes None until authenticated response metadata reaches it. Descriptor plumbing is not a claim that the full frame request-provenance requirement is implemented. No execution.

### Fresh frame response provenance — September 17

Parent HTTP/data/blob and standalone frame response producers now return an optional typed request-state record alongside document referrer. It freezes original native initiator/opaque status and final reduced referrer/policy; response referrer-policy remains separate from the policy used for the request. Dynamic frame commits retain this record even for GET without form authority. Queued frame response memory accounting validates and reserves provenance bytes before publication. Expanded parent and standalone redirect regression sources assert original initiator and reduced referrer after redirect policy changes.

Still open: parser-preloaded frame propagation, replay consumption/admission and auxiliary adoption. No execution or broad closure claimed.

### Parser-preloaded frame request provenance — September 17 source integration

Parser response custody now retains and budgets the original NavigationRequestState and actual document_referrer alongside the body. Initial parser-created frame sessions retain that state by native document ID before author execution; the native document getter receives the actual response referrer instead of the parent URL. The same-URL two-frame regression now checks distinct referrers in author scripts and retained original initiator/request-policy state. Sources only; not executed. Frame reload/traversal consumption and auxiliary history provenance remain open, as does full navigation qualification.

### Frame history request replay — September 17 source integration

Frame navigation context now selects original request state only for an existing history entry or reload of the current entry. Parent ResourceInitiator carries a serde-default request_state; both parent and standalone frame producers validate it, retain bounded request bytes, and use its initiator/opaque status for Fetch Metadata and cookie context and its referrer/policy for header generation. Current creator, CSP, mixed-content, ancestry and response admission remain separately owned. New-navigation replay state is rejected. Reload also recovers POST/form state when no explicit destination snapshot is supplied.

Generated sources cover parent reload/traversal with present/suppressed referrers, rejection without history flags, standalone opaque/nonopaque replay, and current-entry selection versus newly allocated destinations. No execution. Parent wire/provenance authority qualification, auxiliary adoption and complete history/conformance gates remain open; new ResourceInitiator field must be included in final protocol/hash reconciliation.

### Auxiliary adoption and explicit about:blank source reconciliation

Source inspection of page_actor_turn and auxiliary_root_v8 shows adopt_agent_document receives only the already-created initial auxiliary document, before queued network navigation. Its fresh entry construction does not discard a previous HTTP response request state; that earlier suspected gap is not established. Initial-document referrer semantics still need full creator/spec reconciliation.

Confirmed pinned obligation: html/browsers/browsing-the-web/navigating-across-documents/source/navigate-child-src-about-blank.html expects an origin-only referrer for explicit blank navigation under the default policy. The local frame path previously copied the full parent URL. It now calculates referrer through the native request policy and retains original request state for subsequent history handling; srcdoc overrides continue to take precedence. Both Unix and non-Unix local branches consume the calculated referrer, and the non-Unix blank URL is preserved. Added default/no-referrer/unsafe-url/same-origin regression source. No execution or conformance claim.

### Document referrer lifecycle source review

The native getter reads the receiver Document metadata, while document.open clears children without replacing metadata. Added a binding regression source for active versus inert receiver, forged legacy Window value and open/write/close preservation. The DOM replace_document_contents_in API copied URL/base/encoding but discarded referrer; it now preserves the existing request referrer, with replacement/clear regression source. Current native/JS production searches found no callers of that replacement API, so this does not establish additional runtime coverage or close a conformance gate. No execution.

### Actor replies for joint Back/Forward

Native Back/Forward now enters the shared joint-history delta API and returns a retained NativeJointTraversalReceipt. NavigationProgress and PendingNavigationReply have an explicit joint case, so unchanged root generation/readiness cannot falsely complete child-only traversal. Receipt polling handles cancellation/retirement/failure and deadline, follows selected native commits across root replacement, and refreshes final response metadata after completion. Closing/shutting down the actor cancels joint receipts held by ordinary or execution/controller replies.

Controlled navigation retains its emitter separately from root-only load publication, waits for joint receipt completion, and retains the command reply except for explicit PageLoadStrategy::None. Added actor regression source for ready-root/pending-child and deadline cancellation. No execution. End-to-end root/frame lifecycle settlement, controlled emitter failure/retry, joint cancellation and all qualification remain unverified.

### Joint controlled completion acknowledgement custody

Controlled joint replies now freeze their terminal result and retain each publish_pending acknowledgement in sequence. Pending acknowledgements neither advance the phase nor enqueue the milestone again; negative acknowledgements terminate with error. The command reply follows acknowledged completion, with an explicit timeout stop. Regression source injects a pending commit ACK and a negative ACK, checking unchanged phase and no premature later milestones. No execution.

Independent review identified an additional readiness contract gap: selected entry commitment alone cannot justify Interactive/Complete while child parsing/loading continues. The joint-history owner is extending receipt readiness to selected document completion; this remains an open integration requirement.

### Joint cancellation and no-commit semantics

Joint controlled replies retain ordered parent cancellation checkpoints while waiting, because their emitter is intentionally separate from root-only lifecycle. Explicit actor Stop cancels native joint traversal group receipts before load cleanup; normal source-document retirement remains distinct.

Pinned navigation-api/navigation-methods/return-value/back-204-205-download.html requires authored navigation promises to remain pending after no-commit responses. Added a token-checked native receipt-only no-commit settlement API through JS native navigation, host and executable; finish_navigation_without_commit uses it without rejecting authored promises or firing navigateerror. Expanded existing 204/205 regression source to require native receipt cancellation while retaining empty authored event/promise results. Builds/tests remain unrun.

### Queued traversal cache and hidden no-commit integration

The actor's queued script traversal now resolves the frozen history identity and attempts reusable back/forward-cache restoration under the already installed native operation before starting a resource request. It restores temporary request context on either success or failure. Added a regression source that retains a global object and Document through queued Navigation API back traversal without a network broker.

No-content native settlement now receives the actor's frozen destination identity when a hidden cross-origin operation has no exposed Navigation API pending entry. The exact native revision guard remains mandatory; settlement changes only joint receipt custody and leaves authored promises/revision untouched. Added a regression source covering a hidden selected destination, stale-token rejection, and current-token receipt cancellation. All sources remain unexecuted; complete navigation and qualification gates remain open.

### Active joint failure acknowledgement reconciliation

All terminal results from the active joint reply poller now use the retained publication state, including parent checkpoint errors, authority deadlines, and semantic-projection synchronization failures. These paths previously used the immediate completion fallback and could reply before a remote terminal-event acknowledgement. Success and failure now share the same ordered acknowledgement custody; shutdown fallback remains separate. Added error-publication regression sources covering repeated pending polls, accepted and rejected terminal acknowledgements, and preservation of the original stopped reason. Source review only; no execution qualification implied.

### Hidden traversal failure identity

Hidden cross-origin history admission and queued traversal now retain the frozen selected entry id inside the private native operation token. Exact binding, generation, authority epoch and operation revision still gate every settlement; foreign entries remain absent from Navigation API history and pending state. Failure delivery can therefore settle the selected joint receipt after the actor restores its temporary request context, without depending on exposed pending navigation. Aborts cancel the receipt; native network/security/quota failures retain a failed outcome. Added hidden admission-to-queue network-failure source coverage, including private entry non-disclosure and consumed-operation rejection. Corrected an unrelated rejection-helper identifier introduced during earlier no-content editing. No execution.

Independent remote cleanup review traced queued control-job reservations, latched negative acknowledgements, and ContextNavigationStop's retained parent stop ticket. Dropped or failed publishers preserve terminal cleanup custody; a parent terminal acknowledgement releases it. This is source evidence only, not proof of the full cancellation/qualification matrix.

### Joint eager readiness and final load custody

Joint receipts now distinguish selected-document interactive readiness from final load/publication completion. The controller publishes Committed and Interactive when the retained interactive milestone is ready and waits for the exact parent acknowledgement. An eager controller reply then resolves, while the pending traversal continues to own its final load, terminal-event acknowledgement, and cleanup. Normal replies remain pending until final completion. The final phase publishes only Complete after an acknowledged Interactive prefix, avoiding duplicate milestones. Added source regression covering delayed interactive acknowledgement, eager versus normal replies, retained cleanup before load, and final completion/release. All constructor callers were reconciled, including actor settlement. No execution or whole-R-04 closure is claimed.

### Root target-name commit policy — source generation

Read the public [HTML navigation specification](https://html.spec.whatwg.org/multipage/browsing-the-web.html) on September 17. Finalizing a cross-document navigation clears the destination document-state target name when the navigable is top-level, the destination is not an auxiliary context retaining an opener, and the destination strict origin differs from the incumbent origin. Root security publication now applies that predicate atomically with COOP/opener/origin/policy changes. Strict native opaque identity participates in comparison; relaxed document.domain or URL serialization does not substitute for it. Added same-origin retention, cross-origin clearing, auxiliary-opener retention, and COOP-severing regression sources.

This is not complete root name/history generation. Root per-document-state saved-name capture and restoration, including cold history and BFCache, remain to integrate. The frame lane separately retains bounded names keyed by saved document identity. The pinned main-frame window-name fixture does not call test completion and differs ambiguously from its auxiliary fixture; no upstream defect disposition is granted by this observation. All validation remains deferred.

### Parent cached root name custody

Parent CachedDocumentSecurity now retains a root target name with its exact origin/history capability; frame snapshots keep their separate frame-name mechanism. Authenticated root name setters update current state and existing strictly same-origin cached root records under the same pages/origins/snapshots/hub lock order. Shared Arc strings avoid duplicating a setter value across the group; existing 64 KiB name and 1024 snapshot admission limits bound retention. Cache restoration installs the saved name atomically with opener/origin/security restoration, before author code can read it.

Added a source regression for same-origin cached-name updates, cross-origin independent names, round-trip restoration, oversized-name rejection and stale-document rename denial. Cold/refetched root history names and standalone root inheritance remain open integration work; no whole-name or R-04 closure. No execution.

### Root history name capture and bounded metadata

Root suspension now reads the exact live root name through the parent endpoint before authority parking, without authored property access. Standalone roots use their native binding state. Retired/suspended capture and oversized names fail explicitly. This avoids recording a stale isolate cache after a related context renamed the root.

`NavigationHistoryEntry` now retains an optional shared name. Capture updates existing entries of the same strict origin, including exact opaque identity, and copies those values into the staged history list by immutable entry identity. Retention has a 64 KiB per-name and 16 MiB aggregate allocation budget; capacity admission precedes all name metadata mutation. New regression sources cover shared tuple-origin groups, distinct opaque documents, per-name rejection, aggregate rejection without partial mutation, and capacity recovery after replacement.

This is outgoing custody only. Cold/repopulated destination restoration, standalone cached restoration, and destination-origin/auxiliary-context reconciliation remain open. No build or test executed; no parent requirement closed.

### Cold root name restoration before author execution

The root constructor now chooses the saved destination name for a repopulated traversal and the incumbent name for a fresh load/reload. It compares the original history entry's strict origin with the committed destination origin. A changed origin clears the name unless the parent reports that the current root still has an opener; this query occurs after policy/COOP commitment. Standalone roots use their available strict-origin history identity and have no parent-reported auxiliary exception.

The chosen name is carried in an internal initialization resource, removed by the root constructor after history installation and before parser execution. Host setters now publish root names through the exact parent document authority and validate the returned value before updating the local cache; frame names continue to use native frame binding custody. Missing saved names leave initial parent state intact. Source inspection corrected the initialization insertion point to the root constructor, excluding the earlier static-frame readiness site.

Regression sources cover pre-parser initialization and oversize rejection, strict tuple/opaque origin comparison and opener exception, and a standalone cold-history sequence with same-origin latest-name propagation, foreign clearing, restoration after cache eviction, and a redirected destination reset. These regressions remain unrun. Standalone cached-name restoration and root retained departure lifecycle integration remain open, as does full native-authority/opaque-origin caller reconciliation. This does not close R-04.

### Standalone cached name application

Cached authority restoration now applies the saved strict-origin group's name to standalone roots before queued history/pageshow publication. It updates native binding storage while the isolate remains exited: no V8 slot lookup, realm entry, or authored getter runs. Parent-backed roots continue through the authenticated cached-document token path; the standalone wrapper explicitly rejects parent-backed contexts. Rollback uses the same cached authority helper, so it restores the incumbent's captured name as well.

New unexecuted regression sources cover same-origin latest-name propagation into an older cached document, pageshow observing that name while the original global survives, restoration of the foreign group's own name, and isolated metadata update/oversize rejection while an isolate is exited. Root retained lifecycle integration and full origin/authority caller reconciliation remain open. No build, test, or conformance execution occurred.

### Parent opaque name separation regression

Added a parent-authority regression using two independently committed identical data URLs. Their serialized origins are both opaque, but their native security token keys differ. The test requires the second commitment to clear the first name and verifies both cached restores preserve their own saved name after the other document is renamed. This exercises the actual parent security/name/cache routes rather than only the standalone history helper. Source generated only; unrun.
