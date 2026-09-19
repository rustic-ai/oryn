# Frame preparation continuation source inventory — 2026-09-16

Generation only. No build, test, formatter, binding generator, deployment, or conformance run has been executed for these changes. These bounded rows do not close R-06 or the full generation gate.

## Generated production paths

- [x] Initial `finish_parser_resources` and dynamic `drain_dynamic_frames` no longer iterate `run_script_record` synchronously over child scripts. Both construct `FrameScriptPreparation`; `PreparedParserRealms` transfers initial batches into `RustExecutableDocument`.
- [x] Frame external classic/module source jobs retain native script leases and run through `ParserResourceScheduler::schedule_script`/completion publication. Module graphs retain `ModuleGraphLoad` until ready, set `graph_preloaded`, and therefore bypass the legacy synchronous graph loader on these frame paths. Source bytes, graph slots and graph fetch registry reuse `DynamicScriptScheduler` page limits.
- [x] Frame ordered/deferred execution and async execution have separate queues. Async discovery follows preceding ordered records/import maps. Ordered/deferred completion permits DOMContentLoaded; remaining async graphs/sources retain the frame load barrier. Parser-discovered frames retain lifecycle authorization until parent completion has installed observers.
- [x] Frame stylesheets and CSS imports now use retained `ParserResourceScheduler` jobs. Both calls to `load_frame_style_resources` and the superseded helper were removed. Native media/disabled/alternate/revision applicability gates execution of ordered scripts; external script and module preparation can progress while applicable CSS is pending. Preparing-frame stylesheet mutation callbacks route into this owner.
- [x] Dynamic frame GET/POST resource requests now execute through `frame_resource_preparation::schedule_frame_resource` on the bounded resource IO executor. Missing initial snapshot frame resources reschedule into the same production path. Native initiator, form authority, body, encoding and referrer metadata are captured before yielding.
- [x] Request/response byte reservations and context resource ownership survive worker execution/response retention. Worker failure, executor rejection and disconnected producers have terminal completion paths. Response publication checks exact parent document/generation/stream, native frame connection and source/sandbox/referrer attributes. Native rescheduling supersedes even a ready same-URL response, resolving wrapper scheduling realms to the native owner first.
- [x] Frame script execution validates native parent/frame/child linkage plus exact child document/generation/stream. Stop, discarded frames and stale completions release local queue/graph/node custody; already-running resource workers retain their ownership until terminal completion.
- [x] `checkpoint`, `has_pending_page_work`, `has_runnable_page_work`, `flush_document_loads`, native stop and discarded-realm cleanup consume the new queues. Polling is bounded and frame/async script queues rotate across tasks.

- [x] Frame HTML/text source now commits an empty native child shell, then drives a retained `LiveParserState` with selected-document `StreamingParser::with_document`/`with_document_text`. Child script insertion points and document.write retain the child root; shared-arena top-document assumptions were removed from parser identity, encoding, tokenizer replacement and buffered publication helpers.
- [x] Frame XML uses `XmlDocumentParser::with_document` into the selected native child Document, yielding after 256 events or each script/resource event. Native XML script/style/PI/frame/meta/handler consumers preserve close-tag script visibility, script checkpoints, deferred EOF eligibility and async load ownership. Normal final tree import was removed on this frame path; a fatal XML parser error alone replaces the tree with the small parsererror document. HTML parser async sources/modules transfer into the frame owner, keeping DOMContentLoaded separate from load.
- [x] General `drain_dynamic_stylesheets` now schedules owned `StylesheetPreparation` jobs for external/inline sheets and CSS-bearing resource attributes. The worker retains request/response byte reservations and context ownership; publication checks native realm/document/stream/owner/revision. Pending applicable sheets participate in root script and load barriers.
- [x] Native XML stylesheet pseudoattributes, prolog ownership, stylesheet invalidation/media/title/alternate sets, CSSOM owner identity, PI.sheet and production layout publication are integrated. Raw substring PI classification was removed. StyleSheet metadata fields are in IDL source; generated bindings remain deferred.
- [x] Parser CSP/referrer metadata and dynamic referrer mutations use `InstallRealmMetadata` with native realm Creator. Child parser starts after native policy commitment; dynamic child metadata no longer gets discarded or sent as root metadata. Parent authority/enforcement is owned by P/W.

- [x] Typed frame Location document navigation accepts data/blob through the retained parent resource path and explicit about:blank through an inherited local shell, preserving fragment URLs. Nested parent encoding now comes from its selected native Document. Broader scheme policy/redirect execution remains unverified.

- [x] Live-parser frame response maps use exact native NodeId owners, not URL keys. `ParserFrameOwner` retains native document/generation/stream/connection, source/policy attribute snapshot and an identity token refreshed on attribute mutations and explicit rescheduling, rejecting attribute ABA and same-URL supersession. Request/response bytes share the page source budget. The same navigation token also guards dynamic frame fetch queues and retained ready responses. Parent-context paths cannot fall back to author resource-map bodies without a native response.
- [x] Page and child LiveParser Continue/Waiting consumers publish up to 64 ready child responses before parent EOF. Consumed discovery events are removed so EOF cannot replay a policy ticket or overwrite subsequent child navigation. Child lifecycle is independently eligible; native parent load barriers still observe its pending child work.

- [x] Root XML collection transfers at decoded EOF into the same live document preparation consumer, preserving native Document identity and response encoding. Root parser pending/runnable/stop/history/readiness/load integration is explicit. `XmlParseStep::LiveComplete` finishes without copying/importing the full native tree. Existing XML stop regression now checks that the already published prefix remains while later publication stops.

- [x] Parser frame failures retain an exact terminal owner and diagnostic through discovery consumption. EOF cannot implicitly retry the failed request or borrow another frame response. Terminal errors now flow to an inert about:neterror document with a fresh opaque native owner, restrictive inherited policy plus default-src/form-action/base-uri denial and no-referrer, disabled forms/modals, and the normal frame load lifecycle. The same path handles a provisional frame and replacement of a prior loaded context while preserving WindowProxy identity.

- [x] TLA source audit follows pinned `microtasks/evaluation-order-4.html`: module evaluation settlement is not added as a script/document load barrier. Ongoing evaluation uses native module promise custody; its rejection reporter now checks native execution retirement before reporting. New sources cover delayed module continuation after load and suppression of rejected retired-frame evaluations.

## Regression sources generated

`frame_script_preparation.rs`:

- `frame_preparation_retains_ordered_records_and_document_open_cancels_suffix`
- `frame_preparation_defers_defer_until_classics_and_keeps_native_barrier`
- `asynchronous_frame_module_waits_without_blocking_domcontentloaded_and_stop_releases_it`
- `frame_stylesheet_barrier_uses_native_media_disabled_and_revision_state`
- `frame_live_tokenizer_hides_suffix_and_document_write_uses_child_insertion_point`
- `frame_xml_parser_yields_and_publishes_only_its_native_child_document`
- `dynamic_frame_referrer_metadata_uses_native_child_creator`
- `frame_location_about_blank_commits_inherited_empty_document`
- `xml_frame_scripts_run_at_close_tag_before_suffix_publication`
- `same_url_parser_frames_keep_distinct_owners_and_publish_before_parent_eof`
- `parser_frame_owner_rejects_attribute_changes_and_reconnections`
- `parser_frame_failure_is_terminal_and_cannot_retry_at_eof`
- `root_xml_uses_live_script_resource_owner_and_preserves_encoding`
- `retired_frame_top_level_await_rejection_cannot_report_to_replacement`
- `frame_top_level_await_does_not_hold_script_or_document_load`

