# Parallel navigation generation — September 18

Source generation and inspection only. No build, test, formatter, generator, service, or conformance execution occurred. This is bounded R-04 evidence; the complete navigation parent remains open.

## Generated implementation

### Joint timeline disposal and bounded ownership

`crates/oryn-js/src/joint_history.rs` now retires native completion receipts when replacement, forward-history truncation, or single-route disposal removes their exact destination. Previously single-route `discard` removed the timeline entry but left its receipt waiting indefinitely; replacement and push could similarly leave an impossible destination pending. Retiring after the accepted mutation preserves the newly authored cursor/revision, and late commit/publication cannot replace the terminal retirement result. Other receipts whose destinations remain are preserved.

Serialized execution records now reject empty pending sets and unknown route/entry destinations before import acceptance. This complements the existing aggregate byte validation; that existing validation already enforced its bound and was not missing. The separate push admission calculation did omit retained execution/rollback metadata. Push now starts from the complete retained allocation and subtracts disposed forward entries before admitting the new snapshot, preserving the old state on rejection.

Generated regression sources:

- `disposing_one_route_retires_only_its_receipt_and_late_publication_cannot_complete_it`
- `forward_truncation_retires_disposed_destination_without_rolling_back_author_push`
- `replacement_disposes_old_destination_but_failed_replacement_preserves_receipt`
- `serialized_execution_rejects_missing_route_or_destination`
- `push_budget_includes_retained_execution_metadata_before_mutation`

### Prerender descendant retirement ordering

`crates/oryn-native/src/prerender_commit.rs` now merges refreshed cleanup snapshots in current descendant-before-parent order. The old append-only merge placed a newly created grandchild after a parent already captured by the first snapshot. Parent retirement could consequently invalidate a later child's cleanup authority. The merge preserves removed historical owners first, then replaces duplicate current owners with the complete current postorder. Both the activation-preparation and cancellation-recovery snapshot consumers use this operation. Exact generation/epoch matching continues to distinguish old and new streams.

Generated regression sources:

- `newly_created_descendant_precedes_its_previously_captured_parent`
- `detached_owner_stays_before_current_ancestors_and_new_stream_is_distinct`

The existing successful activation/BFCache regression also now initializes its source timing references in the source Document and freezes its timing JSON before departure. Previously that regression referenced globals initialized only by a different cancellation test.

## Combined source review and remaining dependencies

Reviewed actor activation phases and existing original-response/body ownership, history publication, promotion ACK retry, authored-successor preservation, root suspension rollback, and private descendant cleanup snapshot producers. Existing source already carries created-document activation and activationStart; older audit statements saying these algorithms are wholly absent are historical. No new common host/runtime/context-service hook is required for the changes above.

Still open under the broad R-04 source gate:

- Full nested/cross-origin ancestor repopulation fixtures and combined frame-route restoration faults; these new receipt tests are timeline-level evidence, not that actor matrix.
- Full created-response stop/network failure and replacement activation fixture matrix, including delayed cleanup ACKs after callbacks create descendants.
- Complete deferred API, trigger/target-hint/redirect inventory before enabling speculation-rule support.
- Complete pinned scroll/fragment/layout-growth and form-restoration fixture reconciliation. Existing entry/mode/growth implementations were preserved; this lane does not claim that complete matrix.
- Final combined authority/lifecycle and source-generation gate review, followed only then by build/test/fix and pinned conformance qualification.

The root integration owner owns checklist updates. No broad requirement should be marked complete solely from these bounded changes.

## Target sanitization cross-lane review

Read pinned `html/browsers/windows/dangling-markup-window-name.html`. It requires reset for anchor targets combining `<` with LF, CR, or TAB; form/base use LF plus `<`; `window.open` preserves the same string. The existing `submissions.rs::normalize_target` AND condition with all three control characters matches those exact cases. No condition was changed based on an unverified OR interpretation. This bounded pinned review does not establish independent-character behavior absent from that fixture.

