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
    """OpenAI API provider."""

    def __init__(self, model: str = "gpt-4-turbo", **options):
        import openai

        self.client = openai.OpenAI()
        self.model = model
        self.options = options

    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        start = time.time()
        # Convert simple list[dict] to Iterable[ChatCompletionMessageParam]
        # We assume the dicts are compatible structure
        typed_messages: Any = messages

        response = self.client.chat.completions.create(
            model=self.model, messages=typed_messages, **self.options
        )
        duration = (time.time() - start) * 1000

        usage = response.usage
        input_tokens = usage.prompt_tokens if usage else 0
        output_tokens = usage.completion_tokens if usage else 0

        # Simple cost estimation (replace with actual pricing table later)
        cost = (input_tokens * 10.0 + output_tokens * 30.0) / 1_000_000

        content = response.choices[0].message.content or ""

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

    @property
    def context_limit(self) -> int:
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
                    if hasattr(response, 'error'):
                        error_msg += f": {response.error}"

                    # If this is rate limiting, wait and retry
                    if attempt < max_retries - 1:
                        wait_time = (attempt + 1) * 2  # 2s, 4s, 6s
                        print(f"Warning: {error_msg}. Retrying in {wait_time}s... (attempt {attempt + 1}/{max_retries})")
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
                    print(f"Warning: LLM API error: {e}. Retrying in {wait_time}s... (attempt {attempt + 1}/{max_retries})")
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
