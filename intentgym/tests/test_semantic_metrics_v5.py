from scripts.semantic_metrics_v5 import (
    causal_metrics,
    failure_attribution,
    observation_sufficiency,
)


def cell(success=False, compact=False, full=True):
    return {
        "completed": True,
        "success": success,
        "failure_projection_comparison": {
            "captured": not success,
            "compact_contains_required": compact,
            "full_contains_required": full,
        },
    }


def test_observation_metric_uses_only_failed_tasks_and_lower_is_better():
    metric = observation_sufficiency(
        [
            cell(success=True),
            cell(compact=False, full=True),
            cell(compact=True, full=True),
        ]
    )
    assert metric["numerator"] == 1
    assert metric["denominator"] == 2
    assert metric["value"] == 0.5
    assert metric["direction"] == "lower_is_better"


def test_browser_truth_missing_is_not_a_projection_omission():
    assert (
        failure_attribution(cell(compact=False, full=False))["kind"]
        == "browser_truth_missing"
    )


def test_causal_classes_are_scored_independently():
    ledger = [
        {
            "expected": [
                {"label_id": "d", "relation": "direct"},
                {"label_id": "m", "relation": "derived"},
                {"label_id": "c", "relation": "correlated"},
            ],
            "observed": [
                {"matched_label_id": "d", "relation": "direct"},
                {"matched_label_id": "m", "relation": "derived"},
                {"matched_label_id": "c", "relation": "correlated"},
            ],
        }
    ]
    metrics = causal_metrics(ledger)
    assert all(
        metrics[kind]["precision"]["value"] == 1
        for kind in ("direct", "derived", "correlated")
    )
    assert all(
        metrics[kind]["recall"]["value"] == 1
        for kind in ("direct", "derived", "correlated")
    )


def test_unmatched_causal_claim_reduces_only_its_class_precision():
    ledger = [
        {
            "case_id": "one",
            "expected": [{"label_id": "dispatch", "relation": "direct"}],
            "observed": [
                {"matched_label_id": "dispatch", "relation": "direct"},
                {"matched_label_id": None, "relation": "direct"},
            ],
        }
    ]
    metrics = causal_metrics(ledger)
    assert metrics["direct"]["precision"]["value"] == 0.5
    assert metrics["direct"]["recall"]["value"] == 1.0
    assert metrics["derived"]["precision"]["value"] is None


def test_duplicate_label_ids_in_distinct_cases_are_not_collapsed():
    ledger = [
        {
            "case_id": "one",
            "expected": [{"label_id": "dispatch", "relation": "direct"}],
            "observed": [{"matched_label_id": "dispatch", "relation": "direct"}],
        },
        {
            "case_id": "two",
            "expected": [{"label_id": "dispatch", "relation": "direct"}],
            "observed": [],
        },
    ]
    metrics = causal_metrics(ledger)
    assert metrics["direct"]["recall"] == {
        "numerator": 1,
        "denominator": 2,
        "value": 0.5,
    }


def test_unattributed_work_cannot_be_promoted_silently():
    ledger = [
        {
            "case_id": "one",
            "expected": [],
            "observed": [
                {
                    "matched_label_id": None,
                    "relation": "correlated",
                    "source_relation": "unattributed",
                }
            ],
        }
    ]
    assert causal_metrics(ledger)["unattributed_promotions"] == 1
