# Oryn2 security security policy

This document is the release policy for the native autonomous runtime. It
extends the sandbox worker process boundary; the older sandbox worker document remains a historical
description of that increment.

## Threat model

Oryn treats page content and JavaScript as hostile. The worker receives no host
filesystem, environment, credential, listener, or direct socket authority.
The parent verifies the signed worker, owns all network and persistent storage
authority, enforces CPU/memory/IPC limits, and replaces a failed worker with a
new document generation. Web content remains structurally untrusted and never
enters the host instruction channel.

## Action policy

Every context owns an explicit policy for cross-origin navigation, submission,
popups, downloads, credential fill, protected requests, and user-browser
delegation. Decisions are allow, deny, or require-confirmation. Confirmations
are random, exact-request-scoped, one-use tokens. Every decision is appended to
a structured context audit record independently of the model. File-input
upload accepts only parent-imported opaque handles and transfers only the
selected bytes into a page worker after policy authorization. Clipboard access
is a text-only, context-owned virtual clipboard; Oryn never reads or writes the
host system clipboard.

Network policy includes scheme, resolved-address, redirect-hop, hostname and
port controls. Host allow/deny patterns match exact hosts and subdomains; DNS
results remain pinned into the authorized transport.

## Secret handles

Applications import credentials into parent-owned memory and receive an opaque,
unguessable handle. Handles are bound to one serialized origin, are explicitly
revocable, redact their debug form, and require a confirmed credential-fill
operation before materialization. Secret memory is zeroized on removal. Native
semantic observations replace password values with `[REDACTED]`; trace and
policy records contain labels and decisions, never secret bytes.

Standalone CLI, Python, and MCP sessions provision labelled values through an
inherited descriptor consumed by the parent. Secret values never enter argv,
the environment, JSON-RPC, OIL, worker initialization, observations, or traces.

## Download quarantine

Page downloads enter a bounded parent-owned virtual quarantine. Page-provided
filenames are reduced to a basename and cannot select a host path. The store
records origin, URL, media type, size and SHA-256. No API implicitly writes a
download to disk; a trusted host must explicitly release the bytes and choose
their destination.

## Fuzzing

The repository contains libFuzzer targets for html5ever/Oryn tree construction,
DOM mutations and stale identities, URL/network policy, and bounded worker
protocol decoding. Release qualification compiles every target and consumes
the maintained seed/regression corpora. Continuous private infrastructure runs
the same targets for longer budgets and retains every crashing input.

## V8 update train

`security/v8-update-policy.json` pins the Rust V8 crate version and checksum,
the expected runtime line, qualification commands, and response SLA. Critical
security updates are qualified within 24 hours and high-severity updates within
72 hours. Any V8 change must pass the signed sandbox, limit/restart, executable document-layout,
dependency-audit and fuzz gates before release.

The dependency audit must report zero vulnerabilities. Informational
unmaintained or yanked warnings are accepted only when they exactly match
`security/dependency-exceptions.json`, have a documented transitive scope and
remediation, and have not passed their mandatory review date.

`scripts/qualify-v8-update.sh` requires the candidate crate version and
checksum to match the reviewed policy, rejects a dirty tree, and then executes
the complete signed security production gate. The update is not qualified merely by
compiling or passing unit tests.

## Release gate

security closes only for an exact clean source tree and signed worker when every case
in `benchmarks/conformance/security-manifest-v1.json` has raw evidence, all prior gates
regress cleanly, the advisory audit reports zero unmitigated critical/high
findings, fuzz smoke and retained regression corpora pass, the worker denial and
limit suites pass, and the evidence validator recomputes the committed result.
Missing tools or unavailable advisory data fail the release gate; they are not
converted into skips.
