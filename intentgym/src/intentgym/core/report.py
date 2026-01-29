import json
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List

from rich.console import Console
from rich.table import Table

from ..collection.metrics import TaskMetrics


@dataclass
class BenchmarkReport:
    run_id: str
    timestamp: str
    config: Any
    tasks: List[TaskMetrics]
    summary: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_results(
        cls, run_id: str, config: Any, results: List[TaskMetrics]
    ) -> "BenchmarkReport":
        # Calculate aggregate metrics
        total_tasks = len(results)
        if total_tasks == 0:
            return cls(
                run_id=run_id,
                timestamp=datetime.utcnow().isoformat(),
                config=config,
                tasks=[],
                summary={},
            )

        # Check if this is a multi-episode run
        is_multi_episode = any(r.is_multi_episode for r in results)

        if is_multi_episode:
            # Multi-episode summary
            total_episodes = sum(r.episodes_count for r in results)
            episodes_succeeded = sum(r.episodes_succeeded for r in results)
            overall_success_rate = (
                episodes_succeeded / total_episodes if total_episodes > 0 else 0.0
            )

            # Mean task-level success rate
            task_success_rates = [r.success_rate for r in results if r.success_rate is not None]
            mean_task_success_rate = (
                sum(task_success_rates) / len(task_success_rates)
                if task_success_rates
                else 0.0
            )

            # Aggregate costs and steps
            total_cost = sum(r.total_cost_usd for r in results)
            total_duration = sum(r.total_duration_ms for r in results) / 1000.0

            # Mean per-episode values
            mean_steps_per_episode = (
                sum(r.mean_steps_per_episode or 0 for r in results) / total_tasks
            )
            mean_cost_per_episode = (
                sum(r.mean_cost_per_episode or 0 for r in results) / total_tasks
            )
            mean_duration_per_episode = (
                sum(r.mean_duration_per_episode or 0 for r in results) / total_tasks / 1000.0
            )

            total_timeouts = sum(r.timeout_count for r in results)

            # Per-task breakdown
            task_breakdown = []
            for r in results:
                task_breakdown.append(
                    {
                        "task_id": r.task_id,
                        "success_rate": r.success_rate,
                        "episodes_count": r.episodes_count,
                        "episodes_succeeded": r.episodes_succeeded,
                        "mean_steps": r.mean_steps_per_episode,
                        "mean_cost_usd": r.mean_cost_per_episode,
                        "timeout_count": r.timeout_count,
                    }
                )

            summary = {
                "is_multi_episode": True,
                "total_tasks": total_tasks,
                "total_episodes": total_episodes,
                "episodes_succeeded": episodes_succeeded,
                "overall_success_rate": overall_success_rate,
                "mean_task_success_rate": mean_task_success_rate,
                "mean_steps_per_episode": mean_steps_per_episode,
                "mean_cost_per_episode": mean_cost_per_episode,
                "mean_duration_per_episode_s": mean_duration_per_episode,
                "total_cost_usd": total_cost,
                "total_duration_s": total_duration,
                "total_timeouts": total_timeouts,
                "task_breakdown": task_breakdown,
            }
        else:
            # Single-episode summary (original behavior)
            success_count = sum(1 for r in results if r.success)
            total_cost = sum(r.total_cost_usd for r in results)
            total_steps = sum(r.total_steps for r in results)
            total_duration = sum(r.total_duration_ms for r in results) / 1000.0

            summary = {
                "is_multi_episode": False,
                "total_tasks": total_tasks,
                "success_count": success_count,
                "success_rate": success_count / total_tasks,
                "mean_steps": total_steps / total_tasks,
                "mean_cost_usd": total_cost / total_tasks,
                "mean_duration_s": total_duration / total_tasks,
                "total_cost_usd": total_cost,
                "total_duration_s": total_duration,
            }

        return cls(
            run_id=run_id,
            timestamp=datetime.utcnow().isoformat(),
            config=config,
            tasks=results,
            summary=summary,
        )

    def save(self, path: Path):
        """Save report to JSON file."""
        # Convert dataclasses to dicts
        # We need a custom encoder or logic to handle any non-serializable objects if present
        # but pure dataclasses should be fine with asdict

        # Helper to convert config object if it's not a dict (it's a RunConfig dataclass)
        config_dict = (
            asdict(self.config) if hasattr(self.config, "run_id") else self.config
        )

        data = {
            "run_id": self.run_id,
            "timestamp": self.timestamp,
            "config": config_dict,
            "summary": self.summary,
            "tasks": [asdict(t) for t in self.tasks],
        }

        with open(path, "w") as f:
            json.dump(data, f, indent=2)

    def print_summary(self, console: Console):
        """Print summary table to console."""
        console.print(f"\n[bold]Run ID:[/bold] {self.run_id}")
        console.print(f"[bold]Timestamp:[/bold] {self.timestamp}")

        s = self.summary

        if s.get("is_multi_episode", False):
            # Multi-episode summary
            table = Table(title="Multi-Episode Benchmark Summary")
            table.add_column("Metric", style="cyan")
            table.add_column("Value", style="magenta")

            table.add_row("Total Tasks", str(s["total_tasks"]))
            table.add_row("Total Episodes", str(s["total_episodes"]))
            table.add_row("Episodes Succeeded", str(s["episodes_succeeded"]))
            table.add_row("Overall Success Rate", f"{s['overall_success_rate']:.1%}")
            table.add_row("Mean Task Success Rate", f"{s['mean_task_success_rate']:.1%}")
            table.add_row("Mean Steps/Episode", f"{s['mean_steps_per_episode']:.1f}")
            table.add_row(
                "Mean Cost/Episode", f"${s['mean_cost_per_episode']:.4f}"
            )
            table.add_row(
                "Mean Duration/Episode", f"{s['mean_duration_per_episode_s']:.2f}s"
            )
            table.add_row("Total Cost", f"${s['total_cost_usd']:.2f}")
            table.add_row("Total Duration", f"{s['total_duration_s']:.1f}s")
            table.add_row("Timeouts", str(s["total_timeouts"]))

            console.print(table)

            # Per-task breakdown
            if s.get("task_breakdown"):
                task_table = Table(title="Per-Task Results")
                task_table.add_column("Task ID", style="cyan")
                task_table.add_column("Success Rate", style="magenta")
                task_table.add_column("Episodes", style="green")
                task_table.add_column("Mean Steps", style="yellow")
                task_table.add_column("Timeouts", style="red")

                for task_info in s["task_breakdown"]:
                    task_table.add_row(
                        task_info["task_id"],
                        f"{task_info['success_rate']:.1%}",
                        f"{task_info['episodes_succeeded']}/{task_info['episodes_count']}",
                        f"{task_info['mean_steps']:.1f}",
                        str(task_info["timeout_count"]),
                    )

                console.print(task_table)
        else:
            # Single-episode summary (original)
            table = Table(title="Benchmark Summary")
            table.add_column("Metric", style="cyan")
            table.add_column("Value", style="magenta")

            table.add_row("Tasks", str(s["total_tasks"]))
            table.add_row("Success Rate", f"{s['success_rate']:.1%}")
            table.add_row("Mean Cost", f"${s['mean_cost_usd']:.4f}")
            table.add_row("Mean Steps", f"{s['mean_steps']:.1f}")
            table.add_row("Mean Duration", f"{s['mean_duration_s']:.1f}s")

            console.print(table)
