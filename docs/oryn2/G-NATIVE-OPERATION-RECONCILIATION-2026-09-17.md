# Native testdriver operation reconciliation — 2026-09-17

Source-only generation. No build, binding generator, formatter, tests, WPT, services or qualification ran. This supplements G-OPERATION-AUDIT-2026-09-16.md; it does not replace historical raw outcomes.

## Exact scope

Re-read the saved 1,516 operation occurrences: 38 literal API names, comprising 19 mapped names and 19 conditional Bluetooth BiDi helper names. Counts remain source occurrences, never observed executions. Reviewed the actual pinned `testdriver.js` → `testdriver-extra.js` → upstream `actions.py` / `asyncactions.py` → Oryn protocol → HTTP session → native owner chain. Pinned checkout remains `719d5e38fdd0903a18ed9007aba816c98cc491e0`.

The two direct BiDi geolocation override occurrences are in `html/semantics/permission-element/geolocation-element/get-current-position-success.html`; its setup permission call and the error sibling are the direct BiDi permission consumers. The extra adapter supplies `[window]` and `window.location.origin`; upstream async actions then require populated contexts and an origin. The Oryn adapter preserves omitted/error/null distinctions and converts classic native Window/Frame handles for the upstream context extractor. A previous audit example naming `geolocation/tojson.https.window.js` is a broader algorithm reference, not this saved direct-Core call denominator.

Additional bounded source inspection searched Core entry paths plus saved helper source/target paths for computed `test_driver[...]`, direct aliases, and destructuring aliases. The only candidate was `uievents/click/click_events_on_input.html:49`, which aliases `test_driver.Actions.prototype.ButtonType`; it adds no operation. This negative search does not prove arbitrary JavaScript dataflow or generated-string execution reachability. Saved dynamic loader classifications and native module/script requirements remain authoritative.

## Concrete missing behavior repaired

1. **Closed/crashed platform override resurrection.** Geolocation/minimize/RPH used `page_url`, which includes retained closed-page descriptors. `mutate_live_platform_pages` now holds the native page registry through platform-state mutation, validates the entire set and context retirement first, and applies one atomic batch. Close/crash cannot interleave a retired override back into the map. Empty/oversized sets reject without effects.
2. **RPH reset at capacity.** Default mode `none` now removes an override. Absent/default and repeated modes are idempotent; resetting does not consume the 1,024-entry budget or a new revision.
3. **Incomplete selected-frame fallback.** A child snapshot missing its native frame token no longer silently becomes a root Creator. `webdriver_snapshot_target` validates root/frame shape for permissions, cookies, WebDriver file paths and parent OIL file grants, before reading data or mutating state. Existing exact native generation/authority validation remains.
4. **Cookie mutation after stale admission.** Checking authority after deleting cookies could only report the damage. `with_window_worker_creator` now keeps the established page/origin/(optional document policy)/worker-policy/message authority locks through the cookie store effect. It supplies the current committed selected and top-level URLs, so same-document path changes cannot delete cookies for the previous URL. The ordinary Creator wrapper keeps its previous lock behavior; only the cookie caller asks for URL snapshots. No closure calls functions that reacquire those locks.
5. **Platform callback continuation across park/replacement.** `native_platform::poll` now gates new tasks against the captured generation/stream and current parent Window authority between callback-producing stages, queued local errors, and geolocation callbacks. Suspended/transient tasks retain their queues with 25 ms exponential retry capped at one second; readiness reports a bounded delay. Retirement releases native platform custody. Already-entered notification dispatch remains owned by its existing continuation.
6. **Minimize during an earlier callback.** Window visibility/revision are refreshed after callback-producing stages and before each geolocation task. An earlier callback can minimize the page; later position callbacks are retained until restoration instead of consuming a stale visible snapshot.

## Native regression sources added

- `platform_automation_regressions.rs`: real closed-page admission, mixed live/closed all-or-none override batch, crashed context and empty set, incomplete frame permission/cookie/upload rejection, fresh same-document cookie path, and retirement between cookie preparation and store admission.
- `platform_control::registration_reset_is_capacity_free_and_idempotent`.
- `native_platform::geolocation_callback_parking_preserves_later_task_without_busy_polling` and `minimize_in_first_location_callback_defers_later_location_until_restore`.
- Existing native/HTTP sources retained for distinct embedded-origin permission partitions, cookie HttpOnly/path/expiry, option click, contenteditable caret, stale input cleanup, real file metadata/events, layout-backed AX and native protocol-handler prompt/unregister lifecycle.

## Caller and algorithm inventory

| Required operation names | Exact caller/algorithm boundary | Source status and remaining acceptance |
| --- | --- | --- |
| Actions, click, bless, send_keys | Upstream action sequence/element routes → selected native document, trusted keyboard/pointer, option/file special cases, caret, held-input release | Existing native input and selected-lifetime source groups; broad S input algorithms and cross-frame execution qualification remain separate gates. Upload typed/text OIL resolution was also corrected in S-03.e. |
| set_permission, bidi.permissions.set_permission | 15 direct classic source files cover permission elements, background-sync fetchLater and srcdoc lifecycle; two direct BiDi permission files; selected top/embedded partition → native consumer/prompt | Partitioned grants and live owner checks generated; incomplete-frame fallback repaired here. Full consumer acceptance requires each actual permission-element/fetchLater source, not just query success. |
| bidi.emulation.set_geolocation_override | Direct geolocation element success setup/reset; native context-set validation → atomic parent positions → queued native position/error objects | Closed/crashed admission, park/visibility reentrancy and retry fixes generated. Native callback and HTTP adapter sources exist; execute exact element success/error and related position tests later. |
| set_rph_registration_mode | Five protocol-handler path/query/fragment fixtures, including service-worker and no-service-worker variants | Native mode, default prompt, register/unregister, exact origin/template and custom-scheme navigation generated; capacity/default cleanup repaired. Service-worker interception and final navigation outcomes require integrated verification. |
| minimize_window, get_window_rect, set_window_rect | Hidden-document pageswap test; event-handler-onmove test | Native visibility and rectangle routes generated; stale platform mutation and callback-visible minimize repaired. Full pageswap activation/order and move events belong lifecycle group and remain independently verified. |
| get_computed_label, get_computed_role, get_accessibility_properties_for_element, get_accessibility_properties_for_accessibility_node | Native selected element → fresh layout-backed semantic graph; checked state and tab tree from `wai-aria/accessibility_properties_basic.tentative.html` | Production uses layout visibility/generated content/native state and opaque scoped AX identities; source regressions exist. Full layout/AT/property parity and stale-frame execution remain qualification gates. |
| get_all_cookies, get_named_cookie, delete_all_cookies | SameSite and partitioned-cookie sources → native associated cookie store, HttpOnly and ancestor security context | Cookie deletion is now atomic with current native authority/URL. Cookie policy algorithms, especially partitioned/third-party eligibility, still require exact caller acceptance rather than inferred route coverage. |
| set_test_context | `sandbox-top-navigation-helper.sub.js` selects the real harness context | Existing product/native browsing-context identity path; tied to required sandbox navigation source group. |
| 19 Bluetooth BiDi helper names | Two saved helper sources; legacy Core Bluetooth callers take `bluetooth_test`, not `bluetooth_bidi_test` | No new GATT/BiDi simulator claim. Keep the exact conditional exclusion until a required caller selects that branch. |

## Terminal navigation/error evidence

The native `GET oryn/diagnostics/navigation` route is admitted and retains closed-page history. Session serialization forwards native `retired`, `pruned_outcomes`, and per-error truncation markers rather than equating admission ACK with navigation success. Existing driver regression verifies session ownership and closed-page retention. Original-run collector consumption and complete in-flight navigation failure/close ordering remain the lifecycle/qualification gates; this source pass introduces no fabricated terminal outcome.

## Remaining gates

- Independent review of the new authority-lock and native task-retry integration, including shared root/auxiliary actors and no deadlocks.
- Complete the separately owned S/R/V native algorithms required by the caller table; no generic protocol route closes those contracts.
- Preserve explicit conditional/dynamic classifications; no new unsupported branch is silently treated as success.
- After global source generation completes: regenerate bindings, build/fix, run native and HTTP regressions, execute exact pinned callers and all Core WPT, then workflows and qualification on the same candidate.


