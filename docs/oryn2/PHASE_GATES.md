# Oryn2 Native Proof Gates

The committed implementation horizon is P0 through G2 on macOS ARM64. Gates
are evidence-based; they do not carry calendar dates.

## P0 — Reuse and baseline

- Full Rust, scanner, extension, Python, and IntentGym baselines are captured.
- Reuse ownership is complete and enforced during review.
- Benchmark records validate against the versioned schema.
- Existing execution domains accept security and build-maintenance work only.

## G0 — Architecture and measurement proof

- The V8 host decision is resolved by identical correctness and resource tests.
- Minimal DOM, HTML parsing, WebIDL, network broker, semantics, and trace spines run.
- A page worker is confined by the macOS App Sandbox and communicates through
  typed host messages.
- Startup, page creation, RSS, concurrency, and installed/compressed size are recorded.

## G1 — Executable web document

- Native navigation, scripts/modules, event-loop ordering, DOM/events, forms,
  fetch, URL, encoding, compression, cookies, and lifecycle behavior pass the
  advertised conformance manifest.
- Representative SSR and React TodoMVC fixtures operate without scanner, CDP,
  WebDriver, Chromium, or WebKit.
- Unsupported APIs produce truthful capability diagnostics.

## G2 — Agent-native semantic loop

- Existing OIL drives native observations and core actions.
- Numeric OIL aliases resolve to generation-checked semantic references.
- Compact/full/scoped/delta observations and action effects are versioned.
- All controlled tasks pass, the 30-task corpus is published, and zero confirmed
  false reference preservations occur for consequential actions.

The authoritative thresholds and corpus composition are maintained in the
benchmark manifest introduced during P0.
