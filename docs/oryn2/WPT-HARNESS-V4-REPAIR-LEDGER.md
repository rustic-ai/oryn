# Supported WPT harness v4 repair ledger

This work repairs the window/dedicated-worker harness. It does not repair the deferred Range.insertNode crash or add runner families. Historical v3 evidence remains unchanged. Implementation and qualification are separate: no full-cohort conformance improvement is claimed here.

| ID | Classification | Repair | Closure evidence / remaining gate |
| --- | --- | --- | --- |
| H4-01 | Confirmed fixture distortion | Supported executions use direct pinned wptserve HTTP/HTTPS/H2 listeners, native wrappers and narrow reporting routes. The buffered proxy is legacy/preflight-only. | Live first-header/first-body-before-EOF and cancellation regression passes. Broader transport canaries still required for qualification. |
| H4-02 | Confirmed invalid reference automation | Distinct Chromium adapter and CDP host bridge; no Oryn-native calls in reference pages. | Chromium click, typing, clipped element-origin actions, modifier actions, and same/cross-origin iframe canaries pass. Accessibility serialization matches the pinned Chrome backend. Unsupported variants reject before dispatch; the approximate Oryn action-sequence shim is explicitly unavailable to WPT. |
| H4-03 | Shared fixture-state contamination risk | Every execution owns a fixture process. Browser comparisons use fresh stash generations on the same endpoints. | Live stash-write/restart/read test passes; no shared-stash inference is used to reclassify historical failures. |
| H4-04 | Evidence attribution gap | Structured, bounded automation outcomes; no persisted input arguments. Host-side Chromium records include context identity and stale-context rejection. | Unit tests cover unknown variants, stale contexts, terminal ordering and overflow gates. Source-specific automation reviews remain mandatory. |
| H4-05 | Transport audit gap | Bounded native WS/WSS records retain page/socket identity, redacted URL, query digest, handshake result, application-payload byte counts and terminal state. No payloads, cookies, protocol tokens or close reasons are retained. | Native redaction/failed-handshake/abandonment tests pass. WS/WSS live qualification remains a separate gate; counters are application bytes, not wire bytes. |
| H4-06 | Qualification scalability | Exact-execution review-bundle generator, capability manifest, and schema-v4 fixture/reference/automation gates. | Bundles leave audit rationales and verification fields blank; generating one cannot qualify a result. Old evidence requires explicit historical mode in the qualification CLI. |
| H4-07 | Stability and unattended completion | Fresh fixtures have bounded teardown, process failures halt submission, and output paths require explicit resume. Added direct-fixture churn driver with eight slots and unique-worker checks. | Signed 1,000-execution stress passed with 1,000 distinct workers and clean shutdown. Full supported-cohort run remains gated on transport qualification. |

## Qualification boundaries

- Native HTTPS/WSS now validate a run-local CA constrained to localhost/loopback, with hostname verification enabled. The CA is not installed in a system store. Chromium reference runs still bypass certificate validation: TLS-validation-dependent comparisons cannot be qualified; other comparisons need an explicit independent-of-certificate-validation review.
- Chromium implements basic click/typing, accessibility queries and restricted mouse/keyboard actions, including element origins and action modifiers. Touch, pen, wheel, multiple pointer/keyboard sources, unmapped special keys and send_keys modifier sequences reject before input. Oryn action_sequence is explicitly unsupported because the legacy shim ignores pauses and approximates pointer semantics. This is a recorded capability limit, not a passing test or a hidden exclusion.
- Supported does not mean qualified. The capability manifest intentionally records unqualified transports/automation until the corresponding live, source-reviewed gates pass.
- A Chromium status difference is an engine comparison, not a harness failure. Invalid reference completion, missing subtest identities, unsupported automation, stale contexts and fixture failures remain independent blockers.
- Fixture servers restart per execution/browser. Their startup and shutdown time is recorded separately from native execution; the extra cost buys server-state isolation and is not hidden in engine throughput claims.
- The current checkout is dirty. New signed runs are diagnostics, not clean-source release attestations. Do not overwrite or aggregate them into the historical full baseline.

## Commands

