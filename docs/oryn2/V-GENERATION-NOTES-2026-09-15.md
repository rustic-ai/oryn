# V source generation — parallel presentation lane

Source-only work on September 15, 2026. No build, formatter, test, screenshot,
print job, installation or conformance run was executed. Existing dirty work was
preserved. These changes do not close package V or any acceptance gate.

## Generated changes

- `crates/oryn-render/src/stacking.rs` replaces document-order-only opacity
  assembly with nearest-stacking-context ordering. Positive/negative stack
  levels, nested atomic contexts, static z-index exclusion, flex/grid item
  z-index, order-modified siblings, positioned auto and opacity compositing have
  generated regression sources in `lib.rs`. The parent integration owner is
  moving the generic planner to common so hit testing can share ordering.
- Context triggers cover positioned z-index, fixed/sticky, opacity, transforms,
  perspective, filters, isolation, containment, masks/clipping and relevant
  will-change. Recognizing a context does **not** implement its filter/mask paint.
  Backdrop-filter is now included in the computed style transport list.
- `crates/oryn-layout/src/positioning.rs` resolves explicitly specified physical
  insets against the nearest positioned (absolute) or transform/filter/containment
  (absolute/fixed) ancestor's used padding box. Right/bottom placement and opposing
  insets use used dimensions, margins and containing-block direction. Missing
  insets preserve the flow pass's static position.
- Position correction runs after ancestor used heights exist and before
  transforms, scroll projection and overflow measurement. Border/padding/content
  geometry, client rectangles, text fragments and text-cluster rectangles move
  together. Fixed box descendants share the viewport coordinate plane rather
  than receiving a second document-scroll subtraction.
- Viewport-fixed boxes reset inherited element-scroll transforms and overflow
  clips; geometry clipping stops at their fixed ancestor too. Fixed boxes with
  transform/filter/containment ancestors retain their local containing block.
- Generated layout controls cover right/bottom against a non-immediate ancestor,
  fixed descendants under scrolling/clipped ancestors, synchronized geometry and
  paint clips, opposing inset auto-margin allocation and RTL overconstraints.

## Explicit remaining implementation requirements

These are source findings, not a complete failure-to-algorithm mapping. The full
V-01 through V-05 requirements remain authoritative.

1. **Formatting/positioning:** the new placement pass does not repair intrinsic
   or percentage sizing when the flow parent differs from the containing block.
   Auto size with both insets, shrink-to-fit, inline containing-block fragments,
   writing-mode logical axes, sticky constraint rectangles, vertical negative
   auto-margin distribution, and grid-area absolute containing blocks still need
   formatting-level work. Static positions currently inherit the flow adapter's
   approximation. Absolute overflow clips and scroll ancestry between the box
   and its containing block still need specification-correct routing.
2. **Complete painting order:** element commands currently combine box background,
   border and inline content. CSS Appendix E requires finer phase separation;
   inline/float pseudo-contexts and table background/border phases are not complete.
   Top-layer ordering, outlines, preserve-3d depth sorting and blend/filter/mask
   application remain open. The shared planner must be integrated with actual
   native hit testing, not merely exported by the renderer.
3. **Writing modes and layout:** physical-axis block flow, table-row-as-grid and
   bounded float adaptation are not complete tables/floats/vertical formatting.
   Orthogonal flow sizing, ruby, fragmentation and full inline bidi interaction
   require native algorithms and source-bound WPT mapping.
4. **Print:** the current renderer's `render_pdf` rasterizes one viewport to one
   PDF page. That is not print-media reflow, page-size/margins, page selection,
   break propagation, repeated fixed content, pagination or print reftest support.
   A print document/layout contract is needed before a driver print endpoint can
   truthfully claim support; concatenating viewport images would not supply it.
5. **Paint primitives:** SVG image paint and CSS background images/gradients remain
   explicitly unsupported. Replaced SVG layout, per-side border styles/colors,
   CSS clipping/masks/filter effects and native font/OS fidelity need completion.
6. **Style and accessibility:** this lane did not close stylesheet/shadow mutation
   invalidation, namespace/state selectors, ARIA/name/tree projection, incremental
   accessibility recomputation or platform assistive-technology assessment. Those
   span DOM/native/JS/semantic owners outside the assigned two crates.
7. **Evidence:** no pinned-Core test ID mapping was generated here, no test was
   executed, and no pass count or capability completion claim follows from these
   sources. Parent integration must keep all V checklist obligations open until
   complete generation and later qualification.

## Continued generation: actual containing-block sizing and sticky axes

The earlier placement-only sizing limitation is superseded for ordinary physical
absolute/fixed boxes: the flow pass now queues out-of-flow roots, finishes normal
ancestor geometry, then formats each queued root against its actual used padding
containing block. Nested roots join the same queue. Percentage width/height,
percentage padding, opposing-inset auto dimensions, shrink-to-fit and min/max
constraints are resolved before descendant formatting. Taffy receives absolute
positioning for these items, so they do not consume flex/grid/float flow space.
Generated controls cover percentage descendants, both box-sizing modes, min/max
constraints and ordinary/flex/grid flow exclusion. Static position details and
inline/grid-area containing blocks remain open.

`sticky.rs` adds scrollport/containing-block constraint translations before shared
paint and geometry projection, retaining normal sibling flow. It handles pinning,
end constraints, oversized sticky view rectangles, margins and opposite writing
axes. Generated controls compare sticky geometry, painted text and unchanged
sibling/offset dimensions while scrolling. Nested sticky interactions, transformed
scrollport constraint spaces and complete scroll overflow propagation need further
work and validation.

`writing_mode.rs` supplies logical axis direction for relative/absolute/fixed
inset overconstraints and sticky end-inset adjustment; computed style transport
now includes writing-mode and text-orientation. Generated directional controls
cover horizontal-tb, vertical-rl/lr and sideways-lr. This is **axis plumbing, not
complete vertical text or vertical block formatting**. The inspected Parley path
still builds horizontal lines and glyph runs; full writing-mode support needs
logical inline/block formatting, glyph orientation/vertical substitutions and
matching cluster/range/paint projection. No test/build/formatter was executed.

### Pinned source anchors for remaining vertical work

Inspected under pinned WPT revision
`719d5e38fdd0903a18ed9007aba816c98cc491e0`:

- `selection/caret-position-should-be-correct-while-moveup-movedown.html`:
  `Selection.modify` must collapse and preserve the specified caret offset while
  moving between horizontal and vertical-lr/rl lines. This requires the S/native
  selection algorithm and V line/cluster axes together.
- `html/semantics/forms/the-select-element/customizable-select/select-appearance-writing-mode-vertical-rl.html`
  and its vertical-lr counterpart: native `showPicker`, base-select appearance,
  top-layer/popup anchoring and vertical formatting must agree with the reference's
  `showPopover({ source: button })` path. Author script cannot replace that state.
- `html/semantics/forms/the-select-element/customizable-select/select-base-appearance-writing-modes.html`:
  reference uses inline-flex, logical padding/minimum dimensions, generated counter
  disclosure marker, rounded borders and automatic inline margin. It is not a
  text-only rotation obligation.

These are direct source anchors, not a manifest/execution-count reconciliation.
The official pinned manifest and qualification ledger still decide every entry
and variant; references are not independently counted as test obligations.

## Continued generation: native vertical glyphs and ordinary block formatting

The earlier horizontal-only production shaping finding is superseded for the new
vertical path. Inspected Parley 0.11.1's shaper calls: it requests LTR/RTL only.
`vertical_text.rs` now reuses Parley font fallback, style/run and bidi itemization,
then invokes pinned HarfRust 0.12.0 TTB shaping for upright segments. It transports
actual vertical glyph IDs, vertical advances/offsets, variable font coordinates
and font resources. ICU 2.1.2 supplies vertical orientation and line boundaries.
Unicode Tr characters probe vertical shaping output and use the alternate when
present, otherwise retain sideways orientation. CSS upright/sideways overrides
and sideways-lr orientation are represented explicitly.

The parent added `PaintGlyphRun.orientation` with a default upright enum to the
common transfer contract. All constructors in this lane were updated. Renderer
sampling rotates each glyph's actual outline coverage around its physical
baseline; it does not rotate an entire paragraph bitmap. `vertical_text` converts
logical line/cluster boxes to physical columns after the content box is known.
Normal vertical blocks fill their inline extent and advance child blocks on the
horizontal block axis; auto block extent and deferred positioning anchors move
with those children. Selection integration owner was informed that cluster rects
are physical pre-transform viewport coordinates and line IDs are local to their
layout node; computed writing-mode supplies the inline axis.

Generated regressions cover Unicode/grapheme orientation, true TTB advances for
I/W, vertical wrapping, preserved cluster provenance, vertical-lr/rl block child
positions and physical per-glyph coverage rotations. Dependencies were added in
source only; no install, lockfile generation, build, formatter or test occurred.

Remaining vertical requirements include complete orthogonal flow sizing, vertical
flex/grid/table layout, alignment/baselines, inline atomic boxes, ruby and text
combine, line-boundary bidi/contextual reshaping, full line-indent/justification/
spacing and font fallback qualification. These are real open implementation
requirements. The new path is not complete V or conformance evidence; all earlier
print/SVG/AX/platform and fine painting phase gaps remain open.

## Continued generation: logical flex/grid, orthogonal measurement and rebreaking

Vertical flex/grid no longer pass horizontal CSS declarations unchanged to Taffy.
`writing_mode` maps flex main direction, cross progression/wrapping and gaps;
grid tracks, implicit tracks, flow, named areas, line placement and alignment
are transposed before algorithm execution. Logical-start reversal changes physical
placement, not item dimensions or DOM order. Generated controls check physical
row/column positions in vertical/sideways modes and exact track/gap/area geometry.

Orthogonal intrinsic contributions now use vertically shaped block extent with
the available inline limit. Taffy's vertical measurements reshape against the
actual available/known inline size. Percentage margin/padding bases follow the
containing block's logical inline size; an indefinite horizontal parent's block
size uses the initial containing block fallback for vertical auto inline sizing.
The pinned `css/css-writing-modes/sizing-orthogonal-percentage-margin-001.html`
source explicitly requires 50px margins and a 100px vertical inline size for its
500px by 200px parent; a source regression encodes that geometry. Inspected
`orthogonal-parent-shrink-to-fit-001.html` and `001a.html`: their adjacent inline
box outcome additionally requires the remaining atomic inline formatting work.

Vertical line selection now re-itemizes and reshapes each selected source line,
including clipped style ranges and preserved source offsets. A generated AVAV
regression compares each selected line's actual glyph IDs/advances with isolated
line shaping, so paragraph kerning cannot leak across the chosen break.

Still open: iterative fit refinement when line-edge shaping changes which break
fits, complete justification/indent/tab/spacing behavior, orthogonal cyclic sizing,
inline atomic layout/baselines and all table/ruby/fragmentation obligations.
Vertical flex/grid baseline alignment and complex nested orthogonal contexts
still require generation and qualification. SVG, print-media reflow/pagination,
fine CSS painting phases, accessibility and platform assessments remain open.
No build, test, formatter, installation or runtime execution occurred.

## Continued generation: print-media layout, fragmentation and real PDF pages

`oryn_layout::print::compute(input, limits, options)` now creates a fresh print
input, clears screen/element scroll offsets, selects Stylo's actual print media,
and formats at the printable width/height. It does not consume a screen layout
or bitmap. Before transform/paint projection, fragmentainer gaps enforce supported
forced breaks, fitting avoid/replaced boxes, text-line boundaries and basic
orphan/widow counts. Shared document output carries print styles/fonts; per-page
geometry/projections apply margins, scale and fragmentainer clips. Viewport-fixed
content repeats. Ancestor records remain present even on pages where their own
boxes do not paint, preserving opacity and stacking for overflowing descendants.

The agreed `PrintLayoutOutput` contains `document` and `pages`; each page contains
viewport, geometry, paint projections and text fragments. Native integration is
owned by the parent. Shared `PrintSnapshot.source` holds identity/resources and
empty nodes; page-local nodes reference those resources. Page ranges support
single, closed, open and overlapping ranges without duplicate emitted pages.
Options require finite positive dimensions, valid margins/scale, at most 256
pages and bounded fragment metadata. The native integration adds its own stricter
aggregate node quota. Oversized documents fail rather than silently truncate.

`oryn_render::render_print_pdf` emits a genuine PDF page tree and one physical
MediaBox per print page. CSS pixels convert to points at 0.75, independently of
raster sampling scale. Page pixels come from the print-layout fragments. Shared
resource bytes and aggregate page pixels are validated before rasterization;
alpha is composited onto paper when encoding RGB images. The PageHandle `pdf`
helper and renderer CLI PDF route now use native print snapshots. The old
snapshot-only `render_pdf` utility remains for callers explicitly exporting one
provided snapshot; it is not the native print/conformance route.

Generated controls cover fresh print/screen media differences, avoid/forced
breaks, repeated fixed content, margins/scale, range validation/selection,
unchanged screen inputs, physical PDF MediaBoxes/page trees/xref and resource
limits. No rendering, PDF parser, build or test was executed.

### Remaining print requirements

- `@page` cascade, named pages, page-margin boxes/counters, page-size overrides,
  bleed/marks and complete recto/verso/left/right break propagation.
- Full nested break constraint propagation, table headers/footers/rowspan and
  vertical/multicolumn fragmentation, decoration-break, floats/positioned content,
  complete widow/orphan and forced-break edge behavior. Current gap insertion is
  a print-layout fragmentation implementation, but does not complete those rules.
- Reflow/paint of nested cross-origin documents, print-only resource readiness,
  and native beforeprint/afterprint lifecycle are parent runtime dependencies.
- Print shrink-to-fit performs bounded reformatting; CSS page constraints and
  overflow edge cases still need qualification.

Pinned Core print inventory inspected: seven `css/css-cascade/scope-implicit*`
print cases, `custom-elements/pseudo-class-defined-print.html`, three
`html/browsers/windows/iframe*print*` cases, and
`selection/selection-shadow-dom-crash-print.html`. The upstream executor passes
centimeters, 1.27 cm margins, background=true and shrink_to_fit=false. This source
inventory is not new execution or qualification evidence. All remaining SVG,
table/inline layout, fine paint-phase, accessibility and platform gaps remain.

## Page-context and margin source generation continuation

Source-only; no build, test, formatter, PDF renderer or conformance executor ran.

- Added `print_rules.rs`: native Stylo parsing of page selectors/declarations and nested margin rules; print-device evaluation of media conditions; supports filtering; page-name/pseudo-page specificity and important/source-order declaration cascade. Physical named paper dimensions and length/percentage margins now feed fresh print viewport sizing.
- `print::compute` uses generic page context for initial formatting and individual page contexts for equal-sized content areas, including mirrored margin placement. Differing content widths/heights return an explicit incomplete-layout error because they require variable fragmentainer reflow; first-page geometry is not silently reused for these pages.
- Added `PrintPageLayout.generated_nodes`, coordinated with native owner. Margin strings and decimal `counter(page)`/`counter(pages)` are parsed as CSS tokens, shaped with native text engine and share captured font resources. IDs are allocated downward, disjoint from all input node IDs and other generated page nodes. Synthetic nodes have no DOM parent and are not duplicated into page geometry.
- Fixed forced `break-after` processing to occur after descendant fragmentation, sharing recto/verso parity with `break-before`. A row that fits on a page is kept intact at a boundary; complete table fragmentation is still open.
- Generated regression sources for page cascade/media/names/importance, physical sizes, margin glyph resource ownership/identity/page counters, CSS string escapes and shared before/after parity.

Remaining precise print gaps: named page transitions are not wired from element `page` values; differing fragmentainer dimensions, blank-page pseudo classification, cascade layers/custom media/import contexts, page-context inheritance and variables, margin intrinsic/auto sizing (current edge placement uses equal thirds), margin borders/padding/vertical-align, nondecimal counters/running content, repeated table headers/footers/rowspans, nested break constraint propagation, vertical/multicolumn fragmentation and decoration-break. Native iframe printing/lifecycle remain coordinated outside these crates. The PDF encoder still emits image-backed pages with no text objects or ToUnicode mapping; generic PDF text extraction is therefore not supported even though Core print imagecomparison can consume the page images. No completion or conformance claim follows from generated source.

