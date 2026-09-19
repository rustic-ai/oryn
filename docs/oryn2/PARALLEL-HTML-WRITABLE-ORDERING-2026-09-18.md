# HTML Writable script ordering — source disposition

Source generated and reviewed only. No builds, tests, formatters, generators, service launches or WPT execution.

## Frozen callers

`wpt-procedures/core-html-writable-ordering-2026-09-18.json` records exact SHA256 and frozen Core inventory membership for four pinned fixtures at `719d5e38fdd0903a18ed9007aba816c98cc491e0`: `domparsing/tentative/stream-append-html-unsafe.html` and partial-updates `stream-append-with-template-for`, `stream-append-has-invalidation`, and `stream-append-positional-pseudo-class-invalidation`. All four remain Core. The DOM parsing fixture asserts connected execution after the script closing token, disconnected inertness, and no unclosed-script execution on abort. These fixtures establish actual runScripts consumers; they do not independently assert delayed external fetch ordering. That uncovered production continuation gap receives dedicated regression sources rather than an invented existing WPT result.

## Producer and consumer closure

- `html_stream.rs::Sink` now retains one write's converted typed input suffix, parser, event frontier, preparation receipt and blocking completion. Repeated pumping never converts the request again. Pending script preparation stays with the same owner and queue request; successful script preparation is not recaptured. Terminal conversion/parser errors reject and release the sink, preserving already committed DOM effects.
- `post_connection_scripts::capture_stream_event` returns the existing staged preparation receipt directly. Resource stages and script admission run before the event advances.
- For a native `ScriptReady::Blocking` event, an accepted external `PreparedDynamicScript` owns a completion guard. The write remains pending with its unparsed suffix until that preparation is retired by the native scheduler. Inline scripts execute synchronously; Async, Defer, Module and Data scheduling do not acquire this blocking guard.
- `rust_executable.rs::dispatch_ready_dynamic_scripts` retains the immutable preparation across fetch completion, execution interruption and load/error event publication. It removes the preparation only after `finish_dynamic_script` returns true. Script reporting and retained completion publication can return false; parked jobs remain owned. The guard follows that existing terminal custody instead of treating fetch/graph admission as completed script execution. This change makes no new module/TLA completion claim: module events do not block this parser boundary.
- `writable_streams::pump` preserves the same queue request/promise when the native sink reports pending. No undefined-result observation occurs until parsing completes. The platform completion checkpoint resumes runnable streams; waiting on a script does not create immediate runnable work. Abort releases parser/input/continuation roots and never emits EOF. Owner retirement, stop and document/target authority changes reject further publication; suspension parks the request.

## Regression sources

- JS `suspension_inside_conversion_or_inline_script_parks_without_replay`: native suspension invoked from author conversion or inline script retains parser/frontier and pending promise, restores without conversion/script replay. Final lifecycle checkpoint has a retained completion flag.
- JS `blocking_external_script_keeps_write_suffix_and_conversion_until_terminal_custody`: queue transfer, repeated resume, single conversion, suffix and promise blocked, parked completion, resumed inline suffix once.
- JS `async_external_script_does_not_hold_html_stream_suffix`: asynchronous scripts retain native jobs without blocking the stream.
- JS `abort_during_blocking_script_releases_input_and_never_publishes_suffix`: abort while native script owns completion releases input and prevents later suffix publication.
- Native `html_writable_blocking_suffix_waits_for_native_execution_and_load_publication`: real writer → native preparation → scheduler dispatch; injected failure after body execution retains write, suffix and completion event; recovery publishes load and suffix without repeating body/conversion.
- Existing split-token, incomplete entity/DOMString, EOF, disconnected/inert, suspended queue and quota regression sources remain in place.

## Remaining qualification

All listed source is unexecuted. Build/type reconciliation and execution of positive, fault, abort, native lifecycle and pinned caller cases remain behind the global source-generation barrier. This closes the confirmed HTML Writable blocking-script source gap; it does not close other G helper dependencies or claim WPT conformance.
