# Pinned joint-history fixture source reconciliation — September 17

Source root: WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`, `html/browsers/browsing-the-web/history-traversal/`. The policy's `html` Core prefix covers these test paths. These are source files, not manifest entries or a replacement denominator. Helpers and metadata are explicitly separated. No builds or tests ran.

## Confirmed requirements and current source ownership

| Family | Algorithms/callers | Regression evidence / concrete residual |
|---|---|---|
| Event ordering | Native Location fragment admission, History FIFO tasks, retained publication and native platform task poll | Generated inline popstate + deferred FIFO hashchange, pushState-in-popstate and document.open cancellation tests. Traversal popstate/scroll/hashchange regression generated. |
| Scroll/timers | Entry-local snapshots, native scroll projection, cached/fresh root/frame lifecycle, suspended host timers | Generated mode, scroll order, load retry, layout growth, manual cancellation, zero capture. Timer restoration uses existing suspended host ownership; full timer fixture caller review remains. |
| Joint traversal/srcdoc | Root PageState history, frame SessionHistory, saved frame paths/source archives | Generated shared bounded step model, root-visible length, multi-route native traversal, initial-frame replacement, global forward truncation projection, persisted timeline and saved frame-route/archive selection. UI replies own all-selected commit receipts across root replacement. Generated removal pruning, exact uncommitted cancellation rollback, ancestor restoration with shared child archives and network controls. Remaining source review: complete nested/cross-origin fixture matrix and combined caller/fault review. No execution qualification. |
| Original document state | Root traversal/reload request metadata | Parent agent owns exact original initiator/referrer persistence against document-state.https.html. |
| Context names | Native frame lifetime name cells; related root navigation/name inventory | Existing WindowProxy audit/regression sources. Joint name fixtures use the generated shared cursor; full fixture reconciliation remains open. |
| Pageswap/pagereveal | Native page transition receipts and destination disclosure | Existing source tests in host/rust_executable; remaining fixture-level redirect/hidden/prerender reconciliation must distinguish missing algorithms from runner qualification. |
| Baseline events | Native History/PopStateEvent/HashChangeEvent/PageTransitionEvent bindings | Existing trusted event/constructor and lifecycle tests. `001.html` has legacy bubbles/cancelable expectations that conflict with modern events.html; keep pinned discrepancy explicit, do not change modern event flags to satisfy contradictory fixtures. |

## Complete source-file membership

### History events/interface and baseline traversal

- `001-1.html`
- `001-2.html`
- `001.html`
- `PopStateEvent.html`
- `api-availability.html`
- `events.html`
- `hash-empty-string.html`
- `hashchange-same-fragment-does-not-fire.html`
- `hashchange_event.html`
- `popstate_event.html`
- `same-url.html`

### Metadata (not test entries)

- `WEB_FEATURES.yml`
- `event-order/WEB_FEATURES.yml`
- `pagereveal/WEB_FEATURES.yml`
- `pageswap/WEB_FEATURES.yml`
- `persisted-user-state-restoration/WEB_FEATURES.yml`
- `srcdoc/WEB_FEATURES.yml`

### Browsing-context name persistence/reset

- `browsing_context_name-0.html`
- `browsing_context_name-1.html`
- `browsing_context_name-2.html`
- `browsing_context_name-3.html`
- `browsing_context_name-4.html`
- `browsing_context_name.html`
- `browsing_context_name_cross_origin-0.html`
- `browsing_context_name_cross_origin.html`
- `browsing_context_name_cross_origin_2.html`
- `browsing_context_name_cross_origin_3.html`
- `unset_context_name.html`
- `window-name-after-cross-origin-aux-frame-navigation.sub.html`
- `window-name-after-cross-origin-main-frame-navigation.sub.html`
- `window-name-after-cross-origin-sub-frame-navigation.sub.html`
- `window-name-after-same-origin-aux-frame-navigation.sub.html`
- `window-name-after-same-origin-main-frame-navigation.html`
- `window-name-after-same-origin-sub-frame-navigation.sub.html`

### Original request document state

- `document-state.https.html`

### Event task ordering

- `event-order/after-load-hash-twice.html`
- `event-order/after-load-hash.html`
- `event-order/after-load-pushState.html`
- `event-order/after-load-replaceState.html`
- `event-order/before-load-hash-twice.html`
- `event-order/before-load-hash.html`
- `event-order/before-load-pushState.html`
- `event-order/before-load-replaceState.html`
- `event-order/pushState-inside-popstate.html`
- `event-order/same-document-traverse-immediate.html`
- `event-order/same-document-traverse-wait.html`

### Joint parent/frame traversal and source restoration

- `history-traversal-navigate-parent-while-child-loading.html`
- `history-traversal-navigates-multiple-frames.html`
- `srcdoc/consecutive-srcdoc.html`
- `srcdoc/srcdoc-history-entries.html`

### Pagereveal lifecycle ordering

- `pagereveal/order-in-bfcache-restore-iframe.html`
- `pagereveal/order-in-bfcache-restore.html`
- `pagereveal/order-in-new-document-navigation-iframe.html`
- `pagereveal/order-in-new-document-navigation.html`
- `pagereveal/order-in-prerender-activation.https.html`

### Helper sources (caller semantics)

- `pagereveal/resources/iframe.html`
- `pagereveal/resources/order-in-prerender-activation-popup.html`
- `persisted-user-state-restoration/resources/blank1.html`
- `persisted-user-state-restoration/resources/blank2.html`
- `persisted-user-state-restoration/resources/page-with-fragment.html`
- `persisted-user-state-restoration/resources/post_name_on_load.html`
- `resources/a.html`
- `resources/api-availability-1.html`
- `resources/api-availability-2.html`
- `resources/b.html`
- `resources/c.html`
- `resources/unset_context_name-1.sub.html`
- `resources/unset_context_name_stash.py`
- `support/window-name-after-cross-origin-main-frame-navigation-popup.sub.html`
- `support/window-name-after-same-origin-main-frame-navigation-1.sub.html`
- `support/window-name-navigation.sub.html`
- `support/window-name-test.sub.html`

### Pageswap disclosure and navigation types

- `pageswap/pageswap-cross-origin.sub.html`
- `pageswap/pageswap-iframe.html`
- `pageswap/pageswap-initial-navigation.html`
- `pageswap/pageswap-push-from-click.html`
- `pageswap/pageswap-push-navigation-hidden-document.html`
- `pageswap/pageswap-push-navigation.html`
- `pageswap/pageswap-push-with-cross-origin-redirect.sub.html`
- `pageswap/pageswap-push-with-redirect.html`
- `pageswap/pageswap-reload-navigation.html`
- `pageswap/pageswap-replace-navigation.html`
- `pageswap/pageswap-replace-with-cross-origin-redirect.sub.html`
- `pageswap/pageswap-traverse-navigation-cross-origin-redirect-no-bfcache.https.sub.html`
- `pageswap/pageswap-traverse-navigation-no-bfcache.https.html`

### Persisted scroll and timer restoration

- `persisted-user-state-restoration/resume-timer-on-history-back.html`
- `persisted-user-state-restoration/scroll-restoration-basic.html`
- `persisted-user-state-restoration/scroll-restoration-fragment-scrolling-cross-origin.html`
- `persisted-user-state-restoration/scroll-restoration-fragment-scrolling-samedoc.html`
- `persisted-user-state-restoration/scroll-restoration-navigation-cross-origin.html`
- `persisted-user-state-restoration/scroll-restoration-navigation-samedoc.html`
- `scroll-restoration-order.html`

## Generated integration and exact residuals

- `joint_history.rs`: checked revision/step allocation, aggregate 64 MiB snapshot budget, 4096 steps/1024 routes; opaque native execution receipts remain pending until every selected route commits. Weak process registry bridges native serialized timeline restoration without owning a page. Generated native tests cover parent+child selection, initial replacement, forward truncation, stale plans, allocation/revision failures, serialized multi-document completion and late cancellation.
- `navigation_api.rs`: common native delta algorithm for JS History tasks and browser UI; original target-document/operation checks, retained publication retry, shared History.length, sibling forward-entry disposal, frozen destination commits. Retiring unfinished route jobs terminates their receipt; accepted native destination loads continue under their loader ownership.
- Host/executable/runtime: native timeline transport across root reload/traversal and BFCache; saved frame route selection before child scripts; original document-state ids retained for refetched history entries independently of actual execution generation. Root actor owns UI/control reply receipt polling and deadlines.
- `frame_form_history.rs`: generated parent/child length+go regression, native UI equivalent, and sibling forward disposal test. The earlier standalone frame restoration fixture now expects retained logical document-state identity.
- The following integration was completed during this source pass: active-parent navigable pruning, original-revision cancellation rollback, native ancestor restoration via shared child archives, and per-document network control custody. Remaining review must cover every nested/cross-origin prefix/suffix and retained failure path against actual fixtures; it remains a source gate, not an execution-only caveat.

All listed regression sources are unbuilt and unrun. No conformance pass is claimed.

### Further source reconciliation

Cross-document receipts now require exact destination load completion after commitment; same-document steps complete at commitment. Native stop/failure/no-content settle only their owned receipt, preserving the intentionally unresolved Navigation API promises for 204/205/download responses. Generated readiness and lease-transfer regression sources cover these boundaries.

Active-parent removal prunes the native navigable subtree; parent retirement/BFCache keeps archived descendants. Uncommitted canceled plans restore the cursor only while its original revision still owns it, preserving author changes and partial commits. Root entries sharing a logical document-state now share the complete child session archive while retaining separate root controls. This supplies historical child selection during uncached traversal to another same-document root entry.

Residual generation review remains: full nested/cross-origin ancestor repopulation fixtures, per-document controls for archived network child entries (inline source archives already retain controls), and complete fixture-level name/pageswap/timer reconciliation. The broad joint-history row remains unchecked.

### Archived network controls

Generated `ArchivedControls` supplies shared, budgeted native custody independently of response-body replay. Frame replacement now captures controls after pagehide and its owner checks; selected network document-state restoration restores those controls through the existing retained parser form-restoration phase. Serialization validates known document ids, ambiguity with inline source custody, total controls and bytes; frame removal/forward truncation prunes records. Root same-document entries share the full child session archive. Native custody and wire rejection/default regression sources were added. Nested/cross-origin end-to-end fixture reconciliation remains open; no execution result is claimed.

### Initial-document and root transfer caller reconciliation

Dynamic frame native session commit now uses the old realm's explicit initial-document flag to replace its initial entry, independently of URL text. Root replacement captures joint metadata before parking/removing the incumbent executable, holding pending native completion leases until the new isolate imports them. Generated root replacement regression asserts the previous child step remains the immediate back destination. The shared actor route restores BFCache before falling back to a resource fetch; the synchronous test compatibility path uses the same cache decision.

### Hidden cross-origin actor fixture sources

Generated actual HistoryTraversalTask → frozen LocationNavigation → actor tests for cross-origin cached restoration and uncached 204/205 responses. Generated an in-process cross-origin history attachment test after bounded cache eviction, asserting exact quarantined bytes, retained incumbent Document and no navigateerror. These sources exercise the full route rather than only token helpers; they remain unbuilt/unrun.

### Intercepted traversal tasks and interactive readiness (source generated)

Pinned Necessary fixtures `navigation-api/navigate-event/intercept-popstate.html`
and `intercept-popstate-no-handler.html` exposed a missing intercepted traversal
`popstate` task. The native commit now queues a frozen, stream-owned event on the
history event task queue. Handlerless finish remains a microtask preceding this
task; an asynchronous handler can remain pending while the event runs. Joint
completion requires both the event suffix and navigation completion, in either
order. Retry does not reconstruct or replay a consumed listener. Regression
sources cover handlerless ordering, interrupted listener suffix and a pending
handler after event delivery. These are Necessary source cases, not additional
Core manifest entries.

Joint receipts now expose `poll_interactive()` independently of final `poll()`.
Every selected cross-document route must have committed and delivered its actual
DOMContentLoaded suffix; same-document routes must finish their history
publication. A final load implies interactive, and terminal failure remains
visible through both APIs. Per-route interactive state survives native timeline
serialization/rebinding, while the original receipt retains the milestone and
final-load custody. Multi-route/rebind, wrong-destination, same-document and
post-interactive cancellation regression sources are generated. Root actor
integration owns exact controller milestone acknowledgments and early eager
reply while retaining final load work.

The actor-level hidden cross-origin history task now has a network-failure
regression alongside the cached, 204/205 and download cases. It follows the
HistoryTraversalTask -> LocationNavigation -> asynchronous resource transaction
path and checks incumbent identity, undisclosed entries and exact joint cursor
rollback. All regressions remain unexecuted. Nested ancestor repopulation and
full fixture-by-fixture source closure remain open under R-04.joint-steps.

### Consecutive srcdoc fixture reconciliation

Direct Core `srcdoc/consecutive-srcdoc.html` requires same-URL source changes to
replace, while changing from `about:srcdoc#fragment` to `about:srcdoc` pushes.
Dynamic frame pageswap classification and session commit now share the exact
same-URL decision. Restored srcdoc URLs retain the saved fragment instead of
being flattened to `about:srcdoc`. The generated native regression performs
consecutive source replacement, fragment navigation, another source navigation,
and joint back to the earlier source/content and fragment. Execution remains
pending; this closes those concrete source gaps, not the full nested matrix.

