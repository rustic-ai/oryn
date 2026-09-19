# Parallel loading generation — September 18

Source inspection and source edits only. No build, tests, generators, formatter,
services or conformance runs have executed in this lane. This does not close L
or the global generation gate.

## Generated in this pass

### Retained dynamic import settlement

`dynamic_import_v8.rs` now settles live imports after graph timeout, terminal
capacity or cancellation through the retained native rejection publisher. The
old branches dropped a popped resolver or marked publication complete without
publishing a promise outcome. Retired environments still discard their own work.
A prepared evaluation promise/namespace or original exception survives a later
publication deadline: retry neither repeats the module body nor substitutes an
unrelated timeout for an established module outcome. Prepared native capacity
and cancellation failures likewise queue rejection rather than abandon a live
promise. Terminal resource errors do not create sticky fetch-failure cache entries.

The scheduler now includes waiting graphs in deadline wakeups. Once a graph has
completed, publication retries use their retry delay instead of an expired graph
or publication deadline, avoiding a zero-delay spin.

Added regression sources:

- `expired_import_graph_rejects_live_promise_and_preserves_successors`
- `expired_publication_retry_keeps_evaluated_namespace_and_settles_once`

Existing same-namespace, original SyntaxError identity, retired owner, graph
sharing, TLA and retained-source tests remain relevant but have not run.

### Portable parser module graph owner

`FrameScriptJob` owns its module graph on every platform. The non-Unix path now
prepares and polls graph work before execution, retains verified root response
bytes in `ParserLoadedSource`, preserves graph errors, and shares document graph
budgets with dynamic imports. Execution remains gated separately from preparation.
`from_completion` keeps its existing platform-dependent signature; non-Unix
callers initialize their own graph owner.

The shared `prepare_parser_module_graph` helper now has only its `context`
argument conditional on Unix. External records without loaded source enter an
external-root graph with the captured request URL, credentials, integrity,
nonce, parser insertion and render-blocking metadata. Inline standalone modules
check captured CSP before dependency admission. Existing Unix prepared-source
and authenticated parent routes retain their behavior.

Added non-Unix regression source:
`portable_parser_module_prepares_graph_before_execution_and_releases_owner`.
It checks graph preparation while author execution is disabled, then one body
execution through the same retained job. Cross-platform build and execution are
still mandatory after the global gate.

## Caller disposition and integration dependencies

| Entry | Source disposition |
| --- | --- |
| Dynamic import, Window and worker | Shared retained graph and native promise publisher; terminal settlement corrected here |
| Direct native module execution | Existing retained direct queue; waiting graph deadline wakeup now included |
| Unix parser root/frame/document.write jobs | Existing source scheduler and graph owner; shared helper remains compatible |
| Portable `FrameScriptJob` root/frame completion | Graph owner and verified response handoff generated here |
| Portable `FrameScriptJob` classic fetch | Bounded asynchronous source owner, frozen request metadata, retained verified response and cancellation generated |
| Dynamic stylesheet/font dependency publication | Existing retained dependency source and binary publication stages inspected; no duplicate implementation added |
| TrustedScript preparation | Exact-lifetime raw-input/accepted-output receipts generated; transforming policy is not rerun for unchanged preparation |

Final source inspection found two production retained execution consumers:
`FrameScriptJob` on all platforms and `ParserAsyncExecution` on Unix.
The old synchronous non-Unix fetch branch in `execute_script_record` is removed;
prepared source and prepared module graph are now common execution invariants.
The legacy `run_script_record` entry is test-only. The shared completion slice
constant is now available on all platforms where its consumers compile.

## Completed retry and preparation follow-through

### Module graph transactions

`module_graph_load.rs` retains popped completion envelopes until cache and graph
source publication succeed. A post-cache retry adopts the first cached response;
no second fetch or duplicated source accounting is required. Shared successful
bodies remain in the fetch owner when a follower cannot reserve its response
copy. Request infrastructure failures recover from captured request metadata,
and obsolete shared failure entries are removed before admitting a retry.

Dependency parsing retains the original source and reservation until all child
admissions commit. Each visited key/count commits after its successful native
admission; already committed children and parse edges are not duplicated on
retry. Failed native request preflight restores the queued request and returns
the fetch slot. Frame jobs retain these graphs across retryable errors.

