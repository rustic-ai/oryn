# Parallel parser generation — 2026-09-18

Status: implementation and regression **source** delivered for the diagnosed
parser-local gaps below. No builds, tests, formatters, generators, services or
conformance commands were run. This does not close the S-02 parent or the global
generation gate.

## Delivered source

| Gap | Implementation | Regression source |
| --- | --- | --- |
| Native mutation failure continued consuming later tokens without an installed V8 hook | Shared `OrynTreeSink::construction_interrupted` includes the mutation failure latch; HTML delimiter slices and XML event slices stop at the accepted prefix | `failure_without_v8_hook_preserves_html_suffix_until_retry`; `native_mutation_failure_stops_xml_without_a_v8_hook` |
| Adoption-agency reparenting took a child snapshot before preceding failed insertion committed | `ParserMutation::Reparent` resolves and snapshots children when it reaches the queue head; each child becomes a separately retained insertion in order | `reparent_reads_children_after_failed_insertion_commits` |
| Foreign parser key allocation left an unowned detached fragment in the original arena | Reserve the generational key, then release its native reservation immediately; the actual target retains its independent tracked lease | `foreign_keys_release_reservations_without_reusing_live_handles` |
| XML final-text publication could consume EndDocument and drop pending mutation custody | Retain `end_started`; completion preflights text publication and returns the owned parser on failure; retry completes without asking the reader for another event | `eof_publication_failure_retains_owner_and_completes_once` |
| XML closing tags kept the parser-open marker until parser destruction | `finish_element` clears only this parser's original-document marker on the actual migrated owner before producing close-resource events | `xml_end_tag_clears_open_marker_before_terminal_completion` |
| Retired exact document owner borrowed a different live document's realm | `document_parser_gate::parser_owner` prefers a live exact binding, refuses retired/suspended exact binding fallback, and preserves arena fallback only for documents without any exact binding | `exact_retired_parser_document_never_borrows_another_document_realm` |
| A microtask suspended/replaced/retired the document before constructor entry, but initialization marked construction accepted | Validate both original and actual-owner lifecycle identities after the microtask checkpoint; preserve the pending custom construction flag until first author entry | `pre_constructor_microtask_suspension_preserves_first_construction` |
| Doctype insertion bypassed the ordered pending mutation queue | Native doctype handles now enqueue `ParserMutation::Insert` like other parser-owned nodes | Covered by shared insertion queue review; no separate execution claim |

These changes extend, rather than replace, the previously generated HTML/XML/V8
migration, constructor exception/termination, fallback-root lifetime, failed
adoption, post-commit cross-arena publication, EOF retry and cancellation sources
listed in `FACE-SUBMISSION-CUSTODY-AUDIT-2026-09-17.md`.

## Production caller reconciliation

- `html::StreamingParser::push_until_script_boundary` resolves queued mutations
  before accepting a new source slice. Every resource/script boundary variant
  shares this implementation. `prepare_finish` retains one-shot HTML EOF.
- `xml::XmlDocumentParser::advance` retains the reader, stack and tracked handles
  on mutation failure; its new `complete` method also retains terminal ownership.
  Native `take_events` still drains already admitted events; it does not cause a
  second constructor call.
- `document_parser_gate::with_parser_construction` is the lexical bridge shared
  by HTML, XML, root/frame and document.write callers. It never retains the V8
  scope in a parser. Constructor result rooting and migration resolution remain
  intact.
- `document_parser_gate::write` saves the consumed suffix and tokenizer events
  before checking construction errors. The gate owns the tokenizer while idle.
- `frame_script_preparation::poll_html_parser` restores source suffix, event
  queue and retained parser before surfacing a construction error; EOF performs
  retained preflight. XML Pending is installed back into the owner before event
  publication can fail.
- The separately consuming `LiveParserState::resume` route was confirmed to be
  test-only; the navigation lane removed it and moved its regression controls
  onto production `FrameScriptPreparation` custody. That lane also retains the
  observed EOF before fallible preparation and integrates typed source queues.
- Inert fragment/document parsing remains constructor-free; existing native
  registry initialization/upgrading and fragment import paths are preserved.

## Shared dependencies sent to the coordinator

The following are concrete source findings, **not** merely pending execution.
They require coordination with shared native lifecycle/mutation owners and keep
`S-02.parser-migrated-handles` open until reconciled:

1. The originally suspected consuming production parser was test-only. The
   navigation lane removed it, migrated its tests to the actual retained owner,
   added post-slice/EOF fault sources, and made observed EOF once-only before
   fallible publication. This source finding is reconciled; execution is pending.
