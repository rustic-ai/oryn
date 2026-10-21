#!/usr/bin/env bash
set -euo pipefail
# Offline, pinned source inputs. No implicit download or system library fallback.
source_directory="${1:?source directory containing verified upstream archives}"
output_directory="${2:?new absolute output directory}"
case "$output_directory" in /*) ;; *) echo 'output must be absolute' >&2; exit 2 ;; esac
test ! -e "$output_directory"
verify_archive() {
  local path="$1" expected="$2" actual
  actual="$(shasum -a 256 "$path" | awk '{print $1}')"
  test "$actual" = "$expected" || { echo "source hash mismatch: $path" >&2; exit 2; }
}
verify_archive "$source_directory/libxml2-2.14.6.tar.xz" 7ce458a0affeb83f0b55f1f4f9e0e55735dbfc1a9de124ee86fb4a66b597203a
verify_archive "$source_directory/libxslt-1.1.43.tar.xz" 5a3d6b383ca5afc235b171118e90f5ff6aa27e9fea3303065231a6d403f0183a
mkdir -p "$output_directory/source" "$output_directory/prefix"
tar -xf "$source_directory/libxml2-2.14.6.tar.xz" -C "$output_directory/source"
tar -xf "$source_directory/libxslt-1.1.43.tar.xz" -C "$output_directory/source"
prefix="$output_directory/prefix"
(
  cd "$output_directory/source/libxml2-2.14.6"
  ./configure --prefix="$prefix" --disable-shared --enable-static --without-python --without-lzma --without-zlib --without-iconv --without-icu
  make -j4
  make install
)
(
  cd "$output_directory/source/libxslt-1.1.43"
  PKG_CONFIG_PATH="$prefix/lib/pkgconfig" ./configure --prefix="$prefix" --disable-shared --enable-static --without-python --without-crypto --with-libxml-prefix="$prefix"
  make -j4
  make install
)
shasum -a 256 "$prefix/lib/libxml2.a" "$prefix/lib/libxslt.a"
printf 'ORYN_XSLT_INCLUDE_DIR=%s\nORYN_XSLT_STATIC_LIB_DIR=%s\n' "$prefix/include/libxml2" "$prefix/lib"
