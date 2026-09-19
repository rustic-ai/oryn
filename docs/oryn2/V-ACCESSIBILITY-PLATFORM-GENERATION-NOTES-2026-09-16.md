# Native macOS accessibility platform source generation

Status: implementation and regression source only; no build, platform launch,
permission request, VoiceOver session, Swift compilation or test execution occurred.

## Requirement mapping

V-05 explicitly requires accessible-name/ARIA/tree projection and incremental
updates against independent full recomputation **and** OS/assistive-technology
procedures for manual entries. Existing semantic oracle sources satisfy only a
producer comparison obligation. This addition supplies a real platform consumer:

- `accessibility_platform/mod.rs`: native semantic/paint revision reconciliation,
  exact page/document/node identity, excluded-node parent promotion, ownership
  cycle rejection, native action capability checks and bounded snapshot size.
- `accessibility_platform/macos.rs`: AppKit main-thread NSWindow, visible native
  raster image, NSAccessibility elements, parent/children and relation delivery,
  role/name/value/state/position attributes, native press/focus action admission,
  destruction/change notifications and bounded async action queue. No AX callback
  enters V8, performs network I/O or waits on the page actor.
- `oryn-accessibility-host`: `accessibility-host` Cargo feature selects the
  renderer without enabling V8 in the parent. Browser::new uses production native
  page-worker execution. Window lifetime retires published OS elements.
- `scripts/tests/native_accessibility_probe.swift`: external ApplicationServices
  traversal, unique native generation IDs, role/position checks and optional OS
  press delivery. This deliberately does not certify durable action effects.

## Deferred validation procedure

After the whole-plan generation gate, build the production worker and the
accessibility host feature using the repository's pinned configuration. Launch
with the selected pinned WPT/manual fixture URL. Run Accessibility Inspector and
VoiceOver against the actual host window; record OS/AT version, fixture revision,
procedure and expected/observed outcomes. The external probe requires existing
macOS Accessibility permission and never requests it automatically. Compare native
focus/value/activation effects against the fixture's independent oracle. Exercise
navigation and removed elements to confirm stale OS objects cannot target new
native generations. Manual rows retain their original denominator and dispositions.

## Remaining generation obligations

AXValue now uses a single native Type replacement through semantic_edit::replace,
with beforeinput/input, undo and mutation/lifetime checks; it never performs Clear
then Type. Selected-range and text-marker operations need additional typed native
snapshot/selection integration.
Child-frame platform subtrees, table/grid parameterized attributes, complete live
region notification semantics, modal/top-layer platform focus and OS keyboard input
routing remain required follow-on work. Geometry now uses the layout owner's confirmed projected client rectangles, unions
fragments, honors availability, and applies the native visual camera exactly once. The
host is not an assertion of complete platform accessibility or manual WPT passes.

## Native text and lifecycle integration follow-on

Generated typed `oryn_js::accessibility::TextCommand`/`TextSnapshot` and native V8
control adapter. PageHandle, actor, worker and selected-frame command dispatch
transport these values without author access. Reads preserve UTF-16 and redact
password values/length; selection writes require unchanged exact value and native
selection revision, then revalidate after focus callbacks. AXSelectedText,
AXSelectedTextRange, AXNumberOfCharacters and AXStringForRange now consume these
native snapshots. Snapshot text accounting is bounded across controls. Native
beforeinput replacement remains the existing single-operation semantic editor.

Native semantic overlay now excludes modal-inert nodes using DomRuntime::is_inert,
restoring only exclusions that this native overlay introduced. macOS live regions
have native announcement delivery with polite/assertive priority, busy suppression
and repeated-text suppression. Complete aria-relevant/atomic/inherited busy and
text-marker/table/child-frame contracts still require continued generation.

## Child-frame and atomic snapshot follow-on

`frames.rs` now assembles exact native frame inventories and scoped snapshots,
retains per-node semantic revision and native realm lifetime for actions, suppresses
subtrees whose embedding owner is excluded, and maps geometry through real iframe
content-box/projection/camera chains. Native accessibility snapshot IPC captures
semantic, paint and control text in one actor checkpoint; it rejects native stamp
changes and never replaces revision validation with a name match.

A related frame-runtime defect was corrected: read-only scoped queries no longer
unconditionally advance frame revisions or reset semantic graphs. Native paint /
environment stamps plus viewport control invalidation. Regression sources cover
stable repeated child snapshots, wire round-trip, scoped UTF-16 selection, retired
page rejection and native modal exclusion/restoration. Full cross-frame AX host
execution remains deferred; perspective clipping, SVG foreignObject frame
placement and per-entry manual acceptance still require continued review.

## Native OS input bridge source

AppKit key, pointer and wheel events now enqueue typed native ActionsRequest values
against the published exact root realm lifetime. Native key-down identity survives
modifier changes until key-up; wheel fractions accumulate instead of disappearing.
No author script helper or JSON capability is exposed. AppKit application/window
shortcuts retain platform handling. Bounded input-channel failure closes the host
and retires the page rather than silently dropping key-up events.

Required follow-on: full NSTextInputClient marked/composed text; explicit native
CapsLock/right-side modifier state; direct out-of-viewport pointer capture semantics
(the WebDriver coordinate contract is narrower than OS pointer capture); native
window deactivation source release. These limitations remain generation gaps, not
manual-test exclusions or conformance passes.


