from typing import Any

from ..core.agent import AgentAction, AgentState
from ..core.oryn import OrynObservation
from .base import FrameworkAdapter


class GoogleADKAdapter(FrameworkAdapter):
    """
    Adapter for Google Agent Development Kit (ADK).
    Assuming a standard interface: agent.act(observation) -> action_string
    """

    def __init__(self, adk_agent: Any = None, **kwargs):
        self.agent = adk_agent

    def reset(self):
        if self.agent and hasattr(self.agent, "reset"):
            self.agent.reset()

    def step(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        # 1. Convert OrynObservation to ADK native format if needed
        # For now, pass the raw observation string
        # adk_obs = observation.raw

        # 2. Run Agent
        if self.agent:
            # action = self.agent.act(adk_obs)
            action_text = "Action: observe"
        else:
            action_text = "Action: observe"

        # 3. Return Action
        return AgentAction(command=action_text, reasoning="ADK Agent")
