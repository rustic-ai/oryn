# Browser implementation readiness and cutover plan

Reviewed September 13, 2026 against working tree based on
`42cc4ae02ecc96a99c4c652201b0b61a5cd5b8c0` (dirty, including untracked source).
This is a source review and local-test record, not signed release evidence.
No runtime code was changed by this review. Preserve existing user changes;
do not reset, overwrite, commit or clean them as a prerequisite without authority.

Companion to [Browser architecture and migration](BROWSER-ARCHITECTURE-AND-MIGRATION-2026-09-13.md).
This plan makes architectural cutovers actionable. It does not claim every
remaining Core API has a finished algorithm design or that conformance is close.
Work packages below are proposed, not implemented by writing this document.

Implementation progress is recorded separately in the
[implementation ledger](BROWSER-IMPLEMENTATION-LEDGER-2026-09-13.md); the review
record below remains historical rather than being relabeled as completed work.

## 1. Review verdict and corrections

The direction is sound: retain V8, native DOM, generated bindings, parser,
Stylo/layout adapters and parent broker; repair ownership and semantic boundaries.
The earlier plan was not ready for a direct multi-page rewrite. These corrections
are prerequisites, not optional refactoring:

| Priority | Verified code fact | Required correction |
| --- | --- | --- |
| P0 security | WorkerPage owns one client; PageSession owns one page; router has one optional route; ParentContextService has one source page | Do not merely share a client between PageHandles. Add environment routing, process ownership, cancellation and failure fan-out first. |
| P0 identity | NodeId is arena-local slot/generation; DomRuntime has one document, ID index and focus state | Distinguish arena identity from document ownership; add document-scoped state. `(DocumentId, NodeId)` is not immutable node identity across adoption. |
| P0 tree | Same-origin frame loading imports a Document and appends it below the iframe | Replace this ancestry shortcut with a browsing-context relationship. Audit query/event/range/layout/semantic consumers together. |
| P0 origin | RealmRegistry compares origin strings; native SecurityOrigin is a private tuple-string/opaque representation | Consolidate structured origins and cluster selection. Serialized `null` is not an equality key. This is not a demonstrated exploit. |
| P1 reuse | RawV8Host has contexts, frame replacement and incumbent-realm support; WindowProxy and native bridge exist | Extract ownership and generalize existing mechanisms, not a second embedding. |
| P1 agents | WorkerGlobalKind covers dedicated/shared/service globals; creation uses Context::new in the current isolate | Reuse exposure tests but replace page-isolate scheduling/lifetime. Global names do not establish independent execution or registration. |
| P1 forms | PendingDefaultAction::Submit(NodeId) loses submitter; apply_default_navigation reads action/target later and takes a URL path | Add submission stages and owned bodies; preserve existing validation/activation fixes. |
| P1 harness | wpt_session accepts run/shutdown and polls a completion global; upstream requires general window/script/async callback operations | Add general automation transport and a product adapter, not a wrapper around the old runner. |
| P1 inventory | XML/MIME parsing and renderer exist; mapper routes only selected testharness suffixes | Use official dispatch against existing implementations, retaining real engine failures. No new XML parser is required. |
| P1 evidence | Automation trace says page_isolate; build/validators encode v9 | Version process metadata, trace identities and wire contracts atomically. Never relabel old evidence. |

Verified anchors, relative to repository root:

- `crates/oryn-native/src/runtime.rs`: BrowserContextInner, ContextGroup,
  WorkerPage, WorkerPool::{checkout,prepare,recycle}, BrowserContext::create_page.
- `crates/oryn-native/src/host.rs`: RawV8Host, create_realm_with_dom_root,
  replace_frame_realm, incumbent_realm, dedicated_worker_create_callback.
- `crates/oryn-native/src/rust_executable.rs`: RustExecutableDocument,
  load_with_options, drain_default_actions, apply_default_navigation,
  apply_action_with_sensitivity; static and dynamic frame paths.
