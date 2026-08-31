#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-completion-v1.json"


def main() -> int:
    value = json.loads(MANIFEST.read_text(encoding="utf-8"))
    assert value["schema_version"] == 1
    assert value["locked"] is True
    assert len(value["upstream"]["revision"]) == 40
    entries = value["classifications"]
    ids = [entry["id"] for entry in entries]
    assert len(ids) == len(set(ids))
    required = [entry for entry in entries if entry["disposition"] == "required"]
    assert len(required) == 9
    assert value["side_effect_policy"] == "recorded_fixtures_and_read_only_public_canaries"
    print(json.dumps({"manifest": str(MANIFEST.relative_to(ROOT)), "required": len(required), "status": "valid"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
