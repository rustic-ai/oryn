# S ElementInternals ARIA source generation — 2026-09-16

Source-only implementation and regression generation. No build, test, formatter, generator or conformance execution. No passing claim.

## Generated implementation

- Native Node default accessibility strings and element references live separately from authored attributes. DOM attribute reads, serialization and cloning do not synthesize defaults. Effective native accessibility gives authored/explicit Element state precedence, including empty or invalid authored values.
- WebIDL source and native bindings reflect role plus standard ARIA string properties on Element and ElementInternals. Internals getters retain their own defaults when authors override the element. Nullable removal and lossless DOMString conversion are native.
- ARIA element/array references use validated native Element identities, ordered snapshot conversion, iterator closing on element-conversion failure, native scope filtering, and frozen array caches with weak ownership. Different properties/elements have independent caches; caches do not permanently retain removed target wrappers.
- Element reference setters clear the corresponding content attribute to empty; later authored attribute writes clear explicit references. Internals reference setters do not write content attributes. Native relation targets require no fabricated IDs.
- References participate in existing directed native GC tracing, with checked target generations and cleanup of unreachable cycles. Temporary roots protect targets during explicit attribute replacement. Cloning creates no inherited internals defaults.
- Native semantic consumers use effective defaults for role, name, descriptions, ARIA state/properties, hidden state and relations. Label/description dependencies track native reference targets; ARIA ownership rejects cycles/ancestor ownership. Default-state revisions invalidate dependent accessibility and descendants without forcing full dependency rebuilds for ordinary input FormState mutations.

## Regression sources

DOM sources cover author precedence, clone separation, native reference retention/cycle reclamation and repeated explicit assignment. V8 sources cover default/author separation, nullable removal, frozen cache identity, explicit/content precedence, iterator-close failure atomicity and retained old arrays. Native semantic sources cover role/name precedence, references without IDs, dependency invalidation and ownership cycles.

Pinned WPT sources reviewed include `custom-elements/element-internals-aria-element-reflection.html`, `html/dom/aria-element-reflection.html`, and the form-associated validation/nullish tests in checkout 719d5e38fdd0903a18ed9007aba816c98cc491e0. No WPT was executed.

## Related FACE follow-through

Capture now inventories every FACE control, with Null for absent saved state. `describe_custom_element_form_controls` returns metadata only without cloning saved strings/files. Snapshot producer preflights native metadata/files/entry storage and worst-case lossless V8 string representation through FormSnapshotBudget before cloning. Snapshot fidelity regression distinguishes USVString assignment normalization from lossless serialization of already stored native values. Root owns history routing and matching.

## Remaining qualification

Checked-in generated WebIDL tables are intentionally deferred to the global generation gate. Native accessibility remains the existing implementation's supported accessible-name algorithm; this batch does not certify all ARIA/AccName specifications, accessibility platform integration, or experimental HTMLSubmitButtonBehavior. Full source integration/build and complete pinned regression/conformance evaluation remain required after all generation finishes.
