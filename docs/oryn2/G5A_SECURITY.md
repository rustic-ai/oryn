# G5A process-boundary security

G5A moves native page execution into OrynPageWorker.app. On macOS the public
Browser façade and oryn native fail closed unless that worker has the exact
trusted identity and sandbox profile described below. The former in-process
engine remains crate-internal to the worker; a separately compiled
in-process-probe feature exists only for named tests and measurements.

## Trust boundary

The parent owns filesystem access, environment secrets, HTTP transport, DNS
resolution, redirect approval, cookies, response decoding limits, policy, and
worker lifecycle. The worker owns V8, DOM/page state, OIL execution, semantic
references, and page traces. The only privileged child-to-parent operation is
a typed network request on an inherited Unix socket. There is no filesystem,
shell, environment, credential, or generic RPC operation.

The worker process is treated as disposable. A crash, wall-time breach, RSS
breach, malformed frame, protocol mismatch, or correlation failure kills the
worker. Its replacement starts a blank document with a new high-order document
generation; old semantic references therefore fail rather than being replayed
against reconstructed state.

## macOS launch requirements

The parent verifies all of the following before initialization:

- strict code-signature validation;
- bundle identifier ai.rustic.oryn.page-worker;
- Team ID 5HVA9VFF8K;
- leaf signing-certificate SHA-1
  9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90;
- an Apple Development trust chain and hardened-runtime flag;
- exactly two entitlements, both true:
  com.apple.security.app-sandbox and com.apple.security.cs.allow-jit;
- the signed bundle manifest, protocol version, source-tree binding, policy
  fingerprint, and production limits;
- a nonce-bound protocol-v2 handshake whose final executable hash matches the
  artifact checked by the parent.

The parent launches with an empty environment and passes only stdio plus the
inherited broker socket. The handshake is rejected if the worker can see Azure,
OpenAI, Ollama, proxy, home-directory, or DYLD variables.

Non-macOS worker execution is reported as process_isolated_unsandboxed; it is
not presented as equivalent security.

## Network and resource enforcement

Every request is resolved and checked in the parent. Approved addresses are
pinned into the Rustls client so a second DNS lookup cannot select a different
address. Redirects return to the worker and every hop is brokered again.
Loopback and private/link-local/reserved ranges are denied by default; only the
named benchmark context enables loopback. Worker-provided headers are reduced
to a fixed allowlist, cookie-setting response headers remain parent-only, and
decoded bodies are capped at 16 MiB.

Protocol frames are capped at 24 MiB, the command queue at 32, V8 heap at
256 MiB, worker RSS at 512 MiB, ordinary commands at 10 seconds, and navigation
at 30 seconds. Bounded reads reject oversized or unterminated frames without
allocating an unbounded line buffer.

## Threats covered in G5A

- page JavaScript reading application/model secrets or arbitrary user files;
- page JavaScript opening direct outbound connections or listeners;
- bypassing scheme, redirect, DNS, private-network, header, cookie, response,
  decompression, or timeout policy through the worker;
- unsigned, ad-hoc-signed, wrong-team, wrong-certificate, debug, altered, or
  over-entitled worker substitution;
- stale semantic references surviving worker replacement;
- parent loss caused by an infinite loop, worker crash, RSS breach, or IPC hang.

The signed workflow exercises direct filesystem/network/listener denials,
secret-free initialization, a successful parent-brokered loopback request, a
real V8 DOM mutation, protocol rejection cases, and crash/restart generation
invalidation.

## Deferred G5 work

G5A is not notarized distribution and does not add secret-handle APIs, the
broader policy language, long-running fuzz infrastructure, or automatic V8
update qualification. Those remain later G5 increments. Developer ID signing
and notarization are also separate release requirements.

