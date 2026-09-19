# G operation and procedure generation audit — 2026-09-16

Source review only. No build, test, formatter, upstream executor, qualification or deployment has run for these changes. Whole-G behavior and runtime qualification remain open; current bounded source status is below.

Latest bounded lifetime/caller follow-up: [Native operation reconciliation — 2026-09-17](G-NATIVE-OPERATION-RECONCILIATION-2026-09-17.md). Includes atomic cookie effects, live emulation admission, selected-frame shape, and callback park/minimize recovery; source-only.

## Current source status (supersedes older batch-open lists below)

| Obligation | Current source status | Still required |
| --- | --- | --- |
| Historical helper159/44 reconciliation | Saved 1,199 occurrence review with exact source evidence; old unsaved regex counts explicitly superseded | Remaining dynamic/conditional production behavior and runtime observations, not another missing-file list |
| Driver operation inventory | Saved 1,516 literal occurrences and native route/conditional-branch matrix | Full behavior/fault/lifecycle verification; root owns pageswap |
| Manual/visual procedures | All 752 source-bound procedures generated, with candidate-native API branch selection | Actual independent observations and qualification |
| Reference/print source matrix | All 1,015 Core identities and 1,016 existing reference targets bound | Actual renderer output, readiness/fuzzy/print comparison and qualification |
| Glyph/image paint coverage | Private fresh native alpha provider consumes actual glyph/image/Canvas/SVG paint; bounding boxes are fallback only | Native execution and documented unsupported effect/frame-resource cases |
| CSS filter/mask paint | Native filter planes, external/local SVG filters and masks, alpha composition, fixed reference boxes and dependency lifecycle generated; pending render schema 3 | Actual renderer/reference qualification; broader backdrop-root and mask-border capability limits are scope-reviewed below |

Older sections record successive generation batches. Their historical open statements do not override this table or the later bounded completion evidence. No row reports executed conformance.

## Exact upstream source

Pinned checkout: `/Users/rohit/Work/oryn2/oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0`.
The actual injected adapter is `tools/wptrunner/wptrunner/testdriver-extra.js`; `testdriver-vendor.js` is empty. Native HTTP requirements come from that checkout's `executorwebdriver.py`, `actions.py`, `asyncactions.py`, and `tools/webdriver/webdriver/client.py`, not current web documentation.

## Bounded native sources generated

| Upstream caller | Native route and owner | Regression source / remaining scope |
| --- | --- | --- |
| `select.elements_by_selector`; shadow selector arrays | POST elements, element, scoped element(s), oryn/elements; `ElementCommand::Find` through session → page/worker FrameCommand → executable → native V8 selector provider | Native selector poisoning/stable reference/detachment/invalid-selector regression; actual upstream selector transport positive/failure tests; `server::native_element_routes_select_shadow_roots_and_preserve_errors` covers closed shadow chains and exact route failures. Implicit waits and full shadow/frame lifetime matrix remain. |
| `click.element` | POST element/{id}/click; native scroll + layout/paint hit check, then private native pointer source | Native readiness regression distinguishes obscured center and absent geometry. Option special behavior remains; file-input click now rejects invalid argument. |
| `send_keys.send_keys` | POST element/{id}/value; lossless UTF-16 action sequence; full sequence validation before native focus side effects | Modifier/null and astral/lone-surrogate sources. `server::element_send_keys_validates_before_focus_and_preserves_existing_selection` covers rejected oversized sequences before author focus handlers, newly focused caret at end, and retained existing selection. File upload authorization and contenteditable placement have generated sources below; interruption cleanup remains. |
| `accessibility.get_computed_label/role` | GET element/{id}/computedlabel or computedrole; native semantic provider | Shared provider exists; route-specific semantic and lifecycle regressions remain. Expanded accessibility-node property sources are described below; full semantic parity remains open. |
| `cookies.get_all_cookies/get_named_cookie/delete_all_cookies` | GET/DELETE cookie; GET cookie/{name}; BrowserContext native cookie store | Source tests cover HttpOnly visibility, associated path/domain deletion, Secure transport and expiry. Exact selected-document authorization during asynchronous navigation still needs review. |

The worker protocol is **14** because `FrameCommand::Element` and `FrameResponse::Value` extend the worker wire contract. Context service and agent IPC versions are independent; build/security tooling reads each native source constant independently.

Element host command errors now pass through watchdog and document generation/stream-epoch cleanup even when wrapper creation or native focus fails. Cookie reads do not call `document.cookie`; selectors/accessibility do not call author-replaceable JS prototypes.

## Direct Core callers found; not a complete transitive operation audit

Read-only scan of unique Core entry source files found `Actions` (274 files), `click` (160), `send_keys` (120), `bless` (107), `set_permission` (15), computed role (13), computed label (7), RPH registration mode (5), named cookie (2), delete cookies (2), BiDi permissions (2), all cookies (1), minimize (1), BiDi geolocation override (1), window rect get/set (1 each), and both expanded accessibility-properties operations (1 each).

These counts exclude operations referenced only by imported helpers, dynamically selected operation names and generated URLs. They must not be used as a closure denominator.

A subsequent conservative read-only traversal of literal script src, META script and import references visited 14,037 paths; 159 references did not resolve to static files and 44 URL literals were malformed/dynamic. It found additional Bluetooth BiDi calls inside a shared helper. That is evidence to inspect reachability, not proof those helper functions are exercised by a Core entry. The helper-aware counts include Actions 289, click 163, send_keys 128, bless 112, permissions 16, role 14, label 8. Dynamic/generator references and conditional helper reachability remain explicit audit work.

Native permission, geolocation, RPH, minimize/visibility and expanded AX operation sources are now generated as detailed below. Full platform/default behavior and lifecycle closure remain open. Existing generic actions and window/frame/script routes are not evidence that these operations work.

Queued navigation ACK is not a terminal success. Parent native `queued_navigation_status` retains bounded terminal outcomes, a `retired` flag, `pruned_outcomes`, and per-error `error_truncated`; any automation observation must preserve incomplete-history/truncated-error markers. This API alone does not establish upstream navigation/error observation.

## Procedure denominator

The preserved inventory-v3 contains exactly 16,737 Core entries:

- 14,640 testharness
- 1,003 reftest
- 330 crashtest
- 12 print-reftest
- 749 manual
- 3 visual

G-05.d therefore requires **752 individual manual/visual procedures**, in addition to actual automated print/reftest execution. Many accname/wai-aria manual files contain exact ATTAcomm step JSON keyed by AXAPI, ATK, IAccessible2 and UIA; retain those assertions rather than replacing them with generic “inspect accessibility” instructions. Platform APIs represent separate required observations where applicable; one semantic role/name result cannot stand in for the platform property matrix.

The existing `wpt_full_qualification.assess` validator binds source, revision, candidate, independent operator/reviewer, platform, procedure, observations, assistive technology, artifacts and cleanup. It does not yet bind an independently reviewed per-entry procedure inventory. Non-ATTA manual instructions and dynamic scripts require explicit procedure review. No assessment or fabricated observation has been created.

## Completion ordering

Complete every remaining operation implementation, platform/default behavior, caller integration and regression source; complete source-bound per-entry procedure inventory and review requirements; reconcile the complete generation checklist. Only then build, run/fix tests, execute full conformance and independent manual assessments, and run final qualification against the exact same candidate. Current generated sources and historical evidence remain unqualified.

## Continued generated sources: file selection and expanded AX

Still source-only and unrun:

- `ElementCommand::{FileInput,SetFiles}` and `Session::upload_element_files` now distinguish file controls from keyboard controls. `BrowserContext::webdriver_upload_files` authorizes FileUpload before opening paths, requires regular absolute files, bounds count/bytes, and retains display name, bytes, common-extension MIME type and modification timestamp in `WebDriverUpload`. Paths remain parent-local. Native SetFiles validates current reference/lifetime/type, appends for multiple controls, rejects invalid counts before mutation and dispatches native trusted input/change. File-input click now reports invalid argument; strictFileInteractability invokes native focus when enabled.
- Regression sources: `native_element_file_selection_retains_real_bytes_and_dispatches_trusted_events`, `webdriver_upload_authorization_precedes_file_access_and_retains_metadata`. Newly focused contenteditable hosts place selection at the end; existing focused selection remains intact.
- `OrynAccessibilityProtocolPart` implements the pinned upstream property operations previously inherited as NotImplemented. POST `oryn/accessibility/{element,node}` uses `ElementCommand::AccessibilityProperties`, existing native SemanticGraph properties/ownership, separate `ax-` identities backed by scoped native element capabilities, parent/children links, role/label and checked states. Snapshot/reference budgets and stale-element checks apply.
- Regression sources: `accessibility_properties_use_native_checked_state_and_stable_scoped_tree_ids` mirrors the pinned Core checkbox/tablist cases; `NativeAccessibilityContractTests` covers transport binding.

These additions do not close all input or AX work. AX projection must still be reconciled with live layout visibility/generated content, ignored-node flattening and all upstream property consumers. File MIME inference is a bounded common-extension mapping; HTTP upload integration, policy error classification, filename edge cases and strict-file behavior need full source review. Input interruption cleanup and option special behavior remain open.

Permission, geolocation, RPH and minimize require context-owned native state that survives document replacement and is visible to newly created frames/documents before author execution. Adding endpoint acknowledgements or realm-local author-visible state would not satisfy those contracts. Their parent/native binding is now generated below; complete platform/default behavior and lifetime verification remain required.


## Continued generated sources: context-owned platform operations

All entries below are generated, source-reviewed bounded work; none are build- or runtime-verified. Context45/worker14 remain pending compatibility contracts for this single unbuilt generation batch.

- [x] Shared typed platform query contract: `oryn-js::platform_control::{PermissionDescriptor,PermissionState,Coordinates,Request,Response,Backend}`; descriptor normalization and coordinate range regression sources.
- [x] Context-owned grant/position/visibility/RPH state: native `platform_control::State`; bounded identities, explicit opaque-origin denial and missing-position errors; document replacement does not erase grants. Parent mutations wake native event processing after releasing state locks.
- [x] Creator-authenticated worker queries: context service `NativePlatformQuery` validates socket ownership and exact native window document generation/epoch. Author raw context bridge cannot access it. No grant/emulation mutation is delegated to workers.
- [x] Native `Permissions.query` / PermissionStatus state and trusted change event binding; thrown descriptor-getter exceptions become exact Promise rejection reasons. `permission_visibility_and_location_share_native_state_without_author_shims` includes native state changes and getter rejection preservation.
- [x] Native geolocation queued callback/watch binding and parent position override; visibility defers delivery; missing provider returns positionUnavailable rather than invented coordinates. This bounded binding does not yet implement the full Geolocation interface below.
- [x] Native document hidden/visibilityState and visibilitychange driven by parent state; minimize and window rect restore share this state. Invalid rectangle validation precedes restore side effects.
- [x] Pinned HTTP permission/RPH/minimize routes and required async permission/geolocation parts; dedicated asyncio loop advances queued upstream async handlers before classic message wait. `NativeAsyncOperationContractTests` covers ordering, scoped Window/Frame conversion, preservation of unknown references and omitted versus null fields.
- [x] Native RPH same-origin handler registration and context-owned mode; accepted custom-scheme targets resolve before queue/OpenPage/navigation admission. Frame resource navigation applies handler substitution before CSP/network checks and rechecks original form authority against the substituted URL. Non-navigation resources retain original URLs. `handler_navigation_substitutes_encoded_canonical_url_without_rewriting_resources` covers canonical encoding and substitution.
- [x] Driver integration regression source `native_platform_routes_share_permission_location_and_visibility_state` covers grant/query, override callback, minimize, rejected rect, document replacement, restore and RPH scheme validation.

### Required remaining generation (unchecked intentionally)

- [ ] Permission-related Core native consumers: camera/microphone/usermedia elements and capture/cancel, geolocation elements and `:granted`, fetchLater background-sync permission, storage-access behavior. Endpoint acknowledgements and grant-query objects do not satisfy these consumers.
- [ ] Complete permission-consumer/embedded-origin rules beyond the geolocation/camera/microphone native query enforcement below; worker query support where pinned Core reachability requires it. Distinct embeddedOrigin is explicitly unsupported today.
- [ ] Retained PermissionStatus wrapper/GC lifetime review. Native position objects/options/cache/error/callback and pinned top-level-only override rejection sources are detailed below.
- [ ] RPH complete frame/form/custom-scheme and charset matrix, handler removal/prompt lifecycle and service-worker interception dependencies. Registration mode none currently returns explicit unavailable-prompt failure; it does not fabricate user acceptance.
- [ ] Minimize/page activation/pageswap lifecycle beyond visibility; all default consumers and source regressions.
- [ ] Remaining input interruption cleanup, option click behavior, full file input edge cases and AX visibility/generated-content/ignored-node projection.
- [ ] Full direct and transitive pinned Core operation reachability review, resolving 159 unresolved and 44 dynamic/malformed helper references without counting helper definitions as exercised calls.
- [x] Generate the 752 exact manual/visual source procedures and bind inventory to qualification; actual independent observations remain ungenerated. See current source status.

Only after all generation is complete: build, run/fix regression suites, full upstream execution, independent manual/AT/visual assessment and final exact-candidate qualification. Historical run evidence remains unchanged.


## Geolocation object, option and security generation — continued

Pinned source review included `interfaces/geolocation.idl`, `geolocation/PositionOptions.https.html`, `getCurrentPosition-success.https.html`, `tojson.https.window.js`, `non-secure-contexts.http.html`, `non-fully-active.https.html`, and the Permissions Policy cases. Sources remain unbuilt and unrun.