Regression sources: `successful_graph_response_survives_pre_and_post_cache_publication_failure`,
`graph_dependency_admission_retry_keeps_input_and_commits_visit_once`, and
`graph_request_preflight_failure_preserves_request_and_slot`.

### Portable classic script source owner

`PortableScriptSource` freezes prepared URL, encoding, CSP, nonce, integrity,
credentials and referrer policy before admitting shared-executor work. Its slot
uses the same eight graph/source lanes and source bytes use the existing page
budget. Polling receives one retained verified response without blocking the
page actor; destruction signals cancellation. An infrastructure admission error
can retry, while real fetch failure queues the script error. Preparation remains
separate from permission to execute across stylesheet barriers.

Regression source `portable_classic_source_is_retained_before_execution_and_cancelled_on_drop`
checks the frozen source despite attribute mutation, no body execution during
preparation, single eventual body, and cancellation ownership.

### TrustedScript acceptance receipt

`V8BindingState::script_preparation_receipts` records exact native document,
generation, stream and authority lifetime, raw DOM input and accepted output.
The receipt is reserved before calling author policy and published before later
fallible native preparation. A same-input retry preserves UTF-16 output without
calling a transforming policy again; changed input requires new acceptance.
An in-progress receipt prevents recursively preparing the same policy attempt.
Receipts retain no DOM nodes or V8 handles, are pruned against live native nodes,
and are cleared at script start and existing retirement/reset boundaries.
The collection is capped at 1,024 entries and 32 MiB, reserving the 8 MiB source
allowance before author code. An oversized accepted output records a terminal
receipt without keeping that oversized string or replaying the callback.

Regression sources: `transformed_trusted_script_preparation_retries_reuse_accepted_units`
and `trusted_script_receipt_capacity_rejects_before_policy_and_retry_can_recover`.
Existing authority replacement, UTF-16 and parser mutation controls remain.

## Final graph consumer reconciliation

| Consumer | Retry and terminal disposition |
| --- | --- |
| FrameScriptJob, Unix and portable | Same graph retained; native timer and parser retry preserve source and script identity; retry deadline eventually queues script error |
| ParserResourceScheduler async graph | Same removed graph restored to async graph queue before returning retryable failure |
| ParserResourceScheduler root source | Same source graph and completed cache response retained through copy/admission recovery; expired native failures use NativeTerminal, which cannot poison module map |
| Dynamic module source | Retained source graph and existing response cache survive graph/response-reservation recovery |
| Dynamic module graph | Same graph, original loaded source and captured backend return to scheduler on retryable error |
| Dynamic module graph admission | New retained admission stage owns original source while byte budget/backend/graph construction recovers; 60-second terminal bound |
| Direct native module | Initial infrastructure failure enters retained continuation; established continuation already retains graph and has bounded deadline |
| import() | Existing retained graph with bounded deadline; terminal promise settlement corrected above |
| Worker startup | Existing PendingAgentStartup retains graph on infrastructure retry under 60-second bound |
| Modulepreload | Existing retained job handles infrastructure retry and final cache/event publication under fetch/publication deadlines |

Graph source retry uses a context-owned native wakeup and a 10 ms delay, capped
by a 60-second graph retry lifetime. Host timer projection includes this wakeup;
expired wakeups are consumed and still-waiting graph/admission work republishes
its next deadline. Parked contexts do not produce runnable timer work. Retired
script cleanup drops graph admission/source reservations. Terminal native parser
failure is distinct from established fetch failure and does not publish a failed
module-map response.

`ScriptPreparationCapacity` now transports receipt admission/allocation quota
as a typed binding error into `HostError::ScriptLoad(Capacity)`. Oversized accepted
policy output remains a terminal receipt. Identical message text does not change
native failure classification.

Additional regression sources:

- `graph_retry_has_native_wakeup_and_bounded_terminal_deadline`
- `dynamic_module_graph_admission_retry_retains_source_until_capacity_recovers`
- `preparation_capacity_mapping_uses_provenance_not_matching_message`
- `oversized_accepted_script_receipt_is_terminal_without_policy_replay`
- Existing parser failure-publication regression now includes NativeTerminal
  and checks that it never creates a sticky module failure.

