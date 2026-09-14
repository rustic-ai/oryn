# Inert focus and reference-focus repair — 2026-09-12

Subsequent checkpoint: [dynamic inert focus fixup and exact-source qualification](WPT-DYNAMIC-INERT-2026-09-12.md). This report preserves the earlier state and failures.

Full conformance and the complete interaction backlog are not finished. This change repairs explicit inert focus eligibility and makes a reproduced Chromium reference-focus defect fail closed. Reliable reference focus across fast navigation is still unresolved. The cross-frame Window focus controller and deferred Range.insertNode crash remain unchanged.

## Implemented

- `HTMLElement.inert` reflects attribute presence, with JavaScript boolean conversion on assignment. Inherited inertness does not change the element's reflected property.
- Native focus rejects explicitly inert targets and descendants before blurring the previous target, and rechecks after reentrant blur callbacks. Pointer-driven focus uses the same eligibility rule; the native DOM focus entry point also rejects inert targets.
- The eligibility walk includes shadow hosts and named/default slot assignment. `HTMLSlotElement.name` now reflects the slot name, rather than becoming an unrelated expando. Slot assignment is resolved from live attributes on each check.
- Chromium's `/json/new` target was starting in the background. An isolated parser-time input-focus probe reproduced `document.hasFocus() === false` and zero focus events. `Page.bringToFront` repairs individual probes, but repeated probes still lose focus across navigation even after preflight succeeds. Serializing reference browser lifetimes with a kernel advisory lock was insufficient to eliminate this race. The lock releases on EOF/crash and leaves native Oryn executions parallel.
- The harness now also observes focus in the new top-level document before upstream scripts execute. If it is unverified, it emits no comparison result and reports `Chromium document-entry focus unverified`. This observation is conservative: it can reject a document that gains focus later. No focus emulation, `hasFocus` override, or synthetic event substitution is used.
- The current evidence gate requires initial focus, document-entry focus, and host reference isolation. Historical artifacts are immutable; earlier qualification checkpoints do not establish this stricter gate. This is an evidence-integrity repair, not a claim that reference availability is fixed.

## Scope boundaries

This is not complete inert support: dynamic blur when a focused node becomes inert, hit testing, editing/selection, find-in-page, modal/fullscreen exceptions, and general focusability/rendered flat-tree rules remain separate work. The new HTMLSlotElement surface does not implement assignedNodes/assignedElements, manual assignment, or slotchange.

Frame-focus investigation confirmed that `__oryn_make_window_proxy().focus()` is a no-op and top-level `focus()` only asks the page broker to activate a page. A proper repair still needs host-owned focused-realm/ancestor state, native Window/element event ordering, reentrancy, and revocation on frame navigation/removal. No synthetic forwarding patch was applied.

## Verification

- V8 bindings: 203 passed, three existing diagnostics ignored; serial execution.
- Native DOM: 61 passed. Existing native focus regressions: four passed.
- Node action and adapter regressions: 21 passed.
- Formatting, generated WebIDL check, clippy with warnings denied, and whitespace checks passed.
- Signed worker protocol-v9, designated requirement, and filesystem/listener/outbound/secret-environment denial probes passed.
- Offline harness suite: 123 tests, 103 passed and 20 live prerequisites skipped. The final live session subset had six passes and one failure: `test_chromium_fixture_completion_parity` rejected an unfocused new Chromium document. An earlier 122-test live run passed before the stricter document-entry check; that green run does not establish final readiness.
- Experimental original-tab and debugger-entry barriers did not resolve the race (the latter timed out). Their temporary code was removed; it is not part of the production runner.

## Signed diagnostic results

The complete mapped inert slice has **7/29 raw passes**, compared with **2/29** in the preceding interaction assessment. Both new inert runs produced the same seven passing executions; they contain 17 passing named subtests. The five newly passing executions are shadow-host inertness, slotted inertness, direct/nested focus eligibility, and inert editability. The editability case improved through focus rejection; this does not establish general editing/selection inertness.

The latest completed inert run is [inert-v2-2026-09-12.json](../../artifacts/wpt-full-baseline/inert-v2-2026-09-12.json), 130.140 seconds, SHA-256 `58bfa078c75f44a50850af1e3a16e65875b50be721d5e6f4027d8953860e3e7e`. All 29 are unresolved under the final gate because that run predates the document-entry observation. Other overlapping blockers include five unresolved completions, three reference subtest-identity mismatches, four unresolved subtest completions, three unqualified automation executions, one execution-integrity failure, one unsupported automation case, and one unresolved reference completion.

The previous interaction controls still have **10/10 raw passes, 25/25 subtests** in [interaction-regressions-v3-2026-09-12.json](../../artifacts/wpt-full-baseline/interaction-regressions-v3-2026-09-12.json), 44.032 seconds, SHA-256 `977638048a5918557d645ddc9b879675ac117aaa8e4ef7d3a5732460ba32f294`. These ten likewise predate the final document-entry gate and are not newly qualified. No new qualification overlay was published.

Final signed parent SHA-256: `517193c830ebbeebc4b35bf8ced9b647d2e05aa0a134c2fa43b36c2141428c69`. Worker SHA-256: `12edc37bcbdb87a54feafdc944fd7e98349dab5e1260e472f4ca2916c81362fe`. Worker CDHash: `318f752f4cfcb951c410dc3c39e6d6c0c443a9df`.

Next: resolve real reference focus across renderer navigation without focus emulation, rerun and qualify the unchanged selected sources, then implement the host-owned frame-focus controller and remaining inert interaction rules. Do not remove the new evidence gate to recover a green status.

All new worker evidence is signed dirty-tree diagnostic evidence, not clean-source release attestation. The pinned manifest denominator remains unchanged; no historical 4,022-test acceptance target is used.
