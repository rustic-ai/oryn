# XML parser publication source audit — 2026-09-16

Status: generated and source-reviewed; build, tests and conformance execution remain deferred until the complete generation phase finishes.

## Generated custody

- XML `Pending` restores the parser owner before accepting emitted events. A retained event queue owns the unprocessed suffix, including the current event after a native preparation error.
- Script checkpoints precede preparation; source-byte reservations, DOM leases and job queue capacity precede `already started`. Failed admission leaves the exact event and unstarted script available for retry.
- Snapshot event transfer removes the original only after inline-style release succeeds and transfers it into the retained queue before fallible script preparation.
- Live and detached XML EOF have an owned completion record. Detached error-document source remains borrowed during import, deferred scripts transfer once, and readiness publication is acknowledged through a native receipt before EOF ownership is released.
- `readystatechange` resumes the same event and callback cursor after preparation/mid-callback failure. Parser receipts carry the document stream epoch; `document.open()` and native stop retire queued parser work without reviving completed receipts.
- Parser retry deadlines feed frame runnable checks and executable timer delay. Future retries remain parked for 25 ms rather than busy polling.
- Frame setup, stylesheet blocking checks and async realm activation requeue the popped preparation before returning native errors. Retry admission is checked before viewport/restore work.
- Worker close task gates suppress new native platform notification tasks while permitting an already-started event to finish.

## Regression source inventory

`frame_script_preparation::xml_publication_recovery_tests` covers setup failure ownership/deadline, script admission failure with unstarted script and preserved suffix, EOF readiness preparation failure with one interactive notification, lifecycle handoff failure with retained owner and one DOMContentLoaded/load sequence, and stop retiring an old script owner.

`navigation_api::history_publication_tests::parser_readiness_receipt_retains_event_tail_and_document_open_retires_old_receipts` covers native readiness callback continuation, receipt cleanup, and `document.open()` suppressing the old callback tail.

Existing XML source coverage also exercises parser-blocking script order, XML `document.write` rejection, malformed-tail deferred cancellation, and selected frame document identity.

## Remaining confirmed lifecycle work

These are separate open implementation boundaries, not covered by the generated XML custody claim:

1. The previously identified DOMContentLoaded/load/pageshow/frame-load and frame EOF handoff gap now has generated integration and regression sources; see `DOCUMENT-LIFECYCLE-PUBLICATION-AUDIT-2026-09-16.md`. It remains review/execution pending.
2. HTML `LiveParserState::resume` consumes the parser owner and may fail before returning a replacement; `accept_parsed` consumes completion before all resource/job admissions. This source audit does not claim HTML completion recovery.
3. Buffered detached subtree import preserves the original XML source through failure but the existing native import/attachment algorithm still needs a broader rollback audit for partial DOM allocation/attachment failures.
4. Full R-06.k/R-06.b/R-07, Core/OIL and WPT gates remain open pending these integrations, independent source review and final execution.
