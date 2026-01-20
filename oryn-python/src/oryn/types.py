"""User-facing types for OrynClient responses."""

from dataclasses import dataclass, field
from typing import Optional

from .protocol import DetectedPatterns, Element, IntentAvailability, PageInfo


@dataclass
class OrynObservation:
    """Structured observation from Oryn's observe command.

    This is the primary data structure returned when observing a page.
    It contains parsed elements, detected patterns, and available intents.
    """

    raw: str  # Raw Intent Language response
    url: str  # Current URL
    title: str  # Page title
    elements: list[Element] = field(default_factory=list)
    patterns: Optional[DetectedPatterns] = None
    available_intents: list[IntentAvailability] = field(default_factory=list)
    page_info: Optional[PageInfo] = None
    token_count: int = 0  # For metrics
    latency_ms: float = 0.0  # Response time

    def find_by_text(self, text: str, case_sensitive: bool = False) -> Optional[Element]:
        """Find the first element matching the given text.

        Args:
            text: Text to search for (partial match)
            case_sensitive: Whether to match case

        Returns:
            First matching element or None
        """
        for elem in self.elements:
            if elem.matches_text(text, case_sensitive):
                return elem
        return None

    def find_all_by_text(self, text: str, case_sensitive: bool = False) -> list[Element]:
        """Find all elements matching the given text.

        Args:
            text: Text to search for (partial match)
            case_sensitive: Whether to match case

        Returns:
            List of matching elements
        """
        return [e for e in self.elements if e.matches_text(text, case_sensitive)]

    def find_by_role(self, role: str) -> list[Element]:
        """Find all elements with the given role.

        Args:
            role: Role to match (e.g., "email", "password", "submit")

        Returns:
            List of matching elements
        """
        return [e for e in self.elements if e.has_role(role)]

    def get_element(self, element_id: int) -> Optional[Element]:
        """Get an element by its ID.

        Args:
            element_id: Element ID from observation

        Returns:
            Element or None if not found
        """
        for elem in self.elements:
            if elem.id == element_id:
                return elem
        return None

    def get_inputs(self) -> list[Element]:
        """Get all input elements."""
        return [e for e in self.elements if e.is_input()]

    def get_buttons(self) -> list[Element]:
        """Get all button elements."""
        return [e for e in self.elements if e.is_button()]

    def get_links(self) -> list[Element]:
        """Get all link elements."""
        return [e for e in self.elements if e.is_link()]

    def has_login_form(self) -> bool:
        """Check if the page has a detected login form."""
        return self.patterns is not None and self.patterns.login is not None

    def has_search_box(self) -> bool:
        """Check if the page has a detected search box."""
        return self.patterns is not None and self.patterns.search is not None

    def has_modal(self) -> bool:
        """Check if the page has a detected modal."""
        return self.patterns is not None and self.patterns.modal is not None

    def has_cookie_banner(self) -> bool:
        """Check if the page has a detected cookie banner."""
        return self.patterns is not None and self.patterns.cookie_banner is not None


@dataclass
class OrynResult:
    """Result of an Oryn command execution.

    Returned by action commands like click, type, etc.
    """

    success: bool
    raw: str  # Raw response from oryn
    message: Optional[str] = None
    navigation: bool = False  # Whether the action caused navigation
    changes: list[str] = field(default_factory=list)
    error: Optional[str] = None
    latency_ms: float = 0.0

    @classmethod
    def from_success(cls, raw: str, message: str | None = None) -> "OrynResult":
        """Create a successful result."""
        return cls(success=True, raw=raw, message=message)

    @classmethod
    def from_error(cls, raw: str, error: str) -> "OrynResult":
        """Create a failed result."""
        return cls(success=False, raw=raw, error=error)


@dataclass
class NavigationResult:
    """Result of a navigation command."""

    url: str
    title: str = ""
    latency_ms: float = 0.0


@dataclass
class ExtractResult:
    """Result of an extract command."""

    raw: str
    data: list[dict] = field(default_factory=list)
    latency_ms: float = 0.0