## G-02.e exact document and input continuation follow-up

Re-read repository AGENTS.md and the existing G-02.e.1–.7 source evidence before implementing additional changes. Source-only; no builds/tests/formatters/services.

The existing file path route validates UTF-16/path/count bounds, derives selected native authority and FileUpload policy before host file access, uses nonblocking regular-file handles and bounded reads, and revalidates after the read. SetFiles validates the live element and native type/count, appends for multiple inputs and dispatches native input/change. Cookie consumers already validate the complete parent Creator and perform effects atomically under current authority. Their previously generated regressions remain the coverage for those algorithms.

A remaining cross-caller defect was confirmed: several selected-document consumers compared realm/root/generation/stream but omitted authority epoch and embedding connection identity. `AutomationRealmSnapshot::same_document_as` now composes exact document/authority checks with the native complete-connection check. It ignores URL changes intentionally: same-document URLs are observational metadata, not new authority. This replaces partial comparisons in `validate_automation_scope`, `webdriver_element_command`, `select_webdriver_frame` (except explicit top recovery), authorized clipboard, element/coordinate input resolution, focused keyboard resolution, `validate_action_context`, script admission, and WebDriver session context validation. Incomplete child tokens/generations and non-live snapshots reject instead of falling back to root authority. Navigable-only navigation selection remains distinct and unchanged.

Native focus also lacked a lifetime check between author focus callbacks and caret placement. A callback can document.open, reinsert and refocus the same native editing node. The old command must not overwrite the new stream's selection even when focused NodeId still compares equal. The function now captures root/generation/stream/authority and checks them before caret mutation, plus native connected/document ownership. Option callback continuation and final element-command completion now include authority epoch as well as generation/stream. Existing option selectedness/event ordering is retained.

Interrupted input already retires local held/default custody at both helper and PageState boundaries; ordinary nonretirement errors preserve Release Actions custody. Using exact selected-document comparison now makes authority-only invalidation reach that existing cleanup path.

Generated additional regressions:

- `selected_document_requires_live_authority_and_complete_connection`: exact authority/stream/liveness/connection rejection and same-document URL acceptance.
- `selected_authority_rejects_element_routes_and_disarms_interrupted_inputs`: real native element route/input validation and no synthetic keyup into a rejected authority.
- `native_focus_reopened_document_preserves_replacement_stream_selection`: actual focus handler document.open/reinsertion/reselection before the original command resumes.

This completes the newly found source gaps in the listed G-02.e caller group. G-02.e.6 execution and parent G operation/helper/qualification gates remain open. No protocol shape changed in this follow-up.


## G-02.f permission/handler and callback source reconciliation

The official production adapter remains `tools/wptrunner-oryn/oryn_wptrunner/protocol.py`: BiDi permission origin/embeddedOrigin are forwarded to the native HTTP session; geolocation keeps UNDEFINED distinct from explicit reset/null and validates a populated context set. Expanded AX methods call the native element/node routes. The older `scripts/wpt-testdriver-oryn.js` diagnostic adapter is not substituted for this chain or used as conformance proof.

Additional production repairs:

- Permission override admission checks terminal context retirement while holding the platform lock. `retire_workers` takes the same lock only around setting the terminal flag, then releases it before job collection, awaits or callbacks. Thus no new grant can be published after terminal retirement. The HTTP adapter retains a no-such-window error for this case.
- Prompt is the native default permission state. Reset removes an existing override instead of retaining a permanent slot; absent resets consume neither revision nor capacity. A failed revision preflight leaves an existing grant intact.
- Clipboard omitted `allowWithoutGesture` normalizes to false, matching the pinned `interfaces/clipboard-apis.idl` declaration. Irrelevant descriptor members remain ignored as before.
- Accept-mode protocol registration and unregistration previously mutated the handlers before fallible revision allocation. They now reserve revision before mutation, and identical registration is an idempotent success. Default prompt and exact unregister behavior remain on their existing native paths.

Generated regressions: `permission_grants_cannot_resurrect_after_context_retirement`, `permission_prompt_reset_reclaims_capacity_and_is_idempotent`, `clipboard_descriptor_omission_equals_explicit_false`, `protocol_registration_revision_failure_does_not_publish_or_remove_handler`.

Source review also traced the existing geolocation grant/error/reset through native permission policy, hidden-page deferral, cached/zero-timeout semantics and callback task custody. The retained notification engine keeps event identity/listener progress on preparation or callback interruption, checks Creator and parent activity before resumed publication, and commits CSP/storage source cursors only after completion. Minimize/restore flows through the same native window state and retained visibility event path. Expanded AX uses the current shared layout graph plus native scoped node references, including G-02.e.8's exact selected-document validation. No new success-only stubs were added.

The listed native source caller group is reconciled with generated negative regression sources. Full G-02.f/G-02.g operation inventory acceptance, dynamic helper resolution, lifecycle integration and signed execution remain separate open gates. No code/tests/builds/services executed; protocol shape/constants unchanged.

## G-02.c native window movement and lookup waits

The exact direct Core caller `html/webappapis/scripting/events/event-handler-onmove-02.tentative.html`
sets `{x:100,y:200}`, awaits a Window `move` event, then reads back both positions.
The previous rectangle route ignored position and always reported zero. Its
headless-window explanation did not satisfy this caller. Native platform state
now owns bounded per-page virtual screen coordinates. Live-page admission and
retirement use the same lock as other platform mutations; revision allocation
precedes change, equal moves are idempotent, and retirement removes the position.
Window size and position pairs are independently applied, with actual integer
coordinates returned. `WindowPosition` is an appended typed platform request and
response; author JSON cannot mutate native coordinates. The trusted retained
`move` publication follows native platform task ownership and preserves listener
progress on failure. Window global `onmove` also satisfies the sibling `-01`
fixture's synthetic dispatch case. Generated coordinate attributes read native
position and preserve receiver/security checks. Native/route/event regression
sources cover position pairs, invalid/partial input, idempotency, retirement,
revision failure and callback retry. No execution has occurred.

The CSS find route previously accepted an implicit timeout but made one lookup.
It now snapshots the selected native document once, polls only empty results,
and yields through the async scheduler between attempts. Both element and
elements routes, scoped roots and the shadow selector-array route share it.
Invalid selectors, stale references and replaced selected documents retain
native errors and are not retried against replacement content. Session
revocation interrupts inventory, lookup and delay, including an unbounded null
timeout. Empty results at expiry preserve the existing single-element `no such
element` versus multi-element empty-array projection. Generated regression
sources cover native timer insertion, invalid CSS, document.open replacement
and canceled unbounded lookup. These source repairs do not close the entire
G-02.c operation/fault qualification gate.

### Operation-by-operation caller contracts carried forward

| Required operation | Algorithm and lifetime contract now traced |
| --- | --- |
| Actions | Native request preflight, per-source identity/ticks, duration interpolation, selected-document checks, reverse-order Release Actions and interruption cleanup; source regressions in webdriver_actions/action_execution. |
| click | Native scroll/paint hit testing, file-input rejection, option container activation, held pointer cleanup and post-callback lifetime validation. |
| bless | Upstream button/click helper uses the same native trusted activation path; no synthetic JS click substitution. |
| send_keys | UTF-16 native key actions, modifier/null handling, focus/caret preflight, file-upload policy custody and selected-document interruption cleanup. |
| get_computed_label | Native accessible-name computation and exact element owner through the semantic provider; detached/foreign references fail. |
| get_computed_role | Native computed role and exact element owner through the shared accessibility provider. |
| get_accessibility_properties_for_element | Typed element reference and native current AX properties; protocol adapter delegates to the native route. |
| get_accessibility_properties_for_accessibility_node | Native AX node identity and current ownership, separate from authored node IDs. |
| get_all_cookies | Associated native cookie store, HttpOnly/path/domain/Secure/expiry and live selected authority. |
| get_named_cookie | Same cookie selection; absent exact name becomes no-such-cookie. |
| delete_all_cookies | Atomic current selected authority/URL validation with native associated-cookie deletion. |
| set_permission | Descriptor normalization, exact origin/embedded-origin partition, prompt/reset capacity and retirement admission. |
| bidi.permissions.set_permission | Official protocol forwards descriptor/state/origin/embeddedOrigin to the same native algorithm. |
| bidi.emulation.set_geolocation_override | UNDEFINED versus null/reset, complete validated context batch, native location/error cache and visibility/task lifecycle. |
| get_window_rect | Current native viewport plus persistent virtual screen coordinates; closed window retains native error class. |
| set_window_rect | Full argument preflight, paired size/position updates, restored visibility and retained trusted move publication. |
| minimize_window | Native per-window visibility with retained document event and parked/retired owner checks. |
| set_rph_registration_mode | Mode validation and native protocol-registration policy; revision-preflighted acceptance/removal. |
| set_test_context | Pinned testdriver-extra.js sets its private testharness_context and later routes action messages through message_test; this is upstream local helper state, not an additional HTTP operation. Native Window/Frame serialization and session frame ownership carry the subsequent real operation. |

