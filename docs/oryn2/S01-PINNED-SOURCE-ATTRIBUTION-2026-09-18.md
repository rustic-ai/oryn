# S01 pinned source and historical Range attribution

Source review and regression-source generation only. No build, test, WPT run,
formatter, binding generation or service launch occurred. Root owns the checklist.
The historical artifacts retain their original results and binary identities.

## Frozen input and limits of the index

The existing `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl` supplies
all 16,737 Core rows. The new source-attribution JSON preserves the original
row IDs, URLs, test types and source hashes. It selects exact Core rows in the
named DOM-interface directories, plus literal Range/Selection creation/content
callers elsewhere in Core. Every selected source matches its saved SHA-256 in
checkout `719d5e38fdd0903a18ed9007aba816c98cc491e0`; no mismatch was found.
The Core manifest remains
`566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101`.

[Machine-readable source index](S01-PINNED-SOURCE-ATTRIBUTION-2026-09-18.json)
contains 1,221 existing Core rows and 1,193 distinct sources. These are attribution
counts, **not** an alternative denominator or a count of newly passing tests.
Literal-call detection is static evidence and does not claim complete dynamic
import/computed-call reachability. `html/dom` deliberately retains cross-package
entries so render-blocking, forms/reflection, source provenance and partial-update
dependencies are visible rather than silently counted as new S01 algorithms.

## Current source disposition by indexed requirement

| Indexed group | Core rows | Production boundary and regression source evidence | Remaining source / execution disposition |
|---|---:|---|---|
| Nodes, namespaces, mutation/adoption | 354 | `browser.webidl` native metadata; `node_identity`, `node_migration`, native `insertion.rs`/`clone_tree.rs`, `node_mutator_bindings`; `clone_preserves_template_and_shadow_depth_without_recursive_copy`, `resumable_clone_keeps_allocated_identity_and_frozen_source_after_each_fault`, native insertion checkpoints, document-factory/PI/namespace conversion regressions in S01 coverage map | DOM lane owns final S01.f six variadic/grouped mutation caller audit. Existing native receipts, namespace factories, tracked adoption, wrapper identity and inverse cleanup are generated; no additional missing algorithm inferred from historical wording. |
| Range / StaticRange / OpaqueRange | 71 | `range_boundaries`, `range_insertion`, `range_contents_observable`, `static_range_custody`, `opaque_ranges`, `range_geometry`; native live-range mutation journal and opaque control offsets. Boundary conversion/foreign-owner, iterative UTF-16 partial contents, CE clone completion and failure suffix, static endpoint migration, opaque disconnect/offset and geometry tests exist | Current algorithms cover pinned setters/comparison, mutation adjustment, clone/extract/delete/surround, insertion, shadow boundaries, construction and geometry families. Historical crash diagnostic strengthening below was a confirmed regression-source gap; runtime/signed resource attribution remains execution. |
| Selection | 187 | `selection_bindings`, `selection_deletion`, `selection_composed`, `selection_movement`, `selection_text`, trusted input/editing helpers. `selection_extend_replaces_ranges_and_preserves_state_on_rejected_targets`, `native_selection_types_idl_conversions_and_reentrant_state`, foreign Range/root retention and changed-selection event regressions | Historical extend identity bug has generated replacement-range semantics; old failed outputs are not current passes. Pointer/drag geometry and input cases map to native input/rendering lanes, including navigation's historical drag attribution. |
| Traversal | 18 | `tree_walkers`, `node_iterators`, native iterator pre-removal/migration records. `callback_interface_errors_and_conversion_reentrancy_use_filter_realm`, `root_adoption_inside_filter_keeps_iterator_filter_and_original_nodes`, `live_filtering_removal_direction_and_reentry_preserve_reference` | Every indexed traversal family has current algorithm/callback-realm/cursor-owner source; prior generic “traversal missing” language is superseded. Execution remains. |
| Collections | 11 | `node_lists` plus named/attributes/all/migration modules and native indexed/named property interceptors. `html_collections_are_live_named_branded_and_literal`, `document_html_collections_are_live_same_object_and_namespace_filtered`, collection snapshot/iterator poisoning regressions | Live indexed/named/enumeration/deletion/prototype and conversion-migration paths exist. Source grouping is not proof that every descriptor assertion passes. |
| DOMTokenList | 5 | `token_lists` real attribute association; `token_lists_are_live_indexed_branded_and_preserve_literal_values`, `token_list_updates_validate_atomically_and_respect_forced_noops`, `blocking_lists_are_distinct_live_native_associations` | Atomic validation, supported token lookup, stringifier, iteration and live attribute association generated; execution remains. |
| DOM parsing / serialization | 71 | DOMParsing binding audit and `parsing_binding_tests`; native HTML/XML fragment parser/serializer, retained source provenance, safe/unsafe sinks and Trusted Types receipts | S01 sink/parser source audits supersede old parser TODOs. Root reporting and DOM/parser partial/streaming integration remain attributed to their current lanes; no duplicate implementation created. |
| XPath | 34 | `xpath` native context/resolver/result records, native namespace and iterator invalidation. `xpath_context_namespace_and_result_objects_follow_callback_adoption`, `cross_realm_evaluators_use_context_document_matching_and_native_wrapper_owner`, snapshot reuse and retired-document query regressions | Resolver getter/call/result coercion realms, exact context migration, retained results and immutable expressions are generated. Execution remains. |
| DOM IDL / historical singleton entries | 9 | `crates/oryn-webidl/idl/browser.webidl`, binding generator metadata and native descriptor/brand/argument dispatch; document factories and initial owner wrappers | Authored source and native consumers are mapped. Regeneration consistency and actual exposed interface/property tables must be checked after global generation. |
| HTML DOM cross-package | 378 | Element/document native operations and reflection; metadata/source provenance, render-blocking and parser resource drivers, partial-update sinks; documented in S01 binding/sink matrices and parser/loading audits | Includes 63 render-blocking and 30 partial-update rows; these are existing L/parser/DOM dependencies, not 93 newly missing independent S01 features. Qualification remains on original rows. |
| Static Range/Selection callers outside primary directories | 83 | Shared native Range/Selection routes; indexed exact sources include custom-element Range reaction/constructor callers, editing, shadow/layout/hit-testing and framework-facing document code | These rows expose transitive dependencies on the same algorithms. They are not extra Range methods. Input/layout/CE-specific behavior stays with its owner lane; original Core rows are preserved. |

