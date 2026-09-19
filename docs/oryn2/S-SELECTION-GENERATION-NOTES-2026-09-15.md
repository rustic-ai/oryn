# Native Selection movement source-generation notes

September 15, 2026. All changes below are **unbuilt and untested**. No build, test,
formatter, generator, installation or browser execution was performed. This is
not a Core conformance claim or a closure of packages S/V.

## Reviewed inputs and geometry contract

Reviewed the pinned `selection/caret-position-should-be-correct-while-moveup-movedown.html`,
related line navigation in flex/grid, `modify.tentative.html`, paragraph editing
boundaries, Korean/Latin word movement, symbol movement and bidi line boundaries.
The original binding only incremented/decremented one UTF-16 code unit inside the
focus container and rejected every other granularity.

The layout owner confirmed `TextClusterGeometry.rect` is physical pre-transform
viewport geometry, `matrix` carries paint/scroll projection, and `line` is local
to `layout_node`. Vertical/sideways block progression and inline reversal must use
computed writing mode rather than RTL alone. Layout generation is independently
unverified; binding tests supply explicit geometry to exercise the contract.

## Generated source

- `crates/oryn-js/src/v8_bindings/selection_movement.rs`: native document/tree and
  editing-host bounded movement. Shaped cluster endpoints and grapheme boundaries
  supply caret positions. Line/paragraph movement uses physical block progression,
  preserves the inline coordinate across short lines, and replaces the Selection's
  Range while preserving retained old Range wrappers. Explicit selection changes
  reset the remembered coordinate. Invalid options and empty selections are no-ops.
- Character movement uses Unicode graphemes and crosses DOM/BR boundaries, rather
  than splitting surrogate pairs or combining sequences. Word movement uses Unicode
  segmentation, grouped symbol runs and Hangul/letter transition boundaries.
  Line/paragraph/document boundary routes are generated as well.
- `v8_bindings.rs`: native dispatch, coordinate-revision state and regression
  sources for horizontal/vertical/sideways noncollapsed movement, leading collapsed
  source whitespace offsets, forward/backward move/extend, retained Range identity,
  short-line recovery, explicit-collapse reset, detached/hidden content, graphemes,
  word/symbol boundaries and invalid/no-selection behavior.
- JS crate adds exact `unicode-segmentation = "=1.12.0"`, matching the existing
  workspace layout dependency. Dependency resolution/lockfile updates have not run.

## Coordinated native serialization accessors

At the parent/script agent's request, `v8_bindings.rs` now exposes:

- `native_node_identity`: strict native internal-field types, realm/registry owner,
  document generation, Node brand and canonical-wrapper validation. Ordinary or
  Proxy values return None; recognized stale/retired wrappers error. No author
  properties or Proxy traps are read. Actual node document/context authority is
  still the caller's responsibility, especially after adoption.
- `native_node_collection`: native NodeList/HTMLCollection contents via the
  branded store (`node_lists.rs`), without public length, indices or iterators.
- `native_collection_values`: genuine DOMTokenList strings and native FileList
  entries, without public iterator/property access or additional file grants.

Regression sources cover foreign owner/ID collision, poisoned getters/proxies,
retirement, static/live node collections and token/empty-file collection snapshots.
The script owner is integrating these into native WebDriver result cloning.

## Remaining limits and required integration

- Actual layout-generated vertical caret movement still requires subsequent native
  integration and pinned-WPT execution. No test here proves the full layout path.
- Bidirectional visual character movement, sentence movement, complex orthogonal/
  rotated line navigation, cross-node grapheme shaping and complete ligature caret
  positioning need further work. Intra-cluster caret positions are interpolated
  when shaping supplies one cluster for multiple graphemes.
- Paragraph handling recognizes rendered block boundaries and BR segments; all
  preserved-newline/empty-line/atomic inline and editing-boundary variants are not
  yet exhaustively implemented or reconciled. Text-control internal selections and
  keyboard word movement are separate native editing paths still requiring review.
