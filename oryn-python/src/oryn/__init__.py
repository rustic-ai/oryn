"""Oryn Python Client - Intent Language pass-through for browser automation.

This library provides a thin pass-through layer to the Oryn browser automation
engine. Commands are sent as Intent Language strings directly to the oryn backend.

Example (async):
    ```python
    from oryn import OrynClient

    async with OrynClient(mode="headless") as client:
        await client.execute('goto "https://example.com"')
        obs = await client.observe()
        print(f"Found {len(obs.elements)} elements")
        await client.execute('click "Sign in"')
        await client.execute('type email "user@example.com"')
    ```

Example (sync):
    ```python
    from oryn import OrynClientSync

    with OrynClientSync(mode="headless") as client:
        client.execute('goto "https://example.com"')
        obs = client.observe()
        print(f"Found {len(obs.elements)} elements")
        client.execute('click "Sign in"')
    ```
"""

__version__ = "0.1.0"

# Main clients
from .client import OrynClient

# Configuration
from .config import OrynConfig

# Discovery utilities
from .discovery import find_binary, get_binary_version, validate_binary

# Errors
from .errors import (
    BinaryNotFoundError,
    CommandError,
    ConnectionLostError,
    LaunchError,
    OrynError,
    ParseError,
    TimeoutError,
)

# Script runner
from .script import parse_oil_file, run_oil_file_async, run_oil_file_sync
from .sync import OrynClientSync
from .types import OrynObservation, OrynResult

__all__ = [
    # Version
    "__version__",
    # Clients
    "OrynClient",
    "OrynClientSync",
    # Config
    "OrynConfig",
    # Errors
    "OrynError",
    "BinaryNotFoundError",
    "LaunchError",
    "ConnectionLostError",
    "TimeoutError",
    "CommandError",
    "ParseError",
    # Discovery
    "find_binary",
    "validate_binary",
    "get_binary_version",
    # Script runner
    "parse_oil_file",
    "run_oil_file_async",
    "run_oil_file_sync",
    # Types
    "OrynObservation",
    "OrynResult",
]
