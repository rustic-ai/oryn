# First committed-source production bundle

This recipe prepares one new native build for the consolidation migration. It
does not qualify the browser or authorize an unattended launch. The coordinator
must first commit/review the integrated source, independently review the exact
new generation declaration, reserve Cargo/signing resources, and run every build
stage through the owned-process wrapper. Scheduled work remains disabled.

## Retained C++ engine input

The reviewed starting profile is
`../oryn-wpt-acceptance17-page-transition-timer-fixture-r25/staged-release/core-full-manifest.json`,
relative to the integration checkout. Retain that profile unchanged. Its existing
schema-6 validator must pass against the newly committed source before reuse:

```sh
python3 scripts/validate-v8-runtime-profile.py "$ORYN_RETAINED_PROFILE" \
  --profile core-full --target aarch64-apple-darwin
```

The validator checks current V8 patches, Rust vendor source/lock selection,
regression inputs, compiler/header ABI custody and archive/binding/ICU hashes.
Reusing these archived C++ build inputs avoids another V8 build. It does **not**
permit reusing the R25 parent, page worker, snapshot or driver as a new Git build.
Rebuild the native startup snapshot and all three executables below. Do not run
`build-v8-runtime-profile.sh` for this first build unless engine-input validation
actually requires a new C++ archive.

## Identities and locations

Run from the integration checkout. Choose a new `BUILD_ID` once; use a new ID for
an original retry. The data root for the current migration is `../oryn-data`.
All paths stored in new input manifests are relative to that configured root.

| Object | Store-relative location |
| --- | --- |
| Committed source export | `sources/<commit>/export` |
| Raw Git object proof | `sources/<commit>/identity.json` |
| Reviewed generation declaration | `builds/<build-id>/generation.json` |
| Staged prelaunch engine input closure | `builds/<build-id>/inputs/engine/` |
| Mutable build assembly | `builds/<build-id>/assembly/` |
| New typed engine/runtime assets | `builds/<build-id>/engine/` |
| Final sealed executable bundle | `builds/<build-id>/runtime/` |
| Build manifest and process evidence | `builds/<build-id>/` and `runs/<attempt-id>/` |
| Reusable Cargo cache | `cache/cargo/<configuration-key>/` |

Compute the cache key with `builds.cargo_cache_key(configuration)`. The exact
configuration records Cargo.lock SHA256, full observed Rust toolchain identity,
host/target `aarch64-apple-darwin`, profile `release`, features and flags. Source
commit and Cargo.lock revisions do not allocate separate cache trees. Commands
retain the separate package feature selections shown below; a union in cache
configuration is not a substitute for recording those commands.

At the September 27 preparation check, `rustc -vV` reported Rust 1.98.0,
commit `88d9e12ae178fab0fb5cc050a94da85685d449ea`, LLVM 22.1.8. Re-observe these
values at admission; this document is not a toolchain receipt.

## Source and generation preparation

1. Complete a clean committed integration checkout. Audit the reviewed effective
   inputs; any ignored effective source needs its own pinned external declaration.
   A clean-looking index cannot conceal changed Git blobs or executable modes.
2. Call `generation.export_source(source_root, store_root, export_path=...,
   identity_path=..., expected_commit=...)`. This exports only committed content
   and raw Git commit/tree objects, not `.git`, target caches or historical runs.
   Independently validate its exact file/mode membership against the Git objects.
3. Call `generation.prepare_generation_gate(source_root, predecessor_path,
   output_path, expected_commit=...)`, starting with the exact reviewed R25 gate
   retained by R27. The schema-2 declaration preserves all 51 requirements and
   their implementation/test path mapping, binds the unchanged full completion
   plan and refreshes source hashes against the new clean commit. It does not
   create independent approval. Review the new declaration and pin its SHA256
   before production execution.
4. Set `ORYN_GENERATION_GATE` and `ORYN_GENERATION_GATE_SHA256` through
   `GenerationLease.environment(...)`. All existing canonical script checks
   remain active; there is no boolean generation bypass.

Use the shared CLI `source-export`, `source-info` and `generation-prepare`, or the
same Python APIs. Save preparation receipts outside the source checkout and
outside the exported source membership.

