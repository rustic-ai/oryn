# Coverage expansion: second classification

This is implementation progress, not a corrected compatibility score or release
attestation. The original `results-v2` remains immutable and has known harness
contamination. Do not aggregate its outcomes with repaired slices.

## Inventory

`wpt-capability-tiers-v2.json` separates Core/Necessary/Extended priority from
environment hints. Runner support remains an independent dimension. Iframe
lifecycle, FileAPI, streams, WebSockets and security dependencies are Core;
WebAuthn and payments are Necessary even when their fixtures need emulation.
Unmatched Extended paths remain explicitly marked for review, not silently
declared low priority. Environment hints are family-level, not per-test proof.

The pinned official manifest is unchanged: 128,554 entries, 35,010 supported
execution variants and 93,544 entries awaiting other runners. Generated inventory
is `artifacts/wpt-full-baseline/inventory-v3` (policy version 2).

| Priority | Runnable executions | Awaiting runner support |
| --- | ---: | ---: |
| Core | 13,747 | 2,990 |
| Necessary | 15,192 | 34,330 |
| Extended | 6,071 | 56,224 |

`scripts/reclassify-wpt-evidence.py` projects the historical evidence by exact
runner ID, rejects duplicate/unmatched result IDs, and records source hashes.
`artifacts/wpt-full-baseline/reclassification-v2.json` accounts for all 35,010
old executions without rerunning them. These are not new performance or
compatibility observations. `run-full-wpt-baseline.py --tier core` now selects
Core shards and reports both selected and full-manifest denominators.

## Origin repair

- Preserve upstream Location headers, including intentional HTTPS-to-HTTP
  downgrades and distinct subdomains.
- Do not apply a second template substitution to upstream responses. This also
  avoids corrupting literal template-like text and non-UTF-8 bodies.
- Resolve the reserved localhost namespace to loopback while preserving the
  request URL/Host. Default policy still rejects loopback; IP checks still apply.
- Use `alt.localhost` for the alternate server namespace instead of generating
  unresolvable names such as `www1.127.0.0.1`.

Validation: all 154 no-default-feature native library tests pass with local-listener permission;
37 Python runner tests run (35 pass, two Node-dependent tests skip); five
inventory tests and two projection tests pass; live upstream HTTP/TLS/WS/WSS
topology test passes. Targeted no-default-feature clippy with warnings denied,
Rust formatting and diff checks pass.

The signed build passed designated-requirement verification, protocol-v9
ping/exit and sandbox denial probes. The rebuilt signed worker SHA-256 is
`9f48f866a1a33c68d2a7313d6abb686e1ca3909c7cc1ea657e3e53432f8cefda`;
CDHash is `d9d11d91f7f99af7f1912c3d89ecc9071adfdec4`.
The first 20 referrer-policy executions pass 20/20 in
`artifacts/wpt-full-baseline/origin-v2-referrer-canary.json`. This bounded sample
does not qualify the complete secure/cross-origin matrix or all Core features.

The build script now obtains pre-sign metadata from the build output rather
than launching an overwritten, temporarily invalid signed bundle. The initial
attempt was killed at that step; the corrected build and signing completed.

## Remaining qualification gates

### Follow-up harness qualification

The supported `.any.html`, `.window.html`, `.any.worker.html` and worker script
wrappers now come from the pinned upstream `RoutesBuilder`. Collector injection
occurs after testharness loads. Worker dependencies are no longer concatenated
at a synthetic URL on the full-inventory execution path; upstream preserves
their import order, request semantics, source paths and query variants. Legacy
shared/service-worker adapters are still outside the full inventory's supported
mapping and are not newly qualified by this change.

Dispatcher GET/POST and remote-context executor responses now delegate to
upstream, retaining its CORS headers, stash semantics, and secure base URLs.
Live tests cover generated wrappers, query propagation, dispatcher delivery
across Host values and its Origin response header, ten shared helpers and WS/WSS.
Explicit `not implemented by testdriver-vendor.js` errors are annotated as runner
limitations and counted separately without suppressing or converting failures.
This is a conservative detection rule, not an exhaustive diagnosis of every
automation-dependent failure.

The full referrer-policy qualification slice completed 1,391 executions:
1,186 pass, 199 assertion failures, five harness failures and one runtime failure.
It remains diagnostic evidence, not proof that every redirect behavior conforms.
The 20-execution URL canary reached real assertions in window and worker contexts
(five pass, fourteen assertion failures, one runtime failure). Source fingerprints
are recorded per result; these slices are not merged into the new Core score.

The next Core run uses `inventory-v3/plan.json --tier core`: 93 shards and 13,747
executions, eight concurrent cold isolated workers. Output is reserved separately
as `artifacts/wpt-full-baseline/results-core-v3`. Timing is recorded by the
controller. No persistent-parent/prewarm optimization is claimed. Do not edit the
harness or replace the binaries during this run; retain failures and unsupported
entries in the ledger. The resulting score remains dirty-tree diagnostic evidence.

The testdriver vendor currently supplies
click, send_keys, actions and accessibility operations, not the full upstream
automation API. Missing operations must be distinguished from browser failures.
Other legacy fixture adaptations and incomplete native automation support remain
limitations; this qualification is not a claim that the entire harness is
equivalent to wptrunner.

After that baseline, the first runtime batch remains initial iframe document,
stable Window identity and navigation/message lifecycle, with exact failing IDs
and passing neighbors. No frame/navigation compatibility gain is claimed yet.