## Generated-content, editing text, tables and input cancellation follow-on

Source generated, **not built, run, manually accepted or certified**:

- Native before/after `GeneratedContent` now feeds accessible name-from-content,
  label and ARIA-reference propagation. Explicit author labels retain precedence;
  speech alternatives replace visual pseudo text. Layout owner IDs remain native
  DOM IDs. A semantic-only arena projection carries metadata, never author text,
  attributes or mutation records. Generation replacement clears it. Regression
  sources cover updates, removal, precedence and transitive dependencies.
- Native editing-host UTF-16 text now has real DOM endpoint mappings across split
  text nodes, paragraphs and consecutive BR elements; native text-control values
  (including protected descendants) are excluded from this projection. Selection
  validates expected text, selection revision, lifetime and post-focus endpoint
  identity. Geometry comes from actual native clusters and caret line edges.
- Native table snapshots infer row/column occupancy without dense-grid allocation,
  preserve explicit sparse ARIA coordinates and unknown row counts, account for
  HTML spans, and exclude nested tables. AppKit exposes rows, cell index ranges,
  cell lookup and geometric header associations. Source regressions cover sparse
  positions, nested-table isolation and spanning cells.
- New private typed `ActionCommand::Cancel` differs from WebDriver Release:
  active pointers receive pointercancel through native frame/drag routing; Space
  activation is disarmed before key releases; callback failure still clears native
  sources, feedback, capture and payload ownership. AppKit deactivation and current
  out-of-viewport fallback enqueue cancellation. Rejected native OS requests cancel
  partial state and close the host. No author input hook is involved.

Still required source/integration work: virtual AX columns; exact HTML headers/id
and scope association; complete live-region relevant/atomic/inherited-busy behavior;
opaque native text-marker APIs and cross-node marker ranges; IME marked/composed
text and native modifier-side/CapsLock fidelity; true OS capture outside the viewport
(current cancellation prevents wrong retained-position clicks but is not capture);
layout whitespace/empty-block fidelity for editing-host text. Generated-name arena
cloning is correct isolation but adds full-arena cost when generated content exists;
a borrowed projection context remains a performance optimization obligation. The
native SDK has no public AXTextMarker header; attempted read-only WebKit SPI source
retrieval failed DNS, so no guessed ABI or opaque-marker support is claimed.

No historical evidence, fixed denominators, visual/manual requirements or acceptance
thresholds were changed by these source batches. All regression sources await the
whole-plan generation gate.


### Native OS DTO continuation

`OsPointerMove`, `OsKey`, and `OsModifiers` now preserve trusted native-host input
without broadening the WebDriver JSON action parser. Pointer capture carries real
fractional client and screen coordinates outside the viewport, including the
primary-display top-origin conversion. Native runtime paint preparation includes
this drag-capable entry point. Keyboard DTOs preserve physical code, location,
repeat and CapsLock/AltGraph state; AppKit side-specific modifier keycodes survive
release and cancellation. Snapshot-only modifier updates do not invent key-down
events merely because a modifier was already held when the window became active.
Regression sources cover outside capture, screen coordinates, keyboard layout/code
independence, repeat/CapsLock and rejection of OS-only JSON action types.

This replaces the earlier out-of-viewport cancellation fallback. Window/application
deactivation still uses native cancellation. IME marked/composed text remains open,
as do full international physical-key mapping and platform execution/AT acceptance.
Explicit HTML `headers` references now resolve through native tree scope and are
filtered to the owning table; full auto/scope rowgroup/colgroup associations and
virtual AX columns remain outstanding.

## Native IME continuation source (2026-09-16)

Added the private native text-system DTOs in `oryn-js/src/composition.rs`, native
CompositionEvent/marked-text editing in `v8_bindings/native_composition.rs`, and
native keyboard default deferral. An accepted native keydown grants an exact
realm/document-generation/stream-epoch/target continuation. `OsText` requires the
same native source, opaque host token and monotonically increasing sequence.
These DTOs cannot be requested by the WebDriver JSON action parser. Runtime and
worker action replies carry private `OsKeyReceipt` state, separately from the
public action response.

The macOS NSTextInputClient retains key events until the native keydown result
arrives, then invokes AppKit interpretation without holding a native map borrow
or waiting for a worker. Marked-text callbacks enqueue ordered typed operations.
Further native input events are retained behind the continuation until a native
Finish/Cancel receipt, preventing a later key from overtaking earlier text
callbacks. Queues are bounded and cleared on native input retirement. Dead-key
and multi-scalar event strings use Unidentified for key dispatch and preserve the
original NSEvent for AppKit text interpretation. Replacement-range overflow is
rejected, never silently converted to the current selection.

Native marked updates preserve UTF-16, fire trusted composition and input events,
retain the original edit snapshot, and make a completed composition one undo
unit. Cancellation restores the original value only while native lifetime,
content and mutation-cursor checks still match; it never overwrites author edits.
Password text is omitted from query snapshots. Regression source covers native
event flags, UTF-16/surrogate preservation, canceled ordinary insertion, password
redaction, stale author mutations, private token/source/sequence checks,
retirement, supersession and public-parser rejection.