`frame_resource_preparation.rs`:

- `frame_resource_replacement_discards_ready_result_and_shared_byte_custody`
- `pending_frame_resource_gates_parent_load_and_stop_cancels_local_publication`
- `frame_resource_disconnect_becomes_one_runnable_terminal_failure`
- `native_frame_resource_producer_yields_and_retains_post_and_initiator_until_reply`
- `failed_frame_navigation_commits_opaque_document_and_one_load_for_new_and_existing_contexts`

`dynamic_stylesheet_preparation.rs`:

- `dynamic_stylesheet_ready_custody_and_replacement_use_native_revision`
- `css_resource_attribute_completion_requires_exact_native_attributes`
- `xml_pi_completion_reaches_cssom_media_and_production_layout`

`oryn-dom/src/xml.rs`: `live_xml_yields_at_script_close_and_completes_without_tree_snapshot`

`oryn-dom/src/xml_stylesheets.rs`:

- `xml_stylesheet_pseudoattributes_decode_xml_and_reject_invalid_grammar`
- `xml_stylesheet_owner_requires_prolog_and_native_sheet_metadata`

Existing production regressions continue to exercise frame removal during script bootstrap, initial/dynamic frame discovery, child load ordering and frame history. They have not been executed against this generation.

## Explicit residual full-inventory work

- Root and frame live XML production consumers are generated. `BufferedNavigationInput` transfers the selected-document XML parser at source EOF into the shared document preparation owner; root readiness/history/stop/load paths consume it. `LiveComplete` avoids a final whole-arena snapshot/import. Detached XML parsing remains for explicit parse APIs/direct buffering compatibility; fatal parsererror replacement remains bounded by its small generated tree. Full XML encoding/entity/parsererror and script cancellation matrices still require source reconciliation and eventual execution.
- Exact native frame response ownership and earlier child publication are generated. Terminal failure/no-implicit-retry and ABA/reconnection sources are present. Opaque error-document/load paths and two-context regression sources are now generated, using pinned frame-src-blocked/redirect-to-about expected behavior. Redirects, parent interruption and independently queued navigation still need full caller/spec matrix reconciliation and eventual execution.
- Full task-source ordering, top-level-await/module evaluation completion, resource cancellation/redirect matrix, frame render opportunities and pinned conformance remain open at the R-06/package level. Already-running resource workers retain custody to terminal completion; no per-job parent abort protocol was added. The legacy synchronous module helper remains for other callers.
- Generic CSS resource jobs now capture and recheck their native resource-bearing attributes in addition to stylesheet revisions. CharacterData mutations also initialize preferred PI stylesheet sets through the shared native mutation path. Imported CSSOM parentStyleSheet semantics and broader V obligations are not established by outer-sheet metadata support.
- The normal generated bindings/build/test/fix/complete Core and five-family verification phase remains gated on all required source generation.

## Native viewport and frame navigation destination integration (source only)

- Generated native replaceable Window.innerWidth/innerHeight bindings and IDL, exact parent content-box viewport propagation (excluding transforms/border/padding), current child media-query evaluation and coalesced trusted resize tasks. Initial about:blank receives a forked layout provider synchronously. LiveLayoutProvider now retains its selected native Document rather than implicitly querying the shared arena root. Production initial/dynamic frame and checkpoint callers are wired; viewport source regression covers transform, parent resize and author replacement of innerWidth.
- FrameNavigationContext now retains accepted NavigationHistorySnapshot, native history, replace/reload flags. FrameResourcePreparation transfers that context with its exact document/connection/attribute/navigation-token completion, so asynchronous resource loading cannot silently discard accepted state.
- Native host snapshot_history accessor reads binding-owned history without author property lookup. Dynamic frame commit captures old history before retirement, retains accepted destination key/id and independent Navigation state/History state, updates final redirected URL, synchronizes history and activation before child script preparation. Same-origin push/replace/reload/traversal preserves the visible history segment; foreign/opaque commits do not expose old entries.
- Added source regressions: frame_navigation_api_destination_survives_native_commit_with_separate_history_state; frame_resource_completion_retains_native_navigation_context; accepted_frame_destination_preserves_identity_and_forward_traversal; cross_origin_frame_commit_does_not_disclose_old_entries; frame_push_after_traversal_truncates_only_forward_entries.
- Remaining explicit scope: frame cross-document back/forward production traversal routing and complete frame pageswap/pagehide/pagereveal ordering; whole lifecycle/task/redirect/cancellation/render matrix remains open. These additions do not close R-06 or S/V globally. No build, test, formatter, generator or conformance execution performed.

### Follow-up frame history routing / transition source

- Frame drain now consumes native history traversal deltas and routes the selected native destination through the authenticated navigation backend (self Creator/source metadata), retaining accepted history identity through the frame loader. Added frame_history_back_routes_native_destination_and_swap_before_hide source regression.
- Replaced child receives trusted pageswap then pagehide before retirement. Native accepted destination is staged for swap activation; foreign/opaque destinations suppress prior activation disclosure. Handler-induced typed navigation, owner removal/reconnection or native token change aborts obsolete commit. Existing native load delivery owns pageshow; executable render phase owns subsequent pagereveal before rAF, avoiding eager reveal.
- Actual initial empty child is stamped initial=true; parser/dynamic commits are stamped false, including initial-realm adoption, before scripts/history publication.
- Residual: history restore of srcdoc/non-network archived document content and full frame history serialization/BFCache remain distinct from URL-backed traversal routing. NavigateEvent behavior for history.back traversal and full multi-document traversal/reentrant lifecycle matrix need further reconciliation; no claim of full history conformance.

### Archived inline history and traversal producer reconciliation

- FrameRealm retains inline srcdoc source with shared queued-source byte reservation. Departing native document IDs index archived source/form state, independent of author src/srcdoc attributes; native history selection recovers only that exact document ID. Reconstructed srcdoc retains its original inherited base URL, passes current native owner policy, restores form state at parser lifecycle completion, and reports persisted=false. Storage is bounded to 1024 native entries and the shared 32 MiB byte limit, pruned on forward-history truncation and lost native frame ownership. Network history continues through authenticated loading rather than replaying unvalidated response bodies.
- Generated source regressions: srcdoc_history_restores_native_source_after_element_attribute_is_removed; frame_pageswap_removal_prevents_obsolete_document_commit; frame_pageswap_new_location_prevents_obsolete_document_commit.
- History.back/forward/go now dispatch native NavigateEvent with destination/traversal identity before same-/cross-document commitment; cancelled/superseded dispatch cannot queue obsolete navigation. Intercepted traversal uses existing native commit machinery. History.go(0) routes native reload. Native History URL helpers no longer read author Location properties; URL rewriting now implements scheme/credentials/host/port and non-HTTP path/query constraints, including srcdoc/blob fragments and about:blank query rejection (pinned history_pushstate_url_rewriting.html).
- Structured frame history update consumer verifies selected native id/document_id before changing committed URL; legacy URL-only update path removed by root.
- Independent live-frame BFCache is **not claimed or required by inspected evidence**. Whole-root retained isolates preserve child contexts. Separate frame-context-reattachment.patch and C++ regression source remain unintegrated/unbuilt; P/W generated native frame park capability handlers remain unconsumed pending a concrete independent-frame cache requirement. No per-frame execution freeze or live realm reattachment is asserted. Root requested preserving these pending sources while prioritizing required Core behavior.
- Remaining source review: complete archived-document navigation policy/history serialization across root restoration, built-in versus FACE persisted state timing, whole-root frame transition tree ordering, and complete selected Core negative/lifecycle matrix. No builds/tests/formatters/generators executed.

