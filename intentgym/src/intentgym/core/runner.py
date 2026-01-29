import logging
import re
import statistics
import time
import traceback
from typing import Callable, List, Optional

from intentgym.agents.react import ReActAgent

logger = logging.getLogger(__name__)

from ..benchmarks.base import Benchmark, MockBenchmark, Task
from ..collection.metrics import (
    EpisodeMetrics,
    Evaluation,
    MetricsCollector,
    TaskMetrics,
    TokenBreakdown,
)
from ..collection.transcript import TranscriptLogger
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

            try:
                # Restart browser between tasks to prevent accumulated state issues
                if i > 0:
                    logger.info(f"Restarting browser before task {task.task_id}...")
                    try:
                        self.oryn.close()
                        time.sleep(1)
                        self.oryn = OrynInterface(mode=self.config.oryn_mode, **self.config.oryn_options)
                        self.oryn.connect()
                        logger.info("✓ Browser restarted successfully")
                    except Exception as e:
                        logger.warning(f"⚠ Browser restart failed: {e}, continuing anyway...")

                result = self._run_task(task)
                self.results.append(result)

            except Exception as e:
                # If browser crashed/hung, try to recover
                if "TimeoutError" in str(type(e).__name__) or "ConnectionLostError" in str(e):
                    logger.error(f"Browser connection lost for task {task.task_id}. Attempting to recover...")
                    logger.error(f"Traceback:\n{traceback.format_exc()}")

                    try:
                        # Close and reinitialize Oryn
                        self.oryn.close()
                        time.sleep(1)
                        self.oryn = OrynInterface(mode=self.config.oryn_mode, **self.config.oryn_options)
                        self.oryn.connect()
                        logger.info("✓ Browser connection recovered")

                        # Create a failed result for this task
                        from ..collection.metrics import Evaluation, TaskMetrics
                        failed_result = TaskMetrics(
                            task_id=task.task_id,
                            config=self.config,
                            success=False,
                            partial_score=0.0,
                            total_steps=0,
                            total_input_tokens=0,
                            total_output_tokens=0,
                            total_observation_tokens=0,
                            total_cost_usd=0.0,
                            total_duration_ms=0.0,
                            observation_ratio=0.0,
                            peak_context_tokens=0,
                            failed_actions=1,
                            turns=[],
                        )
                        self.results.append(failed_result)

                        # Continue to next task
                        logger.info(f"Skipping failed task {task.task_id}, continuing to next task...")
                        continue

                    except Exception as recover_error:
                        logger.error(f"✗ Failed to recover browser: {recover_error}")
                        raise
                else:
                    raise

        return self.results

    def close(self):
        """Clean up resources."""
        if hasattr(self, 'oryn'):
            self.oryn.close()

    def _run_task(self, task: Task) -> TaskMetrics:
        """Dispatch to single-episode or multi-episode runner based on config."""
        episodes_per_task = self.config.benchmark.episodes_per_task

        if episodes_per_task == 1:
            return self._run_task_single_episode(task)
        else:
            return self._run_task_multi_episode(task, episodes_per_task)

    def _run_task_single_episode(self, task: Task) -> TaskMetrics:
        """Run a single episode of a task (original behavior)."""
        # Create transcript logger if enabled
        transcript = None
        if self.config.save_transcript:
            transcript = TranscriptLogger(
                run_id=self.config.run_id,
                task_id=task.task_id,
            )
            transcript.start_episode(1, 1, task.intent)

        collector = MetricsCollector(
            task_id=task.task_id, config=self.config, llm=self.llm
        )
        collector.start_task()
        self.agent.reset()

        state = AgentState(task=task.intent)
        self.oryn.goto(task.start_url)

        # Start with no observation - agent should initiate observe on first turn
        observation = None

        try:
            while state.step_count < self.config.max_steps:
                # Count tokens (mock logic for history token count)
                history_tokens = sum(len(str(h)) for h in state.history) // 4

                token_breakdown = TokenBreakdown(
                    system=self.llm.count_tokens(self.agent.prompt.system),
                    task=self.llm.count_tokens(task.intent),
                    observation=observation.token_count if observation else 0,
                    history=history_tokens,
                )

                action = self.agent.decide(state, observation)
                result = self.oryn.execute(action.command)

                # Log turn summary
                status = "✓" if result.success else "✗"
                error_msg = f" ({result.error})" if not result.success and result.error else ""
                logger.info(
                    f"  Turn {state.step_count + 1}: {action.command[:50]} → {status}{error_msg}"
                )

                collector.record_turn(
                    observation=observation,
                    llm_response=self.agent.last_llm_response,
                    action=action,
                    result=result,
                    token_breakdown=token_breakdown,
                )

                # Log to transcript
                if transcript:
                    transcript.log_turn(
                        observation=observation,
                        llm_response=self.agent.last_llm_response,
                        action=action,
                        result=result,
                        system_prompt=self.agent.prompt.system if state.step_count == 0 else None,
                    )

                self.agent.update(state, action, result)

                # Get fresh observation for next turn
                observation = self.oryn.observe()

                evaluation = self.benchmark.evaluate(task, self.oryn)
                # For episodic environments (MiniWoB++), stop immediately when episode ends
                # regardless of success/failure. For non-episodic tasks, only stop on success.
                if evaluation.episode_done or evaluation.success:
                    task_metrics = collector.finish_task(evaluation)
                    if transcript:
                        transcript.end_episode(
                            success=task_metrics.success,
                            steps=task_metrics.total_steps,
                            duration_ms=task_metrics.total_duration_ms,
                        )
                        transcript.end_task(
                            {
                                "is_multi_episode": False,
                                "success": task_metrics.success,
                                "steps": task_metrics.total_steps,
                                "cost": task_metrics.total_cost_usd,
                            }
                        )
                        logger.info(f"Transcript saved to: {transcript.get_path()}")
                    return task_metrics

        except Exception as e:
            # Log error with full traceback
            logger.error(f"Error executing task: {e}")
            logger.error(f"Traceback:\n{traceback.format_exc()}")
            from ..collection.metrics import Evaluation

            task_metrics = collector.finish_task(Evaluation(success=False, error=str(e)))
            if transcript:
                transcript.end_episode(
                    success=False,
                    steps=task_metrics.total_steps,
                    duration_ms=task_metrics.total_duration_ms,
                    error=str(e),
                )
                transcript.end_task(
                    {
                        "is_multi_episode": False,
                        "success": False,
                        "steps": task_metrics.total_steps,
                        "cost": task_metrics.total_cost_usd,
                    }
                )
                logger.info(f"Transcript saved to: {transcript.get_path()}")
            return task_metrics

        evaluation = self.benchmark.evaluate(task, self.oryn)
        task_metrics = collector.finish_task(evaluation)
        if transcript:
            transcript.end_episode(
                success=task_metrics.success,
                steps=task_metrics.total_steps,
                duration_ms=task_metrics.total_duration_ms,
            )
            transcript.end_task(
                {
                    "is_multi_episode": False,
                    "success": task_metrics.success,
                    "steps": task_metrics.total_steps,
                    "cost": task_metrics.total_cost_usd,
                }
            )
            logger.info(f"Transcript saved to: {transcript.get_path()}")
        return task_metrics

    def _run_task_multi_episode(self, task: Task, num_episodes: int) -> TaskMetrics:
        """Run multiple episodes of a task and aggregate results."""
        episode_results = []

        # Create transcript logger if enabled
        transcript = None
        if self.config.save_transcript:
            transcript = TranscriptLogger(
                run_id=self.config.run_id,
                task_id=task.task_id,
            )

        # Navigate to task URL once
        self.oryn.goto(task.start_url)

        for episode_num in range(1, num_episodes + 1):
            logger.info(f"Episode {episode_num}/{num_episodes} starting...")

            if transcript:
                transcript.start_episode(episode_num, num_episodes, task.intent)

            # CRITICAL: Harness clicks START button (not agent!)
            # This gives the agent the full 10 seconds for the task
            start_clicked = False
            try:
                # Wait briefly for page to be ready
                time.sleep(0.2)

                # First observe to find START button
                obs = self.oryn.observe()

                # Check if timer is already running (START already clicked or no START button)
                if "/ " in obs.raw and ("sec" in obs.raw or "second" in obs.raw):
                    logger.info(f"  ✓ Timer already running (START clicked or task started)")
                    start_clicked = True
                else:
                    # Look for START button in the observation
                    # It's usually a div with text "START"
                    for line in obs.raw.split('\n'):
                        if 'START' in line and line.strip().endswith('"START"'):
                            # Extract element ID from line like: [9] div/generic "START"
                            match = re.match(r'\[(\d+)\]', line.strip())
                            if match:
                                element_id = match.group(1)
                                logger.info(f"  Clicking START button (element {element_id})...")
                                result = self.oryn.execute(f"click {element_id}")
                                if result.success:
                                    start_clicked = True
                                    logger.info(f"  ✓ START clicked successfully")
                                    time.sleep(0.1)  # Brief pause for task initialization
                                else:
                                    logger.warning(f"  ✗ START click failed: {result.error}")
                                break

                    if not start_clicked:
                        # START button not found - check if task needs it
                        logger.debug(f"  No START button found (task may not require one)")

            except Exception as e:
                # Log the error but continue - START clicking is optional
                logger.warning(f"  ✗ Failed to click START: {e}")
                logger.debug(f"Traceback:\n{traceback.format_exc()}")

            # Reset agent for fresh episode
            self.agent.reset()

            # Run single episode
            episode_metrics = self._run_single_episode(task, episode_num, transcript)
            episode_results.append(episode_metrics)

            # Log episode completion
            status_icon = "✓" if episode_metrics.success else "✗"
            duration_s = episode_metrics.total_duration_ms / 1000
            timeout_msg = " [TIMEOUT]" if episode_metrics.timeout else ""
            logger.info(
                f"Episode {episode_num}/{num_episodes} complete: {status_icon} "
                f"{'Success' if episode_metrics.success else 'Failed'} "
                f"({episode_metrics.total_steps} steps, {duration_s:.1f}s){timeout_msg}"
            )

            if transcript:
                transcript.end_episode(
                    success=episode_metrics.success,
                    steps=episode_metrics.total_steps,
                    duration_ms=episode_metrics.total_duration_ms,
                    error=episode_metrics.error,
                )

            # Reset environment for next episode
            self.oryn.goto(task.start_url)
            time.sleep(0.2)

        # Aggregate results
        aggregated = self._aggregate_episode_metrics(
            task.task_id, self.config, episode_results
        )

        # Log final summary
        if transcript:
            transcript.end_task(
                {
                    "is_multi_episode": True,
                    "episodes_count": aggregated.episodes_count,
                    "episodes_succeeded": aggregated.episodes_succeeded,
                    "success_rate": aggregated.success_rate,
                    "mean_steps": aggregated.mean_steps_per_episode,
                    "total_cost": aggregated.total_cost_usd,
                }
            )
            logger.info(f"Transcript saved to: {transcript.get_path()}")

        return aggregated

    def _run_single_episode(
        self, task: Task, episode_num: int, transcript: Optional[TranscriptLogger] = None
    ) -> EpisodeMetrics:
        """Run a single episode and return episode-level metrics."""
        collector = MetricsCollector(
            task_id=task.task_id, config=self.config, llm=self.llm
        )
        collector.start_task()

        state = AgentState(task=task.intent)
        evaluation = None

        # Start with no observation - agent should initiate observe on first turn
        observation = None

        try:
            while state.step_count < self.config.max_steps:
                # Count tokens
                history_tokens = sum(len(str(h)) for h in state.history) // 4

                token_breakdown = TokenBreakdown(
                    system=self.llm.count_tokens(self.agent.prompt.system),
                    task=self.llm.count_tokens(task.intent),
                    observation=observation.token_count if observation else 0,
                    history=history_tokens,
                )

                action = self.agent.decide(state, observation)
                result = self.oryn.execute(action.command)

                # Log turn summary
                status = "✓" if result.success else "✗"
                error_msg = f" ({result.error})" if not result.success and result.error else ""
                logger.info(
                    f"  Turn {state.step_count + 1}: {action.command[:50]} → {status}{error_msg}"
                )

                collector.record_turn(
                    observation=observation,
                    llm_response=self.agent.last_llm_response,
                    action=action,
                    result=result,
                    token_breakdown=token_breakdown,
                )

                # Log to transcript
                if transcript:
                    transcript.log_turn(
                        observation=observation,
                        llm_response=self.agent.last_llm_response,
                        action=action,
                        result=result,
                        system_prompt=self.agent.prompt.system if state.step_count == 0 else None,
                    )

                self.agent.update(state, action, result)

                # Get fresh observation for next turn
                observation = self.oryn.observe()

                evaluation = self.benchmark.evaluate(task, self.oryn)
                if evaluation.episode_done or evaluation.success:
                    break

        except Exception as e:
            logger.error(f"Error in episode {episode_num}: {e}")
            logger.error(f"Traceback:\n{traceback.format_exc()}")
            evaluation = Evaluation(success=False, error=str(e))

        # If evaluation not set or loop completed without break, do final evaluation
        if evaluation is None or not (evaluation.episode_done or evaluation.success):
            evaluation = self.benchmark.evaluate(task, self.oryn)

        # Convert TaskMetrics to EpisodeMetrics
        task_metrics = collector.finish_task(evaluation)

        # Check for timeout
        # MiniWoB assigns reward=-1.0 for timeouts (when the 10-second timer expires)
        # We detect this by:
        # 1. Checking if raw_reward is exactly -1.0 (MiniWoB timeout indicator)
        # 2. Or if episode took >= 9s and failed (duration-based heuristic for older evals)
        timeout = False
        if evaluation.raw_reward is not None:
            # Precise detection: MiniWoB uses -1.0 for timeouts
            timeout = evaluation.raw_reward == -1.0 and task_metrics.total_duration_ms >= 9000
        else:
            # Fallback heuristic for evaluations without raw_reward
            timeout = (
                evaluation.episode_done
                and not evaluation.success
                and evaluation.partial_score == 0.0
                and task_metrics.total_duration_ms >= 9000
            )

        return EpisodeMetrics(
            episode_number=episode_num,
            success=task_metrics.success,
            partial_score=task_metrics.partial_score,
            total_steps=task_metrics.total_steps,
            total_input_tokens=task_metrics.total_input_tokens,
            total_output_tokens=task_metrics.total_output_tokens,
            total_observation_tokens=task_metrics.total_observation_tokens,
            total_cost_usd=task_metrics.total_cost_usd,
            total_duration_ms=task_metrics.total_duration_ms,
            observation_ratio=task_metrics.observation_ratio,
            peak_context_tokens=task_metrics.peak_context_tokens,
            failed_actions=task_metrics.failed_actions,
            timeout=timeout,
            error=evaluation.error,
            turns=task_metrics.turns,
        )

    def _aggregate_episode_metrics(
        self, task_id: str, config: RunConfig, episodes: List[EpisodeMetrics]
    ) -> TaskMetrics:
        """Aggregate multiple episode results into a single TaskMetrics."""
        episodes_succeeded = sum(1 for ep in episodes if ep.success)
        success_rate = episodes_succeeded / len(episodes) if episodes else 0.0

        # Calculate statistics
        steps_list = [ep.total_steps for ep in episodes]
        cost_list = [ep.total_cost_usd for ep in episodes]
        duration_list = [ep.total_duration_ms for ep in episodes]

        def calc_stats(values):
            if not values:
                return {"min": 0, "max": 0, "mean": 0, "stddev": 0}
            import statistics

            return {
                "min": min(values),
                "max": max(values),
                "mean": statistics.mean(values),
                "stddev": statistics.stdev(values) if len(values) > 1 else 0,
            }

        # Aggregate totals across all episodes
        total_steps = sum(steps_list)
        total_cost = sum(cost_list)
        total_duration = sum(duration_list)
        total_input_tokens = sum(ep.total_input_tokens for ep in episodes)
        total_output_tokens = sum(ep.total_output_tokens for ep in episodes)
        total_obs_tokens = sum(ep.total_observation_tokens for ep in episodes)

        # Mean values per episode
        mean_steps = statistics.mean(steps_list) if steps_list else 0
        mean_cost = statistics.mean(cost_list) if cost_list else 0
        mean_duration = statistics.mean(duration_list) if duration_list else 0

        # Aggregate observation ratio (weighted by input tokens)
        if total_input_tokens > 0:
            obs_ratio = total_obs_tokens / total_input_tokens
        else:
            obs_ratio = 0.0

        return TaskMetrics(
            task_id=task_id,
            config=config,
            # Single-episode fields (aggregated)
            success=episodes_succeeded > 0,
            partial_score=statistics.mean([ep.partial_score for ep in episodes])
            if episodes
            else 0.0,
            total_steps=total_steps,
            total_input_tokens=total_input_tokens,
            total_output_tokens=total_output_tokens,
            total_observation_tokens=total_obs_tokens,
            total_cost_usd=total_cost,
            total_duration_ms=total_duration,
            observation_ratio=obs_ratio,
            peak_context_tokens=max((ep.peak_context_tokens for ep in episodes), default=0),
            failed_actions=sum(ep.failed_actions for ep in episodes),
            turns=[],  # Empty for multi-episode (turns are in episodes)
            # Multi-episode fields
            is_multi_episode=True,
            episodes_count=len(episodes),
            episodes=episodes,
            success_rate=success_rate,
            episodes_succeeded=episodes_succeeded,
            mean_steps_per_episode=mean_steps,
            mean_cost_per_episode=mean_cost,
            mean_duration_per_episode=mean_duration,
            timeout_count=sum(1 for ep in episodes if ep.timeout),
            episode_stats={
                "steps": calc_stats(steps_list),
                "cost_usd": calc_stats(cost_list),
                "duration_ms": calc_stats(duration_list),
            },
        )


class AdapterWrapperAgent(Agent):
    """Wraps a FrameworkAdapter to match the Agent interface."""

    def __init__(self, adapter):
        self.adapter = adapter
        # Mock LLM/Prompt for base class compatibility
        # In a real impl, we might separate Agent interface further
        from .llm import MockLLMProvider

        super().__init__(llm=MockLLMProvider(), prompt=None)

    def decide(
        self, state: AgentState, observation: Optional[OrynObservation] = None
    ) -> AgentAction:
        # Adapters may not handle None observation, so default to observe
        if observation is None:
            return AgentAction(
                command="observe", reasoning="First turn, need to observe page state"
            )
        return self.adapter.step(state, observation)

    def reset(self):
        super().reset()
        self.adapter.reset()
