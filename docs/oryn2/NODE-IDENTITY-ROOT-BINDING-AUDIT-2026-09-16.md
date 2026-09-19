# S-01 Node identity, comparisons and roots

Source-only generation; no build, test, generator or WPT invocation. These bounded rows do not close the full binding inventory or runtime acceptance.

## Selected requirement evidence

Canonical inventory `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl`, pinned WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`:

| Core source | SHA-256 | Concrete obligation |
| --- | --- | --- |
| `dom/nodes/Node-isSameNode.html` | `274cb59d9a6a382f98bbb98dce4cbebffa4a591cfd6366b09f8f4d12dcc70afc` | Native identity for Document, element, Attr, DocumentType, PI, Text, Comment, DocumentFragment; identical structure does not imply identity; null is false. |
| `dom/nodes/rootNode.html` | `60411f94bcb3f5da7eb283409bab895ac19d9ecfebb4c062fdae8cb4734898ba` | Default/false options return the ordinary root, true returns shadow-including root; connected/disconnected elements, shadow descendants and fragment roots. |

Pinned `interfaces/dom.idl` supplies required nullable Node argument, optional `GetRootNodeOptions` dictionary, and `composed=false`. The generated descriptor path remains the single installation route.

## Source reviewed / generated

- Added missing `Node.isSameNode` IDL and native operation. Exact argument owner is established before resolving its NodeId, and identity compares arena ownership plus native NodeId. Proxies/plain objects fail conversion; null/undefined convert to null.
- Replaced `getRootNode` option-ignoring operation with `node_identity::root`; dictionary conversion occurs before observing the tree, native receiver owner is re-resolved afterwards, and existing ordinary/shadow-including traversal is reused. No new shadow-tree algorithm is introduced.
- Corrected related `contains`, `isEqualNode`, and `compareDocumentPosition` conversion for valid foreign-arena Nodes. Foreign containment is false; structural equality uses `DomRuntime::is_equal_node_from` (same shared iterative comparison as ordinary equality); disconnected ordering is stable and opposite without aliasing identical numeric IDs.
- `node_identity` regression sources cover the pinned node-kind matrix, receiver/argument brands and descriptors, nested closed/open shadow roots, detached host and getter-driven tree change, null/invalid dictionaries, exact thrown exception identity, two independent native arenas, symmetric structural equality, foreign containment, and repeatable opposite disconnected order.

## Explicit remaining inventory

S-01 still requires the full manifest-linked interface/member exposure/conversion/descriptor map beyond these rows and FileAPI. Node comparison attribute-order/ancestor-special cases outside this change remain subject to that broader review; these source changes do not claim every DOM algorithm complete. Generated bindings and candidate artifact compatibility are intentionally pending the whole-generation barrier, followed by compilation/regressions and fixed-denominator qualification.
