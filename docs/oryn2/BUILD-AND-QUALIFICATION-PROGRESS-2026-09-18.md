# Complete-plan build and qualification progress

## Latest checkpoint — September 19: native31 compile result; repair46 in progress

Repair45 is integrated, independently reviewed and frozen: **51 requirements, 961 bound paths and 19,463 whole-source inputs**. Its declaration is `d07a624e5e2d4830e1e0b6e3b9fc565461d263edb7840cdf0f18a26ba9c8af66`. Formatting and WebIDL freshness passed. Native31 is **terminal, exit 101 after 56.49 seconds**; all source hashes matched before and after, and owned process cleanup completed.

The previous Stylo `ToTyped` compiler errors no longer occur. Compilation stopped at **three distinct JavaScript compiler sites**: storage-event weak-handle conversion, clipboard scope/context lifetimes, and an encrypted-media regression using a contextless scope. Cargo's two library and three library-test errors overlap; they are not five distinct defects. **No native runtime assertions executed.** Repair46 assigns all three corrections and needed regression coverage in parallel, followed by complete integration and independent review before the next frozen run.

Latest Python evidence remains **727 passed, zero failures/errors, 12 skipped out of 739** in Python12. Both previous Chromium failures passed; the signed-runtime prerequisites for the skips remain open. The earlier native29 results remain the latest runtime observations: 5,166 observed passes, 174 failures, seven ignored and one unfinished IPC case, with incomplete aggregates. Source repairs do not establish recovery of those outcomes.

Full Core conformance in every required profile, all five families/eight frameworks, required workspace checks, signed reproducible release, clean-source acceptance and the requested code commit remain outstanding. The requested `cargo clean` and exact restoration of its pinned validation dependencies are complete.

Evidence: [native31 result](../../artifacts/completion-validation-2026-09-18/native-js-tests-31.json), [native31 compiler log](../../artifacts/completion-validation-2026-09-18/native-js-tests-31.log), [source custody](../../artifacts/completion-validation-2026-09-18/repair45-native31-whole-custody.json), [WebIDL freshness](../../artifacts/completion-validation-2026-09-18/webidl-check-31.json), [Python12 result](../../artifacts/completion-validation-2026-09-18/fq-python-regressions-12/test-results.json). Earlier checkpoints below are historical.

## Latest checkpoint — September 19: repair44 executed; repair45 compile correction

Repair44 was frozen after complete source and independent review: **51 requirements, 957 bound paths, 19,460 whole-source inputs and 103 Python regression sources**. Its declaration is `ffa81614dfecf6cd2bfedfe6aff8e7b88d449f3037f7b4bd21458bc4d57be09d`. Both validation processes are terminal, with source hashes unchanged before and after and owned process cleanup complete.

**Native30:** WebIDL freshness passed. The clean build exited 101 after 53.96 seconds on two Stylo `ToTyped` compile errors for the constrained integer used by generated `orphans`/`widows` properties. No native runtime assertions executed. Repair45 is preparing the complete wrapper correction and specified/computed Typed OM regressions; it is not yet integrated or tested. The earlier 174 native29 failures, including 41 materially unresolved source dispositions and the separate unfinished IPC case, remain unverified by a newer runtime run.

**Python12:** 739 tests collected and executed: **727 passed, zero failures, zero errors, 12 skipped**. Both previous Chromium failures now pass. The strict full-regression gate exited 1 because signed native CLI/worker prerequisites remain missing; skips are open obligations. Environment identities also matched before and after.

The requested `cargo clean` completed. All 2,026 pinned V8 ABI inputs and the ICU data required for validation were restored with their exact original hashes. Final Core profiles, five families/eight frameworks, required workspace gates, signed reproducible release, clean-source acceptance and requested commit remain outstanding.

Evidence: [native30 terminal inventory](../../artifacts/completion-validation-2026-09-18/repair44-native30-terminal-inventory-01.json), [Python12 terminal inventory](../../artifacts/completion-validation-2026-09-18/repair44-python12-terminal-inventory-01.json), [actual freeze readback](../../artifacts/completion-validation-2026-09-18/repair44-actual-freeze-root-readback-01.json), [post-clean dependency restoration](../../artifacts/completion-validation-2026-09-18/repair44-post-clean-abi-restoration-02.json). Earlier checkpoints below are historical.

## Latest checkpoint — repair44 authored source complete; derivative checks passed

All authored repair and regression sources are integrated through receipt57: **49 successful reviewed patch applications across 135 source paths**. Full historical source readback found no unexplained changes. The complete135-path integration is distinct from the later generated and formatted output.

The independently accepted failure ledger preserves all174 original named failures: **133 corrective source mappings,16 partial and25 diagnostic-only**. There are no missing mappings;41 cases remain materially unresolved. The unfinished IPC case and original143-case comparison (111 passed/32 failed in native29) remain separate. Source mapping does not change an observed failed result into a pass.

Canonical WebIDL generation and freshness check passed, followed by `cargo +1.98.0 fmt --all` and its check. The postformat readback covers19,460 source inputs: one canonical generated file and88 formatted files changed; no unexpected non-Rust changes occurred. Independent derivative/format review and final source/evidence binding are underway. **No repair44 candidate declaration or native/Python/workspace test result exists yet.** Freeze only after these reviews close, then execute the complete required validation batch.

Latest runtime evidence remains native29:5,166 observed passes,174 failures,7 ignored and one unfinished case, with incomplete aggregates; Python11:715 passed,2 failed,12 skipped. Full Core profiles, all five families/eight frameworks, required workspace gates, signed reproducible release and requested commit remain outstanding.

Evidence: [authored-source admission](../../artifacts/completion-validation-2026-09-18/repair44-authored-source-complete-01.json), [coverage ledger03](../../artifacts/completion-validation-2026-09-18/repair44-native29-failure-coverage-reconciliation-03.md), [prederivative readback](../../artifacts/completion-validation-2026-09-18/repair44-prederivative-source-readback-01.json), [postformat manifest](../../artifacts/completion-validation-2026-09-18/repair44-postformat-source-snapshot-01/manifest.json). Earlier checkpoints below are historical.

## Latest integrated checkpoint — repair44 receipt56

Repair44 has **48 successful reviewed patch applications across 130 source paths**. The coordinator read back every latest source hash against its integration receipt; all match. Since receipt50, integrated changes cover retained-history fixtures, bounded startup diagnostics, the foreign custom-element regression, navigation/authority corrections, retained DOM identity and the three remaining history diagnostics.

The last outstanding source proposal covers raw Window failures22/23/34/36. Independent review is checking that its COOP/session reference correction preserves synchronous logical-close behavior for still-related session references. This proposal is not integrated. Final per-test coverage, requirement/source bindings and evidence reconciliation remain in progress. Corrective, partial and diagnostic-only source dispositions remain distinct; diagnostics do not mean a failure is repaired.

**No repair44 declaration, build or test result exists.** Finish the complete source batch and review, canonical WebIDL generation and formatting, then freeze and execute the full candidate. Latest executed evidence remains native29:5,166 observed passes,174 named failures,7 ignored and one unfinished case, with incomplete aggregate results; Python11:715 passed,2 failed,12 skipped. Final Core profiles, five families/eight frameworks, required workspace gates, signed release and requested commit remain outstanding.

Evidence: [current source readback](../../artifacts/completion-validation-2026-09-18/repair44-current-state-03.json), [integration receipts](../../artifacts/completion-validation-2026-09-18/repair44-integration-receipts/), [retained DOM review](../../artifacts/completion-validation-2026-09-18/repair44-retained-dom-independent-review-01.json), [history diagnostic review](../../artifacts/completion-validation-2026-09-18/repair44-history-gap-diagnostics-independent-review-03.json). Earlier snapshots below are historical; source-reviewed changes remain execution-unverified.

