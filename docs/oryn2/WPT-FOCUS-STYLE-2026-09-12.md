# Computed-style focus eligibility — 2026-09-12

Core conformance remains incomplete. This batch extends the shared native focus path to reject CSS-suppressed targets rather than special-casing an upstream test.

- `display:none` on the target or its ancestors prevents focus.
- Resolved `visibility:hidden`/`collapse` prevents focus. A descendant's `visibility:visible` override is respected; hidden ancestry alone is not sufficient to reject it.
- Lack of a layout box alone does not reject focus, preserving relevant canvas fallback content.
- Focus eligibility is rechecked after blur handlers, and dynamic CSS suppression participates in native rendering-checkpoint focus fixup.
- Style state comes from the native layout provider, not the replaceable JavaScript getComputedStyle function.

Seven native focus regressions pass, including stylesheet-based suppression, visibility override, canvas fallback, dynamic hiding, and reentrant hiding. The first version of the new regression accidentally used Window.parent instead of the element with ID parent; it was corrected to explicit element lookup. V8 binding tests: 203 passed, three existing diagnostics ignored. Clippy with warnings denied passes.

This does not complete all focusability rules, sequential navigation, slot/rendering behavior, or frame focus. The source review of `inert-form-control.html` also confirmed a separate form activation track: native HTMLElement.click currently queues navigation defaults, while checked/defaultChecked are still attribute aliases. Repair requires actual checkedness/dirty-state semantics, radio grouping, preactivation/cancellation, and label forwarding—not an inert-specific toggle.

The full Core boundary remains [Core completion gates](WPT-CORE-COMPLETION-GATES.md). Reference-focus readiness remains separately tracked and fail-closed; no full Core or release attestation is claimed.

## Signed verification

The unchanged 29-execution inert inventory now has **9 raw passes**, compared with eight before this change; none of the previous passes regressed. The newly passing canvas fallback execution passes all **12 named subtests**, matching Chromium.

[Signed raw run](../../artifacts/wpt-full-baseline/inert-focus-style-v1-2026-09-12.json): 123.121 seconds, SHA-256 `23e5fe5967e600cd4d944781d3becfee3e07ba2a23ad1df9e44fd8cb9e4b2a18`. Isolated prewarmed workers, persistent parent, two jobs, fresh direct fixtures, production limits.

[Source-bound reviews](../../benchmarks/conformance/wpt-inert-focus-style-v1-reviews.json) and [qualification](../../artifacts/wpt-full-baseline/inert-focus-style-v1-qualification-2026-09-12.json) establish **9 exact executions / 35 passing subtests**. The remaining results are 11 review candidates and nine unresolved executions. These qualifications are restricted to the audited sources, tested worker and satisfied prerequisites, not the entire inert feature or Core tier.

Signed identities: parent SHA-256 `517193c830ebbeebc4b35bf8ced9b647d2e05aa0a134c2fa43b36c2141428c69`; worker SHA-256 `ae200e78360c9002fa08a5d85a9584b630d38b9959170181094df5c7fc114ed8`; CDHash `b0ab976cdfcafaff8fb04c02bb70060103d744a5`. Protocol-v9, designated requirement and filesystem/listener/outbound/secret-environment sandbox probes pass. This remains signed dirty-tree diagnostic evidence.
