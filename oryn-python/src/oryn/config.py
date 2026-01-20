"""Configuration dataclasses for OrynClient."""

from dataclasses import dataclass, field
from typing import Literal


@dataclass
class OrynConfig:
    """Configuration for OrynClient.

    Attributes:
        mode: Browser mode - 'headless', 'embedded', or 'remote'
        binary_path: Explicit path to oryn binary (optional)
        timeout: Default command timeout in seconds
        connect_timeout: Timeout for initial connection in seconds
        driver_url: WebDriver URL for embedded mode (optional)
        port: WebSocket port for remote mode
        env: Additional environment variables for subprocess
    """

    mode: Literal["headless", "embedded", "remote"] = "headless"
    binary_path: str | None = None
    timeout: float = 30.0
    connect_timeout: float = 60.0
    driver_url: str | None = None
    port: int = 9001
    env: dict[str, str] = field(default_factory=dict)

    def get_cli_args(self) -> list[str]:
        """Generate CLI arguments for oryn binary."""
        args = [self.mode]

        if self.mode == "embedded" and self.driver_url:
            args.extend(["--driver-url", self.driver_url])
        elif self.mode == "remote":
            args.extend(["--port", str(self.port)])

        return args
