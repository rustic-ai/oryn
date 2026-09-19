# HTML parsing, serialization and sanitizer source map

Scope: S-01 dependencies in the full Core/OIL completion plan. Pinned WPT revision: `719d5e38fdd0903a18ed9007aba816c98cc491e0`.

Status: source generation and inspection only. No project generation, build, tests or conformance execution. All parent/package gates remain open.

## Production and regression map

| Obligation | Production sources/callers | Generated regression sources |
| --- | --- | --- |
| Shadow serialization flags, recursive getHTML, explicit/serializable roots | `oryn-dom/src/serialization.rs`, native Node flags, `v8_bindings/template_links.rs`, browser.webidl | Native serializer tests; `custom_elements.rs` flag matrix, nested/closed/template/registry/adoption and IteratorClose cases |
| Native configuration canonicalization, default preset, modifier algorithms | `oryn-dom/src/sanitizer.rs`, `sanitizer/default_policy.rs` | Configuration presence, duplicates, namespace, defaults, local/global overrides, ordering and actual-change results |
| Sanitizer.get sorted snapshots | Native Config::sorted_snapshot and binding configuration_value | All configuration lists, per-element overrides, null/empty namespace, UTF-16 astral/isolated-surrogate ordering and nested copy isolation; native policy remains unchanged |
| Mandatory safe baseline and URL filtering | Native sanitizer safe filtering; shared `oryn-dom/src/event_handlers.rs`; Trusted Types classifier | Baseline idempotence/order, namespace distinctions, URL parser controls/case/UTF-16, SVG animation attributes, template/shadow traversal |
| WebIDL dictionary/sequence conversion and native Sanitizer instances | `v8_bindings/sanitizer.rs`, platform record/constructor/operation dispatch | Getter order, inherited dictionaries, nested IteratorClose, original exceptions, UTF-16, copy isolation, reentrancy, subclasses, native branding |
| Sink option defaults and foreign native instances | `sanitizer::options`, native receiver-owner lookup and delayed configuration snapshot | Safe/unsafe defaults, ignored runScripts for Document parsing, cross-realm instance mutation, public get override |
| Document.parseHTML/parseHTMLUnsafe | Native static callbacks, inert parse/filter/import, original Document provenance | Policy/default/custom behavior, metadata, declarative shadows, conversion and Trusted Types order, method names/arity/non-construction |
| Element/ShadowRoot.setHTML/setHTMLUnsafe | Receiver re-resolution, template-content ownership, FragmentContext, native filtered replacement | Table/SVG/form context, invalid policy preserving children, conversion-time receiver/template/shadow adoption, safe-script no-op |
| Top-level declarative context | Retained html5ever context handle, existing/disabled shadow checks, filtered shadow import | Closed serializable root filtering, existing-root template fallback, shadow-before-light script traversal |
| Host mutation observation | Native replacement excludes ShadowRoot from ChildrenReplaced public payload | Shadow-only insertion emits no host childList record; mixed insertion contains only light nodes |
| Fragment scripts and insertion | Native TreeSink already-started hook; shared post-connection script batch and host external/module queue | Inert defaults/moves/template extraction, inline/declarative scripts, full-tree visibility, removal of later scripts, filter precedence, CE reaction ordering, external/module captured request admission |
| Legacy innerHTML/outerHTML and insertAdjacentHTML | Contextual inert parser, post-conversion identity, staged native mutation, capacity preflight and retained resource batch | SVG/table context, synthetic-wrapper exclusion, template/shadow identity, inert reconnect, mutation grouping, conversion movement, detached/Document handling and retry identity |
| HTML rules on detached/XML documents | Inert Document parser; HTML fragment parser regardless of owner document type | Scripting-disabled noscript, noembed raw text, doctype mode, null browsing context, XML/SVG error recovery and ownership |

Paths above are relative to `crates/` unless explicitly a documentation or IDL source. Full change history and additional regression identifiers remain in `S01-BINDING-COVERAGE-MAP-2026-09-16.md`.

## Explicit remaining source obligations

- Staged frame/image/stylesheet/modulepreload preparation now shares retained batch admission with script targets. Multi-stage failure, suspension and retirement regression sources are generated. HTML setters now preflight the connected fragment root/script budget before replacement, preserving old children on capacity rejection; detached/inert targets are excluded. The generated regression checks preserved identity, no script execution, no leaked admission and a later successful insertion. Shared node_migration insertion arguments now preflight after hierarchy validation and before source adoption, with two-realm preservation regressions. Variadic methods now check capacity after their required staging and hierarchy validation, before destination mutation; all-six-method regression sources preserve staging side effects and destination identity. Legacy innerHTML/outerHTML/insertAdjacentHTML now use preflight and retained batch preparation. XML legacy fragments now use native namespace-aware XML parsing with pre-mutation syntax rejection. DTD/entity inheritance and the full XML matrix, fallible target capture and remaining per-producer admission still require source reconciliation.

- Complete the pinned HTML/sanitizer matrix-to-caller audit, including style visibility, all unsafe script scheduling modes, custom registry transitions, mutation observations, and failure/retry custody after replacement.
- The setDataAttributes instance-filtering edge is source-integrated: branded instance snapshots retain provenance and bypass dictionary reconstruction/validation at filtering, while raw dictionaries retain strict canonicalization. Exact modifier output and local removals are preserved. The specification's validity assertion still conflicts with its modifier-produced state; this is documented below, not treated as resolved specification wording or qualification.
- Review detached Document origin/base URL/encoding and parser script-adoption cases across every pinned helper, beyond currently generated bounded regressions.
- Reconcile any final WebIDL/schema generated output only after the global generation gate permits project generators.
- Build/test/fix, complete Core WPT, five families, eight frameworks and all plan qualification gates remain mandatory and unrun.

## Superseded routes

The previous unsanitized Document.parseHTMLUnsafe callback path is removed. DOMParser retains its separate inherited-URL, non-declarative parsing behavior. Sanitizer handler-name classification shares the existing Trusted Types inventory rather than retaining a duplicate list. Setter scripts use native post-connection preparation rather than a separate execution path.

### Independent XML fragment review — source only

Pinned `domparsing/createContextualFragment-xhtml.xhtml` exposed missing Range container flattening. The native Range fragment path now repeatedly promotes children of boundary-level HTML-namespace html/head/body elements, preserving order and foreign namespace elements. Containers under ordinary elements remain untouched. A native regression covers nested stripping, foreign controls and ownerDocument.

The XML legacy outerHTML path now honors the synthetic HTML body context for a DocumentFragment parent while retaining XML syntax rules. Added namespace/ownerDocument and malformed-input-before-replacement regression assertions. Serializer namespace validation findings were handed to the coordinator's separate serializer review; no serializer changes were made in this lane. All added regression sources remain unrun, and full source/qualification gates remain open.

## XML specification versus pinned compatibility evidence

