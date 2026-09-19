# Interest lifecycle generation audit

Source generation delivered for coordinator review. No build, formatter, tests or WPT execution has occurred for this group. This document does not close the Interest Core rows.

Pinned input: WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`, `html/semantics/interestfor/` and `shadow-dom/reference-target/tentative/interestfor.tentative.html`; tentative IDL `interfaces/interest-invokers.tentative.idl`.

## Generated source

- Native DOM single-element interestfor reflection uses traced native element references, attribute-write invalidation, exact single-ID resolution and shadow ancestor scope. Native source/target selector flags reset on Document ownership changes and do not clone.
- Layout projection supplies source, target and eligible-invoker flags. Resource lane supplies Stylo classes, delay longhands/shorthand, generated interest-button and owner anchoring; DOM query matcher reads native flags.
- InterestEvent native constructor/source getter and private payload; source follows the dispatcher's related-target retargeting and propagation suppression, without exposing a relatedTarget property.
- Hover/focus scheduling, delayed gain/loss, target-descendant retention, cancelable transitions, reverse-order noncancelable Escape loss, suppression until a new input cycle, bounded pending transitions, 25 ms retained-dispatch retry deadline, native scheduler readiness/delay, retirement reset.
- Retained InterestEvent listener dispatch resumes after infrastructure interruption without repeating completed callbacks. Native selector publication follows uncanceled event completion.
- Regression sources cover native reflection/GC/clone, exact IDs/ineligible controls, private constructor source/shadow suppression, delay/cancellation/Escape, and interrupted dispatch/nonbusy retry.

## Additional generated caller closure

- Popover transitions retain beforetoggle, recursive closes, native state, focus, and queued-toggle publication independently. Toggle wake tasks no longer own callback progress; a retained notification queue preserves exact listeners after the wake is consumed. Native scheduler deadlines cover both queues.
- Popover-owned cancelable loss dispatch preserves canceled interest while an independent hide completes. Nested interest chains retain outer sources through transitive native target relationships, including circular target layouts with a live input seed.
- Native interest-button pointerdown/hit identity marks one trusted click through a consumed private slot. Native source form/link/control default actions are bypassed for that pseudo activation only; click cancellation suppresses interest. Sticky interest survives touch movement and is released by an outside pointer press.
- A native input cycle captures its initial target. Reassignment and target reinsertion cannot trigger interest until a new hover/focus cycle. Native focus uses the deepest same-document node and the embedding-frame retarget for child Documents; synthetic focus events do not modify that state.
- Mutation-cursor readiness wakes idle actors for changes. A retained failure deadline takes precedence over mutation readiness, avoiding immediate retry loops. Native nodes remain leased through active interest and popover publication; generation/stream checks guard callback tails.
- ShadowRoot referenceTarget reflection, attachShadow options, chained resolution and mutation are native. html5ever now attaches declarative roots; navigation and parseHTMLUnsafe opt in, while DOMParser and ordinary fragment parsing do not. Parser graph imports preserve nonclonable roots; DOM importNode retains cloning rules. Declarative attachShadow reuse and registered custom-element disabled-shadow policy are connected, with parser-only handles released before snapshot.
- Pointer selection clips to the near boundary of unselectable button/ancestor content and respects computed user-select text/none/all. Resource lane supplies the computed property.

## Regression source inventory

Native regression sources now cover reflection/GC/clone, exact IDs/ineligible controls, declarative reference targets and parser flags, custom-element disabled shadow, declarative root reuse, private constructor source/shadow suppression, delay/cancellation/Escape ordering, listener interruption and retry deadlines, target reassignment/reinsertion, transitive interest chains, native pseudo activation/sticky release/default-action suppression, selection boundary clipping, retained beforetoggle/focus, and retained queued toggle delivery.

The pinned file inventory with hashes and variant declarations is recorded in `INTEREST-CORE-SOURCE-INVENTORY-2026-09-17.json`. The shared helper defines longPress, but no pinned Interest test invokes it; no new long-press behavior is claimed by this bounded dependency delivery.

## Review and execution gates

- Coordinator source review remains pending, particularly parser graph import versus DOM cloning, callback retirement/parking boundaries, native pseudo hit routing, and nested cancellation ordering.
- The outer native scheduler excludes parked realms. Retained transitions check Document generation/stream around author callbacks; no redundant parent WindowState query was added per callback.
- No build, formatter, test, browser run or conformance result is claimed. Broader Core/HTML/Web IDL gates stay open until whole-plan generation, compilation, regression repair and WPT verification complete.