Bluetooth BiDi operation occurrences remain conditional helper branches under the
existing capability-tier proof, not required native device emulation inferred
from an imported helper. All 20 dynamic helper-parameter occurrences have exact
source/consumer records. These contracts are source evidence; source route
presence and this table do not substitute for full fixture execution and
independent negative controls.

Additional exact-source reconciliation: the six get_named_cookie occurrences
use top-level Partitioned/default/explicit SameSite fixtures. Native cookie
selection retains partition keys and reports Default as Lax and explicit
Strict/Lax/None, with the existing partitioned regression sources. The five
registration-mode fixtures import handler-tools.js: native registration uses
hyperlinks::parse with the document encoding before parent registration, and
State::resolve_navigation encodes the serialized external URL into the first
%s in the saved path/query/fragment template. Existing parent/root/frame URL
resolution callers consume that native mapping; service-worker and Broadcast
Channel helpers remain on the generated W package path. Expanded AX's five
occurrences assert checked, children, parent/accessibilityId, role and label;
webdriver_script_v8 regression sources already exercise these exact properties
and stale child-node rejection. These checks found existing implementation,
not a new reason to regenerate those algorithms.


## Independent G-02.f publication custody reconciliation

The exact BiDi callers `html/semantics/permission-element/geolocation-element/get-current-position-{success,error}.html` cross the protocol adapter, parent permission/location state, `permission_elements::poll`, native position object construction and element `location` delivery. The classic permission occurrences additionally include the camera, microphone and usermedia fixtures listed by the existing operation inventory. Reviewing only PermissionStatus events missed this consuming element-event route.

Source corrections:

- `native_platform::PositionTask` retains its acquired observation and acquisition revision before any fallible V8 publication. Re-fetching on allocation retry could otherwise replace a successful result with a later coordinate/error. Clearing a watch discards this exact pending owner; completing a watch retains the old revision so a newer provider change is still observed.
- `permission_elements` stores per-element native `Notification` continuation before event allocation and retains once-listener/phase progress. Prompt results and their follow-up activation have explicit custody; `promptaction` finishes before activation. Position data and partially prepared stream wrappers survive allocation failure. A 25 ms event retry deadline avoids a pending failed delivery spinning the actor.
- Each operation carries generation/stream/authority identity. Replacement clears old activation/observation, and each subsequent element task refreshes the parent native Window state. Retirement cancels retained dispatch flags. Location/stream state can be read by the original event callbacks without accepting authored objects as permission authority.
- `native_media::Captured` owns accepted native stream/track IDs and completed wrapper handles until stream construction completes. Allocation retry does not issue another Capture/PollCapture or release already accepted tracks. Cancellation/disconnection moves failed native Release calls into the existing cleanup queue. Existing URL-media resources/playback ownership is preserved.

New regression sources (unexecuted): `acquired_position_survives_allocation_retry_and_watch_observes_later_revision`, `clear_watch_discards_acquired_unpublished_position`, `geolocation_element_retries_same_observation_and_event_listener_prefix`, `old_stream_acquisition_and_activation_do_not_publish_into_reused_element`, `prompt_activation_follows_retained_delivery_and_retirement_clears_event_flags`, and `accepted_capture_retains_native_ids_and_partial_wrappers_until_publication`.

The 19-operation contract table above remains the routing inventory, not a conformance result. Input-dependent selection/editing/activation/drag, permission media constraints, hidden-document pageswap, accessibility and navigation helpers must retain their corresponding package caller coverage. The source occurrences remain 1,491 across these 19 required operations plus 25 conditional Bluetooth-helper occurrences (1,516 total); the Core denominator is unchanged. No build, generator, regression test or WPT execution was performed for this review.
# Independent capture cancellation review — 2026-09-17

`native_media::cancel_capture` now requires `Response::Stopped` before dropping
the pending capture ID. The parent registry explicitly uses this response as
the cancellation acknowledgement; successful transport with a different response
does not prove cleanup. Source regression
`pending_capture_cancellation_retains_identity_until_terminal_ack` covers an
interrupted acknowledgement, an unexpected pending response, retry with the
original ID, and idempotent local cleanup after acknowledgement. No build or test
was run. Broad G-02.f lifecycle reconciliation and qualification remain open.

Independent final-wrapper publication review now requires successful V8
prototype and private track-array installation before native media records are
published. The existing accepted-capture regression source also injects failure
at final stream backing publication, after every track wrapper exists. It checks
that both native tracks and the original wrapper prefix remain retained, no
stream record is committed, and retry returns the same track wrapper identities.
These changes and regression sources remain unbuilt and unrun.

The MediaStream binding review also moved `getTrackById` DOMString conversion
before capturing track membership. Regression source removes/adds tracks during
conversion and preserves the original thrown exception. Pinned
`interfaces/mediacapture-streams.idl` requires the track ID argument: omission now
throws while explicit undefined converts normally. MediaStream/MediaStreamTrack
operation functions now have their declared names/required argument lengths and
are nonconstructable; the fixture checks these descriptors. Source only; full
media/WebIDL conformance and runtime acceptance remain open.

Further pinned-IDL source reconciliation replaced the own data property for
`OverconstrainedError.constraint` with a configurable, enumerable readonly
prototype accessor backed by a native private value. The constructor preserves
UTF-16 DOMString conversions in constraint/message order. Media and event-handler
accessors have their specified names/lengths and are nonconstructable. Added
regression sources check descriptor shape, invalid/forged/proxy receivers,
author-shadowed properties, lone surrogates and conversion exceptions. Constructor
subclass/NewTarget work is recorded below; complete media lifecycle/IDL acceptance
still requires reconciliation. No execution was performed.

MediaStream and OverconstrainedError constructors now use the existing native
deferred-prototype template hook. After argument conversion they read NewTarget's
prototype once, use the native constructor-realm lookup for primitive fallback,
and install the result on the native wrapper. MediaStreamTrack's illegal
construction path rejects before authored prototype access. Regression source
`constructor_prototype_conversion_order_subclasses_and_foreign_fallback` covers
subclasses, two native realms, conversion/prototype ordering, and proxy revocation
with object versus primitive prototype results. This reuses the already-generated
engine hook; no additional engine patch or wire field is introduced. The source
regression and existing engine hook remain unbuilt/unrun.

The pinned MediaStream IDL declares distinct zero-argument, MediaStream, and
sequence overloads. Native dispatch now distinguishes omission from explicit
undefined and propagates an authored iterator getter's exact exception instead
of replacing it with TypeError. Added source assertions cover invalid primitive
arguments, throwing iterator access, and native MediaStream copying that bypasses
an authored iterator getter while retaining the same track wrappers. No test or
constructor execution was performed during generation.

### Retained clipboard/default-action generation in progress

`clipboard_actions::ClipboardNotification` now owns a native Notification,
original document identity, event wrapper and DataTransfer store. It resumes the
listener suffix after infrastructure failure, retains access mode while pending,
and detaches/cancels on completion, explicit cancellation or document retirement.
The shared editing-event constructor is available to the keyboard receipt work.
Regression source `retained_clipboard_event_resumes_suffix_and_detaches_only_on_completion`
checks once-listener progress, retained payload, idempotent completion and canceled
dispatch cleanup. This event component does not yet close the full clipboard
operation: backend read/write, native mutation, undo/clone completion and final
input publication still need integration into the retained ClipboardJob. Existing
one-shot callers remain until that complete replacement is generated. No tests ran.

