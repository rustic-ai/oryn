# Rusty V8 weak callback teardown repair

Source generated; runtime regressions are pending the coordinated validation batch.

## Observed failure and ownership repair

The test08 `window_target_names` crash reached `Weak<Object>::first_pass_callback` during native CppHeap teardown after the Rust isolate annex had revoked its handle. Slot-owned Weak destruction could consequently release its callback parameter without resetting the native weak global. The native collector still held that parameter.

The Rust-only patch retains each callback parameter in an isolate registry. It resets live native globals before revoking isolate access. Queued second-pass parameters remain retained independently of finalizer-map membership, and callback-local strong owners survive registry removal and user finalizers. Snapshot creation retains its annex through destruction of the native SnapshotCreator. No C++ source, generated ABI binding, or archive has been changed.

## Reproducible source

`vendor/rusty-v8` is an archive-consumer subset of authenticated crates.io `v8-150.4.0.crate`, SHA256 `42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49`. The compared active source and manifests were byte-identical to this archive. `scripts/vendor-rusty-v8.py` authenticates the archive and patch, copies the declared Rust source/target/helper subset, applies the patch, and writes the exact complete source inventory. Existing destinations are rejected. `V8_FROM_SOURCE` is explicitly rejected by this subset; engine compilation remains the separate isolated source-checkout workflow.

The upstream tests require ICU data absent from the published crate. The included `third_party/icu/common/icudtl.dat` is separately attributed to the retained pinned engine source, ICU revision `ee5f27adc28bd3f15b2c293f726d14d2e336cbd5`, SHA256 `1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612`. Its bytes were checked at materialization and it is explicitly separated from upstream base inventory. Every declared Cargo target source is present.

Canonical inventory: `vendor/rusty-v8/oryn-source-manifest.json`. Patch and before/after file hashes: `patches/rusty-v8/weak-callback-teardown.{patch,json}`. Cargo uses the local override; the parent validation coordinator owns workspace lock resolution and schema6 signed profile regeneration.

## Required execution

Use the candidate's same verified V8 archive/bindings and sandbox feature environment:

- Workspace API regression target: `cargo test -p oryn-js --features v8-host --test rusty_v8_weak_teardown`. The wrapper imports the exact reviewed repository source, covering slot destruction, guaranteed finalizers releasing other Weak handles, raw roundtrip/cancellation, regular GC finalizers releasing their own handles, snapshot teardown, and a native CppHeap owner destroying a Weak.
- Internal registry custody: `CARGO_TARGET_DIR="$PWD/target/rusty-v8-weak-regressions" cargo test --manifest-path vendor/rusty-v8/Cargo.toml --locked --features v8_enable_sandbox --lib weak_callback_custody_tests`. This deterministic source models pending second-pass parameters after external-owner/finalizer removal, both eventual callback completion and native-dispose release. The separate target directory prevents build products from changing the inventoried vendor tree. Its copied upstream lockfile has no new dependencies; resolver acceptance remains to be executed.
- Repeat the originally failing native/window target tests in the complete validation run.

Independent source review found the registry ownership and snapshot ordering coherent. This is source-review evidence, not a passing runtime or conformance result.

## Repair24 source correction after first frozen execution

Repair23 private unit run reached compilation with its unchanged upstream lockfile, then failed E0282 in the internal assertion because the inherited `Borrow` trait made `Rc::borrow()` ambiguous. Both assertions now explicitly dereference the Rc with `as_ref().borrow()`. No private unit ran; lease checks before and after the attempt passed. Original artifacts remain under `rusty-v8-custody-tests-01.*`.

The workspace regression executable separately exposed missing static-archive framework metadata. The retained engine's `third_party/partition_alloc/src/partition_alloc/BUILD.gn` lines603–613 declares Security on macOS and CoreFoundation on Apple platforms; `page_allocator_internals_posix.cc` calls the SecTask entitlement functions and CF APIs. The vendored build script now emits Security and CoreFoundation links when the Cargo target OS is macOS, so standalone consumers receive the archive's required dependencies directly. This changes Rust build metadata only; the C++ archive is unchanged. No wrapper-only dependency anchor was added.

Updated tree digest: `0aed403b55e45b17148d0ad018ba534215ddff74760b3fc67dae43ed6c9f9184`. Patch digest: `ac4352f5cb39d1b434fdcf8dfefb42e2ffdcf3d409a0b824dab0c604de5a7574`. These corrections await the next frozen validation run.
