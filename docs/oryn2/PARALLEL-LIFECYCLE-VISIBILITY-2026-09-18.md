# Native lifecycle visibility — source generation

No builds, tests, formatters, generators or browser runs executed.

The pinned Core fixture `html/webappapis/dynamic-markup-insertion/opening-the-input-stream/ignore-opens-during-unload.window.js` has SHA256 `e9d52e144781e810709279b36d9d663904b4834fd22c05a9b8d8091a986f359f`. Exact frozen inventory membership is recorded in `wpt-procedures/core-lifecycle-visibility-2026-09-18.json`. It checks beforeunload/pagehide/unload/**visibilitychange** for selected frame, parent-and-child departure and child-only departure. A missing visibility producer leaves an actual required test pending; generic pagehide/unload dispatch cannot satisfy it.

## Generated behavior

- NativePlatform now keeps backend hidden, lifecycle hidden and published effective hidden separately. Document getters, rendering visibility, permission controls and location delivery compose backend/lifecycle/prerender state.
- `NativePublicationEvent::Page` owns the complete per-document sequence in one retained publication batch: pagehide → visibilitychange to hidden; clearing lifecycle visibility → pageshow. Backend minimized/prerender state can suppress an unchanged visibility event. Pageshow samples current backend state and never blindly makes a minimized document visible.
- Visibility changes target Document, bubble to Window, are trusted, and commit state before the first callback. The same Notification retains snapshot/event/listener/phase custody across failure or suspension. No-change publications still commit native state without emitting a duplicate event.
- Existing `NativePageTransition` retains original root/recipient/ancestor identity through the combined receipt. Navigation's native unload-tree guards enclose pagehide, visibilitychange and unload. The visibility addition does not widen the affected document subtree or install global ignore-opens state.
- Prerender initialization silently establishes hidden state. Native activation retains a visibility notification after clearing prerender state and before prerenderingchange/reveal; minimized activation remains hidden. Its event suffix and history publication gate survive interruption.
- The old synchronous page-transition binding now transfers its accepted batch into the same retained native queue and removes orphan receipt bookkeeping. Retained host transition callers continue to own their explicit receipt.

## Regression sources and qualification

JS native navigation regressions cover a failure after pagehide, hidden state before visibility callbacks, trusted Document target, document.open suppression under the unload guard, subsequent unload order, pageshow visible ordering, minimized suppression, prerender suppression and suspension before replay. A prerender activation regression injects failure after the first visibility listener and verifies the remaining listener and prerenderingchange occur once before reveal. Navigation owns the native parent/child guard matrix. Native execution and pinned WPT results remain unverified until the global source-generation barrier ends.
