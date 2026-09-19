# S-01.f complete variadic mutation caller source review

Source generation and inspection only; no builds, formatters, binding generation,
tests or conformance executed. This closes the named caller inventory, not runtime
acceptance. Frozen WPT checkout is `719d5e38fdd0903a18ed9007aba816c98cc491e0`;
Core manifest hash remains `566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101`.

## Required caller set

All six variadic methods enter `node_mutator_bindings::operation` from native
mixin dispatch. Parent receivers are Document, DocumentFragment (including
ShadowRoot through inheritance) and Element; child receivers are Element,
CharacterData (Text, Comment, CDATASection and ProcessingInstruction) and
DocumentType. Exposure/descriptors/length/unscopables are already covered by
`mixin_exposure_variadic_conversion_and_family_dom_updates`. `remove` and
`moveBefore` are separate nonvariadic algorithms and are not silently substituted
for these six methods.

| Method | Stable insertion position after conversion and staging | Destination operation |
| --- | --- | --- |
| append | Captured original parent, null reference | `insert_dom_node` |
| prepend | Captured original parent's current first light child | `insert_dom_node` |
| replaceChildren | Entire incoming sequence validated as final children | `replace_dom_children` |
| before | Saved viable previous sibling's **current** next sibling, or captured parent's first child when no saved previous exists | `insert_dom_node` |
| after | Saved viable next sibling, re-resolved through its original wrapper | `insert_dom_node` |
| replaceWith | Saved viable next sibling; replace original receiver only while still a child, otherwise insert | `replace_dom_child` or `insert_dom_node` |

## Algorithm obligations reconciled

| Obligation | Production evidence and regression sources |
| --- | --- |
| Convert all arguments before method effects | `arguments` performs ordered Node-or-DOMString conversion and retains original wrappers/UTF-16. Throwing conversion retains only author side effects. Existing two-arena conversion/adoption and new complete matrix sources cover all six methods. |
| Zero, one and multiple arguments | `stage` directly retains one Node, creates one Text for one string, otherwise builds a leased real DocumentFragment. Empty fragment insertion emits no target record; empty replacement removes only old children. |
| Self references, repeated nodes and sibling exclusion | `local_nodes` computes viable siblings before staging; repeated nodes move to their final position in the fragment. New `complete_variadic_self_reference_document_validation_and_grouping_matrix` covers all three child methods × Element/Text/Comment/CDATA/PI × self/nonself, duplicate arguments and Element/DocumentFragment/ShadowRoot parent receivers. Existing grouped-fragment test covers source/target records and receiver already staged away. |
| Staging callback reentrancy | `node_migration::insertion_argument` handles source script children-changed preparation, retaining original parent/reference/source wrappers. New `before_reads_saved_previous_siblings_current_successor_after_script_staging` covers detached, moved-last, moved-with-next and foreign-document saved siblings, with one callback and correct failure/staging custody. |
| Same-/cross-arena validity before single-node adoption | `validate_pre_insertion`, `validate_foreign_insertion` and `validate_foreign_replacement` validate kind, host-inclusive cycles, reference ownership, final Document cardinality and doctype order. Multiargument staging removals occur first, as required. Foreign argument/receiver adoption is already exercised by `argument_conversion_adopts_receiver_before_all_parent_and_child_mutations`. |
| Complete Document validation classes | The pinned shared helper covers ancestor/self, Document input, Text, doctype in non-Document, multi-element fragment, duplicate root/doctype and doctype order. New matrix adds CDATA/Attr, invalid single-input preservation, intact invalid fragment, valid replacement/comments/PI and all DocumentType child methods. Existing fragment staging regression verifies final Document rejection retains required source removals. |
| Successful mutation grouping | `take_dom_insertion_nodes` publishes one fragment removal row; target insert/replace wrappers suppress only target primitive observer projections and publish one ChildrenReplaced row with exact added/removed arrays and siblings. Primitive journal events remain available to ranges, CE and resource consumers. Staging removals from the old target remain distinct. |
| Precommit resource admission | `check_insertion_capacity` and captured `PreparedInsertion` precede destination mutation. Existing all-six capacity and partial-capture failure matrices preserve required staging and destination state. |
| Postcommit failure custody | **Fixed in this pass:** synchronous TreeInsertion/TreeRemoval wrappers retain only committed native tails in a pre-reserved bounded runtime queue (256 entries). A detached-but-not-inserted move completes its removal bookkeeping without performing the unexecuted insertion. Completed insertion/removal finishes ownership/range/style/control/journal stages exactly once. No variadic suffix is queued. |
| Native tail lifecycle | Queue drains at native lifecycle checkpoint, before subsequent structural operations, before mutation-root release and before cross-arena migration planning. Parsed-arena replacement clears old custody without releasing reused IDs. Retry performs no author callbacks. `synchronous_variadic_failure_finishes_only_committed_primitives_once` faults across insert/replace-all/replace-one checkpoints and asserts unchanged structural prefix plus one observer addition per committed child; `detached_move_failure_never_finishes_unperformed_destination_insertion` covers detached-only outcomes. |
| Resource callbacks after an error | Existing `PreparedInsertion::finish_mutation` retains connected captured roots while discarding detached suffixes and preserving the original error. It does not replay the failed public method. The native tail queue complements this resource custody rather than replacing it. |

## Confirmed corrections

1. `before()` formerly searched the saved previous sibling in the original
   parent's children after staging. A source script could move it elsewhere,
   so the correct current successor was lost or a detached foreign node was
   incorrectly rejected. The caller now reads the original node's current
   nextSibling and validates any resulting reference against the destination.
2. Synchronous native mutation wrappers formerly always aborted their cursor
   after an infrastructure error, even after structural commit. The new queue
   retains the committed primitive's exact phase and native root custody;
   error return does not authorize the unperformed method suffix. Range/iterator removal targets are captured before detachment but applied after structural commit, so a precommit failure cannot leave live boundaries changed for a node that never moved. The fault matrix checks both cases.

## Pinned direct caller evidence

| Source | SHA-256 |
| --- | --- |
| `dom/nodes/ParentNode-append.html` | `4669ba130ee70624b3b346ae50f1127831c8b650d7fa9a4a3153f1028db70c41` |
| `dom/nodes/ParentNode-prepend.html` | `d87d1bf60021fe83d3073b3b8a564551cacd9c6aca1d8ec1e4f9275589d7c00b` |
| `dom/nodes/ParentNode-replaceChildren.html` | `7f773653fd6b3e2cdbf2522766f7aefc64de585c43c1d43cecf42811d6e008dc` |
| `dom/nodes/ChildNode-before.html` | `ee61c00b9503ca41b34b22dc88759893fa60cf2c7f4fb14ec45f3b547501a00e` |
| `dom/nodes/ChildNode-after.html` | `b72ad3984ad5ea96f5277cdca095225a0dba2a7b57ba0e6b5bf4a69d3ff8c38e` |
| `dom/nodes/ChildNode-replaceWith.html` | `81b10f695d7f3e46982b5916fdc5d3a58dd505b2b5751c741f704e57244d6f09` |
| `dom/nodes/pre-insertion-validation-hierarchy.js` | `bfec5a1f731250895220a0d175e951553a8e77cd7ae0b2e953c53c1c3d44de28` |

Generated sources still require the global build/regeneration/test and complete
WPT/family execution phases. No passing result is claimed.
