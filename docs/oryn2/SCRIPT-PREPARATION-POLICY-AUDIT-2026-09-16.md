# Script preparation policy source audit

Source-only checkpoint. No builds, tests, generators or conformance runs were performed.

## Required behavior and generated native paths

| Obligation | Current source evidence | Remaining evidence |
| --- | --- | --- |
| Preserve policy at preparation | `CaptureScriptPolicy` authenticates current Window Creator and stores native AgentPolicy in `ScriptPolicySnapshots`. | Complete parser/dynamic producer review. |
| Keep identity and bounds | Registry-qualified tokens, owner/generation binding, 1024 entries and 4 MiB serialized policy budget; unchanged snapshots deduplicate. | Integration resource qualification after generation. |
| Preserve cached lifetimes | Capture-time pruning uses retained authority; use requires active Creator. | Review complete cache/service replacement topology. |
| Apply snapshot throughout loading | LoadScript/LoadTypedModule carry token through lease/direct dispatch; loader resolves captured CSP before redirects. Origin and live identity checks still use current native authority. | Full module/dependency/cache caller review. |
| Preserve inline checks and reporting | Inline gate uses snapshot nonce/hash policy; reports retain original policy/disposition/Creator. | All parser/inserted/standalone inline callers. |
| Preserve parser provenance | Native requests carry parser_inserted; CSP strict-dynamic blocks parser roots from falling through to host/self matches. | Producer true for parser roots, false for dependencies/dynamic; standalone review. |
| Keep ordinary fetch policy separate | Generic HTTP stream path reads current policy; captured lookup belongs only to script loader. | Broader network-policy caller audit. |

## Regression sources

`script_policy_snapshot.rs` covers immutable capture, changed snapshots, owner and registry mismatch, single-registry capacity, deduplication at capacity, pruning recovery, oversized policy rejection and retirement.

`context_service.rs` adds native inline/meta, redirect/parser provenance, report-only metadata, stale-owner, service retirement and parked-owner/successor/eviction cases. These are generated test sources, not passing results.

`csp.rs::strict_dynamic_discards_hosts_only_for_script_like_requests` distinguishes parser roots, nonce authorization, non-parser requests and style/image/connect matching. The pinned upstream reference is `content-security-policy/script-src/script-src-strict_dynamic_discard_source_expressions.html` at WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`.

## Integration gate still open

Review parser/snapshot/frame/dynamic preparation, deferred inline execution, module roots and dependencies, typed modules, deduplication keys, standalone policy snapshots and protocol request literal coverage together. Source storage and native authorization alone do not close L or the global generation gate. Complete this integration before any build or execution.

## L-02.j import-map caller reconciliation (2026-09-17, source only)

Pinned revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`, direct Core `import-maps/multiple-import-maps/*`, `not-overridden/*`, and module `error-type-{1,2,3}.html` establish the current generation targets.

Generated changes:

- Shared map implementation now lives in `oryn-js/src/import_maps.rs`; `oryn-native/src/import_maps.rs` reexports that exact type. Existing normalization, scopes, prefixes, null blocking, merge precedence, resolved-set bounds and integrity tests move intact, with no duplicate resolver.
- Connected dynamic import maps install synchronously in post-connection script preparation after CSP authorization; already-started prevents reinsertion from installing a replacement. The same context slot feeds static/dynamic import and import.meta.resolve. A binding-held map reference ensures retirement clears maps first installed before native module-context tracking.
- Parser and dynamic invalid JSON/shape report global SyntaxError/TypeError instead of treating map source as JavaScript or throwing from appendChild. External import maps queue an element error; no external map fetch is introduced.
- Native graph resolution records successful resolution before fetch/link completion. Failed resolution remains eligible for later maps. Source-table graph fallback no longer bypasses null/invalid map entries; linking owns the original resolution error.
- Regression sources cover synchronous dynamic merge, resolved URL and failed bare-specifier distinction, reinsertion, parser/dynamic global errors, later valid map recovery, and graph resolution frozen before network completion.

Remaining implementation review: module graph parse failures currently reach frame/parser element-error handling despite required global parse-error plus load semantics; shared retained parse-error custody and parse-before-instantiation precedence are being generated. Review typed-module and dynamic/static error propagation, delayed evaluation reporting, all load-event suffix owners, and import-map infrastructure failure custody before closing L-02.j. Nothing in this section reports executed validation or closes the broad script gate.

### Module parse/fetch classification and dynamic completion custody

Generated production integration now retains native module parse exceptions as bounded context-owned receipts. Each graph captures its own receipt; parser, dynamic-script, direct-module, dynamic-import, worker and AudioWorklet callers pass that exact receipt into prepared execution. The provisional URL/source alias lookup was replaced: identical inline preparations retain distinct root exception objects, while imported external module parse errors preserve document module-map identity. Discovery chooses the first reachable parse error in requested-module depth-first order rather than network completion order. Native receipt lookup fails closed after retirement; clearing the context does not reuse its receipt counter.

