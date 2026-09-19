#!/usr/bin/env python3
"""Record a comparison of two complete Core profile executions; never merge passes."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/wptrunner-oryn"))
from oryn_wptrunner.cookie_profile import comparison


def read(path):
    path = path.resolve()
    data = path.read_bytes()
    return json.loads(data), {"path": str(path), "sha256": hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow", type=Path, required=True, help="Allow-profile execution.json")
    parser.add_argument("--block", type=Path, required=True, help="Block-profile execution.json")
    args = parser.parse_args()
    allow, allow_ref = read(args.allow)
    block, block_ref = read(args.block)
    if allow["cookie_profile"]["third_party_cookies"] != "allow" or block["cookie_profile"]["third_party_cookies"] != "block":
        raise ValueError("execution profile differs from requested report label")
    result = comparison(allow, block)
    result["executions"] = {"allow": allow_ref, "block": block_ref}
    result["qualification"] = "configuration comparison only; inspect each run's complete raw results"
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