The shared `retained_editing::Transaction` foundation is generated for rich
clipboard and keyboard defaults. It retains the original snapshot, mutation
commit marker and split-undo metadata; after native mutation succeeds, clone
completion/synchronization/snapshot retries cannot replay it. Before mutation
commit, native-only failure uses the existing snapshot recovery path. Regression
source `completion_retry_keeps_edit_and_commits_one_undo_unit` injects failure
after clone completion and requires the committed text to remain, a single undo
publication on retry, and restoration by one undo. Integration into ClipboardJob
and keyboard receipts, callback-adoption invalidation and native control-edit
commit review remain generation work. No execution occurred.

ClipboardJob implementation is now generated with retained read, clipboard event,
default selection, write, beforeinput, native edit and input phases. Successful
clipboard writes and native control mutations advance private commit markers;
rich edits use the retained transaction. The synchronous grant entrypoint now
owns the same job on suspension/error and the old one-shot action implementation
has been removed. Grants bind document authority in addition to generation and
stream epoch. Operation exclusion is bound to the original document identity so
cleanup of an old operation cannot unlock a replacement document's operation.

Regression source `interrupted_paste_and_cut_keep_each_event_write_and_edit_once`
interrupts each event phase, changes clipboard contents after the original read,
checks single read/write/edit effects, rejects nested admission, and covers
explicit cancellation and replacement-document cleanup. Keyboard receipt caller
integration is generated in parallel. Backend write failures now terminate the
original job and preserve its error: the backend offers no idempotency token and
a lost reply cannot establish that the write did not commit. The job releases its
operation lock without deleting cut content or replaying the write. Regression
source `lost_write_reply_is_terminal_and_does_not_delete_cut_selection` uses a
backend that commits and then reports failure, replaces clipboard contents, and
requires subsequent polls to preserve that replacement and original selected
text, with one cut event and no beforeinput/input. Rich-edit snapshot authority
is now checked before mutation and again after clone completion. Adoption before
mutation cancels the edit; adoption after mutation preserves the committed content
and drops the obsolete undo snapshot without replay or stale typing-override
restoration. Regression source
`adopted_edit_completion_preserves_new_owner_without_replay_or_stale_undo` covers
both boundaries, repeated completion, original undo-history size and adopted
content/parent ownership. Other infrastructure errors remain observable and
retryable; this does not establish full callback or cross-document coverage;
full input lifecycle acceptance is open. No tests/builds ran.

Coordinator source review traced control mutation through
`edit_native_control` → `DomRuntime::edit_control_value` →
`native_keyboard::record_control_edit`: these calls do not run author callbacks
between mutation and the caller's commit marker. Subsequent input notification
owns its independent retry state. Rich snapshot ownership now uses the shared
`editing_snapshot_authority_is_current` predicate, also consumed by ordinary
snapshot validation, rather than recognizing an error by its display text.
Leased-node identity/document changes return obsolete ownership; DOM access
failures still propagate. No runtime atomicity or test pass is claimed.

ClipboardJob now yields at every phase boundary while its document is locally
suspended, preserving the original read payload, event suffix and operation lock.
`suspended_clipboard_receipt_keeps_payload_without_advancing` covers parking
before the first backend read and after an interrupted paste listener, changing
the backend clipboard while parked, and resuming the original payload once.
Suspension inside shared Notification callback/property delivery is tracked in
the notification lane; this bounded job guard does not close that broader work.
Regression source only; no execution.

### Remaining native editing caller integration

Source call-site inspection found no production caller of the old direct
`dispatch_authorized_clipboard_in_context` / `dispatch_authorized_clipboard`
adapter; only selected-document regressions use it. Both adapters are now
test-only. Production keyboard Actions retain ClipboardJob through their input
receipt. Script `execCommand` remains a synchronous script API and is not
represented as a retryable host action.

The remaining production native one-shot editing callers are explicit:
`semantic_edit::replace` (OIL Type/Clear through host and executable),
`native_drag` and `native_drag_cross_frame` (beforeinput/native move/input), and
keyboard temporal/picker/undo compatibility defaults. Semantic replacement still
combines focus, selection, callbacks, mutation and change delivery in one call;
it requires a retained operation across the full OIL caller chain. Keyboard and
picker defaults are owned by the notification lane. Drag editing remains a
separate full-family integration requirement. Source availability of the shared
event/focus/edit transaction helpers does not close any of these callers.

Semantic replacement generation now includes `semantic_edit::Replacement`:
original owner/document authority, retained focus transition, frozen requested
value, selection/mutation guards around beforeinput, one native mutation commit,
retained rich undo transaction, and independent input/change notification phases.
Cancellation is terminal; parked documents yield without mutation. Generated
regressions interrupt beforeinput/input/change separately and require one undo,
and cover canceled and suspended replacements. Host begin/poll entrypoints bind
the exact original state/realm and cancel on execution timeout. These sources
are not yet the production Type/Clear route: executable/actor operation ownership
and removal of the original synchronous implementation remain required. No tests
or builds have run.

The actor Type/Clear route now constructs one replacement through the selected
executable realm and retains it while polling page tasks between pending phases.
It preserves the reserved action ID and existing after-dispatch failure wrapper,
uses the configured command deadline, bounds infrastructure retries, and returns
the original failure when retry cannot complete. The old semantic edit body is
removed; the synchronous compatibility function delegates to the same job and
reports suspension as failure. Source regression
`retained_semantic_replacement_keeps_one_action_and_respects_cancellation` checks
actor action identity, UTF-16 text, beforeinput/input/change order and canceled
Clear preserving content. This source chain still requires combined review and
execution after global generation; no passing claim is made.

Replacement follow-up separates initial widget equality from retained focus and
separates beforeinput dispatch from post-event verification. A focus listener
that establishes the requested widget value cannot suppress unfinished focus
listeners; verification failure cannot dispatch the original editing request
again. Editability is rechecked after focus and before mutation, including
readonly/disabled/inert and changed input kind. Regression source
`widget_verification_and_focus_retries_keep_completed_callbacks` covers these
boundaries. Actor polling checks the command deadline before every poll as well
as after pending work. Source review only; regressions remain unexecuted.

Drag lifecycle source review added document-authority epochs to selected-source
snapshots, Session, cross-frame realms/targets, and embedding-ancestor records.
Default-copy callbacks check that epoch as well. Native target identity now
refuses to project a retired session's target onto the current document. Regression
source `document_authority_change_revokes_drag_session_source_target_and_ancestor`
changes only authority, verifies all old owners reject it, and verifies fresh
realm/ancestor admission. This is a lifecycle prerequisite; retained drag event,
move/default and undo continuation generation remains open. No tests ran.

`data_transfer::TrustedTransfer` is generated as the retained drag/clipboard
event component. It owns the original state/authority, native event continuation,
per-event store and final accepted result. Interrupted delivery preserves the
listener suffix and access mode; completion publishes payload/negotiation changes
to the session store once, while completion/cancellation detaches saved wrappers.
The synchronous transfer adapter now delegates to this component. Regression
source `retained_drag_event_keeps_suffix_payload_and_per_event_access` covers
listener interruption, suspension, repeated completion, a later drop not reopening
the earlier wrapper, and explicit cancellation. Full Session begin/advance/drop,
cross-frame and default move operations still require retained ownership rather
than merely calling the synchronous adapter. No execution.

Native copy/drop default generation adds `native_drag::drop_default::DropDefault`
for file assignment, text-control insertion and rich-content insertion. It owns
beforeinput/input/change continuation, one native mutation commit and rich undo
completion; snapshots the transferred payload; and checks original document
authority and target kind. The old default implementation delegates to this job.
Regression source `file_drop_retry_keeps_input_suffix_and_does_not_reinstall_files`
interrupts input and change independently, parks the operation, replaces selected
files after the original commit, and requires that replacement to survive retry.
Full drag-session callers must still retain this job through their operation
lifetimes; move-source deletion integration remains separate and open. No tests
or builds ran.

Shared Store source inspection confirmed Detached mode clears payload items.
ClipboardNotification now uses a separate event store and publishes its completed
contents to the protected native source store before detaching the event wrapper.
This preserves canceled copy/cut handler payloads for the default write while
ensuring saved DataTransfer wrappers lose access. The retained clipboard-event
regression now checks these distinct stores explicitly; canceled incomplete
delivery does not publish partial handler data. Source only, unexecuted.

