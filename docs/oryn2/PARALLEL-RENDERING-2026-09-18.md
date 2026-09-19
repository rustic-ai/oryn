# Parallel rendering source generation — September 18

Status: source generation and inspection only. No build, test, formatter,
generator, WPT run, service or deployment was executed. This delivery does not
close V-01 through V-05 or the observer parent gate.

## Production matrix

| Concern | Production source / custody | September 18 disposition |
| --- | --- | --- |
| SVG descendant bounds, viewBox, nested viewports, foreignObject | `oryn-layout/src/svg_geometry.rs::project` | Existing usvg normalization and independent foreignObject formatting retained. Fixed repeated resolution of the outer SVG's percentage dimensions. CSS has already resolved that outer content box; only nested viewport dimensions resolve against it. A viewBox still replaces user space. |
| Per-tspan metric bounds | `svg_geometry.rs::text_element_bounds`, pinned usvg span source IDs | Already generated before this lane. Inspected and preserved; not counted as new implementation. |
| ResizeObserver callback custody | `oryn-js/src/v8_bindings/resize_observers.rs::{construct,broadcast}` | Removed permanent native callback Global. Observer wrapper now retains callback through a private JS edge; native observation retains the wrapper while active. An idle callback capturing its own observer is collectible. |
| IntersectionObserver callback/root custody | `intersection_observers.rs::{construct,geometry,get,poll}` | Callback and explicit root retained through private JS edges. Native root handle is weak. Active/pending work keeps observer alive, which keeps these private edges alive; idle root→observer and callback→observer cycles no longer have those permanent native roots. |
| ResizeObserver target retirement | `resize_observers.rs::{sample,needs_render}` | Retained target depth is computed independently of rendering eligibility, so a retired target's zero geometry is eligible at initial depth threshold. Dependency comparison no longer excludes retired owners. Root integration must include lifecycle changes in the native paint revision. |
| Observer rendering wakeup | `v8_bindings.rs::native_paint_revision`, `set_layout`, provider `layout_revision` | Shared-file integration finding sent to root: revision omits provider revision/DPR/lifecycle and set_layout does not dirty observer queues. Font/image-only geometry updates can therefore be missed by dependency wakeups. Root owns integration. |
| External SVG clips | `svg_clip_external.rs::resolve_with_space` | Current successful-resource origin/MIME/budget/cascade checks retained. UTF-8-only decoding remains a confirmed gap; PaintImageResource does not retain response charset. Source-owner proposal sent to root/loading lane. |
| AX input coherence | `accessibility_runtime.rs::accessibility_snapshot_prepared`, `accessibility_resolved.rs`, native semantic projection | Existing identity/revision/text guards preserved. No claim of exhaustive accessible-name/ARIA/OS behavior review from this lane. |

## New regression sources

- `svg_viewport_percentages_use_the_resolved_outer_box_once`: outer percentage
  dimensions, nested percentage dimensions and a viewBox user-space replacement.
- `inactive_callback_cycles_collect_and_active_observation_keeps_callback_alive`
  in each observer module: idle callback cycles, active native retention and
  collection after disconnect. Uses authored V8 GC calls, none executed.
- `retained_retired_target_keeps_depth_and_reports_zero_geometry`: native retained
  target before/after execution retirement; zero dimensions retain positive depth.
- Existing pending-entry preparation/retry, author callback forgery, disconnect
  pending delivery, document retirement and tspan regression sources remain intact.

## Remaining source obligations

1. Integrate shared provider/layout/lifecycle invalidation hooks and regression
   sources in root-owned V8 bindings, plus loading provider revision producers.
2. External SVG response decoding must honor BOM, response charset and XML
   declaration without introducing network access in geometry. Reuse a pure shared
   decoder; do not substitute decoded text re-encoding for raw-byte evidence.
3. External SVG stylesheet/font/subresource admission must retain the external
   document's base URL, policy, cascade and font owner; embedding document fonts
   cannot be copied into an independently fetched SVG document.