- CSS visibility overriding, user-select policy, collapsed whitespace character
  navigation and complete shadow/composed-tree selection behavior remain broader
  dependencies. This implementation deliberately does not fabricate line positions
  when native shaped geometry is absent.
- Full S/V production-caller reconciliation, remaining APIs, all regression sources,
  tests and complete frozen-candidate qualification remain open under the main plan.

The print host also requested `dispatch_print_event`: generated source accepts only
`beforeprint`/`afterprint`, rejects retired state, creates a trusted native Event
with non-bubbling/non-cancelable flags, and dispatches through native global
listeners. Tests poison public constructors/dispatch methods. The host remains
responsible for root/descendant traversal and generation checks after callbacks.

## Continuation: projected lines, sentences and native controls

Additional source generation now provides Unicode UAX #29 sentence movement and
sentence-end movement excluding trailing spaces; shared segmentation is reused by
focused native text controls. Text-control operations update the native selection
map and queue native select tasks without modifying the value or author fields.
They cover grapheme/word/sentence movement, move-versus-extend, noncollapsed-range
collapse, document boundaries, hard paragraph boundaries and invalid requests.

Line candidates are compared in the source line's projected inline/block basis,
including rotation and orthogonal destination axes. Visual character movement
uses resolved cluster positions rather than the paragraph direction alone;
noncollapsed visual movement chooses the physical range edge on the current line.
Preserved LF/U+2028/U+2029 delimiters increment paragraph identity across text runs.
Layout reports existing delimiter clusters with real zero-inline-advance geometry,
which the selection code consumes. It still cannot materialize a truly empty
block without native layout caret records.

Additional regression sources cover rotated basis projection, orthogonal candidate
selection, RTL-run movement in an LTR paragraph, sentence/grapheme UTF-16 offsets,
focused control selections, empty hard control lines, immutable values and file
input crash resistance. All are **unbuilt and untested**, like the first batch.

Remaining refinement requirements supersede the earlier blanket gaps above:

- Sentence segmentation still occurs per DOM text node; joining inline text across
  node boundaries and locale tailoring remain open. Unicode segmentation supplies
  UAX #29 boundaries, not a locale-tailored ICU break iterator.
- Visual dual-caret affinity across bidi run/wrap boundaries is not persisted;
  full mixed/auto-direction upstream vectors still require reconciliation. Visual
  word movement and text-control bidi/soft-wrap geometry remain incomplete.
- Truly empty DOM blocks need native layout caret records. Preserved delimiters are
  supported only where the native layout supplies clusters; anonymous text-control
  soft wraps likewise need explicit native geometry integration.
- Inline-goal storage remains a scalar; changing the source basis midway through
  multiple orthogonal lines needs an affinity/goal representation that preserves
  the intended physical goal. Perspective edge cases remain to be reconciled.
- Native key editing and Selection.modify are separate entry points; keyboard
  conformance is not implied by the new focused-control binding path.

The continuation also adds an explicit native-cluster regression for an empty
preserved-newline paragraph between two nonempty paragraphs, in both movement
directions. Line/paragraph movement now stores a revision-bound caret affinity
(line identity and inline position) so a shared DOM offset at the end of an empty
line and start of the next line keeps its selected line across reverse movement.
Explicit Selection boundary changes reset that affinity. This does not yet provide
full strong/weak bidi caret affinity, which remains listed above.

## Continuation: inline text runs and control soft-wrap consumer

Logical character/word/sentence movement now segments concatenated native inline
text runs, mapping UTF-16 results back onto the owning DOM text node. Equivalent
positions on either side of a split text node are one movement position. Paragraphs,
BRs and foreign editable hosts split runs; invisible nodes are excluded. New native
regression sources cover a combining grapheme split across spans, words/sentences
split across different inline elements, reverse movement, extension anchor retention,
BR stops and exclusion of noneditable content. This supersedes the earlier per-node
sentence/grapheme limitation for logical movement; locale tailoring and visual
cross-node shaping affinity still need reconciliation.

