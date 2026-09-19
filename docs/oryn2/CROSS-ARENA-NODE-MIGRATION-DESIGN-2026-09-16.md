# Cross-arena DOM migration: integration design

Source inspection only; no runtime execution. This identifies a concrete production dependency for S-01.f. No foreign Node is substituted with a clone for adoption.

## Existing ownership that must move together

- `DomRuntime` owns `DomArena`, rooted/connected sets, native ranges and boundary journal, opaque control ranges, slot assignment state, form/control state, frame identities, and mutation journal. A `NodeId` is meaningful only in that arena.
- `WrapperRegistry` (`oryn-js/src/bindings.rs`) keys wrappers by `(ArenaId, RealmId, NodeId)`. Generated V8 wrappers additionally carry wrapper lease, slot/generation, realm and document generation in internal fields.
- `V8BindingState.dom` is an immutable shared runtime pointer for a realm. Its maps separately own wrappers, event listeners, handlers, mutation observers, Range wrappers, live collections, custom-element registrations/reactions and resource queues. Callback function realm and node owner are distinct facts.
- `argument_node` remains arena-local for native operations; the generated migration bridge resolves a foreign argument before adoption. Earlier design baseline: it permitted only the caller's arena. `Document.adoptNode`, `importNode` and ordinary mutation callers therefore reject valid Nodes from independent same-origin root arenas. The new comparison/serialization helpers correctly resolve other arenas only for read operations.

## Required bridge contract

A migration transaction must accept a source native wrapper identity, destination Document identity and exact expected source/destination arena generations. Admission rejects Document and ShadowRoot adoption, cycles and foreign authority before changing state. It retains source root and destination document leases across removal and publication.

1. Run source removal/adoption algorithms first: detach Attr from its owner or ordinary node from its parent, update source ranges/iterator positions, retain transient observer registrations, and publish source removal records. These are observable standard side effects, not speculative clone work.
2. Extract/move the original Node records and native subtree state into destination storage. Return a total `old NodeRef -> new NodeRef` mapping including attributes, template-content roots and shadow roots. Hosted browsing-context resources need their exact reparent/retirement hooks; do not copy another Document as a child. Allocation planning must precede extraction and failed publication must retain retry custody.
3. Preserve existing V8 object identities. Remap each affected wrapper registry entry and embedded Node identity in place, including wrappers in other realms. Transfer the native root lease exactly once; a queued old weak-finalizer must not release the replacement node. The wrapper's JS prototype/private fields stay intact.
4. Separate callback ownership from object authority. Existing listeners/functions stay in their creation realms while dispatch resolves the moved node in its new arena. Associated NodeList/NamedNodeMap/DOMTokenList/Range/ElementInternals and observer targets must follow the remap without selecting unrelated equal numeric IDs.
5. Transfer or rebase native slot/form/resource associations, custom-element adopted callbacks, script/image/style queues and native frame custody under exact new document identity. Source mutation observations stay available. New target insertion records occur only after adoption commits.
6. Use the bridge from explicit adoptNode and every implicit insertion adoption. `importNode` has different semantics: it creates new identities and can use a deliberately cloning import path, but that cannot implement adoptNode.

## Shared-source coordination and generation order

The necessary shared bridge spans `DomRuntime`/`DomArena`, `WrapperRegistry`, `BindingStateRegistry`, per-object owner lookup and generated mutation calls. A change solely to `argument_node` or `import_subtree` is insufficient and unsafe. Before editing these ownership seams, coordinate the actor/realm owner with root and S/V's live-parser/resource ownership. Parent/Child fragment staging is independently integrated and does not require fabricated foreign authority.

Recommended implementation decomposition within the single generation batch:

- Native move transaction and total remap with explicit retained-failure custody.
- Wrapper remap and associated-object node-owner lookup, preserving source callback realm.
- Native state/listener/range/observer/CE/resource consumers of the remap.
- Explicit adopt/import and all implicit mutation caller integration.
- Source regression matrix: held JS identity and expandos/private fields; listeners/once handlers; live collections; Attr and template/shadow descendants; source ranges/iterators and transient observers; adopted callbacks; queued image/script/frame resources; repeated reverse adoption; failed allocation/publication/closed owner; numeric-ID collision; cross-origin rejection.

All steps must be generated before execution under the existing whole-plan gate. This document does not mark migration complete or waive the requirement.


## Implementation checkpoint

The transaction and primary callers described above are now source-generated in `v8_bindings/node_migration.rs`, `template_links.rs`, `range_insertion.rs`, `node_lists`, `static_range_custody`, native `node_migration` and their production dispatch sites. Detailed checked source cells and exact remaining audit cells are maintained in [S01-BINDING-COVERAGE-MAP-2026-09-16.md](S01-BINDING-COVERAGE-MAP-2026-09-16.md). Retained staged publication replaces fallible one-shot post-commit cleanup: it owns the original pins/objects until wrapper and dependent-state publication succeeds. This is source status, not a claim of completed whole-group behavior or passing conformance.
