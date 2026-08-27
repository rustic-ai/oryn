#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
poetry="$repo_root/artifacts/tools/poetry/bin/poetry"

cd "$repo_root/intentgym"
if [[ -d "$repo_root/artifacts/g2r/model-panel-v3/turns" ]]; then
  "$poetry" run python scripts/validate_g2r_evidence.py --allow-historical-source
else
  evidence="$repo_root/benchmarks/evidence/g2-model-panel.json"
  jq -e '
    .schema_version == 3 and
    .fingerprint.binary.runtime.native_v8 == true and
    .fingerprint.binary.runtime.build_profile == "release" and
    .fingerprint.models.hosted_provider == "azure_openai" and
    .fingerprint.models.hosted_deployment == "gpt-5.6-terra" and
    .fingerprint.models.local_model == "qwen3:4b" and
    .deterministic.native == {"completed": 24, "passed": 24} and
    .deterministic.chromium == {"completed": 24, "passed": 24} and
    .panel.completed_runs == 96 and
    .panel.azure_native_completed == 24 and
    .panel.azure_native_passed >= 15 and
    .panel.cumulative_hosted_spend_usd <= .panel.hosted_cost_cap_usd and
    .metrics.false_reference_preservation.numerator == 0 and
    ([.criteria[]] | all) and
    .outcome.panel_complete == true and
    .outcome.status == "passed"
  ' "$evidence" >/dev/null
  echo "G2R committed summary is valid; raw-cell recomputation is available in the run workspace"
fi