A graph with a cached or newly failed fetch returns fetch failure, including cache publication after discovery and shared-fetch followers. Successfully fetched modules with parse errors proceed to execution's original global error reporting, before instantiation and without running the module body. Regression sources cover parse-before-instantiation precedence, original exception identity, identical inline distinction, global error followed by external element load, and sticky fetch failure classification.

Dynamic-script execution now retains the current decoded result, preparation owner, execution-start marker, source reservation and native load/error receipt. A preparation failure retries before execution; once the body starts it is not replayed by a completion retry. Pending completion uses a 10 ms retry deadline, while a parked owner contributes no runnable deadline. Source retirement/adoption cancels its event receipt. Successful inline modules do not emit element load, matching `module/load-error-events-inline.html`; failed graph fetches still emit element error. Regression sources cover post-body failure recovery and inline suppression.

Still open: independently review native global module/import-map error-report callbacks under mid-report infrastructure failure/termination, all typed-module parsing/error-value cases, and final graph receipt reclamation/admission pressure. These generated paths and tests have not been built or executed; L-02.j remains open until that source reconciliation and later execution complete.

### Retained global ErrorEvent delivery

The shared script exception reporter now owns the original native exception/message before allocating ErrorEvent or preparing its event path. Interrupted listener delivery, property handling, suspension and worker forwarding resume from retained progress. Reports are bounded to 128 pending jobs with a 10 ms retry deadline; parked owners have no runnable deadline, and retirement discards obsolete publication. Deferred uncanceled reports feed the native diagnostic drain only after dispatch completes. Original `window.onerror` five-argument invocation and true-return cancellation now also apply in the retained notification engine; beforeunload result handling is preserved.

Regression sources cover a once listener interrupted before the remaining listener and legacy handler, exact event/error object identity, five handler arguments, cancellation, and suspension without a zero-delay loop. Original exception custody on pre-event preparation failure is in production source. The broad gate still requires independent source review and the deferred full build/test/WPT phase; no runtime result is claimed.


### L-02.j follow-up: prepared error ownership and typed precedence (source only)

Graph parse-error tokens now have graph-owned RAII leases. Cancellation/drop releases receipt roots; inline parse roots are removed at receipt creation, while external parse failures remain in the native module map. Completed parser/frame/dynamic graphs remain owned until execution consumes their token. Generated cancellation regression distinguishes inline cleanup from sticky external parse identity. This supersedes the earlier open receipt-reclamation note; execution remains deferred.

The bounded ErrorEvent queue now rejects overflow before listener dispatch, returns an allocation error, and retains one native exception/message plus the first detail and saturated rejection count. Host diagnostics drain this explicit rejected-admission record; no emergency unbounded queue or successful-delivery claim is made. Generated saturation/recovery/retirement regression complements the existing interrupted-listener suffix regression.

Graph traversal keys now include module type. Prepared JSON responses are parsed before ECMAScript linking, with original cached SyntaxError objects leased into graph error receipts; requested-module DFS selects the error independent of response order. CSS remains forgiving native stylesheet parsing (no invented CSS syntax exception). Generated JSON-before-link-error and JSON primitive/null/array value regressions cover the concrete additional cases. Full L-02.j acceptance remains open for independent source review and deferred build/native/WPT execution.

Typed synthetic-module values now use exact retained V8 module identity plus environment, replacing hash-only authority; environment teardown removes the corresponding retained records. Admission reserves value custody before synthetic module publication. Existing frame-retirement and new multi-value module regression sources cover this path. Other legacy module-name/credential hash maps are not silently claimed collision-safe by this bounded change.


### L-02.j exact module/environment identity reconciliation

This supersedes the preceding remaining hash-map note. JavaScript module cache keys now use the authenticated native `EnvironmentId` and requested URL, replacing global-object identity hashes. Names, credentials, owners and evaluation promises use retained V8 Module handles as HashMap keys: V8 identity hashes select buckets only, and exact V8 equality resolves collisions. Synthetic typed exports already use exact module/environment records. Promise-rejection diagnostics now use exact retained Promise keys and owner environments, preventing one handled rejection from removing a colliding unhandled rejection.

Environment retirement removes module cache entries directly by native environment, including requested/response URL aliases, then removes exact metadata/evaluation/value/rejection records. Resolution and evaluation require the current native environment to own the compiled module. Prepared error receipts remain context-owned monotonic IDs and graph-owned leases; they are not V8 hash identities. Immutable import.meta data is read from the exact module record.

Source regressions added: forced constant-hash Module/Promise key buckets retain independent records and removal; equal requested URL in root/frame executes distinct records with correct import.meta; retiring a frame releases only its rejected promise diagnostics. Existing interrupted Evaluate cancellation, pending TLA frame retirement, typed record retirement and graph-lease cancellation regressions remain the native lifetime evidence. No test has run.

