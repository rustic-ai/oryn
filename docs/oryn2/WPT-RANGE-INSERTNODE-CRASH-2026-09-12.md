# Unresolved crash: WPT Range.insertNode

Reported: 2026-09-12. Status: **unresolved; investigation deferred at user request**.

## Failure and impact

`dom/ranges/Range-insertNode.html` aborts the signed native page worker with a stack overflow. The latest run failed after 184 ms, before producing a valid WPT completion or any reported subtest results. This is a runtime crash, not an assertion failure, timeout, or environmental block.

Suggested priority: **P0 for the crash-repair queue**. The affected page is lost; the persistent parent remains alive and acknowledges shutdown with exit code 0. The scope beyond this fixture and the precise native root cause are not yet established.

Observed diagnostics:

```text
sandboxed native worker crashed: worker closed its response stream
page PageId(3) crashed before teardown
thread '<unknown>' (6636064931) has overflowed its stack
fatal runtime error: stack overflow, aborting
```

The first two messages are page-specific. The stack-overflow text is captured in shared parent stderr; its repetition in individual result records does not mean multiple crashes. The resource audit for this case is complete, with no recorded gaps. Completion validation reports `completion_not_object` because the worker died; this is a consequence, not a separate root cause.

## Preserved evidence

- Result: [crash-regressions-v2-signed-chromium.json](../../artifacts/wpt-full-baseline/crash-regressions-v2-signed-chromium.json), assertion path `dom/ranges/Range-insertNode.html`.
- Result SHA-256: `a7b2c4db92b8da9f0a4a4c9e52c48b8a6bfa0bac6787a4cf013bea94564bf94c`.
- Inventory: [wpt-crash-regressions-v1.json](../../benchmarks/conformance/wpt-crash-regressions-v1.json).
- Pinned WPT revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.
- Environment: signed macOS ARM64 release worker, protocol 9, App Sandbox, `isolated_prewarmed`, persistent parent, three jobs, upstream harness with Chromium reference checks.
- Worker SHA-256: `ba2b6f06cd3fc82568b9131344708e6f4a09dd2e4aaf54b84ac8d04a92b685dc`.
- Worker Code Directory hash: `ed35f3e8db32b4c072286aeafea9a39a4c330d4e`.

This is dirty-source-tree diagnostic evidence, not a reproducible clean-source release attestation. Do not rewrite the result or use this incomplete case to infer a subtest pass rate. Other compatibility failures are outside this report.

## Investigation so far

The upstream fixture uses two iframes and restores their documents using `referenceDoc.documentElement.cloneNode(true)` before running Range tests. Recursive DOM cloning and iframe/document lifecycle are investigation leads only; neither has been established as the cause.

A prior signed-run macOS crash report is available locally at:

```text
/Users/rohit/Library/Logs/DiagnosticReports/oryn-page-worker-2026-09-11-233015.000.ips
```

That report contains repeated native frames, but the release binary is stripped. No reliable function-level attribution has been obtained.

An exploratory ignored test, `pinned_range_insert_node_fixture_does_not_overflow_native_stack` in `crates/oryn-native/src/rust_executable.rs`, did not reproduce the crash under LLDB. **That apparent success is invalid evidence:** required fixture scripts were not loaded, `testDiv` was undefined, and there was no WPT completion. Resource aliases were subsequently added and compiled, but the revised diagnostic was not executed before investigation stopped. The test does not yet assert valid harness completion and must not be treated as a passing regression.

No confirmed fix or passing signed reproduction exists. Stack limits and security limits were not relaxed.

## Reproduction when resumed

From the repository root, with the pinned checkout, signed release build, Chrome, and Node available, rerun the existing diagnostic inventory. Use a new output filename to preserve earlier evidence:

```sh
python3 scripts/run-upstream-wpt.py \
  --checkout /Users/rohit/Work/oryn2/oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0 \
  --binary target/release/oryn \
  --inventory benchmarks/conformance/wpt-crash-regressions-v1.json \
  --output artifacts/wpt-full-baseline/crash-regressions-v3-signed-chromium.json \
  --jobs 3 --harness-profile upstream --parent-mode persistent \
  --chromium '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  --node /Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node
```

This command runs the existing surrounding regression slice, not just the crashing file. Confirm binary identity against the recorded hashes before claiming an exact-binary reproduction; rebuilding the dirty tree may produce a different binary.

## Next investigation and closure criteria

1. Establish a faithful debuggable reproduction: verify all fixture scripts and both iframe test bodies execute before trusting any no-crash result.
2. Capture a symbolized native stack and minimize the triggering Range/DOM operation. Upstream `?subtest=i,j` subsets may help isolation but cannot stand in for the full test.
3. Fix the confirmed cause and add a regression that checks execution/completion, not merely process survival. Do not mask recursion by increasing stack limits.
4. Rerun the complete upstream file with the signed sandboxed worker and the surrounding regression inventory. Require valid completion and no worker crash or crash-related cleanup error. Record any remaining assertion failures separately; crash resolution alone is not conformance.

Work is paused for future investigation. No additional run was launched for this report.

## September 13 retest after frame association migration

A subsequent signed diagnostic rerun still crashed with stack overflow after
200 ms, without valid WPT completion. The parent acknowledged shutdown and
exited successfully. Removing live iframe-to-Document DOM child edges did not
resolve the crash; do not treat that architectural repair as its root cause or
fix. No stack or security limits were relaxed.

New immutable result: `/private/tmp/oryn-frame-initial-range-20260913.json`,
SHA-256 `9fa8a2c8c4caa67aa44acd75c282a44e925e88afe0a145a67b62ccb13618726b`.
Source and signed-worker identities are recorded in
[the initial frame-association diagnostic](../../benchmarks/evidence/browser-frame-association-initial-2026-09-13.json).
That snapshot also failed a separate semantic-action regression; its full native
gate was not passed. The original historical evidence above remains unchanged.
