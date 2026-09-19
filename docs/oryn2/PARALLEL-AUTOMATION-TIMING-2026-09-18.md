# Automation and resource timing source completion — September 18

Source generation and inspection only. No builds, tests, formatters, binding generators, services or qualification were run. Historical outcomes and the fixed 16,737-entry Core denominator are unchanged.

## Generated changes

| Requirement | Production source | Generated regression sources |
| --- | --- | --- |
| Generic resource worker timing | `oryn-js/src/v8_bindings/resource_timing.rs::resource_worker_start` projects the retained native worker-selection stamp using the response producer's monotonic anchor and original request timeline. Missing anchors, timestamps before admission/after headers, and denied timing disclosure return zero. It does not borrow a wall clock or manufacture a worker start for cache replay. Existing native interception producers already carry `TransportTiming.worker_start_clock_micros`. | `resource_worker_timing_requires_current_response_clock_and_permission`: measured/coarsened value, opaque permission, missing anchor, obsolete worker stamp and future stamp. |
| Resource redirect timing and inherited JSON | Resource entries expose measured redirect intervals and final fetchStart from retained `navigation_phases`. Inherited workerStart/redirectStart/redirectEnd appear in resource JSON as well as navigation JSON. Missing phase records remain zero. | `resource_json_includes_inherited_worker_and_redirect_timings`: distinct initial request/final fetch/redirect phases, inherited JSON parity and withheld foreign details. |
| Exact terminal stream identity | Both native external timing queue and JS PlatformHost timing queue ignore a foreign HttpStreamChunk rather than completing the original request or adding its bytes. The original stream remains retained for its own terminal receipt. | `resource_timing_scope::foreign_stream_reply_cannot_complete_the_observed_request`, `resource_timing::foreign_platform_chunk_does_not_publish_original_stream`: foreign terminal receipt followed by exact terminal completion, one entry and no foreign bytes. |

Changes are confined to `crates/oryn-js/src/v8_bindings/resource_timing.rs` and `crates/oryn-native/src/resource_timing_scope.rs`. Existing source in both files is preserved. Binding declarations already include workerStart and redirect fields; no IDL regeneration is needed for these edits.

## G operation and helper reconciliation

- The operation contract continues to map 19 native operations and 19 conditional Bluetooth helper names. The prior source audit retains the 1,516 occurrence inventory and exact direct-Core versus conditional boundary. No new operation was inferred merely from a helper name.
- `scripts/wpt-testdriver-oryn.js` is explicitly historical and requires the historical native-input feature. Official execution uses the pinned upstream extra adapter and `tools/wptrunner-oryn/oryn_wptrunner/protocol.py`; rewriting the historical adapter would not close the official route requirement. It was left intact.
- Preload observations initially recurse through their underlying resource request, but the production `ContextServiceTransport::request_with_timing_metadata` producer explicitly overrides preload initiator to `link`. A presumed missing preload initiator was disproved by following the caller; no duplicate adapter was added.
- Existing selected-document operation source, authority/cookie atomicity, permission/geolocation callback continuation, visibility refresh and prompt-resolution receipts remain generated as documented in `G-NATIVE-OPERATION-RECONCILIATION-2026-09-17.md`. This pass inspected the parent override state and native geolocation polling boundaries, including retained acquired positions before V8 allocation and per-callback lifetime checks. It does not replace the DOM/input lane's current completion work.
- The dynamic helper contract still names seven exact loader contexts: Bluetooth conditional script loading, CSP redirected stack traces, CORS 304 scripts, CORP scripts, ranged scripts, stale-while-revalidate scripts and generated anonymous-frame dispatcher code. Generated dispatcher code is not reclassified as a missing URL.

## Remaining requirements and handoff

The two earlier navigation-audit residuals concerning generic ResourceTiming workerStart and inherited redirect exposure now have generated consumers and regression sources above. This is source evidence, not an executed conformance result. Native monotonic stamps are absent on platforms whose existing `timing_clock_micros` producer is unavailable; absence remains explicit rather than substituting wall time.

Root retains CSP hash/preload consumption evidence and shared transport integration ownership. The loading lane retains full preload/resource/cancellation algorithms; DOM/input retains actions and editing semantics; media/rendering retains the remaining media helper and output-close/resize matrix. A complete combined helper closure cannot be declared from this lane's resource getter changes.