## Preserved loading algorithms and source evidence

- Shared `oryn-js::import_maps::ImportMap` retains exact/prefix/scoped/null
  resolution, insertion/merge atomicity, first accepted mapping and resolved-key
  protection. Existing source regressions cover structural/capacity rollback;
  `captured_import_map_root_is_not_resolved_a_second_time` covers dynamic roots.
- Import-map/element SRI stays in captured fetch metadata and verifies raw
  response bytes through the native backend; this pass does not weaken policy,
  reconstruct bytes from decoded source or consult a source-only cache as proof.
- Existing native evaluation/settlement records retain TLA dependency promises,
  original rejection objects and namespaces. This pass retains that outcome
  across publication retries and does not reevaluate an entered module body.
- Existing stylesheet dependency/binary/font publication stages remain the
  single implementation. No second loader or authored bridge was introduced.

## Remaining qualification

All scoped retry/portable preparation/acceptance-receipt source obligations
identified in this pass have implementation and regression sources. This is
source completion, not execution evidence. Cross-platform builds, all regression
execution, full Core and five-family qualification remain deferred until the
global generation gate. Broader pinned L-01 helper inventory and destination
policy/accounting work remain with their owning lanes; this audit does not close
those inventories or the L parent gate.

## Redirect reporting and deferred container admission follow-up

Source generation only; no build, test, formatter, service or conformance execution.

- CSP `authorize_redirect_from` separates current-hop admission from original-request reporting. The canonical CSP `create-violation-for-request` explicitly uses request URL, not current URL, to avoid exposing redirect targets. Production callers reconciled: context Fetch/XHR/image/object/media and cached image preload hops; context script and page-resource (frame/style/font) loops; standalone script/module/style/import/font/image/frame; agent worker/module/worklet entry and agent HTTP; deferred Fetch; both preload reuse receipt consumers; paint snapshot image loader and exact-realm reporting RPC. Legacy initial-only authorization remains compatible.
- Agent HTTP previously discarded CSP violations while checking `allowed`. It now publishes the existing native agent CSP event/report custody before denying an enforced request or continuing a report-only request.
- Paint reporting transports original URL plus redirect flag with serde defaults; parent policy recalculation retains redirect path semantics and never substitutes the secret target into `blocked-uri`.
- Generated regression sources exercise nine destinations under enforce/report-only, original same-origin query with credential/fragment stripping, foreign original URL reduction, current-hop blocking and redirected path matching. Existing standalone script and paint redirect producer regressions now assert original URL custody.
- Deferred Fetch iframe reservation now occurs after the initial frame obtains its stable token and before first response-producing navigation. Parent `src` scheduling reserves again before executor submission; document-driven navigation does not reallocate. Context and standalone schedulers call the same host admission helper. A native producer regression asserts reservation precedes LoadPageResource. Family lane owns stable-token accounting, committed-origin reconciliation and frame-removal release.

### Current authoritative hash invocation distinction

Canonical CSP source `https://github.com/w3c/webappsec-csp/blob/main/index.bs`: worklet destinations map to script-src-elem; that postcheck invokes script-post-request and hence potentially-report-hash, using normal script-src-elem → script-src → default-src fallback. Worker effective directive is worker-src: explicit worker-src and child-src fallback do not invoke the hash helper; script-src fallback does; default-src fallback delegates worker-src postcheck and does not. The pinned WPT report-hash helper at revision `719d5e38fdd0903a18ed9007aba816c98cc491e0` only creates script/link elements, so provides no contrary worker/worklet expectation. Root owns corresponding hash implementation/regressions.

### Remaining L03/L05 inventory status

The redirect caller inventory is reconciled above. Combined body/accounting inventory is still open and must not be marked complete: per-agent HTTP and navigation-preload bodies share the same 64-entry stream map (not separate 64-entry maps), retained preload cancellation owners have a separate 64-entry bound, and WebSockets have a separate 64-entry bound. Source-only bounded counts and per-body limits do not by themselves prove a single aggregate byte budget across all simultaneous producer families. Native context response publication owns an admitted operation permit and cancellation guard through publication; broader aggregate overlap remains to audit after newly assigned Core streaming HTML dependency generation.