- [x] Native `GeolocationPosition`, `GeolocationCoordinates` and `GeolocationPositionError` prototypes, illegal constructors, native brands, read-only accessors and standard error constants. Position/coordinate constructors are exposed only in native secure contexts. Payloads live in GC-traced private arrays, not an unbounded native result registry. `toJSON` reads private payloads and ignores author replacements of getters or nested `toJSON`.
- [x] PositionOptions dictionary conversion follows lexical member order, Boolean conversion and unsigned-long Clamp (including NaN, infinities and ties to even). Conversion failures precede request insertion. `clearWatch` converts a required signed long; positive watch IDs remain signed-long bounded.
- [x] Native per-realm last-position cache retains acquisition timestamp/monotonic age and accuracy hint; maximumAge and accuracy mode control eligibility. Permission denial takes precedence over cache/timeout. A zero acquisition timeout skips provider acquisition and produces TIMEOUT unless a valid cache can satisfy the request. Positive timeout measures the existing terminal native provider call. Visibility waiting does not consume acquisition time.
- [x] Watch results suppress unchanged location/error repeats on unrelated parent revisions, terminate on denial, and remove on clearWatch. Native callbacks remain asynchronous; callback exceptions report through the callback's native realm. Retiring a document releases pending callbacks/cache; invoking retained retired-document Geolocation returns watch ID zero and queues POSITION_UNAVAILABLE to the live error-callback realm.
- [x] Production security publication now precedes permission/location consumption: native realm and ancestor secure-context state is published through the existing authenticated worker service. Parent `native_platform_allowed` evaluates committed document headers, ancestry and iframe container snapshots, default-self and explicit delegation, inherited denial, and exact native document lifetimes. Permission/geolocation queries cannot turn an automation grant into authority for an insecure or policy-disabled document. Camera/microphone query results also respect these gates; their media consumers remain open.
- [x] Regression sources expanded in `permission_visibility_and_location_share_native_state_without_author_shims`: brands/readonly/toJSON, author getter poisoning, cache/zero-timeout/accuracy mismatch, dictionary getter ordering/throws, exception reporting, duplicate watch errors, secure constructor exposure and retired-document callback delivery. Driver `native_platform_routes_share_permission_location_and_visibility_state` now includes native JSON/brand and timeout results.
- [x] Native enforcement regression sources: `production_query_cannot_turn_a_grant_into_insecure_or_policy_disabled_location`, `default_self_explicit_delegation_and_ancestor_denial_use_committed_native_chain`, `insecure_committed_ancestor_disables_a_trustworthy_child`, and `geolocation_policy_uses_feature_specific_headers_and_container_origins`.

Remaining scope is explicit: there is no hardware geolocation provider (absence is POSITION_UNAVAILABLE), no partitioned embedded-origin grant storage, and frame-target overrides correctly reject invalid argument per the pinned BiDi test (they are not a missing support obligation). The terminal native provider API does not yet model an independently pending/cancellable acquisition. Permissions Policy report delivery and policy mutation/navigation matrix remain review work; this source review does not establish whole-Geolocation, whole-G, or conformance completion. Newer `accuracyMode` extensions beyond the pinned IDL must be reconciled with actual required caller reachability before being claimed. All regression execution is deferred by the global generation-first gate.


A further pinned-source correction: `webdriver/tests/bidi/emulation/set_geolocation_override/invalid.py::test_params_contexts_iframe` requires **invalid argument for a live iframe target**. Session resolution now distinguishes that from an unknown/stale context and rejects it before mutations; the driver integration regression generates a real native frame reference and checks the rejection. `coordinates.py` establishes omitted accuracy defaults to 1; shared coordinate deserialization now applies that default, with omitted/null regression coverage. Stationary speed produces null heading in native position projection. These are source-only corrections to previously over-broad remaining-scope wording.

## Permission-element production consumer generation — continued

Pinned source review: `interfaces/geolocation-element.idl`, `geolocation-element.tentative.idl`, `usermedia-element.tentative.idl`; `html/semantics/permission-element/granted-selector.https.html`; usermedia constraints combinations/sanitization, untrusted-click, iframe and oncancel cases; geolocation-element get-current-position-error. This is generated source evidence, not an execution result.

- [x] `HTMLGeolocationElement` and `HTMLUserMediaElement` IDL, native tag/constructor mapping, node getter/setter/operation dispatch, native IDL/content event-handler plumbing. Geo reflects autolocate/watch; exposes native position/error and permission status snapshots. Usermedia exposes constraints/error event routes; the continued capture batch below adds native stream success.
- [x] New `v8_bindings/permission_elements.rs` consumes the existing authenticated parent permission and location state. Connected elements (including shadow trees) receive aggregate camera/microphone or geolocation status; initial status remains fixed. Native parent changes refresh current status, mark style mutation, and revoke `:granted`. Autolocate/watch uses native position/error objects and trusted location events; no author property is grant authority.
- [x] Native pointer/keyboard/input activation and author click/dispatch are distinguished. Untrusted usermedia activation produces an asynchronous InvalidStateError event and cannot start capture. Canceled click suppresses the default action. Constraint dictionaries are converted before replacement, preserving thrown getter identity and the previous constraints on failure.
- [x] DOM `permission_granted` is restricted to the native supported element kinds. Document adoption clears it; records and pending actions are filtered by current document ownership. Retirement clears queued element observations. A native attachment delay participates in host timer selection; geometric visibility/hit checks gate trusted activation. These are bounded initial blockers, not complete locked-style/intersection behavior.
- [x] `:granted` is a real Stylo pseudo-class, with native query matching, DOM fallback matching, `LayoutSelectorState` transport, `rust_executable` population and Blitz style matching. It is not an authored attribute or a rewritten selector. Basic UA control geometry lets native input target these new element types; full reference appearance remains open.
- [x] Regression sources: `parser_elements_consume_native_grants_and_untrusted_activation_never_captures`, `granted_state_is_native_element_specific_and_revoked_on_adoption`, `granted_query_uses_native_state_with_stylo_negation_and_lists`, `granted_selector_uses_native_permission_state_and_restyles_on_revocation`.

### Remaining production work for these consumers

- [x] Bounded usermedia success path generated in the continued capture batch below: parent provider, stream/track custody, explicit fixture devices, native activation, stop/clone/retirement and regression sources. This checkbox records source generation only; no capture run or family conformance is established.
- [ ] Picker cancel behavior, complete constraint Web IDL conversion and device selection. Native iframe-specific delegation/activation checks are generated below, pending execution and full mutation/navigation matrix review.
- [ ] Full geolocation/permission element activation blocker semantics, style locking/bounds, UA labels/icons, child/pseudo suppression, intersection-change delay and validation notifications; the basic box and hit checks above do not satisfy the full rendering/manual matrix.
- [ ] Promptaction/promptdismiss production UI flow; current generated IDL handlers do not establish a native permission prompt implementation.
- [ ] Broader permission consumer coverage (fetchLater/background-sync and storage access), complete transitive operation reachability, and 752 independently reviewed manual/visual procedures remain as before.

All builds, generators, tests, renders and conformance execution remain deferred. No whole-element-family or whole-G completion is claimed.


## Usermedia capture provider and ownership generation — continued

Source-only; no builds, generators, tests, renders or conformance runs. The shared context45/worker14 contract remains a pending unbuilt batch. Historical fixture/run evidence is unchanged.

- [x] `oryn-js::media_capture` defines typed capture/inspect/stop/release/clone/enable requests and native stream/track replies. Parent `media_capture::Registry` owns native audio PCM/video RGBA fixture samples, opaque identities, exact Window owner, independent clone state, shared sample lifetime, track-count and 64 MiB unique-source bounds. Both sources are allocated before publication; unsupported exact requirements fail without partial tracks.
- [x] `ContextOptions.media_devices` defaults to `None`. `SyntheticFixtureV1` is explicit parent configuration, also carried in the validated hash-bound WebDriver fixture contract. Historical omission means no device. Permission grants never install a device; absence returns NotFoundError. Fixture example configuration declares its synthetic mode visibly.
- [x] Production `BrowserContext::native_platform_query` validates current document owner, secure context, native camera/microphone policy and grants, explicit iframe delegation and consumed native transient activation before capture. It rechecks authority and grants after allocation and reclaims failed capture. Native permission denial ends affected tracks; root/page/frame retirement releases registry custody.
- [x] `v8_bindings/native_media.rs` implements native MediaStream/MediaStreamTrack EventTarget wrappers, stable track identity, stream membership, active/readyState, enabled, native settings/constraints, stop, independent clone and trusted ended notification on parent revocation. Stream tracks live in private GC-traced arrays; author id/getter replacements cannot become custody authority. Failed clone/wrapper allocation rolls back native sources. Unacknowledged cleanup retains a bounded retry queue; transport failure does not fabricate terminal evidence.
- [x] Permission-element trusted activation passes converted constraints to the native provider and publishes stream/error events; default/untrusted/denied behavior remains distinct. MediaStream/MediaStreamTrack interface declarations and typed HTMLUserMediaElement.stream accompany the bindings. Native host timer selection includes cleanup retries.
- [x] Generated regression sources: parent `grants_are_not_devices_and_failed_constraints_do_not_publish_partial_capture`, `synthetic_tracks_own_samples_clone_independently_and_retire_by_native_principal`, `unsupported_exact_device_or_processing_constraints_never_publish_capture`; security production caller `native_capture_requires_grants_secure_authority_activation_and_explicit_frame_delegation`; V8 `native_track_identity_iterator_close_clone_rollback_and_cleanup_retry`; driver `native_usermedia_capture_tracks_stop_and_clone_only_with_explicit_fixture_devices`; Rust fixture `capture_devices_require_explicit_hash_bound_fixture_configuration` and Python fixture mode validation coverage.

### Remaining capture scope and qualification

- [x] Continued constraint batch below adds pinned MediaTrackConstraintSet conversion, native settings selection/advanced backoff, capabilities and applyConstraints regression sources. Multi-device/hardware selection and independent conformance remain unverified.
- [ ] Physical camera/microphone provider integration, continuous sample consumption through HTMLMediaElement/WebAudio/recording and rendering. Synthetic sample buffers are native owned test devices; their creation is not evidence of playback, recording or hardware capture.
- [x] Native permission prompt/action/dismiss UX sources generated in the continuation below.
- [ ] Full device selection UI and element rendering/activation-blocker matrix remain open; the bounded prompt implementation does not close these independent obligations.
- [ ] Review lifecycle/permission races against final shared authority implementation, then execute all generated regressions only after the global generation barrier is complete.

Whole G remains open: full pinned operation/transitive reachability, input/AX gaps, other permission consumers, 752 per-entry manual/visual procedures, then independent actual qualification. No source checkbox in this section is a passing test or measured conformance result.


## Continued native media constraints, capabilities and applyConstraints

Generated source only; the generation-first execution gate still applies.

- [x] Pinned `interfaces/mediacapture-streams.idl` dictionaries/unions and range inheritance are reflected in source IDL and native conversion: unsigned-long Clamp with ties-to-even, finite doubles, boolean/string unions, iterable string alternatives, inherited range getter order, advanced sequence dictionaries, bounded conversion and IteratorClose. Getter exceptions survive synchronous setConstraints and Promise-returning applyConstraints without replacing previous native constraints.
- [x] `oryn-native::media_constraints::{select,capabilities}` selects settings for the explicitly configured single native fixture device of each kind. Required exact/min/max constraints intersect; basic ideals guide fitness, integer video dimensions and aspectRatio are coupled, unsatisfiable advanced sets back off without replacing earlier feasible sets. Native IDs, facing/resize and processing capabilities are truthful; unsupported exact processing requests fail.
- [x] Parent ApplyConstraints atomically replaces one owned track's source/settings only after successful selection and source-budget admission, preserves IDs/enabled, and leaves clone sources independent. Failed selection retains the prior track; ended tracks cannot restart acquisition. Security/permission authority is rechecked after acquisition.
- [x] Native getCapabilities and Promise applyConstraints, standard typed IDL, and native `OverconstrainedError : DOMException` with constraint name. Generated error objects use intrinsic native prototypes and exact thrown getter reasons, independently of author constructor replacement.
- [x] Regression sources: `required_intersection_and_advanced_backoff_preserve_feasible_settings`, `device_identity_processing_and_coupled_dimensions_are_truthful`, `apply_constraints_replaces_only_owned_track_after_success_and_preserves_state_on_failure`; V8 conversion/error/Clamp/sequence checks and driver native apply/failed-apply preservation added to the capture regression.

Remaining: independent review and execution of the complete selection/error/IDL matrix, device selection across real multiple devices and continuous sample/rendering consumers, native picker/prompt/cancel UX, complete permission-element geometry/style/intersection blockers and manual reference rendering. No implementation or conformance claim is inferred from these unrun regression sources.


### Pinned cancellation and focus corrections

- [x] Pinned `usermedia/usermedia-cancel-prompt.https.html` explicitly simulates denial with the native permission command. Trusted denied capture now emits **cancel**, sets NotAllowedError and clears the element stream without firing error/stream. The driver regression includes this exact event/result contract. This does not claim a real prompt UI exists.
- [x] Pinned `usermedia/no-focus.html`: script focus requires native transient activation, while native pointer focus remains allowed. `focus_html_element` now enforces that distinction, with `usermedia_script_focus_requires_native_activation_and_pointer_focus_remains_allowed` regression source. Author event dispatch cannot grant the activation.

Remaining picker and blocker scope is unchanged: actual permission prompt UI/action/dismiss flow; style locking, bounds, icons/labels, pseudo/child suppression, ancestor masks and intersection-change delays require further production generation and source regression coverage.


## Native permission prompt and control presentation continuation

**Source reviewed/generated, not built or executed.** Earlier notes saying no real permission prompt exists describe the earlier capture batch and are superseded by this section. Whole G remains open.

