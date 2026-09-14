# Core conformance completion gates

Objective: complete Core conformance. Individual repaired slices do not redefine this objective.

The authoritative coverage boundary is the pinned full WPT manifest and the current tier policy, not the historical 4,022-test subset. The current inventory (`artifacts/wpt-full-baseline/inventory-v3/plan.json`) binds revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`, manifest SHA-256 `566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101`, and policy SHA-256 `a655bd38bbd3cc90e49344f7ceae1db1a3bfb09c8b84419b336ebf72f81eb84f`.

## Coverage that must remain visible

| Core test type | Mapped executions | Unsupported entries |
| --- | ---: | ---: |
| testharness | 13,747 | 893 |
| reftest | 0 | 1,003 |
| crashtest | 0 | 330 |
| manual | 0 | 749 |
| print-reftest | 0 | 12 |
| visual | 0 | 3 |
| Total | 13,747 | 2,990 |

These are inventory counts, not current pass results. Unsupported execution is unfinished coverage, not an exemption or a passing result. Manual/platform-dependent requirements need a supported execution or explicit independently verified assessment; do not silently remove them to declare completion.

## Required evidence before completion

1. Reconcile every Core manifest entry, global and variant against the coverage ledger, including previously unsupported types and contexts. No missing or duplicate IDs; no pass-dependent tier changes.
2. Establish working fixtures, resource and automation prerequisites for each execution. Reject missing helpers, incomplete subtest observations, unsupported automation, unresolved crashes/timeouts, and unverified reference prerequisites. Source-bound reviews must actually cover the prerequisite being asserted.
3. Fix the browser mechanisms behind observed Core failures, with regression tests that cover state transitions, lifecycle and security boundaries—not test-path-specific success responses.
4. Run the entire Core selection on a frozen signed candidate, preserve all raw outcomes, reconcile aggregate counts, and qualify results. Historical multi-version diagnostics or a small green slice cannot prove this gate.
5. Verify production isolation/security behavior and clean-source build identities. Parent, worker, V8, bindings, ICU, snapshot, signature and source provenance must match the tested candidate. Historical evidence remains immutable.

Current status: unproven/incomplete. The recovered diagnostic Core baseline recorded 3,618 passes and 10,129 failures across the 13,747 mapped executions; it is not a current qualified score. Later bounded repairs do not establish a new whole-Core pass rate. The reference-focus defect and all unresolved feature/runner coverage remain open until authoritative evidence proves otherwise.
