from typing import Any, Dict, List, Optional

from ..core.agent import AgentAction, AgentState
from ..core.oryn import OrynObservation
from .base import FrameworkAdapter

# Swarm is experimental and may not be installed.
# We implement a mock/stub adapter that follows the pattern.


class SwarmAdapter(FrameworkAdapter):
    """
    Adapter for OpenAI Swarm agents.
    """

    def __init__(self, agent_func: Any = None, **kwargs):
        self.agent_func = agent_func  # The Swarm Agent instance
        self.client = None
        self.history: List[Dict[str, str]] = []

    def reset(self):
        self.history = []
        if self.agent_func and hasattr(self.agent_func, "reset"):
            self.agent_func.reset()

    def step(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        # 1. Convert Observation to Swarm Message
        user_msg = {
            "role": "user",
            "content": f"Observation: {observation.raw}\nTask: {state.task}",
        }
        self.history.append(user_msg)

        # 2. Run Swarm Agent (Mock logic if no real agent provided)
        if self.agent_func:
            # Real Swarm usage would go here:
            # response = self.client.run(agent=self.agent, messages=self.history)
            # For now, we assume agent_func returns a string
            response_text = "Action: observe"
        else:
            response_text = "Action: observe"

        # 3. Convert Swarm Response to AgentAction
        return AgentAction(command=response_text, reasoning="Swarm Agent")
