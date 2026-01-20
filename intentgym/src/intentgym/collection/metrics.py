import time
from dataclasses import dataclass, field
from typing import Any, List, Optional

from ..core.agent import AgentAction
from ..core.llm import LLMProvider, LLMResponse
from ..core.oryn import OrynObservation, OrynResult


@dataclass
class TokenBreakdown:
    system: int
    task: int
    observation: int
    history: int


@dataclass
class TurnMetrics:
    turn_number: int
    timestamp: float
    observation_tokens: int
    history_tokens: int
    system_tokens: int
    task_tokens: int
    total_input_tokens: int
    output_tokens: int
    llm_latency_ms: float
    oryn_observe_latency_ms: float
    oryn_action_latency_ms: float
    cost_usd: float
    action_command: str
    action_success: bool
    action_error: Optional[str] = None


@dataclass
class Evaluation:
    success: bool
    partial_score: float = 0.0
    criteria_met: dict = field(default_factory=dict)
    error: Optional[str] = None


@dataclass
class TaskMetrics:
    task_id: str
    config: Any  # RunConfig placeholder
    success: bool
    partial_score: float
    total_steps: int
    total_input_tokens: int
    total_output_tokens: int
    total_observation_tokens: int
    total_cost_usd: float
    total_duration_ms: float
    observation_ratio: float
    peak_context_tokens: int
    failed_actions: int
    turns: List[TurnMetrics]


class MetricsCollector:
    """Collects metrics throughout a task run."""

    def __init__(self, task_id: str, config: Any, llm: LLMProvider):
        self.task_id = task_id
        self.config = config
        self.llm = llm
        self.turns: List[TurnMetrics] = []
        self.start_time: float = 0.0

    def start_task(self):
        self.start_time = time.time()

    def record_turn(
        self,
        observation: OrynObservation,
        llm_response: Optional[LLMResponse],
        action: AgentAction,
        result: OrynResult,
        token_breakdown: TokenBreakdown,
    ):
        """Record metrics for a single turn."""
        if llm_response is None:
            # Create dummy response for failed/empty turns to avoid None errors
            llm_response = LLMResponse("", 0, 0, 0.0, 0.0)

        turn = TurnMetrics(
            turn_number=len(self.turns) + 1,
            timestamp=time.time(),
            observation_tokens=observation.token_count,
            history_tokens=token_breakdown.history,
            system_tokens=token_breakdown.system,
            task_tokens=token_breakdown.task,
            total_input_tokens=llm_response.input_tokens,
            output_tokens=llm_response.output_tokens,
            llm_latency_ms=llm_response.latency_ms,
            oryn_observe_latency_ms=observation.latency_ms,
            oryn_action_latency_ms=result.latency_ms if result else 0.0,
            cost_usd=llm_response.cost_usd,
            action_command=action.command,
            action_success=result.success if result else True,
            action_error=result.error if result and not result.success else None,
        )
        self.turns.append(turn)

    def finish_task(self, evaluation: Evaluation) -> TaskMetrics:
        """Finalize and return task metrics."""
        duration_ms = (time.time() - self.start_time) * 1000

        total_input = sum(t.total_input_tokens for t in self.turns)
        total_obs = sum(t.observation_tokens for t in self.turns)

        return TaskMetrics(
            task_id=self.task_id,
            config=self.config,
            success=evaluation.success,
            partial_score=evaluation.partial_score,
            total_steps=len(self.turns),
            total_input_tokens=total_input,
            total_output_tokens=sum(t.output_tokens for t in self.turns),
            total_observation_tokens=total_obs,
            total_cost_usd=sum(t.cost_usd for t in self.turns),
            total_duration_ms=duration_ms,
            observation_ratio=total_obs / total_input if total_input > 0 else 0.0,
            peak_context_tokens=max(
                (t.total_input_tokens for t in self.turns), default=0
            ),
            failed_actions=sum(1 for t in self.turns if not t.action_success),
            turns=self.turns,
        )
