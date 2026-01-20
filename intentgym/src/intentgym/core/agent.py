from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from string import Template
from typing import Any, Dict, List, Optional

from .llm import LLMProvider, LLMResponse
from .oryn import OrynObservation, OrynResult


@dataclass
class PromptTemplate:
    """Configurable prompt template."""

    name: str
    version: str
    system: str
    observation_format: str
    action_format: str
    few_shot_examples: List[Dict[str, str]] = field(default_factory=list)
    error_recovery_hints: Optional[str] = None

    def format_observation(
        self, observation: OrynObservation, task: str, history: List[Dict[str, str]]
    ) -> str:
        """Format observation for LLM consumption using string.Template."""
        formatted_history = "\n".join(
            [f"Action: {h['action']}\nResult: {h.get('result', '')}" for h in history]
        )

        return Template(self.observation_format).safe_substitute(
            observation=observation.raw,
            task=task,
            history=formatted_history,
            url=observation.url,
            title=observation.title,
        )


@dataclass
class AgentAction:
    """Action decided by the agent."""

    command: str
    reasoning: Optional[str] = None


@dataclass
class AgentState:
    """Mutable state of the agent during a task."""

    task: str
    history: List[Dict[str, Any]] = field(default_factory=list)
    step_count: int = 0
    total_input_tokens: int = 0
    total_output_tokens: int = 0
    total_cost_usd: float = 0.0
    # PlanAct specific
    plan: Optional[List[str]] = None
    plan_index: int = 0


class Agent(ABC):
    """Abstract agent architecture."""

    def __init__(self, llm: LLMProvider, prompt: PromptTemplate, **config):
        self.llm = llm
        self.prompt = prompt
        self.config = config
        self.last_llm_response: Optional[LLMResponse] = None

    @abstractmethod
    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        """Decide next action based on state and observation."""
        pass

    def update(self, state: AgentState, action: AgentAction, result: OrynResult):
        """Update agent state after action execution."""
        state.history.append(
            {
                "step": state.step_count,
                "action": action.command,
                "reasoning": action.reasoning,
                "result": result.raw,
                "success": result.success,
            }
        )
        state.step_count += 1

    def reset(self):
        """Reset agent for new task."""
        self.last_llm_response = None
