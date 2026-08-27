#!/bin/sh
set -eu

evidence="${1:-benchmarks/evidence/g2-controlled-native.json}"

jq -e '
  .schema_version == 1 and
  .gate == "G2-controlled-native" and
  .execution_domain == "native" and
  (.harness | length) == 12 and
  (.framework | length) == 6 and
  ([.harness[], .framework[]] | all(.status == "passed")) and
  .summary.required_tasks == 18 and
  .summary.passed_tasks == 18 and
  .summary.failed_tasks == 0 and
  .summary.false_semantic_reference_preservations == 0 and
  .reference_churn.consequential_false_preservations == 0 and
  .summary.status == "passed"
' "$evidence" >/dev/null

echo "G2 controlled native evidence is valid: 18/18 passed, zero false reference preservations"
