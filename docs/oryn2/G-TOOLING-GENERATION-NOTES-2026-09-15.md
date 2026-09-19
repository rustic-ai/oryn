# G tooling generation: full inventory, qualification and fixture configuration

Source generation only. No new test/build/deployment results were collected.
Nothing here promotes the historical 4,415 raw Core passes to qualified passes.
The fixed Core denominator remains 16,737 and all current package gates remain open.

## Generated implementation and regression sources

- `scripts/wpt_full_qualification.py` and
  `scripts/reconcile-full-wpt-qualification.py`: official-report qualification
  separate from legacy executor semantics. Validates the actual manifest and
  policy bytes against pinned hashes, reconstructs all Core IDs and source hashes,
  preserves variants and all attempts, rejects duplicate/unknown IDs and mixed
  candidate/executor reports. Missing results remain explicit obligations. Owner,
  prerequisite, observed failure, confirmed cause, change and acceptance fields
  exist per entry without inventing causes.
- Qualification requires source/report/candidate-bound independent prerequisite
  review, exact testharness subtest names/order, verified resources/TLS/focus,
  cleanup and negative-control records with artifact hashes. Reftest/print review
  additionally requires native comparison and reference readiness evidence. A
  later passing attempt never erases an earlier failure.
- Manual/visual records require separate operator/reviewer, procedure, platform,
  observations, assistive-technology procedure or justified non-applicability,
  source/candidate identity and hashed artifacts. They cannot waive an automatable
  entry. They are independently reviewed records, not an implemented OS adapter.
- `scripts/tests/test_wpt_full_qualification.py`: generated negative controls for
  candidate substitution, ID/variant loss, omitted/renamed/duplicate/failed
  subtests, malformed subtests, stale source, missing prerequisites/cleanup,
  retries, readiness, manual independence and denominator changes.
- `scripts/wpt_fixture_contract.py` and
  `scripts/tests/test_wpt_fixture_contract.py`: explicit test-only host/port/IP
  topology, TLS artifact locks, protocol-port completeness and live-probe record
  validation. Configuration alone returns **unqualified**. Public endpoints must
  use globally routable addresses; loopback aliases and documentation IP ranges
  are rejected. Peer/TLS negative controls are required before a probe qualifies.
- `tools/wptrunner-oryn/fixture-contract.example.json`: template with unresolved
  owned private/public endpoint addresses and TLS hashes. This is intentionally
  not a usable deployment or evidence record. Hostnames must be expanded to the
  entire pinned upstream fixture host set before actual execution.
- `tools/wptrunner-oryn/canaries/`: deliberately failing assertion, missing helper,
  missing completion, never-ready reference and mismatching reference sources.
  `controls.json` defines required rejection outcomes and remaining native fault
  controls. This separately mounted corpus must not alter the official manifest.

## Interfaces and known remaining work

1. Native driver must consume explicit fixture CA trust and **host+port endpoint
   routes**, without bypassing certificate or web policy checks. Pinned
   `fetch/local-network-access/resources/support.sub.js` selects address space by
   changing the port of the same hostname. Aliases alone cannot implement this.
   Actual sockets must reach the appropriate owned VM private/public interfaces;
   record native peer addresses. No host, routing, firewall or CA store was changed.
2. Provision owned network resources after generation, start pinned upstream
   fixtures including every port family, verify all host aliases and helpers,
   collect real TLS positive/negative and focus controls on Oryn and the reference.
   The example config is not infrastructure provisioning automation. A real owned
   public IP allocation and native endpoint-routing support are still required.
3. The official report producer must add `oryn_candidate_sha256`,
   `oryn_wpt_revision`, and `oryn_executor: upstream-wptrunner` to `run_info`, with
   the frozen candidate record binding runtime/driver/plugin/fixtures. The ledger
   rejects missing identities. It does not manufacture upstream run metadata.
4. Mount and execute the canary corpus separately using pinned upstream routes;
   collect resource, process/cleanup and comparison artifacts for independent
   reviews. Native fault injection, print pagination, trusted input, script,
   lifecycle/crash and full command coverage remain native integration work.
5. Evidence review is intentionally explicit per official entry. No mass approval,
   implicit pass prerequisite rule or new upstream-defect waiver is generated.
   Existing legacy reviewed dispositions are not automatically migrated across
   executor semantics; review provenance must be reconciled separately.
6. Establish parity before replacing the historical runner. These files leave
   `prepare-full-wpt-baseline.py`, the legacy runner and raw historical artifacts
   untouched. New ledger output requires a new directory and locks all inputs.
7. CLI binds complete pinned inventory before reconciliation. The lower-level
   `reconcile` helper accepts synthetic inventories for regression sources and
   must not alone be treated as a full Core gate. Its `complete` field concerns
   Core evidence only; combined product/security/performance acceptance remains
   separately required.

## Deferred verification

After **all** implementation and test generation for the complete plan finishes:
run the new Python regressions; validate actual pinned inventory reconciliation;
run authenticated native upstream executor controls; verify fixture topology and
TLS/focus evidence; then run and independently qualify the full candidate. No
source inspection or test-source count establishes completion of G.

## Continued generation: integrated official workflow

Supersedes the earlier open item about producing candidate-bound `run_info`:
`oryn_wptrunner/provenance.py` now implements the pinned upstream
`Product.run_info_extras` hook. It verifies candidate record, runtime/driver,
source snapshot/signature evidence artifact identities, every loaded plugin Python
source and fixture/TLS identities before emitting metadata. This verifies bytes;
the signature evidence itself still requires Q's independent signature audit.
Browser creation rejects a contract changed since `run_info` was established.

The plugin consumes `--oryn-run-contract` (or `ORYN_WPT_RUN_CONTRACT`) and supplies
native `--configuration` plus `--configuration-sha256`. Arbitrary driver arguments
are rejected. It validates upstream's pregenerated CA/certificate/key paths and
finalized ports/positive hostname set before fixture startup. **Use domains_set,
not all_domains_set:** upstream's deliberately nonexistent DNS names must remain
unmapped. Native socket routing, CA loading and parent network authorization are
still integration dependencies until implemented and verified.

`wpt_official_workflow.py`, `run-official-wpt.py` and `wpt-upstream-entry.py` now
provide a concrete preparation/execution workflow. Both preparation and execution
require a hash-bound declaration covering implementation and test sources for
**all 51** plan requirements. No gate record was created and no workflow was run.
Preparation invokes the pinned upstream manifest API only for the separate canary
root, writes upstream manifest-mount configuration and separate exact selection
lists, and locks those inputs. Execution invokes genuine upstream wptrunner with
normal TLS, one process, no retries/reuse-window, fresh output and retained raw
report/log/exit artifacts. The official manifest remains read-only and unmodified.

`wpt_canary_qualification.py` and `qualify-wpt-canaries.py` require every deliberate
failure, the exact subtest outcome, missing-helper resource evidence, native
reference provenance, a failing-suite exit and complete worker cleanup. These
five controls cannot qualify the remaining native fault/TLS/focus controls. Added
regression sources for provenance drift, gate enforcement, command arguments,
canary completeness, incorrect passes and missing cleanup/resource observations.

### Run contract schema

A run contract is `{schema_version: 1, upstream_revision: <pin>, candidate:
{path: <absolute candidate JSON>, sha256: <bytes hash>}}`. The candidate JSON has
`schema_version: 1`, `artifacts` binding absolute paths/hashes for `runtime`,
`driver`, `source_snapshot`, `signature_evidence`, `fixture_contract`, and
`plugin_sources` mapping every relative Python source in `oryn_wptrunner/` to its
SHA-256. Never label an existing mixed-source diagnostic as this candidate.

