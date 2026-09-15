# Broker routing threat model and staged cutover

This is a code-review record, not independent security attestation or release
qualification. It supplements R6 in the browser implementation readiness plan.

## Trust boundary

Treat every worker frame as untrusted, even after executable authentication.
Only the parent issues and binds authority. Page IDs, request IDs, session IDs,
and document generation numbers alone do not confer authority. A worker's
inherited sockets remain tied to its parent-side router; no request selects an
arbitrary process or browser context.

The staged v10 transport binds the legacy root-page execution binding using
`WorkerRoute`: session, root environment/page ID, initial generation, and a random
256-bit grant. Each binding receives fresh entropy. Grants are private native
transport state, redacted from Debug output, and not installed into page globals.
JSON serialization is necessary for the inherited transport, not a diagnostic API.
Environment/page zero is the existing prewarm placeholder. It also receives fresh
authority and is reset before binding a user page; zero is not a wildcard.

## Controls implemented in this stage

| Attack or race | Enforcement |
| --- | --- |
| Guess a page ID or copy public identity fields | Exact bound credential match, including the independently issued grant |
| Delay an old context/network frame until after reset/rebind | Immutable credentials on scoped clients; the parent rejects the old credential against the replacement binding |
| Omit authority | Production router rejects unrouted context/network dispatch; diagnostic direct transports remain separate |
| Change route on a live command stream | Worker validates active route and session; parent validates echoed route and correlation |
| Forge a completion with the correct numeric correlation | Context/network pending entries record and validate expected route |
| Cancel an unrelated route's numerically identical request ID | Client correlation state is keyed by route plus request ID; parent cancellation requires the bound route and an owned request |
| Retain an admitted parent operation across reset | Existing admission/revocation lock, bounded drain, cancellation, second cleanup, and permanent retirement on failed drain |
| Exhaust parent threads or reuse exhausted IDs | Parent context-dispatch bound and checked monotonic identifiers remain enforced |

Wire versions move together: worker 10, context service 11, network broker 5.
These are distinct protocol version spaces. Current build/validation entrypoints
require worker v10. Historical evidence files are not rewritten; their original
source commits retain their validators and protocol implementations.

## Scope and gates that remain open

This stage deliberately retains one active root-page binding per worker. It does
not implement the entire R6 package, and it must not activate connected-window
placement. In particular:

- The generation in this credential is the binding's initial generation. It is
  not yet a freshly parent-issued credential for every document navigation,
  iframe environment, or dedicated-worker environment.
- Per-document retirement still uses native environment/lifecycle checks. A full
  navigation cutover must rotate broker authority and reject delayed pre-commit
  requests without resolving them against replacement-document policy.
- A WorkerProcessSession must own correlation/dispatch, process budgets and
  failure fan-out independently of page handles before multiple environments
  share the process through the public API.
- A multi-environment route table must validate partition/agent ownership and
  enforce bounded environment quotas, not simply accept any credential known to
  the worker process.
- Signed stale/forged request controls, reset churn, navigation cancellation,
  cross-partition denial, callback deadlock controls and Linux compilation are
  required before closing R6.

The new socket-level regressions cover immutable context authority after rebind,
forged response authority, route-scoped network cancellation with colliding IDs,
and missing/forged worker command routes. Tests do not stand in for the remaining
document/agent ownership and process-session changes.
