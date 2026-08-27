import os
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Any, Dict, List


@dataclass
class LLMResponse:
    """Standardized LLM response."""

    content: str
    input_tokens: int
    output_tokens: int
    latency_ms: float
    cost_usd: float


class LLMProvider(ABC):
    """Abstract base class for LLM providers."""

    @abstractmethod
    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        """Generate completion from message history."""
        pass

    @abstractmethod
    def count_tokens(self, text: str) -> int:
        """Count tokens in text."""
        pass

    @property
    @abstractmethod
    def context_limit(self) -> int:
        """Maximum context window size."""
        pass


class OpenAIProvider(LLMProvider):
    """OpenAI Responses API provider with a cumulative hard cost ceiling."""

    def __init__(self, model: str | None = None, **options):
        import openai

        azure_endpoint = os.environ.get("AZURE_OPENAI_ENDPOINT")
        azure_api_key = os.environ.get("AZURE_OPENAI_API_KEY")
        base_url = os.environ.get("OPENAI_BASE_URL")
        api_key = os.environ.get("OPENAI_API_KEY")
        if azure_endpoint:
            base_url = base_url or f"{azure_endpoint.rstrip('/')}/openai/v1"
            api_key = azure_api_key or api_key

        client_options = {}
        if base_url:
            client_options["base_url"] = base_url.rstrip("/")
        if api_key:
            client_options["api_key"] = api_key
        self.client = openai.OpenAI(**client_options)
        self.model = model or os.environ.get("AZURE_OPENAI_DEPLOYMENT", "gpt-5.6-terra")
        self.budget_usd = float(options.pop("budget_usd", 100.0))
        self.input_cost_per_million = float(options.pop("input_cost_per_million", 1.25))
        self.output_cost_per_million = float(
            options.pop("output_cost_per_million", 10.0)
        )
        self.spent_usd = 0.0
        self.options = options

    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        start = time.time()
        typed_messages: Any = messages
        estimated_input = sum(
            self.count_tokens(item.get("content", "")) for item in messages
        )
        maximum_output = int(self.options.get("max_output_tokens", 4096))
        reserved_cost = self._cost(estimated_input, maximum_output)
        if self.spent_usd + reserved_cost > self.budget_usd:
            raise RuntimeError(
                "hosted model budget ceiling would be exceeded before request: "
                f"spent=${self.spent_usd:.6f}, reserve=${reserved_cost:.6f}, "
                f"cap=${self.budget_usd:.2f}"
            )

        response = self.client.responses.create(
            model=self.model, input=typed_messages, **self.options
        )
        duration = (time.time() - start) * 1000

        usage = response.usage
        input_tokens = usage.input_tokens if usage else 0
        output_tokens = usage.output_tokens if usage else 0

        cost = self._cost(input_tokens, output_tokens)
        self.spent_usd += cost
        if self.spent_usd > self.budget_usd:
            raise RuntimeError(
                f"hosted model budget ceiling exceeded: ${self.spent_usd:.6f}"
            )

        content = response.output_text or ""

        return LLMResponse(
            content=content,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            latency_ms=duration,
            cost_usd=cost,
        )

    def count_tokens(self, text: str) -> int:
        # Simplified estimation for now
        return len(text) // 4

    def _cost(self, input_tokens: int, output_tokens: int) -> float:
        return (
            input_tokens * self.input_cost_per_million
            + output_tokens * self.output_cost_per_million
        ) / 1_000_000

    @property
    def context_limit(self) -> int:
        if self.model.startswith("gpt-5.6"):
            return 1_050_000
        return 128000


