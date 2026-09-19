# OIL native input generation notes

Source generation only. No build, test, WPT, workflow, or qualification result is
claimed by this record. The complete Core/OIL plan remains open.

## Generated integration

- Semantic click/hover/check/uncheck use native scrolling, layout hit testing,
  pointer transitions, focus, cancellation, and activation. Covering elements
  reject the action. Checkbox no-ops emit no events; canceled state changes fail.
- Select updates live selectedness, preserving reset defaults. Disabled options
  reject and unchanged selections emit no events.
- Control notifications and focus bypass author constructors and DOM methods.
- Text replacement/clear enter native editing with beforeinput, typed input,
  UTF-16 caret positions, text-control history, and editing-host transactions.
  Callback changes to lifetime, focus, selection, or native mutation state stop
  stale replacement. Input controls retain committed change notifications.
- Release Actions clears residual drag payload ownership and drag feedback.
- Selection edge autoscroll shares the native input scheduler, uses native
  projected scrollports and signed bounds, and updates selection after scrolling.
  Deferred drag sources do not autoscroll as selection. Release, cancellation,
  document retirement, and suspension remove pending selection work.
- Cross-frame pointer routing updates native hover boundaries in old documents
  and the embedding path, with forward/inverse coordinate mapping and no extra
  parent pointermove. Capture routing is rechecked after boundary listeners.
  Regression source covers scaled frames, exit/re-entry, null cross-document
  related targets, and frame removal during pointerover.
- Drag initiation is separate from destination dispatch. Active drags use the
  native cross-frame transport with exact source/target lifetime and immutable
  embedding-ancestor checks. Releases clear mirrored parent button state;
  source retirement abandons native payload/feedback ownership. Regression
  source covers native drop, pointer cancel, Escape, and canceled dragstart
  attempted once per physical press. Transport transaction coverage and final
  same-origin/cross-origin policy audit remain open.
- Drag-leave routing now maps the current pointer position into the previous
  target's exact native frame lifetime, including transformed sibling frames.
- Touch contacts snapshot native `touch-action` intersections before pointerdown
  callbacks. Pan, pinch, double-tap defaults and TouchEvent delivery remain open;
  policy source alone is not touch conformance.
- Regression source covers overridden author APIs, cancellation, covering
  elements, readonly fields, reset defaults, callback edits, and undo.

## Still open

Full source audit against every S requirement; cross-frame drag transport and
full platform boundary ordering; touch gesture defaults; full platform selection
autoscroll policy and cross-frame selection;
remaining legacy testdriver adapters; platform-specific editing/IME and input
type behavior; integration with completed pre-action resource capture; all
runtime verification and full-plan qualification.

These additions do not certify either Core conformance or any workflow family.

## Legacy migration dependency

The plan's G package requires historical-runner reproduction until official
executor parity is established. `native_actions.js` and the historical vendor
adapter currently call page-visible native input hooks, including
`__oryn_dispatch_user_input` and `__oryn_native_drag_input`. Their continued
presence does not satisfy S's no-public-privilege-hook exit requirement.

The migration must preserve the historical fixture/evidence path through an
explicit test-only authority while default production execution exposes no
trusted-input capability to author script. Existing legacy adapter regression
sources must be carried to native authenticated action requests, including
drag/cancel, file selection, keyboard focus and frame retirement. Neither
deleting those regressions nor treating official HTTP Actions implementation as
proof that the old hooks are gone closes this obligation.

## Native touch pan generation (source only)

Touch contacts now retain their initial native touch-action policy and paint
projection before pointerdown listeners. Recognition uses viewport slop and local
axis/direction policy; an accepted direction permits reversal while a rejected
initial direction remains rejected for that contact. Native pan consumption walks
scroll containers, maps residual motion through each native transform, clamps to
native scroll bounds, and respects overscroll containment. Hidden overflow does
not admit user panning.

Recognition commits pointer suppression before pointercancel callbacks, releases
capture, emits exit boundaries, clears click/selection state, and retains native
routing until physical release. Release/cancel removes the suppressed contact.
Regression source covers slop, direction rejection, reversal, axis filtering,
projection horizon rejection, viewport offset consumption and release cleanup.