Explicitly register the exact `sources/<commit>`, `builds/<build-id>` and
`runs/<attempt-id>` roots in the retention index before execution/assembly, plus
any separately admitted engine/environment roots. Source/engine preparation does
not infer ownership registrations. An existing build-parent registration owns
its published child; do not register overlapping roots.

## Build environment and sequence

The coordinator constructs an explicit environment and records it in owned build
evidence. These variables are required:

```text
CARGO_TARGET_DIR=<data-root>/cache/cargo/<configuration-key>
ORYN_DATA_ROOT=<data-root>
ORYN_GENERATION_GATE=<data-root>/builds/<build-id>/generation.json
ORYN_GENERATION_GATE_SHA256=<independently reviewed declaration SHA256>
ORYN_TYPED_SOURCE_IDENTITY=<data-root>/sources/<commit>/identity.json
ORYN_ARTIFACT_ROOT=<data-root>/builds/<build-id>/assembly
ORYN_CODESIGN_IDENTITY=<verified current production certificate SHA1>
```

The retained signing contract expects team `5HVA9VFF8K`, worker identifier
`ai.rustic.oryn.page-worker`, Apple Development certificate chain, hardened
runtime, and exactly App Sandbox/JIT entitlements. The prior selected certificate
was `9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90`; verify its current availability and
trust before selection. Never substitute ad-hoc signing. The canonical worker
script performs fresh certificate, signature, runtime and sandbox probes.

Before any native command, call `runtime_metadata.stage_engine_inputs(...)`
with the retained profile, `output_directory=builds/<build-id>/inputs/engine`
and `diagnostics_directory=builds/<build-id>/assembly/crashreports`. This copies
all eight schema-6 artifacts and every declared compiler/header input into the
data root, preserves compiler resource-directory layout, and emits a new
schema-6 profile and compile descriptor with known path operands remapped.
Normalized compiler ABI flags and archive/binding bytes must remain identical.
The historical descriptor remains a separate retained input; no old file is
changed. Compiler diagnostics use mutable assembly storage outside the input
lease. V8 API headers come from the committed `vendor/v8-api/include` inventory;
unused historical GN include roots become private empty directories.

Lease all returned `build_inputs` records for every build stage: exact directory
membership plus individual profile, compiler, header and artifact records.
Include it in the native compilation digest separately from post-build evidence.
Use the returned `environment` paths for compilation:

```text
RUSTY_V8_ARCHIVE=archive_path
RUSTY_V8_SRC_BINDING_PATH=binding_path
ORYN_V8_BRIDGE_ABI=bridge_abi_path
ORYN_ICU_DATA_FINGERPRINT_PATH=icu_data_path
SDKROOT=the explicitly observed system SDK
```

The system SDK remains an explicit platform dependency. Staging records its
selected/resolved location and SDKSettings metadata hashes; this observation
does not attest all SDK headers. The build coordinator must admit and recheck
the platform/toolchain observation around compilation. All compiler paths,
headers and engine data outside that SDK resolve to the staged store. Final
typed metadata binds the remapped descriptor actually embedded by the fresh
native build, not the historical descriptor digest.

### A. Launch the complete owned build

Set:

```text
ORYN_V8_PROFILE_MANIFEST=<staged-engine-inputs>/core-full-manifest.json
ORYN_TYPED_RUNTIME_OUTPUT=builds/<build-id>/engine
```

Leave `ORYN_TYPED_ENGINE_PROFILE` unset when using this automatic publication
route; setting both is rejected. Leave all legacy prebuilt-candidate substitution
variables unset. Then execute the existing canonical stage:

```sh
/bin/bash scripts/build-sandboxed-worker.sh
```

It now executes these native build commands under the same cache and source gate:

```sh
env -u ORYN_STARTUP_SNAPSHOT /bin/bash scripts/build-startup-snapshot.sh \
  core-full "$RUSTY_V8_ARCHIVE" "$ORYN_ARTIFACT_ROOT/oryn-startup-core-full.blob"
cargo build --locked --release -p oryn --bin oryn --bin oryn-webdriver \
  --no-default-features --features "native core-full webdriver-server"
cargo build --locked --release -p oryn-native \
  --features v8-host,v8-sandbox,core-full --bin oryn-page-worker
```

