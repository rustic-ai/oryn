# S package open-row source disposition — September 18

Source inspection and generation only. No build, binding regeneration, formatter,
test, browser/service launch, WPT or five-family qualification was run. Parent
checklist ownership remains with the coordinator. An unchecked historical row is
not evidence of missing implementation.

## Disposition vocabulary

- **Generated:** named production paths and regression sources exist; this is not
  a pass or an exhaustive claim about the whole parent package.
- **Inventory review:** the named broad obligation has not yet been completely
  mapped to current source. No missing algorithm is inferred from this label.
- **Cross-lane integration:** concrete caller work is owned by an active lane;
  this table does not duplicate its shared edits.
- **Execution:** generation consistency, build, tests, WPT and persisted family
  acceptance remain deferred until the global generation gate opens.

## Every open S row reconciled

| Open row / requirement | Current production and regression evidence | Actual remaining disposition |
| --- | --- | --- |
| S-01.f ParentNode/ChildNode, cross-arena adoption, grouping/failure | `node_migration` retains original wrappers and adoption context; variadic/adjacent/setter paths capture `PreparedInsertion` before mutation. Native `oryn-dom/src/insertion.rs` has staged insertion/removal, iterator/range/option/root custody. Its tests include `every_insert_checkpoint_resumes_one_structural_and_journal_commit`, `every_removal_checkpoint_preserves_exact_range_transfer`, migrated cleanup and wrong-owner rejection. | Inventory review: enumerate all variadic self-reference/Document validation/grouped mutation callers against these later native receipts. Old generic “no retained insertion” claims are superseded. This review did not establish a new missing algorithm. |
| S-01.partial-html-updates | `sanitizer_insert::run`, all five Element safe/unsafe insertion pairs and ShadowRoot append/prepend; typed native fragment filtering before `apply_fragment_patches`; parser PI provenance/template-for buffering/source handling. Regression sources cover native grouped mutation, conversion movement, registry, filter, captured work and all five methods × four resource stages. | Generated; parser's `PARALLEL-PARSER-2026-09-18.md` supersedes initial partial-update handoff. Combined authored/generated IDL consistency and execution remain. |
| S-01.streaming-html-updates | `html_stream` true incremental parser, retained-byte accounting and native WritableStream backpressure/error/abort; parser fragment sessions; automation's blocking external script continuation and accepted-batch enqueue ownership. `PARALLEL-HTML-WRITABLE-ORDERING-2026-09-18.md` records frozen callers and regression sources. | Generated; earlier “remaining streaming bridge” wording in sink notes is superseded. Execution remains. No unsupported sibling API is invented from directory breadth. |
| S-01.trusted-policy-reporting parent | Root per-policy native reporting, observers, worker custody and script-fetch integration; existing checked siblings remain source evidence. | Cross-lane root integration/reconciliation; this parent must not be counted as a second independent missing implementation in addition to its children. |
| trusted-preparation-caller-closure | Typed DOMString/fetched input, accepted transformed-script receipt, exact document/generation/stream, capacity retry; host script record, portable/parser and dynamic routes. Loading reports no newly confirmed TT gap; parser callback/terminal routes are in its latest source audit. | Generated bounded routes; root combined caller review and execution remain. |
| trusted-event-transport-closure | Existing Window/Worker native producers, worker batch ACK and parent journal append ordering/custody. | Root owns final journal capacity/identity and retirement reconciliation. No DOM-lane duplicate implementation. |
| reporting-realm-retirement-closure | Callback realm arrays/dictionaries, native observer graph/private ownership and retained failure sources already generated. | Root owns remaining observer operations/retirement source disposition; execution remains. |
| reporting-core-caller-attribution | Core operation inventory is recorded in `G-SOURCE-DISPOSITION-2026-09-18.md` and its JSON; root owns native report producer attribution. | Exact attribution review; absent directory-wide helpers are not automatically Core dependencies. |
| csp-hash-reporting | Parent/standalone preload, Window worker/sharedworker/serviceworker/worklet/blob producers and raw digest/exposure receipts are recorded in `PARALLEL-ROOT-INTEGRATION-2026-09-18.md`. | Root integrated fault/caller review and execution, not a second DOM implementation. |
| S-01.editing-author-boundaries; old inverse reorder/partial/paired rows | `editing_continuation`, `editing_inverse`, `retained_editing` retain owned operations, detach/insert cursors, per-field ownership and whole/paired attempt cursors. Author controls, weak ElementInternals witness, topology/child/attribute changes and source retirement have regression sources; direct Range failure aborts the unperformed suffix. | Generated bounded algorithms; run complete fault and conformance matrices after the gate. Earlier proposals to resume a failed synchronous Range method are superseded. |
| Old pre-await editing quota growth row | `editing_continuation::prepare` preflights typed Attribute/Style/Control/Override/Text/Split growth and newly admitted subtree/parent snapshots before `mutate`. Production detached create/import/clone blocks reach `Context.append` before connection. `oversized_native_growth_rejects_before_publication_and_keeps_prior_inverse` exists; checked `S-01.editing-growth-admission` supersedes the older open paragraph and the ending of `EDITING-INVERSE-REVIEW-2026-09-17.md`. | Generated; no still-missing connected-growth journal was confirmed by this review. Execution remains. |
| S-01.shadow-serialization; html-sink-full-review | Exact sink and declarative stylesheet caller matrices cover native serialization, UTF-16, Document provenance, scoped/global/null registries, parser scheduling, mutation/failure custody and module-map CSS identity. `Config::set_data_attributes` and native/binding policy edge regressions exist. Actual style visibility integration is in `tests/set_html_unsafe_style_visibility.rs`. | Generated named matrix. Older setDataAttributes/style-visibility/registry/native CSS adapter absence notes are superseded; combined consistency and execution remain. |
| S-01 broad binding/conversion/exposure/traversal/XPath/historical Range | Factory/attribute/collection/adoption/selection/parser rows and `S01-BINDING-COVERAGE-MAP-2026-09-16.md` contain substantial later generation. | Inventory review still required for the full frozen manifest and historical crash/RSS attribution. A source module or a boundary journal does not prove exhaustive coverage. Binding regeneration and all execution deferred. |
| S-02.parser-migrated-handles | Parser lane confirms tracked handles, actual-owner attributes/construction/adoption, native insertion/removal receipts, retained mutation/EOF/input and terminal/admission sources. `PARALLEL-PARSER-2026-09-18.md` is the current evidence, replacing the older “in progress” wording. | Generated; combined binding/engine source consistency and pinned qualification remain. |
| S-02 parent CE/shadow/slot | `custom_elements`, native parser registry boundaries, cloned/selectedcontent cursors, adoption hooks and FACE audit caller table. Parser construction and null/global/scoped registry fixture sources exist. | Generated mapped callers; parent whole-package execution remains. Additional full-manifest source attribution is an inventory review, not proof of missing construction. |
| S-03 target/rel/noopener + S-03.c/R-04.d | `submissions`, UTF-16 target/name helpers, native Window.open, parent selection and native opener map; keyword ancestry, duplicate-name ordering, frame replacement/disown/destruction sources. New `_blank` prerender producer/actor/native activation receipts preserve GET query and exclude POST. | Generated native context path, including final retired-route/ACK/focus fixes from this pass. Renderer owns standalone auxiliary root creation; navigation owns standalone frame/resource lifecycle. Preserve these active boundaries. |
| S-03.d/R-04.e navigation supersession/close/outcomes | Native target/navigation receipts and close/failure publication are navigation/root-owned. | Cross-lane current lifecycle source reconciliation; not a form-serialization gap. |
| S-03.e FACE/reset/file body and e.5 | `form_activation::reset`/`retained_form_reset` shared by script/trusted activation; `submissions`, form entry construction and authorized file custody feed persisted family receipts. `FACE-SUBMISSION-CUSTODY-AUDIT-2026-09-17.md` has 25 exact pinned dependency rows and file/reset/entry regression sources. | e.5 explicitly contains execution. Broad all-destination/persisted-receipt review remains distinct from absent native queue/reset/FACE code. |
| S-04 input/editing/focus/validation/defaults | Native keyboard, pointer, touch, composition, controls and semantic actions use shared primitives/receipts. Current G inventory maps 1,516 operation occurrences and 1,199 helper exception occurrences; this does not alone certify all S algorithms. | Historical drag inventory is now mapped by `S04-HISTORICAL-DRAG-SOURCE-ATTRIBUTION-2026-09-18.md`: 180 rows/72 paths, three confirmed source gaps fixed, three explicit pinned/profile conflicts retained. Platform IME/OS clipboard and WPT execution are qualification. |
| S-04 drag/DataTransfer/cross-document | `native_drag`, `native_drag_cross_frame`, clipboard native serialization and drag image snapshots; shared/paired move histories and Actions Release/Cancel receipts. Cross-frame ownership/teardown and actual production input regression sources are checked in the central checklist. | Old cross-frame/drag-image absence notes in September 16 notes are superseded by later generation. Historical matrix attribution is complete in the S04 drag source map; constructor defaults, interactive source admission and missing draggable IDL/reflection were corrected by that review. Three pinned/profile conflicts and all execution remain. |
| S-05 all OIL targets/actions/extraction/recovery | `S05-OIL-CALLER-RECOVERY-SOURCE-AUDIT-2026-09-17.md` covers text/context/worker/CLI/MCP/full-contract and legacy family routes, typed partial outcomes and no prefix replay. Native extraction/target/error/alias/revision sources exist. Scoped native `Dismiss` now replaces the global-button behavior called out in that older audit. | Generated mapped caller inventory. Whole frozen recipe/attempt recovery, nested replacement and persisted outcome qualification remain; no broad absence claim follows from unchecked S-05. |
| S-05 network-idle/observation freshness | Native document/selected-frame resource scopes, broker transfer and parser/dynamic loader client selection are reflected in later S-05.j.1–j.4 and loading/root audits. | Root/loading full producer reconciliation plus execution; portable frame lifecycle remains with navigation. |

