"""Parser for Intent Language responses from Oryn."""

import re
from typing import Optional

from .protocol import (
    CookieBannerPattern,
    DetectedPatterns,
    Element,
    ElementState,
    IntentAvailability,
    LoginPattern,
    ModalPattern,
    PageInfo,
    PaginationPattern,
    ScrollInfo,
    SearchPattern,
    ViewportInfo,
)
from .types import ExtractResult, NavigationResult, OrynObservation, OrynResult

# Regex patterns for parsing
# Page header: "@ https://example.com \"Page Title\""
PAGE_HEADER_PATTERN = re.compile(r'^@\s+(\S+)\s+"([^"]*)"')

# Element line: "[1] input/email \"Label\" {modifiers}"
# More flexible pattern to handle various formats
ELEMENT_PATTERN = re.compile(
    r"^\[(\d+)\]\s+"  # [id]
    r"(\w+)"  # element_type
    r"(?:/(\w+))?"  # optional /role
    r'(?:\s+"([^"]*)")?'  # optional "text/label"
    r"(?:\s+\{([^}]*)\})?"  # optional {modifiers}
)

# Navigation response: "Navigated to https://..."
NAVIGATION_PATTERN = re.compile(r"^Navigated to\s+(.+)$")

# Scan summary: "Scanned N elements."
SCAN_SUMMARY_PATTERN = re.compile(r"^Scanned\s+(\d+)\s+elements?\.")

# Error pattern: "Error: message"
ERROR_PATTERN = re.compile(r"^Error:\s+(.+)$")

# Intent availability: "- [icon] name (params) [reason]"
INTENT_PATTERN = re.compile(
    r"^-\s+([\U0001F534\U0001F7E0\U0001F7E2\u26AB])\s+(\S+)(?:\s+\(([^)]*)\))?(?:\s+\[([^\]]*)\])?"
)


def parse_observation(raw: str) -> OrynObservation:
    """Parse an observe command response into structured data.

    The response format from oryn observe looks like:
    ```
    Scanned N elements.
    Title: Page Title
    URL: https://example.com

    Patterns:
    - Login Form
    - Search Box

    Available Intents:
    - [green] login (username, password)
    ```

    Args:
        raw: Raw response string from oryn

    Returns:
        Parsed OrynObservation
    """
    lines = raw.strip().split("\n")
    url = ""
    title = ""
    elements: list[Element] = []
    patterns = DetectedPatterns()
    available_intents: list[IntentAvailability] = []

    section = "main"

    for line in lines:
        line = line.strip()
        if not line:
            continue

        # Check for section headers
        if line == "Patterns:":
            section = "patterns"
            continue
        elif line.startswith("Available Intents"):
            section = "intents"
            continue

        # Main section parsing
        if section == "main":
            # Try scan summary
            scan_match = SCAN_SUMMARY_PATTERN.match(line)
            if scan_match:
                # element_count available for future use
                _ = int(scan_match.group(1))
                continue

            # Try Title/URL lines
            if line.startswith("Title:"):
                title = line[6:].strip()
                continue
            if line.startswith("URL:"):
                url = line[4:].strip()
                continue

            # Try element pattern
            elem_match = ELEMENT_PATTERN.match(line)
            if elem_match:
                elem = _parse_element_match(elem_match)
                if elem:
                    elements.append(elem)
                continue

        # Patterns section
        elif section == "patterns":
            if line.startswith("- "):
                pattern_name = line[2:].strip()
                _add_pattern(patterns, pattern_name)

        # Intents section
        elif section == "intents":
            intent_match = INTENT_PATTERN.match(line)
            if intent_match:
                intent = _parse_intent_match(intent_match)
                if intent:
                    available_intents.append(intent)

    page_info = PageInfo(
        url=url,
        title=title,
        viewport=ViewportInfo(),
        scroll=ScrollInfo(),
    )

    return OrynObservation(
        raw=raw,
        url=url,
        title=title,
        elements=elements,
        patterns=patterns if _has_any_pattern(patterns) else None,
        available_intents=available_intents,
        page_info=page_info,
        token_count=len(raw),
    )


