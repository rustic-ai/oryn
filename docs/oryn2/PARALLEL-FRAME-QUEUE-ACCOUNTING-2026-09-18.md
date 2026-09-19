# Frame queue accounting source closure — 2026-09-18

Source generation only. No build, test, formatter, generator, service, WPT, or physical qualification has run for this change.

## Confirmed gaps closed

| Custody boundary | Generated source |
| --- | --- |
| Request admission | `frame_resource_preparation::frame_request_bytes` accounts typed request URL, creator/frame authority, post capacity and MIME, provenance, encoding/referrer and optional stylesheet policy metadata. Standalone request charges URL/referrer/post and retained Loader separately before IO launch. |
| Captured loader | `standalone_frame_resource::Loader::retained_bytes` accounts embedded type, encoding/container policy, ancestry, CSP and Permissions Policy, request provenance and local blob payload/policy. Shared broker/report backing retains its own owner. |
| Response publication | `reserve_frame_response` combines root-owned complete `FrameResourceResponse::retained_bytes` with standalone FramePolicy. Rejection drops response/policy; standalone broker is not transferred; Unix body lease is explicitly canceled. |
| Completion metadata | Owner reservation covers preparation/completion, attributes, ancestor snapshot and terminal error room. It follows the queue into completion. Navigation context receives its own reservation. Attributes, ancestor snapshot and context move instead of cloning. |
| Postlaunch context rejection | Cancels local/native resource guards, invalidates weak producer lifetime, disconnects late response channel and stores one bounded error completion. Navigation observer receives Failed. The accepted operation cannot silently retry with missing context/default request semantics. |
| Departure | `frame_departure::admit_frame_departure` takes mutable context custody and moves it into retained Departure; navigation lane updated caller accordingly. |
| Error fallback | Submission/parser errors shrink/truncate UTF-8 safely to 512 bytes, inside admitted terminal-error room. |

## Accounting precision and ownership

`retained_size::Size` uses checked arithmetic, String/Vec capacities and a conservative BTree node estimate. It is not an exact allocator-overhead measurement. Origin and LocationAncestorCapture expose retained allocation helpers. CSP and standalone policy helpers include their nested containers.

Url does not expose allocation capacity: Loader uses twice serialized length as an estimate for its captured URLs, with original owned request strings charged independently. Structured-clone history state uses the existing WireValue payload accounting contract; this does not claim allocator-exact accounting for every structured-clone backing. Shared SessionHistory/ArchivedSource backing retains its existing reservation rather than charging another full copy.

NavigationTiming clones share document/BFCache-owned state. Response-body queue credit is not extended to cover that later document lifetime. Exact frame installer validates ResourceTimingMetadata before copying redirect provenance. Valid metadata bounds 21 hops and 256 KiB aggregate hop text, 64 KiB origin, 8 KiB content type; server timing parsing caps 64 KiB and 256 metrics; direct timing redirect storage caps 20 redirects/64 allow-origin entries, and stages have seven keys. A universal document-name truncation was not introduced: names remain author-observable. The follow-up below closes the confirmed retained timing metadata admission gap; it does not truncate navigation names or retain response-body credit after commit.

## Regression sources generated

All in `frame_resource_preparation.rs`:

- `response_policy_capacity_is_charged_and_rejected_policy_is_released`: reserves successful response/policy, checks reserved String capacity, forces full-budget rejection and verifies credit release.
- `context_capacity_failure_cancels_original_producer_and_queues_one_terminal_failure`: producer cancellation flag, weak identity invalidation, rejected late send, one Failed observer result and one completion.
- `accepted_metadata_moves_into_completion_and_releases_with_last_owner`: original allocation identity survives queue transfer, credit stays held and drops with completion.
- `typed_frame_request_charges_reserved_post_and_policy_metadata`: unused post/MIME/referrer/policy/integrity capacities count.
- `bounded_error_releases_excess_capacity_and_preserves_utf8`: short oversized backing shrinks; multibyte error truncation remains valid.

Existing stale frame identity, stopped-load, producer-held transport and late-response tests remain, including navigation lane portable held-broker sources. None were executed.

## Source review disposition

