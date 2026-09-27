#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

v8_package_pattern='^[[:space:]│├└─]*v8 v[0-9]'

default_tree="$(cargo tree --locked -p oryn -e normal,build)"
if grep -Eq "$v8_package_pattern" <<<"$default_tree"; then
  echo "default/macOS parent dependency graph contains V8" >&2
  exit 1
fi

release_tree="$(cargo tree --locked -p oryn --no-default-features \
  --features 'native core-full webdriver-server' -e normal,build)"
if grep -Eq "$v8_package_pattern" <<<"$release_tree"; then
  echo "explicit macOS parent dependency graph contains V8" >&2
  exit 1
fi

worker_tree="$(cargo tree --locked -p oryn-native --features v8-host \
  -e normal,build)"
if ! grep -Eq "$v8_package_pattern" <<<"$worker_tree"; then
  echo "page worker dependency graph does not contain the required V8 host" >&2
  exit 1
fi

echo "default/macOS parent is V8-free and page worker owns V8"