Remaining generation obligations: complete AppKit command selector handling
beyond newline/cancel; secure-input OS activation for password fields; explicit
recovery when author callbacks invalidate a marked session; precise initial
selection in empty editing hosts; IME geometry across full marked ranges; native
CompositionEvent WebIDL generation at the global execution gate; compiled ABI
and manual input-method verification. Existing AX text-marker, virtual table
column/header inference and full live-region obligations remain open. Source
presence does not establish runtime correctness or WPT/assistive-technology
qualification. No build, test, formatter or generator was run for this work.

IME follow-through: added native AppKit move/extend, word/line deletion and
newline/line-break command defaults without synthesizing a second keydown.
Password focus now activates balanced main-thread Carbon secure event input,
using the installed SDK's documented OSStatus ABI; deactivation, root retirement
and host destruction release the acquisition. Source regressions cover balanced
transitions/retry and distinguish unspecified from overflowing replacement
ranges. Empty editing-host initial composition now maps to its real child-offset
zero boundary. These close portions of the earlier remaining-generation list;
full input-method command coverage, reactive invalidation recovery and marked
range geometry still require generation and later execution verification.

Native select picker integration: the semantic overlay now projects the native
staged active option as ActiveDescendant, independently of selectedness, and
restores authored/native ARIA references when the picker closes. AppKit exposes
AXFocusedUIElement and publishes focus changes for active-descendant changes.
A native graph regression covers highlight, unchanged selected attributes and
close restoration. IME caret coordinates now explicitly convert view to window
to screen. These source additions remain unexecuted.

Composition interruption now has an explicit private `StateSnapshot.aborted`
outcome. Author value/DOM mutation retires the marked transaction without
restoring its stale snapshot, reports composition end when the same live target
remains, and returns current/redacted native state. The action grant remembers
abortion and drains already-queued continuation operations through Finish without
performing more edits. AppKit discards marked text outside native-map borrows.
Exact retired-realm failures remain errors; no lifetime checks were relaxed.
The author-mutation regression now requires a recoverable aborted result and
unchanged author value. This source is still awaiting execution verification.

## Native live-region consumer continuation

Added `accessibility_platform/live.rs` and wired AppKit notifications to its
snapshot-only tracker. It uses native contribution IDs to distinguish additions,
text replacement and removals; applies inherited aria-relevant and busy state;
retains the delivered baseline while busy; handles atomic whole-region and
explicit atomic subtree grouping; excludes nested live/off regions from parent
announcements; announces initial alert roles without treating every newly seen
assertive region as an alert. Text and aggregate snapshot sizes are bounded and
traversal is iterative. Regression sources cover busy release, relevant removals,
atomic changes, initial alert distinction and inherited/nested projection.

Remaining: descendant aria-atomic=false overrides inside atomic ancestors and
partially busy descendant subtrees need finer contribution-level grouping and
baseline retention; platform notification ordering/coalescing and actual screen
reader output remain unverified. Existing manual accessibility denominator is
unchanged. No execution occurred.

Live-region follow-through now records per-contribution nearest atomic owner,
full native subtree context, busy descendants and parent identity. Explicit
atomic=false stops inherited grouping for that contribution, while another
change in the atomic ancestor still receives its complete context. Busy
subtrees retain previous contributions (including removals) without delaying
unrelated non-atomic changes; atomic groups wait together. Iterative spans avoid
recursive traversal, and text/output budget exhaustion is an explicit error.
Added regressions for atomic override/context and deferred removal alongside an
unrelated immediate update. Notification coalescing and real platform/AT output
still require the later validation phase; no execution was performed.

Native AX table scope continuation: semantic projection now retains native
colgroup/col span intervals, and table metadata retains row-group identities and
header scope. AppKit header queries resolve rowgroup/colgroup membership lazily,
with explicit headers (including explicit empty lists) taking precedence. This
avoids a quadratic table-sized header matrix. Regression sources cover row-group
isolation, column-group membership and empty explicit header suppression. Fully
automatic unscoped HTML header scanning and virtual AX column objects remain
separate generation obligations; this does not claim their completion.

Native IME geometry continuation: added immutable `text_geometry.rs` range and
caret queries, consumed from the same exact-value native text snapshots after
frame/camera/view/window projection. NSTextInputClient firstRect returns only the
first native line's covered UTF-16 range; characterIndex uses native caret
rectangles in both axes with current affinity as a tie-breaker. Missing/stale
geometry returns unavailable, never fabricated metrics. Regression source covers
multi-line ranges, surrogate offsets, vertical carets and bidi ties. Password
candidate geometry and immediate geometry before a fresh snapshot remain open;
password text stays redacted.

## Native opaque text markers

Read-only GitHub source retrieval resolved the ABI dependency: WebKit
[`HIServicesSPI.h`](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/PAL/pal/spi/mac/HIServicesSPI.h),
blob `1f20289bdfed220bcd63025ebda715e4d854e8c8`, declares the native CF marker/range
constructors, type IDs, byte access and copy ownership. Installed macOS SDK
ApplicationServices/HIServices `.tbd` also exports these exact functions.