2. The authority/storage lane generated same-arena adoption publication in the
   existing agent queue, using tracked identities and four independently retained
   stages. Parser Checkpoint already resumes that queue before insertion retry;
   no alternate hook is needed. Its four-stage injected failure source is present.
3. `DomRuntime::insert_before` has fallible ownership/radio/select/selectedcontent
   work after structural insertion and journal publication. The automation/timing
   lane now owns the full native continuation and parser receipt API. Parser
   integration awaits that API; this remains a source dependency rather than a
   passing or completed fault matrix.

No central checklist counts were edited by this lane. The coordinator owns those
counts and the overall integration gate. Full pinned Core/scoped-registry mapping,
combined independent source review, generated-binding consistency and all runtime
qualification remain separate obligations.


## Explicit Unicode and DOMString source integration

The initial inert-arena decoding approach was replaced before publication with
shared incremental **token payload decoding**. There is no final-arena decode.

- `html/dom_input.rs::ParserInput` is an owned transport with distinct `unicode`
  and `dom_string` constructors. Its field is private and it has no implicit
  string conversion. Prefix/split operations preserve UTF-8 and escape-pair
  boundaries; offsets measure the retained transport bytes.
- The shared HTML TreeSink decodes element names, custom `is`, attributes,
  comments, doctypes and text **before** native publication and custom-element
  construction/reaction callbacks. Native element-name queries reencode names
  for html5ever's matching rules. XML opts out of the HTML transport.
- The optional default-off html5ever/markup5ever character-reference hook escapes
  reference-produced markers independently of raw input. Escaped references emit
  one character token, preserving foster-parent text boundaries. Numeric surrogate
  references still follow upstream replacement rules; literal lone surrogates
  supplied as DOMString remain lone units.
- `DocumentParserGate::write` turns one native DOMString into typed input once;
  paused/nested/failed writes retain that typed source. Raw Unicode APIs stay
  explicit. The navigation lane converted retained production fetch/replay queues
  to `ParserInput::unicode` and feeds typed safe prefixes.
- Detached document writers have `StreamingParser::push_dom_string`. Root owns
  the shared write/writeln variadic conversion fix: concatenate UTF-16 units,
  preserve TrustedHTML branding, append newline in units and avoid lossy V8
  conversion. That caller integration is required before overall source closure.
- Inert DOMString APIs serve native sanitizer/fragment callers and JS Document,
  Element/ShadowRoot, editing and DOMParser paths. The DOM lane and coordinator
  own those shared call-site edits. Ordinary Unicode parsing explicitly escapes
  literal reserved characters, preserving fetched storage-prefix lookalikes.

Additional generated regression sources:

- `inert_transport_preserves_lone_units_names_comments_and_reference_provenance`
- `literal_storage_prefix_and_tokenizer_escape_never_become_surrogates`
- `fragment_context_and_foster_text_decode_complete_escape_pairs`
- `typed_stream_boundaries_preserve_unicode_literals_and_dom_units`
- `written_token_payload_is_decoded_before_constructor_and_attribute_reactions`
- `split_written_entities_and_surrogate_pairs_keep_distinct_source_identity`

The DOM lane also supplies the safe/unsafe/legacy sanitizer sink matrix. All are
source evidence only; no test, build or generator has run.

## Native selectedcontent and clone continuation

Assigned after the initial parser lane. `clone_tree.rs::CloneCursor` now freezes
source state before allocation and retains each target allocation, materialized
Attr and attachment stage. Immutable attribute copying does not invoke authored
setAttribute algorithms. Both synchronous `clone_node` and selectedcontent use
the same cursor. Tracked roots reject migration for an old-arena continuation and
release from the node's actual owner on cancellation; synchronous cleanup follows
the existing bare-NodeId handoff convention.

`selected_content.rs::SelectedContentUpdate` retains original selected sources,
per-target clones, old children and each removal/insertion receipt. Completed
targets and allocated clones are not recreated when a later stage fails. The
single replacement record is published once. Primitive observer suppression
covers only the exact uninterrupted native resume slice, including an admitted
prefix before error; it does not hide intervening authored records. A child moved
to another parent before its removal starts is not removed from that new parent.