- `crates/oryn-native/src/{worker,worker_client,context_service,network}.rs`:
  PageSession, WorkerBrokerRouter, ParentContextService and request provenance.
- `crates/oryn-js/src/{realm,scheduler,platform,bindings}.rs` and
  `crates/oryn-js/src/v8_bindings/{window_proxy,validation,form_activation}.rs`.
- `crates/oryn-dom/src/{lib,html,xml,form_controls,option_controls}.rs`;
  `crates/oryn-layout/src/{lib,dom_queries}.rs`; `crates/oryn-render/src/lib.rs`.

## 2. Ownership and interface decisions

Names below describe planned types/modules, not existing public APIs.

### Identity, arena and host ownership

Use existing crates; avoid a dependency cycle or a new omnibus browser crate.

- `oryn-dom`: ArenaId/DocumentId and arena-qualified node references for cross-owner
  use. Keep local NodeId generation checks. Separate allocation/roots from document
  metadata, ID indexes and connectivity. Scope focus appropriately with explicit
  parent-frame focus propagation.
- `oryn-js::realm` or a small sibling module: agent, cluster, environment and
  browsing-context-group IDs beside RealmId. Consolidate Origin here; DOM remains
  independent of browser origin policy. Native can consume these types without
  reversing the current crate dependency graph.
- Native coordinator: authoritative browser-context, group, cluster, process and
  environment registrations. Adapt public PageId; do not reuse it as DocumentId.
  Current ContextGroup is a context-wide page registry, not a standards BCG.
- AgentRuntime: thread-affine RawV8Host, agent scheduler, arena and environment
  registry. DocumentRuntime owns parser/loading and document state, not the isolate.
  PageHandle is a routed public handle, not the process owner.
- WorkerProcessSession: sole owner of WorkerClient, process budget and routes.
  One dispatcher owns IPC reads/correlation; separate page mutexes must not read
  the same stream concurrently. Environment commands retain explicit targets.

Adopting between documents in one arena preserves NodeId and existing wrapper
identity. Do not clone/import a subtree to implement adoptNode. Scope wrapper
lookups by arena and realm as appropriate. Distinguish detached-but-reachable
documents from active environments: navigation revokes authority and cancels
tasks at required lifecycle boundaries, not by indiscriminately freeing old
objects. Test ranges, listeners, custom-element callbacks and observers during
adoption and navigation. Define ID exhaustion as an error, not saturating reuse.

### Attributes

Replace writable String maps with one ordered attribute collection. Records
contain namespace, local name, prefix, value and associated Attr identity.
Expanded-name and qualified-name lookup differ; auxiliary indexes cannot be
independently writable. NodeKind::Attribute and element storage share authoritative
values rather than maintaining unsynchronized copies.

Migrate HTML/XML sinks, clone/import, named properties/collections, selectors,
Stylo input, serialization, controls, dataset/style and mutation records. Read-only
adapters are transitional; removal of direct writes is an exit gate. Test two
namespaces with the same qualified name, preservation of the existing prefix on
setAttributeNS value updates (distinct from Attr-node replacement), insertion
order, retained detached Attr objects, null-namespace reflection
and namespace-sensitive CSS.

### Submission/navigation

SubmissionTransaction roots form/submitter and records initiating environment,
submission source, image coordinates, user involvement and stage. Reuse
validation::{request_submit,check_form} and form_activation. Keep specified
differences between submit(), requestSubmit(), click and implicit submission.

Extract construct_entry_list from generated FormData into a shared algorithm.
Add FormData(form, submitter), FormDataEvent, button/image participation, empty
file entries, dirname, encoding/newline cases and custom-element dependencies in
tested increments. Move selected files from data-oryn-upload-files into native
input state. Author-created File values remain valid web data; neither attributes
nor author values grant access to parent filesystem handles.

Stages: eligibility/validation -> submit event -> entry-list construction,
including formdata callbacks -> spec-ordered parameter resolution and eligibility
checks -> planned navigation -> commit/cancel. This outline does not replace HTML
algorithm steps. Do not freeze action/method/target before callbacks allowed to
affect them. RAII guards clear construction/submission flags and roots on every
exception, cancellation and navigation replacement.

