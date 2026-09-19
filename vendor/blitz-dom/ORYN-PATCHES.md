# Oryn Blitz adapter patch

Source: DioxusLabs/blitz commit `9d5cef2fa3d01423081681843d91b76ef593443e`,
package `packages/blitz-dom` (0.3.0-beta.2). MIT OR Apache-2.0 licenses retained.

The package manifest expands upstream workspace metadata/dependencies; sibling
packages remain pinned to the same upstream commit. In `src/stylo.rs`,
`is_html_element_in_html_document` checks the element's
HTML namespace instead of unconditionally returning true. This lets Stylo apply
its existing attribute-value case-sensitivity rules to SVG and empty-namespace
elements. Oryn's mirror currently models HTML documents; XML-document mode is
separate remaining work. No selector parser or cascade is duplicated.

Regression: `oryn-layout` Stylo namespace/attribute case-sensitivity test and
pinned WPT `css/selectors/attribute-selectors/attribute-case/value-case-sensitivity-svg.html`.

`BaseDocument::resolved_primary_style` delegates to Stylo's own sequential
undisplayed-element resolver when traversal pruned a display:none descendant.
Oryn uses this for CSSOM values instead of copying its ancestor's declaration
map. Selector matching, cascade, and inheritance remain owned by Stylo.
Regression: `stylo_resolves_rules_inside_display_none_subtrees`, plus pinned
`css/cssom/getComputedStyle-display-none-002.html` and `-003.html`.

### HTML list ordinal presentational hints (2026-09-16)

`src/stylo.rs` adds native PresHints-origin counter-reset for ol start/reversed
and counter-set for li value. Integer parsing uses Stylo's HTML microsyntax
parser, and author counter declarations retain normal cascade precedence.
Reversed lists without start retain the native auto-initialization sentinel;
Oryn's counter-tree pass computes the starting value from actual scoped effects.
No builds, property generators or tests have been run.

## Native shadow-style mirror (generation, 2026-09-16)

`shadow_style.rs`, `stylo.rs`, and Node metadata now carry actual native shadow
roots, hosts, assigned slots and flattened style traversal separately from DOM
selector ancestry. Author sheets are rebuilt into Stylo `CascadeData` per root;
`:host`, `::slotted`, `::part`/`exportparts`, implicit scope and inheritance use
those hooks. Opaque element tokens are globally unique non-dereferenceable
identities. An RAII shadow-data owner removes its copied implicit-scope entries
on document drop; no pointer-to-slab recovery is used. Fresh native mirrors are
rebuilt on native mutation rather than reusing incremental slot invalidation.
Regression sources live in `oryn-layout/src/shadow_tree_tests.rs` and native
runtime snapshot tests. No build, formatting, generation or tests were run.

Native state reconciliation: modal/open/popover-open and required/optional now consume projected ElementState flags rather than unconditional false matcher arms. Regression sources live in oryn-layout; no validation execution in generation phase.
