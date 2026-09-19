# S-01 DOM Parsing binding caller audit

Source-only review and generation; no build, test, generator, or WPT execution. This audit covers the binding/conversion group, not every XML/HTML parser or serializer algorithm.

## Pinned selected evidence

Inventory `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl`, WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`:

| Core source | SHA-256 | Obligation |
| --- | --- | --- |
| `domparsing/DOMParser-parseFromString-html.html` | `2182f12266e5bafb2e1dacbd649b1bc067deab5c3a65b6bd9ae6e2e90c071e85` | SupportedType enum rejection, inert parsed Document, HTML parsing semantics. |
| `domparsing/XMLSerializer-serializeToString.html` | `3d18794c20905cd7dbb9edbb539229fd8ab57b18ccb65e367012f02ee534dd32` | Native Node argument serialization across node kinds and namespace/escaping cases. |

`browser.webidl` declares Window-only constructors, a required `(DOMString or TrustedHTML)` plus SupportedType for parsing, and required Node for serialization. Generated operation entry checks required arity, resolves native receiver identity, and invokes `call_stateful_operation`. Receiver branding is held in inaccessible internal fields, not public prototype/name properties.

## Reviewed callers and generated correction

- DOMParser converts input then MIME enum, preserving author conversion exceptions and invoking the existing TrustedHTML gate before `parse_detached_document`. The helper uses inert native HTML or XML parsing, records the caller-owned Document URL, imports into the parser owner's arena and wraps in that realm.
- XMLSerializer previously converted the argument against the serializer receiver's arena. This incorrectly rejected real Nodes belonging to independent same-origin arenas. It now uses `node_identity::argument` to establish the Node's native owner and serializes directly from that owner's arena. Neither tree is imported or mutated; receiver branding remains separately enforced.
- Existing `DomRuntime::serialize_xml` remains the single serializer. No replacement string-based serialization or page adapter is introduced.
- `parsing_binding_tests` regression sources cover ordered input/type conversion, original exception short-circuit, case-sensitive MIME rejection, invalid Node/Proxy and receiver brands, descriptor flags/arity, inert parsing and XML parsererror Document, independent source/receiver realms, borrowed serializer methods, colliding local document content, and absence of source mutation.

## Remaining mapped scope

All runtime results and generated binding consistency remain pending the global generation barrier. The selected serializer's full namespace/escaping matrix and parser-specific malformed input/resource/inertness details remain tied to the existing native implementation and require their broader package source review and later qualification; the binding correction is not blanket domparsing closure. Whole S-01 manifest/interface/member breadth remains open.
