# Fetch request body dispatch — source closure

Confirmed L gap: `platform_fetch_callback` built a native HTTP request from cached
`RequestRecord.body`, even when the request owned a live `body_object_id`. Streaming
bodies therefore dispatched empty or obsolete bytes.

Generated production path:

1. Native Request construction retains the source BodyOwner (P/W ownership).
2. Fetch freezes request metadata and creates cancellation custody before any
   body pull callback. The pending registry retains the Request and body wrappers,
   original native requester generation/authority, and source owner identity.
3. `response_body_operation_bounded(...,"arrayBuffer",16 MiB)` consumes actual
   native stream chunks. The existing shared consumer owns disturbance/locking,
   non-Uint8Array rejection and source errors. Pending drains are limited to the
   existing platform active-operation budget.
4. Native promise reactions transfer the actual bytes into PlatformHttpRequest
   only after successful close, recheck authority/abort/source lifetime, and call
   the existing HttpStart admission. No HTTP request is fabricated on failure.
5. The root-owned abort graph calls `fetch_body::abort` before the public event.
   Fetch rejects with the exact original reason and cancels the owned stream;
   cancellation failures cannot replace that reason, and retained native cleanup
   receives fallback abandonment when needed. Already errored/closed streams do
   not spuriously invoke underlying cancel a second time.
6. Realm retirement releases pending wrappers and schedules native cancellation
   in surviving source/requester realms. A bounded weak dependency list notifies
   requesters when a foreign source retires, rather than leaving fetch pending.

Sources: `v8_bindings/fetch_body.rs`, native fetch callback, binding registry and
retirement/deferred-task callers. No wire change or independent emergency thread.
Cached-body dispatch was removed. P/W retains ownership of constructors, BodyOwner,
consumer, clone and service/cache integration.

Generated regression sources cover delayed binary chunks and no early/duplicate
HTTP, exact abort reason and underlying cancellation, source errors, native body
limit rejection, and foreign-body source retirement. They have not been run.
All build/test/conformance execution remains behind the whole-generation barrier.
