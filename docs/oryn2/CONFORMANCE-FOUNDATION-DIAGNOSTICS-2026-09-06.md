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