### Semantic PDF text and control wrapping continuation

The prior “no PDF text objects” limitation is superseded by newly generated semantic text source: `render/print_text.rs` projects actual shaped fragment text through page transforms and admits complete visible line boxes through page/ancestor clips. The PDF encoder emits invisible text objects with Unicode CMaps (including surrogate pairs), bounded encodings, physical positioning and width scaling over the unchanged raster page. Source tests cover Unicode maps, page clipping/duplication admission and font/page/xref object references. Text/encoding resource bounds are checked before raster allocation.

This is not full vector PDF output: visible glyphs remain rasterized. Partial clipped lines are omitted from semantic extraction until source-cluster paint metadata permits exact substring selection. Text selection boxes approximate fragment bounds, not per-glyph transform/spacing; reading order is stable physical top-to-bottom/left-to-right and needs multicolumn/bidi/vertical refinement. Extractor interoperability is unverified until the user permits execution after all generation completes.

Textarea defaults now use pre-wrap, break-word overflow wrapping and scrolling in both the real Stylo UA sheet and fallback UA style; case-insensitive wrap=off uses pre/no emergency wrapping. Generated shaping regression covers native control-ID UTF16 cluster ownership and wrap behavior. Native owner supplies live/masked control values; layout does not read stale DOM defaultValue children as a substitute.

## Variable-page, margin-sizing and table-group continuation sources

All work remains source generation; no build/test/render/executor was run.

- Added `print_continuation.rs`. Named and pseudo-page contexts now choose each fragmentainer's physical/content dimensions and format the remaining native source at that width/height. UTF16 source break tokens remove only consumed prefixes; completed boxes remain selector-stable through a private formatting style override. This replaces the previous variable-dimension error fallback. Explicit remaining heights and slice leading-edge continuation have source state. Named context changes force a new page; recto/verso transitions can create blank pages with `:blank` context and physical page counters. Font resources merge across layouts.
- Repeated table header/footer groups retain source text across continuation pages. Footer height is reserved from body fragmentation; repeated footer projections move into that reserved area, and body projections clip before it. The last fragment retains ordinary footer placement. Generated tests cover repeated groups and source-page identities; this does not yet supply full table track/rowspan layout.
- Added `print_margin_boxes.rs`. Equal-thirds placement is superseded by native intrinsic min/max-content sizing, proportional grow/shrink, and centered-box sizing. Generated numerical regressions encode the pinned `css/css-page/margin-boxes/dimensions-004-print.html`, `dimensions-007-print.html`, and `dimensions-009-print.html` algorithms. Margin boxes now carry actual border/padding/content geometry, fixed/percentage dimensions, cross-axis auto margins, vertical-align, and page-context inherited font/text properties. Page `width`/`height` descriptors and font-relative margins are projected into dimensions.
- Generated additional source regressions for named sibling sizes, changing first-page text width with exact source continuation, UTF16 surrogate boundaries, named recto blank pages and table group repetition.

Remaining V/print requirements are substantial: full table shared-column sizing, rowspan/colspan continuation, collapsed-border conflict resolution and repeated-group constraints; monolithic/inline formatting boundaries for named pages; nested float/column/orthogonal fragmentainers; break avoidance/widow/orphan constraints across resumed contexts; full decoration/background continuation and transformed positioned descendants; page auto/negative margins, cascade-layer/custom-media/import context and page variables; margin min/max constraints/overconstraint, corner auto-axis rules and vertical margin text. Semantic PDF partial-line extraction, exact glyph selection geometry and multicolumn/bidi/vertical reading order remain open. Existing notes about shared runtime iframe/lifecycle dependencies still apply. Generated regression assertions remain unverified.

### Shared table grids, resolved borders and empty caret geometry

Generated `tables.rs` builds table-wide column constraints (including col/colgroup widths, colspan, fixed/auto width distribution), row occupancy and rowspan height allocation. Rows consume shared placements instead of independent equal grids; header/footer groups follow table formatting order and cell tracks share spacing. Collapsed border used widths stay separate from DOM computed styles. Conflict selection implements hidden/none, width, style and originating-box priority. Physical border segments split spanning sides at neighbor endpoints and paint each shared segment once.

The coordinated common paint contract adds `PaintBorderEdge`, `PaintBorderSegment`, and defaulted projection edge/segment fields. Renderer sources now paint independent edge colors/styles, double/dashed/dotted/inset/outset/groove/ridge patterns, rounded border rings and rounded backgrounds through the existing transform/clip path. Segment borders suppress ordinary edge painting. Regression sources cover a rowspan with two different adjacent winning borders, shared track positions, declared computed width preservation, conflict priority and rounded/style sampling. CSSOM declaration values remain unchanged.

The coordinated caret contract adds `CaretStopGeometry` and `LayoutOutput.caret_stops`; native/JS owners handle transfer and consumers. Empty element/br boundaries use actual selected-font line metrics, separate DOM element child-index offsets, and the layout projection matrix. The metric probe creates no painted text or invented text-node UTF16 clusters. Regression source covers line-height, RTL inline origin, transformation and absence of fake text.

Remaining work includes resumed-rowspan placement/ownership across page continuations, vertical table axes, baseline alignment and full table width edge cases; collapsed corner joins and all origin/tie cases; exact dashed/dotted corner phase and border-image; contextual br-after/empty-inline caret placement; and the previously listed fragmentainer/paint-phase requirements. No source here has been built or executed.


## Native line-break carets and table continuation follow-through (source only)

Generated `br` paragraph provenance using U+2028 forced line separators. Before/after element child-index caret stops come from the actual shaped line boxes, including consecutive and vertical line breaks; hidden inline subtrees are excluded from aggregate text. Logical subtree relocation also moves the underlying break provenance. No UTF16 text-node identity is invented for `br`.

Rowspan continuation retains the final covered row while spanning text remains, so zero-height consumed carriers cannot strand the remaining source. Added a multipage regression asserting one copy of the span text and stable occupied columns. Collapsed border candidates now restrict row, column, and group borders to coincident edges; equal cell conflicts apply the table direction when choosing the horizontal winner. Cell `middle` and `bottom` vertical alignment moves actual text, source clusters, descendant boxes and caret provenance within the allocated cell box before pagination/projection. Added native geometry regressions.

All changes remain unbuilt and unrun under the global generation gate. Still open: table baseline ascent/descent row sizing, vertical table grid axes, full column/group background phases, colspan segment-specific column-origin conflicts, complete decoration/rowspan behavior at fragmentainer boundaries, and the earlier recorded print/painting/orthogonal formatting requirements. These additions do not establish conformance or close V.


### Table baseline sizing follow-through

Generated native first-line baseline measurement for in-flow cell content and row-wide baseline constraints. Baseline displacement increases the row/span height constraint before formatting, then shifts cell content through the same geometry path as middle/bottom alignment. Added a mixed-font-size and padding regression requiring equal native glyph baselines and no row-bottom text overflow. This replaces the earlier missing basic baseline-sizing entry; complex inline replaced baselines, vertical tables, and nested formatting-context baseline rules remain open. Source only; no execution.


### Column-edge segmentation and RTL tables

Retained shared physical column offsets in collapsed-table plans, splitting spanning horizontal borders at column boundaries as well as neighboring-cell boundaries. Each segment now considers only its coincident column/column-group edge. RTL tables map both cell spans and column owners onto reversed physical tracks before sizing and painting. Added a regression combining RTL cell order and differently styled column borders across one colspan. This closes the specifically recorded column-origin segment gap in generated source; execution and full border/fragmentation conformance remain pending.


## Vertical tables and logical page fragmentation (source only)

Generated logical inline/block table constraints, physical row/cell allocations, and column/colgroup geometry for horizontal, vertical-rl/lr and sideways modes. Cells use native vertical shaping at their allocated inline size; the same allocation correction applies to vertical flex/grid items. Rowspan placement and collapsed border sides/segments now map through the table axes. Native central baseline and middle/bottom alignment move physical content along the block direction. Column elements no longer participate as ordinary flow children, and their grid geometry provides background paint boxes.

Added internal `print_axis.rs`: fragmentation normalizes only geometry used by break decisions into page block progression and restores physical geometry before painting. Glyph positions/orientation are not swapped or re-shaped by that mapping. Uniform and variable-page paths now use the same block extent, page origin, source-cluster consumption, continuation dimensions, leading decoration edge, footer reservation/relocation and clipping. Fixed content retains page-relative physical placement. Shrink-to-fit measures the inline axis. Generated page-margin text also shapes and physicalizes vertical text natively.

Regression sources cover table modes/directions, column geometry and border segments, vertical fixed/uniform pages, named variable-width vertical pages, and repeated table groups in both vertical directions. No builds, formatters, tests, rendering or conformance runs performed.

Remaining limitations include complete orthogonal-cell intrinsic/baseline negotiation, first/last inline replaced baselines, transformed fragmentainer boundary semantics, full table background layer ordering (especially interleaved column groups and row groups), decoration continuity/collapsed joining at repeated headers/footers, CSS fragmentation break precedence and advanced constraints, and the previously recorded SVG/filter/paint/vector-PDF requirements. Source generation here does not establish completion or conformance.


### Table background phase generation

Split background commands from each node's remaining paint commands. The renderer orders static table-part backgrounds sharing the table stacking context as column groups, columns, row groups, rows, then cells, before cell content. A rowspan background consequently stays above later row backgrounds. Positioned parts and parts establishing their own opacity/transform context remain atomic in the shared stacking planner. Added raster regression sources for a spanning cell over a later row and a cell opacity context with descendant content. Common paint ordering and native hit-test APIs are unchanged. No tests executed. Full transformed table-part layering and collapsed border intersections remain separate requirements.


### Orthogonal table-cell constraints

Generated physical intrinsic sizing branches for cells orthogonal to the table and synthesized orthogonal baseline fallback. Cell vertical-align uses the containing table's axes, including top/middle/bottom placement from the actual occupied content interval. Continuation caches also preserve the table inline dimension rather than the cell's own writing-mode dimension. Added an orthogonal horizontal cell in a vertical-rl table regression verifying native horizontal glyph orientation and all three block-axis alignments. Explicit dimensions and bounded orthogonal fallback use native shaping; iterative cyclic auto-size negotiation and replaced-element baseline rules remain open. All source only.


### Break-boundary precedence and spanning geometry

Deferred break-after until the next flow box so its adjoining break-before can take precedence; trailing break-after no longer manufactures an empty page. The variable-page parity path likewise recognizes overriding page/always requests. Gap insertion moves ancestors beginning at that boundary instead of growing a leading empty fragment, and grows spanning-cell boxes/client rectangles across intervening row breaks. Generated regressions for conflicting parity breaks and rowspan border geometry across a row page gap. No execution; advanced fragmentation constraints remain in scope.


### Vertical intrinsic widths and PDF line sequence

Source review found the shared intrinsic-source path still used horizontal Parley advances. It now derives vertical min/max inline constraints from actual native vertical shaping, including mandatory/soft breaks and the min-content distinction between overflow-wrap break-word and anywhere. Added a glyph-metric regression. PDF searchable text now orders vertical document columns in the document block direction without reversing the logical text inside each native fragment; generated a column-order regression. Mixed page-margin/body reading order and exact selection quadrilaterals remain open. No tests run.


### Pinned writing-mode obligation correction

Inspected pinned `css/css-writing-modes/table-cell-valign-003.html`, `table-cell-align-003.html`, and `table-progression-vrl-001.html`. The orthogonal vertical-align entry above overstated table-axis alignment: alignment must use the cell's own line-over/under edges. Corrected that mapping and replaced the regression with a vertical-lr cell in a horizontal table. Grid progression, however, uses the table writing mode/direction: generated private formatting overrides for rows/row groups/columns without changing exposed computed styles or cell text orientation. Added the pinned progression override regression. No execution.


### Native linear gradient backgrounds for pinned table reftests

Pinned table-cell alignment reftests require linear-gradient red/transparent backgrounds. Generated CSS linear/repeating-linear gradient parsing, angle/corner geometry, absolute/percentage/auto/double stop placement, hard stops, and premultiplied sRGB interpolation in the CPU renderer. Background layers paint back-to-front through native projection, clipping and rounded bounds. Added hard-stop, repeating-stop, transparency and display-list raster regression sources. No dependency installation or execution. Radial/conic gradients, non-default background sizing/tiling/position and alternative interpolation spaces remain open; complex unsupported gradient syntax retains diagnostics.


### Auto tables and caption constraints

Generated auto table inline sizing bounded by actual shared column min/max constraints instead of filling the containing block. Caption intrinsic/explicit inline constraints and table minimum inline size participate in that solution. Caption-side top/bottom determines visual flow placement independently of DOM child order. Added source regressions for intrinsic auto width, caption ordering, and explicit caption width. Complex caption margins and cyclic percentage sizing remain open; no execution.


## Static SVG production renderer generation

Verified pinned resvg/usvg 0.45.1 Options, image resolver and rendering APIs from upstream source, then added source-only dependencies (resvg text/raster-images, roxmltree, url). Generated static SVG parsing and native vector rasterization at projected destination density, including paths, paints/gradients, clips, masks/filters and text supported by resvg. The shared compositor receives correctly unpremultiplied RGBA and retains CSS object-fit/position, transforms, clipping and opacity. Captured fonts and captured/data images are the only resource inputs; the default filesystem resolver is replaced. XML/expanded-byte/reference/font/surface bounds are explicit. Missing references and foreignObject produce errors rather than silently advertising complete output.

Native inline SVG capture is coordinated with root using existing image_resource + image/svg+xml bytes and absolute base URL; no common schema added. Regression sources cover vector gradients/clip/opacity at enlarged resolution, captured relative PNG and SVG text, and rejected filesystem references. No dependency installation, lockfile generation, build or test execution. Remaining SVG obligations include native foreignObject composition, external use references, custom font-face family aliases, and full SVG DOM geometry; PDF vector emission remains a distinct next requirement.

## Vector PDF and SVG source continuation (2026-09-16)

Production `render_print_pdf` now assembles independent vector page forms with globally renumbered PDF object references. Native display-list rectangles, rounded paths, simple borders, affine projection/clip geometry, atomic opacity, nonrepeating linear gradients, captured SVG and actual shaped glyph outlines remain vector. The outline exporter preserves quadratic/cubic segments, native font collection/variation instance, synthetic skew/bold, run color and physical glyph orientation. It does not reshape HTML strings. The existing invisible ToUnicode text layer remains attached to actual unclipped native fragment lines and survives multipage assembly; glyph visuals and semantic text are separate.

Pinned source dependencies: resvg/usvg 0.45.1, svg2pdf 0.13.0 and pdf-writer 0.12.1. Dependency installation and lockfile resolution remain deferred by the generation gate. Production PNG rendering continues to use native display-list rasterization.

Localized primitive raster fallback preserves perspective, repeating gradients (including SVG spreadMethod repeat/reflect), complex CSS border styles/radii, explicit pixelated image sampling and legacy unshaped text. SVG filters use the converter's localized group rasterization. Primitive fallback keeps original transforms/clips/opacity and is bounded by 128 fallbacks plus aggregate pixel budget per page; the reference compositor uses a bounded transparent page surface internally, then crops the encoded fallback image to its nontransparent pixel bounds. Generated XML is bounded to 16 MiB, intermediate SVG layer surfaces are checked before conversion/rasterization, and aggregate page chunks/final PDF are bounded to 256 MiB.

Added unrun regression sources for vector-only shape/native-glyph PDF forms, unsupported primitive raster fallback and pixel bounds, multipage form-reference renumbering with Unicode supplementary-plane text, native variation outline equality, and oversized SVG filter-layer rejection. Source review corrected run-color type handling, transparent synthetic glyph overlap, and bounded vector font sizes.

Remaining limitations: semantic selection rectangles for rotated/partially clipped lines still need exact per-cluster PDF geometry; clipped partial lines are deliberately omitted from semantic text rather than duplicated. SVG `foreignObject` requires native composition, custom CSS font-family aliases need captured alias metadata, and SVG2 features beyond the pinned resvg implementation remain unimplemented. Native inline SVG computed-style capture is owned by the root agent. Full V sizing/fragmentation gaps above remain active. No build, test, formatter, installer, renderer execution or conformance run was performed.

