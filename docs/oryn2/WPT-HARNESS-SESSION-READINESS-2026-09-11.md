# WPT harness session repair — implementation checkpoint

This is a partial implementation checkpoint, not full harness readiness or a new
35,010-execution conformance baseline. Historical evidence has not been rewritten.

## Implemented

- Fail closed on failed exits, malformed/duplicate completion payloads, duplicate
  subtest names, session errors, teardown failures, and parent shutdown failures.
  Assertions remain available independently of execution integrity.
- Explicit `native --wpt-session` JSONL transport, eight execution slots, fresh
  contexts and permanently assigned isolated workers. Recycled mode is rejected.
  Explicit upload prerequisites are imported into each fresh context.
- Default upstream fixture profile leaves HTML bytes untouched. Completion uses a
  versioned external report adapter and evaluation, not a DOM/OIL result element.
  Legacy instrumentation remains explicitly selectable and unqualified.
- HTML-parser-based static script discovery, including unquoted URLs and base
  elements. Preflight inspects public served responses, including the report adapter.
- Single-cohort full-baseline controller, immutable source references, exact-ID
  checks, atomic checkpoints, 30-second progress, and a retained manifest-coverage
  reference. It prohibits reuse of historical shards in cohort mode.
- Chromium CDP canaries use fresh temporary profiles and the same served fixtures.
- Qualification schema 2 requires successful execution/shutdown and complete
  resource evidence. Old rows lacking that evidence remain unresolved.

## Verified

- 73 Python WPT regression tests passed, no skips (Node and live pinned WPT enabled).
- Rust `cargo check`, `cargo clippy ... -- -D warnings` for native/core-full passed.
- `cargo fmt --all -- --check` passed.
- Signed macOS build passed designated-requirement verification, protocol-v9
  handshake, and filesystem/listener/outgoing-network/environment sandbox probes.
- `session-upstream-15-v1.json`: 15 completed, 14 passed, one assertion failure;
  all transport/teardown paths succeeded. The failure is the existing FormData
  select-2 value mismatch, not a claimed harness repair.
- `session-cold-15-v1.json`: also 14/15. These tiny runs are not a performance
  qualification or evidence of a pooling speedup.
- `session-chromium-canary-v1.json`: four executions matched Chrome 152 exactly
  on subtest names and statuses, including a dedicated-worker wrapper.

Artifacts are under `artifacts/wpt-full-baseline/`. Source is dirty, so these are
signed diagnostic results, not clean-source release attestations.

## Still required before full-baseline acceptance

1. Runtime prerequisite telemetry is incomplete: the current trace exposes
   navigation but not every subresource. Add bounded parent-broker observations
   including response hash/MIME, initiator, worker/page identity, timing, errors,
   cancellation and redirects, without logging bodies or secrets. Current rows
   explicitly set `resource_evidence_complete: false` and cannot qualify.
2. Bind audited static and observed prerequisite sets, expected-error allowances,
   and supported automation operations to the qualification review. Literal
   dependency discovery alone is not exhaustive.
3. Expand fixture-fidelity and Chromium canaries across modules, redirects, CSP,
   dynamic resources, nested contexts, intentional errors, and uploads. The four
   passing comparisons are not exhaustive validation.
4. Add signed lifecycle fault injection, broader isolation/leak checks, native
   session bounded-frame tests, and parent/aggregate-worker RSS progress. The
   native input currently checks size after reading a line; it needs a bounded read.
5. Run the fresh 35,010-execution cohort only after these gates; retain the 93,544
   unsupported manifest entries and publish a strictly qualified mixed ledger.

The full run has not been started. These remaining issues are implementation and
validation work, not an environmental/signing block.
