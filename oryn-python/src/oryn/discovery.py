import os
import shutil

from .errors import BinaryNotFoundError

# Default search paths for oryn binary
DEFAULT_SEARCH_PATHS = [
    "~/.local/bin/oryn",
    "~/.cargo/bin/oryn",
    "/usr/local/bin/oryn",
    "/usr/bin/oryn",
]


def find_binary(config_path: str | None = None) -> str:
    """Find the oryn binary using the standard search order.

    Search order:
    1. ORYN_BINARY environment variable
    2. config_path parameter (if provided)
    3. shutil.which("oryn") - checks PATH
    4. Default paths: ~/.local/bin, ~/.cargo/bin, /usr/local/bin, /usr/bin

    Args:
        config_path: Optional explicit path from configuration

    Returns:
        Absolute path to the oryn binary

    Raises:
        BinaryNotFoundError: If oryn binary cannot be found
    """
    searched_paths: list[str] = []

    # 1. Check ORYN_BINARY environment variable
    env_path = os.environ.get("ORYN_BINARY")
    if env_path:
        searched_paths.append(f"ORYN_BINARY={env_path}")
        expanded = os.path.expanduser(env_path)
        if os.path.isfile(expanded) and os.access(expanded, os.X_OK):
            return os.path.abspath(expanded)

    # 2. Check config_path parameter
    if config_path:
        searched_paths.append(f"config.binary_path={config_path}")
        expanded = os.path.expanduser(config_path)
        if os.path.isfile(expanded) and os.access(expanded, os.X_OK):
            return os.path.abspath(expanded)

    # 3. Check PATH using shutil.which
    searched_paths.append("PATH (shutil.which)")
    which_result = shutil.which("oryn")
    if which_result:
        return os.path.abspath(which_result)

    # 4. Check default search paths
    for path_pattern in DEFAULT_SEARCH_PATHS:
        expanded = os.path.expanduser(path_pattern)
        searched_paths.append(expanded)
        if os.path.isfile(expanded) and os.access(expanded, os.X_OK):
            return os.path.abspath(expanded)

    raise BinaryNotFoundError(searched_paths)


def validate_binary(path: str) -> bool:
    """Validate that a path points to a valid oryn binary.

    Args:
        path: Path to check

    Returns:
        True if the path is a valid executable
    """
    expanded = os.path.expanduser(path)
    return os.path.isfile(expanded) and os.access(expanded, os.X_OK)


def get_binary_version(path: str) -> str | None:
    """Get the version of the oryn binary.

    Args:
        path: Path to oryn binary

    Returns:
        Version string or None if it couldn't be determined
    """
    import subprocess

    try:
        result = subprocess.run(
            [path, "--version"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        if result.returncode == 0:
            # Parse version from output like "oryn 0.1.0"
            output = result.stdout.strip()
            parts = output.split()
            if len(parts) >= 2:
                return parts[1]
            return output
    except (subprocess.TimeoutExpired, FileNotFoundError, PermissionError):
        pass
    return None