## Latest integrated checkpoint — repair44 receipt50

Repair44 has **42 successful reviewed patch applications across 118 source paths**. The coordinator read back every latest source hash against its integration receipt; all match. The latest integration adds the supported-hash integrity parsing correction and nine navigation/worker/policy fixture or diagnostic changes. Six of these ten cases have cause-backed corrections; four remain unresolved with diagnostics only.

Additional complete proposals cover13 navigation cases and18 retained-history cases; independent review is underway. Native wrapper/lifetime, foreign custom-element constructor, popup lifecycle and corrected startup diagnostics remain in generation/review. Earlier coverage counts and allocations below are historical; final per-test reconciliation is pending these deliveries. Diagnostic coverage does not mean a failure is repaired.

**No repair44 declaration, build or test result exists.** Complete all remaining source integration and review, canonical WebIDL generation and formatting, then freeze and execute the full candidate. Latest executed evidence remains native29:5,166 observed passes,174 named failures,7 ignored and one unfinished case, with incomplete aggregate results; Python11:715 passed,2 failed,12 skipped. Final Core profiles, five families/eight frameworks, required workspace gates, signed release and requested commit remain outstanding.

Evidence: [current source readback](../../artifacts/completion-validation-2026-09-18/repair44-current-state-02.json), [receipt50](../../artifacts/completion-validation-2026-09-18/repair44-integration-receipts/50.json), [independent residual10 review](../../artifacts/completion-validation-2026-09-18/repair44-navigation-residual-independent-review-02.json). Source-reviewed changes remain execution-unverified.

## Repair44 coverage audit — source generation remains open

The final per-test audit found **55 native failures without an accepted cause-specific repair/diagnostic mapping**, plus startup failure142 with a diagnostic correction still under review. These are assigned across four parallel lanes:17 navigation/data/form/history,18 retained-history/cache/close,10 named-target/policy/message, and10 raw Window/wrapper/lifetime cases. Existing integrated fixes must be checked first; a shared file or group label is not proof of coverage.

Current audit accounting for174 named failures is110 accepted corrective proposals,8 accepted diagnostic-only proposals, and56 lacking accepted coverage. The unfinished IPC case is separately repaired in source. These are source dispositions, not passing results. Repair44 remains unfrozen; no repair44 tests/builds have run. See [exact residual allocation](../../artifacts/completion-validation-2026-09-18/repair44-residual-native-allocation-01.json).

## Current checkpoint — repair43 executed; repair44 integration

Native29 under repair43 is terminal (exit 101). Compilation and canonical WebIDL freshness passed. Across observed test lines there were **5,166 passes, 174 failures and 7 ignored tests**; these are observations, not a complete aggregate. JavaScript aborted with a borrow panic; the native library was terminated after process sampling and source analysis established an IPC inventory deadlock, leaving one test unfinished. Of the original 143 native27 failures, **111 now pass and 32 still fail**. Completed suites include DOM 446/446, layout 514 passed/2 failed, render 166 passed/11 failed, and window targets 7 passed/1 failed.

Python11 remains **715 passed, 2 failed, 12 skipped out of 729**. Browser diagnostics proved a macOS Keychain wait; the reviewed Darwin-only disposable-profile launcher correction is integrated but not yet tested. Twelve skips still require current signed native executables.

Both admitted required Clippy profiles failed. Separate diagnostic collection found five unique compile-error sites and 126 warning issues, plus 15 native compiler warnings; downstream lint coverage is incomplete because compilation stopped. Diagnostic lint caps are not acceptance evidence.

Repair44 is open for source integration. Through integration receipt41, 33 accepted patches are integrated across 112 source paths with exact before/after receipts. Loading, corrected service activation/opaque authority, geometry with ancestor occlusion, semantic/action and all primary lint repair proposals are integrated. Print-capture followup, remaining diagnostics and runner cleanup revisions are in final generation/review. Combined canonical WebIDL generation, final source reconciliation, formatting, freeze and all repair44 execution remain pending. No repair44 tests or builds have run.

Final Core conformance in all required profiles, all five families/eight frameworks, required workspace checks, signed reproducible clean-source release and requested commit remain outstanding. Preserve all original failures and qualification denominators.

Evidence: [native29 terminal inventory](../../artifacts/completion-validation-2026-09-18/repair43-native29-terminal-inventory.md), [Clippy inventory](../../artifacts/completion-validation-2026-09-18/repair43-clippy-terminal-diagnostic-inventory-01.md), [integration receipts](../../artifacts/completion-validation-2026-09-18/repair44-integration-receipts/), [preserved repair43 baseline](../../artifacts/completion-validation-2026-09-18/repair44-preintegration-checkpoint-01/manifest.json). Earlier checkpoints below are historical and do not describe current execution.

## Current checkpoint — repair42 terminal; repair43 preparation

Repair42 was frozen with all51 requirements,908 bound source paths and19,449 checked source files. Whole-workspace formatting passed; all684 formatting changes received independent review. Native28 and Python11 ran outside the sandbox with unchanged source/review hashes before and after.

- **Native28:** terminal exit101 after144.945s; canonical WebIDL freshness passed, compilation failed with one E0603 private-import reference in a new activation regression. No runtime tests ran; all143 native27 failures remain unobserved under this candidate.
- **Python11:**729 tests,715 passed,2 failed,12 skipped; no collection, execution, cleanup or environment drift errors. The two Chromium Page.navigate timeouts remain. New server evidence shows accepted connections reached EOF without request-line bytes; no fixture handler was entered. Twelve skips still require current signed native executables.
- **Repair43:** the canonical enum-path correction is integrated with unchanged assertions. Browser timeout diagnosis and next-candidate preparation continue. No repair43 execution result exists.
- **Delivery:** full Core in all required profiles, five families/eight frameworks, required workspace/clippy checks, signed clean-source release and requested commit remain outstanding. Source review and formatting are not runtime qualification.

Evidence: [native28 terminal inventory](../../artifacts/completion-validation-2026-09-18/repair42-native28-terminal-inventory.md), [Python11 execution](../../artifacts/completion-validation-2026-09-18/fq-python-regressions-11/execution.json), [whole-source custody](../../artifacts/completion-validation-2026-09-18/repair42-native28-whole-custody.json). Earlier checkpoints below are historical.

## Latest verified checkpoint — repair41 / native27 / Python10

Native27 compiled and passed canonical WebIDL freshness, then ended with exit101 after437.560s. Five targets failed. Completed targets totaled1,355 passes/20 failures; JavaScript and native libraries aborted with SIGABRT and have no complete aggregate totals. Python10 executed718 tests across102 sources:704 passed,2 failed,12 skipped, no errors/collection errors. Full formatting check failed. [Exact terminal inventory](../../artifacts/completion-validation-2026-09-18/repair41-test27-inventory.md) preserves all143 named failures and original evidence.

Repair42 implementation, regression and diagnostic code is generated and integrated. All eight initial functional deliveries and the loading, remote Window, worker, secure-exposure, popup, activation and JavaScript realm/XML follow-ups have source acceptance. Final review and source reconciliation, whole-workspace formatting and candidate admission remain before tests. Several original failures have diagnostics only; they are not claimed repaired. Native28/Python11 are prepared but have not run. The unchanged browser diagnostic reproduced navigation timeout despite exact-file HTTP success and an established Chrome-to-fixture connection; its cause remains unresolved. Full Core, five-family/eight-framework acceptance, clean-source signed release and requested commit remain outstanding. The [granular checklist](CORE-AND-OIL-GENERATION-CHECKLIST-2026-09-15.md) separates reviewed source from verified execution. Earlier snapshots below are historical.

