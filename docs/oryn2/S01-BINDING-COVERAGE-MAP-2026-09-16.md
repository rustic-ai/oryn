# S-01 requirement and caller coverage map

Source generation only. No builds, tests, binding generators or qualification. This is an obligation map with bounded reviewed contributions, not a declaration that interface-name presence proves implementation.

## Whole-plan obligation map

| Obligation | Native production boundary | Reviewed contribution / open coverage | Five-family impact |
| --- | --- | --- | --- |
| Exposure, brand, descriptors, conversions and realm allocation | `browser.webidl` → generated interface/member/argument metadata → native constructors, getters/setters and `generated_operation` | FileReaderSync subtype exposure, FileList clone ownership, Node identity/root and DOM Parsing binding rows reviewed; full selected interface/member matrix still open | All frontend frameworks |
| Namespace/tree mutation and adoption | `call_operation`, native DOM arena identity, ownership journal and wrapper registry | Node foreign comparisons and XMLSerializer owner conversion reviewed; ParentNode/ChildNode binding corrections below. Cross-arena **mutation/adoption** remains distinct from read-only comparison | All dynamic workflows |
| Collections and serialization | native `node_lists`, `file_lists`, generated indexed interceptors, structured clone and storage envelope | FileList local/wire/storage graph and destination wrapper sources reviewed. Whole live collection descriptor/mutation matrix not implied | Forms/uploads, shopping and SaaS |
| Traversal/XPath/parsing | native walker/filter state, `xpath`, inert DOMParser and XMLSerializer callers | DOM Parsing argument/realm review is bounded; whole traversal callback reentry/filter/namespace matrix remains open | Research/extraction and OIL targeting |
| Range/Selection and historical failure attribution | native range journal, opaque range store, V8 wrapper roots, edit primitives | Existing checked S-01.a/b rows identify bounded source generation; historical crash/RSS reproduction and complete conversion matrix remain qualification obligations | Rich editors, forms and SaaS |
| Adoption/mutation lifetimes | arena/native leases, exact wrapper identities, mutation records, CE hooks | Same-arena conversion and owner guards are reviewed in the bounded rows; distinct-arena migration requires preserving wrappers, observers, ranges and identity, not subtree cloning | Framed SaaS/booking and extraction |
| Generated consistency | repository WebIDL generator + native dispatch and contract artifact provenance | Source metadata updated, generation deliberately deferred; no generated artifact or historical signed evidence changed | All families and final candidate |

## Confirmed ParentNode/ChildNode binding gap and generated correction

Pinned WPT `interfaces/dom.idl` assigns ParentNode to Document/DocumentFragment/Element and ChildNode to Element/CharacterData/DocumentType. It requires zero-arity variadic `(Node or DOMString)` methods with CEReactions and Unscopable metadata. Previous local declarations placed the methods on Node and made append/before/after require an argument.

Source changes:

- Relocated native IDL declarations to the actual mixin targets, with variadic union, arity, Unscopable and CEReactions metadata. `moveBefore` also moves to ParentNode targets.
- `node_mutator_bindings::applies` routes those interface-specific declarations to the existing native mutation algorithms after receiver-brand validation.
- `arguments` converts every value first, roots actual native Node wrappers, uses ordinary DOMString conversion for objects/non-Nodes, preserves UTF-16 and original exceptions, then resolves exact node ownership. It no longer treats arbitrary objects as invalid Nodes or uses lossy nonthrowing stringification.
- Variadic before/after now compute viable references from the post-conversion tree and support strings/multiple nodes, detached conversion, and repeated-node final positions. Existing insertion validation and dynamic script discovery remain shared.
- The six convenience mutators now stage multiple arguments in a real fragment. Native removals naturally implement repeated-node final positions; destination validation follows staging, preserving specified failure side effects.
- `mixin_exposure_variadic_conversion_and_family_dom_updates` regression source covers target descriptors, zero arity, Unscopable, absence on Node, empty operations, ordered string conversion, atomic conversion failure, variadic adjacency, callback removal, detached conversion, lone-surrogate text and Symbol rejection.

Selected Core inventory entries and original SHA-256:

| Source | SHA-256 |
| --- | --- |
| `dom/nodes/ParentNode-append.html` | `4669ba130ee70624b3b346ae50f1127831c8b650d7fa9a4a3153f1028db70c41` |
| `dom/nodes/ParentNode-prepend.html` | `d87d1bf60021fe83d3073b3b8a564551cacd9c6aca1d8ec1e4f9275589d7c00b` |
| `dom/nodes/ChildNode-before.html` | `ee61c00b9503ca41b34b22dc88759893fa60cf2c7f4fb14ec45f3b547501a00e` |
| `dom/nodes/ChildNode-after.html` | `b72ad3984ad5ea96f5277cdca095225a0dba2a7b57ba0e6b5bf4a69d3ff8c38e` |

## Exact residuals; do not mark whole group complete

The former absent-adoption and fragment-staging items are superseded by the source integration below. Whole S-01 remains open: the entire selected interface/member inventory, callback reentrancy through all graph consumers, regenerated metadata consistency and runtime qualification are not complete.

## Follow-through: fragment staging and observer grouping

The previously listed fragment-staging residual is now source-integrated for the six Parent/Child convenience mutators. `node_mutator_bindings::operation` performs WebIDL conversion, computes viable siblings, stages multiple values into a real DocumentFragment using normal native removals, then validates the destination and inserts/replaces. It replaces all superseded per-method flattening bodies.

`DomRuntime::{insert_dom_node,replace_dom_children,replace_dom_child}` and private `take_dom_insertion_nodes` retain individual structural journal records while suppressing their duplicate observer delivery and publishing `ChildrenReplaced` groups. Fragment-source removals are grouped; incoming single-node/adoption removals precede target replacement rows. Empty fragments emit no spurious insertion row. Dynamic script discovery consumes the actual inserted node list.

Regression source `staged_fragments_group_observers_and_preserve_failure_side_effects` checks fragment-source/target record groups, empty calls, staging removals before replace-all, duplicate/self-reference order, and source removals retained when final Document hierarchy validation fails. No tests ran. Cross-arena adoption/import/mutation migration remains open with its exact source ownership and required bridge contract in `CROSS-ARENA-NODE-MIGRATION-DESIGN-2026-09-16.md`; remaining S-01 coverage and runtime qualification are unchanged.


## Cross-arena bridge source integration (unexecuted)

The native `prepare_subtree_migration` / `commit_subtree_migration` transaction is now called by `node_migration::adopt`. Original Node storage and wrapper objects move; only `Document.importNode` uses `import_node_from` to create fresh identities.

- [x] Source generated: native wrapper registry lease transfer and embedded owner remap; original prototype/expando/private fields retained, obsolete weak finalizers cancelled.
- [x] Source generated: explicit adoption and implicit appendChild/insertBefore/replaceChild, six convenience mutators, Element/NamedNodeMap attribute attachment, insertAdjacentElement, Range.insertNode and surroundContents production callers.
- [x] Source generated: source removal MutationRecords frozen before native old slots retire, registrations/transients follow the graph, observe-after-roundtrip removes obsolete remote registration, original callback values retained.
- [x] Source generated: listeners/handlers, associated wrappers, custom states, canvas state and taint, decoded images and scroll state move. Custom-element adopted reactions are preallocated before commit and published with original callback/constructor values.
- [x] Source generated: live collections, mixed snapshot lists, live/static ranges and native NodeIterator capture/publication APIs are integrated. Static/list/iterator native root custody settles before fallible V8 publication. Their independent lane source reviews do not constitute execution.
- [x] Source generated: retained staged publication owns native pins, all captured objects and source/destination states through errors. Native entry points and CE/observer checkpoints resume pending publication before observing the graph. An injected publication-failure regression source exercises retention and recovery.
- [x] Source generated: TreeWalker derives each operation's root/current owner from original traced Node objects, including current nodes in a different arena from the root.
- [x] Source generated: template host/content mutual traced links, template content getter and host-including cycle checks across arena boundaries; ARIA ordered traced references and native projection refresh restore mixed relationships after reverse adoption.
- [x] Source generated: stylesheet policy/reset helper integrated after wrapper transfer, including same-arena document adoption. Source image/script/style/frame jobs are invalidated; native frame lifetime checkpoint retires disconnected old frame environments before author reactions.

`node_migration::tests::cross_arena_adoption_preserves_objects_and_consumers` contains source assertions for actual independent arenas: same object/prototype/expando; Attr and associated wrappers; live/static collection identity; observer removal/following/repeated moves; event listeners/handlers; TreeWalker; import versus adoption; template cycle/identity; ARIA order; implicit Attr/adjacent/range insertion; detached no-op and validation-before-detach; and retained publication fault recovery.

### Remaining source audit / generation cells

- [x] Source generated: TreeWalker retains original root/start/candidate wrappers throughout every operation, resolving native owner and node afresh for each edge. No raw native cursor crosses acceptNode getter, callback, or result conversion. Regression source covers all seven methods, split-arena SKIP, repeated-migration REJECT, exceptional callbacks and reentry during getter/conversion.
- [ ] Complete reentrant migration review for in-flight event dispatch and all remaining synchronous graph algorithms; TreeWalker source delivery alone does not close this wider requirement.
- [x] Source generated: externally adopted template-content innerHTML getter/setter use the original fragment and explicit template parsing context; iterative clone/import completion includes external content and nested/clonable-shadow paths.
- [x] Source generated: outerHTML/innerHTML and XMLSerializer use captured arena-qualified native SerializationResolver; HTML follows external template edges, XML retains light-child semantics. XHR.send(Document) resolves the actual argument owner and scalarizes serialized UTF-16 for network UTF-8. Native serializer regression sources cover nested edges, namespaces, lossless DOM strings and no mutation.
- [x] Source integrated: native NodeIterator capture and stage-zero ownership settlement use the original native iterator IDs/map; parallel lane owns full live traversal/filter and mixed-reference source review.
- [x] XPath result snapshots retain actual Node/Attr wrappers; namespace records now retain only URI/prefix metadata and resolve the original traced owner wrapper across arenas. Context identity is re-resolved after resolver/type conversion, including adoption. Iterator cursor publication follows successful anchor retrieval. Broader native/V8 node-keyed-state audit remains open.
- [ ] Review complete source against pinned selected obligations, including teardown while publication is retained and all failure branches; execute only after the whole-plan generation gate.

No build, test, formatter, generator, service, conformance run or qualification was executed for this delivery.


## DOM and platform EventTarget signal / reentrant dispatch source review

