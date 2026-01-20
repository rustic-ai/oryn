import json
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional

from rich.console import Console
from rich.table import Table

from ..collection.metrics import TaskMetrics, TurnMetrics


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

        success_count = sum(1 for r in results if r.success)
        total_cost = sum(r.total_cost_usd for r in results)
        total_steps = sum(r.total_steps for r in results)
        total_duration = sum(r.total_duration_ms for r in results) / 1000.0

        summary = {
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

        table = Table(title="Benchmark Summary")
        table.add_column("Metric", style="cyan")
        table.add_column("Value", style="magenta")

        s = self.summary
        table.add_row("Tasks", str(s["total_tasks"]))
        table.add_row("Success Rate", f"{s['success_rate']:.1%}")
        table.add_row("Mean Cost", f"${s['mean_cost_usd']:.4f}")
        table.add_row("Mean Steps", f"{s['mean_steps']:.1f}")
        table.add_row("Mean Duration", f"{s['mean_duration_s']:.1f}s")

        console.print(table)
