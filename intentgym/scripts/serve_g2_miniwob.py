#!/usr/bin/env python3
"""Serve pinned MiniWoB sources with benchmark-only deterministic seeding."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

ALLOWED_SEEDS = {17, 42, 73}
METADATA_PATH = "/.well-known/oryn-g2-fixture.json"
CORE_SCRIPT_MARKER = b'<script src="../core/core.js"></script>'


def content_hash(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def source_commit(checkout: Path) -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=checkout, text=True
    ).strip()


def inject_seed(source: bytes, seed: int) -> bytes:
    marker_end = source.find(CORE_SCRIPT_MARKER)
    if marker_end < 0:
        raise ValueError(
            "fixture does not load ../core/core.js through the pinned marker"
        )
    marker_end += len(CORE_SCRIPT_MARKER)
    injection = (
        "\n<script data-oryn-g2-seed>"
        "if(typeof Math.seedrandom!=='function')"
        "throw new Error('G2 seedrandom unavailable');"
        f"Math.seedrandom('oryn-g2-{seed}');"
        f"window.__ORYN_G2_SEED__={seed};"
        "</script>"
    ).encode("utf-8")
    return source[:marker_end] + injection + source[marker_end:]


class G2FixtureHandler(SimpleHTTPRequestHandler):
    root: Path
    metadata: dict

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(self.root), **kwargs)

    def do_GET(self) -> None:
        parsed = urlsplit(self.path)
        if parsed.path == METADATA_PATH:
            self._send_json(self.metadata)
            return
        seed_values = parse_qs(parsed.query).get("oryn_seed")
        if not parsed.path.endswith(".html") or not seed_values:
            super().do_GET()
            return
        try:
            seed = int(seed_values[-1])
        except ValueError:
            self.send_error(400, "oryn_seed must be an integer")
            return
        if seed not in ALLOWED_SEEDS:
            self.send_error(400, f"oryn_seed must be one of {sorted(ALLOWED_SEEDS)}")
            return
        requested = (self.root / parsed.path.lstrip("/")).resolve()
        try:
            requested.relative_to(self.root)
        except ValueError:
            self.send_error(403, "path escapes the pinned fixture root")
            return
        if not requested.is_file():
            self.send_error(404)
            return
        try:
            response = inject_seed(requested.read_bytes(), seed)
        except ValueError as error:
            self.send_error(409, str(error))
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(response)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Oryn-G2-Seed", str(seed))
        self.end_headers()
        self.wfile.write(response)

    def _send_json(self, value: dict) -> None:
        response = (json.dumps(value, sort_keys=True) + "\n").encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(response)


def main() -> int:
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--checkout",
        type=Path,
        default=repo_root / "artifacts/deps/miniwob-plusplus",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()

    checkout = args.checkout.resolve()
    root = (checkout / "miniwob/html").resolve()
    if not root.is_dir():
        raise SystemExit(f"MiniWoB HTML root does not exist: {root}")
    metadata = {
        "kind": "oryn_g2_seeded_miniwob",
        "allowed_seeds": sorted(ALLOWED_SEEDS),
        "source_commit": source_commit(checkout),
        "content_sha256": content_hash(root),
        "source_modified": False,
        "injection": "after_core_before_load",
    }
    handler = type(
        "ConfiguredG2FixtureHandler",
        (G2FixtureHandler,),
        {"root": root, "metadata": metadata},
    )
    server = ThreadingHTTPServer((args.host, args.port), handler)
    print(
        json.dumps({**metadata, "url": f"http://{args.host}:{args.port}"}), flush=True
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
