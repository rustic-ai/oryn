"""Tests for the Intent Language response parser."""


from oryn.parser import (
    escape_string,
    format_target,
    parse_action_response,
    parse_navigation_response,
    parse_observation,
)


class TestParseObservation:
    """Tests for parse_observation function."""

    def test_parse_basic_observation(self):
        """Test parsing a basic observation response."""
        raw = """Scanned 3 elements.
Title: Test Page
URL: https://example.com"""

        obs = parse_observation(raw)

        assert obs.url == "https://example.com"
        assert obs.title == "Test Page"
        assert obs.raw == raw

    def test_parse_observation_with_patterns(self):
        """Test parsing observation with detected patterns."""
        raw = """Scanned 5 elements.
Title: Login Page
URL: https://example.com/login

Patterns:
- Login Form
- Search Box"""

        obs = parse_observation(raw)

        assert obs.patterns is not None
        assert obs.patterns.login is not None
        assert obs.patterns.search is not None
        assert obs.patterns.pagination is None

    def test_parse_observation_with_intents(self):
        """Test parsing observation with available intents."""
        raw = """Scanned 5 elements.
Title: Login Page
URL: https://example.com/login

Available Intents:
- \U0001F7E2 login (username, password)
- \U0001F7E0 search [Navigate first]"""

        obs = parse_observation(raw)

        assert len(obs.available_intents) == 2
        assert obs.available_intents[0].name == "login"
        assert obs.available_intents[0].status == "ready"
        assert "username" in obs.available_intents[0].parameters
        assert obs.available_intents[1].status == "navigate_required"

    def test_parse_empty_observation(self):
        """Test parsing empty observation."""
        obs = parse_observation("")

        assert obs.url == ""
        assert obs.title == ""
        assert obs.elements == []


class TestParseElements:
    """Tests for element parsing within observations."""

    def test_parse_element_with_id_and_type(self):
        """Test parsing element with just id and type."""
        raw = """Scanned 1 elements.
Title: Test
URL: https://test.com

[1] button"""

        obs = parse_observation(raw)

        assert len(obs.elements) == 1
        assert obs.elements[0].id == 1
        assert obs.elements[0].element_type == "button"

    def test_parse_element_with_role(self):
        """Test parsing element with role."""
        raw = """Scanned 1 elements.
Title: Test
URL: https://test.com

[5] input/email "Email Address" """

        obs = parse_observation(raw)

        assert len(obs.elements) == 1
        assert obs.elements[0].id == 5
        assert obs.elements[0].element_type == "input"
        assert obs.elements[0].role == "email"
        assert obs.elements[0].text == "Email Address"

    def test_parse_element_with_modifiers(self):
        """Test parsing element with state modifiers."""
        raw = """Scanned 1 elements.
Title: Test
URL: https://test.com

[3] checkbox "Accept terms" {checked, disabled}"""

        obs = parse_observation(raw)

        assert len(obs.elements) == 1
        assert obs.elements[0].state.checked is True
        assert obs.elements[0].state.disabled is True
        assert obs.elements[0].state.focused is False


class TestParseNavigation:
    """Tests for navigation response parsing."""

    def test_parse_navigation_success(self):
        """Test parsing successful navigation."""
        raw = "Navigated to https://example.com"

        result = parse_navigation_response(raw)

        assert result.url == "https://example.com"

    def test_parse_navigation_multiline(self):
        """Test parsing navigation with extra output."""
        raw = """Some preamble
Navigated to https://example.com/page
Additional info"""

        result = parse_navigation_response(raw)

        assert result.url == "https://example.com/page"


class TestParseAction:
    """Tests for action response parsing."""

    def test_parse_success_action(self):
        """Test parsing successful action."""
        raw = "Clicked element [5]"

        result = parse_action_response(raw)

        assert result.success is True
        assert result.error is None

    def test_parse_error_action(self):
        """Test parsing failed action."""
        raw = "Error: Element not found"

        result = parse_action_response(raw)

        assert result.success is False
        assert result.error == "Element not found"

    def test_parse_implicit_error(self):
        """Test parsing action with implicit error."""
        raw = "Operation failed: timeout"

        result = parse_action_response(raw)

        assert result.success is False


class TestEscapeString:
    """Tests for string escaping."""

    def test_escape_simple_string(self):
        """Test escaping simple string."""
        assert escape_string("hello") == '"hello"'

    def test_escape_string_with_quotes(self):
        """Test escaping string containing quotes."""
        assert escape_string('say "hello"') == '"say \\"hello\\""'

    def test_escape_string_with_backslash(self):
        """Test escaping string containing backslash."""
        assert escape_string("path\\file") == '"path\\\\file"'


class TestFormatTarget:
    """Tests for target formatting."""

    def test_format_numeric_target(self):
        """Test formatting numeric target."""
        assert format_target(5) == "5"

    def test_format_role_target(self):
        """Test formatting role target."""
        assert format_target("email") == "email"
        assert format_target("password") == "password"
        assert format_target("submit") == "submit"

    def test_format_text_target(self):
        """Test formatting text target."""
        assert format_target("Sign In") == '"Sign In"'
        assert format_target("Click here") == '"Click here"'


class TestObservationMethods:
    """Tests for OrynObservation helper methods."""

    def test_find_by_text(self):
        """Test finding element by text."""
        raw = """Scanned 2 elements.
Title: Test
URL: https://test.com

[1] button "Sign In"
[2] button "Cancel" """

        obs = parse_observation(raw)

        elem = obs.find_by_text("Sign")
        assert elem is not None
        assert elem.id == 1

        elem = obs.find_by_text("NotFound")
        assert elem is None

    def test_find_by_role(self):
        """Test finding elements by role."""
        raw = """Scanned 3 elements.
Title: Test
URL: https://test.com

[1] input/email "Email"
[2] input/password "Password"
[3] button/submit "Login" """

        obs = parse_observation(raw)

        email_elems = obs.find_by_role("email")
        assert len(email_elems) == 1
        assert email_elems[0].id == 1

    def test_get_element(self):
        """Test getting element by ID."""
        raw = """Scanned 2 elements.
Title: Test
URL: https://test.com

[1] button "A"
[5] button "B" """

        obs = parse_observation(raw)

        elem = obs.get_element(5)
        assert elem is not None
        assert elem.text == "B"

        elem = obs.get_element(99)
        assert elem is None