## Current status — delivery reassessment, September 18

Repair38/test24 compiled and terminated with exit101 after721.109s:342 named
failures, JavaScript SIGBUS and native fixture deadlocks. Completed suites and exact
failure accounting are in the [current delivery plan](DELIVERY-RECOVERY-PLAN-2026-09-18.md).
Repair39 has no frozen declaration or test result yet. Eight new agents are assigned
to final repairs, independent review, source reconciliation and release/qualification
prerequisites. Prepared test25 now covers all eight affected workspace test packages.

The sections below retain chronological evidence. Their “running,” “not built” and
“latest candidate” descriptions apply only to their recorded point in time.
Full Core and all five-family/eight-framework acceptance remain pending.

The global source-generation declaration was validated against all 51 parent requirements and all recorded review-source hashes before execution. All eight source lanes delivered. Source closure is not qualification completion.

## First execution results

- Native command: `cargo build --locked -p oryn-native --features v8-host,in-process-probe`. The first attempt hit sandbox DNS restrictions; the network-enabled retry identified required lockfile additions. The original lockfile is preserved, and minimal dependency reconciliation retained existing versions.
- Actual native compilation then failed with **13 compiler errors**, covering the SVG font derive and DOM integration (parser export, dependency, struct initializers, attribute and borrow/type errors). Repairs for all 13 reported compiler errors are integrated with separate file ownership; the next build has not yet verified them.
- Qualification-script batch: **320 tests run, 13 failures, 47 errors, 1 additional collection error** (missing `oauthlib`). These counts include subtest results; do not derive a passing count by simple subtraction. The generation lease remained valid throughout that batch. Missing `oauthlib` accounts for 37 error records and the collection error; one additional error concerns the pinned WPT tools import path. The remaining 9 error records and 13 failure records are being diagnosed and repaired. macOS temporary-path canonicalization in positive test fixtures is one confirmed cause; production path-identity checks remain intact.
- Full Core, family/framework, production signing and all final qualification results remain pending. No passing conformance result is claimed.

Raw logs, preserved lockfile and machine-readable execution status: `artifacts/completion-validation-2026-09-18/`. Failed runs are retained. The original declaration remains historical source-closure evidence; source edits require a reviewed refreshed declaration before the next admitted run.

## Subsequent execution and repairs

- Build05 compiled the repaired DOM and SVG code, then exposed two font tag API calls; both were corrected to the pinned API.
- Build06 exposed missing public V8 header closure and a missing static media-feature atom. Added 15 exact pinned headers with manifest hashes and preserved patched headers; vendored stylo_atoms 0.20.0 with the required atom source entry.
- Build07 advanced to V8 bridge API signature corrections, required public field documentation in reqwest, and libvpx toolchain linking configuration. Repairs are under review/integration; no successful complete native build yet.
- Python batch03: 327 tests, 2 failures and 2 errors, no skips or collection errors.
- Python batch04: **326 passed, 1 error out of 327**, no assertion failures, skips or collection errors. The remaining SaaS readiness timeout stack identifies Python reverse DNS in HTTPServer.server_bind. A dedicated numeric-loopback binding fix and regression source are integrated but not yet rerun.
- Clean pinned V8 source and all 21 submodules were prepared in target/rusty-v8-src-150.4.0-schema5 from local Git objects. Existing archives do not carry the complete current eight-patch/schema-5 provenance. A new archive is required before final native execution and production qualification.

All failed outputs are retained under artifacts/completion-validation-2026-09-18. Initial and successive generation declarations remain distinct. Full Core and family/framework qualification remain pending.

## Python regression closure and continuing native builds

Python batch05 passed **328/328 tests**, with zero failures, errors, skips or collection errors. It includes the real SaaS HTTP flow and new numeric-loopback bind regression. The repair04 source declaration remained valid before and after execution. This closes that regression batch only; it is not native Core or family/framework qualification.

Native builds08/09 progressed past media, header and bridge failures and exposed Rust syntax and Stylo generator/type integration errors. Reviewed repairs are in declaration repair06; build10 and a read-only formatting check are running. Engine archive retry uses all eight applied patches and a hash-verified local pinned toolchain after the initial network failure. Final artifact, native tests and full qualification remain pending.

## Latest candidate: repair09 / native build13

All 856 Rust source files passed read-only syntax checking (rust-syntax-check-02.json). This does not establish formatting, type-checking or behavioral correctness. Build12 reduced JavaScript binding diagnostics from 217 to 23 and reached layout compilation, which reported 47 errors. Seven repair lanes delivered the scoped fixes; the primary integrated callback/object lifetime and document-body ownership repairs. Reviewed repair09 binds 51 parents and the 17 repaired source files. Build13 is running against this declaration; no successful native build is claimed.

The separate patched V8 archive retry remains live under its immutable repair08 engine-input manifest. The corrected microtask queue object compiled successfully; archive completion and engine regressions, bindings, snapshot, worker and final qualification are pending. Python batch05 remains 328/328 passing under its recorded repair04 candidate. Full 16,737-entry Core, five-family/eight-framework and all profile/qualification gates remain unverified.

## Archive success and rendering compiler repair

The patched V8 archive completed successfully (exit 0, 5m14s), retaining 151,018,736 bytes with SHA-256 `7e4604e068025681bd0788e1720a948576363f581ac55c7d46d11e7b14de00ef`. The generated bindings and immutable engine-input provenance are in `artifacts/v8-schema5-2026-09-18/engine-archive-only-repair08.json`. This is archive-only evidence, not full runtime-profile or qualification evidence.

Build13 ended with one layout import error; repair10 corrected the pinned Stylo reexport. Build14 progressed through layout into render-core and reported 17 diagnostics: 16 SVG NodeId map-key errors and one missing filter-source borrow. Those repairs are integrated. Review also identified the native patch-chain fingerprint omitting the last two of eight patches; its correction is required before the next candidate and final snapshot/profile capture. Full native build and tests remain pending.

## Candidate repair12

Build15 reduced renderer diagnostics to one unresolved vector element type. Explicit `Vec<String>` preserves the stylesheet text collection before serde conversion. Repair12 is admitted and native build16 is running. The separate cross-contract fingerprint regression passed, confirming all eight native patch names match the schema5 profile contract. Engine regression build/execution has been assigned against immutable engine inputs; no passing engine-test result is yet claimed.

## Native runtime reached; build16 repair batch

Build16 passed rendering and reached the native runtime, which reported 253 diagnostics. Seven native repair lanes were assigned by file ownership while engine work continued independently. Shared repairs include production dependencies/module access, callback conversions, missing imports, existing policy-accessor use, and preserving CSP report side effects. Parser completion records require an ownership correction: DOM/preparation leases must remain on the isolate thread, while only transferable results cross the I/O boundary. This repair and host integration are still in progress; no new native build is admitted yet.

The first cctest executable build failed before any tests ran. The repairs correct pinned V8 fixture APIs/includes and use an explicit pointer/size for an upstream zero-capacity span. Assertions, production engine patches and GN settings remain unchanged. A separately reviewed inventory binds the test-only changes; retry of all18 cases and10 explicit shared-Wasm execution variants is authorized. The original failed log and successful archive remain preserved.

## Engine regression pass and native candidate repair13

Engine cctest built successfully; all28 invocations passed: all18 cases individually plus10 explicit Liftoff/no-liftoff shared-Wasm variants. Input hashes verified unchanged before/after. The production archive hash remains unchanged; only separately tracked test-source compatibility repairs were needed. DrumBrake was not enabled or tested. Results are retained in `target/v8-cctest-repair14/results.json`; prior failed attempts remain available.

