#!/usr/bin/env python3
"""Correct the historical canary classification without pretending to rerun it."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "benchmarks/evidence/semantic-public-canaries.json"


def corrected(item: dict) -> dict:
    transport = "available" if item.get("final_url") else "unavailable"
    correctness = item["correctness"]
    if item["availability"] == "challenged" and any(
        marker in " ".join(item.get("diagnostics") or []).lower()
        for marker in ("unexpected token", "empty semantic observation")
    ):
        challenge = {"disposition": "not_assessed", "provider": None, "signals": []}
        failure_domain = "native_runtime"
    elif transport == "unavailable":
        challenge = {"disposition": "not_assessed", "provider": None, "signals": []}
        failure_domain = "transport"
    else:
        challenge = {"disposition": "not_assessed", "provider": None, "signals": []}
        failure_domain = "none" if correctness == "passed" else "native_runtime"
    return {
        "task_id": item["task_id"],
        "requested_url": item["requested_url"],
        "final_url": item.get("final_url"),
        "transport_availability": transport,
        "challenge": challenge,
        "native_correctness": correctness,
        "failure_domain": failure_domain,
        "title": item.get("title"),
        "semantic_nodes": item.get("semantic_nodes"),
        "diagnostics": item.get("diagnostics") or [],
    }


def build() -> dict:
    source = json.loads(SOURCE.read_text(encoding="utf-8"))
    results = [corrected(item) for item in source["results"]]
    count = lambda field, value: sum(item[field] == value for item in results)
    challenge_count = lambda value: sum(
        item["challenge"]["disposition"] == value for item in results
    )
    return {
        "schema_version": 2,
        "suite": "public_canaries",
        "policy": "read_only",
        "classification_revision": {
            "source": "benchmarks/evidence/semantic-public-canaries.json",
            "source_commit": source["source_commit"],
            "captured_at": source["captured_at"],
            "method": "classification_only_no_network_rerun",
        },
        "results": results,
        "summary": {
            "published": len(results),
            "transport_available": count("transport_availability", "available"),
            "transport_unavailable": count("transport_availability", "unavailable"),
            "challenge_confirmed": challenge_count("confirmed"),
            "challenge_suspected": challenge_count("suspected"),
            "challenge_not_detected": challenge_count("not_detected"),
            "challenge_not_assessed": challenge_count("not_assessed"),
            "correctness_passed": count("native_correctness", "passed"),
            "correctness_failed": count("native_correctness", "failed"),
            "correctness_not_evaluated": count("native_correctness", "not_evaluated"),
        },
    }


if __name__ == "__main__":
    print(json.dumps(build(), indent=2))