NavigationRequest contains source environment/document generation, target context,
method, URL, owned body/headers, policy/referrer data and supersession identity.
Parent validates grants and destinations independently. GET, POST, dialog and
target selection share the proper algorithm, not a location-assignment shortcut.
Use existing transport and typed PlatformHost requests rather than a new I/O path.

### Placement ADR: recommendation, not approval

Separate explicit execution-isolation policy from WorkerLifecycle. Preserve the
current page-isolated default until a named security/product decision approves
connected-window hosting. Initially qualify the new policy as opt-in; no implicit
environment-variable, WPT-path or pool-availability activation.

First placement policy: one window agent per suitable sandbox process, hosting
that agent's related window realms; no process sharing between independent root
groups. Other agent types get independent execution lanes/isolate ownership;
process sharing, if needed, is an explicit subsequent placement decision.
SharedArrayBuffer backing across agents is a separate memory/transport obligation,
not solved by structured clone or merely creating more isolates.

Parent derives groups/cluster keys, including partition, OAC history and COOP
transitions. A same-origin iframe related to a popup follows its required agent,
not necessarily its containing frame's renderer. Runtime origin checks remain
mandatory even when realms are co-located.

For the new policy, propose capacity counting live processes and max_leases
counting successful process-binding epochs, not popup count. Preserve legacy
semantics. A popup joining an existing agent uses no additional process slot.
When a new required process exceeds capacity, fail explicitly; do not evict live
windows or sever relationships. Before activation, define hosted-environment
quotas using stress/RSS evidence; an unlimited hidden page allowance is not an
acceptable default. Quotas remain part of the placement approval gate.

window.open must synchronously return its initial WindowProxy when permitted.
Use bounded parent authorization/reservation serviced independently of blocked
evaluation. Create the authorized local initial realm before returning, then
schedule navigation. Parent must not synchronously call into the worker whose
evaluation is waiting on that authorization. Named targets, noopener, sandbox
flags and denials follow the same coordinator algorithm.

Closing one page retires its active environment, not siblings. Reset requires all
hosted environments/operations to retire and all retained realm/DOM objects to
be destroyed before acknowledgement. Crash/RSS/timeout invalidates every page in
the process without affecting unrelated processes. Recycling remains reduced
isolation; WPT sessions continue to prohibit recycled mode.

### IPC/broker cutover

v9 authenticates one page session. Stage internal IDs without changing its wire
semantics. The coordinated routing change takes the next unused version (v10 if
still available), with target environment/generation in commands, completions and
broker routing. Keep session, nonce, PID/CDHash, bounded frames and correlations.

Process states: authenticated/unbound -> bound -> draining -> reset/unbound or
dead. Environment states: reserved -> active -> retiring -> retired. Parent alone
authorizes routes. Draining rejects new authority; cleanup has an explicit bounded
path. Cancellation invalidates retained route clones and in-flight completions,
not just registry entries. Supplying another page's IDs never authorizes its
network, context, secret, upload, clipboard or storage operations.

Version WorkerRequest/Response, HostEnvelope/WorkerEnvelope, context/network
provenance, RuntimeBuildInfo, automation trace scope, CLI/MCP readers, signed
build/validators and benchmark readers together. Mixed versions fail closed.
Keep historical v8/v9 evidence and reproduction support immutable.

## 3. Ordered work packages

P0 means prerequisite/security boundary, P1 feature cutover, P2 broader closure.
Every package includes regressions and removal of its superseded runtime routes.
Small commits are encouraged; a package's full exit gate still applies.