Generated pointer-free marker payloads and real AXTextMarker/AXTextMarkerRange
wrappers. Payload validation binds per-host random namespace, semantic target,
realm/document generation/stream epoch and UTF-16 content digest. Selection-only
or geometry changes do not invalidate identical text; stale content and foreign
host/document markers are rejected. Password markers are unavailable. Native
attributes/parameter queries support start/end/selection, index conversion,
next/previous scalar, strings/length/bounds, unordered range construction and
native selected-marker-range actions. Regression sources cover byte shape,
namespace/epoch/content changes, real CF type/length checks and surrogate offsets.

Remaining marker scope: document-wide/static-text traversal, cross-node ranges,
live boundary adjustment after text mutation, grapheme/word/paragraph traversal
and cross-frame marker ordering. The API source is verified; generated code and
actual assistive-technology behavior remain unexecuted and unqualified.

Document/static-text marker backing now has native read-only flat-tree traversal,
including assigned slots/shadow content once, native DOM endpoints, real text
cluster caret geometry, and cross-node document selection. Replaced form controls
and embedded objects contribute an object character; their own native AX text
objects retain value/protection authority. Exposed document/static-text snapshots
join the existing editable/control capture under the same readiness/lifetime
checks. Native text read failures are explicit snapshot failures, not silent
omissions. Regression sources cover light-tree cross-node selection, password and
hidden control exclusion, and slot/shadow traversal.

Remaining document text work includes full rendered whitespace/empty-block
presentation, auxiliary table-cell separators, cross-shadow selection exposure
rules, DOM mutation-adjusting markers and cross-frame ordering. The added flat
text projection is native source-backed, but these limits prevent any claim of
complete platform text-marker conformance. No execution occurred.

### Native marker grapheme traversal

Added shared V8-free UTF-16 extended-grapheme traversal and wired native macOS next/previous AX markers to it. Interior offsets snap toward the requested grapheme edge; combining sequences, emoji ZWJ families, regional-indicator pairs, CRLF, and valid surrogate pairs remain intact. Segmentation replaces unmatched surrogates only in a temporary analysis string; returned offsets address unchanged native UTF-16. Pure regression sources cover both directions, interior/end/out-of-range offsets and malformed UTF-16. A native CF marker regression exercises the actual parameter dispatch path. None were executed.

Still required: word/paragraph marker parameters, cross-object and cross-frame marker order/source mapping, live adjustment on mutation, rendered document whitespace/auxiliary separators, and platform qualification. Grapheme traversal does not close those obligations.

Native AXNextWordEndTextMarkerForTextMarker and AXPreviousWordStartTextMarkerForTextMarker are now wired through validated immutable marker snapshots to the pinned Unicode segmenter's word boundaries. Read-only WebKit WebAccessibilityObjectWrapperMac.mm source confirms parameter spelling/direction. Sources cover punctuation, apostrophes, combining words, astral-prefix offsets, and unmatched surrogates. Locale/dictionary tailoring and left/right word-range parameters remain open; no platform equivalence or test result is claimed.

Follow-on: left/right AX word-range parameters now select only the word touching the marker on the requested logical side. A whitespace gap returns an empty marker range, preserving the side-affinity behavior described by WebKit AXTextMarker::wordRange. Added source cases at both word edges, inside words, between repeated spaces and invalid/empty offsets. This supersedes the left/right parameter implementation gap above; locale tailoring remains unresolved.

### AppKit command deletion cancellation and intent

Native text-system word/soft-line deletion now passes explicit granularity into the accepted editing transaction. It no longer expands selection before cancelable beforeinput or loses the deleteWord/deleteSoftLine inputType by synthesizing a plain Backspace. Native controls keep their selection/value unchanged when beforeinput is canceled; added source regression checks trusted event intent and the accepted follow-up edit. The implemented AppKit command mapping emits character/word/line-boundary deletion only; other private deletion units now reject explicitly until their semantics are generated. Normal keyboard platform modifiers retain their existing path. No execution.

### Control soft-line deletion uses native shaping

Extracted a read-only control line-edge query from the existing shaped caret/remembered affinity source. Accepted keyboard/AppKit soft-line deletion now uses that real line identity instead of scanning literal LF. The query validates the current native control value and does not mutate selection; control undo therefore snapshots the original selection. Missing native geometry yields no edit rather than inventing a wrap boundary. Added regression source with three shaped lines over text containing no LF, checking the resulting text/inputType and undo restoring the original collapsed offset. This supersedes the soft-wrap deletion gap above; unavailable geometry/readiness and full orthogonal/bidi platform qualification remain explicit. Sources only, no build or test run.

### Paragraph text-system commands

Added typed ParagraphBoundary distinct from paragraph movement and soft-line boundaries. AppKit paragraph start/end movement (including extending), paragraph forward/backward movement, and paragraph start/end deletion now route to native selection/editing. Hard-line deletion retains deleteHardLineBackward/Forward inputType, preserves canceled-beforeinput selection and uses actual native control LF boundaries or DOM paragraph boundaries. Source regression checks hard deletion followed by paragraph-boundary selection. Remaining text-system commands include Tab traversal while composing, page movement and transpose/yank-style editing; protected candidate geometry is still open. No builds/tests.

