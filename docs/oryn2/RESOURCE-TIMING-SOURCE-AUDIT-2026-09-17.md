# Resource timing source integration — 2026-09-17

Status: generated source and regression sources; no build, tests, generator, service, or conformance execution.

## Native transport

The exact locked reqwest 0.12.28 source is vendored under `vendor/reqwest` with a narrow direct rustls connector extension. TCP completion and TLS start/end are captured around actual connector futures and attached to the pooled connection using Hyper connection extensions. `oryn-net` combines those with native DNS admission, records HTTP protocol, and excludes old connection phases on reuse. DNS seeds are process-private and skipped by serialization; native policy replaces any local seed. The Cargo patch preserves the locked version; final lock reconciliation is deferred with the complete generation batch.

`TransportTiming` lives in the V8-free common contract. Network response, streaming response, header events and encoded broker IPC preserve it; transparent adapters and 304 revalidation retain current-request phases. Full-body collectors record terminal timing rather than treating headers or Content-Length as completion. Snapshot validation and regression sources cover phase ordering, wire roundtrip and reused-connection exclusion.

## Native document publication

Parent external script/module/style/font/frame requests produce detailed metadata before decoded source transformation. Streaming frame records keep custody to the terminal read. Standalone script/module/CSS import/font/frame producers attach the same metadata through their existing document-owned queue. Queue publication checks exact initiator Creator/page or standalone Document/authority epoch. Prepared-frame IO scope movement does not move the embedding document's timing entry. Host publication runs before the first authored instruction as well as checkpoints; paused/parked execution does not invoke authored callbacks.

The Performance record exposes measured permitted phases, response status, content type, raw decoded bytes and encoded length when available. An absent phase remains absent/zero; no DNS/TLS estimate is synthesized for a cached or reused connection.

## Service response and cache provenance

Private native Response records retain timing permission provenance through clone, CacheStorage and respondWith. Every redirect hop retains its URL and TAO fields. Reuse reevaluates the whole chain for the new native initiator origin; an earlier origin's permission is not reused as authority. The bridge strips old transport timestamps when a prior response body is replayed. Private metadata remains outside authored Response headers and getters, including opaque responses. Cache snapshot validation bounds timing metadata and rejects malformed records.

Generated regressions include exact/mismatched TAO origins, cross-origin redirect reuse, old-phase suppression, opaque timing filtering, CacheStorage metadata retention, native first-instruction visibility, sibling isolation, real HTTP connector phases and broker IPC roundtrip. All remain unrun; runtime qualification and the complete Core/five-family gates remain open.
