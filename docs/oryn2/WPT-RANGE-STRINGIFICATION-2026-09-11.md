# Range stringification crash repair

## Scope

This is a focused runtime repair, not a new full WPT baseline or a conformance attestation. The existing dirty worktree and historical cohort results are preserved.

The audited cohort had worker crashes in `selection/extend-00.html`, `extend-20.html`, and `extend-40.html`. Range stringification treated container preorder positions as range boundaries. An end container that was an ancestor of the start container could produce a reversed slice, causing a Rust panic, or incorrectly produce an empty string.

## Implementation

`range_text` now handles partial boundary Text nodes and contained Text/CDATA nodes separately. Comments and processing instructions are excluded, following the [DOM Range stringifier](https://dom.spec.whatwg.org/#dom-range-stringifier). Child boundary positions are computed once instead of comparing every text node against both endpoints through repeated tree traversals.

Two Rust regression tests cover ancestor endpoints, element child offsets, collapsed ranges, comments, CDATA, processing instructions, and UTF-16 offsets spanning a complete surrogate pair. Lone-surrogate preservation is not addressed by this patch.

## Evidence discipline

- Pinned WPT revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.
- Focused inventory: `benchmarks/conformance/wpt-range-stringification-v1.json`.
- Signed protocol-v9 workers, persistent parent, isolated prewarmed pages, upstream harness, and Chromium reference runs.
- The four-case slice includes the three previously crashing selection cases and the already-passing `dom/ranges/Range-stringifier.html` control. Passing that control is not an incremental pass gain.
- Production command limits remain unchanged. Execution failures and assertion failures are not counted as passes.

The first signed iteration, `artifacts/wpt-full-baseline/range-stringification-v1-signed-chromium.json`, eliminated worker crashes but left two JavaScript execution failures and one completed selection case with 26/176 passing assertions. This motivated the traversal-cost optimization. Its evidence remains unchanged.

## Remaining work

Selection extension currently mutates an existing Range where the upstream tests require a new Range identity. That is a separate semantic fix. Other original crash categories, including reentrant XHR body conversion, are not repaired by this patch. No full-cohort score or qualified-pass count should be updated from this slice.

## Final signed verification

The optimized implementation's second run finished in 24.344 seconds. It produced no worker crashes, no cleanup errors, complete resource evidence for all four cases, and an acknowledged parent shutdown with exit code zero and empty stderr.

| Case | Result | Runtime |
| --- | --- | --- |
| Range-stringifier | 5/5 assertions; matches Chromium | 160 ms |
| extend-00 | JavaScript execution failure; no reported assertions | 11,173 ms |
| extend-20 | JavaScript execution failure; no reported assertions | 11,354 ms |
| extend-40 | Completes; 26/176 assertions pass | 10,715 ms |

The two execution failures remain unresolved. The traversal optimization did not materially improve this slice's observed times. The completed selection case has range-identity, exception, and anchor-state discrepancies; crash removal does not resolve these semantics. Overall: one of four files passes, unchanged from the first iteration and with no new passing-file gain over the corresponding historical cases.

Validation: formatting and strict `oryn-js` all-target clippy pass. All 196 regular Rust tests plus both pinned-WPT tests pass serially. The default parallel test run aborted during V8 address-pool initialization; parallel test-runner readiness is not claimed. Signed build verification, designated requirement, protocol-v9 handshake, and filesystem/listener/outgoing-connection/secret-environment sandbox denial probes pass.

Evidence SHA-256:

- Historical audited raw cohort: `37d482dff827ae332b0a66c73dd8c4ef3d3a354a0bcfa7ff9dcec6565cfd6e47`.
- First focused artifact: `4ab313231d8c49ac21335609d3e2b2dbec8941544cce40d9e8a7f89f309021d0`.
- Final artifact `artifacts/wpt-full-baseline/range-stringification-v2-signed-chromium.json`: `89602ccd9e7dbb1af4f5bfb4118f2d4035fd02b35625cae19c7420f031fb40c3`.
- Parent executable: `c8b2c88e27a5d8b8171d7a35eca6779a3a23154ec737a1d697c866729e9ff8c7`.
- Final signed worker: `72a408b467e3ddf8f68a8a45958b2bbe0ad15979c7b7d361c70415eed1a5ee5b`.
- Final worker Code Directory hash: `73ef53d9c8e479f84e13023a70ffe530f3d06af1`.

These are dirty-tree diagnostic results, not clean-source release evidence. Historical protocol and full-cohort evidence remain unchanged.