Producer construction, completion literals, Unix body cancellation and standalone policy/broker transfer were reviewed against both scheduling routes. Root retains ownership of complete response accounting; navigation retains ownership of portable frame drain and lifetime guards. Source integration and actual failure/retirement execution remain part of the global build/test phase.


## Follow-up: document timing admission closed in source

The caller audit confirmed a real ownership gap: NavigationTiming is native Arc/Mutex storage, outside V8 heap accounting. Existing root policy bounds reusable BFCache documents to two per PageState (`runtime::MAX_CACHED_DOCUMENTS_PER_PAGE`) and cached plus retiring shared-agent roots to 32 (`shared_host_cache::MAX_RETAINED_CACHE_ROOTS`). Frame history has 4,096 entries, 1,024 archived source/control entries, and source/session/form metadata reservations. Those limits do not themselves reserve timing metadata; no universal custom-broker header or author URL length cap was assumed.

### Final producer/consumer mapping

| Producer | Admission / retained owner |
| --- | --- |
| Root HTTP, parent and portable | navigation_http collects exact URL, measured response, redirects; runtime/navigation_runtime prepare local/buffered/blank sources; RawV8Host seed admits metadata before document installation. |
| Initial and dynamic frame | Validated frame response installer produces timing; commit_frame_document_with_timing admits against host owner before replacing old bindings. Its queue response reservation remains independently live until completion release. |
| Default document, initial auxiliary and document replacement | Common install_native_document preflights timing before previous-authority retirement and install_generated_bindings_with_root. Seed stays retained until successful installation. Initial-frame reuse has its own preflight before replace_initial_document. |
| BFCache, prerender, shared-agent auxiliary | Timing Arc retains the original source owner and lease. Cloning does not charge twice; another budget identity is rejected. Scalar lifecycle/activation/EOF updates retain precharged storage. |
| Late metadata update | Fallible name/response/redirect/unload setters reserve prospective candidate storage while the old lease remains live, before cloning Record or parsing headers. On quota failure no candidate metadata is published and original state/lease survives. Production root/frame callers propagate or queue terminal failure. |

New native `navigation_timing_custody::Budget` adapts the existing `ScriptSourceReservation` rather than introducing a different limit. Isolate creation establishes the shared source owner; main and both auxiliary DynamicScriptScheduler initialization paths adopt that owner. Stop preserves it. Timing metadata includes exact owned string/vector capacities, nested metrics, transport exposure, redirects and fixed conservative room for Arc/Mutex and all seven stage-map entries. Prospective updates reserve conservative container-growth/transform allowance before new copies, with old allocation credit still live. Incoming owned URL/details/header buffers remain the producing caller's custody before document admission; this is not a claim that every transport's incoming allocations are precharged by the timing module.

No author-facing name truncation, metadata discard on quota exhaustion, or transfer of whole response-body credit into cached documents was introduced. The original source owner remains alive as long as any cached/current timing clone remains.

### Added regression sources

`navigation_timing_custody.rs`: cached clone/original-owner/drop, separate response-queue plus document admission, quota-failed postadmission response/unload preserving old metadata, foreign-owner rejection and exact long URL preservation.

`rust_executable.rs::navigation_timing_document_admission_tests`: actual root/child host and scheduler share source owner across stop; rejected timing seed preserves incumbent document and credit. Existing dynamic-fetch and frame-queue tests now compare against retained document credit, not zero. Existing timing tests assert the newly fallible setters succeed.

All source only. No tests/builds were run. Source inventory disposition: the retained timing metadata gap is implemented; execution qualification remains the global validation phase.


Final timing reconciliation: Navigation Entry wrappers and PerformanceObserver lists now share the charged canonical record name; both Performance and PerformanceObserverEntryList getEntriesByName predicates match that canonical name without cloning it. Navigation entry getters already read the canonical record. Added clone/filter source regression. Server-Timing parsing reserves separate temporary credit for both geometric `Vec<&str>` split buffers (bounded by the existing 64 KiB parser input limit); this credit drops after update and is not permanently attached to the document. A separator-heavy header regression forces scratch rejection while ordinary metadata credit remains available, then verifies successful retry releases scratch.
