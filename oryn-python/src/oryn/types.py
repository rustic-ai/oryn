from dataclasses import dataclass, field
from typing import Any, List, Literal, Optional

AGENT_INTENT_CONTRACT_VERSION = 3

TargetKind = Literal[
    "reference",
    "alias",
    "text",
    "role",
    "css",
    "xpath",
    "near",
    "inside",
    "after",
    "before",
    "contains",
]

IntentKind = Literal[
    "navigate",
    "back",
    "forward",
    "refresh",
    "observe",
    "click",
    "type",
    "clear",
    "check",
    "select",
    "submit",
    "focus",
    "hover",
    "clipboard_read",
    "clipboard_write",
    "upload",
    "extract",
    "wait_revision",
]


@dataclass(frozen=True)
class OrynTarget:
    """Typed semantic target shared with the Rust agent contract."""

    kind: TargetKind
    arguments: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"kind": self.kind, **self.arguments}

    @classmethod
    def reference(cls, page_id: int, document_generation: int, node: int) -> "OrynTarget":
        return cls(
            "reference",
            {
                "reference": {
                    "page_id": page_id,
                    "document_generation": document_generation,
                    "node": node,
                }
            },
        )

    @classmethod
    def alias(cls, alias: int) -> "OrynTarget":
        return cls("alias", {"alias": alias})

    @classmethod
    def text(cls, text: str) -> "OrynTarget":
        return cls("text", {"text": text})

    @classmethod
    def role(cls, role: str) -> "OrynTarget":
        return cls("role", {"role": role})

    @classmethod
    def css(cls, selector: str) -> "OrynTarget":
        return cls("css", {"selector": selector})

    @classmethod
    def xpath(cls, expression: str) -> "OrynTarget":
        return cls("xpath", {"expression": expression})

    @classmethod
    def relation(
        cls,
        kind: Literal["near", "after", "before"],
        target: "OrynTarget",
        anchor: "OrynTarget",
    ) -> "OrynTarget":
        return cls(kind, {"target": target.to_dict(), "anchor": anchor.to_dict()})

    @classmethod
    def inside(cls, target: "OrynTarget", container: "OrynTarget") -> "OrynTarget":
        return cls(
            "inside",
            {"target": target.to_dict(), "container": container.to_dict()},
        )

    @classmethod
    def contains(cls, target: "OrynTarget", content: "OrynTarget") -> "OrynTarget":
        return cls(
            "contains",
            {"target": target.to_dict(), "content": content.to_dict()},
        )


@dataclass(frozen=True)
class OrynInput:
    """Text or a non-sensitive label for a parent-owned secret."""

    kind: Literal["text", "secret"]
    data: str

    @classmethod
    def text(cls, value: str) -> "OrynInput":
        return cls("text", value)

    @classmethod
    def secret(cls, label: str) -> "OrynInput":
        return cls("secret", label)

    def to_dict(self) -> dict[str, str]:
        field_name = "label" if self.kind == "secret" else "value"
        return {"kind": self.kind, field_name: self.data}


