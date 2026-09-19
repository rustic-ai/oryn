# Native contextual element construction

Upstream crate: html5ever 0.39.0 (crates.io checksum `46a1761807faccc9a19e86944bbf40610014066306f96edcdedc2fb714bcb7b8`). Its paired markup5ever 0.39.0 checksum is `7122d987ec5f704ee56f6e5b41a7d93722e9aae27ae07cafa4036c4d3f9757de`. Exact upstream VCS metadata is retained in each ORYN-PROVENANCE.json.

The extension passes the intended native insertion location into element construction before attributes or insertion. Ordinary insertion computes its existing insertion point before construction; foreign/root/adoption-agency construction passes the corresponding existing location. The sink resolves before-sibling/foster locations to the actual parent. This enables native custom-element registry selection and synchronous constructor results, including a different fallback handle. The original TreeSink method remains supported through a default contextual method, preserving unmodified consumers.

The Oryn sink invokes constructors only through a lexically scoped, arena-matched embedding callback. No callback or V8 scope is stored in the parser across tasks. All DOM RefCell borrows are released before callbacks. Fragment parsing without an embedding hook remains inert. No upstream tokenizer algorithms or token recognition rules were changed.

Source regression coverage is in oryn-dom html contextual constructor tests and oryn-native frame_script_preparation native HTML/XML tests. Execution is deferred by the full-generation directive; these are not passing-test claims.

## Native rendering target completion

The optional `TreeSink::open_elements` callback receives the exact stack after
all returns from token processing and an empty stack at parser end. A scoped
observer reads the existing tree-builder stack; it does not alter parsing or
stack algorithms. This covers implied closes, adoption-agency replacements and
table recovery paths whose direct Vec mutations bypass the legacy `pop` hook.
Oryn records this native state for HTML rel=expect target completion; parser
abandonment clears the corresponding flags through ParserRoots cleanup.

## Explicit Unicode and DOMString transport

The optional `TokenSink`/`TreeSink::character_reference_transport` hook escapes
only decoded character-reference output for a sink which explicitly enables a
lossless UTF-16 transport. Its default returns None and leaves other consumers
unchanged. Entity recognition, numeric-reference replacement rules and HTML tree
construction remain upstream algorithms. Escape pairs are emitted as one text
token so foster-parent publication never sees half an escape.

Oryn's shared HTML sink decodes token payloads before native node construction,
attributes, comments or text publication. Its element-name query reencodes native
DOMString names for the tree builder's matching rules. `ParserInput::unicode`
and `ParserInput::dom_string` distinguish fetched Unicode from authored UTF-16;
source slices preserve scalar and transport-pair boundaries. DOMString inert
APIs and live document.write share that sink; no final-arena decoding workaround
is used. Native regression sources cover surrogate names/text/attributes,
constructor and reaction observations, split writes/character references,
Unicode/storage-prefix collisions, raw script and fostered text. No build or
tests have been executed for this extension.

### Partial HTML update processing instructions

The tokenizer records whether a bogus-comment token originated at genuine
`<?...>` syntax. A default TokenSink callback retains upstream comment behavior;
Oryn's TreeSink opts into ProcessingInstruction construction. Literal comment
contents cannot impersonate marker instructions. The callback is incremental
across token input boundaries and retains the existing tokenizer EOF handling.
Native marker matching, sanitizer ordering, tree-scope isolation and retained
structural patch operations live in `oryn-dom/src/html/partial_updates.rs`.
Source only; no executable validation has been run.