def _parse_element_match(match: re.Match) -> Optional[Element]:
    """Parse a regex match into an Element."""
    try:
        element_id = int(match.group(1))
        element_type = match.group(2)
        role = match.group(3)  # May be None
        text = match.group(4)  # May be None
        modifiers = match.group(5)  # May be None

        state = ElementState()
        if modifiers:
            mods = [m.strip() for m in modifiers.split(",")]
            state.checked = "checked" in mods
            state.disabled = "disabled" in mods
            state.readonly = "readonly" in mods
            state.focused = "focused" in mods
            state.expanded = "expanded" in mods
            state.selected = "selected" in mods

        return Element(
            id=element_id,
            element_type=element_type,
            role=role,
            text=text,
            label=text,  # Often the same in Intent Language format
            state=state,
        )
    except (ValueError, IndexError):
        return None


def _parse_intent_match(match: re.Match) -> Optional[IntentAvailability]:
    """Parse an intent availability line."""
    try:
        icon = match.group(1)
        name = match.group(2)
        params_str = match.group(3)
        reason = match.group(4)

        # Map icon to status
        status_map = {
            "\U0001F7E2": "ready",  # Green circle
            "\U0001F7E0": "navigate_required",  # Orange circle
            "\U0001F534": "missing_pattern",  # Red circle
            "\u26AB": "unavailable",  # Black circle
        }
        status = status_map.get(icon, "unknown")

        params = []
        if params_str:
            params = [p.strip() for p in params_str.split(",")]

        return IntentAvailability(
            name=name,
            status=status,
            parameters=params,
            trigger_reason=reason,
        )
    except (ValueError, IndexError):
        return None


def _add_pattern(patterns: DetectedPatterns, name: str) -> None:
    """Add a detected pattern by name."""
    name_lower = name.lower()
    if "login" in name_lower:
        patterns.login = LoginPattern(password=0)
    elif "search" in name_lower:
        patterns.search = SearchPattern(input=0)
    elif "pagination" in name_lower:
        patterns.pagination = PaginationPattern()
    elif "modal" in name_lower:
        patterns.modal = ModalPattern()
    elif "cookie" in name_lower:
        patterns.cookie_banner = CookieBannerPattern()


def _has_any_pattern(patterns: DetectedPatterns) -> bool:
    """Check if any patterns were detected."""
    return any(
        [
            patterns.login,
            patterns.search,
            patterns.pagination,
            patterns.modal,
            patterns.cookie_banner,
        ]
    )


def parse_navigation_response(raw: str) -> NavigationResult:
    """Parse a navigation response.

    Args:
        raw: Raw response like "Navigated to https://example.com"

    Returns:
        NavigationResult with the URL
    """
    match = NAVIGATION_PATTERN.match(raw.strip())
    if match:
        return NavigationResult(url=match.group(1))

    # Fallback: try to extract URL from response
    lines = raw.strip().split("\n")
    for line in lines:
        if line.startswith("Navigated to "):
            return NavigationResult(url=line[13:].strip())

    # If we can't parse, return with raw as URL
    return NavigationResult(url=raw.strip())


def parse_action_response(raw: str) -> OrynResult:
    """Parse a generic action response.

    Args:
        raw: Raw response from oryn

    Returns:
        OrynResult indicating success/failure
    """
    lines = raw.strip().split("\n")

    # Check for error
    for line in lines:
        error_match = ERROR_PATTERN.match(line.strip())
        if error_match:
            return OrynResult.from_error(raw, error_match.group(1))

    # Check for common error phrases
    lower_raw = raw.lower()
    if "error" in lower_raw or "failed" in lower_raw:
        return OrynResult.from_error(raw, raw.strip())

    # Success case
    message = lines[0] if lines else None
    return OrynResult.from_success(raw, message)


def parse_extract_response(raw: str) -> ExtractResult:
    """Parse an extract command response.

    Args:
        raw: Raw response from extract command

    Returns:
        ExtractResult with parsed data
    """
    # For now, just return raw data - detailed parsing depends on extract type
    return ExtractResult(raw=raw, data=[])


def escape_string(s: str) -> str:
    """Escape a string for use in Intent Language commands.

    Args:
        s: String to escape

    Returns:
        Escaped string safe for use in commands
    """
    # Escape backslashes and quotes
    escaped = s.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def format_target(target: str | int) -> str:
    """Format a target for use in Intent Language commands.

    Args:
        target: Element ID (int) or text/role target (str)

    Returns:
        Formatted target string
    """
    if isinstance(target, int):
        return str(target)
    # Check if it's a known role
    roles = {"email", "password", "search", "submit", "username", "phone", "url", "link", "button"}
    if target.lower() in roles:
        return target
    # Otherwise, treat as text and quote it
    return escape_string(target)