4. Complete cross-language lifetime review for retained target/entry edges,
   including retired documents, cross-realm wrappers and root/target cycles. The
   callback/root fixes above do not prove all native platform records collectible.
5. Full SVG input/selection projection, transformed clip edge cases, complete
   manifest-to-algorithm inventory and accessible-name/ARIA caller inventory stay
   open. Existing module presence is not proof of conformance.
6. After the global generation gate: compilation, runtime failure-recovery and GC
   regressions, full pinned Core and five-family visual/manual/platform evidence.

## SVG ResizeObserver content origin follow-up

`resize_observers.rs::sample` now distinguishes SVG graphical targets without a
CSS layout box from outer SVG roots and SVG roots inside foreignObject. Graphical
content rectangles retain their object bounding-box x/y; CSS boxes retain their
content-to-padding offset. Existing native local paint projections supply
untransformed dimensions. Added `svg_graphics_content_rect_keeps_object_space_origin`
regression source. This does not close CSS zoom/device-pixel geometry: pinned
`resize-observer/svg.html` test17 explicitly requires zoom-dependent device sizes;
that producer remains a separate source obligation.

## Approved integration follow-through (supersedes earlier hook-only status)

The provider/layout/lifecycle and XML decoding gaps listed above now have source
implementations and regression sources:

- `oryn-common/src/xml_encoding.rs` owns the existing XML declaration/signature
  scanner extracted from `oryn-net`, plus strict complete-resource decoding.
  BOM overrides extracted HTTP charset, which overrides XML declaration; duplicate
  Content-Type fields retain Fetch combination/extraction semantics. Malformed
  encoded input fails instead of injecting replacement characters. Net delegates
  its existing scanner to this pure module; no network dependency enters layout.
- `PaintImageResource.content_type` is optional, serde-defaulted metadata alongside
  the unchanged normalized media type. CSS-image streaming, root paint fetch and
  selected-frame paint fetch preserve successful response Content-Type fields;
  synthesized resources explicitly omit it. All Rust literal sites were reconciled
  by source inspection. Protocol constants/lockfiles were not regenerated.
- CSS SVG completion decodes a temporary parser input with this metadata and keeps
  original response bytes in admitted resources. Dependency discovery uses the same
  decoding. Layout external clips, render-core SVG trees, semantic instances,
  foreignObject/coverage, filters and masks consume the retained encoding metadata.
  Render-only parser buffers normalize to UTF-8 and remove their byte-encoding
  declaration before repeated parser use; retained response bytes remain intact.
- Native paint revision now includes provider revision/device scale and document
  generation/stream/authority/retirement/suspension. Explicit `set_layout` clears
  its prior environment and marks active IO/RO sampling dirty.
- `LiveLayoutProvider` now compares the exact independent DOM, animation, font,
  viewport, DPR, scrollbar and media tuple, advancing a checked monotonic cache
  revision. Font publication advances its own revision after successful admission;
  cache revision exhaustion disables caching. Animated layout remains uncached.
- Stylo's actual inherited `effective_zoom` is carried as private native style
  metadata. SVG graphical RO device-pixel sizes apply it while content/border
  dimensions remain object-space measurements. This closes the identified SVG
  device-size producer gap; general zoom/layout/paint acceptance is still unrun.

New source cases include
`native_observer_environment_tracks_provider_scale_lifecycle_and_direct_handoff`,
`native_provider_revision_tracks_independent_font_media_and_viewport_inputs`,
`xml_bom_header_and_declaration_have_native_precedence`,
`external_clip_accepts_native_xml_encoding_without_network_or_page_charset`,
`css_svg_retains_original_bytes_and_http_charset_through_completion`,
`captured_svg_decodes_charset_bom_and_canonicalizes_only_parser_input`,
`svg_native_zoom_metadata_comes_from_the_computed_ancestor_cascade`, and
`svg_device_pixels_follow_native_effective_zoom_without_changing_object_dimensions`.

The automation lane's native microphone control is also reconciled into layout:
locked control eligibility, private labels, UA styling, generated permission icon
and microphone-specific prompt caption. The existing private-label regression
matrix now includes microphone. This does not assert exact platform artwork or
manual permission acceptance.

