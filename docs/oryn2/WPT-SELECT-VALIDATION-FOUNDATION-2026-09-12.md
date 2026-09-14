# Select constraint-validation foundation

Native select validation eligibility is now exposed as willValidate, with live
disabled/fieldset-first-legend/datalist behavior and no readonly exclusion.
Native required-value logic distinguishes a selected placeholder label option
from another empty-valued option. Disabled selected options are not discarded
from the validity calculation. Display size has a shared native algorithm,
including the multiple-select default of four, separate from IDL reflection.

Select now exposes owner-traced live ValidityState, native custom-error state,
validationMessage and checkValidity. The latter snapshots invalidity, dispatches
a trusted cancelable non-bubbling/non-composed invalid event when eligible, and
returns false even when canceled. Disabled controls retain validity state but
return an empty validationMessage and pass checkValidity without an event.
Custom messages preserve DOMString code units and are not copied by cloneNode.
Regression coverage includes incompatible/proxy/inherited receivers, live
state, cancellation, conversion exceptions and clone behavior.

This is not a complete constraint-validation API. reportValidity, form
aggregation, other control types and validation pseudo-classes remain
unfinished. Namespace, cross-realm and message/resource accounting also require
broader coverage. A dedicated GC test retains only the validity object and
proves its detached owner's native required/custom-error state remains live;
128 unreachable cycles collect, and deleting the held validity object restores
the wrapper registry to its initial count. Relevant algorithm:
https://html.spec.whatwg.org/multipage/form-elements.html#placeholder-label-option

Current source passes 230 JS binding tests (three existing ignores), 82 DOM
tests, 158 native executable tests (six existing ignores), targeted DOM/JS/native
clippy, formatting and generated-IDL checks.
These changes and the replacement-record follow-up are not present in the
completed options-batch-v1 signed WPT runs. Do not attribute those results to
this newer source.

## Signed candidate, completed diagnostic

The select-validation-v1 build completed and was signed September 12 at 23:51
local time. Signature/designated-requirement verification, protocol-v9 handshake,
sandbox denial probes and the independent inert-hit-testing probe passed.

- Parent SHA-256: `d3a15289c095c5cca829f6abde44d5c8146a83c99223e6936e7b6a4aa757c147`
- Worker SHA-256: `8bdee82c94c47cc62e08d65dc4c6b07f5e8cbbe197131681bc824e58547c8e0f`
- Code Directory hash: `cb87ed7682e225e79b748cedb73901d2555f0b26`
- IDL manifest: `432b5d30a2227a3d00a714808d572b85f0dd1ff69dcf17b58e446985db18991d`

The unchanged broad (125), regression (66), and constraint-validation (28)
inventories completed under `select-validation-v1` artifact prefixes. They
used two jobs per cohort, persistent parents and isolated prewarmed workers;
Chromium reference focus was host-serialized. This is a signed dirty-tree
diagnostic, not clean-source attestation.

The broad cohort completed 125/125 in 528.529 seconds: 44 raw passes and 81
failures, with 259 passing, 189 failing, three timed-out and four not-run
subtests. Compared with options-batch-v1, three files newly pass and no prior
passing file regressed: select-setcustomvalidity.html, select-validity.html and
select-willvalidate-readonly-attribute.html. Fresh source/resource-bound reviews
qualify 31 files / 198 subtests; 67 files remain review candidates and 27
unresolved. All 31 reviewed files qualified. The earlier Chromium focus failure
for HTMLOptionsCollection is absent in this new run; its old evidence is unchanged.
See select-validation-v1-broad-2026-09-12.json and its qualification overlay in
artifacts/wpt-full-baseline, with reviews in
benchmarks/conformance/wpt-select-validation-v1-broad-reviews.json.
These cohort results are not a global Core conformance score.

The constraint cohort completed 28/28 in 114.434 seconds: zero complete file
passes, 19 passing and 901 failing subtests (920 total). The previous
option-select-v1 diagnostic had five passing and 915 failing subtests. These
are partial raw API gains, not qualified file conformance or a performance
comparison. The regression cohort completed 66/66 in 279.043 seconds: 43 raw
passes, 23 failures and no file-outcome changes against options-batch-v1.
Fresh source/resource/probe-bound reviews qualify 41 files / 336 subtests;
15 remain review candidates and ten unresolved. Overlay/raw/review/helper
hash bindings were verified. Neither result includes the newer validity-selector
source follow-up.