### Frame navigable-name history and timer source review

Frame SessionHistory now owns budgeted saved names keyed by native document-state
identity. Departure captures the latest name for strictly same-origin historical
documents. Fresh cross-origin frame navigation keeps the live name; cross-origin
history return restores the saved destination name. Same-origin history return
keeps the latest live name. Ancestor frame repopulation installs its exact saved
name before child author execution, using existing claimed frame route checks.
Names survive saved frame descriptors, with unknown legacy data distinct from a
known empty name. Source regressions cover both native frame traversal cases,
strict opaque-origin separation, reservation failure preserving prior state,
and duplicate/orphan/legacy serialized descriptors. Root top-level name clearing
and restoration remain independently owned; contradictory/incomplete old main
frame tests do not justify clearing all subframe names.

`resume-timer-on-history-back.html` requires suspended timers not to run while
root or ancestor is parked, then to resume after back. Host source suspends every
saved descendant environment and resumes those exact documents. Isolated-host
fallback excludes parked time from the timer clock. Added a root+nested-frame
cache/back active-time regression; no new timer algorithm was required by this
source review. No regression has been executed.

### Retained pagereveal before animation callbacks

Pinned pagereveal new-document and BFCache iframe fixtures require reveal before
requestAnimationFrame, with pageshow.persisted preceding reveal on restoration.
Source review found the rendering path marked reveal complete before fallible
one-shot dispatch. It now retains its native event cursor across preparation,
callback and termination failures, and admits rAF only after the suffix finishes.
Exact document generation, stream, authority and reset identity prevent an old
reveal from completing for a replacement/restored document. Native reset retires
the old suffix; a new reveal has independent ownership. Rendering readiness also
covers pending reveal without registered rAF callbacks. Regression sources cover
interrupted listener ordering and reset cancellation. No execution results are
claimed; broader prerender activation and complete joint fixture gates remain
separate.

