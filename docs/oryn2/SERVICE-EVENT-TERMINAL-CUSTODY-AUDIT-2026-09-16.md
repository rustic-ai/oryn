# Service event terminal custody — source review, 2026-09-16

Status: generated source and regression sources only. No builds, tests, services,
formatters, generators or qualification were run. Whole-plan acceptance remains open.

## Confirmed gap and generated callers

`v8_bindings/service_worker_events.rs` previously removed cancellation state or
marked a streaming response done before fallible cleanup. A failed preload abort
or response-reader cancellation skipped `handled` rejection and the only terminal
AgentEvent. Normal completed-event preload cancellation had the same removal gap.

- `cleanup_attempt!` contains each native error / JavaScript exception independently.
- `cancel_body` attempts stream closure, closed-reader settlement, tee or transform
  cancellation, native HTTP abort, and author source cancellation without early return
  after acquiring the source. HTTP admission failures retain the existing exact
  `abandon_http_stream` cleanup path. It reports aggregate failure after all attempts.
- `cancel` allocates its abort reason before removing pending state, attempts all
  resources and request-signal abort independently, rejects handled, and publishes
  the original cancellation outcome. Duplicate cancellation emits no second receipt.
- `finish_stream` allocates before committing response_done, tolerates cleanup
  failure, settles handled, publishes the original response outcome, and guards
  duplicate completion.
- Normal lifecycle completion reports preload cleanup failure as terminal error.
- `poll` retains already committed output in `Events.ready` if a later event fails.
  The next successful poll delivers those receipts once. Outstanding receipts count
  against new event admission; they cannot be hidden by accepting unlimited work.

## Regression source inventory

`terminal_cleanup_tests` covers absent preload authority plus invalid stream
controller, independent reader release, handled rejection, response-end/lifecycle
publication, original error preservation, duplicate suppression, normal lifecycle
cleanup failure, and a later poll error retaining an earlier committed receipt.
These are deliberate native fault injections, not conformance pass evidence.

## Remaining limits

Delivery outside the binding poll return is owned by the existing agent transport.
A failed poll retains receipts but does not fabricate transport acknowledgment.
Source cancellation promises may themselves reject asynchronously; this change
preserves the original service outcome while still initiating every cleanup path.
Runtime behavior and full suite qualification remain unverified until the global
generation gate permits execution.
