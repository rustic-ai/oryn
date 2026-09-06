# Oryn Blitz adapter patch

Source: DioxusLabs/blitz commit `9d5cef2fa3d01423081681843d91b76ef593443e`,
package `packages/blitz-dom` (0.3.0-beta.2). MIT OR Apache-2.0 licenses retained.

The package manifest expands upstream workspace metadata/dependencies; sibling
packages remain pinned to the same upstream commit. The only upstream Rust
change is `src/stylo.rs`: `is_html_element_in_html_document` checks the element's
HTML namespace instead of unconditionally returning true. This lets Stylo apply
its existing attribute-value case-sensitivity rules to SVG and empty-namespace
elements. Oryn's mirror currently models HTML documents; XML-document mode is
separate remaining work. No selector parser or cascade is duplicated.

Regression: `oryn-layout` Stylo namespace/attribute case-sensitivity test and
pinned WPT `css/selectors/attribute-selectors/attribute-case/value-case-sensitivity-svg.html`.