This uses the automation lane's `CursorRoots`, `TreeInsertion`, `TreeRemoval`,
`DocumentAssignment` and `OwnershipPublication`; outer insertion/option/radio/
connection integration is owned by that lane. Their concrete constructors, resume
and cleanup signatures have now been reconciled by source inspection. Parser
Insert stores the receipt before its first structural mutation; text/foster
operations normalize to that same retained queue entry. Receipt retries never
repeat adoption or parser detach. Shared nested-stage source review remains with
the automation lane; no executable verification has begun.

Additional regression sources:

- `resumable_clone_keeps_allocated_identity_and_frozen_source_after_each_fault`
- `clone_abort_releases_allocations_and_refuses_later_resume`
- `migrated_partial_clone_is_not_replayed_and_lease_release_follows_owner`
- `selected_target_retry_never_reclones_completed_target_or_rereads_source`
- `replace_retry_preserves_and_does_not_suppress_intervening_author_insert`

The parser bridge also now completes an earlier construction reaction boundary
before pushing a later one within the same tokenizer call. Previously replacing
its guard after pushing a new queue could pop the wrong queue. Source regression:
`multiple_constructions_in_one_tokenizer_call_keep_reaction_stack_order`.
Root's shared actual-JS `document_write_preserves_variadic_utf16_and_trusted_newline`
regression and variadic conversion patch complete that caller's UTF-16 source
integration. These remain unexecuted source changes.

Source reconciliation also corrected reverse-postorder clone attachment to
prepend siblings, preserving captured source order while completing descendants
first. Selected-content cancellation now fences later resume explicitly; added
`cancelled_selected_update_never_resumes_released_target_custody`.

Explicit parser Detach now retains the same TreeRemoval custody rather than
calling the old coarse parser_detach helper. Added source fault matrices:
`insertion_failures_keep_one_native_edge_and_intervening_author_attribute`
(48 nested/outer failure positions) and
`removal_failures_complete_once_and_do_not_replay_after_author_reinsertion`
(36 positions). The automation lane supplies test-only failure admission and
retained cleanup accounting. No test results are claimed.

## Expanded frozen Core partial-update dependencies

The pinned checkout `wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0`
includes `template-for-html-setters.html` and
`partial-updates/tentative/fragment/sanitize-nested.html` in Core. Generated:

- Real HTML processing-instruction provenance, distinct from comment strings,
  opt-in through the vendored tokenizer/TreeBuilder interface.
- Native `apply_fragment_patches` after sanitizer filtering, explicit detached
  fragment boundaries and independent shadow/template content scopes.
- Retained native patch/removal/insertion and nested patch receipts, exact
  case-sensitive PI names, named start/marker and matching nested end behavior.
- Explicit unpatched inert-document parsing for Document safe/unsafe sanitizer
  entry points, and disabled automatic patching in retained-context parsing.
- Incremental `StreamingParser::with_fragment` over tracked live root aliases,
  retaining open-node identity across writes, sanitizer token admission before
  custom constructors, native script-enable policy and independent scope.
- Declarative ShadowRoot registry default selection uses the node Document;
  explicit null/keep-null registry state remains preserved.

DOM lane owns entrypoint/filter ordering; loading lane owns WritableStream
queue, options, lifetime and script preparation. New source regressions cover
comment impersonation, PI case sensitivity, outside-fragment isolation, 80
native patch failure positions, every scalar split/open-node identity, and
removal of `is`/`for` before constructor/patch admission.

Unbuffered template-start publication is now generated separately from buffered
patching: the accepted old range is removed once, tokenizer insertions use the
retained target and the end/void marker's current position, a moved-away marker
falls back to append, and a reinserted marker resumes positional insertion.
Scripts can yield before template close. Close-time marker consumption uses
tracked parser Detach receipts from each node's current owner, including a target
migrated into another arena. Admission, initial removal, buffered patch and
cleanup are separate retained queue stages. New incremental-prefix, marker
movement and cross-arena target regression sources cover this contract.

**Remaining expanded source review:** external declarative fragment `src`
loading/CSP/SRI/cancellation fixtures need cohort confirmation and resource-owner
integration; DOM parser source itself does not fetch them. Exhaustive callback,
buffer and error combinations remain subject to whole-source integration review.
Do not mark broad partial-update conformance complete without qualification.

R-06 caller source inspection confirms retained XML advance installs Pending or
completion ownership before exposing construction errors; live input polling
occurs only when the XML reader requests input and ordered jobs are empty.
The production driver uses 256 XML events per turn, and native XML input has a
64 KiB scalar-safe reader budget. Existing sources include nested inline writes,
XML document.write rejection preserving suffix/order, oversized HTML token
fairness, script/style admission retry, and XML completion/lifecycle custody.
Whole R-06 acceptance remains open; this is source evidence, not executed
conformance or a claim that every scheduling combination is covered.

