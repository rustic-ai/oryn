# Pinned Core operation source matrix

Source-only, unbuilt and unrun. The JSONL companions retain exact source hashes, lines and snippets; counts are occurrences, not executed operations or conformance passes.

## Saved denominators and boundaries

- `core-helper-occurrences-2026-09-16.jsonl`: 1,199 reviewed/saved occurrence records. This includes 208 quoted-scan candidates, official rewrites/authority substitutions, 869 HTML-tokenized or literal-loader additions and 20 dynamic loader expressions. The historical 159/44 exploratory scan did not preserve its occurrence list; it cannot honestly be reconstructed one-to-one. This saved audit supersedes that unsaved count as source evidence, without claiming to have executed any branch.
- `core-driver-operation-occurrences-2026-09-16.jsonl`: 1,516 literal call occurrences, including conditional shared helper code. Both quoted and unquoted HTML script attributes were inspected. Driver library definitions are excluded. Each occurrence records whether its source is itself a Core entry.
- `core-reference-matrix-2026-09-16.jsonl`: all 1,015 Core reftest/print-reftest identities and their 1,016 match/mismatch target occurrences, with source/reference hashes, exact hrefs, metadata and resources. Every target exists in the pinned checkout. Executor names match the actual Oryn Product classes.
- `scripts/wpt_source_matrix.py` validates the saved source/target identity and refuses inferred qualification. Regression sources reject changed reference bytes, fabricated PASS metadata and denominator drift. No validator was executed.

## Production route mapping

| Pinned operation | Native production path | Source regression evidence / boundary |
| --- | --- | --- |
| Actions, click, bless | Upstream actions/click parts → native driver action route; element preparation → `ElementCommand::PrepareClick` → native hit testing and trusted pointer; bless uses that click | Driver/native pointer, drag and element interception sources; full interruption and special option matrix still needs source closure |
| send_keys | Native element value route → Focus/file-input discrimination → native UTF-16 action queue | `element_send_keys_validates_before_focus_and_preserves_existing_selection`; `native_element_file_selection_retains_real_bytes_and_dispatches_trusted_events` |
| get_computed_label, get_computed_role | GET native computedlabel/computedrole → `ElementCommand::Accessibility` → native semantic provider | Native AX/selector tests; remaining generated-content/ignored-node/rendered visibility projection belongs to S/V semantic review |
| get_accessibility_properties_for_element/node | Oryn accessibility protocol parts → authenticated native route → `ElementCommand::AccessibilityProperties` | `accessibility_properties_use_native_checked_state_and_stable_scoped_tree_ids`; no author DOM getters stand in for native nodes |
| get_all_cookies, get_named_cookie, delete_all_cookies | Driver cookie endpoints → parent native cookie store | Native HttpOnly/path/domain/Secure/expiry regression sources; no `document.cookie` shim |
| set_permission, bidi.permissions.set_permission | Native HTTP permission route → parent permission authority → native consumers | `native_platform_routes_share_permission_location_and_visibility_state`; exact origin/embedded-origin limits remain explicit |
| bidi.emulation.set_geolocation_override | Oryn async protocol part → native scoped override route → parent positions → native geolocation callback objects | Same driver state-sharing regression plus geolocation option/cache/hidden/retirement source regressions |
| get_window_rect, set_window_rect, minimize_window | Native window/session state → parent visibility revision → native document visibility consumer | Same driver regression covers failed rect validation before restore and trusted visibility changes |
| set_rph_registration_mode | Native custom-handlers mode → parent registration/handler resolver → real navigation | Native handler encoding/substitution and document grant/retirement sources; default prompt support remains separate |
| set_test_context | Pinned `testdriver.js` calls `test_driver_internal.set_test_context`; injected `testdriver-extra.js` retains selected context | Upstream harness context routing, not a new browser endpoint |
| Bluetooth BiDi methods in shared helper | **Not called by the inventoried legacy Core Bluetooth tests**: those call `bluetooth_test`, while BiDi subscriptions/simulation start only in `bluetooth_bidi_test` and the explicitly loaded BiDi helper | Native legacy `.test.allResponsesConsumed`/getDevices/policy consumer is generated; no GATT or BiDi simulation is claimed or faked |

