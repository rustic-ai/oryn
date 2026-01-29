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
        # MiniWoB environments expose success via window.WOB_DONE_GLOBAL and window.WOB_REWARD_GLOBAL
        # Since Oryn doesn't have a generic JS eval command yet, we use a workaround:
        # Extract the "Last reward:" value from the reward display UI

        episode_done = False
        success = False
        partial_score = 0.0

        try:
            # Get page text which includes the reward display
            # MiniWoB shows: "Last reward: X.XX" where X.XX is the reward
            result = oryn.execute('text')
            text_content = result.raw.strip()

            # Parse reward from "Last reward: X.XX" line
            import re
            reward_match = re.search(r'Last reward:\s*([-\d.]+)', text_content)

            if reward_match:
                reward_text = reward_match.group(1)
                try:
                    reward = float(reward_text)
                    # Episode is done - a numeric reward was assigned
                    episode_done = True
                    # MiniWoB rewards: positive (>0) for success, negative (<0) for failure
                    success = reward > 0
                    partial_score = max(0.0, reward)
                    raw_reward = reward  # Preserve raw reward for timeout detection
                except ValueError:
                    # If we can't parse (e.g., "-" for not started), task not complete
                    episode_done = False
                    success = False
                    partial_score = 0.0
                    raw_reward = None
            else:
                # "Last reward: -" means no reward yet (task not done)
                episode_done = False
                success = False
                partial_score = 0.0
                raw_reward = None

        except Exception as e:
            # If text extraction fails, assume not done yet
            episode_done = False
            success = False
            partial_score = 0.0
            raw_reward = None

        return Evaluation(
            success=success,
            partial_score=partial_score,
            criteria_met={"env_success": success},
            episode_done=episode_done,
            raw_reward=raw_reward,
        )

    def _get_intent(self, name: str) -> str:
        # Map task names to natural language intents
        # This is a simplified map
        intents = {
            "click-button": "Retrieve the instructions from the page and click the button as required by the instruction.",
            "click-link": "Click the specified link.",
            "click-option": "Select the correct option.",
            "enter-text": "Enter the specified text into the input.",
            "focus-text": "Focus into the specified text input.",
            "choose-date": "Select the specified date from the date picker.",
            "login-user": "Read the page content for the username and password. Then log in with the given username and password.",
            "search-engine": "Search for the specified query and click the result.",
            "email-inbox": "Navigate the email inbox to find the required email.",
        }
        return intents.get(name, f"Complete the {name} task.")