### Marked-input Tab continuation

AppKit insertTab/insertBacktab now route through a typed native Tab command. The marked session commits before native focus traversal. A focus transition drains the original continuation with a redacted aborted outcome; it does not retarget the existing token to the newly focused element. Native command handling also revalidates focus/editability after compositionend callbacks before running any default. Regression source covers ordinary marked Tab commit and a compositionend listener moving focus, where traversal must not continue from the new owner. This supersedes the marked Tab generation gap; final cross-frame/platform tests remain unrun.

### Protected IME placement without password text

Added private ProtectedCaret native snapshot metadata: focused selection, native mutation revision and a single real shaped caret. Password value, public selected range, character boxes/carets and AX text-marker operations remain redacted. The exact UTF-16 caret helper consumes masked control geometry directly, including offsets inside original astral text, and never serializes source bytes. Native camera/frame assembly transforms the private caret with the same placement as other geometry. AppKit installs it only for matching protected selection/revision and returns its insertion rectangle to NSTextInputClient; protected substring remains unavailable. Input receipt mutation revisions prevent stale redacted-value equality from admitting geometry after a new edit. Fixed root-realm active input lookup to use root_scope when node.scope is absent. Generated native password redaction/placement and stale revision/selection regression sources; no execution.

### Native AX columns

Added sparse merged column intervals, ordinal lookup and ordered spanning-cell membership to the native table projection. AppKit now materializes AXColumn objects only for represented native columns, independently of large/unknown ARIA total counts. Column identities stay separate from DOM/semantic IDs and persist for the same table/coordinate across snapshots. Native attributes include index, parent, children/visible children, headers, geometry and window; columns have no action authority. Retirement removes native records before releasing objects and posts destruction notifications. AXColumns/AXVisibleColumns now route to these objects. Object count and aggregate cell-reference budgets fail explicitly before column publication; missing sparse coordinates never allocate empty objects. Added sparse interval and stale callback regression sources. Full unscoped HTML header association remains open, and platform behavior is unrun.

### Authoritative resolved AX text

Added common ResolvedTextRun/ResolvedTextSpan native provenance and BindingLayout transfer; layout owns the final normalized/transformed producer. The AX consumer now resolves per-native-node rendered UTF-16, source offsets and distinct start/end affinities for expansions. Erased aliases retain source endpoints without duplicating base glyphs. Document/read-only text and editable AX reads consume this native metadata when available, instead of independently implementing CSS whitespace or text-transform. Actual DOM editing/composition source text remains unchanged. Native selection uses mapped original DOM endpoints; cluster rectangles map through source offsets to rendered AX ranges. Invalid byte boundaries, source lengths, conflicting duplicate runs and budgets fail explicitly. Added collapse/uppercase expansion/erased mark/conflicting metadata and native DOM selection regression sources; none run. Empty block/table separators, cross-frame marker ordering and live marker adjustment remain in scope.

### HTML header assignment from the table-model algorithm

Read the current HTML table-model header algorithm as source, then generated native header/data/empty-cell metadata and bounded assignment. Auto header direction now derives from row/column data occupancy. Scanning tracks opaque header blocks, skips ambiguous overlapping slots, respects explicit headers, removes empty/duplicate/self references, and checks row/column-group anchoring limits. Coordinate bands avoid repeating identical scans across large spans. Native table header classification now overrides the former role-only assumption that every unscoped th is a column header. Regression sources cover auto row/column headers, opaque blocks, explicit overrides, empty cells and overlap ambiguity. Assignment work/reference budgets fail explicitly. Full table-forming edge cases (notably malformed row groups and author role overrides) and platform qualification remain unrun; this does not certify conformance.

### Native live-marker foundation and CharacterData fidelity

Added a bounded numeric BoundaryTransform journal beside native live-Range insertion/removal/replacement/normalization adjustments. It contains generational node IDs and offsets, never text bytes. Snapshots rebase only across continuous retained history; missing/overflowed history invalidates rather than guessing. Native split_text now migrates all live ranges and composes exact split semantics into the journal; JS splitText delegates to it. This replaces the prior JS-wrapper-only migration. Fixed CharacterData replacement, UTF-16 length and normalize concatenation to use lossless dom_string units, preserving lone surrogates. Added native Range/journal/split/normalize/truncation source regressions. AX source-boundary snapshot and opaque marker journal-consumption wiring are the next required integration; the journal alone is not claimed complete live-marker support. No execution.

### Live source markers through native snapshots

Native AX text snapshots now carry original DOM start/end boundary maps and native boundary-journal revision. The atomic accessibility snapshot carries the bounded numeric journal once per document; frame assembly preserves each document's journal on its native entries. Host/executable/runtime capture validates the exact selected lifetime and matching source revision. Opaque AX marker version 2 encodes original generational source boundaries and revision, then resolves through native transformations into the current immutable text snapshot. Content-only markers for controls retain their existing digest invalidation; source markers no longer fail merely because unrelated text changed. Missing history, mismatched realm/epoch, protected text and unavailable mapped endpoints invalidate. Added native marker rebase/serialization/truncation source regression. Cross-entry/cross-frame range composition and control live-value markers remain required follow-ons; no execution.


### Live marker source identity and native designMode follow-through