Run local unit tests with Node on PATH:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_*wpt*.py'
node --test scripts/tests/test_wpt_chromium_driver.mjs scripts/tests/test_wpt_oryn_driver.mjs
cargo test -p oryn-native --features core-full resource_audit --lib
```

Live fixture tests require `ORYN_WPT_CHECKOUT`; Chromium checks additionally require `ORYN_WPT_CHROMIUM` and `ORYN_WPT_NODE`:

```sh
python3 -m unittest discover -s scripts/tests -p test_wpt_direct_fixture.py
```

After signed canaries pass, run `scripts/stress-wpt-direct-fixtures.py` with `--checkout`, `--binary`, and a new `--output`. Defaults are 1,000 executions and eight jobs. It stops submitting after any failed canary and never retries it.

Prepare manual review candidates using `scripts/prepare-wpt-review-bundle.py --evidence RESULT --checkout CHECKOUT --output NEW_FILE`. Review each candidate against pinned source and observed prerequisites before transferring observations into verified fields.

## Verified checkpoint — 2026-09-12

- Native unit suite: 159 passed. The first sandbox-restricted attempt had four local-listener permission failures; the permitted rerun passed all tests.
- Python WPT-related discovery: 109 tests, 98 passed and 11 explicitly skipped live tests. The live suites were run separately: five direct-fixture/reference tests, five signed session tests and two fixture-topology tests passed. Node driver unit tests: four passed.
- Native clippy with warnings denied passed. The signed build passed protocol-v9, designated-requirement, and filesystem/listener/outbound/secret-environment sandbox probes.
- Signed parent SHA-256: `e80e8c59b0296640b197f0a48f01faa77c02f6167d1c211afa8fbd4234701753`.
- Signed worker SHA-256: `6429e70ec9c550c80bd6000e48129b18c6cc262e884290faaa81356f0bf1b589`; CDHash `12fe3170c6faa3b15df2d83665a412e7770736ee`.
- [Eight-way stress](../../artifacts/wpt-full-baseline/harness-v4-direct-stress-1000-2026-09-12.json): 1,000/1,000 executions, 1,000 distinct workers, 160.289 seconds, no parent stderr, acknowledged exit zero. SHA-256 `25c0dcc6750b6a27e985fdf08b280e09c49ed6f3958659935a925e641a97f057`. This is a stability observation, not a cold-start or action-throughput benchmark.
- [Signed differential canaries](../../artifacts/wpt-full-baseline/harness-v4-signed-canaries-2026-09-12.json): seven finished; five passed, two failed; no execution, fixture or cleanup errors. SHA-256 `7bdb4aae1909d9fb53529fe95e8e4575bf13c7f782f3a65b9315c5094be84c00`. Range stringifier, click, typing, and window/dedicated-worker URL wrappers passed. Chromium produced valid completion with matching subtest identities for all seven.
- [Cold-worker canaries](../../artifacts/wpt-full-baseline/harness-v4-cold-canaries-2026-09-12.json): all five non-WebSocket cases passed; exact IDs and assertion names/statuses match the signed prewarmed slice. No retry or merged conformance score is implied.
- [Review candidates](../../artifacts/wpt-full-baseline/harness-v4-review-candidates-2026-09-12.json) preserve blank approval rationales and verified fields. None were automatically qualified.

### Blockers at the earlier checkpoint (superseded for WSS below)

The WS zero-byte-send case reaches handshake 101, then fails an assertion involving an undefined value and zero; its connection is recorded as abandoned during teardown. The WSS variant records a failed handshake and unfinished subtest. The exact WSS failure cause is not established by these records. Neither result is converted into a pass, an environment exclusion, or a confirmed fixture defect.

Full transport qualification, action-variant expansion/qualification, and clean-source release evidence remain unfinished. The full supported cohort has deliberately not been rerun because these readiness gates are not satisfied. The deferred Range.insertNode crash remains untouched.

## Final repair verification — 2026-09-12

The WSS handshake defect is repaired. Its causes were missing explicit fixture trust in the native WebSocket connector and a 10 ms polling timeout incorrectly applied during TLS/HTTP handshake. The connector now uses the normal request deadline for handshake and only switches to polling afterward. Explicit fixture trust is selected only for WSS with both a localhost-family hostname and exclusively loopback resolved addresses. A failed WebSocket send retains its observation, so later received payloads remain counted.

Cold/prewarmed comparison then exposed an additional retirement race: broker threads could retain a context after worker exit, so a cold result could contain a WebSocket with no terminal state. Worker kill/unbind now synchronously revokes the context and cancels its transports, even while another thread retains a route clone. A late WebSocket handshake cannot install a socket after cancellation. The regression checks both lifecycle modes; the original `harness-v4-final-cold-2026-09-12.json` remains immutable evidence of the defect, not accepted readiness evidence.

- Native unit tests: **161 passed**. Native core-full clippy with warnings denied passed.
- Final Python discovery with all live dependencies enabled: **113 passed, zero skips**, 97.155 seconds. This includes the fixture, signed-session and topology suites. The earlier no-live-environment run had 99 passes and 14 explicit skips; it is not substituted for the final live result.
- Node adapter tests: **9 passed**, including fail-before-input checks, stale context/AX rejection and explicit rejection of the approximate native action shim.
- Direct fixture suite: **8 passed** within the final discovery, including streaming-before-EOF, stash isolation, strict CA/hostname checks, HTTP/2 extended-CONNECT echo/close, cold/prewarmed WebSocket retirement, and six pinned Chromium automation pages. Signed-session tests include isolation, timeout/crash replacement, auxiliary pages and Chromium fixture comparison; topology tests include handler cleanup.
- Final signed build: protocol v9, designated requirement, DOM mutation, authorized broker access and sandbox denial probes passed. Parent SHA-256 `293636784d5524115e6663bcab98b44906f332b823f8f2671c528e3fda050e0e`; worker SHA-256 `d2e1bcd616077ab284cc48a8e4a31040c153ba4e7c05ba7c0eed459b7c9a7cdb`; CDHash `487a1d0cf883e106cb0e6fc7c527407e016ea0c3`.
- [Final cold canaries](../../artifacts/wpt-full-baseline/harness-v4-retirement-cold-2026-09-12.json), SHA-256 `31b2e35f28541b871ca9de9e31db8731df7f53ef5fe67d67da4136d5e34e8de3`, and [final prewarmed canaries](../../artifacts/wpt-full-baseline/harness-v4-retirement-persistent-2026-09-12.json), SHA-256 `d7e0a012adea7465f0501078208cf02050c6bd816ae59f9365f0cf7fd3881cff`: **7/7 exact execution-ID and named-subtest/status parity**. All 14 records pass current harness checks and have complete resource evidence. Both modes report five passing cases and two genuine assertion failures; WS and WSS both handshake 101 and terminate as cancelled/abandoned after assertion failure. Neither failure is suppressed.
- [Final review candidates](../../artifacts/wpt-full-baseline/harness-v4-retirement-review-candidates-2026-09-12.json) bind the exact current evidence and pinned sources, with approval fields deliberately blank.
- [Final eight-way signed stress](../../artifacts/wpt-full-baseline/harness-v4-retirement-stress-1000-2026-09-12.json): **1,000/1,000 passed, 1,000 distinct worker PIDs, 162.118 seconds**, no parent stderr, acknowledged shutdown and exit zero. SHA-256 `9eac2dbd1731c3c1cb44778104fa9d661013d4950214e41df35368cf4a1a231b`. Every execution requires complete resource evidence and clean current-harness/retirement checks; this remains a stability diagnostic, not a conformance or throughput score.

The supported-harness repair and diagnostic readiness pass is complete. The full supported cohort has not been rerun or promoted to qualified conformance: the exact-source review and capability-qualification boundary above still applies. In particular, rejection of native action_sequence repairs misleading evidence; it does not implement that missing engine capability.

Remaining conformance work at this checkpoint was separate from these repairs: source-specific reviews, faithful native action-sequence implementation, broader capability qualification and a clean-source release run. The deferred Range.insertNode crash remains unresolved. No historical evidence has been overwritten, and this repair report does not claim a new full-cohort pass rate.

## Subsequent native-action track

The [native action repair report](WPT-NATIVE-ACTIONS-2026-09-12.md) supersedes the action_sequence unsupported boundary above for the restricted version-1 mouse/keyboard scheduler. Seven pinned canaries pass in both signed cold and prewarmed workers, matching Chromium. The current capability remains unqualified, with explicit unsupported variants and cross-frame trace limits; historical results and the deferred crash remain unchanged.