The generation gate JSON binds `plan_sha256` and exactly G-01..08, R-01..07,
P-01..06, L/W/S/V-01..05, F-01..04, Q-01..06 under `requirements`. Each record
requires `implementation_generated: true`, `tests_generated: true`, and nonempty
`implementation_sources`/`test_sources` arrays of repository-relative path/hash
records. This declaration must be produced only when generation actually closes;
the workflow rejects absent, partial and changed evidence. It does not declare
implementation correctness before builds/tests.

### Qualification sequence after the generation gate

1. Freeze/sign/audit the candidate and populate the explicit fixture contract;
   provision real VM endpoints and upstream fixture trust/host prerequisites.
2. `run-official-wpt.py prepare` creates fresh locked mount/selection inputs using
   the pinned dependency Python, inventory, actual manifest, candidate contract
   and completed generation gate.
3. `run-official-wpt.py canaries` executes the separate canary collection.
   `qualify-wpt-canaries.py` validates raw report/execution plus independent
   resource/native-cleanup audit. Retain the expected failing process status.
4. `run-official-wpt.py core` runs the automated Core collection on a fresh output.
   Collect manual/visual procedures and independent prerequisite reviews.
5. `reconcile-full-wpt-qualification.py` writes the separately attributed complete
   Core obligation ledger; Q still owns combined product/security/performance
   acceptance and reviewed upstream-defect migration.

## Native fixture loader review and generation

`crates/oryn/src/webdriver/fixture.rs` now opens the configuration and relative
artifacts through pinned directory descriptors, refusing symlinks at each
component and non-regular files. Nonblocking opens prevent FIFO substitution from
waiting for a writer. Config/CA reads are bounded and hash-verified. The native
browser opens server certificate/key references for metadata validation but never
reads private server-key bytes; upstream still verifies their content identities.
The module is gated by `webdriver-server` so its optional libc dependency matches.

Routes must use their declared loopback/local/public class; IPv4-mapped IPv6 is
canonicalized before policy checks. Public fixtures deliberately require ordinary
global unicast, excluding special, transition, benchmark and documentation ranges.
Python fixture validation now uses the same explicit ranges rather than
Python-version-dependent `is_global` behavior. Raw configurations are unchanged;
rejected addresses remain explicit configuration errors. Scoped IPv6 endpoints
are unsupported by this contract.

Canonical positive hostname validation excludes intentionally nonexistent labels.
Policy configuration narrows unrestricted defaults and refuses conflicts with an
existing caller's host/port/scheme denials. Added source tests for all protocols,
reserved address substitutions, mapped IPv4, real route destination policy,
unknown routes, malformed/duplicate/missing inputs, atomic policy rejection,
configuration/CA hashes, size limits, traversal, symlinks and FIFOs. Native CA
parsing still occurs through the existing real Rustls transport before use.

Read-only review of `webdriver/cli.rs`, the exec-only `oryn-webdriver` launcher and
runtime dispatch found the plugin flags match: browser binary, host, port, private
auth token, fixture config and its SHA-256. The launcher's default `max_sessions=1`
matches the workflow's single upstream process. Upstream's startup availability
probe opens a TCP connection rather than an unauthenticated HTTP command, so it
does not bypass or conflict with bearer-header command admission.

All of this remains unbuilt and untested. Actual certificate validation, peer
routing, complete command behavior, teardown and conformance evidence remain
separate qualification requirements.

## Native WebDriver execution source generation

Added `crates/oryn-native/src/webdriver_script.rs` and `webdriver_script_v8.rs`, with integration in `host.rs` and `rust_executable.rs`. These are source-only additions, not validation evidence.

- Typed request, operation ID, pending/completed result and protocol error envelopes; native function-body compilation and argument construction, private native promise/callback storage, first-settlement handling, host wall watchdog and operation deadline.
- Native recursive JSON cloning, native node/collection identity, bounded traversal, cycle errors, explicit getter/toJSON exceptions, stable opaque element/shadow references and document-generation/stream-epoch retirement.
- Executable root/selected-realm begin, poll and cancellation. Actor owns page-task draining and navigation processing; poll only checks the native operation.
- Native frame inventory carries parent/frame/root/generation/epoch and parent-document ownership liveness. Trusted print lifecycle dispatch snapshots recipients and enters native root/child contexts without author dispatchEvent lookup.
- Regression sources cover poisoned globals, arguments, duplicate callbacks, promises, late exceptions, cancellation, deadlines, cloning, element detachment, document.open, native collections, print and frame inventory.

Remaining integration/qualification limits: parent actor must retain operation ownership until terminal completion/cancellation, process native page work between polls, reject stale selected frame compound identities and bracket print media restoration even on errors. Returned Window references still require a browsing-context handle registry; current native serializer returns unsupported operation. Rust serde_json strings cannot represent lone UTF-16 surrogates; current lossy V8 string conversion is an explicit conformance gap. Full upstream execution, cross-origin print/topology checks and all build/tests remain deferred until the complete generation gate. No qualification disposition changed.

### Lossless wire, context handles and frame selection follow-through

The earlier Window/UTF-16 source gaps are now addressed for installed native contexts: `ScriptValue` carries UTF-16 strings and keys, and the bounded JSON parser/encoder preserves lone surrogates. Script bodies are UTF-16 too. Runtime, worker and WebDriver script HTTP paths now carry this representation without a serde_json::Value conversion; protocol diagnostics remain ordinary Rust strings. Native windows are identified through binding creation-context identity and serialized using actor-supplied session handles. The current HTTP actor supplies the actual top-level window handle; frame and other-window handle registry integration remains owned by the parent.

Generated native frame selection accepts top, parent, index and opaque element reference. It checks native frame ownership, connection generation, document root/generation/stream epoch and native shadow-including frame order. A dedicated compound-context script entry point rejects a stale selected document; top selection can recover from a detached child. These changes have regression sources but have not run.

Native control layout now projects current text-control values, suppresses textarea default-value children and masks password text with one bullet per UTF-16 unit. The layout agent owns textarea wrapping UA defaults. Native Window realm creation and initial-frame document commit now attach the parent context-service prompt backend; bootstrap prompt dismissal stubs were removed. Parent owns independent modal/cancellation transport.

### Selected-frame OIL and modal timing generation (continued September 16)

Added `crates/oryn-native/src/frame_runtime.rs` and scoped worker transport. Each live selected document receives independent semantic, revision, layout, scroll, delta and action state, with a reserved semantic generation; the single page-owned executable is lent to that state for a serialized command and restored on normal/error exits. Native document snapshots retain NodeIds while pruning foreign documents through `DomArena::snapshot_document_scope`. Standard OIL observation, targeting, input/selection/click/submit, extraction, geometry, scrolling, deltas and file upload now route to the selected native DOM. OIL `frames`, `frame TARGET`, `frame parent` and `frame main` share the native frame authority checks. Navigation commands reset OIL frame selection to the page. Frame-scoped transport is implemented for native Unix workers; non-Unix local callers reject scoped execution instead of silently operating on the top document.

Modal waits now pause and shift nested native execution watchdog deadlines through the shared PromptWaitObserver and preserve the remaining WebDriver script budget. Parent prompt-controller retirement cancels a pending native script even with a null script timeout. Added source regressions for cancellation before actor close, nested deadline pause accounting, actual session window/frame handles, exact lone-surrogate request/results, shared-arena and opaque-frame OIL actions, semantic-reference isolation and detached-frame recovery.

These changes are generated source, not passed checks. Builds, tests, WPT runs and deployment remain unexecuted under the all-generation-first gate. Parent navigation integration and modal future ownership are being generated separately; no historical evidence or pass disposition has been changed.


## Auxiliary Window identity generation (2026-09-16)

Source generated, **not built, executed, or qualified**:

