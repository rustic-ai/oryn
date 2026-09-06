# Foundation diagnostics, 2026-09-06

Source baseline: fbb2f29. Experiments used isolated crate copies under /private/tmp; production source, limits and signed worker stayed fixed during the full run.

## Detached wrapper tracing

Retaining newly created detached elements exposed quadratic full-wrapper-graph rebuilds. Measured release-build timings (milliseconds):

| Nodes | Baseline | Prototype |
|---|---:|---:|
| 100 | 2.520 | 0.442 |
| 500 | 64.884 | 0.560 |
| 1000 | 281.371 | 1.429 |
| 2000 | 1178.295 | 2.532 |
| 4000 | 4908.514 | 5.396 |

Prototype attaches the existing owner-document wrapper directly only when the sole mutation since the last valid graph is creation of that isolated leaf. Other changes retain full graph refresh. Mutation journal cursor lookup also avoids scanning unrelated older entries. All 101 V8 binding tests (two existing ignored) and 29 DOM tests passed, including retained-leaf/owner/sibling GC and journal wraparound regressions; strict production-library Clippy passed. These are diagnostic measurements, not signed-worker conformance results or hard timing guarantees.

## XML parser suitability

xml 1.4.0 was evaluated in an external diagnostic crate, without adding a production dependency. Strict single-root mode must be explicitly enabled; default configuration permits multiple roots. Initial tests covered namespace resolution, DTD entity expansion, CDATA, comments, processing instructions, mismatched tags, unbound prefixes, literal and expanded duplicate attributes, multiple roots, and outside-root text.

Expanded checks found that the unmodified library accepts duplicate namespace declarations and aliases for the reserved XML and XMLNS namespace URIs. StartElement events also omit explicit xmlns attributes, including redundant declarations. It is therefore not yet suitable as a drop-in browser XML parser. Integration requires a narrowly tested upstream parser correction or another parser that meets these contracts. Do not silently reconstruct declaration attributes from inherited namespace maps.


