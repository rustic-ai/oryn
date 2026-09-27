"""Exercise canonical packaging path wiring with mocked build/sign commands.

No Cargo build, certificate operation or runtime qualification occurs here.
"""
import os
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from oryn_validation.builds import build_command_plan


SOURCE = Path(__file__).resolve().parents[3]


class CanonicalBuildPaths(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.checkout = self.root / "checkout with spaces"
        self.scripts = self.checkout / "scripts"
        self.scripts.mkdir(parents=True)
        self.commands = self.root / "mock-commands"
        self.commands.mkdir()
        self.target = self.root / "cache with spaces"
        self.artifacts = self.root / "artifacts with spaces"
        self.trace = self.root / "trace"
        self.fixture_binary = self.root / "fixture-binary"
        self.executable(self.fixture_binary, 'printf "%s\\n" "runtime $*" >> "$TRACE"\nprintf "{}\\n"\n')
        self.env = {**os.environ, "PATH": str(self.commands) + os.pathsep + os.environ["PATH"],
                    "CARGO_TARGET_DIR": str(self.target), "ORYN_ARTIFACT_ROOT": str(self.artifacts),
                    "ORYN_CODESIGN_IDENTITY": "mock-identity", "TRACE": str(self.trace),
                    "FIXTURE_BINARY": str(self.fixture_binary)}
        for name in ("build-platform-release.sh", "package-macos-app.sh"):
            shutil.copyfile(SOURCE / "scripts" / name, self.scripts / name)
        (self.scripts / "generation_gate.py").write_text(
            'import os\nfrom pathlib import Path\np = Path(os.environ["TRACE"])\n'
            'with p.open("a") as out: out.write("generation gate checked\\n")\n')
        self.executable(self.commands / "jq", 'cat >/dev/null\nprintf "%s\\n" "jq $*" >> "$TRACE"\nprintf "{}\\n"\n')

    @staticmethod
    def executable(path, body):
        path.write_text("#!/bin/sh\nset -eu\n" + body)
        path.chmod(0o755)

    def test_linux_uses_requested_cache_and_artifact_roots(self):
        self.executable(self.commands / "uname", 'printf "Linux\\n"\n')
        self.executable(self.commands / "cargo", 'mkdir -p "$CARGO_TARGET_DIR/release"\n'
                        'cp "$FIXTURE_BINARY" "$CARGO_TARGET_DIR/release/oryn"\n')
        subprocess.run(["/bin/bash", str(self.scripts / "build-platform-release.sh")],
                       cwd=self.root, env=self.env, check=True, capture_output=True)
        self.assertTrue((self.artifacts / "oryn").is_file())
        self.assertTrue((self.target / "release/oryn").is_file())
        self.assertFalse((self.checkout / "target").exists())
        self.assertIn("runtime webdriver --help", self.trace.read_text())
        self.assertEqual(self.trace.read_text().count("generation gate checked"), 2)

    def test_macos_packages_requested_worker_and_handles_spaces(self):
        self.executable(self.commands / "uname", 'printf "Darwin\\n"\n')
        self.executable(self.commands / "codesign", 'printf "%s\\n" "codesign $*" >> "$TRACE"\n')
        self.executable(self.commands / "ditto", 'exec /bin/cp -R "$1" "$2"\n')
        self.executable(self.scripts / "build-sandboxed-worker.sh",
                        'mkdir -p "$CARGO_TARGET_DIR/release" "$ORYN_ARTIFACT_ROOT/OrynPageWorker.app/Contents/MacOS"\n'
                        'cp "$FIXTURE_BINARY" "$CARGO_TARGET_DIR/release/oryn"\n'
                        'cp "$FIXTURE_BINARY" "$ORYN_ARTIFACT_ROOT/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"\n')
        (self.checkout / "sandbox").mkdir()
        (self.checkout / "sandbox/oryn-Info.plist").write_text("mock plist")
        subprocess.run(["/bin/bash", str(self.scripts / "package-macos-app.sh")],
                       cwd=self.root, env=self.env, check=True, capture_output=True)
        self.assertTrue((self.artifacts / "Oryn.app/Contents/MacOS/oryn").is_file())
        self.assertTrue((self.artifacts / "Oryn.app/Contents/Helpers/OrynPageWorker.app/Contents/MacOS/oryn-page-worker").is_file())
        self.assertFalse((self.checkout / "target").exists())
        trace = self.trace.read_text()
        self.assertTrue(trace.startswith("generation gate checked\n"))
        self.assertIn("codesign --verify --deep --strict", trace)
        self.assertIn("runtime native --runtime-info", trace)

    def test_build_plan_requires_actual_profile_arguments_and_keeps_gate(self):
        config = {"toolchain": "rust-1.90", "host": "aarch64-apple-darwin",
                  "target": "aarch64-apple-darwin", "profile": "release", "features": [], "flags": []}
        with self.assertRaisesRegex(ValueError, "argument count"):
            build_command_plan(SOURCE, self.root, config, script="scripts/build-v8-runtime-profile.sh")
        plan = build_command_plan(SOURCE, self.root, config,
                                  script="scripts/build-v8-runtime-profile.sh",
                                  arguments=["core-full", str(self.artifacts)])
        self.assertFalse(plan["execution_authorized"])
        self.assertIn("GenerationLease", plan["required_admission"])
        self.assertNotIn(str(SOURCE / "target"), str(plan["environment_overlay"]))

    def typed_worker_environment(self):
        for name in ('build-sandboxed-worker.sh', 'build-startup-snapshot.sh'):
            shutil.copyfile(SOURCE / 'scripts' / name, self.scripts / name)
        profile = self.root / 'staged-profile.json'
        artifact = self.root / 'retained-input'
        artifact.write_text('retained')
        profile.write_text(json.dumps({role + '_path': str(artifact) for role in
                                      ('archive', 'binding', 'startup_snapshot', 'icu_data', 'bridge_abi')}))
        env = {**self.env, 'ORYN_V8_PROFILE_MANIFEST': str(profile),
               'ORYN_TYPED_SOURCE_IDENTITY': str(self.root / 'source-identity.json'),
               'ORYN_TYPED_RUNTIME_OUTPUT': 'builds/new/engine', 'ORYN_DATA_ROOT': str(self.root)}
        self.executable(self.commands / 'uname', 'printf "Darwin\\n"\n')
        self.executable(self.commands / 'rustc', 'printf "host: aarch64-apple-darwin\\n"\n')
        self.executable(self.commands / 'security', 'printf "mock-identity\\n"\n')
        self.executable(self.commands / 'plutil', 'exit 0\n')
        # Only setup/profile helpers are mocked. Run the two real shell scripts
        # and stop at the first parent Cargo build, before runtime/signing work.
        python = self.commands / 'python3'
        python.write_text('#!/usr/bin/python3\nimport json,os,sys\nfrom pathlib import Path\n'
            'args=sys.argv[1:]\n'
            'with open(os.environ["TRACE"],"a") as out: out.write("python " + " ".join(args) + "\\n")\n'
            'if args[0].endswith("native_protocol_versions.py"): print(7)\n'
            'elif "source-info" in args: print("{}")\n'
            'elif "engine-profile-refresh" in args:\n'
            '    original=Path(args[args.index("--legacy-profile")+1])\n'
            '    result=json.loads(original.read_text())\n'
            '    result["startup_snapshot_path"]=args[args.index("--startup-snapshot")+1]\n'
            '    Path(args[args.index("--output")+1]).write_text(json.dumps(result))\n')
        python.chmod(0o755)
        jq = self.commands / 'jq'
        jq.write_text('#!/usr/bin/python3\nimport json,sys\n'
                      'print(json.load(open(sys.argv[-1]))[sys.argv[-2].lstrip(".")])\n')
        jq.chmod(0o755)
        self.executable(self.commands / 'cargo',
            'printf "%s\\n" "cargo $* snapshot=${ORYN_STARTUP_SNAPSHOT-unset} profile=$ORYN_V8_PROFILE_MANIFEST" >> "$TRACE"\n'
            'if [ "$1" = run ]; then\n'
            '  if [ "${SNAPSHOT_FAIL:-0}" = 1 ]; then exit 91; fi\n'
            '  for argument do output="$argument"; done\n'
            '  printf "new snapshot" > "$output"\n'
            '  exit 0\n'
            'fi\nexit 73\n')
        return env, profile

    def test_typed_worker_generates_snapshot_before_native_build_and_keeps_original_input(self):
        env, original = self.typed_worker_environment()
        result = subprocess.run(['/bin/bash', str(self.scripts / 'build-sandboxed-worker.sh')],
                                cwd=self.root, env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 73, result.stderr)
        trace = self.trace.read_text().splitlines()
        snapshot = next(line for line in trace if line.startswith('cargo run '))
        parent = next(line for line in trace if line.startswith('cargo build '))
        self.assertIn('snapshot=unset', snapshot)
        self.assertIn('snapshot=' + str(self.artifacts / 'oryn-startup-core-full.blob'), parent)
        self.assertIn('profile=' + str(original), parent)
        refresh = next(line for line in trace if 'engine-profile-refresh' in line)
        self.assertLess(trace.index(snapshot), trace.index(refresh))
        self.assertLess(trace.index(refresh), trace.index(parent))
        self.assertNotEqual(json.loads(original.read_text())['startup_snapshot_path'],
                            str(self.artifacts / 'oryn-startup-core-full.blob'))

    def test_typed_snapshot_failure_prevents_native_compilation(self):
        env, _ = self.typed_worker_environment()
        env['SNAPSHOT_FAIL'] = '1'
        result = subprocess.run(['/bin/bash', str(self.scripts / 'build-sandboxed-worker.sh')],
                                cwd=self.root, env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 91, result.stderr)
        self.assertNotIn('cargo build ', self.trace.read_text())
        self.assertFalse((self.artifacts / 'snapshot-core-full-manifest.json').exists())


if __name__ == "__main__":
    unittest.main()
