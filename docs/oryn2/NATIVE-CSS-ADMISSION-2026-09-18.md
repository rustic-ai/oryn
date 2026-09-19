# Native CSS admission repair after layout test14

Source generated; no build or test execution in this repair batch.

Test14 reported 371 passes and 108 failures. Property admission is one shared cause, not evidence that all 108 failures are fixed. Geometry assertions remain intact and rendering owns the independent layout repairs.

## Confirmed gaps

- BaseDocument enabled grid and columns only during document construction; standalone CSSOM parsing and `CSS.supports` could run earlier and return different results. Writing-mode, variable-font, and container-query gates were never enabled.
- Native formatting/painting consumed properties whose Stylo metadata restricted them to Gecko, preventing the Servo-backed native adapter from receiving their values. These include pagination breaks/page descriptors, column rules/fill, ruby, scroll margins/padding, viewport gutters and selected SVG/control properties.
- BreakBetween parsing did not admit recto/verso/column even though native pagination and multicolumn code consume them. Native-only variants now serialize normally; legacy page-break aliases retain their restricted syntax.
- Field-sizing has a native control sizing consumer, but neither property admission nor native computed projection carried it.

## Source integration

`vendor/stylo/native_profile.rs` installs the explicit process profile once, called from ParserContext construction, property-name admission and BaseDocument before Stylist creation. The generic `layout.unimplemented` flag is no longer turned on by BaseDocument. Native-consumed longhands previously behind that flag are admitted individually. Private Gecko properties and unimplemented corner-shape remain excluded.

Property tables admit compatible native consumers and logical aliases. Handwritten shorthand implementations have matching engine availability. Existing native formatting code remains authoritative; this change supplies the values it was already written to consume.

The exact inventory is recorded in `artifacts/completion-validation-2026-09-18/native-css-admission-source.json`. New integration test `crates/oryn-layout/tests/native_css_profile.rs` runs in its own process before any native document construction, racing parse-first and enumeration-first routes across eight threads. It checks CSS.supports, mutation preservation, page descriptors, legacy break alias restrictions, and rejection of the blanket experimental corner-shape gate. Existing layout geometry tests verify downstream effects in the coordinated run.

## Limits

This admission list is based on concrete native consumers, not every property present in upstream Gecko. For example, text-wrap-style appears only in a generated style-copy list; no native balanced-line consumer was found, so its Gecko-only admission remains unchanged. WebKit text fill/stroke appears in private permission-control suppression, which is not evidence of a general author text-paint implementation; those properties remain excluded. Native mask-image is admitted but unsupported additional mask syntax is not enabled via the blanket flag.

## Test15 compilation reconciliation

Test15 stopped before assertions with nine compile diagnostics. Servo's preference-gated container-type serializer field is optional; serialization now handles that field while retaining the nonoptional Gecko route. Content-visibility uses the existing engine implementation for hidden endpoint interpolation, distance and nonadditivity in both backends. FontSynthesisStyle now derives the Servo serialization traits needed by the admitted font style struct. New integration regression sources cover these contracts.

The previously compiled generated ComputedValuesInner had 21 style-struct Arc fields; the new generated structure has 22, adding only Arc<Page>. Source comparison confirms all other fields in ComputedValuesInner and ComputedValues are unchanged. The 64-bit compile-time invariant is updated from 232 to 240 bytes (one additional 8-byte Arc), retaining the architecture-specific size guard. No size assertion was removed. No tests or builds were run while applying this reconciliation.

## Test16 cascade follow-up

The native profile integration target passed all four tests. Layout totals remain incomplete because the parent stopped the run after confirming three loop threads; the observed assertion inventory is not a completed suite result. Independent cascade fixes generated from that inventory:

