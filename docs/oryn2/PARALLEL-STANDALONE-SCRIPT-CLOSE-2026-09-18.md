# Standalone Window close source closure — 2026-09-18

Production and regression source generation only. No build, test, service, formatter or WPT execution occurred.

## Native route

`standalone_script_close_v8.rs` supplies the missing no-parent-endpoint branch of `native_window_close_callback`. Admission requires an active top-level target, either native web-created auxiliary membership or full native session history with one entry, and a live native incumbent related through the target's native group/opener tree. It validates every native frame ancestry edge and destination sandbox top-navigation flags. It does not infer authority from author `window.opener`, relaxed `document.domain`, an origin string, or sharing an isolate. Nested targets are rejected by the existing common callback guard.

Standalone `window.open` now creates native auxiliary roots and drives their navigation through the shared native agent. See `PARALLEL-STANDALONE-AUXILIARY-2026-09-18.md`. Only those actual native web-created roots bypass the multiple-history-entry restriction; ordinary embedder roots retain it. Parent-backed auxiliary creation continues to use its existing parent route.

Admission sets the native logical `closed` flag synchronously and reserves a bounded task keyed by target root, with a retained lifecycle owner. Repeated close calls are idempotent. The next native task pump selects the current target document independently of the admitting source, invokes `prepare_native_script_close` and retains `NativeScriptClose` across pending/error retries. Beforeunload cancellation leaves the document live and logical `closed` true, matching the existing parent route.

`prepare_native_script_close` and `poll_native_script_close` are now portable. Their existing native beforeunload tree, one-prompt policy, descendant unload guards, pagehide/unload ordering and current-document reselection after beforeunload permission are shared unchanged. The parent proof API and public parent ClosePage route retain their own authority and transport.

On successful departure, the standalone task retires the root and descendant native execution owners, timers and response/observer custody through their existing retirement paths. Inert WindowProxy shells remain available to existing references. No PageId, context client or fake browser process is fabricated.

## Native scheduling and source matrix

Pending close tasks participate in `has_pending_page_work`, `next_timer_delay`, `drain_scheduled_timers` and `drain_async_fetches`. They are not dispatched by the callback or inside a microtask checkpoint. Parked targets retain pending tasks until the native target can resume.

Authored regression sources cover:

- Synchronous logical closed, microtask-before-departure ordering and idempotence.
- Full-history rejection including a same-document pushState entry; nested Window rejection.
- Destination sandbox admission for a descendant caller.
- Beforeunload dismissal retaining live DOM and logical closed without pagehide/unload.
- Replacement-document selection before the queued task begins, independently of the retired admitting document.
- Retained departure resumption after beforeunload permission, selecting a replacement document and retiring its descendants.

All tests are available under the V8 feature across supported native platforms. These are source assertions awaiting the global generation-complete build/test/fix phase; no platform qualification is claimed. The completion plan requires evidence per claimed platform and execution policy on the same frozen candidate.