- Published AX marker lookup now resolves each candidate against that document's native epoch and boundary journal, including child-document targets queried through a root object. It returns the published source-node AX object when available, otherwise the smallest actual text container. Host namespace/window isolation and retired epochs remain enforced. UI-element ranges can use another published same-host text object. Source regressions cover child epoch retirement, separate host namespace and source-node ownership.
- Document.designMode now converts through WebIDL string conversion, accepts ASCII case-insensitive on/off, ignores other keywords, and publishes through native document metadata. All located editing/selection/drag consumers read that native authority. A source regression covers ignored strings, throwing coercion and native-to-JS reflection.
- These changes do not complete cross-document text range concatenation/ordering or live value markers for input controls. Missing native source boundaries still invalidate markers rather than use string matching. All checks remain unrun under the global generation-first gate.


### CharacterData and retained AX storage audit

- The shared native UTF16 slicing helper, CharacterData.length, substring/append/replace offsets and splitText now use lossless DOM string units. CharacterData argument coercion propagates exceptions and reads current length after coercion, preserving author mutation reentrancy. Source regression covers unmatched surrogates, splitting inside a pair then normalize, throwing index coercion and reentrant append/replace.
- isContentEditable now shares editing-host lookup with trusted input, preserving the native designMode shadow boundary.
- AX retained text accounting includes allocated UTF16, source-boundary, box and caret storage. Native per-document capture rejects over 128 MiB; merged frame text/geometry rejects over 256 MiB. This supplements existing unit/node limits. Source accounting regression includes empty vectors with reserved storage. Journal, semantic and raster allocations have their separate existing caps; these limits do not substitute for final resource qualification.
- No builds, tests, formatters or generators executed.


### Empty-line IME geometry consumer

Native AX now consumes real CaretStopGeometry for an empty protected control and for document boundary points already present in its source map. It projects the actual line extent and writing/transform axes; absent layout stops remain unavailable. Source regression covers translated empty-password caret, redaction and no fabricated fallback. Layout owner is checking empty-control stop production. Empty-block text separator/source mapping remains distinct outstanding work.


Native document AX text now retains leading, interior and trailing genuinely empty block lines when the layout supplies an exact empty-line caret stop. Empty inline elements without a native line stop contribute nothing. Source regression covers all three empty-block positions, mapped caret geometry and the unavailable-layout case. Table cell separators and nonempty containers whose descendants are all non-rendered still need their full structured mapping; this bounded change does not infer those from text absence.


AX document text uses tabs between adjacent native table-cell formatting paragraphs sharing an actual table-row ancestor, and line breaks between rows. Explicit CSS display overrides control classification; native DOM ancestry determines row identity. Empty shaped cells remain represented. Added a two-row regression source with an empty cell. Rowspan/grid virtual slots and nested block content require further structured table-text coverage; no execution claims.

## V-05 control, frame and manual-slot follow-on (2026-09-17)

Source generated, no builds/tests/platform execution. Native controls publish explicit control identity and use dedicated exact replacement/reset journal records. Regular DOM boundaries ignore these records. Missing historical control metadata remains unknown; opaque marker v3 rejects v2. Password changes carry no replacement lengths. Replaced rendered objects publish native node identities at exact U+FFFC positions, allowing macOS range ordering/concatenation across current published frame documents and controls without aligning equal strings. Namespace/realm/epoch checks, cycle/depth/size limits and protected text rejection remain enforced. Geometry comes from the contributing snapshots. Cross-document selection mutation is not synthesized: selection continues to require one native document owner.

Manual `slot.assign()` now exposes authoritative effective assignments in native arena snapshots. Empty assignment uses fallback; slot mode/assignment changes dirty semantic ancestors and ARIA reference dependents. A generated oracle regression compares incremental and full projection through mode switch, reordered duplicate assignment, reassignment and fallback.

### Required macOS manual procedure (execution pending)

1. Use a newly built signed candidate with Accessibility Inspector and VoiceOver enabled. Capture candidate/protocol identities and fixture URL with results.
2. In a document with text before/after an iframe, select/read a marker range starting in parent text and ending in child text. Repeat with nested frames and adjacent equal strings. Verify reading order, exact concatenated text and screen bounds against rendered layout.
3. Retain a child marker, navigate that child, and retry. Verify the stale marker is rejected. Repeat after top-level replacement and against another window; no cross-lifetime text is exposed.
4. Retain a textarea marker, prepend/delete native value text, and verify the marker follows the exact edit. Change a clean default value and confirm invalidation; change dirty textarea default content and confirm live value stability. Repeat password input and verify no value/length-bearing marker is exposed.
5. With VoiceOver focus on a labelled control, switch manual slot assignments and ARIA labelledby/owns targets; verify current names/tree order against Accessibility Inspector. Include empty assignment fallback, duplicate targets, ownership cycles, removed targets and shadow boundaries.
6. Record actual observations and failures; these procedures are not passing evidence until executed. Full pinned ARIA caller reconciliation and overall V-05 acceptance remain open.

### Ownership caller reconciliation

