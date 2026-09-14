# FormData overload validation and string-preservation diagnosis

## Implemented

FormData.append/set now require two arguments. A supplied third argument selects
the Blob/filename overload: non-Blob values are rejected with TypeError, including
when the third argument is explicitly undefined. Invalid calls do not change
existing entries. Ordinary two-argument null/undefined values still stringify.
Throwing value/filename conversions preserve the original JavaScript exception.

The common entry conversion helper checks the native Blob record, rather than
accepting arbitrary objects or coercing them into the filename overload. Valid
Blob/filename calls retain payload and media type. Existing Blob default-filename,
timestamp and File identity semantics are not changed or claimed fully conformant.

Regression coverage extends `qualified_wpt_constructor_and_cdata_validation`:
missing arguments, invalid primitive/plain/platform values, explicit undefined
filename, no mutation on rejection, string conversion, exception identity and
valid Blob filename/data. Changes are in `crates/oryn-js/src/v8_bindings.rs`.

## String preservation: diagnosed, not repaired in this change

The remaining constructor-formelement WPT fails before constructing FormData:
assigning an unpaired surrogate to input.name is read back as U+FFFD.
`set_attribute_value` converts the value using `to_rust_string_lossy`, losing the
original UTF-16 code unit. This is a DOMString boundary defect, not a fixture fault.

There is already a prefix-encoded UTF-16 preservation mechanism in selected
bindings (`rust_string_preserving_utf16` / `v8_string`). Merely switching the input
setter to this helper would be incomplete: native values and attributes use Rust
String, and control editing/range code computes lengths with `encode_utf16` on
that stored representation. Encoded surrogate sequences would therefore produce
incorrect offsets. FormData additionally needs USVString conversion, replacing
lone surrogates at entry creation rather than at the input setter.

The next string repair should therefore:

1. Establish a lossless native DOM string representation (or a consistently
   decoded adapter), including literal-prefix collision tests.
2. Preserve DOMString code units through setters/getters, attributes, cloning,
   input values, editing and UTF-16 offsets.
3. Apply USVString normalization explicitly at FormData names/values/filenames,
   not at generic DOM setters. Test that property readback preserves a lone
   surrogate while FormData contains U+FFFD, with paired-surrogate controls.
4. Rerun the constructor fixture and report the next assertion honestly. Its
   broad selection/disabled/file-entry semantics are not proven by fixing the
   first string assertion.

This is broader than the overload repair; no one-property string workaround was
added. Full Core evidence remains historical until rerun on the new worker.

## Verification

The signed 15-execution slice improved from **12/15 to 14/15 executions** and
from **46/51 to 50/51 subtests**. Append-formelement and set-formelement now pass;
all previous passing executions stayed passing. The only remaining failure is
the unchanged input.name surrogate assertion in constructor-formelement. All
15 executions completed normally, with no environment blocks or timeouts.

- Result: `artifacts/wpt-full-baseline/formdata-overloads-after-v1.json`
- Result SHA-256: `4d4c6780b6b295d5aeffc65df553f20ac3670f645e607fe5a6190d42af203eaa`
- Worker SHA-256: `a3e6915949ceb5f31b1fd7d950505c48632d1fb6678ee32f2a51afd9fea9f117`
- Worker Code Directory hash: `d55511f16c927f388ba75affb5cfb285d15c0e00`

All 196 oryn-js library tests passed in individual processes; the targeted
regression also passed through Cargo. Clippy with warnings denied, formatting,
and diff checks passed. Signing/designated-requirement validation, protocol-v9
smoke checks and filesystem/network/listener/secret-environment denial probes
passed. The full workspace and full Core suite were not rerun. This remains
dirty-tree diagnostic evidence, not clean release qualification.
