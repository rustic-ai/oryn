"""Protocol data structures mirroring Rust protocol types."""

from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Rect:
    """Rectangle describing element position and size."""

    x: float
    y: float
    width: float
    height: float


@dataclass
class ElementState:
    """State flags for an element."""

    checked: bool = False
    selected: bool = False
    disabled: bool = False
    readonly: bool = False
    expanded: bool = False
    focused: bool = False


@dataclass
class Element:
    """An interactive element on the page."""

    id: int
    element_type: str  # "input", "button", "link", etc.
    role: Optional[str] = None  # "email", "password", "submit", etc.
    text: Optional[str] = None
    label: Optional[str] = None
    value: Optional[str] = None
    placeholder: Optional[str] = None
    selector: str = ""
    xpath: Optional[str] = None
    rect: Optional[Rect] = None
    attributes: dict[str, str] = field(default_factory=dict)
    state: ElementState = field(default_factory=ElementState)
    children: list[int] = field(default_factory=list)

    def is_input(self) -> bool:
        """Check if element is an input."""
        return self.element_type == "input"

    def is_button(self) -> bool:
        """Check if element is a button."""
        return self.element_type == "button"

    def is_link(self) -> bool:
        """Check if element is a link."""
        return self.element_type == "link"

    def has_role(self, role: str) -> bool:
        """Check if element has a specific role."""
        return self.role is not None and self.role.lower() == role.lower()

    def matches_text(self, text: str, case_sensitive: bool = False) -> bool:
        """Check if element text matches."""
        if self.text is None:
            return False
        if case_sensitive:
            return text in self.text
        return text.lower() in self.text.lower()


@dataclass
class ViewportInfo:
    """Viewport dimensions."""

    width: int = 0
    height: int = 0
    scale: float = 1.0


@dataclass
class ScrollInfo:
    """Scroll position information."""

    x: int = 0
    y: int = 0
    max_x: int = 0
    max_y: int = 0


@dataclass
class PageInfo:
    """Information about the current page."""

    url: str
    title: str
    viewport: ViewportInfo = field(default_factory=ViewportInfo)
    scroll: ScrollInfo = field(default_factory=ScrollInfo)


@dataclass
class LoginPattern:
    """Detected login form pattern."""

    email: Optional[int] = None  # Element ID
    username: Optional[int] = None
    password: int = 0
    submit: Optional[int] = None
    remember: Optional[int] = None


@dataclass
class SearchPattern:
    """Detected search box pattern."""

    input: int  # Element ID
    submit: Optional[int] = None


@dataclass
class PaginationPattern:
    """Detected pagination pattern."""

    prev: Optional[int] = None
    next: Optional[int] = None
    pages: list[int] = field(default_factory=list)


@dataclass
class ModalPattern:
    """Detected modal/dialog pattern."""

    close: Optional[int] = None
    confirm: Optional[int] = None
    cancel: Optional[int] = None


@dataclass
class CookieBannerPattern:
    """Detected cookie consent banner pattern."""

    accept: Optional[int] = None
    reject: Optional[int] = None
    settings: Optional[int] = None


@dataclass
class DetectedPatterns:
    """All detected UI patterns on the page."""

    login: Optional[LoginPattern] = None
    search: Optional[SearchPattern] = None
    pagination: Optional[PaginationPattern] = None
    modal: Optional[ModalPattern] = None
    cookie_banner: Optional[CookieBannerPattern] = None


@dataclass
class IntentAvailability:
    """Information about an available intent."""

    name: str
    status: str  # "ready", "navigate_required", "missing_pattern", "unavailable"
    parameters: list[str] = field(default_factory=list)
    trigger_reason: Optional[str] = None


@dataclass
class ScanStats:
    """Statistics from a scan operation."""

    total: int
    scanned: int = 0
