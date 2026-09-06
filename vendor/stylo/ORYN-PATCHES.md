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