Retained drag admission is generated in `native_drag::begin::Begin`. It retains
the seeded store and dragstart event, checks original authority and source
document after callbacks, and yields one admitted Session only after complete
delivery. The old Session constructor body delegates to it. Regression source
`dragstart_retry_admits_one_session_with_completed_payload` interrupts the first
listener, parks/resumes, checks the completed payload and verifies Session can be
taken only once without its store being detached by admission-owner cleanup.
Native input/session transport still needs to retain Begin across calls; the
synchronous constructor adapter alone does not provide that ownership. No tests.

`native_drag::advance::Advance` now retains source drag, target prepare,
enter/leave, dragover, final negotiation and removed-target leave phases. It pins
the original session store and coordinate/modifier input, retains each transfer
event, and exposes cancellation as a separate required terminal operation.
The old same-document advance body delegates to it. Regression source
`interrupted_enter_and_over_do_not_repeat_source_drag` interrupts all three
authored events and requires one ordered delivery and Copy negotiation. This
does not close cross-frame updates or host ownership: storing these operations
inside their own V8 state would create an Rc cycle through event owners, so the
input host must own the retained operation externally. Full caller integration
and retained cancel/drop/move remain open. No execution.

Retained cancellation is generated in `native_drag::cancel::Cancel`, with a
cross-frame `Leave` continuation that enters the original target realm for each
poll. Leave delivery, negotiation/feedback cleanup and source dragend have
separate progress. A session already marked ended by the cleanup commit can
still finish its retained dragend suffix. The original same-document cancel
body delegates to this operation. Regression source
`cancellation_resumes_dragend_after_single_cleanup_commit` interrupts leave and
end independently, checks the cleanup boundary, and requires final detachment
without repeated callbacks. Host and cross-frame operation ownership remain
open; these components have not been built or executed.

Retained `DropOperation` connects drop event completion to a caller-owned native
default, one negotiation/feedback commit and dragend completion. Its native
default callback can return Pending while preserving the original payload
snapshot; once it completes, dragend retries cannot call it again. The old
Session drop body delegates to this operation and its unused one-shot emit
helper is removed. Regression source
`drop_default_pending_and_dragend_retry_keep_one_default_commit` covers an
interrupted drop, pending default and interrupted dragend with one default
commit. Complete host ownership and retained move transactions are still required;
no conformance or test result is claimed.


### Capture cancellation intent after independent review

The coordinator found that retaining only a pending capture ID after a lost CancelCapture ACK allowed grant restoration to select PollCapture again. Native `canceling` ownership now survives independently of current connection/permission: every retry sends only CancelCapture until the exact Stopped response. `capture` cannot resume or replace that canceled request. Native polling retries cancellation even if the element reconnects, and retirement clears intent under parent document retirement.

The permission consumer separately retains the canceled operation's terminal error/cancel result. A fresh explicit activation can wait behind that cleanup without being consumed when the old result publishes. Stream replacement discards old results and marks native cancellation before admitting fresh activation. Regression source `capture_cancel_intent_survives_grant_restore_and_preserves_fresh_activation` covers lost ACK, grant-restored direct capture attempts, no PollCapture, exactly retained cancel delivery, and a distinct fresh Capture afterward. The coordinator's exact-response regression remains intact. No execution.


## G-02.c Actions: pinned wheel transaction algorithm

The seven `dom/events/scrolling/wheel-event-transactions-*.html` sources require behavior beyond a wheel endpoint: retain the topmost **element** through scroll-induced hit-test changes; retain it when resized; abandon it when removed, moved in the DOM, or changed to display:none/contents; and start a distinct transaction for a separate Actions chain. Existing dispatch re-hit-tested every sample, so the basic/resize/element-target cases lacked production behavior.

Generation now gives each wheel source in an ActionExecution its own owned NativeWheelTransaction. All ticks/interpolated samples share it; another source or another chain gets another owner. Native target selection retains the exact binding/document/generation/stream/authority and mutation cursor, rejects structural movement/removal and non-rendered targets, and permits scrolling/resize to preserve the original element. No global timeout guesses or authored identity are used. The transaction disappears with its action execution, including error/cancellation. Native child-frame routing retains and revalidates the complete AutomationRealmSnapshot, remaps coordinates through existing native transforms, and retargets when the exact frame owner retires; scroll remainder still chains without a duplicate parent wheel event.

Regression sources: `wheel_chain_keeps_element_target_across_scroll_and_new_chain_retargets`, `target_removal_dom_move_and_loss_of_box_end_transaction_but_resize_does_not`, `simultaneous_wheel_sources_have_independent_targets_and_cancelable_defaults`, `wheel_transaction_retargets_after_exact_frame_owner_retires`. Existing native hit-test/cancelable-default and frame scroll-chain regressions remain applicable. This closes the identified wheel transaction production gap source-wise; no tests or WPT ran. Other input/AX/navigation fixture obligations retain their existing package gates.


### Stationary pointer boundary caller reconciliation (source generated)

Pinned mouse boundary fixtures require DOM/layout changes beneath an unmoved pointer to publish boundary events, current keyboard modifiers, and preserved ancestor membership after removal. Native root/frame checkpoints now re-hit-test retained pointer coordinates without synthesizing pointermove; keyboard admission updates modifier snapshots across related live roots/frames. Boundary publication owns the old/new ancestor leases and each event continuation, retaining accepted listener progress across allocation/termination failure. A 25ms native input deadline retries retained work; input-source reset invalidates its unique owner before any later boundary step. Temporarily reappending a target to the same rendered ancestry emits no duplicate enters; changed ancestry uses membership differences.

Source regressions in `webdriver_action_execution::stationary_pointer_tests` cover layout replacement, removed hover leaf/current modifiers, and temporary reappend. `native_pointer::boundary_dispatch::tests` covers callback-prefix identity, native retry readiness, and reset disposal. The primary executable polls fresh root/frame targets; direct host input polling also resumes existing retained publications. These are generation assertions only; no test/build/WPT execution occurred. Full G-02.c remains open for remaining fixture contracts.


### Image-map pointer and CSSOM target integration (source generated)

Direct Core `html/semantics/links/links-created-by-a-and-area-elements/target_blank_useractivation.html` drives pointer Actions on an image and expects its mapped area's trusted click and hyperlink activation. Existing hit testing only enumerated painted boxes; area has no box, so no route reached it. Native image-map resolution now runs at the image's paint position before shadow retargeting, shared by pointer Actions, hover, wheel and elementFromPoint. It resolves the first matching same-tree map by literal name/id after the first hash, re-reading every query, then its first matching area. No href is required for region hit testing.

Source review additionally used pinned `the-area-element/{area-shape,area-coords,area-processing,area-offsets}.html`, `support/hit-test.js`, and `image-maps/image-map-processing-model/hash-name-reference-test-data.html`. Implementation preserves rect endpoint normalization, circle radius rules, even-odd polygons/odd-coordinate truncation, shape aliases/defaults, legacy permissive floating coordinate parsing, case-sensitive references and literal percent characters. Regions use transformed image content coordinates and retain normal paint occlusion; area geometry/offsets remain boxless.

Regressions generated: `pinned_shapes_and_permissive_coordinate_lists`, `map_reference_is_live_tree_order_exact_and_does_not_require_area_href`, `mapped_pointer_uses_area_for_cssom_trusted_click_and_live_shape_changes`, and `transformed_image_map_uses_content_coordinates_and_paint_occlusion`. The source change feeds existing native hyperlink activation; no separate scripted endpoint or navigation shortcut was added. Build/test/WPT execution remains deferred; broader G-02.c input lifetime requirements remain open.


### Input-source lifetime across pointer callbacks (source generated)

The combined Actions review found document identity alone did not protect the native pointer registry: an embedding callback could release/reset sources during mousedown, contextmenu or click, after which the old algorithm could focus/select, unwrap a missing record, or clear a new record reusing the same numeric ID. Pointer dispatch now captures the registry's unique owner plus root/generation/stream/authority, checks it between callback-bearing defaults and terminal cleanup, and aborts the old operation without modifying the reused source. Existing pinch cancellation's separately owned consumed-gesture path is preserved. Regression `pointer_callback_reset_skips_later_defaults_and_preserves_reused_source` covers pointerdown/mousedown/mouseup/click/contextmenu with immediate ID reuse and verifies the replacement's state survives. No execution.