`StreamingParser::retained_input_bytes` now exposes actual buffered lexical
source, queued input, pending table text, sink text and pending attribute payload.
The loading lane uses this instead of cumulative streamed bytes. Regression
source covers a large incomplete attribute, release on completed tag, and many
small completed tokens. Declarative shadow attachment now reads sanitized native
template attributes instead of unfiltered tokenizer attributes; `for` takes
precedence. CloneCursor preserves the new declarative adopted-stylesheet source
field supplied by the DOM lane.

### External template source handoff

The automation lane confirmed all nine external `src-*` fixtures are pinned
Core. Parser-side source now emits `ExternalTemplateDiscovered { node }` once,
keeps external templates out of inline patching and preserves the old marker
range until the resource owner admits a response. New
`StreamingParser::with_external_template` retains incremental native parsing,
marker activation, script/resource events and EOF cleanup independently of the
main document parser. It returns `None` for an unmatched/disconnected target.
The resource owner must check the original template's live lease before each
chunk and own CSP/CORS/referrer/nonce/SRI/buffering/cancellation. Network fetching
is intentionally delegated to that owner. Generated parser regressions cover
nonblocking discovery, old-target preservation, cross-chunk node identity and
EOF-only template/marker cleanup. External loader caller integration remains
owned by the automation lane; parser-side API delivery does not claim passing
network conformance.

### R-06 production caller inventory reviewed

- `BufferedNavigationInput::poll`: transfers live XML before consuming its body;
  detached buffered XML alone advances without an authored realm.
- `FrameScriptPreparation::{poll_xml_input,poll_xml,finish_xml_completion}`:
  one input poll when needed, 256-event XML turns, retained parser/events before
  construction errors, stylesheet/script admission barriers and completion
  receipts before readiness/lifecycle publication.
- `FrameScriptPreparation::poll_html_parser`: typed written input precedes the
  fetched suffix, scalar-safe feed budgets and native token/resource boundaries;
  EOF latch precedes fallible preparation.
- `RawV8Host::with_parser_construction`: all fallible realm/reaction admission
  occurs before invoking a closure that may take the XML parser; resulting
  construction errors return alongside its retained result.
- `DocumentParserGate::write_input`: recursive writes retain their suffix before
  a caller's suffix, started scripts do not replay after callback failure, and
  interrupted events return to the executable owner. Resource-capacity errors
  retain accepted events through the recovery path.
- `require_html_document_stream`: rejects XML writes and synchronous custom
  constructor reentry before touching the parser.

Existing source fixtures for nested-inline writes, parser-written external
scripts, deferred/module suppression, XML write rejection, detached inert
writers, custom/observer-before-script ordering, stylesheet admission retries,
stop-after-failure and oversized-token fairness were reconciled by inspection.
No executable result or broader R-06 qualification is claimed.

### Buffered external response publication

Added `with_external_template_buffered`: complete tokenization occurs in a
retained detached fragment across bounded source slices. EOF creates one retained
marker patch publication; scripts/resources become visible only afterward. The
original `for` source and template sanitizer policy are retained at admission.
`external_template_cleanup_started` distinguishes parser-owned finalization
from an authored removal, and pending-state reporting includes buffered EOF
admission. Final pending text is flushed before either progressive cleanup or
buffered publication. Added positive no-visible-prefix/script-event coverage,
96 native failure positions preserving staged identity, and removed-template
cancellation regression sources. Automation owns the loader switch and terminal
lifetime integration. Source only.

### Legacy loading and policy route disposition

- `LoadWorkerScript`: no native/shim producer remains; deleted its synchronous
  page-origin fetch implementation. The old wire variant now rejects explicitly,
  preserving rejection coverage without a page fallback. Native worker admission
  remains responsible for loading.
- `InstallMetaSecurity` / `InstallReferrerPolicy`: removed runtime completed-source
  regex publication into context policy. Local trace/projection remains. Old wire
  routes reject; live parser `InstallRealmMetadata` and
  `InstallDocumentSourceMetadata` retain exact document identity.
- `GetReferrerPolicy`: added required Creator, exact Window owner validation and
  native policy snapshot lookup (including frame/prerender referrer headers).
  Migrated actual host initialization and all transport fixture source literals;
  registered the test route's root document explicitly. No missing-source root
  fallback. Added child metadata isolation/retirement/missing-source and retired
  worker/metadata no-publication regression sources.