## Source classifications that must not become repairs

The 94 webidl2 unresolved relative imports are JSDoc type annotations inside the bundled source. Documentation examples and English assertion titles are not executable imports. Import-map/declarative stylesheet names must resolve through native module semantics, and compiled string/event-handler/document.write sources use their specified active document/script base. Data/blob sources do not require checkout files. Root-clamped `../../../service-workers/...` META URLs resolve to the actual origin-root helper.

Six unused missing includes remain real 404s: form-stop `resources/helpers.js`, two pageswap `/resources/common.js` occurrences, BFCache `resources/test-helper.js`, CSP parsing `support/helper.sub.js`, and accname `/resources/wpt_test_util.js`. Required symbols are inline or come from separately loaded existing dispatcher/utils/aria-utils/disable_bfcache resources. No empty success helper is substituted. Four redundant local testharness META references similarly supply no extra symbol because the official `.window.js` wrapper already installs the canonical harness.

The two required misplaced helper imports use the already declared exact fixture repairs. Intentional invalid module/resource fixtures retain their failures. Every classification retains original source hashes and context and does not authorize changing assertions, expected results, denominator or raw outcomes.

## Outstanding production obligations

This matrix closes the missing-file classification exercise with durable evidence, not whole-G behavior. Parent owns pageswap activation/order and native form-stop cancellation review identified by the helper callers. Remaining source work includes full conditional/dynamic caller reachability beyond literal driver operations, rendered AX/option/input lifecycle obligations, exact visibility/physical capture fidelity and reference readiness/fuzzy/print behavior. Actual native images, terminal navigation history, fixture delivery and independent manual/platform observations remain deferred until the all-generation barrier.

## Continued integration audit — 2026-09-16, source only

The saved occurrence corpus contains **38 literal API names**: 19 mapped required operation names and 19 Bluetooth BiDi names in the two explicitly conditional helper sources. `wpt_operation_contract.py` now requires every name to have its declared route or exact conditional exclusion. Any newly added/removed name, direct Core Bluetooth BiDi caller, or missing native source reopens the source review. `wpt_source_matrix.py` also verifies each operation name/literal against its exact pinned source line. This checks source attribution, not runtime behavior.

Two actual production defects discovered during this audit are now repaired in generated source:

1. `GET oryn/diagnostics/navigation` had a handler but no `command_methods` admission, so terminal-navigation evidence was unreachable. The method table now admits it; its existing native closed-page-history regression reaches the production dispatch path.
2. Pinned upstream `WebDriverProtocol.teardown` suppresses general session-end exceptions, and `Session.end` additionally suppresses transport and invalid-session errors. Oryn now uses checked session termination plus private original-run connect/teardown custody records. Legitimate last-window closure requires the exact parent's bounded completed-retirement receipt; missing, pending, failed, foreign or pruned records cannot stand in for cleanup.

`POST oryn/qualification/worker-crash` is an authenticated current-session/current-page control that accepts no caller PID/page target. It uses the existing native owned-worker termination and checked reap. The dedicated prerequisite collector uses it alongside ordinary unknown-command, missing/duplicate async completion, real certificate rejection and actual focus transition controls. These controls remain outside the pinned Core denominator.

The earlier special-option gap is superseded by `native_option_click_uses_select_container_and_rechecks_author_callback_detachment`; native pointer state sources now include mouse/pen/touch, wheel, primary-pointer cohort and orientation conversion regressions. Full S/V semantic and cross-package lifecycle correctness still requires their source review and later execution. Neither an endpoint nor this 38-name accounting closes whole G conformance.
