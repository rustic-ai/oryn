# Radio arrow-key activation checkpoint

Core conformance remains incomplete under the unchanged
[full completion gates](WPT-CORE-COMPLETION-GATES.md). This repair implements a
browser mechanism; its 64-test cohort does not replace the full Core inventory.

## Implemented and tested

Native uncanceled arrow keydown on a focused radio now traverses eligible peers
in tree order within the same native radio group, including external form
association and negative tabindex. It skips disabled, inert and hidden peers,
wraps at group boundaries, and respects computed RTL for horizontal arrows.
The selected target receives focus followed by native trusted click activation,
using existing checkedness preactivation, cancellation restoration and input/
change dispatch. Focus-listener mutations are rechecked before activation.

The implementation is consistent with the focus-then-click behavior in
[Chromium's radio input implementation](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/html/forms/radio_input_type.cc).
It does not claim every platform input behavior or all modifier metadata is complete.

The native Actions scheduler and send-keys decoder now accept WebDriver's four
arrow-key values. Synthetic page-dispatched events and Ctrl/Alt/Meta shortcuts
do not trigger this native navigation default.

Four new regressions cover Actions delivery, external-form/tree ordering,
negative tabindex, wrapping, skipped controls, canceled keydown/click, trusted
activation events, redirected/disabled focus targets and RTL direction.

## Signed evidence

The unchanged `benchmarks/conformance/wpt-label-associations-v1.json` inventory
ran with two persistent parents and isolated prewarmed workers.

| Candidate | Raw passes | Qualified executions / subtests | Elapsed |
| --- | ---: | ---: | ---: |
| keyboard-tab-v2 | 39/64 | 36 / 311 | 266.140 s |
| radio-arrows-v1 | 40/64 | 36 / 307 | 278.826 s |

No previous raw passes were lost. The mixed-focusable radio navigation test
now passes both subtests, matches Chromium and qualifies. Its eighteen native
action sequences are source-derived: ten in the first subtest and eight in the
second. Both focus and checkedness assertions pass. Overall observations are
395 passing subtests, 70 failing and one timeout; these are not a full-Core score.

Qualification remains per execution. Four raw passes are excluded:

- `focus-tabindex-order`: native operations all completed, but Chromium's ninth
  send-keys operation has only a started record at final collection. Chromium's
  assertions pass, but the automation-completion gate correctly rejects this
  unfinished trace. This is evidence of a reference terminal-drain race, not
  proof of a native focus regression.
- `checkbox.html`: current Chromium reference focus/completion prerequisites
  failed. Historical passing reference evidence is not carried over.
- `radio-double-activate-pseudo`: its final inactive assertion still does not
  independently prove keyboard activation.
- `clicking-noninteractive-unlabelable-content`: attribute serialization changes
  generated subtest identities versus Chromium.

All review source hashes and actual resource bytes were independently rechecked.
The independent signed inert-hit-testing probe returned `ok` on this worker and
is bound to the new raw result. No old raw, review or qualification was rewritten.

Artifacts:

- `artifacts/wpt-full-baseline/radio-arrows-v1-2026-09-12.json`
- `benchmarks/conformance/wpt-radio-arrows-v1-reviews.json`
- `artifacts/wpt-full-baseline/radio-arrows-v1-qualification-2026-09-12.json`
- `artifacts/wpt-full-baseline/inert-hit-testing-radio-arrows-v1-probe-2026-09-12.json`

Worker SHA-256: `242c682142799b61590c076242ee8638bc2d8ccf4a3b026f5519705eebf33d0c`.
Code Directory hash: `de8c74b61da81874373dd652f3eacd3b4e404155`.
Parent SHA-256: `7c686eaf26d19085263404a6a62c00f1bbbb947386b2608c620cfb851bb5ec14`.
All evidence is signed dirty-tree diagnostics, not clean-source release acceptance.

## Verification and next dependencies

- Native executable tests: 158 passed, six existing ignores.
- V8 binding tests: 211 passed, three existing ignores.
- Input scheduler and Chromium driver tests: 20 passed.
- Targeted clippy with warnings denied, formatting and diff checks passed.
- Signing, protocol-v9 and filesystem/listener/outbound/environment sandbox
  probes passed. No full-workspace/Linux/full-Core run is claimed.

Next harness work is to await real in-flight reference automation termination
before final evidence collection, with bounded deadlines and fail-closed context
ownership. Do not invent a completed record or waive the gate. Intermittent
reference focus remains unresolved separately.

Next browser dependency is live `HTMLFormElement.elements` and its collection
semantics. `radio-keyboard-navigation-order.html` still fails before its arrows
because that API is absent. Cross-frame focus, broader keyboard activation,
form validity, inert editing/selection and the deferred Range.insertNode crash
remain unfinished.
