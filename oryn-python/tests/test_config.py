"""Tests for configuration."""


from oryn.config import OrynConfig


class TestOrynConfig:
    """Tests for OrynConfig."""

    def test_default_config(self):
        """Test default configuration values."""
        config = OrynConfig()

        assert config.mode == "headless"
        assert config.timeout == 30.0
        assert config.connect_timeout == 60.0
        assert config.port == 9001
        assert config.binary_path is None
        assert config.driver_url is None

    def test_custom_config(self):
        """Test custom configuration values."""
        config = OrynConfig(
            mode="remote",
            timeout=60.0,
            port=9999,
            binary_path="/usr/bin/oryn",
        )

        assert config.mode == "remote"
        assert config.timeout == 60.0
        assert config.port == 9999
        assert config.binary_path == "/usr/bin/oryn"

    def test_get_cli_args_headless(self):
        """Test CLI args for headless mode."""
        config = OrynConfig(mode="headless")

        args = config.get_cli_args()

        assert args == ["headless"]

    def test_get_cli_args_embedded(self):
        """Test CLI args for embedded mode."""
        config = OrynConfig(mode="embedded")

        args = config.get_cli_args()

        assert args == ["embedded"]

    def test_get_cli_args_embedded_with_url(self):
        """Test CLI args for embedded mode with driver URL."""
        config = OrynConfig(mode="embedded", driver_url="http://localhost:4444")

        args = config.get_cli_args()

        assert args == ["embedded", "--driver-url", "http://localhost:4444"]

    def test_get_cli_args_remote(self):
        """Test CLI args for remote mode."""
        config = OrynConfig(mode="remote", port=8080)

        args = config.get_cli_args()

        assert args == ["remote", "--port", "8080"]
