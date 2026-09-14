# Native Tab navigation and reference keyboard repair

This completes the bounded Tab-navigation repair, not Core conformance. The
[full Core completion gates](WPT-CORE-COMPLETION-GATES.md) remain unchanged:
13,747 mapped executions and 2,990 unsupported entries remain in scope.

## Repairs

- Native uncanceled Tab keydown now advances focus; Shift+Tab reverses it.
  Synthetic page-dispatched keyboard events do not invoke this default action.
- Candidate ordering handles positive tabindex, default/zero ordering,
  negative starting points, disabled/inert/hidden exclusion, shadow scopes,
  named-slot assignment and radio-group stops. Unchecked radio groups select
  their first eligible member in both directions; eligible checked members win.
- Keyboard events carry legacy key metadata. Keyup and subsequent send-keys
  events are retargeted after focus moves, including closed-shadow targets.
- Traversal skips non-elements before reading tabindex. The first signed run
  exposed a doctype/text-node error absent from minimal unit fixtures. A new
  doctype/comment/whitespace regression covers it alongside reverse radio stops.
- Chromium reference input now supplies navigation-key code metadata and avoids
  DOM.focus for an ordinary non-focusable body target, preserving current focus.
  Unverified target state rejects before input. Native CDP input and existing
  focus gates remain in use; there is no synthetic reference event or focus override.

Reference input follows the [CDP key-event parameters](https://chromedevtools.github.io/devtools-protocol/tot/Input/#method-dispatchKeyEvent)
and [WebDriver keyboard interactability](https://w3c.github.io/webdriver/#keyboard-interactable).
This does not claim complete WebDriver key mapping, keypress metadata or modifier semantics.

## Signed evidence

All runs use the unchanged 64-execution
`benchmarks/conformance/wpt-label-associations-v1.json` inventory, two persistent
parents, isolated prewarmed page workers and direct WPT fixtures with Chromium
comparison. These are signed dirty-tree diagnostics, not release attestation.

| Candidate | Raw passes | Qualified executions / subtests | Elapsed |
| --- | ---: | ---: | ---: |
| label-associations-v2 | 32/64 | 29 / 298 | 494.651 s |
| keyboard-tab-v1, traversal defect | 32/64 | Not promoted | 555.991 s |
| keyboard-tab-v2 | 39/64 | 36 / 311 | 266.140 s |

No prior raw passes were lost. V2 observes 466 subtests: 394 pass, 71 fail and
one times out. Elapsed-time improvement reflects repaired timeout paths, not
a worker-startup benchmark or a full-Core performance measurement.

Seven newly passing, source-audited and qualified executions:

- focus-tabindex-negative, order, positive and zero: one subtest each.
- radio-focus-navigation-checked: five subtests.
- radio-focus-navigation-disabled and group-first-focus: one subtest each.

All seven match Chromium's subtest identities and outcomes with verified native
focus gates. Audited completed operation counts are 1/9/1/1 send-keys calls for
the Tab tests and 17/4/4 action sequences for the radio tests. Reachable upstream
send-keys, Actions and focus-utils helper paths were reviewed. Pinned source
hashes and actual response bytes were independently rechecked for all reviews.

Raw passes excluded from this candidate's qualification:

- radio-double-activate-pseudo: final inactive assertion does not independently
  prove keyboard activation; source audit remains withheld.
- clicking-noninteractive-unlabelable-content: attribute serialization changes
  generated subtest identities versus Chromium.
- inert-form-control: this execution's Chromium reference failed required focus
  and completion checks. The prior candidate's qualification is not carried over.

The independently signed inert-hit-testing probe returned `ok` again. Its new
artifact binds this worker, fixture and raw result; the earlier probe remains
immutable. Intermittent reference-focus failure is still open despite the
successful keyboard reference repair and full harness regression suite.

## Artifacts and identity

- Raw: `artifacts/wpt-full-baseline/keyboard-tab-v1-2026-09-12.json` and
  `artifacts/wpt-full-baseline/keyboard-tab-v2-2026-09-12.json`.
- Reviews: `benchmarks/conformance/wpt-keyboard-tab-v2-reviews.json`.
- Qualification: `artifacts/wpt-full-baseline/keyboard-tab-v2-qualification-2026-09-12.json`.
- Supplemental probe: `artifacts/wpt-full-baseline/inert-hit-testing-keyboard-v2-probe-2026-09-12.json`.

V2 worker SHA-256: `7587dd8cb0d9c9c61423efc7510c85280b9c6bb16ffe0f63897a902d127de4c1`.
Code Directory hash: `1da6f67a2f33e3788654206ae6cd4dd9ddedfed8`.
Parent SHA-256: `7c686eaf26d19085263404a6a62c00f1bbbb947386b2608c620cfb851bb5ec14`.
Parent bytes are unchanged: these runtime fixes reside in the worker.
Raw metadata retains V8, bindings, ICU, snapshot, signature and source identities;
the qualification artifact binds raw, reviews and classifier hashes.

## Verification and remaining work

- Native executable regressions: 154 passed, six existing ignores.
- V8 binding regressions: 211 passed, three existing ignores.
- Native input scheduler: 10 passed; Chromium driver: 10 passed.
- WPT Python harness suite, including live fixtures: 125 passed, no skips,
  110.361 seconds. Qualification-specific tests: 14 passed.
- Formatting and targeted clippy with warnings denied passed.
- Signed protocol-v9 handshake and filesystem/listener/outbound/environment
  sandbox probes passed for the corrected candidate.

Still unfinished: cross-frame focus transitions, delegatesFocus/manual slot
assignment, broader focusability and page-boundary semantics, radio arrow-key
activation (the mixed-focusable radio test now passes its Tab subtest but still
rejects the arrow-key action), editing/selection inertness, full form validity,
attribute insertion-order serialization and the deferred Range.insertNode crash.
No full-workspace, Linux, full-Core or clean-source release acceptance is claimed.