Pinned revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`, `dom/events/AddEventListenerOptions-signal.any.js` explicitly exercises standalone EventTarget: abort removes listeners before future dispatch, once/capture/explicit remove coexist, aborted signals do not register, abort during dispatch skips later callbacks, nested abort terminates recursion, and null signal throws even with a null listener. `AddEventListenerOptions-passive.any.js` requires option getters on add with null callback and no passive getter on remove. These are implementation obligations, not passing run results.

- [x] Source generated: `node_event_listeners::{options,aborted,abort}` shared by DOM and platform registration; exact canonical foreign AbortSignal owner/ID replaces platform same-realm numeric lookup. Conversion finishes before null/aborted return, first duplicate options remain authoritative, remove reads capture only.
- [x] Source generated: signal abort removes exact-owner DOM/platform listeners across native binding states before public abort algorithms; snapshot invocation checks current registration and exact signal state. XHR upload listener detection uses the same foreign signal predicate.
- [x] Source generated: DOM dispatch uses captured path objects and resolves current arena after callbacks; once removal and handlers use current owner. Click activation/previous radio selection use migration-following native TrackedNodeLease, with original object identity and per-event owner checks; nested click-in-progress flags move with the node.
- [x] Regression source: `options_abort_and_dispatch_follow_native_targets_across_adoption` covers option order/throw/null/duplicates, exact foreign signal removal on DOM and standalone EventTarget before public abort callback, abort during snapshot dispatch, receiver adoption during conversion, captured bubbling path and moved cancelled checkbox activation.
- [ ] Complete all callback-driven traversal and synchronous graph algorithm migration review (TreeWalker filter reentry now has source-generated coverage; remaining algorithms still require review), all selected interface binding obligations and whole-plan qualification. No execution performed.

## XPath resolver and result ownership family

Pinned `domxpath/resolver-callback-interface.html`, `resolver-callback-interface-cross-realm.tentative.html`, `resolver-non-string-result.html`, and `resources/invalid_namespace_test.js` require callback getter/call/DOMString-conversion exceptions to be reported through the resolver realm's ErrorEvent, followed by evaluator NamespaceError. They do **not** require direct propagation of the callback exception. The prior local regression assumptions were corrected accordingly. Null/undefined namespace results produce NamespaceError without an exception report; callable resolvers bypass lookupNamespaceURI.

Source generation now enters the resolver creation realm, catches and reports original exception objects, and returns namespace failure to the evaluator. Evaluation resolves the original context object's current native identity after callback/type conversion, permitting cross-arena adoption without using stale IDs. XPathNamespace ownerElement/ownerDocument/baseURI/context and namespace lookup use its traced owner object; namespace identity cache searches native binding states by actual original wrapper. Snapshot Node and Attr anchors remain original objects after migration. Iterator position commits only after successful native anchor retrieval.

Regression sources cover getter/callback/return-conversion/type-conversion context adoption, Node/Attr/namespace result identity through repeated migration, namespace lookup conversion adoption, foreign resolver error realm/identity, null-result non-reporting, and failure preserving a reused result. Source review only; all execution and whole S-01 requirements remain open.

Native migration follow-through: detached-root and orphan-Attr ownership moves invalidate only original-document XPath revision tokens after commit preflight. Failed preflight preserves tokens, and unrelated documents remain valid. `detached_migration_invalidates_only_original_xpath_documents_after_preflight` and the binding detached-context iterator assertions are generated, unexecuted.

## Range contents and editing extraction family

Pinned `dom/ranges/Range-cloneContents.html`, `Range-extractContents.html`, `Range-deleteContents.html`, `Range-surroundContents.html` and their ShadowRoot variants define partially contained CharacterData copies (including empty boundary slices), rejection of contained doctypes before clone/extract mutation, and collapse after the start-side branch when endpoints span branches.

Source generation replaces recursive selection classification and partial-node processing with explicit stacks over the original tree. It preserves CharacterData kinds and stored UTF-16 lengths; fragments use the start node's document. Range delete/extract, surround and Selection.deleteFromDocument publish the specified precomputed collapse boundary. Partial shells use the native shallow-clone shadow behavior once, removing the superseded second shadow clone.

Range clone provenance now retains native source/clone leases and completes external template content using the same template-links integration as Node.cloneNode. Scope-less internal extraction producers record that provenance; all editing transaction entry points (keyboard, clipboard, semantic edit, drag, paired drag, execCommand) receive scope and complete it before post-edit snapshot/history publication. Direct composition edits complete it before input notification. Failure clears abandoned clone custody before rollback, and retirement clears remaining custody. No deferred author-visible template patching is used.

Regression sources cover clone/extract/delete across branches, empty boundary Text, Comment/CDATA kinds, foreign-document fragments, doctype atomic rejection, shared Selection collapse, deep partial paths, lone-surrogate storage, external template content, clonable shadow copies and internal editing clone completion. Source-only delivery; whole S-01 conversion inventory and execution remain open.

### MutationObserver conversion and migrated transient registrations — source generated

- [x] `observe` brand-checks the target before options conversion and resolves the retained original wrapper again after all getters/iterator/item conversions. Existing registration updates preserve their order and queued records.
- [x] `attributeFilter` uses shared WebIDL iterable conversion with IteratorClose on item conversion failure, original exception preservation, and lossless UTF-16 DOMStrings. Record fields and record arrays use own data-property publication, avoiding inherited array setters.
- [x] Transient sources use stable registration identity rather than arena-local NodeIds. Re-observing a source clears its transients across migrated routes while retaining independent registrations; takeRecords preserves transients and disconnect clears routes.
- [x] Regression sources cover four adoption callback stages, dictionary order/defaults/errors, IteratorClose and failed-conversion preservation, surrogate filters, inherited setters, adopted transient takeRecords/reobserve/direct-registration/disconnect behavior.
- [ ] Build/test/WPT qualification remains deferred until all planned source generation is complete. No execution evidence is claimed.

### Observer and slot checkpoint recovery — source generated

- [x] Pending observer batch custody restores only undelivered observers after record-preparation failure or callback termination, preserving original order and merging reentrant pending membership without duplicates. Retired observer owners are discarded.
- [x] Slot batches retain the current notification until event/path preparation succeeds. Native dispatch acceptance hook consumes before listeners, so interrupted/partially dispatched callbacks are never replayed. Remaining slot notifications survive observer or slot interruption.
- [x] Signaled slots resolve original rooted wrappers after adoption, both while collecting queued signals and after earlier checkpoint callbacks; queue normalization follows current arena and deduplicates identities.
- [x] Regression sources cover pending-order/membership restoration, actual missing-MutationRecord preparation failure across three observers/two realms, callback termination and later slot preservation, missing-Event preparation retry, started slot termination without replay, and slot adoption before/during checkpoints.
- [ ] No builds/tests executed; execution and conformance remain gated on completion of all source generation.


## Live collection and select conversion/adoption caller closure

Source-generated, unexecuted. The listed DOM/HTML test paths are Core under the
pinned longest-prefix policy. The two interface files are their IDL dependencies.
This bounded group does not close S-01 or assert WPT results.

| Required caller | Concrete native integration |
| --- | --- |
| NodeList, HTMLCollection, NamedNodeMap, StyleSheetList `item` | Brand-check the original collection, perform unsigned conversion, then resolve its current native owner/id before fetching the live member. |
| HTMLCollection `namedItem`, form controls `namedItem`, RadioNodeList `value` | Preserve original wrapper through authored string conversion; refresh moved collection records and current member ownership afterward. |
| NamedNodeMap qualified and namespace get/remove | Convert all string arguments in order before resolving the current element; normalize qualified names against that element's current HTML/XML document. Existing Attr identities are returned. |
| Options selectedIndex/length/remove and direct select remove | Perform authored numeric conversion, re-resolve current native receiver, then mutate that select. |
| Direct select and options `add` | One shared union conversion/native admission path; foreign option/optgroup identities are adopted through `node_migration::insertion_argument`. Receiver and node arguments are resolved again after numeric conversion; invalid references fail before adoption. |
| Options/select indexed setter | Native option brand check accepts actual foreign nodes; shared replacement/insertion admission moves original identity and preserves wrapper custody. |
| HTMLAllCollection names | Lossless UTF-16 lookup conversion and current-owner resolution; optional argument behavior remains native. |

Removed the duplicate direct-select add body. NamedNodeMap's existing
setNamedItem/default Trusted Types policy path already refreshes element and Attr
after authored conversion and remains shared. Collection migration keeps the
original wrapper and creation prototype; these changes follow its moved private
owner/id rather than looking up obsolete source records.

Regression sources in `node_lists/conversion_migration_tests.rs` cover two
independent native arenas, item/named/form/radio/attribute conversion-triggered
adoption, namespace argument order, original thrown identity, brand-before-
conversion, options setters/removal, foreign add/indexed replacement and invalid
reference admission. Ordinary named-property/iterator/descriptor matrix remains
subject to full source reconciliation and eventual execution; these tests are
not a blanket collection conformance claim.

| Pinned source | SHA-256 |
| --- | --- |
| `interfaces/dom.idl` | `3628b688fdcdb6904b4f69ca692e2dc030dfb4340478d879aa8fac41896c4086` |
| `interfaces/html.idl` | `f9c1783821e89a019428e3b8281fd628e9417e5abf1935ca261ca69f0a635978` |
| `dom/collections/HTMLCollection-live-mutations.window.js` | `51220cab1c293ad0f4ddbd3d22b506e43754698bf08cb3a00191836caf105ffc` |
| `dom/collections/HTMLCollection-supported-property-names.html` | `a2b981c66e13d5de73efd7467b86ce9e3d7ce88766ffd07d422ff1c4b576fc4a` |
| `dom/collections/HTMLCollection-supported-property-indices.html` | `71668c880f4f6d0769091f6d3a5032763a59239d55bb8fce2f8f475d657ed4da` |
| `dom/collections/namednodemap-supported-property-names.html` | `59ef0e0e866353a3a3ab8efb596cad5e01788329cd2084c74b104fb371df8fe0` |
| `html/semantics/forms/the-select-element/select-add.html` | `6af88415b85d5c601016c9184d224eb9a2ce8b17df8ad92f19b4b31b2347b6c6` |
| `html/semantics/forms/the-select-element/common-HTMLOptionsCollection-add.html` | `6f21a741f158dca03b57dc9ca754dd50fb7146739c496c3e2f19864567aa45e9` |
| `html/semantics/forms/the-select-element/select-remove.html` | `0629763acb491dc77fd443bce9046cfc64490b80393652af12b5112c74e4849b` |
| `html/semantics/forms/the-select-element/common-HTMLOptionsCollection-namedItem.html` | `f5822d6dd151aa0dd83b30a014e88b7c9a10bf3216163f021864d64f8f3c5c26` |


The same caller review found native select/options indexed setters and definers
bypassed generated CEReactions metadata. Both now wrap their shared native
mutation in an explicit `ReactionBoundary` and finish retained reactions before
returning. The new customized-option regression asserts synchronous connected
and disconnected callbacks for assignment and `Object.defineProperty`.
Additional source assertions cover post-migration indexed/named descriptors,
live iterator indices after migration/mutation, and lone-surrogate document.all
names. These are bounded sources awaiting independent review and execution.


## Range boundary conversion and cross-document ownership (2026-09-17)

Source generated; independent review and execution pending. The shared
`range_boundaries` dispatcher replaces the old setter/selector/comparison branches.
It preserves original numeric conversion exceptions, applies unsigned-short modulo
for `compareBoundaryPoints`, validates argument brands before algorithm errors,
and refreshes original Range/Node identities after author conversion. All eight
boundary setters/selectors support native nodes from other arenas without adopting
the node: the existing Range wrapper, prototype and expandos follow the destination
native live-range record. Boolean membership/intersection and comparison preserve
their distinct different-root results. Moving an associated Range into a detached
fragment or another document clears the original selection, matching the pinned
selection fixture rather than leaving an out-of-document selected range.

Regression sources cover throwing conversions, adoption during conversion, all
eight cross-document setters/selectors, live mutation after transfer, wrapper
identity, wrong-root comparisons, foreign/fragment selection removal, and native
range ID exhaustion without endpoint-root changes. Native creation and migration
preflight enforce the exact JavaScript-number identifier limit; creation also
preflights both endpoint-root increments. Transfer retains old endpoint leases
through native record removal. The adjacent contents dispatch continues to use the
shared contents classification, clone-completion and collapse helpers and is
included in independent review. Broader Range contents failure/reentrancy, XPath,
traversal, descriptor and realm matrices remain parent gates; these generated
regressions are not execution/conformance evidence.

Pinned requirement sources (same frozen WPT checkout as above):

| Source | SHA-256 |
|---|---|
| `dom/ranges/Range-compareBoundaryPoints.html` | `ed3fb1fdfe45985a5138429e958841e7012c00638648ac2b012b863681108bf2` |
| `dom/ranges/Range-comparePoint-2.html` | `0070855ad9d02766f846db8b80e44d91cdf9fdf1824f9d6ea5e199b4dc3e3efd` |
| `dom/ranges/Range-selectNode.html` | `0a8fca48d34e54ab3ce8af570b1b5dd8d117336a946aaae1dfeb3e2183828e8a` |
| `selection/move-selection-range-into-different-root.tentative.html` | `60e4621946cbd71b59d970fa090d61e31bab41998a6a3cc4923879845fd6690f` |


## XPath / traversal callback and retained-object reconciliation (2026-09-17)

Source-only review of actual pinned callers confirms native XPath already covers
resolver callback-interface lookup/call/result coercion, reporting callback errors
in the resolver realm before caller NamespaceError, context refresh after numeric
conversion/resolver adoption, cross-document HTML/XML matching, anchored node/Attr
snapshots, reusable result staging and document-scoped iterator invalidation.
`xpath_context_namespace_and_result_objects_follow_callback_adoption` covers all
four author-code phases and repeated namespace-owner migration. TreeWalker already
retains original Node wrappers across getter/callback/result-conversion adoption
for every cursor algorithm; NodeIterator keeps a separate mutable candidate cursor
through filter callbacks and native removal/migration journals.

Confirmed corrections generated in this review:

- NodeIterator now creates the non-callable `acceptNode` TypeError inside the
  filter callback realm, matching TreeWalker and the pinned callback-interface
  requirement. New regression covers missing/non-callable methods, revoked object,
  revoked function and revoked method proxies in a foreign realm, plus getter and
  result-conversion reentrancy, exact exceptions, failed cursor publication and
  unsigned-short return conversion for both traversal interfaces.
- XPath record lookup no longer revokes immutable expressions, retained results or
  namespace objects merely because execution services retired. Existing native
  `retire_execution` explicitly preserves detached DOM wrappers. XPath pure query
  operations now compare a captured generation/stream/retirement tuple, preserving
  rejection of lifecycle changes during resolver/conversion callbacks while
  permitting later access/query of an already-retained document. New regression
  retires a real binding state before reading its snapshot/namespace, evaluating
  its expression against retained and active documents, and checking that actual
  subsequent DOM mutation still invalidates an iterator. The pinned cross-document
  expression tests establish context-document semantics; the retirement regression
  is derived lifetime coverage, not claimed as a directly pinned fixture.
- Additional snapshot-index regression reuses the same result during numeric
  conversion, checking current replacement anchors/kind, preserved wrapper identity,
  original thrown value, unsigned-long conversion and brand-before-conversion.

No execution evidence is claimed. Callback realm/proxy behavior and lifecycle
regressions require later native execution; broader S-01 acceptance, complete IDL
and manifest outcome gates remain open. No tests/builds/generators ran.

| Pinned caller / dependency | SHA-256 |
|---|---|
| `dom/traversal/TreeWalker-acceptNode-filter-cross-realm.html` | `a4f33e1f404a79503ed68213cac44852c343e1ff94392fc8bf2321311a2d4dd8` |
| `dom/traversal/NodeIterator-removal-during-filtering.html` | `df919a487b2b594859523019ee248939d5401383c32c2e6401619b536688a163` |
| `domxpath/resolver-callback-interface-cross-realm.tentative.html` | `9fc03fbb8a0484632f2ec6a83c0c540818a129e7ccbb9af1734718a5cb1ca4b1` |
| `domxpath/resolver-non-string-result.html` | `e7768896f3365d5fbd791c8a96bf7255b1375a5999ed42c8ef790628c75b02b2` |
| `domxpath/resources/invalid_namespace_test.js` | `2dccb7da5aae765c783875ced8227d0533e0c29dbeaa6cd1f42dac283978011c` |
| `domxpath/expression-different-document.tentative.html` | `b56d3d44e35ed0d56184185f7f7d13b151751d530c8fbdd949b94acc9ec8fc19` |
| `domxpath/expression-cross-realm.tentative.html` | `d354a217c6039834cea6115d80d59be4c9f1b603a67e84af32884c180ca298b7` |
| `domxpath/result-iterateNext.html` | `397320f645feb5cd942af1f48a441b79957f453a29a6c3fdd8e803fcdde3b8b2` |


## Range contents caller/custody follow-up (2026-09-17)

Independent source review of the replacement contents branch found and corrected
three concrete defects: detached output could lose its final native lease before
return-wrapper creation; native Range clones omitted the CE registry/upgrade
completion used by Node.cloneNode; and taking the entire pending clone queue lost
unfinished work after a completion failure. Range clone/extract and surrounding
now root the output wrapper before clone completion and commit the calculated
collapse before reaction publication. No boundary write follows author reactions.
Clone jobs retain source/target leases and deep/shallow mode, progress through
external-template completion, registry completion and upgrade admission, and remove
only completed jobs. A failed stage retains that job and the suffix; completed
upgrade admission is not repeated. Scope-less editor callers still use their
existing snapshot rollback and explicitly clear jobs when abandoning the edit.

Partial Range shells now carry a shallow completion job rather than only shadow
child jobs. The template completion helper filters target edges according to the
same shallow/deep rule, so later inserted selected light children are not mistaken
for the shell's cloned shadow topology. Registry/upgrade completion includes the
partial shell itself. This covers the missing native caller behind pinned
`custom-elements/range-and-constructors.html` and `custom-elements/reactions/Range.html`.

Generated regressions cover detached complete output survival, partial custom
shell upgrades and tree order, constructor observation of collapsed boundaries,
constructor boundary changes surviving operation return, and injected completion
failure preserving the exact staged job plus suffix. Existing external-template,
clonable-shadow, UTF-16 CharacterData, doctype preflight, cross-container collapse,
Selection.deleteFromDocument and scope-less editing regression sources remain.
All are unexecuted. This is bounded source correction, not a claim of complete
allocation-failure atomicity for all native DOM mutations or S-01 conformance.

| Pinned caller | SHA-256 |
|---|---|
| `custom-elements/range-and-constructors.html` | `747f39a728292ac7625350f73bef776b5cf41eeeab6abcfb2b763042b19564be` |
| `custom-elements/reactions/Range.html` | `156460b6b0618adc878c3bfa20794396a62d32c16fffef4ce2f6f238c753a90e` |
| `dom/ranges/Range-cloneContents.html` | `54bf1b4d4692d0bbb3a1f3fc83d7afad603ab2eba5ce5458754abb092e354d5a` |
| `dom/ranges/Range-extractContents.html` | `396cf82e3cba87928b53262cfe597a1f279f2666707863e89b4cb62b6943fb08` |
| `dom/ranges/Range-surroundContents.html` | `047817cacaea3535c8ec19f1a3d1c8e80af4ad646e9e6beb7e777758837fa78c` |


Additional contents review: `surroundContents` now classifies CDATA as Text for
partial containment, matching pinned `dom/common.js::isText`, and selects its
original new-parent wrapper through the current Range owner after post-connection
hooks. The custom-shell regression includes adjacent CDATA surround.

The adjacent parsing caller had a distinct confirmed stale-owner bug:
`createContextualFragment` converted the string/TrustedHTML value but then used the
pre-conversion native Range store and returned a NodeId wrapped in that old owner.
It now resolves the original Range wrapper after conversion, reads its current
native live record, parses in that context and returns a wrapper from the actual
owner. A two-arena regression moves the Range during string conversion into a
foreign table context, checks row parsing/result ownership/prototype identity, and
preserves an original thrown conversion value and its deliberate Range side effect.

Next confirmed S-01 conversion inventory: Selection.getRangeAt, collapse/setPosition,
extend and related numeric routes retain nonthrowing numeric fallbacks and/or
capture arena-local Node IDs before authored conversion. That full Selection
caller group remains open for generation; this entry does not claim it fixed.


## Selection conversion and replacement group (2026-09-17)

The previously recorded Selection conversion residual is now source-generated in
`selection_bindings.rs`; superseded dispatch branches were removed. Numeric
arguments use throwing ToUint32 conversion, nullable collapse still converts its
offset before clearing, setBaseAndExtent converts each argument left to right, and
all Node/Selection identities are refreshed after author conversion. Algorithm
checks preserve their specified order: collapse checks node type/offset before
document membership; extend converts first then checks membership before empty
selection and boundary errors; setBaseAndExtent validates both offsets before
membership. Foreign/detached nodes are handled by native document membership,
without adopting nodes or treating valid foreign Node arguments as brand errors.

Pinned identity obligations exposed separate defects: collapse, setPosition,
selectAllChildren and collapseToStart/End now allocate and associate a new Range,
leaving retained original Range objects unchanged. Extend and setBaseAndExtent use
the same replacement primitive. Internal pointer/editing mutation helpers retain
their separate existing behavior. selectAllChildren uses child count rather than
CharacterData length. addRange/removeRange validate genuine live Range wrappers;
same-arena borrowed Range acceptance uses the existing shared native Range/wrapper
registry, preserving prototype and expando identity. Source review confirmed that
these maps are shared: no separate rehoming path is needed or retained. getRangeAt refreshes
the association after conversion and preserves exact original conversion errors.
Foreign containsNode returns false. Selection.deleteFromDocument now declares the
pinned CEReactions metadata; modify has the pinned optional parameters without
invented movement defaults (omitted inputs produce no movement).

Generated regression sources cover replacement/original Range identity, Text
child counts, numeric exceptions/modulo and null conversions, doctype/offset/root
error order, receiver/Range argument brands, getRangeAt conversion clearing the
selection, incoming/outgoing cross-arena adoption during conversion, two-argument
conversion order, foreign Range handling, and same-arena borrowed wrapper custody
through replacement and old-store collection. No builds, tests or generators ran.

The initially absent getComposedRanges/ShadowRoot sequence dependency is superseded
by the source integration below; independent review and execution remain pending.

| Pinned requirement / caller | SHA-256 |
|---|---|
| `interfaces/selection-api.idl` | `1500d82b7f72f52c6a5658fe06123ba15e035e5a0d38f138f567362275c1b61c` |
| `selection/collapse.js` | `410d166f607ec6634efa1caa4f3a266f1607ade2a1b9d85044e2ce030f8d6818` |
| `selection/extend.js` | `0974d28a87ff79c12df15779b06013c2d4d2c8e53fb33af2372b05308cd76753` |
| `selection/collapseToStartEnd.html` | `130ef0cd0cbca9f666e65092fa44724778a8076472c4e345072782e7ff447ed7` |
| `selection/selectAllChildren.html` | `42f962d2afb424b6b03feb0dc875081e0b85f150886dc83b0128efa68d938302` |
| `selection/setBaseAndExtent.html` | `b9284f5c7ecd37b17fff2d5a0d6c47b907b7e3d2659d0e9f8bd9a6b3ce21e24b` |
| `selection/getRangeAt.html` | `e2d2f58988e0e6033a0798331121f530f48701b66594b6e3899ef75e96f6e915` |
| `selection/shadow-dom/tentative/Selection-getComposedRanges.html` | `144fccd1cdf1f0ce9fb21fe3670498b2ae396de50512743996cc25a653c379d7` |


## Composed Selection endpoint custody and projection (2026-09-17)

Generated `getComposedRanges` IDL/dictionary and native sequence conversion, including
getter/iterator exception identity, per-item ShadowRoot brands, IteratorClose on
conversion failure, and refreshing original root wrappers after author iteration.
The returned StaticRange uses the existing mixed-arena frozen-endpoint custody.
Endpoint rescoping hides unlisted shadow roots, permits listed inner roots and their
shadow-including ancestors, and projects start/end around hidden hosts separately.

This required fixing the underlying Selection storage: an ordinary Range cannot
represent the cross-shadow selection without losing an endpoint. Native composed
companions now retain both actual endpoints, participate in the shared Selection
range root list, follow live text/removal adjustments and determine anchor/focus,
direction and composed snapshots. Script Range mutation invalidates its companion;
selection replacement/clear and arena migration update the associated custody.
Noncollapsed selections involving shadow trees use native marked companions so
host removal adjusts the actual selection through shadow descendants. Ordinary
Range removal still preserves script-held ranges inside the removed shadow tree.
A native shadow-removal revision lets a collapsed selection disassociate after
host removal or remove/reinsert while preserving the original Range, as required
by the separate pinned removed-shadow fixture. Native range release, arena reset
and migration clear or transfer this metadata.

Generated regression sources cover hidden/listed/nested roots, forward/backward
cross-shadow endpoints, static snapshot immutability, native text updates, host
removal, collapsed remove/reinsert, explicit Range mutation, dictionary effects,
invalid iterable items/IteratorClose, and adoption of a host during option iteration
with mixed-arena retained StaticRange endpoints. A native DOM regression isolates
ordinary Range preservation versus composed Selection removal behavior.

No builds/tests/generators ran. These are source contributions awaiting independent
review. Full flat-tree editing/stringification, arbitrary cross-shadow deletion,
all IDL descriptor/realm outcomes and the broad S-01 qualification remain distinct
acceptance gates; this projection work does not establish those outcomes.

| Pinned caller | SHA-256 |
|---|---|
| `selection/shadow-dom/tentative/Selection-getComposedRanges.html` | `144fccd1cdf1f0ce9fb21fe3670498b2ae396de50512743996cc25a653c379d7` |
| `selection/shadow-dom/tentative/Selection-getComposedRanges-dom-mutations-removal.html` | `3d2d30abae0fde95ac79043da115912bb72bc7b9faea23c9dd61c974acfc2ba7` |
| `selection/shadow-dom/tentative/Selection-collapse-and-extend.html` | `560caa376b6a65e2963471a1ac881a764a471744ae90b5e51a705e6e050189c1` |
| `selection/shadow-dom/tentative/Selection-direction.html` | `ac80c640639de66cc0d88eda4812c12b558a3c19a12463ea56c692c2abc73453` |
| `selection/selection-range-in-shadow-after-the-shadow-removed.tentative.html` | `3eebb6bfc3b03617670ca21c6911fbf9cda7ba5c80f739556efb9775d453cb5b` |
# Independent Selection association review — 2026-09-17

Source inspection of `node_migration::migrate_ranges` and
`synchronize_selection_range` found that migration revokes the shared primary
and composed Range association cells without clearing each binding's cached
anchor/focus. Synchronization now clears Selection when the primary association
is absent. The regression source
`selection_composed::tests::revoked_primary_association_clears_cached_endpoints_once`
covers empty observable Selection state, revision idempotence, and preservation
of a script-held Range after association revocation. This is a source-only
correction; the regression has not run. Complete Selection editing, composed
lifetime reconciliation, S-01 acceptance and whole-plan qualification remain open.

The companion lifetime review also traced `selected_range_ids`,
`collect_dead_ranges`, and native `drop_live_range`. The added regression source
`unwrapped_companion_is_rooted_only_while_associated` exercises collection while
an unwrapped companion is selected, replacement with a new companion, and final
Selection clearing. It checks that obsolete companions are released while a
script-held primary Range retains its original endpoints. No garbage-collection
or regression execution has been performed; these are generated assertions for
the eventual global test phase.

Synchronization now refreshes the ordinary associated Range record as well as
the composed companion. Previously, a companion caused only its own cached
record to refresh, leaving primary-record consumers dependent on an unrelated
global refresh after native mutations. The regression source
`native_mutation_refreshes_primary_and_composed_records_together` mutates native
CharacterData directly and checks both cached records and visible endpoints.
This source correction and regression remain unbuilt and unrun.

Independent rendered Selection review corrected the `hidden` attribute fallback:
when resolved display is present, it determines visibility instead of an
unconditional attribute exclusion. Regression source
`selection_text::tests::resolved_display_can_override_hidden_attribute` covers
an explicit inline override, default hidden behavior without resolved style,
and explicit display:none. The source test supplies resolved layout values;
end-to-end style resolution and conformance execution remain pending.

### Selection displayed text / DOM deletion source reconciliation (2026-09-17)

Generated `selection_text.rs` and `selection_deletion.rs`; replaced the legacy Selection stringifier (which reused Range text and silently swallowed errors) and cached-record deletion branch. **Source only: no build, test, generator, or WPT executed.**

Pinned caller evidence: `selection/stringifier.tentative.html`, `toString-user-select-none.html`, `toString-ff-bug-001.html`, `stringifier_editable_element.tentative.html`, `deleteFromDocument-HTMLDetails.html`, and `shadow-dom/tentative/Selection-deleteFromDocument-around-shadow.html`. The last fixture explicitly requires DOM Range deletion rather than rendered flat-tree deletion.

- Stringification traverses native flat children (including slot order), excludes hidden/display-none text and native used `user-select:none` while honoring explicit descendant `text`, and keeps Range.toString on its separate DOM collector. Native shaping mappings provide transformed/collapsed text with UTF-16 source affinity. Text-to-text endpoints use flat positions even when assigned light nodes lie between shadow endpoints. Source regression covers reordered slots, unslotted/hidden exclusion, partial shadow endpoints, lone surrogates, and DOM/Selection text differences.
- A tracked native control lease remembers the selected text control across button focus, follows adoption identity without borrowing another document's control authority, and is replaced by another text control or explicit DOM Selection. Native focus, select(), and pointer control selection publish it; explicit empty/collapse-null and DOM range publication clear it. Native control value/endpoints provide whitespace-preserving text. Regression source covers focus/select/selection setters, button focus, another input, explicit DOM selection, empty/reselect, and disconnection.
- Deletion reads the current native associated Range, keeps its script-visible identity, runs the existing DOM contents algorithm, and commits collapse before the surrounding CEReactions boundary. Collapsed primary ranges are no-ops. Generated regressions cover unassigned and assigned host light children without deleting shadow content, same-shadow text, and closed details content.

Remaining source obligations in this group: reconcile mixed element/text flat endpoint projection, CSS line/table separator details, all native focus paths outside `focus_html_element`, and the tentative actual-pointer hyperlink/button selection preservation cases against the full input caller graph. The text fallback without shaping is a source-only native fallback; visual acceptance requires later layout-backed verification. This bounded entry does not close the S-01 parent or claim complete Selection editing conformance.

Follow-up source generation: Selection text now refreshes native layout before inspecting rendered state. The retained navigation focus transition also preflights/publishes selected-control custody before focus listeners (so retry cannot repeat a partially accepted focus). Actual native pointer admission preserves remembered control selection for hyperlink ancestors with `href`; ordinary pointer caret publication replaces it, while existing `user-select:none`/button exclusion leaves it intact. Production `focus(Some(..))` caller scan found only those element-focus paths plus browsing-context root focus, which does not choose a new text control. Exact rendered mixed endpoint/table/line semantics remain under source reconciliation, rather than being inferred from routing.

Additional source review corrections: resolved display also overrides script/style/head UA hiding; shaped text keeps explicit selected edge spaces instead of re-trimming native shaping output. Table-cell separators reuse the existing native formatting-role separator helper instead of duplicating tag-only rules. The retained focus retry regression now asserts that the destination's selected native value is visible inside its focus callback and after resumption, while preserving the existing once-only blur/focus suffix assertions. None of these regression sources has executed.

Mixed endpoint follow-up: collector now projects a text endpoint paired with a visible element/ShadowRoot child boundary into native flat preorder/subtree limits. This excludes preceding assigned slot text when the start is later shadow text and the end is after the host. The slot regression includes that mixed case. Nonprojectable unrendered endpoints retain the ordinary DOM comparison fallback; complete browser-specific visual edge/line behavior remains an execution/reconciliation gate rather than a claimed pass.

### Remaining Selection caller audit and ParentNode conversion ownership (2026-09-17)

Read-only inventory of the pinned `selection/` subtree found 182 HTML/JS source files excluding `-ref` files (this is a source-file count, not a manifest test/variant count). Direct lexical caller groups include 39 modify sources, 14 stringifier sources, 9 composed-range sources, 8 deletion sources, 13 selectionchange sources, and 2 selectstart sources. Shared addRange/collapse/extend helpers and crash/reference cases remain part of dependency review; these counts do not claim passing coverage.

Confirmed and generated corrections:

- `selection-at-nodes-not-part-of-flattened-tree.html` requires `containsNode` false for unassigned light nodes and suppressed fallback descendants, while Range.comparePoint and Selection endpoint storage remain DOM-based. Native flat-ancestry membership now precedes containment comparisons; regression preserves ordinary Range semantics. **The fixture explicitly marks mixed rendered/unrendered boundary behavior TODO.** Existing valid DOM endpoints and the collector fallback are intentional compatibility behavior; an undefined upstream TODO is not a missing conformance algorithm.
- `selection-content-visibility-hidden.html` requires excluding skipped content. Collector checks computed content-visibility (already exported by the native layout provider). Range.toString stays DOM-based.
- `script-and-style-elements.html` requires rendered script/style text with structural leading/inter-block breaks. The collector retains collapsed whitespace block transitions, trims source spaces beside structural breaks, and preserves preformatted separators. Flex/grid line-navigation callers require two newlines between paragraphs; required paragraph breaks now survive whitespace normalization in either selection direction. Generated source regressions cover these expectations without changing movement endpoints.
- Native `focus(Some(...))` caller reconciliation covers ordinary element focus and retained navigation focus; browsing-context root focus does not select a new text control. The existing native pointer exclusion algorithm preserves user-select:none/button source selection; hyperlink admission has the explicit native href check described above.

S-01.f production review confirmed cross-arena argument adoption already exists in `stage`/`node_migration::insertion_argument` and final foreign insertion/replacement validation. Removed outdated checklist wording claiming same-arena-only arguments. A separate actual gap was fixed: all variadic DOMString conversion finishes before resolving the original receiver wrapper's current owner/NodeId. Thus append/prepend/replaceChildren/before/after/replaceWith operate in the receiver's post-conversion arena. A two-arena regression exercises all six methods, original wrapper/prototype identity, migrated observer grouping, foreign argument adoption, and a throwing conversion that retains its author adoption side effect but performs no staging.

Temporary conversion nodes/fragments now hold a migration-following native lease from creation through final insertion or failure. This supplies explicit custody during wrapper/observer migration allocation and releases on failure without reverting specification-required earlier removals. Existing source grouping/self-reference/failure-side-effect regressions remain authoritative; no clone substitutes for foreign adoption.

**Execution-only obligations:** binding generation/build checks, executing the generated source regressions, pinned WPT variants/reftests and historical crash/RSS verification all remain deferred. **Further source review:** finish the full S-01 exposure/descriptor/realm member inventory and remaining convenience-mutator failure/allocation publication callers; no whole-parent completion claim is made by this entry.

### Native keyboard Selection admission — further confirmed implementation gap

Pinned `onselectstart-on-key-in-contenteditable.html` distinguishes collapsed-to-extended selection from unchanged movement and an already noncollapsed selection, and requires cancellation to preserve the original Range. The native keyboard path previously called Selection movement/select-all without selectstart.

Generated a movement admission boundary after shaped/textual target calculation and before Range/affinity publication. Trusted keyboard and AppKit movement dispatch cancelable selectstart only for the transition that needs it; cancellation retains the original Range identity and offsets. Selection/tree/document changes during the callback supersede the prepared movement rather than applying stale native offsets. Keyboard select-all uses the same admission; text-control select-all retains its own native custody and queues its select event only when endpoints change. Script Selection.modify retains its ordinary non-event API behavior.

Native PageUp/PageDown now target a shaped line approximately one native viewport page away, preserving the inline goal and source writing-mode basis; text controls share that geometry helper. Public Selection.modify still rejects the internal page unit. Generated regression sources cover canceled/allowed extension, no-moving-edge and already-noncollapsed suppression, author selection reentry, canceled select-all, forward/backward viewport distance and inline column preservation. Full native input interruption/receipt review remains separate; no execution performed.

## Retained native keyboard/Selection/edit source integration (2026-09-17)

Generated `native_keyboard_receipt` owns the original binding instance, stream/generation, key input, tracked target and event objects. It resumes native Notification listener suffixes for keydown/keyup/keypress and cancelable selectstart. Selection proposals retain original range identity/revision until admission completes, and author selection changes supersede the prepared default. Native controls preflight select-event admission before publishing selection endpoints. Rich edits use `retained_editing::Transaction`; beforeinput/input use `ClipboardNotification`, preserving successful mutation and callback suffixes. Clipboard defaults share `ClipboardJob` rather than introduce a second backend path.

The native host enters the receipt's exact original realm and validates binding instance as well as document identity. Keyboard admission refreshes modifiers across the native root/frame pointer tree. WebDriver Actions now retains a source cursor and frozen keyboard progress between polls: held input mutation is performed once, prior sources are not replayed, and accepted OS interpretation suffixes retain the native result. Recoverable retained-stage infrastructure errors have a 10ms retry deadline, at most eight failed attempts, and return the original failure on exhaustion; author exceptions remain normal event-dispatch reporting. Unretained compatibility phases are never retried automatically.

Regression sources cover callback suffix identity/once listeners, canceled selectstart preserving the original Range, successful extension only after callback completion, control and rich beforeinput/input failures with one mutation, pending second Actions source preserving its cursor and key repeat flag, delayed/bounded retry and original-error custody. No tests/builds/generators executed.

**Implementation still required:** focus fixup/Tab/radio transitions, native picker and permission-prompt keyboard defaults, key activation/implicit form submission, OS deferred text/composition defaults, and the synchronous Release Actions path must join retained default/operation ownership. Existing direct native keyboard dispatch is still the compatibility path for non-Actions callers. These are actual code-generation requirements, not execution-only gates. Broad S-01/S-04 and Core/five-family acceptance remain open. Metadata regeneration, compilation, generated regression execution, full WPT and five-family qualification follow global completion of source generation.

Follow-through superseding three keyboard residuals above: the native runtime Release Actions branch now owns a polled `ReleaseExecution`; keyup suffixes complete before the next reverse-order release and before clearing input sources. The prior synchronous Release entry is now test-only. Native keyboard admission owns retained focus eligibility change/blur/focusout publication, with dirty-control state consumed only after event allocation. OS deferred text admission now retains keypress before publishing the interpretation target/grant. Added source regressions for suspended reverse-order release, focus fixup suffixes before key delivery, and deferred keypress admission without native edit. Tab/radio focus, picker/prompt/activation defaults, composition-operation jobs and direct compatibility dispatch still require further generation. Release pointer defaults continue through their existing pointer implementation; keyboard custody does not claim complete pointer default recovery.

Tab follow-through: extracted `focus_navigation::next_sequential` without changing candidate/radio-group ordering, and added retained `FocusJob` with traced previous/destination nodes, change/blur/focusout/focus/focusin phases, live eligibility refresh and author-focus revision guards. Source regression covers interrupted blur suffix and callback redirect superseding the original destination. Separately corrected `form_activation::prepare`: checkbox/radio preactivation now allocates both native leases and wrapper custody before toggling checkedness; injected-allocation regression preserves checkbox and the entire radio group. Retained click activation finish/reset/submit generation is active work, not yet delivered by these changes.

Shared publication suspension review: `Notification::resume` now checks native execution suspension before preparation, between listeners and before/after property handlers. Property getter completion retains its returned callback independently; a parked getter resumes the callback after restoration without rereading the author property. Regression sources cover listener-triggered and getter-triggered suspension. Native keyboard now shares `retained_dom_event` and `retained_focus` with the ongoing activation work. `retained_form_reset` and `retained_validation` helpers have been generated but activation/submission caller wiring is still in progress; helper presence is not completion evidence.

Listener-object follow-through: Notification now retains `handleEvent` resolution separately from method invocation, using `global_events::resolve_listener`. Once-listener removal stays committed, returned callable/receiver remain rooted while parked, and abort/removal during the getter does not cancel the already-started callback. Ordinary getter exceptions are reported without retry; native allocation failures before getter invocation remain retryable. Expanded suspension source regression covers a getter that aborts its signal and parks, followed by one method invocation and the remaining listener after restore.

Native activation integration now routes Enter/Space keypress and Space keyup through retained Click/Finish jobs. Click guards use weak traced native leases, preserving nested-click suppression through adoption and releasing immediately on completion/abandonment. Checkbox/radio input/change callbacks and reset/invalid/submit event suffixes retain progress. Canceled keyup clears the armed Space action. Native submission Request captures original form-document authority before validation callbacks. Source regressions cover interrupted click with once/nested click, canceled click rollback, canceled keyup disarm, interrupted reset recursion guard, invalid/submit cancellation and FormData entry identity. Post-validation queue/FormData/selected-target continuation and link/picker/label/general command defaults are still active generation work; their compatibility tails explicitly reject automatic retry. These are not closed source or runtime gates.

### Command activation and fullscreen integration (generation in progress)

Native `command::Job` now freezes the invoker, target, command and trusted composed
`CommandEvent` before callbacks, retains callback progress, and admits defaults
only after uncanceled completion. Native click `Finish` owns this job; the script
click path uses the same algorithm synchronously. Media defaults call the shared
media implementation, and popover defaults own `PendingToggle` rather than
re-enqueue a new transition on retry. Generated regressions cover callback suffix,
payload identity, target changes and form-owned button type restrictions.

The shared `fullscreen` module now supplies script and command entries, native
`TopLayerKind::Fullscreen`, policy/activation admission, document/shadow getters,
and bounded retained fullscreen change/error publication with actor deadlines.
This is **not a completed fullscreen gate**: ancestor/descendant browsing-context
propagation, native removal/stream reset reconciliation, transactional allocation
around activation consumption, and the complete fullscreen fixture/regression
inventory remain under generation. Dialog, details, stepping and logical-page
scroll command defaults also remain open. No build, generator or test execution
has occurred for these changes.

Fullscreen follow-up source generation: same-actor entry now authenticates native
parent/frame ownership; exit collects simple fullscreen ancestors and descendant
documents, with a native iframe fullscreen flag distinguishing explicit iframe
requests. Native removal reconciliation keeps document-scoped events alive across
`document.open`; Escape shares the exit algorithm. Initial and committed frame
producers in `host.rs` now publish a separate private embedding-frame identity.
The existing author-visible `frameElement` remains null for cross-origin frames;
fullscreen never borrows that filtered author interface as its authority. The
nested-context regression uses null public frameElement and a replaced `parent`
property while native entry/exit still updates the real frame chain.

Traversal-owner review identifies same-page iframe attachment as same-agent;
independent-actor DOM/frame attachment is still the broader P/R source gate, not
an implemented path silently counted here. Fullscreen lifecycle retirement/park
and the remaining fixture matrix still require reconciliation. This evidence is
source generation only, not execution or acceptance.

Command defaults now include physical/logical page scrolling using the native
scrollport, computed writing mode/direction and native coordinate bounds. The
command job separates its single offset mutation from scroll notification
publication; shared scroll-task preparation now prepares both missing leases
before appending either event. Details toggle/open/close and number stepping use
native element state. Invalid built-in/target pairs are rejected before emitting
CommandEvent (as pinned invalid-popover fixtures require).

A shared retained `dialog::Job` now backs script show/showModal/close/requestClose
and dialog command defaults. It retains cancel/beforetoggle/focus continuations,
commits open/top-layer state once and queues toggle/close tasks. Native
`dialog_return_value` replaces the authored data-oryn-return-value authority in
getters, setters, close and dialog-form submission. Argument conversion occurs
outside the DOM borrow and resolves the receiver again. Generated regression
sources cover callback retry, native return-value isolation and conversion-time
mutation. Remaining caller review includes disconnected/adopted dialog ownership,
modal focus restoration and dialog-form retained submission tails; these are not
marked complete by the presence of the shared module. All execution is deferred.

Native text-system continuation source: `native_keyboard::TextCommandReceipt`
now covers Tab, native Selection/control movement, all declared deletion units,
and newline through the shared retained focus/selectstart/beforeinput/input edit
jobs. Control movement preparation is callback-free and shared with Selection
modify; deletion expands its range only after accepted beforeinput. Source
regressions cover DocumentBoundary deletion retry and cancellation preserving
control selection. Composition/Actions caller integration belongs to the root
lane and is not inferred from this helper delivery. No execution has occurred.

### Composition command and form preparation continuation source review

- Generated composition-level control/rich-edit regression for interrupted compositionend followed by interrupted command beforeinput, suspended resume, callback suffix identity, one mutation, and the shared two-unit undo history. Root owns production composition integration.
- Form Queue now freezes parameters/source transaction, selects its native navigable once, then prepares query/body from retained FormData. Serialization failure preserves the selected receipt and frozen method/action/enctype; no repeated popup selection or parameter reads. Generated injected-body-failure regression changes form fields before retry and verifies the original POST payload/target.
- Source only: no build, generator, tests or WPT executed. Remaining activation compatibility tails and fullscreen/dialog lifecycle coverage remain open; these bounded changes do not close S-01 or broader default-action gates.

### Native picker and permission keyboard receipts — source generation

- Select picker/listbox commits now retain selectedness mutation and the original trusted input/change event suffix. Native keyboard holds the job through completion (including Tab focus follow-on); native click/pointer activation owns it through Finish.
- Temporal/datalist picker choices retain beforeinput, original session/value validation, one native edit, input and change. Native keyboard and Finish share the job. Generated regressions cover interruption at every callback stage, suspension, undo and author replacement cancellation.
- Keyboard permission choice now uses the same process-wide resolution receipt namespace as pointer input, retains parent PromptResolve response through publication and never repeats a grant after a lost response. Generated source regression checks identical receipt reuse and no backend calls while suspended.
- These changes are source-only. Synchronous script picker adapters and native AX callers still need their own complete continuation ownership; AX Adjust/Select focus/key and file picker callers are included in the next transaction inventory. Generic link admission and lifecycle gates remain open.

### AX retained operation chain — generated, independent review pending

- NativeAccessibilityReceipt owns original document/target, Select focus then guarded native publication, custom Adjust focus and keydown/keyup receipts, native Adjust input/change suffix, temporal/permission child receipts, file payload commit/cancel events, and final read snapshot. Snapshot retry does not replay a committed selection or adjustment. Superseded direct adjustment/selection bodies were removed from the native AX implementation.
- RawV8Host enters the receipt owner context; RustExecutableDocument stores and revalidates the exact selected AutomationRealmSnapshot. PageState owns bounded pending replies, deadlines, original failure and a 10ms retry deadline. Root and ScopedWorker actor commands both enqueue; idle timer/readiness/poll integrate the queue. Closed receivers, replaced scopes and deadlines release the receipt. Direct synchronous PageState text calls are now read-only.
- Generated native regression sources cover custom keyboard suffix/keyup once, final snapshot failure after selection commit, and file input/change suffix with immutable native payload. Existing child-document actor selection regression now traverses the retained actor route. No tests/builds/generators have run; actor failure injection, deadline/close review and further source-contract review remain before a source-complete claim.

### AX scheduler/BFCache review corrections

- Parked owners now contribute only the original request deadline; polling skips their V8 context. Revoked/closed owners schedule cleanup immediately. Active requests preserve FIFO without a parked sibling blocking them, and later deadlines are checked without zero-delay polling behind a delayed head.
- Added source regressions for original-error deadline completion, canceled reply cleanup, document.open invalidation, suspension-only deadline wakeup, and actual load/BFCache park/restore returning the original document text rather than replacement text. Scoped admission retains existing page checkpoints before its Read; the selected scope is captured/revalidated after that checkpoint.
- Script dispatch now retains interrupted Finish defaults in a bounded native activation queue, integrated with deadline/readiness and retirement. Generic dispatcher preflight allocation and remaining link/permission-open compatibility admission still require review. No execution performed.

### Accepted hyperlink destination custody (source generation, 2026-09-17)

The earlier open hyperlink tail is now replaced by `submissions_link::LinkActivation`: freeze source parameters/provenance, retain successful target selection, authorize, reserve destination publication capacity before Navigate callbacks, then publish the admitted navigation into that destination's native default queue. Source generation/stream/authority is checked before admission; accepted destination work survives source retirement without rereading the source element. Counted reservations prevent reentrant enqueue from consuming the reserved slot. Selection/remote dispatch errors remain terminal because an ambiguous service response must not allocate or navigate a second target; interrupted Navigate delivery uses its existing canceled-admission continuation rather than issuing a new token.

Native host queue readiness now exposes live, nonparked active-root default actions. The executable retains an already admitted local link on native scheduling failure, using its existing 25 ms retry deadline; this safe tail revalidates the saved destination/token and does not dispatch Navigate again. Unadmitted form and remote service operations are not blindly retried by this change.

Generated regression sources cover destination Navigate retiring source, post-publication failure without duplicate queue entry, interrupted Navigate suffix without re-admission, and executable pre-schedule failure preserving original URL/target plus one Navigate callback. The latter also asserts host readiness before extraction. These are source assertions, not executed results. Remote service lost-response reconciliation, broader form continuation/authority review, and final native/WPT execution remain separate open acceptance obligations.


## Custom-element callback conversion review — 2026-09-17

Pinned `custom-elements/registries/define.html` explicitly requires conversion of every non-undefined lifecycle callback to Web IDL Function and includes null as a TypeError case. Source review found `registry_operation` skipped null for both ordinary and form-associated callbacks. Both loops now skip only undefined; null and other non-callable values reject before publishing a definition. The existing definition guard releases on that failure.

Regression source `definition_callback_conversion_rejects_null_and_allows_retry` covers all nine callback names with null, object, number, boolean and string values, checks TypeError and absent definition, and retries the same name/constructor with undefined. It also checks that an ordinary non-form-associated definition does not convert an unused form callback. These are generated assertions, not execution evidence. Full S-01/S-02 interface, reaction and qualification requirements remain open.


## whenDefined promise binding boundary — 2026-09-17

`generated_operation_impl` previously performed the arity check before routing CustomElementRegistry, and `registry_operation` synchronously threw receiver/name-conversion errors. The registry route now owns its generated arity check; whenDefined wraps those binding and algorithm exceptions in a rejected promise, preserving the original exception value. Other registry methods retain synchronous exceptions. Execution termination remains termination rather than becoming an authored rejection.

Source regression `when_defined_rejects_binding_and_conversion_errors_without_throwing` covers missing arguments, invalid and proxy receivers, Symbol conversion, exact thrown-object identity and one conversion call, invalid-name SyntaxError, asynchronous rejection handlers, and the synchronous get() control. Source inspection confirmed the metadata arity is u8 and the single production registry dispatcher uses the new boundary. No build or test execution; cross-realm promise allocation and the complete interface inventory remain open.


### Borrowed whenDefined realm and shared pending identity

Generated `borrowed_when_defined_uses_function_realm_and_preserves_pending_promise` using two separately installed native Window contexts. It checks that borrowing the other realm's operation creates promises and conversion TypeErrors in that function's realm, preserves an authored thrown object from the caller realm, shares the registry's pending promise across borrowed/local calls, resolves that exact promise with the accepted constructor, and creates a fresh resolved promise after definition. This adds the concrete cross-realm regression source missing from the preceding entry; runtime behavior and full binding qualification remain unverified until the global generation gate permits execution.


## Input and textarea string-attribute exposure — 2026-09-17

Pinned `html/dom/elements-forms.js` includes input accept/alt/placeholder/dirName and textarea placeholder/dirName as reflected strings. These attributes were absent from browser.webidl and native reflection dispatch. Added CEReactions DOMString declarations, getter/setter routing, and the dirName-to-dirname attribute mapping.

Generated `control_string_attributes_use_native_reflection_and_reactions` checks missing defaults, one authored conversion, lone-surrogate DOMString preservation, actual native attributes rather than own-property expandos, attribute-to-property updates, null conversion, removal defaults, and customized built-in input attributeChangedCallback delivery for all four attributes. Binding generation and execution remain deferred. This is bounded interface coverage; full S-01 remains open.


## Control dimensions and input textLength — 2026-09-17

Added missing input size/textLength and textarea cols/rows/wrap IDL declarations. Input textLength uses the existing UTF-16 native value-length getter; wrap uses raw DOMString reflection. Size/cols/rows implement HTML non-negative integer parsing, defaults (20/20/2), positive signed-32-bit range, unsigned-long setter conversion, input zero IndexSizeError, and textarea zero fallback. Pinned elements-forms.js and reflection.js supply the categories and edge rules.

Generated `control_dimension_reflection_and_input_text_length` covers missing/invalid/raw attributes, HTML versus non-HTML whitespace, integer prefixes, fractional and unsigned-wrap setters, zero behavior, original conversion-error identity with unchanged state, UTF-16 input length and raw wrap reflection. Binding generation and execution remain pending; full S-01 remains open.


### Reflected-control conversion ownership follow-up

Source inspection found the generic setter retained a source-arena node ID and used lossy string conversion for newly exposed control strings. Dedicated setters now retain the original native wrapper, preserve DOMString UTF-16, and resolve the current owner after conversion for input accept/alt/placeholder/dirName/autocomplete, textarea placeholder/dirName/wrap/autocomplete, and form/select autocomplete. Input size and textarea cols/rows likewise resolve after numeric conversion. Removed the superseded new-string generic setter arms.

`reflected_control_setters_follow_conversion_time_adoption` uses two native arenas, moves each receiver during string/numeric conversion, checks destination attribute writes and one conversion, and preserves prior numeric value when a subsequent conversion moves back then throws. No execution; broader setter migration review remains open.


### Existing select and text-limit setter migration

Extended original-wrapper custody to select selectedIndex/length/size and input/textarea minLength/maxLength. These paths now resolve the destination owner after authored numeric conversion, before selection changes, option-count changes or attribute writes. Existing conversion and range-error rules are preserved; a throwing conversion may move the node but does not commit the requested property change.

Expanded `reflected_control_setters_follow_conversion_time_adoption` to cover all five property kinds on both text-control interfaces where applicable, with independent native arenas, numeric valueOf adoption, property/attribute results and move-back throwing conversions. Source-only; no execution and no broad setter-audit closure claim.


### valueAsNumber conversion-time migration

Numeric/date value setters now retain the original input wrapper and resolve its current native owner before applying the converted value. The authored number conversion may move the input or change its type; the native setter evaluates applicability against that resulting control. The pinned input-valueasnumber-typeerror.html explicitly requires infinity TypeError even on unsupported input types, so its existing precedence is preserved.

Expanded the two-arena setter regression with number conversion adoption, conversion changing number to text (InvalidStateError without value assignment), text to number plus adoption (successful assignment), infinity precedence and move-back throwing conversion preserving its exact exception and prior value. Date handling retains native Date extraction without calling authored valueOf. Source only; no execution.


### Shared control-string setter migration

The remaining shared conversion suffix now retains the original wrapper for input, textarea, select, button, option, optgroup and form controls and resolves the current owner after conversion. It also preserves DOMString UTF-16 for these interfaces while keeping src/formAction/action on USVString conversion. This covers values, default values, names and labels that bypass the earlier dedicated accessors; it avoids writing through a source-arena NodeId after toString adoption.

Expanded the two-arena source regression with all seven control interfaces, native values/defaults/name/pattern/label/text assignments, one conversion, lone-surrogate preservation, and move-back throwing conversions preserving the previous property. Source only, unrun; URL policy and remaining non-control setters remain separate review obligations.


### Text-selection property conversion ownership

selectionStart/selectionEnd/selectionDirection now retain the original native wrapper before numeric/string conversion and resolve the current owner afterward. Type checks, value-length clamping, selection lookup, select-event admission and state publication use that owner. This preserves the previous conversion-before-type-check ordering while removing stale source-arena access.

Expanded the two-arena setter regression for input/textarea start/end/direction adoption, endpoint collapse, move-back throwing conversion, and conversion that changes an input to number before applicability validation. Source only; queued event execution and full input qualification remain unverified.


### Text-editing and stepping method conversion ownership

setSelectionRange, setRangeText, createValueRange and stepUp/stepDown now retain and resolve the original control after all authored argument conversions. Native selection/value snapshots and applicability checks use the resulting owner. createValueRange also propagates original conversion exceptions rather than replacing them with allocation errors.

Expanded the two-arena regression with multi-argument round-trip adoption and explicit conversion order, range replacement plus selection outcome, native opaque-range offsets, original thrown-object identity, and numeric step changes in both directions. No execution; these are generated assertions, not conformance results.


### Validation-message and color-space setters

setCustomValidity now resolves its retained original control after message conversion, before applying native validity state. Input colorSpace now shares the tracked DOMString reflection path with colorspace attribute mapping; removed the lossy, source-node-only fast path. Expanded the two-arena regression with colorSpace raw attribute preservation and validation-message set/throw/clear across adoption, including lone-surrogate message identity. Source only; no execution.


## Attribute-node setter conversion ownership — 2026-09-17

Attr value/nodeValue/textContent setters previously reread an arena-local node after authored string conversion, despite their comment promising moved-owner resolution. They now retain the native wrapper and resolve the current attribute before sink-policy lookup, then resolve again after a default Trusted Types policy may run before publishing the accepted string. Post-write resource notification uses the final actual owner. Ordinary DOMString conversion preserves lone UTF-16 surrogates.

Expanded the two-arena source regression for all three setters: owner-element adoption during conversion, stable Attr ownerElement identity, native attribute updates, single conversion, move-back throwing conversion, and distinct null semantics. No execution. The broader Trusted Types policy/adoption matrix remains open; this is not a claim of full S-01 completion.


## Pinned IDL correction: textarea-only textLength

Cross-checking against pinned interfaces/html.idl showed the earlier input textLength addition was incorrect: the standard declares textLength on HTMLTextAreaElement only. Removed the new input declaration and the pre-existing unreachable input getter arm. Renamed the source regression to `control_dimension_reflection_and_textarea_text_length`, now checking textarea UTF-16 length and absence on input. The prior entry describing input textLength is superseded by this correction. No execution; full interface review continues.

### September 17 — Relation-list interface and textarea type reconciliation

Pinned `interfaces/html.idl` and `html/semantics/rellist-feature-detection.html` expose form/anchor/area relList and textarea readonly type. These were absent from generated interface input; anchors/areas also lacked rel. Added their declarations and reused the native associated DOMTokenList wrapper and existing PutForwards setter. Supports now resolves the current native owner after argument conversion and selects navigation tokens (noopener/noreferrer/opener) separately from link resource tokens. Removed link metadata-only false positives. Generated `relation_lists_are_live_and_support_tokens_for_their_element` covers all four element kinds, identity, attribute liveness, forwarded assignment, indexed deduplication, case-insensitive supports, authored conversion/error identity, association separation, and readonly textarea type. No binding generation, build or test execution occurred. Resource feature support qualification remains part of the full loading/rendering gates.

Shared token-list follow-up: `associated_set` used lossy UTF-16 conversion while token operations already used `required_string` with preservation. Replaced that conversion with `rust_string_preserving_utf16`. Generated `associated_token_list_values_preserve_dom_strings_and_conversion_errors` exercises all seven rel/blocking associations, lone surrogates, surrogate pairs, NUL, non-HTML whitespace, direct and PutForwards setters, token add/remove, single conversion, exact thrown object, Symbol rejection without mutation, and null stringification. Source inspected only; no execution.

Token mutation review against pinned `dom/nodes/Element-classlist.html`: both class and associated replace paths now reject either empty argument before whitespace validation, after converting both arguments. Associated token update steps preserve absent attributes when the resulting set is empty, while direct value assignment still creates an empty attribute. Generated `token_list_replacement_validation_and_empty_updates` covers eight associations, MutationObserver records, exception precedence, authored second-conversion failure before validation, ordered replacement/deduplication and absent-token no-op serialization. Source only, unrun.

Class-list consolidation: generated getter/setter/operation dispatch now routes every DOMTokenList through the native association implementation, whose absent marker denotes class. Removed duplicate class getter/setter/operation arms and class token parsing/storage helpers. Class supports still converts its argument then throws TypeError. This eliminates captured pre-conversion DOM/node use from classList item/value/add/remove/replace/contains/toggle. Expanded the existing two-native-arena adoption regression to class, form/link rel, and script blocking, with retained wrapper identity, multiargument order, roundtrip movement, index conversion and authored exception after adoption. All source-only; no execution.

Image-input dimensions: added missing width/height IDL, sharing native image content-box and attribute/natural-dimension fallback. The HTML specification input dimension algorithm explicitly returns zero outside Image Button state (https://html.spec.whatwg.org/multipage/input.html#dom-input-width; read during source review). Input/img setters now perform unsigned-long conversion, retain the original element across conversion, resolve its current owner, and notify native resource change; removed the old img string-parse setter. Regression sources extend native projection checks to image inputs and two-arena conversion tests to both interfaces, including wraparound/fraction/NaN, conversion exceptions, attribute persistence and non-image zero getters. Broader image default sizing for incomplete natural dimensions remains under resource/layout reconciliation; these source checks do not close that gate. No build or test run.

### September 17 — Control attribute declaration inventory including partial interfaces

Reconciled pinned interfaces/html.idl against browser.webidl, including obsolete partial interfaces. Added input align/useMap DOMString reflection with conversion-time ownership resolution and output htmlFor as a native SameObject/PutForwards DOMTokenList associated with the for attribute. Output token support throws TypeError after argument conversion; output is included in string preservation, mutation-record and cross-document regression sources.

| Interface | Pinned attribute names | Missing declaration names |
| --- | ---: | --- |
| HTMLInputElement | 46 | None |
| HTMLTextAreaElement | 23 | None |
| HTMLSelectElement | 17 | None |
| HTMLButtonElement | 16 | None |
| HTMLOptionElement | 8 | None |
| HTMLOptGroupElement | 2 | None |
| HTMLFormElement | 13 | None |
| HTMLFieldSetElement | 8 | None |
| HTMLOutputElement | 10 | None |
| HTMLLabelElement | 3 | None |
| HTMLDataListElement | 1 | None |

This is declaration-name coverage only, not complete overload/type/extended-attribute/constructor or algorithm coverage. Named/indexed form operations and HTML constructor machinery need their independent integration evidence; no execution occurred.

Control setter integration follow-up: fieldset/output name setters now participate in the generic control receiver lease and current-owner resolution after DOMString conversion. Label/script htmlFor now preserves UTF-16, propagates authored exceptions and resolves current ownership after conversion. HTML/SVG tabIndex likewise resolves after int32 conversion and preserves authored exceptions. Expanded two-native-arena regression sources cover roundtrip adoption, fieldset/output names, label/script lone-surrogate targets, tabIndex wraparound and thrown conversion values without mutation. Source only, unrun.

Popover setter review: nullable DOMString conversion now preserves UTF-16 and the exact authored exception, then resolves the original element in its current owning document before mutation. Generated two-arena regression covers manual mode, invalid lone-surrogate raw attribute, conversion-time adoption, thrown conversion without mutation, and null/undefined removal. Inspected select add/remove and option-collection remove: these existing implementations already re-resolve the receiver after their numeric argument conversion; this observation does not certify their full insertion semantics. No execution.

Shadow reference integration review: attachShadow previously retained pre-dictionary-conversion state/node, and referenceTarget setter retained pre-string-conversion state/node. Both now retain the original wrapper and resolve current native ownership after all applicable conversions, before disabled-shadow validation or mutation. Reference target DOMStrings preserve UTF-16 both in initialization and subsequent assignment. Expanded two-arena regression sources cover host adoption during mode conversion, original root identity, target setter movement, lone surrogates, null versus empty target, and throwing referenceTarget initialization without creating a root. Full shadow dictionary/algorithm conformance is still subject to the broader source gate and later execution.

### Shadow serialization dependency — generation in progress

Pinned dom.idl requires ShadowRootInit.serializable and ShadowRoot.serializable. Added native shadow_serializable storage/default, clone/import copies, declarative shadowrootserializable parsing, readonly binding and lexicographically ordered dictionary conversion. Generated flag/default/clone/getter/exception-order regression sources. Full getHTML serialization options, explicit root inclusion, recursive declarative template serialization and associated integration/regressions remain open; these foundational sources do not establish serialization completion. No execution.

Shadow serialization native algorithm generated: HtmlSerializationOptions selects serializable and/or explicit arena-qualified roots; existing serializer delegates to empty options preserving ordinary inner/outer behavior. Explicit-stack traversal can serialize only root children and emits selected shadow templates before light children with mode/delegates-focus/serializable/clonable/reference-target attributes. Native regression sources cover closed roots, flag ordering, escaping, default exclusion, explicit inclusion and direct shadow-root children. JS getHTML dictionary conversion, template resolver traversal through shadow roots, registry serialization details and complete pinned case matrix remain in generation. No execution.

getHTML integration generated: Element/ShadowRoot IDL and operation routes use a shared native serializer; dictionary reads serializableShadowRoots then shadowRoots, consumes the iterable with the existing IteratorClose helper and validates native ShadowRoot brands. Retained V8 wrappers are re-resolved after complete iteration to handle adoption of already-converted roots and receiver. Template resolver now optionally traverses shadow subtrees. Regression sources cover default/null options, closed and explicit roots, deduplication, template content, getter ordering, invalid iterable members/IteratorClose, primitive/null-sequence rejection and original exceptions. Remaining full matrix includes nested roots, cross-arena iterator adoption and scoped registry serialization details; source generation is not execution evidence.

Scoped/nested serialization integration: native DOM exposes binding-installed global-registry classification to serialization resolvers; selected shadow templates append shadowrootcustomelementregistry after reference target for scoped or explicit-null registries, preserving omission for a global registry. Generated regression sources cover all three registry modes, selected nested open/closed roots, inability to reach a selected descendant through an excluded outer root, combined serializable/explicit selection and host adoption after a sequence yields its root. Sources were inspected against pinned reference-target/gethtml-serialization.html and declarative/gethtml.html; no execution. Full schema/serialization source gate remains open pending remaining fixture reconciliation.

Pinned fixture reconciliation: `shadow-dom/declarative/gethtml.html` invokes missing Element/ShadowRoot.setHTMLUnsafe for its declarative cases. Pinned html.idl includes SetHTMLUnsafeOptions sanitizer union plus runScripts, and ParseHTMLUnsafeOptions sanitizer. These require the real parser/sanitizer/TrustedHTML integration, not an innerHTML alias or ignored options. Added imperative serialization regression matrix across all eighteen allowed host names, both modes, delegatesFocus/serializable/clonable flags, empty/populated trees, and Element/ShadowRoot receivers. Declarative fixture completion remains open until unsafe parsing entry points and all option semantics are generated. No runtime checks have run.

Unsafe parser source reconciliation correction: Document.parseHTMLUnsafe already exists as `document_parse_html_unsafe_static`, manually installed on Document, and uses TrustedHTML plus inert declarative document parsing. Earlier wording calling the entire static entry point missing was incorrect; its sanitizer option conversion/application remains absent. Element/ShadowRoot.setHTMLUnsafe are absent. Added explicit `parse_fragment_with_shadow_context` entry point preserving ordinary fragment parsing with declarative mode false; opted-in callers may use full namespace/context attributes/form ancestry. Generated nested closed/open nonclonable declarative fragment import regression and serializable flag checks. Sanitizer union/config processing, unsafe setter integration and runScripts remain required generation; no execution.

Sanitizer generation started in `oryn-dom/src/sanitizer.rs`: native namespace-qualified names normalize empty namespace to null without changing name case; Config preserves dictionary field presence and list order. Validation implements mutually exclusive present lists, duplicates, replacement-list conflicts/non-replaceable roots, processing-instruction target uniqueness, per-element attribute override constraints and data-attribute conflicts from pinned sanitizer-config.html. Generated native regression sources cover namespace distinctions, invalid configuration immutability, defaults-presence conflicts and representative list constraints. This model is not yet connected to JS parsing sinks: default policy/canonicalization, modifiers, filtering, bindings and complete regression matrix remain required. No execution.

Sanitizer canonicalization generated: Config::canonicalize fills omitted element/attribute removal lists, chooses processing-instruction allow/removal defaults by permissive mode, fills per-element removal lists and missing comments/dataAttributes, then validates the full candidate. Explicit values remain intact. Regression sources distinguish unsafe empty configuration from explicit empty attribute allowlist, restrictive defaults, explicit overrides and invalid dataAttributes/removal combinations. Algorithm source reviewed at https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html#canonicalize-the-configuration alongside pinned sanitizer-config.html and sanitizer-boolean-defaults.html. Newer online javascriptURLs schema field is absent from pinned IDL and is not used to change the pinned contract. Built-in preset, modifiers/filtering and binding/sink integration remain open; no execution.

Native sanitizer tree filtering generated: iterative traversal prunes removed/not-allowed elements, sanitizes then unwraps configured elements without promoting private shadow children, applies global/per-element namespace-qualified attribute policy, filters comments/processing instructions and visits template and retained shadow contents. It validates configuration before any mutation and is explicitly limited to inert native trees until sink integration provides publication custody. Generated pruning/unwrapping/attribute/template/shadow/PI regression sources. Safe baseline, URL/SVG script-capable attribute rules, preset/modifier/binding integration remain open; no execution.

Default sanitizer preset generated as native policy data in sanitizer/default_policy.rs using pinned sanitizer-default-config.html normative expectations (121 element rules and 58 global attributes; WPT revision and source SHA-256 recorded in the file). Config::default_preset exposes this policy separately from the explicitly permissive empty configuration. Added regression sources for policy validity, HTML/SVG attribute retention and script/event-handler/comment removal. This default allowlist is not complete safe-sink behavior by itself: JavaScript URL/SVG animation rules, removeUnsafe baseline, modifiers and sink/binding integration remain required. No execution.

Sanitizer modifier generation: setComments, allow/removeProcessingInstruction, removeElement and replaceElementWithChildren now operate on native canonical lists, preserving allow/removal mode, insertion order, and actual-change boolean results. Non-replaceable HTML/SVG/Math roots remain untouched; element removal clears replacement rules. Regression sources cover repeat no-ops, transitions between replacement/removal, namespace-specific constraints and post-operation validity. Remaining modifier algorithms include allowElement and attribute/data-attribute policy reconciliation, plus removeUnsafe baseline and JavaScript wrappers. No execution.

Sanitizer modifier continuation: allowAttribute/removeAttribute/setDataAttributes reconcile global and per-element lists; allowElement canonicalizes and deduplicates overrides, removes globally covered/forbidden entries, intersects local removals with global allows, preserves removal-list mode and moves changed element rules to the end. Generated regression sources cover repeat no-ops, local-to-global promotion, removals, data-attribute pruning, ordering, invalid removal-mode overrides and resulting validity. removeUnsafe/safe URL baseline and native JavaScript wrappers remain open; no execution.

### Native sanitizer safety generation — 2026-09-17

- Generated pinned `removeUnsafe` baseline (six HTML and two SVG qualified element names), preserving configuration list mode and reporting actual changes. Reused existing native modifier operations for local/global attribute reconciliation.
- Moved the existing Trusted Types handler names to `oryn_dom::event_handlers`; sanitization and attribute sink classification consume the same native inventory.
- Added safe tree filtering through a copied configuration, URL-standard parsing of navigating attributes, and exact SVG animation attribute rules. Ordinary unsafe configuration filtering retains URLs. Template and shadow trees share the native traversal.
- Added regression sources for baseline ordering/idempotence, namespace distinctions, URL controls/case/invalid schemes, SVG animation exclusions, nested template/shadow content, and preservation of caller configuration.
- Added pinned Sanitizer, dictionaries, union aliases and HTML sink declarations to browser.webidl. JS instance/conversion/operation bindings and HTML sink integration remain **open**; these declarations do not establish API implementation. No project generator, build or tests executed.

### Sanitizer binding generation — 2026-09-17

- Added native Sanitizer records, platform interface classification, constructor dispatch, get serialization and modifier operation dispatch. Configuration snapshots serialize with own data properties and fresh nested lists.
- Added complete dictionary conversion in inherited/member order and native iterator conversion, preserving original exceptions through IteratorClose. Native policy borrows occur after author conversion, retaining reentrant edits.
- Regression sources cover member access order before validation, nested IteratorClose, original getter exception identity, namespace/presence/UTF-16 conversion, configuration copy isolation, brand-before-conversion, reentrant edits, subclassing and poisoned prototype setters.
- Source review fixed URL parsing/data-prefix matching on the internal lossless UTF-16 encoding; added lone-surrogate safety regression sources.
- HTML sinks, options carrying Sanitizer instances across realms, runScripts integration and complete caller review remain open. Generated browser bindings have not been regenerated: all build/generator/test execution remains deferred to the global generation gate.

### Document HTML sink and options generation — 2026-09-17

- Added native Sanitizer instance union acquisition through receiver ownership, including foreign realms and proxy dictionary fallback. Options preserve live instance state until the sink algorithm acquires configuration; safe/unsafe defaults are distinct.
- Added Document.parseHTML and integrated options/sanitization into Document.parseHTMLUnsafe. HTML argument conversion precedes options; Trusted Types policy evaluation follows options. Parsing and filtering occur in an inert native arena before import.
- Removed the previous unsanitized Document static parser path; the remaining DOMParser helper is explicitly separate and retains its inherited URL/no-declarative behavior.
- Regression sources cover cross-realm live instances, option getter order, deferred validation, ignored runScripts on document parsing, safe/unsafe/custom policies, shadow filtering, script inertness and static arity.
- Element/ShadowRoot setHTML/setHTMLUnsafe, runScripts execution and full review remain open. No build, project generation or test execution.

### Sanitized fragment preparation generation — 2026-09-17

- Added native fragment context snapshots preserving namespace, attributes and form ancestry, independent of the destination arena for adopted template-content ownership.
- Added off-tree declarative fragment parsing/filtering and complete import before native ChildrenReplaced publication; existing host shadow roots remain attached.
- Added regression sources for table/SVG/form context, qualified safe-script no-op recognition, preserving old children on invalid configuration and preserving existing closed shadow roots.
- Setter bindings, script flags/execution custody and complete declarative top-level context review remain open. No execution.

### Element/ShadowRoot sink binding generation — 2026-09-17

- Wired setHTML/setHTMLUnsafe to native receiver re-resolution after conversions, contextual off-tree filtering, template-content targeting and native replacement. Safe script receivers return before configuration validation.
- Added cross-owner fragment registry initialization; runScripts clears parser-started state only in ordinary/shadow descendants and routes insertion through the existing retained post-connection script queue. Template contents retain their independent inertness.
- Added regression sources for both receiver types, safe filtering, policy failure preserving children, script inertness after moves, synchronous inline/declarative-shadow scripts, template inertness and conversion order.
- Full script-mode semantics, top-level declarative parser context, cross-arena adoption and caller reconciliation remain open. No build/generator/test execution.

### Top-level declarative context and adoption review — 2026-09-17

- Source inspection of pinned html5ever confirms a top-level declarative template attaches to the detached context handle, not the synthetic fragment root. Added a retained-context parser entry and transfer of that shadow tree into the filtered/imported fragment.
- Context snapshots prevent new attachment when the receiver already has a shadow root or disables shadow attachment. Failed declarative attachment retains the template fallback.
- Preserve explicitly initialized/null custom-element registry associations when propagating the receiver registry to fragment roots.
- Regression sources cover top-level closed serializable root filtering, existing-root fallback, receiver adoption during string conversion, template-content identity during options conversion and shadow-root owner changes. Full script/parser/registry qualification remains open; no execution.

### Fragment script-state review — 2026-09-17

- Found and implemented the missing native TreeSink::mark_script_already_started hook; html5ever's fragment/EOF flags previously used the trait no-op default. runScripts clears fragment script state for ordinary/shadow descendants, while template content remains inert.
- Added regression sources for native parser flags in ordinary/template/shadow trees, extracting a template script after runScripts insertion, whole-tree visibility before inline execution, removing a later script, sanitizer precedence and custom-element reaction order.
- Configuration review also found a specification consistency question: setDataAttributes(true) removes global data allow entries but retains local data removals, whereas the documented validity algorithm requires local removals to be a subset of the global allowlist. This edge remains explicitly unresolved; no invented normalization is claimed conformant.
- Full source review and execution gates remain open. No builds, generators or tests run.

### Fragment transport and Trusted Types regression generation — 2026-09-17

- Added native scheduler-admission regression sources for classic external, shadow external and inline module scripts; inert/filtered/template scripts produce no jobs, admitted metadata is captured before later changes, and reconnecting does not readmit started scripts.
- Added Trusted Types regression sources for options-before-rejection, safe sinks without TrustedHTML requirements, html/options/policy order, live Sanitizer mutation by a default policy and explicit TrustedHTML bypass.
- These sources cover additional production callers but are unexecuted. Complete caller mapping, configuration edge reconciliation, full build/test/Core/family/qualification gates remain open.

### HTML static surface and shadow script ordering — 2026-09-17

- Set explicit native function names and non-constructor behavior for manually installed Document.parseHTML/parseHTMLUnsafe. Added name/arity/non-construction regression sources.
- Retained context shadow trees now enter the prepared fragment before light children, matching shadow-including post-connection traversal. Added a top-level shadow/light script-order regression source.
- All changes remain source-only; package/parent/conformance gates are open.

### Declarative shadow mutation publication — 2026-09-17

- Native replacement previously grouped every imported node into ChildrenReplaced.added_nodes, including an attached ShadowRoot. The shared replacement primitive now filters shadow roots from the public child-list payload while retaining native insertion/ownership mutations.
- Added MutationObserver regression sources for a shadow-only insertion (no host childList record) and mixed shadow/light insertion (one light-only record).
- Failure/retry custody after replacement and the rest of the full sink matrix remain explicit open source work. No execution.

### Setter retry custody review — 2026-09-17

- Added post-connection fault-injection regression sources specifically through setHTMLUnsafe: script preparation retry preserves imported node identity, does not repeat HTML/options conversion or replacement, and drops disconnected old scripts after a newer setter replaces the content.
- Confirmed a separate open gap before script-batch admission: prepare_dynamic_script_subtrees performs frame/image/stylesheet/modulepreload preparation before creating the retained post-connection script batch. A failure there can leave later resource/script preparation without retained continuation. This needs a native staged insertion-preparation batch with exact root ownership and retirement handling; script-body retry tests do not close it.
- No execution; the full source generation gate remains open.

### Retained insertion resource preparation — 2026-09-17

- Replaced the one-shot prepare_dynamic_script_subtrees body with shared retained batch admission. Native tracked resource roots and script/meta targets are captured before any preparation starts.
- Resource cursors advance through frame, image, stylesheet and native preload preparation only after success. All resources finish before the lifecycle checkpoint and script suffix. Retry retains the current resource stage; current-owner resolution resets preparation after document adoption.
- Readiness suppresses suspended resource roots; retirement removes both resource and script work. Jobs reject original document generation/stream-epoch invalidation. Combined resource/script admission remains bounded.
- Added source regressions for failures at multiple resource phases, no early script, preserved DOM, suspension readiness and retirement cleanup. Existing script-body/HTML setter retry regression sources remain in the same shared module.
- Per-producer partial admission, capacity failure before batch admission and full caller review remain open. No execution.

### Partial image admission reconciliation — 2026-09-17

- Found image_sources published before fallible poster reset/rooting/native host submission. Key equality then suppressed retries of a request never accepted by the host. Source keys now commit after successful native operation admission, alongside generation identity.
- Moved fallible poster reset ahead of image preparation-error key/generation publication as well.
- Added a retained-batch regression source with three images: first accepted, second rejected once, third pending. Retry preserves the first admission, admits only the remaining requests, and runs the following script once after preparation completes.
- Frame/stylesheet stage loops use existing native prepared-node deduplication; general preload caller integration remains with the loading agent. Capacity-before-batch admission and full source qualification remain open. No execution.

### Sanitizer configuration snapshot ordering — September 17

Source comparison confirmed get() requires sorted copies of every returned list. Added native namespace-first/null-first then UTF-16 local-name sorting, including per-element attributes and processing-instruction targets, and wired native snapshots into the binding. Generated native and binding regression sources cover all list modes, empty namespace normalization, astral and isolated-surrogate ordering, deep-copy isolation and reentrant modifier expectations. No build or tests run; the broader sanitizer matrix and dataAttributes specification reconciliation remain open.

### HTML setter preparation capacity — September 17

Added detached-fragment capacity preflight before connected HTML setter replacement. The check uses the same resource-root plus script/meta budget as retained preparation and includes existing admitted work. It skips detached or inactive Document targets, reserves batch queue space and runs after all author conversions. Native registry initialization before preparation invokes no author callbacks. Generated a regression for over-capacity old-child identity preservation, no early script execution, admission cleanup, a later successful setter and detached insertion. Source only; target-lease allocation failure and generic DOM insertion pre-mutation custody remain open.

### Shared insertion admission preflight — September 17

Extended the root/target capacity preflight into node_migration::insertion_argument after native hierarchy checks and before source adoption. appendChild, insertBefore, replaceChild and adjacent-element insertion now reject oversized preparation work before moving the source or changing the destination. Generated a two-realm regression retaining fragment children, original ownerDocument and destination children on rejection, checking hierarchy-error precedence and subsequent admission. Variadic fragment assembly and direct markup mutation callers, and allocation custody beyond capacity, remain open. No execution.

### Variadic insertion capacity boundary — September 17

ParentNode/ChildNode append/prepend/replaceChildren/before/after/replaceWith now check preparation capacity after required argument-fragment staging and destination hierarchy validation, before final insertion/replacement (and before foreign single-node adoption). The staging removals are specified effects and are intentionally preserved on rejection. Generated all-six-method regression sources check conversion once, retained staging ownership, unchanged destination, and later admission. Direct markup mutation and fallible target capture still need source reconciliation. No execution.

### insertAdjacentHTML contextual insertion — September 17

Replaced the div-only parse/import loop with native contextual fragment parsing, synthetic-root removal, one staged insertion, whole-batch resource preparation and custom-element initialization. The receiver/insertion point is resolved after markup conversion; Document parents reject before mutation. Legacy parsing disables declarative shadow roots and retains inert script flags. Shared capacity preflight runs before insertion. Generated table/SVG context, mutation grouping, inert reconnect, declarative-template preservation, conversion-time movement and forbidden Document-parent regression sources. No execution. innerHTML/outerHTML and target-allocation custody remain under source review.

### Legacy innerHTML/outerHTML native integration — September 17

Both binding setters now delegate to contextual inert fragment preparation after Trusted Types/string conversion and receiver re-resolution. innerHTML follows template-content ownership and ShadowRoot host context; outerHTML handles detached no-op and forbidden Document parent, stages imports and publishes one replacement. Both preflight capacity before mutation, preserve inert scripts/non-declarative templates, initialize native custom registries and share retained resource preparation. Removed the old binding mutation branches. Generated SVG, template identity, null clearing, shadow, conversion movement, Document rejection and injected resource-stage retry/identity regression sources. All unrun. Full XML fragment parsing, target allocation and broader caller audit remain open.

### XML legacy fragment parsing — September 17

Added native XML fragment preparation for innerHTML/outerHTML/insertAdjacentHTML when the context belongs to an XML Document. It captures in-scope default/prefixed namespaces, parses behind a detached wrapper, rejects malformed XML (including isolated UTF-16 surrogates), removes the wrapper from insertion and retains inert script flags. Syntax errors are raised before destination mutation. Generated prefix-shadowing/default-reset, namespaced attributes, CDATA/PI/comment, malformed/duplicate-attribute/unbound-prefix, outer and adjacent regression sources. Explicit HTML APIs retain their HTML parser. No execution. DTD/entity inheritance and the full pinned XML helper matrix remain open.

### XML markup getter source reconciliation — September 17

Pinned domparsing/innerhtml-01.xhtml and innerhtml-03.xhtml exposed HTML-only legacy getter serialization. Getters now choose HTML/XML by native owner Document. Strict XML output validates local-name colon restrictions and serialized XML lexical/namespace validity and returns InvalidStateError; XMLSerializer retains its separate non-strict contract. Generated namespace/void/raw-text and invalid-name/character regression sources, including outerHTML. No tests run. Full pinned matrix and namespace/DTD edge review remain open.

### XML fragment boundary/source checks — September 17

Inspected pinned innerhtml-05.xhtml and the xml 1.4.0 EOF implementation: progressive-input mode still reports malformed final XML, so fragment parsing does not treat premature EOF as success. Added native regression sources for context/synthetic-wrapper breakout strings, unbound prefixes, forbidden fragment doctypes, unchanged context and mixed XML character-node output. Namespace attribute values now decode the native lossless DOMString representation and reject isolated surrogates instead of serializing internal storage escapes. No execution.

### Range native fragment consolidation — September 17

Removed Range’s duplicate XML namespace-wrapper implementation and routed both HTML/XML parsing through the shared native fragment parsers. Preserved the missing/html-root context HTML-body fallback for XML parsing, scoped registry upgrade and post-conversion live boundary lookup. Range scripts explicitly clear both already-started and parser-document flags before insertion. Extended regression sources for isolated surrogate rejection, XML Document-boundary fallback and execution on insertion. Reconciled six bounded checklist rows and package counts; all parent gates remain open. No execution.

### XML doctype literal source reconciliation — September 17

Pinned domparsing/xml-serialization.xhtml requires omitting an empty system literal after a nonempty public ID. Corrected the shared native XML serializer and generated empty/public-only/system-only/both/quotation-mark regression sources; non-strict XMLSerializer continues preserving literal quote characters. Also corrected the namespace argument type in the newly generated fragment regression during source review. No execution.

### Default XML namespace serialization — September 17

Pinned XMLSerializer-serializeToString.html exposed redundant/inconsistent default namespace output. Native unprefixed element namespaces now determine default binding changes; redundant authored defaults are omitted and null-namespace attributes named xmlns are not emitted as conflicting declarations. Generated pinned default reset/redundancy/inconsistent declaration cases and a native attribute-preservation check. Prefixed namespace selection/order remains under review; no execution.

### Prefixed namespace selection and strict source records — September 17

Native XML namespace scope now retains declaration order and selects the nearest valid existing prefix before inventing a binding. Element tags drop their prefix when their namespace equals the inherited default and reuse available prefixes otherwise. Generated nearest-prefix, inherited-default and start-tag regression sources. Independent source review identified strict getter validation after normalization could conceal invalid original XMLNS records; added checks before serialization and negative regression sources. Declaration emission order and remaining pinned prefix collision behaviors remain open. No execution.

### XML namespace declaration emission order — September 17

Replaced sorted namespace-prelude emission with attribute-order emission. Authored namespace declarations remain in their native attribute positions; generated element declarations precede attributes, and generated attribute declarations appear immediately before their first use. Generated regression sources cover later conflicting declarations, mixed ordinary/declaration ordering and monotonically numbered sibling attribute bindings. Updated the older collision regression expectation to preserve authored order. Independent review also delivered Range HTML-container flattening and XML outerHTML fragment-parent body fallback. No execution; remaining prefix collision/spec discrepancies remain open.

### Pinned XML prefix collision algorithm — September 17

Read the public DOM Parsing specification (§5.2.1.1 prefix retrieval/generation; issue 44) after source tests exposed unusual expectations. Replaced current-prefix-only lookup with namespace-to-prefix alias lists retaining earlier bindings, and generated nsN names now advance without collision skipping as specified/pinned. Added non-strict alias-rebinding and duplicate-generated-prefix regression sources. Strict serialization separately rejects malformed output and original forbidden records; added original PI target and null-namespace xmlns checks. The current specification and some pinned tests still disagree on unused authored attribute prefix selection and doctype quote selection; these are recorded as unresolved source reconciliation, not conformance success. No execution.

### Pre-mutation insertion batch capture — September 17

Added PreparedInsertion ownership of native target leases, captured resource/script jobs and queue admission. appendChild/insertBefore/replaceChild capture before their final native mutation and finish the same retained batch afterward; mutation failure drops reservations without starting work. Existing post-mutation callers share the extracted capture machinery. Generated injected partial-capture failure regression sources preserve source fragment/destination identity, no script execution, no retained admission and successful retry. Cross-arena adoption before capture and remaining markup/variadic caller migration remain open. No execution.

### Variadic/adjacent pre-mutation capture — September 17

Migrated all six ParentNode/ChildNode variadic methods and insertAdjacentElement to retained batch capture before destination mutation. Variadic staging remains prior to this boundary as specified. Adjacent element/text operations now resolve receiver identity and insertion point after conversions, and valid outside positions on detached receivers return null/no-op. Generated all-six-method partial-capture/staging regression sources and adjacent conversion/detached/error-precedence cases. Cross-arena adoption-before-capture and markup caller migration remain open. No execution.

### HTML setter pre-mutation capture and shadow registry — September 17

Split native fragment import/Document initialization from final child replacement. Safe/unsafe and legacy inner/outer setters now initialize detached imports and capture retained jobs/leases before destination mutation. Generated seven-setter injected capture-failure sources preserve existing light children, shadow root and shadow children with no admitted residue. Independent review found parsing-host context was also incorrectly used as registry context; setters now initialize shadow children from the actual ShadowRoot registry while retaining host parsing context. Agent supplies dedicated scoped/null-registry regressions. No execution; final native mutation allocation and cross-arena adoption custody remain open.

### Adjacent HTML retained capture and registry regression registration — September 17

insertAdjacentHTML now initializes its detached staging children and captures the retained batch before final insertion, then finishes that captured batch. Added all-four-position capture-failure regression sources checking conversion once and unchanged destination. Registered the independently generated sanitizer_registry_tests module covering scoped/global conflicts, explicit-null ShadowRoot registries and delayed-definition tree order. No execution; caller/source and native mutation allocation review remain open.

### Resource custody after partial native mutation error — September 17

PreparedInsertion::finish_mutation now preserves captured work for roots still connected when a native mutation returns an error, without running author code at the error boundary. Detached remainder is discarded and original error propagates. Migrated generic, variadic, adjacent and setter mutation-result callers. Generated partial-native-mutation regression sources assert one connected root/job retained, detached suffix untouched, delayed execution once and admission release. This closes preparation loss for the connected prefix, not native structural mutation atomicity or resumable DOM insertion. Those remain explicit open obligations. No execution.

### Native deep connection traversal — September 17

Source review of mutation failure paths found recursive mark_connected/mark_disconnected traversal. Replaced both with ordered iterative walks preserving child-before-hosted-document notification order, focus/top-layer handling and state-preserving frame generation behavior, with cycle detection. Generated 20,000-descendant connection/disconnection and native-cycle regression sources. Native root retain now checked-adds and reports MigrationCapacity without wrapping accounting; generated overflow regression source. These are bounded lifecycle improvements, not a full mutation atomicity proof. No execution.

### Native connectivity graph follow-up — September 17

Source review extended the iterative connection/disconnection work with regression sources for ordinary iframe children before hosted Documents, descendant-before-host notification order, nested hosted connectivity, preserved focus/frame generations during preserving moves, and focus clearing/generation advancement during ordinary disconnection. Hosted Documents retain separate non-DOM ancestry.

`rebuild_document_indexes` now collects and validates the complete reachable connectivity set before publishing it, rejecting repeated retention edges instead of looping forever on a malformed parsed/snapshot graph. Added a cyclic-graph regression preserving the previous index and verifying subsequent repair admission. These are generated sources only; no build/test execution and no whole S-01 or runtime gate closure. Full mutation structural atomicity remains separate open work.

### Document hit-test argument failures — September 17

Document.elementFromPoint/elementsFromPoint now propagate each numeric conversion failure immediately instead of replacing it with NaN and attempting the next conversion. Pinned cssom-view.idl declares restricted double arguments and elementFromPoint-parameters.html requires TypeError for nonfinite inputs. Each argument now rejects NaN/infinity immediately, before converting the next argument; the receiver is resolved after successful conversions. Added source regression coverage for first/second callback exceptions, Symbol/BigInt failures, once-only conversion order and first/second nonfinite rejection. No execution; native geometry correctness remains separately unverified.
