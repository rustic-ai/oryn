from abc import ABC, abstractmethod
from typing import Any, Dict, List, Optional

from ..core.agent import AgentAction, AgentState
from ..core.oryn import OrynObservation


class FrameworkAdapter(ABC):
    """
    Adapter interface for external agent frameworks (e.g., Swarm, LangChain).
    Maps IntentGym's OrynObservation -> Framework's Input
    Maps Framework's Output -> IntentGym's AgentAction
    """

    @abstractmethod
    def step(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        """Execute one step of the external agent."""
        pass

    @abstractmethod
    def reset(self):
        """Reset the external agent state."""
        pass
