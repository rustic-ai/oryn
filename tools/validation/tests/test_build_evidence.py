import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from oryn_validation.build_evidence import ENGINE_ROLES, FEATURES, ROLES, verify_build_evidence, verify_generation_review
from oryn_validation.generation import REQUIREMENTS
from oryn_validation.inputs import snapshot_input
from oryn_validation.manifests import manifest_digest


class StagedEngineAdmissionTests(unittest.TestCase):
    def test_relocated_selected_manifest_admitted_only_with_exact_bytes_and_closure(self):
        import importlib.util
        import sys
        from unittest.mock import patch
        from oryn_validation import build_evidence
        source = Path(__file__).resolve().parents[3]
        spec = importlib.util.spec_from_file_location('build_engine_profile_fixture',
            source / 'scripts/test_validate_v8_runtime_profile.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        fixture = module.ProfileValidationTests()
        with patch.object(sys, 'path', [str(source / 'scripts'), *sys.path]):
            fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        store = Path(fixture.temp.name).resolve()
        profile = fixture.profile
        for name in module.MODULE.ARTIFACTS:
            profile[name + '_path'] = str(Path(profile[name + '_path']).resolve())
        fixture.bridge['compiler'] = str(Path(fixture.bridge['compiler']).resolve())
        for row in fixture.bridge['inputs']:
            row['path'] = str(Path(row['path']).resolve())
        fixture.write_bridge()
        retained = store / 'selected-source-manifest.json'
        retained.write_bytes(Path(profile['rusty_v8_source_manifest_path']).read_bytes())
        profile['rusty_v8_source_manifest_path'] = str(retained)
        profile_path = store / 'profile.json'
        profile_path.write_text(json.dumps(profile))
        paths = [Path(profile[name + '_path']) for name in module.MODULE.ARTIFACTS]
        paths += [Path(row['path']) for row in fixture.bridge['inputs']]
        declarations = [snapshot_input(store, path.relative_to(store).as_posix()) for path in paths]
        result = build_evidence._engine_files(profile_path, source, store, declarations)
        self.assertEqual({row['path'] for row in result}, {row['path'] for row in declarations})
        with self.assertRaisesRegex(ValueError, 'absent from prelaunch input closure'):
            build_evidence._engine_files(profile_path, source, store, declarations[:-1])
        retained.write_bytes(retained.read_bytes() + b' ')
        profile['rusty_v8_source_manifest_size_bytes'] = retained.stat().st_size
        profile['rusty_v8_source_manifest_sha256'] = hashlib.sha256(retained.read_bytes()).hexdigest()
        profile_path.write_text(json.dumps(profile))
        with self.assertRaisesRegex(ValueError, 'Rust V8 selected source mismatch'):
            build_evidence._engine_files(profile_path, source, store, declarations)


class BuildEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / "source"
        self.source.mkdir()
        plan = self.source / "docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md"
        plan.parent.mkdir(parents=True)
        plan.write_text("complete plan")
        (self.source / "implementation").write_text("implementation")
        (self.source / "regression").write_text("regression")
        self.identity = {"commit": "a" * 40, "tree_sha256": "b" * 64, "dirty": False}
        gate = {"schema_version": 2, "kind": "committed-source-generation", "source": self.identity,
                "plan_sha256": hashlib.sha256(plan.read_bytes()).hexdigest(), "requirements": {}}
        for key in REQUIREMENTS:
            gate["requirements"][key] = {"implementation_generated": True, "tests_generated": True,
                "implementation_sources": [{"path": "implementation", "sha256": hashlib.sha256(b"implementation").hexdigest()}],
                "test_sources": [{"path": "regression", "sha256": hashlib.sha256(b"regression").hexdigest()}]}
        self.assets = {"source_export": self.source}
        self.records = {}
        self.write("build_generation_gate", gate)
        review = {"schema_version": 1, "kind": "generation-review", "gate_sha256": self.records["build_generation_gate"]["sha256"],
                  "source": self.identity, "author": "producer", "reviewer": "independent", "decision": "accepted"}
        self.write("build_generation_review", review)
        self.write("source_identity", self.identity)
        # The complete original compiler closure remains readable through typed
        # paths even when none of its historical absolute paths exists.
        original_root = Path("/historical/build-store")
        self.write("compile_compiler", {"compiler": "fixture"})
        compiler = self.records["compile_compiler"]
        profile = {"schema_version": 6, "profile": "core-full"}
        engine_files = [compiler]
        for name in ENGINE_ROLES:
            content = {"engine": name}
            if name == "bridge_abi":
                content = {"compiler": str(original_root / compiler["path"]), "inputs": [
                    {"path": str(original_root / compiler["path"]), "sha256": compiler["sha256"],
                     "size_bytes": compiler["size"]}]}
            self.write("compile_" + name, content)
            record = self.records["compile_" + name]
            profile.update({name + "_path": str(original_root / record["path"]),
                            name + "_sha256": record["sha256"], name + "_size_bytes": record["size"]})
            engine_files.append(record)
        self.write("compile_profile", profile)
        self.build_inputs = [self.records[name] for name in ("build_generation_gate", "build_generation_review", "source_identity", "compile_profile")]
        self.build_inputs.extend(engine_files)
        self.environment_inputs = {"ORYN_V8_PROFILE_MANIFEST": self.records["compile_profile"],
                                   "ORYN_TYPED_SOURCE_IDENTITY": self.records["source_identity"]}
        literals = {"ORYN_CODESIGN_IDENTITY": "test-identity", "ORYN_TYPED_RUNTIME_OUTPUT": "build-inputs/new"}
        command = ["/bin/bash", "scripts/build-sandboxed-worker.sh"]
        self.write("build_process", {"argv": command, "returncode": 0, "error": None,
                   "timed_out": False, "cleanup_complete": True, "cleanup_errors": [], "cancellation_signal": None,
                   "forced_cleanup": False})
        self.write("build_stdout", {"output": "preserved"})
        self.write("build_stderr", {"output": "preserved"})
        outputs = {}
        for role in ROLES:
            self.write(role, {"artifact": role})
            outputs[role] = self.records[role]
        self.execution = {"schema_version": 1, "kind": "typed-build-execution", "source": self.identity,
                          "configuration": {"profile": "release", "features": sorted(FEATURES),
                                            "toolchain": "rustc fixture", "host": "aarch64-apple-darwin"}, "command": command,
                          "toolchain_observation": "rustc fixture\nhost: aarch64-apple-darwin\n",
                          "build_inputs": self.build_inputs, "environment_inputs": self.environment_inputs,
                          "engine_input_files": engine_files, "original_store_root": str(original_root),
                          "literal_environment": literals,
                          "environment_sha256": manifest_digest({"inputs": self.environment_inputs, "literals": literals}),
                          "generation_gate_sha256": self.records["build_generation_gate"]["sha256"],
                          "generation_review_sha256": self.records["build_generation_review"]["sha256"],
                          "integrity_errors": [], "outputs": outputs,
                          "evidence": {role: self.records["build_" + role]["sha256"] for role in ("process", "stdout", "stderr")}}
        self.write("build_execution", self.execution)
        self.build = {"source": self.identity, "configuration": self.execution["configuration"],
                      "build_inputs": self.build_inputs, "commands": [command]}

    def write(self, role, value):
        path = self.root / (role + ".json")
        path.write_text(json.dumps(value))
        self.assets[role] = path
        self.records[role] = snapshot_input(self.root, path.name)

    def verify(self):
        self.build["immutable_inputs"] = list(self.records.values())
        bound = {"assets": self.assets, "run": {"assets": self.records}, "records": list(self.records.values()), "store_root": self.root}
        return verify_build_evidence(bound, self.build)

    def test_original_build_joins_all_outputs_after_store_relocation(self):
        self.assertEqual(self.verify()["outputs_joined"], sorted(ROLES))

    def test_compiler_cannot_be_omitted_even_with_refreshed_receipt_hash(self):
        self.execution["engine_input_files"] = self.execution["engine_input_files"][1:]
        self.write("build_execution", self.execution)
        with self.assertRaisesRegex(ValueError, "exact compiled closure"):
            self.verify()

    def test_unreferenced_engine_inputs_cannot_replace_exact_population(self):
        self.execution["engine_input_files"].append(self.records["source_identity"])
        self.write("build_execution", self.execution)
        with self.assertRaisesRegex(ValueError, "population differs"):
            self.verify()

    def test_new_unsigned_worker_cannot_be_replaced_with_old_one(self):
        self.write("engine_worker", {"artifact": "old native build"})
        with self.assertRaisesRegex(ValueError, "original build output"):
            self.verify()

    def test_refreshed_hashes_cannot_hide_failed_process(self):
        value = json.loads(self.assets["build_process"].read_text())
        value["returncode"] = 1
        self.write("build_process", value)
        self.execution["evidence"]["process"] = self.records["build_process"]["sha256"]
        self.write("build_execution", self.execution)
        with self.assertRaisesRegex(ValueError, "custody differs"):
            self.verify()

    def test_source_mutation_and_self_review_are_rejected(self):
        (self.source / "implementation").write_text("changed")
        with self.assertRaisesRegex(ValueError, "source differs"):
            self.verify()
        gate = json.loads(self.assets["build_generation_gate"].read_text())
        review = json.loads(self.assets["build_generation_review"].read_text())
        review["reviewer"] = "Producer"
        with self.assertRaisesRegex(ValueError, "review"):
            verify_generation_review(gate, review, self.identity, gate_sha256=self.records["build_generation_gate"]["sha256"])

    def test_saved_accepted_flag_cannot_replace_required_raw_receipt(self):
        self.execution = {"accepted": True, "build_process_evidence_required": False}
        self.write("build_execution", self.execution)
        with self.assertRaises(ValueError):
            self.verify()

    def test_implicit_cargo_configuration_cannot_replace_observed_compiler(self):
        from oryn_validation.build_evidence import _reject_implicit_cargo_configuration
        home = self.root / "home"
        (home / ".cargo").mkdir(parents=True)
        (home / ".cargo/config.toml").write_text('[build]\nrustc-wrapper="unrecorded"\n')
        with self.assertRaisesRegex(ValueError, "implicit Cargo"):
            _reject_implicit_cargo_configuration(self.source, home)


if __name__ == "__main__":
    unittest.main()