- [x] Parent-owned opaque prompt identities, exact owner/replay checks, bounded pending registry, explicit allow/deny/dismiss and atomic two-permission updates: native `platform_control::{begin_prompt,answer_prompt,cancel_prompt}` and `runtime::native_platform_query`. Opening and answering require separate native activation; no author event creates a grant. Owner/page retirement removes pending prompts.
- [x] Production private prompt presentation: DOM `PermissionPrompt` and top-layer state; layout `permission_prompt::append`; JS `permission_prompt::{open,pointer,keyboard,answer,fixup,accessibility}`. Native pointer and keyboard interception precedes author event dispatch. Exact snapshot/parent-issued identity governs the shared AX answer path; S/V provides platform virtual button integration. Dismiss/denied usermedia produces cancel and NotAllowedError; geolocation delivers prompt events and rechecks document lifetime after callbacks.
- [x] Native permission element style projection before main/subtree layout: `permission_controls::{adjust,project,measure}`, child/pseudo suppression, native label shaping, bounded font/spacing/size/padding, forbidden effect/image reset, ancestor text-decoration barrier. Nearest language label changes and granted state project from native state. Geometry/style/ancestor masking and intersection-change delay are connected through `permission_elements::invalid_reason` and poll.
- [x] Typed permission icons: common `BuiltinImage::PermissionIcon` constructor, generated-content private descriptor and `PaintNodeSnapshot::builtin_image`, `PaintCommand::BuiltinImage`, native SVG raster provider capped at 1 Mi pixels. Static SVG has no arbitrary markup, URL, href, image, font or filesystem resolver. Author URL resources and CSP admission have no native-prefix exception. Print uses the same bounded raster path; icons do not enter semantic text extraction.
- [x] Regression sources: native `explicit_native_choice_mutates_both_grants_and_rejects_foreign_or_replayed_prompt`, `dismissal_and_retirement_do_not_create_permission`, `production_permission_prompt_requires_distinct_native_open_and_answer_activation`; driver `native_permission_popup_accepts_only_a_fresh_trusted_choice`; layout `locked_native_control_style_and_private_label_share_the_layout_projection`, `native_permission_popup_paints_three_choices_without_author_nodes`, `permission_icon_uses_private_typed_paint_and_cannot_request_author_images`; renderer `typed_native_paint_honors_colors_and_bounds_without_resources`, `native_icon_authority_cannot_be_selected_by_author_url_or_tag`.

### Remaining generation and subsequent qualification

- [ ] Complete pinned geometry/intersection/ancestor-effect/blocker and reference-rendering matrix, including rounded clipping, transformed ancestors, style races, every locale/text variant and author event cancellation regressions. Existing finite hit samples are not proof of full visible-region containment. Native icon shapes need independent reference comparison.
- [ ] Real multi-device/hardware provider and device selection UI; continuous native sample consumers through media elements, WebAudio and recording. Explicit synthetic fixture buffers do not prove these consumers exist.
- [ ] Complete transitive pinned operation reachability, remaining input/AX/permission consumers and per-entry manual/AT/visual procedures (historical inventory: 752 entries).
- [ ] Once **all** packages have generated code and regressions, build the complete batch, fix failures, execute complete regressions and then verify conformance with exact candidate provenance. No earlier execution is permitted by the user's generation-first directive.


## Physical capture and continuous media continuation (generation in progress)

No compiler, tests, SDK probes, hardware access or generators have been executed.

- [x] Read installed macOS 26.5/15.4 AVFoundation, CoreMedia, CoreVideo and AVFAudio headers. Generated parent-only Objective-C bridge in `media_capture/macos.m`: bounded device/format inventory, actual capture sessions, RGBA video/float PCM callbacks, OS authorization checks and independently owned stop/drop. Parent app usage descriptions added; worker receives no device pointer or Objective-C entry point.
- [x] Native System provider route and per-context salted device identifiers; selection considers available device formats, required constraints, ideals and feasible advanced sets. System provider is selected for ordinary new ContextOptions; loopback fixture contexts and historical omitted serialized configuration retain explicit no-device semantics. OS denial/unavailable devices are errors, never synthetic fallback.
- [x] Continuous native Sample route with exact owner, monotonic sequence/timestamp metadata, missing/retired/end handling, disabled black/silent samples and explicit dropped-sample counts. Owner and permission are checked again after sample retrieval. Track stop/clone/release/applyConstraints preserve separate custody and capability metadata.
- [x] Generated stream-backed media element `srcObject`, play/pause and first-frame state, video dimensions, native PNG paint publication and canvas drawImage(video). Native stream identity is checked; author getTracks/id properties cannot supply custody. Parent AudioRender uses bounded AVAudioEngine output queues; muted output needs no hardware sink. Pause/source replacement/track stop/revocation retire audio sinks.
- [x] Regression source: physical candidate selection across devices/salted IDs/required failures/advanced backoff; parent sample ownership/disabled pixels/retirement; driver stream-backed video play/canvas sampling/source identity and replacement. These are generated sources only.

Still open in this batch: complete physical format/rate/processing capability selection, asynchronous OS first-permission completion UX and device selection UI, audio discontinuity/backpressure and hardware-loss/event propagation, complete media-element state/event/autoplay/seek and callback-race semantics, nested-frame/drag/print media capture review, MediaRecorder/WebAudio consumers, and the full visibility/reference/operation matrix. A generated native bridge and basic production consumer do not close these remaining obligations or qualify hardware capture.


### Async OS completion, loss observation and required-scope reconciliation

- [x] Parent Capture now returns an opaque Pending identity while OS permission is outstanding. PollCapture resumes the exact stored constraints once, after both native grants; CancelCapture is exact-owner/idempotent. The registry bounds requests to 64 and expires them after 120 seconds. Resume uses original native activation authority; it does not forge a new click or consume a second activation. Each resume rechecks current parent principal, delegation and permission.
- [x] Native permission elements retain pending capture tickets, poll without producing premature cancel/error, and cancel detached/adopted/invalid/denied consumers. Parent retirement removes pending tickets. `pending_capture_is_owned_cancelable_expiring_and_resumes_once_without_new_activation` supplies positive/negative regression source without touching OS hardware.
- [x] Physical capture observes AVFoundation session runtime errors and device disconnection; native Inspect polls alive state without exporting sample bytes. Track observation runs on a bounded timer even without playback, and ended tracks retire samples/audio outputs. Media elements observe ended streams. Source review removed iOS-only output-dimension properties after reading their explicit macOS-unavailable SDK declarations.
- [x] Physical candidate generation includes advanced frame-rate breakpoints; selection can retain an interior requested rate instead of incorrectly discarding that advanced set. Selected device/format remains native-owned.
- [x] Scope evidence: read-only search of the exact pinned `html/semantics/permission-element` subtree finds no MediaRecorder, AudioContext, srcObject, getUserMedia or enumerateDevices references. Original completion plan also names no WebAudio/MediaRecorder obligation. These APIs remain excluded unless authoritative Core/five-family transitive reachability establishes a caller; the root explicitly confirmed preserving original plan scope. Native permission choices and actual deviceId/format selection are generated; a separate multi-device picker UI is not inferred from this subtree.

Remaining required review: capture permission completion races and every lifecycle/error caller, physical setting fidelity and supported constraints, audio backpressure/discontinuity, frame/print/drag capture integration, full permission-element visibility/reference requirements, remaining pinned operation reachability and exact manual inventory. Required-scope exclusions do not certify global Core reachability: that inventory remains open. All new sources remain unbuilt and unrun behind the all-generation gate.


### Closed production media failure/callback/backpressure gap

- [x] `media_element::poll_one/poll/fail_playback` contains sample/sink failures at the owning media element: pending play promises reject, playback pauses, native MediaError code/message and trusted error event are published. Failures no longer escape through the sample consumer into the document event loop.
- [x] MediaError source IDL, native object identity/getters and media error attribute are connected. Resetting srcObject clears the prior error and preserves independent mute/volume controls.
- [x] Per-element session generations invalidate old callbacks even when srcObject is assigned the **same** native stream. A loadedmetadata callback that resets or pauses the source prevents stale loadeddata/canplay/playing delivery and rejects the old play promise with AbortError.
- [x] Pause/replacement/error attempts native audio stop immediately and retains failed cleanup for bounded timer retries; it does not abandon output custody when transport fails. Parent retirement remains final owner cleanup.
- [x] AVAudioEngine queue pressure is distinguished from output failure: full queue returns explicit backpressure without rejecting playback or advancing its accepted sample cursor. The consumer retries on the next bounded poll. Bad/unavailable output remains an error.
- [x] Native track observation transport failure produces temporary muted state and one mute event rather than fabricating ended state or aborting the event loop. Successful observation clears muted with unmute; parent-confirmed loss still ends the track.
- [x] Regression source `playback_sample_failure_settles_promises_and_same_stream_replacement_retires_old_callbacks` covers genuine injected provider error, native MediaError/trusted error event, pending backpressure and recovery, same-stream reentrant reset, preserved controls, and observation mute/unmute recovery. No execution occurred.

This closes the specific previously documented sample-error and same-source callback escape paths in generated production sources. It does not mark G or the whole media API matrix source-complete. Remaining required work continues with capture variants, permission visibility/reference algorithms and authoritative operation reachability.


### Native capture variant reconciliation

- [x] Confirmed existing `snapshot_native_paint_resources` iterates native decoded image stores by node, so video frames already flow through `RawV8Host::capture_paint_images`, selected-frame resolution and drag-image capture without author resource lookup. Added a regression assertion reading the actual captured PNG and matching its RGBA pixels to the delivered fixture frame.
- [x] Closed missing root SVG foreignObject consumer: `capture_svg_paint_tree` now binds the native video frame PNG into the local HTML paint node's image resource under the same aggregate capture budget. The driver capture regression moves a paused live-source video into foreignObject and asserts its native image resource is present. Root page/print snapshots share this path; selected frames use their existing realm-scoped decoded-resource binding.
- [x] Clone/import review was coordinated with root: both clone_node and import_shallow_node allocate fresh native nodes and copy explicit fields, so media_frame and image_dimensions already remain unset. Root owns the accompanying clone/import isolation regression; no unnecessary clone implementation change was made.

Remaining visibility work is specifically the finite-point occlusion approximation, full clipping/distortion and callback/style race matrix, pinned visual comparisons and remaining operation/manual reachability. Capture producer/consumer sources and regression assertions above remain unexecuted.


## Continuous native permission visibility generation

- [x] Removed the five-point hit-test loop from `permission_elements::invalid_reason`. Its production caller now uses `permission_visibility::assess` over native paint order and full projected regions.
- [x] Shared `oryn_common::visibility` clips convex projected fragments against viewport and every native ancestor clip as continuous half-planes. Full target coverage is compared before/after clipping; singular/camera-crossing/nonfinite geometry cannot certify visibility. Occluder intersections examine region area, including thin interior overlays between every former sample point.
- [x] Native visibility considers painted content independently of pointer-events, ignores fully transparent/hidden paint, respects stacking and generated descendants, and handles inert controls. Effective translation/uniform upscaling remains admissible; distortion, uncertain filter footprints and unsupported geometry fail closed.
- [x] Ancestor mask detection now emits `style_invalid`, matching pinned `parent-mask.tentative.html` and corresponding geolocation/usermedia requirements, rather than the previously incorrect generic occlusion reason.
- [x] Box-shadow parsing and projected paint bounds are shared between renderer and visibility via `oryn_common::box_shadow`; a shadow originating outside the target border can still obstruct it. No duplicated divergent shadow-footprint formula was introduced.
- [x] Permission prompt owners no longer accidentally enter top-layer paint/hit ordering themselves: page/drag/native hit and escaped-root membership exclude the owner, while private prompt geometry retains its native top-layer placement. This aligns visibility ordering with rendering.
- [x] Visibility-state changes participate in the native attachment/intersection settling timer, so removing an overlay cannot immediately reactivate a control while its new visibility is still settling. Initial attachment shares its existing timestamp rather than adding a second delay.
- [x] Regression sources: continuous thin occlusion, one-axis ancestor clipping, transformed disjointness, singular/camera-crossing targets; native permission consumer with pointer-transparent thin overlay, fully transparent overlay, inward/outward off-box shadows, ancestor mask and subpixel clipping. No tests or generators executed.

Remaining exact scope: conservative paint bounding regions for glyph/image alpha and unresolved filter footprints need required-reference review; this implementation never treats those uncertainties as proof of visibility. Full pinned visual/reference and operation/manual reachability reconciliation remain open. The analytic coverage change closes the confirmed finite-point false-visible gap, not all G conformance.


### Pinned icon reference follow-through

Read the exact pinned geolocation icon margin/height reference pairs. The margin test sets 100px at font-size 10px and matches a 30px reference (3em), while height 100px matches 15px (1.5em). Height was already bounded; margin-inline-end was not. The native projection now clamps it to [0,3em], maps it into the correct physical inline-end margin for direction/writing mode, and preserves inherited typography. Regression source asserts the bounded logical and physical margin as well as height/native typed paint. These references compare native elements with each other, so no external bitmap/icon asset replacement is inferred.

## Current G reconciliation: pinned visibility and missing element callers

This section supersedes earlier future-work lists where later sections supply source evidence. It does not certify execution or close aggregate G cells.

### Bounded visibility correction generated

`permission_visibility::assess` now joins `BindingLayout::text_clusters` (keyed by DOM text node) to each cluster's native `layout_node` paint owner. Previously its element-only lookup could omit text occluders entirely. Individual native cluster rectangles retain their transform and ancestor clips; text-only owners no longer implicitly paint their entire border rectangle. Transparent computed text color contributes no text coverage. The native visibility regression now supplies a text-node-keyed cluster, checks overlap, moves the cluster outside the target and checks transparency. These are regression sources, unrun. Cluster boxes remain conservative glyph bounds, not claims of exact glyph alpha.

Read-only search of all pinned `html/semantics/permission-element` HTML/JS found no glyph/image-alpha/occlusion assertion. Concrete filter/opacity assertions are `usermedia/invalid-css-properties.html` and `geolocation-element/invalid-css-properties.tentative.html` (computed filter must be `none`) and the restricted icon CSS references (author opacity/stroke-opacity must not alter the native icon). Ancestor mask cases require `style_invalid`; this has a generated native caller and regression. Thus exact arbitrary glyph alpha and unknown-filter paint extents remain visibility limitations, but this search does not establish them as individual pinned assertion blockers. It also does not establish whole-Core helper reachability.

### Confirmed required consumers missing from generated permission coverage