This is generated, unbuilt source. TouchEvents and their cancellation policy,
multi-contact pinch/visual viewport behavior, double-tap policy, compatibility
mouse timing, cross-frame residual panning, and comprehensive reentrant lifecycle
coverage remain open generation work. No conformance or runtime result changed.

### Deferred compatibility mouse input

Generated touch compatibility mousemove/mousedown/mouseup at completed release,
with native press/release button state and focus default. Touch down/movement no
longer runs mouse selection or drag defaults while pan recognition is pending.
Recognized panning suppresses that sequence. A canceled pointerdown suppresses
compatibility mouse events while retaining the independently dispatched click
PointerEvent, as required by Pointer Events cancellation semantics.

Added source regressions for trusted event order, deferred focus, pan suppression,
and pointerdown cancellation. TouchEvent cancellation is still pending because
Touch/TouchList and the native contact-list event payload remain incomplete; the
existing TouchEvent IDL is only an empty Event-derived shell. No generation or
runtime validation commands have been run.

### Native Touch snapshots and event integration

Touch/TouchList platform records, getters and native snapshot constructors are
now generated. Native touchstart/move/end/cancel payloads contain immutable
contact lists, initial contact targets, coordinates, dimensions and force.
Touchstart/move cancellation prevents pending pan recognition and suppresses
compatibility activation. Trusted construction bypasses author constructors.
A native dispatch regression retains an initial list across move/end and checks
brands, indexed/item identity, event order and cancellation suppression.

Still source-only: no build/test/generator commands. Author TouchEvent constructor
sequence conversion and full indexed TouchList exotic behavior, shadow retargeting
of touch targets, multi-document contact cohorts and pinch remain generation
obligations. Generated binding tables must be refreshed at the global build phase.

### Touch constructor and indexed-list generation

Author TouchEvent construction now uses UIEvent initialization plus inherited
modifier conversion and Touch sequence snapshots. Sequence conversion authenticates
each item while its iterator is active and uses the shared IteratorClose path.
TouchList now uses native indexed property handlers rather than fixed data
properties, including absence, enumeration, assignment/definition and deletion
behavior. UIEvent payload arrays have null prototypes before native slot writes.

Added constructor regression source for numeric conversion, required members,
finite coordinates, sequence snapshots, invalid item IteratorClose, indexed
mutation and untrusted event identity. This supersedes the earlier note that
constructor conversion and indexed handlers were absent. These paths are still
unbuilt and unrun; full conformance is unproven. Shadow touch-target retargeting,
cross-document contacts, pinch/zoom and remaining lifecycle cases stay open.

### Shadow touch target retargeting source

Native dispatch now snapshots Touch target retargeting for every event-path
observer before listeners run. Each observer receives three immutable lists;
repeated references to one Touch retain identity across those lists. Original
Touch objects remain unchanged, and moving/removing a host during capture does
not recompute later listeners' visibility. Dispatch confined to a shadow tree
clears its retained touch lists with the event target.

Regression source covers a closed root, host removal during document capture,
inside/outside target identity, retained list snapshots and noncomposed cleanup.
No test/build has run. Cross-document contacts, pinch/zoom and remaining gesture
lifecycle integration remain open generation obligations.

### Suspension and contact replacement

Native input suspension now clears touch click/capture/selection state and retains
a suppression marker until physical release, preventing a parked contact from
becoming a tap after restore. A fresh press starts an independent contact. Contact
identity is retained across local snapshots and rechecked after touchmove callbacks
so replacement cannot be overwritten by stale recognition work.

Added regression source for pending and already-panning suspension, silent
move/release after suspension, fresh-contact activation and distinct contact
identities. All source remains unbuilt/unrun; this does not close full lifecycle
or conformance qualification.

### Visual viewport foundation

Added native visual viewport geometry and API source. Pinch geometry preserves
content under the finger midpoint, snapshots native scale policy/contact IDs,
rejects singular/nonfinite samples, clamps offsets and scale, and exposes inverse
input/paint coordinate transforms. Visual viewport panning returns unconsumed
movement in visual CSS pixels. Window.visualViewport and native platform getters
now read this state, with stable object identity and layout-scroll-relative page
coordinates. IDL source includes the VisualViewport EventTarget surface.