Still deferred until the global generation barrier: compilation, authored regressions, exact pinned helper acceptance, normal TLS/focus and cleanup controls, real local/public-IP topology for the 17 blocked Core entries, manual/AT observations, and complete Core/five-family execution. These are not source passes and no parent requirement is checked by this document.

## Full saved operation/helper source reconciliation

This section supersedes any interpretation of the earlier G summary as “mostly qualification.” Following actual consumer calls found additional missing implementation.

The linked [operation source map](wpt-procedures/core-operation-source-map-2026-09-18.json) maps **every saved operation occurrence** to its exact source/line, original inventory row and native route. It also assigns every one of the 1,199 helper occurrence IDs to its original classification. Read-only hashing compared every source with the pinned checkout: **zero mismatches**, across 634 operation-source files and 526 helper-source files. All dynamic-review upstream dependency hashes also matched. Original occurrence inventories are bound by SHA-256 in the new map and remain unchanged.

| Operation family | Occurrences | Source and implementation boundary |
| --- | ---: | --- |
| Actions / send_keys / click / bless | 518 / 336 / 282 / 154 | Exact native selected-document action/element routes; DOM/input lane owns their input/default/lifetime algorithms. |
| Computed label / role | 77 / 72 | Native fresh semantic/layout projection, no historical JS accessibility tree used by official execution. |
| Expanded AX element / node | 3 / 2 | All five calls are `wai-aria/accessibility_properties_basic.tentative.html`: native/ARIA checked state, tablist children, stable IDs, parent, role and label. `webdriver_script_v8::AccessibilityProperties` resolves opaque IDs and recomputes the native graph; existing source regressions cover state mutation, hidden nodes, generated labels and detached stale IDs. |
| Classic permission | 25 | 15 distinct direct Core source files; consumer matrix below. |
| BiDi permission / geolocation override | 2 / 2 | Direct geolocation element error/success fixtures, omitted current context/origin supplied by pinned extra adapter, override reset with coordinates:null. Native parent batch and selected-document guard remain authoritative. |
| Get all / named / delete cookies | 1 / 6 / 2 | Associated native cookie store under exact selected URL/origin/authority; storage algorithm qualification remains independent. |
| Registration mode | 5 | Exact protocol-handler fragment/path/query fixtures with and without SW; native accepted handler substitution shared by navigation. |
| Minimize / set rect / get rect / set test context | 1 each | Native context/window route; minimized pageswap and onmove remain navigation consumers. |
| Conditional Bluetooth names | 30 across 19 names | All helper-only occurrences, no direct-Core calls. Preserve pinned conditional source ancestry; route existence alone is not Bluetooth device implementation. |

Total: **1,516**, comprising **1,486 direct-Core** and **30 helper-only** occurrences. Counts describe source calls, not executed tests.

### Permission and lifetime consumer matrix

Paths below are relative to `html/semantics/permission-element/`, except where stated. The new source map contains exact source hashes and line numbers.

| Concrete sources | Required observable behavior | Current source disposition |
| --- | --- | --- |
| `camera/camera-element-use-cases.https.html`, `camera/camera-set-constraints.https.html` | Native single video track; cross-media/complex-range constraint sanitization; track.stop cleanup | Existing camera consumer and native capture ownership. |
| `camera/camera-error-scenarios.https.html` | Denied trusted click gives cancel + NotAllowedError without track; untrusted click gives error + InvalidStateError | Existing retained failure publication and trusted activation. |
| `microphone/microphone-element-use-cases.https.html`, `microphone/microphone-set-constraints.https.html`, `microphone/microphone-error-scenarios.https.html` | Same contracts for **one audio track**, without requesting camera | **Missing implementation found and generated in this batch**, detailed below. |
| `geolocation-element/granted-selector.html`, `granted-selector.https.html` | Native :granted follows current grants after Permissions.query | Existing parent refresh and native selector state. Microphone eligibility now shares that route. |
| `geolocation-element/get-current-position-{error,success}.html` | Autolocate coordinates/error and exact denial text; native override reset | Existing retained native acquisition and location event; `acquire_element_location` normalizes code1 message to User denied Geolocation. |
| `usermedia/set-constraints-combinations.https.html`, `usermedia/usermedia-set-constraints-sanitization.https.html` | Default both audio/video; missing media constraints enabled; primitive sanitized constraints | Existing standard usermedia consumer and two-track native capture. |
| `usermedia/usermedia-cancel-prompt.https.html` | Denial: cancel only, no stream/error event, NotAllowedError property | Existing retained capture failure/cancel publication. |
| `usermedia/usermedia-iframe.https.html` | Native capture denied without explicit delegation even after top grants | Parent native_capture_features_delegated checks remain separate from grant state. Microphone prompt revalidation now included by root. |
| `usermedia/legacy-mode/legacy-mode.https.html` | Feature-selected legacy isTypeSupported/type/isValid/invalidReason and **no standard stream/events** on click | **Open source/profile obligation.** No UserMediaElementLegacy selector exists. Fixture comments explicitly name a virtual feature suite; standard behavior must not switch by URL. Requires an explicit supported profile or independently reviewed applicability disposition. |
| `html/semantics/embedded-content/the-iframe-element/srcdoc-lifecycle-crash-crbug-1472607.https.html` | requestStorageAccess promise from initial about:blank during srcdoc replacement, no stale host crash | **Missing consumer found:** no requestStorageAccess/hasStorageAccess IDL or JS implementation exists at this inspection. Native generic storage-access authorization test is not this API. Handed to root for storage/lifecycle lane. |
| `fetch/fetch-later/send-on-deactivate-with-background-sync.https.window.js` | fetchLater default/0/60000 requests flush exactly once on BFCache; restored pageshow.persisted; observable server beacon | **Missing consumer found:** no fetchLater IDL or JS/native producer exists at this inspection. Fixture assertions require BFCache flush even with background-sync enabled; comments suggesting otherwise are superseded by its actual assertions. Handed to root for loading/navigation lane. |

