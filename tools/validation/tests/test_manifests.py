"""Strict schema and identity-boundary regressions."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

from oryn_validation.manifests import ManifestError, load_manifest, manifest_digest, validate_manifest

D = "a" * 64
C = "b" * 40
INPUT = {"scope": "bundle", "path": "bin/oryn", "kind": "file", "sha256": D, "size": 10, "executable": False}


def build():
    return {"schema_version": 1, "kind": "build", "build_id": "build-1",
            "source": {"commit": C, "tree_sha256": D, "dirty": False},
            "build_inputs_sha256": D,
            "configuration": {"cargo_lock_sha256": D, "toolchain": "rust-1.90", "host": "aarch64-apple-darwin", "target": "aarch64-apple-darwin", "profile": "release", "features": [], "flags": []},
            "commands": [["cargo", "build", "--locked", "--release"]],
            "build_inputs": [], "immutable_inputs": [], "artifacts": [copy.deepcopy(INPUT)],
            "signatures": [{"path": "bin/oryn", "verifier": "codesign", "identity": "test-identity", "valid": True}],
            "development_only": False}


def run():
    return {"schema_version": 1, "kind": "run", "run_id": "run-1", "build": {"build_id": "build-1", "manifest_sha256": D},
            "runner": {"commit": C, "tree_sha256": D}, "suite": {"name": "wpt", "sha256": D, "upstream_commit": C},
            "profile": "standard-block", "workers": 6, "expected": {"automated": 15985, "assessments": 752},
            "immutable_inputs": [copy.deepcopy(INPUT)], "environment": [],
            "outputs": {"events": "runs/run-1/events.jsonl", "results": "runs/run-1/results.json", "status": "status/run-1.json"},
            "history": [], "attempt": 1, "original_run_id": "run-1"}


class ManifestTests(unittest.TestCase):
    def test_build_and_runner_identities_remain_distinct(self):
        native = build()
        execution = run()
        before = manifest_digest(native)
        execution["runner"]["commit"] = "c" * 40
        validate_manifest(native, "build")
        validate_manifest(execution, "run")
        self.assertEqual(manifest_digest(native), before)
        self.assertNotEqual(execution["runner"]["commit"], native["source"]["commit"])

    def test_prelaunch_inputs_are_required_and_retained_in_admission(self):
        document = build()
        del document["build_inputs"]
        with self.assertRaises(ManifestError):
            validate_manifest(document)
        document = build()
        document["build_inputs"] = [copy.deepcopy(INPUT)]
        with self.assertRaises(ManifestError):
            validate_manifest(document)
        document["immutable_inputs"] = [copy.deepcopy(INPUT)]
        validate_manifest(document)
        # Later evidence belongs to admission without changing compilation inputs.
        compilation = manifest_digest(document["build_inputs"])
        document["immutable_inputs"].append(dict(INPUT, path="receipts/process.json"))
        validate_manifest(document)
        self.assertEqual(compilation, manifest_digest(document["build_inputs"]))

    def test_unknown_version_field_and_boolean_number_rejected(self):
        for key, value in (("schema_version", 2), ("schema_version", True), ("workers", True), ("undeclared_path", "/legacy")):
            document = run()
            document[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ManifestError):
                validate_manifest(document)

    def test_dirty_official_build_or_invalid_signature_rejected(self):
        for mutation in (lambda d: d["source"].update(dirty=True), lambda d: d.update(signatures=[]), lambda d: d["signatures"][0].update(valid=False)):
            document = build()
            mutation(document)
            with self.assertRaises(ManifestError):
                validate_manifest(document)
        document = build()
        document["development_only"] = True
        document["source"]["dirty"] = True
        document["signatures"] = []
        validate_manifest(document)

    def test_output_overlap_and_asset_without_input_rejected(self):
        document = run()
        document["immutable_inputs"][0].update(scope="store", path="status/run-1.json")
        with self.assertRaises(ManifestError):
            validate_manifest(document)
        document = run()
        document["assets"] = {"driver": dict(INPUT, path="bin/driver")}
        with self.assertRaises(ManifestError):
            validate_manifest(document)

    def test_duplicate_json_keys_and_nonfinite_numbers_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            for text in ('{"kind":"run","kind":"build"}', '{"schema_version":NaN}'):
                path.write_text(text)
                with self.assertRaises(ManifestError):
                    load_manifest(path)

    def test_retry_cannot_overwrite_original_identity(self):
        document = run()
        document["attempt"] = 2
        with self.assertRaises(ManifestError):
            validate_manifest(document)
        document["run_id"] = "run-2"
        validate_manifest(document)

    def test_result_execution_outcomes_and_validity_are_separate(self):
        document = {"schema_version": 1, "kind": "result", "run_id": "run-1", "run_manifest_sha256": D,
                    "execution": "completed", "validity": {"valid": True, "complete": True, "reasons": []},
                    "counts": {"expected": 2, "started": 2, "ended": 2, "outcomes": {"OK": 1, "CRASH": 1}, "assessments": 752}, "artifacts": []}
        validate_manifest(document)
        document["counts"]["ended"] = 1
        with self.assertRaises(ManifestError):
            validate_manifest(document)
        document["counts"]["outcomes"] = {"OK": 1}
        with self.assertRaises(ManifestError):
            validate_manifest(document)

    def test_release_requires_all_declared_gates(self):
        document = {"schema_version": 1, "kind": "release", "release_id": "release-1", "build": {"build_id": "build-1", "manifest_sha256": D}, "required_gates": ["core", "security"], "gates": [{"name": "core", "run_id": "run-1", "result_sha256": D, "passed": True}], "qualified": False}
        validate_manifest(document)
        document["qualified"] = True
        with self.assertRaises(ManifestError):
            validate_manifest(document)


if __name__ == "__main__":
    unittest.main()