| Entry path | Shared native identity and continuation source | Source status |
|---|---|---|
| Root/frame parser module (blocking/defer/async) | ModuleGraphLoad → ParserScriptFetch/FrameScriptJob → prepared host execution | Exact environment cache, graph lease, execution marker and retained element event generated |
| Connected dynamic module element | DynamicScriptScheduler retained execution + prepared_graphs → host | Same identity; body/completion progress retained; inline load suppressed |
| Direct native module call | execute_module_inner_with_started → shared resolution/evaluation | Exact cache and evaluation records; interruption removes orphan forwarding record |
| import() from classic/module/import.meta resolution | PendingImport graph → load_dynamic_import_prepared → shared module map | Captured root and graph error; exact evaluation Promise reused |
| Dedicated/shared/service module worker startup | agent_runtime_v8 graph handoff + agent_compile_module/agent_resolve_module | Native environment cache, exact response-name/credential/owner records |
| AudioWorklet initial/later addModule | Agent startup machinery and shared graph error handoff | Same exact records; later failure retains existing worklet registry (resource lane integration) |
| JSON/CSS static or dynamic dependency | typed response cache → TypedModules → synthetic evaluation | URL+type+environment cache; exact Module/value; JSON error-before-link; CSS forgiving parse |
| All script/module global errors | retained global_error_reporting + notification continuation | Original exception, bounded rejected-admission diagnostic, exact callback suffix |

Remaining L-02.j requirements are independent review of the complete caller handoff (especially acceptance before promise-reaction allocation), pinned fixture reconciliation beyond these bounded corrections, and the globally deferred build/native/WPT execution. The table documents generated source; it is not a conformance pass claim.

### L-02.j retained execution/reporting and post-connection closure

Generated source now pre-admits module error reporting before compilation/evaluation. `module_reporting.rs` retains the original evaluated Promise or parse exception, prepared handler/resolver, owner environment/authority and retry deadline. Failed rejection-handler installation resumes without repeating body execution. Parked work contributes no runnable deadline; retirement releases admission. Parser and dynamic element completion wait for pending installation and ErrorEvent suffix delivery. V8's actual evaluation Promise replaces the redundant forwarding Promise; the superseded post-Evaluate forwarding helper is removed. The linked V8 150 API implementation uses `PerformPromiseThen` directly for Promise::Then/Catch, explicitly avoiding authored `then` and species lookup.

Dynamic import completion now retains graph completion, loaded Promise, outer success/failure handlers and settlement marker. Infrastructure retry preserves the original resolver/graph lease and cannot repeat a completed attachment or body. Native worker import propagates an empty infrastructure result for retry rather than synthesizing a script TypeError. Generated regressions cover interrupted settlement, original parse exception identity and deferred scheduling readiness.

Connected script preparation now stages snapshots and bounded queue admission before marking external/module scripts already-started. Script error task admission similarly reserves before marking. Shared post-connection batches retain tracked target leases and unfinished suffixes across failures; nested insertion batches still run synchronously. Target ownership is refreshed after adoption; retired same-document generations are discarded. Classic execution marks started only at accepted compile/run; import-map registration exposes the corresponding commit/report acceptance hook for parser and dynamic callers. Regression sources cover pre-admission failure, interrupted error-listener suffix, nested insertion ordering and retirement cleanup. Frame/image/stylesheet preparation that precedes post-connection script batch construction remains in the broader native loading ownership audit.

JSON SyntaxError source location now comes from bounded private native exception metadata registered with the verified response URL. ErrorEvent construction ignores authored `fileName`/`stack`. This implements the explicit pinned `json-module/parse-error-location.tentative.html` filename expectation; repeated report/properties-tampered source regression is generated.

### L-02.j deferred import entry path

Pinned direct-Core `module/dynamic-import/basic-deferred-evaluation.tentative.html`, `tla-deferred-evaluation.tentative.html` and `cyclic-tla-defer-eval.tentative.html` require the host to preserve import phases. Source now installs V8's phase-aware dynamic callback, captures phase in the retained import owner, and uses V8's `EvaluateForImportDefer` plus deferred namespace API. Eager and deferred evaluation capabilities have separate exact-module/environment caches. This allows asynchronous transitive dependencies to start while entrypoint evaluation remains deferred until namespace access, including the specified cyclic-TLA deadlock. Window and worker import completion share this path. Generated regressions mirror all three fixtures.

Runtime and startup-snapshot generation now share `V8_RUNTIME_FLAGS` enabling the pinned staged deferred-import feature. Snapshot/profile regeneration is an execution-phase prerequisite; it has not been run. JavaScript/JSON/CSS source-representation imports reject with SyntaxError; no Wasm source-module implementation is claimed here.

Concrete next generation requirement (still open): complete native `<link rel=modulepreload>` and Link-header modulepreload admission, module-map reuse, frozen policy/referrer/integrity and retained load/error publication. Pinned `import-maps/acquiring/modulepreload*`, `not-overridden/modulepreload-header.html` and `module/modulepreload-referrerpolicy.html` exercise that path. Existing native `preload_resources.rs` covers image preload only; legacy runtime source scanning is not sufficient implementation evidence. Independent source review and all execution remain deferred; this open requirement is code generation, not just validation.

