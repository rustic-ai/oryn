"""Shared fixtures for E2E tests that run .oil scripts."""

import os
from pathlib import Path

import pytest
from oryn import OrynClientSync
from oryn.errors import BinaryNotFoundError

# Test harness base URL
TEST_HARNESS_URL = os.environ.get("TEST_HARNESS_URL", "http://localhost:3000")

# Possible paths to .oil scripts
# 1. Relative to this test file (when running from repo root)
# 2. Docker path (when running in container)
SCRIPT_PATHS = [
    Path(__file__).parent.parent.parent.parent.parent / "test-harness" / "scripts",
    Path("/app/test-harness/scripts"),
]


def pytest_configure(config):
    """Configure pytest markers."""
    config.addinivalue_line("markers", "e2e: mark test as end-to-end test")
    config.addinivalue_line("markers", "oil: mark test as running .oil script")


@pytest.fixture(scope="session")
def check_oryn_available():
    """Check if oryn binary is available."""
    try:
        from oryn.discovery import find_binary

        find_binary()
        return True
    except BinaryNotFoundError:
        pytest.skip("oryn binary not found")
        return False


@pytest.fixture
def client(check_oryn_available):
    """Create an OrynClientSync instance for testing."""
    mode = os.environ.get("ORYN_MODE", "headless")
    with OrynClientSync(mode=mode, timeout=60.0) as c:
        yield c


@pytest.fixture
def scripts_dir():
    """Get path to .oil scripts directory."""
    for path in SCRIPT_PATHS:
        if path.exists():
            return path
    pytest.skip(f"Scripts directory not found. Searched: {SCRIPT_PATHS}")


@pytest.fixture
def base_url():
    """Get the test harness base URL."""
    return TEST_HARNESS_URL
