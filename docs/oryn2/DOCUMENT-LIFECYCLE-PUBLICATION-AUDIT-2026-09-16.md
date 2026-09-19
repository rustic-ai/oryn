# Native document lifecycle publication audit — 2026-09-16

Status: generated and source-reviewed; independent review, builds, tests and conformance execution remain pending. No execution was performed in this generation lane.

## Generated implementation

- `document_lifecycle_publication.rs` owns per-document generation/stream, progress stage, receipt, retry deadline and reentrancy guard. It accepts native ownership before any DOMContentLoaded callbacks.
- Stages retain DOMContentLoaded admission/acknowledgment, existing image/script/style/frame resource barrier, complete/readystatechange/load/pageshow acknowledgment, and the parent iframe-load acknowledgment.
- Native errors park the same operation for 25 ms. Readiness and timer-delay integration wake it without unrelated tasks. Resource waits remain event-driven. Completed identities prevent a caller retry from dispatching DOMContentLoaded/load again.
- Native frame generation/connection/environment checks precede iframe-load admission. Child retirement cancels an unstarted parent event, while an already-started parent event retains its callback tail under the parent's own lifecycle identity.
- JavaScript publication constructs readiness/event objects and admits a bounded receipt before changing native readiness. Events retain their identity, listener cursor, once-listener custody, event phase and target through failures.
- Window load preserves its legacy Document target, Window currentTarget and Window path. Initial pageshow is marked only when its own live event starts; a load callback opening another document stream cannot prematurely consume pageshow.
- Successful current-stream DOMContentLoaded completion enables the ServiceWorkerContainer message queue in both retained and legacy paths; Interactive readiness alone does not enable it.
- `V8BindingState::document_completely_loaded()` remains false through load/pageshow callbacks and failed partial dispatch, then becomes true only after the whole current lifecycle batch completes. Stream advance/open and document replacement reset it. This is the native source for form submission history handling; readyState Complete alone is insufficient.
- `document.open()`/stop retire parser receipts; native generation/stream checks prevent the old completion from inserting another load barrier. Native receipt cleanup avoids stale parent receipt accumulation.
- The existing tuple load barrier remains compatible with direct native callers and regression fixtures. The owned publication map is bounded at 1,024 document identities; retired completed identities are pruned before new admission, and replacement of an existing realm does not require an extra slot.
- Frame EOF keeps its active preparation published during fallible lifecycle setup and checkpoints. Reentrant polling skips the active preparation. Errors restore retry eligibility and preserve saved forms/refresh data; successful settled owners release their barrier before final load flushing.
- Refresh timer admission uses a private compiled callback and native scheduler; author replacement of `setTimeout` cannot drop or duplicate accepted refresh scheduling.

## Regression sources

Native `document_lifecycle_publication::tests`:

- `native_dom_receipt_retry_preserves_order_and_does_not_duplicate`
- `native_load_receipt_retry_does_not_repeat_load`
- `parent_frame_receipt_retry_preserves_single_callback`
- `document_open_retires_old_native_lifecycle_receipt`

Native XML recovery adds `xml_lifecycle_handoff_failure_retains_owner_and_recovers_once`.

JavaScript history publication tests add:

- `lifecycle_receipts_resume_dom_load_and_pageshow_with_legacy_targets`
- `load_document_open_does_not_consume_initial_pageshow`
- `retired_child_preserves_started_parent_frame_event_but_cancels_unstarted_event`

## Scope remaining open

- The HTML parser resume/`accept_parsed` input-to-completion ownership boundary and initial HTML parser-completion frame list still require their own transactional custody audit; the generated frame EOF/native event queue does not make that earlier phase complete.
- Detached buffered subtree import still needs the separate partial native DOM allocation/attachment rollback audit.
- Broad R lifecycle, XML, Core/OIL and WPT gates remain open until independent source review and the final full execution phase.
