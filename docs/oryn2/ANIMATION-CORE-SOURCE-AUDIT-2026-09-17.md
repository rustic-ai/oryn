# Required Core animation source audit — September 17

Status: **mapped caller source generation and combined source review complete; no build or test execution**. This audit does not claim the whole Web Animations or CSS Animations suites pass.

Pinned source revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`. The accompanying JSON records literal JS source matches, an additional CSS/event source scan (34 policy-matched files, including comments and CSSOM fixtures), and explicit transitive helpers. These source files are not additional runnable Core variants and do not change the fixed 16,737 denominator.

## Required production routes and generated sources

| Caller | Required route | Source coverage |
|---|---|---|
| `interestfor-css-properties.tentative.html` → `css/support/interpolation-testcommon.js` | CSS animations, CSS transitions, Element.animate, pause/currentTime, effect/keyframe easing and extrapolation, property-specific native interpolation | `web_animations.rs`, `animation_timing.rs`, native provider and Stylo animation bridge; native interest-delay sampling sources |
| Customizable-select picker-icon/checkmark animation files | Native pseudo targets, finite/infinite duration, negative delay, persistent CSS identities while rendering, reopen restart | JS effect targets/timing, persistent native eager/lazy-pseudo sampler and picker render-eligibility source regressions |
| `custom-elements/reactions/Animation.html` | `commitStyles()` publishes one style attribute mutation and completes CE reactions | Native sampled-value projection, single merged style publication and scoped CE reaction boundary |
| `dom/nodes/moveBefore/css-animation-commit-styles.html`, transition counterpart | CSS animationstart, retained target identity through move, getAnimations and commitStyles | Retained CSS phase observations/events, native mirror identity, CSS controls and sampled projection |
| `css/cssom/base-uri.html` | KeyframeEffect constructor/setKeyframes, Animation.play/ready/cancel, CSS URL resolution against document base | Native constructor/options/keyframes, ready task completion, canceled sample removal; native property parser uses document URL extra data |
| `css/css-cascade/revert-val-008/009/010.html` | Interior/final `revert` frames and native cascade values | Stylo keyframe cascade and native effect sampler |
| Popover utilities and scrolling helpers | subtree getAnimations().finish, short playback, promise and event completion | JS/native animation records, subtree enumeration, terminal sample, stable actor deadline |
| `dom/events/EventListener-invoke-legacy.html` and prefixed animation event helper | Trusted per-current-target fallback, independent prefixed/unprefixed handlers, temporary event.type and restoration; synthetic dispatch does not fallback | Retained Notification and legacy node/global invocation keys, native event payloads and type guard; dedicated source regression |
| WebIDL setter-argument | Animation.id conversion and receiver branding | Native callback conversion and regression source |

## Generated ownership and regression contracts

- Element.animate, Animation, KeyframeEffect and DocumentTimeline use native records, finite/auto/infinite timing, fill/direction/iterations, cubic/step/piecewise easing, property-indexed and iterable keyframes, specified versus computed offsets, and native ready/finished promises.
- CSSAnimation/CSSTransition wrappers expose native identity, effect timing, animationName/transitionProperty and controls from the retained provider snapshot. Finished no-fill effects do not remain in public getAnimations merely because their event observation is retained.
- Native target references are weak. Private V8 edges preserve reachable target/effect/timeline/promise graphs. Cross-arena and same-arena adoption remap target identities; destination providers receive weakly registered foreign effects with collision-free sample IDs while Animation identity and timeline remain in their original realm.
- Rendering snapshots and rAF callbacks share one timestamp. Paused or zero-rate playback does not request perpetual ticks. Dirty JS samples and unobserved CSS starts/phase changes/removals each request one publication, retaining readiness on failure. Inert Documents have distinct inactive timelines. Queued events have immediate readiness and can drain while rendering is hidden.
- Promise settlement owns resolver/value records and runs after all native animation borrows have been released; authored `then` getters may reenter native animation APIs. Regression source covers this boundary.
- Event batches retain exact callback progress through preparation failures and termination. Reentrant events wait for the next admitted batch. Native CSS observations advance only after all event payloads have been prepared.
- Regression source covers native object/promise/pseudo sampling, options-before-keyframes and IteratorClose, keyframe sequences/offsets, readonly event payloads, timing boundaries, trusted fallback, adoption, timestamp equality, hidden delivery, retry/termination and reentrant dispatch. Native renderer regressions cover actual property sampling, persistent CSS and pseudo timelines, query-to-paint coherence and picker reopen.

## Explicit remaining source review and execution gates

- The additional CSS/event scan was reconciled with native routes: cross-document moveBefore fixtures require rejection, cross-shadow fixtures require preserved transition identity, and changed-selector moves require starting a transition. Native/binding preflight now rejects document changes before mutation; renderer regression sources cover the shadow and selector cases.
- Paused keyframe stylesheet changes now propagate Stylo ForCSSRuleChanges, updating keyframe layer precedence without restarting animation identity. The native CSS name list is reconciled in reverse matching order so duplicate names retain distinct records and later animations are not skipped. Native layer replacement, revert/revert-layer and multiple-animation regression sources are generated. Paused delay edits and pause conversion guard zero-duration division; a native zero-duration delay-change regression source covers before/after sampling. Stylo name registration uses cascade layer IDs and intentionally does not constrain @keyframes by @scope.
- CSSOM rule fixtures continue through existing typed stylesheet mutation routes; animations now consume their invalidation signal. Their full CSSOM qualification remains owned by the broader style package.
- CSS effect mutation overrides beyond the required control/currentTime/commitStyles callers, replacement/removal ordering across multiple effects, complete pseudo-selector grammar and scroll/view timelines are not certified by this bounded source audit. Any requirement found in the fixed Core closure or five-family callers remains implementation work, not an execution-only label.
- Run builds, generated regressions, full Core WPT and five-family qualification only after global source generation completes. No execution result is reported here.

## Remaining-surface caller reconciliation

A second read-only search covered every policy-Core HTML/JS/XHTML source and 64 existing source files reached through literal script, static import, importScripts and META script references from this inventory. It found:

| Surface | Actual caller evidence | Source disposition |
|---|---|---|
| CSS effect keyframe/timing overrides | No CSSAnimation/CSSTransition effect mutation caller in this closure. `base-uri.html` mutates a constructed KeyframeEffect; `revert-val-010.html` mutates an Element.animate effect. | Existing script-effect replacement is integrated; added native direct revert replacement regression. Full CSS override matrices are outside the observed caller scope. |
| Multiple script effects | Interpolation helper supports per-keyframe composition; effects must consume preceding samples. | Native sampler composes underlying maps; contributors now sort by process-wide creation identity after adoption. Added stacked additive effects, replacement and cancellation regression source. Automatic finished-effect removal/replaceState has no observed caller here. |
| Pseudo grammar | Four customizable-select callers pass exactly `::picker-icon` or `::checkmark`. | Both use native pseudo parsing/sampling. No invalid-pseudo grammar matrix is imported by these callers. |
| Scroll/view timeline and iteration accumulation | No ScrollTimeline, ViewTimeline, iterationComposite, updateTiming, persist or animation replaceState caller found. | No added dependency from this closure; wider Necessary suites remain unqualified. |
| Five-family source fixtures | Benchmark JS/HTML search, including archived framework bundles, found only React synthetic event `pseudoElement` payload declarations. | Native AnimationEvent/TransitionEvent already supply these readonly payloads; no new animation option/timeline dependency found. |

This is a concrete literal-source boundary, not a claim that arbitrary computed member access or the entire Necessary animation suite was exhaustively proven. Any subsequently identified Core or workflow dependency remains required implementation work.

## Combined host and rendering source review

- Host readiness includes live document-tree playback/CSS/rAF deadlines and skips parked realms. PageState runs the rendering phase before comparing provider animation revision with the cached layout revision.
- Fixed all initial, replacement-frame and forked LiveLayoutProvider constructors to initialize their retained style engine, clock and revision. A native source regression checks independent clocks/revisions after provider fork.
- A frame callback or its microtasks can mutate paused effects after the initial tick. The callback loop now publishes again before returning, under the same rendering clock.
- A child callback can mutate a parent effect after the parent's callback loop. The host now performs a callback-free final sample pass over current live frame membership. The executable repeats that pass after its page-wide checkpoint; original root identity, parked/retired state and frame membership are checked. A replaced root does not inherit the old rendering opportunity, and callbacks are not replayed.
- Publication uses the original rendering timestamp with scoped restoration on failure. Provider revisions are captured after native sampling and before installation callbacks; subsequent callback changes still invalidate the next layout.
- Native regression sources cover rAF plus microtask edits reaching the same paint, a child rAF editing a parent effect, and independent forked providers. Existing source fixtures cover hidden delivery, interrupted callbacks, snapshot cleanup and retained retry progress.

All statements above concern generated and inspected source. Compilation, regressions, full Core and five-family execution remain deferred to the global qualification phase.
