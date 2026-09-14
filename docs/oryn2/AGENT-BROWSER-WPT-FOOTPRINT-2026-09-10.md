# Agent-browser WPT footprint — 2026-09-10

## Decision

Keep `runtime-api-style-semantics-v2` immutable as the historical 3,184-file,
4,022-execution conformance ledger. Add `agent-browser-conformance-v1` as the
broader browser contract instead of spending qualification time running a
competitor binary through Oryn's release harness.

The additive inventory uses the same pinned WPT revision and source-hash,
global, variant, timeout, crash and unsupported-feature rules. It expands to
**12,708 files and 16,842 executions**:

| Workstream | Files |
|---|---:|
| Browser core | 6,399 |
| Platform relevant | 4,659 |
| Agent interaction | 1,370 |
| Structured visual | 213 |
| Semantic accessibility | 67 |

This is an inventory result, not a measured conformance pass rate. No result
from the 4,022-execution ledger is projected onto the new files.

## Published-footprint comparison

The comparison profiles are machine-readable in
`benchmarks/conformance/agent-browser-conformance-v1.json` and reproducible with
`scripts/summarize-conformance-footprint.py`.

| Published footprint mapped into Oryn's pinned WPT revision | Files | Executions |
|---|---:|---:|
| Obscura Core | 5,690 | 7,735 |
| Obscura cumulative Core + Relevant | 10,678 | 14,069 |
| Kitesurf published areas | 5,528 | 7,480 |
| Oryn agent-critical union | 12,708 | 16,842 |

These are source-footprint counts, not competitor results. Obscura reports
subtests and Kitesurf reports passing subtests; Oryn's current headline reports
whole executions. A later Oryn run must publish both units.

## What the expansion adds

The existing ledger is deep in selected DOM, CSS, editing, IndexedDB, history,
referrer and accessibility behavior. It does not measure enough of the browser
surface used in Obscura's or Kitesurf's published coverage. The additive suite
therefore adds:

- broad HTML rather than history alone;
- URL and URLPattern;
- Encoding and encoding detection;
- Fetch, XHR, CORS, EventSource and WebSockets;
- DOM parsing, XPath, Web Storage, custom elements and shadow DOM;
- Streams, File API, workers, WebAssembly and WebCrypto;
- CSP, SRI, secure contexts, cookies, storage and permissions policy;
- import maps, navigation, page visibility and scheduling;
- Selection, UI/input/pointer events, observers and clipboard behavior;
- structural SVG, excluding SVG paint, animation and rendering suites;
- HTML accessibility mappings in addition to Accessible Name and WAI-ARIA.

Canvas, HTML rendering, media-element, SVG paint/animation/rendering, manual
tests and pixel references remain explicit exclusions from this runtime
conformance profile. They belong to the separate renderer and execution-domain
contracts.

## Reading the competitor reports

Obscura's 2026-07-03 report publishes 318,916/382,891 Core subtests (83.3%),
420,319/494,422 cumulative Core + Relevant subtests (85.0%), and
503,413/839,489 Full subtests (60.0%). Its headline profiles explicitly exclude
all CSS and editing along with layout/rendering, interaction and media areas.
The raw run products are gitignored and the report supplies no per-root or
whole-execution results.

Kitesurf's current documentation reports more than 235,000 passing subtests and
DOM 97%, HTML 96%, Selection 99%, SVG 97%, Encoding 99%, CORS 95%, XHR 95% and
URL 83%. It does not publish the WPT revision, selected inventory, total
denominator, per-execution result, timeout policy or raw ledger. Those figures
are directional until their denominator and inventory are available.

## Measurement order

1. Preserve the 4,022-execution ledger as a regression gate.
2. Run the new inventory by workstream, starting with `browser_core`.
3. Publish whole-execution and subtest totals, plus harness/runtime/timeout
   failures, for every workstream. The release-result validator now computes
   both observed/passing subtests and executions that produced no subtests.
4. Treat module-worker globals as required inventory. They remain failures until
   the production harness executes them rather than silently dropping them.
5. Establish a complete diagnostic baseline before deciding which parts become
   release-blocking. Do not require 16,842/16,842 merely because the inventory
   now exists.