| ID / priority | Dependencies | Edit surface and deliverable | Exit evidence |
| --- | --- | --- | --- |
| R0 / P0 | None | Source/build/inventory provenance and symbol-level capability ledger; characterize frame tree, adoption, forms, popup gaps and known crash | Exact failing/passing controls; no signed/source conflation; reconciled inventory |
| R1 / P0 | R0 | DOM arena/document IDs; JS realm/origin/environment types; native adapters, no placement change | Opaque identity, tuple/default ports, exhaustion, cross-arena collisions and stale generations; existing frame/host tests |
| R2 / P0 | R0 | Ordered DOM attributes; HTML/XML, binding, layout and serialization consumers | Namespace/order/Attr identity matrix; direct writes removed; DOM/JS/layout suites |
| R3 / P1 | R1, R2 | Rooted submission, native file selection, shared entry list, body-carrying navigation and private host entry | Callback/cancel/GC/reentrancy/submitter tests; POST echo verifies method/body; complete signed form families |
| R4 / P0 | R1, R2 | Document-scoped agent arena; frame association registry; remove iframe-to-Document child edges | childNodes/query/event/Range boundaries; contentDocument/adoption identity; explicit layout/semantic traversal |
| R5 / P0 | R4 | AgentRuntime around existing RawV8Host; scheduler checkpoints/environment teardown; preserve process placement | Frame replacement, incumbent/current/relevant realms, microtasks/timers/observers and retained-document tests |
| R6 / P0 | R1, R5; routing threat-model review | Versioned process-session and per-environment broker routes, metadata/validator migration | Forged/stale/cross-partition denial, concurrent IPC/callback deadlock controls; signed security under legacy placement |
| R7 / P1 | R5, R6; placement/quotas approval | Group/cluster placement, connected popups, remote WindowProxy, navigation/COOP transitions and pool lifecycle | Architecture section 8 matrix; failure fan-out/reset/noopener/limit tests; distinct signed attestations |
| R8 / P1 | R5, R6 | Dedicated-worker agents instead of same-isolate contexts; retain bindings/loader | Message ordering/clone/transfer, classic/module execution, busy-JS termination and owner teardown; shared-memory obligations tested/tracked |
| R9 / P1 | R8 | Separate shared-worker identity/ports and service-worker registration/control/update, not one constructor route | Official variants, multi-client partitions, last-client closure, install/activate/fetch/update/restart |
| W0 / P0 | R0 | Repository-owned pinned wptrunner product package and mock protocol contracts | Product discovery, setup/teardown, results/errors and deliberately failing controls |
| W1 / P1 | W0, R1 | General authenticated automation transport in oryn/native Browser API | Window handles/navigation, sync/async script, input and observation; signed stale/foreign/timeout/crash/cancel canaries |
| W2 / P1 | W1; R7 for connected-window dependencies; R8/R9 for worker variants | Official wrappers/MIME and upstream executors; explicit print/manual/visual routes | All Core IDs routed or explicit unmet dependency; fault-injection rejects; no old runner beneath adapter |
| C0 / P2 | R0 for gap study; relevant R/W cutovers for fixes | Mechanism-level packages for layout/paint/AX and remaining full Core matrix | Exact call sites/spec steps/family selection per package; signed results; no unsupported exemptions |
| Q0 / release | All required packages and Core obligations | Frozen clean-source signed candidate; complete Core/security/performance and separately claimed platforms | WPT-CORE-COMPLETION-GATES satisfied, no historical union or outcome-based exclusions |

R1/R2, W0 and C0 gap analysis do not require popup-placement approval. Independent
workstreams are not permission to spawn agents. Do not wait for R9 before exposing
XML/rendering failures. Split R7/R9 into algorithm-level increments; a row is not
permission for an unbounded patch.

First tranche: R0 characterization/ledger, R2 attributes and R1 identity contracts,
then R3 forms. No process placement or attestation change is necessary for it.

## 4. Upstream adapter contract

