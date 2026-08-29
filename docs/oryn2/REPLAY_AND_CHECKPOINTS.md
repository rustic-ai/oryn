# replay traces, replay, and checkpoints

`TaskTraceArchive` records page lifecycle and DOM events, scrubbed OIL intents
and outcomes, semantic revisions, deterministic clock/random configuration,
and policy-controlled network exchanges. Authorization, cookie, and
authentication headers are excluded. Captured bodies are hashed and remain
bounded by the network broker.

## Determinism and replay

The native realm can freeze `Date` and seed `Math.random`, UUID generation, and
`getRandomValues`. Network replay matches method, URL, scrubbed headers, body
hash, and order. It remains inside `PolicyBroker`, so current policy can still
deny a captured request. WebSockets and live external processes are labeled
nondeterministic.

`Browser::replay_task_trace` verifies the checkpoint, restores it with the
completed trace archive, maps source pages to fresh identities, and replays only
post-checkpoint intents. Divergences are typed. Redacted sensitive values are
never reconstructed.

## Checkpoint levels

- L1 restores reloadable documents, storage, selected non-secret form state,
  active page, opener relationships, task metadata, and deterministic inputs.
- L2 adds exact network and post-checkpoint OIL replay.
- L3 live V8 heap forking remains explicitly unsupported.

Branches record strategy, parent checkpoint hash, and page mapping.
`FailureArtifact` seals a checkpoint, completed trace, and failure into one
tamper-evident regression input. CLI controls include `--trace-out`,
`--checkpoint-out`, `--restore-checkpoint`, `--replay-trace`,
`--failure-artifact-out`, and `--replay-failure`.

The frozen gate is `benchmarks/conformance/replay-manifest-v1.json`; compact
evidence is recomputed from content-addressed production artifacts.
