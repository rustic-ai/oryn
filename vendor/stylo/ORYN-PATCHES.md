# Oryn Stylo patch

Source: crates.io `stylo` 0.20.0, upstream commit
`67faaab3ff7aa66780ec1d0f51ca47e177b812d3` (`style` directory).
Original crate archive SHA-256: `ebaeb0b51f5e574bf36606c4dc982430a20d7a17175bdc0deab4d43d2063040b`.
Upstream source headers and MPL-2.0 licensing are retained.

## Changes

`servo/selector_parser.rs`: enable the existing selectors engine's `:has()`
and `:nth-child(... of ...)` grammar through its two parser feature switches.
No selector grammar, specificity, matching, or cascade algorithm is replaced.
Both Oryn DOM queries and Blitz/Stylo stylesheet traversal use this same patch.
Oryn reconstructs each immutable style mirror on invalidation, so it does not
rely on incremental relational-selector invalidation in the upstream adapter.

`stylesheets/rule_parser.rs`: enable the existing `@scope` rule parser in Servo
builds. The upstream scope bounds, specificity, proximity, and matching
algorithms remain unchanged; Gecko continues to use its preference.

## Verification and maintenance

Oryn's layout tests cover relational child/sibling matching, filtered nth-child
positions, invalid nesting, stylesheet cascade, scope roots/end boundaries, and
mutation between snapshots.
Run the complete frozen DOM and CSS WPT roots before treating a new binary as
verified. This patch does not close shadow-tree, state, or CSSOM integration gaps.
On upgrading Stylo, compare these feature switches with upstream and remove the
patch if the supported upstream configuration enables these features. Do not edit
the Cargo registry cache.

## Native background image features (2026-09-16)

`values/specified/image.rs`: enable the already-implemented cross-fade parser
in Servo builds. Percentage parsing, computed-value storage, serialization and
cascade remain upstream; Oryn renders bounded captured-resource component trees.

`values/specified/background.rs`: expose the existing `BackgroundClip::Text`
variant and its background-only validity arm in Servo builds. Other Gecko-only
mask/SVG values remain gated. Native glyph masks implement this painting value.

These are narrow feature-exposure changes atop the pinned source above. No
registry cache or dependency version changes are involved. Generated layout
regressions check authored CSS survives cascade/export; renderer regressions
cover component blending and transformed descendant glyph masks. They have not
been built or run under the all-generation-before-testing gate.
# Native select pseudo projection continuation

The pinned Servo adapter now parses `::picker(select)`, `::picker-icon` and
`::checkmark`, serializes them canonically and treats them as lazy pseudo styles.
The existing `appearance:base-select` value is enabled for this Servo build.
Oryn requests these actual Stylo pseudo styles separately from primary CSSOM
values. Native picker/content production is still being generated; parser
availability is not a conformance claim. No dependency resolution, build,
formatter or test execution accompanied this source patch.

### Native counter property reachability (2026-09-16)

`properties/longhands.toml` exposes counter-reset and counter-increment without
the Servo layout.unimplemented preference, and exposes counter-set to Servo.
The existing upstream typed parser/computed values are retained. Oryn now owns
tree-identity inheritance, reversed auto initialization and native marker
consumers. Property generation remains deferred to the single global build
phase; no generators/builds/tests were run for this patch.

### Native declaration-tree provenance (2026-09-16, source only)

Computed list-style-type and generic content items retain absolute native tree
identity obtained from ElementContext at the winning declaration's cascade level.
CSS serialization omits identity; inheritance and animation conversion retain it.
The embedding resolves it to captured native shadow scope for named counter style
lookup. Reset-rule caching is disabled for these conversions and shadow style
sharing requires the same root, avoiding reuse of absolute identities across equal
cascade data. `values/{specified,computed}/list.rs`, generic/specified counters,
ElementContext, computed Context and sharing checks contain the patch. No property
generator, build or regression execution has occurred during source generation.

## Native CSS admission and process profile (2026-09-18)

Native CSSOM and layout now share `native_profile::initialize` before parsing and property enumeration. The profile enables implemented grid/columns/writing-mode/variable-font/container and native rule syntax consistently; it does not enable the blanket `layout.unimplemented` preference. Property metadata admits native-consumed Gecko-only values and explicitly supported formerly-pref-gated longhands; native print break variants retain legacy alias restrictions. Full source inventory and regression rationale: `docs/oryn2/NATIVE-CSS-ADMISSION-2026-09-18.md`. No geometry assertions were weakened.