- `remote_window_v8.rs` replaces author-property popup brands with isolate-owned native object identities, owner document generation/epoch checks, a bounded retained identity registry, and parent-authoritative liveness. Retired objects remain recognizable and cannot fall back to plain-object cloning.
- WebDriver script values include a worker-only `WindowPage(PageId)` transport variant. Author JSON cannot create this variant and the HTTP JSON encoder rejects unresolved references. Native popup/opener objects serialize before author getters or `toJSON`; typed arguments recover the canonical per-realm object. Same-page arguments recover the actual root Window global.
- Session argument authorization and result finalization resolve through the live BrowserContext registry. New popups created inside a script acquire actual session handles at finalization. Unknown, closed, and foreign-context identities fail. Random session namespaces prevent accidental cross-session handle-name collisions.
- The parent-owned AutomationWindowStatus RPC admits native identity checks for unrelated windows in the same session. The author context-call callback explicitly rejects this operation; the author-visible popup factory remains limited to parent-validated related pages. Parent server integration authorizes arguments and finalizes results under the existing session lease.
- Regression source covers native identity vs forged properties/Proxies, poisoned getters, repeated references, argument round trips, unrelated-window native admission, author RPC rejection, forged source, foreign target, retirement, closed windows, nested result finalization, and transport-boundary rejection.

The auxiliary proxy's postMessage retains the existing JSON-based payload contract; complete structured-clone messaging semantics are not claimed by this identity bridge. Actual engine compilation, native lifecycle runs, official WPT, fixture deployment, and qualification remain deferred to the full-plan generation gate. Parent owns synchronous close admission ordering before the ClosePage acknowledgement.

## W native message transport generation (2026-09-16, ongoing)

Generated auxiliary Window and MessagePort IPC using genuine V8 structured
serialization. Engine bytes preserve graph aliases/cycles, UTF-16, BigInt,
Map/Set, Date/RegExp/Error and typed-array/resizable-buffer semantics; native
host records carry Blob/File/geometry/ImageData/DOMException and transferred
ports. Sender buffers detach only after successful serialization. Parent hub
rotates endpoint capabilities, rejects repeated transfer tickets, bounds queues,
keeps transferred endpoints alive independently of sender retirement, and
releases discarded transferred queues. Destination MessageEvent creation and
dispatch are native and unaffected by author JSON/Event/dispatch replacements.

Frame routes use random native embedding-lifetime tokens, retained across child
navigation and retired on detachment. Parent queues and native polling address
that exact lifetime; a root poll cannot consume frame messages. Script WindowRealm
values and session frame aliases use the same native token. Target-origin checks
occur at delivery. The `/` case compares parent-owned origin authority keys,
including shared vs distinct opaque origins; serialized `null` grants no match.

Source regressions cover cross-isolate graphs/ports, real destination prototypes,
trusted events, poisoned author functions, invalid/duplicate transfers, resizable
buffers, closed targets, frame replies/detachment, transfer replay and rotation,
foreign capabilities, stale frame retirement, queue cleanup and opaque-origin
identity. **All remain unbuilt and unexecuted.**

W generation continues: independent worker agents and native worker clone paths,
shared-worker identity/client lifetimes, BroadcastChannel, and service-worker
registration/update/lifecycle/fetch control. The old JSON worker wrappers and
immediate-activated service shim remain open until their replacement is generated.
SharedArrayBuffer currently requires a real cross-origin-isolated agent cluster;
the existing clone engine rejects it rather than fabricate shared memory.

### Independent worker transport and BroadcastChannel sources

`oryn_js::worker_wire` defines parent-authorized worker bootstraps, typed agent
and client identities, commands/events, and route-bound worker service requests.
`oryn_native::agent_thread` implements an owned thread adapter for AgentProcess:
native allocation happens on the owning thread, an independent interrupt handle
is published before author evaluation, mailbox capacity counts actual serialized
wire bytes, stop takes priority over commands, and teardown joins the thread.
Factories must not evaluate author code before publishing the interrupt handle.
Regression sources cover atomic capacity rejection, wire-size accounting,
out-of-band cancellation, and interrupting a runtime that never reads commands.

Native BroadcastChannel bindings now use parent-owned origin/partition/name
subscriptions and the shared structured-clone wire format. Added cross-isolate
regression source for cyclic Map/BigInt/UTF-16 payloads, trusted event targets,
poisoned author constructors/JSON, asynchronous delivery, source exclusion,
and closed-channel clone ordering. BroadcastChannel IDL source is added; its
generated catalog must be produced only after the complete generation gate.

These additions do not yet replace the existing JSON Worker bootstrap. The
concrete RawV8Host independent Worker-global runtime, native Worker/SharedWorker
constructors, parent service routing/process factory, and service-worker
registration/update/extendable-event/fetch execution are still under generation.
The thread adapter is transport infrastructure, not a worker conformance result.
No builds, tests, formatters, or generators have been run for these additions.

### Independent native agent and service-event follow-through

Generated `host::NativeAgentRuntime::create(AgentBootstrap, Arc<dyn WorkerTransport>)`
and its AgentRuntime implementation. It allocates an independent V8 isolate and
ordinary worker global, installs native Worker-exposed bindings, preserves source
UTF-16, executes classic/modules with top-level await and dynamic imports, and
processes timers/native ports/BroadcastChannel without creator-page task pumping.
Native Worker/SharedWorker constructors, parent service requests, event delivery,
error handling, transfer lists, termination, and client retirement are generated.
The previous JSON Worker/SharedWorker/immediate-activated service bootstrap and
the superseded same-isolate worker helper implementation have been removed.

Native ExtendableEvent/FetchEvent sources now dispatch trusted events and use
native promise reactions to account for `waitUntil` lifetimes. `respondWith` is
synchronous and one-shot; native Response identity and streams supply response
metadata/body without invoking author Response getters. FetchFallback and
FetchResponse are parent-only control results, followed separately by
LifecycleFinished after lifetime extensions settle. Response body accumulation
has a 32 MiB limit. Synthetic dictionaries, provenance, and full request metadata
are still receiving source review; this is not a service-worker qualification.

Regression sources cover independent timers/clones/cancellation, native import
bases, modules/TLA, scope and EventTarget identity, real SharedWorker connect ports,
nested lifecycle extension/rejection, poisoned Response getters, streamed bodies,
post-response lifetime retention, fallback, and invalid/used/rejected responses.
Parent worker loader/service/process integration, agent platform networking and
storage, service registration/update/controller/fetch routing, and subsequent
official conformance verification remain open. All new sources are unbuilt/unrun.

Further source generation adds event-local CancelTask handling: owned response
streams and request signals abort, handled/lifetime promises reject, late
extensions lose authority, and unrelated service events remain live. Oversized
body consumption cancels its underlying stream. Native Request records preserve
mode/redirect/destination across clones. Added regression sources for those
cancellation paths, body quotas, and native Worker constructor conversion order,
UTF-16 names, clone integrity, trusted delivery, and idempotent termination.

An agent-local route gate closes before isolate interruption/cleanup so new
retirement callbacks cannot re-enter a stopping parent route. This does not cancel
an already blocked Rust broker request: the parent must cancel broker waits and
avoid joining agent execution under registry/hub locks. Parent retirement must
revoke the stopped agent's ports/subscriptions and owned nested-worker clients.

Remaining service-event integration obligations include authoritative message
source/client provenance, navigation preload, real Push/Sync event payloads, and
the parent registration/update/controller state machine. These obligations and
agent platform networking/storage are not covered by the runtime factory alone.

The native `AgentPlatformHost` adapter and PlatformSubmit/Poll/Cancel/Retire wire
sources now connect generated fetch/streams/WebSocket/IndexedDB bindings to an
agent-bound parent service. It tracks bounded operation correlations, validates
whole completion batches before committing them, keeps cancellation pending until
the parent confirms completion, and revokes a failed/retired endpoint. It carries
no PageId. Parent admission must derive resource ownership and opaque/tuple origin
plus storage partition from its agent registry, not serialized origin fields.
Added adapter and independent-runtime regression sources for these contracts and
native fetch/IndexedDB execution against a controlled parent-response fixture.
Actual parent async admission/network/storage routing remains unimplemented here;
source generation is not evidence that real agent I/O has run.

