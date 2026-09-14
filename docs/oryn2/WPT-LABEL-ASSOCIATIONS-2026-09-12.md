# Label associations, focus and inert hit testing

Core conformance remains incomplete. The [completion gates](WPT-CORE-COMPLETION-GATES.md)
still cover all 13,747 mapped Core executions and 2,990 unsupported entries;
this checkpoint does not substitute its smaller cohort for that objective.

## Implemented

- `HTMLLabelElement.control`, `.htmlFor` and `.form`, with native receiver
  checking, explicit-ID/first-descendant association and empty-for rejection.
- Live, identity-preserving `.labels` NodeLists for input, button, select,
  textarea, output, meter and progress. Membership follows tree moves, including
  detached and shadow trees. Hidden inputs return null while retained lists
  become empty and recover their original identity when the type changes back.
- Form-owner access for the newly exposed built-in control properties; label
  form ownership is derived from its associated form-associated control, not
  the label's ancestry.
- Programmatic label-focus forwarding, including numeric/empty tabindex,
  hidden labels, inert labels and inert/disabled associated controls.
- HTML/SVG `tabIndex` reflection, Web IDL long conversion, HTML signed-decimal
  prefix parsing and element-dependent defaults, including first-summary logic.
- Inert-element exclusion from `elementFromPoint` and `elementsFromPoint`, so
  native element-click hit testing rejects inert targets as intercepted.

