from pathlib import Path

import click
from rich.console import Console
from rich.table import Table

from .core.config import RunConfig
from .core.runner import BenchmarkRunner

console = Console()


def _parse_value(value: str):
    """Parse CLI value string to appropriate Python type."""
    # Try boolean
    if value.lower() in ("true", "yes", "1"):
        return True
    if value.lower() in ("false", "no", "0"):
        return False

    # Try integer
    try:
        return int(value)
    except ValueError:
        pass

    # Try float
    try:
        return float(value)
    except ValueError:
        pass

    # Return as string
    return value


@click.group()
def cli():
    """IntentGym Benchmark Harness."""
    pass


@cli.command()
@click.option("--config", required=True, type=Path, help="Path to run configuration")
@click.option("--subset", default="all", help="Task subset to run")
@click.option("--oryn-log-file", default=None, help="File to redirect oryn browser logs to")
@click.option(
    "--oryn-opt",
    multiple=True,
    help="Override oryn options (can be repeated). Format: key=value"
)
def run(config, subset, oryn_log_file, oryn_opt):
    """Run benchmark using configuration file."""
    try:
        run_config = RunConfig.from_yaml(config)

        # Override oryn options if log file provided via CLI
        if oryn_log_file:
            run_config.oryn_options["log_file"] = str(oryn_log_file)

        # Parse and apply --oryn-opt overrides
        for opt in oryn_opt:
            if "=" not in opt:
                console.print(f"[yellow]Warning: Skipping invalid --oryn-opt format: {opt} (expected key=value)[/yellow]")
                continue

            key, value = opt.split("=", 1)
            parsed_value = _parse_value(value)
            run_config.oryn_options[key] = parsed_value
            console.print(f"[dim]Setting oryn option: {key}={parsed_value}[/dim]")
            
        console.print(f"[green]Loaded config for run_id: {run_config.run_id}[/green]")

        runner = BenchmarkRunner(run_config)
        console.print("[yellow]Starting benchmark run...[/yellow]")

        try:
            results = runner.run(
                subset=subset,
                progress_callback=lambda i, n, t: console.print(
                    f"Running task {i + 1}/{n}: {t}"
                ),
            )
        finally:
            runner.close()

        # Generate and save report
        from .core.report import BenchmarkReport

        report = BenchmarkReport.from_results(run_config.run_id, run_config, results)

        # Create results directory if needed
        output_dir = Path("results")
        output_dir.mkdir(exist_ok=True)
        report_path = output_dir / f"{run_config.run_id}.json"

        report.save(report_path)
        console.print(f"\n[green]Results saved to {report_path}[/green]")

        # Print summary
        report.print_summary(console)

    except Exception as e:
        console.print(f"[red]Error:[/red] {e}")


@cli.command()
@click.argument("benchmark", type=click.Choice(["miniwob", "webshop", "webarena"]))
def download(benchmark):
    """Download benchmark resources."""
    console.print(f"[yellow]Downloading resources for {benchmark}...[/yellow]")

    if benchmark == "miniwob":
        console.print("MiniWoB does not require large downloads (uses server URL).")
    elif benchmark == "webshop":
        console.print(
            "Please follow WebShop instructions to download data to ~/.intentgym/webshop"
        )
    elif benchmark == "webarena":
        # In a real impl, this would fetch the zip/tarball
        data_dir = Path("~/.intentgym/webarena").expanduser()
        data_dir.mkdir(parents=True, exist_ok=True)
        console.print(f"[green]Initialized data directory at {data_dir}[/green]")
        console.print("Please manually place WebArena files here for now.")


@cli.command()
@click.argument("run_id")
def inspect(run_id):
    """Inspect a specific run."""
    output_dir = Path("results")
    report_path = output_dir / f"{run_id}.json"

    if not report_path.exists():
        console.print(f"[red]Report not found: {report_path}[/red]")
        return

    import json

    with open(report_path) as f:
        data = json.load(f)

    console.print(f"[bold]Inspect Run: {run_id}[/bold]")
    console.print(f"Success Rate: {data['summary']['success_rate'] * 100:.1f}%")
    console.print(f"Total Cost: ${data['summary']['total_cost_usd']:.4f}")

    table = Table(title="Task Breakdown")
    table.add_column("Task ID", style="cyan")
    table.add_column("Success", style="green")
    table.add_column("Steps", style="magenta")
    table.add_column("Cost", style="yellow")

    for t in data["tasks"]:
        status = "[green]PASS[/green]" if t["success"] else "[red]FAIL[/red]"
        table.add_row(
            t["task_id"], status, str(t["total_steps"]), f"${t['total_cost_usd']:.4f}"
        )

    console.print(table)


@cli.command()
@click.argument("run_ids", nargs=-1)
def compare(run_ids):
    """Compare multiple runs side-by-side."""
    if len(run_ids) < 2:
        console.print("[red]Please provide at least 2 run IDs to compare.[/red]")
        return

    output_dir = Path("results")
    reports = []

    for rid in run_ids:
        path = output_dir / f"{rid}.json"
        if not path.exists():
            console.print(f"[red]Warning: Report {rid} not found, skipping.[/red]")
            continue

        import json

        with open(path) as f:
            reports.append(json.load(f))

    if not reports:
        return

    from rich.table import Table

    table = Table(title="Run Comparison")
    table.add_column("Metric")

    for r in reports:
        table.add_column(r["run_id"])

    metrics = [
        ("Success Rate", lambda d: f"{d['summary']['success_rate'] * 100:.1f}%"),
        ("Avg Cost", lambda d: f"${d['summary']['mean_cost_usd']:.4f}"),
        ("Avg Steps", lambda d: f"{d['summary']['mean_steps']:.1f}"),
        ("Avg Latency", lambda d: f"{d['summary']['mean_duration_s']:.2f}s"),
    ]

    for label, extr in metrics:
        row = [label]
        for r in reports:
            row.append(extr(r))
        table.add_row(*row)

    console.print(table)


cli.add_command(run)
cli.add_command(download)
cli.add_command(inspect)
cli.add_command(compare)

if __name__ == "__main__":
    cli()
