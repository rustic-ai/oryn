"""Hold the host's real-focus lease until the reference driver's stdin closes.

Kernel advisory locking releases on crashes; no stale PID-file deletion or
page-visible focus emulation is involved. Native Oryn executions stay parallel.
"""
import fcntl
import os
from pathlib import Path
import sys
import tempfile


def main():
    path = Path(tempfile.gettempdir()) / f'oryn-wpt-reference-focus-{os.getuid()}.lock'
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'rb') as lease:
        fcntl.flock(lease, fcntl.LOCK_EX)
        print('ready', flush=True)
        sys.stdin.buffer.read()


if __name__ == '__main__':
    main()
