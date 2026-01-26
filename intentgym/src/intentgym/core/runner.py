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
        # Initialize LLM
        if config.llm.provider == "openai":
            self.llm = OpenAIProvider(model=config.llm.model, **config.llm.options)
        elif config.llm.provider == "anthropic":
            self.llm = AnthropicProvider(model=config.llm.model, **config.llm.options)
        elif config.llm.provider == "mock":
            from .llm import MockLLMProvider

            self.llm = MockLLMProvider(model=config.llm.model, **config.llm.options)
        elif config.llm.provider == "litellm":
            from .llm import LiteLLMProvider

            self.llm = LiteLLMProvider(model=config.llm.model, **config.llm.options)
        else:
            raise ValueError(f"Unknown LLM provider: {config.llm.provider}")

        # Initialize Benchmark
        # For now, default to Mock if not found, or implement loader
        if config.benchmark.name == "mock":
            self.benchmark = MockBenchmark()
        elif config.benchmark.name == "miniwob":
            from ..benchmarks.miniwob import MiniWoBLoader

            self.benchmark = MiniWoBLoader(**config.benchmark.options)
        elif config.benchmark.name == "webshop":
            from ..benchmarks.webshop import WebShopLoader

            self.benchmark = WebShopLoader(**config.benchmark.options)
        elif config.benchmark.name == "webarena":
            from ..benchmarks.webarena import WebArenaLoader

            self.benchmark = WebArenaLoader(**config.benchmark.options)
        else:
            # TODO: Dynamic loading of benchmarks
            self.benchmark = MockBenchmark()

        # Initialize Agent
        # Load prompt from file using utility
        from ..utils.prompts import load_prompt

        try:
            prompt = load_prompt(config.prompt_template)
        except Exception:
            # Fallback if file not found (e.g. for testing with just a string name that doesn't exist)
            # or create a default minimal one
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

        if config.agent.type == "react":
            self.agent = ReActAgent(llm=self.llm, prompt=prompt, **config.agent.options)
        elif config.agent.type == "plan_act":
            from intentgym.agents.plan_act import PlanActAgent

            self.agent = PlanActAgent(
                llm=self.llm, prompt=prompt, **config.agent.options
            )
        elif config.agent.type == "reflexion":
            from intentgym.agents.reflexion import ReflexionAgent

            self.agent = ReflexionAgent(
                llm=self.llm, prompt=prompt, **config.agent.options
            )
        elif config.agent.type == "ralph":
            from intentgym.agents.ralph import RALPHAgent

            self.agent = RALPHAgent(llm=self.llm, prompt=prompt, **config.agent.options)

        # Adapters
        elif config.agent.type == "swarm":
            from intentgym.adapters.swarm import SwarmAdapter

            # Adapter wrapping logic might need to change Agent interface or wrap it
            # For now, we assume Adapter implements Agent-like interface or we wrap it
            # But Adapter has 'step' not 'decide'.
            # We need an AdapterWrapperAgent.
            self.agent = AdapterWrapperAgent(SwarmAdapter(**config.agent.options))

        elif config.agent.type == "adk":
            from intentgym.adapters.adk import GoogleADKAdapter

            self.agent = AdapterWrapperAgent(GoogleADKAdapter(**config.agent.options))

        else:
            raise ValueError(f"Unknown agent type: {config.agent.type}")

    def run(self, subset: str = "all", progress_callback: Optional[Callable] = None):
        tasks = self.benchmark.load_tasks(subset)

        for i, task in enumerate(tasks):
            if progress_callback:
                progress_callback(i, len(tasks), task.task_id)

            result = self._run_task(task)
            self.results.append(result)

        return self.results

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