- `AuthorizeNavigation`: required `NavigationPolicySource::Document(Creator)` or
  explicit `InitialAboutBlank`. HTTP admission resolves initial identity once via
  native-only `CaptureCommandNavigationSource(initial_only=true)` before network;
  all redirects retain the returned Creator. Parent checks exact owner and source
  origin/base policy rather than page projections. Native actor carries its source,
  prerender carries roster source, and the duplicated prerender policy owner was
  removed in favor of the retained HTTP admission phase.
- Native command capture validates an existing root and only initializes a truly
  initial blank document. Missing/stale navigated roots fail, never receive guessed
  generation. Capture is outside the author compatibility whitelist. Added exact
  child-origin/retirement, initial/captured/replacement and missing-source tests,
  plus a real IPC source showing one capture and the same Creator at both initial
  and redirected policy requests.
- Corrected the caller inventory: runtime's legacy HttpNavigation block was inside
  both cfg(non-V8) function and cfg(V8) local block, unreachable in every build.
  Removed that superseded block. Actual non-V8 native command navigation has no
  context-client field and retains its direct native network route; it never
  called the page-only wire authorization being replaced.
- Central protocol version coordination sent to root for the required source-field
  schema changes; lane does not independently race the shared version constant.

All entries describe source generation and caller inspection, not passing tests.

### Gesture eligibility and target-aware prerender roster

Pinned source review confirms `_blank` activation calls `window.open(url,
'_blank', 'noopener')` after the already-running prerender reports ready; simply
loading a second ordinary page cannot satisfy those fixtures. Authority lane owns
hidden destination-page reservation and existing-candidate popup/form handoff.

Generated native trusted-pointer eligibility in `speculation_registry`: moderate
requires 200 ms continuous hover or pointerdown, conservative requires pointerdown;
authored PointerEvents do not grant eligibility. Delayed hover retains node lease
and exact document identity, revalidates connected anchor and href, and cancels on
pointer cancellation/retarget/removal. Eligibility latches by original rule source
and ordinal; removed/replaced sources cannot inherit it. Native pointer receipt
feeds the producer after prompt/scrollbar/drag consumption and before author events.

Roster consumes explicit eligibility and keeps bounded wakeups for pending hover.
Self activation excludes `_blank`; dedicated `matching_blank_pending` and
`take_for_blank_activation` select the preserved running candidate for the handoff
owner. The prior blanket moderate/conservative/blank exclusions are removed.
Existing loaded-document continuation polling is preserved.

Source regressions cover delayed moderate/conservative timing, native-vs-synthetic
pointer admission, href/removal/cancel invalidation, target-family selection and
hover wake ownership. Hidden-page reservation/activation integration remains with
authority lane; no execution or conformance success is claimed.

### R-06.k / R-06.l full caller reconciliation pass

Reviewed actual production owners instead of treating unchecked parent rows as
missing implementations. This pass found and fixed two additional source defects:

1. XML script reservations were retained in an owner-wide vector after the
   corresponding script job completed. Both source-byte usage and the 1024-script
   bound accumulated across the entire document. Reservations now live in each
   `FrameScriptJob`; completed, failed, stopped or superseded jobs release their
   charge, and admission counts retained jobs. Added
   `sequential_xml_scripts_release_source_custody_before_document_eof` (1100
   sequential scripts with bounded peak source charge) and
   `stopped_xml_input_releases_deferred_script_and_input_reservations`.
2. `stop_pending_document_load_with` returned immediately when font cancellation
   publication failed, leaving root/child stop and remaining loader cancellation
   unperformed. Font cleanup now participates in first-error aggregation while
   all independent cancellations finish. Added
   `failed_font_stop_still_cancels_all_document_sources_and_preserves_first_error`;
   existing failed-root-stop coverage remains intact.