CSSOM image-map target audit was rechecked: pinned `the-area-element/support/hit-test.js` line37 explicitly compares elementFromPoint to the area entries in hitRect/hitAll/hitCircle/hitPoly. Shared CSSOM/activation area resolution is supported by this pinned evidence, rather than inferred from activation behavior alone.

### Retained control moves and compound edit completion (generation in progress)

`native_drag/control_move.rs` now owns same-control and paired-control move beforeinput/input notifications, immutable payload and destination value/cursor, original document authority, the native value commit and shared control undo publication. Completed event prefixes advance before further fallible validation. Suspended jobs yield; changed source/destination ownership or values before commit cancel without deleting replacement content. Native insertion normalization is checked before source deletion. The duplicated control move bodies in `native_drag.rs` were replaced with an adapter to this operation.

Regression sources cover interruption after each of the four notifications, parking between attempts, same/paired control values, one shared undo, and source replacement during delete-beforeinput. These sources have not run.

`retained_editing::Compound` now retains all editor/control snapshots through native mutation, clone callback completion and history capture. A completion failure no longer rolls back committed content or replays the mutation on a retained poll. Adoption invalidates paired undo ownership without restoring adopted content. The old compound implementation is replaced by an adapter. The generated compound regression interrupts after both halves have mutated and requires one undo restoring both editors.

**Still open:** host/Actions ownership of all drag receipts, retained rich/mixed/cross-document move callers and their live destination range cleanup, plus integrated regression sources for these consumers. Synchronous compatibility adapters do not themselves retain receipts across calls. Neither this source delivery nor adapter replacement closes the drag generation gate or establishes passing behavior. No builds, tests or conformance execution occurred.

### Rich/mixed move and drag-input ownership follow-through (generation in progress)

`native_drag/rich_move.rs` now retains editor/editor and control/editor beforeinput/input receipts, a frozen source selection and payload, destination caret ownership, and the compound transaction. The native destination live range is released on completion, cancellation or receipt destruction. Destination control mutation history rejects replace-then-restore callbacks even when final bytes match. The old rich/mixed mutation bodies now delegate to this operation. Regression sources cover all three editor/editor and mixed directions, interruption after each notification, parking, shared undo and destination replacement.

`NativeDragInput` now owns the session, selected-source snapshot, Begin/Advance/Drop/Cancel receipts and native copy/control/rich edit operation. An active receipt excludes nested command admission. Completed commands return a live session to the document; cancellation drops retained events and detaches the abandoned session payload. A regression source retains the entire begin/advance/drop sequence through callback failures and checks payload and event order. Host begin/poll APIs pin the original realm/state and cancel on callback deadline expiration.

**Remaining integration:** pointer drag admission must retain its dragstart, pointercancel and capture-release phases; Actions and cross-frame routing must own pending receipts through polling. Cross-document move generation is assigned to the parallel traversal lane. The compatibility `dispatch_input` entry point is still synchronous; replacing its internals does not establish production retry ownership. This is source generation only and all full drag gates remain open.

Pointer admission follow-through: `NativePointerDragAdmission` now retains dragstart admission, consumption of the original press, pointercancel, and lost/got capture delivery. It pins the registry's unique owner in addition to document authority, so disposal after reset cannot clear a newly reused pointer ID. Capture state is committed once before retained capture notifications. Regression sources inject callback failures through dragstart/pointercancel/lost capture and verify reset/reuse cancellation. The original pointer-admission algorithm now delegates to this receipt; host begin/poll methods preserve its original realm and enforce callback deadlines. **Actions still needs to retain the receipt across its source cursor; the synchronous compatibility caller is not that integration.** No execution occurred.

Actions admission integration now generated: `PointerProgress` owns the selected context, immutable input event and source admission receipt across actor polls. Pointer commands mutate held/position state only before constructing the pending operation. Interpolated motion retains the exact pending sample and commits its progress only after completion; a pending initial sample finishes before later tick sources are admitted. Admission retries use the bounded delayed callback retry policy and preserve the first error; retired/canceled receipts cannot retry. The new native Actions regression preserves original dragstart coordinates/modifiers and proves canceled admission does not recur through local fallback. Earlier JS receipt regressions cover mid-callback interruption. **Cross-frame dispatch after admission is still terminal one-shot in this caller until the parallel `NativeCrossFrameDrag` receipt is connected; full pointer/default publication remains separately open.** No builds/tests ran.

Cross-frame Actions connection now generated: host begin/poll methods and executable context routing retain `NativeCrossFrameDrag` after admission. `PointerProgress` captures the source, destination hit, translated input coordinates and terminal intent once, then polls the same transport through callback faults with its retry-safety check. Successful drag transport bypasses ordinary mouse dispatch. Host release happens only on completion/timeout; an Advance that cancels the session also clears consumed pointer state. The old direct transport method is a compatibility adapter over these APIs. A generated Actions regression drives a full dragstart/enter/over/drop/end sequence, asserts payload custody and absence of click, and polls completion again to check non-replay. Full pointer-reset cleanup review and non-Actions compatibility callers remain open; this entry does not establish complete input generation or executed conformance.

Reset cleanup source follow-through: host admission now registers every mirrored binding state with the cross-frame receipt, freezing each document identity and pointer registry token. Terminal cleanup delegates to the receipt's guarded transition/release method; the former sweep of current active roots is removed. A reset/reused mirror is skipped independently of the source. Local Begin/Advance/Cancel/Drop/Input and copy/control/rich default receipts now check a shared weak input-owner guard between phases; copy/move defaults expose an external source owner for cross-document transport. The file-drop regression interrupts after value commit, resets the registry and installs replacement files, then requires no old change event or repeated file assignment. Full caller/removal review and execution remain open.

Release/cancellation integration: `ReleaseExecution` now retains `PointerProgress` before continuing reverse held-input cleanup. Runtime lifecycle Cancel now owns a `CancelExecution` across actor polls; pointer cancellation and keyup each retain their source cursor and callback receipt. Native defaults and registries are cleared after completion or terminal failure, with stale-context cleanup preserved. The old cancellation body is removed; its synchronous adapter is test-only. Composition cancellation remains the existing separately audited entrypoint. New native regression sources distinguish Release (drop then dragend) from lifecycle Cancel (dragend without drop or click), with all held sources retired afterward. Ordinary pointer/default receipts are under parallel generation and remain a full-input dependency. No execution.

Terminal-state review: `PointerProgress` now caches terminal errors and drops its admitted receipts; a later poll returns the same failure instead of treating a previously failed ordinary dispatch as success. A regression supplies invalid native coordinates and polls the failed operation again. Composition session identity now also includes the document authority epoch; an old marked-text session cannot restore its original value or emit a completion event after authority revocation even if root/generation/stream are unchanged. Its regression preserves the committed marked value while retiring stale custody. Complete retained composition-event/edit phases and OS continuation integration remain open; this identity correction does not close them. Source only.

Composition generation continues: `native_composition/finish.rs` now owns the session through commit/cancel, native recovery, undo publication and retained compositionend delivery. Retry after event interruption cannot restore the old control value again or append another undo unit. Before commit, each retry revalidates original marked bytes/cursor; rich snapshot capture failure preserves the marked edit for retry instead of restoring it prematurely. `ClipboardNotification::composition` now retains composition input flags and callback suffixes; the old helper delegates to it. Regression sources cover commit/cancel end-event interruption, replacement after cancel commit, single undo, and noncancelable composing-beforeinput flags/suffixes. **The outer composition operation, start/update/edit phases and OS Actions continuation ownership are still being generated; synchronous adapters do not yet retain these jobs across host calls.** No execution.

Outer composition receipt generated: `NativeComposition` owns a unique admission token, original authority, session, payload/replacement selection, compositionstart/update, beforeinput/input, native edit commit, clone completion, finish job and final snapshot. Native edits do not replay after event/clone-completion failures. A pending start-event allocation is separate from session creation. An old receipt cannot clear a newer operation's token after retirement. The previous apply body and standalone event/interruption implementations are removed in favor of a compatibility adapter. Regression sources cover all four event interruption boundaries for controls and rich editors, parking, nested admission rejection, single undo and token reuse. **Host/OS Actions retention remains open; TextCommand still uses an explicitly terminal compatibility stage until the parallel keyboard receipt is delivered.** This bounded generation has not been built or tested.

