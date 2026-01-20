"""Async client for Oryn browser automation via Intent Language pass-through."""

import re
import time
from typing import Literal, Optional

from .config import OrynConfig
from .errors import ConnectionLostError
from .parser import parse_observation
from .transport import SubprocessTransport, Transport
from .types import OrynObservation, OrynResult


class OrynClient:
    """Async client for controlling browsers via Oryn Intent Language.

    This is a thin pass-through layer that sends Intent Language commands
    to the oryn backend and returns structured responses. Commands are
    passed through as-is - the client does not interpret or modify them.

    Example:
        ```python
        async with OrynClient(mode="headless") as client:
            await client.execute('goto "https://example.com"')
            obs = await client.observe()
            print(obs.elements)
            await client.execute('click "Sign in"')
            await client.execute('type email "user@example.com"')
        ```
    """

    def __init__(
        self,
        mode: Literal["headless", "embedded", "remote"] = "headless",
        *,
        binary_path: str | None = None,
        timeout: float = 30.0,
        connect_timeout: float = 60.0,
        driver_url: str | None = None,
        port: int = 9001,
        env: dict[str, str] | None = None,
    ):
        """Initialize OrynClient.

        Args:
            mode: Browser mode - 'headless', 'embedded', or 'remote'
            binary_path: Explicit path to oryn binary (optional)
            timeout: Default command timeout in seconds
            connect_timeout: Timeout for initial connection in seconds
            driver_url: WebDriver URL for embedded mode (optional)
            port: WebSocket port for remote mode
            env: Additional environment variables for subprocess
        """
        self._config = OrynConfig(
            mode=mode,
            binary_path=binary_path,
            timeout=timeout,
            connect_timeout=connect_timeout,
            driver_url=driver_url,
            port=port,
            env=env or {},
        )
        self._transport: Optional[Transport] = None
        self._last_observation: Optional[OrynObservation] = None

    async def connect(self) -> None:
        """Connect to the oryn backend.

        This method is called automatically when using the async context manager.
        """
        self._transport = SubprocessTransport(self._config)
        await self._transport.connect()

    async def close(self) -> None:
        """Close the connection to oryn.

        This method is called automatically when using the async context manager.
        """
        if self._transport:
            await self._transport.close()
            self._transport = None

    async def __aenter__(self) -> "OrynClient":
        """Async context manager entry."""
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        """Async context manager exit."""
        await self.close()

    def is_connected(self) -> bool:
        """Check if client is connected to oryn."""
        return self._transport is not None and self._transport.is_connected()

    @property
    def last_observation(self) -> Optional[OrynObservation]:
        """Get the most recent observation."""
        return self._last_observation

    async def execute(self, command: str) -> OrynResult:
        """Execute an Intent Language command.

        This is the primary method for interacting with Oryn. Commands are
        passed through directly to the oryn backend without modification.

        Args:
            command: Intent Language command string (e.g., 'goto "https://example.com"',
                    'click "Sign In"', 'type email "user@test.com"')

        Returns:
            OrynResult with success status and raw response

        Example:
            ```python
            # Navigation
            await client.execute('goto "https://example.com"')
            await client.execute('back')
            await client.execute('refresh')

            # Actions
            await client.execute('click "Sign In"')
            await client.execute('click 5')  # By element ID
            await client.execute('type email "user@example.com"')
            await client.execute('type "Password" "secret123"')
            await client.execute('select "Country" "United States"')
            await client.execute('check "Remember me"')

            # Composite commands
            await client.execute('login "user@example.com" "password123"')
            await client.execute('search "query"')
            await client.execute('dismiss modal')
            await client.execute('accept cookies')

            # Waiting
            await client.execute('wait visible "Success"')
            await client.execute('wait load')

            # Scrolling
            await client.execute('scroll down')
            await client.execute('scroll until "Footer"')
            ```
        """
        if not self._transport:
            raise ConnectionLostError()

        start = time.time()
        raw = await self._transport.send(command)
        latency = (time.time() - start) * 1000

        # Determine success based on response
        success = not self._is_error_response(raw)

        return OrynResult(
            success=success,
            raw=raw,
            error=raw if not success else None,
            latency_ms=latency,
        )

    async def observe(self, **options) -> OrynObservation:
        """Execute 'observe' command and return structured observation.

        This is a convenience method that executes the observe command
        and parses the response into a structured OrynObservation object.

        Args:
            **options: Options passed to observe command (e.g., full=True, minimal=True)

        Returns:
            OrynObservation with parsed elements and patterns

        Example:
            ```python
            obs = await client.observe()
            print(f"URL: {obs.url}")
            print(f"Title: {obs.title}")
            print(f"Elements: {len(obs.elements)}")

            # Find elements
            elem = obs.find_by_text("Sign In")
            email_inputs = obs.find_by_role("email")

            # Check patterns
            if obs.has_login_form():
                print("Login form detected")
            ```
        """
        start = time.time()

        # Build observe command with options
        cmd = "observe"
        for key, value in options.items():
            if value is True:
                cmd += f" --{key}"
            elif value:
                cmd += f" --{key} {value}"

        if not self._transport:
            raise ConnectionLostError()

        raw = await self._transport.send(cmd)
        observation = parse_observation(raw)
        observation.latency_ms = (time.time() - start) * 1000
        self._last_observation = observation
        return observation

    def _is_error_response(self, raw: str) -> bool:
        """Check if a response indicates an error.

        This method needs to distinguish between:
        - Actual errors (like "Error: element not found")
        - Log noise (like "[31mERROR[0m chromiumoxide::conn...")

        We do this by:
        1. Stripping ANSI escape codes
        2. Looking at the last meaningful line for actual error messages
        3. Ignoring log output from the Rust runtime
        """
        # Strip ANSI escape codes
        ansi_escape = re.compile(r"\x1b\[[0-9;]*m")
        clean = ansi_escape.sub("", raw)

        # Split into lines and filter out timestamp-prefixed log lines
        lines = [line.strip() for line in clean.split("\n") if line.strip()]

        # Filter out log lines (they start with timestamps like "2026-01-20T...")
        result_lines = []
        for line in lines:
            # Skip log output (timestamp prefixed or has module path like "chromiumoxide::")
            if re.match(r"^\d{4}-\d{2}-\d{2}T", line):
                continue
            if "::" in line and ("ERROR" in line or "WARN" in line or "INFO" in line):
                continue
            result_lines.append(line)

        # Check the result lines for actual errors
        for line in result_lines:
            lower = line.lower()
            # Actual error patterns from oryn
            if lower.startswith("error:"):
                return True
            if "unknown command:" in lower:
                return True
            if "element not found" in lower:
                return True
            if "navigation failed" in lower:
                return True

        return False
