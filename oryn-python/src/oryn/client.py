"""Async client for Oryn browser automation via Intent Language pass-through."""

from typing import TYPE_CHECKING, Any, Literal, Optional

from .config import OrynConfig
from .errors import CommandError, ConnectionLostError, ParseError
from .transport import SubprocessTransport, Transport

if TYPE_CHECKING:
    from .types import OrynIntent, OrynIntentResult, OrynObservation, OrynResult


class OrynClient:
    """Async client for controlling browsers via Oryn Intent Language.

    This is a thin pass-through layer that sends Intent Language commands
    to the oryn backend and returns structured responses. Commands are
    passed through as-is - the client does not interpret or modify them.

    Example:
        ```python
        async with OrynClient() as client:
            await client.execute('goto "https://example.com"')
            obs = await client.observe()
            print(obs.elements)
            await client.execute('click "Sign in"')
            await client.execute('type email "user@example.com"')
        ```
    """

    def __init__(
        self,
        mode: Literal["native", "headless", "embedded", "remote"] = "native",
        *,
        binary_path: str | None = None,
        timeout: float = 30.0,
        connect_timeout: float = 60.0,
        driver_url: str | None = None,
        port: int = 9001,
        env: dict[str, str] | None = None,
        log_file: str | None = None,
        cli_args: list[str] | None = None,
        secret_values: dict[str, dict[str, str]] | None = None,
        upload_paths: dict[str, str] | None = None,
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
            secret_values: Origin -> label -> secret values provisioned through
                an inherited pipe, never argv or environment variables
            upload_paths: Upload label -> parent-readable host path
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
            secret_values=secret_values or {},
            upload_paths=upload_paths or {},
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
        from .types import OrynResult

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

        return _decode_action_result(payload.get("result") or {}, raw_response)

    async def perform(self, intent: "OrynIntent") -> "OrynIntentResult":
        """Execute the shared typed agent contract without constructing OIL text.

        Typed intents are currently a native-runtime contract. Compatibility
        modes remain available through :meth:`execute` and cannot silently
        reinterpret this request.
        """
        import json

        from .types import OrynIntentResult

        if self._config.mode != "native":
            raise CommandError("typed intents require native mode")
        request = json.dumps(intent.to_request(), separators=(",", ":"))
        raw_response = await self.execute(f"@oryn-intent {request}")
        payload = _first_json_object(raw_response)
        if not isinstance(payload, dict) or payload.get("kind") != "intent":
            raise ParseError("native command returned no typed intent result", raw_response)
        result = payload.get("result")
        if not isinstance(result, dict):
            raise ParseError("typed intent result is malformed", raw_response)
        kind = result.get("kind")
        if kind == "semantic_observation":
            observation = result.get("observation")
            if not isinstance(observation, dict):
                raise ParseError("typed semantic observation is malformed", raw_response)
            return OrynIntentResult(
                kind="semantic_observation",
                raw=raw_response,
                semantic_observation=_decode_semantic_observation(observation, raw_response),
            )
        if kind == "semantic_observation_v6":
            observation = result.get("observation")
            if not isinstance(observation, dict):
                raise ParseError("typed v6 semantic observation is malformed", raw_response)
            return OrynIntentResult(
                kind="semantic_observation_v6",
                raw=raw_response,
                semantic_observation_v6=_decode_semantic_observation_v6(observation, raw_response),
            )
        if kind == "semantic_delta_v6":
            delta = result.get("delta")
            if not isinstance(delta, dict):
                raise ParseError("typed v6 semantic delta is malformed", raw_response)
            from .types import OrynSemanticDeltaV6

            return OrynIntentResult(
                kind="semantic_delta_v6",
                raw=raw_response,
                semantic_delta_v6=OrynSemanticDeltaV6(
                    contract_version=int(delta.get("contract_version", 0)),
                    from_revision=int(delta.get("from_revision", 0)),
                    to_revision=int(delta.get("to_revision", 0)),
                    semantic_revision=int(delta.get("semantic_revision", 0)),
                    changes=dict(delta.get("changes") or {}),
                    diagnostics=[str(item) for item in delta.get("diagnostics") or []],
                ),
            )
        if kind == "observation" or kind == "geometry_observation":
            observation = result.get("observation")
            if not isinstance(observation, dict):
                raise ParseError("typed observation result is malformed", raw_response)
            return OrynIntentResult(
                kind=kind,
                raw=raw_response,
                observation=_decode_native_observation(observation, raw_response),
            )
        if kind == "delta":
            delta = result.get("delta")
            if not isinstance(delta, dict):
                raise ParseError("typed delta result is malformed", raw_response)
            return OrynIntentResult(
                kind="delta",
                raw=raw_response,
                delta=_decode_delta(delta),
            )
        if kind == "action":
            action = result.get("result")
            if not isinstance(action, dict):
                raise ParseError("typed action result is malformed", raw_response)
            return OrynIntentResult(
                kind="action",
                raw=raw_response,
                action=_decode_action_result(action, raw_response),
            )
        if kind == "navigation":
            navigation = result.get("navigation")
            if not isinstance(navigation, dict):
                raise ParseError("typed navigation result is malformed", raw_response)
            return OrynIntentResult(
                kind="navigation",
                raw=raw_response,
                navigation=navigation,
            )
        if kind in {"structured", "text"}:
            return OrynIntentResult(kind=kind, raw=raw_response, value=result.get("value"))
        if kind == "wait":
            return OrynIntentResult(
                kind="wait",
                raw=raw_response,
                revision=_as_int(result.get("revision")),
            )
        raise ParseError(f"unknown typed intent result kind: {kind!r}", raw_response)

    async def observe(
        self,
        *,
        positions: bool = False,
        viewport: bool = False,
        hidden: bool = False,
        full: bool = False,
    ) -> "OrynObservation":
        """Get structured observation of current page.

        Returns:
            OrynObservation object.
        """

        from .types import OrynObservation

        if self._config.mode == "native":
            return await self._observe_native(
                positions=positions,
                viewport=viewport,
                hidden=hidden,
                full=full,
            )

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

    async def _observe_native(
        self,
        *,
        positions: bool = False,
        viewport: bool = False,
        hidden: bool = False,
        full: bool = False,
    ) -> "OrynObservation":
        """Decode the existing native JSON OIL response into the SDK type."""
        import json

        flags = []
        if positions:
            flags.append("--positions")
        if viewport:
            flags.append("--viewport")
        if hidden:
            flags.append("--hidden")
        if full:
            flags.append("--full")
        raw_response = await self.execute(" ".join(["observe", *flags]))
        payload = None
        for line in raw_response.splitlines():
            try:
                candidate = json.loads(line)
            except json.JSONDecodeError:
                continue
            if candidate.get("kind") in {"observation", "geometry_observation"}:
                payload = candidate.get("observation")
                break
        if not isinstance(payload, dict):
            raise ConnectionLostError()

        return _decode_native_observation(payload, raw_response)


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


def _decode_native_observation(payload: dict[str, Any], raw_response: str):
    from .types import OrynObservation, OrynScroll, OrynViewport

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
                "content_trust": node.get("content_trust", "page_authored"),
                "provenance": node.get("provenance"),
                "geometry": node.get("geometry"),
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
        style_revision=_revision_value(payload.get("style_revision")),
        layout_revision=_revision_value(payload.get("layout_revision")),
        viewport=(
            OrynViewport(**payload["viewport"])
            if isinstance(payload.get("viewport"), dict)
            else None
        ),
        scroll=(
            OrynScroll(**payload["scroll"]) if isinstance(payload.get("scroll"), dict) else None
        ),
    )