### Parent admission/lifetime integration source, 2026-09-16

Added `AgentThreadProcess::prepare` and an explicit nonblocking `AgentProcess::start`
admission gate. Prepared independent isolates publish their interrupt handle but
cannot execute author code before the registry commits principal/client routes;
SharedWorker queues its actual connect port before admission. Regression sources
cover admission, cancellation before admission, and idempotent start.

Parent registry access now resolves original owners from genuine client
capabilities and exposes immutable bootstrap snapshots. The runtime-nested
`worker_service.rs` helpers validate original document lifetimes and drain
retirement batches outside registry/message locks, retaining failed teardown
ownership. Native root Worker creation registers its exact document generation
and stream epoch; root stream replacement retires the old identity first. Root
and frame retirement messages retire workers for that exact document after
releasing page/origin/message locks. Added regression sources for forged client
secrets, retired principals, old-document retirement, successor preservation, and
termination that reenters broker locks.

Worker entry decoding now requires successful HTTP status and JavaScript MIME
for classic and module workers, with UTF-8 decoding in both cases. Parent-derived
`AgentPolicy { url, headers }` is carried by bootstrap, and native worker contexts
apply trusted-types and string-evaluation policy. Added status/MIME/encoding and
native evaluation-policy regression sources.

The full W package remains **generation in progress**. The new runtime registry
helpers are not the complete WorkerService dispatch. Outstanding integration:
parent-issued frame response policy tokens and exact-lifetime policy binding;
authorized root/frame/agent script loading, including inherited blob/data policy;
Window/agent WorkerService backends and factory dispatch using `prepare`;
agent-owned asynchronous network/stream/socket/IndexedDB routing with true
partition keys; service registration/update/controller/fetch routing; native
BFCache authority reactivation; and coordination of concurrent nonempty
retirement batches. Existing default policy fields in source fixtures do not
prove deployed policy production. No build, tests, generators, deployment or
conformance runs were performed for this generation slice.

Frame policy snapshot production is now partly connected: the parent resource
loader issues opaque response policy tickets; static, asynchronous and dynamic
native frame loaders preserve and bind tickets to the exact frame lifetime before
scripts. Native-only context routes validate the ticket's source page and final
response URL. Effective policy snapshots omit directives ignored in meta policy,
so serializing an inherited snapshot cannot reactivate a meta sandbox directive.
Added source regressions for foreign ticket reuse, URL mismatch, retirement and
meta filtering. Native inherited-policy routes cover initial about:blank/srcdoc
and data frame commits. Remaining frame policy work includes applying child meta
restrictions on inherited documents, retaining policy across same-document stream
epoch changes, trusted BFCache reactivation, and clearing response tickets on
page retirement using the new cleanup helper. Policy source generation does not
complete WorkerService creation or the asynchronous agent I/O backend.

### Independent worker transport and parent services — source continuation

Subsequent source generation replaces the pending WorkerService foundations above:
`agent_ipc.rs` and `agent_process.rs` provide a separate attested AgentId child
protocol, paused admission, bounded command/event queues, independent broker
cancellation and retained teardown ownership. The verified worker executable entry
accepts dedicated control/service descriptors; no synthetic PageId is used.
`agent_service.rs` now routes native Window/Agent creation to that factory (or the
explicit in-process native factory), using parent-authorized script loading and
exact document revalidation before admission. Native host bindings install the
WorkerService backend.

`agent_platform_service.rs` owns asynchronous I/O tasks through cancellation,
joining and retryable final cleanup. `agent_io.rs` supplies actual streamed HTTP,
WebSocket and partitioned IndexedDB operations under the admitted AgentId.
Response/socket resources have bounded tables; canceled late results are disposed.
Native Window and Agent transaction tokens use separate authority namespaces;
agent retirement aborts retained transactions after accepted I/O tasks join.
Regression sources cover operation retirement, cleanup retry and transaction-token
isolation. A native creation/fetch/message/close integration regression is generated.

Blob URL registration/resolution/revocation now uses parent-generated random
identities and native origin/partition/policy ownership. Worker script loading and
existing page/worker blob fetch, XHR and image lookups share the registry. Exact
creator retirement revokes future lookup, while already retained resource copies
survive URL revocation. Source regressions cover cross-origin/context rejection,
same-origin sharing, revocation and document retirement.

These are generated sources, not validation results. Remaining W work includes
service-worker registration/update/controller and navigation/fetch interception;
shared-worker prefetch deduplication; frame referrer, child-meta and stream/BFCache
policy reconciliation; page transaction retirement; fetch mode/redirect semantics
across the platform wire; and standalone RSS/audit/resource-budget reconciliation.
No build, test, formatter, generator, deployment or conformance run has occurred.

### Service-worker source continuation (generation still open)

Generated `service_worker_wire.rs`, native `service_registry.rs` and
`v8_bindings/service_workers.rs`: parent registration jobs, native install/activate
completion, waiting versions, scope/Service-Worker-Allowed checks, controlled
native document keys, registration promise polling, native lifecycle/fetch event
correlation and owned timeout retirement. Native service agents use the real
zero-client AgentRegistry service admission APIs. Generated the initial browser
container/registration/worker wrappers and skipWaiting/clients.claim entry points.

Navigation interception now enters the existing isolated page broker and an
in-process navigation wrapper. Native streamed fetch/image requests carry a
Creator snapshot captured from their actual live V8 binding; parent validation
uses exact root/frame generation and stream epoch, not authored initiator URLs.
Author context-bridge requests have creator identity overwritten from the native
calling realm. Controlled requests dispatch actual native FetchEvents; returned
responses feed the existing stream pipeline. Added an integration regression
source for registration lifecycle, FetchEvent response, actual ContextService
streaming and stale creator rejection.

Service-worker generation remains incomplete: service/client postMessage with
structured-clone provenance, client enumeration/open/navigate/focus APIs,
registration worker-global exposure, prototype/IDL coverage, update imported
module comparison/cache semantics, durable registration/cache storage,
registration/unregister cleanup, concurrent job ordering and complete activation
error handling, shared navigation cancellation budgets, frame/paint subresource
routing reconciliation, and navigation preload still need implementation. No
service-worker success or conformance result is claimed. All sources and tests
remain unbuilt and unrun under the full-plan generation gate.


### Native service messaging and cached-document authority continuation (source only)

- Service messaging now uses native `MessageSource` Client/ServiceWorker snapshots, native structured-clone payloads and registry-routed transferable ownership. Container polling exposes queued native messages and authentic ServiceWorker sources; ExtendableMessageEvent carries native Client/ServiceWorker sources. Outgoing jobs have count/byte reservations and failed admission releases transferred endpoints. Completed service threads are reaped outside the ownership mutex.
- Native park/restore/discard wire operations carry parent-issued one-use cached-document capabilities; agents cannot invoke them and capability Debug is redacted. Host retains the token until successful restoration and discards on executable destruction. Runtime commits the parent URL before authority restoration and isolate resume, preserves rejected destination ownership, and attempts incumbent rollback. Parent security-origin/CSP snapshot integration is coordinated separately; URL alone is insufficient for opaque/inherited authority.
- Service cleanup is integrated into page-agent retirement with attempt-all error propagation, before agent/hub locks. No builds, tests, generators or runtime commands were run.
- Still open: suspended worker/SW policy and event ownership through BFCache; complete client APIs/worker container consumption; service update job FIFO and activation/in-flight event synchronization; imported-script/cache update semantics and durable service/cache storage; navigation cancellation/deadline integration; exact frame/paint fetch provenance and full request-mode/redirect semantics. These remain implementation obligations, not passing dispositions.


### BFCache worker gate continuation (source only)

