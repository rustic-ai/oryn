# Oryn2 Reuse Ownership

This inventory is the implementation gate for Oryn2. A new implementation is
allowed only when the capability is marked **New** or an ADR records why the
existing implementation cannot be adapted safely.

| Capability | Existing owner | Oryn2 disposition |
| --- | --- | --- |
| OIL grammar, normalization, AST, parsing | `oryn-core` | Reuse and evolve; preserve common OIL syntax |
| Target matching and recovery | `oryn-common`, `oryn-core::resolution` | Reuse algorithms and tests; change input to native semantic views |
| Action and observation vocabulary | `oryn-common::protocol` | Migrate to versioned typed contracts; retain semantic meaning |
| Command execution and formatting | `oryn-engine` | Adapt into the OIL frontend over typed page operations |
| Intent definitions, flows, registry, packs | `oryn-engine::intent`, `oryn-engine::pack` | Reuse; do not rebuild |
| Learning hooks | `oryn-engine::learner` | Preserve behind explicit telemetry policy; do not make browser correctness depend on it |
| Chromium execution | `oryn-h` | Security-maintained adapter and differential oracle |
| WPE WebKit execution | `oryn-e` | Security-maintained adapter |
| User-browser execution | `oryn-r`, `extension` | Security-maintained adapter |
| WASM extension and local agent | `oryn-core::wasm`, `extension-w` | Preserve; adapt only when v2 contract requires it |
| Python SDK | `oryn-python` | Migrate transport/types; preserve OIL entry point |
| Agent evaluation | `intentgym` | Extend with native domain and versioned benchmark records |
| Deterministic web scenarios | `test-harness` | Reuse as the first native task corpus |
| Native JS runtime and event loop | None | **New** |
| Rust-owned DOM and WebIDL bindings | None | **New** |
| Native HTML parser integration | None | **New**, using `html5ever` |
| Native network broker, cookies, storage | None | **New** |
| Native semantic projection and stable references | Scanner is a behavioral reference only | **New**, while reusing resolution semantics |
| Native causal trace and replay envelope | Partial scanner action deltas only | **New** |
| Native style/layout/rendering | None | **New** |

## Change rule

1. Search this repository before adding a type, protocol, parser, resolver, or fixture.
2. Prefer an adapter around existing behavior over parallel implementations.
3. Put compatibility-only behavior behind an execution-domain adapter; do not
   constrain native telemetry to the scanner protocol.
4. Preserve existing golden tests when a public surface is intentionally changed;
   add a v2 expectation next to the old evidence or document its migration.
