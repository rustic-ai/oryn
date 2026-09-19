# S-01 FileAPI binding caller reconciliation

Source generation only. No build, generated WebIDL output, test, worker service, or WPT run has been executed for this change. This is a bounded S-01 contribution, not closure of S-01 or Core acceptance.

## Authoritative selection

Inventory: `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl`; pinned checkout revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`. Existing inventory statuses remain historical.

| Selected Core source | SHA-256 | Obligation / production route |
| --- | --- | --- |
| `FileAPI/FileReaderSync.worker.js` | `780cf21be85a875937c80f91d47e4237140df11ddb59268590ff78f0d2ecbbf9` | Constructor and four synchronous read results; native `FileReaderSync` record, generated platform constructor/operation routing, `file_reader_sync::operation`. |
| `FileAPI/idlharness.any.js` | `6918411b69b5f409bc042522e625d3a2fd5ce7ee28b12492fafa28affa98dc34` | Window and dedicated-worker interface exposure, prototype/member descriptors, brands. `browser.webidl` feeds normal generated installation; `exposed_on_global` admits generic Worker or exact native subtype. |
| `FileAPI/idlharness.worker.js` | `4615f2bf116347a547c6a5db4429ada3ecb2258fdacd7c96af121d2b806d0c3d` | Worker-only FileReaderSync in pinned `interfaces/FileAPI.idl`: DedicatedWorker and SharedWorker, not ServiceWorker. `NativeAgentRuntime::create` publishes parent bootstrap kind before binding installation. |
| `FileAPI/filelist-section/filelist.html` | `61b072f41d295ba2d2f3c52cca1cdda4251f3245b7e87cff29371a416b619150` | Existing native `file_lists` indexed handlers, input setter/getter and native selected-file identities require further descriptor/serialization reconciliation. Existence alone is not closure. |

## Generated bounded implementation

- FileReaderSync IDL and native branded record/constructor/dispatch, with no EventTarget inheritance or asynchronous FileReader state.
- Exact dedicated/shared exposure derived from native context metadata. Generic worker contexts without authenticated subtype do not acquire subtype-only interfaces. Window and service worker stay excluded.
- Native Blob/File argument conversion rejects arbitrary objects and proxies, resolves original owning realm, and rechecks retirement after optional encoding conversion. No registry borrow is held across author conversion.
- Fresh ArrayBuffer, byte-preserving binary DOMString, scalar decoded text, and MIME/base64 data URL (including empty Blob fallback) reuse native byte storage and existing FileReader decoding precedence.
- Regression source `file_reader_sync::tests::worker_subtype_exposure_and_native_sync_reads`: five exposure configurations; native brand; fresh buffers; binary/scalar/BOM/MIME results; empty data URL; method descriptors/arity; invalid receivers/proxy Blob; conversion ordering.

## Remaining requirement-specific source review

1. Cross-realm FileReaderSync input/result realm regression with two native worker realms and retirement during encoding conversion; current owner-aware implementation is present but the broader realm matrix remains unqualified.
2. FileList indexed descriptor/serialization and File/Blob structured-clone authority caller map against pinned selected sources, beyond the existing `file_lists` handlers. Coordinate Blob constructor/slice/read ownership with root and asynchronous FileReader with P/W.
3. Whole S-01 namespace/traversal/Range and remaining manifest interface/member conversion inventory. This FileAPI map does not imply those are generated or verified.
4. After the whole-plan generation barrier: regenerate bindings, compile, run generated regressions, and qualify original selected worker URLs with unchanged denominator and evidence provenance. No current result is claimed.

## FileList structured clone follow-through

The confirmed missing serialization route is now source-generated:

- `structured_clone::Serializer::write_host_object` recognizes exact native FileList (including live DataTransfer file views), snapshots its entries, and writes each File through the same engine graph. File references shared between the list and other payload properties preserve identity.
- `HostData::FileList` / `WireHost::FileList` carry a marker; serialized graph bytes carry the bounded read sequence and existing Blob/File host records carry bytes and metadata. Wire validation/retained-size accounting therefore counts file content through existing Blob records, without a second unaccounted byte vector.
- `Deserializer::read_host_object` reads entries and `file_lists::restore_clone` accepts only native File records. The destination native selected-file snapshot and private entries array share the deserialized wrappers, preserving indexed access/item identity and ordinary FileList descriptors. Author objects, plain Blobs, proxies, and retired source owners are rejected.
- The route is shared by local `structuredClone`, `serialize_wire`/`deserialize_wire` messaging, and `serialize_storage`/`deserialize_storage`; storage does not need a second FileList serialization implementation.
- `WireHost::FileList` is appended, retaining prior variant indices, within the existing unbuilt native contract batch. Historical evidence and runtime wire version are untouched; new artifacts must be generated and qualified together after the generation barrier.
- Native regression sources cover populated DataTransfer list snapshot after mutation, empty input list, repeated list/File graph identity, wire round trip into another realm, destination prototypes/metadata/index descriptors, and rejection of invalid decoded File entries. No regression has run.

Pinned reachability: `html/webappapis/structured-clone/structured-clone-battery-of-tests.js` lines 365–381 explicitly test FileList instance/empty length and nested array/object forms; `IndexedDB/structured-clone.any.js` line 312 also carries FileList obligations in the inventory's necessary tier. This is not an expansion to unrelated APIs. Whole S-01 breadth, complete cross-realm retirement cases, generated artifact compatibility, and runtime acceptance remain open.

## Independent follow-up source review

Reviewed the actual generated-dispatch entry points, exposure predicates, native argument ownership, destination wrappers, wire validation/accounting, and IndexedDB's production JSON/base64 storage envelope.

Corrections generated:

- FileList recognition now checks native numeric identity shape before attempting `stateful_id`. This matters because the structured serializer probes every host object: probing a DOM Node's BigInt wrapper identity must not throw a stray numeric-conversion exception before the intended DataCloneError.
- FileReaderSync Blob conversion and FileList decoded-entry conversion validate the canonical native wrapper, not merely a record at a matching numeric ID. Proxy/plain-object and nonplatform identity probes cannot acquire File bytes.
- Added FileReaderSync regression source with a Blob from another native realm, local colliding-record decoy, callback-realm ArrayBuffer, exact conversion exception propagation, source retirement during encoding conversion, retirement before a read, and invalid label/MIME fallback. This closes the previously listed missing **source regression** matrix for that bounded operation; it does not establish runtime passage.
- Extended the FileList graph regression through `idb_value_to_json` and `idb_json_to_value`, the actual storage envelope and Blob side table, rather than inferring storage coverage from wire helper reuse. Added DOM-node rejection/next-clone checks for probe exception isolation.

Proposed bounded checklist delta for root (main checklist intentionally unchanged):

| Requirement | Source state | Qualification |
| --- | --- | --- |
| FileReaderSync native IDL/dispatch, dedicated/shared-only exposure, conversion/brand/results/descriptors, foreign Blob and retirement regression sources | Generated; caller and regression source reviewed | Not built or run |
| FileList local + wire + actual storage-envelope clone, graph identity, native destination wrappers and descriptors | Generated; caller and regression source reviewed | Not built or run |
| FileList serialization probe/type identity isolation | Generated corrective guard and negative regression source | Not built or run |
| Whole FileAPI/pinned WebIDL interface-member inventory | Still open; these rows do not cover all Blob/File/FileReader algorithms owned by other lanes | Unqualified |
| Whole S-01 interface coverage and regenerated artifact consistency | Still open | Unqualified |

No historical inventory status, denominator, signed evidence, or main acceptance checkbox was changed. Worker/context protocol and generated bindings remain part of the existing unbuilt batch.