## Continued production-route reconciliation

### Remove obsolete parser algorithm and retain EOF

The reported consuming `LiveParserState::resume` route was already `#[cfg(test)]`; production roots and frames use `FrameScriptPreparation::poll_html_parser`. Removed the duplicate consuming algorithm, its `LiveParserStep` return enum, and its insertion helper. Replaced its eight tests with `parser_continuation_tests.rs`, a child test module of the actual production preparation owner. Coverage retains pending-versus-EOF distinction, per-slice yielding, stop/replacement input disposal, pending async execution/adoption, bounded completion bursts, resource lifetime, external script success/failure insertion, and stylesheet completion/failure/media/disabled/removal release. Async burst assertions use the actual production eight-job bound, rather than the obsolete driver's 64-completion behavior.

Production `poll_html_parser` already transfers consumed suffixes and events before reporting construction errors. It did not latch EOF on the direct polling route before fallible metadata/construction. EOF is now retained immediately after receipt; retry cannot poll terminal input again. Added post-tokenization and post-EOF fault points and regression sources proving one constructor/body execution, original element identity, preserved suffix, and exactly one EOF source read across retry. Fault points are test-only; no injected failure was executed.

### Actual nested network restoration fixture

`frame_form_history.rs::cross_origin_nested_network_history_restores_parent_selection_and_each_documents_controls` exercises a root at root.test, a selected second parent entry at outer.test, and a nested second child entry at inner.test. It captures and imports native frame history and verifies restored URLs, independent controls/names, and cross-origin top-document denial. This extends the earlier same-origin srcdoc ancestor test with the network/cross-origin route. No passing result is claimed.

### Commit boundary during activation cancellation

`prerender_commit.rs::recover` previously restored the predecessor even after the successor executable, Document and session history had been locally installed. It now preserves that committed Document on caller cancellation, stop, or later body failure. Recovery completes the retained activation event suffix and releases the canceled caller's original response capture; normal page work owns any surviving parser. Native promotion before local installation still follows exact retirement/rollback. Added `cancellation_after_local_install_keeps_committed_document_and_activation_suffix`, covering cancel and native stop immediately after installation, with Document/global/history/parent-URL identity, one activation event and one fetch.

### Pinned scroll source matrix

Read pinned `scroll-restoration-order.html` and `persisted-user-state-restoration/scroll-restoration-fragment-scrolling-samedoc.html`. Added actual joint delta regressions instead of fabricating publication events:

- `joint_back_restores_each_saved_position_between_popstate_and_hashchange`: three consecutive back operations through saved 100/200, 50/100, and zero offsets; popstate observes the outgoing offset and hashchange observes restored state.
- `manual_mode_allows_new_fragment_navigation_but_blocks_history_fragment_restore`: manual mode does not suppress new fragment navigation, but does suppress back-traversal restoration; auto is a positive control.

Existing saved-scroll growth, manual override, document replacement, cold load/pageshow and interrupted event-suffix sources remain in place. Full WPT execution and every cross-origin fault permutation remain unqualified; the additions above are explicit source coverage, not a broad checklist completion.

## Deferred Fetch generation (parallel native backend)

Added protocol-neutral `fetch_later.rs` capability trait and V8 deferred-fetch
registry/binding. Native Request conversion supplies normalized methods, headers,
credentials, referrer and body bytes; a new source-known flag distinguishes
explicit streams (rejected) from replayable bodies. Secure Window exposure,
nonconstructible FetchLaterResult with branded read-only activated getter,
finite nonnegative activateAfter, trustworthy HTTP(S), aborted-signal and request
size admission checks are generated. Document suspension and retirement activate
all remaining requests; abort publication cancels pending requests before author
listeners; transport ACK retries retain capability identity. Native parent lane
owns aggregate quotas, frozen policy/transport, native retirement backstop and
protocol inventory. Generated regression sources cover timer/default/long delay,
BFCache-style suspension, retirement, abort, body extraction, invalid schemes,
stream rejection, quota, result branding and lost activation ACK.

