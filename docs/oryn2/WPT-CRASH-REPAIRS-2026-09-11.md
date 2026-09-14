# WPT completion and crash repairs

## Selection completion: verified

The large selection cases were exhausting the timer callback budget while upstream testharness constructed its visual results table. The machine reporting adapter now uses the upstream-supported `setup({output: false})` option. Test content, assertions, completion callbacks, and production time limits are unchanged. A runner regression checks that the adapter preserves failing assertions and names verbatim.

Separately, Selection and Range operations now synchronize only the Range records they access, rather than refreshing every retained Range. An isolated diagnostic using upstream selection assertion functions completes 4,400 assertions with no failures in about 1.1 seconds; this custom diagnostic is not official WPT evidence.

Signed upstream evidence with Chromium reference: `artifacts/wpt-full-baseline/selection-machine-report-v1-signed-chromium.json`.

| Case | Assertions | Case time | Chromium comparison |
| --- | --- | --- | --- |
| Range-stringifier | 5/5 | 72 ms | Identical names and statuses |
| extend-00 | 2,024/2,024 | 301 ms | Identical names and statuses |
| extend-20 | 2,376/2,376 | 352 ms | Identical names and statuses |
| extend-40 | 176/176 | 90 ms | Identical names and statuses |

All four files pass. These single-run case times are not a statistical performance benchmark. Total controller time including reference checks and cleanup was 14.157 seconds. SHA-256: `d212fa3f861327d32cbc59189f9a0eb23fe7c43e529b26d3f0e1b336a1c594e3`. Signed worker SHA-256: `02f5c55a7f757cb06a504037b359fd2f0223e20c8a947ab3b08fd2b4f900902f`.

## Diagnostic correctness

The host no longer lets an absent stack trace hide a thrown value. More importantly, stopped timer callbacks now check watchdog termination before reporting an exception: wall-time exhaustion is reported as a limit, and termination state is cleared. A regression checks follow-up execution after a timer is terminated.

The preceding 12-case signed run, with visual output still enabled, is preserved as `crash-regressions-v1-signed-chromium.json` (SHA-256 `0b8d1265f5e5447fe3d1dd58ee85b3d1ba5ad0c111adb2df7b3854eac2183817`). It recorded nine worker crashes, two correctly classified timer limits, and one passing selection case.

## Additional crash repairs under verification

- HTML parser element handles retain native leases until the parser finishes, including when script removes an open element.
- Invalid image and stylesheet URLs no longer reach panic-producing unwraps in the layout resource loader.
- Canvas ImageData uses signed dimensions, normalizes negative rectangles, crops available decoded pixels, and rejects excessive allocations before allocating. Pixel buffers are capped at 32 MiB. This does not implement the existing no-op canvas painting operations.
- Descendant text truncation respects UTF-8 character boundaries.
- XHR body conversion does not hold platform-object borrows across user JavaScript. Explicit send state rejects recursive sends; open cancels outstanding operations, and loadstart reentrancy is checked before submission.

No historical full-cohort result is rewritten. These are dirty-tree, signed-worker diagnostic results, not clean-source release qualification or full browser conformance. The new 12-case inventory is `benchmarks/conformance/wpt-crash-regressions-v1.json` and uses the same pinned upstream revision as the audited cohort.