## Native SVG foreignObject composition source (2026-09-16)

Added `SvgForeignObjectSnapshot` and defaulted `PaintSnapshot.svg_foreign_objects`. Each record references an owning image resource, capture-generated `data-oryn-foreign-object` token (author `id` is preserved), local viewport and independently formatted native HTML paint nodes. Shared image/font indexes remain owner-global.

Public `compute_subtree_with_styles(input, root, viewport, styles, limits)` projects the rendered flat subtree, retains full-document matched styles and inherited language, establishes a local origin/viewport, and removes only wrapper effects owned by outer SVG. Child styles, native shaping, descendant transforms and local scroll are retained. Root owns capture production and external SVG native HTML parsing.

Renderer composition replaces each captured foreignObject with native HTML paint in an SVG-positioned image, preserving outer SVG transform/opacity and author identity. Captured nested SVG images now recursively use the same resource-only parser instead of upstream's image-disabled nested SVG path. Composition is bounded to depth 8, 256 objects per SVG, 100000 aggregate HTML nodes, 16 million aggregate HTML surface pixels and 16 MiB composed XML. Missing layout, duplicate capture identity, recursive resource cycles and incomplete native subtree paint return explicit errors. Regression sources cover actual native HTML position/opacity, author-id preservation, missing/recursive capture errors and independently laid-out percentage widths with full-document selector context. No tests executed.

Remaining foreignObject limitations: native producer integration/external SVG HTML parsing must be supplied by root; local overflow-visible paint extending outside the declared viewport needs expanded surface bounds; capture pixel density currently follows the captured viewport DPR; PDF semantic extraction of foreignObject HTML remains separate from its raster visual composition. Full V gaps remain active.

ForeignObject follow-on: wrapper computed overflow is now retained. A shared native paint-bound walk includes actual shaped glyph metrics, transforms and clips; expanded local composition retains negative/positive visible overflow, while resource bounds reject unbounded perspective surfaces. Foreign HTML composition now embeds canonical native SVG paths through the same exporter as vector PDF, rather than a fixed-DPR PNG. This keeps supported text/shapes sharp at destination density; unsupported primitive effects retain bounded raster fallback. Added overflow-visible and vector-PDF/high-density regression sources. These supersede the previous always-raster/DPR limitations for supported native primitives. Zero-area foreignObject is omitted before metadata lookup. Native producer is now integrated by root; external SVG HTML and PDF semantic mapping remain active follow-ons. All source remains unrun.

ForeignObject semantic PDF continuation: the native vector exporter assigns deterministic IDs to existing first nonempty glyph paths (no synthetic/invisible paint). Semantic extraction finds those actual paths in the parsed SVG tree and removes the native glyph-local transform from usvg's final absolute transform. Native HTML line rectangles therefore follow the same SVG viewBox/ancestor transforms as visible ink. Native clips, page clips, opacity and whole-line ownership are checked before adding ToUnicode text; total extracted bytes/spans remain bounded. Added source regression for SVG viewBox scaling of actual native glyph geometry. Deep SVG-inside-HTML-inside-SVG semantic traversal, nonrectangular SVG mask/clip coverage, partially clipped line extraction and exact rotated selection quads remain open; those lines are omitted where exact coverage is unavailable. No source has been built or tested.

Semantic mapping refinement: source review confirmed pinned usvg preserves named SVG `g` groups. The final implementation identifies the actual foreignObject viewport group, combines its absolute transform with native projected line geometry, and recursively processes SVG images inside native HTML. The earlier first-glyph marker approach was removed; no extra glyph serialization or marker paint remains. Added nested SVG-in-HTML extraction regression asserting one native line. Nonrectangular SVG masks/clips and partially clipped lines remain conservative omissions; exact rotated selection quads remain unimplemented. No execution occurred.

Source review hardened allocation before expanded offscreen paint: native/legacy glyph paths now inspect actual font metrics before allocating bitmaps, and regular raster-image decoding checks encoded bytes and header dimensions before decode with an explicit decoder allocation limit. Added unrun oversized-glyph and CRC-valid oversized-PNG-header regression sources. These bounds apply to screenshots and foreignObject/vector fallback as well as print.

## Native cluster provenance and clipped PDF text (2026-09-16)

Added defaulted `PaintTextFragment.clusters: Vec<PaintTextCluster>` carrying exact fragment-local UTF8 byte ranges, physical preprojection rectangles and bidi direction. Horizontal Parley and vertical Harfrust paths retain explicit source ranges for every shaped line, including resumed paragraph/vertical line shaping. Page-margin content and native positioning/table/page-gap relocations move the same cluster geometry as their fragments. This is source provenance from shaping, not reconstruction from glyph counts.

PDF native text extraction now falls back from whole-line ownership to fully visible native clusters, merging adjacent logical byte ranges while retaining Unicode scalar boundaries and ligature/grapheme clusters. Legacy fragments without cluster provenance keep the conservative whole-line behavior. The native HTML stage of foreignObject extraction uses this same path. Added unrun regressions for real horizontal/vertical shaped UTF8 cluster ranges and clipped emoji/ligature fragments across translated page regions. Outer SVG clipping/masks and exact rotated PDF selection quads remain separate geometry requirements. All builds/tests remain deferred.

PDF selection geometry follow-on: internal semantic records now retain transformed line quadrilaterals. Vector PDF text matrices preserve affine rotation/skew and writing-mode/direction axes, including recursive foreignObject SVG transforms, rather than always writing horizontal text into axis-aligned bounding boxes. Added rotated text-matrix and vertical inline-progression regression sources. This improves line selection geometry; per-character advance differences inside a merged line and genuinely projective (non-parallelogram) selection quads remain limitations. Wire metadata is unchanged beyond the previously approved native clusters. No tests were run.

## Broad missing paint algorithms (2026-09-16)

Checklist audit identified wholly absent multicolumn/ruby/text-combine formatting, text-decoration, box-shadow and radial/conic CSS gradient paths. V-01/V-04/V-05 also cross package ownership and remain with root. Source existence and this audit do not certify V.

Generated native radial/conic/repeating gradient parsing and sampling, ellipse/circle extents, angle/center geometry, stop fixup and premultiplied color interpolation, integrated into mixed background layers and shared projection/clipping. Pinned source inspected: `css/css-images/conic-gradient-angle.html`, `repeating-conic-gradient.html`. Added geometry/hard-stop/repeat/invalid-size regressions. Advanced color spaces, transition hints, CSS math/relative units not normalized by Stylo, and non-auto background sizing remain open.

Generated box-shadow parsing, front-to-back lists, inset/outset ordering around backgrounds/borders, spread-adjusted rounded silhouettes, 4-sigma separable Gaussian convolution and transformed sampling. Masks, caches and convolution work are bounded before allocation. Outer shadows exclude the border-box interior; inner shadows clip to the padding box. Offscreen boxes with shadows remain eligible for paint. Pinned `css/css-backgrounds/box-shadow-multiple-001.html` informed source regressions; additional inset/outset/blur/bounds regressions generated. PDF/foreignObject vector output uses localized native raster fallback for these effects. Text-decoration and broad formatting paths are next; no build/test execution occurred.

## Decoration, combined text and column source generation (2026-09-16)

Added approved defaulted `PaintGlyphRun.decorations`, `vertical`, `scale_x` (default one) and `decoration_rect` (fragment-local physical run box). Decorating ancestor values propagate independently of CSS inheritance through native inline shaping; descendant `none` does not erase them, atomic/out-of-flow boundaries stop propagation, and display-contents ancestors remain represented. Public computed styles remain unchanged. Horizontal and vertical glyph runs retain native decoration bounds.

The raster painter uses captured font variation metrics for underline/strikeout, explicit owner thickness/offset/color, solid/double/dashed/dotted/wavy coverage and actual glyph bitmap intersection for skip-ink. Underlines/overlines paint before the line's glyphs; strikeout paints after. Identical fallback-run decoration surfaces are deduplicated and their skip-ink gaps combined. Transformed ink bounds and clips use the same physical geometry. Regression source references pinned `text-decoration-style-multiple.html` and `text-decoration-thickness-overline-001.html`; native shaping and font-metric tests were added. Decorated text currently uses localized raster fallback in vector export. Owner-font fallback selection, pattern continuity across style boundaries, script-specific automatic skip-ink and all underline-position/language combinations still require source refinement and validation.

Text-combine now performs actual horizontal native shaping across fallback runs before fitting each composition into a one-em vertical cluster. Wide compositions compress only along the glyph horizontal axis; narrow compositions remain uncompressed. Letter/word spacing and text indent do not change the composition. Glyph raster sampling, bounds and vector PDF/SVG emission apply the same validated scale. The combined cluster retains the complete original byte range for caret/semantic provenance; line breaking treats it atomically. Pinned compression-001, layout-rules-001, letter-spacing-001, inherit-all-001 and decorations-001 sources were read. Regression sources cover one-em advance, native upright glyph runs, narrow/wide scaling, ignored spacing and sideways exclusion. Cross-descendant style grouping and digit-only value support remain explicit follow-ons.

Initial multicol formatting source resolves count/width/gap, shapes text at the actual used column inline size, balances native line boxes and maps fixed-height auto-fill overflow columns, RTL progression, source clusters and br caret geometry before physical writing-mode projection. Pinned `multicol-gap-large-001.xht` was inspected. This currently covers direct paragraph content only: child-block fragmentation, spanners, column rules, break propagation and widow/orphan balancing remain active implementation work. Ruby remains unimplemented. These are not completion claims.

PDF semantic text now emits matrices at each native cluster's actual advance/position instead of distributing a merged line evenly across characters. Affine rotation/skew and vertical direction remain preserved. Genuinely projective cluster quads require an affine fit because PDF text matrices are affine; exact projective selection and arbitrary SVG mask/clip semantic coverage remain open. No builds, tests, formatter, dependency installation, renderer execution or conformance runs occurred during this source generation.

Inline-foundation continuation: source audit confirmed the native producer exports direct Text/Element records and does not flatten general inline descendants. Added a formatting-tree paragraph/block participant collector, zero-byte atomic anchors (no fabricated text characters), Parley inline-box insertion/placement and paragraph-boundary rebasing, vertical atomic placement, and private native run owner/byte provenance. Cluster positions now include actual inline-box advances. A single shared paragraph containing ordinary inline descendants and atomic/replaced boxes is integrated into `layout_node`: atomics are laid out independently, placed from native line output, and inline owner fragments/geometry are routed without re-shaping. Relative inline translations and source-cluster projection ownership follow the actual inline owner. Native mixed-text/replaced/atomic-only regression source was added.

This foundation remains incomplete: anonymous paragraphs around block-in-inline boundaries, nonreplaced inline edge/strut/baseline details, floats interacting with line breaking, full bidi atomic ordering and ruby layout are still being generated. The collector can represent block splits, but their layout flow integration is pending. Existing full multicol block fragmentation/spanner/rule and text-emphasis requirements remain open. No source execution or conformance claim follows from these additions.

Further inline source now integrates paragraph/block splits through a continuous logical flow and places independently formatted blocks from native placeholders. Nonreplaced inline boundary anchors contribute padding/border/margin advances while preserving whitespace collapse across ordinary inline boundaries; atomic anchors interrupt collapse. Inline background/border fragments retain slice/clone edge widths, shared projection/clips, and gap-excluding hit regions through the approved defaulted `PaintProjection.fragment_boxes`. Text, replaced images and collapsed-border segments remain single-owned. Native inline-block text baselines and below-baseline descent expansion are generated for horizontal flow. Tests for anonymous paragraph order, inline fragment edges and fragmented hit regions were added.

Initial native ruby source pairs explicit/implicit bases with annotations (including annotation containers), shapes both with captured fonts, preserves text owners/UTF16 geometry, places over/under annotation levels and supplies the base baseline to inline placement. Pinned `ruby-annotation-pairing-001.html` and bundled ruby UA rules were inspected; first-base pairing regression source was added. Centered placement is implemented; space-around/space-between distribution, overhang, intra-ruby line breaking, inter-character annotations, nested atomic annotation content and full bidi/vertical baseline cases remain active requirements. Mixed block multicol flow is shaped at column width but still needs full fragment distribution, spanners, rules and break propagation. No tests/builds were run.


## Multicol fragmentation and generated emphasis continuation (2026-09-16)

The earlier direct-text-only multicol limitation is superseded by new source integration: block-only and mixed anonymous flow format at the used column width, then select native line/block boundaries. The break planner accounts for forced column breaks, monolithic boxes, avoid-column constraints, orphan/widow limits, fixed-height overflow and independently balanced sets. Actual element client rectangles, source/paint clusters, br boxes and deferred static anchors move with the same physical fragment map. Vertical modes use reversible logical/physical axes. Fragment box slice borders and radii now distinguish block-axis fragmentation from inline line splitting.

Full-width spanning blocks separate column sets. Spanners nested through ordinary blocks reach their nearest multicol context; independent formatting contexts and nested multicol/spanner boundaries stop propagation. Captured source is laid out at the full span inline size before placement, not enlarged after shaping. Rules use resolved shared border-segment paint in each set, with no rule across a spanner. Pinned sources inspected include `multicol-span-all-007.html` and `multicol-rule-dashed-000.xht`. Added native block movement, split text/box geometry, forced breaks, spanner width/rule separation, nested formatting-context barrier, orphan/widow and vertical-axis regression sources. Ancestor decoration behavior around nested spanners, all adjoining margin-collapse cases, floats within fragmented line layout, nested multicol/page interactions, complete break propagation and continuous sliced background positioning remain active refinements; this is not full multicol certification.

Ruby source now also distributes native typographic clusters for start/center/space-between/space-around while preserving native glyph byte provenance and source geometry. This supersedes the earlier center-only note. Overhang, inter-character placement, intra-ruby wrapping, nested atomic annotations and complete bidi/baseline behavior remain open.

Generated text-emphasis source resolves named/open/filled/custom marks, skips spacing/control text, shapes annotation glyphs using captured fonts at half size, uses native baseline metrics for horizontal placement and physical sides for vertical placement, and extends line spacing for actual mark extent. Annotation runs have empty semantic provenance; text, UTF16 clusters and accessibility content never acquire mark characters. They use normal renderer/vector glyph paths and require no wire field. Source regressions cover open-dot, default vertical sesame, custom string, under/right placement and absence of mark text in paint-fragment semantic content. Pinned `text-emphasis-style-012.html` and `text-emphasis-position-under-right-001.xht` were inspected. Complex multi-character native ligature clusters, ruby/emphasis collision stacking, full skip-property behavior and custom CSS-string escapes remain refinement work.

No tests, builds, formatters, dependency generation, runtime rendering or conformance execution occurred. Full V obligations and the earlier print/SVG/projective text gaps remain open; source generation continues before the global execution gate.


## Computed-style and background-resource integration (2026-09-16)

Source audit found that the fixed Stylo export list omitted many new formatting and paint properties. Added the actual multicol/ruby/text-combine/decorations/emphasis/shadow/radius/background longhands and a cascade-export regression. Earlier implementation notes did not establish that this missing producer link was present; this source correction is essential to those paths receiving authored computed values.

Background layers now resolve each layer's origin/clip, native image or gradient size, contain/cover, percentage/length position, repeat axes, round and space. A single tile representation performs repeated sampling without allocating commands per tile. Border/content rounded clips and background-color's bottom-layer clip use resolved inset radii. Ordinary single-covering linear gradients retain native vector primitives; other tiled/effect backgrounds use localized raster fallback in PDF. Pinned `background-repeat-space-1a.html` and `background-position-calc-minmax-001.html` informed new source regressions, including negative percentage basis, repetition gaps and requested-URL alias lookup.

With root approval, narrow native paint capture changes queue applied background URLs through the existing bounded image resource path for document and foreignObject trees, including print's fresh document layout. They do not assign those backgrounds as replaced content images. Added bounded percent/base64 data-image support through the existing native Fetch data processor. `PaintImageResource.url_aliases` is defaulted and generated only from a successful original request redirect; the final URL stays the SVG relative-resource base. Alias counts/bytes are bounded in consumers, and all existing struct literals were updated. Native source regressions cover shared page/foreignObject backgrounds and data-image bounds. Renderer code never fetches resources.

