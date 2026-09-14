# Core interaction conformance track — 2026-09-12

Full conformance is **not achieved**. This track closes native cross-frame automation evidence collection, establishes a fresh complete mapped interaction slice, and fixes three confirmed Core event failures. Historical evidence and the deferred Range.insertNode crash remain unchanged.

Subsequent work: [inert focus and reference-focus repair](WPT-INERT-FOCUS-2026-09-12.md). The stricter document-entry focus gate means this report's historical qualification checkpoint does not establish current harness readiness.

## Evidence repair

Automation observations now live in bounded native page-isolate state. The host assigns realm and document-generation identity; caller-local operation IDs cannot collide across frames. Observations survive frame removal, cannot be read through a page-visible JavaScript function, and do not survive a fresh page isolate. Collection rejects invalid transitions and caps observations at 4,096. Incomplete, malformed and overflowing traces cannot satisfy current qualification checks.

The WPT parent retrieves the native snapshot after completion, rather than trusting only the top document's log. Reviewers can require an exact completed-call count, so an empty parent log cannot substitute for the two operations performed in the cross-origin action canary. These are source-bound diagnostic observations, not a tamper-proof attestation against arbitrary author scripts calling private host hooks. Separate auxiliary-page processes are not aggregated by this page-isolate collector.

The external-reference manifest was updated with the new callback and its ordering regression passes. Native cold/prewarmed live tests verify two realms using the same local ID, retained observations after frame removal, and no carryover into the next page.

## Complete mapped interaction assessment

The [inventory](../../benchmarks/conformance/wpt-core-interaction-v1.json) selects **all mapped Core executions** in four roots from the full pinned manifest, without expected-outcome filtering. These roots contain 183 official entries: 172 mapped executions and 11 unsupported runner entries (four focus, seven UI Events). The unsupported entries remain visible in the full coverage ledger; they are not passes.

[Raw assessment](../../artifacts/wpt-full-baseline/core-interaction-v1-2026-09-12.json): **172/172 finished in 393.113 seconds**, using eight isolated-prewarmed workers under a persistent parent and fresh direct fixtures, with Chromium comparison. SHA-256: `afd39ec570b0c1876f3cf91a85e4834bc544ea2fd1c8c1e663e9cf9fa4313e3b`.

| Root | Executions | Raw passes | Raw failures |
| --- | ---: | ---: | ---: |
| focus | 39 | 5 | 34 |
| inert | 29 | 2 | 27 |
| input-events | 33 | 2 | 31 |
| uievents | 71 | 17 | 54 |
| Total | 172 | 26 | 146 |

The [triage ledger](../../artifacts/wpt-full-baseline/core-interaction-v1-triage-2026-09-12.json) separates **77 review candidates** (18 raw passes, 59 raw failures) from **95 unresolved executions**. Unresolved reasons overlap: 55 have unverified reference completion, 34 unqualified automation execution, 28 unverified reference subtest identity, and three incomplete native automation traces. These counts must not be summed. Completion and resource gates apply independently. A raw pass is not automatically qualified, and a timeout is not automatically a confirmed engine defect.

This is not a full Core baseline. The 172-test evidence uses worker SHA-256 `5834b1af310992cdce9c1e3998ecbdfb61c00691697a0d3332f305270c6a9a3e`; it is not silently relabeled as evidence for the later repaired worker.

## Confirmed repairs and verification

Three failures had valid matching Chromium subtest identities and directly reproducible native defects:

1. `event-getmodifierstate.html`: added native `modifierAltGraph` and `modifierCapsLock` dictionary state and independent getModifierState lookup. All 16 constructor subtests now pass.
2. `mouse/attributes.html`: enter/leave input events are non-composed, as well as non-bubbling and non-cancelable. The actual mouse-event assertions now pass.
3. `interface/click-event.htm`: HTMLElement.click creates a native, untrusted PointerEvent with reserved pointerId -1, empty pointerType, zero coordinates/detail and no consultation of replaceable public constructors. Device action click/auxclick events also use PointerEvent rather than MouseEvent.

[Final signed regression run](../../artifacts/wpt-full-baseline/interaction-regressions-v1-2026-09-12.json): **10/10 executions and 25/25 named subtests passed**, all matching Chromium, in 43.058 seconds. This includes the three repairs and seven action controls, with complete native traces and clean current harness checks. SHA-256: `a7647f58b01da4a2761ae447893b42f3739d927cb91d74cadb28b03f9f70e4f6`.

[Exact-source reviews](../../benchmarks/conformance/wpt-interaction-regressions-v1-reviews.json) and the [qualification result](../../artifacts/wpt-full-baseline/interaction-regressions-v1-qualification-2026-09-12.json) qualify only these ten diagnostic executions. They do not promote the entire action capability or the 172-test slice. The full slice has not been rerun after these three repairs; no updated aggregate pass count is claimed.

Final signed identities:

- Parent SHA-256: `c4dc296e666eaf809b3be07b513fb67eae430edb020505fc698b23e81685403f`.
- Worker SHA-256: `cbb163cef52ff340904e55fd4c9b643de62e1ddc6b3c5785652f6c6fba5c8ba9`.
- Worker CDHash: `a6cc0a97150c4247773f00d0dd0a10829b613b03`.
- External-reference manifest SHA-256: `b8464b91295746973a44e9cc7921bff7421f059aaae686038bd14af924ef1d92`.

Verification: 118 live Python harness tests passed with zero skips (95.917 seconds); 21 Node action/adapter tests passed; V8 binding tests passed serially (202 passed, three existing diagnostics ignored); four native focus tests, three native automation-trace tests and the startup-reference ordering test passed. Native/V8 clippy with warnings denied, formatting, whitespace and generated-registry checks passed. Signed protocol-v9, designated-requirement and filesystem/listener/outbound/secret-environment sandbox probes passed. The earlier parallel V8 trap was not investigated or declared repaired here.

## Remaining work, in priority order

1. **Evidence/reference coverage:** resolve the 95 unqualified completions by their exact blocker. Add supported reference input variants and auxiliary-page trace collection where required; never substitute missing operations with no-ops. Audit the 77 candidates individually against their actual prerequisites.
2. **Frame focus controller:** replace the frame proxy's no-op focus behavior with host-owned focused-realm/ancestor state and correct Window/element focus and blur ordering. Cover reentrancy, same/cross-origin frames, navigation, removal and auxiliary windows. Missing Window focus/blur events are directly observed in the focus slice; not all 34 focus failures are asserted to share one cause.
3. **Inert interaction rules:** implement reflected inert state and flat-tree eligibility consistently for focus, hit testing, editing and selection, including slots/shadow roots, dynamic changes and modal-dialog exceptions. The 27 failing inert executions are a priority bucket, not a claim that one guard repairs all of them.
4. **InputEvent/editing semantics:** typed native payloads, null dataTransfer defaults, correct beforeinput StaticRange snapshots, empty input-event target ranges, fresh returned sequences, and editing-host boundary/default-action behavior. Keep constructor semantics separate from native editing synthesis.
5. **Remaining UI Events:** click retargeting/cancellation, keyboard activation, pointer boundary sequencing, legacy event APIs and timing-sensitive interpolation. Reproduce timing-sensitive failures at controlled concurrency before diagnosing their root cause.

After each coherent fix, run exact failing cases and controls, qualify the supported slice, then refresh a comparable full Core run. Keep the pinned full-manifest denominator and Core/necessary/extended classifications; the historical 4,022 subset is not an acceptance target. Clean-source release evidence remains a separate final gate.
