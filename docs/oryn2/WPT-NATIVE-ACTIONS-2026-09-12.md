# Native action-sequence repair — 2026-09-12

The restricted mouse/keyboard action path now passes all seven selected, unchanged pinned WPT canaries in both signed isolated-on-demand and isolated-prewarmed workers. Chromium agrees on every named subtest and status. This is a diagnostic readiness result, not a new full WPT baseline or clean-source release attestation.

## Changes

- Replaced the approximate action shim with a versioned realm-owned scheduler: tick durations, pauses, pointer interpolation, execution-time element positions, held keys/buttons, modifiers, click counts, serialized requests, immutable request snapshots and failure cleanup. Unsupported variants reject before input. Pinned WPT resets held input before the next sequence; successful sequences do not prematurely release it.
- Added generated native MouseEvent, PointerEvent, KeyboardEvent and WheelEvent payload bindings. Coordinates, modifiers and other typed read-only fields survive trusted dispatch. Click default actions use native dispatch rather than synthesizing another `.click()`.
- Routed element-origin and explicit-context actions into the appropriate same-origin frame realm. Added native own indexed frame access and live Window.length; frame removal updates indices without a script-generated frames array. Existing cross-origin access policy remains unchanged.
- Fixed Chromium reference multi-click counts, which previously always reported one.
- Fixed the direct HTTPS fixture's missing TLS close_notify. Close-delimited upstream frame responses previously reached HTTP 200 and then failed strict native body decoding. The fixture now closes TLS cleanly; upstream HTTP bytes, certificate verification and hostname checks are unchanged. Strict TLS regressions cover both HTTPS ports.

## Signed evidence

Pinned WPT revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`. The [seven-case inventory](../../benchmarks/conformance/wpt-native-actions-v1.json) binds exact source hashes. Cases cover element position, layout changes between actions, pauses, Shift modifiers, multi-click counts, same-origin frames and cross-origin frames.

| Run | Outcome | Total wall time | SHA-256 |
| --- | --- | --- | --- |
| [Prewarmed](../../artifacts/wpt-full-baseline/native-actions-v3-persistent-2026-09-12.json) | 7/7 passed | 34.408 s | `9997dda64760cad0db0a640dbdc1747662471d60b3bd14fc38ae3b7f429799f3` |
| [Cold](../../artifacts/wpt-full-baseline/native-actions-v3-cold-2026-09-12.json) | 7/7 passed | 34.439 s | `bdfe31d9b3a7fd5c6dcbcb865142bbf1196ac883e145e90268f7dc476887bd21` |

All 14 records pass current harness checks, have complete resource evidence, clean fixture/worker teardown and valid matching Chromium references. Exact execution IDs and named-subtest statuses match between lifecycle modes. These wall times include fixture/reference work and are not startup or throughput benchmarks.

- Parent SHA-256: `a16455cd9a4189333b3dcc3a891d0034cd5679020e10c520135e6810c33d2f98`.
- Worker SHA-256: `891c01aa7c3e7e06535cac0f1399786ea96b19ba664c6e9ea4e24803c872bca5`.
- Worker CDHash: `e61376e4b95a3f57c53f1ef4a1ef956f9bcdc1cb`.
- WebIDL manifest SHA-256: `eb0d66cc191bfbcd25645c60b988541d423b048e561ca6ac6cd719e965b9df48`.
- Signed build passed protocol-v9, designated-requirement, native DOM/broker and filesystem/listener/outbound/secret-environment denial probes.
- [Review candidates](../../artifacts/wpt-full-baseline/native-actions-v3-review-candidates-2026-09-12.json) retain blank approval and verified fields; they are not automatic qualification.

Earlier v1 and v2 action diagnostics remain immutable. v1 exposed missing generated event registration; v2 passed five cases and exposed frame indexing and HTTPS termination defects. Neither is merged into the final result.

## Regression verification

- Native core-full library: 161 passed.
- V8 binding library, serial: 200 passed, three existing diagnostics ignored. The earlier parallel attempt trapped in V8; it is not reported as passing or resolved by this work.
- V8-enabled native frame tests: 22 passed. The initial restricted run had one listener-permission failure; the permitted complete rerun passed.
- V8-enabled native focus/input tests: four passed.
- WebIDL library: ten passed; generated registry check passed.
- Node scheduler/reference/adapter tests: 20 passed.
- Full Python WPT discovery with signed worker, pinned fixtures, Chromium and Node: 114 passed, zero skips, 95.572 seconds.
- Native/V8 library clippy with warnings denied, formatting and whitespace checks passed.

## Remaining boundaries and next step

The capability manifest now marks Oryn action_sequence **unqualified**, rather than unsupported. Touch, pen, wheel actions, multiple pointer/keyboard sources, unmapped special keys and mixed-frame element origins within one sequence still reject explicitly. WheelEvent payload support does not imply wheel-action support.

Full pointer capture, ancestor boundary ordering, compatibility-mouse cancellation/retargeting and special-key default actions are not qualified by these canaries. The cross-origin canary passes its real input/message assertions, but the parent completion's automation log does not aggregate child-realm operations; it must not be promoted to fully audited cross-frame automation evidence on that basis.

Next: qualify exact action-dependent Core slices against these limits, add native cross-realm automation trace collection before qualifying child-only traces, and then run a fresh comparable Core baseline. Do not silently qualify all action-dependent tests from these seven passes. No full-cohort pass rate is claimed. The separately reported Range.insertNode crash is unchanged and deferred.

## Subsequent Core interaction track

The [Core interaction report](WPT-CORE-INTERACTION-2026-09-12.md) records the native cross-frame trace repair, the complete 172-execution mapped interaction assessment, three further Core repairs and ten source-qualified diagnostic regressions. It supersedes the missing same-isolate child-trace boundary above, not the broader conformance limits or historical evidence.
