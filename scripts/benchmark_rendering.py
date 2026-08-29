#!/usr/bin/env python3
"""Measure the optional renderer without adding benchmark code to production."""

from __future__ import annotations

import argparse
import gzip
import json
import os
import statistics
import subprocess
import time
from pathlib import Path


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))]


def size(path: Path) -> dict[str, int]:
    data = path.read_bytes()
    return {
        "bytes": len(data),
        "gzip_bytes": len(gzip.compress(data, compresslevel=9, mtime=0)),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, required=True)
    parser.add_argument("--parent", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    durations: list[float] = []
    environment = os.environ.copy()
    for sample in range(args.samples):
        start = time.perf_counter()
        subprocess.run(
            [
                str(args.renderer),
                "--html",
                str(args.fixture),
                "--png",
                str(args.output_dir / f"sample-{sample}.png"),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=environment,
        )
        durations.append((time.perf_counter() - start) * 1000.0)
    result = {
        "schema_version": 1,
        "suite": "cpu-reference-renderer-benchmark",
        "samples": args.samples,
        "startup_to_screenshot_ms": {
            "p50": statistics.median(durations),
            "p95": percentile(durations, 0.95),
            "min": min(durations),
            "max": max(durations),
        },
        "sizes": {
            "parent": size(args.parent),
            "worker": size(args.worker),
            "renderer": size(args.renderer),
        },
        "outcome": {"status": "passed"},
    }
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
