from dataclasses import dataclass, field
from typing import Any, List, Literal, Optional

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
