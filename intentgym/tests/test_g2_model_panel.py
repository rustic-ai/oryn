"""Tests for the fingerprinted, bounded, and truthful G2R model-panel contract."""

from types import SimpleNamespace

from intentgym.core.oryn import OrynInterface
from scripts.run_g2_model_panel import (
    EXPECTED_QWEN_DIGEST,
    PRIOR_INVALID_HOSTED_SPEND_USD,
    _aggregate,
    _config,
)


def _args(**overrides):
    values = {
        "hosted_model": "gpt-5.6-terra",
        "hosted_input_cost_per_million": 2.0,
        "hosted_output_cost_per_million": 12.0,
        "ollama_url": "http://127.0.0.1:11434",
        "oryn_binary": "/tmp/oryn",
        "chrome_binary": "/tmp/chrome",
        "command_timeout": 60.0,
        "miniwob_url": "http://127.0.0.1:8765",
        "task_timeout": 300,
        "local_model_digest": EXPECTED_QWEN_DIGEST,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def _preflight():
    return {
        "fingerprint": {"sha256": "f" * 64},
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


def test_local_panel_requests_are_bounded_and_seeded():
    config = _config(_args(), "click-button", 17, "native", "local", 100.0)

    assert config.llm.model == "ollama/qwen3:4b"
    assert config.llm.options["max_tokens"] == 256
    assert config.llm.options["think"] is False
    assert config.benchmark.options["seed"] == 17


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
    assert result["metrics"]["observation_sufficiency"]["denominator"] == 96
    assert result["metrics"]["causal_recall"]["value"] == 1.0


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
