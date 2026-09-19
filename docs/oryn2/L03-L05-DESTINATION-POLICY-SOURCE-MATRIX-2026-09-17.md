# L-03/L-05 destination policy source matrix

September 17, 2026. Source inspection and generation only; no build, test, generator, service or conformance execution. This is an implementation map, not a pass matrix. It does not close either broad plan row.

| Destination / producer | Admission and redirect caller | Generated or traced contract | Remaining source reconciliation |
| --- | --- | --- | --- |
| Window fetch / XHR | `v8_bindings.rs`, `fetch_body.rs`, host HTTP adapter, `context_service.rs::start_http_stream_with_cors` | Native creator authority; CSP upgrade/admission, mixed content, CORS/preflight, credentials, referrer reduction, response embedder policy. Fetch-only mode/redirect/integrity and opaque filtering added; XHR retains separate metadata. Pending HTTP cancellation and complete-body integrity waits share cleanup custody. | Cache behavior generated; remaining RequestInit/body edge cases and abort phase qualification; private service-response backing generated below. Native navigation reset and ordered conversion are generated below. |
| Worker/agent fetch | `agent_io.rs::http_start` | Registry AgentId authority, native policy and cookies, mode/redirect/integrity, local data URL processing, referrer reduction, bounded total header/integrity deadline, filtered service results and body publication. | Complete service-controlled/native navigation Request dependency cases, report delivery and lifetime/accounting matrix. |
| Classic/module script | `context_service.rs::load_page_script_classified`, `script_fetch.rs`, module graph load | Source policy/nonce/integrity/CORS and decoder ordering; per-hop CSP upgrade then mixed-content denial; parent total deadline; exact-document script leases. | Complete destination/redirect negative inventory; CSP redirect path matching generated below. |
| Linked stylesheet / import graph | parser scheduler, `dynamic_stylesheet_preparation.rs`, `standalone_stylesheet.rs` | Link nonce/integrity/crossorigin captured; parent CORS/embedder/SRI before decode; import options cleared independently; graph cancellation scope reaches active leases. | Normative import nonce/source propagation reconciliation, all standalone linked policy permutations and reporting. |
| Font | computed demand plus parent `LoadPageResource` | Existing per-face fallback/demand and CORS; mixed-content/embedder checks and caller cancellation scope generated. | Exhaustive request owner/display-timeout/redirect cancellation and reporting matrix. |
| Image / CSS image / object image | existing `css_images` decoder owner and HTTP image adapters | Existing decoder/intrinsic/paint demand authority remains single-owned; HTTP embedder policy and cancellation custody retained. | Responsive srcset/picture selection generated below; object image response policy and owner retirement permutations remain in the qualification inventory. Mixed-image categories generated below; no visual qualification gate is closed here. |
| Frame navigation | parser/dynamic frame preparation, native `LoadPageResource`, standalone loader, broker | Native captured full ancestor URLs; final-response frame-ancestors and X-Frame-Options; enforced CSP precedence; redirects do not impose destination embedding policy; frame header cancellation and retained body lease. | Remaining synthetic response derivation and secure-context qualification; COEP report-only producer/endpoint generation recorded below. Enforced COEP, inherited Permissions Policy, feature report-only queues and download/no-content precedence generated below. |
| Top-level navigation / downloads | navigation fetch and native prepared navigation/download path | Separate navigation semantics retained; resource mixed-content denial is not applied indiscriminately to top-level navigation. Existing quarantined download and no-content routes retained. | Full policy/cancellation matrix against navigation source, target, redirect and prepared-response custody. |
| Worker script / imports | `agent_script.rs` and registry admission | Native agent origin/policy, post-upgrade mixed-content check, import graph and byte/integrity pipeline traced. | Complete worker destination redirect/report and retirement matrix. |

## Cross-cutting applicability

Trusted Types belongs to authored DOM/script URL and HTML sinks before native request capture; it is not a response-header or redirect check. Sink-to-native-capture coverage still needs explicit inventory reconciliation. Permissions Policy and secure-context state must be carried into the resulting frame/worker identity rather than inferred from a successful network request. Neither property is proved by generic CSP checks.