### Departing frame lifecycle response custody

The new NativePageSwap receipt freezes native Activation/event payload and exact
Document generation/stream/authority, retaining callback progress for pageswap
and PageTransitionEvent. Host/executable begin/poll APIs expose cancellation
separately from completion. Root commit/cached restore callers are owned by the
root integration lane.

Primary-platform dynamic frame departure now happens before consuming its
accepted response body. A bounded128 native queue retains response leases,
navigation context and original parent/frame/child identity, prepares swap and
hide receipts before callbacks, and resumes them without re-fetching the source.
Replacement/removal/stop/drop cancels the exact old task. A completed departure
feeds that same response into normal parsing/commit; scheduler and load-barrier
readiness include the queue. Source regressions cover listener interruption in
the shared receipt and pause-after-swap/removal-before-hide in the native frame
caller. Non-Unix compatibility is preserved. The aggregate ready-frame batch now also lives in a bounded native queue: a
preparation error consumes only its active job and retains unrelated response
bodies/departure markers for the next native poll. Supersession, frame retirement
and stop prune queued owners; scheduler and load readiness include the suffix.
A fault-injection regression covers a first-job failure followed by the second
frame committing without resubmission. Active-job parser failure classification
and root transaction integration remain separate reviews. No execution ran.

### Root departure and cached publication ownership (source generated, unexecuted)

