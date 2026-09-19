# FACE participation and file submission custody source audit

## Status and scope

Source generated; no build, binding generator, formatter, test, browser service or WPT execution performed. This is bounded S-03.e evidence, not a conformance result or closure of S-03, custom elements, navigation, accessibility, or the five workflow families.

The pinned inventory has 25 direct Core rows in the form-associated custom-element/output dependency group (23 testharness, 2 reftest). The behavior constructor test outside `form-associated/` is included. The checkout is WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`. CEReactions annotations additionally follow the pinned DOM/HTML/DOMParsing/CSSOM interfaces and apply to transitive callers.

## Production integration generated

| Contract | Native implementation and callers | Regression source |
| --- | --- | --- |
| Output default/value mode and reset ordering | `oryn-dom/output.rs`, cloned/authored-import versus native migration state; output WebIDL and accessor dispatch; reset_form snapshots and retains controls in tree order before DOM replacement; FACE callbacks observe completed built-in reset | DOM output mutation/import/reset; FACE synchronous reset versus click backup microtask; canceled reset |
| CE reaction stack and backup queue | Metadata-gated generated setters/operations; shared per-element queues with separate nested element queues; native microtask backup; direct animation commitStyles boundary; mutation observer admission stages CE work first; removed public test-only reaction flushing hook | Existing enqueue/conversion/upgrade custody failures plus termination retaining unstarted suffix; unrelated operation does not steal backup callbacks |
| Native submit behavior | `submit_behavior.rs` DOM state + binding record; native constructor brand; frozen attached sequence; private wrapper edge and tracked adoption lease; duplicate/type/reuse rejection; GC release and host-owned private internals edge; reflected properties read native records | Native brand/attachment/type/failed retry; selected FormData entries; disabled/default/association; adoption during conversion and collectible private cycles; trusted Enter/Space; validation, canceled click, requestSubmit, dialog value |
| Behavior integration | Shared is_submit_button and submitter override/value/no-validate helpers; FormData ignores setFormValue for behavior hosts and includes only selected name/value; focus/keyboard/activation and semantic role/disabled projection | Native semantic role precedence and disabled overlay without authored attributes; non-FACE focus versus no form participation |
| Native FACE file snapshots | Existing setFormValue/submission_entries and saved form state use native Blob/File records; regression adds actual bytes, metadata and ordered duplicate entries through snapshot and restoration | `face_shadow_anchor_and_file_state_snapshot_preserve_native_values` |
| Authorized upload admission | `PageHandle::set_files` authenticates selected frame origin before grant materialization, rejects stale scope, revalidates authority before transport; upload store validates the entire bounded selection under one grant lock; typed/text OIL upload resolve file-control Click eligibility without firing a click | Revoked/mixed/oversize selection; selected cross-origin frame policy and stale scope; real native file input to FACE multipart submission after grant revocation and form reset; typed/text OIL upload works when file input deliberately lacks text Type capability |
| Durable forms-family receipt | New native FACE scenario and workflow case use real OIL-authorized file input, ElementInternals FormData snapshot and normal form navigation; server decoder stores ordered fields/file SHA256 in SQLite | Native broker body byte regression; fixture persistence/dedup regression source; workflow expected independent receipt fields/hash |

## Source-review decisions

- **Source reconciliation generated; independent review/execution pending:** the former same-arena scheduler has been replaced by isolate-owned stack, backup and retained element queues. `reaction_states` enumerates all live same-agent Documents; same-arena `agent_states` remains unchanged for definition ownership. Each element retains one shared callback queue. Queue entries follow native adoption, including entries already being invoked; the destination need not register the original definition. Scoped queues pop before callback invocation, while a failed conversion retains its original admission queue and interruption retains unstarted callbacks. Registry define/upgrade use explicit boundaries; nested invocation shares agent-wide limits. Added borrowed-operation, cross-document element order, termination suffix, retirement and actual cross-arena adoption regression sources. These changes have not been built or executed and keep the combined validation gate open.

- Independent attachment review confirms the original wrapper is retained before options conversion, the current owner is resolved afterward, and attached/failed/disabled state is rechecked before publication. Added regression source for an options getter attaching internals reentrantly and an iterator claiming the proposed behavior for another host before conversion finishes; failed outer attachment must preserve existing internals and leave unconsumed behaviors/hosts reusable. No execution.

- Nested shadow validity anchor was already supported by the native parent/host ancestry walk; regression added without introducing a second algorithm.
- A behavior attached to a non-FACE host supplies role/focus but cannot become a form submitter or default button. Existing FACE association is required for form ownership and entry construction.
- Behavior fields are native authority. Authored host name/value/form* properties cannot substitute for the behavior record. Attached behavior object identity survives detach and native adoption.
- Disabled first submitter remains the default submitter but cannot activate or contribute a successful entry.
- File grant revocation prevents future selection; already-owned file bytes remain with the selected input and accepted submission. No host path or opaque parent handle becomes file content or a receipt field.
- Termination consumes an already-entered callback but preserves its unstarted reaction suffix. Ordinary author callback exceptions stay on the existing report-exception path.
- Source-only fixture persistence checks do not establish browser execution; native body capture, persisted server decoding and full workflow replay are separate gates.

## Gates still open

1. Parent independent source review of the new reaction-stack/behavior integration, including cross-realm adoption/GC and generic WebIDL conversion compatibility.
2. Regenerate bindings only after the complete generation pass; then build and fix integration errors. New IDL is intentionally not yet reflected in generated/browser.rs.
3. Execute all existing and added native regressions and the exact pinned WPT dependency inventory, including validity bubble/delegatesFocus reftests and behavior trusted pointer/default-style integration.
4. Replay the complete forms family with the new FACE multipart case and retain independently persisted SQLite evidence. Other four families and broader selected-target/navigation lifecycles remain their own gates.
5. Whole custom-element/IDL closure remains broader than this dependency group. Output htmlFor and unrelated tentative behaviors are not claimed implemented by this audit.

## Pinned direct Core inventory

| Path | SHA256 |
| --- | --- |
| `custom-elements/form-associated/ElementInternals-reportValidity-bubble.html` | `68ab61e4a3783b6b5fb0a2efc181fdabd5919d98409774a646de0203f622c95b` |
| `custom-elements/form-associated/ElementInternals-reportValidity-delegatesFocus.html` | `8598de51512b9131145619da93ba8efeed3bda07831c35257f7fbdcda1568446` |
| `custom-elements/element-internals-behaviors.tentative.html` | `71a137149e6031bfb71d40e4b83db4084e867b5eeb76d0b5b284b85177325a1e` |
| `custom-elements/form-associated/ElementInternals-NotSupportedError.html` | `dd3bde99c1733307a573a6ee900eeaad33685c726448e19502deb8df43050c02` |
| `custom-elements/form-associated/ElementInternals-behavior-accessibility.tentative.html` | `8631060a0b4d7d4490bdff6ad1b1cff8f7097855aa37ffc680702de683d85b97` |
| `custom-elements/form-associated/ElementInternals-form.html` | `0183e707c7dd73a2ac20870071df1dd82264557abcf9633f793fc4a63c6a0f36` |
| `custom-elements/form-associated/ElementInternals-labels.html` | `15fbf6a32ca25ce4c3b4954e96b3d0cd2780b1c546f1b70877fec1c2b59ee504` |
| `custom-elements/form-associated/ElementInternals-setFormValue-nullish-value.html` | `46e0e36dc16204e3414fe476017eacf60fe34c7995a8ae472546bf37b534d3ed` |
| `custom-elements/form-associated/ElementInternals-setFormValue.html` | `b777928aa521f3eed0e26dd3b4a2718f4b402f1ef62ebdb8c0e8c0bfae93621f` |
| `custom-elements/form-associated/ElementInternals-submit-behavior-dialog.tentative.html` | `47c6c4f1215be003293561c84eb77c07bbbc29553aaeb73869cb54dd9373895d` |
| `custom-elements/form-associated/ElementInternals-submit-behavior.tentative.html` | `c88ed5a92d25a764c5138d583dcb2806b841e851cdcc1f2932bb2a6e4021517d` |
| `custom-elements/form-associated/ElementInternals-target-element-is-held-strongly.html` | `ed6012af3cc1ea64209ddc82037b320f2f5d30d7916e1938185769ed74055941` |
| `custom-elements/form-associated/ElementInternals-validation.html` | `6a399fc6d73e9be9d5b233ba02f420eab985f2fbb83a792b90e40d3d7965e8d9` |
| `custom-elements/form-associated/disabled-delegatesFocus.html` | `27c41982202334592c6576eafc40e10c6988cf01fc5e8da0e5cd664d8c8a0ffb` |
| `custom-elements/form-associated/fieldset-elements.html` | `9ba08a91d3924610f5a68e8c00fc2c1364c1b143da31b9c7b03619a5bf1203f2` |
| `custom-elements/form-associated/focusability.html` | `2fb5f35e4d024a118a67f45915dcd58f7460727cb414270a46407a08c1291e2c` |
| `custom-elements/form-associated/form-associated-callback.html` | `af2765824f5cf75c602699958bb8246315fc40059a8949052965dbc3eef76f7c` |
| `custom-elements/form-associated/form-disabled-callback.html` | `8e61eec988c8723b72ba3d851ef8a92c8f008ada10f0bafe7583dcd7f9b1902c` |
| `custom-elements/form-associated/form-elements-namedItem.html` | `f78987e149ca96c83762e7dad3f95ccd1f0a9790cd2a9b7b40219916c1497fc7` |
| `custom-elements/form-associated/form-reset-callback.html` | `e689adba0d513e2efdefc951e64fbd5994a4ef0387335c2055f1474c6837d824` |
| `custom-elements/form-associated/label-delegatesFocus.html` | `1dca3ef9f37fddb7c0270aa16c953e3d7bf6fa247665e460ad42e8b319acda95` |
| `html/semantics/forms/the-output-element/mutations.window.js` | `429e0669c9c464e34bca7c690907c667ea5762bb59e4ee4af8d679c92c697a05` |
| `html/semantics/forms/the-output-element/output-setcustomvalidity.html` | `2358d0b48c571f368a954d6295649d9a67b912b348b0803b067ae77be810655c` |
| `html/semantics/forms/the-output-element/output-validity.html` | `2ba01469045b8bb92c4b8f456557b9e80b3b2a0f3e15fbee51b85c59fd20828b` |
| `html/semantics/forms/the-output-element/output.html` | `8917298f100643877b3c840da7ff27005c71a35b1ec8ca28597842e81f0318d9` |

## S-02 native caller reconciliation (2026-09-17)

This is a production-entry inventory, not a claim that all custom-element WPT rows pass. Every path below reaches the shared native registry/reaction machinery; source presence is distinct from algorithm completeness.

| Entry family | Concrete native route | Generated coverage and remaining obligation |
|---|---|---|
| Global registration and lookup | `custom_elements::install` → native `registry_operation` (`define`, `get`, `getName`, `whenDefined`, `upgrade`, `initialize`) | Native interface/constructor/private receiver records and scoped registry tables generated; WebIDL generator intentionally deferred. Source reconciliation remains in progress. |
| Direct construction | HTML element constructors → `pending_construction`/`construction_target`; `Document.createElement/createElementNS` → `create_synchronously` | Definition-specific construction stack, native IsConstructor, customized built-in interface validation and autonomous synchronous result validation/fresh unknown fallback generated. Source regressions added; full nested/foreign realm review pending. |
| Existing parser/insertion content | Native mutation journal → `collect_for_migration` → `enqueue_upgrade_subtree`; `innerHTML`/fragment insertion → `upgrade_custom_element_subtree`; native `parser_checkpoint` and document-load checkpoint → host `run_custom_element_reactions` | Existing fragment interleaving, inserted-subtree-only, connectedness and parser shadow-disable sources. Synchronous parser construction timing and constructor side effects across all HTML/XML/parser entry paths remain a separate audit gate. |
| Clone/import/inert document | `cloneNode`/`importNode` → `upgrade_for_document`; explicit registry `upgrade` traverses inert roots | Native global/scoped/null associations and creation-time is slot now propagate through clone/import/initialization/adoption. Prior-owner mutation metadata distinguishes first ownership assignment from adoption. Complete pinned scoped-registry caller reconciliation remains open. |
| Attribute/tree/Range/CSS operations | generated setter/operation `metadata.ce_reactions` → `ReactionBoundary`; native `Animation.commitStyles` explicit boundary | Metadata declarations generated; generated bindings deliberately not regenerated yet. Unannotated calls stage backup work through observer/checkpoint admission. DOM/HTML/DOMParsing/CSSOM declared members require the final generation/build gate. |
| Connection/removal/adoption | mutation records → connection/attribute/adopted callbacks; cross-arena `node_migration` capture/publication moves element definition, callback queue and migratable queue target | Native implicit/explicit adoption sources plus new cross-arena queued-adoption regression. State-preserving `moveBefore` now carries native-only provenance and selects the captured move callback or per-element disconnected/connected fallback; source regression includes shadow descendants and getter capture order. |
| FACE state and reset/history | association refresh, native `reset_form`, `enqueue_form_reset`, form restoration queue and snapshot callers | Output reset precedes FACE callback; disabled/form-owner transitions and file history state retain native custody. S-03.e receipt execution remains open. |
| Shadow/slot/accessibility | native shadow attachment checks `shadow_disabled`; parser disabled-shadow policy; DOM migration moves CE/FACE metadata; separate slotchange/observer queues and layout/AX projected tree | Source coverage exists in shadow/slot/layout/AX groups; scheduler completion does not close those groups. Scoped shadow registries and constructor-triggered shadow/adoption ordering still require full reconciliation. |
| BFCache and retirement | host `park_document_authority`/`restore_document_authority`/`discard_cached_authority` mirror all root/frame environment suspension to bindings | Active same-agent siblings retain progress; parked CE entries keep ordered custody until successful restore; failed/partial restore preserves suspension. Native forest assertions plus CE parked-prefix/restore/retirement regression generated. Retirement releases queue target leases and strong environment ownership immediately. |

### Earlier reconciliation findings (superseded where updated below)

- Native registry interfaces, scoped/global/null associations, `getName`/`initialize`, customized built-in selection, definition-specific construction stacks, synchronous createElement result validation and fallback are now **generated, still under source reconciliation**. Two new regression sources exercise registry isolation/initial null custody/descendant stamping/inert scoped creation and synchronous result errors/customized built-in behavior/conversion order/native IsConstructor. Do not infer all 39 scoped-registry files from these bounded sources.
- `connectedMoveCallback` capture and native state-preserving move selection are generated. The regression includes shadow descendants, per-element fallback, no-op/self/detached moves and immutable captured callback order; independent review/execution pending.
- Native parser checkpoints are present, but a complete synchronous parser custom-element construction and post-constructor validation inventory is not established by these scheduler changes. Do not infer it from eventual mutation-journal upgrade.
- Remaining shadow/slot/scoped-registry and accessibility obligations stay under S-02/V; layout/AX state transport alone is not custom-element algorithm completion.

The scheduler now uses an isolate-level invocation budget, all-live unsuspended reaction-state inventory, stack/backup/retry queues, and per-element shared callback queues. Queue entries strongly retain their native environment and a `TrackedNodeLease`; registry backreferences are weak, and retirement clears both owners immediately. Adoption updates every shared queued reference without replay. Failed conversion preserves the original scoped queue; started callbacks are consumed while unstarted suffixes remain owned. Added regression source covers borrowed foreign receiver, B/A/B element order, scoped conversion failure, cross-document termination/retirement, actual adoption, parked-only microtask non-spinning and restoration.

`execution_tasks_stopped` intentionally remains separate from `execution_suspended`: existing task callers often discard custody on stopped/retired, while BFCache requires retention. Host task selection already filters `realm_is_parked`; native platform also validates parent WindowState. This change adds the missing CE agent-queue gate without changing cached-document metadata access or worker close semantics. Further shared-agent task audits must use the temporary suspension accessor where they traverse Documents outside the host's selected-realm gate.

No build, generator, formatter, test, service or WPT command has been run. S-02 remains open on the explicit missing algorithms above and on independent source review/execution.

### Backup microtask owner lifetime

Production parking cannot capture an already-queued shared-agent backup job: pinned `Context::SuspendMicrotasks` rejects `active->size()!=0`, running jobs, nonzero microtask scope depth and entered contexts; host parking first drains the checkpoint. The existing native V8 patch regression `OrynSuspensionRejectsEnteredContextAndPendingSharedJobs` covers that contract. Retirement/global detachment can instead suppress a queued job, so backup custody now records its owner, document/stream generation and monotonic callback serial. A later active Document replaces obsolete ownership; a surviving superseded callback cannot clear or drain the replacement. New `retired_backup_owner_cannot_block_or_clear_replacement_microtask` source covers this race. No execution performed.

### Earlier S-02 generation boundaries (see delivered updates below)

Pinned inventory contains 180 direct Core custom-element files, including 39 registry files. Remaining source work includes synchronous parser construction before attributes/insertion (HTML, XML, document.write and streaming root/frame callers), exact parser-selected scoped registry propagation, fragment/initial declarative registry inheritance, construction exceptions/termination retention, foreign constructor realm and scoped-registry lifetime review. Parser hook design must not hold a native DOM RefCell borrow across author code, retain raw V8 scopes across tasks, or replace an author-retained constructor object with a fallback in place. Source generation is ongoing; none of these gates are checked complete.

### Contextual parser generation update

Vendored html5ever/markup5ever 0.39.0 now expose a backward-compatible intended-insertion-context hook. Provenance/checksums and divergence are recorded under `vendor/html5ever/ORYN-DIVERGENCE.md`. Oryn resolves actual parent/owner registry before constructing and setting token attributes. Native scope wrappers cover retained HTML root/frame slices and EOF, XML advance and document.write; the legacy consuming HTML caller is also wired. Fresh fallback nodes become actual tree-builder handles, preserving constructor objects retained by author code. Document-specific dynamic-markup counters reject open/write/close reentry while constructing. The gate removes its tokenizer from the RefCell during callbacks, so lifecycle observation and retirement do not borrow an already-borrowed parser.

Generated source tests cover native constructor-before-attributes/children/parent observations, foster parent selection, callback lexical lifetime, fallback subtree identity, rejected dynamic markup and XML constructor timing. Remaining review gates: infrastructure failure/termination after constructor acceptance and during parser publication, native parser callback reparent/adoption, scoped fragment inheritance, definition reaction ordering and scoped-registry lifetime. No parser or CE broad completion box has been checked; no execution performed.


### S-02 registry graph and parser reaction reconciliation

Generated source now separates weak definition handles from V8-private registry graphs. Connected native document/task leases root registry wrappers; detached native components trace the registries of all descendants, including descendants with no materialized wrapper. Definition/registry cycles remain collectable. Association-only initialize refreshes wrapper graphs before returning. New source regressions cover collectable definition cycles and a detached deep clone retaining an unwrapped scoped descendant. These assertions have not been executed.

Pinned `registries/scoped-registry-initialize-upgrades.html` exposed an additional branch: initialize must also upgrade an already-associated element after define, even when no association changes. Native initialization now collects upgrade candidates separately from null associations; source regression includes this sequence.

Pinned `throw-on-dynamic-markup-insertion-counter-reactions.html` requires the original document's counter during token attribute reactions as well as construction. The token guard now lasts through reaction publication. Accepted callback records retain that document, and sequence-bounded parser mutation spans preserve the counter when mutation conversion fails before a callback record exists. Spans are cleared only when conversion commits; later mutation records fall outside the span. The injected parser-enqueue regression checks that resumed attribute callbacks still reject document.write and that child tokens remain unconsumed until recovery.

Range.createContextualFragment now has a dedicated native HTML/XML path using the current Range boundary, owner Document, namespace/form context, TrustedHTML conversion, and scoped fragment registry. Source regression covers HTML table context, owner/registry identity and XML namespace/error behavior. The earlier scoped-fragment missing-code note is superseded by generated-under-review status.

Still open: complete independent source reconciliation of pinned scoped-registry and parser-construction callers, author reparent/adoption during construction, and infrastructure termination after synchronous constructor acceptance. WebIDL generation, compilation, tests and conformance remain deferred globally. No broad S-02 completion claim is made.


The concrete parser termination branch is now generated: after an accepted autonomous constructor terminates, native fallback creation completes before the parser bridge returns the termination. The actual tree-builder handle is the fresh unknown element; an author-retained partially constructed object is preserved separately. `parser_constructor_termination_retains_fresh_fallback_without_replay` supplies a native halt callback, cancels termination only in the regression harness, resumes the saved source suffix, and asserts constructor-once and child/fallback identity. This replaces the missing implementation note for that branch; independent review and execution remain open.


### Pinned caller reconciliation: adoption, cloning, construction (continued)

The following were concrete missing algorithms, now generated with native regression sources:

- `registries/scoped-registry-effective-global-registry.html`: native same-arena adoption resolves registry associations synchronously in shadow-including order. A declarative shadow root's keep-null bit survives adoption; ordinary null roots acquire the destination effective global registry; null children below scoped ancestors remain null. Cross-arena migration stages each result before commit. Native 3×3 registry matrices plus cross-arena append/adopt and declarative/imperative cases were added.
- `registries/Document-importNode.html`: the original fallback registry passes through light descendants even beneath a scoped ancestor, and is reset for shadow/template contents. Same-arena shallow cloning now copies registry/is/declarative metadata, matching the existing import path. Fresh clone ownership no longer invokes adoption rebinding. Document cloning preserves only a scoped registry, leaving a cloned global Document's registry null.
- `Document-createElementNS-prefix-timing.html` and registry constructor-reentry fixtures: autonomous synchronous creation uses an active constructor-to-registry map rather than an upgrade stack. Nested direct construction allocates independent objects, and prefix is assigned after successful constructor validation. HTMLConstructor creates in its current realm; wrong requested owner Document produces fallback instead of borrowing requested-document authority.
- The HTML upgrade algorithm explicitly permits nested CEReactions to run before the constructor completes. The former whole-document constructing gate was removed; retained per-element queues remain authoritative. The reaction-order regression now observes the attribute callback before the constructor's final marker.
- `cross-realm-callback-report-exception.html`: constructor and lifecycle callback exceptions report in the callback's global, independently of the Error object's realm. A native two-realm regression supplies an Error created in the caller realm and expects only callback-realm error handlers.

The V8 eager-receiver defect identified here is now addressed by the source integration below. All new regression sources remain unrun.


### Current S-02 reconciliation state: native constructor dependency delivered

The eighth pinned engine patch, `html-constructor-deferred-prototype.patch`, now
adds an unpublished FunctionTemplate opt-in backed by initialized native rare data.
It leaves ordinary V8 API constructors unchanged and defers author NewTarget
prototype access for HTML element constructors until after native registration
and active-interface validation. Native binding installation enables the flag for
HTML element interfaces; native media constructors also opt in for ordered
WebIDL conversion/prototype allocation as recorded in the G native-operation
audit. Accepted HTML construction reads the prototype once;
primitive fallback uses the immutable native interface prototype in NewTarget's
function realm. No authored property controls realm or registry authority.

Upgrade rechecks the tracked construction entry after the getter, so getter
adoption or nested consumption cannot reuse a stale node or consume an entry
twice. The already-consumed marker check occurs after the observable getter, as
required by HTMLConstructor. Direct construction freezes its accepted definition,
then reads/sets the prototype before materializing native element identity.

Generated regressions: native cctest `OrynHtmlConstructorDefersPrototypeUntilAdmission`;
binding `html_constructor_observes_prototype_after_registration_and_before_consumed_marker`
and `html_constructor_nonobject_prototype_uses_new_target_realm_intrinsic`.
The earlier callback-realm, nested synchronous construction and failed-upgrade
sources remain part of this dependency group.

Profile schema 5 now requires the eighth patch and synchronized public header.
Patch installation recognizes complete exact chain prefixes; regression installation
verifies all prerequisite files after newer overlapping patches. Its source tests
cover previous/current installations, changed prerequisites and final-file overrides.
No prepared installer, binding generator, engine build, test, formatter or WPT run
has been executed. New archive/layout-code regeneration, snapshot/profile
qualification and execution remain future phases.

**Current distinction:** constructor ordering, clone/adoption association, scoped
registry object lifetime, parser construction/termination and agent reaction queues
now have generated production paths and bounded regression sources. The earlier
paragraphs naming those algorithms as missing code are historical findings, not a
current missing-code list. Broad S-02 stays open for independent source review and
full pinned caller reconciliation, particularly parser author reparent/adoption and
shadow/slot/style/accessibility cross-group interactions. The 180 direct Core files
(including 39 registry files) have not been executed; no pass count is asserted.


### Definition admission and document association order

Further pinned `registries/define.html` source reconciliation corrected exact
prototype-getter exception propagation and scoped-registry `extends` rejection.
FACE callback getters now run in associated/reset/disabled/restore order while
retaining their existing native callback slot indices. The combined definition
regression verifies thrown identity, failed-name reuse, scoped customized-built-in
rejection and observable getter order.

`registries/scoped-registry-define-upgrade-order.html` exposed a production
ordering gap: native realm/arena iteration is not first registry-document
association order. `DomRuntime` now records native first-association sequence per
registry/Document without rooting Documents. Assignment, ownership/adoption,
clone initialization, native parser inheritance and cross-arena migration update
that record. Document reclamation removes entries. Definition candidate selection
orders the live associated Documents by that sequence, resolves the exact native
environment, excludes retired ownership and retains parked candidates. Within a
Document, native traversal now visits shadow trees before light children and does
not accidentally traverse another Document as part of its parent's tree.

Definition admission stages the complete ordered candidate set under native
`MutationAdmission` rollback guards. A failure removes the provisional definition
and staged Upgrade suffix without resolving whenDefined or invoking constructors.
Successful registry graph publication commits the definition and queues before
resolving promises; an authored constructor `then` getter therefore observes the
accepted definition while all original candidates already have queue custody.
Post-commit constructors/callbacks use the existing retained CE element queues.

Generated sources cover: native first-association/adoption order; three independent
same-agent Documents associated in a different order from realm creation;
shadow-before-light order; define while a destination is suspended then restore;
and injected failure after one candidate was staged followed by a once-only
retry and promise resolution. These sources remain unrun and the new bounded
group is still pending independent source review. No broad S-02 checkbox changed.

### Independent HTML constructor fallback review

The coordinator traced the deferred-prototype engine opt-in, native callback admission, prototype getter, and constructor-realm bridge. The bridge deliberately returns no realm for a revoked proxy without throwing. The primitive-prototype fallback previously converted that absence into `JavaScriptException` even though no exception was pending; it now emits an explicit TypeError. Generated regression `html_constructor_prototype_getter_revocation_only_requires_realm_for_fallback` revokes NewTarget during its one prototype read: primitive fallback must throw, while an object prototype must complete without an unnecessary second realm lookup. The engine patch and broader constructor/parser integration remain unbuilt and unqualified; this review does not close S-02.

### Independent definition admission boundary review

The coordinator traced candidate staging, each owner's MutationAdmission rollback snapshot, shared reaction queue truncation, private registry graph publication, and the post-commit whenDefined resolution boundary. No author callback occurs during candidate staging or private-edge publication. Extended `definition_candidate_failure_rolls_back_acceptance_and_retry_is_once` with a second fault after all candidates are staged but before registry graph publication. It requires the provisional definition, pending upgrades and staged reactions to roll back, no constructor/promise getter to run, and a subsequent accepted definition to resolve/upgrade once. This is generated source coverage; post-acceptance termination and full multi-document/parser behavior still require independent reconciliation and execution.

### Shared-arena registry owner review

`ordered_registry_documents` previously selected the first exact binding and then rejected it if retired, which could hide a later live exact binding. It now prefers a live binding for that exact Document; when exact bindings exist but all are retired it excludes the Document instead of borrowing an unrelated live arena owner. Generated regression `registry_document_owner_prefers_live_exact_binding_and_never_falls_back_from_retirement` covers both cases with three bindings in one native arena. Inert documents without an execution binding retain their existing association-owner behavior. No execution or broad S-02 acceptance is claimed.

### Independent whenDefined reentrancy review

The already-defined `whenDefined` path held `registry.definitions.borrow()` across Promise resolution with the constructor. Resolution can synchronously run an authored `then` getter; defining another element there requires a mutable registry borrow and could panic. The path now clones the accepted definition and releases the map borrow before resolving. Both immediate resolution and invalid-name rejection check the native resolver result instead of ignoring an empty result.

Generated regression `when_defined_constructor_then_can_reenter_registry_and_preserves_rejection` defines another element inside the constructor's `then` getter and verifies the original constructor resolves exactly once. A separate throwing getter in the same regression preserves the original rejection object. No build/test execution occurred; broad S-02 caller/termination qualification remains open.

### Form-internals conversion adoption custody

`setFormValue` previously captured the source custom-element owner and NodeId before authored value/state conversion, then unwrapped that old map entry. `setValidity` similarly used its original DOM/NodeId after dictionary/message getters. Cross-arena adoption during those conversions moves native custom-element records and changes local node identity.

Both operations now hold a native TrackedNodeLease across conversion and locate the moved form-associated record by exact current arena/node afterward. Submission/restoration assignment uses the resolved owner; validity anchor checks, wrapper metadata and native flags use that same owner. A thrown conversion returns before applying either update, while native adoption itself remains observable. No author property supplies the new owner identity.

The generated two-arena regression covers movement in value conversion, a round trip during restoration-state conversion, movement in validity flag/message getters, explicit moved anchor, clearing validity after moving back, and a throwing conversion that moves the host but preserves its previous submission. Source only; no execution or whole-S-03 closure. Other form-internals conversion/order and history restoration callers still require reconciliation.

### Restoration conversion and shadow-anchor reconciliation

Pinned `custom-elements/form-associated/ElementInternals-validation.html` requires direct and nested closed-shadow anchors. Source inspection confirms `DomRuntime::parent` exposes the internal shadow-host ancestry used by validation. The conversion/adoption regression now carries a nested closed-shadow input anchor through both arena moves, preserving that requirement rather than changing the traversal.

The shared form-state restoration helper also now retains the host across conversion and resolves the moved record before queuing the restore callback. A new direct native two-arena regression converts a state object that adopts the host, requires one destination-owned restore callback with the original mode/value, and verifies that restoration itself does not replace the submission value. Both changes are source-generated and unrun; native historical saved-state serialization/caller coverage and complete forms-family acceptance remain open.

### Validation anchor independently adopted after assignment

`report_controls` already re-resolves each invalid control wrapper after event callbacks, so control adoption itself is covered. The custom validation-anchor resolver still decoded its saved anchor against the host arena, however. A formerly valid anchor adopted independently into another arena could therefore raise an argument error instead of falling back to the host.

The resolver now reads the anchor's native wrapper identity and compares arena ownership before walking ancestry. A foreign-arena anchor falls back to the host; a returned descendant resolves through its current node identity. A two-arena regression covers initial acceptance, independent anchor adoption, fallback, return adoption, and preservation of validity/message state. No test execution or full validation acceptance is claimed.

### Validation event-to-report caller check

Source review confirms `validate_form` roots the original invalid-control snapshot as native wrappers and re-resolves each before dispatch; `validate_control` likewise preserves a wrapper for reporting after its listener. No replacement of that existing path is needed for control adoption. The anchor regression now additionally calls actual `ElementInternals.reportValidity` with an invalid listener that adopts only the anchor into the second arena. It requires one invalid callback, a false validation result, retained message and host fallback afterward.

Private anchor lookup now propagates an empty V8 result instead of treating it as an absent anchor and continuing to focus a fallback after a native failure. Unrun source regression only; focus geometry, visual validation UI and the broader pinned validation suite remain qualification obligations.

### Pending history replacement byte accounting

When an unconsumed saved FACE value replaced a captured live value, `capture_form_states_limited` previously added both complete records to its admission calculation. This could reject a final snapshot within the requested byte cap. Replacement accounting now subtracts the existing record before adding the retained history record, and the final materialized snapshot is checked against the caller's cap as well as the global bound.

A generated regression admits a pending saved value on a defined FACE control, recaptures it at the exact final snapshot byte count, requires equality with the saved snapshot, then rejects one byte below the cap without consuming restoration state. No tests/builds executed. This does not close the full form-history/qualification requirement.

### FormData conversion boundary comparison

Compared the corrected form-internals paths with shared FormData construction and mutation. `Creation::new` snapshots the entry list before retaining the formdata event; native submission retains Creation across polls. FormData append/set convert name and value before obtaining the mutable entry record. File renaming drops its source platform-record borrow before authored filename conversion; entry-list body serialization uses native File/FormData records. This review did not identify the same cross-arena custom-host stale-ID update in those non-node record paths, so no parallel replacement was introduced. This bounded source finding is not whole-FormData/WebIDL or multipart qualification.


## Foreign validation anchor type and descendant checks

Source inspection found setValidity used the host-arena argument_node helper to convert its anchor. That incorrectly rejected an actual foreign HTMLElement as a wrapper TypeError before reaching the required shadow-including-descendant check. The native Node identity helper now resolves and type-checks the anchor in its own arena first; a different arena then produces NotFoundError. Same-arena descendant checks retain their existing shadow-host traversal. SVG, Text, proxy and null arguments remain TypeErrors.

Expanded `validation_anchor_adopted_to_another_arena_falls_back_to_host` with foreign adopted-anchor/body rejection, foreign SVG/Text/proxy type rejection, preservation of old message/flags after every failure, and successful resolution of the original saved anchor after returning it to the host. This follows the type-versus-descendant distinction in pinned `custom-elements/form-associated/ElementInternals-validation.html`. Source-only; no tests ran and full S-03 remains open.


## Optional setFormValue restoration-state conversion

The IDL declares a required nullable value and an optional nullable state with no default. The former implementation distinguished state only by argument count, converting explicit undefined to null rather than treating the optional argument as missing. It now defaults missing/undefined state to the already converted submission value. Explicit null still stores null restoration state; required value=undefined still clears submission, matching the pinned nullish-value test.

Generated `undefined_optional_form_state_defaults_to_converted_submission` compares native captured history for omitted versus undefined state across string, File and duplicate-key FormData values, distinguishes explicit null, checks one authored string conversion, verifies the submitted entry, and checks required undefined clearing. No execution; broader form-state restoration acceptance remains open.


## Web IDL conversion before form-associated semantic checks

Removed the common early form_owner_state check from internals_call. setFormValue now converts both arguments while retaining the host, then checks the current form-associated record. setValidity converts flags, message and the optional HTMLElement anchor before checking form association, followed by required-message and descendant constraints. This preserves original authored conversion exceptions and their side effects on non-form-associated hosts. checkValidity/reportValidity retain their form check because they have no arguments to convert. Native host tracking across adoption remains intact.

Regression source `form_method_conversions_precede_form_association_checks` checks thrown-object identity, value/state and flags/message conversion order, anchor type errors before semantic rejection, NotSupportedError before missing-message/descendant constraints, and both no-argument operations. This supersedes earlier descriptions of an initial form-owner check for argument-taking operations. Source only; no execution.


## Saved restoration mode validation

The saved-state dispatcher previously checked restore/autocomplete mode only after taking its custom-value path. Built-in controls could mutate, and null custom state could return success, for a mode rejected by the direct custom-control API. Mode validation now precedes both early paths. Regression source `invalid_saved_restore_mode_preserves_builtin_and_custom_state` rejects an invalid mode for built-in/custom and null snapshots, checks unchanged input/submission and no callback, then restores the original valid snapshots. Production pending-history delivery supplies restore; public saved-state entry points share this validation. Source only; no execution.


## Built-in select option dirty-state preservation

Built-in history capture now includes each dirty option index alongside selected indices, using the existing bounded Entries representation. Restore applies selection setters first, then restores all option dirty-selectedness flags. Previously those setters dirtied selected options and retained unrelated current dirty flags, changing whether subsequent selected-attribute mutations updated selectedness.

Regression source `builtin_select_history_preserves_clean_and_dirty_option_behavior` captures a clean selected option and a dirty unselected option, changes current selection, restores and compares the complete native snapshot, then verifies selected-attribute mutations update only the clean option. No execution. The added dirty-option entry key must be included in the final combined candidate compatibility review; it does not authorize reuse of historical artifacts.


## Exact built-in snapshot entry accounting

Replaced fixed 32-byte payload estimates and the four-entry estimate for three selection fields with actual key/value byte counts plus entry storage. The existing global snapshot accounting now agrees with built-in capture for boolean flags, selected/dirty option indices and selection range/direction. All charges still precede insertion into the captured entry list. Regression source `builtin_history_capture_accepts_exact_serialized_budget` covers text, checkbox, textarea selection and multiple-select options: exact native snapshot bytes are accepted, one byte less fails, and failed capture leaves state unchanged. No execution.

## Pinned custom-element name grammar and exact registry keys

`custom-elements/registries/valid-custom-element-names.html` specifies the relaxed HTML local-name grammar: ASCII lowercase initial character, at least one hyphen, no ASCII uppercase or HTML name delimiter, and exclusion of the eight reserved names. The shared native validator still used the previous XML-derived PCENChar ranges, incorrectly rejecting required punctuation, colons and Unicode values outside those ranges. `oryn-dom/selector_states.rs` now applies the pinned algorithm to actual UTF-16 code units, so registry admission and native `:defined` use one rule. The old colon-rejection regression was corrected.

Registry define/get/whenDefined names, extends and creation-options is values now use lossless DOMString conversion rather than replacement-character conversion. Separate surrogate and U+FFFD keys remain distinct through definition, getName, pending whenDefined resolution and customized-built-in selection. The generic element-local-name validator also checks actual code units, preventing the private lossless-string storage marker from either rejecting valid names or hiding forbidden delimiters. Literal local-name creation continues to preserve colons as local-name content.

Generated native regressions cover punctuation, reserved names, upper Unicode ranges, lone-surrogate values, hidden forbidden delimiter/uppercase cases, global/scoped synchronous construction, pending promise identity, actual owner registry and customized-built-in is lookup. No test/build/generator execution occurred. Parser construction/adoption, complete scoped-registry caller mapping and full S-02 qualification remain open.

## Scoped initialize admission and upgrade custody

Pinned `registries/CustomElementRegistry-initialize.html` and `scoped-registry-initialize-upgrades.html` require permanent null-association initialization, matching-registry-only upgrades in tree order, no nested-shadow traversal and upgrade of already-associated undefined elements. The native traversal already selected those candidates, but published associations before fallible reaction/root admission without rollback. A rejected infrastructure admission could retain a partial upgrade prefix or associations without the complete corresponding work.

`initialize_registry` now snapshots exact native associations and first-associated-document ordering, stages upgrade queues with the existing MutationAdmission transaction, and commits both only after registry-root publication. RAII rollback restores the original tri-state associations and queued upgrade/reaction prefix without invoking constructors. The individual native association setter also restores its prior value if association-document admission fails. No author callbacks occur within this rollback boundary. Existing nested-shadow exclusion and matching-registry selection remain intact.

Generated regression injects failure after one candidate has queued, checks original null associations and association-document inventory, drains to prove no leaked constructor, then retries and requires one constructor each in original tree order. Nested null shadow roots/children remain untouched and repeated initialize does not reconstruct accepted elements. Source only; no tests/build/generators. Broader construction reparent/adoption, scoped registry lifetime and full S-02 qualification remain open.

## Accepted upgrade definition across registry reassociation

The native queued `Reaction::Upgrade` formerly stored no definition and repeated registry lookup at delivery. HTML's upgrade reaction instead retains the definition selected when enqueued. A native adoption/association change before invocation could therefore substitute a different constructor for already-accepted work. Upgrade reactions now own the selected Rc definition and a strong native V8 root for its registry graph; definition callbacks remain valid even when the target's current registry no longer roots that graph. Infrastructure retries requeue the identical definition/root. Migration already transports complete reaction records and therefore preserves the new payload.

Immediate synchronous upgrade still uses a lookup wrapper; queued delivery calls the explicit-definition algorithm. Existing already-custom/failed element guards continue to prevent duplicate construction. New native regression queues a detached candidate under one definition, changes its native association to a second registry with a different constructor, verifies retained original registry ownership, and requires the original constructor once after delivery and later repeated admission. This bounded source regression is not an executed adoption/WPT result; full parser/foreign-realm and retirement qualification remains open.

## Attribute reaction names, fallback names and constructor admission

Three additional diagnosed S-02 gaps are generated:

- Native Attribute mutation producers already capture ContentAttribute.local_name. Reaction collection incorrectly split that local name at its final colon again. It now consumes the captured name unchanged, matching initial upgrade callbacks. Regression sources cover literal null-namespace colons and namespace-qualified attributes whose local part itself contains a colon, set/remove, Attr attachment/value/removal and initial-upgrade versus later-mutation parity. Pinned attribute-changed-callback.html and Element CEReactions callers require local-name/namespace callback identity; pinned relaxed name grammar supplies the colon cases.
- Autonomous constructor-failure fallback reconstructed a qualified string and reparsed it. A literal colon name from createElement or the HTML parser thereby acquired a spurious namespace prefix. Fallback now creates from the captured native QualName directly. Regression sources compare literal and explicitly qualified names, original author-retained constructor objects, fresh unknown fallback identity and actual streaming HTML parser children/constructor count.
- Upgrade constructor handle lookup occurred after upgrading/failed metadata, FACE registration and initial callback staging. A graph-handle failure before author entry could leave an unretryable failed/upgrading record. Constructor and wrapper admission now precede all upgrade-state publication. Injected regression preserves initial shadow metadata, absence of FACE/Element publication, queued retry and constructor/attribute callback-once order. This changes only pre-author failure; entered constructor failure remains terminal.

Source-only caller reconciliation: Mutation::Attribute producers in oryn-dom/lib.rs capture local names for add/change/remove; parser/direct creation share create_synchronously; queued, direct and customized built-in upgrade share upgrade_with_definition through the immediate-lookup wrapper. No build/test/generator has run. Full scoped-registry, parser/adoption interleavings, termination and native engine qualification remain open.

## Parser post-constructor infrastructure outcome

The shared parser bridge previously returned its original undefined token node whenever create_synchronously returned an infrastructure error. Errors can occur after author construction, such as allocating the failure exception/report. Inserting that original candidate allowed later mutation processing to invoke the same constructor again.

Autonomous creation now reserves and natively roots a fresh failed fallback before author entry, preserving the captured QualName, owner Document and registry. `create_synchronously_outcome` returns the chosen node independently from an optional infrastructure error. The HTML/XML/document.write shared parser hook retains the failed fallback after post-entry error and surfaces the original error before consuming further child tokens. Direct Document factories continue to expose Result behavior. Immediate customized-built-in upgrades retain original-node semantics; no alternate identity is introduced for them. Pre-author failures remain eligible for first construction rather than pretending author code ran.

A generated parser regression injects failure during error-report preparation after a constructor adds a forbidden attribute. It checks fresh unknown fallback identity/literal-colon name, preservation of the author-retained constructor object, paused child suffix and constructor count one after reaction drain and source-suffix retry. This supersedes the missing fallback-custody algorithm for that error path; no execution or broad parser/conformance closure is claimed.

## Lossless HTML factory case mapping

The registry's exact UTF-16 keys exposed a remaining factory mismatch: HTML createElement and attribute normalization case-mapped the private encoded String bytes, leaving uppercase code units unchanged when a lone surrogate was present. HTML tagName/nodeName uppercasing could likewise corrupt the storage marker. Shared native DOMString ASCII-case helpers now map actual code units and re-encode losslessly. Native createElement/createAttribute, attribute receiver normalization and reflected HTML node names use them; XML casing remains unchanged.

Generated pure/helper and native-binding regressions cover uppercase custom-element input selecting the lowercase registered definition, exact localName/tagName/nodeName, surrogate attribute creation/set/get/remove, XML preservation and authored text resembling the private storage prefix. This is a bounded factory/registry integration fix; it does not claim every namespace or string-consuming API has been exhaustively audited. No execution.

## Current S-02 source disposition (supersedes earlier broad missing-code notes)

The earlier “synchronous parser construction missing”, “scoped fragment inheritance missing”, generic namespace/name validation and failed-token fallback notes are historical. The following is the current algorithm/caller disposition, not a runtime result:

| Algorithm/caller family | Current source disposition | Remaining work |
|---|---|---|
| define/get/getName/whenDefined, scoped/global identity and conversion | Generated native registry graph, exact DOMString keys, relaxed name grammar, callback conversion, definition admission and Promise reentrancy | Combined binding generation and runtime/GC/foreign-realm execution; no duplicate implementation proposed |
| initialize/upgrade traversal, definition capture | Generated null-association/tree-order/nested-shadow filtering, atomic initialization admission and accepted-definition reaction payload | Execute failure, GC, migration and repeated-admission sources |
| Direct autonomous creation | Generated active registry construction, result validation, native-name fallback and pre-author fallback root | Execute constructor exception/termination, cross-realm and fresh fallback sources |
| Direct customized-built-in creation | Now returns actual owner plus original NodeId after constructor adoption; both createElement/NS callers wrap resolved owner | Execute two-arena constructor-adopts-this sources |
| HTML/XML/write parser construction | Tracked parser handles, actual-owner lexical Construct/Adopt bridge, queued native mutations and one-shot retained EOF now generated, with HTML/XML/V8 migration and retry sources | Combined source/fault review remains open; execution and full pinned qualification deferred. The earlier NodeId-only implementation finding is superseded by the tracked ownership implementation below |
| Clone/import/adoption registry propagation | Generated scoped/global/null and declarative keep-null semantics, original fallback through light descendants and reset at shadow/template boundaries | Reconcile parser handle ownership above; execute cross-arena association and foreign-realm sources |
| Reactions, state-preserving move, attribute names | Generated isolate-owned queues, native adopted target tracking, frozen upgrade definitions, exact attribute local names and connectedMove fallback | Execute event order, interruption, parked/retired and GC matrix; no broad missing queue implementation remains |

The direct customized factory regression covers HTMLButtonElement constructor adoption into a second native arena, original wrapper identity and class, actual destination ownerDocument, both createElement and createElementNS, and native CE-record migration. No tests/builds/generators ran. The parser cross-arena group now has generated implementation pending the source review below; execution rows are not missing-code claims.


## Tracked parser ownership implementation in progress

The earlier NodeId-only parser finding now has production implementation in
`oryn-dom/src/html/handles.rs`, shared HTML/XML TreeSink, the lexical
`document_parser_gate` action bridge, and retained frame EOF preflight. Parser
keys resolve current native owners; customized construction and adopted open
ancestors retain identity. Attributes, insertion, text and closing-resource work
are queued in order; a failed prefix precedes later input. EOF is emitted once
while the owner retains the parser. Native parser roots use separate tracking
from the bounded temporary API lease pool, with the same migration and deferred
release semantics. The parser's existing source/node admission remains the bound.

Generated source regressions cover handle migration/roundtrip/collision, native
HTML open-parent migration, failed adoption retry without repeated construction,
XML element/CDATA/comment/PI migration, and real V8 customized token plus open
ancestor adoption. Pre-constructor failure retains the construction attempt;
post-constructor failure retains the selected result without rerunning author
code. Original-document parser resource events reject foreign owner identities.

Additional EOF failure, pending-insertion cancellation, and committed-adoption
publication retry regression sources are now generated. This group remains
**open** pending combined source review and any diagnosed residual caller gaps. None of these regression sources have been run;
no build, binding generator or conformance execution has occurred.


Source review additionally corrected native failed-construction root lifetime:
the fallback lease is now returned with the outcome and retained until the
parser interns the selected handle or direct creation installs wrapper custody.
The previous inner-function local lease could reclaim a detached fallback before
that handoff. A dedicated native V8 regression holds the outcome, acquires the
wrapper, releases the temporary root and verifies original fallback identity.
This is generated source evidence only.