The production broker's mandatory root ancestor is retained even if the supplied ancestor list omits it. The typed native frame loader independently constructs its complete list from current owner authority. Ordinary HTTP and prepared frame response bodies keep separate retained cancellation custody after response publication; a finished header request is not proof of a drained body.

## New regression source evidence

- `context_service.rs`: `fetch_modes_redirect_filters_and_integrity_gate_response_publication`, `stylesheet_nonce_cors_integrity_and_embedder_policy_survive_redirects`, `owned_style_font_and_frame_cancellation_reaches_pending_transport`.
- `agent_io.rs`: registered native agent fixture extended with redirected fetch mode/integrity/manual/no-CORS/data matrix.
- `v8_bindings.rs`: `request_integrity_clone_and_no_cors_guards_are_native`.
- `network.rs`: `verification_body_rejects_truncation_wrong_identity_limits_and_cancellation`.
- `frame_embedding.rs`: all-ancestor, repeated/invalid/quoted XFO, HTTP whitespace and enforced/report-only CSP precedence.
- `standalone_frame_resource.rs`: schemeful redirect cookie taint and irreversible referrer reduction.

All execution and qualification gates remain open.

## Subsequent CSP redirect source generation

Generated native `authorize_redirect` entry points without changing authored request fields. Parent fetch/script/resource, standalone script/style/frame, agent fetch/import and computed paint-image callers pass their actual redirect loop state. Redirects ignore source-expression paths while retaining scheme/host/port checks. Initial path matching now distinguishes an exact file path from a directory prefix, compares percent-decoded path segments, and supports wildcard hosts/ports without treating a wildcard subdomain as its base host or an omitted port as every port. Added per-destination primitive regressions and a real parent fetch redirect/path admission regression. Native CSS paint images now also apply active mixed-content denial after CSP upgrade.

The earlier table's redirected CSP-path gap is generated for those named loaders; full navigation/form-action and reporting/caller inventory remains open. No execution occurred.

## Subsequent image classification source generation

Image-element `src` preparation now captures an explicit upgradable-image category in its request identity; CSS/object/input-image and imageset (`srcset`/`picture`) callers keep the blockable category. Parent and agent I/O reject the flag on non-image destinations and upgrade eligible HTTP DNS-host requests before CSP admission/mixed-content rejection. IP literals are not auto-upgraded; trustworthy loopback stays local; custom ports survive the scheme change. Added a parent transport regression for upgraded URLs, denied blockable/IP requests and loopback behavior. The existing image source-selection algorithm still needs complete `srcset`/`picture` selection reconciliation; this category change does not claim that visual family complete or add audio/video qualification.


## Standalone frame policy propagation generated

The owning realm now retains response Permissions Policy and COEP headers plus captured container allow/src and ancestor edges. Final frame response admission requires COEP opt-in where applicable; policy reaches the new realm before authored scripts. Root response headers survive native runtime/BFCache transport; inherited blank/srcdoc policies retain container restrictions. Standalone device and document.domain admission consumes this policy; report-only native feature use enqueues bounded ReportingObserver reports without denying access. Isolate-owned first-origin OAC choices are admitted before authored code and retained across same-Document reinstalls. Native parent cluster authority remains separate.

Embedding checks now follow no-content/download disposition: HTTP 204/205 and attachment responses do not create a policy-blocked error Document. Regression sources cover COEP response opt-in, report-only policy, child first-script denial, device denial before backend access, retained control routing, OAC malformed/duplicate/report-only headers and no-Document embedding responses. No regression has been executed.

Remaining inventory: full cancellation/reset destination matrix and synthetic response derivation. Navigation/form-action reporting and responsive image selection now have generated evidence below. Standalone attachments now return no-commit when parent quarantine is unavailable; native parent-backed routes retain bytes in quarantine. RequestInit reset is generated below. This section records generated source only and does not close L03/L05 qualification.


## Native Request navigation reset dependency generated

