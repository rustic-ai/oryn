# Signed worker lifecycle

Oryn2 worker protocol v9 supports three explicit lifecycle modes. The default,
`IsolatedOnDemand`, starts one signed process per page and never reuses it.
`IsolatedPrewarmed { capacity }` prepares a bounded set of sterile workers and
assigns each worker to one user page. `Recycled { capacity, max_leases }` is a
reduced-isolation opt-in which resets and rebinds a process after a page is
explicitly closed.

Pool capacity must be 1–16. Recycled lease limits must be 1–1024. The CLI
requires the corresponding values explicitly:

```text
oryn native --worker-mode isolated-prewarmed --worker-pool-size 4
oryn native --worker-mode recycled --worker-pool-size 4 --worker-max-leases 32
```

`Browser::prepare_workers()` performs deterministic preparation. CLI and MCP
entry points call it before creating their first page. A prewarmed worker is
reset while it has only a sterile bootstrap page; an isolated-prewarmed worker
is destroyed after its assigned user page closes. A recycled worker remains
unbound between `ResetReady` and its next authenticated `Initialize` exchange.
The unbound broker rejects network, context, storage, secret, upload and
clipboard requests.

For high-volume automation, `PageHandle::execute_batch` sends a bounded list of
non-submit actions in one authenticated IPC frame. Submit actions remain
individually authorized and cannot use this path.

On macOS, the parent validates the signed bundle through Security.framework,
including its designated identity, certificate and required/prohibited
entitlements. It authenticates the launched PID and compares the kernel Code
Directory hash and executable SHA-256 reported in the v9 handshake. Verification
is single-flight cached by canonical path, device/inode, size, mtime, ctime and
signed-manifest hash. Runtime RSS enforcement uses `proc_pidinfo` and fails
closed when measurement fails.

The production benchmark refuses to publish results from a dirty tree unless
`ORYN_ALLOW_DIRTY_BENCHMARK=1` is set for a non-publishable smoke run. Its v5
result separates cold, cached-isolated, prewarmed, recycled, reset, warm-action,
1/5/20-page concurrency and RSS measurements.
