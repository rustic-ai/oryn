"""Strict version-one contracts. Only typed input fields declare dependencies.

Digests are SHA-256 of compact UTF-8 JSON with sorted keys. File digests hash
bytes; file records also pin size and executable permission. Directory digests
hash {directories, members}, including empty folders and member permissions.
Unknown fields and unknown schema versions fail closed; migration is explicit.
"""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path, PurePosixPath
from typing import Any

SCHEMA_VERSION = 1
CORE_AUTOMATED = 15_985
CORE_ASSESSMENTS = 752
_HEX = re.compile(r"[0-9a-f]{64}\Z")
_COMMIT = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")


class ManifestError(ValueError):
    """A manifest is malformed or inconsistent with its declared contract."""


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                      allow_nan=False).encode("utf-8")


def manifest_digest(document: dict) -> str:
    return hashlib.sha256(canonical_bytes(document)).hexdigest()


def _object(value: Any, required: set[str], optional: set[str] = frozenset(),
            label: str = "object") -> dict:
    if not isinstance(value, dict) or set(value) - required - optional or required - set(value):
        raise ManifestError(f"{label}: expected fields {sorted(required)}; optional {sorted(optional)}")
    return value


def _text(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise ManifestError(f"{label}: expected nonempty string")
    return value


def _digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or not _HEX.fullmatch(value):
        raise ManifestError(f"{label}: expected lowercase SHA-256")
    return value


def _commit(value: Any, label: str) -> None:
    if not isinstance(value, str) or not _COMMIT.fullmatch(value):
        raise ManifestError(f"{label}: expected full Git object identity")


def _id(value: Any, label: str) -> None:
    if not isinstance(value, str) or not _ID.fullmatch(value) or value in {".", ".."}:
        raise ManifestError(f"{label}: invalid identifier")


def _integer(value: Any, label: str, minimum: int = 0) -> None:
    if type(value) is not int or value < minimum:
        raise ManifestError(f"{label}: expected integer >= {minimum}")


def relative_path(value: Any) -> str:
    _text(value, "path")
    path = PurePosixPath(value)
    if (path.is_absolute() or "\\" in value or ":" in value or
            any(part in {"", ".", ".."} for part in value.split("/")) or
            path.as_posix() != value):
        raise ManifestError(f"unsafe relative path: {value!r}")
    return value


def _list(value: Any, label: str) -> list:
    if not isinstance(value, list):
        raise ManifestError(f"{label}: expected list")
    return value


def validate_input(record: dict) -> dict:
    _object(record, {"scope", "path", "kind", "sha256"}, {"size", "executable", "members", "directories"}, "input")
    if not isinstance(record["scope"], str) or record["scope"] not in {"store", "bundle"}:
        raise ManifestError("input.scope: expected store or bundle")
    relative_path(record["path"])
    _digest(record["sha256"], "input.sha256")
    if record["kind"] == "file":
        _object(record, {"scope", "path", "kind", "sha256", "size", "executable"}, label="file input")
        _integer(record["size"], "input.size")
        if type(record["executable"]) is not bool:
            raise ManifestError("input.executable: expected boolean")
    elif record["kind"] == "directory":
        _object(record, {"scope", "path", "kind", "sha256", "members", "directories"}, label="directory input")
        paths = []
        for member in _list(record["members"], "members"):
            _object(member, {"path", "sha256", "size", "executable"}, label="member")
            paths.append(relative_path(member["path"]))
            _digest(member["sha256"], "member.sha256")
            _integer(member["size"], "member.size")
            if type(member["executable"]) is not bool:
                raise ManifestError("member.executable: expected boolean")
        dirs = _list(record["directories"], "directories")
        for directory in dirs:
            relative_path(directory)
        if paths != sorted(set(paths)) or dirs != sorted(set(dirs)) or set(paths) & set(dirs):
            raise ManifestError("directory membership must be sorted, unique and disjoint")
        directory_names = set(dirs)
        all_paths = set(paths) | directory_names
        for name in all_paths:
            parent = PurePosixPath(name).parent
            if str(parent) != "." and str(parent) not in directory_names:
                raise ManifestError(f"missing declared parent directory: {parent}")
        if manifest_digest({"directories": dirs, "members": record["members"]}) != record["sha256"]:
            raise ManifestError("directory membership digest mismatch")
    else:
        raise ManifestError("input.kind: expected file or directory")
    return record


def _inputs(value: Any) -> None:
    seen = set()
    for record in _list(value, "inputs"):
        validate_input(record)
        key = (record["scope"], record["path"])
        if key in seen:
            raise ManifestError("duplicate declared input")
        seen.add(key)


def _identity(value: Any, dirty: bool = False) -> None:
    _object(value, {"commit", "tree_sha256"} | ({"dirty"} if dirty else set()), label="source identity")
    _commit(value["commit"], "commit")
    _digest(value["tree_sha256"], "tree_sha256")
    if dirty and type(value["dirty"]) is not bool:
        raise ManifestError("source.dirty: expected boolean")


def _build_ref(value: Any) -> None:
    _object(value, {"build_id", "manifest_sha256"}, label="build reference")
    _id(value["build_id"], "build_id")
    _digest(value["manifest_sha256"], "manifest_sha256")


def _build(document: dict) -> None:
    _object(document, {"schema_version", "kind", "build_id", "source", "build_inputs_sha256",
                      "configuration", "commands", "build_inputs", "immutable_inputs", "artifacts", "signatures",
                      "development_only"}, {"runner", "provenance", "platform"}, "build")
    _id(document["build_id"], "build_id")
    _identity(document["source"], dirty=True)
    _digest(document["build_inputs_sha256"], "build_inputs_sha256")
    cfg = _object(document["configuration"], {"cargo_lock_sha256", "toolchain", "host", "target", "profile", "features", "flags"}, label="configuration")
    _digest(cfg["cargo_lock_sha256"], "cargo_lock_sha256")
    for key in ("toolchain", "host", "target", "profile"):
        _text(cfg[key], key)
    for key in ("features", "flags"):
        for item in _list(cfg[key], key):
            _text(item, key)
    commands = _list(document["commands"], "commands")
    if not commands:
        raise ManifestError("build requires exact build commands")
    for command in commands:
        if not _list(command, "command"):
            raise ManifestError("empty build command")
        for arg in command:
            _text(arg, "argument")
    _inputs(document["build_inputs"])
    _inputs(document["immutable_inputs"])
    if any(record not in document["immutable_inputs"] for record in document["build_inputs"]):
        raise ManifestError("prelaunch build input must remain in immutable admission inputs")
    _inputs(document["artifacts"])
    if not document["artifacts"] or any(x["scope"] != "bundle" for x in document["artifacts"]):
        raise ManifestError("build artifacts must declare bundle inputs")
    if type(document["development_only"]) is not bool:
        raise ManifestError("development_only: expected boolean")
    for signature in _list(document["signatures"], "signatures"):
        _object(signature, {"path", "verifier", "identity", "valid"}, label="signature")
        relative_path(signature["path"])
        _text(signature["verifier"], "signature verifier")
        _text(signature["identity"], "signature identity")
        if type(signature["valid"]) is not bool:
            raise ManifestError("signature.valid: expected boolean")
    if not document["development_only"]:
        if document["source"]["dirty"] or not document["signatures"] or not all(x["valid"] for x in document["signatures"]):
            raise ManifestError("official build requires clean source and valid signature observations")
    if "runner" in document:
        _identity(document["runner"])
    if "platform" in document:
        platform = _object(document["platform"], {"system", "architecture", "packaging", "observations"}, label="platform")
        for key in ("system", "architecture", "packaging"):
            _text(platform[key], f"platform.{key}")
        names = set()
        for observation in _list(platform["observations"], "platform.observations"):
            _object(observation, {"name", "expected", "observed"}, label="platform observation")
            for key in observation:
                _text(observation[key], f"platform observation.{key}")
            if observation["name"] in names:
                raise ManifestError("duplicate platform observation")
            names.add(observation["name"])
    if "provenance" in document:
        provenance = _object(document["provenance"], {"original_build_id", "original_manifest_sha256", "original_source_commit"}, label="provenance")
        _id(provenance["original_build_id"], "original_build_id")
        _digest(provenance["original_manifest_sha256"], "original_manifest_sha256")
        _commit(provenance["original_source_commit"], "original_source_commit")


def _run(document: dict) -> None:
    _object(document, {"schema_version", "kind", "run_id", "build", "runner", "suite", "profile", "workers", "expected", "immutable_inputs", "environment", "outputs", "history", "attempt", "original_run_id"}, {"assets"}, "run")
    _id(document["run_id"], "run_id")
    _build_ref(document["build"])
    _identity(document["runner"])
    suite = _object(document["suite"], {"name", "sha256", "upstream_commit"}, label="suite")
    _text(suite["name"], "suite.name")
    _digest(suite["sha256"], "suite.sha256")
    _commit(suite["upstream_commit"], "suite.upstream_commit")
    _text(document["profile"], "profile")
    _integer(document["workers"], "workers", 1)
    expected = _object(document["expected"], {"automated", "assessments"}, label="expected")
    for key, value in expected.items():
        _integer(value, key)
    _inputs(document["immutable_inputs"])
    for observation in _list(document["environment"], "environment"):
        _object(observation, {"name", "expected", "observed"}, label="environment")
        for key in observation:
            _text(observation[key], f"environment.{key}")
    outputs = _object(document["outputs"], {"events", "results", "status"}, label="outputs")
    for path in outputs.values():
        relative_path(path)
    if len(set(outputs.values())) != len(outputs):
        raise ManifestError("outputs must have distinct paths")
    for ref in _list(document["history"], "history"):
        _object(ref, {"run_id", "manifest_sha256"}, label="historical reference")
        _id(ref["run_id"], "history.run_id")
        _digest(ref["manifest_sha256"], "history.manifest_sha256")
    _integer(document["attempt"], "attempt", 1)
    _id(document["original_run_id"], "original_run_id")
    if (document["attempt"] == 1) != (document["run_id"] == document["original_run_id"]):
        raise ManifestError("original/retry identity inconsistent")
    assets = document.get("assets", {})
    if not isinstance(assets, dict):
        raise ManifestError("assets: expected named input mapping")
    for name, record in assets.items():
        _id(name, "asset name")
        validate_input(record)
        if record not in document["immutable_inputs"]:
            raise ManifestError("named asset must also be declared in immutable_inputs")
    for record in document["immutable_inputs"]:
        if record["scope"] == "store":
            for output in outputs.values():
                if output == record["path"] or output.startswith(record["path"] + "/") or record["path"].startswith(output + "/"):
                    raise ManifestError("output and immutable input overlap")


def _result(document: dict) -> None:
    _object(document, {"schema_version", "kind", "run_id", "run_manifest_sha256", "execution", "validity", "counts", "artifacts"}, label="result")
    _id(document["run_id"], "run_id")
    _digest(document["run_manifest_sha256"], "run_manifest_sha256")
    if not isinstance(document["execution"], str) or document["execution"] not in {"completed", "failed", "canceled", "timeout"}:
        raise ManifestError("result execution must be terminal")
    validity = _object(document["validity"], {"valid", "complete", "reasons"}, label="validity")
    if any(type(validity[key]) is not bool for key in ("valid", "complete")):
        raise ManifestError("validity requires booleans")
    for reason in _list(validity["reasons"], "validity.reasons"):
        _text(reason, "reason")
    counts = _object(document["counts"], {"expected", "started", "ended", "outcomes", "assessments"}, label="counts")
    for key in ("expected", "started", "ended", "assessments"):
        _integer(counts[key], f"counts.{key}")
    if not isinstance(counts["outcomes"], dict):
        raise ManifestError("outcomes must be a mapping")
    for key, value in counts["outcomes"].items():
        _text(key, "outcome")
        _integer(value, key)
    if sum(counts["outcomes"].values()) != counts["ended"]:
        raise ManifestError("outcome count does not reconcile with ends")
    if validity["complete"] and not (counts["expected"] == counts["started"] == counts["ended"]):
        raise ManifestError("complete result has incomplete counts")
    if validity["valid"] and (not validity["complete"] or validity["reasons"] or document["execution"] != "completed"):
        raise ManifestError("valid original requires complete terminal evidence with no invalidity reasons")
    _inputs(document["artifacts"])


def _release(document: dict) -> None:
    _object(document, {"schema_version", "kind", "release_id", "build", "required_gates", "gates", "qualified"}, label="release")
    _id(document["release_id"], "release_id")
    _build_ref(document["build"])
    required = _list(document["required_gates"], "required_gates")
    for name in required:
        _id(name, "gate name")
    if not required or len(required) != len(set(required)):
        raise ManifestError("required gates must be nonempty and unique")
    if type(document["qualified"]) is not bool:
        raise ManifestError("qualified: expected boolean")
    seen = set()
    for gate in _list(document["gates"], "gates"):
        _object(gate, {"name", "run_id", "result_sha256", "passed"}, label="gate")
        _id(gate["name"], "gate name")
        _id(gate["run_id"], "gate run_id")
        _digest(gate["result_sha256"], "gate result_sha256")
        if type(gate["passed"]) is not bool or gate["name"] in seen:
            raise ManifestError("invalid/duplicate gate")
        seen.add(gate["name"])
    if seen - set(required):
        raise ManifestError("undeclared release gate")
    if document["qualified"] and (seen != set(required) or not all(g["passed"] for g in document["gates"])):
        raise ManifestError("qualification requires every declared gate passing")


def validate_manifest(document: dict, expected_kind: str | None = None) -> dict:
    if not isinstance(document, dict) or type(document.get("schema_version")) is not int or document["schema_version"] != SCHEMA_VERSION:
        raise ManifestError("unsupported or missing schema_version")
    kind = document.get("kind")
    validators = {"build": _build, "run": _run, "result": _result, "release": _release}
    if not isinstance(kind, str) or kind not in validators or (expected_kind and kind != expected_kind):
        raise ManifestError("unexpected manifest kind")
    validators[kind](document)
    return document


def _unique_object(pairs: list[tuple[str, Any]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ManifestError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_manifest(path: str | Path, expected_kind: str | None = None) -> dict:
    with Path(path).open(encoding="utf-8") as handle:
        try:
            document = json.load(handle, object_pairs_hook=_unique_object,
                                 parse_constant=lambda value: (_ for _ in ()).throw(ManifestError(f"invalid JSON number: {value}")))
        except (json.JSONDecodeError, UnicodeError) as error:
            raise ManifestError(f"invalid manifest JSON: {error}") from error
    return validate_manifest(document, expected_kind)
