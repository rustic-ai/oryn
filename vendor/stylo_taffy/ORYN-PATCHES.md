# Oryn native adapter changes

Base: Blitz revision `9d5cef2fa3d01423081681843d91b76ef593443e`, stylo_taffy `0.3.0-beta.2`.

## Native Ruby display admission

The native Stylo profile exposes Ruby, RubyBase, RubyText, RubyBaseContainer and RubyTextContainer. The Taffy adapter preserves them as measurable Block boxes with Normal box generation, and accepts InternalRuby outer display without hiding the box. This is substrate conversion; Oryn `crates/oryn-layout/src/ruby.rs` owns annotation pairing and placement. It does not claim Taffy implements ruby annotation layout.

The Blitz traversal uses its existing flow/inline child selection for these boxes; flow classification treats InternalRuby as inline-level rather than introducing a block boundary. Other display adapter matches retain their existing explicit cases or wildcard behavior.

Regression source: `crates/oryn-layout/tests/native_css_profile.rs::ruby_adapter_retains_measurable_boxes_and_normal_generation`, including the display:none negative control. Source generated after test17 compilation stopped; not executed yet.

## Test19 exhaustive-consumer correction

The earlier table-match audit was incomplete: `blitz-dom/src/layout/table.rs::collect_table_cells` ends in explicit arms, not a wildcard. All five ruby inside variants now take the same branch as Flow/FlowRoot/Flex/Grid. This preserves that collector's existing treatment of non-table descendants; it does not implement anonymous table cells or alter native table geometry. Oryn's computed-style mirror calls `resolve_stylist` only (`oryn-layout/src/lib.rs`); Blitz table construction is not the native geometry path.

Separately, `block_item_needs_wrap` now includes InternalRuby, matching inline classification so the existing mixed-flow anonymous wrapper preserves its inline sequence. A direct predicate unit source and the native `internal_ruby_boxes_remain_in_anonymous_inline_runs_around_blocks` compute regression retain content/box/source order coverage. No execution yet.

Full display-match audit (complete bodies, excluding commented damage prototype):
- Adapter convert: inside conversion has explicit ruby cases plus feature fallback; outside conversion explicit InternalRuby; box-generation wildcard remains Normal. Wrapper has no display enum match.
- Blitz traversal: explicit complete inside enumeration including five ruby variants.
- Blitz node `is_or_contains_block`: outside wildcard includes ruby as non-block unless Flow descendant logic applies.
- Blitz construct flow classification: explicit InternalRuby inline case. Container-inside match ends in flow fallback. Three tuple matches (pseudo recursion, embedded boxes, inline construction) each have terminal wildcard arms; ruby uses atomic box branches where applicable.
- Blitz table collector: explicit complete enumeration, corrected here.
- Blitz layout/mod matches Taffy Display, not Stylo Display; no added variants apply. Other DisplayInside/Outside uses are equality/matches predicates, not exhaustive matches; mixed-flow wrapping was the inconsistent inline predicate repaired above.

The pinned `Display::to_css` explicitly serializes `(Block, Ruby)` as `block ruby`. Native intrinsic sizing and ruby-layout routing now accept both `ruby` and `block ruby`, and the generic flow fallback excludes both. The exterior remains block-level (`is_inline` is unchanged). Existing ruby pairing regression now also verifies block-ruby computed serialization, non-inline classification, annotation centering/placement and retained base/annotation text.
