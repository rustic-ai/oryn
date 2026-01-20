from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

from ..collection.metrics import Evaluation
from ..core.oryn import OrynInterface


@dataclass
class Task:
    """A single benchmark task."""

    task_id: str
    intent: str
    start_url: str
    success_criteria: Dict[str, Any]
    difficulty: str = "medium"
    category: str = "general"
    max_steps: int = 30
    timeout_seconds: int = 300
    annotations: Optional[Dict[str, Any]] = None
    hints: List[str] = field(default_factory=list)


class Benchmark(ABC):
    """Abstract benchmark."""

    @property
    @abstractmethod
    def name(self) -> str:
        pass

    @abstractmethod
    def load_tasks(self, subset: str = "all") -> List[Task]:
        """Load benchmark tasks."""
        pass

    @abstractmethod
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        """Evaluate task completion."""
        pass


class MockBenchmark(Benchmark):
    """A mock benchmark for verification."""

    @property
    def name(self) -> str:
        return "mock"

    def load_tasks(self, subset: str = "all") -> List[Task]:
        return [
            Task(
                task_id="mock_001",
                intent="Go to example.com and verify title",
                start_url="https://example.com",
                success_criteria={"title": "Example Domain"},
            )
        ]

    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # Mock evaluation
        return Evaluation(success=True, partial_score=1.0)