### Native microphone implementation and regression sources

- Added `HTMLMicrophoneElement` declaration and tag/constructor/event/setConstraints routing. Pinned microphone IDL declares track/error/events; concrete pinned constraints fixtures additionally require setConstraints, as camera's existing implementation does.
- Shared retained permission-element engine now recognizes microphone, requests only microphone permission, defaults video:false, sanitizes audio primitive constraints, publishes a single native audio track and uses the same trusted-error/cancel/lifecycle receipts as camera.
- Replaced video-specific extraction with `native_media::single_track(expected_kind)`, enforcing exactly one native track of the required kind and preserving original wrapper identity. No author getAudioTracks override supplies capture authority.
- Added `PromptKind::Microphone` and exact one-grant resolution; root integrated changed-delegation revalidation in native runtime. DOM prompt admission and :granted now include microphone.
- Rendering lane integrated microphone UA style, locked-control geometry, private caption, icon projection and microphone-specific prompt caption; original author content remains suppressed. This uses its normal layout/AX production path.
- Regression sources: `microphone_constraints_and_errors_use_single_audio_consumer`, `single_audio_capture_preserves_native_track_identity_and_rejects_video`, `microphone_prompt_grants_only_audio_and_rejects_retired_owner`; rendering expanded locked-layout/private-label tests. These complement retained capture cancellation, interrupted publication, replacement stream, parent delegation and prompt replay tests already generated.

### Helper classification boundary

All 1,199 source occurrences are explicitly covered in the JSON map: 865 parsed attributes/literal loaders; 94 nonexecuting type annotations; 64 authority templates with existing paths; 38 official rewrites; 36 intentional error/nonexistent resources; 21 inline resources; 20 dynamic loader parameters; 14 import-map module specifiers; 11 runtime/server URLs; 8 caller-document-base cases; 6 policy attributes; 6 unused missing includes; 4 redundant harness META includes; 3 documentation examples; 3 root-clamped URLs; 2 inert parsed-markup references; 2 declared fixture repairs; 2 assertion descriptions.

Dynamic source families require real scripts/CORS304/CORP/ranged service replay/SWR/cache isolation, not successful helper loading alone. Caller-document-base cases require captured compilation realm/base; policy attributes require actual CSP/Trusted Types checks. Those loading/policy/lifecycle implementations remain owned by their lanes. Existing source-bound repair and intentional-error classifications do not convert raw failures into passes. Full arbitrary JavaScript reachability cannot be proved by this saved literal inventory; new runtime-reached operations must still fail qualification until mapped and implemented.

No generation gate or parent requirement is closed here. StorageAccess, fetchLater, legacy profile reconciliation and cross-lane behavioral review are **source requirements**, distinct from deferred topology, platform execution and conformance qualification.

## Retained native tree mutation generation (2026-09-18)

Source generated; no build, test, formatter, generator, or service was executed.