## Changes made during this reconciliation

The concrete implementation gaps confirmed in this pass were in the `_blank`
activation completion boundary, and were fixed rather than relabeled:

1. Retired worker routes now finish exact Activate/Publish/Acknowledge receipts
   through source-bound retained capability; pending source candidate/ledger
   custody prevents premature route drain.
2. Final actor acknowledgment releases route retention while bounded replay
   state survives. Destroyed destinations preserve authenticated terminal
   tombstones so generic protocol errors cannot cause endless ACK retries.
3. Native queue storage/capacity is preflighted before either producer claims
   a candidate. No ordinary-navigation fallback replays a claimed candidate.
4. Hidden-page visibility and active-page focus publish together; ACK replay
   does not steal later focus. Regression sources cover all these boundaries.

See `R04-TARGET-NAME-CALLER-MATRIX-2026-09-18.md` for exact producer/RPC evidence.
No additional missing S algorithm was established solely by the historical
unchecked wording. The remaining broad inventories above are genuine review
work and must not be silently converted to passing acceptance.


## Follow-up complete S01 inventories

`S01-VARIADIC-MUTATION-CALLER-MATRIX-2026-09-18.md` now closes the exact S-01.f
six-method source inventory, including pinned shared Document validation,
all receiver families, grouping, conversion/staging, migration and committed
native failure tails. It supersedes that table row's inventory-review status.
The before() saved-sibling and discarded committed-tail defects found during
that review have generated fixes and regression sources.

