# Standalone service-worker authority audit

2026-09-18, source inspection only; no execution.

## Requirement and actual route

The completion plan's L deliverable requires shared resource machinery usable by documents, frames and workers; W explicitly requires service-worker registration/install/activate/control/fetch/update. The combined exit requires evidence for every claimed platform and execution policy, with signed macOS the primary current target. Nothing in that contract permits fabricating successful standalone interception or silently excluding a claimed mode.

| Actual route | Source evidence | Service-worker disposition |
| --- | --- | --- |
| Unix BrowserContext page, including in-process execution | `runtime.rs` page actor creation obtains the configured context client or creates `SharedContextClient::direct` using the existing real BrowserContext and allocated PageId. Document constructors receive that client. | `host.rs` installs NativeWorkerBackend and ContextPlatformHost. Parent HTTP uses exact Window Creator and the actual context ServiceRegistry controller. The recently added prefetch route reaches this same interception path with empty destination. This is existing production source, not proof of passing tests. |
| Parent-backed child/root actor | Document's ContextState carries the authenticated endpoint; host installs worker and HTTP backends together. | Existing native context/profile/agent service ownership, controller selection and retained FetchEvent/body pipeline. |
| Raw `RawV8Host::new_with_network` / standalone RustExecutable | No ContextState. Modern standalone HTTP captures native root/frame creator, source policy and broker. `host.rs` does not install a WorkerTransport in this branch. | `service_workers.rs::request` rejects missing transport. There is no registered controller to intercept Fetch. The HTTP engine alone cannot implement registration/control. |
| Non-Unix Browser path | `agent_service`, `agent_io`, `agent_script`, `service_registry` and service network declarations are Unix-gated. | Genuine independent agent/service authority and transport must be made available for any claimed conformant execution policy. Merely adding an HTTP callback does not satisfy W or Core controller dependencies. |

## Required integration shape

1. Bind the actual standalone root/frame lifecycle to a native browsing-context service owner and partition, without allocating a substitute page or consulting a successor root. Registration, clients, worker execution, messaging and fetch must use the same authority.
2. Supply the real WorkerTransport registration/control backend before author code and reconcile document replacement, frame removal, BFCache suspension, auxiliary roots and BrowserContext shutdown against that owner.
3. Capture the selected controller/version and exact client at HTTP admission. Ordinary Fetch may consult that authority at admission; accepted keepalive must retain its frozen controller through source retirement. FetchEvent request destination/mode/credentials/redirect/cache/referrer must match the eventual network request. A missing controller means ordinary network fallback; missing required authority must not be converted into an uncontrolled success.
4. Reuse the existing bounded service response/body and cancellation-ack machinery, including redirects and response policy, rather than inventing an interception-only mock implementation.
5. Generate actual standalone registration→activation→claim→Fetch/prefetch regressions, retired/replaced owner and frozen keepalive controls, and mode/partition negative cases once the production native authority is available. Unix primary browser coverage and low-level standalone coverage must remain distinguishable in qualification.

Authority lane owns the context/agent ownership determination and any necessary extraction; parser/loading lane owns the narrow standalone HTTP captured-controller consumer once that genuine backend is available. Renderer owns portable auxiliary-root integration and is coordinated. No placeholder callback or fake PageId was added.

## Current contract disposition — authority reconciliation

The integration shape above is **conditional on claiming the ownerless or
non-Unix backend**; it is not a newly discovered missing implementation for the
current signed macOS BrowserContext target. Root explicitly directed against
adding a speculative backend or duplicate constructor merely to expand scope.

Source chain for the actual primary target:

1. `BrowserContext::create_page_with_policy_and_actor_visibility` in `runtime.rs`
   uses the configured client or constructs `SharedContextClient::direct` from
   the real context and allocated page. This includes Unix in-process pages;
   “in-process” does not mean the raw `context=None` convenience API.
2. PageState document construction passes that client into
   `load_stream_with_context_and_limits`, `load_parsed_with_schedule`,
   `load_with_context_and_limits`, or same-agent `load_in_agent`. Restored and
   related roots retain their real endpoint and native lifetime.
3. `RawV8Host::new_with_network_context_and_heap_limit` obtains native document
   authority before creating bindings. This public Unix constructor already
   exists for embedders that provide real ownership; no extra convenience
   constructor is required.
4. `install_native_document` installs `NativeWorkerBackend` and the native
   platform/message/history backends from the same ContextState before script.
   NativeWorkerBackend sends WorkerService; ParentContextService validates the
   source socket page and `agent_service::window_request` resolves native exact
   Window/Agent authority, context ServiceRegistry and actual executor.
5. HTTP uses `existing_service_registry`/`fetch_operation_with_check` for ordinary
   requests and the frozen controller for accepted keepalive. Page/navigation
   brokers use ServiceNavigationBroker for document loading. Controller selection,
   FetchEvent, response/body ownership and cancellation therefore share the
   registration's real context and principal.
6. Document/page retirement detaches client ownership; service registration and
   profile state are not document-owned. Actual context shutdown retires agents
   and IO; frozen accepted keepalive is exempt from document retirement only.

Claim review inspected the plan's combined exit and W section,
`benchmarks/conformance/runtime-completion-v1.json`,
`test-harness/workflows/five-family-contract-v1.json`, and the current
`completion_provenance.py`, `completion_execution.py`,
`completion_qualification.py`, `workflow_qualification.py` and V8-profile
validator sources. No explicit raw-standalone or non-Unix service-worker
conformance claim was found there. Profile options and independently evidenced
worker placement are not a claim that ownerless constructors implement browser
context service ownership. The primary macOS target and frozen denominator are
unchanged.

**Disposition:** primary native source ownership/control/interception is mapped;
its existing regression and final signed-runtime qualification remain required.
Raw ownerless convenience APIs and non-Unix service execution remain unclaimed
limitations for this qualification contract. Component tests using those APIs
cannot substitute for browser service-worker conformance evidence. No stub
controller hook, fake PageId, hidden replacement BrowserContext, feature switch,
manifest omission, or portable agent implementation was added. A future explicit
claim requires the genuine ownership work listed above and its own evidence.