These are source-generation evidence only: no build, formatter, tests or WPT
execution has occurred. Native policy quota delegation and backend integration
remain under the parallel native lane's review; this is not a conformance claim.

Root source review closed additional deferred-fetch custody gaps: pending records
retain Request and AbortSignal wrapper roots until native terminal ACK; dropped
results do not cancel admitted requests. Local registry capacity is reserved
before author conversion and rechecked after reentrancy, bounding failed prepare
and failed cancellation cleanup without dropping work. Document generation,
authority, suspension and retirement are rechecked after dictionary getters.
Regression sources now include GC followed by controller abort, retirement from
activateAfter conversion, repeated lost admission/cancel ACK capacity and retained
retirement cleanup. Original getter exceptions are rethrown unchanged.
Permissions Policy recognizes deferred-fetch (self) and deferred-fetch-minimal
(star); source tests preserve explicit denials. Request byte helper excludes URL
fragment; effective frozen referrer sizing belongs to native admission.

## Continued R-04 detach/promise source reconciliation

Pinned `navigation-api/navigate-event/signal-abort-detach-in-onnavigate.html`
requires the captured NavigateEvent signal to abort and invoke its abort listener
when the iframe removes itself during `navigate`; its return-value counterpart
requires both navigation promises to reject AbortError. Existing code rejected
those promises after observing retired execution but never aborted the signal.
Generated pre-retirement frame discard cancellation now runs while its original
realm is callable, before native message/environment/execution retirement.
Reentrant removal is guarded, reentrant navigation rejects InvalidStateError,
and publication failures keep the closed frame and exact abort suffix for the
next checkpoint. Pending successful completion suffixes become cancellation;
completed callback prefixes remain consumed. Native lifecycle test plus direct
binding tests cover synchronous signal/dual promise rejection, reentrant API
rejection and callback fault/retry without replay. Also repaired the missing
AdmissionDocument field in the older admission-cancellation Completion initializer.
No tests/builds have run; this does not close the complete target/scroll matrix.

### Fresh fragment scrolling

The fresh-document load receipt now resolves an initial URL fragment before load
handlers and pageshow, without synthesizing popstate. This is navigation scrolling,
so manual history restoration does not suppress it. Traversal continues to honor
manual mode. The receipt consumes the scroll before callback delivery; retry after
a load-listener fault preserves that listener's authored scroll rather than
scrolling to the fragment again. Generated regression exercises manual mode,
load/pageshow ordering, no popstate, and callback failure/retry.

### Native popup interaction cookie heuristic

New `popup_storage_access.rs` and native creation/input hooks capture an auxiliary
page's actual opener top origin before child execution, then grant only after an
authenticated native interaction in the popup root. Admission revalidates its live
native Document, current popup origin and retained native opener/group relation.
No grant occurs from merely opening a window, an ordinary window interaction,
noopener, an opaque/insecure principal, a stale generation or a severed opener.
The grant uses the context CookieStore's bounded schemeful-site/origin permission;
URL parameters and authored window fields are not authority. Generated positive
and negative native regression sources remain unexecuted.

### Deferred capability replay and terminal errors

Coordinated the native backend's bounded terminal replay inventory: each JS
capability now begins with a checked per-Document monotonic u64 sequence and
24 entropy bytes. Every attempted admission consumes a sequence, including failed
admissions. Only transport failures retry; native expired/invalid/quota terminal
replies release local roots without inventing activated=true. Regression sources
assert failed-admission sequences and terminal-expiry cleanup.

### Remaining integration/qualification boundaries

Target/name/noopener/sandbox UTF-16 helper closure is owned by the DOM/input lane;
this lane did not duplicate its edits. Native quota reservations before iframe IO
are owned by loading/native transport, and the backend's bounded replay inventory
is still being reconciled in that lane. Full history/scroll/frame qualification
requires the global generation gate and subsequent build/test/WPT stage; no
execution result is implied by the source matrix above.