The preserved `inventory-v3/core-html-0014.inventory.json` contains all five `camera/` entries and twelve `install/` testharness entries. Their source files exist in the pinned checkout; these are not stale names or unrelated wdspec expansion.

- `camera/camera-track-attribute.https.html` requires `HTMLCameraElement`, an initially null `track`, and writable `ontrack`. `camera/idlharness.window.js` explicitly loads `camera-element.tentative`. The other three entries exercise use cases, errors and constraints. Native permission `kind` currently accepts only geolocation/usermedia; generated usermedia support is not camera-interface support.
- `install/iframe-not-allowed.tentative.html` requires false `isValid` and `illegal_subframe` both with and without `web-app-installation *`. Other inventoried testharness entries cover feature detection, sandbox, focus, CSS bounds/display, masks and child suppression. Pinned install reference files additionally exist and require inventory/reference reconciliation. No native `HTMLInstallElement` binding was found. Generic prompt or geolocation/usermedia style locking cannot be counted as this implementation.

### Exact checklist disposition for parent reconciliation

| Cell | Current source evidence | Required remainder |
| --- | --- | --- |
| G-01.a/b | Full inventory identity and raw/qualified separation generated | Per-entry observed cause/change/evidence ledger comes from later execution; preserve historical results |
| G-02.c | Bounded native routes and consumers in this audit | Whole direct/transitive operation inventory, confirmed camera/install callers, remaining default/lifecycle semantics; do not close from endpoint presence |
| G-02.e | File upload authority/bytes/metadata/SetFiles and contenteditable initial caret already generated | Option-click special behavior, interrupted input cleanup, strict-file/edge cases and selected-document cookie/element lifetimes still require source reconciliation |
| G-02.f | Parent permission/BiDi geolocation/RPH/minimize/expanded AX routes plus native negative regression sources generated | Split completed route/state work from all consumer/lifecycle coverage; camera/install, embedded-origin grants, permission consumers outside capture/location, default RPH prompt, activation/pageswap and complete AX projection remain unclosed |
| G-02.g | Static scan and bounded queued-navigation native outcomes documented | Resolve 159 unresolved helper references and 44 dynamic/malformed URL literals; connect terminal navigation outcomes to automation with pruning/truncation markers. Historical scan counts are not a complete denominator |
| G-03.b/G-04 | Explicit fixture contracts and native routes exist | Source-level wrapper/helper/MIME/alias/redirect/ws/h2 matrix plus real topology provisioning/peer/TLS observations. Configuration does not close 17 blocked cases |
| G-05.a/b | Four automated executor classes, print/screenshot, independent assessment schema generated | These checked bounded source cells do not imply reference/manual completion |
| G-05.d/e | Procedure denominator established: 749 manual + 3 visual | All 752 individual source-bound procedures/platform assertions still need complete inventory; schema does not provide the procedures |
| G-05.c | Deferred execution cell | Actual independent manual/visual/AT observations after generation |
| G-06.a/b | Canaries/qualification/shared placement sources generated | G-06.c remaining native fault/cleanup/TLS/focus control source audit, then later execution |
| G-07 | Qualification plumbing exists | Reference focus/normal TLS, source-bound reusable prerequisite reviews and qualified ledger require genuine observations |
| G-08.a | Separate historical runner retained | G-08.b actual old/new parity and independently attributed official baseline after generation |

The next generation priority is confirmed missing pinned production consumers and the 752 procedures/transitive operation inventory. Physical capture fidelity and visibility limitations remain explicit; no amount of conservative fallback or added endpoint acknowledgements establishes complete Core support. All builds/tests/generators and qualification remain deferred until the global all-generation gate.

## Camera/install required consumer generation — subsequent source batch

The prior section's missing-interface findings now have generated production paths. This updates those bounded findings only; G-02.c/f remain open for their other callers and lifecycle obligations. No code below was built or executed.

### Camera: five pinned inventory entries

- `browser.webidl` declares the exact pinned `HTMLCameraElement` constructor, nullable native track/error and track/cancel/error handlers. It also declares `setConstraints`, which the pinned camera testharness files invoke despite omission from `camera-element.tentative.idl`. Native tag/class lookup, getters, operations and handler dispatch connect these members.
- `permission_elements::{ensure,constraints,poll}` requests video only, removes complex/range and audio constraint members for the element constraint sanitizer, retains the native track wrapper and emits trusted track/cancel/error events. `native_media::first_video_track` reads private stream custody, validates one video track and never calls author-overridable stream methods.
- Parent capture previously always allocated both devices and checked both permissions. `Registry::capture_features/request` now honors explicit false track selectors, carries exact requested features through original/pending requests and constructs all requested sources before publishing identities. Runtime authorization/delegation/post-acquisition rechecks and macOS OS authorization query only the requested capture features. No microphone permission or device is invented for camera-only capture.
- `PromptKind::Camera` has exactly the camera permission. The existing parent-issued prompt identity, trusted choices, native AX/input and atomic answer pipeline are reused; its private UI title names only camera. Mic revocation does not erase an independently pending video-only acquisition; camera revocation still cancels it.
- Native regression sources: `camera_only_capture_never_allocates_audio_and_mic_revocation_preserves_pending_camera`; `camera_prompt_and_install_eligibility_use_exact_native_features_and_committed_policy`; driver `native_camera_video_only_and_install_policy_bindings_follow_pinned_callers` (camera granted with microphone denied, real fixture video, identity/dimensions/trusted track event, untrusted-click error and denied-permission cancellation). Default no-device mode remains unchanged and no physical device was used.

### Install: twelve pinned inventoried testharness entries

- `HTMLInstallElement` has native constructor/tag identity, `isValid`, `invalidReason` and validationstatuschange handler/event dispatch. `InstallEligibility` is a creator-authenticated native query. `native_install_invalid_reason` reads committed CSP policy and returns `illegal_sandbox` before `illegal_subframe`; author allow/sandbox attributes in the consuming document cannot manufacture eligibility. The generic origin-only platform registry refuses this request because it has no document custody.
- The existing permission visibility/style/timer pipeline applies to install: display eligibility, ancestor masks, native clipping/occlusion and delayed geometry changes. Install does not invoke camera/location grants or capture as an accidental fallthrough.
- Install and camera now participate in shared locked CSS, private generated labels/icons, child/pseudo suppression and text-decoration inheritance barriers. Script focus requires native transient activation; native pointer focus remains available. New typed install icon identity uses the same bounded native SVG paint path, with no author URL privilege.
- Pinned `negative-offset` requires arithmetic expressions to clamp to zero: shared native CSS math now resolves outline-offset before nonnegative projection. Pinned `bounded-sizes` requires equal opposite padding based on left/top; shared permission controls now apply those symmetric values. Tests cover both correction paths and control-specific private labels/icons.
- Driver regression covers constructor identity and unactivated focus rejection, same-origin iframe rejection despite installation delegation, and sandbox precedence. The shared focus regression now checks install/camera script rejection, activation acceptance and trusted pointer focus. Parent policy regression also covers committed response CSP sandbox.

### What remains unqualified/open

These sources address the confirmed missing constructor/consumer paths; they do not establish passed IDL/testharness/reference results. The reference file matrix beyond the twelve testharness records, full style serialization/rendering comparisons, native frame lifecycle/security races and hardware-specific capture observations remain part of later comprehensive review/qualification. Whole G operation/helper reconciliation and 752 manual/visual procedure generation are still open. Context45/worker14 remain the pending compatibility contracts for the entire unbuilt batch; no historical artifact identity or raw result was rewritten.

## Transitive consumers, terminal navigation and procedure sources — subsequent generation batch

This section supersedes the earlier missing-source statements only for the bounded work below. No builds, tests, formatters, generators, services, hardware access or qualification ran.

### G-02.g terminal navigation evidence

`Session::queued_navigation_evidence` and `GET oryn/diagnostics/navigation` expose only the session's native context, including retired pages whose window handles have gone away. Native outcomes retain queue identity, terminal result, target, pruning and error-truncation markers. The official executor `NavigationEvidenceMixin` attaches before/after snapshots and the previous test boundary to raw results; it does not replace the raw test status. `wpt_full_qualification.navigation_errors` rejects outstanding jobs, collection failures, lost/rewritten history and newly truncated/pruned evidence. Expected failures require exact outcome records and independently source-bound navigation-error review. Native queue acknowledgements therefore cannot qualify terminal success. Native server and Python positive/negative regression sources cover the observation and qualification paths.

The old 159 unresolved/44 dynamic-helper scan was an exploratory regex count, not a saved authoritative obligation list. It is still **not closed**. Follow-up source inspection identified the official WebIDLParser serve rewrite, URL-authority template placeholders and intentionally missing resources among false positives, plus real misplaced imports. Complete occurrence-level helper reconciliation remains open; the new work must not silently replace the old count with another regex denominator.

### G-05.d/e complete procedure-source denominator

`wpt-procedures/core-manual-visual-2026-09-16.jsonl` now contains all **752** pinned Core manual/visual identities: **392** exact ATTA payloads and **360** other source procedures. Original URL/path/hash/revision, source instructions, helpers, control requirements, script assertions, expected observations and cleanup are retained. Observations and qualification remain null. `scripts/wpt_manual_procedures.py` binds each assessment to the exact generated source corpus and rejects changed procedures, helpers, missing observations and changed ATTA assertions.

Platform selection is explicit in each candidate-bound assessment. macOS selects AXAPI, Linux ATK, and Windows its actual native API; alternative source branches are retained without requiring mutually exclusive APIs on the same candidate. Missing source assertions for the selected native API remain an open obligation, never a vacuous pass. Required plan cross-platform gates remain separate. Regression sources cover source mutation, foreign-API substitution, positive AXAPI selection and missing native assertions.

### Declared fixture repair custody

`tools/wptrunner-oryn/oryn_wptrunner/fixture-repairs.json` declares twelve exact missing GET paths: ten dynamic manual sibling URLs with an erroneous `-manual` suffix and two misplaced helper imports. Each declaration binds original source bytes, request URL and target bytes to the pinned revision. `fixture_repairs.validate` refuses changed manifests, changed sources/targets, overriding existing files, ambiguous paths and duplicate routes. The official fixture environment installs only these validated rewrites before upstream servers start, preserving source/assertion bytes and inventory denominator; official run information retains the manifest hash. Candidate artifacts now require `fixture_repairs` and `manual_procedures`. Historical reports are untouched.

Manual assessments affected by the ten source defects additionally require a candidate-bound delivery receipt containing each exact GET response and target hash. Manifest validation alone is not delivery evidence. `oryn_wptrunner.repair_receipt` now generates receipts by fetching the exact declared resources from an explicitly selected official fixture origin, retaining failed/redirected/oversized responses and actual body hashes. Actual receipt collection/observation remains required before these entries can qualify. Regression sources cover changed targets, existing-path overrides and source inventory identity; no repair was served during generation.

### Confirmed native Bluetooth helper dependency

The pinned Core permission-policy getDevices tests reach the legacy `bluetooth_test` helper. Its reachable path requires secure `navigator.bluetooth`, an explicitly enabled native `.test.allResponsesConsumed()` endpoint, actual native policy enforcement, and an empty device list when no native origin grant exists. The BiDi/GATT branches in the same helper are not reached by these tests; their mere textual presence does not require fabricating a GATT implementation.

`ContextOptions::bluetooth_testing` defaults false and is an explicit fixture configuration. Parent `platform_control::State` owns origin device grants and exact owner-scoped pending response accounting; retirement clears the latter. Creator-authenticated `native_platform_query` checks native committed Bluetooth policy and carries enforce/report-only dispositions. The ordinary origin-only registry cannot answer these document-policy requests. Secure native Bluetooth bindings preserve SameObject identity, reject blocked operations, return actual empty grants, validate required empty-filter failure and expose the testing object only for the opted-in provider. No permission grant invents a device or a physical Bluetooth provider.

Two pinned Core Bluetooth reporting files also require ReportingObserver. `v8_bindings/reporting.rs` now provides native construction/brand checks, filtered bounded queues, observe/disconnect/takeRecords, buffered delivery, native report serialization and native asynchronous callback dispatch/retirement. Native Bluetooth policy responses enqueue violation reports with committed document URL and native call-frame source coordinates. Report-only delivery does not change the operation's raw Promise outcome; enforced denial remains SecurityError. Regression sources cover parent policy rejection/retired custody, real pending response accounting, native caller-to-observer enforce/report-only delivery, filtered/buffered serialization and retirement. This bounded implementation does not claim unrelated Reporting API worker delivery, HTTP report transport or physical Bluetooth selection/GATT conformance.

### Remaining whole-G generation/qualification inventory

- Saved occurrence-level classification is complete as source evidence; continue the remaining dynamic/conditional production behaviors identified by the operation matrix.
- Complete source-level operation/reference reachability reconciliation beyond the bounded Bluetooth/camera/install routes; do not infer operation completeness from endpoints alone.
- Preserve open native reference/rendering, exact glyph/image-alpha/filter occlusion and physical capture fidelity/lifecycle obligations already documented above.
- Execute generated repaired-resource receipt collection and independent observation, then post-generation manual/visual/AT execution and native branch gaps. All 752 procedure sources exist; their observations do not.
- Complete topology/TLS/fault-cleanup and old/new official baseline qualification after the global generation barrier. Pending context45/worker14 compatibility is still unbuilt and unvalidated.

## Saved helper/operation/reference reconciliation — continued source generation

The source audit now has durable companion artifacts under `wpt-procedures`:

