"""Validate and expand the versioned full five-family acceptance contract.

This module plans required attempts; it neither executes workflows nor treats
existing controlled-case passes as evidence for the expanded contract.
"""
import hashlib
import json
from pathlib import Path

FROZEN_CONTRACT_SHA256 = "3fb9e1173b54ae60828c73f86af6f80dd31956c1c4ccda296bf970af68a0a556"
ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "test-harness/workflows/five-family-contract-v1.json"
FAMILIES = {"research", "forms", "shopping", "saas", "booking"}
DIMENSIONS = {
    "client_rendering", "ssr_hydration", "routing_chunks", "portals_custom_controls",
    "cross_origin_frames", "async_persistence", "client_timezones",
}
FAULTS = ("delayed_response", "worker_failure", "duplicate_delivery", "cancellation")


def validate_contract(contract):
    if contract.get("schema_version") != 1 or not contract.get("contract_id"):
        raise ValueError("unsupported or unnamed workflow contract")
    sampling = contract.get("sampling", {})
    if sampling != {"serial": 3, "mixed_family": 3, "recovery_repeat": 30,
                    "fault_injection": 1, "context_counts": [1, 5, 20]}:
        raise ValueError("sampling differs from the frozen acceptance contract")
    dimensions = contract.get("required_frontend_dimensions", [])
    if len(dimensions) != len(DIMENSIONS) or set(dimensions) != DIMENSIONS:
        raise ValueError("missing or duplicate frontend dimension")
    families = contract.get("families", [])
    ids = [family.get("id") for family in families]
    if len(ids) != len(FAMILIES) or set(ids) != FAMILIES:
        raise ValueError("missing or duplicate workflow family")
    cells = set()
    for family in families:
        if not family.get("cells"):
            raise ValueError("empty workflow family")
        for cell in family["cells"]:
            identity = cell.get("id", "")
            if not identity.startswith(family["id"] + ".") or identity in cells:
                raise ValueError("invalid or duplicate behavior cell")
            cells.add(identity)
            for field in ("behavior", "independent_outcome"):
                if not isinstance(cell.get(field), str) or not cell[field].strip():
                    raise ValueError("behavior cell lacks an independent outcome contract")
            if type(cell.get("recovery_sensitive")) is not bool:
                raise ValueError("behavior cell lacks explicit recovery classification")
    if not contract.get("regression_inputs"):
        raise ValueError("missing immutable regression inputs")
    paths = set()
    for entry in contract["regression_inputs"]:
        path = entry.get("path", "")
        digest = entry.get("sha256", "")
        if not path or path in paths or Path(path).is_absolute() or ".." in Path(path).parts:
            raise ValueError("invalid or duplicate regression input path")
        if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
            raise ValueError("invalid regression input identity")
        paths.add(path)
    return contract


def load_contract(path=DEFAULT_CONTRACT, root=ROOT):
    encoded = Path(path).read_bytes()
    if hashlib.sha256(encoded).hexdigest() != FROZEN_CONTRACT_SHA256:
        raise ValueError("workflow contract differs from the frozen v1 identity")
    contract = validate_contract(json.loads(encoded))
    for entry in contract["regression_inputs"]:
        source = (Path(root) / entry["path"]).resolve()
        if not source.is_relative_to(Path(root).resolve()):
            raise ValueError("regression input escapes repository")
        if hashlib.sha256(source.read_bytes()).hexdigest() != entry["sha256"]:
            raise ValueError("regression input changed: " + entry["path"])
    return contract


def planned_attempts(contract):
    """Stable attempt IDs prevent retries from replacing failed observations."""
    validate_contract(contract)
    families = contract["families"]
    interleaved = [
        (family["id"], family["cells"][index])
        for index in range(max(len(family["cells"]) for family in families))
        for family in families if index < len(family["cells"])
    ]
    attempts = []
    for mode in ("serial", "mixed_family", "recovery_repeat", "fault_injection"):
        for sample in range(1, contract["sampling"][mode] + 1):
            for family, cell in interleaved:
                if mode in ("recovery_repeat", "fault_injection") and not cell["recovery_sensitive"]:
                    continue
                contexts = contract["sampling"]["context_counts"] if mode == "mixed_family" else [1]
                for context_count in contexts:
                    for fault in FAULTS if mode == "fault_injection" else (None,):
                        attempts.append({
                            "attempt_id": f"{cell['id']}:{mode}:{sample}:{fault or 'none'}:contexts={context_count}",
                            "family": family, "cell": cell["id"], "mode": mode,
                            "sample": sample, "fault": fault, "context_count": context_count,
                            "independent_outcome": cell["independent_outcome"],
                        })
    return attempts


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    contract = load_contract(args.contract)
    result = {
        "contract_id": contract["contract_id"],
        "contract_sha256": hashlib.sha256(args.contract.read_bytes()).hexdigest(),
        "kind": "planned_attempts", "plan_schema_version": 2,
        "qualification": "not_execution_evidence",
        "attempts": planned_attempts(contract),
    }
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
