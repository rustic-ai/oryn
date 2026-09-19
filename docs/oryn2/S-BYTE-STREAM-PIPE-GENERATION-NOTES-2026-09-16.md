# Native byte streams and pipe source generation

Generated, not built or tested. The global all-source-before-execution gate remains active.

## Implementation

- Native byte controllers and BYOB readers/requests use transferred ArrayBuffer storage, preserve typed-view offsets and element sizes, validate minimum reads, retain released-reader outstanding requests, and distinguish close requests from closed-state descriptor responses.
- Native finite bodies and Blob streams close after queued bytes drain. Byte tee branches use separate transferred storage. Native fetch reads remain parent-service operations.
- Sequential native pipe state waits for destination readiness and writes; supports preventClose/preventAbort/preventCancel, original abort reasons, locked-reader/writer cleanup and option conversion errors.
- Cross-realm destination retirement queues a source-realm cleanup task, invalidates obsolete shutdown callbacks and does not invoke callbacks in the retired destination.
- Stream graph ownership traces callback/data/promise/tee/pipe relationships from native wrappers, with roots for actual native pending operations.

Regression sources cover typed transfer, minimum/alignment, close/respond, autoallocation/release, tee independent storage, backpressure, pipe option errors, shutdown options and cross-realm retirement. Reviewed normative Streams algorithms and pinned WPT source at 719d5e38fdd0903a18ed9007aba816c98cc491e0; no WPT run occurred.

## Remaining obligations

All integration/build/generated-WebIDL and conformance checks remain deferred. Byte tee descriptor forwarding and borrowed/cross-realm reader construction still require further normative source review; no full Streams conformance claim is made. Resource ceilings reject oversized native autoallocation explicitly. OS accessibility consumer generation follows this stream batch and remains a separate required V-05 obligation.