- `core-helper-occurrences-2026-09-16.jsonl`: **1,199** exact occurrence records. Source hashes, lines, URL literals and classifications preserve the original evidence. The unsaved historical 159/44 regex count cannot be matched one-to-one honestly; the saved review includes its discovered classes and explicitly supersedes that exploratory count. HTML tokenization recovered unquoted script src attributes and 415 additional paths; 869 additional literal-loader/attribute records and 20 dynamic loader expressions are retained. The 94 unresolved webidl2 “imports” are JSDoc type references, not executable fetches.
- `core-driver-operation-occurrences-2026-09-16.jsonl`: **1,516** literal operation occurrences, marked direct Core source versus shared helper. Conditional Bluetooth BiDi definitions are retained but distinguished from the legacy Core caller path. No literal occurrence is labeled an observed execution.
- `core-reference-matrix-2026-09-16.jsonl`: all **1,015** Core reftest/print identities, **1,016** exact match/mismatch targets, source/reference hashes, source metadata and resource occurrences. Every target exists. Actual reference images, print results, fuzzy comparisons and runtime readiness remain unobserved.
- `CORE-OPERATION-SOURCE-MATRIX.md`: native route/regression mapping and explicit branch/scope boundaries.
- `scripts/wpt_source_matrix.py` and regression sources: original source/reference identity validation, rejection of substituted reference bytes or fabricated qualification, and exact Core reference denominator preservation. Not executed.

Six apparent helper defects need no invented replacement: their invoked symbols are inline or already provided by separately loaded real dispatcher/utils/aria-utils/disable_bfcache resources. Their missing requests remain real404s; source classification does not erase raw outcomes. The two genuinely misplaced required imports use the previously declared exact repair manifest. Import-map names, active-script/document-base imports, root-clamped META URLs, inline URLs, templates and intentional failure resources retain their native semantics.

The review identified pinned pageswap activation/event ordering and same-task form-submit/window.stop cancellation for the root lifecycle lane. Root has generated planned submission/location cancellation and `stop_cancels_planned_submissions_but_allows_later_submission`; pageswap work remains root-owned. Source generation continues for remaining native input/AX/lifecycle/reference and visibility/capture obligations; this saved audit is not a whole-G completion or conformance claim. Nothing was built, tested, served or qualified.

### Native option-click production gap closed in source

`option_automation.rs` supplies the WebDriver option algorithm using native select ownership, selectedness and trusted event objects. `PrepareClick` resolves the owning select for native scrolling/hit testing, then returns an explicit handled result so the HTTP caller does not issue a second generic pointer gesture. This supports collapsed options without inventing an option box. The native sequence preserves focus, input/selection/change ordering, multiple toggling and disabled-option behavior; every author callback is followed by document generation/epoch/ownership/connectivity checks. Listener dispatch omits duplicate HTML activation because this algorithm already owns selection. No page-exposed method or protocol version change was introduced.

`native_option_click_uses_select_container_and_rechecks_author_callback_detachment` covers collapsed selection, trusted container events, no accidental picker reopen, disabled options, repeated multiple selection toggles, and an input listener removing the target before mutation/late events. This closes the previously listed generic option-click source gap; interruption/error/focus and broader AX/reference qualification remain separate. No test was executed.

### Physical sample fidelity and loss accounting correction

Source review found retained latest samples incorrectly counted as lost on every replacement, even when a consumer had read them. The macOS callback now increments device loss only for actual AVFoundation dropped-sample callbacks. Parent `normalize_sample_loss` retains this cumulative value as `source_dropped` and computes `dropped` from the requesting consumer's delivered sequence gap, preserving separate clone/consumer cursors. Audio output requests the selected native active-format sample rate and channel count explicitly. Parent `validate_physical_sample` rejects physical sample kind/dimensions/rate/channel drift instead of presenting bytes that disagree with advertised track settings; native media error containment already handles that failure.

Regression sources cover fast versus slow consumers, independent device-loss evidence, empty/retired sequence bounds and actual/advertised format mismatch. No capture hardware or tests were used. Hardware-specific format/runtime behavior and the remaining reference/visibility matrix remain unqualified.


### Native glyph/image alpha and filter coverage generation

Permission visibility now sends a private creator-local paint request containing exact document/generation/epoch, current decoded image/Canvas/SVG bytes, fonts/layout and selected paint-order keys to the native provider. `drag_image_capture::capture_filtered` reuses fresh snapshot assembly, suppresses nonselected own paint while preserving native stacking ancestry and crops the target's polygon region. The native provider examines actual nonzero raster alpha; author scripts receive no pixels, including tainted resources. Whitespace and transparent image holes no longer become opaque layout rectangles. Native identity is rechecked after capture, and absent/failed resource evidence cannot certify visibility.

Render-core now owns CSS Filter display-list groups so regular screenshots, native visibility, drag and print fallback use the same effect pixels. Native bounded SVG filter primitives implement opacity, brightness, contrast, invert, grayscale, saturate, sepia, hue-rotate, blur and drop-shadow. Padded tile input preserves effect halos across renderer tile boundaries; nested filter pixel reservations are bounded and released with scope. Source regressions cover zero alpha, unchanged transparent holes through color filters, blur/drop-shadow actual ink, cross-tile halos, whitespace/glyph/image footprints and undeclared/unbounded effects. A driver regression exercises actual Canvas alpha and opacity-filter changes over an install control.

This batch does not claim unsupported rendering fidelity: URL filter rendering and mask/backdrop composition remain errors/open source work; the subsequent filter-plane batch below supersedes the earlier nonuniform/rotated/perspective restriction, not invisible paint. Missing current native frame inventory likewise cannot produce a visible result. Alpha observations are tied to the native renderer's device-pixel raster, not a mathematical subpixel visibility claim. No source or regression was built or executed.


### Filter coordinate planes and effect resource dependencies (source-only)

`PaintCommand::Filter` now carries the native 4×4 plane. `projected` maps group paint back into local coordinates, applies the bounded native filter there and projects the result through the actual plane. This replaces the previous uniform-axis-only gate. Source regressions `finite_rotated_nonuniform_and_perspective_filter_planes_use_local_pixels` and `singular_filter_plane_is_an_error_not_an_empty_visible_result` exercise finite transforms and fail-closed singular input. No execution or performance claim: full local group rasterization is repeated per output tile, and existing pixel/extent bounds still apply. Serialized display-list compatibility remains pending the common batch contract freeze.

`oryn-layout::effect_resources::discover`, called by `generated::finish`, now discovers filter/backdrop/mask URL dependencies from computed styles. CSS-tokenized quoted, escaped and nested URLs preserve resolved stylesheet URLs, strip only the network fragment and exclude same-document fragment references. These feed the existing creator-bound `css_images` → `PlatformHostRequest::ImageStart` → parent img-src authorization path. Regression sources cover external versus local fragments, computed stylesheet authority, nested escaped URLs and invalid partial input. This is dependency loading, not arbitrary SVG-filter rendering.

Confirmed pinned selected-Core effects scope: `content-security-policy/img-src/css-filter-blocked.tentative.html` requires the external filter.svg request to cause an img-src violation; `css/css-cascade/all-prop-revert-layer.html` checks computed style; `html/semantics/interactive-elements/the-dialog-element/top-layer-parent-mask.html` checks top-layer escape from ancestor mask; permission-control invalid CSS/ancestor mask sources require locked style or style-invalid behavior. No selected Core entries were found under css/filter-effects, css/css-masking or css/compositing. This does not waive required dependencies in other sources.

**CSP replacement source now generated:** `ContextEvent::CspViolation` carries the exact native Creator; authenticated realm report, HTTP/image and page-resource producers preserve it, while legacy root producers snapshot the parent's current root identity. `native_platform_query(CspEvents)` checks current authority and matches the full Creator, not URL. Native per-realm cursors deliver through `csp_events` typed internal payloads and trusted Document dispatch, rechecking authority between callbacks. `poll_context_events` pumps native tasks before its remaining compatibility hook. The superseded JS CSP class/dispatch branch is removed. Regression sources check same-URL sibling/root separation, retired owner rejection, cursor advancement, typed constructor/getter brands and trusted delivery after author constructor/dispatch replacement. This is unbuilt source evidence, not observed CSP WPT success. Broad URL-filter rendering and mask/backdrop alpha composition remain unsupported native visibility inputs (fail closed); do not mark G or runtime conformance complete.


### Mask, backdrop and captured URL-filter generation; pending render schema 3

- Native `Mask` display-list groups now compose captured gradient/image layers using alpha or luminance, repeated mask position/size/repeat/origin/clip lists and add/subtract/intersect/exclude operators. The group clips all descendant paint, while top-layer roots remain outside ancestor masks. The superseded blanket mask-image visibility rejection is removed. Unsupported mask-border and no-clip inputs remain explicit errors; SVG mask-element fragment semantics remain open, rather than being treated as ordinary image alpha.
- `BackdropFilter` copies previously painted pixels into a private native input, applies the same local-plane filter pipeline and clips the result to the rounded border box. Scenes containing backdrop operations use full-surface rendering, and print export captures the complete prior scene. This replaces tile-local backdrop input and isolated print fallback. Alpha interpolation uses premultiplied contributions. Nested group backdrop-root semantics and private permission-only capture's prior-paint input still need source closure; the latter remains fail closed.
- Captured URL filters now resolve exact SVG resource/alias and filter ID, derive the declared effect region, render original SVG definitions through the native captured-resource resolver, and preserve input alpha and local effect coordinates. Missing/ambiguous references, absent nested resources and extent limits are errors. Native ordinary image decoding already accepts dimensionless SVG's default size, so filter-only SVG resources can reach this path. Remaining fidelity review: object bounding boxes with overflow and chained filter intermediates, local fragment ownership and nested SVG dependency scheduling.
- `RENDER_CONTRACT_VERSION` is now **3**, separately from context45/worker14. Raster and vector print reject older display-list versions. The render facade reexports the new mask types. Historical reports/artifact hashes remain untouched; rebuilt candidate hashes can only be produced after the global generation barrier.
- New regression sources: mask alpha/luminance, mask composition and top-layer escape, backdrop source/clipping across former tile boundaries, schema2 rejection, captured URL-filter redirect alias/alpha/serialization and rejection of absent nested filter images. None ran.
- `wpt-procedures/core-effect-source-evidence-2026-09-16.jsonl` records exact pinned source hashes and assertion lines for the three direct computed-style/CSP/top-layer effect obligations; qualification stays null. It is a focused evidence index, not a substitute for the full 1,015-entry reference matrix.

Follow-on mask source closure: `SvgMask` resolves exact captured external SVG resource + mask ID, applies match-source/alpha/luminance through native SVG mask rendering, and retains typed raster/vector fallback. Local fragment IDs remain rejected without document-owned resource identity; unique textual matches across captured realms are insufficient authority. `mask-clip:no-clip` now uses finite native viewport projection while preserving the mask positioning box. Source regressions cover SVG mode override with an existing style attribute, foreign/local ID rejection and no-clip positioning. Supersedes the immediately preceding open bullets for external SVG mask fragments and finite no-clip; mask-border, unbounded projection and prior backdrop input remain open.


### Native local effect bindings and dependency lifecycle (source-only)

`PaintSnapshot.local_effects` now records fragment → captured SVG resource indices from the current native Document tree; the map is defaulted metadata in the still-unbuilt common snapshot batch. Hidden definitions are captured, first document ID wins, and unrelated equal-URL resources cannot supply a binding. Root/frame snapshots retain their own maps and resource arrays. Raster frame validation bounds map sizes, IDs and resource indices. Filter and SVG-mask lookup consume the map for fragment-only or same-document URLs; absent bindings fail closed. Root paint planning and private drag/permission capture populate the map; cached inventories require existing exact document/generation/epoch/mutation checks. Source regressions cover hidden definitions, non-SVG duplicate IDs, same-URL foreign bytes, absent maps and stale indices. Shadow-tree-specific reference semantics remain unqualified.

The existing CSS image lifecycle now maintains a bounded dependency graph for SVG image/feImage sources. Completion resolves dependencies against final redirected URL and XML base, queues them through authenticated ImageStart, and retains them only while reachable from layout/capture roots. Cycles terminate, release cancels orphan operations, and request/byte caps still apply. Inline SVG resources join this native readiness path; source capture includes feImage and rewrites image hrefs using native XML base resolution. Already captured resources are not fetched again by the native SVG paint planner. Private capture no longer drops CSS resources merely because they are not generated-content images. Source regressions cover redirected filter→image requests, root release, XML-base/xlink resolution, cycles and orphan pruning.

No build, test, qualifier or service was executed. These sources supersede the prior missing-local-binding and missing-nested-image-scheduling bullets, without claiming every SVG dependency kind or shadow-scope rule qualified. Remaining selected-scope audit work: prove any mask-border rendering reachability; finish nested backdrop input and chained/overflow filter bounds.


### Effects source checklist reconciliation and selected-scope boundary

- [x] Native local fragment identity: current-document map, hidden definitions, first document ID semantics, per-frame resource validation, raster/print/private-capture consumers and source regressions.
- [x] SVG dependency readiness: redirect/XML/document-base resolution, image/feImage capture, bounded cycle-safe ownership graph, orphan cancellation, existing-resource reuse and source regressions.
- [x] URL-filter chain/overflow source input: private native SourceGraphic/SourceAlpha aliases carry the full input bitmap; a separately retained native border/reference box fixes object-bounding-box coordinates across chains. Missing first/default secondary inputs are rebound to the native source. Primitive object-bounding-box coordinates are normalized explicitly. This removes the prior cropped-input and whole-surface-reference-box substitutions. Source regressions cover outside-reference input shifted into view and opacity-zero followed by generated flood output.
- [x] Pending render schema 3 consumers and rejection of older display lists; common snapshot's local-effect metadata defaults to an empty binding map rather than guessed identities. Historical artifacts remain unchanged.
- [x] Remaining mask-border/backdrop scope review: `core-mask-backdrop-reachability-2026-09-16.json` binds a read-only review of **12,651** selected Core, reference-target and saved helper source files. No mask-border literal occurs. The sole backdrop-filter literal is line 36 of `css/css-cascade/all-prop-revert-layer.html`, a computed-style test. A repository source search finds neither property in test-harness workflow/framework sources. No selected rendering obligation for mask-border or nested backdrop-root composition is established by this evidence. These remain broader capability limitations, not newly invented blockers for this frozen selection. Dynamic source construction is explicitly outside the literal scan's proof; any demonstrated required caller must reopen the appropriate source cell.
- [ ] Runtime qualification: no builds, tests, reference comparisons, visual observations or deployment were run. Source checks do not mark G, Core or the five families conformant.