| Requirement / caller | Source evidence and disposition |
| --- | --- |
| XML before network EOF | `poll_xml_input`, `poll_parser`, `XmlDocumentParser::advance(256)` preserve Pending/chunk/real EOF and close-tag ScriptReady; existing streamed-root and frame close-tag fixtures cover script before suffix/EOF. |
| XML incomplete tokens and fairness | DOM XML scanner keeps 64 KiB scalar-safe slices; every-split, malformed real EOF and late-error fixtures already exist. Source inspection found no new lexer defect. |
| XML script preparation / styles | `accept_parser_event` performs observer checkpoint before script capture, retains event on error, and enqueues deferred/async/ordered jobs; native media/disabled/revision-aware stylesheet barriers remain. Existing admission-failure, stop-before-retry and observer-inertness fixtures cover these callers. |
| Nested document.write | Typed `DocumentParserGate::write_input` retains nested paused suffix before outer suffix, returns accepted events to host on failure, and suppresses already-started script replay. Existing inline/nested/external/deferred/style/handler/frame and XML-rejection fixtures cover actual binding and retained owner. |
| Script preparation callbacks | Native `prepare_script_text` already retains a lifetime-qualified receipt and accepted Trusted Types result before later admission, so retry does not rerun an accepted callback for unchanged input. No duplicate implementation added. |
| Document/sibling fairness | `poll_frame_script_preparations` round-robins up to 64 documents, one ordered job and up to eight async jobs per document; pending IO and parked/retry owners yield. HTML large-token and async sibling/module fixtures already exist. |
| Stop / late completion | `live` checks exact document identity and native load-stop state; stop drops parser/input/completion/frame/resource owners and keeps shared source/graph budgets until late producer ownership retires. This pass repairs the font-cleanup early return. |
| document.open / supersession | Stream epoch and exact script owner checks reject stale queued actions; existing old-receipt replacement, load-listener replacement, HTML retry stop/replacement and constructor-suspension fixtures retain accepted prefixes without resuming stale suffixes. |
| BFCache / parking | Runnable and polling owners explicitly retain parked documents without input polling or script progress; `native_parked_frame_parser_keeps_jobs_without_runnable_work_until_restore` already covers restoration. History publication/close authority remains with the navigation/root lanes, not duplicated here. |

No additional production defect was confirmed in the remaining reviewed parser
callers. Broad history/prerender publication integration remains owned by its
assigned lanes. This evidence closes source gaps only: builds, regression execution,
frozen Core WPT and five-family qualification are still non-source gates and have
not been run. Central checklist decisions remain with root.

Hidden-page prerender follow-up: roster now uses reservation destination page for
ready/load ACKs and `PreparedCandidate::endpoint()` for construction; source-page
generation accounting ignores hidden-page generations. Network authorization
continues to use original source Creator/client. Authority owns activation handoff.


## Native link-prefetch Core source closure (2026-09-18)

The fixed Core dependency audit distinguishes link prefetch from Extended speculation-rule prefetch. Implemented a dedicated `PlatformHostRequest::PrefetchStart` / parent `PrefetchStreamStart`; the existing native preload lifecycle now selects `rel=prefetch` independently of `as`, resolves the native encoded document URL, captures referrer policy, retains target identity, drains arbitrary bytes without image decoding or script execution, and delivers a retained load/error publication. Removal, changed URL/attributes/base, document stop/replacement, and retirement use existing platform cancellation and stream cleanup. Publication receipts preserve accepted listener prefixes and cancel unstarted delivery after retirement. BFCache suspension does not discard owned pending responses.

Parent admission requires an exact document Creator and bodyless no-CORS GET before any network work. Its ordinary Fetch implementation provides existing controller/service-worker interception with empty destination, redirect policy, mixed-content enforcement, credentials/referrer/COEP, cache broker and bounded body/cleanup. `Sec-Purpose: prefetch` is inserted by the native transport after author header filtering at every redirect. Standalone uses the same ordinary standalone Fetch engine, separate Prefetch policy and empty Fetch Metadata destination. Root owns `CspDestination::Prefetch` per-policy any-supported-fetch-directive authorization. The legacy runtime script scanner no longer categorizes prefetch as a module.

Caller source reconciliation: `platform.rs`, `platform_admission.rs`, `resource_timing.rs`, `preload_resources.rs`; host cancellation/transport dispatch; parent context admission/HTTP loop; standalone HTTP capture/dispatch/policy/metadata; worker `agent_io` explicitly rejects this document-only category. Existing native-platform preload scheduling, stop, retirement, and readiness hooks now cover both resource kinds. Ordinary `network.rs` cache adapters supply storage; the producer does not duplicate response body storage.

Regression sources: arbitrary non-image/non-script streamed response and exactly one trusted load; removal plus late stream abort; standalone empty metadata/native purpose/hint policy; parent missing-authority/body/mode rejection. Existing image preload tests continue to cover base/encoding reselection and resource timing. Builds/tests remain unexecuted under the global source-first gate.

Known shared source limitation: standalone ordinary Fetch currently has no service-worker interception hook. Parent route does; this change does not claim portable SW parity. Root informed for shared transport ownership. Full Core WPT and five-family qualification are still non-source gates.

