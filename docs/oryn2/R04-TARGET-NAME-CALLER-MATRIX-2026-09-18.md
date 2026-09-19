# R04 / S target and Window name source closure — September 18

Source generation only; no builds, generators, tests or conformance execution.
Pinned WPT: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.

## Caller and representation matrix

| Boundary | Generated source / audit evidence |
|---|---|
| Generated and native Window.name | UTF-16 code-unit extraction into collision-safe DOMString storage; getter decodes it; post-conversion access/liveness recheck. |
| iframe/frame name and anchor/area/base/form target, submitter formTarget | Existing lossless conversion retained; shared receiver custody now also covers non-control HTML target/name conversion-time adoption. |
| Same-origin Window named keys | window_named resolver decodes exact V8 units; native frame names precede element ID/name lookup. |
| Local cross-origin Window named keys | window_proxy named child conversion preserves units; ASCII safelist/fallback precedence unchanged. |
| Remote cross-origin Window named keys | remote_window_projection descriptor lookup preserves units before matching parent-projected names. |
| Form/base/submitter target precedence | submissions stores original DOMString and applies dangling-markup classification to decoded code units; conjunctive `<` plus TAB/LF/CR rule retained. |
| Native selection and history | Parent context service, worker_service name registry, local inventory and frame/root history copy String storage unchanged. Keyword case matching does not lowercase the selected native name. |
| Window.open | New native callback replaces actual bootstrap JSON entry. URL/target/features convert once in order; URL scalar conversion uses entry-realm base, target preserves DOMString, receiver supplies authenticated native creator. |
| Dangling markup distinction | Pinned dangling-markup-window-name fixture explicitly preserves LF plus `<` through Window.open; form/link normalization remains separate and is not applied to open. |
| Popup relationship features | Native feature tokens handle noopener and noreferrer, case, boolean/integer prefixes and duplicate values; noreferrer implies noopener and no-referrer policy. |
| Duplicate names | Pinned duplicate-name-order uses current subtree, remaining source tree, then other familiar pages; native parent selection now follows that order with live identity checks. |
| Existing named frame opener | Window.open publishes a parent-owned relation keyed by the stable native frame token before navigation. Native and remote getters read it across document replacement; explicit disown removes only that relation, frame destruction/page cleanup releases custody. |
| Parent keyword selection | `_self`, `_parent` and `_top` use authenticated creator ancestry and exact native frame snapshots; no literal-name fallback or unrelated replacement frame. |
| Selected result | Parent OpenPage authorizes destination and queues navigation; selected frame token/native auxiliary root supplies returned Window identity. Noopener/forced severance returns null. Existing auxiliary construction rollback remains owner. |
| Standalone selection | Bounded native window/DOM traversal with native names; no authored querySelectorAll, Window.name getter or JSON operations. |

Removed obsolete `RUST_RUNTIME_GLOBALS` Window.open implementation and its
OpenPage JSON compatibility admission/response projection. A regression asserts
OpenPage can only arrive through native producers. Other compatibility context
operations remain unchanged. Native calls use the typed
OpenPage request, not JSON serialization of lone surrogate strings.

## Regression sources

- `window_named::tests::window_names_and_named_element_keys_preserve_domstring_units`:
  lone high/low surrogates, NUL, supplementary pairs, literal private prefix;
  target/formTarget/base/anchor reflection and conversion-time adoption.
- `oryn-native/tests/window_target_names.rs`: native Window.name round-trip and
  parent named form selection for each exact representation; native open popup
  identity/reuse, argument ordering and thrown suffix, JSON/String/query poisoning,
  noopener/noreferrer behavior, SyntaxError, borrowed frame receiver/entry base,
  duplicate-name selection, existing-frame opener persistence/disown and source
  frame-destruction revocation. Native cleanup sources assert idempotent removal
  in both relationship directions while preserving unrelated frame records.
- `window_open_feature_tests`: duplicate feature tokens and integer-prefix rules.
- Existing submissions UTF-16 delimiter regression sources remain part of this
  matrix; fresh tests do not change target sanitization semantics.

Source review is not qualification. Global validation must build both binding and
native layers and execute these sources plus the pinned caller cohort.

## Pinned caller files