## Core streaming HTML source bridge

The exact pinned callers are `html/semantics/scripting-1/the-template-element/template-for/template-for-sanitization.html`, its multiple-cases/shadow-root/unclosed-content/nested-patch/foster-parenting siblings, and `html/dom/partial-updates/tentative/stream-append-with-template-for.html` plus style-invalidation fixtures. Full pinned `html` source search finds only the real method `streamAppendHTMLUnsafe`; `streamHTML` occurs in prose/title. No speculative sibling API was added.

Generated `v8_bindings/html_stream.rs` and narrow Element/ShadowRoot IDL/dispatcher hooks create an actual native WritableStream. A dedicated WritableKind uses the existing ordered write/close promises, writer pressure, error propagation and abort path. The sink owns a migration-tracked target, weak document owner, exact document generation/stream/authority tuple, and a persistent native `StreamingParser::with_fragment`. Input conversion uses native DOMString transport; chunks are never concatenated and reparsed. Custom-element construction/migration callbacks use the existing parser action gate. Resource and completed-script events enter retained post-connection preparation. Constructor-induced parser publication interruptions resume before a write resolves; EOF is emitted once and retried only for retained mutation completion. Abort/error/GC drop parser custody without EOF or replay. Native queued writes are capped at 1024 and aggregate retained parser input at 32 MiB, released on close/abort/drop.

Parser lane supplies live-target root aliasing, token-level sanitizer filtering before construction, script inertness/runScripts metadata, PI provenance and template patch receipts. DOM lane supplies the common sanitizer option conversion/configuration. Four generated binding regressions cover incremental node identity/entities/lone UTF16 units; live-target patches and template/attribute sanitization across Element/ShadowRoot; abort and exact coercion failure without replay; and default script inertness. These are source obligations generated, not passing test claims. Broader template patch publication semantics remain under parser lane review; all runtime/build/conformance verification remains deferred by the user directive.


### Additional source closure: typed destinations, declarative CSS and stream custody

- Canonical HTML `fetch-destination-from-module-type` selects CSS `style`, JSON `json`, text `text`, JavaScript `script`; CSP effective-directive maps style to style-src-elem and JSON/text to connect-src. Shared ModuleType helpers now drive standalone and parent typed loader redirects and Fetch Metadata. Initiator remains script. Non-JavaScript typed responses do not invoke script hash reporting. Sources backend uses the same CSP destination. Added standalone and parent policy/redirect regression sources; not executed.
- Native declarative CSS hook consumes existing typed module map/preload jobs. Data URLs use existing policy loader synchronously; default sheet identity is shared with static/import/preload consumers. Publication failure retries retained module identity, and enclosing PreparedImportLink custody is restored. Source regressions cover synchronous identity, typed preparation infrastructure recovery, duplicate async admissions, swapped placeholders and failed fetch preservation, and parked completion custody. Binding lane separately covers committed-publication cleanup retry and clone/serialization.
- HTML stream admission charges transient input plus actual retained tokenizer/treebuilder buffers, releasing consumed chunks after each parser advancement. Committed DOM does not consume the 32 MiB input ledger. Every accept error aborts and releases retained parser state, including conversion/owner failures. Queued writes defer while suspended and resume through platform completion pumping. Added many-chunk cumulative-budget and retained-errored-stream reclamation source regressions.
- Agent HTTP attempts now reserve acknowledged cleanup tickets before broker start; preheaders rejection/redirect/error retains cancellation custody. Successful streams carry the same ticket, retirement cancels and drains it. Source regression rejects headers, holds cancellation acknowledgement, and verifies eventual cleanup after acknowledgement.
- HTML stream ExternalTemplateDiscovered forwards to the native resource admission hook. The dedicated external-template transport/parser lane owns subsequent fetching and patching; this forwarding alone is not a conformance claim.

All entries above are source generated only. No builds, tests, formatters, services, or WPT were executed. Combined L03/L05 route-level body/backpressure/cancellation accounting remains an audit obligation; helper presence alone does not close those broad checklist rows.

### Ordinary Fetch keepalive generation (active integration)

