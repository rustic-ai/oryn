# Native scheduling and window performance

The upstream WPT sequence remains context creation, window selection, blank-document readiness, document-element lookup and a real focus click. Every test receives a fresh context/worker according to the existing placement rules. Performance work must preserve lifecycle, origin, focus, event ordering, timeout and retirement semantics.

## Component ownership

Native PageState orchestration delegates to private scheduling, lifecycle, semantic-projection and layout-projection modules. RawV8Host delegates task/timer/observer draining and runtime polling to its private scheduler module. These are real Rust modules with unchanged public browser interfaces; they are not additional Cargo projects or candidate copies.

## Checkpoint fixed point

Every executable checkpoint performs at least one complete pass, including external observations and callbacks in their original order. The previous eight-pass bound remains for active or uncertain work. Early exit requires equal before/after identities and DOM mutation cursors for all live realms in the selected root, no runnable executable work, and no pending local/observer/creation/discard/release work. The identity includes binding instance, document, generation, stream, authority and cancellation epochs. Missing state and borrow conflicts conservatively retain another pass. A terminal-control checkpoint follows readiness inspection.

The witness does not authorize a platform operation or reuse a previous WindowState response. External events are sampled again on the next invocation; IPC/callback boundaries keep their original live checks. Future or otherwise uncertain retained queues may conservatively keep all eight passes. Diagnostic counts never determine idleness.

## Projection reuse

An absent pending navigation response returns explicit NoWork, allowing the scheduler to omit only its immediate second semantic synchronization. Deferred, obsolete and completed responses retain Processed; errors and retirement preserve custody and original precedence. Layout caches still compare complete inputs. A semantic revision alone cannot establish unchanged CSSOM, fonts, geometry, focus, validation, media or animation, so this change does not introduce that shortcut.

## Diagnostic interpretation

Native timing remains optional and capability-controlled. Nested stages aggregate calls, inclusive/exclusive elapsed time and counters in memory; they do not write a record per idle poll. Actor-turn summaries may be coalesced, with explicit merged count, time range and missing/incomplete status. A returned synchronous scope is not a completed WebDriver command or proof of process settlement. Retained navigation identity is labeled separately.

The request.* counters measure request-API attempts and elapsed calls, including possible cache/preload waits; they do not universally count physical IPC exchanges. Wall time spanning merged turns includes gaps; summed scope durations and per-stage durations must be interpreted separately. Reject truncated diagnostics for claims requiring complete coverage.

The developer window probe accepts `--no-native-timing` for an overhead comparison. It records that setting and removes only the validated native diagnostic descriptor argument; upstream setup, Python timing, collector cleanup and browser behavior remain unchanged. This option does not apply to official admission or disable protocol custody.

## Evaluation

The retained baseline under oryn-data/migration/verification/native-performance-baseline-20260928-01 contains three serial sessions of 20 measured windows each. Median 3.73655s, p95 3.97598s; all 60 setups and the three separate smoke tests completed with valid cleanup and input custody. No warm-up samples were discarded. This baseline is a developer profile, not a conformance result.

Use the same sample count/configuration against a newly built and signed identity. Report first windows separately, median/p95/max, create/close/setup/cleanup costs and outcomes. Compare detailed diagnostics on/off before attributing small gains. Six-worker throughput requires the exact preserved 50-URL population and truthful new-build admission, in a fresh original. Never stitch partial outcomes or replace failures.

Sub-second is an investigation milestone, not an asserted result. Worker pooling, synthetic readiness, focus shortcuts, speculative cross-turn authority caches and unrelated crate changes are outside this refactor. Further layout-cache invalidation or IPC-notification redesign requires evidence that the remaining cost justifies it and regression coverage for the affected observations.