Reload and traversal provenance now travels from native root history transactions and captured frame history selection through redirect-preserving request context, parent broker and service FetchEvent payload. Authored RequestInit cannot set those flags. Request native records, cloning and IDL expose isReloadNavigation/isHistoryNavigation; any recognized defined init member resets flags, navigation mode, destination and inherited referrer state. An ordered native RequestInit snapshot reads getters once, converts body/header/string/enum values and ignores undefined/unknown members for reset purposes. Source regressions cover service-event flags/clones/reset, inherited cache getter, empty/unknown/undefined dictionaries, frame new-destination versus existing history selection, and redirect preservation. General cache storage behavior is not claimed by this reset dependency. No execution occurred.


## Frozen form-action report generation

Signed form source policy now routes denials to its exact original creator. Explicit final initial/redirect checks emit report-only decisions once; preliminary allowed checks do not duplicate them. Root/frame redirect producers carry the original navigation URL for reports, preserve same-origin query text, remove credentials/fragments, and reduce cross-origin report URLs. Network reports use captured original top-level identity and remain possible after source retirement; DOM events retain original generation and cannot reach its replacement. Added source regression for initial report-only deduplication and redirected denied reports after source retirement. Shared non-form CSP URL sanitization is now generated below; form-action matching semantics are unchanged.


Standalone Permissions Policy feature predicates now consult the live parent binding and current iframe allow/src attributes, recursively checking ancestors on each use. They retain response policies while allowing same-turn container restriction and relaxation without navigation. Retired or missing parent ownership denies admission. Added a no-reload, same-turn document.domain container policy mutation regression. Native parent registry refresh is generated through an authenticated child/parent update. The host installs a weak lifetime bridge that reads live native attributes, checks frame connection identity, recursively refreshes ancestors and caches the exact owner/attribute snapshot. Device/domain/shared-policy/worker/clipboard admission calls refresh before use. Sandbox/origin/COEP/OAC state remains committed. Native registry and real-host same-turn mutation regressions are generated, including stale parent/retired frame rejection.


## Responsive image destination selection (generated, not executed)

`v8_bindings/responsive_images.rs` now parses HTML source sets (including embedded URL commas, descriptor recovery, width/density/future-height constraints), chooses the first supported matching picture source before the image, and normalizes widths using Stylo's `SourceSizeList` evaluator. Native viewport, device-pixel ratio, CSS media features, calc/font-relative lengths and auto-size inputs feed selection. The existing image request owner receives the selected URL, CORS/referrer policy and blockable-versus-upgradable image classification.

Source/image attributes, source insertion, source removal/reordering, base changes and environment updates reselect through native mutation/deferred checkpoints. Same selected URL with only density changed retains its request/decoded backing; new URLs use existing cancellation and late-result guards. Decoded intrinsic dimensions are density-corrected while canvas/paint retain physical pixels. `currentSrc`, `complete`, source/picture brands and reflection, selected source width/height presentation hints and the auto-size contained fallback now have production callers. Native source-set/CSS-size/image-owner regression sources cover these paths; none have run.

This closes the previously absent responsive selection producer, not the entire image loading standard: lazy visibility scheduling, image decode promises, current-versus-pending request presentation, cache reuse across image elements, and full auto-size CSS containment qualification remain separate inventory obligations. Broad L03/L05 runtime and destination-by-redirect gates remain open.


## Fetch cache mode and conditional representation source integration

Generated typed `FetchCacheMode` transport through JS Request records/clones, service-worker FetchEvent records, window/agent requests, parent route-security reconstruction and existing context-owned cache adapters. The existing bounded memory/persistent stores now retain stale validators, honor force-cache/only-if-cached/no-store/reload/no-cache decisions, refuse cache-only network misses, merge native 304 metadata with retained bytes and replay that representation through the existing demand-driven operation contract. HTTP freshness reads Cache-Control, Age, Date, Expires and Last-Modified; zero lifetime is stale. Authored conditional requests and no-store controls bypass reusable cache publication. Policy and cookie authorization still surround the cache.

Generated regressions cover Request mode preservation/validation, stale cache hits, no-store exclusion, cache-only misses, conditional request headers and memory/persistent asynchronous 304 body replay. These sources have not run. Added defaulted FetchOptions/AgentFetchRequest/CookieRequestContext fields remain in the current unexecuted worker16/context49/agentIPC18 generation batch. Full cache qualification (variant retention, cookie header side effects, date/validator edge cases, failure and cancellation stress) remains part of the open L gates.