All native build16 repair lanes delivered and the primary reviewed the integrated48-file repair scope. Repair13 is admitted; native build17 is running. Six installer regression tests also passed under repair13. Full native build/tests, final snapshot/worker/signing, pinned Core and all family/framework/profile qualifications remain pending.

## Build17 result

Build17 reduced native diagnostics from253 to13. The remaining batch consists of three missing RuntimeError references, one ambiguous dynamic-import handle conversion, two executable borrow/move errors, three window-open scope borrows, and four frame completion thread-ownership diagnostics. The import and handle conversions are fixed; independent lanes own the remaining custody/borrow repairs. No native passing result is yet claimed.

## Candidate repair14 / build18

All13 remaining build17 diagnostics have reviewed source repairs across five files. Frame per-request channels transmit context-free data; archived controls stay with the isolate preparation until publication or retirement. Real-thread custody/retirement regression sources were added. Window.open retains the original entered context across author conversion while preserving post-conversion authority checks. The two executable borrow/move fixes, explicit import handle types and missing RuntimeError import are integrated. Repair14 is admitted and build18 is running; no successful native build is claimed yet.

## Rust compiler errors cleared; patched engine integration

Build18 reached final linking with no remaining Rust compiler errors. Linking failed on10 C++ symbols, including the patched context lifecycle, shared memory and deferred HTML constructor APIs. Build19 explicitly selects the retained hash-verified eight-patch V8 archive and bindings, with matching sandbox/core-full features and ICU fingerprint. It remains a debug integration build; final release artifacts and execution qualification are still required.

The independent canonical eight-framework corpus build is also running under repair14 with frozen lockfiles and source validation; no framework pass is yet claimed.

## Build19 ABI diagnosis and framework preflight

Selecting the new patched archive resolved all five context/HTML-constructor entry points. Build19 still failed linking five shared-pointer signatures: the bridge uses platform libc++ namespace `std::__1`, while the retained engine exports bundled Chromium libc++ `std::__Cr`. Matching pinned libc++ headers, ABI configuration and compiler flags are required; simply renaming symbols is insufficient because return-value ABI layout also matters. A dedicated repair lane owns the bridge build configuration and canonical profile-builder integration.

The framework build stopped during mandatory immutable input-lock validation before any of eight apps built. The current controlled-regression script hash differs from its historical lock. Source provenance and a candidate-specific lock are being reviewed; historical lock/evidence are preserved and validation is not bypassed. Native tests and full qualification remain pending.

## Reviewed framework lock delta

The fixture-lock audit found exactly three script hash changes, each solely the identical mandatory generation-admission header. Removing that header reproduces each historical hash. All eight app sources/dependencies, OAuth tasks and other locked populations are unchanged. Revision3 preserves revision2 byte-for-byte, links the historical chain, and validates only the reviewed header change. New regression sources reject unknown rehashes, history mutation and population/dependency changes; execution awaits the next full declaration. Native build fingerprinting now exposes the bridge ABI manifest digest; matching compiler/header contract generation remains in progress.

## Candidate repair15 / build20

The complete17-file ABI, framework-lock and parser publication repair batch is admitted. All12 focused Python regression tests passed: three bridge-generator, six runtime-profile and three framework-lock tests. The bridge manifest binds2,026 compiler/header inputs and the retained archive/bindings, using the exact GN C++ configuration. Native build20 is running with that manifest; the independent canonical framework build02 is also running.

Parser publication retries now decrement pending counts only on the first network dequeue, retain completion/source custody across transient failures, and remain visible to scheduler liveness. Three new native regression sources cover repeated failure accounting, custody and stop cleanup; they remain unexecuted until native test compilation succeeds.

## Native build and framework corpus pass

Native build20 passed (exit0), including all selected native binaries linked to the retained patched sandbox V8 archive and matched bridge ABI. All eight framework corpus builds also passed with frozen dependencies, immutable fixture checks and valid generation lease. Their served output hashes are recorded in `framework-build-02/build.json`. This is build evidence, not the24 native framework qualification attempts.

The complete native and JavaScript cargo test targets are now running under repair15 with no test-name filters and `--no-fail-fast`. Local fixture networking was approved. Final release snapshot/worker/signing, full Core, five-family and all qualification gates remain pending.

## Native/JS test compilation and repair16

The first full native/JS test attempt stopped at compilation:97 JavaScript test diagnostics,81 native test diagnostics and one integration-test diagnostic. No assertions ran. Eight lanes updated test fixtures/callers against the current APIs while preserving scenarios and assertions. Async encoding tests now check resolved Promise results; popup integration uses actual window.open and public page discovery; cleanup tests observe actual retry behavior. The primary integrated shared helpers and PNG/UUID test dependencies; the minimal lockfile delta is preserved and reviewed.

Repair16 is admitted and the complete test02 command is running with locked dependencies, patched sandbox V8 and no test-name filters. Last successful production build and framework corpus results remain repair15 evidence; current test results and full conformance remain pending.

## Native/JavaScript test attempt 02 — runtime failures

The full selected native and JavaScript test command compiled successfully, then exited 101 after 154.0 seconds. The JavaScript library process trapped with V8 address-pool initialization failures; the native library process aborted with a stack overflow. Both integration targets ran and failed all seven tests with context-service shutdown errors. Four test targets failed overall. Library execution was interrupted, so no complete suite pass/fail total is claimed. Original logs and terminal result are retained in `artifacts/completion-validation-2026-09-18/native-js-tests-02.*`; the observed failure inventory is adjacent. Eight independent source investigations are active before the next candidate is frozen. Core and family qualification remain pending.

## Repair17 frozen; native/JavaScript test attempt 03 started

All eight investigation lanes completed. Integrated allocator initialization serialization, deferred direct-page startup, exact staged isolated-worker admission including lazy/reused routes, heap ownership for actor/prerender values, inherited CSP reporting endpoints, and necessary fixture lifetime/secure-context setup. Added regressions preserve concurrent initialization, admission/retirement, ownership and report bytes. The full 51-parent declaration is `GENERATION-COMPLETE-REPAIR-17-2026-09-18.json` (SHA-256 `4c4bf2981fdfe134a6e073a9026e87c7c10fef0f6b7250e393a90b861117b74c`). Test03 runs all native/JS targets with immediate panic output and backtraces, against the unchanged patched archive/ABI. No new passing result is claimed until terminal evidence.

## Test03 terminal result and next complete repair batch

Test03 exited 101 after 132.37 seconds against repair17; the generation lease passed its terminal check. All selected targets compiled, but four test targets failed. The prior address-pool initialization fatal and native stack-overflow fatal were not observed in this attempt; this absence is not proof that their dedicated regressions passed. JavaScript execution instead hit a V8 central-stack assertion in a large pointer-dispatch fixture; native execution aborted when an unhandled test callback panic attempted to unwind across V8. Many earlier failures were captured immediately.

The compiled generated binding registry has 168 concrete interfaces while the current authored IDL has 294. Missing Navigation and Performance cause widespread setup failures. Canonical full regeneration exposed unsupported typedef and static-member syntax; repair includes the generator and complete registry plus build-time drift detection, rather than individual missing-interface stubs. Separate source repairs cover retained pointer-state stack usage, DOM synchronous mutation/import custody, initial LoadHtml policy staging before replacement, CSP inherited meta identity, clipboard formatting/base handling, drag transforms, sandbox typed modules and exact fixture prerequisites. Original failing output remains unchanged; independent panic inventory includes expected injected panics as well as failures and must not be treated as a failure total.

