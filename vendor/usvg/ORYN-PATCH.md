# Oryn native SVG provenance patch

Upstream: linebender/resvg, tag v0.45.1, commit
`1b6c2fddbcbeffa8135df4323b02aaae84890907`.
The 34 retained files are crate source, manifest, README and both upstream
licenses; upstream code generators, docs and test fixtures were not copied.
No generators, dependency resolution, builds or tests were run.

The standalone manifest preserves version/features and expands the upstream
workspace license to MIT OR Apache-2.0. Oryn's workspace patches crates.io usvg.

Native PositionedGlyph source ranges come from shaping ByteIndex and full
cluster byte lengths, then add each original text chunk's byte offset. Every
glyph in a combining cluster shares the complete range; bidi run direction is
retained directly from rustybuzz input, not inferred from glyph positions.
Post-parse generated flattened-text IDs preserve authored IDs and cannot affect
selector matching. Native source-group flattening retains actual glyph paint,
font selection, transforms and color-glyph support while omitting decorations.
Text-cluster trees retain the original font database and resolved paint servers.

Regression sources live in oryn-render-core/src/svg_text.rs. All are unrun.

The positioned cluster also carries its native advance/ascent/descent quad after
span, baseline, writing-mode and text-path transforms. This reuses the existing
convert_span line-metric policy, including nonpositive advances, and avoids
replacing rotated character geometry with an axis-aligned ink rectangle.
UPSTREAM.json records the upstream Git blob identity and size of each copied file.

## Text-element geometry identity

Oryn retains source text-element ancestor IDs on parsed TextSpan and exposes
those IDs plus the existing glyph-metric bounding rectangle on layouted Span.
This does not change shaping, glyph positioning, painting, or font selection.
The native SVG geometry consumer supplies synthetic node identities and unions
only the spans belonging to each original tspan/textPath/anchor. Consequently
child element bounds retain their own metrics instead of receiving the complete
parent text box. This extends the pinned usvg 0.45.1 source API; it is not an
upstream API or an authored-property authority. Regression sources in
oryn-layout/src/svg_geometry.rs cover positioned tspan geometry. Execution is
still deferred under the overall generation directive.
