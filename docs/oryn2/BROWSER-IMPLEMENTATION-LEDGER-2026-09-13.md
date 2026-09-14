# Browser architecture implementation ledger

Implementation has started against the existing dirty working tree based on
`42cc4ae02ecc96a99c4c652201b0b61a5cd5b8c0`. This is not a clean release or a new
WPT baseline. Existing source changes and historical evidence remain intact.

Companions: [architecture](BROWSER-ARCHITECTURE-AND-MIGRATION-2026-09-13.md) and
[reviewed work packages](BROWSER-IMPLEMENTATION-READINESS-2026-09-13.md).

## Implemented foundation

- Process-local arena identity and qualified `NodeRef`, with checked resolution.
  Independent arena snapshots receive distinct identities. Rebuild invalidates
  identity; exhausted node generations retire instead of wrapping.
- Wrapper lookup, identity caching and weak release check arena ownership.
  Exhausted wrapper IDs fail before acquiring a native root. Wrong-arena weak
  releases remain queued for their owner rather than mutating unrelated nodes.
- Structured tuple/opaque origins shared by the parent and realm registry.
  V8 security tokens use structured identity, not serialized `null`. Explicit
  inheritance clones opaque identity; parsing `null` creates a fresh identity.
  Dynamic realm allocation is checked; replacement checks capacity before
  detaching the old global.
- Ordered content attributes keyed by expanded name. Attribute insertion/removal
  is private to the DOM crate. Distinct namespaces can retain equal qualified
  names; value updates preserve the existing prefix, Attr identity and position.
- Namespace-aware Attr access/removal and indexed attribute exposure, clone and
  editing snapshot replay, named-window/collection lookups, and selected HTML
  reflection paths. Qualified-name `getAttribute` remains a distinct operation.
- Native selectors read content records directly instead of scanning all Attr
  nodes. Layout snapshots carry namespaced records to Stylo instead of flattening
  them into a string map. Null-namespace compatibility input remains supported.
- Fixed a discovered layout panic: validity state must not be applied to text
  or comment mirror nodes.

No worker placement default, broker authority, wire protocol, pool policy, or
cross-page process sharing was changed.

## Continued implementation

- R2 content records now use one immutable shared allocation for the element and
  attached Attr. Central mutation replaces that allocation; detached Attrs and
  independent snapshots retain their prior values. Native snapshot/identity tests
  cover this directly rather than inferring it from serialization.
- NamedNodeMap is now a live, branded, SameObject collection with indexed/named
  access, attribute-node replacement/removal, ownership errors and GC tracing.
  Mutation records carry local names. Native node equality compares namespace
  identity and ignores attribute insertion order. HTML control, collection, style
  and focus consumers use null-namespace lookup, distinct from getAttribute.
- R3 has started: accepted submissions retain native form, submitter and document
  leases, source and document generation. Queues are bounded and stale-generation
  submissions are rejected. The semantic host calls native requestSubmit logic
  without consulting replaceable JavaScript globals or prototypes.
- FormData construction is extracted into a shared native module with submitter
  validation, button/image entries, empty file entries and guarded FormDataEvent
  callbacks. Standalone bootstrap no longer replaces native Blob/File/FormData.
  The later owned-body work below connects this extraction to navigation.

- Selected-file bytes now live in native input state, not authored attributes.
  Native branded FileList supports assignment, indexed access and stable identity;
  FormData uses the actual selected File objects. Clone/import preserve selections,
  while reset, value clearing and transitions out of file state clear them.
  Parent-authorized uploads and the byte-only automation bridge populate this state.
- Parent upload execution no longer follows selection with a second typing action
  (which cleared the selection and duplicated events). Selection uses the existing
  action result, observation, trace and navigation processing once.