Pinned `accname/aria-owns.html` requires hidden owners to be ignored and visible targets under ARIA-hidden DOM ancestors to remain ownable. Source review found a hidden first candidate reserved the lookup before a later visible owner; candidate selection now excludes hidden owners before choosing. Both full and incremental ownership publication also reject cycles through already accepted sibling ownership edges, rather than checking DOM ancestors alone. Added hidden-owner and three-sibling mutation regression sources. Pinned `accname/name/shadowdom/slot.html` invokes testdriver labels for assigned/fallback content and ignores slot aria-label; existing name traversal retains those semantics, and the new manual-assignment oracle extends native caller coverage. Cross-arena adoption remaps published slot identities and filters departed source references; a native migration regression source covers this contract. These are concrete source checks, not a claim that every accname/wai-aria/html-aam manifest entry has been qualified.

### Contextual roles and property defaults (source generation continuing)

Reviewed native `LiveLayoutProvider::compute_accessibility_with_layout` → semantic graph → `computed_role_for_node`, and WebDriver AccessibilityProperties → same layout graph. Every previous attribute-only role caller now uses native node context. Required pinned callers include `html-aam/roles-contextual.html`, its tentative companion, `aside-in-prefixed-article.html`, `roles-dynamic-switch.tentative.window.js`, and `wai-aria/role/contextual-roles*.html`. Names are resolved from the native accessible-name algorithm, with a recursion guard for cyclic references; nested aside/header/footer, named section/form, empty-alt ARIA-named image and checkbox switch use current context. Subtree insertion invalidation now refreshes descendants after reparenting. Native boolean switch IDL/getter/setter source and WebDriver regression source are generated.

`wai-aria/checked/checked.tentative.html` and `pressed/pressed.tentative.html` require role defaults. Native properties now default checkable ARIA roles to false and button pressed to undefined, normalize mixed support, and retain native checkbox state precedence. This reaches platform and WebDriver consumers together.

`html-aam/figure-name-no-figcaption.tentative.html` requires author naming for figure itself and caption fallback only for a lone unnamed image. Removed figure's implicit caption name; generated eligible-image fallback, title/alt precedence, sibling dependency tracking and incremental/full projection regressions. `html-aam/names.html` contains no contradictory caption-derived figure expectation. All changes remain unbuilt/unexecuted; full source inventory is still in progress.

### Direct family and manual AX inventory

Static source enumeration of the pinned checkout found 19 accname, 31 wai-aria and 15 html-aam non-manual HTML files; 159 accname and 233 wai-aria manual HTML files; and four JavaScript source files. These are source-file counts, not manifest/variant counts. Manual ATTAcomm AXAPI assertion fields were enumerated from their source JSON without running the fixtures. They include role/subrole/description, orientation, name/help, ARIA row/column/set metadata, current/busy, numeric/value-description/placeholder/error fields, geometry, focus/selection/settable/actions and events.

Generated native follow-on fixes map canonical image to AXImage, add native widget/subrole/role-description mappings, map AXDescription to accessible name and AXHelp to description, expose orientation/current/busy/ARIA collection metadata/value-description/placeholder/validation fields, and admit AXShowMenu through the existing native press action. Semantic numeric ARIA properties retain finite decimal values; native HTML numeric state takes precedence. Error-message dependencies update on text and relation changes and disappear when invalid state clears. Portable role-mapping and semantic property regression sources are added.

Remaining source reconciliation is explicit: manual event-type publication and settable/action semantics, complete native role mapping against each manual role-specific assertion, and all remaining name-fixture expectation matrices. Existing field presence is not sufficient evidence to close these. Native execution and assistive-technology observations remain deferred.

### Native accessibility action and event caller reconciliation

The exact direct source inventory is retained in `V05-PINNED-ARIA-SOURCE-INVENTORY-2026-09-17.json`: 461 HTML/JavaScript source records, including 392 manual fixtures and 948 declared expected-value fixtures (declarations may be comments/todo selectors; they are not counted as executed cases). Script/helper edges and manual AXAPI assertions are retained for review.

Native AX SetValue now selects the existing Check/Uncheck, Select or Type command according to capability. Executable check actions read HTML checkedness or effective ARIA checkedness, use trusted native pointer activation, then verify the requested state; an unchanged satisfied request does not replay callbacks. Custom listbox/combobox/radiogroup selection resolves native owned options, activates the exact option, and checks both continued ownership and resulting selected/checked state. Disabled/read-only targets reject the request. Grid readonly inheritance reaches child cell/header states and updates after parent mutation.

Custom ARIA value models remain application-owned: the native Type adapter sends a trusted beforeinput replacement request and accepts only the requested resulting numeric/text value. It does not rewrite arbitrary ARIA attributes as a fabricated default action. Native range adjustment sends one trusted keyboard sequence and verifies directional value progress (with a bound no-op). Unhandled/canceled-without-change requests return failure. Native HTML editing/default actions remain supported by their existing typed paths. Regression sources cover trusted handlers, no-op non-replay, readonly rejection, unchanged failure, inherited grid states and AX action routing.

Manual role assertion reconciliation additionally corrected treegrid to AXTable, feed to AXApplicationGroup, grid/table and listitem descriptions, and popup-button role/description. Native value publication reads checked/pressed/tab state before text snapshots. Native numeric range bounds use explicit values before role defaults. Property delta events include checked/pressed value, busy, selected-parent and expanded changes; notification delivery follows whole-entry/column publication and pending notifications survive a column preparation error.