Repair17 remains immutable historical evidence. No next declaration or further test result is claimed until all new changes and caller/protocol integration are reviewed and frozen. Core 16,737-entry, five-family/eight-framework and all qualification requirements remain open.

## Repair18 reviewed and test04 started

Integrated the complete reviewed test03 repair batch, including full314-interface registry generation/freshness guard, native DOM handoff/error reclamation, exact LoadHtml receipt and worker18/context51 cutover, static-method contracts, pointer stack custody and independent semantic/fixture fixes. Independent review corrected mutation-journal accounting and added deterministic final-handoff failure coverage. Repair18 SHA-256 is `6aa9a4fd5c46f4b33a6a693dee6d9dc054be3685c0983eb52c92d3f6c1d40330`; all51 parents remain bound. Test04 now includes all native, JavaScript, DOM and WebIDL package tests, preceded by the canonical generated-registry check. No new passing runtime result claimed before terminal evidence.

## Test04 compile repair and test05

Canonical registry freshness passed under repair18. Test04 exited101 after22.66seconds on two XML test calls to a nonexistent runtime text accessor, before assertions. Both now use the existing arena accessor with unchanged expected text. Repair19 (`fd1356bbe52277ba4776fe211e17b1e6b89f8374f79ea5e1dccde2e8e888dd20`) preserves repair18 and binds this two-call change. Full four-package test05 is running; no passing suite result yet.

## Test05 compile repair and test06

Test05 exited101 after22.92seconds with two JS compilation errors. The static observer getter now reads the existing V8 context slot; the boxed pointer receipt retry predicate uses autodereference through a closure. No semantics/assertions removed. Repair20 SHA-256 `eea60052db977712b54aa36428d5c4a5f45ad964a47ef492beb95881be8a4ae2` is frozen; full test06 is running.

## Test06 terminal evidence and repair work

Test06 compiled all selected packages and exited101 after231.71seconds. WebIDL completed16/16 tests; DOM completed413passed/19failed, no ignored cases. JavaScript aborted from an overlapping DOM borrow panic across a V8 callback; native aborted from a1MiB ordinary parser fixture exhausting its V8 heap during the complete binding bootstrap. Both integration targets ran and failed all8tests on intrinsic iterator setup. Five targets failed overall; library aborts prevent a complete global total. The original log, terminal result and per-panic inventory are retained.

The next source batch addresses real missing parser stack-close notifications, retained initialization/borrow ordering, strict native XML character validation and intrinsic setup, alongside explicit fixture ownership/loading-state/API corrections. Normal parser fixture heaps use the existing64MiB convention; production limits and actual payload-limit assertions stay unchanged. No test06 result is rewritten and no next candidate is claimed qualified.

## Repair21 frozen; test07 started

Twenty reviewed source/test files bind the test06 repair batch, including independent parser callback and stack review. Repair21 SHA-256 `b86d2b130fa3c323d46cee5d0ed78709a01d3dbf765fa5403dc5cbf451a53653`. The canonical registry check precedes all native/JavaScript/DOM/WebIDL/html5ever tests, now including the actual vendor driver regression. Original expected outcomes and previous logs remain; no new pass claim before terminal evidence.

## Vendor driver admission and test08

Test07 stopped before compilation: Cargo rejects direct tests of excluded html5ever with dev dependencies. Repair22 adds a DOM integration wrapper importing the unchanged vendor driver source, retaining exact workspace parser dependencies and every driver assertion. Repair22 SHA-256 `53966589b173516de13a94bca1eddafd586b1b506b4d202bedfe5bf16b4e8eb1`. Test08 runs the four full packages including that driver harness; prior result retained.

## Test08 terminal result and parallel repairs

Test08 exited 101 after 198.89 seconds under repair22. All four packages compiled. DOM completed 434 passed and two failed (436 total, none ignored); the actual vendored driver passed both tests; WebIDL passed all 16 tests. Native style integration completed one passed and one failed. JavaScript aborted with SIGABRT, native library execution trapped with SIGTRAP, and window-target integration aborted with SIGABRT. Five targets failed overall; aborted processes prevent a complete aggregate total. Original logs and terminal result remain in `artifacts/completion-validation-2026-09-18/native-js-tests-08.*`.

Eight parallel lanes are repairing and reviewing parser attachment, DOM custody, stream cancellation/cleanup, frame layout/registration, XML completion, binding dispatch, nested document.write stack use and weak-handle teardown. Delivered source changes are unverified. Repair23 is not yet frozen; further execution follows complete source integration and review. No Core or family qualification gate is closed by these results.

### Current repair review and dependency provenance

Source review confirmed the small Document.write dispatch preserves generated receiver checks, conversion order, document ownership and shared publication handling; its nested runtime regressions remain unexecuted. Popup review confirmed native accessor precedence and original-root opener lookup, and identified an adjacent suspended-frame lookup gap now covered by source changes and a same-isolate regression. Stream review separated immediate rejection cleanup from accepted-operation cleanup. Custom-element termination review found nested event-listener TryCatch scopes also require explicit termination preservation; those changes remain in progress.

The weak-callback crash requires a Rust v8 binding lifecycle patch, including pending second-pass ownership through isolate and snapshot teardown. The cached upstream v8 150.4.0 crate was checked against its Cargo.lock checksum before planning reproducible vendoring. The repair must bind the complete selected Rust crate source and patch in runtime provenance as well as retain the separately bound C++ archive. No source patch, new schema, or review note is a passing runtime result. The next candidate remains unfrozen while these source and provenance changes are integrated.

## Repair23 frozen; test09 started

All 51 parent source records and review hashes passed admission for repair23 (`9e0b40490327843ed1fb5b1328b3dec52aa10de8315088dfeb05e2a6384ee7bc`). The reviewed batch binds 227 source/input paths, including every selected vendored Rust V8 file. Canonical WebIDL generation and minimal Cargo dependency integration are complete. Independent review found no source blocker in the expanded teardown regressions; execution is still required.

Full four-package test09 is running, including six exact V8 teardown integration cases and the vendored parser driver. The private vendor custody unit requires a separate vendor library invocation and is not counted as covered by the workspace wrapper. All candidate inputs remain frozen during validation. No new passing suite or qualification result is claimed before terminal evidence.

## Test09 terminal link failure

Test09 exited 101 after 143.68 seconds under repair23, with the terminal generation lease intact. Canonical WebIDL freshness passed. Rust compilation completed for the other targets, but the new V8 teardown integration executable failed linking: the vendored static archive requires macOS Security and CoreFoundation, while those link settings were only supplied by the Oryn embedder crate. The standalone regression target exposed that missing dependency metadata. No assertions ran in this attempt.

The original test09 log/result are retained. Correct target-specific framework linkage belongs in the vendored dependency's reviewed build patch, covering standalone and embedded consumers. Independent vendor custody and Python regression commands are still running against frozen repair23; source repair waits for their terminal evidence.

## Repair24 validation

Python batch06 completed 348 tests: 347 passed and one fixture error; private V8 custody attempt01 stopped at an ambiguous borrow in its new assertion. Repair24 corrects that assertion, provides the static archive's macOS framework link metadata, and updates the input-lock fixture to include all three mandatory framework lock files while preserving drift rejection for all seven inputs. Its immutable declaration is `029fd8d6d8595b72591ca1a059f7ea245290a8e1e7b5a40e5ece0291f2a8831a`.

Private custody attempt02 has now passed its one selected unit test (12 unrelated units filtered), with the generation lease unchanged. This tests pending callback ownership, not complete V8 or browser qualification. Full four-package test10 and Python batch07 are running under the same frozen candidate.