### Full native frame history versus visible Navigation entries

- Added FrameRealm SessionHistory containing full native entry list/cursor and per-document native origins. History traversal now selects from full custody; contiguous same-origin Navigation visibility includes inherited about:srcdoc/about:blank using captured native origins rather than opaque URL reparsing.
- Structured same-document visible updates merge into full history while preserving hidden prefix/suffix except forward truncation. Native `sync_session_history` publishes full History.length/cursor/state without creating foreign NavigationHistoryEntry objects; normal sync_history preserves hidden HistoryRecord portions during synchronous author mutations.
- Public admit_native_history_traversal accepts exact native snapshot, handles hidden foreign destinations without exposing them to source NavigateEvent, reuses existing visible traversal admission and returns exact token/snapshot for cross-document loading. P/W owns host wrapper integration; root owns full-page caller.
- Source regressions frame_session_keeps_hidden_cross_origin_history_outside_visible_segment and inherited_srcdoc_uses_native_origin_for_history_visibility generated. Tests/builds remain unexecuted.

## Frame request and persisted-control generation follow-through

Source generated, not built or executed:

- Native full frame session history retains POST bytes/content type and sealed form authority by document ID through `FrameRequest`; shared page byte reservations survive clones until last owner. Final response `effective_post` reflects redirect rewriting before capture. Traversal producers replay retained metadata through `FrameNavigationContext` and parent-checked `LoadPageResource`.
- Typed `SavedFrameNavigation` serializes full entries, native visibility metadata, archived srcdoc/base/control state, and request bodies. Descriptor validation rejects duplicate entries, duplicate/orphaned document custody, excessive controls and bytes. Capture prunes descriptors against retained entries. Serialized origins never grant realm/network authority.
- Reserved `oryn:frame-history` consumer runs before child parser continuations. `restore_pending_frame_before_scripts` selects the saved source/request by native document ID, cancels original descendant preparations, and stages required replacement before their scripts run. Matching existing source receives native history before scripts; controls restore at parser lifecycle eligibility. Root producer/capture timing remains root-owned.
- Built-in input/textarea/select persisted values, checkedness, selected options/files, and dirty/user-validity state now share native FACE mapping and bounded snapshots. Restoration uses native setters without input/change dispatch, respects autocomplete=off, and retains delayed FACE definitions. Exact selection/caret restoration and the complete pinned lifecycle timing matrix remain unclosed; this is not whole-package qualification.
- Parent frame resource handling quarantines raw attachment/eligible download bytes using the original native initiator before decode/commit. HTTP 204/205 and downloads return `PageResourceNoCommit`; dynamic consumer preserves current realm and emits no replacement load. Initial parser consumer acknowledges without publishing a response document.
- Parent-issued `PageResource.native_origin` now survives parser `LoadedFrameResource` and dynamic response custody into pre-script realm creation through `native_frame_response_origin`. Effective target sandbox still produces distinct opacity. P/W owns local response native policy ticket generation/binding.

New regression sources: `native_builtin_form_history_restores_without_input_or_change_events`, `reserved_frame_history_replaces_srcdoc_source_before_first_child_script`, `frame_history_post_custody_shared_until_last_owner_and_redirect_semantics`, `frame_response_native_origin_retains_opaque_grant_identity`, `no_commit_frame_response_preserves_current_document_without_load`, `frame_history_serialization_preserves_binary_body_and_sealed_source`, `frame_history_rejects_orphaned_and_duplicate_request_custody`.

Remaining explicit generation audit: full selected transition-negative matrix (nested restoration, redirects plus cancelled/superseding lifecycle handlers, delayed control definitions and exact restoration-event timing), producer-side aggregate accounting before archive/control clones, caret/selection state, and complete source/caller reconciliation. Optional independent live iframe BFCache remains unintegrated and is not claimed required by inspected Core evidence. Whole-root retained-child lifecycle is root-owned. No test, build, formatter, generator, deployment, or conformance command was run.

Follow-up source closure: built-in text/textarea history now captures UTF-16 selection start/end/direction from `text_control_selections`, restores through that native map with live-value clamping and allowed-direction validation, and extends the built-in restoration regression. This supersedes the caret/selection gap above for supported native text controls. Full transition timing/negative matrix and pre-clone aggregate archive accounting remain open.

## Archive accounting and restoration admission follow-through

- [x] `ArchivedSource::capture_forms` reserves available shared page capacity before native form-state capture; `capture_form_history_in_realm_limited` forwards the remaining-byte ceiling into FACE/built-in producers. The lease shrinks to actual payload size and remains with archived forms. Replacement failure preserves old forms and custody.
- [x] `ArchivedSource::clone_forms` validates and reserves before copying saved controls/files; source reservations include retained base URL. `capture_child_form_history` passes remaining output capacity, and `capture_frame_navigation_history` preflights source/body/control serialization copies. Built-in entry accounting includes actual entry record size.
- [x] Early descriptor binding (`bind_pending_frame_history_owners`) records exact parent/frame/navigation-token ownership when native child creation publishes its owner. Pre-script consumption checks that claim; a removed/reinserted or superseded frame cannot claim the saved descriptor by path coincidence.
- [x] Restaged control delivery retains the exact native navigation token, excludes the stopped outgoing same-URL document, consumes the claim before author reactions, and discards a committed redirect mismatch instead of waiting for a future URL match. Pending parser controls attach to the existing lifecycle restoration slot rather than restoring into a partially parsed document. Already committed native history is not overwritten after child scripts.
- [x] Every new frame Document takes its fresh native history document ID while accepted entry key/ID/state survive. Serialized restoration remaps the selected entry to the new native document group. Old same-document entries retain their original origin mapping rather than becoming part of a redirected document's group.
- [x] Inherited native worker-policy publication now sends `state.current_location_url()` through `InheritFrameWorkerPolicy.document_url` into P/W's `inherit_frame_worker_policy_at_url`; actual about:srcdoc/about:blank/error URL no longer implicitly becomes the parent's URL.

Additional regression sources: `archived_controls_reserve_before_clone_and_release_with_source`, `removed_and_reinserted_frame_cannot_replay_claimed_history`, `redirected_claim_is_terminal_instead_of_waiting_for_future_url_match`, `stopped_prior_document_cannot_consume_same_url_restoration`. Existing pre-script srcdoc restoration now asserts preserved entry ID and fresh document ID.

Remaining audit, not checked: allocation peaks from existing metadata/session snapshot cloning and temporary descriptor matching; complete selected transition matrix beyond the explicit cases listed (especially nested replacement, reentrant FACE restoration and browser-equivalent restoration timing). A pinned-tree search found `formStateRestoreCallback` only in CustomElementRegistry.html, so no uninspected pinned timing test is claimed covered. Earlier broad allocation/timing gaps are only closed for the exact production paths above. No tests/builds/generators executed.

## Shared session metadata and nested/reentrant source pass

- [x] `SessionHistory` shares immutable full entries, origins, and request maps through `Arc`; `FrameRealm`/`FrameNavigationContext` cloning no longer deep-copies entry state or maps. Metadata leases remain shared until the last owner drops.
- [x] `merge` derives visible bounds without materializing an old visible snapshot, checks entry capacity, and reserves new metadata before cloning retained prefixes/suffixes. Failed reservation preserves the previous snapshot. Cross-document publication reserves before copying prior history/maps; restoration reserves before saved history copies. Reservations include entry/state payloads and conservative map/identity overhead.
- [x] `SessionHistory::sync_native` reserves visible and full native publication copies before the first update. `snapshot_history_limited` inspects native borrowed entries and their payload sizes before cloning, avoids a duplicate current-entry copy, and supplies the frame producer's remaining output ceiling. Eager fallback history cloning was removed from the frame traversal consumer.
- [x] Claim keys now contain only stable tag/id/name/occurrence metadata; archived srcdoc source is not duplicated in each claim key.
- [x] Authorized native parent-history replacement clears only its old descendants' initial claims so nested saved documents can bind to the new native parent. Ordinary author movement/replacement cannot clear those claims. Nested saved-source and reentrant FACE-removal cases have regression sources.

