#!/usr/bin/env python3
"""Recompute the compact schema-v4 G2R/G5A gate ledger."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
import subprocess
from pathlib import Path
from typing import Any

import jsonschema

TASKS = {
    "click-button",
    "click-link",
    "click-option",
    "enter-text",
    "focus-text",
    "choose-date",
    "login-user",
    "search-engine",
}
SEEDS = {17, 42, 73}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def canonical_hash(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, separators=(",", ":"), sort_keys=True).encode()
    return hashlib.sha256(encoded).hexdigest()


def ratio(numerator: int, denominator: int) -> dict[str, Any]:
    return {
        "numerator": numerator,
        "denominator": denominator,
        "value": numerator / denominator if denominator else None,
    }


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))]


def distribution(values: list[float]) -> dict[str, Any]:
    return {
        "p50": statistics.median(values) if values else None,
        "p95": percentile(values, 0.95),
        "samples": len(values),
    }


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def files_sha256(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.name.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def main() -> int:
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--evidence",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/g2-model-panel-v4.json",
    )
    parser.add_argument(
        "--schema",
        type=Path,
        default=repo_root / "benchmarks/schema/result-v4.schema.json",
    )
    parser.add_argument(
        "--churn",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/g2r-churn.json",
    )
    parser.add_argument(
        "--deterministic",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/deterministic/aggregate.json",
    )
    args = parser.parse_args()

    evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
    schema = json.loads(args.schema.read_text(encoding="utf-8"))
    jsonschema.validate(evidence, schema)
    require(evidence["schema_version"] == 4, "evidence is not schema v4")

    fingerprint = dict(evidence["fingerprint"])
    claimed_fingerprint = fingerprint.pop("sha256")
    require(
        canonical_hash(fingerprint) == claimed_fingerprint,
        "fingerprint hash is not canonical",
    )
    source_tree_sha256 = subprocess.check_output(
        [str(repo_root / "scripts/source-tree-hash.sh")], text=True
    ).strip()
    require(
        fingerprint["source_tree_sha256"] == source_tree_sha256,
        "evidence does not match the tracked source tree",
    )
    runtime = fingerprint["binary"]["runtime"]
    worker = fingerprint["worker_bundle"]
    require(
        runtime["worker_binary_sha256"] == worker["sha256"],
        "runtime and signed-worker hashes differ",
    )
    require(
        runtime["limits"] == fingerprint["limits"],
        "runtime and fingerprint limits differ",
    )
    require(
        runtime["policy_digest"] == fingerprint["policy"]["digest"],
        "runtime and policy digests differ",
    )
    require(
        worker["manifest"]["source_tree_sha256"] == source_tree_sha256,
        "signed manifest does not match the tracked source tree",
    )
    require(
        worker["manifest"]["limits"] == fingerprint["limits"],
        "signed manifest and runtime limits differ",
    )
    deterministic = json.loads(args.deterministic.read_text(encoding="utf-8"))
    require(
        evidence["deterministic"] == deterministic,
        "embedded deterministic result differs from its aggregate",
    )
    require(
        file_sha256(args.deterministic) == fingerprint["deterministic_evidence_sha256"],
        "deterministic aggregate hash differs",
    )
    deterministic_cells = sorted(args.deterministic.parent.glob("*-native-oracle.json"))
    require(
        len(deterministic_cells) == 24, "native deterministic raw set is incomplete"
    )
    require(
        files_sha256(deterministic_cells) == fingerprint["deterministic_cells_sha256"],
        "deterministic native cell hash differs",
    )

    ledger = evidence["cell_ledger"]
    require(len(ledger) == 96, f"expected 96 compact cells, found {len(ledger)}")
    require(
        len({cell["cell_id"] for cell in ledger}) == 96,
        "cell IDs are not unique",
    )
    require(
        all(cell["fingerprint_sha256"] == claimed_fingerprint for cell in ledger),
        "a cell fingerprint differs from the aggregate",
    )
    expected_matrix = {
        (task, seed, domain, model)
        for task in TASKS
        for seed in SEEDS
        for domain in ("native", "chromium")
        for model in ("hosted", "local")
    }
    actual_matrix = {
        (cell["task_id"], cell["seed"], cell["domain"], cell["model"])
        for cell in ledger
    }
    require(actual_matrix == expected_matrix, "96-cell matrix is not exact")

    action_latency = [value for cell in ledger for value in cell["action_latency_ms"]]
    observation_bytes = [
        value for cell in ledger for value in cell["observation_utf8_bytes"]
    ]
    causal = evidence["causal_ledger"]
    require(len(causal) == 24, "causal ledger is not 24 cells")
    causal_expected = sum(cell["expected_effects"] for cell in causal)
    causal_observed_expected = sum(cell["observed_expected_effects"] for cell in causal)
    causal_observed = sum(cell["observed_effects"] for cell in causal)
    churn = json.loads(args.churn.read_text(encoding="utf-8"))
    require(churn["schema_version"] == 4, "churn evidence is not schema v4")
    require(churn["status"] == "passed", "churn evidence failed")
    require(
        file_sha256(args.churn) == fingerprint["churn_evidence_sha256"],
        "churn evidence hash differs",
    )

    metrics = {
        "task_success": ratio(sum(cell["success"] for cell in ledger), 96),
        "observation_sufficiency": ratio(
            sum(cell["observation_sufficient"] for cell in ledger), 96
        ),
        "recovery": churn["recovery"],
        "reference_survival": churn["reference_survival"],
        "false_reference_preservation": churn["false_preservation"],
        "causal_recall": ratio(causal_observed_expected, causal_expected),
        "causal_precision": ratio(causal_observed_expected, causal_observed),
        "action_latency_ms": distribution(action_latency),
        "observation_utf8_bytes": distribution(observation_bytes),
    }
    require(metrics == evidence["metrics"], "metrics do not recompute from ledgers")
    panel = evidence["panel"]
    hosted_spend = round(
        sum(cell["total_cost_usd"] for cell in ledger if cell["model"] == "hosted"),
        6,
    )
    require(panel["completed_runs"] == 96, "panel is incomplete")
    require(
        panel["new_hosted_spend_usd"] == hosted_spend,
        "hosted spend does not recompute",
    )
    require(
        panel["cumulative_hosted_spend_usd"]
        == round(panel["prior_hosted_spend_usd"] + hosted_spend, 6),
        "cumulative spend does not recompute",
    )
    azure_native = [
        cell
        for cell in ledger
        if cell["domain"] == "native" and cell["model"] == "hosted"
    ]
    require(len(azure_native) == 24, "Azure/native matrix is incomplete")
    require(
        sum(cell["success"] for cell in azure_native) >= 15,
        "Azure/native floor failed",
    )
    require(
        all(cell["classifications_explained"] for cell in ledger),
        "one or more action classifications is unexplained",
    )
    require(
        churn["false_preservation"]["numerator"] == 0,
        "false semantic-reference preservation is nonzero",
    )
    require(
        churn["worker_replacement"]["status"] == "passed",
        "worker replacement suite failed",
    )
    deterministic = evidence["deterministic"]
    require(
        deterministic["native"] == {"completed": 24, "passed": 24},
        "native deterministic floor failed",
    )
    require(
        deterministic["chromium"] == {"completed": 24, "passed": 24},
        "Chromium deterministic comparison failed",
    )
    require(
        len(deterministic["differential"]) == 24
        and all(
            item.get("first_divergence") is None
            for item in deterministic["differential"]
        ),
        "deterministic differential contains a divergence",
    )
    require(all(evidence["criteria"].values()), "a gate criterion is false")
    require(evidence["outcome"]["panel_complete"] is True, "panel_complete is false")
    require(evidence["outcome"]["status"] == "passed", "G2R/G5A outcome failed")
    print("Schema-v4 G2R/G5A evidence recomputed from compact ledgers")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
