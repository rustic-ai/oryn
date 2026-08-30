#!/usr/bin/env python3
"""Run the four public canaries with read-only commands and separated outcomes."""

from __future__ import annotations

import argparse
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def objects(output: str) -> list[dict[str, Any]]:
    values = []
    for line in output.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            values.append(value)
    return values


def run(binary: Path, task: dict[str, Any], timeout: float) -> dict[str, Any]:
    url = task["source"]
    try:
        completed = subprocess.run(
            [str(binary), "native", "--url", url],
            input="observe --full\ntrace\nexit\n",
            text=True,
            capture_output=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {
            "task_id": task["id"],
            "requested_url": url,
            "final_url": None,
            "transport_availability": "unavailable",
            "challenge": {
                "disposition": "not_assessed",
                "provider": None,
                "signals": [],
            },
            "native_correctness": "not_evaluated",
            "failure_domain": "transport",
            "title": None,
            "semantic_nodes": None,
            "diagnostics": [
                f"navigation exceeded the {timeout:g} second public-canary budget"
            ],
        }
    payloads = objects(completed.stdout)
    observation = next(
        (
            value.get("observation")
            for value in payloads
            if value.get("kind") == "observation"
            and isinstance(value.get("observation"), dict)
        ),
        None,
    )
    trace = next(
        (
            value.get("events")
            for value in reversed(payloads)
            if isinstance(value.get("events"), list)
        ),
        [],
    )
    challenge_event = next(
        (
            event.get("kind")
            for event in reversed(trace)
            if isinstance(event.get("kind"), dict)
            and event["kind"].get("kind") == "challenge_assessed"
        ),
        None,
    )
    raw_signals = (challenge_event or {}).get("signals") or []
    challenge = {
        "disposition": (challenge_event or {}).get("disposition", "not_assessed"),
        "provider": (challenge_event or {}).get("provider"),
        "signals": [
            signal.get("marker") if isinstance(signal, dict) else str(signal)
            for signal in raw_signals
        ],
    }
    transport = "available" if observation else "unavailable"
    nodes = (observation or {}).get("nodes") or []
    correctness = (
        "passed" if nodes else "failed" if transport == "available" else "not_evaluated"
    )
    if challenge["disposition"] == "confirmed":
        failure_domain = "site_challenge"
    elif transport == "unavailable":
        failure_domain = "transport"
    elif correctness == "failed" or completed.returncode != 0:
        failure_domain = "native_runtime"
    else:
        failure_domain = "none"
    page = (observation or {}).get("page") or {}
    diagnostics = list((observation or {}).get("diagnostics") or [])
    if completed.returncode != 0 and completed.stderr.strip():
        diagnostics.append(completed.stderr.strip()[-500:])
    return {
        "task_id": task["id"],
        "requested_url": url,
        "final_url": page.get("url"),
        "transport_availability": transport,
        "challenge": challenge,
        "native_correctness": correctness,
        "failure_domain": failure_domain,
        "title": page.get("title"),
        "semantic_nodes": len(nodes) if observation else None,
        "diagnostics": diagnostics,
    }


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--oryn-binary", type=Path, default=root / "target/release/oryn"
    )
    parser.add_argument("--timeout", type=float, default=90.0)
    args = parser.parse_args()
    corpus = json.loads((root / "benchmarks/corpus/semantic-loop.json").read_text())
    tasks = [task for task in corpus["tasks"] if task["class"] == "public_canary"]
    if any(task.get("policy") != "read_only" for task in tasks) or len(tasks) != 4:
        raise RuntimeError("public canary corpus is not exactly four read-only tasks")
    results = [run(args.oryn_binary, task, args.timeout) for task in tasks]
    count = lambda field, value: sum(item[field] == value for item in results)
    challenge_count = lambda value: sum(
        item["challenge"]["disposition"] == value for item in results
    )
    print(
        json.dumps(
            {
                "schema_version": 2,
                "suite": "public_canaries",
                "captured_at": datetime.now(timezone.utc)
                .isoformat()
                .replace("+00:00", "Z"),
                "policy": "read_only",
                "classification_revision": {
                    "source": "live",
                    "method": "confirmed_only_detector_v1",
                },
                "results": results,
                "summary": {
                    "published": 4,
                    "transport_available": count("transport_availability", "available"),
                    "transport_unavailable": count(
                        "transport_availability", "unavailable"
                    ),
                    "challenge_confirmed": challenge_count("confirmed"),
                    "challenge_suspected": challenge_count("suspected"),
                    "challenge_not_detected": challenge_count("not_detected"),
                    "challenge_not_assessed": challenge_count("not_assessed"),
                    "correctness_passed": count("native_correctness", "passed"),
                    "correctness_failed": count("native_correctness", "failed"),
                    "correctness_not_evaluated": count(
                        "native_correctness", "not_evaluated"
                    ),
                },
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
