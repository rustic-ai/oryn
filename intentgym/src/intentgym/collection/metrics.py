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
    observation_bytes: int = 0
    observation_revision: Optional[int] = None
    document_generation: Optional[int] = None
    observation_capabilities: List[Any] = field(default_factory=list)
    observation_diagnostics: List[str] = field(default_factory=list)
    action_classification: str = "failed"
    action_accepted: bool = False
    action_effects: List[Any] = field(default_factory=list)
    action_delta: Optional[Any] = None
    action_diagnostics: List[str] = field(default_factory=list)
    revision_before: Optional[int] = None
    revision_after: Optional[int] = None
    observation_raw: Optional[str] = None
    observation_elements: List[Any] = field(default_factory=list)
    trace_slice: List[Any] = field(default_factory=list)
    failure_domain_reason_code: Optional[str] = None


@dataclass
class EpisodeMetrics:
    """Metrics for a single episode in a multi-episode task run."""

    episode_number: int
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
    timeout: bool = False  # Episode hit time limit
    error: Optional[str] = None
    turns: List[TurnMetrics] = field(default_factory=list)


@dataclass
class Evaluation:
    success: bool
    partial_score: float = 0.0
    criteria_met: dict = field(default_factory=dict)
    error: Optional[str] = None
    episode_done: bool = False  # For episodic environments like MiniWoB++
    raw_reward: Optional[float] = (
        None  # Raw reward before clamping (for timeout detection)
    )


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
    # Multi-episode fields (with defaults for backward compatibility)
    is_multi_episode: bool = False
    episodes_count: int = 1
    episodes: List[EpisodeMetrics] = field(default_factory=list)
    success_rate: Optional[float] = None
    episodes_succeeded: int = 0
    mean_steps_per_episode: Optional[float] = None
    mean_cost_per_episode: Optional[float] = None
    mean_duration_per_episode: Optional[float] = None
    timeout_count: int = 0
    episode_stats: Optional[dict] = None


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
        observation: Optional[OrynObservation],
        llm_response: Optional[LLMResponse],
        action: AgentAction,
        result: OrynResult,
        token_breakdown: TokenBreakdown,
    ):
        """Record metrics for a single turn.

        Args:
            observation: Observation for this turn, or None on first turn
            llm_response: LLM response, or None if failed
            action: Action taken
            result: Result of action execution
            token_breakdown: Token usage breakdown
        """
        if llm_response is None:
            # Create dummy response for failed/empty turns to avoid None errors
            llm_response = LLMResponse("", 0, 0, 0.0, 0.0)

        turn = TurnMetrics(
            turn_number=len(self.turns) + 1,
            timestamp=time.time(),
            observation_tokens=observation.token_count if observation else 0,
            history_tokens=token_breakdown.history,
            system_tokens=token_breakdown.system,
            task_tokens=token_breakdown.task,
            total_input_tokens=llm_response.input_tokens,
            output_tokens=llm_response.output_tokens,
            llm_latency_ms=llm_response.latency_ms,
            oryn_observe_latency_ms=observation.latency_ms if observation else 0.0,
            oryn_action_latency_ms=result.latency_ms if result else 0.0,
            cost_usd=llm_response.cost_usd,
            action_command=action.command,
            action_success=result.success if result else True,
            action_error=result.error if result and not result.success else None,
            observation_bytes=observation.byte_count if observation else 0,
            observation_revision=observation.revision if observation else None,
            document_generation=(
                observation.document_generation if observation else None
            ),
            observation_capabilities=(
                [vars(item) for item in observation.capabilities] if observation else []
            ),
            observation_diagnostics=observation.diagnostics if observation else [],
            action_classification=result.classification if result else "failed",
            action_accepted=result.accepted if result else False,
            action_effects=[vars(item) for item in result.effects] if result else [],
            action_delta=(vars(result.delta) if result and result.delta else None),
            action_diagnostics=result.diagnostics if result else [],
            revision_before=result.revision_before if result else None,
            revision_after=result.revision_after if result else None,
            observation_raw=observation.raw if observation else None,
            observation_elements=(list(observation.elements) if observation else []),
            trace_slice=list(result.trace_slice) if result else [],
            failure_domain_reason_code=(
                None
                if not result or result.success
                else (
                    "capability_unsupported"
                    if result.classification == "unsupported"
                    else "oil_action_failed"
                )
            ),
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