Regression additions: `frame_session_clones_share_history_and_failed_merge_preserves_old_snapshot`, `native_history_snapshot_rejects_capacity_before_owned_snapshot_and_keeps_current`, `nested_saved_source_rebinds_only_after_native_parent_restoration`, `face_restore_removing_its_frame_cannot_deliver_following_old_callbacks`.

Source-review boundary: these rows close the identified retained/temporary session-copy and nested-claim omissions, not empirical conformance. The explicit transition matrix now includes initial/matching/restaged/nested source restoration, preserved entry identity with fresh document groups, native opaque response grants, redirect mismatch, stopped outgoing same-URL documents, removed/reinserted owners, delayed FACE definitions, reentrant frame removal, failed metadata admission, and no-commit responses. Browser-equivalent ordering across the complete selected Core corpus still needs the authorized post-generation build/test/conformance phase. Root owns remaining direct/standalone blocking module-graph consumer removal (preload_module_graph/run_script_record), separately handed off with exact call sites. No tests/builds/formatters/generators were executed.

## Required HTTP frame streaming and live XML input — generated source, unexecuted

- `LoadPageResource` now publishes HTTP frame response headers with `PageResource.stream_id` and the prepared policy grant; HTML/XML/text body collection is removed from that response path. Redirects retain method/source policy checks; 204/205 and download responses remain no-commit. Download/non-parser MIME fallback still intentionally collects bounded raw bytes.
- `SocketLifecycle::bind_frame_stream` binds the pending stream to the exact committed child Creator under retirement locks. Before binding no read is authorized; old/source/foreign document reads fail. Child retirement and page shutdown reclaim the bound stream. Stream registry admission failure releases the prepared policy.
- `ContextFrameResponseLease` is reserved before request and attached before completion capacity/staleness checks. `ReleaseFrameResponseStream` validates its policy grant, cancels body custody, and releases any unconsumed prepared policy. `ResponseGuard` independently owns both stream and policy until delivery receipt.
- `frame_response_stream::DecodedFrameInput` reads on bounded resource IO lanes, never the page actor; raw chunks are capped at 64 KiB with encoded-size admission, fixed decoder byte reservation, parent aggregate 16 MiB body cap, and retained 30-second frame deadline. A frame read publishes an available prefix without waiting to fill 64 KiB. Dropping/stopping parser input requests cleanup immediately even while a read clone is active.
- Initial parser and dynamic frame completion consumers both call `FrameScriptPreparation::start_document_stream` only after native child policy binding, passing the exact child Creator. Existing frame script jobs still prepare external source/graphs before execution; no context-backed synchronous script fallback is required.
- Root and frame XML parsers accept decoded prefixes before EOF with WouldBlock suspension, preserving partial XML lexer state and Unicode scalar boundaries. XML scheduling polls only on demand, uses cumulative source limits, and keeps stylesheet/script ordering through the existing owner. HTML parser feed slices cap oversized tokens; parser `document.write` and queued close commands have bounded admission. Selected-document clearing preserves parent DOM.
- Decoder early publication requires a valid parsed transport charset or complete XML declaration, preserving HTML meta prescan when charset text is invalid or occurs inside an unrelated quoted MIME parameter.

Regression sources added/reconciled (not executed): `frame_headers_and_prefix_publish_before_eof_only_after_native_child_binding`, `rejected_frame_stream_receipt_reclaims_stream_and_prepared_policy`, `frame_response_release_is_grant_scoped_before_native_commit`, `decoded_http_frame_prefix_enters_child_parser_before_terminal_chunk`, `frame_chunk_bounds_and_terminal_empty_are_distinct`, `incremental_transport_charset_and_xml_publish_before_eof`, `invalid_or_quoted_transport_charset_does_not_skip_html_prescan`, `root_xml_executes_script_before_network_eof_and_idle_input_does_not_spin`, `streaming_xml_resumes_every_scalar_split_without_premature_eof`, and `xml_large_single_text_event_yields_by_bytes_without_losing_unicode`. Existing service-worker navigation integration source now binds and reads the granted child stream instead of asserting a fully buffered frame response.

Residual scope remains explicit: root response-header/redirect wait must move to actor-owned pending fetch state (root-owned implementation); root standalone/compatibility consumers remain under root reconciliation. This slice does not establish test passage, conformance, the full selected transition matrix, or whole R-06 completion. No build, tests, formatter, generator, or conformance execution occurred.

## R-06.k follow-up source audit

Generated fixes:
- `FrameScriptPreparation::accept_xml_events` now performs the native microtask checkpoint before script preparation, matching pinned `html/webappapis/scripting/event-loops/microtask_before_prepare_the_script_element-02.xhtml`. It then rechecks the exact live Document, connectivity and already-started state, captures post-observer attributes/source, and leaves inert/data/empty script elements unstarted. Mutation observers can change a would-be classic/module/async/defer script to data before fetch/execute admission.
- `LiveParserStep::InputWaiting` in frame preparation now marks parser readiness waiting. Pending HTML response input no longer advertises immediate runnable work and spins; normal page idle polling still resumes it when data arrives. XML already distinguishes input demand from runnable parser work.
- The old XML parser regression's `Cursor::position` assertion was reconciled to owned `XmlInput.offset`.

Additional unexecuted regression sources: `xml_observers_run_before_script_preparation_and_can_make_script_inert`, `xml_document_write_rejection_preserves_parser_suffix_and_script_order`, `malformed_xml_tail_cancels_queued_detached_deferred_scripts`, and `idle_html_frame_input_is_pending_without_runnable_spin`.

## Native tokenizer reentry and execution-entry source generation

The earlier queued-write/full-tree-close omissions are superseded by these generated paths:

- `DocumentParserGate` owns the live HTML tokenizer shared by `LiveParserState` and binding `Document.write`. Split tokens and nested inline scripts are processed synchronously; outer suffixes remain local to their invocation so recursive writes cannot consume them prematurely. External/module/style/native metadata boundaries return typed events and retained input to executable preparation. Native handler attributes are installed synchronously without delaying following markup.
- `Document.open` creates a native owned gate. `Document.close` finalizes a locally complete gate or signals EOF to its transferred executable continuation. `FrameScriptPreparation::written` consumes root/child owned gates through the same resource scheduler and shared page limits. Detached documents retain an inert tokenizer between calls. The previous `ParserInputCommand` queue, full-tree close replacement, recursive script scanner, and currentScript insert-after fallback are removed.
- Gate input admission uses a process aggregate 32 MiB reservation retained until completion/abort/drop, a 1 MiB write-call limit, 1024 retained record limit and depth 32. Epoch/Document checks reject replaced execution owners; mutation observers run before inline preparation and are followed by fresh owner/connectivity checks.
- Inline gate CSP uses a callback installed with the actual native realm endpoint before first author code (including initial empty children). Each call derives the current stable Creator; parent `AuthorizeInlineScript` receives `creator: Some(...)` and `parser_inserted: true`.
- `DestructiveWriteGuard` holds the original binding through external and module execution, restoring the native ignore-destructive-writes counter on every return. `run_script_record`, dynamic script completion and host module evaluation install it. Parser-blocking insertion points remain available; async/deferred/module writes without an insertion point are ignored. Explicit open creates a writable parser. Immediate module microtasks execute under the counter; later promise/timer continuations run after its release, as required by pinned module-write cases. Dynamic-import evaluation is intentionally not globally suppressed (pinned `module-dynamic-import.html` expects a later destructive write).

