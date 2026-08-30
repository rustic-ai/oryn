"""Tests for the fingerprinted, bounded, and truthful semantic-validation model-panel contract."""

from types import SimpleNamespace

from intentgym.core.oryn import OrynInterface
from scripts.run_semantic_model_panel import (
    EXPECTED_QWEN_DIGEST,
    LEGACY_SCHEMA_V4_WORKER_PROTOCOL_VERSION,
    PRIOR_HOSTED_SPEND_V5_USD,
    PRIOR_INVALID_HOSTED_SPEND_USD,
    REQUIRED_WORKER_PROTOCOL_VERSION,
    _aggregate,
    _config,
    _required_worker_protocol,
)


def _args(**overrides):
    values = {
        "hosted_model": "gpt-5.6-terra",
        "hosted_input_cost_per_million": 2.0,
        "hosted_output_cost_per_million": 12.0,
        "ollama_url": "http://127.0.0.1:11434",
        "oryn_binary": "/tmp/oryn",
        "chrome_binary": "/tmp/chrome",
        "chromium_oryn_binary": "/tmp/oryn-chromium",
        "command_timeout": 60.0,
        "miniwob_url": "http://127.0.0.1:8765",
        "task_timeout": 300,
        "local_model_digest": EXPECTED_QWEN_DIGEST,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def _preflight():
    return {
        "fingerprint": {
            "sha256": "f" * 64,
            "source_dirty": False,
            "source_tree_sha256": "s" * 64,
            "binary": {
                "runtime": {
                    "execution_mode": "sandboxed_worker",
                    "sandboxed": True,
                    "worker_protocol_version": LEGACY_SCHEMA_V4_WORKER_PROTOCOL_VERSION,
                }
            },
            "limits": {"queue_capacity": 32},
            "worker_bundle": {"manifest": {"limits": {"queue_capacity": 32}}},
        },
        "deterministic": {
            "native": {"completed": 24, "passed": 24},
            "chromium": {"completed": 24, "passed": 24},
            "differential": [
                {"task_id": "click-button", "seed": index, "first_divergence": None}
                for index in range(24)
            ],
        },
        "deterministic_causal": {
            "cells_sha256": "c" * 64,
            "recall": {"numerator": 48, "denominator": 48, "value": 1.0},
            "precision": {"numerator": 48, "denominator": 96, "value": 0.5},
        },
        "churn": {
            "reference_survival": {"numerator": 4, "denominator": 4, "value": 1.0},
            "false_preservation": {"numerator": 0, "denominator": 4, "value": 0.0},
            "recovery": {"numerator": 2, "denominator": 2, "value": 1.0},
            "worker_replacement": {"status": "passed"},
        },
    }


def _cell(index: int):
    domain = "native" if index % 4 < 2 else "chromium"
    model = "hosted" if index % 2 == 0 else "local"
    return {
        "completed": True,
        "task_id": "click-button",
        "domain": domain,
        "model": model,
        "success": True,
        "total_cost_usd": 0.01 if model == "hosted" else 0.0,
        "turns": [
            {
                "observation_raw": 'Click on the "Ok" button. actions:["click"]',
                "observation_elements": [
                    {"id": 1, "type": "button", "role": "button", "text": "Ok"}
                ],
                "observation_bytes": 52,
                "oryn_action_latency_ms": 1.0,
                "action_command": 'click "Ok"',
                "action_success": True,
                "action_classification": "state_changed",
                "action_effects": [{"kind": "event"}, {"kind": "dom_mutation"}],
            }
        ],
    }


def _v5_preflight():
    value = _preflight()
    value["fingerprint"]["binary"]["runtime"]["worker_protocol_version"] = 5
    value["deterministic_causal"] = {
        "metrics": {
            relation: {
                "precision": {"numerator": 1, "denominator": 1, "value": 1.0},
                "recall": {"numerator": 1, "denominator": 1, "value": 1.0},
            }
            for relation in ("direct", "derived", "correlated")
        }
        | {"unattributed_promotions": 0},
        "ledger": [
            {
                "case_id": "controlled",
                "expected": [
                    {
                        "label_id": "dispatch",
                        "relation": "direct",
                        "kind": "event_dispatched",
                    }
                ],
                "observed": [
                    {
                        "sequence": 1,
                        "action_id": 1,
                        "relation": "direct",
                        "source_relation": "direct",
                        "kind": "event_dispatched",
                        "matched_label_id": "dispatch",
                    }
                ],
                "passed": True,
            }
        ],
        "recall": {"numerator": 1, "denominator": 1, "value": 1.0},
        "precision": {"numerator": 1, "denominator": 1, "value": 1.0},
    }
    value["challenge"] = {
        "evidence": {"status": "passed"},
        "metrics": {
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
    }
    value["churn"]["reference_survival"] = {
        "numerator": 8,
        "denominator": 8,
        "value": 1.0,
    }
    value["churn"]["invalidation"] = {
        "numerator": 8,
        "denominator": 8,
        "value": 1.0,
    }
    value["churn"]["false_preservation"] = {
        "numerator": 0,
        "denominator": 8,
        "value": 0.0,
    }
    return value


def _v5_cell(index: int):
    value = _cell(index)
    value.update(
        {
            "cell_id": f"cell-{index}",
            "fingerprint_sha256": "f" * 64,
            "seed": (17, 42, 73)[index % 3],
        }
    )
    return value


def test_local_panel_requests_are_bounded_and_seeded():
    config = _config(_args(), "click-button", 17, "native", "local", 100.0)

    assert config.llm.model == "ollama/qwen3:4b"
    assert config.llm.options["max_tokens"] == 256
    assert config.llm.options["think"] is False
    assert config.benchmark.options["seed"] == 17


def test_current_panel_uses_the_streaming_worker_protocol():
    assert REQUIRED_WORKER_PROTOCOL_VERSION == 5
    assert _required_worker_protocol(5) == 5
    assert _required_worker_protocol(4) == 4


def test_schema_v5_emits_corrected_metrics_and_exact_identity_gate():
    result = _aggregate(
        _args(schema_version=5, prior_hosted_spend=PRIOR_HOSTED_SPEND_V5_USD),
        _v5_preflight(),
        [_v5_cell(index) for index in range(96)],
        "2026-08-29T00:00:00Z",
    )
    assert result["schema_version"] == 5
    assert result["metrics"]["observation_sufficiency"]["direction"] == (
        "lower_is_better"
    )
    assert result["metrics"]["causal_attribution"]["direct"]["recall"]["value"] == 1
    assert result["metrics"]["challenge_detection"]["confirmed_precision"]["value"] == 1
    assert result["criteria"]["exact_identity_reference_survival_8_of_8"] is True


def test_completeness_does_not_override_quality_criteria():
    cells = [_cell(index) for index in range(96)]
    for cell in cells:
        if cell["domain"] == "native" and cell["model"] == "hosted":
            cell["success"] = False

    result = _aggregate(
        _args(),
        _preflight(),
        cells,
        "2026-08-26T00:00:00Z",
    )

    assert result["outcome"]["panel_complete"] is True
    assert result["outcome"]["status"] == "failed"
    assert result["criteria"]["azure_native_at_least_15_of_24"] is False


def test_prior_invalid_spend_is_included_in_cumulative_cap():
    result = _aggregate(
        _args(),
        _preflight(),
        [_cell(index) for index in range(96)],
        "2026-08-26T00:00:00Z",
    )

    assert result["panel"]["prior_hosted_spend_usd"] == PRIOR_INVALID_HOSTED_SPEND_USD
    assert (
        result["panel"]["cumulative_hosted_spend_usd"] > PRIOR_INVALID_HOSTED_SPEND_USD
    )
    assert result["metrics"]["observation_sufficiency"] == {
        "numerator": 0,
        "denominator": 0,
        "value": None,
    }
    assert result["metrics"]["causal_recall"]["value"] == 1.0


def test_observation_sufficiency_measures_projection_caused_failures_only():
    cells = [_cell(index) for index in range(96)]
    cells[0]["success"] = False
    cells[0]["failure_projection_comparison"] = {
        "captured": True,
        "compact_contains_required": False,
        "full_contains_required": True,
    }
    cells[1]["success"] = False
    cells[1]["failure_projection_comparison"] = {
        "captured": True,
        "compact_contains_required": True,
        "full_contains_required": True,
    }

    result = _aggregate(
        _args(),
        _preflight(),
        cells,
        "2026-08-26T00:00:00Z",
    )

    assert result["metrics"]["observation_sufficiency"] == {
        "numerator": 1,
        "denominator": 2,
        "value": 0.5,
    }


def test_native_trace_slices_advance_and_reset_with_the_existing_oil_command():
    class FakeClient:
        def __init__(self):
            self.responses = [
                '{"kind":"trace","events":[{"sequence":1}]}',
                '{"kind":"trace","events":[{"sequence":1},{"sequence":2}]}',
                '{"kind":"trace","events":[{"sequence":3}]}',
            ]

        def execute(self, command):
            assert command == "trace stop"
            return self.responses.pop(0)

    oryn = OrynInterface(use_mock=True)
    oryn._client = FakeClient()

    assert oryn._take_native_trace_slice() == [{"sequence": 1}]
    assert oryn._take_native_trace_slice() == [{"sequence": 2}]
    assert oryn._take_native_trace_slice() == [{"sequence": 3}]
