# ADR 0001: Use raw V8 for the native page host

- Status: Accepted for G0–G2
- Date: 2026-08-25
- Reference platform: Apple M2 Max, 32 GB, macOS ARM64
- Engine revision: V8 150.4.0 for both candidates

## Decision

Use the raw `v8` crate behind `oryn-native` as the page JavaScript host. Keep
`deno_core` as a feature-isolated comparison probe, not a runtime dependency of
the selected native profile.

The browser owns task scheduling, modules, Web APIs, and event-loop semantics.
Only borrow independently useful Deno utilities when they do not pull the
`deno_core` runtime into the native dependency graph.

## Evidence

`scripts/benchmark-g0-hosts.sh` ran 5 warmups and 30 measured processes for
each host. Each process created one runtime and executed the same script 100
times in a persistent realm.

| Measurement | Raw V8 | `deno_core` |
| --- | ---: | ---: |
| Startup P50 | 1.117 ms | 3.747 ms |
| Startup P95 | 1.377 ms | 4.096 ms |
| 100 executions P50 | 0.094 ms | 0.069 ms |
| Stripped probe size | 40,709,536 bytes | 42,608,176 bytes |

`deno_core` exceeded the plan's maximum 10% startup regression by a wide
margin. Its execution-loop advantage does not compensate for the startup cost,
and the raw host better matches Oryn's browser-owned scheduling boundary.

## Consequences

- V8 isolates remain thread-affine inside `PageRuntime`; only typed page
  handles cross threads.
- Oryn must implement module loading and browser event-loop integration.
- Both probes remain buildable so major dependency updates can be remeasured.
- G1 must validate that the owned event loop passes the advertised HTML tests;
  this ADR does not treat lower startup time as evidence of semantic correctness.