Unexecuted regression sources: `parser_write_is_visible_before_return_and_preserves_split_tokens`, `nested_inline_writes_execute_before_each_caller_resumes`, `parser_written_external_script_keeps_remaining_writes_before_network_suffix`, `written_child_document_uses_its_own_tokenizer_and_is_visible_before_close`, `detached_document_write_keeps_tokenizer_between_calls_and_scripts_inert`, `external_defer_write_is_ignored_but_parser_blocking_write_is_inserted`, `module_write_counter_covers_immediate_jobs_and_expires_before_later_tasks`, `explicit_open_during_external_execution_creates_a_writable_insertion_point`, `parser_written_handler_attributes_do_not_delay_following_markup`.

Confirmed residual source obligations (whole R-06.k remains open):
- Resource discovery now retains up to 1024 typed events separately and continues synchronous ordinary markup; the next script pauses until native policy/style/frame consumers process earlier events. Regression `parser_written_style_discovery_preserves_synchronous_markup_before_later_script` covers visibility and subsequent execution. Written frame events now invoke the existing scope-native DOM lifecycle hook synchronously, creating the initial child Document before write returns; remote loads retain parser ownership, local loads retain their existing deduplicated dynamic queue. Regression `parser_written_frame_has_initial_native_document_before_write_returns` covers child open/write.
- Detached inert tokenizer input now uses `DetachedParserInputBudget`, sharing the active-gate 32 MiB aggregate with per-document 1 MiB admission and RAII release on close/replacement/drop.
- Gate collection no longer marks ownership claimed prematurely: successful written preparation explicitly claims it after native frame lookup/admission. Provisional/unadmitted gates remain available for retry. Regression `transferred_written_parser_reopen_retires_old_owner_and_keeps_new_stream` now covers close/open after executable adoption; `detached_input_admission_rejects_before_growing_retained_usage` covers detached admission rollback. The broader selected XML script/style lifecycle matrix remains unqualified until the authorized all-generation-first build/test phase.

No build, tests, formatter, generator, or conformance execution occurred.

Audit correction: root inspected `network_identity.rs` and confirmed `SharedNetworkBroker` already retains cancellation custody through receiver closure and acknowledged cancel retry. The earlier root-header audit suggestion to add a duplicate body lease is superseded; terminal adapter errors should close the receiver immediately, which root owns.

## Root actor navigation continuation source pass

Source generation only; no build, test, formatter, generator, or conformance execution.

- Generated `navigation_actor.rs` retained begin/poll transactions for browser, script, history, refresh and controlled navigation. `page_actor_turn` dispatch attaches typed pending replies to exact transactions; no pending operation returns a fabricated `NavigationResult`. HTTP policy/header/redirect, download authorization/body/publication, and local response preparation advance through bounded IO producers with one retained deadline. The previous synchronous V8 navigation implementation is excluded from production V8; its test adapter drives the retained state machine.
- Captured source Creator, native load-cancellation epoch, accepted Navigation token/destination, history entry identity, POST/form/source/service authority, cookie/referrer metadata and reply remain owned through completion. Supersession, Stop, Close, closed caller and deadlines reject the retained operation. Final remote ACK settlement rechecks current document identity and native load-cancellation epoch.
- Controlled navigation retains `PendingNavigationAck` across authority checkpoints and ordered terminal flush. Postcommit readiness refreshes cancellation with exact emitter identity. Strategy None acknowledges after Started/first authority ACK while response headers remain pending; parent navigation-job grant retention is implemented independently by root.
- `navigation_local::PendingLocal` reserves parent-token cleanup before preparation RPC. The token is attached before response decoding/shape validation, and retained through actor commit or dropped/late response cleanup. Cleanup uses acknowledged `DiscardPreparedLocalNavigation`; frame response cleanup also recognizes `ReleaseFrameResponseStream` in the shared cleanup executor.
- Reload restoration reads controls captured after pagehide from the committed retained-key entry; traversal revalidates immutable entry identity before commitment/restoration. Delayed final ACK does not allow an intervening native load stop to settle success.

Generated regression sources: `held_headers_yield_to_evaluate_stop_and_close_commands`, `newer_navigation_commits_while_older_headers_are_held`, `initial_documentless_actor_still_polls_header_deadline`, `none_strategy_acknowledges_admission_without_waiting_for_headers`, `retained_traversal_rejects_replaced_destination_before_commit_or_form_restore`, `reload_restores_controls_captured_after_pagehide_before_new_parser_completion`, and `abandoned_local_preparation_discards_late_grant_after_delivery_receipt`. These names identify source coverage, not passing results.

Remaining integration/qualification: shared-agent unique-host constructor/adoption is concurrently owned by P/W and root and must preserve these transaction/reply hooks; whole-source compile/type and runtime validation remain deferred. Standalone/non-V8 compatibility branches and the wider selected R/S/V transition/visual inventory are not closed by this actor slice. Final qualification must exercise policy denial/redirect/download/local grant cancellation and remote ACK ordering alongside the generated actor regressions.

### Engine reattachment provenance closure

Generated exact `frame-context-reattachment.json` from pinned V8 source and the
six predecessor patch manifests, using in-memory source reconstruction only.
Normalized the diff against those exact predecessor files and synchronized the
vendored public header. Added seventh-patch build/apply/profile-validation
integration, including complete-prefix recognition for upgrades from an already
patched source checkout. The cctest installer now includes preserved-frame
reattachment, pins both regression source hashes, validates required patched
engine files, and upgrades only the exact previous cctest target. Runtime
profiles carry the regression manifest hash as well as the reattachment patch
hash. Added profile rejection and installer clean/upgrade/idempotence/divergence
regression sources. Project generators, patch installers, builds and tests have
not been run; engine linking/qualification and constructor adoption are still
separate obligations.

### Recoverable shared constructor source review

Reviewed `ConstructionHost`, `ConstructionPage`, `load_with_options_and_host`,
`load_in_agent`, and active-arena validation. The host guard covers fallible
initialization after the recovery slot is taken; the page guard takes over the
same recovery reference after exclusive host installation. Successful finish
removes recovery ownership before moving out the driver. No normal fallible
exit was found that drops the unique host instead of returning it. Recovery
preserves the isolate, not an unchanged partially initialized destination:
runtime rollback must still select/retire the provisional root appropriately.

Added constructor regression sources for invalid parsed input before host
consumption, malformed native history after consumption but before driver
creation, malformed frame history after driver creation, successful nonzero
root transfer exactly once, retained sibling lexical/object/DOM identity, and
missing-host rejection without allocating a replacement isolate. These test
sources invoke the real constructor and native host custody methods. No tests
or builds were executed; runtime constructor/cache adoption remains root-owned.

### Shared cache eviction retirement generation

Delivered `shared_host_cache.rs` and production eviction callers. Cache
replacement, capacity eviction, forward truncation and clearing now retain
cached drivers until exact native realm retirement succeeds. Restoration
removals still transfer entries. The same queue accepts provisional root IDs
from constructors that failed before driver creation; their native bindings,
park tokens and endpoint authority remain owned by the shared host. Admission
reserves two retirement slots before suspension; cached plus queued roots are
bounded at 32. Each drain attempts at most four entries and rotates failures,
preserving failed entries for later acknowledgement/retry.

