"""Async client for Oryn browser automation via Intent Language pass-through."""

from typing import TYPE_CHECKING, Any, Literal, Optional

from .config import OrynConfig
from .errors import ConnectionLostError
from .transport import SubprocessTransport, Transport

if TYPE_CHECKING:
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
        mode: Literal["native", "headless", "embedded", "remote"] = "headless",
        *,
        binary_path: str | None = None,
        timeout: float = 30.0,
        connect_timeout: float = 60.0,
        driver_url: str | None = None,
        port: int = 9001,
        env: dict[str, str] | None = None,
        log_file: str | None = None,
        cli_args: list[str] | None = None,
    ):
        """Initialize OrynClient.

        Args:
            mode: Browser mode - 'native', 'headless', 'embedded', or 'remote'
            binary_path: Explicit path to oryn binary (optional)
            timeout: Default command timeout in seconds
            connect_timeout: Timeout for initial connection in seconds
            driver_url: WebDriver URL for embedded mode (optional)
            port: WebSocket port for remote mode
            env: Additional environment variables for subprocess
            log_file: Path to file for redirecting Oryn output (optional)
            cli_args: Additional CLI arguments to pass to oryn binary (optional)
        """
        self._config = OrynConfig(
            mode=mode,
            binary_path=binary_path,
            timeout=timeout,
            connect_timeout=connect_timeout,
            driver_url=driver_url,
            port=port,
            env=env or {},
            log_file=log_file,
            cli_args=cli_args or [],
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

    async def execute_typed(self, command: str) -> "OrynResult":
        """Execute OIL and decode native JSON without changing ``execute()`` compatibility."""
        from .types import OrynDelta, OrynEffect, OrynResult

        raw_response = await self.execute(command)
        if self._config.mode != "native":
            failed = not raw_response or raw_response.strip().lower().startswith("error")
            return OrynResult(
                success=not failed,
                accepted=not failed,
                raw=raw_response,
                error=raw_response if failed else None,
                classification="failed" if failed else "event_only",
            )

        payload = _first_json_object(raw_response)
        if payload is None:
            return OrynResult(
                success=False,
                accepted=False,
                raw=raw_response,
                error="native command returned no JSON result",
                classification="failed",
            )

        kind = payload.get("kind")
        if kind == "unsupported":
            diagnostic = payload.get("diagnostic") or {}
            detail = diagnostic.get("detail") or diagnostic.get("capability") or "unsupported"
            return OrynResult(
                success=False,
                accepted=False,
                raw=raw_response,
                error=str(detail),
                diagnostics=[str(detail)],
                classification="unsupported",
                execution_domain="native",
            )
        if kind == "navigation":
            navigation = payload.get("navigation") or {}
            return OrynResult(
                success=True,
                accepted=True,
                raw=raw_response,
                classification="navigation",
                revision_after=_as_int(navigation.get("revision")),
                execution_domain="native",
            )
        if kind != "action":
            return OrynResult(
                success=True,
                accepted=True,
                raw=raw_response,
                classification="event_only",
                execution_domain="native",
            )

        action = payload.get("result") or {}
        effects = [
            OrynEffect(kind=str(effect.get("kind", "unknown")), data=dict(effect))
            for effect in action.get("effects") or []
            if isinstance(effect, dict)
        ]
        delta_payload = action.get("delta")
        delta = None
        if isinstance(delta_payload, dict):
            delta = OrynDelta(
                contract_version=int(delta_payload.get("contract_version", 0)),
                from_revision=int(delta_payload.get("from_revision", 0)),
                to_revision=int(delta_payload.get("to_revision", 0)),
                upserted=list(delta_payload.get("upserted") or []),
                removed=list(delta_payload.get("removed") or []),
            )
        effect_kinds = {effect.kind for effect in effects}
        if "navigation" in effect_kinds:
            classification = "navigation"
        elif "request" in effect_kinds or "response" in effect_kinds:
            classification = "request_effect"
        elif delta is not None or "dom_mutation" in effect_kinds:
            classification = "state_changed"
        else:
            classification = "event_only"
        diagnostics = [str(item) for item in action.get("diagnostics") or []]
        return OrynResult(
            success=True,
            accepted=True,
            raw=raw_response,
            classification=classification,
            effects=effects,
            delta=delta,
            diagnostics=diagnostics,
            revision_before=_as_int(action.get("revision_before")),
            revision_after=_as_int(action.get("revision_after")),
            execution_domain=action.get("execution_domain"),
        )

    async def observe(self) -> "OrynObservation":
        """Get structured observation of current page.

        Returns:
            OrynObservation object.
        """

        from .types import OrynObservation

        if self._config.mode == "native":
            return await self._observe_native()

        # 'scan' returns the element list in OIL text format
        raw_response = await self.execute("scan")

        # OIL format: [id] type/role "label" {flags}
        # e.g. [1] input/email "Username" {required}
        import re

        elements = []
        element_pattern = re.compile(r'^\[(\d+)\]\s+([^\s"]+)(?:\s+"([^"]*)")?(?:\s+\{(.*)\})?')

        lines = raw_response.splitlines()
        page_info = {"url": "", "title": ""}

        for line in lines:
            if line.startswith("@ "):
                parts = line[2:].split(" ", 1)
                page_info["url"] = parts[0]
                if len(parts) >= 2:
                    page_info["title"] = parts[1].strip('"')
                continue

            match = element_pattern.match(line)
            if match:
                eid, type_role, label, flags = match.groups()
                # Split type/role
                if "/" in type_role:
                    etype, role = type_role.split("/", 1)
                else:
                    etype, role = type_role, None

                elements.append(
                    {
                        "id": int(eid),
                        "type": etype,
                        "role": role,
                        "text": label if label else None,
                        "state": {f: True for f in (flags.split(", ") if flags else [])},
                    }
                )

        return OrynObservation(
            raw=raw_response,
            url=page_info["url"],
            title=page_info["title"],
            elements=elements,
            token_count=len(raw_response) // 4,
            byte_count=len(raw_response.encode("utf-8")),
        )

    async def _observe_native(self) -> "OrynObservation":
        """Decode the existing native JSON OIL response into the SDK type."""
        import json

        from .types import OrynObservation

        raw_response = await self.execute("observe")
        payload = None
        for line in raw_response.splitlines():
            try:
                candidate = json.loads(line)
            except json.JSONDecodeError:
                continue
            if candidate.get("kind") == "observation":
                payload = candidate.get("observation")
                break
        if not isinstance(payload, dict):
            raise ConnectionLostError()

        page = payload.get("page") or {}
        elements = []
        for node in payload.get("nodes") or []:
            elements.append(
                {
                    "id": node.get("alias"),
                    "type": node.get("role"),
                    "role": node.get("role"),
                    "text": node.get("name") or None,
                    "state": {name: True for name in node.get("states") or []},
                    "actions": node.get("actions") or [],
                    "semantic_ref": node.get("semantic_ref"),
                    "parent": node.get("parent"),
                    "document_order": node.get("document_order"),
                    "selector": node.get("selector"),
                    "value": node.get("value"),
                    "description": node.get("description"),
                    "provenance": node.get("provenance"),
                }
            )

        return OrynObservation(
            raw=raw_response,
            url=page.get("url", ""),
            title=page.get("title", ""),
            elements=elements,
            token_count=len(raw_response) // 4,
            contract_version=_as_int(payload.get("contract_version")),
            revision=_as_int(payload.get("revision")),
            document_generation=_as_int(page.get("document_generation")),
            capabilities=[_capability(item) for item in payload.get("capabilities") or []],
            diagnostics=[str(item) for item in payload.get("diagnostics") or []],
            byte_count=len(raw_response.encode("utf-8")),
            execution_domain=payload.get("execution_domain"),
        )


def _first_json_object(raw_response: str) -> Optional[dict[str, Any]]:
    import json

    for line in raw_response.splitlines():
        try:
            candidate = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(candidate, dict):
            return candidate
    return None


def _as_int(value: Any) -> Optional[int]:
    return value if isinstance(value, int) else None


def _capability(value: dict[str, Any]):
    from .types import OrynCapabilityDiagnostic

    return OrynCapabilityDiagnostic(
        capability=str(value.get("capability", "")),
        support=str(value.get("support", "unsupported")),
        alternatives=[str(item) for item in value.get("alternatives") or []],
        handoff_lossy=bool(value.get("handoff_lossy", False)),
        detail=value.get("detail"),
    )