Association and live-list behavior follow the
[HTML label specification](https://html.spec.whatwg.org/multipage/forms.html#the-label-element).
Tabindex reflection follows the
[HTML tabindex rules](https://html.spec.whatwg.org/multipage/interaction.html#the-tabindex-attribute).
This does not claim complete focus navigation, all APIs on the newly branded
output/meter/progress interfaces, form-associated custom elements or MathML
tabIndex support.

## Signed WPT results

The inventory is `benchmarks/conformance/wpt-label-associations-v1.json`.
It contains the prior 57-execution forms/inert cohort plus all seven mapped
Core executions in the sequential-focus-navigation-and-the-tabindex-attribute
directory. The selection is not filtered by results.

| Candidate | Original 57 raw passes | Expanded 64 raw passes | Qualified executions / subtests | Elapsed |
| --- | ---: | ---: | ---: | ---: |
| Prior form-activation-v3 | 24 | Not run | 21 / 112 (57-test scope) | 237.629 s |
| label-associations-v1 | 28 | 31 | 28 / 297 | 493.647 s |
| label-associations-v2 | 29 | 32 | 29 / 298 | 494.651 s |

V2 has 466 observed subtests, 383 passing. All prior raw passes are preserved.
These observations include incomplete/failing tests and are not a full-Core
pass rate. Runs used two persistent parents, isolated prewarmed fresh page
workers, fresh fixtures and Chromium comparison. The added navigation tests
take their long-timeout path; the larger elapsed time is not a startup benchmark.

Newly passing executions relative to the prior 57-test cohort:

- `forward-focus-to-associated-element.html`: 6/6.
- `label-attributes.sub.html`: 20/20, including moving a control into an iframe.
- `labelable-elements.html`: 26/26.
- `inert-label-focus.html`: 3/3, with the independent qualification caveat below.
- `inert-inlines.html`: 1/1 after native inert hit-test filtering.

The added tabindex getter tests pass 2/2, 8/8 and 120/120. All four added
keyboard-navigation tests still fail; their keydown assertions observe keyCode
0 instead of 9 for Tab, and native Tab default navigation is not implemented.

## False positive found and repaired

The first signed run reported `inert-label-focus.html` passing. Its source wraps
both the click and its focus assertion in the same catch block, so an assertion
failure can be swallowed. An independent native regression reproduced exactly
that problem: the click completed while focus remained on the text input.
V1 deliberately leaves this execution unqualified.

After inert hit-test filtering, the upstream driver's preflight rejects the
inert click before vendor dispatch, matching Chromium's observed path. Zero
backend automation calls are therefore expected, not evidence of a successful
device click. The test explicitly allows interception.

The signed supplemental probe `test-harness/inert-hit-testing.html` verifies six
targets: inert anchor, button, div, span, label and an inert ancestor's child.
It requires exact `element click intercepted` rejections, exclusion from both
hit-test APIs, no click/mousemove delivery and preservation of prior focus.
It returned `ok` with exit code zero against the V2 signed worker. Its result,
fixture hash, parent/worker hashes and related raw hash are recorded in
`artifacts/wpt-full-baseline/inert-hit-testing-probe-2026-09-12.json`.
The two V2 inert reviews bind this independently checked probe.

V2 raw passes still excluded from qualification:

- `radio-double-activate-pseudo`: its final assertion does not prove keyboard
  activation occurred; source audit remains withheld.
- `clicking-noninteractive-unlabelable-content`: generated subtest names differ
  because of attribute serialization order; the identity gate is unchanged.
- `proxy-click-to-associated-element`: this execution's Chromium reference
  failed focus verification. Earlier successful evidence remains immutable.

The intermittent reference-focus issue is not resolved by these repairs.

## Evidence and identities

Each version has separate immutable raw, review and qualification artifacts:

- `artifacts/wpt-full-baseline/label-associations-v1-2026-09-12.json`
- `benchmarks/conformance/wpt-label-associations-v1-reviews.json`
- `artifacts/wpt-full-baseline/label-associations-v1-qualification-2026-09-12.json`
- `artifacts/wpt-full-baseline/label-associations-v2-2026-09-12.json`
- `benchmarks/conformance/wpt-label-associations-v2-reviews.json`
- `artifacts/wpt-full-baseline/label-associations-v2-qualification-2026-09-12.json`

Pinned source/helper hashes and actual response bytes were checked again for
the new reviews. The two label-attributes template substitutions were reproduced
independently using the observed localhost fixture port, then hashed. Dynamic
iframe/frameset resources were reviewed and bound rather than assumed present.

V2 raw SHA-256:
`c9ea17acafad42c7d6235a2c2a9099fa4181bce153c0a2dca4cc128d7bbdb952`.

V2 build:

- Parent SHA-256: `7c686eaf26d19085263404a6a62c00f1bbbb947386b2608c620cfb851bb5ec14`
- Worker SHA-256: `4b6f0917572c3227648998bcba638b99c35c9a0e1427268b69eefaf1d689bbc5`
- CDHash: `9f02f6c4cbe4bbf6614c3e82505392733a7e17af`
- WebIDL SHA-256: `638b469170aa712c80675bc344005e02db593e34831cdbd50811d22f76f1a47f`

The signed build passed designated-requirement verification, protocol-v9
handshake and the build script's filesystem/listener/outbound/environment
sandbox probes. The artifacts are signed **dirty-tree diagnostics**, not
clean-source release/security attestation.

## Validation and next dependencies

- DOM: 65 passed.
- V8 bindings: 211 passed, three existing ignores, including live-list GC,
  unreachable list cycles, type changes, native brands and numeric conversion.
- Native executable group: 150 passed, six existing ignores.
- Targeted clippy with warnings denied passed; formatting and WebIDL generation
  checks passed. No whole-workspace/Linux/full-Core run is claimed here.

Next work should implement keyboard event metadata and native sequential focus
navigation together: default actions after uncanceled keydown, positive/zero/
negative ordering, reverse traversal, disabled/inert/hidden eligibility,
radio groups, shadow scopes and frame boundaries. Merely correcting keyCode
will not repair Tab behavior.

Also unfinished: general focusability (including editable-label cases), pointer
focus forwarding, dispatch-time activation ownership, full form validation and
successful-control/reset rules, attribute insertion-order serialization,
form-associated custom elements and the deferred Range.insertNode crash.