The nonreusable/no-rollback shared path now preserves the exclusive host in
`pending_agent_host`, enqueues the root, and only then drops its detached driver.
Standalone owned cached drivers resume their private isolate for retirement
and re-suspend on failure. P/W integrated the actor drain after host extraction
and retains closing page state/reply until queued and current-root retirements
succeed. Root integrated provisional retirement after constructor rollback.

Added runtime regression sources for shared navigation/cache restoration,
eviction preserving a sibling realm, constructor rollback retaining original
Document identity and retiring the provisional root, and a failed parent
retirement acknowledgement retaining the native root/driver until retry.
No test/build execution. Open: final combined constructor/restore integration
review, native queue-suspension extension/provenance changes from G agent,
and the global compile/runtime/conformance qualification phase. These source
rows do not claim the full R/S/V checklist complete.

Partial cache-restoration failure reconciliation now marks the destination
nonreusable before rollback, because its native park token may already have
been consumed. Successful incumbent restoration queues destination retirement;
failed rollback retains both roots and the host donor. Added real native-grant
source regressions for post-restore location failure and a second incumbent
rollback failure, then exact Rc-identity host extraction from the cached donor,
sibling survival, successful later rollback and retirement. P/W owns the actor
extraction implementation; source review confirms the generated regression
uses that exact helper rather than constructing a replacement isolate.

G agent's parked microtask queue API extension has been reconciled into the
same exact-input engine patch manifest, vendored header manifest, and cctest
source manifest. Public comments now describe ReattachGlobal's retained paused
queue behavior. Current patch SHA is
`664d8f28bf575eadee7247043d74adee18f3251b76f135de8c35133967092f75`;
current reattachment cctest source SHA is
`906711e98a9969b0a38107bfa0aacb1f111dc4dff611450b3ce3604f2f626ef3`.
Hash computation used source reconstruction only, without applying patches to
installed sources or executing build/test/generator scripts.

### Native asynchronous `import()` source reconciliation (generation only)

- [x] Unix/native Window and worker import callbacks now capture exact environment, stable document epoch, import attributes, referrer/credentials, resolved root and integrity; they enqueue an owned resolver/graph and return without network IO (`dynamic_import_v8.rs::dynamic_import`). Service-worker and worker-CSS rejection remains before IO.
- [x] Root and static dependency requests use the existing bounded graph pipeline through immutable `ModuleGraphBackend::{Window,Worker,Broker,Sources}`. Native Window requests retain parent script-load admission; broker requests reserve the existing realm endpoint's request IDs before IO. Worker bootstrap/source-map snapshots are shared `Arc` values. IO custody retains its slot through completion after consumer cancellation.
- [x] Native current-context import budgets share parser/dynamic-script source bytes, graph permits and in-flight registry in root/frame constructors. Initial frame contexts inherit the parent's budget. Pending imports retain original budget ownership; graph deduplication uses stable document authority while parser liveness still uses input-stream epoch.
- [x] Completion alone compiles/links/evaluates after response preparation. Prepared-link guards prohibit synchronous source fetch fallback for JavaScript, JSON/CSS and worker JavaScript. Root resolution is consumed once, avoiding a second import-map remapping. Namespace/evaluation and sticky response caches remain native. Syntax errors preserve SyntaxError. Completion has the native execution watchdog and microtask checkpoint.
- [x] Imports rotate fairly (32 jobs/turn), retain parked jobs, resume only with live native authority and release graph/resolver custody on exact environment retirement. `document.open` preserves stable import ownership. The module graph source parser only discovers dependencies and does not evaluate modules.
- [x] Regression source added: deferred standalone graph evaluation/duplicate namespace; immutable source snapshot sharing; syntax rejection; exact retirement; held response yielding and duplicate fetch; typed cache separation/sticky MIME failure; document.open plus exhausted shared slots; actual native park/restore; held worker transport with referrer/final URL; worker JSON import and service-worker rejection. Existing immediate-import host tests now explicitly drive host turns.

These rows describe inspected/generated source, **not executed results**. No build, tests, formatters, project generators or conformance run occurred. The non-Unix compatibility callback still uses its separate legacy path; this pass closes the primary macOS/Unix native import continuation. Synchronous static bootstrap/compatibility loaders outside the prepared dynamic-import path are not reclassified as asynchronous by this change. Broader R-06 and the full plan remain open until all other generation obligations and subsequent integrated qualification complete.

### R-06 / L-02 parser and module caller reconciliation (source only)