### Reload scroll versus fresh fragment

Read pinned `scroll-to-fragid/scroll-restore-on-reload-non-ascii-fragment.html`:
both ASCII and percent-encoded non-ASCII fragment reloads preserve the prior
saved offset. Reload now arms saved-scroll restoration before load/pageshow,
without popstate; traversal retains its distinct post-pageshow/popstate ordering.
This prevents the newly generated fresh-fragment path from overriding reloaded
saved scroll. Generated source regression covers both fragment encodings and
load/pageshow/no-popstate observations. Native selected-entry capture remains the
existing producer; all execution remains deferred to the global gate.

Root review also separated terminal result metadata from pending request custody:
activated is transferred to a native private boolean on the result wrapper, then
the completed record/backend/token leaves the registry. Retaining completed result
objects therefore does not exhaust the pending-request cap. The native poll and
new admission finalize records with a V8 scope; getters finalize results completed
by scope-free deactivation. A generated regression retains more than 1024 completed
results and confirms all getters remain correct while pending custody stays empty.

### Admission stream identity and independent backend review

Navigate admission now captures its Document identity before author conversion
and dispatch. A changed stream cannot enqueue the old callback tail or create a
cancellation completion bound to the replacement stream. Generated document.open
regression checks dual AbortError rejection, unchanged destination URL and no old
queued publication.

Independent source review of the native deferred backend identified three issues
handed to its owner for correction: standalone backend liveness needed exact
Document/stream identity; isolate-shared standalone roots needed distinct quota
and replay issuer namespaces; and original-document Permissions Policy admission
needed separation from same-origin shared quota ownership. The native lane owns
those edits and resulting regressions. None of these reviews executed code.

## September 18 full R04/R05 source disposition

This table records source review, not a test result. A broad row remains open
when another caller or a native producer still needs generation.

| Checklist obligation | Current source disposition | Remaining work before closure |
| --- | --- | --- |
| R-01.b / R-03.b insertion, initial claim, sandbox | Native insertion/initial reuse/commit, typed parser continuation and original owner guards exist. Obsolete test-only LiveParserState advancement was removed. | Loading/parser lanes reconcile all insertion and quota reservation callers; then execute matrices. |
| R-02.a Window/Location | Structural and lifetime submatrices exist; this lane added discard admission and original Document identity before author conversion. | Remote independent-actor attachment and complete familiar/allowed navigation relationships remain shared integration work. |
| R-04.prerender-core-dependency / actor-source-review / loader-activation | Immediate/eager same-context HTTP/parser loading, private root/children, accepted promotion, retained rollback and event publication exist with actor tests. Source review found loaded candidates stopped executable-owned work; roster now continues bounded post-load turns while preserving one loaded ACK. New regression creates a child after load and observes its parser ran while prerendering. | Moderate/conservative gesture admission and target_hint `_blank` still deliberately excluded in roster. Those are real producer omissions for broader speculation support; feature support remains disabled. Full combined actor qualification remains unrun. |
| R-04.authority-recovery-review | Parent exact restore/discard receipts, retained cleanup stages, terminal expiry, original host retention, failed suspension rollback and prerender post-install recovery have source implementations/regressions. | Combined failure matrix execution and root integrated authority review, not a replacement implementation. |
| R-04.root-departure-review | Actor HTTP/local/data/blank/cache input custody, descendant pagehide tree, callback progress, staged history and retained cached publication exist. | Combined authority-fault execution and source integration with close admission. |
| R-04.joint-steps | Shared bounded timeline, disposed receipt retirement, saved frame routing, nested cross-origin selected entries, descendant ordering and source lifetime regressions exist. | Joint fixture execution, including intercepted completion and replacement failures; no claim of qualified conformance. |
| R-04.history-scroll-matrix | Per-entry mode/offset, layout-growth continuation, fragment ordering, fresh load fragment, reload saved scroll, same-document events and original-entry guards generated. Reload ASCII/non-ASCII and fresh manual-fragment fault cases added. | Execute native and pinned fragment/BFCache matrices; verify layout lane integration. |
| R-04.b / R-05.a broad target/history/lifecycle | Navigation discard synchronously aborts its captured signal and rejects pending promises before frame retirement; reentrant abort suffix and document.open guards generated. Target/noopener/sandbox UTF16 changes belong to DOM lane. | Script-close eligibility found genuinely missing: ordinary self-close previously ignored history/provenance. Native full-history projection, immutable provenance, root-owned synchronous public admission and retained target departure now generated; source integration and execution qualification remain. |
| S-03.c / R-04.d target selection | Local native names/tree/target policy and source identity paths exist; pure UTF16 target/rel policy repairs delegated to DOM/input. | Remote familiar/allowed relationship closure and combined named/reused/noopener/COOP matrix remain integration work. |
| S-03.d / R-04.e supersession/close | Controlled navigation and prerender accepted-cancel recovery, retained script-close physical teardown and pending promise discard exist. | Admission/cleanup split and ordered retained departure generated; integrated source review and execution remain. |

