"""Exception hierarchy for Oryn client errors."""


class OrynError(Exception):
    """Base exception for all Oryn-related errors."""

    pass


class BinaryNotFoundError(OrynError):
    """Raised when the oryn binary cannot be found."""

    def __init__(self, searched_paths: list[str] | None = None):
        self.searched_paths = searched_paths or []
        paths_str = "\n  ".join(self.searched_paths) if self.searched_paths else "none"
        super().__init__(
            f"Could not find oryn binary. Searched paths:\n  {paths_str}\n"
            "Set ORYN_BINARY environment variable or install oryn."
        )


class LaunchError(OrynError):
    """Raised when the oryn subprocess fails to start."""

    def __init__(self, message: str, stderr: str | None = None):
        self.stderr = stderr
        full_msg = message
        if stderr:
            full_msg += f"\nStderr: {stderr}"
        super().__init__(full_msg)


class ConnectionLostError(OrynError):
    """Raised when the oryn subprocess dies unexpectedly."""

    def __init__(self, returncode: int | None = None):
        self.returncode = returncode
        msg = "Connection to oryn subprocess lost"
        if returncode is not None:
            msg += f" (exit code: {returncode})"
        super().__init__(msg)


class TimeoutError(OrynError):
    """Raised when an operation times out."""

    def __init__(self, operation: str, timeout_seconds: float):
        self.operation = operation
        self.timeout_seconds = timeout_seconds
        super().__init__(f"Operation '{operation}' timed out after {timeout_seconds}s")


class CommandError(OrynError):
    """Base class for command execution errors."""

    pass


class ParseError(CommandError):
    """Raised when a response cannot be parsed."""

    def __init__(self, message: str, raw_response: str | None = None):
        self.raw_response = raw_response
        super().__init__(message)


class NavigationError(CommandError):
    """Raised when navigation fails."""

    def __init__(self, url: str, message: str):
        self.url = url
        super().__init__(f"Navigation to '{url}' failed: {message}")


class ElementError(OrynError):
    """Base class for element-related errors."""

    pass


class ElementNotFoundError(ElementError):
    """Raised when an element cannot be found."""

    def __init__(self, target: str | int):
        self.target = target
        super().__init__(f"Element not found: {target}")


class ElementNotVisibleError(ElementError):
    """Raised when an element exists but is not visible."""

    def __init__(self, target: str | int):
        self.target = target
        super().__init__(f"Element not visible: {target}")


class ElementDisabledError(ElementError):
    """Raised when trying to interact with a disabled element."""

    def __init__(self, target: str | int):
        self.target = target
        super().__init__(f"Element is disabled: {target}")
