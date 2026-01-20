from typing import Any, List

from ..collection.metrics import Evaluation
from ..core.oryn import OrynInterface
from .base import Benchmark, Task


class WebShopLoader(Benchmark):
    """
    WebShop: Towards Scalable Real-World Web Interaction with Grounded Language Agents.
    """

    def __init__(self, data_dir: str = "~/.intentgym/webshop", **options):
        self.data_dir = data_dir
        self.options = options

    @property
    def name(self) -> str:
        return "webshop"

    def load_tasks(self, subset: str = "all") -> List[Task]:
        # TODO: Load real WebShop data from JSON/files
        # For now, we simulate a few tasks

        simulated_tasks = [
            {
                "id": "webshop_001",
                "intent": "I need a red dress under $50 with good reviews.",
                "url": "http://localhost:3000/webshop/params",
            },
            {
                "id": "webshop_002",
                "intent": "Find me a 32-inch 4K monitor.",
                "url": "http://localhost:3000/webshop/search",
            },
        ]

        return [
            Task(
                task_id=t["id"],
                intent=t["intent"],
                start_url=t["url"],
                success_criteria={"attribute_match": True},
                category="ecommerce",
                max_steps=15,
            )
            for t in simulated_tasks
        ]

    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # WebShop evaluation usually compares the selected product attributes
        # with the goal attributes.
        # This requires parsing the final page/URL to get the product ID.

        # Mock evaluation logic
        current_url = oryn.execute("url").raw
        # Assume success if URL contains "confirmation" or similar
        success = "confirmation" in current_url

        return Evaluation(
            success=success,
            partial_score=1.0 if success else 0.0,
            criteria_met={"attribute_match": success},
        )
