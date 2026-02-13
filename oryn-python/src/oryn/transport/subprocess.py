"""Subprocess transport for communicating with oryn via stdin/stdout."""

import asyncio
from collections import deque
import os
from typing import Optional

from ..config import OrynConfig
from ..discovery import find_binary
from ..errors import ConnectionLostError, LaunchError, TimeoutError
from .base import Transport


class SubprocessTransport(Transport):
    """Transport that communicates with oryn via subprocess stdin/stdout.

    This transport spawns the oryn binary as a subprocess and communicates
    using the REPL interface over stdin/stdout.
    """

    def __init__(self, config: OrynConfig):
        """Initialize subprocess transport.

        Args:
            config: OrynConfig with connection settings
        """
        self._config = config
        self._process: Optional[asyncio.subprocess.Process] = None
        self._binary: Optional[str] = None
        self._connected = False
        self._reader_task: Optional[asyncio.Task] = None
        self._response_queue: asyncio.Queue[str] = asyncio.Queue()
        self._stderr_task: Optional[asyncio.Task] = None
        self._stderr_tail: deque[str] = deque()
        self._stderr_tail_chars = 0
        self._stderr_tail_limit_chars = 16_384
        self._lock = asyncio.Lock()
        self._log_file_handle = None

    async def connect(self) -> None:
        """Launch oryn subprocess and establish connection."""
        # Find the binary
        self._binary = find_binary(self._config.binary_path)

        # Build command line arguments
        args = [self._binary] + self._config.get_cli_args()

        # Set up environment
        env = os.environ.copy()
        env.update(self._config.env)

        # Prepare stderr logging (always pipe so we can drain and avoid deadlocks).
        stderr = asyncio.subprocess.PIPE
        if self._config.log_file:
            try:
                self._log_file_handle = open(self._config.log_file, "a", encoding="utf-8")
            except Exception:
                # Keep stderr piped even if file opening fails.
                pass

        try:
            self._process = await asyncio.create_subprocess_exec(
                *args,
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=stderr,
                env=env,
            )
        except FileNotFoundError as e:
            if self._log_file_handle:
                self._log_file_handle.close()
                self._log_file_handle = None
            raise LaunchError(f"Failed to launch oryn: {e}")
        except PermissionError as e:
            if self._log_file_handle:
                self._log_file_handle.close()
                self._log_file_handle = None
            raise LaunchError(f"Permission denied launching oryn: {e}")

        if self._process.stderr:
            self._stderr_task = asyncio.create_task(self._drain_stderr())

        # Wait for the initial prompt/ready message
        try:
            await asyncio.wait_for(
                self._wait_for_ready(),
                timeout=self._config.connect_timeout,
            )
        except asyncio.TimeoutError:
            returncode = self._process.returncode if self._process else None
            stderr_tail = self._get_stderr_tail()
            await self._kill_process()
            if returncode is not None:
                raise LaunchError(
                    f"oryn subprocess exited before ready (exit code: {returncode})",
                    stderr=stderr_tail or None,
                )
            raise TimeoutError("connect", self._config.connect_timeout)

        self._connected = True

    async def _wait_for_ready(self) -> None:
        """Wait for oryn to be ready (initial output)."""
        if not self._process or not self._process.stdout:
            raise LaunchError("oryn subprocess has no stdout pipe")

        # Read until we see the initial banner and prompt
        # Oryn outputs: "Backend launched. Enter commands..."
        # Then shows "> " prompt
        buffer = ""
        while True:
            try:
                chunk = await self._process.stdout.read(1024)
                if not chunk:
                    returncode = self._process.returncode
                    if returncode is None:
                        await asyncio.sleep(0.01)
                        returncode = self._process.returncode
                    stderr_tail = self._get_stderr_tail()
                    if returncode is not None:
                        raise LaunchError(
                            f"oryn subprocess exited before ready (exit code: {returncode})",
                            stderr=stderr_tail or None,
                        )
                    raise LaunchError(
                        "oryn subprocess closed stdout before becoming ready",
                        stderr=stderr_tail or None,
                    )
                buffer += chunk.decode("utf-8", errors="replace")
                # Look for the ready indicator (prompt)
                if "\n> " in buffer or buffer.endswith("\n>") or buffer.endswith("> "):
                    break
                if "backend launched." in buffer.lower() and "> " in buffer:
                    break
            except LaunchError:
                raise
            except Exception as e:
                raise LaunchError(f"Failed while waiting for oryn readiness: {e}")

    async def send(self, command: str) -> str:
        """Send a command and receive the response.

        Args:
            command: Intent Language command to send

        Returns:
            Response string from oryn
        """
        if not self._connected or not self._process:
            raise ConnectionLostError()

        async with self._lock:
            return await self._send_locked(command)

    async def _send_locked(self, command: str) -> str:
        """Send command while holding the lock."""
        if not self._process or not self._process.stdin or not self._process.stdout:
            raise ConnectionLostError()

        # Check if process is still running
        if self._process.returncode is not None:
            raise ConnectionLostError(self._process.returncode)

        # Send command
        cmd_bytes = (command.strip() + "\n").encode("utf-8")
        self._process.stdin.write(cmd_bytes)
        await self._process.stdin.drain()

        # Read response until we see the next prompt or end of output
        try:
            response = await asyncio.wait_for(
                self._read_response(),
                timeout=self._config.timeout,
            )
            return response
        except asyncio.TimeoutError:
            if self._process.returncode is not None:
                raise ConnectionLostError(self._process.returncode)
            raise TimeoutError(command.split()[0] if command else "command", self._config.timeout)

    async def _read_response(self) -> str:
        """Read response from stdout until prompt appears."""
        if not self._process or not self._process.stdout:
            return ""

        buffer = ""
        while True:
            try:
                # Read available data
                chunk = await self._process.stdout.read(4096)
                if not chunk:
                    # EOF means stdout closed - treat as lost connection.
                    returncode = self._process.returncode
                    if returncode is None:
                        await asyncio.sleep(0.01)
                        returncode = self._process.returncode
                    raise ConnectionLostError(returncode)

                buffer += chunk.decode("utf-8", errors="replace")

                # Check for prompt indicating response is complete
                # The REPL shows "> " at the start of a line after each response
                if "\n> " in buffer or buffer.endswith("\n>") or buffer.endswith("> "):
                    # Remove the prompt from the response
                    if "\n> " in buffer:
                        buffer = buffer.rsplit("\n> ", 1)[0]
                    elif buffer.endswith("\n>"):
                        buffer = buffer[:-2]
                    elif buffer.endswith("> "):
                        buffer = buffer[:-2]
                        if buffer.endswith("\n"):
                            buffer = buffer[:-1]
                    break

            except asyncio.CancelledError:
                raise
            except Exception as e:
                # Connection issue
                returncode = self._process.returncode if self._process else None
                raise ConnectionLostError(returncode) from e

        return buffer.strip()

    async def close(self) -> None:
        """Close the connection and terminate the subprocess."""
        self._connected = False

        if self._process:
            try:
                # Send exit command
                if self._process.stdin and not self._process.stdin.is_closing():
                    self._process.stdin.write(b"exit\n")
                    await self._process.stdin.drain()
                    self._process.stdin.close()

                # Wait briefly for graceful shutdown
                try:
                    await asyncio.wait_for(self._process.wait(), timeout=5.0)
                except asyncio.TimeoutError:
                    pass
            except Exception:
                pass

            await self._kill_process()

        if self._stderr_task:
            try:
                await self._stderr_task
            except Exception:
                pass
            self._stderr_task = None

        if self._log_file_handle:
            try:
                self._log_file_handle.close()
            except Exception:
                pass
            self._log_file_handle = None

    async def _kill_process(self) -> None:
        """Force kill the subprocess."""
        if self._process:
            try:
                self._process.kill()
                await self._process.wait()
            except ProcessLookupError:
                pass
            except Exception:
                pass
            self._process = None

    async def _drain_stderr(self) -> None:
        """Continuously drain stderr to avoid subprocess backpressure deadlocks."""
        if not self._process or not self._process.stderr:
            return

        while True:
            chunk = await self._process.stderr.read(4096)
            if not chunk:
                break

            text = chunk.decode("utf-8", errors="replace")
            self._append_stderr_tail(text)

            if self._log_file_handle:
                try:
                    self._log_file_handle.write(text)
                    self._log_file_handle.flush()
                except Exception:
                    pass

    def _append_stderr_tail(self, text: str) -> None:
        """Append stderr output to a bounded in-memory tail."""
        if not text:
            return

        self._stderr_tail.append(text)
        self._stderr_tail_chars += len(text)

        while self._stderr_tail and self._stderr_tail_chars > self._stderr_tail_limit_chars:
            removed = self._stderr_tail.popleft()
            self._stderr_tail_chars -= len(removed)

    def _get_stderr_tail(self) -> str:
        """Get recent stderr output for diagnostics."""
        if not self._stderr_tail:
            return ""
        return "".join(self._stderr_tail).strip()

    def is_connected(self) -> bool:
        """Check if connected to oryn."""
        if not self._connected or not self._process:
            return False
        return self._process.returncode is None

    @property
    def binary_path(self) -> Optional[str]:
        """Get the path to the oryn binary."""
        return self._binary
