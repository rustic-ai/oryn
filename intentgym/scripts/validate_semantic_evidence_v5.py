#!/usr/bin/env python3
"""Validate schema-v5 semantic evidence from raw, fingerprint-matched artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

try:
    import jsonschema
except (
    ModuleNotFoundError
):  # Lightweight software checks can run before dev deps are installed.
    jsonschema = None

try:
    from semantic_metrics_v5 import (
        causal_metrics,
        compact_cell,
        observation_sufficiency,
    )
except ModuleNotFoundError:
    from scripts.semantic_metrics_v5 import (  # type: ignore[no-redef]
        causal_metrics,
        compact_cell,
        observation_sufficiency,
    )

PRIOR_HOSTED_SPEND_USD = 6.362286
HOSTED_CAP_USD = 100.0


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_controlled(
    repo: Path, rerun: bool
) -> tuple[dict[str, Any], dict[str, Any]]:
    causal_path = (
        repo / "benchmarks/evidence/semantic-validation-v5/causal-controlled.json"
    )
    challenge_path = (
        repo / "benchmarks/evidence/semantic-validation-v5/challenge-controlled.json"
    )
    causal = load(causal_path)
    challenge = load(challenge_path)
    require(causal.get("status") == "passed", "controlled causal evidence failed")
    causal_result = causal_metrics(causal["ledger"])
    for relation in ("direct", "derived", "correlated"):
        require(
            causal_result[relation]["precision"]["value"] == 1.0,
            f"{relation} precision failed",
        )
        require(
            causal_result[relation]["recall"]["value"] == 1.0,
            f"{relation} recall failed",
        )
    require(
        causal_result["unattributed_promotions"] == 0, "unattributed work was promoted"
    )
    require(challenge.get("status") == "passed", "challenge detector evidence failed")
    require(
        challenge["confirmed_precision"] == {"numerator": 6, "denominator": 6},
        "challenge precision failed",
    )
    require(
        challenge["confirmed_recall"] == {"numerator": 6, "denominator": 6},
        "challenge recall failed",
    )
    native_failures = {item["id"]: item["observed"] for item in challenge["ledger"]}
    require(
        native_failures.get("native-parser-failure") == "not_detected",
        "native failure was labelled a challenge",
    )
    if rerun:
        challenge_run = json.loads(
            subprocess.check_output(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "oryn-native",
                    "--example",
                    "challenge_detection_evidence",
                    "--all-features",
                ],
                cwd=repo,
                text=True,
            )
        )
        require(
            challenge_run["ledger"] == challenge["ledger"],
            "challenge evidence does not reproduce",
        )
        causal_run = json.loads(
            subprocess.check_output(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "oryn-native",
                    "--example",
                    "semantic_causal_evidence",
                    "--features",
                    "v8-host in-process-probe",
                ],
                cwd=repo,
                text=True,
            )
        )
        require(
            causal_metrics(causal_run["ledger"]) == causal_result,
            "causal evidence does not reproduce",
        )
    return causal, challenge


def validate_canary_correction(repo: Path) -> None:
    path = repo / "benchmarks/evidence/semantic-public-canaries-v2.json"
    value = load(path)
    if jsonschema is not None:
        jsonschema.validate(
            value, load(repo / "benchmarks/schema/public-canaries-v2.schema.json")
        )
    require(
        value.get("schema_version") == 2 and len(value.get("results", [])) == 4,
        "corrected canary evidence is malformed",
    )
    generated = json.loads(
        subprocess.check_output(
            [sys.executable, str(repo / "scripts/reclassify-public-canaries-v2.py")],
            cwd=repo,
            text=True,
        )
    )
    require(generated == value, "corrected canary evidence does not reproduce")
    by_id = {item["task_id"]: item for item in value["results"]}
    for task_id in ("canary-wikipedia-search", "canary-crates-search"):
        require(
            by_id[task_id]["challenge"]["disposition"] == "not_assessed",
            f"{task_id} historical challenge state was fabricated",
        )
        require(
            by_id[task_id]["failure_domain"] == "native_runtime",
            f"{task_id} failure domain is incorrect",
        )


def main() -> int:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--software-only", action="store_true")
    parser.add_argument("--rerun-controlled", action="store_true")
    parser.add_argument(
        "--evidence",
        type=Path,
        default=repo / "artifacts/semantic-validation-v5/model-panel-v5.json",
    )
    parser.add_argument(
        "--cells",
        type=Path,
        default=repo / "artifacts/semantic-validation-v5/model-panel",
    )
    parser.add_argument(
        "--churn",
        type=Path,
        default=repo / "artifacts/semantic-validation-v5/reference-churn.json",
    )
    args = parser.parse_args()
    causal, challenge = validate_controlled(repo, args.rerun_controlled)
    validate_canary_correction(repo)
    if args.software_only:
        print(
            "Schema-v5 semantic metric, causal, and challenge software evidence passed"
        )
        return 0

    evidence = load(args.evidence)
    require(
        jsonschema is not None, "jsonschema is required for final schema-v5 attestation"
    )
    jsonschema.validate(
        evidence, load(repo / "benchmarks/schema/result-v5.schema.json")
    )
    cells = [load(path) for path in sorted(args.cells.glob("*.json"))]
    require(len(cells) == 96, f"expected 96 raw cells, found {len(cells)}")
    fingerprint = evidence["fingerprint"]
    require(
        all(cell.get("fingerprint_sha256") == fingerprint["sha256"] for cell in cells),
        "cell fingerprint mismatch",
    )
    compact = [compact_cell(cell) for cell in cells]
    require(
        compact == evidence["cell_ledger"], "compact cell ledger does not recompute"
    )
    metrics = evidence["metrics"]
    require(
        observation_sufficiency(cells) == metrics["observation_sufficiency"],
        "observation metric does not recompute",
    )
    require(
        causal["ledger"] == evidence["causal_ledger"],
        "causal ledger differs from controlled evidence",
    )
    require(
        causal_metrics(causal["ledger"]) == metrics["causal_attribution"],
        "causal metrics do not recompute",
    )
    require(
        metrics["challenge_detection"]
        == {
            "confirmed_precision": {
                "numerator": 6,
                "denominator": 6,
                "value": 1.0,
            },
            "confirmed_recall": {
                "numerator": 6,
                "denominator": 6,
                "value": 1.0,
            },
        },
        "challenge metrics do not recompute",
    )
    require(
        fingerprint.get("causal_evidence_sha256")
        == sha256(
            repo / "benchmarks/evidence/semantic-validation-v5/causal-controlled.json"
        ),
        "causal evidence fingerprint mismatch",
    )
    require(
        fingerprint.get("challenge_evidence_sha256")
        == sha256(
            repo
            / "benchmarks/evidence/semantic-validation-v5/challenge-controlled.json"
        ),
        "challenge evidence fingerprint mismatch",
    )
    churn = load(args.churn)
    require(
        churn.get("schema_version") == 5 and churn.get("status") == "passed",
        "schema-v5 churn failed",
    )
    require(
        churn["reference_survival"] == {"numerator": 8, "denominator": 8, "value": 1.0},
        "reference survival is incomplete",
    )
    require(
        churn["invalidation"] == {"numerator": 8, "denominator": 8, "value": 1.0},
        "reference invalidation is incomplete",
    )
    require(
        churn["false_preservation"]["numerator"] == 0, "false preservation is nonzero"
    )
    panel = evidence["panel"]
    hosted_spend = round(
        sum(
            float(cell.get("total_cost_usd") or 0)
            for cell in cells
            if cell.get("model") == "hosted"
        ),
        6,
    )
    require(
        panel["prior_hosted_spend_usd"] == PRIOR_HOSTED_SPEND_USD,
        "prior hosted spend changed",
    )
    require(
        panel["new_hosted_spend_usd"] == hosted_spend, "hosted spend does not recompute"
    )
    require(
        panel["cumulative_hosted_spend_usd"]
        == round(PRIOR_HOSTED_SPEND_USD + hosted_spend, 6),
        "cumulative hosted spend does not recompute",
    )
    require(
        panel["cumulative_hosted_spend_usd"] <= HOSTED_CAP_USD, "hosted budget exceeded"
    )
    require(
        panel["hosted_provider"] == "azure_openai"
        and panel["hosted_model"] == "gpt-5.6-terra",
        "hosted deployment changed",
    )
    require(panel["local_model"] == "qwen3:4b", "local model changed")
    require(
        panel["expected_runs"] == panel["completed_runs"] == 96,
        "model panel is incomplete",
    )
    require(all(evidence["criteria"].values()), "one or more schema-v5 criteria failed")
    require(evidence["outcome"]["status"] == "passed", "schema-v5 outcome did not pass")
    require(challenge["status"] == "passed", "challenge evidence failed")
    print("Schema-v5 semantic evidence recomputed from raw artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
