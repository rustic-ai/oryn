# HTML sink caller closure — September 18

Source inspection/generation only; no generator, build, tests or WPT execution.
Pinned checkout: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.
Frozen manifest SHA-256: `566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101`.

## Core attribution

`wpt-capability-tiers-v2.json` classifies html, custom-elements, shadow-dom,
domparsing and trusted-types as Core. sanitizer-api is unmatched Extended;
its algorithm fixtures provide useful source controls, while safe/unsafe sinks
are directly invoked by the Core callers below. The frozen full-current cohort
explicitly includes the partial-update/template-for cases as `workstream: core`.
They cannot be omitted as merely experimental or out of scope.

## Production and regression reconciliation

| Contract / pinned fixture family | Native caller | Source closure |
|---|---|---|
| Document parse URL/base/encoding/style/adopted scripts | sanitizer::parse_document, retained_document_base_url, CSSStyleDeclaration, inert script flags | Added `detached_html_documents_keep_encoding_base_style_and_script_inertness`; provenance/borrowed realm already owned by Document sources. |
| Full replacement, script order, removal of following scripts, CE reaction ordering | set_html → prepare_sink_fragment → capture_replacement → PreparedInsertion | Existing inline/shadow/template/filter/order regressions plus external/module captured requests. Typed units preserved before constructors. |
| Full style visibility before either script | Same replacement/resource phases and native computed style | New native integration source `tests/set_html_unsafe_style_visibility.rs` exercises Element and ShadowRoot with conflicting style rules. |
| Customized built-ins filtered by `is` | Config::filter_tree before native CE lookup | Fixed parsed `custom_element_is` surviving removal of null-namespace `is`. Native and binding positive/negative/custom-registry fixtures added. Ordinary DOM removeAttribute semantics unchanged. |
| Scoped light tree and default declarative root registry | initialize_fragment_registry_from + stamp_uninitialized_subtree | Parser lane fixed new ShadowRoot defaults to node Document registry, preserves explicit null. Added nested/top-level/global/scoped/template-extraction fixtures. |
| HTML append/prepend/before/after/replaceWith safe and unsafe | sanitizer_insert::run, shared conversion/filter, native fragment staging, retained insertion batch | Added Element all five pairs and ShadowRoot append/prepend pairs to authored IDL and dispatch. Position, identity, grouped mutation, safe scripts, conversion movement, invalid policy and detached receiver sources added. |
| Declarative attachment during HTML insertion | attach_imported_fragment_shadows then native grouped light insertion | Native attachment journal suppressed only for observer projection; no ShadowRoot in host childList. Source fixture covers append/prepend and existing-root fallback. |
| Partial updates isolated to parsed fragment and filtered first | prepare_sink_fragment / native prepare_fragment → html::apply_fragment_patches | Parser lane owns PI provenance/template-for native semantics; sanitizer calls apply only after filtering. Legacy inert fragments apply without sanitizer. Document parser now uses the explicit unpatched entry and applies after filtering. |
| Streamed HTML admission/backpressure/cancellation | Loading lane WritableStream bridge + parser incremental session | Concrete remaining integration, assigned to loading/parser lanes; cannot substitute repeated whole-fragment reparsing. |
| Shadow adopted stylesheets | Native style/module preload and parser declarative root routes | Dedicated declarative binding/provenance/placeholder/cleanup sources generated; native module-map adapter owned by loading. See S01-DECLARATIVE-STYLESHEET-CALLER-MATRIX-2026-09-18.md. |
| Precommit capture failure and postcommit resource retry | shared post_connection_scripts capture and per-stage retained batch | Existing setters/adjacent capture and retry tests retained; new insertion methods use same admission and custody. New-method precommit capture and all-five-method × four-resource-stage postcommit retry fixtures are generated. |

## Exact direct source consumers

The following 55 files matched direct setHTML/setHTMLUnsafe/parseHTML/parseHTMLUnsafe
call syntax in the five Core directories above. This is a reproducible direct
caller index, not a claim that dynamic method lookup or legacy innerHTML callers
are exhausted. The template-for fixture additionally enumerates all new insertion
method names through dynamic dispatch.

