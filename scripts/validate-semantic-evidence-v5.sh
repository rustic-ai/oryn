#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root/intentgym"
python3 scripts/validate_semantic_evidence_v5.py "$@"