## Test10 terminal results

Test10 exited 101 after 158.28 seconds, preserving the repair24 generation lease. DOM passed all 437 tests; parser driver passed both; WebIDL passed all 16; all six V8 teardown integration cases passed. Both native style integrations passed. Window-target integration completed five passes and two failures. JavaScript and native libraries aborted from Rust RefCell borrow panics, so no complete aggregate library total is claimed. Three targets failed overall. Original logs, terminal results and an observed panic inventory are retained; injected expected panics are not counted as failures from that inventory.

Independent Python batch07 passed all 348 tests across 45 sources with no failures/errors/skips/collection errors; private V8 custody attempt02 passed its selected test. These candidate-specific results do not close full Core or five-family qualification. The next repair batch addresses remaining callback borrow lifetimes, native layout failures and independent semantic/fixture failures exposed by the more complete run.

## Repair25 frozen; test11 started

The complete test10 repair batch is integrated and bound in repair25 (`6298f0e61670a98b545d468a2c378957d6ca3c8f7f6ffe074f40b602d914f16d`). It covers callback borrow lifetimes, native bootstrap/polling, frame policy/identity, float width and overscroll, FormData/token strings, stylesheet publication retries, editing ownership and precise lifecycle fixture corrections. The remaining ambiguous ARIA/termination cases retain their expected behavior with better diagnostics.

Test11 now runs all native, JavaScript, DOM, WebIDL **and layout** package tests; layout is included explicitly to execute its new regressions rather than merely compile it as a dependency. The candidate is frozen. Existing repair24 passes remain historical evidence, not proof of passing repair25.

## Test11 compile repair; test12 started

Test11 exited 101 after 7.32 seconds before assertions: one new DOM regression supplied `Option<&str>` where the existing namespace setter requires `Option<String>`. The literal now converts to the required owned string; its namespaced-attribute negative assertion is unchanged. Repair26 (`cf6648100df60641636e851eda5a28137a3452245247a42e7f2973101b66bcf2`) records this single source correction. Test12 reruns the same complete five-package scope.

## Test12 layout-test repair; test13 started

Test12 exited 101 after 33.66 seconds before assertions. Newly admitted layout test compilation found five fixtures missing the required element-scroll map and one invalid SVG float literal. All six calls were corrected without changing assertions; a source sweep covered the remaining full layout input literals. Repair27 (`7df955099f82f85eca7614e7a156975387b1b109a171a1e1c49963167a2d47f9`) is frozen, and test13 repeats the full five-package suite.

Reference preparation independently found no official Chrome-for-Testing driver for installed Chrome153.0.8010.48. An isolated matching153.0.8010.52 browser/driver pair is being staged from official metadata; it must be explicitly identified in future reference evidence, with no claim to have qualified the installed .48 browser.


## Attempt 13 terminal; repair 28 integration

Attempt 13 under repair 27 exited 101 after 31.587702249998983 seconds during compilation; no runtime assertion totals were produced. The source freeze is released for the diagnosed bootstrap constructor, Cell qualification and private-field fixture fixes. Original log/result/declaration are retained. Latest runtime passes remain candidate repair 24; complete qualification remains pending.


## Repair 28 frozen; attempt 14 started

Reviewed five native source files for bootstrap compiler fixes and complete failed-bootstrap retirement. Auxiliary constructors capture the poll function before readiness; main installation failure retires native authority and rejects subsequent execution, preserving unrelated parked contexts. Regression uses Rust-side DOM evidence for no replay. Declaration SHA-256 `7792b49cf2eb387910c706a26b2d23a23043ec633adacf1988a6ee6187cab3e3` validates all51 parent requirements and review hashes. Full native/JS/DOM/WebIDL/layout attempt14 launched in session47517; source inputs frozen. No passing claim until terminal evidence.


## Attempt 14 terminal — runtime repair continues

Repair28 compiled; attempt14 exited101 after221.93728987499344 seconds. DOM438/438, parser driver2/2, V8 teardown6/6, native style2/2, WebIDL16/16 passed. Layout371passed/108failed of479; window targets5passed/3failed of8. JavaScript library SIGABRT and native library SIGTRAP prevent aggregate totals; four failed targets. All original logs and panic inventory retained. Eight lanes dispatched for disjoint failure repairs; root owns native fatal-stack diagnosis and evidence integration.

Reference153.0.8010.52 Chrome/ChromeDriver local about:blank session succeeded with acceptInsecureCerts=false, no errors, and owned process group cleaned up. This resolves local launch readiness, not full native/TLS/platform qualification. Earlier automatic-review infrastructure failures remain recorded.


## Repair 29 integration review — source only

Parallel source work has addressed the observer registry borrow abort, actual shared-buffer fixture metadata, frame policy-before-provenance admission and standalone native identities, foreign Event owner tags, same-owner option insertion, missing HTMLDetailsElement bindings, and native CSS profile admission before document creation. Canonical WebIDL source regenerated; next candidate check remains required. Independent review verified termination-guard destruction order and found additional setter/operation and nested native catchers; these are being integrated with original exception identity retained.

Layout source repairs cover flat-tree inheritance, inline-only Parley ranges, lazy vertical range subtraction, SVG geometry serialization, exact fractional/zero viewport state, lazy pseudo animation cascade and CSS flow height. Original108 failure records remain preserved; no claim that all108 pass. Fixture corrections are limited to proven mismatches between intended target and actual DOM/async lifecycle.

Exact test14 disassembly measured call_operation at242,768 bytes and generated binding installation at153,072 bytes of stack frame. Semantics-preserving dispatch/installation extraction is in progress to reduce nested allocation stack pressure without changing V8 checks or stack flags. Loader budget/publication, activity delegation and typed policy denial repairs are also integrating. Inputs are not yet frozen; no new test pass or qualification is claimed.


## Repair29 frozen; attempt15 started

Eight-lane repair batch integrated and reviewed. Declaration `GENERATION-COMPLETE-REPAIR-29-2026-09-18.json`, SHA-256 `5d1878f1aa23ff85a0c4ca98cc5c084be240203191b7a623bb409f5874a5a9c5`, validates all51 parents and unchanged review-source hashes. Explicitly refreshed26 previously bound paths and additionally bound21 reviewed source/evidence paths, including extracted dispatcher/installer, shared termination helper and native CSS profile regression. Source review receipt retained in `repair29-source-review.json`. Full five-package test15 launched in session63681; candidate sources frozen until terminal. No passing claim for this candidate.


## Attempt15 terminal — Stylo integration compilation

Attempt15 under repair29 exited101 after20.41082599999936 seconds before assertions. Nine compiler diagnostics identify newly admitted property integration: optional container-type serialization, content-visibility animation traits, font-synthesis-style Servo serialization and computed-style size assertion. Original logs retained; source freeze released for complete integration repair. Runtime results remain candidate28/test14; no new pass inferred.


## Repair30 frozen; attempt16 started

Nine Stylo integration diagnostics repaired with optional container serialization, existing content-visibility animation behavior enabled for Servo, font synthesis serde and size guard240 derived from one added Arc<Page>. Regression sources retained. Declaration SHA-256 `3a01585ad7da02f0ac932b0fdfde083384d3f34d916f71a3cfb8f5675ccc2b12` validated all51parents/review hashes. Full five-package attempt16 launched session12851; source freeze active and runtime verification pending.


## Attempt16 terminal — runtime progress and retained failures