Cache follow-up source review removed cached Set-Cookie replay (including historical persisted entries), preserved repeated 304 CSP/policy/cookie fields, and combined all Vary/Cache-Control fields. Native cache regression sources assert these custody/policy cases. One stored variant per key is an allowed bounded eviction choice; matching still validates the complete Vary set. No cache execution or conformance result is claimed.


### COEP endpoint delivery and shared CSP report URLs (generated, unexecuted)

Committed enforcing and report-only COEP policies now preserve reporting endpoint parameters through parent policy storage and standalone document capture. HTTP fetch, worker/agent fetch and scripts, classic/module/parser/dynamic scripts, styles/fonts/images, and frame response producers evaluate reports before enforcing denial. Report-only policy emits reports without changing credential mode or allowing a denied response. Frame reports distinguish cross-origin CORP failure (`corp`, `iframe`) from child COEP opt-in failure (`navigation`), matching the pinned `html/cross-origin-embedder-policy/reporting-to-endpoint.https.html` dependency. Frame response checks now run on the final HTTP response after no-content/download disposition.

COEP payloads use the existing bounded native reporting transport and standalone owner cancellation rather than a second sender. Parent endpoint churn is bounded; standalone payloads retain existing pending/size/time/credential limits. Reporting-Endpoints quoted values preserve commas and validate escapes/parameters; endpoint credentials are rejected. Shared CSP URL stripping now removes credentials/fragments, preserves same-origin query strings, reduces foreign HTTP URLs to their origin and non-HTTP URLs to their scheme. The superseded form-only sanitizer was removed.

Regression sources cover report-only/enforce disposition, script callback delivery before denial, frame CORP/navigation distinction, bounded standalone endpoint requests, reporting dictionary syntax and shared CSP URL stripping. No tests were executed. Remaining qualification includes the full synthetic/service-worker and destination/reset/cancellation regression matrix; these statements do not close L03/L05.

Explicit cache modes now apply their request headers idempotently at the final Rustls transport as well as cache adapters; native cache validators retain their existing mode. Added a pure source regression. Reporting-Endpoints fields are combined before parsing, including implicit boolean members with parameters.


### Service response and disabled-cache reconciliation (generated, unexecuted)

The disabled-cache branch now uses a stateless adapter that applies Default plus authored conditionals as NoStore before transport, preserves response bodies without cache buffering, and rejects OnlyIfCached before all three transport entry points. Reporting dictionaries accept non-string structured items and inner lists while registering only endpoint strings; last duplicate member replaces earlier values, including replacement by a non-string. Positive/negative source regressions were added.

Native Window/agent fetch now preserves the service worker's readable basic/cors/default response authority and URL. These responses no longer fail a repeated network CORP check or become opaque solely from the original request URL; integrity still verifies their bytes. Unknown/error response types remain rejected. A real service-worker fixture covers cross-origin synthesized readable response under require-corp and an error response. Streaming navigation now retains the incoming cache/reload/history flags rather than resetting them.

Cancellation phase mapping: stylesheet/font native resource leases now have source cases before headers, after redirect and during body; frame leases cover headers/redirect, with published frame bodies covered by retained response-stream teardown and target-retirement cases. Classic/module script cancellation uses retained script-load scope; fetch/XHR/image/object share HTTP admission/request-abort/body custody; agent bodies retain per-AgentId stream ownership; service responses retain event/version cancellation tickets through body and waitUntil. Existing reset drain tests include ordinary HTTP streams and unattached admissions. These are source contracts and regression inputs, not executed proofs.

Confirmed remaining implementation boundary: opaque Fetch responses currently lose internal status/headers/body before `ResponseRecord` construction. Service-worker respondWith and CacheStorage replay therefore cannot preserve an opaque internal response's bytes/policy authority. Fixing this requires private internal response backing through native Window/agent transport, binding body lifetime, clone/cache storage and service response serialization, while retaining filtered authored getters. This is a concrete residual generation requirement; L03/L05 stay open.


### Private filtered response backing (generated, unexecuted)