`aria-actions/idl-reflection.tentative.html` required a previously missing Element/ElementInternals relation. Native frozen reflection, migration retention and semantic action-target exclusion now include ariaActionsElements, with source regressions for content/IDL precedence, order, null and shadow filtering.

Pinned `html-aam/roles-dynamic-switch.tentative.window.js` explicitly queries a detached switch and expects an empty computed role. The native accessibility query now allows a detached node only when its document, generation and authority still match; action/property resolvers remain strict. A regression source verifies detached/attached switching and action rejection. Final action-source review also rejects an aria-disabled selection owner before an already-satisfied no-op, preserving disabled authority and avoiding callback replay.

Name-fixture review continued through the pinned name-from-heading tentative source: article already traversed descendants, but dialogs only inspected immediate children. The native fallback now shares depth-first flat-tree heading selection across article/dialog/alertdialog, honors explicit heading roles and exclusion, and retains generic-group non-naming. Generated nested-heading/exclusion regression sources; no execution claim.

### Fixture-specific follow-through (continued source review)

The inventory now attaches exact source reconciliation and regression names to reviewed fixture groups. `html-aam/names.html` has a generated 16-element by eight-precedence native regression matrix (128 declared cases, not executed). Pinned presentation-conflict fixtures exposed missing native table descendant inheritance: rowgroups/rows/cells now inherit effective presentation while explicit descendant roles and focusability retain their conflict behavior. A role mutation regression restores table semantics. Manual `div_element_without_role_roledescription_valid` exposed an AX adapter error: generic containers now ignore authored role descriptions, and whitespace-only descriptions fall back to native descriptions. Portable regression source covers both.

Full 461-source closure is still not asserted. Manual native event observations, all remaining name/content/rendering matrices and complete role-context fixture review must remain distinct from these source-backed rows.

The pinned marker-name source exposed a concrete layout-to-semantics disconnect: native layout emits marker text/alternative, but semantic ingestion previously ignored marker records. Marker text now precedes before-content regardless of record order; an explicitly empty alternative suppresses it, and nonempty markers separate from following content. Incremental referenced-name and native WebDriver stylesheet-change regression sources cover the complete producer/consumer chain. Native range property defaults were also missing despite advertised AX bounds: shared semantic properties now publish slider/scrollbar/separator midpoint values and spinbutton zero with native bounds, while later live HTML control projection remains authoritative. Fixture-level audit entries identify these changes and their unexecuted regressions.

Computed text formatting source review found that semantic names ignored both CSS display separators and case conversion. Native projection now retains display/text-transform alongside visibility, carries it through arena snapshot/migration, and invalidates names when those properties change without a DOM text mutation. Block/inline-block runs separate words; inline runs do not. Case conversion reuses layout CaseContext over the native paragraph with scalar source positions and language. Accessibility deliberately does not apply full-size-kana/width presentation substitutions. Semantic, native transform and actual WebDriver layout-caller regression sources cover the new path, including stylesheet replacement and unchanged DOM text. The projection metadata is native-only, not a wire schema change.

Nested content naming now carries an explicit per-computation native visited set and labelledby traversal depth. Descendant labelledby can replace descendant contents, and an image already used by an earlier nested reference is not repeated later in the same heading name. Independent requests start fresh, so the later link still includes its image in its own name. Reference traversal suppresses further labelledby chaining and handles self-reference label fallback. This removes the prior label-only recursive path; direct native regression sources match the pinned two-links/shared-image and nested-link fixture shapes.

### Manual AXAPI context source reconciliation

All 159 accname and 233 wai-aria manual markup/AXAPI expectation groups were inspected in bounded source batches, including numbered name cases and role/property contexts. This review covers AXAPI assertions only; it does not claim platform execution or other accessibility backends. Newly confirmed production gaps were implemented with regression sources: embedded menu exclusion and combobox owned popup values; own-control exclusion boundaries and label tooltip fallback between pseudos; directly referenced empty-text tooltip fallback; native pseudo display separation; explicit HTML row/column spans overriding ARIA while missing HTML attributes allow ARIA; virtual focus on active descendants with focus requests routed to the exact DOM focus owner; modal accessibility-only scope; heading level defaults/AXValue; aria-placeholder; menubar orientation; explicit false popup override; nonfocusable separator value-description suppression; inherited table coordinates at AX property reads.

`GeneratedContent.separates_text` retains native pseudo display information rather than modifying the authored text or alternative payload. It is serde-default false for historical JSON and appears in the P06 combined-candidate audit. All local struct literals were reconciled. Source regressions remain unexecuted. Broad V05 remains separate from this manual-context source row until final combined caller/fixture reconciliation is complete.

Final direct role-token review found the pinned `fallback-roles.html` case `region group` without a name still fell back to the native tag role. Context-sensitive named-landmark rejection now continues through the remaining supported role tokens, including case-insensitive tokens and missing label targets. Shared explicit-role canonicalization removes the duplicated synonym/conflict path; a native mutation regression covers unnamed group→named region. GeneratedContent historical JSON/default and new separation round-trip regression sources were also added.