The current source status table above supersedes old batch bullet lists. Native SVG href serialization now retains the document's resource identity while resolving nested image/feImage requests using the same native document base plus XML base, with a regression for their separation. Local shadow-tree fragment semantics, inherited external SVG filter definitions and generic nested backdrop-root composition remain implementation/qualification limits unless authoritative selected-source reachability establishes a required cell.

## Render contract provenance and selected executor review — source only

- [x] **G/Q rendering identity producers:** `RuntimeBuildInfo` publishes compiled `render_contract_version` separately from `local_effects_contract_version`. The latter is an additive snapshot-v4 identity owned by `oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION`; absence of a compiled renderer yields `None`, not a claimed renderer. `build-sandboxed-worker.sh` copies actual worker runtime metadata into the manifest before signing. Historical manifests are unchanged.
- [x] **Native consumers:** `WorkerManifest` requires both fields. Static worker admission compares them with parent compiled identities, initial worker handshake compares compiled metadata, and subsequent handshake compares the admitted runtime. This batch stays pending context45/worker14; no artifact was activated.
- [x] **G/Q provenance consumers:** `completion_provenance.validate_engine` now validates the signed worker manifest against separate authoritative render/source constants. Official `load_contract` requires its hash-bound worker manifest, validates both rendering contracts and publishes them in original `run_info`; `wpt_full_qualification.reconcile` rejects missing/stale metadata. These checks preserve independent raw outcomes and do not establish signature validity or pixel correctness by themselves; the existing independent signature/artifact/qualification gates remain required.
- [x] **Selected effects evidence:** the three source-bound effect obligations now participate in `wpt_source_matrix.CORPORA`. Validation checks pinned revision, exact file hashes, and exact assertion line/text. A line-number change cannot silently inherit the original review even when the overall file identity is valid.
- [x] **Regression source:** native metadata serialization/absence, old display-list/current local-extension combinations, missing/boolean extension versions, stale rehashed manifest rejection before original `run_info`, qualifier rejection of old rendering metadata, and mismatched assertion locations. Existing 1,015 reference/print identity checks continue to preserve the selected denominator.
- [ ] **Execution and acceptance:** none of these sources has been built or run. Render pixels, print pagination/comparison, action readiness/focus/cleanup and physical-device behavior still need the original plan's post-generation qualification. `product.py` routes all four selected modes to official Oryn executors, including `OrynPrintRefTestExecutor`; this metadata work does not replace their actual execution or independently reviewed outcomes.

The frozen plan requires selected Core graphics/print/AX and five-family behaviors, not every upstream graphics suite. The earlier 12,651-source literal scope evidence remains limited to that review; it does not prove arbitrary dynamically constructed mask/backdrop behavior absent. Outstanding source review must reopen concrete selected callers when found. Earlier generic missing-filter/local-fragment/dependency bullets are superseded by the later generated sections, while shadow-scope fragment ownership, inherited SVG filter definitions and exact physical fidelity remain unqualified limitations; do not label the whole G package complete from these bounded checks.

Official `reconcile-full-wpt-qualification.py` now calls the source-matrix validator against the selected checkout before reconciliation and preserves corpus/validator SHA-256 inputs, rechecking them before writing a new ledger. Source-review record counts stay explicitly unqualified; an inventory cannot manufacture a passing pixel or action result.

## G/F/Q requirement closure review — official custody and frontend witnesses

### Newly generated bounded source cells

- [x] **G-06.c / Q-05.f official supervisor custody:** `wpt_official_workflow.execute` now uses the shared `completion_execution.run_owned` process-group supervisor with the actual pinned checkout as its working directory. Preflight is persisted before launch; timeout, launch error, descendant survival/cleanup and raw exit remain separate. After completion it revalidates the generation declaration, candidate/run contract, prepared inputs and producer/supervisor sources. A zero exit with surviving descendants or changed inputs cannot set `custody_complete`.
- [x] **Original-run consumers:** `wpt_execution_evidence.verify` joins report → execution → original preflight → candidate/run contract/lock and actual producer sources. The Core ledger CLI, canary CLI and shared-placement CLI require this custody. Core exit 1 remains a valid *raw failing suite* outcome; it does not become PASS. The final completion input inventory also binds the Core preflight/execution files. `run-official-wpt.py` returns failure for incomplete custody even when the raw process returned zero.
- [x] **Q-05.f profile terminal boundary:** the profile runner now rechecks generation declaration bytes, candidate identity, complete working-source identity and executable bytes after every attempt and after the final attempt. Its new `execution-custody.json` binds all original attempt hashes. `qualify-workflow-q.py` independently requires the complete frozen profile population and this receipt; a saved summary cannot hide last-attempt drift.
- [x] **F-02.b frontend credit:** the full-candidate path no longer credits `MixedCellAdapter` family-wide `DIMENSIONS` declarations. `workflow_frontend_evidence` defines concrete witnesses: research v2 hydrated pagination covers client rendering, SSR/hydration and routing/chunks; forms v3 custom role covers custom controls; shopping v2 framed persisted checkout covers cross-origin frames and async persistence; booking v2 first fold covers client timezone conversion. Together they cover exactly the seven frozen dimensions. These are representative covering witnesses, not replacement cases or a smaller family denominator.
- [x] **F production and independent oracle consumers:** each witness binds current fixture/module/task/adapter source identities, original native plan/result, exact session and sealed oracle. The qualifier reconstructs the recipe's OIL task from current pinned sources, requires every native step to complete, rejects native/cleanup/identity errors and independently reruns the existing family outcome validator. Unknown recipes, changed task lines, truncated native results, wrong persisted outcomes and label-only claims cannot establish a dimension. Fault-modified attempts receive no normal-frontend witness credit. Historical diagnostic adapters remain separate.
- [x] **Regression sources:** `test_wpt_execution_evidence.py`, expanded `test_wpt_official_workflow.py`, `test_profile_custody.py`, `test_workflow_frontend_evidence.py` and updated existing canary/candidate-identity controls cover these boundaries without requiring native processes. Existing fixture/client tests continue to own their algorithms; these added tests cover joins and rejection behavior.

### Exact remaining generation and acceptance inventory

The above closes concrete missing supervisor/credit implementations, not every parent G/F/Q requirement. Remaining generation reconciliation is: full G-02 operation/lifecycle matrix against current root/S/V/P/W producers; every G-06 native fault/TLS/focus control beyond the five mounted canaries; final source lock refresh for all changed fixture/task/adapter/runtime inputs; and all 51 parent-requirement caller/regression coverage. The F witness change addresses the false-credit mechanism; it does not assert that every native dependency or all 29 family cells already work.

Real local/public endpoint provisioning, normal TLS/reference-focus evidence, 752 manual/visual/AT observations, old/new parity, full Core/family/framework executions, performance/cleanup measurements and the original booking timeout's confirmed cause are still post-generation evidence work. These were not executed. The original frozen sources and historical artifacts remain untouched; new pending source contracts require a newly frozen matching candidate. No main checklist boxes were edited by this lane.

## G/F/Q native prerequisites, terminal custody and current input freeze — source only

The following bounded cells supersede the corresponding missing-code bullets above. They do not mark parent G/F/Q, all 51 plan requirements, or any runtime result complete.

- [x] **G-06 native TLS/focus/fault producer:** `run-wpt-native-controls.py` and `wpt_native_controls.py` own real trusted, untrusted-issuer and wrong-host servers, independently verify TLS, exercise native and reference WebDriver sessions, observe denied/restored focus under a host lock, and retain original wire responses. Native controls cover unknown command, missing/duplicate completion and actual current-session worker termination/reaping. Timeout cannot count as certificate rejection. `create-wpt-native-controls.py` prepares explicit new certificates after the generation gate; it neither installs trust nor disables certificate validation, and refuses outputs inside source or pinned checkout.
- [x] **G-06 transitive native fault/cleanup callers:** the authenticated worker-crash route accepts no caller-selected PID/page. Existing native owned-child termination is consumed directly. The navigation diagnostics GET now has method admission. `CheckedSession.end` bypasses upstream exception suppression and only accepts already-deleted sessions with an exact successful native retirement receipt. Native Registry retains bounded pending/completed/failed receipts; unknown/pruned/pending/failed receipt cannot prove cleanup.
- [x] **G/Q protocol custody:** original plugin connect/teardown events append to a private, no-symlink file, with candidate and unique session identity. The supervisor and independent report verifier require the unchanged file, a complete connect→teardown population and no failed event. Upstream malformed response decoding, propagated failed teardown, symlink rejection and successful native retirement evidence have regression sources in `test_protocol_faults.py`; their actual execution is a required prerequisite, not a predeclared success.
- [x] **Q-06 independent prerequisite join:** `native-prerequisites` is a required original execution receipt and final component. The final validator binds candidate, producer/preflight/input artifacts, original wire-command observations and independently recomputed verdicts. Candidate artifacts explicitly include the control contract and exact reference browser/driver. No existing historical candidate is rewritten to acquire those claims.
- [x] **Q-05 final input freeze:** `workflow_input_lock.py` derives an exact current source lock while preserving frozen v1 behavioral population/sampling. `completion_source_snapshot.py` creates and validates a deterministic source archive with a hash-bound index, rejecting traversal, duplicate entries, outside-source symlinks and changed content. Gated `freeze-completion-inputs.py` requires already-built/signed artifacts, writes outside source, and freezes candidate/snapshot/current workflow input identities. This is generated executor source; no lock, candidate or artifact was generated this turn.
- [x] **G-02 operation source admission:** `wpt_operation_contract.py` fixes 19 required and 19 conditional Bluetooth operation names, rejects new/unreviewed names, and restricts the conditional classification to the two actual helper files. `wpt_source_matrix.py` checks the pinned literal occurrence and hashes. This covers 38 operation names; it does not substitute route names for algorithm correctness or prove arbitrary dynamic calls absent.
- [x] **Current worker contract consumers:** layout/app compatibility producers and validators, suite evidence emission, layout/app benchmarks, sandbox worker benchmark and execution replay gate now read the distinct authoritative worker constant through `native_protocol_versions.py`. The stable historical assertion ID remains unchanged. App compatibility's explicit `--allow-historical` mode retains its historical worker11 expectation; normal candidate validation uses current source. Worker, context and agent contracts remain distinct. Regression sources cover independent constants, invalid/ambiguous declarations and typed shell admission wiring.
- [x] **Regression source inventory:** `test_wpt_native_controls.py`, `test_completion_source_snapshot.py`, `test_workflow_input_lock.py`, `test_wpt_operation_contract.py`, `test_native_protocol_versions.py`, extended execution/candidate/generation-gate tests and native Registry/WebDriver regression modules. No source test was executed.

### Exact remaining source work and evidence separation

- [ ] A real **fixture topology probe producer** is still missing: `wpt_fixture_contract.validate` consumes supplied endpoint/peer/helper/TLS evidence, but it cannot itself prove all HTTP, HTTPS, HTTP/2, WS/WSS and local/public routes. Generate the producer and original-run provenance join; never manufacture positive probes from configured IPs or grant state.
- [ ] Complete cross-package all-parent caller/regression reconciliation remains root/P/W/S/V shared work; this lane's operation map and prerequisite implementation do not prove every parent obligation generated.
- [ ] Review actual physical-device fidelity and selected reference/visibility outcomes after the generation gate; source generation alone establishes no hardware, pixel, focus or timing result. New prerequisite code still needs build/test correction and real reference CA/DNS setup.
- [ ] All execution remains pending: freshly built/signed current artifacts and final freeze, all generated regression suites, real fixture provisioning/probes, 752 manual/visual/AT observations, Core and all 29 family cells with frozen sampling, framework/security/architecture/reliability receipts, performance/cleanup and independent qualification. Historical reports remain historical; no PASS or deployment is claimed.

### G-03/G-04 live topology producer — generated source closure

- [x] `run-wpt-topology-probe.py` now consumes the frozen candidate/fixture contract, exact pinned upstream checkout and unchanged helper sources, then probes every configured endpoint. `wpt_topology_probe.py` connects to the actual configured IP and records `getpeername`; HTTP/HTTPS require byte-exact pinned `common/square.png`, HTTP/2 requires verified TLS plus negotiated h2 and a completed successful stream, and WS/WSS require a valid RFC6455 upgrade, random masked text echo and completed normal close through upstream `/echo` (`websockets/handlers/echo_wsh.py`). Socket/read/frame/header/body/event limits fail closed.
- [x] Verified TLS uses the selected fixture CA and real hostname; observed leaf hashes are retained. Negative TLS verdicts join the separately verified original native-prerequisites receipt, whose independent issuer/hostname controls are rerun by the final validator. No TLS bypass or generated positive endpoint observations are used.
- [x] The fixed `fixture-topology` original-run command uses the hash-bound upstream Python and pinned dependency registration. Preflight binds helpers, fixture, candidate, generation declaration, probe sources and negative receipt. Post-run identity drift or missing routes keeps `topology_passed` false. Final completion independently checks transport details and fixture peer/IP-space contract, plus original execution custody. This adds the sixth required receipt and a `fixture_topology` component; historical candidates/reports remain unchanged.
- [x] In-memory regression sources cover masked client frames, truncated/oversize/malformed server frames, exact upgrade acceptance, mismatched echo/close, missing TLS leaf identity, substituted helper bytes and global generation-gate ordering. Final join fixture sources include the new mandatory component.
- [ ] Runtime/provisioning qualification remains unexecuted. Run native prerequisite temporary servers before starting the actual fixture servers (their port sets overlap); then provision the declared routes and run the topology receipt. The producer observes already-provisioned routes and never installs trust, changes hosts, assigns public IPs, or starts servers. Failure to provision a real local/public endpoint remains a failed prerequisite, not permission to change the denominator.

