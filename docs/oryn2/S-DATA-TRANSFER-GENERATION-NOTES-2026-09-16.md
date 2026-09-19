# S data-transfer source generation — 2026-09-16

Source generated only. No build, tests, formatter, WebIDL generator, deployment or conformance run has been executed for this work. It does not certify S completion.

## Generated behavior

- `oryn-js/src/data_transfer.rs`: V8-free drag/clipboard data store with read/write, read-only, protected and detached modes; ordered strings and native file bytes; format aliases, ASCII type folding, lossless UTF-16, item identity and mutation revisions.
- `oryn-js/src/v8_bindings/data_transfer.rs`: native DataTransfer, live item/file collections, frozen types identity, deferred string callbacks, DragEvent/ClipboardEvent associations and trusted native dispatch. Each native event gets its own store association which detaches after dispatch; an earlier saved event cannot regain access during a subsequent drop.
- `v8_bindings.rs`, `file_lists.rs`, `global_events.rs`: bounded constructor, event payload, deferred task, handler attribute and live FileList integration.
- `oryn-webidl/idl/browser.webidl`: interface/dictionary/callback source plus drag/clipboard handler attributes. Checked-in generated bindings intentionally await the global generation gate.
- Native store and V8 regression sources cover modes, file preservation, actual bytes assigned to input.files, ordering, duplicate types, removed identities, callback scheduling, trusted/synthetic events and retired realms.

Pinned WPT source reviewed: `html/editing/dnd/datastore/datatransfer-types.html` and indexed/removal tests in checkout `719d5e38fdd0903a18ed9007aba816c98cc491e0`. Types caching follows item-list changes, including replacement; no-op clear/remove retains the original frozen array.

## Integration and remaining work

Parent owns native drag hit testing, transitions, effect negotiation, default editing/clipboard behavior and dispatch integration. Public `dispatch_native_transfer_event` accepts the native shared store and physical event fields and returns cancellation. The dispatch API itself does not implement a drag action.

`setDragImage` currently records the validated native Element identity and offsets. Immediate image snapshot/raster integration remains required; recording a node is not a completed bitmap implementation. OS clipboard authority and default copy/cut/paste actions remain host responsibilities. Cross-realm and broader pinned DnD/manual coverage remain unverified. FACE/ElementInternals implementation is the next assigned S source batch.