The opaque response boundary identified above now has generated native implementation: optional private metadata carries raw status/headers/URL/redirect/range provenance across Window and agent response admission; the existing retained body stream survives public filtering. Binding Response records retain this metadata and hidden stream, with public body/bodyUsed/consumption staying empty. Clone tees the internal stream. CacheStorage persists/restores private backing and validates the filtered/public metadata shape. Service-worker buffered and streaming respondWith paths consume internal bytes and serialize native policy metadata; opaque range replay to a request without Range is rejected before consumption. Internal headers are captured before public CORS exposure filtering.

Generated regression sources cover Window/agent opaque and manual-redirect private body reads, preserved Location, CacheStorage private persistence and invalid-public-shape rejection, authored getter/consumption isolation, hidden-stream clone replay and range rejection before consumption. Runtime and conformance qualification remain deferred. Subsequent direct pinned helper review identified a separate required cache gap: `fetch/stale-while-revalidate/stale-script.html` requires returning a stale representation while an owned background revalidation refreshes it; existing synchronous conditional 304 revalidation does not implement that scheduling contract.


### Owned stale-while-revalidate cache refresh (generated, unexecuted)

Memory and persistent cache now return eligible stale representations and schedule bounded, deduplicated revalidation through the original policy/cookie/cache chain using NoCache. Existing validators, conditional-304 merge, response-cookie processing and cache publication remain the production path. Freshness uses response age plus elapsed time, with stale windows and mandatory revalidation directives checked independently. Range requests bypass whole-representation reuse; partial 206 responses remain deliverable to script loaders but are not inserted/reused as complete HTTP cache entries.

Native lexical background admission spans retain every initiating IdentityBroker lease across executor handoff and cancellation acknowledgement. Normal foreground EOF does not cancel a refresh. Scope retirement and request cancellation signal the captured owner; background scope drain precedes lease release. Default broker deferred readers explicitly restore the captured span; replay transport lookup occurs before its event-forwarding threads, and cancellation-only spawn_blocking tasks do not admit cache work. The private refresh uses a new native request identity and re-enters cookie/policy admission. Per-cache dedup never supplies document activity counts.

Regression sources cover memory/persistent stale-then-updated delivery, matching-request dedup, initiating-document activity and sibling isolation, panic restoration, cancellation custody until ACK, age/window/must-revalidate bounds, 304 terminal consumption, and partial script/cache separation. These complete the identified source gaps, with all build/test/conformance and five-family evidence gates still unexecuted. Further failures discovered during global validation must be fixed before qualification closure.

Final custody review: background collection panics still enter retained scope-drain cleanup. Native leases distinguish actual foreground terminal/EOF from cancellation: canceling a completed stale delivery signals its background owner without sending its completed cache-hit ID back to transport. Parent document activity remains retained until the background operation acknowledges cancellation. Both synchronous and asynchronous 304 adapters consume the empty response terminal before replacement. Root separately added exact captured-None lexical span restoration, including panic/nesting regression sources.

## Fetch Metadata producer integration (source-only)

Shared native admission strips author-supplied Sec-Fetch headers and derives
mode/destination/site/user from native request provenance. Site taint accumulates
across redirects independently of cookie ancestry; opaque initiators remain
cross-site. Trustworthy destinations receive headers; insecure non-loopback
endpoints do not. Script/module, CSS/font, image/media, frame/navigation, worker
entry/import/fetch, report and WebSocket producers now supply their actual modes
and destinations. Native CSS `stylesheet` is normalized to Fetch `style`.
WebSocket site comparison uses its HTTP-equivalent URL. Browser-initiated top
navigation retains `none`; authored navigation uses signed source identity and
activation frozen at acceptance. Regression sources cover broker-boundary author
header replacement, redirect taint, secure delivery, opaque/handshake behavior,
and activation before/after acceptance, consume and retirement. All are unrun.

Authenticated local Blob HTTP/media reads now retain admitted bytes across URL
revocation, enforce owner/CSP before resolution and provide bounded single-range
206 responses. Revoked new reads fail; existing stream ownership still lasts to
terminal ACK. Positive/negative source regression covers that distinction.
These bounded additions do not close the entire destination/cancellation matrix.
