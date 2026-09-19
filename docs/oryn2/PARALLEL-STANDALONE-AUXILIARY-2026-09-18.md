# Standalone auxiliary Window source integration

Status: production and regression **source generated**, not built or executed. This is not WPT qualification evidence. Global implementation-before-validation directive remains in force.

## Native ownership

- `standalone_auxiliary_root_v8.rs`: native initial about:blank Document, inherited strict/opaque origin, base, CSP sandbox, native group/opener/creator membership, named target lookup, noopener publication, and retained native WindowProxy identity. No BrowserContext or ContextServiceClient is fabricated.
- `standalone_root_driver.rs`: bounded per-root PageState turns, using the existing native navigation/resource/parser/history pipeline. Unique RawV8Host custody moves to each driver and returns through current/pending/cached/recovery owners. Parent active-root enumeration is unchanged.
- The agent owns the driver table; document handles are weak references. This preserves auxiliary drivers when the opener navigates and avoids an ownership cycle. Constructor failures retain unclaimed root identities with an explicit retry deadline. The caller retains a driver-table handle independently of host custody and recovers the unique native host after an extraction failure; ambiguous owners remain retained and fail closed.
- `standalone_root_custody_v8.rs`: per-Document suspended microtask queues, native environment/timer suspension, proxy/authority restoration, and cached-root retirement. Suspending one popup does not suspend its opener's agent.
- `runtime.rs`, `shared_host_cache.rs`: standalone shared-host commit, native origin inheritance, provisional-root rollback, cached authority restoration, and retirement. Existing parent routes continue to use parent authority.
- `navigation_actor.rs`: standalone blob navigation captures immutable bytes and actual creator origin/CSP/permissions metadata from the native blob owner before departure. Revoked or foreign blobs fail admission. It does not route through parent-only blob RPC.
- COOP same-origin policy transitions allocate a new proxy/group and remove the opener edge. Same-group navigations reuse the original proxy. Cached native roots retain their distinct proxy custody for restoration.
- `standalone_script_close_v8.rs`: actual auxiliary creation permits multi-entry script close; the original embedder root still requires one native history entry. Close tasks select the current native Document and preserve logical closed through document replacement. Completed departure fences authored execution and retires all descendant environments.

## Generated regression matrix

`standalone_auxiliary_regressions.rs` covers synchronous blank DOM/global preservation, named identity, noopener creation, native HTTP navigation, same-agent proxy identity, BFCache restoration, cross-origin denial, auxiliary multi-entry close, opener liveness, sandbox popup denial, suspended popup timers, native blob navigation/revocation, COOP proxy severing, first-adoption failure wakeups and extraction-failure custody recovery.

`standalone_root_driver.rs::tests` covers opener navigation retaining the unique agent and auxiliary driver table.

Earlier close regression sources remain in `standalone_script_close_v8.rs`: logical-close ordering, full-history eligibility, canceled prompt, sandboxed descendant close, replacement Document selection, descendant retirement and post-departure execution denial.

## Verification still pending

All builds, compiler reconciliation, tests, complete Core WPT conformance, five-family qualification and report results remain deferred until the global generation barrier. No source row is a passing test claim. Primary Unix BrowserContext service-worker source is already present, per the shared applicability audit. This document makes no raw-embedder or non-Unix service-worker conformance claim; standalone auxiliary navigation does not substitute a parent or legacy Fetch fallback.
