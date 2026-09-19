# V-01/V-02 caller audit — source generation

No build, test, renderer, generator or conformance run was executed.

## Pinned required findings implemented

The frozen reference inventory points to revision
`719d5e38fdd0903a18ed9007aba816c98cc491e0`.

| Exact selected source | Required algorithm/caller | Confirmed source gap and generated correction |
|---|---|---|
| `css/selectors/focus-within-shadow-002.html` | Shadow host receives focus-within when its shadow descendant is focused | Stylo mirror state followed selector ancestry, ending at a shadow-root child whose selector parent is intentionally absent. `shadow_tree::focus_within` now follows the rendered flat-tree parent map; production style state consumes it. |
| `css/selectors/focus-within-shadow-003.html` | Host ancestors also receive focus-within | Same correction continues through host/document ancestors. Selector combinators retain their original shadow boundaries. |
| `css/selectors/focus-within-001.html` through `008.html` | Self/ancestors/native controls and universal selectors | Existing ordinary ancestry remains covered; new regression source also checks focus transfer/blur removing former state. |
| `css/selectors/i18n/lang-pseudo-class-across-shadow-boundaries.html` | Language inheritance through the shadow host | Existing native paint language inheritance retained; DOM query adapter follows parent-or-containing-host with namespace-aware own language. |
| `css/selectors/invalidation/lang-pseudo-class-in-has-xhtml.xhtml` | Namespaced xml:lang precedence and mutation inside relational selectors | DOM query adapter previously read a literal null-namespace `xml:lang`; it now reads expanded XML namespace/local name. Paint no longer treats a null-namespace lookalike as XML authority. Existing fresh style mirror recomputes inheritance after mutation. |
| `css/selectors/selectors-4/lang-012.html` | Language range list matching | DOM query adapter now consumes Stylo's range list and RFC 4647 extended filtering, including wildcards, skipped script subtags, singleton boundaries and unknown-language empty range. Native paint already implements the same range algorithm. |

## Production integration and regression sources

- `oryn-dom::DomRuntime::element_focus_within` follows native flat-tree ancestry,
  including slot assignment. Focus in another same-arena document is retargeted
  to the queried document's focused element before ancestry traversal.
- `oryn-layout::dom_queries` and the production mirror FOCUS_WITHIN state now agree
  on shadow and slot ancestry. OIL native target resolution already consumes the
  live DOM query adapter, so these corrections reach semantic selector callers.
- New `shadow_tree_tests` regression bodies cover shadow/slot/host/document style
  changes, selector ancestry preservation and blur; DOM query bodies cover native
  focus transitions. Language bodies cover range lists, extended ranges, namespaced
  mutation/empty override, `:has`, null-namespace lookalikes and paint inheritance.

## Remaining mapped scope

These are bounded V-01 source closures, not whole V acceptance. The saved 1,015
Core reference identities / 1,016 reference targets preserve the denominator;
this pass inspected the exact sources above, not every visual assertion.
V-02 still requires the full per-entry reconciliation of inline/block, table/float,
intrinsic sizing, flex/grid dependencies, writing modes, positioning, overflow,
transforms and fragmentation against existing Taffy/Parley/Blitz adapters.
Existing modules are not sufficient evidence by existence alone.

Language fallback from HTTP Content-Language and HTML content-language processing
remains a distinct document-metadata producer review; the exact meta-language
invalidation fixture switches the document element to explicit `lang=en` before
its reference assertion, but that does not prove initial fallback correctness.
No new metadata field or invented fallback was introduced in this pass.

The checklist's FACE user-validity wording also needs careful scope reconciliation:
current `element_user_validity` deliberately applies to input/textarea/select and
existing custom-element regression sources assert that FACE validity does not
fabricate built-in user-validity selectors. FACE validation/submission/reset remains
owned by native custom-element/form state; do not change applicability just to
check that row without exact pinned requirements.
