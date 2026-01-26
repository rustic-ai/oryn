from dataclasses import dataclass, field
from typing import Any, List, Optional


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


@dataclass
class OrynResult:
    """Result of an Oryn command execution."""

    success: bool
    raw: str
    changes: List[str] = field(default_factory=list)
    error: Optional[str] = None
    latency_ms: float = 0.0
