# Selected WPT inventory execution

Use `scripts/wpt_selected_bridge.py` to prepare and reconcile the retained
4,022 execution IDs. It does not run tests or qualify a candidate. Execution
uses canonical `oryn-validate.py wpt-dev` and authenticated native WebDriver,
not the historical JavaScript testdriver adapter.

Prepare using the pinned inventory, its source export and WPT manifest, and
the original ledger:

```text
python3 scripts/wpt_selected_bridge.py --inventory benchmarks/conformance/runtime-api-style-semantics-v2-inventory.json --manifest PINNED_MANIFEST --checkout PINNED_EXPORT --ledger ORIGINAL_LEDGER --output FRESH_MAPPING_DIRECTORY
```

Run `selected.include` through an existing admitted run manifest and signed
bundle. Preserve production limits and all terminal evidence:

```text
python3 scripts/oryn-validate.py wpt-dev EXISTING_RUN_MANIFEST --store-root OWNED_STORE --bundle-root SIGNED_BUNDLE --output FRESH_RUN --include-file FRESH_MAPPING_DIRECTORY/selected.include --workers WORKERS --timeout-seconds TIMEOUT
```

Repeat preparation with `--report ACTUAL_WPTREPORT` and a fresh output directory
to reconcile results. Validate the report's candidate, run contract, fixture,
revision and executor metadata against admitted inputs independently. URL
presence is not assertion completeness; zero-assertion results cannot pass.
The mapper always reports qualification as `not_evaluated`.

The pinned population maps to 4,020 testharness URLs and two other required IDs.
All shared/service-worker wrappers are retained. `WebIDL/` aliases `webidl/`
only after source hash and Git-blob checks.

## Required non-testharness entries

- `IndexedDB/resources/idb-partitioned-coverage-iframe.html|window|` is a support
  resource, not a standalone testharness test. It requires an explicit reviewed
  scope disposition, not an invented passing observation.
- `dom/svg-insert-crash.html|window|` is a crashtest. Assess it separately using
  a supported canonical executor, preserving its original ID and raw result.

Both remain visible in required dispositions and missing-ID reporting. Running
all 4,020 testharness URLs cannot establish 4,022/4,022 completion. Preserve the
original inventory and ledger; do not silently remove these obligations.
