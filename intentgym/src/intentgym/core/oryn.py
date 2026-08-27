"""Oryn interface for IntentGym.

This module provides the interface between IntentGym agents and the Oryn
browser automation engine. It wraps the oryn-python client with
IntentGym-specific types and metrics collection.

The interface uses Intent Language pass-through - commands are sent as
strings directly to the oryn backend.
"""

import json
import time
from dataclasses import dataclass, field
from typing import Any, List, Literal, Optional

# Try to import the real oryn client, fall back to mock if not available
try:
    from oryn import BinaryNotFoundError, ConnectionLostError
    from oryn import OrynClientSync as _OrynClientSync
    from oryn import OrynObservation as _RealObservation
    from oryn import OrynResult as _RealResult

    _HAS_ORYN = True
except ImportError:
    _HAS_ORYN = False
    _OrynClientSync = None
    _RealObservation = None
    _RealResult = None
    BinaryNotFoundError = Exception
    ConnectionLostError = Exception


@dataclass
class OrynObservation:
    """Structured observation from Oryn."""

    raw: str
    url: str
    title: str
    elements: List[Any] = field(default_factory=list)
    patterns: List[str] = field(default_factory=list)
    available_intents: List[str] = field(default_factory=list)
    token_count: int = 0
    latency_ms: float = 0.0
    contract_version: Optional[int] = None
    revision: Optional[int] = None
    document_generation: Optional[int] = None
    capabilities: List[Any] = field(default_factory=list)
    diagnostics: List[str] = field(default_factory=list)
    byte_count: int = 0
    execution_domain: Optional[str] = None

    @classmethod
    def from_real(cls, obs: "_RealObservation") -> "OrynObservation":
        """Convert from oryn-python observation."""
        patterns = []
        if obs.patterns:
            if obs.patterns.login:
                patterns.append("login")
            if obs.patterns.search:
                patterns.append("search")
            if obs.patterns.pagination:
                patterns.append("pagination")
            if obs.patterns.modal:
                patterns.append("modal")
            if obs.patterns.cookie_banner:
                patterns.append("cookie_banner")

        intents = [i.name for i in obs.available_intents]

        return cls(
            raw=obs.raw,
            url=obs.url,
            title=obs.title,
            elements=obs.elements,
            patterns=patterns,
            available_intents=intents,
            token_count=obs.token_count,
            latency_ms=obs.latency_ms,
            contract_version=obs.contract_version,
            revision=obs.revision,
            document_generation=obs.document_generation,
            capabilities=obs.capabilities,
            diagnostics=obs.diagnostics,
            byte_count=obs.byte_count,
            execution_domain=obs.execution_domain,
        )


@dataclass
class OrynResult:
    """Result of an Oryn command execution."""

    success: bool
    raw: str
    changes: List[str] = field(default_factory=list)
    error: Optional[str] = None
    latency_ms: float = 0.0
    accepted: bool = False
    classification: str = "failed"
    effects: List[Any] = field(default_factory=list)
    delta: Optional[Any] = None
    diagnostics: List[str] = field(default_factory=list)
    revision_before: Optional[int] = None
    revision_after: Optional[int] = None
    execution_domain: Optional[str] = None
    trace_slice: List[Any] = field(default_factory=list)

    @classmethod
    def from_real(cls, result: "_RealResult") -> "OrynResult":
        """Convert from oryn-python result."""
        return cls(
            success=result.success,
            raw=result.raw,
            changes=result.changes,
            error=result.error,
            latency_ms=result.latency_ms,
            accepted=result.accepted,
            classification=result.classification,
            effects=result.effects,
            delta=result.delta,
            diagnostics=result.diagnostics,
            revision_before=result.revision_before,
            revision_after=result.revision_after,
            execution_domain=result.execution_domain,
        )


