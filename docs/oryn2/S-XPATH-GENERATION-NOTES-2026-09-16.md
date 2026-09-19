# Native XPath generation — not qualification

No build, test, formatter, WebIDL generator or conformance run was performed. Generated IDL bindings must be regenerated only after the global all-source gate. The current implementation has not been compiler-checked.

## Requirement and implementation

S-01 now has a native XPath 1.0 parser/evaluator in `crates/oryn-dom/src/xpath/` and native V8 APIs in `crates/oryn-js/src/v8_bindings/xpath.rs`. Evaluation traverses authoritative native NodeIds, not a mirrored document. All thirteen axes, predicates, reverse-axis position, node-set coercion/comparison, standard XPath 1.0 functions and namespace resolution have generated source.

`XPathEvaluator`, `XPathExpression`, `XPathResult`, `XPathNSResolver`, legacy namespace nodes and Document entry points are declared in the WebIDL source. Result snapshots retain actual Attr wrappers, including after removal/replacement. Iterators retain per-document opaque mutation-token identities; focus, listener activity and other-document mutations do not invalidate them. Detached Attr contexts retain identity/value with empty parent axes. Shadow-tree rooted evaluation stays within that tree.

Namespace nodes preserve immutable prefix/URI and owner identity. Node comparison supports both receiver directions, namespace lookup uses native DOM, and namespace EventTarget dispatch uses a separate native platform identity and native event path. Borrowed EventTarget methods reject XPath evaluator/expression/result values. Adjacent Text/CDATA sequences map to the first nonempty DOM node while string values aggregate the logical text node.

Resolver callbacks run without a DOM borrow, preserve abrupt exception identity, permit an empty resolved URI, and revalidate the native context lifetime after conversions/callbacks. Native Node lookupNamespaceURI/lookupPrefix/isDefaultNamespace share `crates/oryn-dom/src/namespaces.rs`.

## Pinned source inspection

Read-only public source retrieval used WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`, recorded in the repository capability manifest. Inspected `domxpath/` sources include document/evaluator cross-document and cross-realm cases; resolver callback-interface/non-string cases; lexical structure; booleans/numbers/predicates/node-sets/tree-order; text HTML element/attribute matching; `fn-id`, `fn-lang`, substring/translate; shadow DOM; iterator invalidation; orphan Attr crash; attribute-parent relationships; and the XML query corpus. These are source observations, not pass results.

Concrete corrections derived from those sources include SyntaxError for invalid expressions, JS TypeError for wrong result access, invalidated exhausted iterators, HTML matching from the context document, empty-string resolver namespaces, callable resolver precedence, ASCII-only language folding and first duplicate ID behavior.

## Generated resource and regression controls

- Source/token and recursive/constructed AST depth bounds also cover flat left-associative operator/path chains.
- Namespace resolution has a cumulative retained-byte bound; per-realm XPath registry storage has a 64 MiB budget and weak-wrapper reclamation accounting.
- Evaluation has traversal/work limits, bounded native string copying and comparison work accounting. Unicode character operations use flat codepoint storage retaining unmatched UTF-16 units; translate retains first-occurrence semantics without quadratic character lookup.
- Native and V8 regression sources cover axes/order/comparisons, coercion/functions, namespaces, mutation/snapshot lifetime, resolver mutation/abrupt completion, orphan attributes, shadow roots, logical text runs, receiver rejection, namespace events, UTF-16 literals and AST limits.

## Remaining source audit and integration obligations

Result reuse now preflights native storage before Attr/wrapper materialization and publishes its record only after anchor-array allocation/private installation succeeds; ordinary evaluation/type/resolver failure regressions preserve the old result. Cumulative temporary-value storage, attribute/namespace allocation and comparison work now have generated bounds and negative regression sources; resource-budget edge cases still require validation. Review orphan empty Text context behavior against normative sources. Resolver document-stream retirement now revalidates the receiving realm before publishing compiled expressions/results, with a generated document.open regression; broader cross-realm matrix remains a deferred validation obligation. Namespace events now use the native shared platform path. The discovered callback-root cycle is being addressed in S-PLATFORM-LISTENER-GRAPH-GENERATION-NOTES-2026-09-16.md. Global WebIDL generation, compiler integration, native regressions and the complete pinned cohorts remain deferred execution obligations; no Core/family pass claim follows from this file.
