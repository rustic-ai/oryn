#!/usr/bin/env python3
"""Run pinned wptserve privately behind Oryn's conformance harness."""

from __future__ import annotations

import argparse
import json
import logging
import signal
import socket
import sys
import threading
from pathlib import Path


def free_loopback_port() -> int:
    probe = socket.socket()
    probe.bind(("127.0.0.1", 0))
    port = int(probe.getsockname()[1])
    probe.close()
    return port


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--public-port", type=int, required=True)
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    sys.path.insert(0, str(checkout / "tools/wptserve"))

    try:
        import wptserve
        from wptserve.config import ConfigBuilder
    except ModuleNotFoundError as error:
        raise SystemExit(
            "pinned wptserve dependencies are missing; run "
            f"{checkout / '_venv3/bin/pip'} install -e {checkout / 'tools/wptserve'}"
        ) from error

    logger = logging.getLogger("oryn-wptserve")
    internal_port = free_loopback_port()
    stopped = threading.Event()

    with ConfigBuilder(
        logger,
        browser_host="localhost",
        alternate_hosts={"alt": "127.0.0.1"},
        subdomains={"www1", "www2"},
        ports={"http": [args.public_port]},
        check_subdomains=False,
        doc_root=str(checkout),
        bind_address="127.0.0.1",
    ) as config:
        server = wptserve.WebTestHttpd(
            host="127.0.0.1",
            port=internal_port,
            use_ssl=False,
            certificate=None,
            doc_root=str(checkout),
            rewrites=[
                (
                    "GET",
                    "/resources/WebIDLParser.js",
                    "/resources/webidl2/lib/webidl2.js",
                )
            ],
            config=config,
        )

        def stop(_signum: int, _frame: object) -> None:
            stopped.set()

        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        server.start()
        print(json.dumps({"port": internal_port}), flush=True)
        stopped.wait()
        server.stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