This supersedes the prior **missing topology producer** source cell. It does not prove the provisioning or runtime observations have occurred, nor close cross-package caller/algorithm review. No build, test, probe, generator or service ran during this source batch.

### Review-packet and transient custody source reconciliation

- [x] Manual review packets now preserve exact pinned source markup and helper text, including download URLs and dynamic-script instructions lost by text-only transcription. Packets bind candidate, corpus and selected platform/API; they contain no observations or qualification. Final assessment requires the unchanged packet among its evidence artifacts and independently reconstructs it. Platform branches remain mutually selected, not jointly required. Source matrix now validates all 752 procedure source/helper identities.
- [x] Full OIL checks generation-declaration bytes on every identity probe and checks candidate custody immediately after each attempt. A transient drift is retained even if subsequent probes see restored bytes; native outcomes remain in the failed attempt and later launches cancel while the complete denominator is persisted. Versioned input locks include the newly generated WPT runner/packet/freeze entrypoints.
- [x] Regression sources cover exact source links, wrong candidate/source/platform, missing AT assertions, actual manual final join packet binding, gate-before-side-effects and transient post-attempt drift.

The proposed root-owned checklist edits are separately recorded in `GFQ-PROPOSED-CHECKLIST-DELTA-2026-09-16.md`. They explicitly separate generated children, execution, and cross-package all-51 review. No main checklist changes or execution occurred.

### Related-root endpoint prerequisite (P-04 support)

Generated `RealmEndpointRegistry` and exact live realm/environment endpoint accessors replace singleton ContextState/FetchState reads in host context/network/module callbacks and remote/driver/message consumers. Native Creator and page endpoint now originate from the same current/target/incumbent realm as appropriate. Generation rotation, initial-frame reuse, matching retirement and parked cleanup retain exact custody. Regression sources exercise nested distinct endpoints and stale/retired/rollback rejection. See `RELATED-ROOT-AGENT-INTEGRATION-2026-09-16.md` for the source delta and still-pending shared-root actor/ownership implementation. No execution or whole-P/G completion is claimed.

### Shared-root implementation continuation: live BFCache/timer consumers

Generated per-realm parked-document capability storage with exact issuing endpoint/environment/generation/authority-epoch checks, root-specific restore/discard, discard-before-realm-removal and issuer-specific final cleanup. Generated document-scoped scheduler suspension/resumption and cancellation; native park/restore/discard paths consume it. Regression sources cover independent native parent capabilities for two roots, parked versus runnable sibling tasks, preserved active-time timer delays and cancellation without resurrection.

P-04 is still **not source complete**: production WorkerPage attachment remains independent; RustExecutableDocument still owns its agent and the current actor loop is per page. Synchronous popup allocation/adoption, unique movable/borrowed agent ownership, shared actor dispatch, non-timer/microtask suspension and process failure fanout remain implementation work. No process-only attachment was activated as a substitute for synchronous same-agent semantics. No execution occurred.


### Shared-root suspension source continuation (2026-09-16)

Generated source now captures the exact live frame-environment subtree when a
root authority enters the parent history vault. The scheduler freezes and
restores each captured document; unrelated roots retain runnable tasks. Native
input, selection, media-query, viewport, platform, control, stylesheet and
remote-message delivery, and pending-work readiness, skip parked realms.
`AsyncFetches.deferred` retains completed native responses without resolving
parked promises; its population is bounded by the existing 32-operation admission
limit. Duplicate retained completions fail closed. Environment retirement drops
retained response custody without adding an already-completed operation to the
retired-in-flight set. The two-root native endpoint regression now covers parked
channel draining, independent restore/delivery, and deferred retirement.

These changes are source-reviewed and unexecuted. They do **not** complete P-04:
the shared page actor, actual synchronous popup adoption, exclusive host transfer
between document drivers, root-local failure propagation and V8 microtask
suspension remain generation work. V8 150.4.0's `microtask.rs` explicitly requires
one queue for synchronously accessible contexts, so assigning arbitrary separate
per-root queues is not an acceptable shortcut. Existing independent root launch
must remain until its complete replacement is integrated.


### Shared-agent consumer reconciliation and G-02.e interruption custody

The previous shared-root “still independent” paragraphs are historical source
checkpoints. Production callback allocation, exact route reservation, lazy actor
adoption and exclusive host transfer are now integrated by the related-root lane;
see RELATED-ROOT-AGENT-INTEGRATION-2026-09-16.md. Runtime behavior remains untested.

Source review found and repaired driver operations that still enumerated every
agent root: keyboard cancellation, pointer-source reset, cross-frame pointer
release/drag termination, CSS capture pending/release, and drag-feedback drains
now consume only the active root's native frame tree. The native popup regression
creates a child hover, resets the parent's input sources, checks child state still
exists, then resets the child and checks it clears. This closes a concrete
interruption/capture custody defect without asserting whole G-02.e complete.

Retired worker routes now retain exact navigation cancellation and frame/local
resource cleanup capabilities. Missing/forged/wrong-page grants remain denied;
only stop/release operations bypass live authority. Reset requires terminal
navigation and drained resources. Parent post-registration controller refresh
avoids capturing a permanently absent controller before page publication.
Regression sources cover cancellation, superseded IDs, timeout retry and cleanup
route admission. All source changes and test sources remain unexecuted.


### G-02.e selected-document upload authorization

Generated `webdriver_upload_files_in_context` and the session caller now capture
the selected native document before opening files. Authorization uses that
frame's committed origin, with Creator lifetime checks before authorization,
before each read, and before returning bytes. The existing root-only wrapper
captures the current parent root authority rather than authorizing only a page
origin. Element reference resolution still checks the native selected lifetime
before SetFiles consumes the bytes. The new regression source distinguishes a
denied cross-origin frame from its allowed top root and rejects stale, foreign
and non-live selected authority. No test was executed.


### G-02.f pinned protocol-handler generation closure

Pinned `html/webappapis/system-state-and-capabilities/the-navigator-object/protocol.https.html`
asserts register/unregister presence and successful valid calls (lines 15–45),
SecurityError scheme precedence (64–71), and mixed-case safelisted/custom schemes
(166–216). The handler-query manual files require a user-selected registration.
Source inspection confirmed missing unregister, missing ftp/ftps/sftp, missing
case folding, and default registration throwing rather than presenting a choice.

Generated shared `protocol_scheme`, native unregister dispatch with exact
origin/template removal, native navigator binding and IDL conversion ordering.
Default-mode registration now returns a parent-issued protocol prompt, retaining
its exact native owner, scheme and handler URL. Existing trusted popup keyboard,
pointer and AX consumers display a protocol-specific title and accept/reject via
the parent; this kind never creates camera/location grants. Auto-accept/reject
remain native mode branches. Secure-context publication and admission apply to
both register and unregister. Unregister cancels only matching pending templates.

Regression sources cover default prompting, foreign-owner rejection, acceptance
without permission grants, exact-template removal, stale prompt rejection,
case folding/safelist, native method presence and conversion/error precedence,
and private DOM presentation ownership. No source test or manual run occurred.
Whole G-02.f remains open for the complete remaining consumer/lifecycle audit;
this is a bounded protocol-handler implementation delta, not qualification.

## Native HTTP stream cleanup custody follow-up

Confirmed resource dependency of selected image/fetch consumers: accepting
`HttpStarted` formerly consumed `ContextNetworkReply`, so later failed HttpAbort
admission could discard the last explicit cleanup path. Generated sources now
retain an Arc guard per exact native stream **before publishing** the start
completion. Header ACK retains that guard; matching terminal read/abort parent
ACK finishes it. Read failure/malformed response, rejected receipt, send failure,
retirement and explicit abandonment cancel it using the existing bounded cleanup
executor and original Creator/page/route. Late header ACK cannot resurrect a
removed guard. No extra emergency threads or new wire privileges were added.

`PlatformHost::abandon_http_stream` is a typed cleanup-only hook. Binding central
submission invokes it when HTTP read/abort fails budget, identity or native
admission. `AdmissionHost` forwards it and handles queued dispatch failure,
queued cancellation and internally generated abort failure. Context host targets
only the owned stream. The default revokes the endpoint: AgentPlatformHost uses
its existing retained full-endpoint retirement custody, with broader cancellation
rather than silently dropping ownership. This is fail-closed behavior, not a
claim that worker cleanup is isolated to a single stream.

Generated regressions: `accepted_http_stream_retains_cleanup_through_receipt_races`,
`exhausted_binding_admission_retains_native_stream_abandonment`, and
`queued_read_failure_and_abort_cancellation_signal_retained_cleanup`. Includes
foreign stream no-op, cancellation-before-delayed-header-ACK, terminal completion,
retirement, identity exhaustion, budget rejection and queued native failure.
All are source-only and unexecuted. Global source-generation gate and remaining
cross-package acceptance stay open.

### G-02.e final caller reconciliation: selected lifetime and interruption

The earlier open labels for option click and contenteditable placement were stale source inventory, not missing production implementations. Reviewed session/server → FrameCommand::Element → RawV8Host::webdriver_element_command consumers:

- [x] Upload: `Session::upload_element_files` captures the selected native snapshot, calls `webdriver_upload_files_in_context`, then submits SetFiles against the original opaque element reference. Parent authorization uses native selected origin before opening paths and validates Creator before each read/return. SetFiles revalidates selected lifetime/reference/type/count and preserves real bytes, timestamps, multiple-input append and trusted input/change. `webdriver_upload_binds_selected_frame_origin_and_rejects_stale_or_foreign_authority` and `native_element_file_selection_retains_real_bytes_and_dispatches_trusted_events` cover those bounded paths. Unix open now uses O_NONBLOCK/O_NOCTTY before fstat rejects non-regular handles; `webdriver_upload_rejects_fifo_without_waiting_for_a_writer` covers the concrete blocking-path defect. Parent Upload errors map to WebDriver invalid argument; closed context maps to no such window.
- [x] Option click already uses native select-container geometry and the WebDriver event/selectedness order through `option_automation::click`, revalidating document stream, connectivity, container and disabledness after author callbacks. Existing `native_option_click_uses_select_container_and_rechecks_author_callback_detachment` exercises actual driver route and input-handler detachment. No duplicate pointer implementation was added.
- [x] `native_automation_focus` preserves already-focused selection and collapses a newly focused contenteditable host after its final child. Added `native_contenteditable_focus_collapses_at_end_but_preserves_existing_selection` over the production native element command; ordinary text-control source coverage remains `element_send_keys_validates_before_focus_and_preserves_existing_selection`.
- [x] `cancel_actions` now disarms local defaults/pointer sources/held inputs even if the selected snapshot is already stale, without dispatching synthetic releases into a replacement lifetime. `PageState::execute_webdriver_actions` also handles early context-validation/cancellation failures before cancel_actions is reached. Ordinary action errors still retain successfully dispatched held inputs for Release Actions. `stale_cancel_retires_held_inputs_without_dispatching_into_a_new_lifetime` and `stale_action_cancel_at_page_route_drops_old_held_source_custody` cover both boundaries.
- [x] Cookie read/delete uses native committed policy URL and ancestry, never snapshot URL. Added missing snapshot.live rejection before any access; pre/post exact Creator validation remains. `webdriver_cookie_snapshot_requires_live_native_authority_and_ignores_authored_url` covers non-live/stale/foreign rejection without deletion, forged snapshot URL ignored, HttpOnly visibility and accepted deletion.
- [ ] All generated regressions and upstream driver qualification remain unexecuted under the global generation-first gate. This source review does not turn G-02.c's full helper/protocol or G-05's observation obligations into passes.

### G-02.f / G-02.g transitive operation and failure-custody follow-up

Reviewed the actual pinned chain for `geolocation/tojson.https.window.js`: it calls `test_driver.bidi.permissions.set_permission` and geolocation override/reset; resources/testdriver.js and vendor messaging feed upstream `asyncactions.py::{BidiPermissionsSetPermissionAction,BidiEmulationSetGeolocationOverrideAction}`, then product `protocol.py` HTTP adapters, session handle validation and native platform state. The upstream async action requires a populated contexts list and preserves omitted/error/null distinctions. No new direct-source count is used as completion evidence. `wpt_operation_contract` now names session/runtime/platform consumers for both operations, in addition to transport endpoints.

- [x] `webdriver_permission_origin` now rejects non-live snapshots before resolving exact native Creator authority. Existing native cookie-snapshot regression also checks this rejection and ignores forged snapshot URL when deriving the real permission origin.
- [x] Session geolocation requests now call `webdriver_set_geolocation_many` once. Native `State::locations` preflights coordinates/error, complete capacity and revision before changing any page; resets work at full capacity. `batch_capacity_and_revision_failures_preserve_every_existing_override` covers partial-mutation prevention.
- [x] Geolocation callback tasks retain registry custody through native response/object publication. `geolocation_publication_failure_retains_callback_until_exact_retry` injects pre-publication failure, confirms the task remains pending, then delivers exactly once on retry. Previous remove-before-allocation path is removed.
- [x] RPH and expanded AX route producer/consumer sources were reconciled against existing protocol/native implementation and regression evidence, rather than creating duplicate endpoints. Prior default protocol prompt, exact origin/template registration and same-document AX identity regressions remain relevant.
- [x] Generated retained native notification publication in `v8_bindings/native_platform/publication.rs` for CSP (Window and worker), storage, visibility and PermissionStatus. Native observation payloads, event identity, per-phase listener snapshots, callback roots and once-removal progress survive pre-/mid-publication errors; cursors advance after completed dispatch. Native parent liveness gates retries, creator changes cancel stale work, and retirement releases retained publication. Shared window revision no longer suppresses undelivered permission changes. Source-only regressions inject event-preparation, once-removal and after-callback failures and cover snapshot identity, capture/bubble stopPropagation, native PermissionStatus state/handler delivery and retirement. Standalone `deliver_native_csp_event` transfers source-task custody into the same dispatcher without requiring parent transport or acknowledging parent cursor ids. Admission is bounded to 1,024 queued/active events and 8 MiB native payload bytes; rejected standalone admission retains the original source task. Typed `WindowSuspended`/`WindowRetired` responses derive from native live-page and MessageHub active/retained authority; suspension retains, retirement cancels, and transport failures surface with custody intact. Property-handler getters revalidate document lifetime before invoking returned functions. Added regression sources for these cases and native parked-versus-retired authority. Coordinator integration review and all execution remain pending.
- [ ] G-02.g dynamic/transitive unresolved references and conditional-helper evidence remain open; added native route paths do not close them. No tests, builds or services were run.