Still open: external SVG independently owned CSS/font admissions; complete SVG
selection/input and transformed clipping edges; retained target/entry cycle
inventory; whole-manifest visual and AX caller reconciliation; all execution.

## Processing-mode and AX reconciliation follow-through

Source generated; not built or executed. This section supersedes the preceding
blanket external-SVG CSS/font network-admission item. SVG Integration's
[referencing modes](https://svgwg.org/specs/integration/#referencing-modes) assign
both image documents and external resource documents secure processing modes:
external references are disabled, including CSS and fonts. Top-level/embedded
active SVG documents remain a separate native document-loading path. Loading a
page's font/image inventory into a fetched SVG is therefore incorrect isolation.

`PaintImageResource.svg_document_snapshot` now distinguishes native inline SVG
serialization from fetched SVG (serde default false). Only inline SVG capture
and inline drag capture set it true. Render-core loads page fonts and resolves
admitted string image references only for this native mode; nested fetched SVG
stays secure. Data images remain bounded and recursively secure. CSS image
completion retains the fetched root but issues no nested image/CSS/font request.
This uses existing async native resource ownership and introduces no synchronous
IO. Source-mode fields on Rust literals and derived resources were reconciled;
protocol/schema generation is still deferred with all other generation execution.

| Caller or algorithm | Positive source coverage | Negative source coverage | Remaining limit |
| --- | --- | --- | --- |
| CSS fetched SVG completion | Successful redirected root retained with Content-Type and original bytes | Nested feImage, CSS import and font-face URL produce no request | No executed stream/conformance evidence |
| Native inline SVG rendering | Captured relative image and text use document environment | Fetched copy cannot borrow even an already-admitted page image | Independently embedded data-font CSS aliases/descriptors still need implementation/reconciliation |
| Intrinsic SVG drag | Secure unavailable image renders absent content | Missing admission on native serialization remains a capture error | All raster assertions unrun |
| External URL renderer | Secure file/network references stay unavailable | Inline missing admission errors without opening any path | Secure foreignObject's independent HTML formatting remains incomplete |
| AX naming and published ownership | Same accepted map; DOM order after reparent; visible competing owner wins | Hidden/disconnected owners or targets, duplicate owners, direct and mixed DOM/ARIA cycles rejected | No whole-manifest acceptance claim |
| AX mutation rebuilding | Reparent, hidden-owner change, ownership removal and native ElementInternals references compare incremental/full projections | Rejected cycle never becomes a published edge | All regression sources unrun |
| Slot naming | HTML slots distribute assigned light content | Foreign namespace slot lookalikes retain own children/ARIA naming | Broader SVG/MathML implicit-role and host-language name mappings remain to reconcile |

New source cases: `fetched_filter_svg_does_not_admit_external_images_stylesheets_or_fonts`,
`svg_processing_mode_distinguishes_disabled_requests_from_missing_native_admissions`,
`aria_ownership_names_and_published_edges_share_document_order_after_reparenting`,
`aria_ownership_rejects_cycles_with_interleaved_dom_and_explicit_edges`, and
`only_html_slots_distribute_light_content_or_ignore_aria_labels`. Existing inline
SVG and ElementInternals cycle fixtures now assert the same production mode and
accepted ownership policy as their callers.

Still required before V closure: independently embedded SVG font-face support
(including aliases, descriptor selection, conditions and fallback), secure
foreignObject native layout, full SVG input/selection and transformed clips,
remaining observer target/entry lifetime inventory, every pinned AX caller's
native mapping and mutation matrix, then all global build/test/conformance gates.
The completed fixes above do not make those unchecked rows complete.

SVG accessibility direct-child title/desc alternatives now also feed both the
public node semantics and referenced/content text traversal. ARIA label and
description precedence is preserved; nested titles and foreign namespace
lookalikes cannot supply the root's title. HTML alt/input-default/legend/caption
host-language alternatives are namespace-qualified. The new
`svg_direct_title_description_and_aria_precedence_share_name_callers` source
covers title text mutation, dependent labelled control, incremental/full graph
comparison, and ARIA overrides. This still does not reconcile all implicit
SVG/MathML roles or complete the pinned AX inventory.

### Embedded SVG faces and pinned usvg shaping source

Added `oryn-layout/src/svg_fonts.rs`, reused by external clip, SVG geometry,
clip glyph extraction and renderer. Native CSSOM/media/supports parsing admits
bounded data font candidates only, with ordered source fallback and decoded-font
validation. Faces retain CSS aliases, descriptors and Unicode coverage separately
from intrinsic font metadata. An added optional resolver callback in the pinned
usvg 0.45.1 text engine selects complete grapheme clusters; native CSS descriptor
ranking and unicode-range eligibility choose their admitted face. Bidi visual
runs are retained across adjacent face runs and byte offsets stay in original
text coordinates. The default usvg resolver path remains unchanged when no
native faces are installed. The nested image resolver forwards this callback.

Vendor source provenance: existing repository `vendor/usvg` 0.45.1, upstream
`linebender/resvg`; additive edits in text/mod.rs, text/layout.rs and forwarding
literal in parser/image.rs, plus unicode-segmentation dependency. Fontdb API was
read from upstream RazrFalcon/fontdb v0.23.0 src/lib.rs (load_font_source,
face cloning, push_face_info); no downloaded executable or generated output.
Regression sources cover media/supports exclusion, network/local source rejection,
data fallback and percent decoding, alias registration, weight selection,
Unicode-range splits and combining-cluster eligibility. No execution occurred.

### Secure foreignObject source formatting

`svg_secure_foreign.rs` now builds a private namespace-preserving LayoutInput
from the SVG XML, matches its own sheets, admits only embedded fonts/images,
and runs the existing native foreignObject subtree formatting and paint-font
projection. Incoming capture-identity attributes are discarded. Native capture
records are generated privately and used by the existing vector/raster and PDF
semantic-text callers; the embedding snapshot's DOM/fonts/frame/capture inventory
is not consulted. Root viewBox dimensions provide percentage bases. Source tests
cover native box painting with a resource-local stylesheet, forged capture IDs,
and an unavailable external background preserving the underlying box color.
All code and tests are unexecuted.

Remaining precision work is explicit: mixed allowed/disabled CSS image layers
currently suppress the whole image property; nested inline SVG inside secure
foreignObject needs recursive private capture; variable font axis/metric overrides
are not yet propagated through usvg glyph rendering; the whole AX pinned inventory
is still not reconciled. These are source gaps, not conformance passes.

Observer public-entry target retention is now represented by a private JS edge.
Queued native entries retain a strong target until successful delivery; published
entry records drop that native root while their wrapper privately retains the
same target identity. This closes entry -> target -> entry cycles without
weakening pending records or allocation-failure retry. Both IntersectionObserver
constructor entries and sampled entries use this representation; RO delivery
uses the same ownership transition. Added unexecuted GC regression sources for
constructed IO entry cycles and delivered RO/detached-target cycles. Retired
observer queues still need their complete retained-wrapper ownership audit.

Secure foreignObject follow-through also serializes nested inline SVG with
namespace-qualified XML and native computed styles, preserving its private
resource document mode, and suppresses duplicate descendant paint. Mixed CSS
image declarations preserve unaffected layers (per-image-set candidate fallback
and failed-mask semantics remain separate source obligations).

Retirement follow-through: both native observer registries now release target
observations when their owning execution retires. IO queued entries retain weak
native target references paired with a private queue-target array on the observer
wrapper. Sampling publishes that retention before committing threshold updates;
`takeRecords` and delivery transfer it to public entry wrappers. A retained
retired observer can still read its existing records, while an unreachable
observer/target/queued-record cycle is collectable. `observe` after retirement
does not leave native target roots. Added a retirement queue-cycle GC source.
These updates supersede the specific retired-queue root gap above; their behavior
still requires the deferred execution gates.

Secure CSS image used values now inspect the native density-selected image-set
candidate before deciding external unavailability. Unaffected layers retain
original source/resolution syntax. A disabled mask layer becomes transparent
black coverage (expressed as a transparent gradient), preserving mask failure
semantics rather than removing the mask. Added an unexecuted transparent-mask
regression. Nested secure SVG serialization also has a regression covering
ancestor-dependent computed fill transferred into the private nested capture.
These supersede the corresponding nested/layer/mask source gaps above; variable
font axis/metric propagation and the full pinned AX reconciliation remain open.

Variable-face follow-through adds optional native `FontFaceSettings` to pinned
usvg. CSS descriptor-derived axes/features/size-adjust/ascent/descent/line-gap
travel with resolved fonts and positioned glyphs. The shaper sets design axes
before glyph shaping; native metric reads and outline/raster/SVG/COLR face reads
apply the same axes. Adjusted glyph scale remains in native cluster transforms,
so PDF text extraction continues to use the already positioned source ranges and
actual outline coverage. Family matching preserves author family order and keeps
private aliases out of fallback; canonical-equivalent grapheme coverage is
checked before selection. Native text/flatten.rs and nested resolver forwarding
are part of the recorded vendor patch; no source text is rewritten per fixture.

New source cases compare variable P outlines at weights 100/900 against the
pinned font's independent slash/comma reference glyphs and check metric/feature
settings plus doubled outline width under size-adjust. Fixture provenance remains
`crates/oryn-render/tests/assets/VARIABLE-FONT.md`. No execution. Remaining review
includes descriptor/element feature precedence, language override and full pinned
AX callers; this does not assert complete V conformance.

Element/descriptor precedence follow-through: inherited SVG feature, variation
and language values now enter native Font records and text-preserving writing.
Descriptor features apply before variants/kerning; element feature settings apply
afterward. Explicit element axes override descriptor defaults. OpenType language
overrides use Rustybuzz's documented source-level private language tag path
(`x-hbot...`, checked against rustybuzz v0.20.1 src/hb/tag.rs), reaching shaping
rather than remaining metadata. Added a source regression for inherited element
settings, face defaults, language tag and SVG round-trip preservation.

Vendor patch additionally touches parser/text.rs, writer.rs, tree/text.rs,
parser/svgtree/mod.rs, and hand-maintained explicit extensions around names.rs's
unchanged generated perfect-hash table. No generator/build/test was run.

## AX caller reconciliation matrix (source, not execution)

The pinned inventory remains 461 source/helper records. Its existing per-manual
markup reviews are retained; this pass independently traced all distinct AXAPI
assertion names into the actual AppKit publisher, alongside direct name/role
bindings and native selection/readonly/modal callers. Counts below are assertion
occurrences in that inventory, not runnable tests or passing cases.

| Inventory assertion family | Source caller and owner | Source disposition |
| --- | --- | --- |
| AXRole (220), AXSubrole (152), AXRoleDescription (150) | semantic computed role → role_mapping → macos attribute | Existing native widget/landmark/synonym/protected-text mappings retained; authored descriptions filtered for generic roles |
| AXDescription (145), AXHelp (14), placeholder/value description | semantic accessible_semantics → native Node → macos attribute | Shared label/reference/slot/SVG title/desc algorithms and source precedence; native attributes publish names/descriptions |
| AXValue settable (42), AXFocused settable (6), actions (18) | semantic actions/states → macos settable/actions → Snapshot::validate_action | Published AXValue now also rejects Disabled explicitly; native execution validates current revision/state |
| Orientation (31), current (9), required (9), busy (7), selected (11), expanded (14), focused (8) | role_mapping defaults and native projected state → macos attribute | Defaults/explicit values and virtual focus use actual published state |
| Numeric min/max/value and validation | semantic/native control property merge → role_mapping numeric_bound/toggle_value | Shared numeric/checkable/text/protected value producers retained |
| Row/column counts, indices and ranges | semantic structures → accessibility_platform/table → macos attribute/parameter | Native table occupancy, explicit spans, headers and published object IDs retained |
| Set size/position | semantic properties → macos numeric ARIA attributes | Explicit values including unknown set-size remain typed |
| Linked elements, selected descendants and label references | native accepted ownership map / explicit relations → published identities | Selection and embedded selected-name traversal now use identical accepted ownership; readonly inheritance and aria-modal scope use accessibility ancestors |
| AXServesAsTitleForUIElements (advertised native attribute) | native published LabelledBy reverse relation → macos attribute | Missing getter added; identity and paint-order source regression, no author-ID string matching |
| autocomplete (5), unchecked menu mark (1), absent attributes | macos unsupported-attribute null return | Inventory explicitly expects nil for these cases; no fabricated value |
| Accessibility inclusion/hidden and events (6) | semantic exclusions → platform snapshot → role_mapping/live publication | Existing native exclusion/event owners retained; owned modal descendants now remain in accessible scope |

New regression `ownership_is_shared_by_selection_readonly_and_modal_callers`
checks competing owners, selected option traversal, reparented grid readonly,
owned modal content and ownership-removal mutation against full graph rebuild.
`reverse_labels_follow_published_identity_and_document_paint_order` covers
multiple controls, removed relations and retired generations. These caller fixes
were necessary despite earlier per-file coverage notes. No full source-manifest
or platform execution pass is claimed; all execution remains globally deferred.


## Per-embedding fetched SVG media closure — source generated, unexecuted

The previously recorded hardcoded light/no-motion gap is now implemented. This supersedes that source gap, not the pending build/conformance verification.

- `oryn-layout/src/lib.rs` projects the actual native preferred scheme and reduced-motion state alongside each computed style, including pseudo styles, and publishes computed `color-scheme`. Existing layout media/font generation invalidation remains the producer.
- `oryn-render-core/src/svg_environment.rs` defines renderer-local serializable consumer context: used color scheme, reduced motion, CSS image viewport, DPR. `normal` supplies the default light used scheme; explicit supported schemes negotiate the native preference. No environment is attached to globally fetched bytes or common resource metadata.
- SVG image, background, canvas, cross-fade, mask, filter and backdrop commands retain their owning consumer environment. SVG tree and raster caches and PDF original-effect tree caches include all environment fields. Print vector and semantic paths enter the same lexical context; nested parsing restores prior context on exit/error.
- Fetched SVG resolves its independent native CSS cascade before vector conversion, removes re-evaluated stylesheet text and serializes used computed values. Embedded font faces use original stylesheets under the same media environment. SVG viewBox/user geometry is distinct from its CSS media viewport; foreignObject uses the independent native cascade and used geometry. External resource access remains disabled.
- Vendor provenance: additive optional `ImageHrefResolver::resolve_svg_viewport` in vendored usvg 0.45.1 `parser/image.rs`, exported via `parser/mod.rs`; default is absent. The hook re-resolves only SVG images after concrete width/height are known and carries inherited `color-scheme` through an explicit `AId` extension (generated hash tables unchanged). Renderer callbacks use secure data/captured admissions and the existing bounded native parse guard. Existing default resolvers retain upstream behavior.
- Reconciled existing `reference_box` schema discrepancy: all prior usages were on `Filter`, but the field declaration was on `BackdropFilter`; declaration now matches filter usages, with serde defaults.

Authored positive/negative regression sources cover same URL and same bytes in different owners, light/dark negotiation, reduced-motion changes, viewport and DPR cache identity, CSS media viewport versus viewBox/raster size, lexical context restoration, and nested identical data SVGs with contrasting concrete viewports/schemes. Existing secure resource denial and foreignObject sources remain. No tests, builds, formatters, generators or services were run. These source additions are not a WPT pass claim; exact native cascade/font/geometry integration and pixel comparisons require the global verification phase.

External clip geometry callers also now pass the embedding computed style into the independent resource resolver; used scheme and actual native reduced-motion apply to its cascade and embedded font selection. Added same-resource light/dark/motion shape coverage source. Its geometry reference space remains independently resolved by the existing clip algorithm.

## Final six-row V source-gate reconciliation — 2026-09-18

This appendix is the final source disposition for the six granular V rows in the
shared generation checklist. It supersedes earlier dated “remaining source”
statements in this report where the implementations are described below. It does
not mark build, regression, WPT, visual, OS or assistive-technology qualification
complete. The root agent owns the checklist edits.

### Definitive source dispositions

| Open granular row | Source disposition | Reviewed producer → actual caller chain | What remains |
| --- | --- | --- | --- |
| `V.svg-variable-fonts` | **Source complete** | `oryn-layout/src/svg_fonts.rs` selected-face/whole-cluster resolver supplies `FontFaceSettings`; pinned `usvg/src/text/layout.rs` applies axes, feature precedence, language, size and metric adjustments; `text/flatten.rs` passes the same face settings to outline, raster, SVG and COLR; positioned glyph/source ranges remain available to PDF extraction. Parser/tree/writer preserve inherited element settings. | Compile and execute variable weight, descriptor precedence, metric/size, fallback/unicode-range and PDF/paint regressions. No remaining required source branch identified. |
| `V.geometry-observer-source-closure` | **Source complete** | Private IO/RO records sample native layout and composed paint geometry; `svg_geometry::project` supplies descendant/tspan/user-space/viewport/foreignObject geometry; native document-tree sampling reaches `native_presentation.rs` after child geometry. RO delivery/stamp changes request another layout before scene acceptance; IO sampling follows stable RO delivery. Native retirement clears scheduler roots; parked roots retain deliveries; private edges and weak handles release idle/cyclic observer custody. | Execute callback stabilization, loop error, tree ordering, SVG/device-size, disconnect/retirement, cross-realm and GC regression sources, then Core results. Earlier per-tspan, XML encoding, external SVG media/font and provider wakeup source gaps are superseded. |
| `V-01` input/form transitions and selector reconciliation | **Source complete** | DOM applicability/state (`selector_states`, numeric controls, native form/control owners) → `LayoutSelectorState` → Stylo. Native pointer/keyboard/picker/AX, submission, reset, history restoration and custom-element changes publish native transitions. `USER_VALID/USER_INVALID` remains restricted to applicable built-in controls; no FACE expansion is used to manufacture coverage. | Execute selector/form transition regressions and full selector conformance. DOM/input owner confirmed no remaining bounded native state transition after FACE/validity/input/reset/submission review; renderer applicability and typed state transport retain the existing built-in/FACE distinction. No exact missing required selector producer/consumer branch remains identified. |
| `V-01/V-02/V-03` visual inventory and algorithms | **Source complete for the declared Core visual inventory and its routed algorithm families** | All 537 pinned Core HTML reftest/print-reftest rows have source identity, references and nonempty algorithm routes. Final family boundary review below connects those routes to native implementations and the delivered dependency fixes. Direct Core selectors/CSSOM/cascade and animation/control helper obligations retain their separate source audits. | Actual expected pixels, print pages and helper-driven behavior are qualification. The JSON's lexical routing is an inspection index, not itself proof of implementation or pass results. No required missing algorithm branch identified by this reconciliation; this is not a claim about every CSS/SVG WPT directory. |
| `V-04` shared layout inputs/invalidation | **Source complete** | `LiveLayoutProvider::layout_revision` compares independent DOM, animation, font, viewport, DPR, scrollbar and native media inputs. `refresh_layout` adds custom state and document/element scroll inputs; `native_paint_revision` adds lifecycle and visual viewport; explicit `set_layout` invalidates IO/RO. Font publication increments native font revision; retained CSS image/font completion notifies the owning DOM. Geometry, native hit testing/focus/OIL, resource capture and AX consume the same native provider/projection and stamp checks. | Execute async publication, style-only changes, frame/viewport/scroll/media, animation, lifetime and coherent capture regressions. No remaining confirmed input producer or consumer bypass. |
| `V-05` pinned name/ARIA/ownership/slot callers | **Source complete** | The retained 461 source/helper inventory and all distinct AXAPI assertion families are reconciled above. Node-aware roles and shared name/description/accepted ownership/slot traversal reach semantic projection, direct computed-label/role callers, typed native action validation and AppKit publication. The final ownership selection/readonly/modal and reverse-label getter gaps have production fixes plus regressions. | Execute incremental/full-rebuild and marker/cross-document regressions; perform explicit native OS/AT procedures. Source reconciliation does not claim manual or platform acceptance. No remaining pinned assertion getter or shared-name/ownership branch identified. |

### Core visual family boundary review

Counts below overlap because a source can require several algorithms. Their union
is the 537-row inventory; no row has an empty route list. The final review checks
native caller boundaries and the concrete missing dependency fixes already
recorded in the layout, control, animation and rendering audits, rather than
promoting the mere existence of an algorithm module.

| Routed family | Rows mentioning family | Native boundary and delivered source |
| --- | ---: | --- |
| Replaced elements and frames | 25 | Native intrinsic/selected resource state → layout replaced boxes → frame composition; admitted SVG/image geometry and descendant resource lifecycle preserved. |
| Print/cross-document composition | 3 | `print::compute`/`print_continuation::compute` → print display-list/frame composition; variable page dimensions, continuation tokens, repeated table groups and UTF-16 boundaries. |
| Paint effects | 140 | Computed native paint projections → render-core clips, masks, filters/backdrops and SVG; owner media/viewport/DPR follows each consumer and all relevant caches. |
| Scroll/overflow | 22 | Native scroll offsets and overflow clips → layout projection, hit testing and captured scene; geometry and explicit native viewport changes invalidate the same input tuple. |
| Direction/text | 150 | Stylo direction/writing mode → Parley/native vertical text, text transform and source spans → paint, selection and AX text projection. |
| Position/top layer | 134 | Native positioning/anchor containing blocks and top-layer promotion → paint/hit ancestry; parent transform/clip escape preserves the promoted element's own clip. |
| Base style/text layout | 55 | Native namespace/scoped Stylo cascade and CSSOM → block/inline layout, shaping and paint; shadow selector ancestry remains distinct from formatting ancestry. |
| Fonts | 129 | Retained independent font admission/display timelines → selected native face shaping; HTML and secure SVG use their own font owners with descriptor/axis/feature/metric handling. |
| Native controls | 226 | DOM control/picker/interest state → native selector metadata/private generated boxes → paint and input/AX; dirty/user-validity/default state remains native. |
| Flex/grid | 19 | Computed Stylo values and intrinsic contributions → actual Taffy child placement and native text/replaced measurement; resulting geometry feeds the same projection consumers. |
| Generated content/markers | 107 | Scoped counters and pseudo-style/content generation → native text/marker placement; marker provenance and explicit-empty/removal behavior reach accessible names. |
| Table/ruby | 1 | Native table occupancy/span/alignment and ruby/text shaping → geometry/paint; native AX header relationships use published identities and actual table occupancy. |

The earlier report phrases “full SVG input/selection projection” and “complete
manifest inventory” did not identify an additional unimplemented required branch.
The relevant declared callers now use the shared native SVG geometry/text
projection and the mapped visual/native input owners. Broader unselected SVG API
or CSS feature expansion is not a residual source obligation of these six rows.

### Final V-01 owner cross-check

The DOM/input owner confirmed no remaining bounded form/input selector transition
in the delivered FACE, validity, input, submission or reset source. That cross-check
uses the FACE submission custody audit (25 pinned fixtures), historical drag/input
source attribution (180 source paths/72 caller paths), and G operation inventory
(1,516 occurrences); those counts are distinct audit scopes, not selector pass
counts. Renderer review separately retains native namespace/shadow applicability,
required/optional and range applicability, default checked/selected/form-owner
order, built-in user validity, target/open/defined/custom-state projection and
Stylo consumption. No author compatibility flag replaces these native states.

**Final generation recommendation:** all six rows have delivered source and
regression bodies with no confirmed remaining required code branch from their
pinned caller reconciliation. Mark their generation portions complete; retain
build/test-fix, full Core/five-family and platform/manual qualification as open.
No test, build, formatter, generator, service or conformance run was executed in
this final reconciliation.
