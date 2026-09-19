# Body read-all-bytes custody — source review, 2026-09-16

Source generation only. No tests, builds, services, formatters or generators run;
Core/family qualification remains open.

## Pinned requirement evidence

WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`:
`fetch/api/response/response-stream-bad-chunk.any.js` requires rejection for
non-Uint8Array chunks across Body methods; `response-stream-disturbed-1/2/4/6.any.js`
checks usable/locked/disturbed/canceled bodies. `response-body-read-task-handling.html`
and `response-stream-with-broken-then.any.js` exercise native promise/author callback
boundaries. The native Body read-all-bytes operation retains its acquired lock;
it must not apply the explicit reader-release behavior of service response readers.
Invalid chunk rejection does not invent an underlying author cancellation.

## Generated implementation and consumers

- `response_body_operation_bounded` captures traced source identity before locking
  and registers exact source/result retirement relationships.
- `pump_response_body` delegates custody to `body_consumer::pump`, retaining native
  resolver/source locals while its inner implementation takes the consumer.
  Native allocation/conversion/pull failures reject the returned promise, release
  native HTTP custody, and attempt cleanup rather than losing the resolver via `?`.
- Pumping enters the source creation context; completed values still allocate in
  the result owner context. Retired result owners are rejected before conversion.
- Retirement runs before `stream_graph::retire`. Source retirement queues rejection
  in a surviving foreign result realm; result retirement cancels only the source
  consumer that still names that exact result owner. Native HTTP cleanup starts
  immediately; author cancellation runs at the surviving source checkpoint.
- New deferred events use traced `Owned` handles and optional lookup. Temporary
  locals preserve in-call custody; no permanent Global roots were added to pure-JS
  Body consumers. `stream_graph` traces the source wrapper alongside its resolver,
  preserving collection of unreachable JS-only cycles.

## Regression sources

`body_consumer::tests` covers success with held lock/no author cancel; invalid chunk
rejection without cancel and repeated-use rejection; injected queue-drain failure
after taking the consumer with returned promise settlement; foreign source
retirement/result rejection; and result retirement/source cancellation custody.

## Still unqualified

Tests and GC behavior have not run. Existing native transport/agent cleanup still
owns external completion acknowledgment. This bounded fix does not claim complete
Fetch/Streams conformance or whole L-package source closure.
