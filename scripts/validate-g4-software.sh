#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_G4_ARTIFACT_DIR:-$repo_root/artifacts/g4-v1/software}"
mkdir -p "$artifact_dir"

ORYN_G3_ARTIFACT_DIR="$artifact_dir" "$repo_root/scripts/validate-g3-software.sh"
"$repo_root/scripts/validate-g4-manifest.sh" >"$artifact_dir/g4-manifest.log" 2>&1
"$repo_root/scripts/validate-g4-wpt.sh" >"$artifact_dir/g4-wpt.log" 2>&1

echo "G4 Rust, V8, Python, IntentGym, scanner, extension, manifest, and WPT checks passed"
