from typing import List

from ..collection.metrics import Evaluation
from ..core.oryn import OrynInterface
from .base import Benchmark, Task


class MiniWoBLoader(Benchmark):
    """
    MiniWoB++: Reinforcement Learning on Web Interfaces.
    """

    def __init__(self, server_url: str = "http://localhost:8765", **options):
        self.server_url = server_url
        self.options = options

        # Standard MiniWoB tasks
        self.task_names = [
            "click-button",
            "click-link",
            "click-option",
            "enter-text",
            "focus-text",
            "choose-date",
            "login-user",
            "search-engine",
            "email-inbox",
        ]

    @property
    def name(self) -> str:
        return "miniwob"

    def load_tasks(self, subset: str = "all") -> List[Task]:
        selection = self.task_names
        if subset != "all":
            # Simple subset filtering "click-button,enter-text"
            if "," in subset:
                requested = subset.split(",")
                selection = [t for t in self.task_names if t in requested]
            elif subset in self.task_names:
                selection = [subset]
            # If subset is "train"/"test", we would need a split mapping.
            # For now, treat as "all" or specific list.

        return [
            Task(
                task_id=name,
                intent=self._get_intent(name),
                start_url=f"{self.server_url}/miniwob/{name}.html",
                success_criteria={"env_success": True},
                max_steps=10,
                category="miniwob",
            )
            for name in selection
        ]

    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # MiniWoB environments have built-in success detection via JS
        # We check window.__miniwob_success__ (assuming helper snippet injected or standard env)

        # In a real Oryn implementation, we would execute JS.
        # For now, we simulate or use the execute command.
        # Note: Oryn 'execute' returns OrynResult.
        # We assume there's a way to run JS or check state.

        # Try to read success state from the page
        # check = oryn.execute("js return window.__miniwob_success__")
        # But Oryn might not support direct JS eval in Intent Language yet.
        # SPEC says: result = oryn.execute("execute window.__miniwob_success__")

        try:
            result = oryn.execute("execute window.__miniwob_success__")
            success = "true" in str(result.raw).lower()
        except Exception:
            success = False

        return Evaluation(
            success=success,
            partial_score=1.0 if success else 0.0,
            criteria_met={"env_success": success},
        )

    def _get_intent(self, name: str) -> str:
        # Map task names to natural language intents
        # This is a simplified map
        intents = {
            "click-button": "Click the button required by the instruction.",
            "click-link": "Click the specified link.",
            "click-option": "Select the correct option.",
            "enter-text": "Enter the specified text into the input.",
            "focus-text": "Focus into the specified text input.",
            "choose-date": "Select the specified date from the date picker.",
            "login-user": "Log in with the given username and password.",
            "search-engine": "Search for the specified query and click the result.",
            "email-inbox": "Navigate the email inbox to find the required email.",
        }
        return intents.get(name, f"Complete the {name} task.")