Repair30 compiled. Test16 exited101 after471.8485798749971 seconds: DOM439/439, parser2/2, V8 teardown6/6, native CSS profile4/4, native style2/2, WebIDL16/16 passed. Window targets5/8. JS/native libraries SIGTRAP; layout child interrupted with SIGTERM only after native sample and source proved an infinite loop in vertical itemization/Parley custom-float acknowledgement. Cargo continued all other targets. No complete aggregate totals for aborted/interrupted targets. Four failed targets; exact raw logs, panic inventory, target records and interruption evidence retained.

Disassembly proves dispatcher242768->2656 bytes and installer coordinator153072->912 bytes; separately exposed pointer/touch chain retains1.50MiB. Parallel repair31 covers heap-owned input continuations, vertical itemization, remaining layout groups, actual pending V8 termination semantics, frame URL-before-policy binding, foreign platform owner tags, and HTML recoverable-error versus fatal-XML deferred-script classification. No runtime claims for these new repairs.


## Repair31 integration review — no new execution

Reviewed pointer/touch heap-owned continuations and separate construction frames, explicit vertical custom-box acknowledgement, frame URL-before-policy ordering and reduced nested link activation frames. Source repairs also address HTML versus fatal-XML completion provenance, foreign platform-object owner tags, optional editing-event probes and missing anchor/area download IDL reflection. Canonical download bindings regenerated; the next candidate still requires freshness validation.

SVG variable-font oracle now compares exact raster bytes at1x/2x against the same pinned glyph references, with shifted-reference negative sensitivity. Added only the existing locked resvg dependency to oryn-layout's Cargo.lock dependency list. Fixture repairs require actual overflow/body/source-backend setup while retaining intended assertions. Original failures remain visible in test16-repair-inventory.json; 71 of181 named test14 failures were observed passing in16,100 still failed and10 were unobserved/unresolved. These are comparison counts, not complete suite totals.

