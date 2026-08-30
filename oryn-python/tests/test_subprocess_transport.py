"""Tests for subprocess transport robustness."""

import textwrap

import pytest

from oryn.config import OrynConfig
from oryn.errors import ConnectionLostError, LaunchError, TimeoutError
from oryn.transport.subprocess import SubprocessTransport


def _make_executable_script(tmp_path, name: str, body: str):
    path = tmp_path / name
    path.write_text("#!/usr/bin/env python3\n" + textwrap.dedent(body), encoding="utf-8")
    path.chmod(0o755)
    return path


@pytest.mark.asyncio
async def test_connect_fails_fast_with_exit_code_and_stderr(tmp_path):
    binary = _make_executable_script(
        tmp_path,
        "fake_oryn_launch_fail.py",
        """
        import sys

        sys.stderr.write("launch failed on purpose\\n")
        sys.stderr.flush()
        sys.exit(7)
        """,
    )

    transport = SubprocessTransport(OrynConfig(mode="headless", binary_path=str(binary)))
    with pytest.raises(LaunchError) as exc_info:
        await transport.connect()

    message = str(exc_info.value)
    assert "exit code: 7" in message
    assert "launch failed on purpose" in message


@pytest.mark.asyncio
async def test_connect_and_send_drains_large_stderr_without_deadlock(tmp_path):
    binary = _make_executable_script(
        tmp_path,
        "fake_oryn_large_stderr.py",
        """
        import sys

        print("Backend launched. Enter commands.", flush=True)
        print("> ", end="", flush=True)

        while True:
            line = sys.stdin.readline()
            if not line:
                break

            command = line.strip()
            if command in ("exit", "quit"):
                break

            # Large stderr burst to exceed pipe capacity quickly if not drained.
            sys.stderr.write("E" * (256 * 1024) + "\\n")
            sys.stderr.flush()

            print(f"ok {command}", flush=True)
            print("> ", end="", flush=True)
        """,
    )

    transport = SubprocessTransport(
        OrynConfig(
            mode="headless",
            binary_path=str(binary),
            timeout=2.0,
            connect_timeout=2.0,
        )
    )

    await transport.connect()
    try:
        for i in range(3):
            response = await transport.send(f"ping-{i}")
            assert f"ok ping-{i}" in response
    finally:
        await transport.close()


@pytest.mark.asyncio
async def test_send_timeout_when_process_alive(tmp_path):
    binary = _make_executable_script(
        tmp_path,
        "fake_oryn_hang.py",
        """
        import sys
        import time

        print("Backend launched. Enter commands.", flush=True)
        print("> ", end="", flush=True)

        line = sys.stdin.readline()
        if line:
            time.sleep(1.0)
        """,
    )

    transport = SubprocessTransport(
        OrynConfig(
            mode="headless",
            binary_path=str(binary),
            timeout=0.05,
            connect_timeout=1.0,
        )
    )

    await transport.connect()
    try:
        with pytest.raises(TimeoutError):
            await transport.send("observe")
    finally:
        await transport.close()


@pytest.mark.asyncio
async def test_send_connection_lost_when_process_exits_during_read(tmp_path):
    binary = _make_executable_script(
        tmp_path,
        "fake_oryn_exit_after_command.py",
        """
        import sys
        import time

        print("Backend launched. Enter commands.", flush=True)
        print("> ", end="", flush=True)

        line = sys.stdin.readline()
        if line:
            time.sleep(0.05)
            sys.exit(11)
        """,
    )

    transport = SubprocessTransport(
        OrynConfig(
            mode="headless",
            binary_path=str(binary),
            timeout=0.5,
            connect_timeout=1.0,
        )
    )

    await transport.connect()
    try:
        with pytest.raises(ConnectionLostError) as exc_info:
            await transport.send("observe")
        assert exc_info.value.returncode == 11
    finally:
        await transport.close()