- AgentProcess now has an out-of-band suspension request/status contract. Native thread runtimes acknowledge only at a task checkpoint and termination wakes the suspended condition. Standalone agent IPC version 2 carries priority/coalesced suspension controls and epoch-correlated acknowledgement, separate from author messages; pending commands remain queued.
- AgentRegistry tracks frozen exact document owners and computes worker reachability from runnable Window owners, including shared siblings and nested agents. Connect/release/retirement recompute suspension. Parent history parking waits at most 500 ms outside registry/hub locks for quiescence and resumes on rejection. Successful restoration resumes after parent security identity is restored. Cached snapshots retain exact owner lists; eviction attempts service/blob/agent cleanup.
- Added regression source for checkpoint acknowledgement, termination while suspended, queued-command retention, priority control mailbox admission, and shared/descendant ownership propagation. Navigation bookkeeping preserves native registered document origins rather than recreating BFCache opaque/inherited origins from URL.
- Remaining lifecycle issues: timer active-time accounting while suspended; cached-owner authority for already admitted service jobs/blob resolution/client message queueing; retry ownership if eviction cleanup fails after consuming the cache token; service activation and registration job concurrency. No execution or conformance result is implied.


### Cached resources and service-client continuation (source only)

- Worker/page timer accounting now excludes native suspension intervals. Cached security records retain original per-document native worker authority for already admitted service jobs, blob owner retention and service-client message queueing; active-only Create/Import/network admission remains separate. Hub retained-document lookup does not revive author WindowStatus.
- Eviction consumes message authority while marking its parent security snapshot evicted; failed SW/agent cleanup retains exact owner lists for retry, rejects restore/resource admission, and is retried before the next park. Page retirement remains the final attempt-all cleanup path.
- Native service `Clients.get` and `matchAll` use actual native Window/agent inventory, context partition/origin filtering, controlled-client selection and unforgeable IDs. Branded JS Clients/Client/WindowClient wrappers, dictionary conversion and IDL source are present. Native WorkerNavigator now hosts ServiceWorkerContainer so worker clients consume real queued service messages. Added source regressions for client lookup/forgery/receiver validation, cached authority separation and timer origin accounting.
- Outstanding: precise focus/visibility ordering, native focus/navigate/openWindow grants; inherited worker controller assignment; secure ancestor exposure; service activation/in-flight reservation and FIFO update jobs; full durable registration/CacheStorage, update script graph/cache behavior, fetch cancellation and comprehensive request-mode/redirect/subresource routing. No test/build/generator/runtime execution has occurred.

### Continued W generation: controlled agents and event cancellation

Source now carries parent-derived service-controller registration identity into each non-service agent bootstrap. Worker entry scripts, non-service worker imports, and agent HTTP/image requests consult that controller through the native registry. CSP, creator lifetime, strict script MIME decoding, size bounds, credentials and network fallback remain on the original native loading path. Service-worker installation/import loading bypasses controller interception. Fetch client IDs are generated from the live native owner instead of copied from request metadata.

Service fetch response completion now differs from event lifetime completion: an owned background task holds the version lease and bounded completion inbox through `waitUntil`. Concurrent event completions route by event ID and stage. Caller cancellation can send native event-local `CancelTask` and return AbortError while cleanup remains owned. Native regression source covers pre-admission/mid-flight cancellation, another event surviving cancellation, response-before-waitUntil, and service-controlled script MIME acceptance/rejection.

**Generation and qualification remain incomplete. No build, tests, formatters, generators, deployment or conformance run has been performed for these changes.** Remaining W implementation obligations include durable registrations and CacheStorage; complete imported script update graphs/updateViaCache; caller deadline propagation through navigation/response streaming; precise service client focus/visibility and notification interaction grants; unregister/idle version cleanup; ancestor secure-context exposure; full response-type/redirect fidelity and navigation preload. Existing G fixture qualification and full-manifest evidence still require the later global execution gate.

### Native persistent service backend and idle teardown source

Added a parent-only service record API to the existing profile-owned `ContextStore`, with memory and redb implementations, compare-and-replace revisions, separate table, partition/origin isolation and bounded records. No author storage RPC can select this table. Source regressions cover stale writes, schemeful partition separation and actual profile reopen. BrowserContext derives the stable service-store partition from native creator authority while profile/store ownership supplies context isolation. Registration restoration and CacheStorage consumers are still pending; the backend alone does not establish durable service-worker behavior.

Unregistered service registrations now retain live controlled clients and admitted event leases, then tear down real idle versions when the last owner retires. Failed native teardown retains version ownership for later cleanup. Added regression source for unregister while controlled and final-client teardown. All changes remain unbuilt and unrun.

### Cache transaction and native wire source

Added `storage_cache.rs` and `oryn-js/cache_wire.rs`: native Request/Response snapshots, Vary/search/method matching, atomic batch validation, insertion order, DOMString cache names, and deleted-name/retained-handle identity. Transactions use profile service-record CAS with bounded retries. Native ServiceRequest::Cache routes exact creator authority through BrowserContext to the same store, with no author-selected profile path or origin. Regression sources cover atomic rejection, language variants, query/fragment handling, unpaired name surrogates and retained deleted-cache handles.

The Cache/CacheStorage V8 wrappers, asynchronous native Response-body consumption, add/addAll network flow and deleted-handle reclamation remain generation work. The wire/backend is not a completed browser Cache API. Durable service registration restore/update-graph integration remains open separately. No execution performed.

### Integrated cache bindings, registration restart and cancellation generation

Native Cache/CacheStorage wrappers now connect Window and independent worker realms to the typed cache service. Receiver identity, Request/Response snapshots and body consumption use native records. Pending bodies are bounded and polled; add/addAll use the native fetch callback, validate all requests before network admission and cancel owned fetch/body work on batch failure. Interface source was added without running the IDL generator. Profile-shared handle coordination keeps a deleted cache alive while another context owns it, then reclaims tombstones on native owner retirement; failed cleanup remains retryable. Added native worker body/name/receiver and cross-context profile reclamation regression source.

Service process startup now waits for registry admission plus version/snapshot publication. Imported script resource maps are retained natively and checked during updates. New persistent registration source stores active/waiting admitted script graphs, replaces saved runtime IDs with fresh native admission, restores current limits/partition, and reinstalls activated workers without replaying installation. Matching navigation paths lazily restore the namespace using their authorized network broker. The profile-reopen regression restores a real service worker and cached import with a network-denying fixture.

Service navigation waits now carry page/request-specific cancellation. Worker route revocation, broker cancellation and page retirement notify those waits; Window HTTP-request abort and parent shutdown feed the same event-local service cancellation path. Regression sources cover foreign-page cancellation rejection, navigation cancellation and a pending Window HTTP fetch abort.

**Still source-only and incomplete overall.** Remaining review/implementation includes full streaming response transport/navigation preload, cross-context registration update ordering and HTTP-cache/updateViaCache behavior, secure-context ancestry and precise client activation/focus/visibility, per-object cache GC before realm retirement, and remaining conformance fixture qualification after the global generation gate. No builds, tests, generators or conformance execution have run.

### Secure exposure and cache fidelity source continuation

- Native realm secure-context snapshots now conjoin frame ancestry before binding installation; independent agents inherit parent-admitted secure state, not their script URL alone. Native-only publication records immutable exact document lifetime authority in the parent; absent authority fails closed, and retirement clears it independently of service registry availability. Added stale generation, forged HTTP publication, insecure-worker, and nested secure-frame ancestry regression source. Context protocol 18 and agent IPC 3 carry the schema.
- Cache response snapshots now preserve null versus empty-stream bodies and redirected metadata. Added native binding regression source. No build, test, generator, or WPT execution occurred.
- Remaining source obligations include opaque precursor secure-context derivation, streaming service response/cancellation, navigation preload, full updateViaCache HTTP caching, durable cross-context registration coordination, and cache object reclamation/iterable close review. These are open implementation work, not qualified results.

- Follow-up source: frame commits now receive the native response URL before binding installation, allowing sandboxed HTTPS precursor secure exposure while preserving opaque origin and conjoining parent ancestry. Regression source covers both HTTP and HTTPS parents. Top-level native opaque-origin installation remains under review. Cache.addAll uses shared Web IDL iteration conversion with original-exception preservation and IteratorClose; regression source covers observable conversion ordering.