## L package open-row reconciliation

See [L-OPEN-ROW-SOURCE-DISPOSITION-2026-09-18.md](L-OPEN-ROW-SOURCE-DISPOSITION-2026-09-18.md) for every open L row mapped to later source delivery, current callers and remaining source versus execution work. This reconciles old module/aggregate-inventory wording without checking broad acceptance rows. Embedded declared-type/source/cancellation remains a genuine unresolved source inventory; no missing algorithm was asserted merely because response types omit an authored `type` carrier. Active portable resource and stream-ordering owners were preserved. No additional confirmed parser/loading source defect was established during this pass beyond the native prefetch producer closed above.

## Test16 termination repair: pending request versus caught exception

Source inspection of pinned rusty_v8 150.4.0/V8 `src/api/api.cc` confirms that
`Isolate::TerminateExecution` requests a StackGuard interrupt; it does not
immediately create the termination exception. `TryCatch` destruction propagates
a caught termination at nonzero V8 call depth, and requires explicit `ReThrow`
to retain that exception at native depth zero. The prior Drop guard incorrectly
queued another request after catcher destruction; it is removed.

`oryn-js::v8_termination::checkpoint` caches an Oryn-private context and `void 0`
script in the existing isolate, with its own explicit microtask queue. A bounded
JavaScript entry services pending interrupts before another author callback is
consumed. It has no author globals, bindings, task delivery, IO, or policy access.
Actual caught termination is then rethrown. Ordinary caught exceptions are
retained without entering this checkpoint. Native callback conversions checkpoint
before later conversions or resource admission; CE queues checkpoint before pop
and after successful callbacks. The private context/script handles drop before
their queue during isolate-owned slot teardown. No C++ bridge or engine/header
ABI changes were introduced.

Regression source includes private-queue/no-microtask behavior, pending versus
materialized termination, immediate post-cancel evaluation, untouched first
reaction on preexisting interruption, already-started reaction no replay, parser
fallback identity, iterator-close ordering, and zero network admission after
interrupted conversions. All source is unexecuted pending coordinated validation.


### Test20 termination boundary disposition

The test20 log reports passing CE reaction suffix/cross-document ordering,
parser constructor and error-listener fallback retention, and native network
conversion interruption. The remaining ARIA termination fixture correctly
received `None` from `Script::run`, then incorrectly expected the exception to
remain without an outer catcher. Pinned V8 `api-inl.h`,
`CallDepthScope::~CallDepthScope`, explicitly clears termination when returning
to native depth zero without a TryCatch handler. The fixture now installs that
host-boundary catcher, asserts both `has_terminated` and the isolate termination
state, rethrows, then preserves the existing explicit cancellation and first-next-
evaluation assertion. No production behavior or interruption assertion was
removed. Cold/warm checkpoint tests have no terminal result in test20 because
the JS process aborted earlier in unrelated destructor cleanup. This fixture
repair is source-only pending the next coordinated batch.


### Test20 retained parser task and reconnect repairs

- `ParserTurnBudget` shares a single preparation slice across the page task,
  its eight host fixed-point passes, nested lifecycle checkpoints, and trailing
  parser resume. The remaining host resource/timer/observer work still advances.
  Readiness prefetch is also skipped after the slice is claimed. RAII releases
  the nested budget on normal completion and errors. Direct low-level parser
  polls each obtain a fresh budget.
- A terminal root input failure marks the document failed before cancellation
  normalization. The caller retains its `Cancelled` error; native author stop
  and replacement-stream cancellation keep their separate normal outcomes.
- Finished-document fixtures now admit replacement input using native
  `document.open()` before installing the retained parser. Reusing the old
  completed lifecycle identity had correctly prevented a second load event.
- Disconnecting a connected node retires an already-captured frame navigation
  identity even before a browsing context has been registered. State-preserving
  moves keep it; removing/recreating generation storage cannot revive it.
- Script.supports coverage now recognizes the implemented speculationrules
  producer and retains case/whitespace/empty/unknown negative cases.

New source regressions:
`nested_page_checkpoints_share_one_parser_input_slice_and_release_budget`,
`parser_turn_budget_resets_after_early_error_and_nested_unwind`, and
`disconnected_request_identity_expires_without_a_context_generation`.
The existing task/EOF/error and reconnect assertions remain. The 130-source
burst failed during its initial document load, before burst admission; root
owns construction performance diagnosis, and its deadline is unchanged here.
No tests or builds executed in this repair batch.


