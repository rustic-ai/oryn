# Select add prerequisite repair

Core conformance remains incomplete. Native HTMLSelectElement.add now handles
option and optgroup arguments, numeric or element references, nested reference
parents, out-of-range append, same-element no-op, and hierarchy/not-found errors.
Argument conversion occurs before reading the live option list; conversion
exceptions propagate. Generated IDL and native optgroup branding are integrated.
Optgroup disabled/label reflection includes UTF-16-preserving label conversion.

The regression exercises the constraints helper's add(new Option(...)) setup
without modifying its source. It also tests group placement, movement, conversion
side effects, missing/invalid arguments, ancestor insertion and exceptions.
Additional checks cover WebIDL long conversion for selectedIndex and lossless
option/select value setters, fixing previously lossy paths.

The full V8 binding suite passes 218 tests with three existing ignores. Targeted
JS/native clippy with warnings denied and generated WebIDL checks pass. Native
executable tests passed 158 cases with six existing ignores on the final source.
The signed rebuild passed signature validation, protocol-v9 handshake and
filesystem/listener/outbound/secret sandbox probes. Direct signed probes returned
ok for inert hit-testing and Option default/current state, select.add integration
and explicit empty selection. No new WPT qualified pass is claimed yet.

Parent SHA-256: `1ae2aec8d91dcc70847b2b80b08acdde2938610c7c2426a8e18017ca13041929`.
Worker SHA-256: `7faea1318fb155ff6e6ef6e7a89bc87b2d63c7a9c321385ed1a1c0122e85cf93`.
Code Directory hash: `924eec1aebc8109f51900b66b78dae0ae6426a72`.
WebIDL SHA-256: `7a6a0d7e980766051ccd20a374bbca756eb9098c0490058694528589b5a8149b`.
This remains signed dirty-tree diagnostic evidence, not release attestation.

The new benchmarks/conformance/wpt-select-options-v1.json contains all 125
mapped Core testharness executions under the select, option and optgroup element
directories in inventory-v3. Every pinned source hash was checked; no outcome
filtering was used. This is additional diagnostic coverage, not a replacement
for the full Core ledger or an exemption for unsupported/manual entries.

The unchanged 66-test regression cohort completed in 274.578 seconds: 43 raw
passes, 23 failures, with exactly the same file outcomes and 417 passing,
69 failing and one timed-out subtest as the fieldset candidate. No regression
or new qualified pass is inferred from this comparison.

All 28 constraints tests completed in 227.363 seconds. The six previous
`Option is not defined` harness aborts are gone: every file now reports harness
status zero. Observed assertions grew from 885 to 920 (five passing, 915 failing),
while file passes remain zero. This is prerequisite repair, not validation API
conformance. Both runs report zero environment-blocked executions.

The broader 125-test selection completed in 515.710 seconds on the same frozen
candidate: 15 raw passing files and 110 failures, with 82 passing, 366 failing,
three timed-out and four not-run subtests. Environment-blocked executions: zero.
The Option constructor (11 assertions), option-selected (three) and select-add
(two) files pass raw; missing option text/label/index/form APIs account for a
separate actionable failure group. No new qualified score is claimed.
Runs used persistent parents, isolated prewarmed workers and host-serialized
Chromium reference focus. Raw outputs are
artifacts/wpt-full-baseline/option-select-v1-regression-2026-09-12.json,
option-select-v1-constraints-2026-09-12.json and option-select-v1-broad-2026-09-12.json.
Preserve every raw
failure and independently qualify current source/resource/automation/reference
evidence. Reaching more subtests after prerequisite repairs is not conformance
by itself.

Live HTMLOptionsCollection, length/indexed mutation, remove and further option
APIs remain incomplete. Option's cross-realm non-object newTarget.prototype
fallback also remains open. Constraint validity APIs are still missing. None
of these gaps is waived by implementing the helper's immediate setup calls.