- [x] `ParserResourceScheduler::schedule_script_auto` selects captured native Window, broker or explicit source-map backend. Live parser blocking/async/deferred sources, parsed snapshot sources, `resume_parser_completion`, and `FrameScriptJob::poll` all call this entry. Empty-src/non-executable/importmap handling remains outside network preparation. Standalone source-map aliases and non-URL fixture keys remain supported without admitting decoded maps as native fetch evidence.
- [x] `prepare_parser_module_graph` captures an immutable backend for both context-backed and standalone graphs. `queue_async_module`, snapshot inline modules, root deferred modules and frame modules poll retained graphs. Snapshot inline modules now enter deferred preparation even when no previous external classic exists.
- [x] Parser module roots, dynamic module roots and import/dependency graphs use the same `SharedFetchRegistry` key `(realm, generation, document authority epoch, requested URL, module type)`. A source-only graph obtains the root response without running/discovering its graph prematurely. Cancellation releases only that consumer; admitted IO retains its slot and cancellation state until terminal completion. Obsolete parser `module_requests`/followers and their separate coalescing path are removed.
- [x] Source compilation/discovery processes at most eight module records per poll; parser and dynamic source/graph consumers rotate bounded batches. Request URL/initiator/integrity/nonce metadata is charged to the shared source budget. Standalone classic IO holds metadata reservation through producer completion.
- [x] Unix `execute_script_record` rejects an unprepared external source or module graph. `execute_module_script_for_request` installs the prepared-link guard, preventing missing JavaScript/typed dependencies from performing synchronous network IO in V8 linking callbacks. Direct public host convenience module APIs retain their separate contract; spec-synchronous worker `importScripts` is unchanged. `preload_module_graph` is absent; graph `wait_for_completion` and `SharedFetch::wait` are test-only.
- [x] Added source regressions: `standalone_parser_source_and_graph_tasks_preserve_order_and_aliases` (live and parsed snapshot, external classic/module plus inline module); `standalone_scheduled_parser_yields_to_tasks_while_classic_headers_are_held`; `standalone_parser_source_cancellation_keeps_old_record_out_of_replacement`; `prepared_document_module_linking_cannot_fall_back_to_broker_io`; `external_source_request_metadata_is_charged_until_owner_drops`; `cached_dependency_compilation_yields_between_bounded_source_slices`; `parser_dynamic_root_and_import_share_fetch_after_parser_cancellation`; and independent-record shared root adoption. Later dynamic-import evaluation no longer inherits a script-element destructive-write counter.
- [x] Follow-up closes the confirmed standalone stylesheet caller omission: `schedule_stylesheet_auto` now selects parent or captured standalone CSS IO for live parser and frame/XML style queues. Dynamic stylesheet jobs select the same standalone loader instead of returning broker-unavailable. Parser native disabled/media/alternate applicability remains authoritative; no network wait runs on a page task. Snapshot construction retains native preorder style/script boundaries, admitting only sheets preceding the current script and retaining trailing sheets as load work. Stop clears boundary ownership.
- [x] `standalone_stylesheet::Loader` captures exact native realm broker/source snapshot, initiator/top-level cookie metadata and isolation policy; reserves a disjoint request-ID range; retains cancellation, 30-second aggregate deadline, 256-resource/8-import-depth bounds and shared raw/decoded/expanded byte reservations. It handles stylesheet redirects, CSS charset fallback, nosniff, link integrity/CORS, font CORS, COEP, native CSP and nested import layer/supports/media wrappers. Parent-backed CSS continues to use its existing native resource authority route.
- [x] Standalone policy is now a native per-context snapshot, initialized before root parsing. Initial frames inherit an independent snapshot; committed external frames reset it; srcdoc/about:blank commits inherit and rebase. HTML/XML parser meta restrictions update that realm. Resource loaders capture the policy at preparation, preventing later root/frame selection from changing the request's source authority.
- [x] CSS regression sources cover held matching/nonmatching stylesheet parser barriers and unrelated task progress; snapshot scripts before/after a held sheet; srcdoc child script after stylesheet completion; dynamic nested imports; redirect referrer and CSS charset decoding; import layer/media preservation; Style cookie destination/request identity; cancellation and meta denial before network; nosniff/CORS/integrity denial before CSSOM publication.
- [x] P/W generated distinct inherited-header self origin and native document/meta self origin for blank/srcdoc/blob/opaque documents. Inline source authorization now has an exact Creator parent route and a captured standalone native policy callback; see the bounded producer/caller map below. This is source evidence only.
- [x] CSS expansion no longer fetches fonts/images before stylesheet publication. `StylesheetSource` carries URL/referrer/policy dependency metadata; `apply_parser_resource_completion` publishes CSS then queues native context dependencies; `poll_dynamic_stylesheets` creates a separate retained `StylesheetPreparation` per resource reference. Matching sheet source/recursive import work alone blocks parser scripts. Failed font/image jobs emit diagnostics without changing the stylesheet load result. Parked contexts do not drain the native queue; generation/revision/connection checks and cancellation prevent stale publication.
- [x] Live parser EOF transfers pending stylesheet completions and node leases to the executable/frame load owner. Deferred/parser-blocking scripts still inspect native matching stylesheet applicability. With no waiting script, DOMContentLoaded can precede the CSS response, while load waits its retained resource work. Regression sources: `stylesheet_without_waiting_script_allows_dom_content_loaded_before_response`, `unavailable_font_does_not_hold_parser_script_or_fail_stylesheet` (now also requires independent image request before held font release).
- [x] Parent recursive CSS uses `StylesheetExpansionBudget` with shared source-byte leases, 256 import request limit, 32MiB cumulative expansion bound, cancellation and one 30-second expansion deadline. Parent parser response bytes remain in scheduler custody until consumed; standalone metadata copies are charged too. Active parent RPC cancellation remains cooperative: its resource job is retained through the in-flight reply, and cancellation prevents subsequent imports. No claim of interrupting a synchronous IPC receive.
- [ ] Full selected timing/negative matrix, non-Unix compatibility scope and integrated compilation/conformance remain unqualified. R-06/L-02 parent rows must not be closed from these bounded source rows alone. No tests/builds/formatters/project generators were executed.


### Inline style native policy integration source pass

- [x] `V8BindingState::reconcile_inline_style_policy` retains exact source/document/stream decisions, reauthorizes changed attribute revisions, excludes incomplete parser style text, and runs at parser/JS/layout/CSSOM boundaries. Native DOM flags are independent of author `.disabled`. `stylesheet_ownership::for_node/available` excludes denied owners. Native HTML/XML sink closing marks parser sheets eligible; pure inline CSS no longer bypasses the policy gate merely because it has no fetched URLs.
- [x] Parent callbacks in `install_native_document` and initial-frame replacement call `AuthorizeInlineStyle` with current native Creator. Standalone root/inherited frame callbacks use their per-context policy snapshots and queue deferred trusted `SecurityPolicyViolationEvent` payloads through `deliver_native_csp_event`. Parent report-only/report endpoint custody stays in the native parent CSP route. Standalone report endpoint transport is not established by this event-queue implementation.
- [x] CSSOM declaration and `cssText` edits use native exemption state; normal `setAttribute`/Attr mutation resets it. DOM style text remains visible. `LayoutSelectorState::inline_style_blocked` reaches Blitz/Stylo `style_attribute()` so denied declarations disappear from cascade without removing the actual style attribute from selector matching. Regression sources: `inline_style_policy_preserves_dom_but_suppresses_layout_and_cssom_cannot_enable_denied_sheet`, `later_meta_style_policy_does_not_revoke_already_prepared_sheet`, `standalone_inline_style_violation_is_deferred_trusted_and_not_repeated_by_layout_reads`, and `denied_inline_declarations_preserve_style_attribute_selector_matching`.
- [ ] Remaining source reconciliation: complete shadow/adoption and parser reentry policy-event timing matrix, remaining report lifecycle qualification and readiness/render consumers beyond the inspected native declaration bridge. Snapshot meta/style ordering and native nonce hiding were subsequently generated below. CSS dependency discovery still follows lexical resource references; matching media/font-use demand and cross-consumer image/font cache adoption require their full selected inventory review. These are distinct from the source-complete job/lifetime split; R-06/L-02 remain open.
- [ ] All listed regressions are generated source only. No builds, tests, formatters, generators or conformance execution occurred.

## Snapshot ordering and script policy capture source pass

- Source generated: root snapshot construction now feeds the existing frame/parser event owner with ordered script, stylesheet, metadata and frame events; future inline styles remain staged until their native source boundary. Event metadata (CSP/referrer/href/frame URL) has retained shared byte accounting.
- Source generated: native nonce slot/hiding, internal CSP nonce reads, cloning/insertion rules and HTMLElement/SVGElement nonce IDL. Header-vs-meta delivery comes from native CspPolicySet/FrameWorkerPolicyBound metadata. No IDL generation executed.
- Source generated: CaptureScriptPolicy callback captures native authority in ParserScriptFetch and PreparedDynamicScript. Parent inline/classic/typed/module requests carry authority; parser element roots carry parser_inserted=true, dependencies/dynamic roots false. In-flight module registry keys distinguish policy and parser provenance; retained per-document module maps preserve their URL/type semantics.
- Source generated: synchronous DOM inline insertion now invokes the native policy gate; document.write inline gate carries the same captured policy.
- Regression sources added: snapshot_deferred_module_keeps_preparation_policy_across_later_meta, snapshot_style_and_meta_boundaries_publish_in_source_order, pending_fetch_registry_separates_captured_policy_and_parser_provenance. None executed.
- Still in progress: full snapshot async/lifecycle caller reconciliation, broader style shadow/adoption/reentry. Standalone report transport was subsequently generated below. Subsequent sections record delivered standalone immutable policy and nonceable/dangling source fixes. These bounded additions do not close the full R/L or S/V package.

### Follow-up source closure: standalone preparation and metadata