Focused controls now consume actual value clusters keyed by the control NodeId for
line/paragraph/line-boundary movement. The consumer uses projected cluster geometry,
checks source bounds, and remembers selection/line/inline goals across soft wraps.
Native select-task queuing resets explicit-selection goals. A generated regression
uses a dirty textarea value, three shaped soft lines and forward/backward/extend
movement, checking the value and defaultValue remain unchanged.

Production integration remains a named dependency: source inspection found the live
`populate_validation_states` rendered-tree projection populated text from DOM
children, not `dom.control_value`. The parent/host owner was asked to project current
control values with control-owned source IDs. Until that provider integration is
present, the new consumer and manually supplied geometry regression do not establish
live control conformance. Anonymous style fallback uses the control's computed
writing mode; password masking/source-offset mapping must remain consistent.

At the script owner's request, `native_window_identity` now recognizes genuine V8
context globals via native creation-context identity, validates the live registry
owner, rejects retired states and ignores author proxies/lookalikes. Host mapping
from returned RealmId to actual live session context remains the caller's duty.
Regression sources cover native identity, poisoned lookalikes and retirement.
Everything in this continuation is source-only, unbuilt and untested.

The host owner subsequently generated current control-value projection keyed by the
control NodeId (including password bullets per UTF-16 unit), and the layout owner
reported textarea UA pre-wrap/off generation. These production sources are still
unbuilt/unverified. A final selection source review additionally changed transformed
axis comparison to a dual basis so shear does not mix inline movement into block
movement; shear/degenerate-projection regression sources were added.

## Explicit empty caret records and visual affinity

The layout owner added a dedicated CaretStopGeometry contract with element child
boundary offsets. BindingLayout and live native layout transport now preserve it;
selection validates live node/generation/boundary membership, editing host and
visibility, and projects these real caret stops alongside text clusters. Empty
block forward/backward line/paragraph movement and stale removed-node geometry
have native regression sources. BR-after contextual geometry remains a layout
refinement; no element offsets are disguised as UTF-16 text offsets.

Visual character movement now persists the selected physical caret affinity across
repeated bidi-boundary steps. Visual word movement filters real shaped carets by
cross-node logical word boundaries and resolved run direction; left/right line
boundaries use physical projected line edges. Regression vectors cover mixed LTR
and RTL/auto layouts, move/extend anchors, repeated run transitions, visual words
and physical boundaries. All are unbuilt/unrun; full multiline bidi wrap affinity,
visual cross-node grapheme geometry and changing orthogonal inline goals still
need full production-path reconciliation.

A further source review made text-run concatenation operate on native UTF-16 units
before segmentation, preserving surrogate pairs split across DOM text nodes and
avoiding interpretation of the internal lone-surrogate envelope as rendered text.
Additional regression sources cover split surrogate pairs and isolated surrogate
movement offsets. Native activation source review similarly narrowed propagation
to actual ancestors and same-origin descendants, excluding sibling documents.


### 2026-09-16 additional source completion

The earlier physical line-boundary statement is superseded: inspection of pinned
`selection/bidi/modify.tentative.html` requires DOM line endpoints mapped by base
paragraph direction, including a single opposite-direction run. Implementation
and mixed-direction regression expectations now follow that contract.

Remembered DOM and control caret goals now retain viewport points and reproject
through each current line's dual basis. This fixes reinterpreting a horizontal
scalar as a vertical coordinate after orthogonal entry. Regression sources cover
a second movement after an orthogonal transition, rotation and shear. Visual
character wrap movement preserves destination line affinity even at a shared DOM
soft-wrap offset. Visual word wrap filters destination carets by actual word
boundaries and run direction. Native text controls now use shaped visual bidi
character/word movement and retain wrap affinity when the UTF-16 offset does not
change. Added forward/back LTR and RTL wrap source cases.

All sources remain unbuilt/unrun. Contextual mixed-direction wrap behavior,
control geometry integration, split-node grapheme geometry and locale-tailored
word segmentation still require the final comprehensive execution/conformance
phase; these source additions do not establish passing coverage.