### Deferred Fetch native timer and observation custody

The public backend now receives activateAfter duration and reports Pending,
Activated or Canceled. Result getters query native state before observing the
terminal boolean, so a parent timer can win while JavaScript is busy and a late
abort preserves that winner. Terminal state is stamped privately before the
acknowledge call permits parent replay eviction. Lost observation ACK retains
only bounded backend/token cleanup custody, with a retry deadline; request and
signal roots are already released. Generated regressions cover independent parent
timer observation, late abort, durable terminal state and lost-ACK cleanup.

### Native script-close source

`native_history_has_single_entry` reads the full native session history,
including foreign forward entries omitted by Navigation.entries(). The canonical
HTML close algorithm requires exactly one session history entry for a traversable
not created by web content. Fragment and pushState entries count separately even
when they share a Document. This corrects the earlier distinct-Document reading.
Immutable created-by-web-content provenance supplies the other eligibility branch.

Parent admission now reads an exact-Creator history projection. Begin marks the
projection unavailable before local history mutation; Publish acknowledges the
new count before author history notifications; Abort restores the prior count.
Lost acknowledgements retain the same revision and disposition. Initial history,
full-session synchronization, staged commit and prerender activation publication
have source hooks. A local flush precedes close RPCs. Remote pending projection
reads wait outside native authority locks with a bounded deadline; absent or
unresolved projections are infrastructure errors, never ordinary ineligibility.
Eight global blocked-read permits preserve capacity for publication RPCs.

Root's public admission checks native familiarity, target sandbox and exact source
and target authority, reserves a runnable retained operation and synchronously
sets sticky is-closing. The runner starts in Preparing, so later history changes
do not revoke accepted definitely-close. Canceling beforeunload releases operation
custody but keeps Window.closed true; subsequent calls do not repeat departure.
The actor query remains available for internal reconciliation, not as a substitute
for synchronous author-visible eligibility.

Auxiliary cleanup is separate: RollbackAuxiliaryRoot authenticates its own admitted
endpoint, web-created provenance and initial native root identity, and rejects
generation/stream replacement. Both failed-construction callers use it. Ordinary
author ClosePage requires source Creator and cannot invoke this bypass.

Source regression matrices cover projection reservation/abort, lost publication
ACK, foreign/replaced authority, concurrent publication, bounded pending failure,
synchronous public closed state, canceled unload and no duplicate departure. These
are generated sources; build, execution and pinned WPT qualification remain due.

### Target-owned close departure generated

