from dataclasses import dataclass
from typing import List, Optional

import numpy as np

from ..core.agent import Agent, AgentAction, AgentState
from ..core.oryn import OrynObservation


@dataclass
class Trajectory:
    task: str
    final_state: str
    actions: List[str]


class TrajectoryStore:
    """Store and retrieve successful trajectories for RALPH."""

    def __init__(self, embedding_model: str = "text-embedding-3-small"):
        self.trajectories: List[Trajectory] = []
        self.embeddings: List[np.ndarray] = []
        self.embedding_model = embedding_model

    def add(self, trajectory: Trajectory):
        """Add successful trajectory to store."""
        # TODO: Use actual embedding model provider
        # For now, we use a random vector for mock
        embedding = self._embed(trajectory.task + " " + trajectory.final_state)
        self.trajectories.append(trajectory)
        self.embeddings.append(embedding)

    def retrieve(self, task: str, observation: str, k: int = 3) -> List[Trajectory]:
        """Retrieve k most similar trajectories."""
        if not self.embeddings:
            return []

        query_embedding = self._embed(task + " " + observation)

        # Cosine similarity
        similarities = [
            np.dot(query_embedding, emb)
            / (np.linalg.norm(query_embedding) * np.linalg.norm(emb) + 1e-9)
            for emb in self.embeddings
        ]

        # Top-k
        top_indices = np.argsort(similarities)[-k:][::-1]
        return [self.trajectories[i] for i in top_indices if i < len(self.trajectories)]

    def _embed(self, text: str) -> np.ndarray:
        # Mock embedding
        # In real impl, call OpenAI/HF
        rng = np.random.default_rng(len(text))
        return rng.random(1536)


class RALPHAgent(Agent):
    """
    RALPH: Retrieval Augmented Language Model for Planning in Hypertext.
    """

    def __init__(self, trajectory_store: Optional[TrajectoryStore] = None, **kwargs):
        super().__init__(**kwargs)
        # If store not provided, create empty one (or load from disk in future)
        self.trajectory_store = trajectory_store or TrajectoryStore()

    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        # Retrieve similar past trajectories
        similar_trajectories = self.trajectory_store.retrieve(
            task=state.task, observation=observation.raw, k=3
        )

        # Build prompt with retrieved examples
        messages = [{"role": "system", "content": self.prompt.system}]

        if similar_trajectories:
            examples = "\n\n".join(
                self._format_trajectory(t) for t in similar_trajectories
            )
            messages.append(
                {
                    "role": "user",
                    "content": f"Similar successful tasks for reference:\n{examples}",
                }
            )

        # Standard ReAct/Observation flow
        messages.append(
            {
                "role": "user",
                "content": self.prompt.format_observation(
                    observation=observation, task=state.task, history=state.history
                ),
            }
        )

        self.last_llm_response = self.llm.complete(messages)
        response = self.last_llm_response

        # Update metrics
        state.total_input_tokens += response.input_tokens
        state.total_output_tokens += response.output_tokens
        state.total_cost_usd += response.cost_usd

        # Simple action parsing (assume output is just action or ReAct style)
        cmd = (
            response.content.strip()
            if "Action:" not in response.content
            else response.content.split("Action:")[1].strip().split("\n")[0]
        )

        return AgentAction(command=cmd, reasoning="RALPH retrieval")

    def _format_trajectory(self, t: Trajectory) -> str:
        return f"Task: {t.task}\nActions: {' -> '.join(t.actions)}"