`root_departure.rs` and `navigation_actor.rs` now retain a root departure job before consuming an HTTP response body, parent local-navigation grant, standalone data/blank body, or cached destination. `pageswap` owns its original event cursor; `pagehide` uses the native descendant-tree receipt. Staged history entry IDs are passed into construction unchanged. Original root/document-stream and navigation-operation checks run between callback phases. Form controls, descendant form archives, target name and semantic projection are captured before the actor releases its incoming-body ownership. Completed callback prefixes survive preparation/publication errors; actor deadlines and source replacement terminate the job.

Actor cached selection now happens inside the admitted transaction, following the existing beforeunload gate. Its destination publication phase preserves the installed destination identity through retained pageshow/history callbacks and semantic/layout publication; commit alone cannot finish the navigation reply. Cached joint-history rebinding happens before cache removal, and committed-event emission follows installation of the executable. Failed-commit rollback schedules its original-document pageshow at the front of the retained history queue, including preparation retries and original persisted value.

Generated regression sources cover buffered-body retention, exact staged-history ID reuse, cached target retention, source `document.open()` cancellation and rollback publication preparation recovery. Existing frame departure/ready-job regressions cover the parallel frame body/callback custody. These are source assertions, not executed results. The broader joint-history gate stays open for combined source review of post-suspension parent authority failures and complete pinned nested/cross-origin fixtures. Synchronous compatibility construction/suspension calls use bounded retained-cursor polling; production actor HTTP/local/data/blank/cache paths use scheduler phases.