OS text Actions integration generated: host composition begin/poll APIs bind the exact realm/state and cancel on callback timeout. Executable wrappers validate the original action context. `OsTextProgress` now owns one admitted OS sequence, grant, immutable operation and native composition receipt; the Actions source cursor waits for completion. Native callback errors retry only while the receipt owns resumable work. The final native snapshot is recorded once and the returned OS receipt is published once; terminal errors revoke the grant. A native regression obtains the deferred target through keyboard admission, delays the text receipt, checks rejection of sequence reuse, checks one text mutation/returned receipt, then completes the next sequence and verifies retirement. **Lifecycle multi-context composition cancellation and other direct composition adapters still need retained caller review; TextCommand receipt remains pending.** No execution.

Lifecycle/read caller follow-through: `CancelExecution` freezes all native composition targets and owns one cancellation receipt at a time across actor polls. It retains the first terminal composition error while still attempting later documents, then performs pointer/key cleanup. The original multi-context synchronous cancellation method is removed. Keyboard OS interpretation now also retains its initial composition Read, because stale marked-state detection can publish compositionend; it no longer drops that event on callback failure. Unused one-shot key dispatch helpers and the executable synchronous composition wrapper are removed. Generated regression sources delay cancellation, verify no early restore/retirement, then include stale and valid targets in one cancellation batch and require cleanup of the valid target despite the earlier error. TextCommand and final direct-host caller reconciliation remain open. No tests/builds ran.

Post-commit composition source review found that a task/callback mutation during clone completion could otherwise be adopted as the new marked session's expected text. The receipt now freezes its mutation cursor immediately after native commit, then aborts marked custody if another mutation precedes completion. A regression injects a failure after native mutation, replaces control text, and requires the replacement to survive without a new marked range or repeated mutation. The main checklist now records bounded composition and drag Actions receipts and replaces references to removed one-shot implementations. Full parent gates remain open; source generation only.


## Retained cross-frame drag and paired editing completion (source generation)

`native_drag_cross_frame::NativeCrossFrameDrag` now owns the admitted Session,
original source/target realms and embedding ancestry, transfer event receipts,
negotiation phase, immutable drop snapshot and retained default operation across
polls. Drag/enter/leave/over/drop/end callbacks retain their consumed prefix;
cancel/retirement detach retained transfers and release feedback. The former
one-shot cross-frame algorithm and move body were replaced by compatibility
adapters over the receipt. Host and Actions integration is in the combined source
batch; executable qualification remains deferred.

`cross_document_edit::Transaction` owns both pre-edit snapshots and marks native
source deletion/destination insertion committed before invoking clone completion.
Only native mutation failure rolls back. Completion failure resumes the original
source/target context suffix; adoption or authority replacement after commit
cannot restore the old owner or create stale paired undo. Both after snapshots
and control-history budgets are prepared before either paired undo publication.
Cross-document undo identity now includes the document authority epoch.

The cross-frame Move owns both beforeinput and input receipts, original control
value/mutation cursor, and an explicitly released live destination range for rich
editing. Cancellation after commit preserves native changes. External source
pointer reset is checked between phases. Copy/local move children receive the
source input-owner guard even when their editing target is another document.
`track_pointer_owner` freezes exact mirrored registry/document identities at host
admission; `finish_pointer_transition` never clears a reused mirror pointer.

Regression sources: `retained_cross_document_move_keeps_input_suffix_and_pair_identity`
(including target input resetting the separate source),
`retained_pair_resumes_completion_without_reapplying_either_half` (including
post-commit adoption), and the expanded
`foreign_realm_drop_preserves_event_store_modes_and_detaches_after_end` (each
advance/drop/end callback interruption and suspension). No tests/builds ran.
Full Actions/pointer-default route qualification and all broader G-02.c fixture
contracts remain open; these are bounded source-generation assertions.


## Ordinary pointer receipt generation (integration review in progress)

`native_pointer::NativePointerDispatch` now owns one physical sample through
mouse, pen, and touch dispatch. Its source token survives callback interruptions;
reset/reused IDs revoke defaults. Explicit phases retain capture lost/got events,
boundary suffixes, pointer/touch/compatibility mouse events, focus transitions,
selectstart and selection publication, slider input/change, click preactivation
and `form_activation::Finish`, interest/popup light dismissal, and terminal cleanup.
The old one-shot dispatch/touch/pinch/capture bodies are removed; the public
synchronous function is a compatibility adapter over the retained receipt.

Touch suppression owns each contact cancellation suffix and publishes pinch
geometry only after both original contact sequences finish. Native pan keeps
per-ancestor absolute scroll plans across failure. Scroll notifications are
admitted before offsets commit, preventing missing events after a failed queue
admission. Scrollbar command plans retain their initial desired offset. Permission
pointer input retains the consumed choice, native response, and element-publication
suffix. **A genuinely lost parent PromptAnswer ACK still needs a distinct native
idempotent request contract; existing deliberate repeated PromptAnswer stays
rejected.** This is assigned for parent protocol reconciliation, not claimed solved.

New regression sources cover Down/Up/capture/focus/click interruption at every
registered callback, reset/suspension disposal, slider input/change, selectstart
adoption, and both pinch contacts before geometry commit. All remain unrun.
Host/Actions wiring and combined source review are active; full G-02.c stays open.


### Retained ordinary pointer Actions integration and composition commands

NativePointerDispatch now crosses the binding export, host context/deadline guard, executable selected-document guard and PointerProgress actor polling boundary. The physical sample and target document are captured once after routing; subsequent callback/default retries keep that receipt. Final failure drops all child receipts and is cached. The direct convenience API delegates to the same preparation/polling source instead of duplicating its ordinary dispatch algorithm. New Actions regression sources cover delayed original coordinates/modifiers, single focus/click and stale-target cancellation.

NativeComposition Command now retains TextCommandReceipt rather than terminally invoking the one-shot command helper. This supersedes the earlier command-dependency caveat; the composition-level regression/caller review remains in progress.

Remaining routing gap: stationary pointer boundary preparation and ancestor/old-document synchronization still run before ordinary dispatch receipt creation. They need route-level snapshot/cursor custody so an interrupted boundary does not lose or reconstruct the remaining document transitions. This bounded integration does not close that gap, direct-host compatibility review, historical/Core mapping or any parent gate. Source inspection and regression generation only; no build/test execution.

### Ordinary pointer and cross-document boundary custody follow-through (source only)

`NativePointerBoundary` now freezes each route step's input and old/new targets, owns a per-pointer boundary claim, and retains the existing native callback cursor. Stationary sampling and generic drains cannot steal an admitted route's suffix. Reset/retirement cancels the old receipt; completed cancellation is a no-op and cleanup compares the exact claim before touching a reused pointer. Host/Actions route integration is owned by the root lane. Added source regression covers listener failure, generic-drain interference, duplicate lane admission, reset/reuse and event cleanup.

Retained popup choices now send `PromptResolve` with an exact replay receipt. Parent tombstones acknowledge a lost successful response without repeating the grant; current denied permission remains denied after replay. Legacy explicit duplicate answers remain errors. Source regression covers grant changes, conflict, foreign owner, dismissal, precommit failure, bounded pruning and retirement. The earlier lost-ACK generation residual is addressed for retained pointer choices. Keyboard now owns the same PromptResolve choice receipt and shared identity allocator, with a lost-reply regression source; broader synchronous adapters remain under activation caller reconciliation. The full operation/activation source gate and all execution qualification remain open.


### Cross-document boundary route and idle handoff generation

The previous pre-admission routing gap is now addressed in generated source. Callback-free boundary planning produces original-context/input/target steps. NativePointerBoundary claims each pointer lane and owns callback continuation; PointerDispatchRoute keeps the route cursor through stationary boundaries, active ancestor/exit boundaries, and ordinary pointer dispatch. Idle StationaryPointerRoute persists its own cursor and transfers the remaining work into a new Actions operation. No successful prefix is recalculated on callback retry. A canceled boundary cancels the physical sample; stale documents release only the old claims. The synchronous executable synchronization loop and obsolete host boundary adapter were removed.

