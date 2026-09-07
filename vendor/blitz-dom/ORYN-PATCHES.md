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