## Modulepreload and text modules — generated source reconciliation (2026-09-17)

This section supersedes the earlier statement that modulepreload is an unimplemented native entry path. It records generated code and source review, **not a build, test run, or conformance result**.

| Required entry / dependency | Generated production path and custody |
| --- | --- |
| Parser `<link rel=modulepreload>` | Native `ModulePreloadDiscovered` event, retained parser event queue, HTML and XML/snapshot preparation callers. Discovery precedes following script admission and excludes disconnected template content. |
| Dynamic insertion and relevant attribute changes | Native binding hook captures URL/base, destination type, credentials, referrer policy, integrity, internal CSP nonce and parser flag at admission. `relList` uses a private native attribute identity, live indexed tokens and normal mutation/reaction paths; it is separate from `classList`. |
| Actual document `Link` response headers | Bounded quoted-delimiter-aware parsing; root response metadata and both static/dynamic frame consumers. `PageResource.response_headers` carries actual parent/standalone response headers with retained-byte accounting. Inherited about:blank policy installation does not replay parent Link headers. The authored compatibility bridge does not admit `LoadPageResource`. |
| Preload fetch and module-map admission | Existing authenticated graph backend, shared in-flight response registry and per-document budgets. JavaScript root is compiled/cached without instantiation/evaluation; JSON/CSS/text prepare native typed records. Repeated requests share the first module-map result. Descendant prefetch failure does not turn a successfully fetched root into a link error. |
| Load/error and lifecycle | Fetched parse errors produce link `load`, not a global ErrorEvent; root fetch failure produces `error`. Native retained publication receipt owns listener suffix. Realm/authority/load-cancellation checks, parked scheduling and retirement release apply. Publication cannot retry forever: separate bounded publication deadline cancels its receipt once and exposes native failure with the bounded first infrastructure error; it does not fabricate successful delivery. Parser encoding rollback also captures/prevents baseline preload advancement in the resource lane. |
| Integrity and CSP | `HTMLLinkElement.integrity` IDL/reflection and internal nonce capture; typed parent fetch carries nonce too. `ScriptLoadFailure::Integrity` / worker wire provenance are separate from a sticky module-map fetch failure, so failed request integrity does not poison later imports without that requirement. |
| `as=text` / `with {type:'text'}` | Native `ModuleType::Text`, exact type+URL+environment identity, default synthetic string export, arbitrary representation MIME, UTF-8 decoding with one UTF-8 BOM removed. Window, dedicated/shared/module worker graph and parent-service import routes share the implementation; decoded text is never compiled as JavaScript. Persisted import metadata checks the exact import kind before reuse. |

Pinned callers reviewed include `preload/modulepreload.html`, `modulepreload-{json,style,text,multiple,sri,sri-importmap,already-in-module-map,consumed-by-script}.html`, the `avoid-delaying-onload-link-modulepreload*.html` helpers, `import-maps/acquiring/modulepreload{,-link-header}.html`, `import-maps/not-overridden/modulepreload-header.html`, the module referrer-policy fixture, and `html/semantics/scripting-1/the-script-element/text-module/{charset-2.html,charset-bom.any.js,ignore-extension.any.js,repeated-imports.any.js,load-error-events.html}`. Required transitive helper behavior includes a real `relList.supports('modulepreload')` and native link integrity reflection.

Regression sources cover parser ordering/inert templates; native preload compile-without-evaluate and later cache reuse; parse-error load semantics; typed text identity; quoted header parameters; duplicate header admission; parked ownership; prepublication failure recovery; fetch expiry producing link error after a preparation pause releases; terminal publication deadline cleanup without synthetic events; native relList triggering; nonsticky integrity failure; text UTF-8/BOM behavior; separate JSON/text identity; and worker startup graph text delivery/cancellation.

Pending acceptance work is global execution plus independent source review of the completed entry-path integration. The append-only Text/Integrity wire vocabulary, `LoadTypedModule.nonce`, and `PageResource.response_headers` must be included in the final combined protocol/profile reconciliation before execution. No generator or profile build has been run. The broader L-02.j parent remains open until its other entries and independent review are reconciled; this section does not claim full Core conformance.


Coordinator review correction: preload preparation pauses are checked before deadline, graph polling and event publication, matching the delay calculation. Fetch expiry cancels graph work and enters the ordinary link-error publication stage with a separate ten-second publication deadline. Publication expiry cancels its retained receipt and returns the first native infrastructure failure once. Thus a paused baseline cannot advance or time out under a provisional import map, while an active fetch timeout still produces its required link error. The dedicated generated regression expires a paused baseline fetch, checks no work/callback before release, then verifies exactly one error afterward.

### L-02.j authored import resolver custody (source review correction)

The earlier “outer success/failure handlers” description is superseded. Review found that three native Promise forwarding callbacks ignored `resolve`/`reject` failure after graph ownership had been released. The generated replacement removes those callbacks and the extra intermediate Promise. Both document and worker prepared imports now return the exact evaluation Promise plus namespace, or the original exception. A bounded isolate-owned settlement receipt forwards directly to the authored import resolver. Successful handler attachment transfers custody; original namespace/exception, environment and authority epoch remain rooted until accepted settlement, retirement, or an explicitly reported native publication deadline.