The parser's original queued `Insert` could retry `parser_detach` and insertion after a postcommit failure. The retained source now separates the request from its completed prefix:

| Obligation | Source and retained frontier |
| --- | --- |
| Insert and remove structural request | `oryn-dom/src/insertion.rs` `TreeInsertion` / `TreeRemoval`: original owner, rooted identities, former edge, structural commit, captured siblings, and remaining stages; rejected after explicit abort or arena replacement |
| Parser queue ownership | Parser lane `html.rs`: queued `Rc<RefCell<TreeInsertion>>` / `TreeRemoval` installed before resume; retry uses the original receipt; Foster normalizes to Insert |
| Range/iterator mutation | `RangeRemoval` and `node_iterators.rs::IteratorRemoval`: prepared old/new endpoints, per-endpoint publication, independent old-root release, one boundary journal entry, and per-range shadow invalidation; author-modified unprocessed endpoints are not overwritten from stale snapshots |
| Native decrement custody | `RootRelease`: existing root transferred to tracked parser custody without increment; accounting callback consumes exactly once even when reclamation fails afterward |
| Synchronous cancellation | `DomRuntime.pending_cursor_releases`: pre-decrement obligations retained for the next operation/mutation-root checkpoint; remapped alongside counters in `node_migration.rs`, discarded with replaced arena accounting |
| Cross-document ownership | `DocumentAssignment` traverses a retained node frontier; `OwnershipPublication` retains per-node registry association, permission/interest reset, and ownership-journal frontier |
| Styles and connection | `StyleInvalidation` per-node frontier; `ConnectionUpdate` per-node connection, nonce/top-layer, frame generation, focus, and hosted-document notification stages |
| Native atomic leaves | Nonce hiding validates element and materialized Attr before updating both views and its journal; focus validates the old target before replacement; language pragma traversal completes before publishing the final language |
| Radios/options | `RadioReconciliation` retains owner refresh and checked-peer frontier; `OptionReconciliation` retains incoming selected options, peer changes, captured select owner, normalizations, and selected-content cursors |
| Selected content and clones | Parser lane `SelectedContentUpdate` / `CloneCursor`: allocation/Attr/attachment and per-target removal/insertion/publication frontiers; completed normalizations explicitly release nested cursor custody |
| Opaque value ranges | `OpaqueDisconnect`: range-disconnect frontier plus exact former-owner root release, rather than re-querying a disconnected range and losing the old obligation |

Ordinary native append, insert, remove, and parser detach use the same source engine. Superseded `clone_shallow_node` was removed after CloneCursor replaced its callers. Direct low-level arena tree APIs remain available; retained runtime insertion performs their structural writes and fallible tails through the receipt.

New regression sources cover every insertion/removal checkpoint, single structural and journal publication, exact boundary offsets, completed radio effects surviving later failure, iterator/range author changes during recovery, original-arena receipt rejection, wrong-owner admission, raw pre-decrement cancellation custody, and errors after decrement commit. Parser lane additionally generated 48-position Insert and 36-position Detach matrices. Earlier ownership/radio and parser clone/selected-content regression sources cover nested prefix preservation. These are authored tests, not execution results or WPT qualification evidence.

## Parent context-event custody generation (2026-09-18)

Source generated; no execution or conformance result is implied.

