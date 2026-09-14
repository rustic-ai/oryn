# Oryn browser architecture and implementation migration

Status: proposed design and execution plan, September 13, 2026. This document
does not authorize a security-boundary change, claim implementation completion,
or replace historical evidence. No runtime rewrite was performed for this review.

Code-validation update: concrete cutovers are specified in
[Implementation readiness and cutover plan](BROWSER-IMPLEMENTATION-READINESS-2026-09-13.md).
That companion records verified reuse points, corrections, dependencies, tests
and approval gates. First non-placement packages are ready to implement;
multi-document process placement retains its explicit security decision gate.

## 1. Decision and scope

Retain the native Rust/V8 browser direction, but replace fragmented orchestration
with an explicit browser runtime. Reuse working component integrations and tested
algorithms; do not rewrite V8, html5ever, Stylo, the broker or the DOM wholesale.
Rewrite boundaries whose contracts cannot express browser semantics correctly.
Splitting large files alone is not an architectural repair.

The objective remains complete Core conformance against the full pinned WPT
manifest and tier policy. The existing 13,747 mapped Core executions and 2,990
unsupported entries remain obligations. Necessary/Extended features stay visible
in the inventory; implementing a dependency outside Core is allowed, but changing
tiers to make the Core score improve is not. A headless browser is still required
to implement observable rendering, focus and navigation semantics.

Product assumptions: native execution, an embeddable Rust API, semantic/OIL/MCP
automation, parent-brokered privileges, signed disposable macOS workers, bounded
resources and reproducible evidence. We do not assume Chromium-level conformance
comes from embedding V8 or that a general layout library supplies all CSS layout.

If a near-term full-browser compatibility deadline is imposed, compare this
native-engine programme with embedding a complete browser engine. That would be
a product/security/performance decision, not a transparent library replacement.
No credible completion date can be inferred from current test counts.

## 2. Findings from current code

| Finding | Evidence | Design consequence |
| --- | --- | --- |
| Useful canonical native DOM foundation exists | `crates/oryn-dom/src/html.rs`: html5ever TreeSink and shared incremental runtime | Keep it; parsing must feed the same document later seen by JS and rendering. |
| Attribute identity cannot be represented fully by the current primary maps | `crates/oryn-dom/src/lib.rs`: attributes keyed by String; Attr nodes keyed by `(NodeId, String)` | Replace identity with ordered namespace/local-name records; null-namespace lookup patches are not the final design. |
| Browser orchestration is concentrated in an executable document and binding dispatch | `crates/oryn-native/src/rust_executable.rs`, `crates/oryn-js/src/v8_bindings.rs` | Extract semantic ownership and typed interfaces before deleting old routes. |
| Duplicate browser behavior has caused real failures | September 13 form/activation reports: semantic submission bypass and duplicated submit events | Every entry point must converge before executing web-platform algorithms. |
| Submission transport loses semantic information | `PendingDefaultAction::Submit(NodeId)` | Replace with a rooted, lifecycle-bound submission transaction and later an immutable navigation request. |
| JS exposure is too coarse as an architectural model | `BindingGlobal` distinguishes Window and Worker | Model dedicated/shared/service-worker environments separately, without confusing realm, agent, process and document lifetimes. |
| Multi-realm hosting already exists | `RawV8Host` has a context map, RealmRegistry, frame creation/replacement and native WindowProxy support | Extend this embedding, not a second V8 host/frame implementation. |
| Frame embedding and DOM ancestry are conflated | Static/dynamic frame loading imports same-origin child documents and appends them beneath iframe elements; DomRuntime retains one document/id/focus state | Separate frame relationships from DOM parentage and introduce document-scoped state before extending the arena to popups. |
| Worker globals are not independent worker execution | WorkerGlobalKind and tests cover several globals, but worker creation makes contexts in the current isolate | Reuse bindings; migrate agents, scheduling and lifetime separately. These globals are neither wholly absent nor proof of complete worker support. |
| IPC and pool ownership are page-shaped | PageSession, WorkerPage, WorkerBrokerRouter::route, ParentContextService::source_page_id | Add process-session ownership and environment routes before multi-page hosting. |
| XML/MIME support exists | `oryn-dom::xml::{parse_navigation, parse_by_mime}` and MIME-bearing DocumentStream | Route upstream tests through existing implementations and fix observed semantics; do not add another parser. |
| Compatibility bootstrap remains in the code | generated platform objects are saved and restored around handwritten classes | Audit reachability and compile-time mode separation. Presence does not prove these classes are active in production. Remove production dependence on install-then-restore behavior. |
| Rendering integrations are real but incomplete | Stylo snapshot adapter; Taffy/Parley; renderer reports SVG images and gradients unavailable | Keep adapters only where they convey the required semantics; budget missing layout/paint algorithms explicitly. |
| Many exclusions originate in the inventory mapper | `scripts/prepare-full-wpt-baseline.py::runner_mapping` | Replace suffix allowlists with official manifest/executor dispatch; do not infer engine support from this map. |
| Worker test adapter includes a nonstandard service-worker constructor | `scripts/run-upstream-wpt.py` | Retire it, use upstream wrappers and real registration/lifecycle. |
| Evidence is stronger than earlier baselines but incomplete | Core completion gates and harness-v4 repair ledger | Preserve auditing; decouple engine results, executor readiness and reference-browser diagnostics. |