Independent repair35 review follow-through: terminal root input errors now call
`observe_native_root_stop` before returning the original error, so marking the
load stopped cannot suppress frame input, CSS preparation, lifecycle barrier,
and resource cleanup. Legacy buffered-input and completion branches also claim
the shared task budget before consuming work. Setup/restore/activation failures
rotate their document owner to the back of the queue; the ordered script deque
within that owner is unchanged. New regression sources retain a real child input
and 4096-byte resource reservation across root failure, repeatedly fail one
realm's setup with already-expired deadlines while its healthy child runs once,
and exercise buffered source observation and actual nested panic unwinding.
Loading lane is performing the final source-only review; no execution occurred.


### Test22 / repair37 source disposition

The termination/CE interruption regressions no longer appear among test22
failures. Remaining parser fixture failures shared a premature synchronous
construction return: the old loop stopped at parent tokenizer completion while
native child parser and lifecycle work remained. The constructor now waits for
actual document publication and descendant/resource work. Scheduled construction
still returns its owner without reading input. Native author-created, unclosed
streams may return incomplete once they alone remain; this exemption affects
construction waiting only, never the strict load-event barrier. Active lifecycle
receipts and retry stages, ordinary sibling work, and retained script/network
receipts prevent that exemption. Existing unclosed-child stop/remove assertions
remain unchanged.

New source coverage includes
`synchronous_load_waits_for_child_scripts_and_load_while_scheduled_load_returns_owner`,
`unclosed_author_stream_does_not_hide_retained_network_script_work`, and
`construction_retains_child_load_receipt_when_handler_opens_descendant`.
The parser-limit fixture now inspects the actual retained production owner;
the import-map replay fixture commits its native document URL after installing
its document, preserving relative-map resolution and all replay assertions.

The 130-source setup timeout also had a concrete boundary cost: every inert
application/json script forced an entire page task. HTML parsing now preserves
those captured Data events without treating them as resource-aware task
boundaries. Captured ImportMap and SpeculationRules kinds still pause before
later tokens, as do executable scripts. Byte limits, actual script slices and
construction deadlines are unchanged. The 130-source native fixture is unchanged;
`inert_data_blocks_share_a_slice_but_real_scripts_and_policy_keep_boundaries`
covers the native parser boundary contract. No tests/builds executed in repair37.

## Repair38: test23 parser disposition and source repairs

Test23 passed the 130-ready-response burst, module snapshot replay, synchronous descendant construction, passive author-input exemption, mixed sibling/resource custody and retained lifecycle receipt regressions. The remaining Data checkpoint failure exposed an overbroad optimization: inert script blocks still require preparation microtask checkpoints before subsequent tokenizer input. HTML again emits captured Data boundaries; the native frame parser batches at most 64 such boundaries and 64 KiB from an already-retained suffix, running the existing checkpoint at each boundary. Executable/policy events, newly queued jobs, owner retirement and new source reads end the batch. New `data_block_batch_keeps_each_microtask_boundary_and_bounds_one_source_slice` covers 70 Data blocks, observer timing, source-poll custody and the following actual script boundary; DOM `data_and_policy_boundaries_preserve_preparation_checkpoint_insertion_points` covers Data/importmap/speculation boundaries.

The page-budget test previously looked for the retained frame parser before the scheduled owner had been polled. It now verifies identical page budget handles on initial scheduled input and on the retained owner after a Pending read, retaining all capacity/stop assertions.

The buffered data-frame timeout had a concrete production cause: both native discovery and dynamic loading required an HTTP broker before entering the existing local data/blob loader. Local scheme admission now routes through that loader with the existing fail-closed UnavailableBroker only when the captured request URL is data/blob. HTTP still requires an actual broker; blob bytes and policy still come from the native registry. Added local capture/raw-byte/cancellation/HTTP-negative coverage and a dynamic blob child regression; the original data-frame encoding assertion is unchanged.

The async boolean fixture now distinguishes executed parser classic (false) from rejected nonexecutable Data preparation (true): preparation restores force-async for a parser script that cannot yet execute. Existing native flag semantics are retained. Loading's deferred-then termination fixture was separately diagnosed against pinned V8 `src/objects/module.cc::JSDeferredModuleNamespace::TriggersEvaluation`: `then` and symbol keys deliberately do not trigger deferred body evaluation. Loading owns its fixture reconciliation; no engine/termination workaround is appropriate.

All repair38 additions remain source-only until the root freezes and executes the complete batch.
