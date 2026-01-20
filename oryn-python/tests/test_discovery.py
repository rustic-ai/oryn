"""Tests for binary discovery."""


import pytest
from oryn.discovery import find_binary, validate_binary
from oryn.errors import BinaryNotFoundError


class TestFindBinary:
    """Tests for find_binary function."""

    def test_find_binary_env_var(self, monkeypatch, tmp_path):
        """Test finding binary via ORYN_BINARY env var."""
        # Create a fake binary
        fake_binary = tmp_path / "oryn"
        fake_binary.touch()
        fake_binary.chmod(0o755)

        monkeypatch.setenv("ORYN_BINARY", str(fake_binary))

        result = find_binary()
        assert result == str(fake_binary)

    def test_find_binary_config_path(self, tmp_path, monkeypatch):
        """Test finding binary via config path."""
        # Clear env var
        monkeypatch.delenv("ORYN_BINARY", raising=False)

        # Create a fake binary
        fake_binary = tmp_path / "oryn"
        fake_binary.touch()
        fake_binary.chmod(0o755)

        result = find_binary(config_path=str(fake_binary))
        assert result == str(fake_binary)

    def test_find_binary_not_found(self, monkeypatch):
        """Test error when binary not found."""
        monkeypatch.delenv("ORYN_BINARY", raising=False)
        # Make sure oryn is not in PATH for this test
        monkeypatch.setenv("PATH", "/nonexistent")

        with pytest.raises(BinaryNotFoundError) as exc_info:
            find_binary()

        assert "Could not find oryn binary" in str(exc_info.value)

    def test_find_binary_not_executable(self, monkeypatch, tmp_path):
        """Test that non-executable files are skipped."""
        # Create a non-executable file
        fake_binary = tmp_path / "oryn"
        fake_binary.touch()
        # Don't make it executable

        monkeypatch.setenv("ORYN_BINARY", str(fake_binary))

        with pytest.raises(BinaryNotFoundError):
            find_binary()


class TestValidateBinary:
    """Tests for validate_binary function."""

    def test_validate_existing_executable(self, tmp_path):
        """Test validating an existing executable."""
        fake_binary = tmp_path / "oryn"
        fake_binary.touch()
        fake_binary.chmod(0o755)

        assert validate_binary(str(fake_binary)) is True

    def test_validate_nonexistent(self):
        """Test validating nonexistent file."""
        assert validate_binary("/nonexistent/oryn") is False

    def test_validate_non_executable(self, tmp_path):
        """Test validating non-executable file."""
        fake_binary = tmp_path / "oryn"
        fake_binary.touch()
        fake_binary.chmod(0o644)

        assert validate_binary(str(fake_binary)) is False