PrepareScriptClose(token, expected Creator) now has an async PageHandle/worker/
PageCommand route and returns Pending, Allowed(proof) or Canceled. The target
retains one bounded admission, preserves the same token across retries, replays
terminal outcomes. Internal callers can replace a canceled token; the public sticky
is-closing gate prevents another author close operation. Replacement drops
only the obsolete owner's custody. A shared retained native beforeunload tree
uses existing BeforeUnloadReceipt objects; the former local per-document retry
wrapper was replaced by this owner. After permission, existing page transition
receipts publish pagehide(false) and unload(false), without pageswap/history
mutation. Parent owns accepted-close authority and awaits Allowed before physical
teardown. Sources cover descendant order, retained beforeunload prefix, replay,
prompt dismissal preserving the document, and replacement during pagehide.

### Keepalive Request metadata generated

Request.keepalive now persists conversion, inheritance and clone, with explicit
false override and stream-source rejection (including inherited streams). Fetch
options and service-worker fetch request metadata carry the flag; ordinary fetch
uses the Request value and deferred fetch forces true. IDL getter/dictionary
source updated; generators remain deferred. Native shared in-flight body quota
and response/retirement custody are owned by family/loading lanes, so metadata
alone is not full keepalive qualification.

### Navigation generation disposition after close integration

The consuming test-only LiveParserState route and old beforeunload tree wrapper
were superseded by retained production owners. Native close, projection and
prerender activation paths now have concrete source implementations and regression
sources. R04/R05 aggregate checklist rows still require the combined native build,
fault matrices and pinned WPT run; generated source is not a passing conformance
result. Prerender eagerness/_blank coverage is concurrently owned by parser lane,
target/name/embedded opener work by DOM lane, and cross-API keepalive transport by
family/loading lanes. Those active source obligations must not be marked complete
based on this navigation report alone.

### Accepted close owns the traversable

Canonical definitely-close selects the active Document when the queued task starts,
then selects the active Document again for the traversal unload step. The actor
now retains the admitted Creator as token/replay metadata while selecting current
native state for departure. A replacement between admission and task execution no
longer strands a logically closed live page. Parent runner trusts the same target
actor/token outcome rather than requiring the obsolete admission Document proof.

The beforeunload receipt now retains the specified unload counter through dispatch
and prompt; close departure also retains descendant counters across pagehide and
unload. Document.open/write therefore cannot replace those unloading documents.
Sources cover pre-task replacement, replacement between permission and traversal,
ignored destructive writes, terminal replay and actual canceled prompts. No build
or execution has been performed.

Completed departure now fences actor author commands centrally, including LoadHtml,
controlled navigation, evaluation, WebDriver and capture entrypoints. Page-task and
navigation polling stop while the parent queues physical Close. Exact close replay
and non-author metadata/cleanup remain available. Existing pending capture and
navigation replies retain custody until Close teardown; they are not dropped by
the fence. A native actor source regression checks rejected author commands,
terminal replay, retained capture and destruction-before-close-ACK ordering.
The fence is conditional on Allowed, so dismissed beforeunload leaves work live.

Close waits for an earlier native Departure/Publication transaction before taking
the root. Once its retained departure starts, navigation transaction commits pause
until cancellation or physical teardown. Network preparation alone does not delay
close. This serializes retained departure retries with native traversal commits.

## R package open-row reconciliation — September 18

This table reconciles the 18 open rows in the R section against the full plan's
native lifecycle deliverable. **Generated** means production source and regression
source exist. **Source review** means a specific coverage/integration obligation
remains; it is not an assertion that the whole feature is absent. No row has
execution evidence from this pass, and the global build/test gate remains closed.

