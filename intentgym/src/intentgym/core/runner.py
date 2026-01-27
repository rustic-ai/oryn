from typing import Callable, List, Optional

from intentgym.agents.react import ReActAgent

from ..benchmarks.base import Benchmark, MockBenchmark, Task
from ..collection.metrics import MetricsCollector, TaskMetrics, TokenBreakdown
from .agent import Agent, AgentAction, AgentState, PromptTemplate
from .config import RunConfig
from .llm import AnthropicProvider, LLMProvider, OpenAIProvider
from .oryn import OrynInterface, OrynObservation


class BenchmarkRunner:
    """Orchestrates the benchmark run."""

    def __init__(self, config: RunConfig):
        self.config = config
        self.results: List[TaskMetrics] = []

        # Initialize Oryn
        self.oryn = OrynInterface(mode=config.oryn_mode, **config.oryn_options)
        self.oryn.connect()

        self.llm: LLMProvider
        self.benchmark: Benchmark
        self.agent: Agent

        # Initialize LLM provider
        self.llm = self._create_llm_provider(config)

        # Initialize Benchmark
        self.benchmark = self._create_benchmark(config)

        # Initialize Agent
        self.agent = self._create_agent(config)

    def _create_llm_provider(self, config: RunConfig) -> LLMProvider:
        """Create the LLM provider based on configuration."""
        provider = config.llm.provider
        model = config.llm.model
        options = config.llm.options

        if provider == "openai":
            return OpenAIProvider(model=model, **options)
        if provider == "anthropic":
            return AnthropicProvider(model=model, **options)
        if provider == "mock":
            from .llm import MockLLMProvider
            return MockLLMProvider(model=model, **options)
        if provider == "litellm":
            from .llm import LiteLLMProvider
            return LiteLLMProvider(model=model, **options)

        raise ValueError(f"Unknown LLM provider: {provider}")

    def _create_benchmark(self, config: RunConfig) -> Benchmark:
        """Create the benchmark based on configuration."""
        name = config.benchmark.name
        options = config.benchmark.options

        if name == "mock":
            return MockBenchmark()
        if name == "miniwob":
            from ..benchmarks.miniwob import MiniWoBLoader
            return MiniWoBLoader(**options)
        if name == "webshop":
            from ..benchmarks.webshop import WebShopLoader
            return WebShopLoader(**options)
        if name == "webarena":
            from ..benchmarks.webarena import WebArenaLoader
            return WebArenaLoader(**options)

        raise ValueError(f"Unknown benchmark: {name}")

    def _create_agent(self, config: RunConfig) -> Agent:
        """Create the agent based on configuration."""
        from ..utils.prompts import load_prompt

        # Load prompt template
        try:
            prompt = load_prompt(config.prompt_template)
        except Exception:
            print(
                f"Warning: Could not load prompt '{config.prompt_template}', using minimal default."
            )
            prompt = PromptTemplate(
                name="minimal",
                version="1.0",
                system="You are a helpful web agent. Respond with commands.",
                observation_format="${observation}",
                action_format="Action: ",
            )

        agent_type = config.agent.type
        options = config.agent.options

        # Standard agents
        if agent_type == "react":
            return ReActAgent(llm=self.llm, prompt=prompt, **options)
        if agent_type == "plan_act":
            from intentgym.agents.plan_act import PlanActAgent
            return PlanActAgent(llm=self.llm, prompt=prompt, **options)
        if agent_type == "reflexion":
            from intentgym.agents.reflexion import ReflexionAgent
            return ReflexionAgent(llm=self.llm, prompt=prompt, **options)
        if agent_type == "ralph":
            from intentgym.agents.ralph import RALPHAgent
            return RALPHAgent(llm=self.llm, prompt=prompt, **options)

        # Framework adapters
        if agent_type == "swarm":
            from intentgym.adapters.swarm import SwarmAdapter
            return AdapterWrapperAgent(SwarmAdapter(**options))
        if agent_type == "adk":
            from intentgym.adapters.adk import GoogleADKAdapter
            return AdapterWrapperAgent(GoogleADKAdapter(**options))

        raise ValueError(f"Unknown agent type: {agent_type}")

    def run(self, subset: str = "all", progress_callback: Optional[Callable] = None):
        tasks = self.benchmark.load_tasks(subset)

        for i, task in enumerate(tasks):
            if progress_callback:
                progress_callback(i, len(tasks), task.task_id)

            result = self._run_task(task)
            self.results.append(result)

        return self.results

    def close(self):
        """Clean up resources."""
        if hasattr(self, 'oryn'):
            self.oryn.close()

    def _run_task(self, task: Task) -> TaskMetrics:
        collector = MetricsCollector(
            task_id=task.task_id, config=self.config, llm=self.llm
        )
        collector.start_task()
        self.agent.reset()

        state = AgentState(task=task.intent)
        self.oryn.goto(task.start_url)

        try:
            while state.step_count < self.config.max_steps:
                observation = self.oryn.observe()

                # Count tokens (mock logic for history token count)
                history_tokens = sum(len(str(h)) for h in state.history) // 4

                token_breakdown = TokenBreakdown(
                    system=self.llm.count_tokens(self.agent.prompt.system),
                    task=self.llm.count_tokens(task.intent),
                    observation=observation.token_count,
                    history=history_tokens,
                )

                action = self.agent.decide(state, observation)
                result = self.oryn.execute(action.command)

                collector.record_turn(
                    observation=observation,
                    llm_response=self.agent.last_llm_response,
                    action=action,
                    result=result,
                    token_breakdown=token_breakdown,
                )

                self.agent.update(state, action, result)

                evaluation = self.benchmark.evaluate(task, self.oryn)
                if evaluation.success:
                    return collector.finish_task(evaluation)

        except Exception as e:
            # Log error
            print(f"Error executing task: {e}")
            from ..collection.metrics import Evaluation

            return collector.finish_task(Evaluation(success=False, error=str(e)))

        evaluation = self.benchmark.evaluate(task, self.oryn)
        return collector.finish_task(evaluation)


class AdapterWrapperAgent(Agent):
    """Wraps a FrameworkAdapter to match the Agent interface."""

    def __init__(self, adapter):
        self.adapter = adapter
        # Mock LLM/Prompt for base class compatibility
        # In a real impl, we might separate Agent interface further
        from .llm import MockLLMProvider

        super().__init__(llm=MockLLMProvider(), prompt=None)

    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        return self.adapter.step(state, observation)

    def reset(self):
        super().reset()
        self.adapter.reset()
