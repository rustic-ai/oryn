# Historical input isolation — source generation

No builds, tests, formatters, generators or conformance runs were executed. This is source coverage, not qualification.

## Implemented

- Removed seven privileged native input globals from normal Window binding installation. Native keyboard/pointer APIs remain available to the authenticated Rust host.
- Added an explicit native historical authority setter. Credentials are bound to the native binding instance, document generation, document stream epoch and renewal serial. Revocation blocks retained functions; renewal does not reactivate old functions. Worker globals and wrong/retired realms reject grants. Author properties do not confer authority.
- Added the `historical-test-input` Cargo feature, disabled by default, propagated through `oryn`, `oryn-native` and `oryn-js`. Even feature-enabled ordinary bindings require the explicit setter. Historical RustExecutable bootstrap opts in before author execution in each bootstrap realm.
- Extracted legacy keyboard/send_keys bootstrap to `historical_keyboard.js`. Production runtime does not execute it or `native_actions.js`. The latter remains an explicitly historical adapter.
- Preserved 35 existing RustExecutable and three binding regression cases under the historical feature; binding tests explicitly grant authority. Existing native keyboard/pointer/default regressions remain in production configuration. Added production no-hooks/bootstrap isolation, native trusted keyboard cancellation, and revoked/renewed/stale-epoch credential regression sources.
- Added required `historical_test_input` to native runtime attestation and CLI runtime-info output. Worker startup rejects a parent/worker feature mismatch; reset rejects any change from the captured profile. Missing wire attribution fails decoding. The WPT runner requires explicit boolean attribution, rejects privileged binaries for the upstream profile, and requires a separate privileged binary for legacy reproduction. Added profile-attribution negative regression sources.

## Eventual validation configuration

After the whole-plan source-generation gate opens, run normal production tests with the feature disabled, and repeat historical tests with `historical-test-input` explicitly enabled for both parent and worker artifacts. Production candidate qualification must use the feature-disabled artifact and upstream profile. Historical reproduction must use a separately attributed feature-enabled artifact and legacy profile. Do not overwrite historical evidence or combine the results as one candidate qualification.

## Precise limits

- The historical adapter intentionally grants broad page-script input privileges inside that dedicated historical artifact. It is not a production authority model. Native generation/epoch expiry still applies.
- Document-stream replacement expires historical credentials; retained old functions cannot carry privilege into the replacement stream. Any historical harness requiring renewed input after document.open needs an explicit native lifecycle renewal rather than a page flag.
- Native production behavior coverage exists in native_keyboard/native_pointer and authenticated host action tests. The entire preserved legacy matrix still requires comparison after execution; source migration is not evidence that the two implementations produce identical outcomes.
- Runtime-info reports the actual worker profile for production execution; worker startup and reset require matching historical feature authority. Existing signed binary/hash attestation remains necessary alongside this profile field.

## Authenticated production regression expansion

`production_input_regressions.rs` exercises the real `RustExecutableDocument` typed native input host methods, without calling historical JavaScript input helpers. Added independent regression sources cover:

- radio form-owner/tree-order stepping, disabled/inert/hidden exclusion, canceled key and click defaults, native focus-listener mutation revalidation;
- Tab order and keyup retargeting, reverse navigation, cancellation;
- UTF-16 native editing focus inside nested closed shadows, canceled beforeinput, maxlength/default-value provenance and post-handler geometry changes;
- trusted keyboard construction despite author constructor/helper replacement;
- pointer ancestor boundary order, common-ancestor click targeting, canceled-pointer compatibility suppression and reset;
- native label activation/inert exclusion, wheel cancellation/remainder/containment, and keyboard/pointer focus-visible outline transitions.

Existing authenticated host regressions separately cover cross-frame pointer/keyboard/wheel routing, drag cancellation/cross-frame drop, transformed coordinates, and retired/sibling lifetime rejection. Native binding modules cover selection drag moves, files, beforeinput cancellation, compound undo and immutable drag data. Full preservation includes the historical tests as well; these sources are not a claim of a passing comparison. Added scheduler mixed-source inverse release order/idempotence, host file byte fidelity, beforeinput drop cancellation/selection revalidation, Escape drag cancellation, rich sanitized HTML drop/undo, and effective editing-host/designMode Enter oracles. The historical release adapter suppressed clicks; the authenticated WebDriver release dispatches the inverse pointerUp action and retains its click default. These are deliberately separate oracles, pending pinned source confirmation and eventual execution.

Further production sources preserve native selection-task timing/change-on-blur/reset semantics, the full pointer/mouse interface payload matrix with replaced author constructors, and live RTL radio direction changes. The historical Promise-return timing test remains historical: the direct Rust host method is synchronous; authenticated actor/request completion timing is a separate transport contract, not an author Promise shim.