The evaluation observer runs in its original Promise reaction checkpoint. An interrupted resolver attempt retains its outcome and schedules a bounded retry without re-evaluating the module. Pending TLA has no arbitrary evaluation timeout or zero-delay runnable signal. Queue scanning filters waiting TLA before its per-turn bound, so early unresolved evaluations cannot starve later completed imports. Parked owners contribute only a publication deadline; retirement removes the receipt and V8 roots. Native resolver publication expires after 10 seconds with operation failure evidence, rather than claiming delivered JavaScript rejection. Original module exceptions remain JavaScript rejection values. The direct worker errored-module branch now shares checked resolver settlement.

Generated regressions in `module_settlement.rs`: callback failure retains namespace and single evaluation; original author exception identity after callback failure; more than 64 pending TLA jobs do not starve a later ready import; publication deadline releases once and reports native failure; parked custody has no immediate runnable wakeup and retirement releases roots. Existing dynamic-import pre-attachment failure and original parse-error regressions remain in place. The native host scheduler and environment retirement invoke the new receipt owner; superseded forwarding functions have been removed.

Source generation for this correction is complete and awaits independent review. No build, generator, unit test or WPT execution has occurred. Broader L-02 acceptance still requires the complete pinned entry-path review and global execution phase; this entry does not claim that all script conformance obligations are closed.

The same caller audit identified indefinite `ModuleReport` observer-install retry. Generated source now bounds installation to 10 seconds, including parked custody, reports one native operation failure on expiry, and releases admission without re-running the module or inventing an ErrorEvent. Accepted evaluation promises are marked handled before delayed observer attachment. A generated terminal-install regression asserts one body execution, no fabricated error callback, admission release and no repeated terminal failure. Worker Text imports also now have an interrupted native settlement regression through the actual agent graph/loader entry.

### L-02.j native preparation provenance and resolver termination correction

Pinned V8 150.4.0 source (`v8/src/api/api.cc`, `Promise::Resolver::Resolve`, and `v8/src/objects/objects.cc`, `JSPromise::Resolve`) sets `is_native_resolver_invoked` before reading `then`. A terminating getter can leave the Promise pending; retry returns true from the latch without completing it. The retained native owner now distinguishes pre-invocation suspension from empty results after invocation. The latter records terminal native failure, releases custody on host disposition, and never retries the author getter or reports successful settlement. Rejection forwarding also goes through this owner. This supersedes the earlier broad claim that every interrupted resolver invocation retries. Added actual getter-termination and deferred-module `then` export termination regression sources, in addition to pre-entry retry tests.

Typed preparation now retains private native `ScriptLoadFailure` provenance through synthetic module callbacks. Capacity, cancellation and infrastructure are not invented JavaScript TypeErrors and do not poison the URL/type module cache; ordinary fetch/integrity rejection and original JSON parse exceptions preserve their prior authored semantics. Document and worker dynamic-import handoffs consume that provenance. Empty native compile/link/evaluate returns are tagged at the failing operation rather than inferred from exception message text. Dynamic graph infrastructure errors retain the graph; graph/capacity deadlines terminate with native failure evidence. Completed preparation retries have a separate 10-second publication limit. Terminal capacity releases the original import admission; subsequent imports may retry the unpoisoned module map.

Direct native module preparation, preload typed compilation, and worker/service/worklet startup consume the same private provenance. Direct pending modules and agent startup retain completed graphs across pre-execution infrastructure failure, with retry deadlines; typed capacity is terminal. No accepted body is replayed for these retries. Generated source regressions cover dynamic typed retry without authored rejection, terminal capacity followed by a successful fresh import, and typed worker startup retaining its completed graph. All changes remain unexecuted and require independent source review plus the global build/test/WPT phase.

Retry scheduling retains the first native preparation cause in dynamic import, direct pending modules and agent startup. Recoverable failure does not escape a poll as a fatal worker/actor error; it returns pending and retries after 10 ms. Expiry reports the retained native cause once. The V8 module linking retry basis is also source-verified: `SourceTextModule::PrepareInstantiate` returns false for an empty native resolver callback, and `Module::Instantiate` calls `ResetGraph` and restores `kUnlinked` before returning. Successful retry therefore restarts native linking without repeating evaluated module bodies.

### L-02 parser-document identity (moving-between-documents caller reconciliation)

Pinned `moving-between-documents/resources/moving-between-documents-helper.js` distinguishes parser document, preparation document and execution document, including moving a still-open parser parent before its script is inserted and later moving it back. Source previously retained only preparation/execution identity; parser creation merely cleared force-async. Generated native script records now carry the original parser Document allocation identity. HTML and XML share the contextual sink producer. Dynamic preparation, retained parser event preparation, compatibility parser branches and final parser execution check that identity before author execution or resource publication. A valid moved parser script is consumed without fetch/load/error, so moving it back cannot revive the rejected preparation. Empty/non-executable preparations clear parser identity and restore force-async for later dynamic preparation.

