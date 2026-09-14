# Form controls and FormData UTF-16 boundaries

The input.name/value loss of unpaired UTF-16 surrogates is repaired at the
form-control binding boundary. Input and textarea name/value/defaultValue setters
now use lossless conversion rather than `to_rust_string_lossy`. This extends the
existing binding storage encoding; it is not a wholesale replacement of every
String-backed DOM field.

## Coordinated changes

- `oryn-dom::dom_string` shares the existing lossless UTF-16 storage adapter
  between bindings and native control algorithms. Valid Unicode remains ordinary
  text; lone surrogates and literal encoding-prefix strings are encoded, with
  collision-safe round trips through these conversion APIs.
- Native control range lengths and splices operate on decoded UTF-16 units,
  including a splice that splits a surrogate pair. Opaque-range updates use
  those same lengths.
- Input/textarea selection getters/setters, select(), setSelectionRange() and
  setRangeText() count decoded units rather than the internal encoding's length.
- Text-like input value assignment removes CR/LF without discarding surrogates.
  Textarea assignment normalizes CRLF/CR to LF. Selection endpoints use the
  normalized current value.
- FormData constructor names/values and append/set/get/getAll/has/delete names
  apply USVString normalization. Filename conversion does the same. Lone
  surrogates become U+FFFD here, while DOM property readback remains lossless.
  Multipart names/filenames decode the internal representation before encoding
  the wire body.

## Tests

The binding regression covers high/low lone surrogates, a valid surrogate pair,
literal encoding-prefix strings, reflected/cloned attributes, constructor and
method USVString behavior, filename conversion, newline normalization, and
selection/splice offsets. Native tests cover round trips and opaque-range updates.
`test-harness/formdata-utf16.html` repeats the main checks through the signed
worker and reports success only by setting `FORMDATA_UTF16_PASS` as its title.

## Scope limits

This qualifies the tested form-control and FormData paths, not all DOMString
behavior. Other String-backed DOM fields, parser ingress, generic text-node
algorithms, selector/serialization consumers and contenteditable editing have not
all been migrated or qualified for encoded lone-surrogate storage. The helper's
literal-prefix protection applies to values entering via its encoder; raw native
String callers must not treat arbitrary author text as already encoded storage.
The original constructor-formelement WPT also contains broader successful-control
and select-option behavior, so clearing its string assertions need not make the
whole test pass. Full Core historical ledgers remain unchanged.

## Verified outcome

Both signed local fixtures passed: `FORMDATA_UTF16_PASS` and
`FORMDATA_LIVE_VALUES_PASS`. The unchanged 15-execution WPT slice remains
**14/15 executions, 50/51 subtests passed**, without regressions or incomplete
execution. The remaining constructor-formelement test now passes its initial
surrogate and newline assertions, reaching the later failure:
`select-2` expected `submit-me-1`, received an empty string. The string failure is
resolved; select-option collection is the next confirmed constructor gap. Later
assertions in that same subtest remain unexecuted and unqualified.

- Result: `artifacts/wpt-full-baseline/formdata-utf16-after-v1.json`
- Result SHA-256: `c56055f59791eccbe31f527c5db895d422a5d4bc77f011d5dffe34a4e4fbd299`
- Worker SHA-256: `cadd7d323a611488ba174e519077b5557cbb344132340e371e668f4618c6de63`
- Worker Code Directory hash: `c0493f24a0d8b48c70bfd9e13919d0d13b70ada3`

Validation: 60 DOM tests passed; all 196 binding tests passed individually, and
the expanded targeted binding test passed through Cargo. Clippy passed for
oryn-dom and oryn-js with warnings denied; formatting and diff checks passed.
The signed build passed designated-requirement validation, protocol-v9 smoke and
filesystem/network/listener/secret-environment denial probes. Full workspace and
full Core WPT runs were not performed. Evidence is dirty-tree diagnostic evidence,
not clean release qualification.