| Open row | Actual source disposition | Remaining obligation / ownership |
|---|---|---|
| R.navigation-timing-source-closure | Original clocks, transport/terminal sizes, cache provenance, lifecycle and activation records are generated. `navigation_timing.rs`, root/frame producers and ServerTiming cover the audited direct Core dependencies. | Source caller closure remains, principally dynamic helper inventory; use the timing audit's final sections rather than superseded unknown-size bullets. Generic ResourceTiming workerStart is a separate L/G obligation. |
| R.navigation-timing.caller-matrix | Initial/snapshot, reload/cold/cache, prerender root/child, held body, cancellation and report-hash cases now have regression sources. | Dynamic/transitive caller inventory and final combined lifecycle review remain; no broad execution-only designation yet. |
| R-01.b / R-03.b | `ensure_native_initial_frame`, insertion checkpoint, `claim_initial_frame_document`, commit/reuse and retired-environment guards exist; claim tests preserve author objects and sandbox opaque identity. | Parser/DOM insertion and target-name lanes are still changing their callers; finish their combined insertion/reuse/sandbox matrix, then execution. |
| R-02.a | Native structural Window/Location surface, descriptors, caches and frame rebinding are generated; same-agent auxiliary attachment and native window.open now exist. | Complete independent-agent/group-transition and restored-root receiver/cache matrix with P/DOM integration. Existing property tests do not prove that whole attachment matrix. |
| R-04.prerender-core-dependency | Native private candidate loading, preserved Document activation and timing sources exist. | Active prerender lane owns eagerness, target and support-exposure closure; keep open until its final source review, then execute the pinned helper chain. |
| R-04.prerender-actor-source-review | Retained roster/load/milestone/activation/cancel/rollback sources and actor fixtures exist. | Active prerender lane combined child/message/activation-fault reconciliation. Not an execution-only row while that generation is active. |
| R-04.prerender-loader-activation | HTTP/parser scheduling and retained promotion/event receipts exist. | Same active prerender source scope; avoid generating a duplicate loader. |
| R-04.authority-recovery-review | `cached_authority_completion.rs`, `initial_document_cleanup.rs` and root failed-install/source-resume owners retain exact replies, cleanup suffixes and original errors. Lost/wrong ACK, expiry, stale policy, unreceived park and reentrant completion sources exist. | No newly missing recovery algorithm identified in this pass. Combined nested-owner/network fault execution remains; parent source integration review can distinguish this bounded generated algorithm from broad joint-history qualification. |
| R-04.root-departure-review | HTTP/local/buffered/cache bodies and staged history remain actor-owned through swap/hide/unload and post-hide snapshots. New selected-tree guards cover pagehide destructive writes; snapshots now follow pageswap callbacks. | Lifecycle visibilitychange producer is a newly identified source gap, separately assigned/requested below. After that integration, combined parent authority-failure execution is still due. |
| R-04.joint-steps | Shared native timeline, exact route receipts, ancestor source archives, controls/names and cursor rollback are generated. `cross_origin_nested_network_history_restores_parent_selection_and_each_documents_controls` adds nested source restoration coverage. | Real actor combined nested/cross-origin traversal plus authority failure remains a fixture/integration gate; the resource-map native fixture alone is not that complete actor matrix. |
| R-04.history-scroll-matrix | Entry-local mode/position, fresh/reload ordering, popstate-before-restore-before-hashchange, manual cancellation, zero capture and bounded layout-growth retry are generated. Form restore admission/callback custody is separate and generated. | Final pinned cross-origin/nested joint integration review and execution; no additional scroll algorithm defect identified by this pass. |
| R-04.b / R-05.a | Most concrete subalgorithms now have source: targets, history, fragment, names, authority, close and retained lifecycle. | Remains the aggregate source gate for active prerender/DOM/portable work, lifecycle visibility and the combined actor matrix. It must not describe Navigation API generation as wholly absent. |
| R-06.a | Production `FrameScriptPreparation` now owns HTML and XML continuations; the consuming test-only LiveParserState driver was removed. | Active parser/stream lane complete-caller reconciliation; old checklist references to the removed driver are historical. |
| R-06.j | Module graphs and primary standalone scripts use retained preparation; direct compatibility waits remain explicitly bounded. | Portable frame resource/departure extraction is now generated: shared response enum, bounded broker preparation, current-owner checks, departure receipts, source commit, readiness/stop/retirement and all executable initializers. Parent IPC body leases remain Unix-specific; standalone broker loading uses the existing buffered response contract on every platform. Accounting/representation integrations remain separately owned active source review; portable compilation and execution are deferred. |
| R-06.k | Incremental XML and typed HTML/parser inputs, token suspension, retained script/resources and failure custody are generated. | Active parser lane owns remaining document.write/fairness/stylesheet matrix. |
| R-06.l | Stable Document authority is independent of parser stream; stop/new work, pending response and lifecycle identity sources exist. | Active parser lane stop/supersession caller reconciliation; combined BFCache/native authority execution remains. |
| R-06.b / R-07.b | Scheduler parking/retirement, native callback receivers, retained publications and scoped ownership are generated in multiple lanes. | Still a cross-package source audit: all task-source/realm/borrow boundaries cannot be established from navigation tests alone. Root/DOM/authority/parser integration must close the inventory. |
| R-05.public-departure-integration | Prior correction is generated: current Document selection at queued/traversal stages, sticky closing, retained prompt/tree guards, typed actor rejection fence, traversal ordering and physical ACK custody. Portable helper shares the native owner. | Visibilitychange during departure is the remaining newly identified lifecycle source dependency. Existing close code is not missing wholesale; execution remains deferred. |