### Demand-driven service response generation

- Added native service response reader sharing the real ReadableStream reader algorithm, bounded chunk emission, exclusive reader ownership, typed byte validation, explicit demand, and source cancellation. Native fetch events now emit separate headers/chunks/end while waitUntil retains its independent lifetime.
- Added parent-owned NetworkOperation producer, bounded per-event mailboxes, exact task correlation, cancellation guards, version leases, and retained cleanup tasks. Navigation, Window fetch/image, and independent worker fetch/image consumers now use the streaming path; script compilation remains a deliberate complete-body consumer.
- Source regressions cover headers before EOF, no body without demand, poisoned public methods, exact chunk bytes, invalid byte chunks, waitUntil after EOF, and body cancellation. Context protocol 19 and agent IPC 4 carry the additions. All source is unbuilt and untested; navigation preload and HTTP cache update policy remain open.

- Root native authority continuation: native-only GetNativeDocumentAuthority validates admitted response policy URL and returns parent opaque identity plus secure precursor. Fallible root context construction consumes it before binding installation, using a separate remote opaque namespace to prevent local ID collisions. Author context JSON denies access. Added opaque inheritance/collision/forgery/secure sandbox regression source; Context protocol is now 20. No execution.


### Shared-profile service lifecycle and native authority source (2026-09-16)

Generated, not built or executed:

- Native root document authority now transports the parent's actual opaque origin identity and committed secure-context state before V8 bindings are installed. Author bridge requests cannot request this authority.
- Cache wrappers use weak native identities and deferred GC release hints; the parent releases exact cache leases and reclaims deleted caches only after the last realm/operation owner retires. Null response bodies and empty streams remain distinct.
- Service update resource loading enforces `updateViaCache` for entry and imported scripts, the 24-hour revalidation override, native service request headers and entry redirect rejection. Request cache directives now bypass stale local cache reuse.
- Persistent profile service jobs serialize by native partition/origin/scope. Revision refresh stages restored workers before publication; per-registration fingerprints prevent a stale context overwriting a sibling's registration.
- Profile-wide controlled-client and event leases retain BFCache documents and streaming/waitUntil work. Activation is exclusive with profile update jobs, denies new client/event admission during transition, and waits for sibling clients/events unless the applicable skip-waiting rule permits clients. Native release watchers enqueue activation work outside the profile lock. Regression source includes stale sibling writes, retained controllers, last-client release, event retention, activation races and scope isolation.

Still open in this owned source area: navigation preload routing and bindings; review of cross-context notification/retry paths and complete scope/update edge semantics; final integration/type/build/test and actual WPT qualification. No conformance result is inferred from generated source.


### Navigation preload source integration (2026-09-16)

Generated native `NavigationPreloadManager` methods, persisted enabled/header state, exact registration authority checks, and activation-lifecycle-safe configuration. Parent navigation creates a separate authorized asynchronous network operation with the native preload header; only the exact live service agent/event can consume it once. `FetchEvent.preloadResponse` uses the existing native fetch response/stream machinery and a private AbortSignal, so public fetch/Response methods do not substitute for the operation. Pending and consumed operations retain cancellation/retirement ownership; disabled preload resolves undefined. Added end-to-end source regressions for activation enabling, streamed response consumption, native header/ID correlation, one-use/foreign task rejection, cancellation before admission, and disabled preload avoiding a request. Protocol was advanced to context 24 / agent IPC 6; root subsequently owns context 25 for classic-script error taint.

Remaining audit work includes preload startup overlap with cold worker restoration, redirect referrer and response policy fidelity, complete cross-context state-change notification/retry behavior, remaining service scope/update semantics, and actual generation-gate validation. All changes remain unbuilt and untested.


### Follow-through source audit (2026-09-16)

- Preload transport admission now precedes cold service restoration. A prepared operation has no agent principal until bound to the selected real service agent/event; retirement and failed restoration drop/cancel it.
- Service resource maps now persist SHA-256 of raw entry/import representations. A BOM-only byte change triggers update even when decoded JavaScript is identical; unchanged subsequent bytes retain the installed version. Native `skipWaiting` state is retained in durable bootstrap metadata.
- Profile registration changes now notify subscribed native contexts after releasing persistence locks. Restoration/activation wake native page event checkpoints. Unregister/re-register creates a new registration identity while old controlled documents retain the old unregistered version.
- Service wrapper identities and the ready promise use weak native handles. Private traced wrapper relationships preserve reachable controller/worker/preload identities, without a native Global rooting unreachable self-capturing listener cycles. Generated GC/identity regression source accompanies the change.

These are source-generation results only. Agent IPC is 7; context protocol is maintained by root (25 at this audit). Full source review, build/test repair and WPT qualification remain behind the global generation gate. Cross-context transient installing-state visibility and activation retry/error propagation still need review; existing notes are not a qualification claim.


### Retryable native retirement source (2026-09-16)

AgentRegistry now retains exact service retirement receipts (`Live`, `Pending`, `Retired`) across detached termination failures and unrelated subsequent cleanup. Only actual completed termination produces `Retired`; service owners acknowledge it before releasing Version ownership. Native acknowledgement does not run unrelated teardown afterward. Prepared restoration failures retain orphan IDs for retry, and a failed old-version teardown leaves the replacement waiting instead of stranding it in activating. Failed persistence remains owned and retried before restoration can replace native state. Added regression source for failed termination, unrelated retry, repeated retirement, foreign ID denial and final receipt acknowledgement. No tests or builds ran.

Per-registration native lineage now separates retained unregistered clients/events from replacement registrations sharing a URL scope. Cached-only `updateViaCache=all` checks no longer postpone the 24-hour network revalidation clock. Context protocol is root-owned at 26; agent IPC remains 7.

Orphan restoration teardown now publishes retained ownership before calling the broker, admits only one receipt consumer per AgentId, and releases the admission guard on failure or panic. Cleanup attempts every orphan. Version teardown likewise resets its admission flag on unwinding and retains ownership when the parent context cannot supply terminal acknowledgement. Added source regressions for reentrant duplicate cleanup rejection, no registry lock during termination, panic recovery, retry and completed-operation idempotence. Execution remains deferred.

Navigation preload redirects now retain the parent navigation referrer policy, apply response policy updates across hops, and reduce the already-admitted referrer source instead of substituting the service-worker script or redirect destination. Regression source covers cross-origin stripping, later permissive policy, and persistent no-referrer absence. Shared HTTP ByteString encoding remains under transport review; no execution performed.

Shared Rustls HTTP transport now explicitly encodes header ByteString scalars into original bytes and decodes raw response bytes losslessly, preserving duplicate fields. It rejects non-byte scalar values and invalid HTTP header syntax. Added source regressions for Latin-1/obs-text round trips, duplicate custom/Set-Cookie headers, U+0100/supplementary scalars, CRLF and NUL; preload parent/persistence validation also rejects non-byte scalar metadata. URL/body encoding is unchanged. This closes the header transport source gap noted above; build and runtime verification remain deferred.

### Logical service version identity generation (2026-09-16)

Added native `ServiceVersionId` independent of process AgentId. JS ServiceWorker wrappers, controller comparison, lifecycle comparison and postMessage requests now use the logical token; parent dispatch resolves it only within current native origin/storage authority. Durable active/waiting version slots preserve the token across process restarts; legacy records derive a stable migration identity. Added source regressions for native restart preserving version identity, wrapper identity across changed process ID, invalid/duplicate/orphan durable tokens. Context wire advanced to 27 (root subsequently reserves 28); agent IPC to 8. Transient cross-context installing observations and shared process/client/port routing remain under generation and are not claimed complete. No builds/tests.

