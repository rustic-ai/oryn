# G6 execution domains

Native Oryn is the canonical execution domain. Chromium and a user-controlled
browser are optional compatibility adapters in `oryn-domain`; neither appears
in the default `oryn` or `oryn-native` dependency graph.

## Selection

`CapabilityLedger` publishes installed domains and their support levels.
`select_execution_domain` applies an explicit user selection, allowed-domain
policy, required capabilities, known-bad origins and site challenges,
authenticated-session ownership, user-presence requirements, and consequence
policy. `DomainDecision` contains the selected domain, a stable reason code,
and every rejected candidate. Explicit selections fail closed.

`DomainOrchestrator` owns adapters and records each handoff. Its aggregates
expose domain mix, decision-reason mix, handoffs, and failures without changing
the native page API.

## Handoff boundary

A handoff is a controlled reload, not migration of a live JavaScript realm. The
parent may transfer URL, policy-approved cookies, local/session storage,
selected non-secret form values, task metadata, and checkpoint lineage.
JavaScript heap objects, pending promises, sockets, IndexedDB state, native host
objects, and secret handles do not cross. Every loss is listed; lossy handoff
requires approval, and cookie transfer uses the consequential-action policy.
The destination receives a fresh revision, so source references cannot be
reused in the adapter.

The frozen gate is `benchmarks/conformance/g6-manifest-v1.json`. Production
validation uses the signed worker, both optional adapters, dependency-purity
checks, and the complete G5 production regression.