@dataclass(frozen=True)
class OrynIntent:
    """Versioned agent intent executed without constructing OIL text."""

    intent: IntentKind
    arguments: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"intent": self.intent, **self.arguments}

    def to_request(self) -> dict[str, Any]:
        return {
            "contract_version": AGENT_INTENT_CONTRACT_VERSION,
            "intent": self.to_dict(),
        }

    @classmethod
    def navigate(cls, url: str) -> "OrynIntent":
        return cls("navigate", {"url": url})

    @classmethod
    def back(cls) -> "OrynIntent":
        return cls("back")

    @classmethod
    def forward(cls) -> "OrynIntent":
        return cls("forward")

    @classmethod
    def refresh(cls) -> "OrynIntent":
        return cls("refresh")

    @classmethod
    def observe(
        cls,
        *,
        profile: Literal["compact", "full", "scoped", "delta"] = "compact",
        since: int | None = None,
        include_geometry: bool = False,
        viewport_only: bool = False,
        include_hidden: bool = False,
        include_structures: bool = False,
        semantic_contract_version: int | None = None,
        scope: dict[str, Any] | None = None,
    ) -> "OrynIntent":
        return cls(
            "observe",
            {
                "options": {
                    "profile": profile,
                    "since": since,
                    "include_geometry": include_geometry,
                    "viewport_only": viewport_only,
                    "include_hidden": include_hidden,
                    "include_structures": include_structures,
                    "semantic_contract_version": semantic_contract_version,
                    "scope": scope,
                }
            },
        )

    @classmethod
    def click(cls, target: OrynTarget, *, force: bool = False) -> "OrynIntent":
        return cls("click", {"target": target.to_dict(), "force": force})

    @classmethod
    def type(cls, target: OrynTarget, input: OrynInput) -> "OrynIntent":
        return cls("type", {"target": target.to_dict(), "input": input.to_dict()})

    @classmethod
    def clear(cls, target: OrynTarget) -> "OrynIntent":
        return cls("clear", {"target": target.to_dict()})

    @classmethod
    def check(cls, target: OrynTarget, *, checked: bool = True) -> "OrynIntent":
        return cls("check", {"target": target.to_dict(), "checked": checked})

    @classmethod
    def select(cls, target: OrynTarget, value: str) -> "OrynIntent":
        return cls("select", {"target": target.to_dict(), "value": value})

    @classmethod
    def submit(cls, target: OrynTarget) -> "OrynIntent":
        return cls("submit", {"target": target.to_dict()})

    @classmethod
    def focus(cls, target: OrynTarget) -> "OrynIntent":
        return cls("focus", {"target": target.to_dict()})

    @classmethod
    def hover(cls, target: OrynTarget) -> "OrynIntent":
        return cls("hover", {"target": target.to_dict()})

    @classmethod
    def clipboard_read(cls) -> "OrynIntent":
        """Read the browser-context virtual clipboard, never the host clipboard."""
        return cls("clipboard_read")

    @classmethod
    def clipboard_write(cls, value: str) -> "OrynIntent":
        """Replace text in the browser-context virtual clipboard."""
        return cls("clipboard_write", {"value": value})

    @classmethod
    def upload(cls, target: OrynTarget, files: list[str]) -> "OrynIntent":
        """Select labels previously imported into the parent upload store."""
        return cls("upload", {"target": target.to_dict(), "files": files})

    @classmethod
    def extract(
        cls,
        kind: Literal["text", "html", "links", "images", "tables", "url", "title"],
        selector: str | None = None,
    ) -> "OrynIntent":
        return cls("extract", {"kind": kind, "selector": selector})

    @classmethod
    def wait_revision(cls, after: int, timeout_ms: int = 30_000) -> "OrynIntent":
        return cls("wait_revision", {"after": after, "timeout_ms": timeout_ms})


ActionClassification = Literal[
    "failed",
    "unsupported",
    "event_only",
    "state_changed",
    "navigation",
    "request_effect",
]


@dataclass
class OrynCapabilityDiagnostic:
    capability: str
    support: str
    alternatives: List[str] = field(default_factory=list)
    handoff_lossy: bool = False
    detail: Optional[str] = None


@dataclass
class OrynEffect:
    kind: str
    data: dict[str, Any] = field(default_factory=dict)


@dataclass
class OrynDelta:
    contract_version: int
    from_revision: int
    to_revision: int
    upserted: List[dict[str, Any]] = field(default_factory=list)
    removed: List[dict[str, Any]] = field(default_factory=list)
    style_revision: Optional[int] = None
    layout_revision: Optional[int] = None


@dataclass
class OrynViewport:
    width: float = 1280.0
    height: float = 720.0
    device_pixel_ratio: float = 1.0


@dataclass
class OrynScroll:
    x: float = 0.0
    y: float = 0.0
    content_width: float = 1280.0
    content_height: float = 720.0


@dataclass
class IntentTemplate:
    name: str
    description: str = ""
    examples: List[str] = field(default_factory=list)


@dataclass
class PatternMatch:
    login: bool = False
    search: bool = False
    pagination: bool = False
    modal: bool = False
    cookie_banner: bool = False


