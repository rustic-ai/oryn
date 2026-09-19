# Native prerender Core dependency — source generation

No builds, tests, services or generators have run for this group. `HTMLScriptElement.supports("speculationrules")` remains false until the native loading and activation path is integrated; parsing alone is not support.

## Pinned callers

The policy assigns `html` to Core. These source paths were inspected in WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`:

- `html/browsers/browsing-the-web/history-traversal/pagereveal/order-in-prerender-activation.https.html`
- Its `resources/order-in-prerender-activation-popup.html` helper.
- `html/semantics/document-metadata/the-base-element/crashtests/base-with-speculation-rules-onerror.html`.

The first fixture unconditionally asserts speculation-rule support. Its helper inserts an inline list rule, loads the candidate without changing the initiator, requires `document.prerendering` to be true during candidate execution, uses native fetch channels before activation, and navigates with `location.replace` to activate the already loaded Document. It requires retained globals and event order `pageshow,prerenderingchange,pagereveal,raf`. The base-element crash source requires malformed top-level JSON to fail safely while error handling mutates the base element.

These are direct/helper source counts, not manifest counts or a changed Core denominator. A literal scan of the pinned html sources found these three consumers; complete external-helper closure remains part of source review.

## Required production integration

- Bounded rule parsing, original script/document ownership and removal cancellation.
- Native nonactive document root with independent generation/epoch, policy and resource lifetime. It must run parser/script/network tasks while rendering remains suppressed.
- Admission constrained by native URL, origin, referrer, target hint and supported requirements; invalid or ineligible rules cannot become unconditional loads.
- Activation selection by exact candidate and current initiating document, atomic incumbent parking/candidate promotion, retained WindowProxy/global identity and accepted response ownership.
- Exact cancellation on script removal, initiator replacement, failed policy/network admission, capacity and shutdown. No abandoned resource clients or author callback replay.
- Document.prerendering and prerenderingchange publication; retained pageshow must precede activation change, reveal and rAF.
- Native authority self-message routing, child tree and pending worker/resource ownership must follow the selected nonactive root, without changing unrelated current roots.

## Generated source evidence

`oryn-js::speculation_rules` now parses bounded native list rules with relative URL bases, HTTP(S) filtering, eagerness, target hints, referrer policy, tags and explicit anonymous-IP requirements. Individual invalid rules preserve valid siblings; invalid top-level JSON and total capacity failures are distinct. Native regression sources cover base choice, policy metadata and invalid rules. Unsupported document-rule sources are explicitly rejected in this component; no whole-language completion is claimed.

Binding registration, nonactive root authority/loading, activation and cancellation remain in generation. The implementation will reuse the existing native parser/resource pipeline and retained pageshow rendering gate, rather than reload a candidate or treat rule parsing as activation.

The binding rule registry now owns original script leases, document/stream/authority identity and nonconsuming candidate snapshots. Removal/adoption/document replacement removes candidates from the authoritative snapshot, allowing native loader cancellation; repeated registration preserves candidate identity. Parser/dynamic preparation branches are being integrated by the script lane. Rule registry regression sources cover identity, removal and replacement.

The native message hub now has bounded nonactive root reservation and atomic loaded-candidate promotion primitives. They keep the incumbent current until promotion, preserve its native parked rollback capability and reject stale/foreign/unfinished candidate promotion. Page retirement disposes candidate message custody. These are foundation primitives with generated regression sources; loader policy/resource scope, self-message/frame routing and actual actor activation consumers remain open and support is still disabled.

Parent nonactive policy primitives are now generated in `prerender_policy.rs`: reservation allocates monotonic generation/security identity without changing the current epoch; accepted same-origin response headers commit to a private candidate policy. Atomic native creator/cookie URL admission and worker-policy lookup select that exact candidate scope only after response policy commitment. Existing active document policy and URL remain unchanged. Native regression sources cover independent CSP, unchanged active root and stale initiator rejection. No actor loader consumes these primitives yet; promotion policy publication, candidate cleanup, scoped messaging/child tree and parser/activation consumers remain open.


### Cancellation and preparation policy source reconciliation

Private candidate cancellation now revokes native admission before resource cleanup, retaining transaction/socket/service/blob/worker cleanup stages across failures outside registry locks. The existing context response cleanup checkpoint retries these stages. Lost reservation responses transfer disposal into `ResponseCleanup::Prerender`; typed Reserve/CommitResponse/Cancel endpoints are native-only. Dedicated authenticated candidate capabilities support bounded disposal replay after consumption, with full tags, page binding and checked sequence exhaustion. Disposal never proves activation. Generated regressions cover failed suffixes, revocation, retry, foreign/tampered capabilities and capability-domain separation.

Inline speculation-rule CSP now carries an explicit native kind through dynamic bindings, parent authorization and shared parser/standalone captured-policy authorization. `'inline-speculation-rules'` permits only inline rule data; ordinary scripts, external resources, other policy intersections and report-only reporting retain their existing behavior. Regression sources cover nonce/strict-dynamic interaction, default/directive fallback, reporting, stale creator denial and actual dynamic preparation kind dispatch.

Still open: actual nonactive parser loading, private child/message routing, parent activation policy commit, retained same-Document activation and event publication. Typed reservation transport and authority foundations do not close those requirements or enable supports(). No execution has occurred.


The shared host can now construct an independent native root context without detaching the incumbent WindowProxy or publishing it as the page's current navigation root. Native authority epoch and `Document.prerendering` initialize before transport/provenance admission; Document.hidden is true and the shared rendering gate suppresses candidate rAF/paint. Candidate construction bypasses active-page URL/referrer commits, and private metadata/URL policies preserve the incumbent. Stable opaque sandbox origins are retained through response retries. Original response policy remains separate from later meta policies so a replay cannot discard parser changes.

A bounded IO reservation owner now retains cancellation across the extra thread-to-actor handoff after IPC has accepted the response. Its cleanup ticket is reserved before IO; late success after receiver drop attaches the exact token before disposal. CancelPrerender is routed through retained revoked-route cleanup authority and exact ACK handling, and keeps the cleanup route retained while candidates exist. Regression sources cover late handoff disposal and native reservation response custody.

The actual candidate HTTP/parser scheduler and promotion consumers remain unfinished. The new host factory and reservation owner are integration foundations, not evidence that the pinned prerender fixture now runs.


### Actor scheduling integration (source only)

`prerender_roster.rs` now reconciles exact root/document/stream/authority and rule snapshots from the executable. It reserves bounded native candidates, drives asynchronous reservation, HTTP/MIME/policy phases and the actual detached parser driver, returning the shared isolate host after every turn. Each callback turn rechecks source/rule ownership. Failed records do not repeatedly issue network requests; disconnected/replaced sources dispose their drivers and retain native-root retirement retries. Loaded candidates do not add perpetual timer ticks; shared host task queues continue native callbacks. The actor retains native generation high-water marks and reports optional-load diagnostics separately from active navigation.

`MarkPrerenderLoaded` is a native-only, exact-token milestone with idempotent ACK; it requires committed response policy and current source authority and never promotes a candidate. `promote_prerender_after_departure` accepts the exact incumbent token already owned by retained departure, preventing a second park/callback cycle. Its preflight preserves both owners on rejection and rejects a newer active root. Native regression sources cover prepared/load prerequisites, replay, cancellation and parked-source rollback.

`NativePrerenderActivation` prepares a retained trusted Document event before promotion, changes the flag only when polled after promotion, and keeps reveal blocked until its callback suffix completes. Cancellation is distinct from successful completion and does not release that gate. This is an activation publication component, not complete activation integration.

Still required: selecting and atomically promoting the candidate in actual navigation transactions, parent policy/security publication with rollback/ACK custody, WindowProxy adoption, private message/child routing, trigger/target-hint producers beyond eager same-context candidates, and full pinned actor fixture regressions. `supports("speculationrules")` remains false; the broad source gate is open. No execution qualification is claimed.


### Activation identity source decision

Reviewed the public [WICG prerendering specification](https://wicg.github.io/nav-speculation/prerendering.html), section 2.3 (retrieved 2026-09-17). Its activation algorithm moves the successor session-history entry **including its Document’s browsing context**, explicitly requiring a browsing-context-group switch and severing opener relationships, including traversal back across that boundary. Thus the candidate must retain its independent native WindowProxy; ordinary BFCache same-proxy reattachment is not the activation operation. Parent candidate scope now preallocates an independent group identity and keeps its policy/origin with the candidate. Actual publication and cross-group history restoration remain required production work.

The parent activation primitive now atomically publishes the candidate lifetime, strict origin, live policy, referrer/isolation, independent browsing group and URL, severing the opener. It accepts only the exact loaded candidate and already-parked predecessor. Completion records preserve exact token/predecessor identity, return current versus obsolete ACKs, and are bounded to 64 per page; replay never republishes native effects. The host root-map publication keeps the candidate proxy attached, and roster selection consumes the original matching source record before departure. Native source regressions cover activation replay, group ownership and disposal.

**Historical checkpoint, superseded by the actor integration below:** activation transport was left unwired until its retained recovery owner existed. A generic asynchronous policy request is insufficient: after a lost ACK the parent may already have promoted the candidate. The actor must retain candidate+predecessor through retry/installation or exact rollback, rather than drop the response and leave native authority without an executable. Cross-group BFCache proxy restoration and private child/message routing also remain implementation requirements.


## Retained actor activation and cancellation source integration

`navigation_actor.rs` now selects the exact loaded roster candidate after native navigation admission for eligible Push/Replace requests. `prerender_commit.rs` retains departure callbacks, the original history/source lease, the immutable asynchronous activation request, history publication, and the complete prerenderingchange tree. Pageswap uses the candidate's pre-reserved generation. The existing candidate executable and WindowProxy are installed; its already-delivered pageshow is not repeated. Ordinary publication follows activation, with native Committed preceding Interactive/Complete. Rendering remains gated until the activation tree completes.

Cancellation transfers the whole job into actor recovery. An uncertain Activate reply is resolved by identical retry before exact parent root retirement, native root disposal, and predecessor restoration. Lost retirement acknowledgments retain the exact generation/epoch. A subsequent request waits for recovery and captures source authority afterward. A newer Document/stream in the already-published destination is excluded from old activation disposal. Candidate policy/origin/request broker are carried into actor installation rather than recomputed from the predecessor.

Generated actor regression sources cover original Document/global identity, one network load, one pageshow, pageshow → prerenderingchange → pagereveal → rAF, cross-group back restoration, cancel after native promotion but before local publication, and a new request waiting for recovery. These sources have **not been built or run**. The pre-existing lost-ACK transport regression complements the actor cancellation case.

Remaining source requirements: combined source review of lifecycle/authority replacement and teardown; private child frame admission/policy/message routing; activationStart and deferred API postactivation inventory; complete transitive trigger/target/redirect scope and pinned actor fixtures. The feature and broad checklist gates remain open; `supports("speculationrules")` is still false.


Private descendant source integration now includes exact native parent registration, inherited and HTTP/local response policy tickets under the candidate root epoch, child creator/origin/storage lookup, scoped message projection, metadata URL/CSP/referrer updates, and exact child retirement. Promotion moves the same private frame registrations into the active table. Candidate cancellation retains separate owner/stage cursors for children and root. Native regression sources cover hidden child authority, old-lifetime rejection and promotion preserving committed child origin. Full host/parser/actor descendant fixture and remaining cross-caller authority review are still open; this does not close the private-frame or feature gate.


Early same-context navigation now retains a `PrerenderWaiting` actor phase while the original candidate reserves/fetches/parses, avoiding duplicate fetch and repeated beforeunload. Rule removal/failure resumes the ordinary resource continuation after the already-completed admission prefix. Regression sources cover waiting and fallback ordering. The later created-Document readiness integration below replaces the initial completed-load restriction. Private origin-agent-cluster/domain/shared-memory group lookup now selects the independent candidate group, with private aliases included in native domain commit validation.

### Created-Document activation and retained load continuation

Source generation now separates `MarkPrerenderReady` (native candidate Document created under its committed response policy) from the existing actual loaded milestone. Native promotion requires ready authority rather than EOF. The actor moves the original executable/parser/body capture into the visible page, publishes activation against that Document, continues the original parser, and retains its single pageshow until actual load if it did not run privately. Interactive publication precedes completed load, and its cursor prevents duplicate Interactive emission in the terminal actor publication. Response status and completed-body trace/challenge processing follow the candidate response, not a synthetic 200. Failed rule records are scoped to the original source identity; a reused rule ID in a new document is eligible again. Frame response-policy admission now rejects worker creators before allocating any ticket.

Generated regression sources cover a held response body activating before EOF and completing through the same Document with one fetch/one pageshow; original non-200 response status in actor publication; foreign/stale ready authority; source-reused failed rule identities; and worker-creator rejection without prepared-policy leaks. Cancellation snapshots now include exact children created during activation callbacks before issuing retirement. All are unexecuted. Remaining combined source obligations include interrupted pre-install history callbacks that replace the document, early-load failure/stop outcomes, render/activationStart and deferred APIs, private frame/message caller breadth, and the complete trigger/target/redirect matrix. Feature support remains unclaimed.

The pre-install successor recovery path is now generated: after committed parent/root-map promotion, a newer authored stream in the same authorized candidate cancels only the obsolete history-publication receipt, prepares activation against the current stream, and retains installation/activation independently of the canceled caller. It preserves the destination and source cache instead of restoring the predecessor. Original response capture is discarded for that newer stream. A source regression injects document.open/close after history publication and before installation, cancels the old caller, and checks destination authority, live Document identity and history retention. Further retirement/authority-error and repeated replacement scenarios remain part of combined source review.

## Final feature-detection integration — September 18

Earlier statements that `supports("speculationrules")` stays false are superseded after native parser/actor loading, retained promotion, blank activation, private policy and lifecycle integration were delivered. The static native callback now recognizes the exact `speculationrules` token alongside classic/module/importmap. Regression source `feature_detection_reaches_native_speculation_registration` performs author-visible feature detection with one DOMString conversion, rejects case/whitespace variants and registers an actual native candidate through the conditional path. Existing native actor/blank activation regressions remain required; this change is source generation, not runtime qualification.