def _decode_delta(payload: dict[str, Any]):
    from .types import OrynDelta

    return OrynDelta(
        contract_version=int(payload.get("contract_version", 0)),
        from_revision=int(payload.get("from_revision", 0)),
        to_revision=int(payload.get("to_revision", 0)),
        upserted=list(payload.get("upserted") or []),
        removed=list(payload.get("removed") or []),
        style_revision=_revision_value(payload.get("style_revision")),
        layout_revision=_revision_value(payload.get("layout_revision")),
    )


def _decode_semantic_observation(payload: dict[str, Any], raw_response: str):
    from .types import (
        OrynAccessibilityNode,
        OrynSemanticObservation,
        OrynSemanticStructure,
    )

    base = payload.get("observation")
    if not isinstance(base, dict):
        raise ParseError("semantic observation has no base observation", raw_response)
    return OrynSemanticObservation(
        contract_version=int(payload.get("contract_version", 0)),
        observation=_decode_native_observation(base, raw_response),
        accessibility=[
            OrynAccessibilityNode(**node)
            for node in payload.get("accessibility") or []
            if isinstance(node, dict)
        ],
        structures=[
            OrynSemanticStructure(**structure)
            for structure in payload.get("structures") or []
            if isinstance(structure, dict)
        ],
        diagnostics=[str(item) for item in payload.get("diagnostics") or []],
    )


def _decode_semantic_observation_v6(payload: dict[str, Any], raw_response: str):
    from .types import (
        OrynAccessibilityNode,
        OrynSemanticObservationV6,
        OrynSemanticStructure,
    )

    return OrynSemanticObservationV6(
        contract_version=int(payload.get("contract_version", 0)),
        observation=_decode_native_observation(payload, raw_response),
        semantic_revision=int(payload.get("semantic_revision", 0)),
        nodes=[dict(node) for node in payload.get("nodes") or [] if isinstance(node, dict)],
        accessibility=[
            OrynAccessibilityNode(**node)
            for node in payload.get("accessibility") or []
            if isinstance(node, dict)
        ],
        structures=[
            OrynSemanticStructure(**structure)
            for structure in payload.get("structures") or []
            if isinstance(structure, dict)
        ],
        projection=dict(payload.get("projection") or {}),
        diagnostics=[str(item) for item in payload.get("diagnostics") or []],
    )


def _decode_action_result(action: dict[str, Any], raw_response: str):
    from .types import OrynEffect, OrynResult

    effects = [
        OrynEffect(kind=str(effect.get("kind", "unknown")), data=dict(effect))
        for effect in action.get("effects") or []
        if isinstance(effect, dict)
    ]
    delta_payload = action.get("delta")
    delta = _decode_delta(delta_payload) if isinstance(delta_payload, dict) else None
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


def _as_int(value: Any) -> Optional[int]:
    return value if isinstance(value, int) else None


def _revision_value(value: Any) -> Optional[int]:
    if isinstance(value, int):
        return value
    if isinstance(value, dict):
        nested = value.get("0") or value.get("value")
        return nested if isinstance(nested, int) else None
    return None


def _capability(value: dict[str, Any]):
    from .types import OrynCapabilityDiagnostic

    return OrynCapabilityDiagnostic(
        capability=str(value.get("capability", "")),
        support=str(value.get("support", "unsupported")),
        alternatives=[str(item) for item in value.get("alternatives") or []],
        handoff_lossy=bool(value.get("handoff_lossy", False)),
        detail=value.get("detail"),
    )
