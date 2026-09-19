# document.open: native capability lifetime and parser input lifetime

Source design and implementation evidence only. No tests/builds were run.

## Evidence

Pinned WPT checkout inspected: `oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0` under `/Users/rohit/Work/oryn2`.

The following files are under `html/webappapis/dynamic-markup-insertion/opening-the-input-stream/`:

- `tasks.window.js` explicitly says an older rule removing all document tasks must no longer execute. It asserts survival of timeouts, Window messages queued before and after open, canvas.toBlob callbacks, MessagePort messages, and promise rejection reporting.
- `no-new-global.window.js` asserts global variables, object identities, constructors and prototypes survive.
- `event-listeners.window.js` removes listeners from the target Document tree and its Window. This does not remove listeners on unrelated platform objects such as MessagePorts (the queued MessagePort task test depends on its listener surviving).

These tests directly establish that replacing the parser input is not realm/document retirement. Worker, service-client, storage, socket and physical-memory survival follows the same native lifetime split: document.open does not perform a navigation/global replacement or unload those capabilities. The inspected WPT task file does not individually test every such capability; integrated regression execution remains necessary.

## Required ownership split

| Identity | Changes on document.open | Used for |
|---|---|---|
| Document generation + stable `document_authority_epoch` | No | Native Creator; worker/service client; storage/IDB transaction; HTTP/WebSocket; MessagePort/BroadcastChannel; shared mapping; activation/platform capability; remote Window wrapper and native history identity |
| Document generation + `document_stream_epoch` | Yes | Parser input, prepared scripts/styles/images, removed-DOM target freshness, load readiness, parser-owned error/completion callbacks, automation/AX/paint/input snapshots |
| Native response security publication | No | Origin/CSP/isolation authority; replacement navigation still commits a new native security publication |

Creator wire `stream_epoch` remains the historical field name but now carries the stable authority epoch for production native callers. It is not the mutable parser input epoch. Existing pure native fixture owners may retain explicit nonzero epochs.

## Generated changes

- [x] `V8BindingState` has stable `document_authority_epoch`, initialized/reset on actual document generation construction/commit, not input-stream replacement.
- [x] `advance_document_stream_epoch` no longer sends RetireRootDocument/RegisterRootDocument. Parser/input counters and parser-owned queue cancellation remain separate.
- [x] `workers::native_creator`, native shared-wait authority, and real execution retirement use stable authority. Storage/network/media/service producers using native Creator inherit it.
- [x] Native frame message publication and remote Window wrapper identity use stable authority. An existing remote wrapper does not become stale merely because its owner called document.open.
- [x] Source regression `input_stream_replacement_preserves_native_creator_and_queued_port_capabilities` checks unchanged Creator and native queued port delivery while the parser stream changes; actual execution retirement still rejects the Creator.
- [x] Root integration: history Register/Park/Restore and frame inheritance/commit host callsites use the new getter. Root added queued global task and pending WebDriver callback survival regression sources.
- [x] Service Clients.focus/navigate/openWindow uses AutomationRealmSnapshot.document_authority_epoch for native owner matching and registration, while command target stream_epoch retains DOM freshness. Native inventory populates both; all source struct literals were reconciled.
- [x] Native BroadcastChannel regression document_open_preserves_broadcast_subscription_and_object_listener checks pre/post-open messages and retained object listener across full open/write/close.
- [ ] Execute pinned tasks/no-new-global/event-listeners WPT plus native worker/storage/socket continuation and real document-replacement denial controls after the global generation gate.

## Caller audit guidance

Keep `rust_executable::ParserDocumentIdentity`, `ParserFetchContext.stream_epoch`, prepared script dispatch and parser/load continuation comparisons on `document_stream_epoch`. `ParserFetchContext.resource_initiator` already obtains native_script_creator, which carries stable capability authority independently.

Keep host automation_realm_identity, AX/composition/input/document paint checks on parser/DOM stream freshness. They operate on replaced tree targets. Change host native history and frame-policy bindings to stable authority (root owns these sites).

Existing regression fixtures that explicitly call RetireRootDocument still model actual authority retirement; they should not be globally weakened. document.open-specific expectations must instead stop expecting retirement or lost queued platform tasks.

## Additional root caller reconciliation

- WebDriver cached Window and node references retain stable document authority; DOM-bound command snapshots remain stream-fresh. Node references still require native connectivity and correct document ownership.
- `BrowserContext::webdriver_permission_origin` now validates the snapshot's document_authority_epoch, not its parser epoch. Regression source `permission_origin_uses_document_authority_after_parser_stream_replacement` accepts differing parser epochs but rejects wrong authority/generation.
- All entries remain source-only; no execution qualification has occurred.

## Listener-removal regression breadth

Source review confirmed clear_document_stream_listeners uses native document descendants/owner filtering, removes only the Window global platform target, and dispatch checks current registration identity before invoking a snapshotted listener. Added `document_open_clears_tree_listeners_immediately_but_preserves_detached_targets` for immediate same-dispatch removal, nested closed/open shadow listeners, Window listeners and a pre-detached node listener. Complements persistent MessagePort/Broadcast/task survival regressions. Source only; execution remains deferred.