The follow-up isolated parser patch preserves namespace attributes behind an explicit configuration option, rejects duplicate declarations and reserved-URI aliases, rejects xmlns-prefixed elements, and permits ordinary relative default namespace names. The expanded diagnostic cases and all 38 upstream library unit tests pass. This remains a candidate, not an integrated production parser. Namespace constraints were checked against [Namespaces in XML 1.0](https://www.w3.org/TR/xml-names/#ns-decl).

Integration audit found another prerequisite: `DomRuntime::import_subtree` and `clone_node` copy the element attribute string map but do not reconstruct its namespace-bearing Attr records. XML document import must preserve these records and their owners; otherwise a correct parser would lose namespace information at activation. Add native import/clone namespace regressions before closing the XML workstream.


After the clean baseline completed, the detached-leaf fast path and journal cursor
lookup were integrated into the repository. Locked production-feature checks pass
101 V8 binding tests (two existing ignored) and 29 DOM tests. The large-leaf test
also verifies native reclamation after the last JavaScript reference is removed.
Strict Clippy and formatting pass. Signed-worker results remain pending and must
be recorded separately from the fbb2f29 baseline.


## Signed validation of the detached-leaf fix

Commit `038f7aa`, signed worker
`a6ce0ceabc8e178de5ebe1af9480a258ddee421fdb7e43dc1ef93cbb313359ed`:
complete DOM selection 333/664 (331/662 DOM plus two accname files), no file gains
or lost passes versus the clean full baseline. Every selected ID and passing
completion was verified. `name-validation.html` completes with harness status 0:
two assertion groups pass; missing createAttribute/createAttributeNS and NUL
acceptance in doctype names account for its three remaining semantic failures.
This replaces the previous runtime wall-time outcome without changing limits.
`Range-mutations-splitText` also reaches assertions from an incomplete outcome;
`moveBefore/iframe-document-preserve` changes from incomplete to harness failure.
These failure-mode changes remain open and are recorded in the signed summary.

A separate mixed-lifetime GC diagnostic retained half the wrappers and reclaimed
half. Release processing took 1.220, 3.548, 11.882 and 45.857 ms for 2,000, 4,000,
8,000 and 16,000 wrappers respectively. Queue cancellation scans once per live
wrapper, so this identifies another quadratic path for a future bounded fix;
it is not proof of the cause of any particular remaining WPT failure.


## Attribute creation and validation batch

The next implementation adds native `Document.createAttribute` and
`createAttributeNS`, shares namespace extraction with element creation while
using attribute-specific local-name rules, and rejects NUL in doctype names.
Attribute getters/mutations apply HTML case folding at the binding boundary;
XML names remain case-sensitive. Native Attr materialization preserves literal
colon names, and qualified-name value updates preserve existing namespace
metadata and object identity. String conversion failures preserve the original
JavaScript exception. Fresh detached Attr/doctype wrappers use the verified leaf
fast path, with owner-document GC coverage for Attr.

Locked checks pass 102 V8 binding tests (two existing ignored), 30 native DOM
tests, generated-binding verification and strict Clippy. Signed conformance
validation is pending; this does not change the recorded full baseline score.


Signed validation at `fba789e`, worker
`3aa9da604e334c8bca0e6d22571eefe0456ebf127558522363ef2bf769fcf36b`:
name validation passes all five groups; complete DOM selection passes 340/664
(338/662 DOM plus two accname files), seven gains and no lost passes; complete
CSS selection passes 231/569 (226/564 CSS plus five referrer files), unchanged.
Exact selected IDs and every passing harness completion were verified.
The seven new DOM file passes are historical interfaces, Document-createAttribute,
Element-hasAttribute, Element-setAttribute-crbug-1138487, case, name-validation,
and querySelector-mixed-case. The clean full baseline remains 2,501/4,022.


## Mixed-lifetime release queue

Release cancellation now filters the weak-release queue once against live wrapper
IDs, preserving the existing stale-finalizer protection. Measured isolated timings
for 2,000/4,000/8,000/16,000 wrappers were 0.624/0.933/1.939/3.959 ms, compared with
1.220/3.548/11.882/45.857 ms before. Both runs released exactly half the wrappers.
Locked repository checks pass 103 V8 tests (two existing ignored), including bulk
cancellation and native reclamation with half the wrappers retained; strict
Clippy passes. This optimization has no separate signed ledger yet. It will be
included in the next foundation candidate; no WPT gain is inferred from timings.


## Native attribute ownership and namespace preservation

A reduced native regression proved that releasing an Attr wrapper could free its
record while its owner element remained connected, leaving a stale cache entry.
The attribute index now lives in DomArena, survives snapshots, and is updated
when records are inserted/restored/reclaimed. Native retention includes owner/Attr
edges; V8 traces attached wrappers with their owner trees while preserving the
DOM's null parentNode and false isConnected for attributes. Removal preserves the
attribute's node document. Clone/import preserve namespace metadata, and cloning
an Attr clears its ownerElement. html5ever's namespaced foreign attributes now
retain their qualified names and namespaces at the native boundary.

The batch also adds getAttributeNodeNS and the pinned boolean-depth importNode
contract, with ownership assigned through children, attributes and template
content. The newer scoped-registry import options are not implemented here and
remain outside this batch's closure claim. The stale-cache, namespace import,
connected/detached expando and GC regressions pass. Locked checks: 104 V8 tests
(two existing ignored), 33 DOM tests, 141 native runtime tests, generated bindings,
formatting and strict Clippy. Signed DOM/CSS validation is pending.

## Signed attribute ownership validation

Clean source `a67666b`, worker SHA-256
`fca22287bad3fa7eca76405ea21a56163e6a23c75f94836d03f8012224bbae39`:
DOM selection 343/664 (341/662 DOM plus two accname), CSS selection 232/569
(227/564 CSS plus five referrer). Exact selected IDs and all passing harness
completions validated. No lost passes against the attribute-creation candidate.
New passes: Attr-prefix, Document-importNode, Node-cloneNode-svg, and
css/selectors/quirks-mode-import. Raw ledgers remain in local artifacts; hash-bound
summaries are in CONFORMANCE-ATTRIBUTE-OWNERSHIP-2026-09-06.json.
The complete full result remains 2501/4022; these selections are not a release gate.

## XML document activation implementation

The native DOM now adapts the vendored xml 1.4.0 event reader into the existing
HTML tree sink. DOMParser dispatches its XML MIME types; static and dynamic frame
loading retain the response Content-Type through the parent broker. XML metadata,
qualified names, namespace declaration attributes, CDATA, processing instructions,
comments, doctype identifiers and internal entity text survive the native tree.
Generic XML elements receive Element wrappers; SVG wrappers require the SVG namespace.
Malformed input replaces the partial tree with a parsererror document and drops
partial resource events. HTML/SVG resource discovery is namespace-aware.

The network decoder recognizes XML encoding declarations and UTF-16 signatures,
with existing BOM and HTTP charset precedence. See the XML 1.0 encoding discussion:
https://www.w3.org/TR/xml/#sec-guessing-no-ext-info . DOMParser receives Unicode and
therefore does not reinterpret its string using an encoding declaration.

This is foundation implementation, not closure of XML conformance. External entity
loading, DTD attribute defaults/validation, XML fragment parsing/serialization and
full XHTML template behavior remain open. Top-level navigation MIME dispatch also
requires a separate audit; this batch routes DOMParser and frame responses.
Production DOM tests: 38 pass; V8 tests: 105 pass, two existing ignored;
native runtime tests: 141 pass. Network prototype tests: 11 pass. Strict production
DOM/network/V8 Clippy and formatting pass. Vendored XML: 38 unit tests and 14
doctests pass, two existing doctests ignored. Signed-worker XML evidence is pending.

## Parser-prefetched XML frames and element interfaces

The signed d27f2b6 Document-createElementNS diagnostic completed normally but still
failed 390 XML/XHTML assertions: the parser scheduler had stripped MIME metadata
before realm construction. The focused ledger is hash-bound in
CONFORMANCE-XML-PREFETCH-DIAGNOSTIC-2026-09-06.json. The follow-up carries a typed
LoadedFrameResource from asynchronous completion through LiveParseResult into
initial frame construction, retaining final URL, body and Content-Type together.

HTMLSpanElement and HTMLUnknownElement now have generated interfaces backed by
native nodes. Interface selection distinguishes known, unknown and valid custom
HTML names and remains namespace-sensitive. Existing known elements whose more
specific interfaces are absent retain their previous HTMLElement implementation;
this does not close the remaining HTML element-interface inventory.
Selection follows https://html.spec.whatwg.org/multipage/dom.html#elements-in-the-dom
and https://html.spec.whatwg.org/multipage/custom-elements.html#valid-custom-element-name .
Production V8 tests: 105 pass, two existing ignored. Signed follow-up validation pending.

## Signed XML activation results

Clean `9ffc345`, worker SHA-256
`de133caaa6f3213da115309770ccb01f1a8612599fb351e7ce298aea226a5d0f`:
Document-createElementNS passes all 596 assertions. DOM selection passes 349/664
(347/662 DOM plus two accname); CSS passes 232/569 (227/564 CSS plus five referrer).
Six DOM gains, no lost passes in either selection against the ownership candidate.
Exact selected execution IDs and every passing testharness completion validated.
Hash-bound summaries: CONFORMANCE-XML-ACTIVATION-2026-09-06.json.
The full baseline remains 2501/4022 until another complete run is recorded.

## Native DOM constructors, lookup and resource activation

Text, Comment and DocumentFragment constructors now allocate native nodes and bind
the actual constructed wrapper (including subclass prototypes). Text and Comment
reuse detached-leaf graph tracing. Defaults, string conversion exceptions, UTF-16
length, insertion identity and fragment extraction are covered by V8 tests.
Text.wholeText reads the contiguous native Text/CDATA sibling sequence.
DocumentFragment.getElementById is inherited by ShadowRoot; Document and fragment
lookups now search their own tree in order, excluding nested shadow trees and
empty IDs, without interpreting the ID as CSS syntax.

Static and dynamic script/frame collectors now check expanded names. Dynamic
image, referrer-meta and stylesheet-link preparation requires HTML elements;
script and embedded stylesheet activation accepts HTML/SVG elements. This closes an XML integration gap
where matching only local tag names could activate generic XML elements as HTML.
The regression checks generic XML and null namespaces against real HTML/SVG scripts.

Native DOM tests: 38 pass. V8 tests: 108 pass, two existing ignored. Strict DOM and
V8 Clippy pass; generated interfaces checked. Complete signed-worker evidence follows.

## Complete DOM-primitives baseline

Clean `73ee49b`, signed worker `3f198e05549dbbff88403eba4854ab40b9456a3f75f3cf53d9982443030a0436`:
**2527/4022** with 26 gains and no lost passes versus `fbb2f29`.
DOM passes 356/662; CSS passes 227/564. Other roots are unchanged.
Exact inventory IDs and every passing completion were independently verified.
The complete baseline and failure backlog now use this run. This remains a failing
conformance result, with 1495 open executions; it does not qualify a release.

Two still-failing executions changed failure domain: history_forward.html changed
from harness to runtime, and iframe-upgrade-request-to-cross-origin.sub.html
changed from runtime to timeout. Both remain open in the single-run backlog;
no successful execution was lost. These transitions need navigation/task diagnosis.

## Native observer records and event dispatch lifecycles

MutationObserver now keeps independent registrations per target, deduplicates
matching registrations, reads the native journal synchronously for takeRecords,
and uses observation-time tree ancestry rather than the final tree. Temporary
registrations preserve removed-subtree observation until notification. Updating
one target replaces only its registration; disconnect removes all registrations
and queued records. Queued old-value visibility preserves the options in effect
when each mutation was observed. Dictionary member order, presence-based defaults,
iterable attribute filters and original page conversion exceptions are covered.
Native attribute journal entries retain namespaces after mutation or removal.

MutationRecord now has branded readonly generated getters and native NodeList
snapshots. Its immutable binding payload retains actual native node wrappers.
Callbacks receive the observer as both receiver and second argument. This batch
follows the [DOM observer algorithms](https://dom.spec.whatwg.org/#mutation-observers)
and [Web IDL sequence conversion](https://webidl.spec.whatwg.org/#es-sequence).
Previous/next sibling snapshots, compound child-list coalescing, callback exception
reporting across realms and bounded-journal overflow remain open; this does not
claim complete MutationObserver conformance.

Event records now distinguish initialized and actively dispatched states. A native
scope guard clears dispatch/propagation state on exit, including suppressed dispatch
and error paths. initEvent performs reentrant string conversion before borrowing
native state, resets flags/targets when allowed, and cannot alter an event during
its dispatch. initCustomEvent also preserves detail during dispatch. The native
ancestor path is captured before global capture listeners run; disconnected trees
do not dispatch through the global object. Recursive and uninitialized dispatch
are rejected. Broader event interfaces, retargeting, composed paths, exception
reporting and trusted-event integration remain open.

Repository checks pass 110 V8 tests (two existing ignored), 39 native DOM tests,
strict V8/DOM Clippy, generated bindings and formatting. The V8-enabled native
compile check passes. Signed-worker validation follows.

## Signed observer/event lifecycle results

Clean `f4512fb`, signed worker `60970e97b737472e9dc6e96f7a2f82c9519a7a251d5fa34e2bc195ef1e5978d8`:
MutationObserver selection passes 2/12 (two gains). DOM passes 366/662 plus two
accname executions, CSS passes 228/564 plus five referrer executions, and IndexedDB
passes 341/456. There are ten DOM gains, one CSS gain, and no lost passes in any
selection. Exact selected IDs, passing completions, source/artifact hashes and
unchanged security/resource policy were independently verified. Hash-bound reports
are in CONFORMANCE-OBSERVER-EVENT-LIFECYCLE-2026-09-06.json.

Remaining observer failures exposed the Window bootstrap's getter-only ID bindings:
assignments such as `var n00 = document.getElementById('n00')` did not replace the
live accessor. Changing the ID could null the variable, or assigning firstChild
could leave a variable resolving to an Element instead of CharacterData. A separate
fix is being tested. This is not evidence that the existing appendData implementation
is absent. Mutation siblings, compound records and microtask ordering also remain
separate causes; first errors are not interchangeable with API inventory gaps.

## Sibling snapshots and Window assignment preservation

Native insertion/removal journal entries now snapshot previous and next siblings.
Ordinary insertion reuses the child indices already computed for live Range updates;
record getters do not reconstruct siblings from a later tree. Reparenting preserves
both removal and insertion positions, and parser detach records carry the same
metadata. MutationRecord exposes the original native sibling wrappers. Detached-leaf
GC fast paths accept the richer journal entry without changing resource limits.

The existing Window named-property adapter now lets assignments create ordinary
writable properties and removes those names from generated-binding tracking. Refresh
checks getter/setter identity before changing an existing property, preserving
explicit page definitions. Tests exercise parser-time var assignment, ID changes,
explicit defineProperty, element removal and subsequent refreshes. This fixes the
confirmed observer-test variable loss; it does not complete Window named-property
prototype placement, uninitialized var declarations, duplicate-name collections,
name attributes or frame-name semantics.

The standalone native-DOM/V8 diagnostic passes. Repository native DOM tests pass
40 cases; broader repository and signed-worker checks follow.

## Native normalization

Node.normalize now traverses light-tree Text descendants in document order,
removes empty nodes and merges adjacent exclusive Text nodes. CDATA and shadow
boundaries remain separate. The native algorithm moves live Range boundaries from
merged nodes and intervening parent offsets before removal; UTF-16 offsets and
mutation ordering are covered by native and V8 regressions. The generated Node
operation calls this shared Rust implementation. See the
[DOM normalization algorithm](https://dom.spec.whatwg.org/#dom-node-normalize).

The two parser-time Window integration tests pass with V8 and the V8 sandbox
features enabled. Combined repository checks pass 111 V8 tests (two existing
ignored), 41 native DOM tests, strict DOM/V8 Clippy, generated bindings and
formatting. The V8-enabled native compile check passes. Signed-worker validation
remains pending.

## Complete normalization and Window baseline

Clean `c12167f`, signed worker `15d09a52d087bf38138bca64241874ef6f9a4fb930ced54f1fc0b368913c5f34`:
**2539/4022**, 16 gains and 4 lost pass(es) versus `73ee49b`.
DOM passes 371/662; CSS 227/564. Exact IDs and every passing completion were
independently validated. The full baseline and failure backlog now use this run.

The lost CSS pass, `focus-within-009`, now reaches 12 assertions instead of one.
Those additional assertions expose missing focus state in the Stylo mirror and
focus retained across removal/moves. History cases `008`, `011`, and `012` retain
passing assertions in an isolated diagnostic probe, but their primary runs lack
verified harness completion. All four remain failures in this baseline; neither
expanded coverage nor passing partial assertions erase the regressions.
All three history cases passed isolated rechecks on the unchanged signed worker.
Their full-run failures remain in the baseline; completion intermittency is unresolved.

Parser script errors are currently retained as host diagnostics without Window
error-event delivery. The earlier one-assertion result demonstrates why reported
harness completions alone cannot establish exhaustive assertion registration.
Native exception reporting is an explicit W0/W3 completion prerequisite; this
baseline remains a diagnostic measure of reported outcomes.

The [baseline evidence](CONFORMANCE-BASELINE-2026-09-06.json) retains changed failure
domains and regression assertion counts. This is a failing conformance result with
1483 open executions, not release qualification.

## Compound replacements, token updates, and native focus projection

`textContent` and `innerHTML` now queue one native compound child-list observer
record with ordered added/removed node identity. The journal retains each structural
insert/remove for semantic, rendering, Range, connectedness and lifetime consumers;
only their observer delivery is suppressed. The operation groups records within
one Rust mutation, without guessing from adjacent journal entries. Empty-to-empty
replacement queues no record. Replacements preserve internal shadow roots, exclude
them from light-tree sibling snapshots, and mark only newly parsed scripts inert.
Document textContent is a no-op; Comment and Attr use their native update paths.
General fragment insertion, replaceChild/replaceChildren and outerHTML grouping
remain open; this does not claim complete compound-mutation coverage.

DOMTokenList add/remove accept zero or more tokens, convert all arguments before
validation, validate atomically, and update the ordered token set once. Empty and
ASCII-whitespace tokens throw their specified DOM exceptions; forced toggle no-ops
do not write the attribute. The WebIDL compiler now represents variadic arguments,
excludes them from required arity, preserves their element type, and rejects
optional/defaulted/nonterminal variadics. Remaining DOMTokenList members and indexed
or iterable behavior are separate gaps.

The CSS focus regression exposed an adapter gap: the private Stylo mirror did not
receive native focus, and Blitz returned false for FocusWithin. LayoutInput now
carries the focused native element; both synchronous and page layout paths project
FOCUS and ancestor FOCUS_WITHIN states directly into the mirror before Stylo runs.
The reused Blitz matcher reads that state. Native removal and ordinary moves clear
focus from the removed subtree, and detached focus attempts leave focus unchanged.
This does not claim full focusability, focus-visible heuristics, or focus-event
ordering conformance.

Prototype checks passed 113 V8 binding tests (two existing environment-dependent
ignored), 43 native DOM tests, eight WebIDL compiler tests, and 32 layout tests.
Strict relevant Clippy passed. Native integration and signed-worker evidence are
recorded below after running the integrated candidate. No inventory, sandbox,
heap/RSS or timeout limits were changed.

Integrated production checks passed 113 V8 binding tests (the same two ignored),
43 native DOM tests, eight WebIDL tests, 32 layout tests, and the native V8 focus
integration test. Strict V8/DOM/layout Clippy, generated bindings, formatting,
27 WPT-runner tests, documentation truth, and diff checks passed. The runner now
retains primary-run and probe diagnostics for incomplete recovered results.
Signed-worker checks are pending for this batch; the full baseline remains c12167f.
