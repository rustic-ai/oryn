# Dynamic inert focus fixup — 2026-09-12

Subsequent checkpoint: [computed-style focus eligibility](WPT-FOCUS-STYLE-2026-09-12.md).

Core conformance remains incomplete. This batch implements focus fixup when a focused element becomes inert through an attribute, ancestor, or slot-assignment change. It does not claim to finish the inert feature or repair cross-frame Window focus.

The native rendering checkpoint checks live inertness after script/microtask work, clears the focused element, and dispatches trusted blur/focusout events in its owning document realm. It preserves focus moved by a reentrant blur handler. A transient inert toggle removed within the same task does not spuriously blur. Same-origin frame documents share an arena, so the fixup explicitly checks the nearest document root before dispatching.

Native validation: six focus tests pass, including two new dynamic-inert tests; 203 V8 binding tests pass with three existing diagnostics ignored. Clippy with warnings denied, formatting and whitespace checks pass. The reference evidence gate's 12 tests and the kernel focus-lease serialization/EOF/crash test pass.

The Chromium reference issue is still open. Additional probes distinguished an actually unfocused new-document observation from a missing CDP callback. Enabling Page events, renderer-initiated navigation, and calling native Window.focus during document initialization did not establish reliable readiness; the latter two experiments were reverted. The fail-closed comparison gate remains. No unresolved reference result is promoted to qualified conformance.

The full objective and required coverage are recorded in [Core completion gates](WPT-CORE-COMPLETION-GATES.md). Historical diagnostics and the deferred Range.insertNode crash remain unchanged.

## Signed results and exact-source qualification

The full 29-execution inert slice now has **8 raw passes**, up from seven before dynamic focus fixup (and two before this inert work began). The dynamic-inert case passes all six named subtests through the signed worker.

The first run, `inert-dynamic-v1-2026-09-12.json`, completed in 62.733 seconds with an overly early document-creation focus observation. Its reference failures are preserved; it is not qualified. SHA-256: `e005e77dc5da9a8f40e6480b71dfb9be3fc64cf34cc389b13dc5c488ffdcc263`.

The reporting adapter now establishes native Window focus and records hasFocus before the subsequent test scripts. The original document-creation observation remains diagnostic, not a substitute for test-entry readiness. Source audit must establish that the tests/setup callbacks follow this adapter; this mechanism does not qualify arbitrary script order or all navigation/focus behavior. No focus emulation is enabled.

The [final signed run](../../artifacts/wpt-full-baseline/inert-dynamic-v2-2026-09-12.json) completed in **134.201 seconds**. SHA-256: `40e7ad01239ed2fb18754059cf7a2eb4d5fc550c25687f6e86b78c9e78d61b13`. The eight passing executions match Chromium and have clean current prerequisites in this exact run. Their pinned sources, helper/adapter bytes, resources and automation counts were audited in [source-bound reviews](../../benchmarks/conformance/wpt-inert-dynamic-v2-reviews.json).

[Qualification](../../artifacts/wpt-full-baseline/inert-dynamic-v2-qualification-2026-09-12.json): **8 qualified executions / 23 passing subtests**, 13 additional review candidates, and eight unresolved executions. This is not an inert-feature conformance claim or an updated whole-Core score. In particular, the contenteditable consistency test does not prove any particular editing result, and the disabled-selector test does not assert the computed color it reads.

Final signed identities: parent SHA-256 `517193c830ebbeebc4b35bf8ced9b647d2e05aa0a134c2fa43b36c2141428c69`; worker SHA-256 `fb03565608390380a47f509e7dae1c7ce76f8a9421c270674ff786101d5537f2`; CDHash `8948377f58ecb7444fd4d5c0fbbc85deb8a63c35`. Protocol-v9, designated requirement and sandbox-denial probes passed. Evidence remains dirty-tree diagnostic evidence, not release attestation.

Reference readiness remains intermittent outside these qualified executions: the seven-test live session run passed, including four concurrent parser-focus controls, but the subsequent full 125-test live suite had 124 passes and one reference-focus rejection in fixture parity. That failure is not waived or converted to success. The current runner emits no reference comparison when the required focus observation is absent/false. Next work remains reliable reference readiness and the host-owned frame-focus controller, followed by the remaining Core failure groups.
