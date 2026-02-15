"""Transcript logger for LLM <> Agent <> Oryn communication."""
import time
from datetime import datetime
from pathlib import Path
from typing import Optional

from ..core.agent import AgentAction
from ..core.llm import LLMResponse
from ..core.oryn import OrynObservation, OrynResult


class TranscriptLogger:
    """Logs LLM <> Agent <> Oryn communication to a markdown file."""

    def __init__(self, run_id: str, task_id: str, output_dir: str = "transcripts"):
        self.run_id = run_id
        self.task_id = task_id
        self.output_dir = Path(output_dir)
        self.output_dir.mkdir(parents=True, exist_ok=True)

        # Create filename: <run_id>_<task_id>_<timestamp>.md
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        self.filename = self.output_dir / f"{run_id}_{task_id}_{timestamp}.md"

        self.current_episode = 0
        self.current_turn = 0
        self._init_file()

    def _init_file(self):
        """Initialize the transcript file with header."""
        with open(self.filename, "w") as f:
            f.write(f"# Transcript: {self.run_id} - {self.task_id}\n\n")
            f.write(f"**Generated**: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n\n")
            f.write("---\n\n")

    def start_episode(self, episode_num: int, total_episodes: int, task_intent: str):
        """Log episode start."""
        self.current_episode = episode_num
        self.current_turn = 0

        with open(self.filename, "a") as f:
            f.write(f"\n## 🎯 Episode {episode_num}/{total_episodes}\n\n")
            f.write(f"**Task**: {task_intent}\n\n")

    def log_turn(
        self,
        observation: Optional[OrynObservation],
        llm_response: Optional[LLMResponse],
        action: AgentAction,
        result: OrynResult,
        system_prompt: Optional[str] = None,
    ):
        """Log a single turn with LLM input/output and Oryn execution."""
        self.current_turn += 1

        with open(self.filename, "a") as f:
            # Turn header
            f.write(f"### Turn {self.current_turn}\n\n")

            # Observation section
            f.write("#### 👁️ Observation\n\n")
            if observation is None:
                f.write("```\n[First turn - no observation yet]\n```\n\n")
            else:
                # Truncate very long observations
                obs_text = observation.raw
                if len(obs_text) > 2000:
                    obs_text = obs_text[:2000] + "\n... [truncated]"
                f.write("```\n")
                f.write(obs_text)
                f.write("\n```\n\n")
                f.write(f"*URL*: `{observation.url}`  \n")
                f.write(f"*Tokens*: {observation.token_count}  \n\n")

            # LLM Response section
            if llm_response:
                f.write("#### 🤖 Agent Decision\n\n")

                # Show system prompt in collapsible section (only on first turn)
                if self.current_turn == 1 and system_prompt:
                    f.write("<details>\n<summary>System Prompt</summary>\n\n")
                    f.write("```\n")
                    f.write(system_prompt)
                    f.write("\n```\n</details>\n\n")

                # LLM reasoning/thought
                f.write("**LLM Response**:\n\n")
                f.write("```\n")
                f.write(llm_response.content)
                f.write("\n```\n\n")

                # Parsed action
                f.write(f"**Parsed Action**: `{action.command}`  \n")
                if action.reasoning:
                    f.write(f"**Reasoning**: {action.reasoning}  \n")
                f.write(f"**Tokens**: {llm_response.input_tokens} in / {llm_response.output_tokens} out  \n")
                f.write(f"**Cost**: ${llm_response.cost_usd:.6f}  \n")
                f.write(f"**Latency**: {llm_response.latency_ms:.0f}ms  \n\n")

            # Action execution section
            f.write("#### ⚡ Oryn Execution\n\n")
            f.write(f"**Command**: `{action.command}`  \n")

            # Result with status indicator
            if result.success:
                f.write(f"**Result**: ✅ Success  \n")
            else:
                f.write(f"**Result**: ❌ Failed  \n")
                if result.error:
                    f.write(f"**Error**: `{result.error}`  \n")

            # Show raw output if available and non-empty
            if result.raw and result.raw.strip():
                raw_text = result.raw.strip()
                # Truncate very long output
                if len(raw_text) > 500:
                    raw_text = raw_text[:500] + "\n... [truncated]"
                f.write(f"**Output**:\n```\n{raw_text}\n```\n")

            f.write(f"**Latency**: {result.latency_ms:.0f}ms  \n\n")

            f.write("---\n\n")

    def end_episode(self, success: bool, steps: int, duration_ms: float, error: Optional[str] = None):
        """Log episode completion."""
        with open(self.filename, "a") as f:
            status = "✅ **SUCCESS**" if success else "❌ **FAILED**"
            f.write(f"\n### Episode {self.current_episode} Result: {status}\n\n")
            f.write(f"**Steps**: {steps}  \n")
            f.write(f"**Duration**: {duration_ms / 1000:.2f}s  \n")
            if error:
                f.write(f"**Error**: {error}  \n")
            f.write("\n---\n\n")

    def end_task(self, summary: dict):
        """Log final task summary."""
        with open(self.filename, "a") as f:
            f.write("\n## 📊 Final Summary\n\n")

            if summary.get("is_multi_episode"):
                f.write(f"**Total Episodes**: {summary['episodes_count']}  \n")
                f.write(f"**Episodes Succeeded**: {summary['episodes_succeeded']}  \n")
                f.write(f"**Success Rate**: {summary['success_rate']:.1%}  \n")
                f.write(f"**Mean Steps/Episode**: {summary['mean_steps']:.1f}  \n")
                f.write(f"**Total Cost**: ${summary['total_cost']:.4f}  \n")
            else:
                f.write(f"**Success**: {'Yes' if summary['success'] else 'No'}  \n")
                f.write(f"**Steps**: {summary['steps']}  \n")
                f.write(f"**Cost**: ${summary['cost']:.4f}  \n")

            f.write(f"\n**Transcript saved**: `{self.filename}`\n")

    def get_path(self) -> str:
        """Get the transcript file path."""
        return str(self.filename)