### Newly confirmed R lifecycle source correction

Pinned Core `html/webappapis/dynamic-markup-insertion/opening-the-input-stream/ignore-opens-during-unload.window.js`
requires ignored opens during beforeunload/pagehide/unload, including child handlers
opening a departing parent, and requires the parent to remain writable when only
its child departs. The ordinary root/frame pagehide paths lacked these guards.

Generated `begin_native_unload_tree` snapshots only the selected native subtree.
Root and dynamic-frame departure retain its guards from pagehide through unload,
including retained errors; cancellation/drop releases them. Pagehide recipients
are selected after pageswap, so changes made by pageswap are reflected. The shared
beforeunload tree now retains parent protection through descendant callbacks.
Regression sources cover persisted/nonpersisted root departure, abandoned retry,
nested parent-vs-child selection and unchanged accepted frame replacement.

The same pinned fixture also demands visibilitychange during unloading. Current
native Page publication produces pagehide/pageshow without a visibility event;
external minimize/permission snapshot visibility is a different producer. This is
a concrete remaining source dependency reported to root/G ownership. Its state
must compose with minimized/prerender visibility rather than unconditionally
making a background page visible on restoration.

## Portable frame custody extraction (source only)

`frame_resource_preparation.rs`, `frame_departure.rs` and `standalone_frame_resource.rs` are now unconditional executable modules. `FrameResourceCompletion` uses the portable `frame_resource_response::FrameResourceResponse`; only the actual parent response body lease and parent transport scheduler remain Unix-specific. The obsolete non-Unix dynamic-frame resource-map-only branch was removed in favor of the common native broker producer, retained ready queue, selected-subtree departure and commit pipeline. All executable initializers, parser-frame failure transfer, parent load barriers, task readiness, stop and retired-realm cleanup share this custody.

The shared source regression matrix includes held IO retaining the incumbent, replacement/stop rejecting late script and document publication, one fetch admission, source-byte release, disconnected producer handling, departure retry/removal, snapshot and streaming parser insertion with classic/document-written/module/style resources, and policy/referrer/encoding/redirect coverage. The selected-subtree ignore-opens matrix now includes visibilitychange as well as pagehide. These regression sources have **not been executed**.

This extraction does not claim that a standalone buffered broker response delivers headers before EOF: the parent IPC stream lease remains a separate transport implementation. Root owns portable response byte accounting; automation owns associated completion/policy reservation integration; loading owns embedded representation type propagation. No remaining confirmed extraction code gap was found in the reviewed frame loader/departure callers; those concurrent integration reviews and all compilation/platform qualification remain pending.
