from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Optional

import yaml


@dataclass
class BenchmarkConfig:
    name: str
    data_dir: Optional[str] = None
    server_url: Optional[str] = None
    options: Dict[str, Any] = field(default_factory=dict)


@dataclass
class LLMConfig:
    provider: str
    model: str
    options: Dict[str, Any] = field(default_factory=dict)


@dataclass
class AgentConfig:
    type: str
    options: Dict[str, Any] = field(default_factory=dict)


@dataclass
class RunConfig:
    run_id: str
    seed: int
    benchmark: BenchmarkConfig
    llm: LLMConfig
    agent: AgentConfig
    prompt_template: str
    oryn_mode: str = "headless"
    oryn_options: Dict[str, Any] = field(default_factory=dict)
    max_steps: int = 30
    timeout_seconds: int = 300

    @classmethod
    def from_yaml(cls, path: Path) -> "RunConfig":
        with open(path, "r") as f:
            data = yaml.safe_load(f)

        return cls(
            run_id=data.get("run_id", "default_run"),
            seed=data.get("seed", 42),
            benchmark=BenchmarkConfig(
                name=data["benchmark"], options=data.get("benchmark_options", {})
            ),
            llm=LLMConfig(
                provider=data["llm_provider"],
                model=data["llm_model"],
                options=data.get("llm_options", {}),
            ),
            agent=AgentConfig(
                type=data["agent_type"], options=data.get("agent_options", {})
            ),
            prompt_template=data.get("prompt_template", "minimal"),
            oryn_mode=data.get("oryn_mode", "headless"),
            oryn_options=data.get("oryn_options", {}),
            max_steps=data.get("max_steps", 30),
            timeout_seconds=data.get("timeout_seconds", 300),
        )
