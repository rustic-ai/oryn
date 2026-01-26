"""Synchronous wrapper for OrynClient."""

import asyncio
from typing import TYPE_CHECKING, Literal, Optional

from .client import OrynClient

if TYPE_CHECKING:
    from .types import OrynObservation


class OrynClientSync:
    """Synchronous client for controlling browsers via Oryn Intent Language.

    This is a synchronous wrapper around the async OrynClient.
    Use this when you don't need async support.

    Example:
        ```python
        with OrynClientSync(mode="headless") as client:
            client.execute('goto "https://example.com"')
            obs = client.observe()
            print(obs.elements)
            client.execute('click "Sign in"')
            client.execute('type email "user@example.com"')
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
        """Initialize OrynClientSync.

        Args:
            mode: Browser mode - 'headless', 'embedded', or 'remote'
            binary_path: Explicit path to oryn binary (optional)
            timeout: Default command timeout in seconds
            connect_timeout: Timeout for initial connection in seconds
            driver_url: WebDriver URL for embedded mode (optional)
            port: WebSocket port for remote mode
            env: Additional environment variables for subprocess
        """
        self._client = OrynClient(
            mode=mode,
            binary_path=binary_path,
            timeout=timeout,
            connect_timeout=connect_timeout,
            driver_url=driver_url,
            port=port,
            env=env,
        )
        self._loop: Optional[asyncio.AbstractEventLoop] = None
        self._owns_loop = False

    def _get_loop(self) -> asyncio.AbstractEventLoop:
        """Get or create an event loop."""
        if self._loop is None or self._loop.is_closed():
            try:
                self._loop = asyncio.get_running_loop()
                self._owns_loop = False
            except RuntimeError:
                self._loop = asyncio.new_event_loop()
                self._owns_loop = True
        return self._loop

    def _run(self, coro):
        """Run a coroutine synchronously."""
        loop = self._get_loop()
        if loop.is_running():
            new_loop = asyncio.new_event_loop()
            try:
                return new_loop.run_until_complete(coro)
            finally:
                new_loop.close()
        else:
            return loop.run_until_complete(coro)

    def connect(self) -> None:
        """Connect to the oryn backend."""
        self._run(self._client.connect())

    def close(self) -> None:
        """Close the connection to oryn."""
        self._run(self._client.close())
        if self._owns_loop and self._loop and not self._loop.is_closed():
            self._loop.close()
            self._loop = None

    def __enter__(self) -> "OrynClientSync":
        """Context manager entry."""
        self.connect()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        """Context manager exit."""
        self.close()

    def is_connected(self) -> bool:
        """Check if client is connected to oryn."""
        return self._client.is_connected()

    def execute(self, command: str) -> str:
        """Execute an Intent Language command.

        Args:
            command: Intent Language command string

        Returns:
            The raw string response from Oryn.
        """
        return self._run(self._client.execute(command))

    def observe(self) -> "OrynObservation":
        """Get structured observation of current page.

        Returns:
            OrynObservation object.
        """
        return self._run(self._client.observe())
