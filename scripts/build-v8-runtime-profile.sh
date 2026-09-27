#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
profile="${1:?usage: build-v8-runtime-profile.sh core-full|core-lite <output-directory>}"
output="${2:?usage: build-v8-runtime-profile.sh core-full|core-lite <output-directory>}"
case "$profile" in core-full|core-lite) ;; *) echo "invalid profile: $profile" >&2; exit 2;; esac

rusty_v8_version="150.4.0"
rusty_v8_revision="5c15a6995c9bb4bacd3e341b59fff32c909c80bf"
icu_revision="ee5f27adc28bd3f15b2c293f726d14d2e336cbd5"
engine_cache_root="${ORYN_ENGINE_CACHE_ROOT:-$repo_root/target}"
case "$engine_cache_root" in /*) ;; *) engine_cache_root="$repo_root/$engine_cache_root" ;; esac
export ORYN_ENGINE_CACHE_ROOT="$engine_cache_root"
source_dir="${ORYN_RUSTY_V8_SOURCE:-$engine_cache_root/rusty-v8-src-$rusty_v8_version}"
case "$source_dir" in /*) ;; *) source_dir="$repo_root/$source_dir" ;; esac
if [[ "$profile" == core-full ]]; then
  default_v8_target_dir="$engine_cache_root/rusty-v8-build-$rusty_v8_version"
else
  default_v8_target_dir="$engine_cache_root/rusty-v8-build-$rusty_v8_version-$profile"
fi
v8_target_dir="${ORYN_V8_TARGET_DIR:-$default_v8_target_dir}"
case "$v8_target_dir" in /*) ;; *) v8_target_dir="$repo_root/$v8_target_dir" ;; esac
oryn_target_dir="${CARGO_TARGET_DIR:-$repo_root/target/v8-$profile}"
case "$oryn_target_dir" in /*) ;; *) oryn_target_dir="$repo_root/$oryn_target_dir" ;; esac
bootstrap_target="${ORYN_V8_BOOTSTRAP_TARGET_DIR:-$engine_cache_root/rusty-v8-build-$rusty_v8_version}"
case "$bootstrap_target" in /*) ;; *) bootstrap_target="$repo_root/$bootstrap_target" ;; esac

mkdir -p "$output" "$engine_cache_root" "$(dirname "$source_dir")"
output="$(cd "$output" && pwd)"
rust_sources="$output/rusty-v8-source-inputs.json"
python3 "$repo_root/scripts/rusty_v8_source_provenance.py" --output "$rust_sources"
if [[ ! -e "$source_dir/.git" ]]; then
  git clone --depth 1 --branch "v$rusty_v8_version" \
    https://github.com/denoland/rusty_v8.git "$source_dir"
fi
test "$(git -C "$source_dir" rev-parse HEAD)" = "$rusty_v8_revision"
git -C "$source_dir" submodule update --init --recursive --depth 1
test "$(git -C "$source_dir/third_party/icu" rev-parse HEAD)" = "$icu_revision"
v8_revision="ac1e23989121713ca642f6650b34deff7b686896"
test "$(git -C "$source_dir/v8" rev-parse HEAD)" = "$v8_revision"
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8"
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch shared-memory-growth
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch shared-memory-wait
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch shared-wasm-size
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch shared-wasm-backing
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch shared-memory-factory
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch frame-context-reattachment
python3 "$repo_root/scripts/apply-v8-window-patch.py" "$source_dir/v8" --patch html-constructor-deferred-prototype
html_constructor_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/html-constructor-deferred-prototype.patch" | awk '{print $1}')"
frame_context_reattachment_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/frame-context-reattachment.patch" | awk '{print $1}')"
v8_regression_manifest_sha="$(shasum -a 256 "$repo_root/patches/v8/tests/manifest.json" | awk '{print $1}')"
shared_memory_factory_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/shared-memory-factory.patch" | awk '{print $1}')"
shared_wasm_backing_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/shared-wasm-backing.patch" | awk '{print $1}')"
shared_wasm_size_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/shared-wasm-size.patch" | awk '{print $1}')"
shared_memory_wait_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/shared-memory-wait.patch" | awk '{print $1}')"
python3 "$repo_root/scripts/install-v8-regression-sources.py" "$source_dir/v8"
shared_memory_growth_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/shared-memory-growth.patch" | awk '{print $1}')"
v8_patch_sha="$(shasum -a 256 "$repo_root/patches/v8/window-proxy-structural-semantics.patch" | awk '{print $1}')"


extra_gn_args="v8_drumbrake_bounds_checks=true v8_enable_sandbox=true v8_enable_pointer_compression=true v8_enable_external_code_space=true"
icu_source="$source_dir/third_party/icu/common/icudtl.dat"
if [[ "$profile" == core-lite ]]; then
  if [[ -n "${ORYN_LITE_ICU_DATA:-}" ]]; then
    icu_source="$ORYN_LITE_ICU_DATA"
  else
    icu_source="$engine_cache_root/generated-icu-$icu_revision-core-lite.dat"
    "$repo_root/scripts/build-lite-icu.sh" "$source_dir" "$icu_source"
  fi
  test -f "$icu_source"
  extra_gn_args="$extra_gn_args icu_use_data_file=true icu_use_stub_data=true v8_depend_on_icu_data_file=true icu_copy_icudata_to_root_build_dir=true"
fi

libclang_path="${LIBCLANG_PATH:-}"
if [[ -z "$libclang_path" && -f /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/libclang.dylib ]]; then
  libclang_path=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib
fi
bootstrap_tools="$bootstrap_target/release/ninja_gn_binaries"
gn_path="${GN:-$bootstrap_tools/gn/gn}"
ninja_path="${NINJA:-$bootstrap_tools/ninja/ninja}"
clang_base_path="${CLANG_BASE_PATH:-$bootstrap_target/release/clang}"

(
  cd "$source_dir"
  CARGO_TARGET_DIR="$v8_target_dir" \
  V8_FROM_SOURCE=1 \
  EXTRA_GN_ARGS="$extra_gn_args" \
  GN="$gn_path" \
  NINJA="$ninja_path" \
  CLANG_BASE_PATH="$clang_base_path" \
  LIBCLANG_PATH="$libclang_path" \
    cargo build --locked --release --features v8_enable_sandbox
)

archive="$v8_target_dir/release/gn_out/obj/librusty_v8.a"
binding="$v8_target_dir/release/gn_out/src_binding.rs"
test -f "$archive"
test -f "$binding"
archive_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
binding_sha="$(shasum -a 256 "$binding" | awk '{print $1}')"
icu_sha="$(shasum -a 256 "$icu_source" | awk '{print $1}')"
archive_size="$(stat -f %z "$archive" 2>/dev/null || stat -c %s "$archive")"
binding_size="$(stat -f %z "$binding" 2>/dev/null || stat -c %s "$binding")"
icu_size="$(stat -f %z "$icu_source" 2>/dev/null || stat -c %s "$icu_source")"
archive_out="$output/librusty_v8-$profile-$archive_sha.a"
binding_out="$output/src_binding-$profile-$binding_sha.rs"
cp "$archive" "$archive_out"
cp "$binding" "$binding_out"
if [[ "$profile" == core-lite ]]; then
  icu_out="$output/icudtl-$profile-$icu_sha.dat"
  cp "$icu_source" "$icu_out"
  icu_source="$icu_out"
fi

bridge_abi_out="$output/v8-bridge-abi-$profile.json"
python3 "$repo_root/scripts/prepare-v8-bridge-abi.py" \
  --source "$source_dir" --gn-out "$v8_target_dir/release/gn_out" \
  --archive "$archive_out" --binding "$binding_out" \
  --target "$(rustc -vV | awk '/host:/ {print $2}')" --output "$bridge_abi_out"
bridge_abi_sha="$(shasum -a 256 "$bridge_abi_out" | awk '{print $1}')"
bridge_abi_size="$(stat -f %z "$bridge_abi_out" 2>/dev/null || stat -c %s "$bridge_abi_out")"
export ORYN_V8_BRIDGE_ABI="$bridge_abi_out"

export CARGO_TARGET_DIR="$oryn_target_dir"
export RUSTY_V8_ARCHIVE="$archive_out"
export RUSTY_V8_SRC_BINDING_PATH="$binding_out"
export ORYN_ICU_DATA_FINGERPRINT_PATH="$icu_source"
if [[ "$profile" == core-lite ]]; then
  export ORYN_ICU_DATA_PATH="$icu_source"
fi
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo build --locked --manifest-path "$repo_root/Cargo.toml" -p oryn-native --release \
  --features "v8-host v8-sandbox $profile" --bin oryn-startup-snapshot

snapshot_raw="$oryn_target_dir/release/oryn-startup-$profile.blob"
"$oryn_target_dir/release/oryn-startup-snapshot" "$snapshot_raw"
snapshot_sha="$(shasum -a 256 "$snapshot_raw" | awk '{print $1}')"
snapshot_out="$output/oryn-startup-$profile-$snapshot_sha.blob"
cp "$snapshot_raw" "$snapshot_out"
snapshot_size="$(stat -f %z "$snapshot_out" 2>/dev/null || stat -c %s "$snapshot_out")"
export ORYN_STARTUP_SNAPSHOT="$snapshot_out"

external_references="$repo_root/crates/oryn-native/assets/external-references-v1.json"
external_references_sha="$(shasum -a 256 "$external_references" | awk '{print $1}')"
external_references_size="$(stat -f %z "$external_references" 2>/dev/null || stat -c %s "$external_references")"
icu_filter_sha=""
break_iterator_manifest_sha=""
if [[ "$profile" == core-lite ]]; then
  icu_filter_sha="$(shasum -a 256 "$repo_root/config/icu/core-lite-en.json" | awk '{print $1}')"
  break_iterator_manifest_sha="$(shasum -a 256 "$repo_root/config/icu/core-lite-break-iterator-files.txt" | awk '{print $1}')"
fi
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo build --locked --manifest-path "$repo_root/Cargo.toml" -p oryn-native --release \
  --features "v8-host v8-sandbox $profile" --bin oryn-page-worker

worker="$oryn_target_dir/release/oryn-page-worker"
worker_sha="$(shasum -a 256 "$worker" | awk '{print $1}')"
worker_size="$(stat -f %z "$worker" 2>/dev/null || stat -c %s "$worker")"
python3 "$repo_root/scripts/rusty_v8_source_provenance.py" --verify "$rust_sources"
worker_runtime="$output/worker-runtime-info.json"
"$worker" --runtime-info > "$worker_runtime"
python3 "$repo_root/scripts/rusty_v8_source_provenance.py" --verify "$rust_sources"
python3 "$repo_root/scripts/validate-v8-rust-runtime.py" "$rust_sources" "$worker_runtime" --binding "$binding_out" --archive "$archive_out"
jq -n \
  --slurpfile rust_sources "$rust_sources" \
  --arg bridge_abi "$bridge_abi_out" \
  --arg bridge_abi_sha256 "$bridge_abi_sha" \
  --argjson bridge_abi_size "$bridge_abi_size" \
  --arg profile "$profile" \
  --arg target "$(rustc -vV | awk '/host:/ {print $2}')" \
  --arg v8 "$rusty_v8_version" \
  --arg rusty_v8_revision "$rusty_v8_revision" \
  --arg icu_revision "$icu_revision" \
  --arg icu_data_path "$icu_source" \
  --arg v8_revision "$v8_revision" \
  --arg v8_patch_sha256 "$v8_patch_sha" \
  --arg shared_memory_growth_patch_sha256 "$shared_memory_growth_patch_sha" \
  --arg shared_memory_wait_patch_sha256 "$shared_memory_wait_patch_sha" \
  --arg shared_wasm_size_patch_sha256 "$shared_wasm_size_patch_sha" \
  --arg shared_memory_factory_patch_sha256 "$shared_memory_factory_patch_sha" \
  --arg html_constructor_deferred_prototype_patch_sha256 "$html_constructor_patch_sha" \
  --arg frame_context_reattachment_patch_sha256 "$frame_context_reattachment_patch_sha" \
  --arg v8_regression_manifest_sha256 "$v8_regression_manifest_sha" \
  --arg shared_wasm_backing_patch_sha256 "$shared_wasm_backing_patch_sha" \
  --arg archive "$archive_out" \
  --arg archive_sha256 "$archive_sha" \
  --argjson archive_size "$archive_size" \
  --arg binding "$binding_out" \
  --arg binding_sha256 "$binding_sha" \
  --argjson binding_size "$binding_size" \
  --arg icu_sha256 "$icu_sha" \
  --argjson icu_size "$icu_size" \
  --arg icu_filter_sha256 "$icu_filter_sha" \
  --arg break_iterator_manifest_sha256 "$break_iterator_manifest_sha" \
  --arg snapshot "$snapshot_out" \
  --arg snapshot_sha256 "$snapshot_sha" \
  --argjson snapshot_size "$snapshot_size" \
  --arg external_references "$external_references" \
  --arg external_references_sha256 "$external_references_sha" \
  --argjson external_references_size "$external_references_size" \
  --arg worker "$worker" \
  --arg worker_sha256 "$worker_sha" \
  --argjson worker_size "$worker_size" \
  --arg gn_args "$extra_gn_args" \
  '($rust_sources[0] + {schema_version:6,rusty_v8_archive_compatibility:{cpp_changes:false,crate_version:$v8,archive_sha256:$archive_sha256,binding_sha256:$binding_sha256},bridge_abi_path:$bridge_abi,bridge_abi_sha256:$bridge_abi_sha256,bridge_abi_size_bytes:$bridge_abi_size,v8_revision:$v8_revision,v8_patch_sha256:$v8_patch_sha256,shared_memory_growth_patch_sha256:$shared_memory_growth_patch_sha256,shared_memory_wait_patch_sha256:$shared_memory_wait_patch_sha256,shared_wasm_size_patch_sha256:$shared_wasm_size_patch_sha256,shared_wasm_backing_patch_sha256:$shared_wasm_backing_patch_sha256,shared_memory_factory_patch_sha256:$shared_memory_factory_patch_sha256,frame_context_reattachment_patch_sha256:$frame_context_reattachment_patch_sha256,html_constructor_deferred_prototype_patch_sha256:$html_constructor_deferred_prototype_patch_sha256,v8_regression_manifest_sha256:$v8_regression_manifest_sha256,profile:$profile,target:$target,v8_version:$v8,rusty_v8_revision:$rusty_v8_revision,icu_revision:$icu_revision,v8_heap_sandbox:true,pointer_compression:true,archive_path:$archive,archive_sha256:$archive_sha256,archive_size_bytes:$archive_size,binding_path:$binding,binding_sha256:$binding_sha256,binding_size_bytes:$binding_size,icu_data_path:$icu_data_path,icu_data_sha256:$icu_sha256,icu_data_size_bytes:$icu_size,icu_filter_sha256:(if $icu_filter_sha256 == "" then null else $icu_filter_sha256 end),break_iterator_manifest_sha256:(if $break_iterator_manifest_sha256 == "" then null else $break_iterator_manifest_sha256 end),startup_snapshot_path:$snapshot,startup_snapshot_sha256:$snapshot_sha256,startup_snapshot_size_bytes:$snapshot_size,external_references_path:$external_references,external_references_sha256:$external_references_sha256,external_references_size_bytes:$external_references_size,worker_path:$worker,worker_sha256:$worker_sha256,worker_size_bytes:$worker_size,gn_args:$gn_args})' \
  > "$output/$profile-manifest.json"
python3 "$repo_root/scripts/validate-v8-runtime-profile.py" "$output/$profile-manifest.json" --profile "$profile"
if [[ -n "${ORYN_TYPED_RUNTIME_OUTPUT:-}" ]]; then
  python3 "$repo_root/scripts/oryn-validate.py" typed-runtime-prepare \
    --source-root "$repo_root" --store-root "${ORYN_DATA_ROOT:?typed runtime requires data root}" \
    --legacy-profile "$output/$profile-manifest.json" \
    --output-directory "$ORYN_TYPED_RUNTIME_OUTPUT" \
    --source-identity "${ORYN_TYPED_SOURCE_IDENTITY:?typed runtime requires committed source identity}"
fi
printf '%s\n' "$output/$profile-manifest.json"