New root regression sources: boundary_cursor_preserves_completed_prefix_while_next_step_is_parked, rejected_boundary_document_releases_claim_without_dispatching_sample, actions_takes_over_parked_idle_boundary_cursor. Native pointer lane fault/reset/competing admission regressions are in the pointer generation lane. Combined source review remains active, including ordinary dispatch claim cooperation and direct convenience APIs; these changes are unbuilt/unrun and do not close the historical/Core matrix or parent gates.

Composition command integration now includes control/rich regression source command_finishes_marked_session_then_resumes_edit_suffix_and_shared_undo, covering two interrupted callback stages, parked continuation, exact suffix and one undo unit per command/marked completion.


### Idle boundary scheduling source reconciliation

Claimed pointer lanes are intentionally excluded from generic input deadlines. StationaryPointerRoute therefore now contributes its own page readiness and next-timer deadline, delays callback-error retry by 25 ms, parks suspended owners, and immediately wakes revoked-owner cleanup even while the old document is suspended. Regression source idle_boundary_retry_wakes_at_deadline_and_revocation_wakes_parked_cleanup covers these scheduling distinctions. Source only; no execution.


### Print lifecycle publication and capture custody

NativePublicationEvent::Print now uses the retained global event publication engine for beforeprint/afterprint. NativePageTransition recipients now retain exact binding owners and authority for root, recipient and ancestor identities; suspended owners park, replaced owners are skipped, and dropping a recipient cancels its pending native receipt. The capture actor keeps before/after transitions, retry deadline, first lifecycle error and prepared print layout instead of repeating preparation when a lifecycle callback fails. It rechecks the document after lifecycle completion. The old synchronous print recipient traversal and per-document host adapter were removed; convenience dispatch delegates to the retained tree.

Generated sources cover exact print event identity/flags and callback suffix after failure/suspension, parked tree delivery and abandoned receipt cancellation, unstarted capture rejection and retained pending reply. The direct synchronous runtime print path and rejection cleanup after print has begun still require complete continuation/caller reconciliation. No execution; no print gate closure. Source review also corrected PointerDispatchRoute's module reference to the declared crate::executable module.


### Print cleanup and serialized-frame continuation

The earlier cleanup/direct-caller gaps now have generated continuations. Capture failures after native beforeprint admission retain their original error and afterprint receipt through a bounded cleanup phase; actor shutdown only cancels receipts/releases capture custody and cannot start author callbacks. Partial afterprint resumes its original receipt. Serialized frame print retains the same lifecycle tree while pumping native work, rejects original scope/document changes before layout and after callbacks, and attempts bounded cleanup without replacing the primary capture error. Selected-frame print snapshots only that frame's native descendant tree.

New sources cover original-error preservation with one afterprint, destructor cancellation without new callbacks, and selected-frame lifecycle isolation. Full capture/BFCache/actor source reconciliation and actual execution are still open. No build/test execution.


### Capture actor scheduling and complete lifetime identity

Source review found Root::next_command and shared-root polling considered PageState work only, leaving a retained capture/cleanup queue without a wakeup when ordinary document work was absent. Root now combines capture and navigation reply queues with document readiness/deadlines. Capture supplies immediate stale/closed cleanup, retry timing, lifecycle suspension deadlines and image-readiness deadlines. Regression source capture_queue_remains_runnable_without_document_tasks establishes the no-document-work case; deferred lifecycle source also checks a nonzero retry delay.

Capture identity now uses selected RealmId, native NodeId, document generation, parser stream and authority epoch. Actor captures and serialized frame capture/print validate that identity, so an unchanged generation cannot allow work under a replaced authority or selected document. Lifecycle suspension inspection distinguishes stale identities, which wake cancellation, from valid parked owners. No builds/tests executed; combined qualification remains open.


### Print publication parser-stream custody

Print event notifications now carry the original parser-stream identity and register their native receipt in the parser-scoped receipt table. This closes the interval where generic publication polling could otherwise deliver a retained print callback suffix after document.open before the capture owner next checked its lifetime. Generated regression print_callback_suffix_cannot_cross_document_open_stream covers interrupted first callback, stream replacement, native receipt retirement and no stale tail. Source only; no execution.

### Native auxiliary hyperlink default (source generated; unexecuted)

The pointer receipt now separates middle-button hyperlink activation from primary-click form defaults. It selects and roots the original native hyperlink before auxclick callbacks, retains the callback suffix, honors cancellation, then owns the shared `submissions::LinkActivation` with a `_blank` target override. Link URL/referrer/download/native authority capture uses the same staged implementation as primary activation; an HTML ismap suffix is preserved. Right-button and untrusted events do not navigate or run checkbox preactivation. Callback reparenting cannot select a different ancestor link. Regression sources `auxiliary_link_default_retains_original_link_and_callback_suffix` and `auxiliary_link_default_rejects_canceled_right_and_untrusted_events` cover callback interruption/no replay, changed href on the original link, target/noopener/referrer capture and negative admission. Native input event shape references pinned `uievents/click/auxclick_event.html`; native hyperlink defaults are separately covered by these generated regressions. No execution or complete G-02 closure claimed.


### Actor-only capture deadlines and beforeunload failure sources

The dedicated and shared-root schedulers now preserve retained actor-operation deadlines when no document work requires the regular external-completion poll cadence. Commands reset the shared root's scheduled poll; each shared-agent visit also shortens that deadline when sibling callbacks make the root runnable. Document work retains its existing polling behavior. Added `retained_actor_deadlines_do_not_spin_without_document_work` source cases for parked, due, runnable and externally pending work.

Added `beforeunload_callback_retry_retains_return_and_prompt_failure_is_terminal`: an injected failure after the registered handler retains its return conversion and original event for the listener suffix, and a prompt backend failure cannot replay the prompt on subsequent receipt polls. These sources supplement the suspension and ownership regression. No execution; full generation and qualification gates remain open.


### Queued capture cancellation behind a parked front

Capture scheduling now considers cancellation, document revocation and deadlines for every queued request. Only the front starts lifecycle/layout work; later invalid requests are removed and replied to without author callbacks or CSS capture acquisition. Regression sources cover cancelling the middle request while retaining the first/last replies, and rejecting an expired queued print with no beforeprint/afterprint callbacks. These changes are generated/source-inspected and unrun.


### CSS capture resource custody across actor and serialized captures

Replaced the per-document shared capture set and active-root-wide release adapter with bounded capture IDs. Each native capture retains exact binding/generation/stream/authority owners; release removes only that capture's URLs and preserves other captures and layout demand. Readiness follows only that capture's transitive SVG image dependencies. Native guard objects switch the executable's current capture across nested callers and restore only their own slot. Actor Pending owns its capture across yields; serialized preparation owns a distinct capture for its full preparation interval. Cleanup visits retained owners rather than the host's active root, and old-document cleanup skips replacement streams.

Source regressions cover overlapping inventories, independent readiness, bounded owner admission, idempotent release, native nested guard replacement, and serialized document.open cancellation preservation. No build/tests run. Additional production caller and destructor review remains required before the global generation gate; broader print/navigation qualification remains open.


### Capture reconciliation failures and nested frame preparation

Source review found the frame paint-media/viewport override branch called the raw plan builder, bypassing readiness and resource ownership. It now uses the same serialized preparation wrapper, retaining a separate capture owner and inherited deadline while preparing nested frames. CSS inventory admission validates the full dependency closure before mutating owner demand. Cancellation now commits the remaining desired inventory even when native stream-abort delivery fails; abort fallback still abandons the exact stream. Added regression sources for abort failure cleanup and dependency-limit admission without partial owner publication. No execution.

### Coordinator source review of native notification delivery

Reviewed native_platform::poll and native_platform/publication::{Notification::prepare,resume,admit,drain}: pending notifications drain before provider repoll; phase listener snapshots retain once-removal and resolved callbacks; callbacks leave the deque before invocation; partial work returns to the queue front; original creator and optional parser stream are checked; parent suspended/retired responses park/discard; payload bytes and queue count are bounded. CSP/storage cursors advance only after completed, noncancelled delivery. Observed visibility/permission values change after event preparation and before callbacks.

Existing CSP prepare/once-removal/after-callback, storage callback-prefix, PermissionStatus identity/state and retirement regression sources cover these branches. Added visibility prepare/after-callback retry, same-event bubbling suffix, final event flags and subsequent visibility transition source coverage. This completes the bounded coordinator source review requested in G-02.f.3, not execution, full callback-source inventory or platform qualification. No tests/builds run.