The broad S01 parent is thus split into generated native algorithms, the active
DOM S01.f grouped-mutation source task, shared reporting/parser/input integration,
and actual deferred validation. This audit found no additional confirmed missing
production Range/traversal/XPath algorithm. Name presence alone is not treated as
proof: the linked earlier coverage map records native observable boundaries,
conversion/reentry and retained failure regressions for these groups.

## Historical Range observations and current attribution

| Original exact Core row | Preserved historical observation | Generated source relevant to it | What remains |
|---|---|---|---|
| `dom/ranges/Range-insertNode.html` | Signed protocol-9 worker stack overflow before valid completion; `WPT-RANGE-INSERTNODE-CRASH-2026-09-12.md` preserves raw artifact/hash. Recursive cloning and initial frame association were leads, not established function-level attribution. | Native clone traversal is iterative and retained; insertion carries original Range/Node identities across observable children changes; initial frame association/reuse has regression sources. The ignored original-fixture diagnostic is now strict as described below. | Run faithful current signed reproduction and collect debuggable crash/resource evidence if it fails. Do not relabel the historical crash “fixed” based on source inspection. |
| `dom/ranges/Range-stringifier.html` | Historical control 5/5; not a new passing-file gain | `range_text`, `selection_text`, UTF-16 join path; ancestor/text-only/CDATAs and split-surrogate regression sources | Current full candidate qualification; no historical result rewrite. |
| `selection/extend-00.html`, `extend-20.html` | Historical execution failures after stack-crash removal; no complete assertion result | Native boundary-based stringification and selection `replace` create a new Range while retaining original wrappers; rejection preserves anchor/focus state; numeric conversion and root checks generated | Current full upstream executions and resources; timeout/cost cannot be declared resolved from code alone. |
| `selection/extend-40.html` | Historical completed 26/176 assertions, with range identity/exception/anchor discrepancies | Same replacement-Range, conversion, native owner/root and state-preservation regressions | Current assertion execution; no extrapolated pass count. |

Saved coverage rows for these five sources are all `not_run`, with the source
hashes recorded in the new index. This metadata is input classification, not the
separate historical diagnostic outcome. The two must not be conflated.

## Confirmed regression-source correction

`pinned_range_insert_node_fixture_does_not_overflow_native_stack` in
`rust_executable.rs` previously printed diagnostics/completion without asserting
anything. It could succeed when `common.js` failed and `testDiv` was undefined,
which the historical investigation explicitly identified as invalid evidence.

The ignored test now:

- Hash-checks the original Range fixture, upstream harness, common helper and
  child-frame source against the pinned checkout before constructing a page.
- Runs the original full source without a `subtest` narrowing or HTML rewrite;
  resource aliases resolve the original harness/common/iframe requests.
- Polls real context/parser tasks with a bounded 120-second diagnostic deadline.
- Requires the actual setup helpers, range/node tables and both child frame
  helpers, exactly one complete receipt, successful harness, a nonempty original
  result list and successful assertions. Zero results and printed errors cannot
  satisfy the test.

It remains ignored and requires `ORYN_WPT_CHECKOUT`; no execution occurred.
It is a component regression source, not a substitute for signed worker, sandbox,
parent teardown, resource/RSS or complete official WPT qualification. The older
clone diagnostic is explicitly a rewritten, `subtest=61` focused component
probe and cannot attest to the whole original Range fixture.