class AnthropicProvider(LLMProvider):
    """Anthropic API provider."""

    def __init__(self, model: str = "claude-3-opus-20240229", **options):
        import anthropic

        self.client = anthropic.Anthropic()
        self.model = model
        self.options = options

    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        start = time.time()
        # Convert messages to Anthropic format if needed
        # (Assuming standard role/content dicts work or need slight adjustment)
        system_prompt = next(
            (m["content"] for m in messages if m["role"] == "system"), ""
        )
        user_messages = [m for m in messages if m["role"] != "system"]
        typed_messages: Any = user_messages

        response = self.client.messages.create(
            model=self.model,
            system=system_prompt,
            messages=typed_messages,
            max_tokens=self.options.get("max_tokens", 4096),
            **{k: v for k, v in self.options.items() if k != "max_tokens"},
        )
        duration = (time.time() - start) * 1000

        input_tokens = response.usage.input_tokens
        output_tokens = response.usage.output_tokens

        # Simple cost estimation
        cost = (input_tokens * 15.0 + output_tokens * 75.0) / 1_000_000

        # Handle content blocks
        text_content = ""
        for block in response.content:
            if hasattr(block, "text"):
                text_content += block.text  # type: ignore

        return LLMResponse(
            content=text_content,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            latency_ms=duration,
            cost_usd=cost,
        )

    def count_tokens(self, text: str) -> int:
        return len(text) // 4

    @property
    def context_limit(self) -> int:
        return 200000


class MockLLMProvider(LLMProvider):
    """Mock LLM provider for testing."""

    def __init__(self, model: str = "mock-model", **options):
        self.model = model
        self.options = options

    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        content = "Action: observe"
        # Simple heuristic to make it do something interesting
        last_msg = messages[-1]["content"]
        if "Observation:" in last_msg:
            content = "Thought: I see the page.\nAction: click 1"

        return LLMResponse(
            content=content,
            input_tokens=100,
            output_tokens=20,
            latency_ms=50.0,
            cost_usd=0.001,
        )

    def count_tokens(self, text: str) -> int:
        return len(text) // 4

    @property
    def context_limit(self) -> int:
        return 10000


class LiteLLMProvider(LLMProvider):
    """LiteLLM provider for multi-engine support."""

    def __init__(self, model: str = "gpt-3.5-turbo", **options):
        try:
            from litellm import completion, token_counter

            self._completion = completion
            self._token_counter = token_counter
        except ImportError:
            raise ImportError("Please install litellm: pip install litellm")

        self.model = model
        self.options = options
        # Set specific options for liteLLM if needed, e.g. api_base
        self.completion_kwargs = options.copy()

    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        start = time.time()

        # Retry logic for API failures
        max_retries = 3
        last_error = None

        for attempt in range(max_retries):
            try:
                # litellm expectation: messages list of dicts {role, content}
                response = self._completion(
                    model=self.model, messages=messages, **self.completion_kwargs
                )

                # Check if choices array is empty (API failure or rate limiting)
                if not response.choices or len(response.choices) == 0:
                    error_msg = "LLM API returned empty choices array"
                    if hasattr(response, "error"):
                        error_msg += f": {response.error}"

                    # If this is rate limiting, wait and retry
                    if attempt < max_retries - 1:
                        wait_time = (attempt + 1) * 2  # 2s, 4s, 6s
                        print(
                            f"Warning: {error_msg}. Retrying in {wait_time}s... (attempt {attempt + 1}/{max_retries})"
                        )
                        time.sleep(wait_time)
                        continue
                    else:
                        raise RuntimeError(error_msg)

                # Success - extract content
                break

            except Exception as e:
                last_error = e
                if attempt < max_retries - 1:
                    wait_time = (attempt + 1) * 2
                    print(
                        f"Warning: LLM API error: {e}. Retrying in {wait_time}s... (attempt {attempt + 1}/{max_retries})"
                    )
                    time.sleep(wait_time)
                else:
                    raise

        duration = (time.time() - start) * 1000

        # liteLLM normalizes the response object to be similar to OpenAI's
        usage = response.usage
        input_tokens = usage.prompt_tokens if usage else 0
        output_tokens = usage.completion_tokens if usage else 0

        # Cost is usually provided or calculatable, but let's see if usage has cost
        # litellm calculates cost if possible in a separate call or we rely on its method
        from litellm import completion_cost

        try:
            cost = completion_cost(completion_response=response)
        except Exception:
            cost = 0.0

        content = response.choices[0].message.content or ""

        return LLMResponse(
            content=content,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            latency_ms=duration,
            cost_usd=cost,
        )

    def count_tokens(self, text: str) -> int:
        # litellm.token_counter requires model
        return self._token_counter(model=self.model, text=text)

    @property
    def context_limit(self) -> int:
        from litellm import model_cost

        # Try to look up context window from litellm's model_cost map
        try:
            info = model_cost.get(self.model, {})
            return info.get("max_tokens", 4096)
        except Exception:
            return 4096
