# Select validation namespace follow-up

HTML control algorithms now read null-namespace attributes explicitly for select
required, multiple and size, plus control/ancestor-fieldset disabled eligibility.
Namespaced attributes with these same qualified names no longer activate HTML
validation or selection behavior. Existing mutation hooks already exclude
namespaced attributes; regression coverage also checks that setting, changing
through Attr.value, and removing namespaced size/multiple preserve an explicitly
empty selection.

Validation: 231 JS binding tests passed (three existing ignores), 82 DOM tests
passed, and 159 native executable tests passed (six existing ignores). Targeted
DOM/JS/native clippy passed with warnings denied. The expanded namespace test
also passed after adding the empty-selection assertions. Formatting, generated
IDL consistency and diff whitespace checks passed.

This source and the validity-selector integration are newer than the signed
select-validation-v1 candidate. Its completed WPT results do not verify these
changes. General storage of attributes sharing a qualified name across different
namespaces, option selected/disabled namespace behavior, other control validation,
and complete form aggregation remain separate open work. No full Core completion
or clean-source attestation is claimed.

## Signed worker verification

The follow-up build was signed September 13 at 00:12 local time. Signature,
designated requirement, exact entitlements, protocol-v9 handshake and sandbox
denial probes passed. The selector/computed-style regression, namespace regression
and independent inert-hit-testing regression each returned `ok` through the
signed worker. These are focused regressions, not a new upstream WPT run.

- Parent: `53cb2bf86188722477d74a3232a3d71cd0de01868d39ec45ac32dfb651f2ffb9`
- Worker: `e7b5b9d98f01f6bba736c87836c65e71d6166638ecd984effabd4d9f7802e6fb`
- Code Directory: `8521e6c573f38e03533f12dff20baef4b4b4001d`

Recorded results and source/fixture hashes:
`artifacts/wpt-full-baseline/select-validity-namespaces-worker-probes-2026-09-13.json`.
The probe used the exact JS bodies of the named Rust regression tests, with
the selector fixture preserved as `test-harness/select-validity-worker-probe.html`.
The source tree remains dirty; this does not replace clean-source attestation.
