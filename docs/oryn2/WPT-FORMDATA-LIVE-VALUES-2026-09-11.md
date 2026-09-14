# FormData live control values

FormData construction previously read only each control's `value` attribute.
It now uses `DomRuntime::control_value`, matching the native input/textarea
getters: a current value takes precedence over the attribute, an explicitly empty
value stays empty, and an untouched textarea uses its text content.

The prior regression test's property-based form construction is restored. New
checks cover dirty values versus changed default attributes, textarea defaults
and live multiline values, empty overrides, duplicate field ordering, and
FormData snapshot independence. The separately runnable signed-worker fixture is
`test-harness/formdata-live-values.html`; success sets the document title to
`FORMDATA_LIVE_VALUES_PASS`. The pre-fix signed worker retained the initial title
instead, confirming the fixture did not complete successfully.

## Scope and remaining gaps

This is a focused live-value repair, not full form entry-list conformance. The
15-execution WPT inventory in
`benchmarks/conformance/wpt-formdata-live-values-v1.json` retains the prior nine
controls and adds six form-element cases. Its signed pre-fix baseline passed
12/15 executions. The remaining failures were:

- `constructor-formelement.html`: input.name loses an unpaired UTF-16 surrogate
  before the test even reaches FormData construction. This masks the later
  constructor assertions; do not claim them covered.
- `append-formelement.html` and `set-formelement.html`: supplying a filename with
  a string or URLSearchParams does not throw TypeError as expected.

Additional constructor behaviors such as full selected-option enumeration,
fieldset/datalist exclusions, empty file entries and submitter handling are not
qualified by this change. Historical full-Core and qualification ledgers remain
unchanged because the worker binary changed.

## Verification

- Signed local regression now prints `FORMDATA_LIVE_VALUES_PASS`.
- Identical before/after WPT slice: **12/15 executions and 46/51 subtests pass**
  after the fix. All three failures have unchanged symptoms. No execution changed
  status; there are no claimed WPT gains from this focused repair.
- All 196 oryn-js library tests passed in individual processes, avoiding the
  previously observed shared-process V8 initialization collision. The updated
  targeted test also passed through Cargo directly.
- Clippy for oryn-js, all targets, with `v8-host,v8-sandbox` and warnings denied
  passed. Formatting and diff checks passed.
- The release worker was signed with the existing Apple Development identity.
  Signature/designated-requirement validation, protocol-v9 smoke checks, and the
  filesystem/network/listener/secret-environment denial probes passed.

Evidence:

- `artifacts/wpt-full-baseline/formdata-live-values-before-v1.json`
- `artifacts/wpt-full-baseline/formdata-live-values-after-v1.json`
- After-run SHA-256: `25ab01a02fb893543bbf0476c796868c49fed1a5639f24e992f9f60019e26282`
- Worker SHA-256: `ca5b375b2803880ce84054bea24699782d3b50217456254a12c9d92cbd6eea4f`
- Worker Code Directory hash: `c3a725bb0e4d5811defb556fdad21f5755e1700b`

Reproduce the local signed check with:

```sh
target/release/oryn native --html test-harness/formdata-live-values.html --evaluate document.title
```

This output must equal the success marker; the CLI exit status alone is not the
assertion. The full workspace and full Core WPT corpus were not rerun. All evidence
remains dirty-tree diagnostic evidence, not clean release qualification.