The target remains pinned WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`; source expectations alone are not passing evidence.

| Case | Pinned source expectation | Current specification/source reconciliation |
| --- | --- | --- |
| Prefix alias after descendant rebinding | Reuse the old namespace's last prefix even though it is rebound | Implemented namespace-to-prefix alias lists; matches the published retrieval algorithm |
| Generated ns1 collides with authored ns1 | Emit the generated name without skipping, including duplicate declaration | Implemented pinned generation; explicitly documented by specification issue 44 |
| Attribute p:name in a new namespace, with unrelated xmlns:xx already present | Generate ns1 | Current published algorithm permits unused p; current implementation follows that algorithm. Pinned compatibility remains unresolved; no ad hoc special-case rule added |
| Doctype IDs containing quotes | Preserve double-quoted literal output | Current non-strict implementation matches pinned cases; published algorithm instead chooses quote delimiters. Strict getter path validates its output separately |
| Prefix bound to empty URI with redundant empty default declaration | Retain both declarations | Remaining pinned edge; strict original-record validation rejects prefixed undeclaration |

Native strict-character regression sources cover invalid XML Char/lone surrogate, comment hyphens, CDATA terminator, PI reserved/colon target and PI terminator, plus valid counterparts and unchanged non-strict serializer results. No execution.

### Modifier state versus authored configuration — source review

The current HTML `get-a-sanitizer-instance-from-options` algorithm configures dictionaries but returns native Sanitizer instances directly. Its sanitization algorithm asserts validity without specifying a new constructor-validation exception. `setDataAttributes(true)` explicitly removes data-* allow entries but retains per-element removals; those removals can then violate the separately stated explicit-global-subset validation rule.

Generated distinct native instance-snapshot filtering and fragment preparation routes. Sink snapshots carry branded-instance provenance through options acquisition after conversion; raw dictionaries still canonicalize and reject before mutation. Instance filtering preserves all local exclusions and exact modifier output instead of silently dropping policy. Added a source regression covering safe/unsafe element sinks, Document parsing, repeated modifier calls, disabling again, rejected raw snapshots and unchanged destination on rejection. No test/build execution occurred. This resolves the observable spurious instance-filter exception without claiming the contradictory validity assertion is settled.

Safe-sink mutation review is now source-integrated as described below. General parser/resource admission and qualification obligations above remain open.

### Safe-sink removeUnsafe mutation reconciliation

The HTML sanitization algorithm removes unsafe entries from the acquired Sanitizer instance itself. Pinned `sanitizer-removeUnsafe.html` establishes baseline removals and idempotency; no pinned safe-sink test establishes a conflicting unchanged-instance contract. Generated the shared sink filter to mutate the native branded instance only after fragment/Document parsing succeeds and before filtering. Safe script targets still return before sanitizer acquisition; conversion failures and unsafe sinks do not perform the mutation. Authored dictionaries produce separate configured objects and remain unchanged.

Element, ShadowRoot and Document paths now share this mutation point. Removed the superseded instance-specific parse-and-filter helper; a separate unfiltered native fragment parser lets the binding retain instance identity through parse completion. Added regression sources for all three safe sink families, unsafe/no-op/conversion-failure controls, raw dictionary nonmutation and removeUnsafe idempotency. Source-only: tests and builds remain deferred.

### Shadow target registry review

Independent source comparison against `custom-elements/registries/scoped-registry-define-upgrade-order.html` found that both shadow markup setters reused the parsing host as the registry source. The parser needs host tag/context, but imported children need the actual target ShadowRoot's scoped/null registry. The coordinator owns the production correction alongside retained pre-mutation capture. `sanitizer_registry_tests.rs` adds source regressions for legacy/safe/unsafe sinks, conflicting global/scoped definitions, explicit null and delayed-definition order. The sanitizer scoped-constructor negative fixture alone does not detect wrong-registry upgrades, since its elements are removed before construction.

Read-only follow-up checked getHTML dictionary member order, retained root wrappers through iterator conversion/adoption, and explicit/serializable shadow selection, without identifying another concrete defect in those inspected paths. It also checked inert template-content registry separation and declarative null-registry markers. This bounded review is not an exhaustive HTML/parser matrix result. DTD/entity-context inheritance, full XML setter matrix and remaining admission/custody rows above are still tracked independently. Tests remain unrun.

### Script children-changed producers — source generation in progress

Pinned `html/semantics/scripting-1/the-script-element/emptyish-script-elements.html` requires direct Text insertion into connected unstarted scripts to prepare them, while excluding descendant/comment payload and already-started scripts. Shared insertion/replacement capture now admits a deduplicated direct-parent script job, covering generic and variadic destination insertion. New `script_children_changed.rs` adds Text/CDATA-only CharacterData-parent capture. CharacterData/data/nodeValue/textContent, innerText/script.text, splitText, insertAdjacentText and explicit removeChild/remove paths capture before mutation and use retained finish_mutation. Receiver identity is refreshed after conversion, including cross-arena text adoption.

Regression sources cover direct setters/insertion, descendant/comment exclusion, inert scripts, preparation failure after mutation with one retry, and conversion-time adoption to another script realm. All remain unrun. Full children-change closure remains open for old-parent notification ordering during implicit child moves/staging, normalize and Range mutations; those require careful mid-mutation author reentrancy integration rather than merely firing delayed notifications after destination commit.

### XML template-content traversal — September 17 source review

The DOM Parsing XML element algorithm explicitly serializes HTML template contents instead of ordinary children. The native XML serializer previously ignored the captured template reference. Both ordinary serialization and strict record validation now follow `SerializationResolver::template_content`, retaining namespace scope and arena-qualified identity. The existing binding resolver already captures local and externally owned template contents for XMLSerializer and legacy outerHTML.

Added native regression sources for nested local templates, external-arena content, foreign-namespace template names, ignored ordinary children (including invalid XML comments), strict rejection of invalid actual content, non-strict emission, and ownership cycles. No execution; XML namespace compatibility differences and full S-01 qualification remain open.

### XML Attr dispatch and binding integration — September 17

Pinned `domparsing/XMLSerializer-serializeToString.html` explicitly requires an Attr root to serialize as the empty string. Native dispatch now implements that rule instead of emitting the attribute value; element attribute serialization retains its separate escaping path. Expanded the XMLSerializer binding regression source with nonempty attached/detached Attr roots, escaped owner attributes, nested template contents, XML-document adoption, strict outerHTML validation of invalid template comments and non-strict XMLSerializer emission. All regression sources remain unrun.

### removeChild return identity across callback adoption — September 17

Independent caller review found that `removeChild` reconstructed its return wrapper from the pre-callback owner and node number after running script children-changed preparation. It now returns the original Node argument, preserving WebIDL identity after the callback adopts and reinserts it into another arena. Added a two-context/two-arena regression source asserting removal-before-callback, one callback, exact return identity, destination ownerDocument and parent. Source only; no execution. Self-replacement continuation and remaining Range/normalize producers remain under the mutation agent's review.

### splitText return and post-mutation failure custody — September 17

`splitText` now captures the newly created wrapper and synchronizes live-range state before children-changed callbacks. Its return therefore follows adoption of the new sibling. Added a cross-arena callback regression alongside the removeChild identity case. Shared retained preparation now accepts binding errors as well as native mutation errors, preserving connected pending work without executing author code at the failure boundary; the existing connected-prefix regression now also injects a return-wrapper allocation error and checks one later delivery and released admission. Sources only; no execution.

### importNode dictionary reentrancy — September 17 source review

`importNode` validates its Node argument before converting options, then resolves the original source and receiver wrappers again after all options getters. Unsupported Document/ShadowRoot checks now follow argument conversion. This prevents stale source-arena reads when a getter adopts the source and preserves dictionary exception precedence. Added two-arena regression sources for adoption plus content mutation, destination ownership and distinct clone identity, single getter conversion, unsupported-node ordering and original thrown-value propagation. `upgrade_for_document` queues upgrade reactions rather than executing constructors in this caller; no speculative clone return-path change was made. No execution.

### Attribute argument conversion and current receiver — September 17

Native getAttribute/getAttributeNode/hasAttribute/removeAttribute/toggleAttribute and their namespace variants now resolve the original receiver after string argument conversion, before normalization, lookup or mutation. This follows cross-arena adoption during conversion and preserves namespace-before-local-name conversion order. `toggleAttribute` now invokes the shared resource-change hook when it actually changes an attribute, consistent with other attribute mutations. Expanded the two-arena binding regression source with all nine operations, argument conversion counts/order, Attr owner identity and resulting values/removals. Setter Trusted Types conversion and other DOM string-argument callers remain separate review scope. No execution.

### Attribute setter conversion and publication owners — September 17

setAttribute/setAttributeNS now complete the Trusted Types-or-string union conversion before resolving the receiver and normalizing/validating names. They resolve the original wrapper again after default-policy processing before publishing. Thus adoption during name/value conversion cannot leave a stale arena reference or apply HTML lowercasing based on the previous Document. Name/namespace validation precedes sink policy processing but follows argument conversion. Added two-arena HTML-to-XML adoption regression sources for each setter and each conversion stage, including exact conversion order, uppercase attribute preservation and final owner. Native trusted union values remain branded; raw strings convert once. No execution; broader Trusted Types policy and setter matrix review remains open.

### Selector and collection receiver conversion — September 17

querySelector/querySelectorAll, matches/closest, tag/namespace/class/name collection factories and getElementById now resolve their original receiver after string conversion. This prevents stale-arena lookup and constructs live collections against the adopted root. Added cross-arena regression sources covering selected-node identity, self matching/closest, static querySelectorAll versus live HTMLCollection updates, namespace lookup and DocumentFragment ID lookup. No execution; whole S-01 qualification remains open.

### Dynamic script children-changed: insertion and Range source continuation

Source generation now includes typed insertion continuation after old-parent script preparation: original parent/reference/replacement wrappers survive reparenting and adoption, including self replacement after observable removal. Direct Text/CharacterData mutations, insertAdjacentText, generic/variadic insertion, Range insertNode and normalize use retained preparation before mutation and synchronize native live ranges before author callbacks. Parser-inserted scripts are excluded from children-changed preparation until parser preparation clears their parser-document association.

The dedicated `range_contents_observable.rs` collector now drives direct Range clone/extract/delete/surround dispatch. Initial selection and doctype preflight precede source mutations; selected wrappers survive callback-driven adoption; extraction removal runs before destination insertion; partial data changes and normalize concatenation expose the required intermediate tree to script preparation. New regression sources cover those orderings, self replacement, original reference invalidation, cross-arena destination/source changes, normalize boundary remapping and Range direct-script insertion. Sources remain unexecuted.

Remaining generation is explicit: native editing line-join extraction, insert_editing_element extraction and replace_selection_with_text deletion still call scope-free native Range collectors. Their run_edit_transaction rollback model must be reconciled with author callbacks before routing them through the observable collector, so author side effects cannot be erased by rollback. General mutation continuation after infrastructure failure and the full editing/Range producer inventory remain open; the existing retained queue preserves committed mutation preparation but does not establish replay of an interrupted multi-step algorithm suffix. No full S01/Range qualification claim is made.

### Trusted Types DOMString preservation — September 17

Policy names, HTML/Script policy results, Trusted Type stringification and raw sink output use native lossless DOMString helpers. Pinned trusted-types.idl declares CreateScriptURLCallback as USVString: ScriptURL policy results (including default-policy results) deliberately replace lone surrogates with U+FFFD through an explicit shared conversion. This scalar conversion is not applied to HTML/Script results or ordinary raw attribute strings. Added regression sources for all three Trusted Types, lone high/low surrogates, supplementary pairs, NUL, policy name reflection, branded attribute sinks and default-policy attribute sinks with once-only callback counts. Detached script/iframe attributes avoid executing supplied strings. No execution; full Trusted Types conformance remains unverified.

Pinned Trusted Types interface follow-up: generated the required toJSON IDL operations and Window/Worker exposure for all five Trusted Types interfaces. Native prototype dispatch now shares exact branded serialization for toString/toJSON; removed per-instance toString replacement. Factory/empty-value installation now runs for Window and Worker, excluding AudioWorklet. Added Window/Worker regression sources for prototype placement, cross-brand/proxy rejection, native JSON payload despite an authored toString override, scalar ScriptURL output and empty values. No generator/build/test execution; combined generated-output reconciliation and complete policy/factory descriptor/defaultPolicy/cross-realm review remain open.

### Trusted Types default policy identity — September 17

Generated readonly defaultPolicy IDL and native accessor, backed by the exact default policy object retained in the policy registry. The factory now carries a private native owner brand; defaultPolicy and createPolicy reject forged/proxy receivers before conversion and use the receiver's native owner. Added Window/Worker regression sources for initial null, stable identity, readonly assignment and invalid receiver conversion order. Full factory prototype placement/remaining method brands, cross-realm policy allocation and policy publication failure custody remain open; no execution.

### Native factory prototype surface — September 17

Removed per-instance TrustedTypePolicyFactory method replacements. Generated prototype dispatch now brand-checks all six operations before arity/argument conversion and calls native implementations. Added readonly emptyHTML/emptyScript IDL/accessors with stable values held in private factory slots; removed writable own constants. Extended Window/Worker sources for prototype placement, forged/proxy receivers and readonly/stable empty values. Global trustedTypes and policy name/method descriptors, cross-realm allocation and failure custody remain under review. No execution.

### Policy prototype identity and admission — September 17

TrustedTypePolicy now has native private identity, a readonly prototype name getter and receiver-resolving prototype createHTML/createScript/createScriptURL methods. Removed function-token/per-instance methods and writable name properties. Factory result and policy-created values allocate in the receiver's creation realm. Policy object preparation and checked ID admission precede name reservation, preventing a failed allocation/capacity step from consuming the name. Added Window/Worker sources for method borrowing, forged/proxy rejection before conversion, readonly name and identity-exhaustion retry. Full cross-realm/prototype-tampering and global factory getter review remain open; no execution.

### Trusted prototype and value-ID custody — September 17

Trusted Types installation now captures the five native prototypes before authored execution. Later factory/policy/value allocation uses those retained prototypes instead of reading authored constructor globals. Value IDs use checked advancement and publish records only after object/prototype preparation succeeds. Added Window/Worker regression sources that replace constructor globals with throwing getters, verify native result prototypes, exhaust value identity capacity, and confirm old payloads and registry size survive rejection. Source only; no execution.


### Native global factory and nullable callbacks — September 17

Replaced the writable trustedTypes global data property with a readonly, enumerable, configurable native accessor. A realm context slot retains the factory independently of author replacement; receiver validation and Window access checks precede retrieval. Added Window/Worker sources for descriptor shape, stable identity, strict assignment, forged/proxy receivers and property replacement/restoration.

Aligned policy callback dictionary members with the pinned IDL: absent/undefined members remain absent, explicitly null members fail callable conversion, HTML/Script callback results are nullable DOMString and ScriptURL is nullable USVString. Explicit policy calls map null/undefined callback results to the empty trusted payload, as covered by pinned TrustedTypePolicy-createXXX.html; default-policy sink rejection remains separate. Added all-three-kind null/undefined-result and failed-name-reservation retry regression sources. No build, generator or test execution; full cross-realm and remaining binding audit stays open.


### Cross-realm Trusted Types regression generation — September 17

Added a two-independent-realm source fixture for borrowing the global getter, factory createPolicy, policy create methods and value serializers. It checks destination native prototypes, independent default policy registries, cross-realm brand predicates and exact registry counts. Source inspection also removed a nested mutable scope borrow in fixture setup. These are generated assertions, not runtime evidence; complete exposure placement and cross-origin behavior remain subject to later qualification.


### Policy admission and configuration lifetime — September 17

Pinned TrustedTypePolicyFactory-createPolicy-unenforced.html requires duplicate ordinary names without an enforcing trusted-types directive. Updated initial binding configuration and native CSP compilation for this case, including require-trusted-types-for-only and report-only policies. Policy configuration updates now preserve the native created-name history, preventing a later configuration refresh from admitting a second default policy or forgetting earlier names. Generated native CSP/registry and Window/Worker independent-callback regression sources. Reconciled the earlier dictionary-order fixture to use undefined for an absent callback; explicit null is now correctly rejected. No execution; broader CSP reporting and full policy qualification remain open.


CSP name-set follow-up: the pinned mixed-none fixture allows `trusted-types * 'none' 'allow-duplicates'`. Updated compilation so quoted none adds no names instead of overriding wildcard/named allowances. Intersections now omit unconstraining wildcard name sets, retaining the names permitted by other enforcing policies. Generated multi-policy wildcard/named/mixed-none/sole-none regression sources; unexecuted.


### Reflected Trusted Types property sinks — September 17

Source review found script.src and iframe.srcdoc using generic string reflection without policy enforcement. Added native union conversion, current-owner policy lookup, post-policy receiver resolution and resource notifications for both properties. Standardized default-policy sink labels for script text, legacy HTML setters and insertAdjacentHTML. Generated required-policy rejection, callback kind/sink arguments, thrown-value identity and unchanged-property-on-rejection sources. Complete reporting/default-policy reentrancy and cross-document sink provenance remain open; no execution.


Property conversion follow-up: pinned HTML IDL distinguishes script.src USVString from iframe.srcdoc TrustedHTML-or-DOMString. Reconciled those declarations and added pre-policy scalar conversion for ordinary script URL inputs, with lone-surrogate callback/storage regression sources alongside lossless srcdoc behavior. Removed the superseded unreachable legacy HTML conversion branch; sanitizer-owned legacy setters remain the active route. No generator/build/test execution.

Explicit reporting gap: trusted_string currently rejects a failed required sink without authorizing/reporting through per-policy native CSP records; the aggregate TrustedTypesConfig drops report-only requirements and original-policy/report destination evidence. Full implementation must preserve per-policy disposition, report on failed default-policy fallback, retain original-document task/report custody, and integrate Window/Worker/standalone/parent-owned paths. Existing native CSP event queues provide delivery but do not establish this missing decision/report producer. Keep qualification open until source generation and the pinned default-policy event-count/report-only matrix are complete.


### Per-policy Trusted Types decision generation — September 17

Native CspPolicySet now provides a report-only-aware sink-check query and separate sink-failure/policy-name decisions. Each decision retains individual original policies/dispositions and standard blockedURI/directive/sample fields, with 40-unit source clipping and lossless native DOMString storage. Generated sources cover report-only acceptance, combined enforcement, independent report counts, duplicate-policy reports and clipping. These producer methods are not yet wired into binding/host/worker delivery; S-01.trusted-policy-reporting remains open. No execution.

### Native editing author boundaries and inverse custody (generation in progress)

The formerly scope-free production Range consumers now have an owned Rust continuation path: `editing_algorithms` and `editing_range` suspend at native text/removal/insertion/split operations; `editing_continuation` owns the operation, mutation-committed flag, inverse-captured flag and retained children-changed preparation. Suspended state contains native leases and owned Rust values, with DOM guards dropped before awaits. Marker ranges have explicit native Drop custody.

Production admissions now include execCommand, keyboard edit receipts, clipboard cut/paste, semantic edits, composition updates, drop insertion, same-document rich moves and cross-document drag moves. Old closure-only editing entrypoints are test-only. Native platform polling/retirement owns direct-command retries through a weak per-state registry, while input receipts own their own continuation. Cancellation drops remaining operations and inverse roots.

`editing_inverse` applies only native-owned fields and topology edges still matching the expected segment, preserving author-added children and callback adoption. Segments are captured before authored preparation and compacted to changed records; a fresh post-callback baseline excludes author changes from later undo. Composition retains segments through commit/cancel; paired drag halves refresh the destination baseline after source callbacks and publish paired inverse ownership. Generated regressions cover failures before and after author preparation, no callback replay, undo/redo preserving author additions, cancellation, capacity rejection before mutation and interleaved changes between paired document halves. All are unrun.

Source review remains open for helper-level mutation coverage, inverse topology conflict/reorder completeness, callback-driven document changes, and all bounded failure paths. This appendix supersedes the earlier statement that the four main editing Range producers have no continuation route; it does not establish whole editing/S01 conformance or successful compilation.


### Native reporting hook and transport integration — September 17

Generated typed PolicyCheck requests and binding hooks for report-only-aware default-policy processing, failed sink fallback, and policy creation after dictionary conversion/allocation. Parent-owned Windows select CSP by exact creator generation/stream epoch and route violations through existing native reports; standalone hosts use retained local policy/report queues without overriding parent authority. Workers use their own agent transport/identity and native worker report queue. Generated binding hook sources distinguish report-only acceptance, enforced rejection, successful default conversion, thrown callback identity and genuine-brand bypass. No execution. Parent/worker transport regression inventory, report retry/retirement behavior, remaining sink labels and full source-location metadata remain open; these integrations do not yet close S-01.trusted-policy-reporting.


### Trusted Types transport and attribute sink regression generation — September 17

Added parent-service source coverage for report-only frame requirements, independently enforcing/reporting policy-name decisions, exact report destination creator, unchanged root event stream, stale epoch rejection, cross-page rejection and frame retirement. Attribute mutation paths now share native sink labels for HTML iframe/script, SVG script and generic event handlers, including Attr and setAttributeNode. Generated a four-interface/four-mutation callback matrix against pinned attribute reporting labels. Source only; worker/standalone delivery and retry/retirement qualification remain open.


### Worker reporting fixture and explicit realm policy — September 17

Generated a native Worker fixture with enforcing and report-only response policies. It checks successful report-only policy creation, rejected enforcing admission, exact trusted violation identity/sample/disposition order, all reports before message/close and registry retirement. The fixture uses the actual native agent service rather than a permissive transport mock. Binding source now retains explicitly configured realm enforcement when an installed host hook reports no applicable sink directive, preserving embedding-configured policy tests; report-only native allowances apply when the host actually required sink processing. No tests/builds executed; fixture and full delivery/retirement coverage remain unqualified.


### Conversion-time policy owner and text storage — September 17

Legacy HTML setters and script text setters now finish string/union conversion before resolving the receiver's native policy owner, and still resolve publication ownership again after policy callbacks. Unsafe HTML setters likewise re-resolve after option conversion. Added independent-realm adoption regressions for HTML/script text. Nullable Node textContent/nodeValue normalize null/undefined to empty; text/CharacterData storage preserves UTF-16, with node-kind/surrogate/NUL source regressions. Native script detection uses actual HTML/SVG namespace/local name. Complete script child-mutation trust provenance is newly confirmed as missing: preparation must distinguish trusted setter text from untrusted DOM child changes and invoke/report default policy appropriately. This remains source work, not a completed gate; no execution.

### Editing helper and inverse source review — September 17

Generated retained mutation paths for the remaining confirmed helper producers in the owned editing algorithms: root-boundary merges, nested-line deletion, remove-format unwrapping, alignment cleanup/wrapping and range-preserving moves, list replacement, and paragraph placeholder insertion/removal. Helpers now await owned native operations, with DOM guards released; this replaces their direct structural/text mutation calls. Attribute-only native edits remain within the next captured native segment.

Range-preserving moves retain the original endpoint records and the native post-removal records. Destination insertion restores only endpoints whose records still match the native removal result, before destination script preparation. Callback-written ranges remain authoritative. Original node leases reject adoption into another arena; insertion prevalidates before source removal and uses a bounded reparent-removal loop with original reference custody. New regression sources exercise callback range changes, callback adoption rejecting an upcoming native edit, and remove-format moving text into a connected script.

Inverse compaction now retains complete subtree witnesses for edges that undo/redo can remove. This supports ordinary removal of unchanged native insertions while preserving callback-added descendants. Attribute restoration compares each namespace/local-name independently, and unchanged controls no longer acquire inverse authority; changed control flags and selection use separate expected-value checks. Dedicated source regressions cover subtree evidence, author-added children, independent attributes and untouched controls. Marked composition accumulation has a bounded interruption path; compound cancellation records committed native work before dropping the remainder.

**Still open:** full inverse reorder/partial-publication source review, paired-document terminal/capacity recovery, direct authored Range collector infrastructure-error suffix retention, exhaustive helper/caller qualification, compilation and execution. No source-generator/build/test command was run. These bounded additions do not close S-01 or the complete implementation gate.


### Remembered script text and preparation — September 17

Read the current Trusted Types enforcement algorithms alongside pinned script-enforcement-001/003/005 fixtures. Native script records now retain accepted script text; parser end-tag handling seeds it, while clone/import creates a fresh empty slot and adoption preserves the record. Script-specific innerText/textContent metadata and dispatch are distinct from borrowed Node/HTMLElement setters. Preparation compares remembered text with actual direct-child text, runs default-policy processing before empty-source rejection, absorbs policy exceptions as required by script preparation, and executes the accepted source without rewriting DOM child text. Post-callback connectivity, owner lifetime and already-started state are rechecked. Generated parser/clone and binding sources cover splitting/normalization, borrowed setters, clone trust loss, default source replacement, empty-source conversion and no repeated conversion of accepted setter text. No execution. XML/parser mid-parse mutation provenance, callback interruption/retirement, and full sink/preparation integration remain under review; no gate closure claimed.

### Script preparation integration follow-up

Source inspection confirmed XML EndElement calls the shared OrynTreeSink::pop hook, including HTML and SVG namespace scripts. Added `xml_script_close_remembers_parser_text_before_author_mutation` to cover parser-close trust before an author mutation and subsequent parser completion. Fixed HTMLScriptElement.innerText receiver capture so its script-specific conversion path can resolve the native receiver. Script preparation now also revalidates document authority epoch after default-policy callbacks, alongside document root, generation, stream epoch and tracked node identity. Existing script preparation regression source exercises the own innerText setter. No builds, generators or tests executed. Full script producer coverage, parser interruption/adoption and reporting custody remain open.

Added `trusted_script_preparation_rejects_callback_replaced_document_authority` regression source for stream replacement, authority replacement and retirement during default-policy conversion, asserting no remembered trust publication, started bit or dynamic admission. Further source tracing found a concrete remaining integration gap: native `rust_executable.rs` parser ScriptReady admission and `frame_script_preparation.rs` XML admission use `script_record` raw child text rather than the dynamic preparation helper. These paths require shared trusted-source preparation before source/type/empty checks and scheduler admission; execution-time-only enforcement would be too late and could reconvert already prepared source. Checklist reporting row remains open and now records this dependency.

### Editing exception and inverse retry reconciliation — September 17

Synchronous authored Range methods differ from owned input receipts: once a method throws, its unperformed mutation suffix is aborted rather than resumed later. Existing retained insertion preparation keeps the committed mutation's resource/script work. Directly created clone completion records now have explicit background eligibility and a native platform completion-only poll/retry deadline, excluding input-receipt-owned clone jobs; suspension pauses delivery and retirement releases the records. Generated fault sources cover delete/extract/surround leaving later selected nodes and the surround wrapper untouched after a thrown error, then completing only the committed script preparation, plus clone completion failure and suspension. This resolves the earlier proposed “direct Range suffix retention” requirement by making the synchronous exception boundary explicit, rather than introducing out-of-order mutations after an exception.

Inverse application now shares a native progress cursor across history-unit clones. It records actual detach/insert commit before surfacing an error, resumes the remaining edges and rejects intervening author reparenting. Selection restoration distinguishes author updates from native range rebasing. Control value and associated dirty/pending/edit flags are restored as one ownership group; generic node-value restoration no longer bypasses that guard. Regression sources cover failure after detachment, retry via a cloned segment, author control changes and subsequent value-attribute updates.

Independent paired-edit review also generated a terminal-error latch: finish committed half publication, stop the opposite unperformed half, and surface the original typed error once. Oversized duplicate control-history metadata falls back to the existing bounded native segment inverse without partial control markers; immutable metadata capacity no longer causes endless retries. All additions remain source-only. Compilation, runtime fault injection, full inverse conflict matrix and broad conformance qualification remain open.

### Native parser trusted-source integration

Extracted `oryn_js::v8_bindings::prepare_script_text` and added the active-realm host bridge with wall-time handling. Native HTML nonblocking and blocking ScriptReady paths, XML frame admission, buffered root scans and both frame bootstrap scans now call this shared conversion before admission. Callers retain accepted source in ScriptRecord and reread attributes after callbacks. Snapshot enumeration holds native leases across callbacks and rejects moved document ownership. Shared preparation ignores already-started scripts, including reentrant starts during a policy callback. Added `parser_snapshot_uses_trusted_source_without_rewriting_dom` for required-policy rejection, default conversion, callback type changes, unchanged DOM child text and captured accepted source after later mutation. No builds or tests executed. Preparation retry custody and parser scheduling changes during policy callbacks remain open; this integration is not full Trusted Types qualification.

### Shared HTML/SVG external script source

Added native `DomRuntime::script_external_source`: HTML uses null-namespace src; SVG uses null-namespace href first (including an explicitly empty value), then XLink href. Integrated parser scheduling, parser-move checks, native ScriptRecord capture, dynamic preparation and document.write inline classification. SVG href/XLink href mutations now reach dynamic script preparation, while already-started state prevents a second admission. Added `svg_script_source_prefers_href_and_preserves_empty_attribute` and `svg_href_attribute_changes_admit_external_script_once` regression sources. No tests/builds/generators ran. Full SVG script interface/reflection and scheduling semantics remain subject to source review.

Further source issue to resolve: `child_text_source` intentionally scalarizes UTF-16 before Trusted Types comparison/default callbacks, while script-specific setters retain lossless DOMString values. Parser trust seeding and Trusted Types input collection must preserve code units through policy processing; source compilation consumers must then handle those units consistently. This remains an open integration item, not an execution result.

### Lossless policy input collection

Added bounded `child_text_dom_string` alongside scalar `child_text_source`. Parser remembered text and shared Trusted Types preparation now use original UTF-16 units, including split surrogate pairs and unpaired surrogates. Only after policy conversion and trust comparison does the existing scalar executable transport conversion run. Added `script_policy_receives_lossless_child_text_before_source_transport` to verify exact callback units and no repeated policy conversion when the remembered source matches. The existing direct-child collector regression also checks lossless output. No execution. Full lossless executable transport/compilation remains open: that change needs explicit source provenance so internally encoded DOM strings are decoded without accidentally interpreting ordinary fetched source as an internal encoding.

### Trusted Types report source locations

Native PolicyCheck now carries a serde-default source URL/line/column for policy admission and failed sink conversion. V8 captures the top native stack frame without reading authored Error or stack properties. Window and Worker service payload limits include the source URL; CspPolicySet transfers the location to each individual violation after applying the existing report URL stripping rules. Existing original-policy/disposition/destination custody is retained. Added regression sources for native frame location, per-policy source metadata/redaction and wire defaults/roundtrip/payload accounting. No builds or tests ran. Report failure/retry custody, full pinned event matrix and protocol/profile qualification remain open; source metadata by itself does not close reporting.

### Worker CSP event custody

Removed eviction of the oldest unread Worker CSP event at the existing 256-event bound. Native registry admission now checks the entire policy-violation batch and reserves queue storage/identity range before publishing its first event. Capacity or identity exhaustion leaves existing unread events and the sequence unchanged. Worker polling acknowledges only events up to the native consumer cursor and replays unacknowledged events. Added regression sources for full-batch capacity rejection, replay/acknowledgement/reclaimed capacity, and identity exhaustion without partial publication. No tests executed. A bounded queue can still reject admission; network-report queuing failures after event publication and broader Window transport custody remain open and require retained progress handling.

### Window report partition preflight

Window CSP reporting now resolves all report targets and captures the page partition URL before publishing the first violation event. Removed repeated post-publication page URL lookup. Added `csp_report_partition_failure_does_not_publish_event_prefix` with multiple policies and endpoints, verifying a missing page fails before any event publication. No execution. This closes that specific partial-publication path; queue-lock failure after event publication, request/response loss and full retained report progress still require implementation/reconciliation.

### Native editing growth admission and DOMImplementation reconciliation — September 17

Remaining production attribute/style/control/typing-override mutations now use typed owned operations with growth admission before native mutation. Snapshot accounting includes typing-override strings. Newly prepared detached imports/clones stay outside inverse authority until admission; their exact subtree snapshots (including a new insertion parent) are charged before mutation. Rejected detached preparation therefore cannot make the previous connected mutation's inverse disappear. Owned clone completions hand off completion-only custody when a receipt is canceled/dropped. Added oversized attribute and detached import regression sources preserving an earlier committed text inverse. No execution.

Pinned `dom/nodes/DOMImplementation-createDocumentType.html` explicitly accepts empty, digit-start, punctuation and colon combinations; retained the existing relaxed delimiter validation rather than adding XML QName restrictions. `interfaces/dom.idl` requires all three doctype strings and a DocumentType-typed optional createDocument argument. Updated IDL and native dispatch accordingly, preserving UTF-16 conversion and explicit-undefined conversion. A dedicated `dom_implementation` module now adopts a real foreign doctype through the native migration publication path, preserving wrapper identity/source removal, and derives the new document provenance before adoption. Generated same-/cross-arena, wrong-brand/error-order, title omission, metadata and relaxed-name regression sources. Existing optional-doctype test helpers were corrected to provide required public/system arguments. Binding regeneration and all execution remain pending; this bounded factory group does not close the full parsing/S-01 gate.

### CSP event conversions and standalone report admission

Authored SecurityPolicyViolationEvent string dictionary members now preserve UTF-16 through native storage; added regression sources for lone surrogates, supplementary characters, null DOMString conversion, integer modulo conversion and enum rejection. Standalone report delivery now classifies malformed endpoints, exhausted request identities and executor admission rejection as recorded delivery outcomes, consistent with its existing size/queue/rate limits. Executor rejection releases the report permit and does not propagate failure into an already-admitted violation event/policy check. Added fault-injected executor admission and malformed-endpoint regression sources. No builds/tests/generators ran. Poisoned report-state handling, parent report job admission and complete pinned reporting qualification remain open.

### Parent report job admission and cleanup

Parent native CSP report jobs now use the shared bounded resource IO executor, replacing one std::thread::spawn per report. A native permit owns pending admission and releases it on success, queue rejection or unwinding. Executor rejection records queue_failed without retrying the policy event; request IDs use checked allocation and report identity_exhausted instead of wrapping. Added regression sources for injected executor rejection/diagnostic/pending release, checked request identity and panic cleanup. No builds/tests/generators executed. Full report transport qualification, poisoned-state policy and lifecycle cancellation remain open.

### Processing-instruction factory and lossless delimiter checks — September 17

Pinned `dom/nodes/Document-createProcessingInstruction.js` requires XML Name validation, including distinct start/continuation character sets, and rejection of `?>` in data. The previous binding created every target/data pair without validation. `document_processing_instruction.rs` now performs both required DOMString conversions before algorithm validation, enforces arity, validates decoded UTF-16 target scalars against XML Name ranges, and rejects the data delimiter in actual code units. The existing relaxed element/attribute/doctype validators remain separate. `browser.webidl` now declares the actual ProcessingInstruction result rather than Comment; generated bindings still await the global generation/execution phase.

The same review found CDATA's `]]>` test operated on the private lossless-string representation, missing the delimiter when an unpaired surrogate was also present. It now checks UTF-16 code units. Regression sources cover pinned target cases, range boundaries, lone surrogates, data-conversion exception precedence, required versus explicit undefined arguments, HTML/XML owner documents, native brands, and PI/CDATA delimiters adjacent to unpaired surrogates. These are source-generated, not executed; full S-01 inventory and qualification remain open.

Pinned `interfaces/dom.idl` also confirms `[NewObject] CDATASection` rather than Text and a required data argument. Both the IDL return and native pre-conversion arity check are corrected; regression sources distinguish omitted arguments (TypeError) from explicit undefined and preserve HTML document NotSupportedError behavior after conversion.

### Required arguments across native Document factories — September 17

Pinned `interfaces/dom.idl` declares required names/data for createElement, createElementNS, createAttribute, createAttributeNS, createTextNode, createComment, createCDATASection and createProcessingInstruction. Native dispatch previously let missing names/data pass through DOMString conversion as `undefined`. Shared `document_factory_arguments::require` now admits the required count before any authored conversion, and PI/CDATA use the same helper. Explicit undefined still converts to the string; nullable namespace remains null. Dedicated native regression sources cover omitted arguments, supplied first-argument objects whose conversion must not run when the second argument is absent, explicit undefined/null, UTF-16 data, namespace/name conversion order and actual owner Documents. No generated binding regeneration or execution ran.

The bounded DOMImplementation, PI/CDATA and required-argument rows are now in the granular S section (moved from appendices where applicable), with the S table recounted from Markdown rows. Namespace algorithm completeness and full S-01 qualification remain open.

### Internal subset source reconciliation

Inspected pinned `domparsing/DOMParser-parseFromString-xml-internal-subset.html` against xml-rs Doctype handling and native parser metadata. The parser already takes name/public/system IDs from doctype_ids and expands internal entities, while XMLSerializer deliberately omits internal subsets. Extended the native MIME/doctype regression to assert expanded text and added binding-level sources for expansion, exact serialized element/doctype, IDs and ownerDocument. No implementation absence is claimed for that bounded case. XML fragment DTD-context requirements and the broader parser matrix remain separately open; no tests executed.

### Policy admission matrix source review

Reconciled pinned `TrustedTypePolicyFactory-createPolicy-non-tt-policy-name.html` and `should-trusted-type-policy-creation-be-blocked-by-csp-001.html`. Policy creation deliberately accepts arbitrary DOMString names (including empty/control/non-ASCII names) absent an applicable restriction; no CSP-token grammar check was added to the factory. Added an explicit native eight-configuration/four-attempt matrix for named and wildcard policies, duplicate allowance, quoted none mixed with names, enforcing/report-only combinations, per-policy violation identity and failed-attempt name history. Expected decisions/report counts are written independently of the implementation. Sources remain unexecuted; the full event-task/realm/reentrancy matrix stays open.


## CSP directive grammar and report-accounting failure custody

Source generation, not executed qualification:

- `csp.rs::trusted_policy_name_token` implements the Trusted Types named-directive ABNF independently of the unrestricted `createPolicy` DOMString API. Both individual policy decisions and the aggregate named set use it. Wildcards still admit arbitrary API names. Generated regressions cover invalid explicit tokens, valid punctuation, unquoted keyword-looking names, wildcard admission and report-only decisions.
- Standalone and parent reporting now treat poisoned report accounting as a recorded `queue_poisoned` delivery failure. They preserve poison and admit no new IO; failure does not escape after a violation event has already been published and therefore does not request replay of the policy operation. Standalone outcome recording and permit cleanup recover only their bounded diagnostic/accounting fields from a poisoned guard.
- Generated standalone regression poisons admitted accounting, releases its permit, and attempts two report targets without IO or propagation. Generated parent regression preserves both enforced/report-only violation events and records two rejected deliveries after poisoning.
- Remaining: parent report-job lifecycle cancellation, complete Window transport/event reentrancy matrix and source preparation caller closure; all builds and execution gates remain deferred.


## Parent report-job context lifetime

Source generation, not execution evidence:

- Parent report closures now retain a weak context reference. `CspReportLifetime` is owned by the context; its drop cancels a separate token retained by jobs. All three constructors install this owner. Terminal `retire_workers` marks the same token before asynchronous cleanup; later report admission is ignored.
- Report response collection uses the existing cancellable native operation path, releasing its RAII admission on completion/cancellation. The ten-second deadline starts at queue admission, including executor waiting. Diagnostic publication upgrades the weak context only after response completion.
- Generated `dropping_context_cancels_report_operation_without_retaining_context` covers a pending native response, context destruction, cancellation and released accounting. Generated `retired_context_does_not_admit_new_reports` covers explicit terminal retirement and no new endpoint/pending accounting.
- This establishes bounded context lifetime source coverage. Page/document-specific retirement semantics, complete Window event transport and reentrant policy counts remain under review; no execution or whole S-01 closure is claimed.


## CSP Reporting API caller reconciliation

Pinned source evidence: `content-security-policy/reporting-api/reporting-api-report-to-only-sends-reports-to-first-endpoint.https.sub.html`, `report-to-directive-allowed-in-meta.https.sub.html`, and both `reporting-api-report-to-overrides-report-uri-{1,2}.https.sub.html` fixtures.

Generated changes:
- `report_targets` selects only the first `report-to` token; a missing endpoint does not fall back to `report-uri` or later tokens. Meta installation and duplicate detection preserve `report-to`, while continuing to remove the disallowed legacy directive.
- Added enforced/report-only, directive-order, unresolved-group, first-token and meta installation regression sources.
- Retained native CSP publication now enqueues a `csp-violation` ReportingObserver record exactly once when event preparation commits, before callbacks. Preparation failure emits no report; resumed listener dispatch does not replay report publication. Both local and transported Window/Worker CSP payload variants share the hook.
- CSP report JSON retains exact DOMString UTF-16 units, including lone surrogates. Unknown source-file/line/column values use null in report bodies. Existing observer filtering, buffered delivery and takeRecords apply.
- Added native publication regression source covering preparation/callback interruptions, one report, type filtering, buffered observer delivery, nullable source fields and lossless sample/toJSON values.

All changes remain unexecuted. Complete Report/CSPViolationReportBody WebIDL surface and full ReportingObserver lifecycle/qualification remain open; these source deliveries do not close the full Reporting API or S-01.


## Pinned Reporting/CSP IDL string-category reconciliation

The pinned `interfaces/reporting.idl` declares Report and ReportBody as dictionaries, not interfaces. The prior note requesting Report/CSPViolationReportBody interface surface is superseded: no constructor/prototype implementation is required for those dictionaries. ReportingObserver remains an interface and still requires full lifecycle/conversion review.

Pinned `interfaces/CSP.idl` declares documentURI, referrer, blockedURI and sourceFile as USVString in both the event and event-init dictionary. Corrected authored browser.webidl plus native constructor and event producer conversion accordingly. Ordinary policy/directive/sample fields continue preserving DOMString UTF-16. The prior blanket all-fields DOMString claim is superseded.

CSPViolationReportBody likewise uses USVString URL fields and DOMString policy/sample fields. Native report JSON now applies those categories individually; Report.url itself remains DOMString. Updated constructor and transported publication regression sources compare surrogate replacement for URL fields with exact preservation for samples. No generators, builds or tests ran; generated IDL consistency remains gated with the whole programme.


## ReportingObserver disconnect and per-type buffer source reconciliation

Pinned sources `reporting/disconnect.html`, `reporting/bufferSize.html` and `reporting/reporting-api-honors-limits.https.sub.html` establish pending delivery after disconnect and 100 retained historical reports per type. Replaced three duplicated producer queues with `queue_report`; history eviction now removes the oldest record of the same type while preserving cross-type chronological order. Active observer queues preserve every admitted report rather than silently evicting at 100. Disconnect prevents future admission without clearing already queued delivery; scheduling/polling consumes that queue independently of active registration.

Generated regression produces one CSP record and 110 permission records, disconnects before delivery, requires all 111 pending records, and requires a new buffered observer to see one CSP plus the newest 100 permission records in order. Future reports do not enter the disconnected observer. Existing suspension/argument-failure regression now drains its pending first-observer record explicitly instead of relying on the old incorrect disconnect cancellation. No execution performed.


## ReportingObserver native interface source completion

Generated explicit constructor arity (1), named zero-argument nonconstructible methods, the ReportingObserver toStringTag, checked native descriptor installation and explicit proxy receiver rejection. Regression source checks method descriptors, constructor arity, native tag, forged/proxy receivers, method construction rejection, options getter order and original iterable exception identity. Existing native queue/filter/buffer sources are unchanged by this surface correction. No execution performed.

Read-only repository search found no `generate_test_report` operation. The reporting-directory fixtures exercise it, but full Core dependency attribution must establish its required caller set before expanding the native automation route. This finding does not claim the Core operation inventory complete. ReportingObserver graph/GC ownership and complete cross-realm lifecycle review remain open.


## ReportingObserver GC ownership

Replaced unconditional native Global observer/callback roots with a weak observer handle and a private callback edge owned by the JavaScript observer. Registration retains a native self root; disconnect releases it only when pending reports are empty. takeRecords/callback acceptance release disconnected empty roots. Constructor admission prunes dead weak records and installs private identity before publishing native registry membership.

Generated GC regression sources cover an inactive observer/callback cycle, registered retention followed by empty disconnect collection, disconnected pending-report retention through exactly one delivery, subsequent collection, and pruning before fresh admission. Source-only: no forced GC, tests or runtime code were executed during generation. Cross-realm and complete retirement semantics still require review.


## Parent event-journal cursor ordering

Source review found `push_context_event` allocated the monotone ID before taking the journal lock. Concurrent CSP report completion and other event producers could therefore publish ID n+1 first, let a poller advance to it, and later publish ID n where it would never satisfy the poller's cursor filter. ID allocation now occurs under the journal publication lock so append order and cursor order agree for every producer.

Generated regression sources use four concurrent publishers and verify strict journal identity order plus complete exactly-once traversal with small cursor batches. This does not close journal capacity/identity-exhaustion or full recipient-lifecycle qualification; no tests executed.


## Shared context protocol source integration correction

While reviewing exhaustive event polling, found PreloadedResource and PreloadConsumptionAuthorized declared under ContextEvent even though preload_authorization.rs, prepared_resource_preload.rs and their regression sources use ContextServiceResponse. Moved the two existing variants to the response enum; removed the erroneous event variants. This is source reconciliation of existing production callers/regressions, not a new event route. Protocol constants/regeneration and builds remain deferred.


## Reporting caller attribution follow-up

Read the frozen capability policy `benchmarks/conformance/wpt-capability-tiers-v2.json`: content-security-policy is Core, reporting is unmatched Extended. A whole pinned-tree source search found direct generate_test_report uses only in reporting-directory fixtures, infrastructure/testdriver, and shared driver/executor definitions. No direct Core call was found; transitive/dynamic attribution remains open, so no additional automation route was inferred merely from these Extended controls.

A separate concrete Core gap is confirmed: `content-security-policy/report-hash/resources/report-hash-test-runner.sub.js` expects network-delivered csp-hash records, excludes them from ReportingObserver, checks same-origin/CORS digest exposure, opaque empty hash, successful integrity gating and per-policy hash algorithms. Source searches in native CSP and JS found no report-hash producer. The navigation agent is implementing its ServerTiming dependency; root must implement the native hash-report producer and script-fetch integration. This is required Core work, not expansion into the unmatched Reporting API directory.


## Native CSP hash-report generation and script-loader integration

Generated `CspPolicySet::script_hash_reports`: effective directive fallback, strongest sha512/sha384/sha256 reporting keyword per policy, first report-to endpoint, raw response-byte digest, empty opaque-response digest, stripped document URL and original request subresource URL. Hash reports have a distinct native payload and never enter violation-event or ReportingObserver queues.

Standalone parser/classic/module loaders send through their existing bounded document-owned report transport. The collector records whether its native client global is Window; worker-global loaders do not emit hash reports. Parent LoadScript success routes capture original request URL and emit through the bounded context report queue after revalidating creator ownership; Agent clients are excluded. Data and HTTP paths are integrated, and typed module loads retain their script request destination.

Generated sources cover pinned sha256/sha512 fixture digests, independent policies/report-only fallback, strongest effective directive, document URL stripping, opaque digest suppression, no legacy report-uri fallback, successful/failed SRI and CORS visibility, plus redirects retaining original request URL.

Specification audit (current w3c/webappsec-csp index.bs, potentially-report-hash) confirms strongest algorithm, request URL, Window client and network-only visibility. Its post-response placement precedes Fetch SRI/MIME checks, while pinned report-hash fixtures explicitly require no hash report after failed integrity. The generated hook follows the pinned negative cases at successful script-fetch admission; this sequencing difference remains documented, not presented as identical standards text.

Remaining: Window-created worker/sharedworker/worklet script-like destinations; full preload/service/blob producer/reuse reconciliation and parent/worker end-to-end fault regression sources. No builds or tests ran; the broad hash-reporting requirement remains open.


## Window-owned worker/worklet hash-report producers

Generalized native hash selection with the actual Fetch destination and effective CSP directive. Explicit worker-src/child-src does not invoke hash reporting; worker/sharedworker/serviceworker fallback to script-src/default-src does. Audio/paint worklet selection bypasses script-src-elem and uses script-src/default-src.

Added optional native hash-report producer to agent script authority. Parent Window-owned agent creation and service entry loading install it; worker-global imports and nested Agent-owned creation do not. Data/HTTP/service-intercepted entry responses calculate hashes from retained raw response bytes and publish only after decode/cancellation checks. Owner authority is revalidated before parent report admission. Blob entry loading carries the real worker/sharedworker/audioworklet destination, revalidates its Window owner and reports raw blob response bytes; Agent imports remain excluded.

Generated policy regression matrix covers worker-src/child-src exclusion, script/default fallback, actual body destination and worklet script-src-elem exclusion. Agent-loader regression distinguishes a Window entry producer from imports with no producer. Existing cancelled-blob regression was reconciled with the destination signature. No execution performed.

Remaining major hash integration gap: preload reuse under a new consumer policy currently retains URLs/owner/proof but not the original byte digests needed to report a newly selected algorithm. Retain all supported raw-byte digests plus exposure provenance in parent/local preload evidence rather than hashing decoded source or refetching. Full native owner/fault integration regressions remain pending.