Some documents are stale: the sandbox security document still describes a v2
handshake while current source uses protocol v9. Build-derived facts must be
generated and linked to exact evidence rather than maintained in prose alone.

## 3. Target architecture

```text
Rust API / CLI / MCP / OIL / semantic actions / WebDriver adapter
                         |
                  Browser command API
                         |
     Browser coordinator and privileged broker (parent)
     context groups, routing, storage partitions, transport,
     permissions, file/secret handles, process supervision
                         |
        authenticated, bounded, typed IPC; scoped grants
                         |
     Sandboxed web runtime processes (agents / agent clusters)
       environment + document + lifecycle registries
       scheduler and V8 embedding
       generated WebIDL -> platform algorithms -> native state
       parser -> native DOM -> style/layout -> paint/AX snapshots
                         |
              versioned observations and results

WPT runner -> the same Browser command API and production runtime
           -> upstream fixtures, wrappers and testdriver protocol
           -> external evidence collector (not browser semantics)
```

### 3.1 Ownership contracts

| State or operation | Single authority | Consumers |
| --- | --- | --- |
| Worker identity, lifecycle, privilege grants | Parent coordinator | All public adapters and runtime sessions |
| Transport, DNS, TLS, persistent stores, filesystem access | Parent services | Typed web-runtime requests only |
| Browsing-context group membership and cross-process routing | Parent coordinator | Window/worker runtime registries |
| Document tree, attributes, native control state and mutation rules | Native DOM/platform runtime | JS wrappers, parser, layout, semantics |
| JS objects, realm prototypes, traced JS edges and execution | V8 embedding | Generated bindings and host callbacks |
| Web-observable task/reaction/event ordering | Agent scheduler | Parser, networking, storage, input, timers, rendering |
| Web fetch/HTML policy algorithms | Platform policy implementation | All resource consumers; parent independently enforces host privilege constraints |
| Computed styles and geometry | Style/layout services over revisioned native input | CSSOM, hit testing, rendering, AX/semantics |
| Pixels and print output | Renderer over versioned paint input | Screenshots, reftests and product APIs |
| Test outcomes and evidence validity | External runner and validator | Reports; never read by browser algorithms |

"Single authority" does not prohibit defensive checks at the privilege boundary.
The parent must not trust a compromised worker's claimed origin, permissions or
cookie partition. It derives or validates these against scoped registrations.

### 3.2 Identity, agents and process placement

Introduce or consolidate typed identifiers for BrowserContext/storage partition,
BrowsingContextGroup, BrowsingContext, AgentCluster, Agent, Realm, Document,
Navigation and Operation. IDs carry generations where lifetimes can be replaced.
Do not use a NodeId or an origin string as a substitute for these concepts.

An origin is a structured tuple or a unique opaque identity, not just a string.
About:blank/srcdoc inheritance, sandboxing, blob URLs, COOP/COEP/OAC and permitted
legacy origin behavior must use spec algorithms rather than URL-string equality.

Recommended placement: independently created root browsing sessions remain
isolated. Script-connected, same-agent windows must have an execution arrangement
that supports required synchronous object access and reentrancy; the default
target is co-location in the appropriate window agent/agent-cluster runtime.
Separate window agent clusters and worker agents can be placed in separate
sandbox processes. Cross-origin WindowProxy exposes only the specified surface;
postMessage uses structured clone/transfer rather than generic object RPC.

An agent cluster, an agent and a V8 isolate are not interchangeable. Use a V8
isolate per JS agent as the initial implementation model; window realms that must
share an agent live together. Dedicated/shared/service-worker agents have their
own execution and termination paths. Validate actual cross-agent requirements
before choosing shared-memory backing and transport.

**Security decision gate:** the current literal one-process-per-page contract
cannot simply be assumed to cover script-connected windows. The proposed
co-location changes its failure/isolation unit. Before implementation, approve a
revised threat model, public lifecycle semantics and attestation language. Keep
the current default intact until then. Do not invent asynchronous same-origin
DOM access or silently sever opener relationships to make the old model fit.
If this decision is not approved, the affected conformance remains explicitly
unfinished; it is not removed from Core.

#### Published implementation comparison (reviewed September 13, 2026)

