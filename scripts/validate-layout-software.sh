#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_LAYOUT_ARTIFACT_DIR:-$repo_root/artifacts/layout-v1/software}"
mkdir -p "$artifact_dir"

ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$artifact_dir" "$repo_root/scripts/validate-app-compatibility-software.sh"
"$repo_root/scripts/validate-layout-manifest.sh" >"$artifact_dir/layout-manifest.log" 2>&1
"$repo_root/scripts/validate-layout-platform-subset.sh" >"$artifact_dir/layout-platform-subset.log" 2>&1

echo "layout Rust, V8, Python, IntentGym, scanner, extension, manifest, and WPT checks passed"