Remaining background refinements: local attachment/scrolling and transformed fixed attachment, full CSS image-set/cross-fade selection, background-clip:text, sliced-fragment continuous painting, some mixed math/three-value positions and advanced color interpolation. Native paint fetch still requires the exact-realm controlled-service-worker subresource bridge identified by the worker owner; it currently uses the existing broker path. Generated @page margin-box-only background resources and external SVG/HTML integration remain coordinated producer dependencies. No code has been built or tested.


Native float/inline source continuation uses Parley 0.11 custom out-of-flow yields and mutable line constraints with Taffy's existing FloatContext. Floated participants retain zero-byte source anchors; their actual independently laid-out dimensions determine exclusion bands. Text line breaking and alignment consume the resulting native bands. Float placement after preceding text can defer to the actual native line boundary, and max-height yields retry against an exclusion band fitting the actual measured line. Float contexts persist through preserved paragraph separators; float-only BFC height and auto-width shrink-to-fit are integrated. Added beside/below text, opposing float/BFC height and preserved-newline regression source. General cross-block/BFC exclusion propagation, floats within multicol/page fragmentation, vertical float semantics and all clearance/margin collapse cases remain active work. Existing block/flex/table fallback paths remain until those scope boundaries are integrated. No build/test validation has occurred.

Background image-set selection by captured supported MIME and device density, including density-adjusted intrinsic size, now supersedes the earlier wholly missing image-set note. Custom emphasis strings now use the CSS parser's escape decoding. These are source refinements, not conformance claims.


Vector export now emits native SVG/PDF tiling patterns for repeated/gapped linear-gradient backgrounds and normally sampled raster-image backgrounds, preserving no-repeat axes and space limits. Only effects not represented by that path continue through localized raster fallback. Added pattern/no-rasterization regression source; it is unrun.


Native print capture now also queues background resources referenced only by generated @page margin boxes before completing the shared image snapshot. Added an unrun generated-margin-background regression. Ruby annotation decoration propagation now stops at ruby-text containers, with a source regression; inline-table and ruby internal display roles are recognized as inline-level for sizing and fragment edge semantics.

## Native scroll geometry and local backgrounds continuation

Generated `PaintScrollGeometry` through `PaintProjection.scroll` (serde default), with exact applied finite offsets and local pre-world-transform scrollport/scrollable rectangles. Layout now collects subtree overflow before applying scroll translation, clips contributions at intervening overflow containers, excludes viewport-fixed descendants, propagates boxless wrapper descendants, and unions transformed/untransformed descendant extents. Native background-attachment:local consumes these dimensions and offsets, clips ink to the stationary scrollport, and retains the existing projected paint plane. New source regressions cover nested clipping, local metrics unaffected by outer scale/scroll, nonfinite overflow rejection, and local background tile placement. Three-component side/offset background-position parsing and regression sources are also generated. All projection literal initializers located in source were updated, including the three JS test literals.

Still open in this work: precise perspective/3D-chain overflow rather than conservative transformed bounding unions; containing-block propagation for positioned descendants escaping an intermediate clip; all negative writing-axis overflow cases; trailing scroll-area padding and scrollbar reservation; transformed fixed background attachment; continuous sliced-box background painting. Root owns integration of native metrics into CSSOM/driver scroll consumers. No build, test, formatter, renderer, or conformance execution was performed; this is not completion or correctness certification.

Follow-on source refinement: scroll overflow now routes absolute/fixed descendants to their actual nearest positioned/transformed/contained containing block, bypassing unrelated DOM clipping wrappers; viewport-fixed contributions remain outside element scroll overflow. The paint clipping/projection consumer still requires its matching containing-block escape integration, so this source change alone does not close positioned clipping across the pipeline.

The corresponding paint projection follow-on is now generated: absolute/fixed descendants with an element containing block inherit that block's child projection/clip state directly, instead of accumulating unrelated intermediate DOM clipping and scroll translations. The native scroll regression additionally covers positioned overflow escaping a hidden wrapper and confirms only the actual containing scrollport clips the descendant. Initial-containing-block/root overflow special cases, perspective-chain precision and all negative-axis cases still require further source work. Root reports native CSSOM and wheel/programmatic scroll consumers now use this contract; this has not been executed or validated.

## Classic scrollbars, padding, transformed attachments, and perspective continuation

Generated the explicit host contract `BrowserConfig.native_scrollbar_width` → `LayoutInput.scrollbar_width` (finite 0..128 CSS px, default zero/overlay). Classic gutters are reserved in actual native formatting before line shaping/child layout; auto overflow triggers bounded full-layout stabilization, with an explicit diagnostic on exhaustion. `scrollbar-width:none/thin`, `scrollbar-gutter:stable/both-edges`, native scrollport clipping, and outer used-size preservation have regression sources. Root supplied live provider configuration/cache invalidation; worker owner supplied serialized config roundtrip. No runtime validation performed.

Scrollable content now includes native trailing padding in physical progression axes, with horizontal RTL and vertical/sideways negative axes distinguished. Fixed backgrounds inside computed transformed ancestry now behave as scroll backgrounds, including identity transforms; source regression follows pinned `background-attachment-fixed-inside-transform-1.html`.

Overflow geometry now retains homogeneous depth through ancestor perspective, reduces flattened planes to convex hulls instead of repeatedly projecting bounding rectangles, and clips hull edges at scrollport planes. The collector releases consumed child vertices and bounds retained vertex counts. Perspective-horizon crossings and geometry budget exhaustion produce explicit diagnostics rather than claiming exact finite overflow. Source regressions cover translateZ under parent perspective and clipping a diamond polygon, plus native layout integration.

Remaining: perspective-horizon extent policy, full root/canvas fixed-background rules and initial-containing-block behavior, native classic scrollbar chrome painting, complete intrinsic/table/vertical interactions with gutter reservation, and validation of all generated source. All builds/tests/conformance remain unrun under the user gate.

Follow-on paint sources: deterministic classic scrollbar tracks/thumbs now paint inside native reserved gutters with `scrollbar-color`, proportional thumb extent and signed offset placement; zero-gutter overlay mode emits no persistent chrome. Canvas background selection uses actual HTML root/body entries, propagates only from a boxed body when the root background is transparent/none and containment permits, suppresses duplicate body background painting, and fills the viewport beyond short root/body boxes. Root fixed attachment remains viewport-positioned even when root transforms exist; descendant transformed attachment keeps scroll behavior. Native HTML/body background resource capture now includes offscreen entries because their backgrounds may paint the canvas. New source regressions cover canvas color extent, propagation exclusions, identity-transform attachment, and signed scrollbar thumb placement.

Remaining generation/integration risks: classic scrollbar pointer dragging/track clicks, full canvas root-transform and document scroll-origin edge cases, horizon overflow policy, nested sizing interactions, and the previously documented broader V requirements. These source additions are not runtime or conformance evidence.

Shared scrollbar geometry is now extracted to `oryn_common::scrollbar::geometry`, consumed by renderer and available to native pointer actions. The typed result includes track/thumb rectangles, signed min/max/current offsets, page step, and finite-checked inverse thumb-origin mapping. Root is integrating pointer track/drag interaction. Root canvas paint coverage now inverse-projects viewport corners into the native root plane, covering transformed canvas backgrounds without treating the viewport rectangle as local geometry; horizon failures remain explicit diagnostics. Initial-containing-block positioned elements now escape unrelated DOM scroll/clip state, with a corresponding source regression.

## Explicit viewport scrolling continuation

Added `PaintViewportScroll { scroll, style }` through optional `LayoutOutput.viewport_scroll` and `PaintSnapshot.viewport_scroll`. The viewport has an identity coordinate plane and effective HTML/body overflow policy; it does not borrow a transformed root element's scrollbar surface. Native document formatting reserves viewport gutters before root sizing and updates initial/fixed containing-block dimensions accordingly. The bounded cross-axis reflow loop now handles viewport and element gutters, preserves authored CSSOM overflow values, includes root transformed/untransformed overflow, and emits signed viewport metrics. Renderer paints viewport scrollbar chrome after document stacking. Print-page and foreignObject local snapshot clones explicitly clear unrelated viewport chrome. Root owns native snapshot and BindingLayout transport/pointer routing; integration regressions for document reflow, body overflow propagation, and identity viewport chrome are generated.

Viewport fixed-position clipping at reserved gutters, child-frame viewport scrollbar composition, horizon crossings, scrollbar autofocus/track autorepeat, and complete intrinsic/table/vertical interactions still need integration/source review. No execution or certification has occurred.

Viewport clipping follow-on: document content (including viewport-fixed/initial-containing-block descendants) now receives an identity-plane scrollport clip that excludes reserved gutters, while canvas backgrounds retain full canvas coverage. Non-document subtree/foreignObject overflow-visible remains unconstrained by this document viewport rule. Intrinsic width and Taffy native text measurements now include/subtract the same reserved gutter contributions; post-formatting min/max width and height constraints operate on authored CSS sizes before reserving gutter space.

Sticky positioning now consumes the same authoritative scroll metrics and viewport scrollport used by paint/CSSOM, including reserved classic gutters and exact applied offsets. Added a source regression for an end-inset sticky box against a narrowed classic scrollport; no tests run.

Pinned perspective source audit added `overflow-perspective-001.html` through `-003.html` coverage: auto/clip/hidden containers with translated Z-depth must retain identical perspective-projected child geometry and avoid spurious overflow. The new native regression asserts the exact projected coordinates derived from the pinned dimensions. It has not been run.

## Nested frame composition and homogeneous clipping source (2026-09-16)

Generated nested `PaintFrameSnapshot` documents with independent resource tables,
fonts, viewport scroll geometry and bounded live native realm capture. Frame
content is captured at its untransformed content viewport and embedded through
the owner projection. Raster, vector PDF and semantic text paths now recurse
through the captured document tree. Child viewport chrome stays within its own
content viewport; print frame layout uses print media and omits viewport chrome.
The renderer core now lives in `crates/oryn-render-core` after the coordinated
native capture dependency split.

Scoped frame resource completion consumes exact native decoded raster, canvas
and retained SVG resources first. Missing URLs use parent-owned ImageStreamStart
with the selected native frame token/generation/epoch. The parent supplies CSP,
referrer, cookies and service interception; generic default child PageState
resource fetching is bypassed. Stream status, chunks, bytes, URL size, lifetime
and aggregate composition budgets are bounded. Unix in-process Browser uses the
same direct context service; standalone executables without a parent authority
fail missing-resource capture explicitly. New regressions cover nested source
documents, local viewport sizes, scrollbar gutters, repeated parent capture,
resource ownership validation and native vector composition.

Raster paint bounds clip homogeneous polygons against the finite target
viewport before dividing by w. Overflow clip edges are also intersected before
perspective division, retaining visible portions of camera-crossing polygons.
Frame raster density samples the visible camera region rather than a source
corner on the horizon. Regression sources cover fully behind-camera geometry
and a camera-crossing polygon bounded by the target viewport.

Still open in this area: truly unbounded perspective scroll extents and native
coordinate saturation semantics; infinite root canvas projection crossing its
horizon; iframe capture inside foreignObject-local ownership; external SVG HTML
producer integration. Existing diagnostics remain for unsupported unbounded
cases. This is source generation only: no build, test, formatter, dependency
installation, WPT or conformance verification has run during this work.

Follow-on source closes the foreignObject-local iframe ownership lookup: native
frame capture selects the iframe from the independently formatted HTML nodes;
renderer validation, local SVG composition and print retention preserve that
owner without projecting it into top-level DOM geometry. A native source
regression covers the 60×40 child viewport inside an offset SVG foreignObject.
Aggregate node budgets now count foreignObject nodes as well. No tests run.

The root canvas horizon fallback is now replaced in source by a dedicated
renderer Canvas primitive. It keeps finite native tile placement and samples
color, gradient and captured-image layers analytically across the finite output
viewport using inverse camera projection. Behind-camera regions remain empty;
no fabricated enormous CSS rectangle is required. PDF uses the existing bounded
localized raster fallback for this projective infinite canvas, preserving vector
output for unrelated paint. A generated regression covers repeating gradient
canvas beyond the root's finite box and camera-plane rejection. SVG canvas tile
rasterization remains bounded at output scale; adaptive projective SVG detail
and truly unbounded native scroll-extent saturation remain follow-on concerns.
The child frame screen snapshot now installs the immutable exact-lifetime drag
capture inventory before restoring its parent realm; print snapshots are excluded.

## Native scroll coordinate domain and cancellation source

`oryn-common::layout_domain` now documents a conservative signed scroll domain
of ±2^23 CSS pixels. Taffy/Parley use binary32 coordinates; the full signed span
of 2^24 preserves consecutive integer CSS pixels and lies within Stylo's
app_units 0.7.8 envelope ((2^30)-1)/60. Homogeneous edges are clipped before
perspective division. Native projected rectangles and overflow use this domain;
mathematical infinity and out-of-domain saturation still emit explicit layout
diagnostics. Native layout scroll offsets clamp to the resulting signed range.
Generated regressions connect saturated layout metrics to scrollbar thumb/range
geometry and compare native hit-coordinate inversion to raster camera rejection.
This is a native representation policy, not a finite-conformance assertion for
an unbounded mathematical plane.

Selected frame resource requests now check the native action PromptController
and absolute deadline before stream start, after start, before each bounded
read and on completion; Abort cleanup remains active on cancellation. The
coordinated caller installs/restores this token around action paint planning.
Individual service calls still obey the context transport's blocking bound.
Decoded SVG resources now preserve the final response URL plus original-request
aliases, so relative SVG dependencies use the authenticated redirect destination
base. All new source and regression work remains unbuilt and unrun.

Sliced backgrounds now reconstruct their hypothetical unfragmented positioning
box along native inline/block axes and independently clip each actual fragment.
This preserves image/gradient continuity across line and column breaks for
`box-decoration-break:slice`, while `clone` retains independent positioning.
Generated raster regressions compare slice/clone and vertical reverse origins.
Renderer-core codec features now explicitly match the existing native JS image
decoder (BMP/PNG/JPEG/GIF/WebP), and image-set MIME admission uses those codecs.
Generated standalone codec rendering regressions avoid relying on unrelated
workspace feature unification. No dependency installation or test run occurred.

## Background masks, cross-fade and shared float formatting source

Background clipping now uses native shaped glyph alpha masks, including
transformed descendant text, rather than bounding-box silhouettes. Transparent
foreground text still masks the background; annotation-only runs are excluded.
Fragmented backgrounds retain their per-fragment boundary. PDF uses bounded
localized raster fallback for these masks while retaining ordinary semantic
text once. Cross-fade parsing/rendering preserves omitted percentage allocation,
normalization above 100%, premultiplied alpha and transparent unused weight;
nested captured images, gradients and colors share the bounded resource path.
The existing pinned Stylo vendor exposes its upstream cross-fade parser and
BackgroundClip::Text value for Servo, with provenance in ORYN-PATCHES.md.
Source regressions cover native cascade, weighted alpha, glyph overhang and
transformed masks. Mask inheritance/descendant opacity details and adaptive
projective SVG cross-fade density still need further source review.

TextLayout carries actual native line baselines. Table first-baseline lookup and
intrinsic block measurement now reuse the inline participant formatter instead
of summing separate inline children as block heights. Generated regression
sources exercise mixed font sizes and replaced inline baseline alignment.

Horizontal normal-flow blocks now share one float exclusion context until a
real independent formatting context boundary. Nested padding is mapped into
that context; clear uses the shared float bottom. Independent BFCs use Taffy's
native margin-aware slots and retry below exclusions when their border width
does not fit. Percentage widths remain based on the actual containing width;
only auto available width is reduced. Root/BFC auto height includes descendant
floats. Adjacent sibling block margins collapse in this custom float flow.
Generated regressions cover cross-block/nested float effects, return to full
line width, BFC isolation, clear and percentage-width slot fitting. Remaining
float work includes vertical/multicol exclusions, full parent/empty-block margin
collapse and more complex future-band BFC intersections. These are explicit
remaining algorithms, not completed conformance claims. All work here is source
and regression generation only; no build or test has run.

