"""Pure, independently testable semantic evidence metrics for schema v5."""

from __future__ import annotations

from collections import Counter
from typing import Any

FAILURE_ATTRIBUTIONS = {
    "projection_omission",
    "browser_truth_missing",
    "model_or_action_failure",
    "runtime_failure",
    "capability_failure",
    "not_applicable",
}
CAUSAL_RELATIONS = ("direct", "derived", "correlated")


def ratio(numerator: int, denominator: int) -> dict[str, Any]:
    return {
        "numerator": numerator,
        "denominator": denominator,
        "value": numerator / denominator if denominator else None,
    }


def failure_attribution(cell: dict[str, Any]) -> dict[str, Any]:
    if cell.get("success") is True:
        return {"kind": "not_applicable", "checkpoint": None}
    comparison = cell.get("failure_projection_comparison") or {}
    if comparison.get("captured") is True:
        compact = comparison.get("compact_contains_required")
        full = comparison.get("full_contains_required")
        if compact is False and full is True:
            kind = "projection_omission"
        elif full is False:
            kind = "browser_truth_missing"
        elif compact is True and full is True:
            kind = "model_or_action_failure"
        else:
            raise ValueError("projection comparison has inconsistent truth values")
        return {
            "kind": kind,
            "checkpoint": comparison.get("checkpoint", "terminal"),
            "compact_contains_required": compact,
            "full_contains_required": full,
            "compact_utf8_bytes": int(comparison.get("compact_utf8_bytes", 0)),
            "full_utf8_bytes": int(comparison.get("full_utf8_bytes", 0)),
            "compact_sha256": comparison.get("compact_sha256"),
            "full_sha256": comparison.get("full_sha256"),
        }
    turns = cell.get("turns") or []
    diagnostics = " ".join(
        str(value)
        for turn in turns
        for value in (
            turn.get("failure_domain_reason_code"),
            turn.get("action_error"),
            *(turn.get("action_diagnostics") or []),
        )
        if value
    ).lower()
    if "capability" in diagnostics or "unsupported" in diagnostics:
        return {"kind": "capability_failure", "checkpoint": None}
    return {"kind": "runtime_failure", "checkpoint": None}


def observation_sufficiency(cells: list[dict[str, Any]]) -> dict[str, Any]:
    completed_failures = [
        cell
        for cell in cells
        if cell.get("completed") is True and cell.get("success") is not True
    ]
    attributions = [failure_attribution(cell)["kind"] for cell in completed_failures]
    counts = Counter(attributions)
    metric = ratio(counts["projection_omission"], len(completed_failures))
    metric.update(
        {
            "definition": "failed_task_projection_omission_rate",
            "direction": "lower_is_better",
            "attribution_counts": {
                kind: counts[kind]
                for kind in sorted(FAILURE_ATTRIBUTIONS - {"not_applicable"})
            },
        }
    )
    return metric


def causal_metrics(ledger: list[dict[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for relation in CAUSAL_RELATIONS:
        expected = {
            (case.get("case_id"), label["label_id"])
            for case in ledger
            for label in case.get("expected", [])
            if label.get("relation") == relation
        }
        observed = {
            (case.get("case_id"), label["matched_label_id"])
            for case in ledger
            for label in case.get("observed", [])
            if label.get("relation") == relation and label.get("matched_label_id")
        }
        matched = expected & observed
        observed_claims = sum(
            1
            for case in ledger
            for label in case.get("observed", [])
            if label.get("relation") == relation
        )
        result[relation] = {
            "precision": ratio(len(matched), observed_claims),
            "recall": ratio(len(matched), len(expected)),
        }
    result["unattributed_promotions"] = sum(
        1
        for case in ledger
        for label in case.get("observed", [])
        if label.get("source_relation") == "unattributed"
        and label.get("relation") in CAUSAL_RELATIONS
    )
    return result


def compact_cell(cell: dict[str, Any]) -> dict[str, Any]:
    turns = cell.get("turns") or []
    return {
        "cell_id": cell["cell_id"],
        "fingerprint_sha256": cell["fingerprint_sha256"],
        "completed": cell.get("completed") is True,
        "task_id": cell["task_id"],
        "seed": cell["seed"],
        "domain": cell["domain"],
        "model": cell["model"],
        "success": cell.get("success") is True,
        "total_cost_usd": float(cell.get("total_cost_usd") or 0.0),
        "failure_attribution": failure_attribution(cell),
        "action_latency_ms": [
            float(turn.get("oryn_action_latency_ms") or 0.0) for turn in turns
        ],
        "observation_utf8_bytes": [
            int(turn["observation_bytes"])
            for turn in turns
            if turn.get("observation_bytes") is not None
        ],
    }