### G-02.f embedded-origin caller closure — generated, unexecuted

Pinned `resources/testdriver.js` documents `origin` as the top-level permission origin and `embeddedOrigin` as the embedded origin (defaulting to the top-level origin). The HTTP session previously rejected distinct embedded origins with `unsupported operation`. That rejection is removed: the native context permission registry now keys grants by `(top-level origin, embedded origin, normalized descriptor)` and validates both serialized nonopaque origins before mutation. Omitted-origin classic commands derive both principals from the selected live native document, never its authored URL.

Parent PermissionStatus and geolocation queries, capture acquisition/revalidation, and permission-prompt read/write paths use the committed top-level/embedded pair. A grant in one embedding partition does not grant the same embedded origin under another top-level origin, the embedded origin as a top-level page, or the top-level page itself. Native permissions policy remains an independent denial check. Existing same-origin convenience methods retain their original public signature and route to a same-origin partition.

Generated regression sources cover registry/Geolocation isolation, atomic invalid-origin rejection, prompt-answer partition custody, exact native frame and forged-URL origin resolution, and the actual WebDriver distinct-embedded-origin route plus invalid embedded value. These supersede older notes that distinct embedded origins are unsupported. Full permission-consumer and platform/default conformance remains open.

Notification retry scheduling now publishes a 25 ms exponential retry deadline capped at one second for suspended/transient publication failures; the host readiness predicate does not treat delayed custody as an immediately runnable task. Success, new admission, replacement backend and retirement reset the deadline. The scheduler regression asserts nonzero bounded delay while preserving the suspended event. No code or test was executed.

PermissionStatus lifetime follow-up: the native listener activity graph now roots subscribed PermissionStatus objects while `change` listeners/handlers exist, then releases that root after unsubscribe/once/abort/retirement. Collected status wrappers remove their native permission/object registry records instead of exhausting the 1,024-status budget with dead objects. The shared native `handler_set` now inserts/removes an ordered event-handler listener entry and retains its registration position on reassignment; previously native PermissionStatus/media/service-worker setters populated only the handler value map. Regression source covers subscribe→GC→native change→unsubscribe→GC and registry release. No GC test was run.

Current exact route scope: expanded AX operation occurrences map to `wai-aria/accessibility_properties_basic.tentative.html`; existing source regressions cover its checked state and tab child/id/parent/role/label assertions. The general WebDriver AX projection still builds its SemanticGraph from the raw DOM arena, so CSS-resolved visibility/generated-content parity remains an explicit integration obligation. RPH registration-mode and minimize routes match saved caller operations; complete pageswap/default navigation lifecycle remains in the coordinator's lifecycle group. Literal operation coverage does not prove dynamic conditional behavior or runtime conformance.

### Native layout-backed AX property snapshot — source follow-up

`AccessibilityProperties` now obtains the current native layout before borrowing the live DOM. `layout_accessibility_graph` applies computed visibility, generated before/after content, native control state, and geometry visibility in the same sequence used by the production semantic projection. Computed role/name uses that same graph through `compute_accessibility_with_layout`; the raw-arena-only producer has been removed from the production provider. `BindingLayout` now retains generated-content provenance. Only actual DOM-backed graph nodes receive stable, scoped WebDriver AX identities.

The existing checked-state/identity regression now installs the production layout provider. `accessibility_properties_share_fresh_layout_visibility_and_generated_names` covers CSS-hidden exclusion, pseudo-generated accessible labels, role/name parity, and stylesheet mutation changing both children and names. All evidence remains source-only; no builds/tests were run and expanded AX qualification remains open.

## G-02.g dynamic helper and navigation terminal reconciliation — 2026-09-17

Source generation only; no executable, test, generator or WPT run was performed.

All 20 `dynamic_loader_parameter` occurrences (IDs 1179–1198) in the existing
1,199-record helper inventory now identify their exact upstream dependencies,
source hashes and native consumers. The seven source contexts are Bluetooth's
conditional helper loader, CSP redirected exception reporting, CORS 304 replay,
CORP script admission, range script helpers, stale-while-revalidate scripts and
anonymous-iframe CacheStorage dispatcher code. The last context produces code
for the dispatcher, not dynamically selected script URLs. The source-matrix
validator checks these classifications and dependency hashes without changing
any original occurrence identity, source classification, qualification or Core
manifest denominator. New regression sources reject missing/duplicate rows,
misclassified dispatcher code and missing native consumers.

A concrete dependency remains open: background stale-while-revalidate scheduling
for `fetch/stale-while-revalidate/stale-script.html`. Native script 2xx (including
206) and conditional 304 replay exist. The resource lane owns opaque ranged
service-worker replay denial in the private Response backing group. These are
required caller dependencies, not optional helper gates.

Queued remote navigation now carries a parent-minted execution receipt, distinct
from both queue admission and document generation. The parent claims/releases
one exact queue ticket. The receiver reserves publication capacity and claims
before running the operation; lost claim ACK cannot cause an author callback
replay. `NativeNavigationSource`, pending Navigation completion and
`LocationNavigation` retain the execution observer until loader transfer. Root
transactions retain `NavigationReply::Execution` through parser/load completion.
Frame requests retain it through resource preparation, the original document
stream and child/parent retained load publication. Stream replacement cancels
that exact frame operation. A loader transfer removes the observer from the
source promise record so discarded native preparations cannot remain hidden
behind intentionally unresolved source promises.

The receiver publishes terminal outcomes outside the actor, with a process-wide
limit of 256 reserved publishers. It retains the exact token/outcome on lost ACK;
retry does not repeat navigation or callbacks. Typed claim rejection discards
never-admitted custody; typed parent-retirement acknowledgement releases the
publisher while preserving the parent's recorded retirement failure. Unknown or
conflicting tokens do not become successful evidence. Same-document/no-op,
committed, canceled and failed outcomes remain distinct. Source regressions
cover one-shot observer transfer/abandonment, root parser readiness, frame parent
load callback retry, replacement streams, lost ACK and rejected claims.

The qualification validator now requires `stage=Execution`, validates baseline
and new outcome schemas, rejects admission-only acknowledgement, validates
integer generation/result values, and checks typed terminal outcomes against
legacy result fields. Existing active/pruned/truncated/missing/rewrite guards
remain. Raw upstream status is not rewritten. New source tests cover forged
admission success, mismatched generations and malformed baseline outcomes.
Combined execution and signed qualification remain pending.


### G-02.g.4 independent resource helper review (source only)

The pinned `fetch/range/resources/utils.js` dependency is more than `loadScript`.
`general.window.js` also uses `appendAudio` to observe `progress` and checks the
server received `Accept-Encoding: identity`. `sw.https.window.js` uses that same
URL-media producer for native `Range: bytes=0-`, continuation at the actual
received byte offset, service-worker fetch interception, stored responses,
image preloads and equal ResourceTiming visibility on 206/416 outcomes.

- Generated cache evidence: `stale_memory_and_persistent_revalidation_retains_only_initiating_document`
  covers foreground stale bytes, owned background refresh, unrelated-document isolation
  and updated bytes; `partial_content_script_status_remains_loadable` preserves 206 script
  acceptance. The pinned stale-script helper has no validator: background NoCache must
  also fetch and replace a non-validator entry, not require a 304.
- Generated replay evidence: `opaque_range_replay_checks_internal_provenance_before_consuming_body`
  checks hidden 206/range provenance and denial before taking the response body.
- Concrete Request correction: untouched native Request copies preserve privileged Range;
  nonempty recognized init dictionaries reapply the no-cors guard. Successful safelisted
  header mutation removes Range; ignored mutation or deletion of an absent header retains it.
  `native_range_request_clone_and_successful_header_mutations_follow_guards` mirrors these
  helper operations on an actual native FetchEvent request.
- **Still in generation:** URL-media selection/loading/continuation and retained cancellation.
  Existing MediaStream `srcObject` playback does not satisfy URL `src` audio loading.
  Image preload completion and resource timing caller coverage must also be reconciled before
  closing this row. SWR/opaque primitives alone are not combined helper completion evidence.

No builds or tests executed. Full transitive inventory and signed execution qualification stay open.


#### G-02.g.4 URL media caller source delta

Generated the URL resource-selection path in `v8_bindings/media_resource.rs`,
with DOM `src`/ordered child `source` candidates, preload/load admission, native
currentSrc/networkState/error, byte-offset range continuation, bounded response
memory/continuation count, queued trusted progress/metadata/error tasks and
source/lifetime cancellation. PCM WAV metadata is parsed from checked RIFF chunks
without allocating the declared sample data; metadata preloading releases its
stream through native abort custody. Unsupported media is not reported decoded.
This does not establish general codec decoding or URL-backed audio playback.

`MediaStart` passes the exact native document endpoint through existing host
admission to parent `MediaStreamStart`, media-src, Media cookie destination,
redirect/CORS/COEP policies and service interception with audio/video destination.
The HTTP transport adds Range's default identity encoding after interception;
worker-visible Request headers remain free of Accept-Encoding. The native Request
copy/mutation fix and both opaque-denied/readable-206-accepted replay regressions
cover the range helper's independent response semantics.

Source regressions now include:

- `media_source_range_continues_at_received_bytes_and_late_completion_is_cancelled`
- `media_stop_cancels_pending_header_admission_without_retiring_document`
- `wav_metadata_checks_chunks_without_allocating_declared_pcm_body`
- `media_stream_uses_media_policy_cookie_destination_and_private_opaque_metadata`
- `range_identity_encoding_is_inserted_only_at_transport_boundary`
- `readable_partial_service_response_is_accepted_without_range_request`

**Residual exact helper requirements:** link rel=preload as=image must initiate the
image fetch, publish its load/error task and create native ResourceTiming entries
with equivalent opaque success/failure visibility. Those callers were not found
in the source review and are not covered by the media or SWR tests. Full G-02.g.4
and signed runtime qualification remain open. All new regression sources are unrun.


#### G-02.g.4 preload/resource timeline follow-up

The previously absent image preload producer and native resource query interfaces
are now generated in `preload_resources.rs` and `resource_timing.rs`. Native
Performance replaces the bootstrap-only now/toJSON object and retains the existing
monotonic document clock. PlatformHost start/read/abort admission supplies actual
Fetch/XHR/image/CSS-image/media/preload entries; immutable snapshots support sorted
getEntries, type/name filtering, clear and bounded resource buffers. Opaque and
failed resources preserve an entry while filtering body-size/status details.
No expected-count table or fixture URL dispatch is used.

Image link preloads select href/as/rel/media/crossOrigin/referrer policy, fetch through
the existing image destination/cache path, validate decoded images, and queue real
trusted load/error tasks. Source changes, removal, document.stop/open and retirement
retain native cancellation; callbacks revalidate later recipients. Regression sources
exercise actual preload admission, denied-response event/timing publication, native
query snapshots/filter/clear and equivalent opaque status/body visibility.

Remaining source review is explicit: parser/host script/style/font/frame loads outside
PlatformHost still need timeline integration; native detailed transport phases and
TAO state are not fully transported (unavailable fields remain zero), and shared HTTP
cache integration does not establish the separate HTML preload-response reuse algorithm.
PerformanceObserver and broader timing families are not implied by this resource query
implementation. These limitations keep G-02.g.4 and the broader G gate open. No source
regression, build or conformance test has been executed.

### Image preload reuse and terminal transport source update

The separate parent HTML preload cache is now generated: single-use responses are keyed by authenticated Document Creator, URL, CORS/credentials, isolation policy, integrity and Range, with lifetime/count/byte bounds. Consumption rechecks current CSP along retained redirect hops and passes through common response policy/taint admission. Cancellation, short body, extra body and disconnection cannot publish a reusable entry; matching Content-Length alone does not release stream custody. Source regressions cover exact key separation, expiry, delayed terminal receipt, zero length, late extra bytes and cache publication across terminal/failure/cancel outcomes. Transport framing parsing rejects malformed/conflicting lengths and distinguishes compressed decoded size and HEAD/304 representation size.

This supersedes the earlier statement that no separate preload cache exists. Integration qualification, external resource timing producers and detailed timing/TAO provenance remain open; these generated regressions have not been run.

### External timing producer source integration

External parent script/module, stylesheet/font and frame requests now produce bounded records in their exact SharedContextClient document scope. Streaming frame entries await the stream's terminal response. Standalone broker script/module, CSS import/font and frame loaders use the existing document-owned native report queue for timing custody. Host publication verifies the complete Creator or standalone document/authority epoch, skips parked/retired realms, and replacement binding metadata resets the old timeline. A sibling/ancestor isolation source regression confirms timing entries do not propagate through network-activity ancestry. Detailed transport fields for these external producers remain to be propagated; they currently expose only measured start/end, not inferred byte counts from transformed source.

Parent/agent PlatformHost responses now carry private TAO metadata measured before header filtering. The native resource timeline consumes permitted status, encoded size, response-header offset and content type even for opaque Responses, while denied details remain zero. HTTP redirect admission accumulates TAO; single-use preload reuse retains that accumulated decision. Service responses with unavailable redirect timing provenance remain conservatively hidden. DNS/connect/TLS phases and full service/cache timing provenance still require native transport metadata, and are not claimed generated by this update. All regression sources remain unrun.

### Transport/service/external timing continuation

The previously listed missing DNS/connect/TLS, redirected service timing provenance and external detailed metadata paths are now generated. See `RESOURCE-TIMING-SOURCE-AUDIT-2026-09-17.md` for native producers, consumers, retained authority and regression sources. This supersedes those earlier source-gap statements, while keeping execution and integrated qualification open.