Cross-arena adoption preserves a foreign parser identity; migrating its whole native Document remaps the identity with that document. DOM clone/import does not copy parser identity (only already-started), while internal arena snapshots remap their local parser document identities. Generated regressions cover classic/module/empty-src move-back suppression, empty-script later preparation state, clone/arena snapshot behavior and actual cross-arena adoption. Parser chunk-equivalence comparison now normalizes the arena component of native identities. This is bounded source evidence; the complete L-02 insertion/order/import-map/consumer matrix remains under reconciliation and no tests ran.

Adoption cleanup review also replaced the raw `ParserScriptLease` arena/NodeId destructor with the shared `TrackedNodeLease`. This covers parser roots, frame script jobs, and stylesheet consumers of the same lease. Publication authority remains captured separately; cleanup alone follows the adopted node. A generated native regression moves a queued script across arenas, releases migration/mutation roots, drops the queue lease while the destination is borrowed, and verifies deferred lease drainage releases the destination allocation. The superseded ignored `release_native` error path is removed.

Prepared execution also now resolves the tracked current script allocation after validating the captured preparation realm/Document/stream. This permits an adopted script moved back to its original preparation Document before completion, even when cross-arena migration changed its local NodeId, while rejecting execution in the foreign document. Dynamic script native completion uses `current_node`; parser records share a tracked target through fetch/graph clones and use that target for execution, error and load publication. A generated binding regression performs an actual two-way native migration and asserts foreign rejection, original-document acceptance with a changed NodeId, and retirement rejection.

### Independent normal thenable settlement source review

Reviewed `ModuleSettlement` source custody, epoch checks, ready-job filtering, terminal entered-resolver handling, and the host polling path. The normal exported-then case now has a regression source complementary to the termination cases: inject a pre-entry interruption, retain the same namespace and authored import resolver, then require exactly one module evaluation and one `then` invocation with the namespace receiver. Its first resolution must win over later rejection/resolution attempts and preserve the resolved object identity. This checks that removing the intermediate Promise did not introduce duplicate assimilation.

No test executed. This bounded review does not close complete module preparation caller/status rollback or L-02 conformance.


### Preparation owner versus execution target during adoption (source generated)

Admitted parser and dynamic script I/O now retains the original live preparation
realm/document/stream and frozen fetch context while a tracked script temporarily
moves to another document. Async readiness, source completion, dependency graph
polling, and compatibility EOF preparation use that owner guard; they do not
reacquire the adopted document's authority or discard a pending fetch solely due
to its target moving. Execution and element load/error publication separately
validate the tracked current node against the original preparation Document.
The load tail repeats that check after authored execution and retires an existing
publication receipt when adoption/replacement invalidates its target.

Generated regression sources cover held native network I/O polled while the
script is foreign, return to its original Document before completion (one body),
remaining foreign at completion (no body), original frozen initiator reuse,
post-body adoption suppressing old-document load, and dynamic prepared-owner
retirement. These extend the pinned moving-between-documents caller inventory;
no builds, tests, generators, or conformance runs have executed. Broader L-02
script-entry and cross-environment acceptance remains open.


### Async parser scheduler transfer keeps execution continuation (source generated)

The compatibility parser scheduler transferred to live root/frame EOF work still
executed completed async scripts with temporary progress. It now queues bounded
`ParserAsyncExecution` owners for successful classic source completion and module
graph completion. Each owner retains `ParserScriptProgress` and its graph error
lease through reporting/load publication. Failed completion admission requeues
the same owner rather than replaying its body. Scheduler readiness, EOF transfer,
root/frame load blocking, and retirement/live-owner pruning include these owners.
Transferred node leases follow their tracked current arena after adoption.

A generated regression injects failure between body execution and completion
admission, verifies runnable retained work, then resumes to one body and one load.
No tests have executed. Remaining source review must cover error-event admission
failure and cancellation of a scheduler that already owns a native event receipt;
these are distinct from this successful-body continuation correction. Broad L-02
is not marked complete.


### Async error publication and script receipt cancellation (source generated)

Transferred async fetch and module preparation failures now retain an error
completion job before admitting the element ErrorEvent. Recoverable native graph
preparation failures retain the original record for retry and do not become
an authored error. Async source infrastructure failure re-admits the original
frozen fetch; capacity failure remains a native terminal error. Root/frame
`FrameScriptJob` fetch/graph error tails use the same retained completion phases.

`ParserScriptProgress` now owns a native `ScriptEventReceipt` guard. Dropping a
parser/frame/transferred/dynamic script job cancels its exact original owner
receipt, including an already-created event continuation, without selecting the
host's current realm. Successful completion releases the guard after consuming
the receipt. The weak owner avoids extending a retired environment's lifetime.
Generated tests cover failed async error-event admission (no script body, one
error after retry) and progress-drop receipt cleanup (no event replay).

