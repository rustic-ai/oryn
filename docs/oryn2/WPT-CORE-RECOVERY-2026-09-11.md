# Completed Core diagnostic baseline recovery

The Core selection is complete: 13,747 unique expected execution IDs across
93 shards, with no duplicate or missing IDs. Results: 3,618 whole-execution
passes and 10,129 failures. Observed subtests: 596,354, of which 154,349 passed.
Twenty-four executions explicitly report unsupported testdriver operations;
these annotations overlap failures, not additional executions.

Failure domains: 5,912 assertion, 2,299 harness, 1,907 runtime, six outer timeout,
and five incomplete. These describe observed outcomes, not proven defect owners.
The selected executions all reached a recorded terminal runner outcome; an
individual incomplete browser outcome still counts as a failed execution.
Core's 2,990 unsupported runner entries are separate from the runnable denominator.

## Repair and evidence integrity

The first XHR shard saved all 200 results but its runner aborted during Python
finalization: daemon request threads were still writing to stderr. Public server
request handlers are now non-daemon, sockets have bounded read timeouts, and
cleanup stops accepting requests and joins handlers before terminating upstream.
Both primary listeners are now closed as well as secondary listeners.

The original `artifacts/wpt-full-baseline/results-core-v3` is unchanged. Recovery
uses `results-core-v4-recovery/summary.json`, which references the original 89
controller-accepted shards by absolute path and SHA-256, plus four newly executed
shards. The aborted XHR shard was rerun, not silently accepted; it again passed
46/200 and this time its runner exited normally. The other three shards covered
the 233 previously unrun executions. No old fingerprint check was bypassed.

This is explicitly multi-harness, dirty-tree diagnostic evidence, not a clean
single-version release attestation. Every result retains its original fingerprint.
All 93 referenced files were hash-verified and their exact execution IDs reconciled
against the selected plan. All use the same parent and signed worker hashes:

- Parent: `df87fc9dc172158fe0f7922e031cf8aa85899be030d0f55d6ab3038ff90a1a03`
- Worker: `9f48f866a1a33c68d2a7313d6abb686e1ca3909c7cc1ea657e3e53432f8cefda`

Original controller time: 8,164.6 seconds. Recovery: 131.6 seconds. Combined active
controller time: 2h 18m 16s, including the aborted shard's first attempt, excluding
the idle interval and repair work. Cold isolated workers and production limits
were unchanged.

Validation: two live topology/shutdown tests pass, including proof that closing
the public server waits for an active request handler. Runner unit tests: 35 pass,
two Node-dependent skips. All four recovery shard processes exited normally;
the recovery controller exited zero and reports complete. Diff checks pass.

The progress monitor remains paused because the run is complete. Next work is
diagnostic grouping and the bounded frame/navigation compatibility batch, not
another baseline infrastructure restart.
