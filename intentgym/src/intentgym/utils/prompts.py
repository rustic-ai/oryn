from pathlib import Path

import yaml

from ..core.agent import PromptTemplate

PROMPT_DIR = Path(__file__).parent.parent.parent.parent / "prompts"


def load_prompt(name: str) -> PromptTemplate:
    """Load prompt template from YAML file."""
    # Check if name is a path
    if Path(name).exists():
        path = Path(name)
    else:
        path = PROMPT_DIR / f"{name}.yaml"

    if not path.exists():
        raise FileNotFoundError(
            f"Prompt template not found: {path} (and {name} is not a valid path)"
        )

    with open(path, "r") as f:
        data = yaml.safe_load(f)

    return PromptTemplate(
        name=data.get("name", name),
        version=data.get("version", "1.0"),
        system=data["system"],
        observation_format=data["observation_format"],
        action_format=data["action_format"],
        few_shot_examples=data.get("few_shot_examples", []),
    )
