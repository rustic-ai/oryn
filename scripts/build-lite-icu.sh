#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="${1:?usage: build-lite-icu.sh <rusty-v8-source> <output.dat>}"
output="${2:?usage: build-lite-icu.sh <rusty-v8-source> <output.dat>}"
source_dir="$(cd "$source_dir" && pwd)"
mkdir -p "$(dirname "$output")"
output="$(cd "$(dirname "$output")" && pwd)/$(basename "$output")"
filter="$repo_root/config/icu/core-lite-en.json"
break_iterator_list="$repo_root/config/icu/core-lite-break-iterator-files.txt"
icu_source="$source_dir/third_party/icu/source"
icu_revision="$(git -C "$source_dir/third_party/icu" rev-parse HEAD)"
engine_cache_root="${ORYN_ENGINE_CACHE_ROOT:-$repo_root/target}"
case "$engine_cache_root" in /*) ;; *) engine_cache_root="$repo_root/$engine_cache_root" ;; esac
tools_dir="${ORYN_ICU_TOOLS_DIR:-$engine_cache_root/icu-tools-$icu_revision}"
case "$tools_dir" in /*) ;; *) tools_dir="$repo_root/$tools_dir" ;; esac

test -f "$icu_source/runConfigureICU"
test -f "$filter"
test -f "$break_iterator_list"
mkdir -p "$tools_dir"
if [[ ! -x "$tools_dir/bin/pkgdata" ]]; then
  (
    cd "$tools_dir"
    "$icu_source/runConfigureICU" MacOSX \
      --disable-shared --enable-static --disable-tests --disable-samples
    make -j"${ORYN_BUILD_JOBS:-4}"
  )
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/oryn-lite-icu.XXXXXX")"
out_dir="$work_dir/out"
tmp_dir="$work_dir/tmp"
mkdir -p "$out_dir" "$tmp_dir"

library_path="$tools_dir/lib:$tools_dir/stubdata:$tools_dir/tools/ctestfw"
tool_icu_data="$tools_dir/data/out/build/icudt77l"
test -d "$tool_icu_data"
# ICU's generators are linked against stubdata. Seed only the Unicode core
# tables they require while the filtered package is being assembled; the
# databuilder's list remains authoritative for what pkgdata includes.
find "$tool_icu_data" -maxdepth 1 -type f \
  \( -name '*.icu' -o -name '*.nrm' \) -exec cp {} "$out_dir" \;
(
  cd "$icu_source"
  ICU_DATA="$tool_icu_data" \
  DYLD_LIBRARY_PATH="$library_path${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
  LD_LIBRARY_PATH="$library_path${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  PYTHONPATH="$icu_source/python" \
    python3 -m icutools.databuilder \
      --mode=unix-exec \
      --src_dir="$icu_source/data" \
      --filter_file="$filter" \
      --include_uni_core_data \
      --out_dir="$out_dir" \
      --tmp_dir="$tmp_dir" \
      --tool_dir="$tools_dir/bin"
)

list="$tmp_dir/icudata.lst"
test -s "$list"
DYLD_LIBRARY_PATH="$library_path${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
LD_LIBRARY_PATH="$library_path${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  "$tools_dir/bin/pkgdata" \
    -m common -p icudt77l -c \
    -O "$tools_dir/data/icupkg.inc" \
    -s "$out_dir" -d "$tmp_dir" "$list"

generated="$tmp_dir/icudt77l.dat"
test -s "$generated"
break_iterator_dir="$work_dir/break-iterator"
augmented="$work_dir/icudt77l.dat"
mkdir -p "$break_iterator_dir"
"$tools_dir/bin/icupkg" --auto_toc_prefix --ignore-deps \
  -x "$break_iterator_list" -d "$break_iterator_dir" \
  "$source_dir/third_party/icu/common/icudtl.dat"
"$tools_dir/bin/icupkg" --auto_toc_prefix --ignore-deps \
  -a "$break_iterator_list" -s "$break_iterator_dir" \
  "$generated" "$augmented"
cp "$augmented" "$output"
shasum -a 256 "$output"