Pending integration includes synchronous observation of V8 pending termination (prior guard's TerminateExecution merely queued another request), remaining CSS/resource demand and input issues, and real native worker fixture authority. No tests run while this source batch is mutable.


## Repair31 independent reviews and next execution preparation

Independent source reviews found no blocking defect in the private termination checkpoint (including cold/Auto isolation and report-induced termination) or pointer continuation custody/allocation-frame separation. Actual runtime behavior and stack frame sizes still require candidate execution. Corrected the MessageChannel fixture against pinned independent-port queue semantics while retaining exact delivery, per-port FIFO, self-transfer rejection and doomed-message exclusion. Three worker fixtures now use real parent-backed native pages with bounded completion/error checks.

Prepared test17 wrapper for repair31; it has not executed and repair31 is not frozen. Remaining source work includes compound editing undo/designMode and radio contract reconciliation, final rendering delivery, parser construction deadline investigation and native CSS image demand investigation. Review receipt: `artifacts/completion-validation-2026-09-18/repair31-integration-review.json`. Full scope and original test16 failures remain unchanged.


### Repair31 remaining input integration delivered

DOM/input source delivery now includes contents-before-selection compound undo/recovery, designMode barrier handling, native owner identity, filename string storage, maxlength event admission, post-beforeinput inert/hidden target checks, datalist default type and nonempty-name radio group validation. Regression sources preserve same-value replacement, named required-radio invalidity and exact event/selection behavior. Label/showPicker remain diagnostic-only where source did not prove a defect.

Parser construction timeout retains its9500ms default and cleanup; new diagnostics separate setup/parser-wait elapsed time and structural progress counters. No speculative scheduler change or timeout increase. Root reconciled newly changed source paths against candidate30 and prepared additional manifest bindings; repair31 remains mutable pending final reviews, with no new runtime claim.


### Final repair31 review findings

Independent undo review confirmed all companion contents precede shared selection and full transaction leases survive failures; identified one stale regression expecting one retained snapshot instead of both. Root review found designMode override also needs actual traversal to the owning document to avoid overriding detached explicit editing hosts. Both corrections and regression coverage are assigned before freezing. Stale CSS image demand remains diagnostic-only: preserved assertions now report serialized style, native revision, computed background and exact demand sets. No new execution or passing claim.


## Repair31 frozen; attempt17 started

Final rendering delivery and DOM follow-up corrections integrated and reviewed. Declaration SHA-256 `ee69608fad3d29b9593b08e7f87c077f04b88f4d8c6954dc90af05701db03e4a` validates all51 parent requirements and review sources. Refreshed34 reviewed bound paths and additionally bound14 source/test/evidence paths. Source review receipt retained in `repair31-source-review.json`. Full five-package test17 launched in session56460; source inputs are frozen until terminal evidence. No passing result yet.


## Attempt17 terminal — ruby adapter integration

Test17 under repair31 exited101 after20.514659624997876 seconds during compilation; canonical WebIDL freshness passed. No runtime assertions ran. Newly admitted InternalRuby display exposed an exhaustive match in pinned stylo_taffy, with further source review identifying Blitz traversal/construction consumers. Source freeze released for complete ruby adapter integration. Root vendored the same pinned stylo_taffy revision with upstream file hashes and licenses, expanded its original workspace metadata/options, and changed only its source identity in Cargo.lock. Ruby remains required; no disabling or None mapping. Repair32 integration is in progress.


## Attempts18 terminal; repair33 frozen

Repair32 froze as `466be89cb407a77650b2633dbf905fbc5c186bd9b13d3f7546727560de73c804`. Attempt18 exited101 before compilation during manifest admission: normalization mistakenly expanded rust-version using package version. Corrected to pinned upstream1.90.0; compared all normalized package fields, dependency options and features against upstream originals. No runtime assertions ran. Repair33 frozen as `bc548f33e23e895d90638824113c4144ea8fa46b61d659b8b5e1637c96c330e1`; all51 parent and review hashes validated. Original candidate/results retained.


## Attempt19 terminal — remaining table display consumer

Attempt19 under repair33 exited101 after19.846257458993932 seconds during compilation. Vendored stylo_taffy compiled and WebIDL freshness passed. Blitz table collector has another exhaustive DisplayInside match missing ruby variants; no runtime assertions ran. Source freeze released for repair34. Prior source audit incorrectly classified this match as having a fallback; complete match/caller review is now required before selecting table handling. No runtime or full qualification improvement is claimed from compilation alone.


## Repair34 frozen; attempt20 started

Complete ruby display consumer review and source integration delivered, including native block-ruby routing/geometry and anonymous inline grouping regressions. Declaration SHA-256 `e7d21d5e34309266bace141b4cc7778e9399a50cf8bc4370a53dbe6ef2a119d3` validates all51 parent and review source hashes. Full five-package attempt20 launched session53425; source freeze active until terminal. Latest runtime evidence remains test16; no pass inferred.


## Attempt20 terminal — complete layout result and remaining aborts

Repair34 compiled; attempt20 exited101 after317.2254668749956 seconds. DOM440/440, parser2/2, V8 teardown6/6, native CSS profile6/6, native style2/2, WebIDL16/16 passed. Layout449passed39failed/488 finished without the prior hang; window targets7/8. JS library SIGABRT during destructor cleanup and native library SIGTRAP during actor-thread realm construction have no valid aggregate totals. Four failed targets; original logs/results remain intact.

Against original108 layout test14 failures,70 now pass and38 remain; current39th failure is the newly added internal-ruby grouping regression. Partial test16 observations are not complete totals. Eight lanes assigned repair35: destructor/storage custody, navigation, termination/parser bindings, loading/modules, input/forms, geometry, CSS/selector, and evidence inventory; root owns native fatal-stack diagnosis and shared integration. No new execution until the repair batch is integrated and frozen.


## Repair35 integration and independent review

Three measured native stack-frame extractions have passed independent source review: auxiliary-root construction, prerender waiting, and post-host document assembly preserve their original bodies and recovery/control flow. Compiled stack reduction remains unverified. Test21 wrapper is prepared but has not executed; repair35 is not frozen.

Header-policy audit corrected a stale fixture using pinned Fetch/WPT behavior: explicit author Authorization/custom headers survive both credentials settings; forbidden and malformed headers are rejected. Sticky-scroll fixture now has sufficient sibling overflow while preserving the containing block and exact assertions. Media registration, native wire-ID correlation, CSS resource discovery, parser/loading, layout and input repairs are integrated or under review.

Independent reviews found two activation defects (same-origin sibling/cousin propagation and complete live ancestry validation) and a parser failure-cleanup defect (premature document_load_stopped suppresses resource cleanup). These are assigned for repair and regression coverage before freeze. Source drift inventory records36 changed bound paths and8 recently modified unbound candidates, with timestamps used only for discovery; final explicit source reconciliation remains required. No new runtime passing claim or qualification result.


### Repair35 review follow-up

Activation fixes now pass independent source review: same-top sibling/cousin notification, complete live ancestor validation, descendant environment retirement and exact successor backend identity have regression sources. Data-script fixes also pass independent source review: sandbox admission precedes route selection and accepted resource timing retains render-blocking/preload metadata. Parser failure cleanup, buffered/completion budget coverage and failing-owner fairness remain under repair/review. The explicit declaration integration draft covers36 bound paths and9 additional source/wrapper bindings; it is not a frozen candidate. Test21 has not run.


## Repair35 frozen; attempt21 started

All final independent reviews accepted, including parser input cleanup, legacy task budgets and retry fairness. Refreshed36 reviewed bound paths and bound9 additional source/wrapper paths; all51 parent requirements and review-source hashes validated. Declaration SHA-256 `01d9fa9dd69bb03799dbe2d37d48018fac30b2ad03d81efac2f229814fdf1789`. Complete five-package attempt21 launched in session17772. Source inputs are frozen until terminal evidence. No new runtime result yet; compiled stack measurements and full qualification remain pending.


## Attempt21 terminal — activation context integration

Attempt21 under repair35 exited101 after81.93834141700063 seconds during compilation. WebIDL freshness passed; no runtime assertions ran. Three E0308 errors identify activation installation at host.rs realm-construction callers: the helper requires an entered Context scope while those callers retain an unentered scope. Repair36 is limited to correct native context integration and complete caller audit before a fresh candidate. Source freeze released; test20 remains latest runtime evidence. Native stack remeasurement is unavailable because no repaired native binary linked.


## Repair36 frozen; attempt22 started

Three activation factory callers and the equivalent successor test helper now enter the exact returned V8 Context for installation; source review preserves surrounding context and factory behavior. Declaration SHA-256 `12f783342163695ca46a909f6e33188f1ec1a0b32757320e31c09ecabebe9d26` validates all51 parent and review-source hashes. Complete five-package test22 launched session58090. Source freeze active until terminal; no new runtime result yet.


## Attempt22 terminal — repair37 source phase

Repair36 compiled and WebIDL freshness passed. Test22 exited101 after250.0745087499963 seconds: DOM441/441, parser2/2, V8teardown6/6, CSSprofile7/7, style2/2, WebIDL16/16 passed. Layout461passed29failed/490 and window7/8; both libraries SIGABRT with no aggregate total. Of262 previously observed test20 failures,67 now explicitly pass,155 still fail and40 are unobserved; unobserved is not passing. Original failures and exact comparisons retained in test22-repair-inventory.json.

Actual native binary measurements confirm constructor fixed frame225344→26544 bytes and related actor215472→82416; navigation step196192→191920 remains large. Test22 native abort instead arises in the new activation fixture: bare hosts have no bindings, then multiple-isolate vector teardown double-panics. Production activation install also ran before binding state existed. Repair37 moves it to actual native document installation and uses one-isolate real-document fixtures. JavaScript abort is a PerformanceObserver registry borrow held while registering entry wrappers; scoped snapshot/handle fixes and neighboring ResourceTiming correction are delivered for review.

Source freeze released. Parallel lanes cover remaining parser/load readiness, resource/module/preload behavior, navigation/exception realm and target identity, layout/print line custody, activation, observer custody and independent source/evidence reviews. No new tests until complete batch integration and candidate freeze. Full qualification remains incomplete.


### Repair37 reviewed source progress

Independent review accepted PerformanceObserver/ResourceTiming short borrow lifetimes, print UTF-16 suffix/line-alias custody, real activation installation, and fullscreen/media fixture lifecycle checks. Root replaced two Register-only mock transports with exact identity recorders; all8 original test bodies still assert behavior before explicit retirement and exactly-one-retirement checks. No finalizer callback assertion can double-panic in these mocks.

Parser review caught intentional open child streams: waiting for all parent load barriers would deadlock callers that must close/stop/remove a child. The correction separates constructor return readiness from actual load-event publication, preserving pending resource/script work. Loading source repairs preserve genuine preload authorization and descendant fetch-cache ownership; CSS source repairs admit sideways modes and directional part selectors with positive/negative coverage. These and remaining navigation/input changes are still in integration/review. Test23 wrapper is prepared but not executed; repair37 is not frozen.


### Further repair37 review closure

Independent review accepted sideways-axis admission, directional ::part selector matching, wrapped text alias regression coverage, and narrow obsolete-document AbortError handling for History/Location. The constructor filter now retains resource work, queued frames and lifecycle callbacks while allowing passive author-open streams to return; final mixed-resource regressions remain under integration. Loading and navigation source review continues, including preload proof/cache contracts and outgoing history scroll snapshots. No test23 execution or repair37 runtime result is claimed.


## Repair37 frozen; attempt23 started

All scoped source reviews complete, including final CSP alias checks and actual response-completion wait for Window.open. Refreshed27 reviewed bound paths and9 additional source/wrapper bindings. Declaration SHA-256 `e611f7047219d99c791ee9b41ceac2db60c44c79ea555eb459ddbee2125b464f` validates all51 parent requirements and review hashes. Full five-package test23 launched session26967. Source freeze active until terminal. No new runtime passing claim; all previously unresolved behavior and qualification requirements remain in scope.


## Attempt23 terminal — repair38 integration

Repair37 compiled; WebIDL freshness passed. Test23 exited101 after300.2922718749978 seconds. DOM442/442, parser2/2, V8teardown6/6, CSSprofile8/8, style2/2, WebIDL16/16 passed. Layout468passed23failed/491; window7/8. JS SIGABRT and native SIGTRAP have no aggregate total. Of191 prior observed test22 failures,54 now pass,133 still fail,4 unobserved. Exact original logs, new regressions and newly reached failures retained in test23-repair-inventory.json.

The JS abort exposed missing PiP method dispatch followed by a retirement-mock panic; all5 PerformanceObserver tests now pass. Native abort occurs during prerender bootstrap: measured selected fixed native frames total1,022,288 bytes, excluding V8/test ancestors. Repair38 boxes roster phase/candidate ownership, keeps ordinary navigation temporaries off the waiting-prerender stack, and moves final page assembly after bootstrap. Independent reviews preserve original body hashes, polling/retirement/recovery and boxed transfer custody. V8/thread stack limits unchanged; fresh measurement remains required.

Parallel repair38 deliveries include PiP routing/reentry, InterestEvent shadow retargeting, composition listener bookkeeping, typed observer errors, collapsed source aliases, font inheritance/fractional line/column geometry, computed table properties, bounded Data checkpoints and local data/blob frame routing, plus contract-backed async/typed-loader fixtures. Source integration/reviews continue; no repair38 tests have run and full qualification remains incomplete.