- Multipart encoding now uses fresh OS-random boundaries, CRLF normalization for
  string fields, percent escaping for disposition parameters and unchanged file
  bytes, following the [HTML encoding algorithm](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#multipart-form-data).

- Submission entry lists and formdata callbacks now run synchronously in the
  native submission algorithm. Action/method/enctype/target are read afterwards;
  owned URL-encoded, plain-text and multipart POST resources survive scheduler
  checkpoints. Planned submissions from one form supersede earlier ones.
- Top-level, named-frame and popup loaders carry the body without a JavaScript
  location-assignment shim. Parent authorization remains in the submission/body
  paths. Redirects 301/302/303 discard POST resources; 307/308 preserve them.
  History entries retain POST resources; reload resubmission rechecks parent
  form policy. The additive GotoResource command retains v9 page/session routing,
  existing frame bounds and the navigation watchdog; it is not R6 multiplexing.
- DOM now owns the existing ICU-based directionality algorithm, shared with
  layout and dirname entries. Charset-name matching is ASCII case-insensitive.
  Frame sandbox form permission is native realm state; dialog submission has a
  no-navigation path. Targeted local tests cover these mechanisms.

- Follow-up native fixes provide live FormData pair iterators/forEach, stable set
  position, Blob-to-File conversion and current/preserved File timestamps.
  Image-button coordinates are native state read before submit callbacks, with
  pointing coordinates distinct from HTMLElement.click; dialog return values
  consume those coordinates. Regression controls failed before these repairs.
- Parser-time pending submissions now precede dynamic frame load dispatch, as
  they already do in ordinary checkpoints. A regression reproduced an initial
  blank-frame load overtaking POST delivery and passed after the ordering fix.
- POST loaders reject non-HTTP resources and header injection, and top-level/
  frame redirect paths recheck parent form policy before forwarding a body.
  Redirect confirmation currently fails closed when a fresh confirmation is
  required; complete source-environment/target policy routing remains open.
  Supersession replaces a queued submission even at the action capacity limit.

Still open in R3: complete image geometry qualification, legacy document/accept-charset encoding,
custom-element contributions, complete target/rel policy semantics and whole-family signed gates.
Do not describe this as complete submission support.

The owned-body source snapshot built and signed successfully, including native
handshake and filesystem/network/environment denial checks. Before subsequent
repairs above, its full native suite passed 448 tests (6 existing ignores) in
654.60 seconds; JS passed 247 (3 ignores), DOM 89. Signed diagnostics selected
18 FormData window executions (12 passed) and 35 form-submission executions
(9 passed), with zero environment blocks. These failed family results are not
full variant coverage or conformance gains. Their exact hashes and source identity
are preserved separately in the owned-submission diagnostic evidence record.

## Characterization and local verification

Before the identity fixes, three added controls failed: cross-arena wrapper
resolution, opaque-origin equality and normalized tuple-origin equality. Before
the attribute change, the insertion-order control returned alphabetical order.
These are regression controls, not evidence of an externally exploitable escape.

`test-harness/architecture-attribute-contracts.html` is a development probe, not
WPT. Its executable test runs through native bindings and covers insertion order,
same-qualified-name namespaces, prefix/Attr identity, namespaced computed style,
null-namespace reflection, cloning, selector matching and detached Attr ownership.

Verified during implementation:

| Local suite | Result |
| --- | --- |
| DOM library | 86 passed |
| V8-backed JS library | 246 passed, 3 existing ignores |
| Layout library | 74 passed |
| Native host | 25 passed |
| Native executable | 163 passed, 6 existing ignores |
| Generated WebIDL consistency | passed |

Scoped library clippy (warnings denied), formatting and the no-default-features
native compile check passed. The first whole-native-library attempt encountered
tool-sandbox loopback binding denial and was interrupted. HTTP/2, asynchronous
XHR and cross-origin credential tests passed on the permission-corrected rerun;
the complete rerun subsequently passed 440 tests with 6 existing ignores in
665.08 seconds, before the final prefix correction below. Do not count the
interrupted run as successful. The attribute development probe contains eight checks.

The initial signed attribute slice completed all three selected files with no
environment blocks, but failed one assertion: setAttributeNS incorrectly changed
an existing prefix. That exposed an incorrect expectation in the local probe,
which has also been corrected. Value updates now preserve the existing prefix;
setAttributeNode replacement installs the new Attr's prefix. This follows the
[DOM attribute-value algorithm](https://dom.spec.whatwg.org/#concept-element-attributes-set-value).
The initial signed result remains at /private/tmp/oryn-architecture-attributes-20260913.json;
it must not be relabeled as passing or silently overwritten.

The corrected signed attribute slices passed all 11 selected HTML/window files.
An added structural control (`dom/attributes-are-nodes.html`) then exposed three
failures: appendChild/insertBefore/replaceChild accepted Attr children. The
binding pre-insertion validator and native arena/runtime edge checks now reject
Attr tree edges before mutation. The first structural failure remains in
/private/tmp/oryn-attributes-corrected-attributes-are-nodes-20260913.json.

Final signed rerun results, when available, are recorded separately in
[the diagnostic report](../../benchmarks/evidence/browser-architecture-2026-09-13.json).
That report is excluded from source-tree hashing and cannot establish clean
release qualification or completion of the remaining work packages.

These tests do not establish upstream conformance or signed sandbox qualification.
The final handoff records any subsequent build/smoke results separately. The
release source-tree hash, when built, identifies the exact dirty source snapshot;
it must not be represented as clean qualification evidence.

## Remaining package exits

### Image-resource and signed-family follow-up

The second signed diagnostic improved the selected FormData window slice from
12/18 to 17/18; the selected submission slice remains 9/35. Neither run had
environmental blocks. Exact hashes and immutable raw-result references are in
[the corrected diagnostic](../../benchmarks/evidence/browser-owned-submission-corrected-2026-09-13.json).
These are selected executions, not all upstream variants or a new full baseline.

The remaining FormData coordinate failure exposed missing initial image-button
loading and intrinsic SVG sizing. Local changes now prepare images before the
document load event, including child-realm loaders, without traversing another
document's resource ownership. A typed parent image request applies `img-src`
instead of `connect-src`, including redirects, and rejects non-GET/body payloads.
Decoded dimensions are native resource state, not authored width/height attrs.
SVG sizing uses a validated SVG root; SVG rasterization is not implemented here.
Layout preserves intrinsic ratio with one CSS dimension, including border-box
deductions. Old request generations cannot overwrite newer resources; encoded
resources are capped at 16 MiB; excess requests defer within the existing
32-operation bound. Removing `src` clears resource dimensions.

Local regression coverage includes SVG coordinates at window load, a 40-image
document, stale completions/source removal, SVG root validation, image policy
denial, and intrinsic sizing. DOM 89 and layout 73 tests pass; JS 251 tests pass
with 3 existing ignores (before the subsequent deferred-request change). The
native image regressions pass. Fresh signed qualification of these image changes
is pending; do not attribute their results to the earlier signed binary.

Image CORS/taint semantics, complete SVG sizing/rasterization, full form encoding
and target scheduling, and the architecture packages below remain open. This
follow-up is not an R3/C0 package completion claim.

The first signed image attempt is preserved in
[its failed diagnostic](../../benchmarks/evidence/browser-image-initial-2026-09-13.json):
450 native tests passed, one slow-image regression failed, six were ignored;
the selected FormData slice remained 17/18. Follow-up repairs defer document
load events until live image requests finish without holding the page actor.
DOMContentLoaded remains separate, and lifecycle traces no longer claim Load
while those resources are pending. Image operations retain their native target;
removed or superseded requests do not delay document load. The slow-resource
event/trace regression and native-retention regression pass locally.

The remaining coordinate mismatch reproduced with blank lines around the form:
collapsible block whitespace created spurious line boxes. Layout now suppresses
those empty boxes without removing DOM text, preserving preformatted whitespace
and non-breaking spaces. The whitespace-bearing coordinate regression passes
at 23,46; a fresh signed rerun is still required. All 73 layout tests and the
169 active executable/frame tests passed before the last retention/trace edits.

Latest native-file/multipart follow-up verification (local source, not signed
release evidence): DOM 87 passed; V8 JS 247 passed / 3 existing ignores;
layout 74 passed; native executable 164 passed / 6 existing ignores.
The parent-authorized upload integration initially failed because its follow-up
typing action cleared the new selection; after repairing the action pipeline,
that test passed with explicit single-event-pair and File-identity assertions.
The expanded FileList brand/null/Proxy regression also passed after its final edit.
Scoped library clippy with warnings denied, generated WebIDL consistency,
formatting, diff whitespace checks and the no-default-features native compile
passed. The complete native suite was not rerun for this follow-up, and existing
signed attribute evidence above describes the earlier source, not these changes.

Latest signed image correction: 451 native tests passed, six ignored; selected
FormData 18/18 and submission 11/35, with no environmental blocks and no
file-level regressions against the previous submission slice. Exact provenance
is in [the corrected image diagnostic](../../benchmarks/evidence/browser-image-corrected-2026-09-13.json).

The subsequent form-reflection follow-up adds form submission properties and
submitter overrides to generated WebIDL, preserving native content attributes,
known-keyword defaults, DOMString/USVString conversion and abrupt exceptions.
HTMLBaseElement now exposes href/target; empty actions use the document URL,
not the base URL. Native form target selection honors the first base target,
explicit empty overrides and dangling-markup target sanitization. Activation
checks the input type after click handlers, including changes from non-submit
types. No authored navigation transport or change to process isolation is added.

Local follow-up validation: 254 JS tests pass, three existing ignores; the native
POST regression passes all twelve target/encoding combinations (frame, popup,
parser-created frame, base target; URL-encoded, plain and multipart bodies).
Scoped library clippy passes with warnings denied. Fresh full-native and signed
WPT verification of this follow-up is pending. Frozen base-URL lifecycle,
document ownership, named targets and remaining R3 obligations are not closed.

| Package | Status and next obligation |
| --- | --- |
| R0 | Regression controls added; full package/provenance automation remains. |
| R1 | Identity foundation implemented; environment/agent/cluster IDs and document-scoped ownership cutover remain. |
| R2 | Shared records, live NamedNodeMap, local-name mutations, equality and remaining identified null-namespace consumers implemented. Broader upstream family and signed verification remain. |
| R3 | Rooted/bounded submissions, synchronous native entry construction, file selection and body-carrying navigation implemented. Remaining entry/encoding/target obligations and full signed family gates remain; package is not closed. |
| R4–R5 | Live frame loader uses explicit same-arena hosted-document associations, with separate layout/retention edges and retired-document semantic filtering. Full document-scoped ownership/adoption, agent scheduler ownership and lifecycle cutover remain. |
| R6–R7 | Routed process-session protocol and connected-window placement remain behind their explicit security/placement gates. |
| R8–R9 | Independent worker agents and shared/service lifecycle remain. |
| W0–W2 | Upstream product integration and general automation transport remain. |
| C0 / Q0 | Platform gaps and clean signed qualification remain. |

Next: qualify the current submission paths and complete remaining R3 obligations before document/agent cutovers. Do not infer
completion of the architecture or an increased WPT pass rate from this tranche.

## Related-window direction approved; frame association migration

The user approved the stated opt-in related-window process-sharing direction by
replying "Proceed": related windows may share a crash boundary, independent root
groups remain separated, and isolated execution remains the default. This does
not activate sharing. R6 routing review, bounded hosted-environment quotas,
stress evidence and revised security attestation remain required before R7
activation. No clean-source snapshot/commit authorization is inferred.

The R4 follow-up replaces both static and dynamic same-origin iframe-to-Document
child edges with a native hosted-document registry. Snapshots preserve the
registry for explicit layout traversal. DOM parents/children, cloning/importing,
and selectors do not traverse hosted documents. Retention follows a separate
host edge; replacement disconnects the old document without destroying nodes
still held by native/wrapper leases. Internal association notifications update
wrapper retention and semantic projection without exposing child-list mutations.
Retired or detached hosted documents are excluded from semantic observations.
Node.isConnected recognizes document-rooted trees independently of activity.

Local tests cover frame cloning/import, ordinary document import, association
retention/replacement, separate parent/child boundaries, and stale semantic
projection removal. The initial executable suite found a primary-document-only
isConnected assumption; after repair, 170 tests passed with six existing ignores.
The 15 semantic tests pass, including retained/detached frame cases. Full native,
signed sandbox and upstream validation of this snapshot are pending. The older
signed form-reflection evidence is not evidence for this cutover. The deferred
Range.insertNode crash is still unresolved until a faithful signed rerun proves
otherwise. Full R4/R5 and conformance are not claimed complete.

The initial full native gate caught a remaining DOM-parent-only action check:
452 tests passed, one same-origin frame action failed, six were ignored. The
failed signed snapshot and a still-crashing Range.insertNode rerun are preserved
in the initial frame-association diagnostic. Follow-up routing validates page
and generation as before, then follows explicit native embedding ancestry, not
DOM ancestry. A regression requires both successful connected-frame actions and
rejection after detachment. Fresh full/signed validation remains pending.

## R4 document-boundary follow-up

The arena-wide native ID cache has been removed. Primary-document convenience
lookup now delegates to the same document-rooted, tree-ordered algorithm used by
Document.getElementById, preventing hosted-document collisions and incorrect
duplicate-ID ordering after moves or attribute changes.

Native insertion entrypoints now reject Document-as-child edges and cycles in
the explicit embedding graph before modifying nodes, ranges or mutation records.
The remaining legacy frame-tree test setup uses the hosted-document association.
Invalid insertBefore reference checks precede Range adjustments, including the
foreign-reference self-insertion case; direct arena self-insertion is a no-op.

Native focus retargeting follows explicit document hosts and shadow hosts.
Document.activeElement uses this scoped result, and returns null for inactive
retained documents rather than an unrelated realm's fallback. Tests cover nested
frame focus, child-to-parent focus changes, removal, event/contains boundaries,
duplicate IDs and rejected-mutation atomicity. The real-frame integration test
initially exposed the retired-document fallback and passes after its repair.

These changes do not implement native ownerDocument/adoptNode or AgentRuntime.
Full local regression validation is in progress. Earlier signed evidence remains
evidence for its recorded source only; no new signed/WPT qualification is implied.

The subsequent receiver audit found document.open/write/close replacing the
arena's primary document even when the receiver was a hosted document. Native
replacement now requires an explicit validated Document receiver. Buffered write
sessions are keyed by document, keep the existing aggregate 1 MiB budget, and
hold/release native document leases on open/close and binding teardown. Parser
commands and current-script insertion are restricted to their addressed document;
closing an inert document does not execute its scripts in the caller's realm.
Interleaved two-frame writes preserve parent/sibling identity and execute scripts
in the corresponding frame realms. Same-realm detached-document tests cover
separate buffers, reopening and aggregate-budget rejection/recovery.

This corrects document targeting, not the full HTML dynamic-markup algorithm:
incremental parser behavior and authoritative native document ownership remain
open obligations. The earlier full native attempts are preserved as incomplete
diagnostics (one restricted by local-server permissions, one paused for this
repair); the complete regression gate must run against this follow-up source.

## Native ownership and adoption follow-up

Native nodes now carry authoritative document ownership; the realm-local
node-to-document map is removed. Explicit adoptNode detaches without cloning,
preserves wrapper identity, listeners and expando state, and updates descendants
and attributes. Native insertion performs implicit adoption. Ownership tracing
retains documents reachable through detached wrappers without making template
contents or adopted stylesheet cycles permanent native roots.

Same-arena document realms reuse existing node wrappers and trace their owner
documents through collectable V8 references. A real-frame regression covers
two-way adoption and ownerDocument identity. This is wrapper routing inside the
existing isolate, not the planned multi-environment process dispatcher.

Current local checks: DOM 98 passed; JS 256 passed, 3 ignored. Broker unbinding
now marks retained network-route clones revoked as well as shutting down context
services; a focused regression passes. This rejects calls begun after revocation
and suppresses subsequently received responses/events. Atomic cancellation of
in-flight dispatch and per-environment route generations remain R6 obligations.

Completion gates still open: inert template document ownership, custom-element
adoption reactions, additional cross-realm GC coverage, AgentRuntime/environment
dispatch, process-session protocol migration, full native regression, and fresh
signed sandbox/WPT qualification. Historical signed results do not qualify this
source. The opt-in sharing modes remain inactive and protocol remains v9.

## Agent ownership and environment retirement follow-up

The document driver now owns an AgentRuntime which owns the RawV8Host and shared
DOM arena, instead of owning the isolate adapter and arena as unrelated fields.
Construction validates the primary realm's arena. Opaque cross-origin contexts
retain their existing separate arenas under legacy co-hosting; they are not
reclassified as the same standards agent. This preserves current process
placement; it does not implement connected-page placement.

AgentEnvironments allocates non-reused, exhaustion-checked environment identities
and records realm, document generation and owned-worker ancestry. Native context
slots carry these identities. Timer scheduling now uses its actual environment
instead of the hard-coded realm 0/generation 1; cross-environment timer cancellation
is rejected. Retirement cancels owned timer queues, discards pending fetch
resolvers and late completions, and retires owned worker environments. Retained
retired contexts cannot schedule timers, raw fetch/context calls or module fetches.
Generated platform endpoints have an explicit retirement operation: retained
endpoint clones reject new work and queued/late results are suppressed. Detached
DOM references are not invalidated merely because execution services retire.

Each ordinary document now has its own associated inert template document;
nested templates reuse that inert owner. Creation, parser insertion, import,
clone and adoption route template content ownership through this native edge.
Native graph tracing retains reachable inert owners and collects unreachable
cycles. Tests distinguish ordinary, nested and adopted ownership, including
HTML/XML metadata. This implements the ownership algorithm from
https://html.spec.whatwg.org/multipage/scripting.html#the-template-element.

Focused native tests cover frame-timer retirement/sibling isolation, late fetch
completion rejection, retained-platform-endpoint rejection, and adopted wrapper
identity through source-realm retirement plus forced V8 GC. Current local JS
suite: 258 passed, three ignored; DOM: 99 passed. Strict library Clippy passes.
Full native and signed/WPT validation are tracked in a new evidence snapshot.

Still not complete: custom-element adoption reactions, process-session ownership
and multi-environment broker routing/wire migration, parent-authoritative atomic
in-flight revocation, connected-window placement/quotas, and clean-tree release
qualification. The current environment registry is agent-local and is not a
parent-issued broker credential. Do not label these changes as protocol v10 or
enable process sharing based on them. Historical Range crash remains deferred.

The first full regression attempt caught an over-strict arena check that rejected
legacy opaque cross-origin contexts. That interrupted failure is preserved in
`browser-agent-retirement-initial-2026-09-13.json`; the corrected executable suite
passes 173 tests with six ignored. A reset audit also found that local close only
queued shutdown. Close now acknowledges after PageState and its receiver are
destroyed, and the worker broker is unbound before Reset is sent, not after
ResetReady. This prevents a reset acknowledgement from racing the old isolate's
destructor; it does not replace the remaining parent-side in-flight route drain.

## Broad-family follow-up

The signed agent-retirement snapshot passed 462 native tests (six ignored),
adoption 3/3 files, timers 6/13 files, and templates 7/67 files. The latter two
are failing qualification results, not conformance claims. Complete results and
artifact hashes are retained in `browser-agent-retirement-signed-2026-09-13.json`.

The template family exposed missing host-including cycle validation, which
could exhaust worker RSS. Native template fragments now preserve their host
edge across adoption; insertion and binding prevalidation share the native
hierarchy check. This internal edge does not affect parentNode, contains or
connectivity. GC traces it without making it a permanent root. DOM and binding
regressions cover nested hosts, adoption, mutation-free rejection and collection.
The DOM suite passes 101 tests. Timer delays now perform WebIDL signed-long
conversion before negative clamping; initial zero-delay intervals no longer
receive an incorrect extra millisecond. A regression covers wrapping, initial
interval/timeout ordering, cancellation and conversion exceptions.
The broader host run then exposed stale cancellation bookkeeping: future and
already-queued interval occurrences survived in the scheduler after clearing.
Cancellation now removes both by their native timer identity and does not retain
unknown-ID tombstones. A deterministic scheduler regression covers peer-task
preservation and the absence of cancelled wakeups.

These subsequent fixes require a separate signed snapshot. Synchronous initial
iframe environments, complete timer error/nesting semantics, newer template
features and the architecture completion gates above remain open.

The subsequent timer error-reporting repair dispatches ErrorEvent in the
callback's creation realm, distinct from both the timer-owning Window and the
thrown object's realm. Uncancelled ordinary exceptions no longer terminate the
event loop; watchdog termination remains fatal. Strict timer callbacks receive
the timer-owning global as their receiver. Native regression coverage separates
these realms and verifies that peer tasks continue after an uncancelled error.

## Parent admission/drain barrier

Worker routes now use a parent-owned admission barrier shared by network and
context dispatch. Admission and revocation serialize on one lock. Synchronous
network/context requests hold an admission through dispatch; asynchronous network
forwarders retain theirs until completion/cancellation. Revocation rejects new
admissions, cancels owned network IDs and context resources, and waits at most two
seconds for admitted work. A second cleanup runs after successful drain to catch
resources registered by previously admitted requests. During drain the router is
explicitly unavailable; timeout permanently disables it and the caller kills the
worker instead of resetting/recycling. Unknown cancellation IDs are not forwarded
to underlying network authority. Admissions are bounded at 64 across services.

Thirteen worker-client tests pass, including asynchronous operation lifetime,
cancel-to-drain, stale retained authority, bounded admission, concurrent rebind
rejection and permanent retirement after a failed drain. This is not the R6 wire
migration: delayed IPC frames still require explicit parent-issued environment
credentials rather than current-route selection. No process-sharing activation
or protocol-version change is justified by the admission barrier alone.

## Isolate-owned watchdog follow-up

The drain-barrier snapshot's full native run failed the unchanged 250 ms image
responsiveness assertion (468 passed, one failed, six ignored). A focused repeat
passed; neither result establishes the cause. Investigation found one watchdog
thread spawned for every JavaScript execution, including many executions per
checkpoint. RawV8Host now owns one reusable watchdog thread. Deadline admission,
expiry and cancellation serialize on one schedule; expired generations cannot
poison later scripts. Nested deadlines retain the earliest deadline, registrations
are bounded and IDs fail closed on exhaustion. Shutdown joins the watchdog before
isolate destruction. Tests cover these properties and infinite-loop termination.
The image fixture also releases blocked I/O on assertion failure without changing
its 250 ms deadline. New signed/full-suite qualification is required; the previous
failure remains preserved and is not retrospectively relabeled as resolved.