The worker script first creates a fresh snapshot using locked Cargo resolution
and the recorded V8 binding/compiler environment. Snapshot generation clears any
inherited `ORYN_STARTUP_SNAPSHOT`. Before compiling the parent or worker, it calls
`engine-profile-refresh` to create
`<assembly>/snapshot-core-full-manifest.json`, then selects the new snapshot for
both native builds. Preexisting snapshot/profile outputs are rejected.

The intermediate profile is a build output, not an undeclared prelaunch input.
The inherited `ORYN_V8_PROFILE_MANIFEST` remains the original staged profile for
the owned build's input receipt; the script uses the new profile locally for
subsequent checks. Its historical worker record is retained only until the fresh
worker refresh below. Do not run snapshot generation as a separate unrecorded
preparation step.

The parent WebDriver command and the exec-only driver's `--help` must succeed.
These are role/readiness checks, not WPT results. The driver still launches the
explicit selected browser through process replacement; it is not a second browser
implementation.

After the new worker's runtime probe, the script refreshes the intermediate
profile with the **newly compiled raw worker** and validates compiled Rust V8,
archive/binding and embedded bridge descriptor identity. It publishes typed
assets, copies the schema-7 profile into the worker app, writes the clean source
commit and tree identity into its manifest, and only then signs. The original
schema-6 profile, old worker and signed app remain untouched.

Do not pre-run `typed-runtime-prepare` against the retained R25 worker and call
that the new build. Typed metadata conversion itself has no authority to prove
which commit produced a binary.

### B. Assemble and seal

After the owned build process has terminated successfully with complete cleanup,
call `assembly.assemble_build(source_root, store_root, ...)` or `build-assemble`
with these store-relative arguments:

```text
execution_path=runs/<attempt-id>/build-execution.json
source_export=sources/<commit>/export
source_identity=sources/<commit>/identity.json
generation_gate=builds/<build-id>/generation.json
generation_review=builds/<build-id>/generation-review.json
output_directory=builds/<build-id>/published
build_id=<build-id>
```

The assembler acquires signing, build-ID and Cargo-cache resources, rechecks
the original build and all seven output roles, then independently copies the
parent, driver, complete signed worker app (including empty directories), typed
engine assets and unchanged original process logs into `published/runtime`.
It creates `build.json` and `assets.json`, verifies fresh production signing and
source/engine/process joins, and publishes `assembly-receipt.json` only after
the final input-lease check. Failed assembly has no completion receipt; preserve
its partial directory and choose a new published directory for an authorized retry.

Retention pins the published object's registered owner and its immutable source
and engine dependency owners. Copied executable outputs do not retain a Cargo
cache dependency. No compiler outputs are hardlinked. The result still requires
native readiness and independent run review; assembly is not qualification.

The coordinator's build evidence joins the actual owned process receipt, exact
commands, source/generation/configuration and produced executable hashes.
`build_process_evidence_required: true` in a preparation receipt is a reminder,
not such proof. Independent review and relocated production admission follow
before any WPT native/topology/canary/placement execution.

## Disk admission

Measured on September 27 before this build: free space was 279,756,800,000 bytes.
The tracked checkout had 19,637 files totaling 346,679,664 bytes. The retained
archive, binding, ICU data, snapshot, raw worker, external-reference table, bridge
descriptor and 2,026 compiler/header inputs totaled 419,190,774 bytes. These are
observations, not a forecast for the new source tree or compiler outputs.

The source export and one typed engine copy therefore have a measured lower
bound of approximately 766 MB before the new parent/driver, extra app copy,
signatures, logs and native Cargo cache. Native compilation's peak cache demand
has not yet been measured under the new configuration. Reserve at least 50 GiB
of free space, check available space and active cache usage at admission, and
monitor actual demand throughout the owned build. Do not claim the lower bound
is a complete build-space estimate or rebuild V8 merely to create another cache.

Pin the source export, engine assets and selected build; register the reusable
cache before leasing it. Cache eviction must respect current leases and exact
preview receipts. Keep R27's fallback inputs until the consolidation migration
has produced and reviewed one complete standard-block original.
