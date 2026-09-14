# Qualified Core audit: DOM, Request and XHR

A fresh signed nine-execution slice reproduced four passing controls and five
failing executions. All nine completed normally with harness status zero and the
source-expected terminal subtest count. Parent and worker hashes match the prior
Core composite. No runtime code changed and no timeout budgets were increased.

## Confirmed fix queue

| Order | Group / type | Qualified evidence | Repair scope |
| --- | --- | --- | --- |
| 1 | Fetch Request constructor validation | CONNECT, TRACE and TRACK, upper/lower case, do not throw TypeError: 6 failed subtests in each of window and dedicated worker | Reject forbidden methods during Request construction in both globals. Test allowed methods and Request cloning as controls; separately audit token syntax and normalization. This evidence does not prove forbidden methods reach the network. |
| 2 | FormData argument conversion | `new FormData(null)` does not throw TypeError in either global | Distinguish omitted/undefined from invalid supplied arguments and validate the form interface. Test null, strings, unrelated nodes/objects, omitted arguments and valid forms. The string assertion in this WPT is not reached after null fails; do not count it as observed. |
| 3 | DOM document-kind validation | HTML `document.createCDATASection("foo")` does not throw NotSupportedError | Add the HTML-document guard while preserving XML CDATA creation; separately cover XML invalid content. |

Read-only implementation inspection agrees with the observed symptoms:

- `crates/oryn-js/src/v8_bindings.rs` Request construction checks method characters
  but has no forbidden-method rejection in that validation block.
- FormData construction skips null/undefined and otherwise conditionally accepts
  a node, allowing invalid arguments to fall through to empty FormData creation.
- The createCDATASection dispatch unconditionally creates a CDATA node.

These are confirmed narrow compatibility gaps, not estimates of how many other
WPT failures a fix would resolve. Runtime repairs are not part of this audit.

## Passing controls and qualification limits

AbortSignal.abort passes its two subtests in window and dedicated worker,
including the two-second no-abort-event observation. XHR abort after an
unsupported-protocol send passes in both globals. The latter needs no reachable
network server; it does not qualify ordinary network-error event ordering.

All five source files were read completely. They require no privileged testdriver
operations, dynamic imports, remote resource fixtures or unusual origin topology.
Only standard harness scripts and upstream generated wrappers are used. Both HTTP
and HTTPS script preflights succeeded. The runner now records the implicit harness
script dependencies of `.any.js` and `.window.js` wrappers, previously absent from
literal dependency discovery.

The qualified overlay now contains **10 executions: 5 passed and 5 failed**,
including the previous FileReader seed. This deliberately selected small slice is
not representative; a qualified pass percentage would be misleading.

All 13,747 Core IDs remain present: 10 qualified, 10,027 review candidates, 3,693
unresolved and 17 environment blocked. Raw totals are unchanged: 3,861 passed,
9,869 failed and 17 blocked. Historical artifacts and reviews remain immutable.

## Evidence and reproduction

- Inventory: `benchmarks/conformance/wpt-audit-slice-v2.json`
- Fresh raw run: `artifacts/wpt-full-baseline/audit-slice-v2.json`
- Source/evidence-bound reviews: `benchmarks/conformance/wpt-qualification-reviews-v2.json`
- Refreshed composite: `artifacts/wpt-full-baseline/audit-composite-v2/`
- Qualified ledger and summary: `artifacts/wpt-full-baseline/qualification-v2/`

`scripts/refresh-wpt-composite.py` checks exact inventory IDs, completed run
accounting and matching parent/worker identities before replacing a slice. It
retains superseded origins and unchanged IDs. Qualification then validates all
current source hashes, review bindings and dependencies.

```sh
python3 scripts/refresh-wpt-composite.py \
  --composite artifacts/wpt-full-baseline/fixture-repair-v5-composite \
  --replacement artifacts/wpt-full-baseline/audit-slice-v2.json \
  --inventory benchmarks/conformance/wpt-audit-slice-v2.json \
  --output artifacts/wpt-full-baseline/audit-composite-v2

python3 scripts/qualify-wpt-evidence.py \
  --composite artifacts/wpt-full-baseline/audit-composite-v2 \
  --checkout /Users/rohit/Work/oryn2/oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0 \
  --reviews benchmarks/conformance/wpt-qualification-reviews-v2.json \
  --output artifacts/wpt-full-baseline/qualification-v2
```

Choose new output paths when reproducing; existing outputs are rejected by these
overlay tools. Evidence remains diagnostic and dirty-tree, not a release attestation.

Validation: 63 WPT-related tests ran; 61 passed and two Node-dependent tests were
skipped. Live fixture topology checks passed, as did `git diff --check`. The signed
slice had four passes and five expected conformance failures; none was a timeout,
crash or incomplete harness result. No Rust changes were made, so Rust tests were
not rerun.