Window HTTP dispatch now reserves the shared exact-client keepalive byte lease after preflight and before actual payload IO; JSON/base64 transport bytes do not replace decoded body accounting. Redirect method/body rewriting shrinks the shared charge. Successful response EOF (including SRI's full-body read) releases the charge, while failures keep it in acknowledged cleanup custody.

`keepalive_drain.rs` reserves a bounded orphan-response consumer before IO. It polls retained native operations without blocking an agent or allocating a retirement-time work slot. Native HTTP entries transfer to this consumer on document retirement or discarded response publication; explicit AbortSignal/native request cancellation still cancels. The consumer respects the response body limit and content length, releases at EOF, and retains failed cancellation until acknowledgement. Source regressions cover a retired consumer holding the full 64 KiB charge through partial body consumption until EOF, and protected retirement vs explicit cancellation.

Authority lane owns the immutable prelaunch request/creator/controller receipt and host dispatch; family lane owns AgentIo/platform lifetime and the common quota. `FrozenFetchController` retains the exact selected service-worker executor/version and captured client identity, reserving each redirect event from that retained version rather than selecting the current controller after retirement.

Remaining integration concerns: a prepared request must dispatch even if the child transport disappears before sending Start; standalone Window Fetch needs a production PlatformHost path; true BrowserContext shutdown must cancel retained orphan IO independently from document retirement. These are active source obligations, not verified behavior. No test/build/service execution has occurred.

Follow-up source reconciliation: authority lane now schedules the accepted request in the parent during Prepare; Start only retrieves its result. Both failed wire-publication custody and explicit unreceived-result release transfer keepalive responses to reserved native drain ownership. Cleanup scopes distinguish ordinary document retirement from forced shutdown, so a protected redirect cannot reopen a forcibly cancelled scope. Parent header/SRI and orphan-body consumers observe true BrowserContext shutdown; retained CSP reports use the frozen policy/partition without addressing a successor Window. Parent HTTP attaches cancellation custody before broker start, covering brokers that register work before returning an error. A frozen-controller regression now removes live client/controller lookup entries before dispatch and confirms the previously captured streaming controller still serves the request. All verification remains source-only; standalone Window integration continues in the rendering lane.

### Combined producer/consumer source inventory

| Producer family | Native admission and response consumer | Backpressure / byte custody | Cancellation / late publication |
| --- | --- | --- | --- |
| Window Fetch/XHR, object/media, image fetch | `ContextPlatformHost` → typed HTTP stream request → `HttpStreamEntry` → bounded reads | Parent dispatch limit32, parent aggregate operation permit64; response16MiB; stream registry32; native demand channel bounds outstanding chunks | Native response receipt owns unpublished stream; exact creator/retirement gate; keepalive uses frozen prelaunch and reserved orphan drain |
| Agent Fetch + navigation preload | `AgentIo` shared HTTP stream map | Shared64-entry map, current native chunk and downstream demand; preload is not a second independent64-stream allowance | Agent scope, preload custody and acknowledged HTTP tickets; family lane adds frozen keepalive producer and orphan consumer |
| Parser/dynamic classic + module graphs, modulepreload | `frame_script_preparation`, graph queue/backend and raw host module map | Shared graph/classic fetch slots; per-page32MiB ScriptSourceReservation ledger; source graph retries retain popped work and first response | Script admission token, native creator, cancellation guard; consumed module evaluation result retained through publication retry |
| Dynamic stylesheet + imported CSS/font/image dependencies | `dynamic_stylesheet_preparation` and typed page resources | Request metadata, accepted stylesheet source and font/binary resources charge the same per-page32MiB source ledger; fonts retain bytes through layout admission | Prepared style policy, native document epoch/revision, retained dependency jobs and fallible publication receipts |
| Frame navigation and decoder | `frame_resource_preparation` / `frame_response_stream` | Native HTTP stream ownership, bounded64KiB decoded response chunks and upfront decoder reservation; parser input shares source ledger | Parent frame policy receipt, response lease and stable frame token; cancellation releases policy/stream through acknowledged cleanup |
| Paint image cache | Native runtime image loader and rendering image consumers | Bounded network collector plus native image cache ownership; original redirect URL remains separate from current admission URL | Captured policy and epoch/revision caller; rendering lane owns raw Content-Type/XML decoder and consumer publication |
| Streaming HTML | `html_stream` → retained native fragment parser | Global32MiB actual retained input/tokenizer/treebuilder ledger; fully consumed chunks released; queued writes bounded1024 | Tracked target + exact document lifetime; every failed acceptance aborts parser custody; suspended queued writes resume |
| External template sources | Dedicated native template resource task and retained parser patch stream | Existing typed HTTP chunk transport; SRI holds bytes until verified; exact source publication queue | Template lane owns parser event/patch receipt; native insertion enqueue commits once before fallible shared polling |
| Ordinary keepalive + activated deferred fetch | Shared exact client `keepalive::Budget` | Combined64KiB unfinished request bodies at actual HTTP dispatch, excluding preflight; redirects shrink charge | Lease remains through EOF or cancellation ACK; ordinary pending headers/body survive Document retirement; true context shutdown cancels |

The resource IO executor is shared (8 workers,1024 admitted jobs), as is the acknowledged HTTP cleanup pool (1024 tickets). Orphan keepalive responses use128 pre-reserved nonblocking consumers. These are overlapping scopes, not independent capacities to add as though each producer owns its own pool. Browser cache objects, decoded layout objects, socket buffers, and JavaScript-visible Response/Blob data have separate ownership domains; this inventory does not claim a single whole-process byte ceiling. Qualification must exercise simultaneous producers and late publication paths, rather than treating each helper's isolated unit tests as aggregate conformance evidence.

Canonical scripting-disabled reconciliation: HTML's create-CSS-module and fetch-single-module algorithms do not short-circuit CSS loading when scripting is disabled. The Sources backend now restricts its generic sandbox execution gate to JavaScript modules, while CSS still follows Style CSP. Added Sources negative-JavaScript/positive-CSS regression, native declarative data-CSS preparation with scripting disabled, and parent/standalone destination matrices combining sandbox with blocked Script and allowed Style/Connect directives. These are generated regression sources, not passing runtime results.

### Dedicated script-close sandbox source

Generated `script_close_sandbox.rs` and runtime module hook. `BrowserContext::script_close_sandbox_allowed(source_page, source_creator, target_page, target_creator)` implements Window.close's unusual snapshot ownership: sandbox flags and nonconsuming transient activation come from the target active Document; ancestry comes from the incumbent source navigable. It uses exact active native creators, rejects non-root targets and prerender authorities, honors the one permitted sandboxed navigator, and revalidates both creators after its snapshot. It does not reuse ordinary navigation's sticky-activation/frame-busting or domain rules. Regression sources cover reversed target/source policies, target activation, same-context and permitted-navigator exceptions, stale/non-root creators and prepared private prerender authorities. Root owns public close admission integration. No tests were run.

## Origin producer reconciliation — source generation, 2026-09-18

- Window modern Fetch/XHR/Beacon and SW network fallback: `context_service::start_http_stream_with_destination` uses shared `referrer::request_origin_header`, captured source authority, retained CORS response taint, redirect-rewritten method and response-updated policy. Keepalive retains the same original authority after retirement. Preflight OPTIONS keeps its independently required Origin.
- Agent Fetch/image/native fallback: `agent_io::http_start` uses the shared helper. Its `origin == "null"` condition preserves CORS taint on a cross-origin return hop. Extended the existing real broker redirect regression to assert `[absent, original origin, null]`, including a forged initiator and `no-referrer` captured policy.
- Standalone modern Window Fetch and activated deferred/fetchLater: shared helper, frozen source, mutable redirect origin. Renderer owns their producer regression extension. Deferred admission itself creates no network request; actual dispatch shares this rule.
- Module/static import/dynamic import/modulepreload/declarative CSS: parent classified module loader uses the helper. Portable `script_fetch` emits Origin only for CORS-cross GET and retains an opaque redirect-origin bit. Typed CSS/JSON/text select their own Fetch/CSP destinations independently of Origin. Preload reuses those loaders rather than synthesizing another request.
- Worker entry/importScripts/module descendants/worklet/service update: `agent_script` GET producer sends Origin only for module CORS-cross requests, with opaque redirect origin retained. Worker entry same-origin checks remain independent. Captured headers feed both SW interception and fallback; no fallback reconstructs a different Origin.
- Standalone stylesheet and external-template GET producers already retain opaque redirect origins; simple no-CORS classic/style/image GET has no Origin. Fonts and CORS styles in parent resource loading preserve null redirect origin.
- Corrected installed compatibility `__oryn_fetch_async` (still registered in normal/snapshot globals and used directly by lifecycle source regressions): capture document referrer policy before dispatch, omit same-origin GET Origin, use shared non-GET policy branch, retain CORS redirect taint and process redirect response policy. Added direct callback broker regression for GET versus POST.
- Corrected parent frame/form POST producer: previously only font/CORS requests added Origin. It now uses the original navigation source or replay provenance, including opaque origin, current response policy and method after redirect. Extended history/reload source regression to exercise POST and distinguish original origin from current page origin.
- Corrected standalone frame POST producer: previously unconditionally exposed initiator URL origin. It now uses original/replay authority and shared policy logic. Added source regression matrix for successor vs original source, opaque source, 302 POST→GET (Origin absent), and 307 retained POST after `no-referrer` (Origin null). Retained cross-origin redirects also serialize null.
- WebSocket handshake Origin remains mandatory and separate from ordinary Fetch Origin generation.
- Actual Core `link rel=prefetch` remains a separately assigned parser-lane implementation: legacy runtime scan currently classifies it as a module, which is not conformance evidence. Family supplied exact Core fixtures; parser owns dedicated request/cache/lifecycle closure.

All entries above are source inspection/generation evidence only. No tests, builds, formatters, services or conformance runs were executed. HTML Writable blocking-script ordering ownership is transferred to the automation lane; loading does not concurrently edit its stream/parser receipt hooks.

### Accepted blank prerender loading custody

`PendingCandidate::start` previously ran HTTP and redirect authorization through the initiating page's client, creator and broker, even after a blank-target claim transferred ownership to a hidden destination. It now captures the reservation's destination endpoint and broker before any IO. Direct admission requires the destination broker; it cannot fall back to the source broker. The existing request cookie/referrer snapshot stays unchanged.

`NavigationPolicySource::Prerender { token, generation, epoch }` carries the native destination scope receipt through each `AuthorizeNavigation` request. `BrowserContext::authorize_prerender_navigation` validates the exact unguessable scope, lifetime and live destination, and requires credential-free HTTP(S) within the scope's immutable request origin. An unselected candidate still requires its original source creator. A claimed blank candidate survives source retirement without consulting the successor. Scope cancellation and foreign/mutated receipts reject subsequent admission. Existing response/lease custody handles canceled or dropped candidate results.

Generated source regressions cover selected versus unselected source retirement; original origin versus successor, foreign page/epoch and URL checks; explicit scope cancellation; and the actual retained `PendingCandidate` HTTP→302→MIME→commit path with destination route assertions after source replacement. These are unexecuted sources.

### Top-level navigation Origin producer

The final ordinary-navigation producer audit found `navigation_http::HttpNavigation` omitted Origin for POST. It now captures original initiator origin separately from reduced referrer text, preserves opaque authority and redirect taint, and uses the shared Origin helper with the current method and response-updated policy. Existing redirect producer tests now assert original POST Origin, absent header after 303→GET, and null after retained 307 POST/no-referrer. A new opaque-source test proves referrer text cannot recover a tuple origin. Navigation scheduling is unchanged; no tests or builds were run.

### L-01.o final assigned embedded-source reconciliation

See the September 18 continuation in `L01-EMBEDDED-CONTENT-SOURCE-AUDIT-2026-09-17.md`: declared-type conditional selection, typed no-handler outcome, object/embed notification provenance, exact empty/whitespace source predicates, navigate metadata/credentialless distinction, captured wire/source bytes and registered-then-error cancel-ACK identity now have production callers and regression sources. String fixture resources remain documented decoded-HTML input with no invented HTTP metadata. Wire generation is stable: optional `ResourceInitiator.embedded_type`, unit `EmbeddedResourceUnavailable`, plus the earlier `NavigationPolicySource::Prerender` receipt. No tests/builds/formatters/services were run.