| Path | SHA-256 |
|---|---|
| `html/browsers/windows/clear-window-name.https.html` | `d153b99056e8d7824f7a0f92561d9521df7cdd4af5d462d4da6a7289a5824111` |
| `html/browsers/windows/dangling-markup-window-name.html` | `5506613313287991f58c62ca79f684071e572281c224d408b93b3e6266a10277` |
| `html/browsers/windows/embedded-opener-a-form.html` | `81900a08f66488e394c1cc20d110abc61b8fb43b44e5cf6c91285df402277762` |
| `html/browsers/windows/embedded-opener-remove-frame.html` | `c0bbe49cc7f0afc5d9a1e4c0ecdc8a13e2e3a1553e82e6ea7347cc0e3f03fbd6` |
| `html/browsers/windows/embedded-opener.html` | `ea175717f63936c64e241a7825145f9642bab7c466e7054b1fe0142caa68f586` |
| `html/browsers/windows/noreferrer-cross-origin-window-name-manual.sub.html` | `24876e29f1cefbd1d586dca3e01a09ec626f6c5fce83da10e4de3379482f2dd4` |
| `html/browsers/windows/noreferrer-null-opener.html` | `4d82187ebfc63b842b70c58a68d1d19eea87db7776a6cd814f9973117bb1f9f4` |
| `html/browsers/windows/noreferrer-window-name.html` | `65194355a163d46adc113fed062fb0b32174a3be2b1ac7872392166c5cbe7def` |
| `html/browsers/windows/opener-cross-origin-manual.sub.html` | `ba168b42a56c151c1a6c93d1318b4147b7a225399034dbe3ca6de0df86025a26` |
| `html/browsers/windows/opener-string.window.js` | `b2e66b4117e322cca22d6573726cc7190bb10dd0bc8a20b9ff9cdd6cee5668e5` |
| `html/browsers/windows/restore-window-name-manual.https.html` | `064a3b3e1281c099ce3b3f0a3e9932651981e8c1c49605a11801fad90caf83aa` |
| `html/browsers/windows/targeting-cross-origin-nested-browsing-contexts.html` | `56be0d783f5d7617fed7ba76c65db7749b4aeedd51971717efb9e88b554872a5` |
| `html/browsers/windows/targeting-multiple-cross-origin-manual.sub.html` | `aff20c565cd16b38f35c740681339af522b4c3172cf5db885ae31638b49641a8` |
| `html/browsers/windows/targeting-with-embedded-null-in-target.html` | `0e9ce06e0be9c258662569855b23881b5618779db40b13a5c109437c7b7ec310` |
| `html/browsers/windows/browsing-context-names/duplicate-name-order.html` | `d455c2882a0760b390db3e72f592bd8877f5219718522fba4dec3aab893172b6` |
| `html/browsers/windows/browsing-context-names/choose-_self-001.html` | `e5365fd155b8d0f17530b2f3865cc78c38528cbe0c464e0f6725f33870125af3` |
| `html/browsers/windows/browsing-context-names/choose-_parent-003.html` | `0b91ef58e7e1eff24c2481b97cdce66094b8b0e205160fc1802bce20803d4198` |
| `html/browsers/windows/browsing-context-names/choose-_top-001.html` | `a918755aff75e8b1f43c73c60f781907b06f88be27354dbccac3d6c1aad9fe35` |

## Blank-target prerender final producer integration

Source generated, not executed. Pinned `speculation-rules/prerender/activation-by-form-submission.https.html` reconstructs the final GET query from hidden controls, sets `_blank` plus `rel=noopener`, and requires activation of the existing prerender. Its `prerender_until_script` variant also requires the previously fetched candidate to resume rather than load again.

| Caller | Generated selection and custody behavior | Regression source |
| --- | --- | --- |
| Native `Window.open` | Final effective noopener `_blank` GET receives a parent-owned `BlankPrerenderActivation`; queues source actor handoff and returns null; does not create an auxiliary root or queue a navigation | Parent exact candidate selection matrix; native feature/conversion sources |
| Form preparation | Existing targets retain their native receipt before serialization. New noopener blank intent remains `SubmissionTarget::New` until frozen GET query or POST resource is complete | `submissions::parent_selection_tests::noopener_blank_defers_new_context_selection_until_get_or_post_is_finalized` includes injected body failure, mutation and retry |
| Final form action | Finalized GET reaches `OpenPage`; activation receipt enters native source actor queue and returns immediately. POST uses normal popup/resource navigation | Parent matrix covers GET, POST, deferred selection, opener, different query |
| Parent selection | Popup/sandbox/consequential/URL policy precedes claim; exact source and URL required; selected existing contexts keep their old route; link-specific route remains separate | `selected_navigation_request_tests::blank_prerender_final_selection_matches_get_url_without_queueing_another_load` asserts hidden publication, no candidate navigation queue, one claim, and rejected matches remain available |
| Activation error | Consumer propagates queue error without restarting navigation; authority lane retains candidate cleanup custody | Authority lane source roster/cancellation and actor handoff regressions |

Authority lane owns hidden candidate reservation/loading, exact source liveness checks, activation queue and actor transfer. This document records producer source integration, not a build/test/conformance result.

### Native blank activation ACK and publication protocol

Generated `prerender_blank_activation.rs` now owns a bounded (1,024 records) destination-scoped ledger. `ActivateBlankPrerender` authenticates the original source socket and the selected hidden destination/token/generation/epoch/URL, parks the initial document once, and reuses the exact predecessor through native promotion ACK retries. It then discards that initial snapshot. `PublishBlankPrerender` requires the retained activation ACK and current canonical native promotion identity; only the actor calls it after adopting the existing executable. Original source document retirement does not revoke this completion receipt. Destination destruction clears its ledger; source destruction does not erase destination completion custody.

Regression sources cover injected ACK loss after native promotion, no second park, no premature publication, source retirement, repeated publication, foreign/mutated receipts and an intervening destination successor. RPCs are native-only, handled before shutdown/current-document admission. Concurrent duplicate activation is rejected for retry while the existing operation retains custody. These sources remain unexecuted pending the global generation gate.

### Final route, queue and terminal custody reconciliation

Original retired worker routes now retain source-bound Activate/Publish/Acknowledge completion access through `NavigationResourceCleanup`; normal navigation authority stays revoked. Source candidate scopes and unacknowledged activation records prevent early route drain. The actor sends final acknowledgment after publication and retained activation events. Completed records no longer retain the route and remain bounded replay evidence; only completed records can be evicted at the 1,024-record bound. Destination destruction retains an authenticated terminal tombstone, yielding `PageNotFound` only after exact receipt/source matching, so close-during-activation cannot become endless generic-protocol retries.

Native producer preflight reserves the bounded activation queue before sending `OpenPage`, eliminating the capacity-failure-after-claim gap. Publication sets active-page state with hidden-page visibility in one locked commit and emits one Opened/Activated pair; retries never refocus a later-selected page. Added sources exercise a retired route, missing/forged routes, final ACK drain release, focus replay and authenticated destroyed-destination completion. No source or regression has been executed.