The BFC slot follow-on now checks the entire resulting border rectangle against
both native float lists, including exclusions beginning below its first line.
It retries at the next intersecting bottom and uses finite float-band progress;
queued out-of-flow placements from discarded trials are removed. A generated
regression covers a taller BFC crossing a later right-side exclusion. This
replaces the future-band intersection gap listed immediately above; vertical
and multicol float integration and full margin collapse remain open.

Native line exclusion now bounds Parley's first break at the next float start
and retries using the full shaped line height across all intersecting float
bands. Regression source checks the union of left/right exclusions for a tall
line. BFC auto margins resolve inside the remaining slot rather than the full
containing width, while percentage lengths retain their correct basis.
Fixed table sizing now tracks definite columns separately from numeric width:
an explicit zero-width column or first-row cell is no longer confused with an
unspecified column, and later-row widths cannot allocate fixed columns. Source
regressions cover both zero-width sources and later-row conflicting widths.
No validation execution has occurred.

Sibling/empty-block margin collapsing now retains separate positive and negative
extrema through zero-height block chains, in both ordinary block layout and the
shared float formatter. A generated document regression compares the same
positive/negative chain with and without a float context. Parent/first-child and
parent/last-child collapse, clearance interaction and vertical-axis collapse
remain distinct follow-on work; this does not certify complete margin behavior.

## Vertical and column float source follow-on

Vertical/sideways formatting now uses the same logical float exclusions while
retaining native TTB/sideways shaping and contextual per-line reshaping. Floats
carry real logical dimensions, vertical clear and BFC slot sizing use those
axes, and float-only BFCs contribute to auto block size. Text-combine's nested
horizontal shaping isolates the enclosing float context. Generated document
regressions cover all four vertical/sideways modes and float-only sizing.

Column float placement now sees fragment boundaries before line breaking:
floats that fit a fresh column move there when the remaining column space is too
small, and native line height drives boundary retry. The geometry fragmenter
uses those selected boundaries. Balanced float columns replay native formatting
against candidate heights rather than moving an already-shaped narrow line into
a different exclusion context. Replay is bounded to 15 candidates per balance
and 128 total work units per TextEngine; exhaustion emits an explicit incomplete
layout diagnostic. Regression sources cover fixed and balanced columns, float
movement, line exclusion, and oversized-fragment progress. Spanner boundary
resets, oversized float continuation, clearance interactions, and full
parent-child margin collapse still require further source generation/review.
No build, tests, formatter, runtime or conformance run has occurred.

Parent/first-child and auto-height parent/last-child margin propagation now has
an explicit source module. It preserves positive/negative struts, recognizes
border/padding/formatting-context and inline-content barriers, and records which
child margin was absorbed so allocation and subtree translation count it once.
Sibling placement includes propagated descendant struts. Generated regressions
exercise both ordinary and float formatting, adjacent siblings, parent height,
and padding/flow-root barriers. Vertical margin propagation, clearance decisions
that depend on actual float interference, and intrinsic/constraint-sensitive
nested percentage bases remain source-review/refinement obligations.

SVG backgrounds, canvas layers and nested cross-fade SVG components now select
raster density from a conservative analytic Jacobian bound over the finite
visible output rectangle. This replaces sparse corner/center density sampling
that could miss projective magnification between samples. Existing pixel limits
reject over-budget detail; cross-fade tree nodes now count toward the same
bounded preparation budget as decoded image bytes. Source regressions cover
affine exactness, visible projective derivative bounds, and tree budget denial.
No rendering or tests were executed.

Margin propagation now treats `clear` as a barrier only while native float bands
can still interfere, and nested percentage margins use min/max-constrained
content widths with the correct box-sizing adjustment. Generated regression
source combines `clear:both` without floats, max-width and a percentage margin.
Side-specific/positive-margin clearance against live float bands and intrinsic
width-dependent bases remain narrower follow-ons.

Column-spanning blocks now clear the preceding native float context, establish
an independent formatting context, and restart fragment origins for following
column sets. Forced boundaries are generated relative to each set, with native
balance acceptance checked against resulting per-set column counts. A source
regression covers a nested float followed by a full-width span and fresh text.
Independent optimal balancing of differently sized spanner-separated float
sets remains a refinement beyond the shared bounded height target.

PDF semantic traversal now includes captured HTML frames even when the owner
snapshot has no SVG foreignObject resources. A native-font regression source
checks the child's text and transformed coordinates without any SVG dependency.

Vertical ordinary block flows now use the shared native flow formatter and
collapse sibling/empty-block margin struts along left-to-right or right-to-left
block progression. Orthogonal writing-mode boundaries stop those adjoining
struts. Generated document regressions verify both vertical directions and the
resulting auto block extent. Vertical parent-child edge propagation remains a
separate open refinement.

## Native visual viewport paint metadata

PaintSnapshot now has defaulted per-document visual_viewport geometry. Screen
capture obtains it directly from the exact native realm/generation/stream epoch;
author replacements of window.visualViewport are not consulted. Renderer
validation rejects invalid, stale-size or nonfinite transforms and maps layout
paint through native scale/pan while preserving the output surface dimensions
and fixed viewport chrome. Nested captured documents apply their own camera
inside their iframe viewport. Print recursively removes screen cameras;
foreignObject local layout and layout-coordinate drag crops clear the owning
camera while retaining nested frame semantics. Source regressions cover native
author-object poisoning, screenshot coordinates, nested cameras, invalid data,
print clearing and drag crop alignment. The native getter refreshes layout and
propagates failure before capture. Root is coordinating physical chrome/input
range conversion; screenshot consumers that
crop a full PNG must convert layout bounds through the native geometry.
No builds, tests or renderer executions have run.

Clearance barriers in margin propagation now consult the relevant native left
or right float list, rather than blocking all clear values when either side has
a live float. Source regression covers a right-clearing child beside only a
left float. Positive-margin clearance and full vertical parent/child propagation
remain separate refinements.

Empty inline line suppression and parent-margin barriers now share a predicate
over normalized text, in-flow atomic boxes and individual inline border,
padding and margin edges. Opposing nonzero margins do not disappear merely
because their advances sum to zero. Empty undecorated inline boxes do not add
a synthetic line; display:contents contributes no boundary advance or margin
barrier. Regression source covers whitespace normalization, edge cancellation,
vertical borders, and display:contents with authored padding. No execution.

Vertical parent/child margins now propagate native before/after struts through
same-writing-mode block chains, resolving inline percentage bases and stopping
at borders/padding, independent formatting contexts, columns and relevant
clearance. Private physical left/right absorbed margins keep descendant
allocation and translation from counting the strut twice. Vertical-lr/rl
regression source covers parent size and adjoining sibling gaps. Complex
clearance and intrinsic percentage bases still require refinement and runtime
verification; this is not conformance certification.

Viewport classic chrome now uses common geometry_for_viewport_scroll in both
renderer and native pointer consumers. Tracks remain physical; thumb proportions,
combined offsets and page steps derive from the zoomed visible layout extent.
Native thumb/track/repeat commits preserve layout scroll while visual pan can
absorb movement, then carry the residual into signed layout scrolling. The
common split helper bounds both components. Chrome hits alone convert incoming
layout CSS coordinates to physical coordinates, preserving the WebDriver API
contract. Content/touch physical input admission remains root-owned. Common,
renderer and native regression sources cover zoom, signed ranges, inverse thumb
mapping, pan-to-layout carry, fixed tracks and invalid geometry. No runs.

The shared visual Geometry now additionally carries the native physical
scrollport. Full layout surface dimensions remain unchanged; visual dimensions,
maximum camera pan and coordinate mapping use the content port, including RTL
left gutters. Native getter, VisualViewport observation and pinch setup consume
the same current layout port. Renderer scales around that port origin and clips
content to it before fixed chrome. Source regressions cover extreme pinch pan,
left-gutter mapping, clipped pixels and invalid ports. This resolves the earlier
classic-gutter extreme-pan policy gap; runtime verification remains pending.

Float multicol balancing now discovers spanner-separated sets in an initial
native geometry replay, including eligible nested spanning blocks, then adjusts
one height target per set. Every candidate is freshly shaped against its set's
fragment boundaries. Spanner transitions select the following set's height and
ordinary shared float scopes preserve the updated fragment origin. The existing
aggregate 128-replay budget remains; per-layout sets/fragments are bounded to
1024 with explicit errors. A regression source combines short and long sets
around a spanning row and checks their distinct final extents. Oversized float
continuations and advanced clearance/break interactions remain obligations;
the per-set balancing source has not been run or certified.

Source review also traced oversized block floats through native cross-fragment
exclusion bands and retained client-rectangle fragmentation. A new regression
requires a 150px float to continue across 100px columns while real shaped text
remains excluded for the first 50px of the following column and then returns to
full width. This exercises the existing continuation path; it does not establish
all oversized replaced-float or avoid-break behavior without execution.

Monolithic/replaced column participants now retain one unsliced geometry tree;
descendant glyph/cluster/caret and out-of-flow anchors move with that tree.
Break planning ignores synthetic interior breaks in in-flow monolithic boxes,
and floating monolithic boxes no longer prevent unrelated flow from breaking.
Oversized replaced floats retain their full paint height but own exclusion only
in their placement fragment. Both horizontal and vertical float placement
recheck the fragment after native collisions/clearance move the candidate.
Regression sources distinguish a fragmentable 150px block float from a whole
150px replaced float in 100px columns and exercise collision-driven placement.
Complex avoid-break relaxation and cross-spanner overflow remain under review;
no builds/tests have run.

PaintGlyph now exports its already-native fragment-local UTF8 source range;
annotation glyphs retain no text identity. Horizontal, vertical and inline
fragment exports preserve the native run/cluster mapping rather than estimating
glyph counts. Generated regression source checks actual shaped ownership and
UTF8 boundaries.

Arbitrary SVG clip/mask semantics for directly captured HTML text now use native
glyph-outline coverage. The normalized usvg definitions retain the original
object-bounding-box resolution; isolated source-cluster outlines pass through
the original ancestor transforms and effects. The inclusion rule requires
nonzero baseline glyph ink and unchanged alpha after clip/mask at the native
output resolution (including image/frame transform density). Partially removed
or attenuated clusters are omitted; whitespace survives only between retained
clusters. This is a finite pixel-coverage policy, not mathematical vector
containment. Bounds cover 16384 clusters, 64 million aggregate pixels of work including
intermediate layers, 512MiB aggregate generated-source work and 16MiB individual
SVG sources. Source tests include arbitrary circular/path clips, partial glyph
selection, luminance/object-bounding-box masks and one-alpha-unit coverage.

Masked nested frame/SVG-HTML semantics still require projecting child native
glyph provenance through their enclosing document paint path. That combination
currently reports incomplete print output instead of silently dropping text;
it remains an implementation obligation. None of this source has been executed.

Masked nested frame glyph coverage is now generated: selection follows native
frame snapshot paths, keeps each child's font/resources in that child, and
exports transparent glyph-only frame paint through actual frame transforms and
clips. Regression source repeats the arbitrary clip/mask cases with a child-only
font resource. Nested SVG HTML still needs preservation of its original effect
reference boxes while selecting descendant glyphs; that narrower case remains
explicitly incomplete. SVG local-frame filtering now retains frame owners in
nested captured foreignObject subtrees. A stale renderer-core module reference
in frame vector export was corrected during source inspection.

Leading parent/child clearance now compares native per-side float-bottom
thresholds with the complete hypothetical collapsed margin strut. Positive
adjoining margins can move the hypothetical border past a float without creating
clearance; negative struts can move it back into a float that is already above
the current cursor. An absent float is distinct from a signed negative threshold.
Source regression covers complete descendant struts, opposite-side clearance,
negative positions and a real document with a 100px margin past an 80px float.
Trailing-edge clearance remains conservative pending used-position integration.

Nested SVG HTML semantic coverage now has source integration. Candidate identity
uses native frame-owner paths plus captured foreign-object indexes; no resource
lookup crosses document authority. A bounded cache preserves the original full
normalized SVG effect reference boxes before other text or backgrounds are
removed. Selected native glyph paint is then restored under those original
ancestor masks/clips. Native HTML clips remain in its own vector output.
Regression source adds nested objectBoundingBox masks and preserves child-only
fonts, namespaced SVG references and HTML CSS clip declarations. Nested captures
share aggregate glyph/pixel/source-work limits rather than resetting budgets.
The precision policy remains equality of actual glyph alpha at native output
resolution; this does not establish mathematical vector containment. Repeated
shared captured-view instances currently produce an explicit incomplete-print
error because their instance-specific semantic placement is not yet represented.
Pure SVG text semantic extraction, earlier remaining sizing/fragmentation
obligations and all execution/conformance validation remain open. No tests or
builds have run.

Projective localized raster capture now enters the same original-effect SVG
selection path and native frame-owner scope as vector capture. Regression source
repeats nested mask coverage through a projective HTML image owner. Repeating
SVG gradients do not force a PDF compatibility fallback during the intermediate
semantic-only SVG export; ordinary PDF compatibility behavior is unchanged.

## Native SVG provenance, repeated instances and used trailing clearance

The three narrower gaps above now have implementation and regression sources:

- Trailing parent/child margins are resolved from actual child geometry and a
  native record of whether clearance moved the child. A clear declaration alone
  no longer blocks an empty box's adjoining margins. Nonempty cleared children
  can propagate their bottom margin; empty children with actual clearance keep
  it inside the parent. Vertical logical ends resolve before physical glyph and
  child placement, using the same record. Regression sources cover both vertical
  directions and horizontal flow, empty/nonempty cleared boxes and opposite-side
  noninterference.
- Masked semantic capture expands repeated SVG resources into independent private
  paint views. Native URL bases and document resources remain unchanged; private
  node identities are only used inside PDF processing, never exposed as DOM IDs.
  Frame copies retain their own fonts/images. Instance expansion has depth,
  count, node and byte budgets, detects source cycles and performs no fetching.
  Regression sources distinguish visible and clipped repeated instances.
- Pinned usvg 0.45.1 is vendored from upstream commit
  `1b6c2fddbcbeffa8135df4323b02aaae84890907`, with original licenses. Native glyph
  ranges use shaping cluster byte positions plus original chunk offsets, and
  preserve shaping direction. Private flattened-text IDs are assigned after
  style resolution while authored IDs remain intact. Pure SVG semantic extraction
  uses native glyph-group flattening, preserving actual fonts, stroke, glyph
  transforms and color-glyph handling. Direct SVG and SVG nested within masked
  HTML use the same finite-alpha inclusion policy. Ancestor acceptance restricts
  nested selection, so an inner mask cannot re-admit an outer-clipped cluster.

Source tests cover repeated characters, ligatures, combining text, chunk changes,
bidi runs, partial masks, post-style identity assignment and nested pure SVG
instances. The native usvg tree is retained alongside normalized effect XML so
semantic extraction does not lose shaping provenance during serialization.
No builds, dependency resolution, generators or tests have run. These source
paths remain unverified; broader full-plan V obligations and the global execution
gate remain open.

Pure SVG character geometry now retains native cluster quads from usvg's actual
advance/ascent/descent and span/path transforms. Rotation and text-path regression
source checks oriented quads. Glyph-alpha visibility remains based on the exact
native flattened glyph paint, independently of the character's line-metric box.
# Full-package reconciliation continuation (September 16)

Additional source now connects ordered native top-layer state to selectors,
modal inertness, formatting roots, shared paint/hit order and backdrop paint.
Popover transitions use native state, reentrancy guards, cancelable beforetoggle,
coalesced deferred ToggleEvent payloads, invoker activation, Escape dismissal and
matching pointer-down/up dismissal. Native focus restoration and source identity
are retained outside attributes. These paths remain unbuilt/unrun; full picker
and lifecycle qualification remains open.

Native captured control states now drive meter, progress, checkbox, radio, range
and select-indicator theme commands. HTML meter range normalization, optimal
region coloring and appearance:none have source regressions. Indeterminate
progress animation and full select closed-value/picker presentation remain open.

