"""Tests for the reproducible hosted-model provider contract."""

import sys
from types import SimpleNamespace

import pytest

from intentgym.core.llm import OpenAIProvider


class _Responses:
    def __init__(self):
        self.calls = []

    def create(self, **kwargs):
        self.calls.append(kwargs)
        return SimpleNamespace(
            output_text="Action: observe",
            usage=SimpleNamespace(input_tokens=20, output_tokens=5),
        )


def _provider(**options):
    provider = OpenAIProvider.__new__(OpenAIProvider)
    provider.client = SimpleNamespace(responses=_Responses())
    provider.model = "gpt-5-2025-08-07"
    provider.budget_usd = float(options.pop("budget_usd", 100.0))
    provider.input_cost_per_million = 1.25
    provider.output_cost_per_million = 10.0
    provider.spent_usd = 0.0
    provider.options = options
    return provider


def test_uses_responses_api_and_records_cost():
    provider = _provider(max_output_tokens=32)
    response = provider.complete([{"role": "user", "content": "observe"}])

    assert provider.client.responses.calls[0]["model"] == "gpt-5-2025-08-07"
    assert provider.client.responses.calls[0]["input"][0]["role"] == "user"
    assert response.content == "Action: observe"
    assert response.input_tokens == 20
    assert response.output_tokens == 5
    assert provider.spent_usd == response.cost_usd


def test_rejects_request_before_crossing_hard_cap():
    provider = _provider(budget_usd=0.00001, max_output_tokens=4096)

    with pytest.raises(RuntimeError, match="before request"):
        provider.complete([{"role": "user", "content": "observe"}])

    assert provider.client.responses.calls == []


def test_azure_environment_selects_v1_endpoint_and_deployment(monkeypatch):
    captured = {}

    def client_factory(**kwargs):
        captured.update(kwargs)
        return SimpleNamespace(responses=_Responses())

    monkeypatch.setitem(sys.modules, "openai", SimpleNamespace(OpenAI=client_factory))
    monkeypatch.setenv(
        "AZURE_OPENAI_ENDPOINT", "https://example-resource.services.ai.azure.com/"
    )
    monkeypatch.setenv("AZURE_OPENAI_API_KEY", "test-key")
    monkeypatch.setenv("AZURE_OPENAI_DEPLOYMENT", "gpt-5.6-terra")
    monkeypatch.delenv("OPENAI_BASE_URL", raising=False)
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)

    provider = OpenAIProvider()

    assert captured == {
        "api_key": "test-key",
        "base_url": "https://example-resource.services.ai.azure.com/openai/v1",
    }
    assert provider.model == "gpt-5.6-terra"
    assert provider.context_limit == 1_050_000
