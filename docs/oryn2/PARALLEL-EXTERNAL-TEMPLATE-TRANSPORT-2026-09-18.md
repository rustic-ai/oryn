# External template standalone transport — source generation

Status: production source and regression sources generated; no build, formatter, test, generator, or service run. This is not conformance evidence.

## Scope and ownership

`crates/oryn-native/src/external_template_transport.rs` supplies portable broker/local transport to the loader owned by the automation lane. Narrow loader changes replace Unix-only context-client fields with transport custody and typed Started/Chunk responses. Parser construction, buffered publication and event handling remain the loader owner's implementation. Parent requests retain the existing typed `TemplateStreamStart` route and reply/request cleanup capabilities.

## Pinned source mapping

Inspected the nine `html/dom/partial-updates/tentative/fragment/src-*.html` sources in revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`; immutable hashes and prior cohort outcomes remain in `wpt-procedures/core-external-template-source-map-2026-09-18.json`.

| Required behavior | Transport source | Regression source |
| --- | --- | --- |
| `src-streaming`, `src-parser-blocking` | Executor admission; native operation header first; subsequent bounded body demand; no DOM/V8 on IO lanes | Separate headers/first chunk/64 KiB split/terminal read; loader lane host tests |
| `src-buffered` | Same stream; loader buffer flag preserved; bounded response custody | Loader lane atomic parser publication tests; transport boundary source |
| `src-cancel` | Exact document loader epochs plus Drop cancellation; native request namespace; before/after-start race checks; deadline | Cancellation before body and wrong response identity; loader removal/stopped document tests |
| `src-csp`, `src-nonce` | Script destination on every redirect; native nonce snapshot; parser-inserted/untrusted request; sandbox/scripts gate and native violation report queue | Accepted correct nonce, rejected wrong nonce/self cross-origin; connect-src does not replace script-src |
| `src-csp-cors` | CORS GET, same-origin credentials, exact single ACAO, no cross-origin credential escalation, redirect-tainted Origin, mixed-content and embedder-policy checks | Independent CSP/CORS success/failure combinations, redirect taint |
| `src-referrerpolicy` | Captured document policy/element override, response policy on redirects, native referrer computation | Origin-only first hop then no-referrer redirect |
| `src-sri` | Original response bytes verified before Started/body publication; no decoder before verification; 8 MiB total bound | Original non-UTF8 bytes, match/mismatch, data URL integrity |

## Authority and custody

- Broker requests use a fresh native request namespace, explicit Script cookie request kind and CORS/script Fetch Metadata. Every response must correlate with the admitted operation. Redirects cancel the previous operation before another admission.
- Sources maps permit only exact registered same-origin HTTP(S) URLs; no basename fallback or cross-origin map entry establishes response authority. Nonempty SRI rejects decoded map sources, which lack original representation bytes. Native data URL decoding retains real bytes and supports SRI.
- Parent Started request IDs and Chunk stream IDs are checked independently. Standalone IDs cannot be substituted across transport owners. A dropped loader cancels pending/active IO, including the race between cancellation and native start.
- Response header, chunk, aggregate body, request-count and deadline bounds apply. A stream remains retained until real terminal acknowledgement; errors cancel it. Existing report queues receive standalone native resource timing with observed response metadata and terminal sizes. Parent timing remains owned by its context route.

## Pending verification

All authored tests are unexecuted. Global verification must compile portable and Unix paths, exercise broker cancellation/redirect metadata and run the nine pinned WPT fixtures after the global code-generation gate opens. Historical passes/failures in the source map are unchanged.
