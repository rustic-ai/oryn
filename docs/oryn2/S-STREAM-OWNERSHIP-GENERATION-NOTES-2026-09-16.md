# Native stream ownership generation

Source generation only. No build, test, formatter, generator or conformance execution.

## Implemented source

`crates/oryn-js/src/v8_bindings/stream_graph.rs` defines bootstrap-owned native handles. Strong bootstrap handles protect callback getters and newly allocated chunks/promises before the owning graph exists. Private native anchors then trace object-valued callbacks, sources/sinks, errors, queue entries and promise resolvers; native handles become weak only after every private edge has been installed. Primitive values retain ordinary persistent handles.

The graph includes readable/writable streams, controllers, readers/writers, both tee branches and source, transformer/readable/writable/controller relationships, encoding streams, Request/Response body ownership, body consumers, pipe destinations, pending aborts and queued write/read promises. Promise objects privately trace their owner anchors. Native reaction callback data retains actual native controllers. Public wrapper identity is preserved within a live anchor component; unreachable cycles remain ordinary JavaScript cycles.

Pending native body reads root the exact stream anchor until native completion or retirement. Pure JavaScript pending promises do not create unconditional native roots. Constructor, property getter, operation, native reaction, tee microtask and platform completion checkpoints update the graph. Replaced private edge arrays remain local until the current native scope ends so dequeued temporary values survive reentrant graph updates.

Weak finalizers enqueue identities only. Safe checkpoints reclaim dead native stream metadata and queue native HTTP stream aborts. Admission failures retain cleanup work for retry. Record removal is deferred when another native operation is borrowing the registry. Retirement releases pending activity/bootstrap roots; readable/writable reactions and source pumping avoid invoking retired callbacks.

## Regression sources

- Self-capturing readable/writable callbacks, queued self-references, reader promises and transform components can be collected; their native metadata is reclaimed.
- An externally held read promise retains the native stream graph after the reader/stream/controller references are dropped, preserving callback and result identity.
- Tee cancellation retains the source until both branches cancel and preserves cancellation reasons.
- Native body-read activity retains the stream until retirement.
- Bootstrap handles remain strong until the private graph has been published.

Existing stream behavioral regressions remain in the readable/writable/writer/tee/transform modules. Their native handle accesses were migrated; none were removed or run.

## Remaining validation and wider scope

All cross-module type/lifetime integration, garbage collection timing, queued native cancellation delivery and WPT behavior still require the deferred build/test/conformance phase. Byte/BYOB and sequential pipe-option source integration is now tracked in S-BYTE-STREAM-PIPE-GENERATION-NOTES-2026-09-16.md. The combined source does not establish complete Streams API coverage; native byte tee forwarding and cross-realm construction still require normative source audit. No source-only result is qualification evidence.