No execution has occurred. Compatibility blocking/deferred resource completion
still requires the same error-record ownership reconciliation, as does error
classification when successful source materialization fails during local module
map adoption. These remaining production callers are not claimed complete.


### Compatibility EOF caller convergence (source generated)

The former separate `resume_parser_completion` fetching/graph/execution body is
removed. Compatibility root EOF completion now retains the same `FrameScriptJob`
as live root/frame preparation, preserving source response, graph, body progress,
and success/error publication across an operation error. Its runnable predicate
uses the child job's preparation/source/graph readiness rather than treating a
moved-out source scheduler as immediately runnable. Existing preparation leases
are shared directly, avoiding a fallible re-pin during scheduler ownership transfer.

Async source completion now calls the native module-map adoption path before
choosing its disposition: established fetch failures retain sticky module-map
semantics and error jobs; successful source materialization failures retain the
original response for local retry; actual source infrastructure failure retries
original-authority I/O; capacity remains terminal native failure. This supersedes
the preceding open compatibility EOF/adoption-classification note. Remaining
legacy live-parser event-loop callers and complete entry-path matrix review stay
open; none of this is execution evidence.


### Legacy caller inventory and typed-resolution capacity (source generated)

All production Unix calls to `run_script_record` are removed; its remaining
adapter and `LiveParserState::resume` are explicitly test/non-Unix compatibility.
All `finish_parser_resources` callers passed an empty script list, so the unused
script-list parameter and one-shot loop are removed. Shared source materialization
now publishes a failed record into caller custody instead of directly queuing an
ErrorEvent; generated regression verifies a retained job publishes it once after
an injected admission failure. Non-Unix consuming parser-loop recovery remains
an explicit cross-platform residual rather than being counted as Unix closure.

Pinned import-map `not-overridden/failed-resolution.html` was reviewed against
successful-resolution recording: unsuccessful bare resolution is not frozen.
Typed module resolution budget exhaustion previously discarded the native error
with `.ok()?`; it now retains Capacity provenance, avoiding both an invented
JavaScript TypeError and transient-preparation retry. A generated saturation
regression verifies exact one-shot native failure consumption. Broader L-02
matrix and all execution acceptance remain open.

### JSON parse-result and cancellation identity review (source generated)

An empty V8 JSON parse result without a captured JavaScript exception is now a
native preparation failure in both typed-module construction and graph error
preparation. It does not fabricate/cache a TypeError. Actual JSON SyntaxError
objects remain sticky and retain their response URL. Generated regressions inject
an empty parser result and verify subsequent native preparation can succeed.

Script receipt cancellation guards additionally capture Document, generation,
stream, and authority before publication. Dropping an old guard after
`document.open()` cannot cancel a replacement stream's receipt even if native
numeric receipt IDs are reused. The old bare realm/receipt cleanup calls are
removed from parser/frame/dynamic progress; generated replacement-stream and
ordinary-drop regressions exercise the new custody boundary. Source only.


### Shared retained HTML parser on all native platforms (source generated)

The platform-independent retained tokenizer/publication loop now serves both
Unix and non-Unix root/frame HTML callers. HTML source readiness, suffix storage,
input-error state, and event publication flags are no longer unnecessarily
Unix-gated; IPC/resource-specific sections keep their existing platform guards.
The non-Unix consuming `LiveParserState::resume` production branch is removed.
The old adapter and consuming parser are test-only reference helpers.

Existing native regressions for prefix publication failure, source-capacity
retry, body/document.write single execution, constructor order, and stop/stream
replacement now compile as platform-independent test sources. This supersedes
the earlier non-Unix parser-custody residual. It does not assert availability of
platform-specific network backends or any test/build result; the complete L-02
acceptance matrix remains open for independent review and execution.

### Script error task insertion points and completion reentrancy (source generated)

Pinned `script-onerror-insertion-point-{1,2}.html` distinguishes a failed admitted
fetch (saved parser insertion point) from an unparseable URL (no insertion point).
Native preparation now recognizes invalid/empty/external-import-map source before
fetch or graph admission and queues its retained error as asynchronous work.
Fetched parser-blocking error delivery resumes its saved gate. Script completion
Notifications carry insertion-point policy across every callback retry; async,
dynamic, and before-fetch delivery suppress the original document's parser gate.
Suppression is scoped to exact Document/generation/stream/authority, restores after
dispatch, and cannot affect a replacement document opened by the callback.

Body execution now ends parser-script-depth and destructive-write guards before
load/error callbacks. Script identity is rechecked after publication admission,
which may itself execute author callbacks, before resuming a numeric receipt.
Generated regressions cover the two actual fetch/no-fetch error-write outcomes,
mid-callback retry of insertion suppression, empty-source admission, and a load
listener replacing its Document. Inline authorization transport/native failures
now propagate to retained retry instead of being logged as successful completion.
No build or test has run; broad L-02 acceptance remains open.

### Classic compilation admission boundary (source generated)

