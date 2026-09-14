# Reference automation terminal-drain repair

Core conformance remains incomplete under the unchanged
[completion gates](WPT-CORE-COMPLETION-GATES.md).

## Failure and repair

The radio-arrows-v1 raw artifact records `focus-tabindex-order.html` passing
native and Chromium assertions, but Chromium's ninth send-keys operation has
only a started record. Its qualification correctly remains rejected. A WPT
completion callback can execute inside the final key event, before the host
input command and its page reply have settled.

The reference collector now waits for the actual automation queue, rereads
the live report, and repeats if another operation was queued during the drain.
It retains the original run deadline. It binds the final read to the execution
context observed at report-ready and rejects context retirement, a missing
report, evaluation errors or an overrun. No terminal event is fabricated and
no qualification check is waived. Existing incomplete/failed automation traces
remain failures even when assertions pass.

The helper lives in the already hash-bound Chromium driver module; the canary
imports it. This avoids adding an untracked harness dependency to fingerprints.
Worker code and signatures were not changed by this repair.

## Verification

- Driver tests: 12 passed. New tests cover waiting for outstanding work,
  requests added during the final read, rereading the updated report, retired
  contexts, vanished completion and a non-settling operation deadline.
- Existing WPT Python suite with live fixtures: 125 passed, no skips,
  110.883 seconds.
- New live regression, run separately: one test passed in 18.367 seconds.
  It runs the pinned `focus-tabindex-order.html` three times and requires all
  nine operations to have started/completed records in both host and final
  page traces. All three samples passed with passing assertions.
- Diff whitespace checks passed.

Relevant files:

- `scripts/wpt-chromium-canary.mjs`
- `scripts/wpt-chromium-driver.mjs`
- `scripts/tests/test_wpt_chromium_driver.mjs`
- `scripts/tests/test_wpt_direct_fixture.py`

Tested canary SHA-256:
`eb09ca74d25e59d5c9c7e2bcf9cbae58df3b50de1084086ff1d2538f7f31d165`.
Tested driver SHA-256:
`66b61412a8060e37854b95db2d0f2a6f2922b1eb73e5968c059fac1b0874ebb9`.

## Evidence boundaries and remaining work

No historical raw, review or qualification artifact was rewritten. In
particular, the rejected radio-arrows-v1 execution is still rejected; these
live regressions do not retroactively complete its trace. No fresh full cohort
or whole-Core qualified pass rate is claimed for the changed harness.

Three successful samples establish this regression, not universal absence of
reference races. Intermittent native Chromium focus failures remain separate
and fail closed. Broader browsing-context coverage and full Core validation
remain required.

Next browser work: live HTMLFormElement.elements and form-controls collection
semantics, which currently prevent the broader radio tree-order WPT from
reaching its navigation assertions. Cross-frame focus, form validity, inert
editing/selection and the deferred Range.insertNode crash also remain open.