class OrynInterface:
    """Interface to the Oryn browser automation engine.

    This class provides a unified interface for IntentGym agents to
    interact with browsers via Oryn. It automatically uses the real
    oryn-python client if available, otherwise falls back to mock mode.

    Commands are passed through as Intent Language strings.

    Args:
        mode: Browser mode - 'native', 'headless', 'embedded', or 'remote'
        use_mock: Force mock mode even if oryn is available
        **options: Additional options passed to the oryn client
    """

    def __init__(
        self,
        mode: Literal["native", "headless", "embedded", "remote"] = "headless",
        use_mock: bool = False,
        **options,
    ):
        self.mode = mode
        self.options = options
        self._use_mock = use_mock or not _HAS_ORYN
        self._client: Optional["_OrynClientSync"] = None
        self._mock_state = {"url": "about:blank", "title": "Blank"}
        self._native_trace_cursor = 0

        # Try to initialize real client
        if not self._use_mock:
            try:
                self._client = _OrynClientSync(mode=mode, **options)
            except BinaryNotFoundError:
                # Fall back to mock if binary not found
                self._use_mock = True
                self._client = None

    def connect(self) -> None:
        """Connect to the oryn backend."""
        if self._client:
            self._client.connect()
            self._native_trace_cursor = 0

    def close(self) -> None:
        """Close the connection."""
        if self._client:
            self._client.close()

    def __enter__(self) -> "OrynInterface":
        """Context manager entry."""
        self.connect()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        """Context manager exit."""
        self.close()

    @property
    def is_mock(self) -> bool:
        """Check if using mock mode."""
        return self._use_mock

    def observe(self, **options) -> OrynObservation:
        """Get structured observation of current page."""
        if self._use_mock:
            return self._mock_observe(**options)

        start = time.time()
        real_obs = self._client.observe(**options)
        # Check for fatal backend errors in the raw response
        if real_obs.raw and (
            "webdriver connection lost" in real_obs.raw
            or "WebDriver session has been closed" in real_obs.raw
        ):
            raise ConnectionLostError(None)

        obs = OrynObservation.from_real(real_obs)
        obs.latency_ms = (time.time() - start) * 1000
        return obs

    def execute(self, command: str) -> OrynResult:
        """Execute an Intent Language command.

        Commands are passed through directly to the oryn backend.

        Args:
            command: Intent Language command string

        Returns:
            OrynResult with success status and raw response

        Example:
            ```python
            oryn.execute('goto "https://example.com"')
            oryn.execute('click "Sign In"')
            oryn.execute('type email "user@example.com"')
            oryn.execute('login "user" "password"')
            ```
        """
        if self._use_mock:
            return self._mock_execute(command)

        start = time.time()

        real_result = (
            self._client.execute_typed(command)
            if self.mode == "native"
            else self._client.execute(command)
        )

        # Check for fatal backend errors in the response string
        if isinstance(real_result, str):
            if (
                "webdriver connection lost" in real_result
                or "WebDriver session has been closed" in real_result
            ):
                raise ConnectionLostError(None)

        duration = (time.time() - start) * 1000

        # If real_result is string, wrap it
        if isinstance(real_result, str):
            success = bool(real_result) and not real_result.strip().lower().startswith(
                "error"
            )
            return OrynResult(
                success=success,
                accepted=success,
                raw=real_result,
                error=None if success else (real_result or "empty backend response"),
                classification="event_only" if success else "failed",
                latency_ms=duration,
            )

        # If it returned an object (future compatibility or if I change client)
        result = OrynResult.from_real(real_result)
        result.latency_ms = duration
        if self.mode == "native" and not command.strip().lower().startswith("trace"):
            try:
                result.trace_slice = self._take_native_trace_slice()
            except Exception as error:
                result.diagnostics.append(f"trace_capture_failed: {error}")
        return result

    def _take_native_trace_slice(self) -> List[Any]:
        """Read the existing OIL trace stream and return only new events."""
        raw = self._client.execute("trace stop")
        events: List[Any] = []
        for line in raw.splitlines():
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(payload, dict) and payload.get("kind") == "trace":
                events = list(payload.get("events") or [])
                break
        if len(events) < self._native_trace_cursor:
            self._native_trace_cursor = 0
        sliced = events[self._native_trace_cursor :]
        self._native_trace_cursor = len(events)
        return sliced

    # Convenience methods
    def goto(self, url: str) -> OrynResult:
        """Navigate to a URL."""
        return self.execute(f'goto "{url}"')

    def click(self, target: str | int) -> OrynResult:
        """Click on an element."""
        if isinstance(target, int):
            return self.execute(f"click {target}")
        return self.execute(f'click "{target}"')

    def type(self, target: str | int, text: str) -> OrynResult:
        """Type text into an element."""
        if isinstance(target, int):
            return self.execute(f'type {target} "{text}"')
        return self.execute(f'type "{target}" "{text}"')

    def select(self, target: str | int, value: str) -> OrynResult:
        """Select an option in a dropdown."""
        if isinstance(target, int):
            return self.execute(f'select {target} "{value}"')
        return self.execute(f'select "{target}" "{value}"')

    def scroll(self, direction: str = "down") -> OrynResult:
        """Scroll the page."""
        return self.execute(f"scroll {direction}")

    def wait(self, condition: str, timeout: int = 30) -> OrynResult:
        """Wait for a condition."""
        return self.execute(f'wait "{condition}" {timeout}')

    # Mock implementations
    def _mock_observe(self, **options) -> OrynObservation:
        """Mock observe implementation."""
        start = time.time()
        obs = OrynObservation(
            raw="[1] Mock Element",
            url=self._mock_state["url"],
            title=self._mock_state["title"],
            token_count=10,
        )
        obs.latency_ms = (time.time() - start) * 1000
        return obs

    def _mock_execute(self, command: str) -> OrynResult:
        """Mock execute implementation."""
        start = time.time()
        success = True
        error = None

        # Parse command for mock state updates
        cmd_lower = command.lower().strip()
        if cmd_lower.startswith("goto "):
            url = command.split(" ", 1)[1].strip().strip('"')
            self._mock_state["url"] = url
            self._mock_state["title"] = f"Page: {url}"
            result_raw = f"Navigated to {url}"
        elif cmd_lower == "fail":
            success = False
            error = "Mock failure"
            result_raw = "Command failed"
        else:
            result_raw = f"Executed: {command}"

        result = OrynResult(success=success, raw=result_raw, error=error)
        result.latency_ms = (time.time() - start) * 1000
        return result