Classic compilation now distinguishes a native empty result without a JavaScript
exception from a caught authored SyntaxError. Native pre-body failure leaves the
started callback untouched so retained parser/dynamic owners can retry; a syntax
error consumes the script and reports the original exception once. Body entry
still records acceptance before running any author code. An empty post-entry
result without exception is surfaced as a native failure rather than silently
claiming success. Generated regression covers unentered failure/retry, one body,
SyntaxError reporting, and restoration of document.currentScript. Source only.

### Local dynamic preparation-error continuation (source generated)

Empty `src` and unsupported external import maps now admit a dedicated retained
script-error task rather than the generic one-shot deferred DOM event. Queue
capacity is reserved before started-state publication. A tracked native target
lease follows adoption; delivery checks the original Document, generation,
stream, authority and arena. The same trusted event and listener suffix survive
preparation/callback interruption. Parking retains work, replacement cancels it,
and cleanup cannot clear a replacement Document's reused event ID. These
before-fetch errors suppress parser insertion points on each retained resume.

Generated regressions exercise actual dynamic empty-src/import-map insertion,
pre-dispatch and mid-callback failures, event identity, once-listener custody,
parser `document.write` replacement, parking and document replacement. This
closes the local preparation-error route identified after shared parser caller
reconciliation; broad L-02 source matrix and later execution remain open.

### Legacy classic script event eligibility (source generated)

Pinned `the-script-element/script-for-event.html` requires both legacy `for` and
`event` attributes to pass an ASCII-only `window` / `onload` or `onload()` check.
Missing either attribute permits execution; NBSP, Unicode case folding and event
arguments do not match. Its module counterpart ignores this filter. A shared DOM
helper now implements the exact predicate. Dynamic insertion applies it before
empty-source error/fetch admission. Parser preparation freezes the predicate,
marks suppressed nonempty scripts started, and does not admit source work;
retained job/body adapters enforce the same decision. Generated native regressions
cover inline/parser/dynamic/module cases, reinsertion after attribute removal,
and invalid/empty external source suppression. No execution performed.

### Synchronous document.write failure handoff (source generated)

The local inline-classic fast path now passes its entered V8 scope to shared
preparation and retains the current tokenizer event plus unconsumed written
markup before any fallible preparation/callback. On interruption, the live exact
gate pauses and hands those records to the host. Already-started bodies are
removed from the retry set; their retained error report owns callback completion.
Completed local attribute handlers are not repeated. Parser frame navigation is
acknowledged before propagating checkpoint termination. CSP-denied inline scripts
are consumed, and the nested-write depth limit is checked before byte reservation.

Regression sources distinguish unentered compilation failure (script retained)
from a body followed by interrupted global error delivery (only suffix retained),
then consume the saved input and verify one body and surviving written markup.
This also corrects the stale two-argument preparation call in this production
entry. Source-only; no execution or whole L-02 closure claim.

### Target adoption after event admission (source generated)

Pre-execution and pre-event admission still require the prepared script to belong
to its original Document. Once the native load/error receipt owns the target and
propagation path, target adoption alone no longer discards the accepted listener
suffix. Parser/dynamic completion revalidates original environment custody at
that stage; document replacement and retirement still cancel it. The local
pre-fetch error task similarly switches from target validation to retained path
custody after dispatch begins. Added native accepted-receipt adoption and binding
mid-listener adoption regressions. All source remains unexecuted.

Nested write recovery additionally preserves insertion order: unread markup from
an inner write paused on an external script stays ahead of the outer write's
suffix after reporting failure. A generated regression checks both retained
external ScriptReady custody and inner-before-outer input ordering.

### Reviewed no-change module event/order obligations

Pinned `module/load-error-events-inline.html` explicitly expects no load event
for successful inline module execution and an error event for a failed imported
fetch. The JSON/CSS/Text `load-error-events.html` fixtures retain that distinction;
existing source's inline-success suppression is intentional, not a missing event.
Pinned `module/inline-async-execorder.html` is served by ordered-front discovery
which transfers nondeferred inline module jobs into the asynchronous queue before
polling their graph, allowing later fast graphs to overtake an earlier slow one.
These are source-path observations, not runtime conformance results.

Pinned `interfaces/html.idl` obsolete HTMLScriptElement members are now declared:
charset, event and htmlFor. Their native reflection routes feed the same charset
capture and legacy classic eligibility paths; generated regression verifies
property-to-attribute effects on execution. Render-blocking token reflection and
the remaining full rendering group are tracked separately in
RENDER-BLOCKING-SOURCE-AUDIT-2026-09-17.md.

### Captured inline CSP caller correction (source review, 2026-09-17)

Source reconciliation found the captured `script_policy` selection accidentally placed inside `authorize_inline_style`, whose signature has no such token, while `authorize_inline_script_kind` still consulted only the current document policy. Moved the exact native-owner/token validation into the script-kind admission path and restored inline style admission to its current native policy. Both classic/module and the new speculation-rules kind retain current creator validation and use the captured policy snapshot when supplied. Existing `captured_inline_policy_survives_later_meta_but_not_owner_retirement` now exercises the intended production code. No build/test execution performed.
