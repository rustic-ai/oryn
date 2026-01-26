"""Async client for Oryn browser automation via Intent Language pass-through."""

from typing import Literal, Optional

from .config import OrynConfig
from .errors import ConnectionLostError
from .transport import SubprocessTransport, Transport


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

    async def execute(self, command: str) -> str:
        """Execute an Intent Language command.

        This is the primary method for interacting with Oryn. Commands are
        passed through directly to the oryn backend without modification.

        Args:
            command: Intent Language command string (e.g., 'goto "https://example.com"')

        Returns:
            The raw string response from Oryn.

        Example:
            ```python
            await client.execute('goto "https://example.com"')
            response = await client.execute('describe')
            print(response)
            ```
        """
        if not self._transport:
            raise ConnectionLostError()

        return await self._transport.send(command)
