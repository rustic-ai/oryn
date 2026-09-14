# Selection.extend repair

## Scope and implementation

This follows the Range stringification crash repair. `Selection.extend()` now replaces the selected Range without mutating retained old Range objects. It ignores detached targets before checking for an empty selection or invalid offsets, rejects document types with `InvalidNodeTypeError`, and collapses both endpoints at the new focus when crossing tree roots within the associated document.

The shared internal editing helper retains its previous mutation behavior; replacement is explicitly selected for `extend()`. Other Selection methods are not claimed repaired. Replacement happens after validation and allocation, preserving the current selection if validation fails. Existing weak-release collection owns cleanup of superseded ranges.

Reference: [Selection API extend algorithm](https://www.w3.org/TR/selection-api/#dom-selection-extend).

## Regression coverage

The new Rust regression covers empty-selection errors, detached-target no-ops, forward/backward transitions, preservation of old Range identity and endpoints, same-point replacement, invalid-offset and document-type errors without mutation, connected shadow-root transitions, and collection after 256 replacements.

All 197 regular `oryn-js` Rust tests pass serially. Strict all-target clippy, formatting, and diff whitespace checks pass. Signed upstream evidence is recorded separately below; no historical cohort results are rewritten.

Both pinned-upstream tests also pass (83.38 seconds), for 199 passing tests across the serial runs. Signing, designated-requirement verification, protocol-v9 handshake, and sandbox denial probes pass.

## Signed upstream result

Inventory: `benchmarks/conformance/wpt-range-stringification-v1.json`, pinned to WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`. Execution used three jobs, a persistent parent, isolated prewarmed pages, the upstream harness, and Chromium reference checks. Production limits were unchanged.

| Case | Previous result | Current result |
| --- | --- | --- |
| Range-stringifier | 5/5 assertions | 5/5; matches Chromium |
| extend-00 | Execution failure | Execution failure; 12,541 ms |
| extend-20 | Execution failure | Execution failure; 13,540 ms |
| extend-40 | 26/176 assertions | 176/176; matches Chromium; 14,228 ms |

The focused slice improves from one to two passing files: 150 additional passing assertions in `extend-40`. All four cases were attempted; neither remaining execution failure reported subtests. Total run time was 28.332 seconds. No worker crashes, cleanup errors, or missing resource evidence were recorded. Parent shutdown was acknowledged with exit code zero and empty stderr.

This is a semantic improvement, not a performance improvement: the observed times increased relative to the preceding single run. The two larger selection cases remain unresolved and need execution-cost/termination diagnosis. Reentrant XHR and other crash families remain separate pending work. This dirty-tree diagnostic slice does not change the full-cohort baseline, establish full Selection conformance, or constitute clean-source release evidence.

Evidence SHA-256:

- Previous focused artifact `range-stringification-v2-signed-chromium.json`: `89602ccd9e7dbb1af4f5bfb4118f2d4035fd02b35625cae19c7420f031fb40c3`.
- Current artifact `artifacts/wpt-full-baseline/selection-extend-v1-signed-chromium.json`: `f9941ca66925fe2cfe9c1e42ff773f420e0c8bd0d0773b56b84e528802fa54df`.
- Parent executable: `c8b2c88e27a5d8b8171d7a35eca6779a3a23154ec737a1d697c866729e9ff8c7`.
- Signed worker: `836835543e747c0d432ebc8e8243d2aadbc81d86807ce23e7f1b8a901adb7a01`.
- Worker Code Directory hash: `113b8f1522d588c3f5a34db28abb1b390f372215`.