Follow-up combined source review changed frame departure pagehide to the same retained descendant-tree engine used by root departure. A nested frame source regression asserts parent/child pagehide order before replacement. Root post-pagehide snapshot/semantic failures now retain both the completed event prefix and the still-owned actor body; cached destination publication retains Interactive/Complete milestone admission separately from final acknowledgement. Rollback preparation-failure and post-hide capture-failure regression sources are present in `root_departure.rs`.

### Post-suspension completion and rollback source reconciliation

The parent now records exact restore/discard requests before consuming a cache capability, saves the native commit response before worker/service cleanup, and retains each owner/stage suffix across failures. Successful identical retries return the saved response without repeating binding or completed cleanup; foreign/conflicting requests fail. Root native restoration separately checks the original creator against current parent authority before releasing suspended queues. Initial-document outcomes have a bounded, page-owned completion record independent of ordinary ACK-tombstone pruning.

Initial construction keeps its cleanup token and complete previous runtime policy/header snapshot after transport or wrong-response failure. Retry runs without an executable, preserves the original construction result, and cannot overwrite a newer installed generation. An obsolete-restore disposal ACK explicitly prevents stale snapshot installation. Suspension failures after native parking retain the original executable behind a scheduler-owned resume gate, followed by one rollback pageshow publication.

Unreceived park-response custody now rolls the original source back when its exact policy and document slot remain current; if a newer document already occupies that slot or the policy epoch changed, it discards only the unreceived old capability. The restore precondition is rechecked under the native security/hub commit locks. Source regressions cover consumed restore followed by suffix failure, identical ACK after a newer commit, conflicting/foreign cleanup, lost/wrong initial ACK, installed-source suspension failure, and unreceived park-response rollback. No tests have been executed.

