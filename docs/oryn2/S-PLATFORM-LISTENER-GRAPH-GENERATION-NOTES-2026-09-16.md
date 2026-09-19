# Platform listener graph generation

Source generation only. No builds, tests, formatters, generators or conformance runs were executed.

## Problem and implementation

The native platform listener/handler maps previously owned strong V8 callbacks. A callback capturing its otherwise-unreachable target could keep that target alive through native state indefinitely.

`crates/oryn-js/src/v8_bindings/platform_listener_graph.rs` now stores weak native callback handles while each actual JavaScript target privately traces its registered callbacks. Window handlers trace from the native global. Listener removal, once invocation, abort, handler replacement and document-stream listener clearing refresh those edges. Once callbacks are promoted to local handles before removing their target edge. Native identity watcher finalizers only enqueue serial-tagged releases; cleanup happens at safe checkpoints and cannot delete a newer wrapper for the same native platform object.

An explicit native activity-root map retains wrappers while pending/queued platform tasks can deliver observable callbacks. It accounts for XHR/upload, IDB requests/transactions, worker connection capabilities, sockets, remote started ports and broadcast subscriptions. It releases roots after native task completion, termination, close or retirement. Local port wrappers trace entangled peer wrappers without creating an unconditional native root for a dormant local pair. Remote peer closure schedules close after already-received message tasks and stops repeated terminal polling.

Native EventTarget dispatch uses the shared native phase/path implementation. Dispatch no longer coerces the Event to a string, and removal of another listener during dispatch suppresses the removed registration.

## Integration and regression source

Narrow binding/global-event registration, getter/setter, dispatch, weak-release, document.open, platform-operation return, completion-drain and external-poll call sites are integrated. Service-worker handler accessors use the new helper; the service owner is independently converting service entity permanent Globals to weak identities plus required traced relationships.

Generated regressions cover unreachable self-capturing listener cycles, live callback survival, pending native event roots and release after delivery, once/removal/abort ordering, reentrant removal and rejection of Event.toString side effects. Native GC test source uses nested handle scopes around rooting/delivery so temporary test handles do not accidentally retain the target.

## Further owned source audit

AbortSignal.reason and IDB request.result now distinguish primitive persistent values from weak object identities; native target private edges retain object payloads only while their owning JavaScript graph is reachable. Native controller/request/channel/IDB relationships and AbortSignal dependents are traced. Regression source covers live payload identity and otherwise-unreachable self-referential reason/result cycles. XHR cached responses now use the same weak-value/private-edge ownership, with native XHR-to-upload identity edges and immediate cache-edge clearing on open/abort. A regression source covers cached identity while live and collection of a response-to-XHR cycle. Separate stream source/sink callbacks still need a full ownership audit. Confirm every async source/retirement entry point against its actual capability and pending task lifetime, and add graph-boundary regression sources as needed. Source generation does not establish resource/conformance qualification.

## Native timeout source integration

AbortSignal.timeout now uses a typed native scheduler installed in Window and worker binding states. It never reads public setTimeout or an author timer bridge. The native callback traces the signal wrapper, uses the live exact realm/document environment, has unsigned-long-long EnforceRange conversion, and avoids HTML nested-timer clamps. Existing native timer active-time suspension and environment retirement apply to the scheduled callback. Generated host regression source covers author timer replacement, pending signal/listener reachability, asynchronous delivery, range/abrupt conversion, long delays, suspension and retirement. IDL source is updated; generated bindings and all execution remain deferred to the global generation gate.

IDBRecord key/primaryKey/value and IDBCursor cached key/primaryKey/value use the same privately traced payload ownership, including cached old cursor values retained across an asynchronous iteration request. Regression source checks live cache identity and collection after a value points back to its otherwise-unreachable owning record/cursor.