| Pinned source | SHA-256 |
|---|---|
| `custom-elements/registries/ShadowRoot-init-declarative.html` | `8f2311e0f927103eb03f7a95baa3a7d4fca20a9c6da31d11857209ed0624ecdc` |
| `custom-elements/registries/scoped-custom-element-registry-customelementregistry-attribute.html` | `50f5c913f9c26450886dffa0e5991d051714db6a7666187ab00dfc3e76d9792a` |
| `custom-elements/registries/scoped-registry-append.html` | `81b6e14c26bc65a856eee3470cce6c6ceae77c8025a592782a0308a4ca837f93` |
| `custom-elements/registries/template.window.js` | `69829de50e5ec66f4ca4b74973f89c655422eca344213c13e830c82e7cf41b16` |
| `html/dom/partial-updates/tentative/fragment/sanitize-nested.html` | `df37ef32b81f235fb571d6810278aa4b63ecfde578a10ccfebb4118fe65705ef` |
| `html/semantics/scripting-1/the-template-element/template-for/template-for-html-setters.html` | `296879f2f1d1c5ef787f0a6d638f19058a444f26e3c0e7a68d0bbdf8a2da954a` |
| `html/semantics/scripting-1/the-template-element/template-for/template-for-multiple-cases.html` | `29ec21fa20ce76c4b5594cfac487bbc33d4678e6e524fb54aeb8a92d300c1b54` |
| `html/semantics/scripting-1/the-template-element/template-for/template-for-sanitization.html` | `ae03f0a46834167397777793298c5812c7e7ee7c0ba38b53f45f1c73c8555ec4` |
| `html/syntax/parsing/newline-normalization-cr-then-lf.html` | `ebf00ea912008b80466a6d6d2e339aa2cb0040712c881f7cf3d9a40dc58158ba` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/Document-parseHTMLUnsafe-encoding.html` | `d20039da574af0cf595e0e849479bd3022c2c5628bb3794822d8eb49ec16db25` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/Document-parseHTMLUnsafe-style-attribute.html` | `c8cd71bedc2671baa95dc100d92f65549b8a3109f32de74024de4dafcfa979a6` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/Document-parseHTMLUnsafe-url-moretests.html` | `fa6d943f16f4ad0f570ca0fc2337aa04abca0bc6f3467b516efd512ce4194960` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/Document-parseHTMLUnsafe.html` | `22bf3eb7ef3c9957200f6df2275a36e3e657e6dc9243ed273d03f181f4ff11e5` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/Element-setHTMLUnsafe-04.html` | `6b1afb3e0fee79b9b5e0deb7ad0f1121799148b1082f34d8477ebcc361b4ab02` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/resources/parseHTMLUnsafe-iframe.js` | `671693195a4f966ab627f640e83fb24205404b8cf8eb72b6ff73dd11eb490623` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/resources/parseHTMLUnsafe-url-tests.js` | `4fb5275224b30aa8f61e8965dbc86682e4472272316a12cccda360d9f6938f3b` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/setHTMLUnsafe-CEReactions.html` | `71c26e10327ebb9fbd2adab75cfd12d70bd14c2ac01ec0c9eb06e850b77b8806` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/setHTMLUnsafe-runScripts.html` | `8acec7e283427ef09038a15b467f30e7a15dd1597e7a00bfee38210f528d7ef2` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/setHTMLUnsafe-xml.html` | `f63ba87537523ecc60dab5a369e58596ace12acca2f7c6867f878601f98770fc` |
| `html/webappapis/dynamic-markup-insertion/html-unsafe-methods/setHTMLUnsafe.html` | `0120874e626dc1a3ecdb128f61edf18d60a2b6619658e65b3f488c84f5511310` |
| `shadow-dom/Document-caretPositionFromPoint.tentative.html` | `ca2222354b360ef1728b4a985f884306201ab3f3300e9e2faf09463eaadeb533` |
| `shadow-dom/HighlightRegistry-highlightsFromPoint.html` | `28ce765e3f7ef41b42074966bb0e75ce6744e2364ddc5d5a30f7aeb9ceeadebf` |
| `shadow-dom/declarative/declarative-shadow-dom-attachment.html` | `476db5f0c501d3b1c6d8ac7e634b405d4281a9264ec28306c1997c58a7de34d4` |
| `shadow-dom/declarative/declarative-shadow-dom-basic.html` | `f4eb6d3e2a5459e08453f1ff25ffcbc9da0cfe696c76221d194f646532274bca` |
| `shadow-dom/declarative/declarative-shadow-dom-opt-in.html` | `d1b7e6157c3d01c37cf5db7fe46f4734152b3abf2234e7368bea6083268c67f6` |
| `shadow-dom/declarative/declarative-shadow-dom-slot-assignment-serialization.html` | `9185a61c97434edc3c771580d78d97cacb2d75db003301045c37d3d9ffed0e56` |
| `shadow-dom/declarative/declarative-shadow-dom-slot-assignment.html` | `4f787845ea19657912b8d0d9aa8a3d445f448ba990a6f387d1e5a04b752efc82` |
| `shadow-dom/declarative/gethtml.html` | `392ece06798c9a21083b0f3a49cc4ff7a08a57cb00ca2bbcee45a08c62df8d26` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-async-fetch-disconnect-iframe.html` | `2176026538c343f3e351ef26cdd672c70e3804a4c488ac3632b61e404ad7dfa8` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-async-fetch-shared.html` | `d69dd0baf5a97af22a7bfa9dc2e3e8fcdaa215683a9efa3cdc2ebe1f4f9417eb` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-clonenode-async-fetch.html` | `971b5f9751b561ef4e2b26f3892a847c6a3476f60054bb0fd673e7f56e510185` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-document-parse-crash.html` | `f93ae22232207ce240f5f4c1e90e3d54564f06b5c1663e6e9f877bcc4bb262a5` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-modulepreload-nested.html` | `5f356724f8f78b42ea838945de785f42890eb7ff8d76a569790a6dbfe4f1aed3` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-modulepreload-shared.html` | `7cf4df8aacc84c9621bcc61e53ab0ba9447848ff6875c4acf65658d3eb845136` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-nested-async.html` | `6fea2741b478db41fab8f9d50c1509caf88658172405149860c7eb80dbb29c18` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-nested-sync.html` | `138f873114ba0cf28f8d24c56457ed69adaf0da104d9c5214fc79215c231d8dd` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/shadowrootadoptedstylesheets-same-document.html` | `f18598320daa5982796f4a6b29f46e129f635d82f7ea58a43b82993415250c36` |
| `shadow-dom/declarative/tentative/shadowrootadoptedstylesheets/support/helpers.js` | `a2540e7ea1bf94207fe8123da0a158d3c5b1a8b73f4d117b69bd35741577bc1b` |
| `shadow-dom/reference-target/tentative/dom-mutation.html` | `e3db2e932e66636ed72b187a908955cfc297e478b3652288a493375ce4184680` |
| `shadow-dom/reference-target/tentative/label-descendant.html` | `a28c690372bf31ae87756bf56d2916518fd90177608775d6386a26eba4fba167` |
| `shadow-dom/reference-target/tentative/property-reflection-idl-setters.html` | `b032e20b1672968486972bb77ccd7ee15a9f1f7f0c24122ebbd4f9bb3279c055` |
| `shadow-dom/reference-target/tentative/property-reflection-imperative-setup.html` | `427fa6c4c058db55f3fe49e79fd2928fcf1d0e45e0955941117741f7226020df` |
| `shadow-dom/reference-target/tentative/property-reflection.html` | `5dca2249d50f949da19c63d4ab4ea6ab70878d7f2c26be698452acffd70ba47a` |
| `shadow-dom/reference-target/tentative/resources/property-reflection-helper.js` | `76c449825c8860a330c2fb520cb4693c584e39df62e0ba0de50f55f81ed26430` |
| `shadow-dom/shadow-root-clonable.html` | `0fb8294bda8da8b309f571fe11a697078e9a595bfadf34e449610a68b46434b2` |
| `trusted-types/block-string-assignment-to-Document-parseHTMLUnsafe.html` | `0398d754fe803f86acfd963c1a4677cd1dbd939ea961e6ccf28622a7ce25f32c` |
| `trusted-types/block-string-assignment-to-Element-setHTMLUnsafe.html` | `d2d5b866ec070addadb1ed07993e3e0377fc088a820f3e41dd60d28577c8fc6b` |
| `trusted-types/block-string-assignment-to-ShadowRoot-setHTMLUnsafe.html` | `305e2673028cc3c5fe04a7edb05947f836a44f877a552a4254eeb4e6d6125820` |
| `trusted-types/script-enforcement-001.html` | `22e8f6b575e861edb8bdd417d58c981b409d196a6aa8e1a614d7c5c1db00dee3` |
| `trusted-types/script-enforcement-002.html` | `a6451dd6855800e882c95af2a5106d6889086af28cbb39cea2146a193d6fdeda` |
| `trusted-types/script-enforcement-003.html` | `49d12eed2827a8b544fd0cce927a6d9de506e22346e4733c2686439682d5fcda` |
| `trusted-types/script-enforcement-004.html` | `0e33730a97bb45852d20d69195bbb5cfbd3dbf2b8d8dfe954a0f25a362482373` |
| `trusted-types/trusted-types-reporting-for-Document-parseHTMLUnsafe.html` | `49757f7b2cd8f75767d6d5f93c81069d091f9475bb4886ff889a6d7de3fec156` |
| `trusted-types/trusted-types-reporting-for-Element-setHTMLUnsafe.html` | `d88933a31d75a5a8b5b087546af58ca2a1a6453f0ddb16ffebbc13df566fb0f3` |
| `trusted-types/trusted-types-reporting-for-ShadowRoot-setHTMLUnsafe.html` | `817f785330860c3211c4214340bd216aebf02c93d2aeed4f15fc963a8129a7e9` |