Chromium's process-placement unit is not a page or tab. Its `BrowsingInstance`
tracks a browsing-context group of related windows and frames; its `SiteInstance`
tracks a principal instance within that group. Documents with the same principal
in the same group must share a renderer process to support synchronous access.
The principal can be site- or origin-based depending on policy. A `SiteInstance`
roughly corresponds to an agent cluster, but these are not identical concepts.
Unrelated same-origin tabs need not share a process. Cross-site frames can occupy
separate renderers under Site Isolation, with browser-enforced IPC restrictions.
COOP and navigation can change group membership. These are placement constraints,
not a promise of one process per origin or one process per browsing-context group.
See [Chromium's process model](https://chromium.googlesource.com/chromium/src/+/main/docs/process_model_and_site_isolation.md).

For example, a dashboard at `https://app.example/dashboard` opens `/editor` with
`window.open`, retaining its opener relationship. The related same-origin windows
can synchronously access each other's DOM, share object references and invoke
callbacks across windows. They need distinct window realms, not independently
serialized copies of their objects. Chromium co-locates these documents; they
consequently share a renderer crash boundary. A separate unrelated tab at the same
origin does not, merely by sharing that origin, require this placement. If a
security policy severs the relationship, the resulting behavior must follow that
policy rather than an implementation shortcut.

Obscura provides useful but narrower evidence:

- Its v0.2.1 release reports child-frame V8 realms, same-origin frame access and
  parent/frame messaging. Current `Page` source owns an `ObscuraJsRuntime` and
  child `FrameRealm` values; the runtime constructs a V8 isolate and creates
  additional frame contexts within it. This is evidence of multi-document realm
  support, not proof of complete cross-window semantics.
- The published bootstrap inspected defines `globalThis.open` to return `null`
  and initializes `opener` to `null`. That path does not implement the dashboard/
  editor popup example. Do not infer complete popup support from iframe support
  or from aggregate conformance results.
- The June architecture wiki says all pages share one isolate, while the current
  page/runtime constructors indicate per-page runtimes with their own isolates.
  Treat the wiki as outdated on this point, not as the execution-model authority.
  A V8 realm/isolate is not an OS sandbox or a guarantee of native crash isolation.

Sources: [Obscura v0.2.1 release](https://github.com/h4ckf0r0day/obscura/releases/tag/v0.2.1),
[Page ownership](https://github.com/h4ckf0r0day/obscura/blob/main/crates/obscura-browser/src/page.rs),
[runtime construction](https://github.com/h4ckf0r0day/obscura/blob/main/crates/obscura-js/src/runtime.rs),
[bootstrap](https://github.com/h4ckf0r0day/obscura/blob/main/crates/obscura-js/js/bootstrap.js)
and [architecture wiki](https://github.com/h4ckf0r0day/obscura/wiki/Architecture-overview).
This is a published-source review, not an Obscura runtime qualification. The
`main` links are mutable; archive exact revisions before using this comparison
as release evidence.

#### Oryn placement and lifecycle contract to approve

Adopt Chromium's semantic grouping principle, not its entire implementation and
not Obscura's unsupported-popup behavior. Keep browsing-context groups, agent
clusters, agents, realms and OS processes explicit and separately identified.
The coordinator decides placement from browser context/storage partition,
relationship and agent-cluster rules, isolation policy and process suitability;
origin equality alone is insufficient. Workers cannot self-authorize a join.

Independent root groups remain isolated. Within a group, co-locate window realms
requiring synchronous access in their window agent. Keep cross-origin
WindowProxy restrictions even when two realms happen to share a process; route
permitted cross-process operations through authenticated, scoped messages.
Navigation preserves or replaces window/document identities and relationships
according to the standard, including stable WindowProxy behavior and revocation
of obsolete document grants. Do not copy JS objects over IPC as a substitute for
required shared identity, or implement arbitrary synchronous DOM RPC while
holding scheduler/broker locks.

Prewarming is a supply of authenticated, unbound processes, not a placement
policy. A connected popup must join its required execution environment rather
than consume an incompatible independent worker simply because one is idle.
Closing one page does not retire a process while other hosted environments are
live. Retirement/reset eligibility is evaluated over all hosted agents, realms,
operations and grants. Failure of a shared process reports every affected page
and revokes that process's authority without invalidating unrelated groups.

The ADR must define how existing `WorkerLifecycle` options, pool capacity,
`max_leases`, close/reset behavior and metadata apply to multi-document hosting.
Do not reinterpret a page lease as a group/process lease without a versioned
public contract. Attestation must distinguish independent-group isolation from
literal page isolation and report the actual shared failure boundary. Existing
signed-worker authentication and parent-enforced privilege checks remain required.

### 3.3 Document and memory model

Keep one authoritative native tree per document, not necessarily one allocation
arena per document. Target an agent-owned arena with separate document roots and
document-scoped metadata/indexes, allowing same-agent adoption to preserve node
identity. Introduce arena identity wherever NodeId escapes its owner; current
slot/generation pairs alone do not distinguish separate arenas. A frame's child
document must not be an ordinary DOM child of its iframe: store that relationship
in the browsing-context registry, and traverse it explicitly for layout, semantics
and automation. Repair this existing boundary before adding popup co-location.

Parser state mutates the authoritative tree. Parsed snapshots, style mirrors and
serialized observations are derived, never competing mutable trees. Snapshots
carry document/revision identities and reject use after replacement. Navigation
revokes active-document authority but must not destroy old DOM/realm objects still
legitimately retained by script. GC lifetime and active lifecycle are separate;
process reset requires complete teardown of both.

Represent attributes as an ordered collection keyed by `(namespace, local name)`
with prefix/qualified-name data and stable Attr identity. Implement separate
qualified-name and namespace APIs, replacement/adoption rules, namespace-aware
reflection, serialization and mutation records. Migrate every direct attributes
map user; do not leave old and new maps independently writable.

Native operation roots retain nodes while queued algorithms need them. JS-facing
owner, collection, event and cross-realm edges are traced by V8. No unbounded
native table of strong JS handles and no treating reachability as connectedness.
Define tracing/root ownership for each interface and test cycles and teardown.
Every operation ends in completion, cancellation or explicit failure; terminal
cleanup is idempotent and cannot resurrect a revoked environment.

### 3.4 Scheduler and lifecycle

One semantic scheduler per agent owns task sources, timers and lifecycle barriers.
V8 microtasks, custom-element reactions, mutation observers, parser pauses and
rendering opportunities follow their specified checkpoints. Do not assume that
every operation ends with the same generic sequence of flushing everything.

Host I/O is asynchronous and posts typed completions; it never executes JS while
holding the broker registry lock or a mutable DOM borrow. Synchronous platform
callbacks use explicit reentrancy scopes. Tests exercise callbacks that mutate,
navigate, close, transfer and recursively invoke APIs.

Each async operation identifies its owner agent/environment, navigation where
applicable, cancellation policy, budget and terminal state. Document navigation
does not cancel browser-context-owned stores or service-worker registrations;
service-worker lifetime is not page lifetime. Worker failure revokes only the
resources owned by the failed security domain, with affected pages reported.

Production uses real time. Virtual time and deterministic scheduling are explicit
automation controls and cannot be quietly activated by a WPT pathname. Completion
uses test lifecycle signals, not arbitrary sleep or network-idle heuristics.

### 3.5 WebIDL and platform algorithms

Generated bindings own exposure, overload resolution, dictionary conversion order,
exceptions, receivers, descriptors and realm-sensitive object construction.
They invoke named platform algorithms; they do not independently implement form,
navigation, selector or policy decisions in a giant dispatch match.

Support required WebIDL syntax, including dictionary defaults such as `{}`, in
the generator. Runtime handling of an omitted dictionary remains necessary, but
per-interface source omissions are not the long-term solution. Generate a checked
interface/operation coverage manifest and enforce strict regeneration in CI.

Organize platform algorithms by domain, with native state and an explicit
callback/effects context when they can execute script. Rust-only is not itself
a correctness property; a carefully isolated JS builtin is acceptable where
appropriate. The prohibition is duplicate or probe-dependent production semantics.

Remove page-global `__oryn_*` implementation entry points where they expose
privilege or let author script replace internal operations. Embedder/private host
operations invoke intrinsic functions directly. Author-visible APIs must still
have standard mutability and override behavior. Tests must attempt shadowing and
tampering with public constructors, prototypes and globals.

### 3.6 Forms and navigation: first end-to-end migration

All submit sources converge on one form-submission algorithm: activation,
requestSubmit, implicit submission, semantic submit and submit(). Preserve their
specified differences: submit() bypasses validation/events; requestSubmit checks
submitter type/ownership; activation honors cancellation and disabled state.

Use a rooted SubmissionTransaction, not a bare form NodeId. It carries form,
submitter, initiating environment, user involvement and reentrancy state while
callbacks run. It computes candidate controls, validation and SubmitEvent once,
then constructs the ordered entry list at the specified point. FormDataEvent
mutations, disabled controls, files, image coordinates, charset and line endings
must be handled by the same entry-list implementation as FormData(form, submitter).

Resolve submitter overrides, URL, method, encoding, target, referrer and navigation
policy into an immutable NavigationRequest before scheduling transport. The
request owns its body/entry-list and safe file capabilities; it does not reread
mutable DOM after an arbitrary scheduler delay. Recheck navigation eligibility
at the spec-defined callback boundaries. Carry supersession/planned-navigation
identity through commit and teardown; stale requests cannot commit.

Capture each field at its specified algorithm step, not in one early snapshot:
submit/formdata callbacks may affect values consumed later. Reuse generated
FormData value handling, but extract the shared entry-list algorithm and add the
submitter overload, FormDataEvent and missing control cases. Selected file state
moves out of `data-oryn-upload-files` into native input state; parent upload handles
remain parent-owned and are not exposed as JS capabilities.

GET, POST and dialog behavior are distinct, not all converted to a URL load.
Context-target selection and storage/security policy run through shared services.
Forms are the first vertical slice because current evidence exposes all of these
boundary defects, not because passing forms defines Core completion.

### 3.7 Loading, networking and workers

Keep parent-brokered transport, cancellation, backpressure and TLS verification.
Unify web request construction and resource policies across navigation, fetch,
XHR, scripts/modules, styles/fonts/images, frames and workers. Explicit initiator,
destination, credentials, origin/storage partition, referrer, redirect state and
body ownership are mandatory. Preserve the distinctions required by each API;
"shared" must not mean one simplistic universal CORS rule.

Route document parsing by MIME and encoding through HTML or XML pipelines. XML
errors, processing instructions, namespaces and scripting behavior are not HTML
parser options. Raw text/image documents get their correct document behavior.
Use existing XML integration where verified; do not assume its mere presence
makes all XML-family navigation correct.

Model dedicated workers, shared-worker connection ownership and service-worker
registration/install/activate/control/fetch/update lifecycles separately.
Service workers use registration, not `new ServiceWorker`. Persistent registration
authority belongs to the browser context; untrusted worker execution remains
sandboxed. Revalidate every broker operation after registration/agent retirement.

### 3.8 Style, layout, paint and accessibility

Retain Stylo as the cascade/computed-value engine. Feed native namespace, document
mode, tree relations and control/pseudo states through one adapter contract.
DOM selector queries and styling use the same selector semantics; test both
results after each mutation without manufacturing HTML attributes for state.

Keep Taffy/Parley for the algorithms they correctly supply. Produce a gap map for
inline/block formatting, floats, tables, flex/grid intrinsic sizing, replaced
elements, writing modes, fragmentation, positioning, overflow, scrolling and
hit testing. Implement missing required algorithms behind a layout contract or
replace that subsystem if the gap study demonstrates that adaptation is the
larger rewrite. Do not translate unsupported CSS into plausible but wrong boxes.

Paint output must consume the same layout/shaping data as hit testing and CSSOM.
Audit the current Parley-to-renderer text path for shaping fidelity. Add required
SVG, gradients, clipping, transforms, stacking and print pagination based on the
pinned Core inventory. Renderer limitations are engine gaps, not permanent WPT
runner exclusions. Reftests need deterministic readiness and upstream fuzzy
comparison metadata; references are normally rendered by Oryn itself.

The accessibility tree and semantic graph are projections of native document,
style, focus and relationship state. Keep stable identities and incremental
updates, but verify against an independent full recomputation oracle. Semantic
actions resolve targets and invoke input/activation primitives; they never write
alternate checkedness, selectedness or navigation outcomes.

## 4. WPT integration design

Adopt the pinned upstream wptrunner product/executor model with an Oryn control
protocol adapter. Use existing wptserve, manifests, generated wrappers, variants,
fixtures and test-type semantics. The custom evidence wrapper may launch the run,
record identities and validate results; it must not remain a second WPT scheduler
and test-page generator. A product-specific executor is legitimate integration,
not permission to alter browser behavior.

Expose a narrow authenticated automation session: context creation/selection,
navigation, script execution/callback results, input actions/release, screenshot,
print, permissions and lifecycle/crash observation as implemented. Prefer the
WebDriver-compatible surface consumed by upstream executors. A transitional
native protocol may implement the same operations; full WebDriver conformance
is a separately tracked deliverable, not assumed from the adapter existing.

Automation uses the production input and platform paths. Test-driver privileges
are granted by the host/session, not by a globally reachable page helper. Untrusted
page content cannot widen scope, open host files, acquire secrets or change TLS
trust. Native and Chromium adapters remain separate implementations.

The current Core unsupported inventory is an explicit migration backlog:

| Family | Entries | Required work |
| --- | ---: | --- |
| Shared-worker variants | 292 | Official wrappers plus genuine shared-worker execution/lifecycle |
| Service-worker variants | 264 | Official wrappers plus registration/control lifecycle |
| XML/XHTML/SVG/XHT testharness documents | 298 | Manifest routing, MIME-correct loading and result collection |
| Standalone worker scripts | 14 | Official worker wrapper routing |
| Other generated variants | 25 | Inspect exact manifest global, then route or implement its environment |
| Reftests | 1,003 | Screenshot/readiness/reference/fuzzy executor plus missing rendering |
| Crashtests | 330 | Lifecycle/load/crash executor; preserve crash and sanitizer evidence |
| Manual | 749 | Reproducible human assessment or reviewed faithful automation |
| Print reftests | 12 | Print lifecycle, pagination and comparison |
| Visual | 3 | Defined assessment, not silent skips |

Do not turn these into passes by enabling a mapper. Newly runnable tests first
produce genuine failures where browser features are missing. Zero unsupported
classification without faithful execution is not an acceptance criterion.

### Results and qualification

Maintain independent dimensions: inventory membership, executor availability,
fixture/automation readiness, observed engine result, evidence validity and
reference-comparison status. Statuses include NOT_RUN, EXECUTOR_MISSING,
ENVIRONMENT_BLOCKED, HARNESS_ERROR, PASS, FAIL, TIMEOUT, CRASH and manual assessment;
they are not interchangeable and must retain detailed reason codes.

A Chrome failure does not automatically invalidate a legitimate Oryn WPT result.
Mandatory independent prerequisite checks remain; Chrome is an optional
differential diagnostic unless that execution explicitly relies on it. Reftests
compare test and reference in the tested engine, not arbitrary Chrome screenshots.

Qualification should scale through versioned executor/fixture capability contracts
and per-run validation, with source-specific review for exceptional substitutions
or uncertain prerequisites. Do not require hand-authored essays for every ordinary
unchanged upstream test forever; do not remove existing review gates until their
replacement catches injected missing-helper, wrong-global, truncated-result,
stale-session and unsupported-action faults.

Preserve raw outcomes and exact IDs, globals, variants, assertion identities,
timeouts and retries separately. Failed attempts cannot disappear into a final
successful retry. Record parent/worker/V8/bindings/ICU/snapshot/signature/source,
toolchain, dependency, renderer/font, fixture, policy, manifest and executor
identities. Redact secrets and payloads; provenance must not become data exfiltration.

## 5. Implementation order and acceptance gates

These are dependency gates, not promises that a phase alone completes Core.
Do not start an unbounded rewrite or discard the dirty working tree.

| Phase | Concrete changes | Exit gate |
| --- | --- | --- |
| 0. Reconcile and freeze | Inventory current production/probe routes, source changes, accepted contracts and all Core entries. Capture current signed diagnostics; preserve the deferred Range.insertNode crash with a reproducer. Write ADRs for process placement, ownership and runner migration. | Every known gap has an owner and evidence requirement; immutable baseline; security decision explicitly recorded. |
| 1. Boundary contracts | Introduce typed environment/document/operation identities, rooted operation guards and runtime service interfaces. Add tests for lifecycle and reentrancy. Separate host orchestration from V8 binding conversion. | No new duplicate semantic route; stale IDs/completions and mutation-during-callback tests pass. Existing behavior preserved. |
| 2. First vertical cutover | Consolidate attributes and forms/navigation contracts. Replace pending bare Submit, unify FormData entry-list construction, input/textarea constraints and validation reporting. Remove old routes, including host execution of replaceable page-global intrinsics. | Same observable results through JS, trusted input, semantic actions and automation; namespace/GC/cancel/reentrancy tests; complete relevant signed WPT families, with failures retained. |
| 3. Agent/environment cutover | Implement approved context-group/agent placement, WindowProxy/realm routing, task ordering, worker lifetimes and strict broker ownership. | Same/cross-origin frame and popup access, navigation replacement, shared/service-worker lifecycle, teardown and privilege tests pass. Revised security attestation proven. |
| 4. Upstream runner cutover | Add Oryn product adapter and executor families; use official wrappers and MIME routing. Replace old mapper/executor paths after parity. | Every Core entry is reconciled; all automatable families execute or identify an explicit implementation dependency; injected harness faults reject correctly; cold/prewarm parity and stress pass. |
| 5. Remaining platform and rendering closure | Work dependency-first across the full functionality matrix below. Complete layout/paint/AX capabilities required by Core. | Signed whole-family results, lifecycle/security regression matrices, no outcome-based omissions. Every Core failure has a resolved fix or remains visibly open. |
| 6. Full qualification and release | Run all Core coverage on one frozen candidate, complete manual/visual assessments, audit all gates and produce clean-source exact-artifact evidence. | Every Core obligation satisfied and independently verified; all required platform/security/product checks pass. Only then mark the goal complete. |

Runner skeleton and inventory reconciliation can progress alongside phases 1–3;
do not wait for browser features to become green before exposing their failures.
Rendering gap analysis begins in phase 0, not after all forms work. This is a
dependency plan, not authorization to spawn agents or change process isolation.

### Full functionality matrix

| Area (mapped Core executions) | Shared mechanisms and closure focus |
| --- | --- |
| HTML (4,350) | Parser/scripts/modules, lifecycle, forms, documents, focus, navigation/history, frames/popups, embedded and interactive elements, editing/drag-drop |
| Security/policy (3,248) | CSP, Trusted Types, referrer, mixed content, secure contexts, permissions, integrity and embedder policy integration |
| Networking (1,859) | Fetch/XHR/WebSocket/CORS, redirects, streaming, backpressure, abort, credentials and worker consumers |
| Encoding/URL/MIME (1,462) | Shared decoders, URL algorithms, content-type dispatch, incremental boundaries and malformed inputs |
| DOM/components (1,218) | Tree/attributes/namespaces, Range/Selection dependencies, observers, traversal, parsing, custom elements and shadow trees |
| CSS (564) | Cascade/CSSOM, state/namespace selectors, layout/geometry/paint and the additional unsupported visual inventory |
| Data/messaging/storage (560) | Blob/File/streams, structured clone/transfer, messaging, cookies and storage lifetime/partitioning; required dependencies outside this root grouping remain in scope |
| Interaction (304) | Focus, keyboard/pointer actions, selection, input events, inertness and common hit testing |
| IDL/import maps (115) | Generator correctness, brands/descriptors/realms, module resolution and shared loader integration |
| Accessibility (67) | Accessible names/ARIA, tree projection and platform/manual assessments; 392 unsupported entries in this grouping are not omitted |

Root counts are workload organization, not estimates of difficulty, assertion
counts or proof of feature support. The separate 2,990 unsupported entries remain
in their respective functional workstreams.

## 6. Migration mechanics and repository layout

Prefer modules within existing crates initially; introduce a new crate only to
enforce a real dependency boundary. Proposed names below are target modules, not
claims that equivalent new crates are required.

- `oryn-dom`: native tree, namespace-correct attributes, ranges, state, identity,
  parser sinks and mutation journal. No network, page-global JS or WPT imports.
- `oryn-js`: V8 embedding, generated conversions, realm/wrapper tracing and
  platform algorithm modules. Split `v8_bindings.rs` by ownership after contracts.
- Browser-runtime modules: environments/agents, lifecycle, navigation/submission,
  resource loading and input/activation. These coordinate DOM and JS explicitly.
- `oryn-native`: parent coordinator, sandbox worker entry point, broker adapters,
  process/lifecycle supervision and external API integration; shrink executable
  document orchestration as contracts move out.
- `oryn-net`: transport and bounded resource operations; keep privilege ownership
  in the parent. No browser behavior implemented by the conformance runner.
- `oryn-layout` / `oryn-render`: typed, revisioned style/layout/paint services;
  vendored patches carry upstream references and their own regression fixtures.
- WPT tools: upstream product/executor adapter, testdriver bridge, capability
  checks and external evidence collection. Legacy runner retained read-only for
  historical reproduction until new-executor parity is demonstrated.

For each migrated domain: characterize all entry points; add contract tests;
implement one authoritative path; route all callers; compare pure snapshots or
isolated replay runs; delete duplicate runtime paths; then run signed evidence.
Never shadow-execute mutating navigation, network or storage operations against
the same live context. Rollback is a reviewed source-level reversal or selection
of a whole prior artifact, not a silent per-test fallback engine.

Compile probe-only compatibility code behind explicit features and remove it
from release artifacts once no longer needed. Audit actual reachability before
deleting historical diagnostic support. No production fallback based on missing
Web APIs or test filenames. Preserve public Browser/CLI/MCP compatibility with
adapters, while versioning protocol and metadata changes explicitly.

## 7. Security, lifecycle and performance invariants

- Signed identity, exact entitlements, native PID/CDHash authentication, clean
  environment, bounded IPC and parent-owned policy remain mandatory.
- No page-exposed generic RPC. Every grant binds browser context, agent/process,
  document where applicable, operation kind and generation; stale grants reject.
- Prewarming creates empty unbound runtimes. Binding grants arrive only after
  authentication. Placement follows the approved relationship/agent policy, not
  pool availability. Default isolated sessions do not recycle state across users.
- Recycling remains explicit reduced isolation. Its erasure proof must expand to
  all new agent/registration/routing state; closing one hosted page is insufficient
  while another realm, popup or worker client remains live. Do not reuse a process
  with surviving shared-worker or service-worker clients. Crashed, timed-out or
  malformed-protocol workers are killed, never recycled.
- Scope the process failure domain in public metadata accurately. Do not retain
  old one-page isolation claims after approved agent-cluster co-location.
- Production worker RSS/heap/time limits and policy denials are observable
  outcomes, not harness problems to mask. Tests needing a larger explicit profile
  must have separately named evidence and cannot certify the default profile.
- Preserve cancellation and backpressure, avoid synchronous subprocess probes
  in IPC loops, and avoid holding scheduler/DOM/broker locks across callbacks.

Re-run the existing signed performance suite with separate cold verification,
cached isolated launch, prewarm/recycle binding, warm throughput, 1/5/20-context
readiness, aggregate RSS and churn/reset measurements. Retain the prior requested
targets (cold P50 <=250 ms; warm >=300 actions/s; prewarm/recycle binding P50
<=50 ms; worker RSS P50 <=64 MiB/P95 <=160 MiB; no failures in the stated sample
and concurrency runs) as gates to requalify, not as established current results.
Any revised multi-document process model needs additional per-agent/group and
aggregate measurements; do not compare changed workloads as the same benchmark.

## 8. Verification and completion audit

Each domain requires: native unit tests; generated-binding/WebIDL tests; GC,
cross-realm and reentrancy tests; navigation/close/crash/stale-completion tests;
signed production integration; full pinned WPT family results; and relevant
security/performance checks. Deterministic metamorphic tests cover shared state
through queries, styles, rendering and semantics. Fuzz parser/tree/Range mutation,
IPC and structured-clone boundaries with minimized reproducible failures.

The agent-placement cutover additionally requires an explicit connected-window
matrix, through the production runtime and unchanged relevant WPT tests:

- Same-origin opener/popup and parent/frame synchronous DOM access, JS object
  identity, cross-realm brands/prototypes and nested callback reentrancy.
- Cross-origin access denial, permitted WindowProxy operations and asynchronous
  postMessage with structured clone/transfer, including cross-process delivery.
- Initial about:blank inheritance, navigation across origins, stable WindowProxy
  identity, named-target reuse, noopener and COOP-driven relationship separation.
- Independent same-origin groups and storage partitions staying isolated;
  forged joins, stale document grants and late completions being rejected.
- Closing one of several hosted windows without losing the others; crash fan-out
  limited to the actual process; reset refused while any hosted environment is
  live; full erasure before any permitted reuse.
- Cold and prewarmed placement producing identical observable semantics, with
  separate measurements for independent pages and connected multi-window groups.

CI tiers: fast formatting/clippy/generation/unit contracts on every change;
signed domain and fixture canaries for cutovers; broader family runs before
promotion; complete Core and all required product/security/platform checks for
release. Linux paths keep compiling throughout; equivalent Linux security and
conformance must be separately qualified, never inferred from macOS.

Final completion must satisfy `WPT-CORE-COMPLETION-GATES.md` in full. Reconcile
every manifest entry/global/variant, including manual/visual and crash tests.
No mapped-only denominator, no historical-results union, no source-ahead results
attributed to old binaries, no missing assertions interpreted as passes. Where
the pinned test itself is defective, require independently reviewed upstream
resolution and a versioned inventory decision; do not grant local blanket waivers.

## 9. First implementation sequence after approval

1. Create the phase-0 architecture/capability ledger from actual call sites and
   freeze current source/build provenance. Keep the deferred crash visible.
2. Land the namespace-correct attribute representation with migration tests.
3. Land rooted SubmissionTransaction -> entry-list -> NavigationRequest and
   remove remaining DOM-late-read/global-helper submission paths.
4. Prototype the upstream product adapter immediately: unchanged window,
   standalone-worker, XML-family, reftest and crashtest fixtures, including
   deliberately failing controls. Record worker-global dependencies honestly.
5. Resolve and approve agent/process isolation ADR before changing placement.
6. Run the first signed architecture-cutover family suite and publish both new
   failures exposed by broader execution and verified gains. Replan estimates
   using measured failure mechanisms, not optimistic file totals.

Use the companion plan's work-package dependencies for cutovers. Multi-document
DOM ownership, authenticated environment routing and host ownership extraction
precede connected-popup activation. The upstream adapter must pass its pinned
product/protocol contracts; the current wpt_session run-and-poll command is not
that general browser-control protocol.

## References

- Current boundaries: `WPT-CORE-COMPLETION-GATES.md`, inventory-v3 plan/coverage,
  `WPT-HARNESS-V4-REPAIR-LEDGER.md`, `WPT-NATIVE-ACTIONS-2026-09-12.md` and the
  September 13 form/submission reports. Earlier completion plans are historical
  context; their implementation claims do not substitute for current evidence.
- [V8 embedding](https://v8.dev/docs/embed) and
  [html5ever component boundary](https://github.com/servo/html5ever).
- [HTML agents and event loops](https://html.spec.whatwg.org/multipage/webappapis.html)
  and [form submission](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#form-submission-algorithm).
- [wptrunner design](https://web-platform-tests.org/tools/wptrunner/docs/design.html)
  and [executor interfaces](https://web-platform-tests.org/tools/wptrunner/docs/internals.html).