- `:in-range` had a state enum and matching hook but no parser spelling.
- The native element implemented part predicates but inherited empty `each_part`/`each_exported_part` iterators; rule collection therefore could not see exported names. The new iterators preserve multiple aliases and reject malformed mappings.
- Servo inherited the selector trait's rejection of all pseudo-elements after `::slotted`. Native tree-abiding pseudos and `::before/::after::marker` are now admitted, enabling existing declaration-tree counter provenance.
- Ruby display keywords/constants remained Gecko-only despite native ruby layout. They are now admitted, including the internal annotation display that stops decoration propagation.
- Zoom was still chrome-only despite native zoom cascade and SVG metadata; author content admission is explicit. Content-box and stroke-box transform references were additionally hardcoded off for Servo despite native reference-box geometry.
- The permission-icon constraint reads margin-inline-end, which was absent from the projected computed property list. It is now included.
- Native post-cascade adjustments copied otherwise equal style maps; final projection now restores value-based Arc sharing after adjustments, preserving distinct private metadata.
- Effect URL discovery now rejects bad URL/string tokens and EOF-recovered unterminated string tokens before publishing any collected URL. Normal CSS token parsing, escapes and comments remain owned by cssparser.

Added source regressions cover native value/selector admission, multiple exported part aliases with removal, and escaped/commented URL strings plus malformed suffix rollback. Existing cascade, geometry, counter-scope, permission-icon and interning assertions remain unchanged. No tests were run while applying these changes.


## Test20 residual CSS source repair (repair35)

Observed layout target: 449 passed, 39 failed of 488; native CSS profile 6/6 passed. These are completed target results, not whole-suite conformance.

- Native stale image regression reports DOM revision 10→10 and inline/computed/demand all still first.png. `CssUrl::eq` compared only resolved URLs: both relative URLs against about:blank are None. Equality now also preserves original serialized URL, so CSSOM declaration replacement cannot disappear; direct CSSOM regression covers invalid relative URLs, equivalent resolved URLs with distinct spelling, and exact no-op reuse. No accessor or cache workaround.
- Generated content `url(...) / "Selected"` was rejected by the false `layout.css.content.alt-text.enabled` preference despite implemented native alternatives. Explicit native profile admission and CSSOM regression now cover it.
- Select fallback projection previously lost option records because the canvas suppression helper also deleted select children before selected-label capture. Canvas suppression is canvas-only; the existing select projection retains sole ownership of its option suppression. Existing longest-option/current-selection regression is preserved.
- Generated image identities were published without CSS image demand (only marker fallback explicitly populated requests). Finish now publishes the URL beside each rendered generated image identity, including the isolated picker subtree. Existing nested-picker and generated-image assertions remain.
- Directional part fixture had no shadow root: its light-DOM part correctly failed to match. It now asserts the negative case, then declares a real shadow root and retains the blue directional assertion.
- Rendered-tree excluded-node fixture now asserts absent geometry/offset with retained computed style, consistent with the actual projection contract and shadow unassigned-node coverage.

No tests/builds executed during this source repair. Native ruby/flow/print geometry failures remain assigned to their respective lanes.


## Test22 remaining value/selector admission (repair37)

- Writing-mode longhand admission did not expose `sideways-rl`/`sideways-lr`: the pinned WritingModeProperty enum and two axis-construction match arms remained Gecko-only. Both variants and their existing native flags are now available to Servo. The other writing-mode match has a wildcard; no other enum consumers in Stylo, Blitz, or stylo_taffy require a new exhaustive arm. Profile regression checks parse/serialization plus both bidi directions' inline/block axes; invalid sideways-diagonal remains rejected.
- `::part(...):dir(rtl)` was rejected by an explicit after-part parser guard. The native direction matcher already uses the authenticated DOM direction snapshot. The dir branch now admits this combination; lang and structural restrictions are preserved. Actual shadow-tree regression retains light-DOM negative, RTL blue positive and LTR red negative. Profile regression rejects part:first-child.
- Resolved-text fixture inherited width23px, making its collapsed space a soft-line-edge zero-advance cluster. Final-layout resolved text intentionally erases that cluster and retains source aliases. The fixture now separately checks unwrapped width200px text and wrapped width23px erased-space aliases/source positions; no whitespace/layout algorithm changed.

All changes are source-only pending the next frozen validation batch. The independent PerformanceObserver/resource-timing borrow/privacy review found no concrete blocker in the scoped snapshot-extraction repair.
