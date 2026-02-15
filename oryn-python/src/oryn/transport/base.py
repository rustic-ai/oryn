"""Abstract base class for transport implementations."""

from abc import ABC, abstractmethod
from typing import Optional


class Transport(ABC):
    """Abstract base class for transports that communicate with oryn."""

    @abstractmethod
    async def connect(self) -> None:
        """Establish connection to oryn.

        Raises:
            LaunchError: If connection cannot be established
            TimeoutError: If connection times out
        """
        pass

    @abstractmethod
    async def send(self, command: str) -> str:
        """Send a command and receive the response.

        Args:
            command: Intent Language command to send

        Returns:
            Response string from oryn

        Raises:
            ConnectionLostError: If connection was lost
            TimeoutError: If command times out
        """
        pass

    @abstractmethod
    async def close(self) -> None:
        """Close the connection and clean up resources."""
        pass

    @abstractmethod
    def is_connected(self) -> bool:
        """Check if the transport is connected.

        Returns:
            True if connected, False otherwise
        """
        pass

    @property
    @abstractmethod
    def binary_path(self) -> Optional[str]:
        """Get the path to the oryn binary being used.

        Returns:
            Path to binary or None if not applicable
        """
        pass
