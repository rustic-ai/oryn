from typing import List

from ..core.agent import Agent, AgentAction, AgentState
from ..core.oryn import OrynObservation


class ReflexionAgent(Agent):
    """
    Reflexion: Language Agents with Verbal Reinforcement Learning.
    Maintains reflections on past failures to improve future attempts.
    """

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.reflections: List[str] = []
        self.max_reflections: int = 3

    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        messages = [{"role": "system", "content": self.prompt.system}]

        # Include relevant reflections
        if self.reflections:
            reflection_text = "\n".join(
                f"• {r}" for r in self.reflections[-self.max_reflections :]
            )
            messages.append(
                {
                    "role": "user",
                    "content": f"Learn from past attempts:\n{reflection_text}",
                }
            )

        # Standard ReAct prompting logic here (simplified for brevity, usually inherits logic)
        messages.append(
            {
                "role": "user",
                "content": self.prompt.format_observation(
                    observation=observation,
                    task=state.task,
                    history=state.history,  # Should be formatted string
                ),
            }
        )

        self.last_llm_response = self.llm.complete(messages)
        response = self.last_llm_response

        # Update metrics
        state.total_input_tokens += response.input_tokens
        state.total_output_tokens += response.output_tokens
        state.total_cost_usd += response.cost_usd

        # Simple action parsing
        return AgentAction(
            command=response.content.strip(), reasoning="Reflexion decision"
        )

    def reflect_on_failure(self, state: AgentState, error: str):
        """Generate reflection after task failure."""
        reflection_prompt = f"""You failed to complete this task:

Task: {state.task}
Error: {error}

Reflect: What went wrong? What should you do differently next time?
Be specific and actionable. One paragraph."""

        response = self.llm.complete(
            [
                {
                    "role": "system",
                    "content": "You are reflecting on a failed web automation task.",
                },
                {"role": "user", "content": reflection_prompt},
            ]
        )

        self.reflections.append(response.content.strip())
