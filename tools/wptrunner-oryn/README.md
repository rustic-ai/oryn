# Oryn product for upstream wptrunner

This package registers `oryn` through the pinned runner's `wptrunner.products`
entry-point API. Target revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.
It imports the actual upstream testharness, reftest, crashtest and print-reftest
executors. It does not copy their readiness, comparison or result algorithms.

## Implementation state

**Source generated; not installed, built or tested. Not operational yet.**
The native command server is still required. Existing `--wpt-session` accepts
whole-test `run` and `shutdown`, so it cannot be used as this command server.
The plugin expects a separate driver executable accepting `--host`, `--port`
`--browser-binary` and `--auth-token-file`, with standard command-level session/window/navigation,
script, input, screenshot and print endpoints. These launch arguments specify
the integration contract to implement; they are not existing Oryn CLI options.

Package installation and tests are deferred until generation for the complete
Core-and-OIL plan is finished. The package relies on the pinned upstream runner's
dependency environment, not an independently resolved PyPI wptrunner version.

## Open integration obligations

- Implement the native command server and connect every operation to production
  native primitives with environment/generation authority and isolation.
- Configure pinned fixture host aliases and CA trust explicitly. The product
  requires native browser identity and does not accept certificate bypass.
- Bind driver/runtime/upstream identities and source manifests into run evidence.
- Add actual failing-test, malformed-result, readiness, lifecycle and teardown
  canaries, plus native end-to-end protocol tests for every used executor method.
- Finish native render/print and generated worker/wrapper capabilities consumed
  by the upstream executors. Registration does not establish implementation.
- Provide manual/visual assessment and real local/public address-space topology.
- Preserve the historical runner and raw evidence until parity is established.

No wdspec or manual/visual execution is claimed by this registration. All Core
manual and visual entries remain explicit assessment obligations. All four
registered executor routes remain unqualified until the above integration and
native capability work is complete and verified on the final candidate.

## Authentication generation

The browser supervisor creates a mode-0600 temporary credential file and supplies
its path to the driver. The executor receives the credential through upstream's
private executor-browser configuration and sends a bearer header through a custom
transport. Executor subclasses change only the protocol connection, retaining
upstream test execution algorithms. Credentials are absent from URLs and argv.
The native `webdriver/auth.rs` admission code requires the exact loopback Host,
no Origin header, and the bearer credential. HTTP decoding, duplicate-header
checks, token-file loading and endpoint integration still require implementation.
These sources and their tests have not been run.

## Full-manifest qualification source

`../../scripts/reconcile-full-wpt-qualification.py` consumes the pinned full
inventory, actual manifest/policy bytes, official Core-only `wptreport` files,
independent source-bound reviews and optional manual assessments. Output requires
a new directory; reports and historical evidence are never modified. Report
`run_info` must identify `product: oryn`, `oryn_executor: upstream-wptrunner`,
`oryn_wpt_revision` and `oryn_candidate_sha256`. Producing that candidate-bound
metadata is still an integration requirement. Do not hand-label mixed-candidate
reports. A complete synthetic unit-test inventory is not a full Core baseline.

The `canaries` directory is a separate qualification corpus, mounted only for
executor controls. Preserve the pinned upstream manifest and denominator.
`fixture-contract.example.json` is an unresolved infrastructure template;
`../../scripts/wpt_fixture_contract.py` validates explicit topology and probe
records. Native CA trust and port-aware socket endpoint routing, actual owned
public/private VM endpoints and live probes remain required. No infrastructure
was deployed and no tests were run during this generation.

See `../../docs/oryn2/G-TOOLING-GENERATION-NOTES-2026-09-15.md` for exact coverage,
interfaces and remaining native/qualification obligations.

## Integrated workflow sources

The plugin now implements the pinned `Product.run_info_extras` hook and verifies
candidate/runtime/driver/plugin/fixture identities before reporting them.
`--oryn-run-contract` supplies the immutable candidate record. The driver receives
`--configuration` and `--configuration-sha256`; native consumption remains a
required integration, with normal CA validation and real socket destinations.
Upstream's positive hostname set and ports must exactly match the candidate;
intentionally nonexistent hostnames remain unmapped.

`../../scripts/run-official-wpt.py` provides `prepare`, `canaries` and `core`
operations using actual upstream runner and manifest APIs. Every operation requires
a full 51-requirement source-generation gate. Preparation creates a separate canary
manifest/mount and locks inputs; execution writes new raw reports and logs without
retries or window reuse. `../../scripts/qualify-wpt-canaries.py` records the five
expected negative outcomes with independent resource and cleanup observations.
No generation gate or candidate was fabricated, and none of these workflows has
been executed. Read the G tooling notes for contract schemas and remaining gates.

### Native feature profiles

The fixture contract must select one immutable profile for the entire run:

```json
"web_features": {"usermedia_element": "standard"}
```

Use `"legacy"` for the separately declared `UserMediaElementLegacy` configuration.
The driver applies this selection before creating the context; each Window
freezes it before installing bindings. A changed profile requires a newly bound
fixture and candidate. Paths, URLs and individual test results never select a
profile. Normal browser contexts default to standard behavior.

Keep the complete Core include file for each profile. After both full runs,
`compare-wpt-feature-profiles.py --standard <execution.json> --legacy
<execution.json>` verifies their original custody and produces a configuration
comparison. It does not merge passes or claim a single-profile conformance
result. Both raw reports and the explicit virtual-profile attribution remain
part of the evidence.