Pinned source inspected: WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`.
Its tools/wptrunner/wptrunner/products.py supports the wptrunner.products Python
entry-point group. Add a repository-owned package under proposed tools/wpt-oryn/;
register using that entry point. Do not patch test manifests, wrappers or the
built-in product list. Lock package dependencies and hash adapter source separately.

Start with native transport implementing upstream Protocol/ProtocolPart contracts,
not a claim of WebDriver HTTP conformance. Reuse upstream executor/result/readiness
machinery where contracts match. Servo/WebDriver integrations are reference code,
not copied security policy; never copy certificate-error bypasses.

| Pinned upstream requirement | Current foundation/gap | Deliverable |
| --- | --- | --- |
| Browser start/stop/liveness | Browser/CLI and supervision exist | Parent lifecycle, signed path, crash events and fixture settings |
| BaseProtocolPart create/select/list/load windows | BrowserContext/PageHandle exist; no general WPT transport | Session-scoped handles and lifecycle, no PID-as-window assumption |
| execute_script including async callback/timeout | evaluate exists; run command polls a global | Realm/document-bound callbacks, result serialization, exceptions and cancellation |
| TestharnessProtocolPart runner and old-window cleanup | Completion hook is not this contract | Upstream runner/result scripts; honest connected-window dependency |
| Selector/click/keys/actions/permissions/testdriver | Native actions/trace exist; incomplete operations | Explicit capability map, operation traces, rejection instead of success-shaped stubs |
| Reftest readiness/screenshot/fuzzy; crashtest lifecycle | Renderer exists; mapper rejects types | Same-engine test/reference, fonts/viewport controls, crash evidence |
| Print/manual/visual | Not implied by screenshots | Pagination/print executor and reviewed assessment, no blanket PASS |

Keep old session/runner for historical diagnostics until W2 parity. New adapter
must not call run-upstream-wpt.py or its synthetic worker bootstrap. Use explicit
test CA/DNS/loopback grants; preserve normal TLS/network policy. The current
auxiliary_workers_on_demand exception in WPT sessions is not the new placement
policy: migrate it with explicit configuration and source-bound evidence.

Compare old/new runners on the same frozen binary and reviewed overlap, including
failures/timeouts/crashes. Check prerequisites and result fidelity, not pass totals.
Keep corrected outcomes visible. Preserve upstream results separately from Oryn
fixture, transport, security and cleanup validity; executor qualification requires
fault injection, not just a successful launch.

## 5. Verification performed

Local dirty-source checks only: no new signed build or WPT conformance run.
Passing tests validate reuse points, not missing behaviors. Existing ignores stay
unresolved. Commands ran from the repository root.

| Check | Observed result |
| --- | --- |
| cargo test -p oryn-dom --lib -- --test-threads=1 | 82 passed |
| cargo test -p oryn-native --features core-full,v8-host,v8-sandbox,in-process-probe --lib host:: -- --test-threads=1 --quiet | 24 passed |
| cargo test -p oryn-js --features v8-host,v8-sandbox --lib -- --test-threads=1 --quiet | 237 passed, 3 ignored |
| cargo test -p oryn-native --features core-full,v8-host,v8-sandbox,in-process-probe --lib executable::tests:: -- --test-threads=1 --quiet | 160 passed, 6 ignored |
| cargo run -q -p oryn-webidl -- check crates/oryn-webidl/idl/browser.webidl crates/oryn-js/src/generated/browser.rs | Passed |
| python3 -m unittest discover -s scripts/tests -p test_full_wpt_inventory.py -v | 5 passed |
| python3 -m unittest discover -s scripts/tests -p test_wpt_session.py -q | 2 passed |

V8 checks used existing explicit RUSTY_V8_ARCHIVE/RUSTY_V8_SRC_BINDING_PATH values:

- Archive basename: librusty_v8-core-full-e289073ac6ebec82134500225a2dcfdf92c83794f0e5a88fb3c1ff5821136ac4.a.
- Bindings basename: src_binding-core-full-2eb9b14accc74085e6bdbdfa7e039464454286f70e044d19771492af0074db28.rs.

These names identify selected artifacts, not freshly verified hashes. Signed
qualification validates the profile manifest and archive/bindings/ICU/snapshot/
source identities through the build script. Debug tests cannot certify a signed
worker. Neither Linux release compilation nor full clippy/security/performance
qualification was run for this documentation review.

Inventory-v3 still records 13,747 mapped Core executions and 2,990 unsupported
Core entries: 16,737 obligations. Its 128,554 manifest total spans all tiers, not
just Core. Manifest/policy hashes match WPT-CORE-COMPLETION-GATES.md. These counts
are not a pass rate.

For later drift detection, the SHA-256 of the standard `shasum -a 256` output
for these relative paths, in the order below, was
`2fdf34e512029bf711507df077a047bc8193804a703feae79698430aae44af82`.
Reproduce by passing these paths to `shasum -a 256` and piping its output to
`shasum -a 256`. This covers selected audit inputs, not the full source tree or
transitive test inputs, and excludes the design documents themselves.

```text
crates/oryn-dom/src/lib.rs
crates/oryn-dom/src/html.rs
crates/oryn-dom/src/xml.rs
crates/oryn-js/src/realm.rs
crates/oryn-js/src/scheduler.rs
crates/oryn-js/src/v8_bindings.rs
crates/oryn-native/src/host.rs
crates/oryn-native/src/rust_executable.rs
crates/oryn-native/src/runtime.rs
crates/oryn-native/src/worker.rs
crates/oryn-native/src/worker_client.rs
crates/oryn-native/src/context_service.rs
crates/oryn/src/wpt_session.rs
scripts/prepare-full-wpt-baseline.py
```

## 6. Per-patch verification and rollout

1. Add a failing behavior/state-transition regression before a fix, or a
   characterization for an extraction. Retain failure output; do not invent a WPT
   gain from a unit-test count.
2. Run affected suites above plus layout/render/worker/broker tests as applicable,
   cargo fmt --all -- --check, generated WebIDL check and scoped clippy with
   warnings denied. No new ignores to pass cutovers.
3. Inventory consumers of attribute writes, NodeId transport, page-global helpers,
   protocol literals and page_isolate assumptions. Compile checks cannot discover
   every JS/Python/shell serialization dependency.
4. Use scripts/build-sandboxed-worker.sh with approved signing identity and
   hash-validated ORYN_V8_PROFILE_MANIFEST. Run signed sandbox/denial and whole-family
   checks with exact provenance. Label dirty diagnostics; publish release evidence
   only from a clean reconciled candidate. Never erase user changes to obtain it.
5. Compile Linux release features on a Linux runner. Unqualified pool/placement
   modes fail configuration validation rather than silently falling back.
   Equivalent security/conformance needs separate evidence.
6. R7/Q0 rerun cold/cached/prewarm/recycled binding, throughput, concurrency, RSS
   and churn/reset. Report independent and connected-window workloads separately;
   retain original acceptance thresholds.

## 7. Remaining gates

Ready now: characterization, attributes/identity contracts, forms migration,
upstream adapter contract tests and rendering gap analysis. Start dependent work
only after its prerequisites pass; readiness does not mean fixes already exist.

Before R7 activation: approve process failure boundary, public isolation/lifecycle
semantics/quotas, revised attestation and cross-process agent/shared-memory policy.
R6 requires a reviewed routing threat model even under legacy placement. Defaults
remain unchanged until approval and qualification.

Before each C0 patch: specify exact symbols, normative steps, state transitions,
complete WPT family and negative controls. Remaining platform closure is not an
unbounded rewrite authorization or a claim of finished API designs. No credible
completion date exists until broader execution measures those mechanisms.

The deferred Range.insertNode crash stays tracked in
WPT-RANGE-INSERTNODE-CRASH-2026-09-12.md; this review does not close it. Rollback
means bounded source reversal or a prior whole artifact, retaining provenance.
Never select another engine per test or convert failures into exclusions.

Normative cross-checks: [HTML agents](https://html.spec.whatwg.org/multipage/webappapis.html#agents-and-agent-clusters),
[HTML submission](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#form-submission-algorithm),
[DOM adoption](https://dom.spec.whatwg.org/#concept-node-adopt).
Product/protocol contracts were read in pinned local WPT source, not assumed
from online documentation for another tooling revision.
