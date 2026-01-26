import json
from pathlib import Path
from typing import List, Optional

from ..collection.metrics import Evaluation
from ..core.oryn import OrynInterface
from .base import Benchmark, Task


class WebArenaLoader(Benchmark):
    """
    WebArena: A Realistic Web Environment for Building Autonomous Agents.
    """

    def __init__(
        self,
        data_dir: str = "~/.intentgym/webarena",
        server_url: Optional[str] = None,
        **options,
    ):
        self.data_dir = Path(data_dir).expanduser()
        self.server_url = server_url
        self.options = options

    @property
    def name(self) -> str:
        return "webarena"

    def load_tasks(self, subset: str = "all") -> List[Task]:
        tasks = []

        # Check if data directory exists
        if not self.data_dir.exists():
            # For now, return a mock task if data not found, or raise warning
            # Raising warning and returning mock to avoid blocking user flow
            print(
                f"Warning: WebArena data dir {self.data_dir} not found. Using mock task."
            )
            return [
                Task(
                    task_id="webarena_mock_001",
                    intent="Mock WebArena Task: Navigate to GitLab",
                    start_url="http://gitlab.local",
                    success_criteria={"url_contains": "gitlab"},
                    category="cms",
                )
            ]

        for task_file in self.data_dir.glob("*.json"):
            try:
                with open(task_file, "r") as f:
                    data = json.load(f)

                # Handle single object or list of objects
                if isinstance(data, list):
                    items = data
                else:
                    items = [data]

                for item in items:
                    tasks.append(
                        Task(
                            task_id=str(item.get("task_id", "unknown")),
                            intent=item.get("intent", ""),
                            start_url=item.get("start_url", ""),
                            success_criteria=item.get("eval", {}),
                            difficulty=item.get("difficulty", "medium"),
                            category=(
                                item.get("sites", ["general"])[0]
                                if item.get("sites")
                                else "general"
                            ),
                            annotations=item.get("reference_answers"),
                        )
                    )
            except Exception as e:
                print(f"Error loading {task_file}: {e}")

        if subset != "all":
            # Filter by task ID if subset is not all
            tasks = [t for t in tasks if str(t.task_id) == subset]

        return tasks

    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        criteria = task.success_criteria
        results = {}

        # URL-based evaluation
        if "url_contains" in criteria:
            try:
                current_url = oryn.execute("url").raw
                results["url"] = criteria["url_contains"] in current_url
            except Exception:
                results["url_check_failed"] = False

        # Element existence
        if "exists" in criteria:
            selector = criteria["exists"]
            try:
                # Naive existential check
                exists = oryn.execute(f'exists "{selector}"').raw
                results[f"exists_{selector}"] = "true" in str(exists).lower()
            except Exception:
                results[f"exists_{selector}"] = False

        # Mocking evaluation for now if no criteria matched or complex logic needed
        if not results:
            # Just assume fail for safety unless explicitly passed via mock
            return Evaluation(success=False, partial_score=0.0)

        all_passed = all(results.values())
        return Evaluation(
            success=all_passed,
            partial_score=(
                sum(1 for v in results.values() if v) / len(results) if results else 0.0
            ),
            criteria_met=results,
        )