- Standalone native CSP snapshots now travel in parser/dynamic preparation records and module backends; deferred inline execution uses the captured snapshot, and source-map/classic/module network requests authorize against it. ScriptFetch callers carry CSP, nonce and parser provenance through the shared per-redirect checks generated by root.
- Background ScriptCspReports retains bounded violations independently of fetch results. Native context ownership checks document identity and stable document authority epoch before transferring to deferred trusted events; parked contexts wait, stale documents drop, and event-queue failure retains pending violations. Callback delivery is wired for module/dependency, classic, typed and direct fallback consumers.
- Dynamic CSP meta insertion now reaches native InstallRealmMetadata/local native policy callback. Parser/snapshot/dynamic processing checks native document-head membership. Content mutation alone does not reprocess an already inserted CSP meta. Native callback input never uses authored globals.
- Added source regressions: standalone_script_policy_is_captured_before_later_meta_and_inline_insertion_is_checked; background_script_reports_keep_document_identity_and_survive_open_and_producer_drop; csp_meta_requires_head_insertion_and_content_changes_do_not_reprocess. Updated snapshot async lifecycle regression to retain a real pending top-level-await module and use frame parser-owner readiness.
- Remaining: full selected style adoption/shadow/lexical resource demand matrix and allocation qualification beyond the bounded retained source owners described below. No build, test, generator or formatter execution occurred.

### Nonceable element source reconciliation

Pinned `script-src/non-nonceable-elements.html` and `nonce-hiding/dangling-html-or-body.html` exposed two additional producer gaps. Native `element_nonce_for_csp` now rejects duplicate parser attributes and ASCII-insensitive dangling script/style/link tokens without changing the IDL nonce slot. HTML sink records html5ever 0.39 duplicate-attribute provenance and applies header nonce hiding to parser-added html/body attributes. All script/style CSP readers use the native eligibility helper. Added `nonceable_algorithm_rejects_parser_duplicates_and_dangling_attribute_tokens` and `dangling_body_and_html_parser_attributes_cannot_publish_hidden_header_nonce` regression sources. No execution; broader nonce IDL/reflection/clone matrix still requires final qualification.


### Standalone report HTTP delivery source completion

`standalone_csp_reporting::{Owner,Sender}` now selects targets from the captured policy, submits native POST operations to the existing bounded IO executor and uses the cancellable identity-broker collector. Limits are 128 process jobs, 16 per document, 10 per endpoint/minute, 128 retained endpoint buckets/outcomes, 32 KiB request and 64 KiB response, and one 10-second deadline including queue wait. Credentials are omitted. Actual native binding retirement cancels active/queued delivery; admitted jobs retain broker/permit custody until terminal collection. document.open preserves the same native Document/report task source and therefore does not retire this owner.

Inline script/style and stylesheet/font/image policy decisions plus module/classic/typed redirects carry captured reporting targets into this sender and deferred trusted events. Event capacity failures retain violations; HTTP rate/size/queue limits have bounded explicit outcomes. Added sender regressions for captured endpoint/credentials, retirement before admission, active operation cancellation/admission release, and production `standalone_inline_policy_report_reaches_broker_and_native_event_independently_of_load`. These are unexecuted source regressions; build/conformance remain gated.


### Retained policy source allocation

Standalone script preparations now share `Arc<ScriptPolicySnapshot>` including the native typed CSP payload; backend/module/deferred owners retain that same object. A bounded 4 MiB writer prevents oversized temporary serialization, identical current-policy captures reuse the same snapshot, and a 32 MiB process source lease covers serialized/native policy text through last-owner drop. Module fetch keys hash the existing serialized bytes without cloning the full policy. Added `native_policy_preparations_share_immutable_payload_until_meta_changes` source regression.

Inline style decisions now retain shared 32 MiB process source charges, release replaced/stale/retired decisions, cap each source at 8 MiB/nonce at4096 bytes, and fail closed before reserve/authorization errors. Native stylesheet text assembly is iterative and excludes Comment/PI descendant text. Snapshot boundary metadata admission is capped8192 entries. These source bounds are not a claim of total heap or integrated conformance qualification.

### Child-text preparation and stylesheet adoption reconciliation

Source-generated (not built or executed): `DomArena::child_text_units` shares direct Text/CDATA collection with `HTMLScriptElement.text`; `child_text_source` joins UTF16 before scalar conversion and enforces an8MiB execution-source bound in `script_record` and dynamic `prepare_inline_script_or_queue`. Parser document.write fast execution enters the same preparation function. Native split-surrogate/nested-child/capacity and actual dynamic-script regression sources added.

Cross-arena `stylesheet_adoption::publish` preflights destination records/queue admission before clearing source inline checks, snapshot staging, pending/prepared owners and queued stylesheet events. Destination stylesheet revision, response override/base, policy and parser readiness are reset; author media/disabled and moved associated wrappers survive. Detached adoption waits insertion; connected adoption queues new preparation. Dynamic link preparation now performs destination authorization despite prior denied state and enables only a successful live response. GFQ owns retained migration-stage integration.

Remaining: full CSSOM detached old-sheet/new-associated-sheet identity semantics and destination-deny/reinsert/delayed old-receipt integration matrix require further source reconciliation; this bounded reset alone does not close the package. Font/image demand selection and decoded response adoption into actual layout/paint consumers remain open. No build, tests, formatter, generator or conformance execution occurred.

### Stylesheet associations and binary resource consumers

Source-generated: stylesheet ownership now captures original owner-document identity, explicitly detaches migrated owners from source sheets, and keeps detached href/title metadata. Existing independent sheet backing nodes retain old rules; destination preparation creates a new sheet. Inline checks include sheet revision, so reinsertion rechecks policy without retroactive changes merely from later meta. New two-document author regression covers destination denial, old-sheet mutation isolation, and newly allowed reinsertion.

`PageBinaryResource` carries bounded raw image/font bodies separately from frame/CSS text, including local data/blob and HTTP responses. Parent font requests now send/check CORS origin with same-origin credentials and redirect-origin taint. Standalone loading also retains raw binary bodies. Independent dependency receipts hold the shared byte reservation into an exact document/generation/stream/revision/attribute-checked cache. `populate_css_fonts` supplies production layout, and root/frame paint consults cached images and redirect aliases before fetching. Resource publication emits native stylesheet mutation for layout invalidation. Removed legacy runtime regex/synchronous font loading and parent serial subresource discard functions; non-sheet inline resources now use independent dependency jobs.

Regression sources cover binary non-text byte preservation with native CSP denial, font receipt reaching actual layout's loaded-font output, old revision receipt suppression, paint aliases/size limits/removal. All remain unexecuted. Remaining full closure includes demand-driven cascade/media/image selection, ordered font-source fallback/descriptors/unicode-range selection, nested @font-face family association, cross-consumer cache policy and full cancellation/adoption timing qualification. Current lexical dependency discovery must not be described as that complete demand model.

### Retained demand and native geometry follow-through

Source-generated: `css_dependency_sources` keeps bounded metadata/node leases and native owner identity after initial preparation. Each bounded poll derives image demand from native computed visible styles/generated nodes and font family demand from resolved text. Attempt identities prevent repeated fetches; DOM changes re-evaluate sources, and normal checkpoints re-evaluate native media changes. Unsupported/unused initial resources remain dormant rather than disappearing. Stop clears pending demand metadata. Native CSSOM family tokenization preserves quoted commas/escapes; nested media/supports font-face traversal checks current native media and sheet enablement. Added mutation-activation regression source: unused image/font are not initially fetched, then one fetch each when the native computed class change needs them.

LiveLayoutProvider's separate JS geometry path now accepts `BindingFontResource` with its own shared-budget custody. Both production geometry and PageState layout filter owner document/revision and sheet disabled/media applicability. Independent old stylesheet backing roots now use `TrackedNodeLease`; removed raw Store Drop/release and added busy-arena Store Drop regression source.

Still open: full ordered font-src fallback/format/local source handling, descriptors and unicode-range shaping selection; exact image-set/type/resolution selection; complete cross-consumer intrinsic decoding and cache matrix. All source changes/regressions are unbuilt and unexecuted.