@dataclass
class OrynObservation:
    """Structured observation from Oryn."""

    raw: str
    url: str
    title: str
    elements: List[Any] = field(default_factory=list)
    patterns: Optional[PatternMatch] = None
    available_intents: List[IntentTemplate] = field(default_factory=list)
    token_count: int = 0
    latency_ms: float = 0.0
    contract_version: Optional[int] = None
    revision: Optional[int] = None
    document_generation: Optional[int] = None
    capabilities: List[OrynCapabilityDiagnostic] = field(default_factory=list)
    diagnostics: List[str] = field(default_factory=list)
    byte_count: int = 0
    execution_domain: Optional[str] = None
    style_revision: Optional[int] = None
    layout_revision: Optional[int] = None
    viewport: Optional[OrynViewport] = None
    scroll: Optional[OrynScroll] = None


@dataclass
class OrynAccessibilityNode:
    semantic_ref: dict[str, Any]
    excluded: bool
    role: str
    name: str
    description: Optional[str] = None
    exclusion_reason: Optional[str] = None
    relationships: dict[str, List[dict[str, Any]]] = field(default_factory=dict)
    properties: dict[str, Any] = field(default_factory=dict)
    owned_by: Optional[dict[str, Any]] = None
    content_trust: str = "page_authored"
    provenance: dict[str, Any] = field(default_factory=dict)
    evidence: List[dict[str, Any]] = field(default_factory=list)


@dataclass
class OrynSemanticStructure:
    structure_id: str
    kind: str
    root: dict[str, Any]
    name: str
    parent_structure_id: Optional[str] = None
    members: List[dict[str, Any]] = field(default_factory=list)
    groups: List[dict[str, Any]] = field(default_factory=list)
    properties: dict[str, Any] = field(default_factory=dict)
    content_trust: str = "page_authored"
    provenance: dict[str, Any] = field(default_factory=dict)
    evidence: List[dict[str, Any]] = field(default_factory=list)


@dataclass
class OrynSemanticObservation:
    contract_version: int
    observation: OrynObservation
    accessibility: List[OrynAccessibilityNode] = field(default_factory=list)
    structures: List[OrynSemanticStructure] = field(default_factory=list)
    diagnostics: List[str] = field(default_factory=list)


@dataclass
class OrynSemanticObservationV6:
    contract_version: int
    observation: OrynObservation
    semantic_revision: int
    nodes: List[dict[str, Any]] = field(default_factory=list)
    accessibility: List[OrynAccessibilityNode] = field(default_factory=list)
    structures: List[OrynSemanticStructure] = field(default_factory=list)
    projection: dict[str, Any] = field(default_factory=dict)
    diagnostics: List[str] = field(default_factory=list)


@dataclass
class OrynSemanticDeltaV6:
    contract_version: int
    from_revision: int
    to_revision: int
    semantic_revision: int
    changes: dict[str, Any] = field(default_factory=dict)
    diagnostics: List[str] = field(default_factory=list)


@dataclass
class OrynResult:
    """Result of an Oryn command execution."""

    success: bool
    raw: str
    changes: List[str] = field(default_factory=list)
    error: Optional[str] = None
    latency_ms: float = 0.0
    accepted: bool = False
    classification: ActionClassification = "failed"
    effects: List[OrynEffect] = field(default_factory=list)
    delta: Optional[OrynDelta] = None
    diagnostics: List[str] = field(default_factory=list)
    revision_before: Optional[int] = None
    revision_after: Optional[int] = None
    execution_domain: Optional[str] = None


@dataclass
class OrynIntentResult:
    """Decoded result of :meth:`OrynClient.perform`."""

    kind: Literal[
        "observation",
        "geometry_observation",
        "semantic_observation",
        "semantic_observation_v6",
        "semantic_delta_v6",
        "delta",
        "action",
        "navigation",
        "structured",
        "text",
        "wait",
    ]
    raw: str
    observation: Optional[OrynObservation] = None
    semantic_observation: Optional[OrynSemanticObservation] = None
    semantic_observation_v6: Optional[OrynSemanticObservationV6] = None
    semantic_delta_v6: Optional[OrynSemanticDeltaV6] = None
    delta: Optional[OrynDelta] = None
    action: Optional[OrynResult] = None
    navigation: Optional[dict[str, Any]] = None
    value: Any = None
    revision: Optional[int] = None