`oryn-native/src/context_event_journal.rs` replaces the parent event `Vec` and wrapping atomic ID with one serialized admission/publication journal. Author capacity is 4,096 records, 64 MiB of serialized payload, and 65,536 recipient obligations. Individual events and poll batches are limited to 16 MiB (below the context protocol's 24 MiB frame). There are at most 8,192 document/channel cursor receipts. IDs are checked within the exact JavaScript integer range and assigned in publication order after admission; failed or cancelled admission does not consume an ID.

Author rows are never evicted for capacity. A producer receives typed backpressure before its accepted effect commits. CSP batches admit every violation before publishing their first row. Memory, persistent local, and persistent session storage use `execute_with_admission`: the callback sees the exact change while the mutation/transaction is private. Failure rolls back memory state or drops the uncommitted database write. The caller retains the journal lock through transaction completion and publishes the admitted payload without a second fallible admission.

### Producer and consumer closure

| Caller | Final source route and custody |
| --- | --- |
| Native Window `Request::Storage` | `native_platform_query` → `execute_storage_with_events` → `SharedContextStore::execute_with_admission` → journal commit after store success |
| Context protocol `Storage` | Same helper for Web Storage; existing native IndexedDB coordinator remains its independent route |
| Main/document/redirect/Trusted Types CSP | `handle_csp_violations_for_creator` → one checked event batch, then reporting target work |
| Form CSP | `form_policy_for_navigation_report` first computes every redirected blocked URI, then admits the complete violation batch |
| Worker-originated Window CSP | `report_worker_csp` admits the complete Window event batch; agent-owned workers retain their separate existing CSP event registry |
| Native storage receiver | `Request::StorageEvents` → journal Storage channel, exact `(page, target, generation, epoch)` recipient and validated delivered watermark |
| Native CSP receiver | `Request::CspEvents` → independent Csp channel for that exact recipient |
| Compatibility context poll | `ContextServiceRequest::PollEvents` → Compatibility inspection; **does not release author event custody**. `rust_executable.rs::__oryn_poll_context` explicitly skips storage dispatch and has no CSP violation dispatcher; its report trace uses bounded diagnostic rows |
| Native JS completion | `native_platform/publication.rs` advances storage/CSP cursor only after retained event dispatch finishes; the subsequent native request acknowledges that delivered suffix |
| Legacy `ContextEvent::Message` | No production producer found. Actual postMessage uses native MessageHub custody. Unbound legacy Message admission returns an explicit error instead of guessing a replacement document's identity |
| Report outcomes and page lifecycle | Explicit `record_context_diagnostic`, separate from author event admission; forensic ring policy below |

A storage fanout row retains each recipient independently. An ACK is rejected if it exceeds that document/channel's delivered watermark; a lost response or failed author dispatch leaves the row available. Compatibility inspection can advance its own read cursor without acknowledging either native channel. Its view consists of current pending author rows and the bounded diagnostic suffix, so it is not a complete historical delivery log.

Recipient capture includes native active documents plus unexpired cached `worker_authorities`, validated through retained MessageHub authority. Parked documents keep rows but cannot poll until restored. A successor starts with a new generation/epoch and cannot read or acknowledge predecessor rows. Document retirement/cache disposal, page close, and context worker retirement prune retired obligations; late accepted publication cannot redirect to a successor. Wakeups run only after releasing registry/journal locks.

Diagnostics retain at most 256 rows and 1 MiB, within the global journal bounds. Older diagnostics are pruned explicitly, with counts and bytes in `Evidence`; oversize, exhausted-ID, or late closed-page report publication increments rejection evidence. Author-capacity admission may evict diagnostics with the same counters. Lifecycle close also releases that page's undeliverable author obligations. `context_event_evidence` exposes the bounded/pruned semantics to native diagnostics.

Regression sources cover independent fanout ACKs, response replay, forged/unreceived and cross-channel cursors, compatibility inspection versus both native channels, saturation and capacity recovery, ID exhaustion/cancelled admission, retired versus parked identities, diagnostic pruning, oversized batch rejection, all Web Storage rollback cases for memory/persistent local/session, and persistent commit failure after successful event admission. Root adds a context-protocol integration test for compatibility inspection followed by native CSP delivery/ACK and successor rejection. Existing concurrent-publication and closed-target diagnostic tests were updated for the explicit bounded history contract.

## External template resource closure — source generation in progress

Pinned revision `719d5e38fdd0903a18ed9007aba816c98cc491e0` contains nine direct frozen Core entries in `inventory-v3/core-html-0006.inventory.json`. All nine source SHA256 values match inventory. Exact fixture rows, hashes, and retained September 15 cohort results are in `wpt-procedures/core-external-template-source-map-2026-09-18.json`. Seven baseline rows failed (six harness, one referrer assertion); cancel and CSP negative cases passed. The two negative passes do not establish a working loader: an absent producer also leaves target content unchanged. No denominator or classification changes.

The dependency closure includes `resources/chunked-html.py` (two streamed chunks, delay or stash-controlled continuation), `src-nonce.sub.html.headers`, testharness/report, and preload's `stash-referrer.py`. Required behavior: external scripts do not block the main parser; progressive chunk publication; buffered atomic publication at EOF; removal cancellation; CORS response admission; script-src and nonce; response-byte SRI before any publication; per-element referrer policy.

Generated source so far:

- Parser lane: ExternalTemplateDiscovered preserves the template and main parser progress; retained `with_external_template` progressive marker publication and EOF cleanup.
- Native `external_template_resource.rs`: bounded tracked-element task queue, document generation/cancellation fences, parent-owned response and request cleanup tickets, one outstanding bounded IO response, incremental decoder/parser, retained input/event suffix, script-construction scope, async host poll/delay hooks.
- `context_service.rs`: typed TemplateStreamStart requires document-owned CORS GET, Script CSP destination, nonce/parser metadata on redirect admission, existing CORS/mixed-content/referrer/response integrity pipeline. Protocol does not expose this route through author compatibility dispatch.
- `frame_script_preparation.rs`: main/root/frame shared event admission. `rust_executable.rs`: imported event node validation/remap. Loading lane: HTML Writable event admission and public retained post-connection publication helper.
- Regression source: chunk framing/size, native route invalid authority/method/mode/body, Script CSP versus Connect CSP, accepted/rejected nonce, and full-response integrity. Parser lane owns progressive/buffered failure matrices.

Still open before marking source complete: buffered detached parser publication API integration; distinguish own partial EOF cleanup from authored removal; direct standalone backend support; precise script/event consumption under retained helper failure; retired/parked queue and seen-root reclamation; end-to-end loader fault/cancellation regression sources. All implementation and tests remain unexecuted under the global source-first directive.

### External-template follow-up source integration

Buffered/SRI modes now select parser lane's `with_external_template_buffered`, which parses into detached staging and retains one marker publication at EOF. `external_template_cleanup_started` distinguishes native cleanup retry from author removal. Fragment event publication now transfers custody through admission-only `post_connection_scripts::enqueue`; the producer pops each event only after accepted transfer, and shared post-connection polling owns later failures. Root retirement prunes queued tasks and document-aware discovery keys. Request completion and response cleanup have separate tickets; response cancellation remains attached before cross-thread publication.

Added host-driven regression sources for both buffer modes, old-target preservation when template is removed before delivery, document.stop cancellation, and duplicate discovery. Rendering lane is generating portable Broker/Local transport plus CORS/SRI/cancellation sources; these tests are intentionally written before running any code. All new external-template source remains unbuilt and unexecuted. Standalone transport and its integration, full timing provenance, and sustained failure/parked-state review are not yet claimed complete.

### External-template bounded task: source generation closed

- [x] Nine pinned Core fixtures, exact source hashes, helper hashes, and retained cohort rows mapped; no denominator adjustment.
- [x] Parent and portable Broker/Local transports integrated. Broker owns progressive chunks, request cancellation, redirects/CORS/CSP/nonce/referrer/SRI; Local requires exact same-origin source identity and does not invent byte-integrity evidence for decoded text. Native data URLs retain bytes for SRI.
- [x] Attribute custody frozen at fetch admission: `ExternalTemplateOptions` captures marker name and sanitizer mode; captured progressive/buffered constructors never rewrite later authored attributes. Source regression mutates both while the response is pending.
- [x] Progressive parser and detached buffered EOF publication integrated, including retained constructor/mutation/event suffixes and original node identity.
- [x] Event handoff uses admission-only post-connection enqueue; later fallible polling owns the accepted batch. Loading lane generated injected-poll-failure no-recapture regression.
- [x] Cancellation on removal/document stop/retirement, queued response preservation while parked, observed suspension excluded from publication deadline, exact request/stream identity and bounded metadata/chunks/body custody.
- [x] Parent and standalone native timing provenance; parent stream timing waits for body terminal or cleanup acknowledgement, with a no-duplicate-cleanup regression.
- [x] Source regressions: buffer modes, progressive boundaries, marker cleanup, buffered script target visibility, frozen marker/sanitizer, removal/stop/park/dedupe, CORS/nonce/SRI/referrer redirects, cancellation/foreign identity, native parser staged failure matrices.
- [ ] Build, tests, pinned WPT execution, runtime conformance results: intentionally deferred until global source generation is complete.

Files: native `external_template_resource.rs`, `external_template_transport.rs`, exact `host.rs` include/hook/poll/delay/retire blocks, `frame_script_preparation.rs` discovery arm, `rust_executable.rs` event remap arms, `context_service.rs` typed route/CSP arguments/regressions, `resource_timing_scope.rs` stream observation; DOM `html/fragment_stream.rs` captured APIs, `html/partial_updates.rs` captured marker admission, `html.rs` option re-export; JS admission-only callback generated by loading lane. This completion statement concerns source generation for this bounded resource feature, not verified Core conformance.