Stylo before/after pseudo values now materialize native layout nodes with private
paint identities, actual shaping and owner-mapped hit testing. Resolved strings,
attribute content, counter/counters text and quote tokens feed a separate
GeneratedContent owner/pseudo/text/alternative projection for accessibility.
Screen/print/drag producers carry generated paint nodes. Generated images,
custom counter-style/locale quote refinements and scoped SVG-subtree integration
remain active generation work. No generated ID is a DOM identity and no generated
text is appended to DOM selection offsets.

The complete V exit remains open. The authoritative inventory still contains
16,737 Core entries, including 1,003 reftests, 12 print reftests, three visual
entries and 749 manual entries. No denominator or tier edits were made.

Source review found computed CSS outlines without any painter. The new native
outline painter uses physical fragment boxes, computed offset/width/color/style,
existing border raster/vector implementations, projection clips/transforms and
the final stacking-context outline phase inside group opacity. It does not alter
layout geometry or scroll extents. Regression sources check negative offsets,
unchanged geometry and descendant/context opacity ordering. The native theme
currently resolves `outline-style:auto` to a solid indicator; connected irregular
inline outline contours remain a separate refinement from rectangular fragments.

The review also identified missing authoritative ordered top-layer state: dialog
`:modal` currently depends on `data-oryn-modal`, and there is no corresponding
native top-layer paint ordering/ancestor-clip exemption. Parent owns coordination
of DOM state; layout/render integration remains open pending that contract.
The accessibility full-recomputation oracle exists in native semantic.rs, but
the required macOS assistive-technology consumer was not found; its implementation
was assigned to the family/platform owner. Existing semantic tests alone do not
satisfy manual platform assessment.

All observations and additions above are source-only. No build, test, formatter,
runtime or conformance execution has been performed during this generation phase.

## Reconciliation map (production source, not acceptance evidence)

| V obligation | Production path inspected | Remaining integration/assessment |
| --- | --- | --- |
| State/cascade/ownership | layout Stylo projection, DOM-query selectors, per-sheet native base URLs | Replace author-attribute modal state with authoritative ordered native top layer; coordinate popover/picker state and scope invalidation. |
| Geometry/text/scroll | layout inline formatting, table, float, vertical text, pagination; common scroll/viewport geometry and native pointer consumers | Complete actual control formatting/theme/picker dependencies; audit against the pinned HTML visual references rather than unrelated CSS directories. |
| Pixels/stacking/clipping | render-core shared stacking planner, SVG captured resources/provenance, vector print, native frame composition | Top layer/backdrop and native control paint are concrete open paths. Irregular connected inline outline contour remains open. |
| Capture/print | runtime print-media snapshot planning, exact-lifetime child frame capture, print page fragment assembly, render-core frame-scoped resources | Execute all 12 pinned print references after global generation; specifically verify nested/cross-origin/scaled iframe output with print settings and frame lifetimes. Source presence is not passing evidence. |
| Accessibility | semantic.rs incremental/full-oracle projection regressions | Native macOS accessibility adapter and required assistive-technology assessment are delegated; manual denominator remains open. |
| Shared revision/input | live layout projection, native paint revision including visual viewport, scoped frame capture and native pointer chrome | Integrated focus/top-layer/control geometry must consume the same revision; no separate author-derived visual model. |

The Core visual forms inventory is 99 select, 25 input, seven textarea, five
meter, two datalist, one customizable-combobox and one option reftest. Pinned
meter sources were inspected at revision
`719d5e38fdd0903a18ed9007aba816c98cc491e0`: `meter-appearance-none-rendering.html`
and its optimum-value counterpart require a mismatch against native auto
appearance; `meter-min-rendering.html` requires equivalent normalized range
painting. Current source lacks a native meter painter/state contract, so this
remains required implementation rather than an unsupported disposition.

### Subsequent native top-layer, control and generated-content generation

The preceding reconciliation table recorded gaps at that inspection point.
Source now includes authoritative DOM top-layer order, modal inertness,
popover transition guards/source identity/deferred ToggleEvent coalescing,
shared stacking/hit ordering, native backdrop commands, and serialized native
meter/progress/check/radio/range/select theme state. Native meter normalization
and appearance suppression have regression sources. These supersede the earlier
statements that modal state and meter painting were absent; no execution evidence
or passing status is implied.

Before/after pseudo styles now materialize into ordinary shaped private layout
nodes, preserving counters, attribute content, source ownership and separate
GeneratedContent accessibility metadata. Capture, print and hit retargeting use
the same generated geometry. The vendored Servo parser now exposes the standard
select picker/icon/checkmark pseudos and base-select grammar. Full picker layout,
author backdrop geometry, generated images/custom counter styles and scoped SVG
pseudo integration remain active implementation obligations.

Pinned selectedcontent.html, selectedcontent-nested.html and
selectedcontent-mutations.html at the unchanged WPT revision were inspected.
Native cloning now distinguishes explicit selection refresh from option descendant
mutations, supports targets outside the button, disables nested targets, and uses
one native replace-all mutation record. Closed select projection shapes the native
selected label while hiding its option subtree, without inventing DOM text offsets.
Regression source covers clone identity, nested removal, selected label shaping and
hidden option geometry. Incremental parser timing, insertion/move semantics and
full customizable picker behavior remain to be reconciled; tests have not run.

Native picker generation now includes an independently formatted option subtree
anchored to the actual originating box, paint-only parent overrides, and shared
capture/drag/hit ancestry integration. Native select option IDs and semantic DOM
parents are preserved. Trusted click/keyboard and outside pointer lifecycle paths
are generated; showPicker uses the parent-owned exact transient activation check
and atomic consumption, with native origin/rendered/mutability checks. These are
source paths, not conformance results. Detailed platform picker interaction,
author anchor/inset policy, listbox interaction and complete pseudo themes remain
under implementation.

Reachability audit found the checked-in generated WebIDL registry lacks popover,
ToggleEvent, showPicker and HTMLSelectedContentElement. Their authoritative
browser.webidl definitions are now added, but the registry **must be regenerated
in the single global build phase** before handlers are reachable. No generator
was executed during source generation. Selected option Stylo :checked and native
select :open now use live native state; custom quote pairs are parsed instead of
cycling hard-coded quote characters. Locale-dependent auto quotes remain open.

Backdrop source now carries an independently formatted native pseudo snapshot
(one level only) through LayoutOutput, BindingLayout, regular/drag capture and
renderer/hit consumers. CSS insets/transforms are resolved by the layout engine;
owner opacity/ancestor clips do not leak into this box. Native owner identity and
recursive-pseudo rejection have regression sources. Print pseudo background
suppression follows the print background option, and page-margin synthetic IDs
now reserve generated content IDs. Per-page backdrop reflow/projection for varying
print page dimensions is the remaining integration in this path, not considered
complete by the screen implementation.

Base-select picker-icon/checkmark now shape ordinary native glyphs with actual
pseudo styles; unchecked marks retain layout but hide paint. These decorations do
not enter accessible-name GeneratedContent metadata. All changes remain source
only, including new regression sources; no build, test or registry generation ran.

Per-page backdrop integration now supersedes the preceding print gap: each
PrintPageLayout records its actual paper content box/scale and independently
restyles/reflows backdrops at that content viewport. Projection applies paper
margins/scale plus page clipping, independent of owner transforms/fragment offsets.
Native assembly uses these page-specific snapshots, counts them against fragment
limits, validates synthetic identity/nesting, and captures every page's background
resources through the authorized native resource path. Regression source covers
print content bounds, margins/scale and synthetic ID separation. No rendering or
conformance run has validated these algorithms yet.

## Generated image lifecycle and capture preparation (source only, 2026-09-16)

Generated `content: url(...)` now materializes native replaced boxes, including
mixed text/image content, from successful native intrinsic dimensions. Private
image children inherit text state without copying their owner's transforms,
positioning, backgrounds or other non-inherited effects. Author DOM and selection
text remain unchanged. CSS URL resolution uses the native document base and the
existing per-stylesheet base map; URL metadata is not resource authority.

`oryn-js/src/v8_bindings/css_images.rs` supplies queued, pending, failed and ready
states through native `ImageStart`/`HttpRead`. It retains final response bases and
original request aliases, verifies native document generation/stream/token,
bounds count and encoded/decoded sizes, cancels removed or retired requests, and
publishes intrinsic dimensions through native DOM metadata and layout revision.
Layout getters schedule work; they do not await I/O. Failed read submission now
releases readiness and the native stream. Capture-only demand survives screen
style refresh while print resources load.

Native screenshots and print use actor-resumable preparation in
`capture_readiness.rs`: bounded FIFO requests, deadline and native lifetime
checks, provisional layout discard, then fresh layout after image completion.
Print lifecycle events occur once per request. Final resource completion stays
outside the actor. Loaded generated image resources also feed native drag
capture, which rejects a missing decoded resource rather than inventing pixels.
Regression sources cover resource redirects/reuse, stale completion, failed
reads, decode bounds, pseudo intrinsic layout, screen/print reflow, event count,
actor responsiveness and navigation cancellation. None has run.

Native foreignObject composition now excludes native top-layer membership from
local SVG HTML paint and permits it in the owning document's global paint tree.
Membership follows native top-layer entries and picker paint-parent overrides;
DOM ancestry is unchanged. Private generated content is included only in its
owning paint tree, preventing duplicate global/local pseudo paint. Regression
source covers modal geometry/backdrop escape from a transformed small SVG.

`getComputedStyle` now retains the requested pseudo on a native private wrapper,
reads live projected pseudo styles, supports the standard legacy single-colon
forms and `::picker(select)`, and keeps owner inline declarations out of missing
pseudo declarations. Regression source checks live values and empty unknown
pseudo declarations. Registry generation remains deferred to the single global
build phase for the newly added IDL APIs.

Still open: scoped-worker and pre-action inventory readiness must share the
resumable preparation contract; full generated image-function/custom-counter-
style/locale quote behavior, complete select intrinsic/theme/commit behavior,
and remaining Core/package reconciliation remain required. This entry is a
source progress record, not build, test, WPT or V-package completion evidence.

### Shared readiness follow-through

The normal actor accessibility snapshot request now joins the resumable capture
queue. Semantic/text state is captured at the ready native layout checkpoint;
resource completion occurs off the actor, then an actor completion message
checks native generation/epoch/revision before publishing the AX snapshot.
Existing native editable-target and text-size checks are preserved. Native UI
callbacks still perform no fetch or actor work.

Serialized scoped-worker snapshot and pre-action preparation paths now pump
native image completions with exact lifetime, cancellation and deadline checks,
and rebuild the paint plan after intrinsic publication. They execute before UI
callbacks/input ticks; their serialized worker protocol remains synchronous.
Nested child-frame composition is provisional during the owning root request's
readiness phase, so it does not recursively wait for the same pending resources.
Regression source includes AX generated-image geometry at the same dimensions
as screenshot output. All of this remains unbuilt and unrun; remaining full V
requirements and global qualification gates remain open.

### Select sizing, anchored picker and staged selection

Closed native selects now measure the longest option label with the native text
shaper, independently of which option is selected. Option traversal uses source
child order and label fallback includes descendant text while excluding scripts
and generated pseudo text. Picker placement preserves authored physical insets,
size limits and RTL alignment rather than overwriting them with the default
anchor placement. Regression sources cover selection-independent intrinsic
width and authored picker geometry.

Native top-layer entries now retain a validated active option separately from
selectedness. Arrow/typeahead movement updates that highlight; Enter/Tab/Alt-Up
commits the selected value, and Escape closes without committing. Native option
state changes invalidate control/layout/semantic revisions. Default theme paint
uses `PaintControl::Select.active_option`, including pickers extracted from SVG
HTML trees. Listbox pointer selection reads the trusted native modifier payload
and supports range/toggle selection. Regression sources cover staged selection,
disabled-option skipping, Escape, and independent theme highlight. These source
changes remain unbuilt and unrun; full theme/customizable focus and remaining
listbox sizing behavior are not certified complete.

### Counter-style representation and generated text integration

Generated counter text now reads Stylo-validated `@counter-style` descriptors
with screen/print media filtering and per-stylesheet resolution bases. Native
formatting implements cyclic, numeric, alphabetic, symbolic, fixed and additive
systems; descriptor inheritance; range and fallback cycles; negative affixes;
and padding. Built-in decimal, alphabetic, Roman, numeric-script and bullet
styles participate in inheritance. Anonymous `symbols()` styles use the same
formatter. Output and recursion bounds prevent oversized or cyclic styles from
hanging native layout, including empty repeated symbols.

Regression sources exercise the real generated-content layout path, changed
stylesheets, print media, builtin inheritance, anonymous styles, additive zero,
negative values and resource bounds. No builds or tests have run. Remaining
counter obligations include complete predefined international systems,
list-marker box placement/numbering, reversed automatic counters and scoped
counter-style cascade precedence; these remain implementation work, not a
reduced conformance denominator.

Native list-marker generation now shares the counter registry, including author
prefix/suffix and builtin/anonymous representations. Computed style projection
includes list properties and `::marker`. Private marker boxes are shaped in the
ordinary text engine; inside markers participate in inline flow, while outside
markers are moved before projection/fragmentation to the owner's inline-start
edge with RTL/vertical direction handling. Ordered-list `start`, `reversed` and
item `value` feed native ordinals. Added regression sources for custom marker
text, outside geometry, inside reversed numbering and excluded selection IDs.

This does not close list layout: first-line baseline alignment, list-style-image
readiness/fallback, non-direct list-item counter scopes and author/reversed
`list-item` counter interaction remain explicit source work. Marker placement
has not been built or executed, including vertical/fragmented cases.

Outside horizontal markers now retain the shaper's native first-line baseline
relative to the content box and align with the owner's first in-flow line,
including descendant block text and differing marker font sizes. This avoids
inferring baseline ascent from glyph ink. Regression source compares actual
shaped marker/body baseline origins. Vertical first-line baseline alignment and
marker images/counter scoping remain open; no execution has occurred.

`list-style-image` now publishes an independent native CSS-image request set.
`LayoutOutput.css_image_requests` and the live binding projection carry demand
without falsely attaching an unavailable image to the text fallback marker.
Screen/print capture and live resource scheduling union that demand with actual
generated image associations. Pending/failed images retain counter text; native
ready dimensions replace it with a shaped-layout replaced image box. Regression
source covers the request/fallback/ready transition and outside image geometry.
Native request authority and cancellation remain the existing exact-creator
CSS image lifecycle. Additional marker image sizing/gap and stylesheet image
functions remain open; no tests or builds ran.

Gradient marker images now use the renderer's existing native CSS background
paint path in a one-em default image box. HTML list ordinal traversal follows
source child order through ordinary wrappers and stops at nested lists. The
remaining counter scope gap is author CSS counter/reversed-counter interaction,
not wrapped HTML list ownership. Vertical baseline, marker gap/image-functions,
complete international counter systems and scoped definition precedence remain
unimplemented or unreconciled; full V generation and qualification gates stay
open.

Vertical marker alignment now consumes the native vertical line-center baseline
retained during shaping/physicalization, and inline-start placement uses the
same writing-mode axes as normal layout (including sideways-lr reversal).
Regression sources cover vertical-rl, vertical-lr and sideways-lr in both text
directions. Native actor regression source additionally covers ready SVG marker
resources and HTTP-failed marker fallback. These cases remain unexecuted.

