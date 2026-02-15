import json
import logging
from typing import Optional, Tuple

from ..core.agent import Agent, AgentAction, AgentState
from ..core.oryn import OrynObservation

logger = logging.getLogger(__name__)


class ReActAgent(Agent):
    """
    ReAct: Synergizing Reasoning and Acting in Language Models.
    Interleaves reasoning (Thought) with acting (Action).
    """

    def decide(
        self, state: AgentState, observation: Optional[OrynObservation] = None
    ) -> AgentAction:
        # On first turn (no observation), agent should observe first
        if observation is None:
            return AgentAction(
                command="observe", reasoning="First turn, need to observe page state"
            )

        # Build messages with ReAct format
        messages = [
            {"role": "system", "content": self.prompt.system},
        ]

        # Add history
        # We process history to match ReAct format: Thought/Action pairs followed by Observation
        for step in state.history:
            content = f"Thought: {step.get('reasoning', '')}\nAction: {step['action']}"
            messages.append({"role": "assistant", "content": content})
            messages.append(
                {"role": "user", "content": f"Observation: {step['result']}"}
            )

        # Current turn
        messages.append(
            {
                "role": "user",
                "content": self.prompt.format_observation(
                    observation=observation,
                    task=state.task,
                    history=[],  # History is already handled above for ReAct specific format
                ),
            }
        )

        # Get LLM response
        logger.debug(f"LLM input messages: {json.dumps(messages, indent=2)}")

        self.last_llm_response = self.llm.complete(messages)
        response = self.last_llm_response

        logger.debug(f"LLM response: {response.content}")

        # Update state metrics
        state.total_input_tokens += response.input_tokens
        state.total_output_tokens += response.output_tokens
        state.total_cost_usd += response.cost_usd

        # Parse Thought/Action from response
        thought, action_cmd = self._parse_react(response.content)

        return AgentAction(command=action_cmd, reasoning=thought)

    def _parse_react(self, response: str) -> Tuple[str, str]:
        """Extract Thought and Action from ReAct format response."""
        thought = ""
        action = ""

        try:
            if "Thought:" in response:
                thought_start = response.index("Thought:") + 8
                thought_end = (
                    response.index("Action:")
                    if "Action:" in response
                    else len(response)
                )
                thought = response[thought_start:thought_end].strip()
            # If no explicit Thought block, assume start is thought until Action:
            elif "Action:" in response:
                thought_end = response.index("Action:")
                thought = response[:thought_end].strip()

            if "Action:" in response:
                action_start = response.index("Action:") + 7
                action = response[action_start:].strip().split("\n")[0]
            else:
                # Fallback: if only one line and no Action:, treat as action if short
                stripped = response.strip()
                if len(stripped.splitlines()) == 1:
                    action = stripped

        except Exception:
            # Fallback for parsing errors
            pass

        # Default fallback if parsing fails or incomplete
        if not action and not thought:
            thought = response
            action = "observe"  # Safety fallback

        if not action:
            action = "observe"

        return thought, action