The provisional-executable error path now retains the failed executable until its host can be detached, restores the original cached source through the scheduler, and retires the failed root only after source restoration. Failed parent/authority rollback retains both exact cached owners and later evicts only the originally failed generation. The existing shared-host recovery regression now exercises that autonomous continuation. `recover_failed_document_commit` preserves the original typed failure rather than replacing it with a later recovery error. Recovery pageshow owns one extra bounded queue slot, so a full normal history queue cannot deadlock native source restoration.

The retained restore suffix also handles supersession between the parent security commit and worker resumption: exact owner liveness and resume are checked together under native agent/message custody. A replaced owner follows retained disposal stages instead of being resumed; its original restore response remains a historical completion only. Regressions cover this order and unreceived park cleanup after a newer security commit.

### Parent parking recovery and cleanup reentrancy

Source-generated: cached completion processing now takes an exact-token busy lease outside the completion mutex. Worker teardown may recursively reclaim a response; nested same-token completion remains pending, conflicting requests reject, and retirement prevents lease reinsertion. A regression source holds the busy lease across a duplicate initial restore.

Failed parking before token issuance now retains each frozen native owner independently. Freeze, quiescence and native park failures preserve their original error while the parent response-cleanup scheduler retries resume. Every retry verifies the exact registered document identity; superseded owners follow staged resource disposal instead of resuming. The cleanup mutex is released before worker callbacks, and page retirement removes pending owners. Regression sources cover failed park with no token, deferred resume and a newer registered document. These are generated sources, not executed results; broad fixture reconciliation remains open.

### Bounded completion expiry

Generated: pruning ordinary completion ACKs retains no unbounded token tombstones. Tokens carry an authenticated native page/issuance identity; absent active custody with a valid token produces typed `CachedCompletionExpired`, while forged, foreign-page and foreign-hub tokens fail. The host treats expiry as terminal disposal, never successful restoration: it retires only the original saved root identity and clears frozen native queues. Rollback recovery evicts the expired cached entry or retires the abandoned provisional executable, so the cleanup scheduler cannot retry an expired ACK forever. Original initial-document outcomes remain separately bounded and page-owned.

Generated regression sources cover forged/page-tampered/cross-hub tokens, counter exhaustion, 129 completions and bounded record pruning, repeated typed expiry, foreign-page rejection, and expired host restore cleanup with incumbent recovery. No tests or builds were run.

### BFCache reveal and rendering admission

Pinned `pagereveal/order-in-bfcache-restore.html` and its cross-origin iframe counterpart require `pageshow.persisted,pagereveal,rAF`, with no duplicate reveal. Source review found parent-backed restoration resumed native environments without resetting reveal, while pending pageshow suffixes allowed rendering polls. Generated per-Document restoration rendering admission now resets only the restored environment list, blocks reveal/animation-frame readiness until the retained pageshow tree completes, and releases only matching original document/stream/authority identities. Unrelated active roots are not reset. Standalone restoration uses the same gate; replacement streams cannot inherit it. Native source regressions cover delayed child pageshow, parent callback no-replay, gated rendering and replacement-stream cleanup. No execution qualification is claimed.

### Reload pageswap entry identity and preparation retry

Pinned `pageswap/pageswap-reload-navigation.html` requires both `activation.entry` and `activation.from` to equal the departing `navigation.currentEntry`. Source inspection found reload used the newly staged replacement ID. Generated reload handling now keeps the current wrapper identity; push/replace/traverse continue using their selected destination. Swap activation preparation stages native registrations transactionally and retains the same prepared destination/activation across later event-wrapper failures. The staged destination is consumed only after a complete retained PageSwap receipt exists. Native regression sources assert reload wrapper identity, entry-staging failure with no leaked registration, stable activation on retry and final receipt consumption. No execution.