Remaining predefined representations were reconciled with the CSS Counter
Styles3 normative definitions (source read 2026-09-16 at
https://github.com/w3c/csswg-drafts/blob/main/css-counter-styles-3/Overview.bs).
Added Georgian/Hebrew additive tables; Japanese/Korean limited-range additive
systems; Chinese zero collapse, negative signs and informal tens; and the
Ethiopic paired-digit grouping algorithm. These participate in author `extends`
and fallback. CJK decimal's nonnegative range and stem/branch fallback were
corrected from the actual definitions, and padding now counts grapheme clusters.
The optional extended CJK range is not selected; the prescribed limited-range
fallback applies, without changing qualification inputs.

Regression sources include native generated-text cases read from pinned WPT
719d5e38fdd0903a18ed9007aba816c98cc491e0,
`css/css-counter-styles/ethiopic-numeric/counter-ethiopic-numeric.html` and its
reference, plus negative/boundary/extends cases across the other systems.
This is source integration only. Counter scopes/reversed counters, cascade
layers/scoped counter names and continued full V reconciliation remain open.

### Counter identity, reversed initialization and HTML cascade

The previous depth-only counter map has been replaced by a flattened-tree pass
that retains originating-element identity. It inherits counter membership from
the previous sibling/parent and values from the previous tree-order element,
then applies reset, increment and set in normative order. Real pseudo ordering,
non-generating pseudo exclusion, implicit list-item increments and a separate
pass for reversed auto initial values are included. State allocation is bounded.

Native HTML ol start/reversed and li value now produce PresHints-origin typed
counter declarations in Blitz; author CSS retains precedence. Stylo's existing
typed counter-set parser is exposed to Servo, and reset/increment are no longer
behind the unimplemented preference. Registry generation is deferred to the
whole-plan build phase. Added regression sources for tree-order value transfer,
shadowed counter instances, set-after-increment, ineffective pseudos, reversed
increments/sets, and HTML hints overridden by author CSS. These sources are
unbuilt/unrun; scoped definitions/cascade layers and full V audit remain open.

Counter-style rule selection now respects ordered/nested/anonymous CSS cascade
layers across stylesheets, with unlayered definitions taking normal precedence.
Definitions inside `@scope` remain globally named within their tree, as required
by CSS Cascade6; selector scope is not incorrectly reused as name authority.
Regression sources cover explicit layer ordering, nested versus outer layers,
unlayered precedence and a definition inside a nonmatching selector scope.
Native shadow-tree name provenance is still a distinct producer/consumer
integration requirement; current implicit-scope-root metadata is not a substitute.

## Native shadow style ownership and mirror generation — 2026-09-16

Generated `LayoutInput.shadow_trees` with native root/host, element tree scope,
stylesheet tree scope and assigned-slot identities. Parsed snapshots retain
closed roots and default named assignment; live projection replaces assignment
with `DomRuntime`'s authoritative manual/default result. Owned and adopted
shadow sheets preserve sheet bases/media and their actual root. Selector parent
links stop at shadow roots; formatting uses a separately projected flat tree.

Blitz now creates private shadow-root nodes, rebuilds per-root author
`CascadeData` with Stylo, and implements host/containing-root/assigned-slot/
traversal/part hooks. The implicit-scope callback uses globally unique opaque
identity registrations with drop cleanup, without recovering pointers. Text
assignment, fallback exclusion, local host inheritance, unassigned light-DOM
computed style and shadow-including language inheritance are source-integrated.
Shadow metadata validates roots, cycles, sheet membership and slot host ownership
before style traversal. Native generated content uses tree-local counter-style
registries and resolves inherited definitions in their defining registry.
Generated marker quote evaluation now follows marker-before-before order while
physical pseudo insertion independently preserves that same order.

Regression sources cover native closed-root snapshots/adopted sheets, document
rule isolation, host/slotted inheritance, reassignment, unassigned nodes, text
slots, nested exported parts, per-host implicit scopes, native identity errors,
and counter definition scopes. All remain unbuilt and unrun.

Remaining source obligations include declaration-origin tree provenance for
**inherited named counter-style references** (the current generated consumer
selects the element's tree), scoped descriptor fallback provenance when extending
an outer definition, and full V/Core reconciliation. Incremental slot invalidation
is intentionally not used by the fresh snapshot mirror; all native mutations
recreate the style mirror. This is not a V completion or conformance claim.

### Declaration provenance and resolved text follow-through

The inherited counter-name gap in the preceding entry is now addressed in source:
Stylo computed `ListStyleType` carries an absolute native declaration-tree token;
`GenericContentItems` similarly retains its declaration tree for counter()/counters()
in pseudo content. Both retain provenance through computed-to-specified conversion.
Native projection resolves mirror tokens back to captured shadow-root identities,
and generated markers/content select that registry. Cross-root style sharing no
longer treats equal cascade allocation as equal tree identity. Inherited descriptor
fallback and special-system lookup follow their defining registry rather than a
child tree's equal name. Regression sources cover inherited/local marker names,
slotted pseudo content, and outer fallback descriptors. Property source generation
is not a claim that the deferred global property build has succeeded.

Added `LayoutOutput.resolved_text` produced from the actual normalized/transformed
shaping source, published at final placement rather than during intrinsic sizing.
Ruby and multicol paragraph composition preserve individual source runs. Native
zero-advance collapsible ASCII space/tab clusters become erased source aliases;
LF and preserved whitespace remain. Source UTF-16 ranges, rendered UTF-8 ranges,
transform expansion, geometry aliases and anonymous owner identities are retained.
Closed synthesized select labels do not acquire DOM source ranges. Aggregate
provenance is bounded at 64 MiB. The family agent owns the corresponding AX
consumer and native transport. Regression sources for ß expansion, collapse,
and final-layout provenance remain unrun. Full V requirement reconciliation,
pinned qualification and generation gates remain open.

### Connected inline outline contour source

`oryn-render-core/src/outline_contour.rs` now computes bounded unions of native
fragment rectangles. Solid/auto and double outlines paint disjoint union rings;
dashed/dotted contours walk connected boundary segments with shared perimeter
phase and clip marks to that ring. They use existing projected fill/round-fill/
opacity primitives, so raster and vector print consume the same contour geometry
and clips. Alpha is applied once to the resulting group. Source regressions cover
wrapped L contours without an internal horizontal seam, disjoint ring tiles,
negative offsets and patterned styles. Geometry overflow fails display-list
construction explicitly rather than omitting the outline. Single-box rounded
outlines retain the existing border-edge path. Connected inset/outset/groove/ridge
and rounded multi-fragment joins remain follow-on refinements, not certified
source-complete behavior. No execution performed.

Connected inset/outset/groove/ridge outlines now reuse native directional bevel
shading clipped to the same union ring, so internal fragment sides are excluded.
Negative-offset clamping is applied before stroke thickness independently on each
physical axis, including fully inset contours. Expanded geometry and contour work
are explicitly bounded. Rounded multi-fragment join refinement remains open.

### Native selector hook reconciliation

Removed the legacy selector-regex translation for ::part(), :state() and :lang();
those selectors now retain native parsing, specificity and tree ownership.
The mirror receives native custom states and language/direction fields directly.
Added qualified attribute matching and actual HTML/SVG/MathML namespace checks.
Servo :lang() parsing/matching now accepts comma-separated ranges and extended
subtag wildcards, including shadow-including inherited language. Regression
sources cover live state removal, language lists/wildcards, and two attributes
with the same local name but different namespaces. Remaining native state
projection for defined/placeholder/read-write/target is under reconciliation;
none of these source changes have been built or tested.

### Native selector-state lifecycle integration

Added live `LayoutInput.selector_states` for :defined, :placeholder-shown,
:read-write/:read-only and :target. DOM query matching and Stylo read the same
native helpers. Custom-element construction/upgrade success publishes defined
state with a native mutation journal entry; failed upgrade remains undefined,
and attaching internals is not treated as definition. Custom-element name
validation is now a shared native PCEN-character implementation. Placeholder
state reads current control value, read-write uses native disabled/readonly/
editable state, and target uses decoded document fragment plus DOM order without
crossing a shadow or child-document boundary. Native document design-mode
metadata/setter/query are present; family owns the V8 conversion/state sync
correction. Tests for native state transitions, fragment decoding, language/state
selectors and CSS recomputation are generated only, not executed. Remaining
state-family/namespace/document and full visual inventory reconciliation is open.

### Native state matcher and empty-control caret reconciliation

Source review found `:modal`, `:open`, and `:popover-open` matcher arms still returned false despite populated native flags. They now consume the flags; details/dialog open state also projects independently of top-layer membership. Native required/optional applicability now shares a DOM query and layout state, excluding input types where required does not apply while preserving disabled/readonly selector membership. Native designMode rejects non-document targets; unknown input types use the text placeholder default. Regression sources cover these transitions.

Empty input/textarea caret production now reads the live rendered control value rather than textarea default children, emitting offset-zero native caret stops using selected-font line metrics and the existing text projection. Added empty-to-nonempty control regression sources for the protected IME consumer. All changes remain unbuilt and unrun; range/default/user-valid selector families and rounded multi-fragment outline joins remain explicit generation work.

Range selector follow-on: native numeric limit parsing/periodic time range truth now feeds both DOM matching and Stylo state, preserving inapplicable vs in-range distinction and ignoring step mismatch/disabled candidacy. Regression sources cover empty values, missing limits, range defaults, periodic time and CSS state transitions. This supersedes range as a wholly absent selector path above; user-validity/default-state reconciliation remains. Nothing executed.

Default selector follow-on: native default checked/selected attributes and the first submit-capable control in actual form-owner tree order now project through DOM queries and Stylo. External form-associated image submit controls participate. Source regressions cover ownership order and CSS transition. User-validity requires independent native user-interaction bookkeeping rather than dirty-value flags; coordination with input owner is open. No tests/builds executed.

## Native user-validity selector generation — September 16

Source-only; no build, formatter, generator or tests executed. The source-audit
finding that production Stylo always returned false for `:user-valid` and
`:user-invalid` is replaced with native state transport.

- DOM owns an independent `user_validity` flag, initially false. Script value
  assignment does not grant it or clear a previously committed flag. Selector
  membership additionally requires input/textarea/select validation candidacy.
- Committed text edits, OIL replacement, native checkbox/radio activation,
  select keyboard/picker changes, file drop and native control notifications
  publish the flag before input/change callbacks. Pending text edits publish
  at their native commit boundary, not each keystroke.
- Submission marks form controls before interactive validation, including the
  noValidate branch; direct script submit and checkValidity remain separate.
  Successful native reset clears flags; cancelled reset preserves them.
- `LayoutSelectorState.user_validity`, DOM query matching and production
  Blitz/Stylo USER_VALID/USER_INVALID flags consume the same DOM truth.
- Regression sources cover DOM pending-versus-committed edits, script changes,
  disabled candidacy, reset/select/reset, mutation invalidation; JS submission
  callback ordering and cancelled reset; actual Stylo state changes; and
  authenticated native keyboard -> blur -> computed style -> reset/submission.

Bounded source-generation item complete for these native built-in control paths;
validation pending. FACE user-validity reconciliation is now complete at the source
boundary: the HTML selectors exclude FACE, which retains normal validity selectors.
Full selector/Core mapping, all platform input acceptance and other S/V requirements
remain separate reconciliation obligations. No whole-package completion claimed.

## Native numeric editing and media reconciliation — September 16

Source-only generation; nothing built or executed.

- `is_editable_text_control` adds number to native input/clipboard/composition
  editing without changing public selection or text-length constraint applicability.
  `control_edit_value` preserves incomplete user numeric text; `control_bad_input`
  exposes the native invalid-input flag while public `value` stays sanitized.
  Native keyboard undo snapshots, IME, placeholders and production rendered text
  use the editing value. Script assignment/reset clears badInput; cloning does not
  copy incomplete editing text. Native validity, validationMessage and OIL
  `NativeValidationState.bad_input` consume this truth.
- Regression sources: `native_number_edit_retains_incomplete_text_without_exposing_it_as_value`,
  `number_keys_keep_bad_input_text_and_report_native_validity`,
  `native_number_bad_input_reaches_production_style_and_placeholder_state`.
- `oryn_layout::media::matches` parses and evaluates native Stylo media lists,
  ranges, unknown/invalid queries, print/screen, orientation, resolution and user
  preferences. Added Servo reduced-motion feature/device state and fed it to
  production style traversal. DPR uses physical viewport dimensions with native
  scale so CSS width remains stable while resolution is accurate.
- `RawV8Host::native_media_matches` and `native_stylesheet_blocks_scripts` use
  native active-realm state. `install_media` updates native providers, including
  inheritance into new frames, and invalidates binding layout caches. JS
  matchMedia captures the native evaluator instead of reading mutable author media
  globals. Live style computation uses the same preferences.
- CSSOM disabled state is native and separate from owner attributes, follows the
  live stylesheet identity, filters production/adopted sheets, and participates
  with native media/connection/type/alternate checks in parser script eligibility.
  Shadow sheets do not block the document parser. Root owns parser wait integration.
- Regression sources: `native_media_lists_ranges_negation_invalid_and_preferences`,
  `native_media_backend_and_stylesheet_script_applicability_use_native_state`.

Remaining explicit obligations: full MediaQueryList identity/event notification
algorithm; preferred named stylesheet-set selection/switching; broader platform
segmented date/time UI acceptance; final full-plan selector/layout/AX matrix and
conformance execution. These are not implied complete by the bounded sources above.

## Named stylesheet sets and MediaQueryList follow-on — September 16

Generated source and regression sources only; no build, tests, formatter, or
IDL generator executed. The new IDL declarations must be consumed in the later
all-code build phase.

- Native `DocumentMetadata` retains preferred/last set; preferred selection starts
  from the first nonalternate titled owner. `DomArena::stylesheet_disabled` now
  resolves preferred/selected title and explicit disabled state. Native selection
  updates current titled sheets while leaving untitled persistent sheets alone.
  New sheets inherit last-set selection, and shadow-tree sheets do not join a
  document's named sets. Parser eligibility and layout consume the same flag.
- Authored selectedStyleSheetSet/preferredStyleSheetSet/lastStyleSheetSet and
  enableStyleSheetsForSet use native Document receivers. Mixed enabled titles
  return null. The setter records last-set; enableStyleSheetsForSet does not.
  styleSheetSets is a live SameObject DOMStringList with native indexed access,
  enumeration, contains/item, receiver checks and traced document ownership.
- `MediaQueryList`/`MediaQueryListEvent` now have IDL source and native platform
  records. matchMedia returns a fresh native EventTarget identity, with canonical
  Stylo media text and live matches. addListener/removeListener share EventTarget
  listener identity with addEventListener/removeEventListener; onchange uses the
  native handler path. Native listener activity retains active observers.
- Native timer readiness and checkpoint polling compare committed media results,
  snapshot changes before callbacks, dispatch trusted nonbubbling change events,
  preserve event.media/event.matches, and stop on document retirement. Collected
  observer wrappers are reclaimed without resurrection. Author constructors,
  public dispatch methods and mutable media globals do not determine delivery.
- Device pixel ratio now flows from production layout installation into all
  existing/new native providers, alongside viewport size and media preferences.
- Regression sources: `named_stylesheet_sets_control_native_layout_and_parser_applicability`
  covers selection, mixed sets, last-set behavior, live list mutation and native
  parser/style agreement; `native_media_query_lists_have_identity_shared_listeners_and_change_events`
  covers fresh identities, deduplication across legacy/standard listeners, payload,
  no duplicate unchanged delivery, removal and author event construction.

This supersedes the earlier wholly-open named stylesheet selection and MQL
identity/change-event items. Full WPT edge matrix and execution remain pending;
no whole S/V package conformance claim. Broader segmented date/time platform UI
and outstanding selector/layout/AX inventory remain separate obligations.


## September 16 — native segmented temporal editing source

Source generation only; no build, test, formatter, binding generator or conformance execution.

- [x] Bounded S-04 native date/month/week/time/datetime-local field editing:
  `temporal_edit::{TemporalEdit,edit_temporal_control,replace_temporal_edit}`
  retains partial fields separately from sanitized public values, handles native
  field movement/digits/clearing/increment and precision from effective step and
  min/max/value. Invalid calendar combinations remain badInput. Script assignment,
  type changes, reset and cloning do not retain stale user field state.
- [x] Native key defaults use the model in `native_keyboard::temporal_key`, with
  accepted keydown/keypress, cancelable beforeinput, fresh focus/document/type/
  disabled/readonly/inert checks, native selection and undo. Established valid
  whole-value stepping remains available before explicit field selection.
- [x] Clipboard, composition and undo share editable-control raw text. Marked
  composition tracks normalized field range but keeps the actual input payload
  for compositionend.data; cancellation restores prior raw text and dirty/user/
  pending-change flags. Constraint attribute changes preserve failed user input
  (only range min/max/step transitions resanitize the current value).
- [x] Bounded V/OIL integration: production rendered field text comes through
  `populate_validation_states_for_root`; native pointer and accessibility text/
  selection use `control_edit_value` and field alignment. `semantic_edit` routes
  replacement/clear through native beforeinput/edit/input/change, including
  incomplete values whose public value is empty. Native validity projection
  reports badInput and committed built-in user-validity state.
- [x] Regression sources generated:
  `segmented_calendar_and_clock_edits_share_native_validation`,
  `invalid_calendar_dates_and_fractional_clock_values_are_not_normalized_into_valid_dates`,
  `clock_field_precision_follows_effective_step_without_erasing_partial_edits`,
  `segmented_temporal_keyboard_validation_style_and_semantic_replacement`,
  `all_temporal_control_families_accept_native_field_entry`,
  `temporal_ax_text_uses_rendered_fields_and_selection_drives_native_editing`,
  `partial_temporal_edit_updates_oil_bad_input_and_validity`, and
  `temporal_marked_field_updates_preserve_partial_value_and_cancel_state`.
- [ ] Remaining scope: calendar/picker popup and localized presentation, complete
  keyboard/platform/IME/clipboard/drag edge matrix, full S-04/S-05 action/lifecycle
  inventory, V shared geometry/revision/AX obligations and manual acceptance.
  Canonical ISO segmented editing does not establish platform UI completeness.

These checked rows identify generated implementation/caller/regression slices;
S-04, S-05 and V parent packages remain open. Validation is deferred until the
complete programme's source-generation gate closes.


## September 16 — temporal picker and localized presentation generation

Generated source only; no compilation, test, formatting or binding-generation execution.

- [x] Native date/month/week/time/datetime-local candidate lifecycle in
  `temporal_picker::{open_temporal_picker,temporal_picker_snapshot,move_temporal_picker}`.
  Calendar grids retain separate highlight/value state, localized titles/weekdays,
  native weekday calculation, month/year navigation, and exact current min/max/step
  admission through `control_numeric_validity_for_value`. Candidate validation is
  shared with the existing numeric algorithm, not a second approximate validator.
- [x] `HTMLInputElement.showPicker` IDL/native method now checks mutability,
  native top-origin relationship and transient activation before consuming it.
  Temporal controls open via the method, trusted pointer default or Alt+Down;
  native arrows/Home/End/PageUp/PageDown navigate; Enter commits, Escape/Tab
  dismiss. Commit sends beforeinput, rechecks current source/constraints/editor
  state after author callbacks, then native edit/input/change. Synthetic clicks
  cannot open/select. Non-temporal picker families are not implemented by this slice.
- [x] DOM top-layer `TemporalPicker` and layout-input native metadata feed
  `oryn_layout::temporal_picker::append`, which uses production layout/shaping for
  bounded private calendar/clock boxes. Generated hit targets retarget to the real
  control; no author DOM rows or script callbacks supply candidate authority.
  Production layout, paint inventory, hit stacking, print/backdrop/root placement
  were reconciled so popup ownership does not move the original input box.
- [x] Native `:open` and semantic/AX expanded state follow popup ownership.
  Focus changes, reset, type changes, outside pointer actions and disabled/inert/
  hidden/readOnly checkpoint fixup dismiss stale popup state.
- [x] `TemporalEdit` localized native field order/separators for en-US, en-GB,
  French and German, with canonical-order fallback. Native clipboard/IME/undo
  restoration decodes displayed text; public value, submission and OIL ISO
  replacement remain canonical. Language mutations retain field identity while
  refreshing presentation. Calendar labels include French/German month/weekdays
  and US Sunday-first versus Monday-first grids.
- [x] Regression source bodies generated:
  `calendar_candidates_are_constraint_bound_and_focus_reset_retires_native_state`,
  `native_picker_families_share_exact_candidate_validation`,
  `temporal_picker_native_keyboard_constraints_dismissal_and_paint`,
  `temporal_picker_pointer_uses_private_boxes_and_rechecks_native_candidates`,
  `localized_temporal_fields_keep_iso_values_through_native_keys_and_oil`, and
  `input_show_picker_uses_native_activation_and_mutability_checks`.
- [ ] Remaining implementation/assessment obligations: native AX virtual child
  rows/actions for popup candidates; complete locale/calendar/numbering-system
  data and native OS locale selection; full minute/second/time-period popup UI
  (current clock choices preserve the field editor's minute/second suffix);
  small viewport scrolling, transformed/vertical-writing anchoring, system
  appearance/high-contrast/platform visual/manual acceptance; file/color/datalist
  picker consumers; full S/V requirement/caller inventory and conformance matrix.

These are bounded production source slices, not closure of S-04 or any V parent.
The earlier wholly-missing temporal picker is superseded by this native path;
remaining named gaps must stay open. A source review also repaired the older
LiveLayoutProvider scrollbar regression fixture's missing native_media field.

## Native popup consumers and platform actions — 2026-09-16

The following supersedes the preceding slice's wholly missing AX, clock-phase,
color/datalist/file consumer gaps. Checked boxes mean generated production and
regression source inspected, **not compiled, executed or conformance verified**.

- [x] `TemporalPicker` retains a native session and draft. Date-time selection
  progresses calendar → hour → minute → optional second; time uses the same
  clock phases. Intermediate choices do not mutate the input value. Final choices
  share numeric min/max/step validation and post-beforeinput freshness checks.
- [x] RGB channel choices and paged native datalist suggestions reuse that
  candidate model, production pointer/keyboard/default-action consumers and
  private paint boxes. Added native `HTMLInputElement.list`, datalist `options`
  IDL/binding source and legacy color sanitization. Author DOM has no popup rows.
- [x] `NativeSnapshot` publishes native input/permission popup descriptors.
  `accessibility_platform::picker` creates private platform AX nodes with real
  paint geometry, common native stacking order and real owner scope. Existing
  macOS AXPress routes typed `TextCommand::Picker`/`PermissionPrompt` with exact
  expected state to native handlers. Stale rows cannot reenter a new session.
  Scoped AX selection/picker mutations are no longer classified read-only.
- [x] `oryn_layout::temporal_picker::row_window` bounds visible candidate rows
  to viewport height and follows native active selection; narrow title/navigation
  dimensions stay nonnegative. Keyboard navigation exposes further rows on the
  next production snapshot. This does not claim native wheel scrolling.
- [x] File input trusted click/keyboard and activation-gated `showPicker` create
  native `FilePicker` requests. AppKit asynchronous NSOpenPanel retains owner,
  document scope and exact pending request. Reset/type/disabled/retirement and
  accept/multiple changes invalidate stale completion and cancel/replace panels.
- [x] Parent `complete_native_file_picker` checks the live request, applies the
  existing file-upload policy before reading paths, and sends only selected
  bytes/name/type/mtime to the worker. Limits remain 128 files and 64 MiB. Native
  completion rechecks ownership, populates real File/FormData data, emits trusted
  input/change for changed selection or cancel for cancellation/same selection.
  No OS path enters the page worker. OS adapter failure is explicit.
- [x] Regression sources added:
  `clock_picker_native_accessibility_actions_choose_hours_minutes_and_seconds_atomically`,
  `datetime_picker_calendar_then_clock_retains_draft_until_commit`,
  `rgb_picker_and_datalist_use_native_candidate_actions_without_author_dom_rows`,
  `native_file_chooser_commits_real_bytes_cancels_and_rejects_retired_sessions`,
  `file_picker_requests_are_native_bounded_and_supersede_prior_owners`,
  `native_ax_popup_rows_press_real_owner_and_retired_rows_cannot_reenter`,
  `virtual_identity_includes_owner_and_popup_session`, and
  `candidate_window_tracks_native_active_row_with_bounded_height`.
- [ ] Remaining implementation/assessment: full locale/calendar/numbering-system
  data and native OS locale; modern color alpha/colorspaces and closed color-well
  appearance; datalist filtering/range-specific presentation and full list-state
  applicability; native wheel scrolling and transformed/vertical/RTL anchoring
  matrix; high contrast/system appearance; platform file-dialog manual acceptance
  and non-macOS adapters; full S/V producer/caller/requirement and conformance
  inventories. IDL generation, builds and all test execution remain deferred.

Source paths: `oryn-dom/{temporal_picker,file_picker}.rs`,
`oryn-js/v8_bindings/{temporal_picker,file_picker,native_accessibility}.rs`,
`oryn-layout/temporal_picker.rs`, native `accessibility_runtime.rs`,
`accessibility_platform/{picker,frames,mod}.rs`, and
`accessibility_platform/macos/file_picker.rs` (all under each crate's `src`).
This is bounded source coverage; S/V parent rows remain open.

## Native locale, range, datalist and popup edges — 2026-09-16

Source-generation follow-up, with no execution:

- [x] Authenticated `NativePlatformQuery::UiLocale` now supplies the parent OS UI
  locale. `native_locale::current` samples macOS CFLocale, Windows
  GetUserDefaultLocaleName, or Unix LC_ALL/LC_TIME/LANG with bounded normalization.
  Binding `ensure_ui_locale` caches only a successful native response; early
  setup before Window registration retries on subsequent native layout refresh.
  DOM `set_native_ui_locale` supplies fallback to inherited `lang`; canonical
  submission values remain unchanged. Existing full locale/calendar data is not
  implied by selecting the OS locale.
- [x] Native popup wheel input is intercepted before author wheel listeners,
  changes the bounded active row and leaves the public value unchanged. Pointer
  coordinates target private popup geometry; unrelated page wheels retain the
  existing cancelable page-event route.
- [x] Popup placement follows horizontal RTL and vertical/sideways block sides,
  flips when necessary and clamps to viewport bounds. Source inspection confirms
  layout `compute_with_styles` projects border boxes before these consumers, so
  owner/ancestor transforms are applied once, not reconstructed from author APIs.
- [x] Datalist lookup now excludes inapplicable input states; suggestions filter
  native value/label case-insensitively, omit disabled/empty options and reset
  page/highlight when the query changes. Native range datalist values become
  bounded, distinct, sorted valid step-aligned tick fractions.
- [x] Native range pointer press/drag/release now uses inverse paint projection,
  native writing direction and thumb dimensions, shared value sanitization and
  step rounding. Accepted mousedown controls default action; input fires on
  changes, change commits at release. Native touch taps use the same value path.
  `readonly` no longer incorrectly suppresses range keyboard stepping.
- [x] Typed `PaintControl::Color` paints the current native color well; typed
  `Range.ticks` paints datalist marks with existing native direction/projection.
  Production population uses live control values, not default value attributes.
  Author appearance:none suppresses these native decorations.
- [x] Added regression source bodies:
  `locale_normalization_keeps_region_and_rejects_malformed_native_data`,
  `native_locale_fallback_and_explicit_language_keep_canonical_values`,
  `datalist_filters_native_value_and_label_and_list_is_inapplicable_to_password`,
  `popup_follows_logical_block_side_and_inline_direction`,
  `range_pointer_drag_uses_native_projection_constraints_and_single_commit`,
  `native_popup_wheel_changes_highlight_without_author_wheel_or_value_commit`,
  `native_color_well_and_range_ticks_generate_typed_paint_commands`, and
  `range_ticks_use_only_valid_distinct_current_datalist_options`.
- [ ] Remaining confirmed scope: complete locale/calendar/numbering-system data
  (current field/month labels cover only the previously documented languages),
  modern color alpha/colorspaces,
  continuous touch slider gesture arbitration and tick-label presentation,
  platform appearance/manual acceptance and full visual/input matrix.

Platform source boundary: plan §2 names signed macOS as the primary current
runtime and demands separate evidence for every additional claimed runtime.
`accessibility_platform/mod.rs` exports its OS host only under target_os=macos;
`accessibility_host.rs` uses that AppKit host. This is evidence of the current
consumer boundary, not a claim that Linux/Windows picker obligations pass or are
inherently unsupported. Locale selection has cross-platform source adapters;
non-macOS native UI hosting remains unimplemented/unqualified as a larger host
requirement, not an isolated file-dialog adapter omission.

Whole-S/V reconciliation remains open: main checklist S-01 binding/DOM inventory,
S-02 reaction matrix, S-03 receipt/caller matrix, S-04 historical drag/input matrix,
S-05 OIL freshness matrix and V visual manifest mapping are broad unreviewed or
partially mapped requirements. These new slices do not justify closing them.

- [x] Follow-up datalist source reconciliation: multiple-email filtering uses the
  current last token and selection preserves preceding addresses. Email and URL
  options use existing native syntax validators; malformed options are omitted.
  Regression source: `multiple_email_suggestions_replace_only_last_token_and_reject_invalid_options`.

## Modern color, continuous touch and native locale data — 2026-09-16

- [x] Reconciled pinned WPT `html/semantics/forms/the-input-element/color.window.js`
  and `color-attributes.window.js` from checkout
  `wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0`. New `color_controls` uses the
  already locked native `color` 0.3.3 CSS4 parser/converter: named/functional/hex
  values, alpha, limited-sRGB quantization, Display P3 floats including out-of-range
  channels, missing-channel zeroing and canonical serialization. `alpha` and
  `colorSpace` IDL/native reflection update values; non-dirty content attributes
  are reparsed, while dirty state cannot recover discarded alpha. Picker alpha
  phase and live color-well projection consume the same model; FormData receives
  the native serialized value. The existing raster target is sRGB; full physical
  wide-gamut display qualification remains a separate presentation obligation.
- [x] Continuous native touch sliders retain touch/changed-touch lists and native
  input/change semantics while excluding the owned contact from page pan/pinch.
  Pointer/touch cancellation, disabled/retired targets and suspension retire range
  gesture state; release commits once. Existing touch-page panning remains on its
  original path for contacts not owned by a slider.
- [x] Added bounded `ControlLocale` data (12 Gregorian months, 7 weekdays, week
  start, numeric field order/separators and 10 decimal digits). Authenticated
  `UiLocaleData` fetches parent OS data via CoreFoundation date/number/calendar
  services on macOS and GetLocaleInfoEx on Windows. Gregorian calendar is explicit
  on macOS because HTML date values are Gregorian regardless of regional calendar.
  Native layout refresh hydrates up to 64 locale profiles; absence during Window
  registration retries without caching unauthenticated data.
- [x] Native locale profiles drive field display, UTF-16 selection offsets,
  localized digit keyboard edits/clipboard normalization, calendar labels and
  first weekday. Public value and form submission remain ISO/ASCII. No author
  Intl/global formatter supplies this data. Profiles validate dimensions and byte
  bounds; unsupported OS profile lookups retain the documented fallback and do
  not claim full locale coverage.
- [x] Regression source bodies added:
  `css4_input_colors_preserve_alpha_and_display_p3_and_resolve_missing`,
  `nondirty_color_attribute_recovers_alpha_but_dirty_color_does_not`,
  `modern_color_attributes_picker_and_formdata_share_native_color_space`,
  `continuous_touch_slider_retains_contact_events_and_does_not_pan_page`,
  `native_date_patterns_keep_order_and_unicode_separators`,
  `native_number_data_keeps_iso_values_and_utf16_selection`,
  `malformed_or_oversized_locale_profiles_are_rejected`, and
  `native_locale_digits_flow_through_production_keyboard_and_iso_form_values`.
- [ ] Remaining named scope: native Linux/other OS locale-data adapters (current
  lookup returns no profile there); non-macOS OS UI hosts; algorithmic numbering
  systems or multi-scalar digit formats rejected by the current decimal profile;
  complete locale date/time presentation/manual matrix; range tick-label layout,
  physical wide-gamut/system appearance and visual acceptance; complete S/V
  manifest/operation mapping. No build, test, formatter or generator executed.

These replace the earlier entirely missing modern-color/continuous-touch/native
locale-data descriptions. They do not close S-04, V-01, or any full inventory.

## Control operation/AX reconciliation — 2026-09-16

See [bounded operation inventory](S-V-CONTROL-OPERATION-INVENTORY-2026-09-16.md).
Source fixes connect OIL readonly applicability, case-normalized input actions,
nontext color input events, macOS AXColorWell, numeric increment/decrement and
live numeric min/max/value through shared native algorithms. AX adjustment uses
both legacy action names and modern native selectors, with scoped typed worker
commands; numeric NSNumber writes reuse the existing native value action. New
regression sources cover operation discovery, actual OIL replacement, adjustment
bounds/readonly, and numeric-property incremental/full agreement. No execution.