Authority lane delivered `S01-PINNED-SOURCE-ATTRIBUTION-2026-09-18.md` and JSON:
1,221 original Core rows / 1,193 sources plus 83 literal Range/Selection callers,
with no source-hash mismatch. It supersedes the broad S01 inventory-only row.
The ignored historical Range diagnostic now validates pin/helper/result
completion; it remains ignored and unexecuted. All runtime acceptance stays open.

## Final S source-gate reconciliation

This appendix supersedes the historical inventory/cross-lane wording above.
It records source completion, never passing conformance. No tests, generators,
builds, services or qualification were executed for this reconciliation.

| Remaining checklist source row | Final source disposition and evidence |
| --- | --- |
| S-01.f and full S01 binding/Range attribution | Source inventory complete in the six-method variadic matrix and S01 pinned attribution map (1,221 rows / 1,193 sources). Confirmed saved-sibling and committed-native-tail defects have generated fixes and failure sources. Historical Range diagnostic execution remains deferred. |
| S-01.partial-html-updates, streaming-html-updates | Source generated across `sanitizer_insert`, native filtered fragment/PI patching, `html_stream` and retained external-blocking script continuation. Parser, loading and HTML Writable ordering reports supersede their initial handoff gaps. Retained parser bytes, terminal cleanup, suspension and accepted-batch publication have source coverage. |
| S-01.trusted-preparation-caller-closure | Source generated: typed input and `script_preparation_receipts` bind raw/accepted source to document/generation/stream/authority, preflight before author policy, prevent recursive/retry callback replay, clear at start/retirement, and flow through parser/root/frame/XML/dynamic/portable preparation. Loading's acceptance/preparation follow-through replaces the old raw-script-record caller finding. |
| S-01.trusted-event-transport-closure | Source closed. `context_event_journal` bounds records/bytes/recipient obligations/cursors, checks IDs, admits batches before publication, and validates native Storage/CSP watermark ACKs. Compatibility PollEvents cannot discharge author custody. Cached documents retain exact obligations; successor/retirement cannot redirect them. Automation's final caller table and `compatibility_event_delivery_integration_tests` cover competition, failure and retirement. |
| S-01.reporting-realm-retirement-closure | Source closed for the required observer contract. `reporting.rs::{construct,owner,call,poll}` retain registration-global/object identity, use callback creation realm, preserve queue through argument failure and suspension, retain pending delivery after disconnect, and trace callbacks through private edges. `native_platform::retire_native_platform` resets reporting state; poll/producer guards reject retired delivery. Existing sources cover foreign newTarget/borrowed methods, callback realm, conversion, filtering/takeRecords, suspension, disconnect, GC and actual retirement. No additional missing observer branch was found. |
| S-01.reporting-core-caller-attribution | Required saved Core caller attribution is complete using the immutable G operation/helper/dynamic-source maps. `generate_test_report` has no occurrence in that required operation inventory. The pinned full-tree audit found its direct calls only in Extended reporting/infrastructure fixtures and driver definitions; the mapped dynamic helper contexts do not introduce it. Therefore its absence is not a confirmed Core implementation gap and no synthetic producer is added. This is a bounded frozen-inventory conclusion, not a claim about arbitrary dynamic sources or the whole Reporting directory. |
| S-01.csp-hash-reporting | Source generated: `csp::{hash_reports,hash_reports_from_evidence}` selects per-policy digest/directive; native script fetch captures original response bytes and exposure before decoding, with parent/standalone preload publication receipts. Window-created worker/sharedworker/serviceworker/worklet/blob routes and worker fallback versus worklet element/script directives are mapped in the root audit. Full real-collector capacity/retry, opaque/raw-byte, identical-policy reuse and foreign-consumer regression sources exist. Hash reports use endpoint transport, not ReportingObserver. Pinned/current-spec SRI sequencing is an explicit qualification comparison, not an unidentified source branch. |
| S-01.trusted-policy-reporting parent | All listed source children above are generated/reconciled. Retain one combined build/fault/WPT execution gate; do not count this parent as a duplicate missing implementation. |
| S-01 editing/author-boundary/growth and shadow serialization/full sink review | Source generated in editing continuation/inverse/growth admission and HTML sink/declarative stylesheet caller matrices. Old inverse-growth, setDataAttributes, registry and CSS identity absence notes are superseded. Combined interruption/GC/visibility/conformance execution remains. |
| S-02 parser/CE/shadow/slot | Source generated and mapped in parser and S01 inventories: migrated native handles, actual-owner mutation, construction/registry/clone/adoption and retained parser terminal/input custody. Binding consistency/build and pinned execution remain. |
| S-03 form construction/FACE/reset/file-body | Source generated in submissions/form activation/entry construction/native upload and the 25-row FACE custody map. The explicitly open e.5 combines independent fixture execution and persisted forms-family receipts; generated queue/reset/file semantics are not missing merely because that execution row is unchecked. |
| S-03 target/name/noopener/native Window.open | Source generated in R04 target/name matrix, including lossless DOMString, native selection/authority/opener, final GET versus POST and exact retained blank-prerender Activate/Publish/Acknowledge custody. Destroyed authenticated replay, preclaim capacity and idempotent focus publication are included. Root's newly discovered `supports('speculationrules')` exposure switch is separately assigned to root, not an unimplemented form-selection algorithm. |
| S-03 navigation supersession/close; standalone frame integration | Navigation/rendering own the final lifecycle source disposition. These are shared R04/R05 dependencies, not additional missing form serialization. Preserve their exact final source records and deferred close/replacement/rollback execution rather than duplicate implementation. |
| S-04 input/editing/drag | Required native operation and historical drag inventories are mapped. S04 historical attribution covers 180 rows / 72 paths: 69 paths mapped to generated source, three explicit obsolete/profile conflicts. The three confirmed code gaps from that audit were fixed. Coordinator must retain and resolve the conflicting assertions/profile provenance; OS IME/clipboard and WPT execution remain. |
| S-05 target/actions/extraction/error/recovery | Source caller inventory generated in S05 OIL audit plus later scoped Dismiss fix: local/context/worker/frame/CLI/MCP/full-contract routes preserve identity, successful-prefix receipts, typed causes and uncertain-action cache invalidation. Whole frozen-attempt and persisted recipe recovery are execution qualification, not a newly identified missing action branch. |
| S-05 network-idle/observation freshness | Document/frame scopes, broker transfer, preparation clients, cancellation custody and selected-frame wait sources exist; later loading/automation/portable-frame completion records supersede the old blanket producer gap. Final lifecycle dependencies belong to their R/L source disposition. Native activity, not author trace records, controls quiet intervals. |

No additional concrete missing S algorithm was established by this final read.
The explicit outstanding non-execution disposition is the three historical drag
contract/profile conflicts, plus any exact R/L integration still reported by its
owner. All combined compilation, generated-binding consistency, fault runs,
Core denominator/results and five-family persisted evidence remain mandatory.

The shared network producer conclusion uses
`L-OPEN-ROW-SOURCE-DISPOSITION-2026-09-18.md` L-04.a and its final combined
producer table: cancellation-phase pressure/RSS are execution gates, not an
unidentified body owner. The portable frame extraction report records no
remaining confirmed extraction gap; later frame-response/policy accounting
and lifecycle visibility reports must be read with it rather than the older
R table's initial visibility handoff.