Profile `ServiceVersionDirectory` now reserves logical ownership before native process admission and publishes authentic lifecycle snapshots. Native registration query paths merge same-profile/origin installing observations; observer registration IDs are retained by lineage into durable restoration. Every existing snapshot-to-local-process lookup now additionally checks logical version identity. Directory regression sources cover duplicate admission, unpublished reservations, partition/origin isolation and stale owner-lease replacement. Remote process, cross-hub transferred-port and cross-context client routing remain under generation; observation does not fabricate local execution authority.

Generated native cross-hub MessagePort migration and forwarding with explicit bridge endpoint ownership, nested queued-transfer remapping, preflight capacity/entropy, FIFO/backpressure, close propagation, and caller-retained nested bridge ownership on partial failure. Parent profile admission checks exact live documents/agents, native origin and stable partition under ordered context locks. Supervisor has retained thread ownership, shutdown/join and retryable teardown-only poison recovery. Source regressions cover nested reverse transfers, stale source capabilities, profile isolation, stale roots, queue failure after a successful nested admission, and actual parent forwarding/close. Remote installing-worker postMessage now reaches the authentic owning service process through this bridge and target-bound capability validation. Shared restored-active fetch/control/client routing remains under generation; no builds or tests ran.

### Profile service lifetime and client routing continuation (source only)

- Native buffered/streamed fetch and navigation preload now resolve logical service versions to the actual profile-owned process. Sibling registration views retain the same version identity without launching another process.
- Unregistered cleanup holds an exclusive profile lifetime lease through teardown; another context's controlled clients and extendable events prevent retirement. Remote views are reclaimed when no local registration/controller references them.
- `clients.claim`, focus and navigation resolve authenticated profile client identities and native target contexts. Controller comparisons use logical version IDs; a foreign process number is never reused as local authority. Native activation consumption validates the originating document, stable profile partition, native origin and expiry before consuming a grant.
- Restoration now retries explicit pending version admission using a bounded native directory change wait after releasing persistence locks. Retired owner fetch paths can re-admit the saved graph while preserving its logical version identity.
- Regression source covers profile adoption without duplicate native processes, remote fetch dispatch, remote activation consumption, retirement/client/event exclusion, restored import MIME/type rejection and directory publication notifications.
- This remains unbuilt and untested. Remaining generation obligations include full owner-context teardown/restart concurrency, all cross-context claim/action lifetime races, and complete transferred-message/fetch cancellation integration coverage. No new conformance disposition is claimed.

### Shared backing and engine wait integration (source generation only)

- Parent native agent-cluster membership, immutable shared-backing descriptors, SCM_RIGHTS attachment transport, mapped atomic length, absolute/delta growth and exact waiter ownership are generated. They use actual shared OS storage, including descriptor-preserving buffered IPC reads.
- `v8_shared_memory.rs` now matches the pinned engine mapping ABI. Each successful native allocation/open lease is consumed exactly once, including mapping failure, and is retained until engine backing destruction. Native identity maps retain weak references. Growth is installed before the wait delegate.
- `v8_shared_wait_bridge.cc` and `v8_shared_wait.rs` implement synchronous bounded polling and native `Promise::Resolver` completion for `Atomics.waitAsync`. Native environment retirement cancels pending waiters; lost poll responses retain cancellation ownership. No authored Atomics wrapper participates.
- Cached document security now preserves committed isolation policy. Page retirement revokes root, frame and cached-generation shared mapping capabilities while live peer mappings retain their actual backing.
- Regression sources cover parent mapping ownership and cluster rejection, growth/wait ordering, native callback release, lost-response cancellation, page retirement and cached isolation restoration. **None have run.** Engine patches, C++ ABI and Rust integration remain unbuilt.
- Remaining source obligations: native SAB constructor/exposure and complete COEP/CORP/Permissions-Policy enforcement; dedicated/shared/service agent admission; queue-to-receiver backing lease handoff across all message routes; native async wait environment scheduling/reachability edge cases; shared Wasm allocation/growth integration and process-placement attestation. Cross-origin isolation is not exposed based only on the partial policy parser. The full W/G generation gate is not closed.

### Shared backing lifetime and native clone integration (2026-09-16, source only)

Generated the following integration and regression sources without execution:

- Both ordinary SAB and Wasm mapping paths now receive the native wait environment. A process-wide weak mapping identity table checks a native-deleter-controlled live flag, preventing a waiter retaining an old lease from authenticating a reused mapping address. Physical mapping capabilities survive creator-document retirement; new allocation/open/delivery still require current native authority.
- Native V8 serialization resolves actual mapped backing identity, deduplicates aliases, and authorizes the current serialization realm. Wire delivery uses committed sender offers, parent queue leases, and exact-recipient one-use delivery claims. Deserialization maps the native descriptor, including Wasm backing kind, and uses V8's real shared-buffer delegate. Local cross-realm cloning independently authorizes its recipient.
- Send guards abort rejected/unsubmitted offers. Successful independent-worker event enqueue explicitly transfers custody to the native ordered mailbox. Receiver guards release unused claims. Cross-hub migration rejects shared backing hidden inside nested transferred-port queues before mutation.
- `waitAsync` resolves the scheduler from the current native context; a backing's creating realm no longer selects new asynchronous waiter ownership. Synchronous wait/notify rely on retained physical capability. Added the early-notify/current-context regression source and mapping-live/release sources.
- Native `crossOriginIsolated` getter and constructor admission use exact Creator plus private native ancestry. Parent validation compares frame ancestry against committed native parent records. Worker creation binds the creator first; dedicated process admission inherits a compatible native cluster, and Shared/Service agents receive separate clusters. Worker response policy snapshots now retain enforcing COEP and Permissions-Policy denial.
- Added `queued-lifetime.https.html`, its worker, same-origin frame and enforcing header fixtures for aliases, remote growth, real shared writes and surviving-frame waiters. These are source fixtures, not process-placement evidence.

