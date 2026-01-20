"""Script runner for .oil files."""

from pathlib import Path
from typing import List, Tuple

from .types import OrynResult


def parse_oil_file(path: str | Path) -> List[str]:
    """Parse an .oil file and return list of commands.

    Skips empty lines and comments (lines starting with #).

    Args:
        path: Path to .oil file

    Returns:
        List of command strings
    """
    path = Path(path)
    commands = []

    with open(path, "r") as f:
        for line in f:
            line = line.strip()
            # Skip empty lines and comments
            if not line or line.startswith("#"):
                continue
            commands.append(line)

    return commands


async def run_oil_file_async(client, path: str | Path) -> List[Tuple[str, OrynResult]]:
    """Run an .oil file using the async client.

    Args:
        client: OrynClient instance (must be connected)
        path: Path to .oil file

    Returns:
        List of (command, result) tuples
    """
    commands = parse_oil_file(path)
    results = []

    for cmd in commands:
        # Special handling for observe - use observe() method
        if cmd.strip().lower().startswith("observe"):
            obs = await client.observe()
            result = OrynResult(success=True, raw=obs.raw, latency_ms=obs.latency_ms)
        else:
            result = await client.execute(cmd)
        results.append((cmd, result))

    return results


def run_oil_file_sync(client, path: str | Path) -> List[Tuple[str, OrynResult]]:
    """Run an .oil file using the sync client.

    Args:
        client: OrynClientSync instance (must be connected)
        path: Path to .oil file

    Returns:
        List of (command, result) tuples
    """
    commands = parse_oil_file(path)
    results = []

    for cmd in commands:
        # Special handling for observe - use observe() method
        if cmd.strip().lower().startswith("observe"):
            obs = client.observe()
            result = OrynResult(success=True, raw=obs.raw, latency_ms=obs.latency_ms)
        else:
            result = client.execute(cmd)
        results.append((cmd, result))

    return results
