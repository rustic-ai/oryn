# S/V control operation inventory — source audit, 2026-09-16

This is a bounded producer/caller inventory for the control slices inspected in
this lane. Checked means production source and regression bodies exist and were
inspected. It does not mean compiled, executed, WPT-passing, or whole-S/V complete.
The main completion checklist remains the integration owner's document.

| Operation | Production producer → consumer | Source evidence/status |
| --- | --- | --- |
| Color value/default/reset, alpha/colorSpace mutation | DOM `color_controls` and input attribute transitions → binding getters/IDL reflection → FormData | [x] CSS4 parse/convert; alpha/P3/missing data; dirty/default distinction; pinned `color.window.js` and `color-attributes.window.js` reconciled |
| Color native selection | `TemporalPicker` RGB/alpha draft → trusted pointer/keyboard/typed AX → `set_value` → Event input/change | [x] Exact popup/session/candidate recheck; native color Event differs from text InputEvent |
| Color OIL and macOS AX | semantic native type/action projection → `semantic_edit::replace`; AXColorWell → SetValue | [x] Native readonly applicability; current value serialization and typed paint |
| Range pointer/touch | inverse native paint projection → shared numeric sanitization → live value/input/release-change | [x] Touch contact ownership excludes pan/pinch; suspension/cancel release; bounded ticks |
| Range/number AX increment/decrement | macOS legacy and modern actions → `Action::Adjust` → scoped `TextCommand::Adjust` → `native_keyboard::control_step` | [x] Native stepping; current disabled/inert/readonly check; no author key constructor or mutation hook |
| Range/number AX values | native DOM value/min/max → semantic native numeric properties → NSNumber AXValue/AXMinValue/AXMaxValue | [x] Live properties update; numeric NSNumber writes route native replacement; incremental/full regression source |
| Input action discovery | normalized native input type and applicable readonly → semantic actions | [x] Uppercase types; file/image do not advertise text replacement; color/range ignore inapplicable readonly |
| Datalist | tree-scoped ID lookup → native value/label/type filtering → bounded private candidates | [x] Disabled/empty options; multiple-email last-token replacement; ranges use valid unique step-aligned ticks |
| Temporal locale editing | authenticated parent UiLocale/UiLocaleData → bounded ControlLocale → native fields/UTF-16 caret/keyboard/clipboard | [x] macOS native Gregorian symbols/digits; Windows profile source; public ISO stays independent of display |
| Picker presentation/AX | native snapshot → generated private paint boxes → platform-only AX identities → exact owner command | [x] Real stacking order; viewport row window/wheel; RTL/vertical positioning; transformed native bounds |
| File chooser | native pending request → AppKit NSOpenPanel → parent upload policy/read → worker bytes + exact pending request | [x] No worker file paths; reset/type/attribute/retirement invalidation; cancellation and changed selection events |
| Form/target/body policy | native entry construction/FACE → serialized body + source authority → parent target resolution/queued navigation | [x] Earlier inspected slices retained in S form-target notes; complete receipt/target permutation matrix still open |

## Confirmed fixes from this pass

- OIL range/color replacement previously rejected `readonly` even though the
  attribute is inapplicable to those controls. Native eligibility and semantic
  action discovery now agree.
- Case-sensitive semantic input matching advertised text edits for uppercase
  FILE/CHECKBOX types. Matching now normalizes ASCII input type spelling.
- Color picker completion used text InputEvent/beforeinput. Non-text color
  completion now follows the same native Event input/change path as OIL values.
- AX numeric controls lacked increment/decrement, numeric min/max values and
  NSNumber SetValue handling. These now route to existing native algorithms.
- Native boolean AX properties no longer treat irrelevant HTML attributes on
  unrelated elements as checked/selected/readonly/required/open state.

Regression bodies: `semantic_range_and_color_ignore_inapplicable_readonly_and_use_event_input`,
`native_type_case_and_readonly_applicability_match_advertised_operations`,
`native_ax_numeric_adjustment_reuses_step_bounds_and_rejects_readonly_number`,
`native_ax_numeric_properties_update_and_match_full_recomputation`, plus the
named source bodies in V-GENERATION-NOTES-2026-09-15.md.

## Exact residual inventory obligations

- [ ] S-01: Manifest-wide IDL conversion/exposure/brand/descriptor/realm inventory
  and regeneration consistency; DOM collections/traversal/XPath/parsing/adoption
  and historical Range reproducer attribution. These were not exhaustively
  inspected by this control pass.
- [ ] S-02: Full definition/upgrade/reaction/adoption/parser/shadow mutation matrix;
  native custom-element control participation alone does not establish it.
- [ ] S-03: Complete target/rel/source/body permutations through persisted family
  receipts, including all callback/failure/retirement cases.
- [ ] S-04: All 180 historical drag failures and remaining input/clipboard/selection
  requirements mapped to independent regression bodies and actual native callers.
- [ ] S-05: Entire resolve/action/extraction/observation matrix across frame and
  document replacement, including errors and recovery; this pass covers controls.
- [ ] V-01–V-05: Actual full Core visual manifest-to-algorithm mapping covering
  layout, shaping/fonts/replaced/SVG/stacking/print, CSSOM and AX recomputation;
  control-specific source coverage does not close unrelated visual algorithms.
- [ ] Primary macOS native presentation/manual acceptance: locale format matrix,
  physical wide gamut/system appearance, native dialogs and assistive technology.
- [ ] Build, binding generation, test/WPT execution and candidate qualification
  remain deferred by the user's all-source-before-execution instruction.


## Shared autocomplete interpretation — 2026-09-17

Pinned `html/semantics/forms/the-form-element/form-autocomplete.html` requires distinct form and control getters. The prior generic native reflection returned on for every non-off input/select/textarea value, including missing attributes and valid field tokens. Added `oryn-dom/src/autocomplete.rs` with HTML whitespace tokenization, ASCII lowercase serialization, normal/contact field categories, optional section and shipping/billing prefixes, contact modes, final webauthn credentials, and hidden-input anchor rejection of on/off. Form enumeration remains the distinct on/off rule. Removed the superseded generic getter branch.

Native binding getters and built-in history eligibility share this implementation. Invalid/missing control tokens fall back to the owning form for autofill eligibility; valid field tokens and explicit on override form off. Raw content attributes are preserved. DOM regression sources cover ordering, unknown/extra tokens, hidden anchors, normalization and form defaults. Binding/history regression `autocomplete_reflection_and_history_share_native_token_rules` covers input/select/textarea reflection, raw attribute preservation, hidden/form distinction and history inclusion/exclusion. No build, binding generator or tests ran; broader S-01/S-03 remains open.


### Autocomplete exposure and complete keyword source matrix

The follow-up IDL audit found autocomplete was declared only on HTMLFormElement. Added the CEReactions DOMString attribute to HTMLInputElement, HTMLTextAreaElement and HTMLSelectElement so generated accessors reach the shared native getter/setter. Binding regeneration remains deferred with the rest of the whole-plan build phase.

Expanded DOM regression source to enumerate every normal/contact field in pinned form-autocomplete.html, uppercase/HTML-whitespace normalization, each contact-mode prefix, section/mode/credential combinations for normal and hidden controls, duplicate/reversed token rejection, and non-HTML whitespace. The existing binding/history source exercises all three newly declared interfaces and raw attribute preservation. These sources are unrun and do not close the complete interface inventory.