Remaining generation obligations: complete Permissions-Policy allowlist/container inheritance and cross-origin isolated frame cluster behavior; full COOP browsing-context-group/opener reconciliation; bounded/interruptible transport for each synchronous waiter RPC (the engine's local polling budget alone is insufficient); sender-mailbox teardown/adoption ordering and remaining no-consumer delivery cleanup audit; full native worker isolation compatibility/dedup semantics; coverage attribution and actual process-placement attestation. Engine factory/patch qualification remains parent-owned. All source, including these tests, remains unbuilt and unrun; no WPT disposition or conformance result changed.

Follow-on policy/ownership generation (same no-execution gate):

- Context protocol is now 38 (agent IPC remains 12): both native frame policy commit requests require immutable `container_allow` and resolved `container_source`. Parent admission checks committed ancestry, response allowlists and container self/src/origin/wildcard delegation. Cross-origin isolated frames use independent clusters; same-origin frames may inherit. Explicit root declarations survive BFCache security restoration and worker response snapshots. Added allowlist/container regression sources.
- Wait callbacks now receive the actual entered native context. A different executing realm acquires its own parent holder alias; the waiter retains that alias and the original mapping independently. Thus creator retirement does not revoke a surviving realm's new waiter, while executing-realm retirement still cancels it. The early-notify source regression now uses distinct creator/executing identities.
- Shared sender Offer/Commit failures occur before ordinary transfer buffers detach. Guards retain abort responsibility until successful wire construction and parent/mailbox custody. Native Window source-resolution failure and retired-recipient early return now have delivery cleanup guards.

Still open: full COOP browsing-context-group/opener reconciliation and response policy edge cases; bounded/interruptible shared waiter RPCs with client-known cancellation ownership; full failed-transfer atomicity beyond shared-offer admission; ordered final IPC mailbox handoff/discard proof and transport send-failure lease rollback; process-placement evidence and full qualification execution. The added Permissions-Policy handling is focused on cross-origin-isolated and still needs specification/WPT validation; no browser-wide Permissions-Policy completeness is claimed.

### Native browsing groups and interruptible wait admission (source only)

- Context-owned browsing-group tokens now distinguish ordinary native Window proxy relationships from session-issued automation references. Committed COOP/native-origin changes can sever the opener and invalidate captured author proxies; cached document security also snapshots/restores the group identity. Shared document clusters use the actual group token plus native origin. Initial auxiliary documents inherit creator policy with the native sandbox popup-escape exception.
- Context protocol **40** and agent IPC **13** add atomic wait admission and cancellation. Native `WaitAdmissionId` combines an unguessable lane with a checked sequence. Parent admission owns receiver authorization/holder acquisition and atomic compare/enqueue under one registry lock. Cancellation is meaningful before admission, after an unknown reply, and after terminal polling. Compact completed-sequence floors plus bounded holes retain replay protection without one tombstone per historical wait.
- `shared_wait_custody.rs` owns four fixed native broker lanes and at most 256 pending wait tickets. Begin, Admission, Poll and Cancel on the V8 thread perform no broker IO. They retain mapping/transport ownership while replies are pending; failed cleanup remains registered for bounded retries and explicit drain. There is no detached per-request thread or timeout that discards custody. Immutable initial classification remains separate from subsequent notification/timeout outcomes.
- Native Agent cleanup permits exact cancellation/release after execution is revoked, drains wait custody, and attempts backend cleanup even when wait drain fails. Standalone parent termination already retires its real native principal before closing IPC and killing the process. Lost page IPC cleanup remains retained rather than reporting false success; process teardown and parent retirement remain authoritative.
- Added source regressions for admission cancellation before delivery, replay without duplicate holders, lost cancellation acknowledgement/retry, late reply after interrupted admission, early notification with asynchronous classification, bounded sequence gaps, long-run tombstone compaction, descriptor/owner forgery and retirement preserving another physical mapping.

None of these sources has been compiled or executed. Remaining full W/G obligations include native COOP newer-policy/redirect semantics and frame-origin popup authority; final mailbox/IPC shared-lease transfer and send-failure cleanup; remaining complete structured-transfer atomicity; isolation compatibility during SharedWorker deduplication; end-to-end transport teardown races, fixture placement attestation and qualification. Root retains pinned engine patch/build integration. The global source-generation gate is still open.

Frame-origin popup continuation (context protocol **41**, agent **13**, source only):

- Native OpenPage dispatch overwrites creator authority from the entered realm and resolves authored relative URLs against that realm's native base. Parent creation validates exact frame generation, uses that frame's policy/origin, and records its opener frame token before exposure.
- Frame policy binding returns the parent registry's native origin identity. Native realm descriptors and V8 security tokens adopt that identity before document execution. Opaque frame identities are inherited as broker identities rather than reparsed `null` or top-level origins.
- Committed iframe sandbox snapshots and inherited ancestor sandbox restrictions now participate in frame worker/popup policy. Changing a live iframe attribute cannot replace a committed policy snapshot.
- Source regressions cover cross-origin frame popup origin/policy/opener target, retired creator denial, immutable sandbox restrictions and opaque frame/popup identity agreement.

Still auditing atomic popup admission/rollback, newer COOP modes, opener disown semantics and response/redirect handling. No browser or WPT qualification was executed.

Popup admission now revalidates the exact creator generation and native origin while holding the parent page-registry lock through insertion. Failed admission removes the unexposed origin/policy/opener-target records; a regression source covers retirement between policy capture and registration. This closes that specific publication race. Newer COOP/disown-opener behavior, policy commit timing and final shared-message custody remain open generation work.

Response and payload custody continuation (source only):

- Context IPC captures native response ownership before worker-route turnover. Encoding retains a publication status; an oversized success replaced with an error no longer sends an unrelated native descriptor. Unpublished ports, Broadcast subscriptions, shared holders/wait admissions and worker connections are compensated after the socket writer lock is released.
- Failed compensation keeps compact capability metadata in the owning context for retry. Further admission is bounded by cleanup capacity. Native cleanup distinguishes Window and Agent principals; it never substitutes a PageId for an independent worker identity.
- Standalone agent service replies now retain their response through both JSON and FD publication. Write/attachment failure invokes native backend compensation and marks the process mailbox failed. Native backend drain also attempts retained response cleanup.
- Native serialized payloads own transferred ports until deserialize/export commits. Failed decode and discarded local message queues close unreceived endpoints; queue destruction releases nested payloads outside endpoint RefCell borrows.
- Added response replacement/channel ownership and undelivered-port regression sources. No tests were executed. Remaining source audit includes response retry admission synchronization, complete transfer transaction atomicity, SharedWorker isolation compatibility/dedup, newer COOP/disown-opener and loading commit timing, and G placement/qualification coverage.


### Response acknowledgment and retry ownership continuation (source only)

- Context protocol **42** and standalone agent IPC **14** now require correlated native receipt acknowledgment after response decoding and native FD receipt. Kernel write success alone no longer releases parent response capabilities. Rejected, mismatched, disconnected and oversized-replacement responses retain or reclaim parent custody under the actual page/agent principal.
- Multiplexed context publications retain the existing dispatch permit until receipt; disconnect atomically closes publication admission and drops grants outside the publication mutex. Native callers wait for receipt publication before using delivered responses. Child IPC teardown has an independent shutdown socket so it does not need the blocked request writer lock.
- Failed response cleanup is represented by an explicit in-flight batch. Reentrant teardown runs outside locks; concurrent admission sees the batch, and unwinding restores the current and unattempted compact capability snapshots. Native context retirement attempts response cleanup after agent and service teardown, preserving failures for retry.
- Regression source covers accepted/rejected/mismatched receipts, disconnect/write failure, permit retention, unknown/replayed acknowledgments, reentrant cleanup and unwind preservation. No builds, tests, formatters, generators or conformance runs have executed.
- Remaining W/G generation reconciliation still includes transferred-message transaction atomicity and orphan raw-port cleanup, SharedWorker isolation compatibility at shared identity reuse, remaining COOP/opener semantics and commit ordering, and G physical-placement/negative-control qualification sources. Existing engine/policy/worker packages remain unqualified until the global source-generation gate is satisfied and subsequent execution completes.


- Response custody additionally captures `ScriptLoadReserved` against the exact original script registry, independent of later broker routing. Receipt acknowledgment disarms the guard; unpublished reservations cancel even after context retirement. Cleanup requests remain admissible during failed-cleanup recovery. The multiplexed response reader is now explicitly retained and joined after independent socket shutdown.
- Raw transferred port custody now begins before remote Window source resolution and persists until native event dispatch; malformed/retired/undelivered payloads close their transferred capabilities. Native SharedWorker reuse rejects an effective committed isolation-capability mismatch using the same policy calculation as cluster admission. Regression sources added; execution remains deferred.

- Context protocol **43** adds native-only exact-frame `Window.origin` authority. Parent validates source document generation/epoch, target frame lifetime, browsing-group identity, and native origin equality under one registry snapshot. Distinct opaque origins do not compare equal because both serialize as `null`. Cross-origin property access now raises `SecurityError`, while the proxy remains usable for permitted cross-origin operations. Native and JS regression sources generated; no execution.

- Wire structured serialization now retains a temporary native transfer plan until complete wire validation and atomic parent port admission succeed. A rejected port batch or oversized/invalid wire record leaves the source ArrayBuffers and MessagePorts attached. Immediate V8 detachment follows complete revalidation without author callbacks; queued serialized records retain no source JS handles. Regression source covers combined buffer/port admission rejection. Unbuilt and unrun.

- Context protocol **44** adds native root opener disowning. The `opener` accessor forwards null assignment to exact document-lifetime parent authority; the parent descriptor and recorded frame target are cleared without changing the browsing group. Reinitialization cannot resurrect the launcher-supplied opener. Non-null assignment remains an author property replacement, not native authority. Native/JS source regressions cover stale and foreign principals, idempotence, group preservation and reinitialization. Agent IPC remains **14**.
