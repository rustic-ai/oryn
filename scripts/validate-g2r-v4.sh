#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
historical="$repo_root/benchmarks/evidence/g2-model-panel.json"
current="$repo_root/benchmarks/evidence/g2r-v4/g2-model-panel-v4.json"
status_file="$repo_root/benchmarks/evidence/g2r-v4/STATUS.json"

if [[ "${1:-}" == "--allow-historical" && ! -f "$current" ]]; then
  jq -e '
    .schema_version == 4 and
    .gate == "G2R_G5A" and
    .status == "open" and
    .current_result == null and
    .historical_evidence.schema_version == 3 and
    .historical_evidence.committed_by == "a41a231d076378026b9e1827f686e9ac8af44ae0" and
    .historical_evidence.cumulative_hosted_spend_usd == 5.209632
  ' "$status_file" >/dev/null
  jq -e '
    .schema_version == 3 and
    .outcome.status == "passed" and
    .outcome.panel_complete == true and
    .panel.completed_runs == 96 and
    .panel.cumulative_hosted_spend_usd == 5.209632
  ' "$historical" >/dev/null
  echo "Schema v4 is open; preserved schema-v3 evidence remains historical."
  exit 0
fi

poetry="$repo_root/artifacts/tools/poetry/bin/poetry"
cd "$repo_root/intentgym"
if [[ -x "$poetry" ]]; then
  "$poetry" run python scripts/validate_g2r_v4.py --evidence "$current"
else
  poetry run python scripts/validate_g2r_v4.py --evidence "$current"
fi