Generated pure geometry and native API regression source. Pinch contact admission,
input/paint consumers, iframe behavior, resize/scroll/scrollend delivery and native
viewport policy integration are NOT yet connected; this is a foundation, not a
working pinch/zoom claim. No builds, tests or binding generators have run.

### Visual viewport checkpoint delivery

VisualViewport geometry observations now feed the native host checkpoint loop.
Changes are coalesced before dispatch; trusted resize/scroll and subsequent
scrollend events use native EventTarget dispatch and handler storage. Observed
state is committed before callbacks, so callback-driven changes are evaluated at
the next checkpoint. Added source assertions for handler access, event target,
trust, order and stable-state silence.

Still unbuilt/unrun. Native pinch admission, shared input/paint transforms,
iframe viewport semantics, viewport metadata policy and full scrollend scheduling
qualification remain open. This checkpoint integration does not certify complete
pinch behavior or VisualViewport conformance.

### Visual viewport listener ordering

VisualViewport now uses the shared native platform event-path dispatcher with
capture/target phases, ordered handler registration, and immediate propagation
control. The former IndexedDB-specific dispatcher was renamed to describe the
shared path; IndexedDB retains its request/transaction/connection parent chain,
while VisualViewport has no parent. Handler replacement retains its listener slot;
removal and reassignment create a new registration.

Added regression source for capture ordering, target/currentTarget/eventPhase,
handler replacement/re-registration and stopImmediatePropagation. Source-only;
no validation commands ran and full viewport/input/render integration remains open.

### Pinch contact ownership source

Generated PinchCandidate recognition over two exact native Contact identities,
both captured touch-action policies, cancellation state and a viewport-pixel
recognition threshold. Native geometry updates reject a replacement contact even
when it reuses the same pointer ID. Cancellation is now recorded independently
from rejected pan direction so pinch policy does not infer cancellation from pan
axis decisions.

Regression source covers recognition slop, scale change, contact replacement,
cancellation and absent pinch permission. Candidate dispatch, pointer suppression,
coordinate mapping, native root/iframe authority and painting are still pending;
this source does not yet enable working pinch input. No builds or tests ran.

### Pinch dispatch source integration

Native pointer registry now retains a two-contact pinch candidate admitted only
by a native top-level RealmDescriptor. Recognition commits suppression for both
contacts before pointercancel/touchcancel callbacks, releases capture and clears
click/selection state. Either contact ending removes the candidate; both physical
releases remain consumed. Document/input reset removes candidate state. Generated
regression source covers two-contact scale change, cancellation order and release
suppression.

This is not complete working pinch/zoom: native coordinate mapping, renderer
consumption, frame-spanning contacts, viewport metadata policy and reentrant
multi-contact refinement are still open. Source only, no build/test execution.

### Reentrant pinch update ownership

Pinch candidates now retain an update revision in addition to both exact contact
identities. Pointercancel, touchcancel, lost-capture and boundary callbacks are
followed by ownership checks before further dispatch or geometry publication.
Native input reset or a newer reentrant update therefore cannot be overwritten
by an older stack frame. Added a native embedding-callback regression that resets
input and replaces viewport geometry during pointercancel; no public privileged
input hook is installed by that test.

Source-only progress. No builds/tests/generators have run; complete coordinate,
paint and frame integration remain outstanding.

### Shared viewport input/paint contract

Moved visual viewport Geometry into oryn-common::visual_viewport so native input,
capture and rendering share one serializable transform. V8BindingState exposes a
native geometry accessor; paint revision tracking includes visual viewport state.
Boundary validation rejects nonfinite dimensions/scale/offsets, nonpositive scale
and offsets outside the visible range. Added shared inverse-mapping and invalid
geometry regression source. Layout generation owns snapshot/capture/render
consumption of this contract.

WebDriver's existing layout-CSS coordinate contract is preserved pending physical
input-boundary integration; a transform is not applied indiscriminately to all
client coordinates. Full viewport behavior remains unverified and incomplete.
No builds/tests/generators ran.
