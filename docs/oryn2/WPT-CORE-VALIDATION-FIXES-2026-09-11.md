# Core constructor and document validation fixes

Implemented the three defects established by the qualified nine-case audit:

- Request rejects CONNECT, TRACE and TRACK, including mixed-case input, with
  TypeError. This does not change unrelated method normalization behavior.
- FormData distinguishes omitted/undefined arguments from invalid supplied values,
  and requires an actual HTML-namespace form element when supplied. Null, strings,
  unrelated objects/nodes and foreign-namespace form elements are rejected.
- createCDATASection rejects HTML documents with NotSupportedError, preserves XML
  creation, and rejects the XML CDATA terminator `]]>` with InvalidCharacterError.

All changes are in `crates/oryn-js/src/v8_bindings.rs`. The Rust regression test
`qualified_wpt_constructor_and_cdata_validation` covers exception brands, valid
Request methods and copying/cloning, valid and omitted FormData arguments, and
HTML versus XML document behavior. Shared bindings are exercised by WPT in both
window and dedicated-worker globals where applicable.

The old `audit-slice-v2.json`, qualification manifests and full Core composite are
historical evidence for the old binary. A changed binary must not be merged with
that cohort using the same-binary fixture-repair mechanism. Any new slice results
are separate diagnostic evidence, not an updated full-Core conformance score.

## Verified results

The new signed run is
`artifacts/wpt-full-baseline/core-validation-fixes-v1.json`: **9/9 executions and
21/21 subtests passed**, versus 4/9 executions before the fixes. Five failed
executions became passes; all four passing controls remained passing. There were
no crashes, timeouts, incomplete results or environment blocks in this slice.

- Raw evidence SHA-256: `faa4f78b3b5ad52bb90db93687f12dd92152950a2806523389455c9528ad1d32`
- Worker SHA-256: `17b93cf6d58172799490455943f63917e901d1bed46e6510dc8a863bdc167319`
- Worker Code Directory hash: `57c38bd3384bd122d8a44ed783c300e225752960`
- Parent SHA-256 (unchanged): `df87fc9dc172158fe0f7922e031cf8aa85899be030d0f55d6ab3038ff90a1a03`
- Run fingerprint: `7923e826fc61c88cf9290ed076cb6dcb2b2f6e7c5112c6589ed78cc35b06bae1`

The existing Apple Development identity signed the release worker using the
explicit hash-validated V8 profile. Designated-requirement validation, protocol-v9
smoke checks and sandbox probes passed. Filesystem reads, direct outgoing network,
listeners and secret environment access were denied. This is still dirty-tree
diagnostic evidence, not clean release qualification.

## Test details and limitations

- All **196 oryn-js library tests passed in individual processes**. The normal
  multi-test invocation aborted with V8 `address_pool_manager` double-initialization
  failure, so it is not reported as a successful `cargo test` invocation. Each
  listed test was then run with `--exact` in a fresh process, with a 45-second
  per-test timeout; none failed or timed out.
- `cargo clippy --locked --release -p oryn-js --features v8-host,v8-sandbox
  --all-targets -- -D warnings` passed using pinned V8 archive/bindings.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Python WPT suite: 59 passed, four skipped (two Node-dependent, two live fixture
  checks not enabled for that invocation). The signed WPT run separately performed
  its live helper and HTTP/2 preflights successfully.

The first new unit-test attempt exposed a separate form collection limitation:
assigning `input.name` and `input.value` did not produce the expected FormData
entry. The valid-form control now uses explicit attributes to test the existing
supported collection path. Property-based/live control serialization remains a
follow-up; it was not silently repaired or counted as covered here. The full
workspace and full Core WPT suite were not rerun.
