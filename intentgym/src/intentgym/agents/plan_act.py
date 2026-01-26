from typing import List

from ..core.agent import Agent, AgentAction, AgentState
from ..core.oryn import OrynObservation, OrynResult


class PlanActAgent(Agent):
    """
    Plan-and-Act: Generate complete plan first, then execute.
    """

    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        # Check if we need to (re)plan
        if not hasattr(state, "plan") or state.plan is None:
            state.plan = self._generate_plan(state, observation)
            state.plan_index = 0

        # Check if plan is complete
        if state.plan_index >= len(state.plan):
            return AgentAction(
                command="observe",  # Default action if done
                reasoning="Plan complete, verifying result",
            )

        # Execute next step
        action_cmd = state.plan[state.plan_index]
        state.plan_index += 1

        return AgentAction(
            command=action_cmd,
            reasoning=f"Executing plan step {state.plan_index}/{len(state.plan)}",
        )

    def _generate_plan(
        self, state: AgentState, observation: OrynObservation
    ) -> List[str]:
        """Generate execution plan from task and observation."""
        plan_prompt = f"""Task: {state.task}

Current page state:
{observation.raw}

Generate a step-by-step plan to accomplish this task.
Use Oryn Intent Language commands.
Output one command per line, nothing else.

Plan:"""

        messages = [
            {"role": "system", "content": self.prompt.system},
            {"role": "user", "content": plan_prompt},
        ]

        self.last_llm_response = self.llm.complete(messages)
        response = self.last_llm_response

        # Update metrics (attributing plan generation cost to current step,
        # though effectively it amortizes)
        state.total_input_tokens += response.input_tokens
        state.total_output_tokens += response.output_tokens
        state.total_cost_usd += response.cost_usd

        # Parse plan steps
        lines = response.content.strip().split("\n")
        plan = [
            line.strip() for line in lines if line.strip() and not line.startswith("#")
        ]

        return plan

    def update(self, state: AgentState, action: AgentAction, result: OrynResult):
        super().update(state, action, result)

        # If action failed, trigger re-planning
        if not result.success:
            state.plan = None
