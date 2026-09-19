# Related auxiliary roots: shared window-agent integration

## Decision and status

P-04 needs a **single thread-affine window agent owning one V8 isolate and multiple independent top-level document drivers**. Sharing only the child process/IPC session cannot provide synchronous same-origin Window access. This document is a read-only source audit and concrete integration design; the types and methods identified below as proposed are not implemented or qualified by this document. No execution occurred.

Use the existing `agent::AgentRuntime` as the ownership nucleus; do not create another worker-like runtime with a confusingly similar name. `agent_runtime_v8::NativeAgentRuntime` is the separate dedicated/shared/service-worker execution implementation, not the window-agent owner to reuse for auxiliary documents.

## Current production chain and exact gaps

| Stage | Current source/caller | What exists / what must change |
|---|---|---|
| Parent page admission | `runtime.rs::BrowserContext::create_page_with_policy`, `PageGroup::register` | Allocates independent `WorkerPage` with `client: Mutex<Option<WorkerClient>>`; records native creator/origin/group. Initial inherited origin/policy and named targets exist. Must also select and reserve the window agent before returning an auxiliary Window. |
| Parent lazy launch | `runtime.rs::WorkerPage::request` | Each missing client checks out/spawns a worker independently. No production call to `WorkerClient::attach_environment` was found in this chain. Agent placement must govern this branch, including races between opener/popup first commands. |
| Shared process mechanism | `worker_client.rs::WorkerClient::attach_environment` / `initialize_environment` | Shares `WorkerProcessSession` and authenticated router while checking context, direct opener, policy digest and limits. This is transport attachment, not shared isolate or shared DOM. |
| Worker command demultiplexing | `worker.rs::serve_with_services`, `dispatch(Initialize)`, `PageSession::new` | Routes map to `Option<PageSession>`; each Initialize constructs another in-process Browser/context/page. Introduce an explicit worker-side agent registry, not another independent `PageSession::new` for a same-agent attachment. |
| In-process actor | `runtime.rs::create_page_with_policy` non-worker branch | Spawns one thread, `PageState::new`, current-thread Tokio scheduler and command loop per page. A same-agent attachment must enter an existing agent actor, not launch another thread. |
| Executable document | `rust_executable.rs::RustExecutableDocument` | Owns `agent: agent::AgentRuntime`; constructor creates `DomRuntime` and a new `RawV8Host` (around the current constructor's `RawV8Host::new_with_network_context_and_heap_limit` call). Lift agent ownership out of each document. |
| Agent nucleus | `agent.rs::AgentRuntime` | Already owns `RawV8Host` + `SharedDomRuntime` and validates the primary arena. Its comment explicitly says placement is unchanged. Reuse this as the one actor-owned resource. |
| Host | `host.rs::RawV8Host` | One `OwnedIsolate`, context/binding maps by `RealmId`, explicit microtask policy, watchdog and environment registries. Multiple frame realms exist; auxiliary roots need separate root ownership and lifecycle, not fake iframe parents. |
| Privileged host endpoint | `host.rs::ContextState`, `FetchState` | Currently isolate slots hold one page/client and one broker/base URL. Many callbacks read these slots. Sharing roots without replacing these lookups would misattribute child requests to opener. This is a mandatory transitive cutover. |
| Existing auxiliary wrapper | `remote_window_v8.rs::remote_window_factory/remote_window_object` | Native branded remote wrapper, not the target V8 global. P/W is extending permitted remote operations; same-agent same-origin results must resolve a real stable WindowProxy rather than layering unrestricted properties onto this wrapper. |

Plan anchor: completion-plan package P explicitly requires parent-authoritative group/agent/cluster placement, synchronous same-agent access, exact routed authority, bounded counts and retirement of the correct hosted domain. Related pages are not owned workers: `oryn_js::realm::AgentEnvironments::retire` intentionally does not retire related windows as worker descendants.

## Proposed ownership model

1. **Parent `WindowAgentPlacement` registry (new).** Owned by `BrowserContextInner`/its group coordinator, keyed by an opaque parent-issued AgentId. Each entry retains immutable context/group/cluster identity, lifecycle state, process-session association and the set of page-route leases. PageId/group equality is not an attachment credential. Register an auxiliary page and reserve its agent membership atomically with the existing native creator checks. Noopener/COOP separation allocates a distinct eligible agent. Agent-cluster policy must be evaluated from committed native isolation metadata; same process, same origin or direct-opener status alone is insufficient.
2. **Worker `WindowAgentActor` (new wrapper around existing AgentRuntime).** One owning thread, bounded mailbox, one `AgentRuntime`, and `BTreeMap<PageId, RootDocumentState>`. Each root state has its own `PageState` document/history/navigation/layout/viewport/input/capture state and explicit root realm/document identity. Its agent field becomes a handle/key, not an owned runtime. Workers may contain multiple independent agent actors, but no V8 handles cross their threads.
3. **Document driver split.** Refactor `RustExecutableDocument` into root-local driver state plus methods taking a short-lived `&mut AgentRuntime` (or an explicit `AgentExecutionContext` borrowing host + arena + root identity). Preserve existing methods through internal forwarding during generation, then remove the alternate production constructor. Avoid `Rc<RefCell<RawV8Host>>` borrowed across JS execution: reentrant same-agent script calls would panic or deadlock. The actor takes one host borrow to enter JS; callbacks access typed isolate/context registries, as existing initial-frame creation does.
4. **One shared DOM arena for same-agent roots.** Allocate distinct Document nodes; never reuse `DomRuntime::document()` as the default for later roots. Bind each root's layout/capture/parser/style/selector provider to its explicit Document node. Shared arena supplies stable NodeId identity and supports cross-document adoption without copying JS-visible nodes. Cross-origin or separate-agent arenas remain separate. Audit every default-document fallback rather than relying on whichever root was most recently active.
5. **Isolate-wide registries versus per-realm state.** Keep RealmId/EnvironmentId allocation, V8 isolate, watchdog, microtask queue and callback-created pending-context ownership agent-wide. Store endpoint/broker/base URL, root PageId, route generation, security policy, stable document-authority epoch and DOM Document identity on the appropriate context/binding record. Replace all isolate-slot `ContextState`/`FetchState` reads with a realm-derived endpoint accessor; explicit target operations use the target realm, while incumbent-origin checks use the incumbent realm. Never swap a global slot before a command as a substitute: nested cross-window calls can change the current realm before the command returns.

### Concrete API shapes to generate

These names are proposed, not existing contracts:

- `WindowAgentAdmission { agent_id, context_id, group_token, cluster_key, source_creator, target_page, route, reservation }`, issued/validated by the parent, opaque to author script.
- `WindowAgentActor::attach_root(admission, initial_document) -> RootHandle` and `retire_root(root_identity, reason) -> RetirementReceipt`.
- `RawV8Host::create_auxiliary_root(scope, admitted_root, document, origin, inherited_policy) -> RootRealmIdentity`, using `create_native_realm_context` and `NativeRealmIdentities::pending_contexts`; keep `frame_id=None` without inferring that every such context is the one main root.
- `RootRealmIdentity { page_id, realm, environment, document, generation, authority_epoch, group, agent_id }`; a separate stable Window identity survives document replacement.
- `RustExecutableDocument::attach_to_agent(...)`, root-scoped execution/checkpoint/navigation methods, and a root-scoped host accessor replacing reliance on `RawV8Host.active_realm` as permanent ownership.
- Typed completion envelopes carry root identity plus operation token. Existing `WorkerRoute`, `Creator`, `AutomationRealmSnapshot`, navigation token and stream/authority epoch checks remain authoritative; new identity supplements rather than replaces them.

## Synchronous window.open and reentrancy

The initial same-agent about:blank document must exist **before window.open returns**, permitting `popup.document.write(...)`, object identity exchange and synchronous callbacks immediately. Waiting until the next WorkerPage request attaches the popup is too late.

Required sequence inside native open:

1. Validate source Creator and sandbox/user activation in parent; reserve page/group/agent/route and initial document authority. Parent work here must not require another command through the currently blocked worker process dispatcher.
2. Return a bounded typed admission result over the existing privileged context channel. In the current isolate callback, allocate root Document/context/bindings into the agent's callback-safe registry and establish real WindowProxy identity. This allocation follows the existing pending native-frame context pattern; it does not borrow an actor or document driver already mutably borrowed by the outer script.
3. Record a pending actor attachment descriptor in an agent-owned queue. The next checkpoint adopts its already-existing realm/document into the document driver. Driver attachment must not recreate the context or erase synchronous changes made before the checkpoint.
4. Queue URL loading asynchronously, using existing navigation custody; the returned popup object represents the initial Document immediately. Any failed reservation/realm allocation rolls back parent authority and local pending registries together.

A nested popup → opener → popup function call executes directly through V8 contexts on the same thread. No synchronous IPC round trip to the target page actor, JSON value surrogate, author-property proxy, or global `Mutex<AgentRuntime>` is acceptable. Objects, prototypes, exception identity, DOM nodes and callbacks must remain actual JS values. Native WindowProxy security checks enforce incumbent origin even for co-located foreign realms.

## Navigation, close, cancellation and failure domains

- **Same-agent document navigation:** preserve stable WindowProxy identity using the existing native detach/rebind global technique (`create_native_realm_context` accepts a global; frame commit paths preflight before detach). Replace only the target root's realm/document environment; old bindings fail generation checks. Preserve other root globals and node wrappers.
- **COOP / cluster transition:** parent revokes old relationship/admission at commit before exposing new origin. A migration cannot move V8 objects between isolates. Retire the old environment and establish a new-agent target, with old references using the permitted remote/severed behavior. P/W's remote boundary owns the public cross-agent behavior. BFCache needs separate parked root records, not the single `RawV8Host.parked_document` option; cache/restore cannot park the whole agent when a sibling remains active.
- **Close one root:** cancel only its navigation/parser/resource/capture/input/prompt/network/storage/message tasks, retire owned frames/workers, then revoke root environment and release route. Popup membership is not an `EnvironmentDescriptor.owner` child of its opener; closing opener must not implicitly close a live auxiliary page.
- **Task scheduling:** one agent task/microtask execution boundary with root-tagged task sources. Never recursively run a second root's full actor queue during a synchronous getter. Deferred events run after the appropriate script/checkpoint; root-local DOM/custom-element/observer delivery must not drain sibling queues under the wrong Creator.
- **Cancellation:** distinguish cooperative root operation cancellation from isolate termination. Existing `WorkerPage::interrupt_inflight` SIGKILL is process-wide; with sharing it must mark every hosted route failed, not only the requesting page. A watchdog terminate-execution request affects the running isolate; generation-tag it so a stale timeout cannot terminate a later sibling script. If recovery cannot prove the isolate usable, retire the whole agent; if the OS process is killed, retire every agent in that process.
- **Reset/recycling:** only after all root leases, pending context allocations, task queues, handles, parked documents, native services and route teardown receipts are drained. Shared-session `Arc` counts remain a necessary guard, not sufficient proof of empty V8 registries. Record one physical process owner and separate environment lifetimes in provenance.
- **Quota/admission:** charge heap once per agent/process and account root/realm/task/resource quotas separately. Attachment cannot multiply per-process heap allowance. Enforce agent and environment caps before authority allocation and roll back partially admitted roots.

## Integration ownership and generation ordering

Root can generate the actor/document ownership split and host realm endpoint accessor while P/W completes remote permitted Window/Location paths. P/W reported reserved edits in `remote_window_v8.rs`, parent remote snapshot/context request and the narrow target FrameCommand branch. Agree on a single resolver: local admitted same-agent targets return the actual WindowProxy; remote targets delegate to P/W. Remote indexed children require authoritative DOM order publication, not token ordering from `activation_parents`.

Generate all affected source and regression paths before activation/testing: parent placement → typed attachment admission → shared worker actor → borrowed document drivers → callback-safe synchronous root creation → endpoint accessor cutover → replacement/retirement/BFCache → native local/remote resolver → metadata and regression inventory. This is dependency ordering within the complete generation batch, not permission to execute small cycles. If new wire payloads are incompatible, reserve one coordinated pending version and update producers/consumers/signing/source validators together; this audit does not bump any contract.

## Required regression source map

| Boundary | Required source regression (new unless already covered in narrower frame tests) |
|---|---|
| Production admission | WorkerPage opener/popup first-launch race creates one admitted agent and one process session; unrelated contexts/groups and forged source Creator reject; denied admission releases reservations. |
| Synchronous initial document | Actual native window.open returns a popup with immediately writable Document; child invokes opener callback synchronously; shared object/DOM identity and realm-specific constructors/exceptions remain real. Run through WorkerClient, not host-only helper. |
| Actor integration | Two routes address distinct roots in one actor; attaching driver preserves pre-checkpoint document.write/global mutations; root viewport/style/layout/capture/AX never reads sibling default Document. |
| Security endpoint | Popup fetch/cookie/storage/CSP/service/media call uses popup PageId, document authority and origin; nested cross-window invocation preserves incumbent and target identities. Frame beneath popup uses popup root endpoint. Co-located cross-origin document/global reads fail. |
| Navigation | Popup replacement keeps WindowProxy stable, retires old environment callbacks, leaves opener untouched; cross-origin return restores only permitted access; COOP severance cannot resurrect old group references. |
| Cancellation | Closing popup during pending network/input/capture cancels only popup tasks; opener close leaves popup usable; stale timeout cannot terminate later script; process crash fans out every hosted route and reports actual shared PID. |
| Cache | Park/restore one root with active sibling; discarded cache document cannot publish tasks; per-root parked tokens and realm policy restore remain exact. |
| Ownership/reset | Recycle refused with any root/realm/pending-context/service authority alive; last-root release drains once; actor exit drops host before arena; quota failure leaves no route/realm leak. |
| Remote seam | P/W proxy handles cross-agent allowed methods without exposing document; native local resolution never becomes available via public page numbers or author constructors; indexed children use committed DOM order. |

Existing narrower sources to extend: worker-client attachment negatives, `AgentEnvironments` non-worker retirement semantics, native frame initial-creation/commit/preflight tests, native stable-authority generation tests, `browsing_groups` COOP non-resurrection tests, and remote Window stale-creator/closed-identity tests. These are foundations, not coverage of the proposed production shared-agent chain.

## Generated prerequisite: realm endpoint custody

The endpoint singleton gap above now has generated source (unbuilt/unrun); the shared actor/root ownership design remains pending.

- `RealmEndpointRegistry` stores an exact EnvironmentDescriptor and RealmEndpoints per RealmId. `endpoints_for_environment` rejects retired or mismatched descriptors; context and binding accessors also reject execution-retired bindings. No isolate-wide ContextState/FetchState slots remain in native source.
- `create_native_realm_context` now requires explicit endpoint input. Initial host construction supplies its native endpoint; callback-created documents inherit the validated creator's endpoint; driver-created frame realms select their explicit active creator context. Endpoint state is installed before native callback/binding installation.
- Document environment rotation and initial-frame reuse move endpoint custody to the new descriptor. Retirement removes an entry only when its stored descriptor matches the retired environment: failed allocation rollback for a later environment in the same realm cannot erase the live predecessor.
- Native context/fetch, parser script loading, typed/dynamic modules, remote Window/Location, WebDriver conversions and native message paths use these accessors. Root/frame target callbacks select their actual receiver context. PostWindow uses the incumbent binding for both scoped Creator and page endpoint. Privileged request Creator validation stays in the existing parent services.
- Parked-document discard retains the endpoint that issued the parked token; destruction does not guess ownership from whichever realm is active. Per-root BFCache storage still belongs to the later shared-root ownership change.
- S/V integrated its parser-inline policy callback with the realm-selected endpoint at initial binding creation and initial-frame reuse, deriving Creator on each call; this closes the pre-parser-resume initial-frame endpoint gap.
- `realm_endpoint_tests.rs` contains source regressions for nested same-isolate broker/base/request-counter selection, distinct page-service routes, retirement, old-generation rejection and rollback of a never-installed environment. These source tests do not establish production same-agent popup support.

No builds, tests, services or protocol activation occurred. Parent-owned root placement, one shared actor/arena, synchronous popup attachment, process failure fanout and the remaining regression matrix are still required.

### Per-root parked authority source integration

`RawV8Host` now retains a `BTreeMap<RealmId, ParkedRealmDocument>` rather than singleton token/endpoint fields. Each parked entry retains its exact environment descriptor, page endpoint, document generation and stable authority epoch. Existing production park/restore callers use this map. Restore verifies the same live identity before presenting the capability; a successful restore removes only that root. `discard_cached_authority` uses the original issuing endpoint and preserves custody on failure; existing realm removal invokes it before tearing down local ownership. Host Drop attempts every retained discard through its own issuer.

The generated regression `two_native_roots_park_restore_and_discard_their_own_parent_capability` uses native parent document-authority constructors and two realms in one host, then checks independent parking, duplicate rejection, single-root restoration and sibling discard. It is not a production multi-root actor test and has not run.

Still pending: existing `suspend_page_isolate` freezes the entire isolate and its timer clock. Shared-root actor activation must replace this with root task suspension plus agent-level suspension only when every hosted root is parked. Moving parked-token custody alone does not permit enabling shared WorkerPage launch. The document/actor extraction, synchronous popup attachment, root-local task scheduler and process failure fanout remain part of the assigned implementation.

The same source batch adds `PageScheduler::suspend_document/resume_document`: document-keyed queued tasks and timer entries are removed from runnable work, retain their remaining active-time delay, accept deferred tasks while parked, and rejoin after restoration without delaying siblings. Timer/document cancellation also removes parked entries. The native parent-authority park/restore/discard paths call these methods, so these are integrated consumers rather than unused scheduler APIs. Regression sources cover sibling progress, preserved delays/order, queued-while-parked tasks, cancellation and retirement. Native non-timer queues, isolate microtask/reveal suspension and actor ownership still require the remaining root-scoped integration before shared placement can activate.


## Custody extraction implementation note (2026-09-16)

`AgentRuntime.host` now uses `DocumentHost`, a non-Clone unique optional owner
with checked take/install operations and ordinary borrowed dereference during a
document turn. Existing callers still execute with the owned host installed.
This provides a safe movable ownership boundary without an `Rc<RefCell>` borrow
held across JavaScript callbacks and without an unsafe aliased host reference.
The two-document transfer regression source checks duplicate take and occupied
recipient rejection while preserving returned ownership. Actor acquire/release
and popup adoption are still pending production integration; the type alone is
not shared-agent completion and does not authorize worker attachment.

Frozen native task delivery now includes frame descendants and deferred fetch
receipts. The installed V8 dependency documents that synchronously accessible
contexts must use the same microtask queue. A per-root queue substitution would
violate that requirement; queued promise-job freezing remains an explicit
integration requirement before shared BFCache can be enabled.

## Production actor-root extraction (source generation, unexecuted)

The existing production page thread now invokes `page_actor_turn::Root`.
`Root` owns `PageState`, its receiver, pending navigation replies and pending
capture replies. `dispatch` retains the navigation actor's begin/commit/stop
handling and its transaction/committed-reply polling. Close returns the reply
sender to the surrounding actor; that actor destroys the root before sending
the acknowledgement. Root destruction explicitly completes queued navigation
and capture replies with `Closed`, including disconnect paths. Regression
sources cover independent pending captures for two roots and acknowledgement
ordering after root destruction.

The root turn is now reusable by the shared actor, but production scheduling
still creates one such root per page thread. Remaining cutover must combine
that owner with the exclusive host transfer, a constructor that adopts the
callback-created root instead of reloading its document, and parent worker
route reservation without reentrant `Initialize`. Do not mark P-04 complete
based on the extracted dispatcher.


## Production integration and cleanup reconciliation (source only)

The earlier “remaining cutover” paragraph describes the extraction checkpoint,
not the current source. The native callback now reserves and creates an admitted
auxiliary root synchronously; `CreatedAuxiliaryRoot` is consumed by the related
actor and adopted without document/bootstrap replay. Lazy worker Initialize
claims the exact existing actor route. Direct endpoints use parent-issued actor
identity without treating it as a transport grant. Root replacement and cached
root drivers transfer the exclusive host through the actor. These paths remain
unbuilt and unqualified.

The latest source review adds exact retired-route navigation cancellation:
`navigation_cleanup`, `refresh_navigation_cleanup`, and the post-registration
caller retain the original NavigationControl only. Only Stopped bypasses live
route admission, and reset waits for terminal navigation custody. The regression
`retired_navigation_grant_retains_only_original_stop_and_waits_for_terminal_cleanup`
covers missing/forged/wrong-page grants, nonterminal event rejection, superseded
IDs, and reset release. `cancel_pending_initialization` revokes an unopened child
without entering Initialize; `Retiring` preserves exact route cleanup on timeout.

Native queue restoration now compares actual queue association before retrying a
partially completed restoration. Recoverable BFCache errors retain the queue.
Final-owner rollback/destruction fail-stops on an invalid native checkpoint,
rather than freeing storage still referenced by V8. Root retirement removes only
its frame/creation/discard and isolation records, preserving siblings and current
navigation mappings. These are source-reviewed changes, not runtime validation.

Still requiring source reconciliation: the complete shared actor close/retirement
ACK integration, resource cleanup receipts across retired routes, cross-root
module execution continuation, and the overall G operation coverage inventory.
All generation precedes builds, regression execution, and qualification.
