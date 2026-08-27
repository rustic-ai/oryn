#!/usr/bin/env python3
"""Recompute G2R aggregate metrics from fingerprint-matched cell artifacts."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import jsonschema

try:
    from scripts.run_g2_model_panel import (
        EXPECTED_QWEN_DIGEST,
        HOSTED_CAP_USD,
        HOSTED_MODEL,
        _canonical_hash,
        _deterministic_causal,
        _harness_hash,
        _measured_metrics,
        _sha256,
    )
except ModuleNotFoundError:
    from run_g2_model_panel import (  # type: ignore[no-redef]
        EXPECTED_QWEN_DIGEST,
        HOSTED_CAP_USD,
        HOSTED_MODEL,
        _canonical_hash,
        _deterministic_causal,
        _harness_hash,
        _measured_metrics,
        _sha256,
    )

HISTORICAL_PRIOR_HOSTED_SPEND_USD = 3.190558


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def main() -> int:
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--evidence",
        type=Path,
        default=repo_root / "benchmarks/evidence/g2-model-panel.json",
    )
    parser.add_argument(
        "--cells", type=Path, default=repo_root / "artifacts/g2r/model-panel-v3"
    )
    parser.add_argument(
        "--churn",
        type=Path,
        default=repo_root / "benchmarks/evidence/g2r-churn.json",
    )
    parser.add_argument(
        "--deterministic",
        type=Path,
        default=repo_root / "artifacts/g2r/deterministic/aggregate.json",
    )
    parser.add_argument(
        "--schema",
        type=Path,
        default=repo_root / "benchmarks/schema/result-v3.schema.json",
    )
    parser.add_argument("--allow-historical-source", action="store_true")
    args = parser.parse_args()

    evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
    schema = json.loads(args.schema.read_text(encoding="utf-8"))
    jsonschema.validate(evidence, schema)
    churn = json.loads(args.churn.read_text(encoding="utf-8"))
    deterministic = json.loads(args.deterministic.read_text(encoding="utf-8"))
    deterministic_causal = _deterministic_causal(args.deterministic, deterministic)
    cells = [
        json.loads(path.read_text(encoding="utf-8"))
        for path in sorted(args.cells.glob("*.json"))
        if path.name != "aggregate.json"
    ]
    require(evidence.get("schema_version") == 3, "evidence is not schema v3")
    require(len(cells) == 96, f"expected 96 cell artifacts, found {len(cells)}")
    require(
        all(cell.get("completed") is True for cell in cells), "incomplete cell artifact"
    )

    fingerprint = dict(evidence["fingerprint"])
    claimed_fingerprint = fingerprint.pop("sha256")
    require(
        _canonical_hash(fingerprint) == claimed_fingerprint,
        "aggregate fingerprint hash is not canonical",
    )
    if not args.allow_historical_source:
        require(
            fingerprint["harness_sha256"] == _harness_hash(repo_root),
            "current SDK/IntentGym harness differs from the recorded fingerprint",
        )
    require(
        all(cell.get("fingerprint_sha256") == claimed_fingerprint for cell in cells),
        "a cell was produced by a different runtime/fixture/model fingerprint",
    )
    require(
        evidence["deterministic"] == deterministic,
        "embedded deterministic evidence differs from its source aggregate",
    )
    require(
        fingerprint["deterministic_evidence_sha256"] == _sha256(args.deterministic),
        "deterministic aggregate hash does not match the fingerprint",
    )
    require(
        fingerprint["deterministic_cells_sha256"]
        == deterministic_causal["cells_sha256"],
        "deterministic cell-set hash does not match the fingerprint",
    )
    require(
        fingerprint["churn_evidence_sha256"] == _sha256(args.churn),
        "churn/recovery evidence hash does not match the fingerprint",
    )

    for cell in cells:
        turns = cell.get("turns") or []
        trace_path = args.cells / "turns" / f"{cell['cell_id']}.jsonl"
        require(trace_path.is_file(), f"missing per-turn JSONL: {trace_path}")
        traced_turns = [
            json.loads(line) for line in trace_path.read_text().splitlines()
        ]
        require(traced_turns == turns, f"per-turn JSONL mismatch: {cell['cell_id']}")
        for turn in turns:
            require(
                isinstance(turn.get("observation_elements"), list),
                f"typed observation elements missing: {cell['cell_id']}",
            )
            require(
                isinstance(turn.get("trace_slice"), list),
                f"trace slice missing: {cell['cell_id']}",
            )
            raw = turn.get("observation_raw")
            if raw is not None:
                require(
                    turn.get("observation_bytes") == len(raw.encode("utf-8")),
                    f"observation byte count was not measured: {cell['cell_id']}",
                )
            if turn.get("action_classification") in {"failed", "unsupported"}:
                require(
                    bool(
                        turn.get("action_error")
                        or turn.get("action_diagnostics")
                        or turn.get("failure_domain_reason_code")
                    ),
                    f"unexplained action classification: {cell['cell_id']}",
                )

    recomputed_metrics = _measured_metrics(cells, churn, deterministic_causal)
    require(
        recomputed_metrics == evidence["metrics"], "aggregate metrics do not recompute"
    )
    new_spend = round(
        sum(
            float(cell.get("total_cost_usd") or 0.0)
            for cell in cells
            if cell.get("model") == "hosted"
        ),
        6,
    )
    panel = evidence["panel"]
    require(
        panel["expected_runs"] == panel["completed_runs"] == 96, "panel is incomplete"
    )
    require(
        panel["prior_invalid_hosted_spend_usd"] == HISTORICAL_PRIOR_HOSTED_SPEND_USD,
        "prior spend omitted",
    )
    require(
        panel["new_hosted_spend_usd"] == new_spend, "hosted spend does not recompute"
    )
    require(
        panel["cumulative_hosted_spend_usd"]
        == round(HISTORICAL_PRIOR_HOSTED_SPEND_USD + new_spend, 6),
        "cumulative hosted spend does not recompute",
    )
    require(
        panel["cumulative_hosted_spend_usd"] <= HOSTED_CAP_USD, "hosted cap exceeded"
    )
    require(panel["hosted_provider"] == "azure_openai", "hosted provider is not Azure")
    require(panel["hosted_model"] == HOSTED_MODEL, "Azure deployment changed")
    require(panel["local_model"] == "qwen3:4b", "local model changed")
    require(panel["local_model_digest"] == EXPECTED_QWEN_DIGEST, "Qwen digest changed")

    azure_native = [
        cell
        for cell in cells
        if cell["domain"] == "native" and cell["model"] == "hosted"
    ]
    require(len(azure_native) == 24, "Azure-native matrix is incomplete")
    require(
        sum(bool(cell["success"]) for cell in azure_native) >= 15,
        "Azure-native floor failed",
    )
    require(
        evidence["deterministic"]["native"] == {"completed": 24, "passed": 24},
        "native deterministic floor failed",
    )
    require(
        evidence["deterministic"]["chromium"] == {"completed": 24, "passed": 24},
        "Chromium comparison is incomplete",
    )
    require(
        len(evidence["deterministic"].get("differential") or []) == 24
        and all(
            item.get("first_divergence") is None
            for item in evidence["deterministic"].get("differential") or []
        ),
        "deterministic differential contains a divergence",
    )
    require(all(evidence["criteria"].values()), "one or more G2R criteria are false")
    require(evidence["outcome"]["panel_complete"] is True, "panel_complete is false")
    require(evidence["outcome"]["status"] == "passed", "G2R outcome is not passed")
    print("G2R evidence recomputed successfully from 96 fingerprint-matched cells")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
