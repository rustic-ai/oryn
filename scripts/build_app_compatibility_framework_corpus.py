#!/usr/bin/env python3
"""Build locked framework fixtures and preserve content-addressed served output."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
from generation_gate import check_execution_generation
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "benchmarks/app-compatibility/frameworks"
OUTPUTS = {
    "angular": "dist/browser",
    "next": "out",
    "nuxt": ".output/public",
    "preact": "dist",
    "react": "dist",
    "svelte": "dist",
    "sveltekit": "build",
    "vue": "dist",
}


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inventory(directory):
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"symlink in framework output: {path}")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = digest(path)
    if "index.html" not in files:
        raise ValueError(f"framework output has no index.html: {directory}")
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return {"sha256": hashlib.sha256(encoded).hexdigest(), "files": files}


def command(args, directory, name, env, timeout=240):
    check_execution_generation()
    started = time.monotonic()
    with (
        (directory / f"{name}.stdout").open("wb") as stdout,
        (directory / f"{name}.stderr").open("wb") as stderr,
    ):
        check_execution_generation()
        process = subprocess.Popen(
            args, cwd=CORPUS, env=env, stdout=stdout, stderr=stderr, start_new_session=True
        )
        timed_out = False
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)
    return dict(
        command=args,
        returncode=process.returncode,
        timed_out=timed_out,
        elapsed_seconds=round(time.monotonic() - started, 3),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--pnpm", default="pnpm")
    args = parser.parse_args()
    check_execution_generation()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = dict(schema_version=1, planned=8, builds={}, status="failed")
    env = dict(os.environ, PATH=str(args.node.resolve().parent) + os.pathsep + os.environ["PATH"])
    try:
        report["node_version"] = subprocess.check_output(
            [str(args.node), "--version"], text=True
        ).strip()
        report["pnpm_version"] = subprocess.check_output(
            [args.pnpm, "--version"], cwd=CORPUS, env=env, text=True
        ).strip()
        required = json.loads((CORPUS / "package.json").read_text())["packageManager"]
        if required != "pnpm@" + report["pnpm_version"]:
            raise ValueError("package manager version does not match locked corpus")
        report["fixture_lock_sha256"] = digest(
            ROOT / "benchmarks/app-compatibility/fixtures.lock.json"
        )
        report["lock_validation"] = command(
            [str(ROOT / "scripts/validate-app-fixtures.sh")], output, "lock-validation", env
        )
        if report["lock_validation"]["returncode"] != 0:
            raise ValueError("immutable fixture lock validation failed")
        report["install"] = command(
            [args.pnpm, "install", "--frozen-lockfile"], output, "install", env
        )
        if report["install"]["returncode"] != 0:
            raise ValueError("frozen dependency installation failed")
        for framework, relative in OUTPUTS.items():
            record = command(
                [args.pnpm, "--filter", f"app-compatibility-{framework}", "run", "build"],
                output,
                framework,
                env,
            )
            record["status"] = "failed"
            if record["returncode"] == 0 and not record["timed_out"]:
                try:
                    directory = CORPUS / "apps" / framework / relative
                    snapshot = inventory(directory)
                    destination = output / "served" / snapshot["sha256"]
                    shutil.copytree(directory, destination)
                    if inventory(destination) != snapshot:
                        raise ValueError("framework output changed while copying")
                    record.update(
                        snapshot,
                        artifact=destination.relative_to(output).as_posix(),
                        status="passed",
                    )
                except (OSError, ValueError) as error:
                    record["error"] = str(error)
            report["builds"][framework] = record
            print(json.dumps(dict(framework=framework, status=record["status"])), flush=True)
        report["post_build_lock_validation"] = command(
            [str(ROOT / "scripts/validate-app-fixtures.sh")], output, "post-build-lock", env
        )
        report["passed"] = sum(r["status"] == "passed" for r in report["builds"].values())
        if report["passed"] == 8 and report["post_build_lock_validation"]["returncode"] == 0:
            report["status"] = "passed"
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report["infrastructure_error"] = str(error)
    report["scope"] = (
        "Exact locked inputs and content-addressed output; not a claim of byte-identical builds across runs. Nuxt/Next may embed build identifiers."
    )
    (output / "build.json").write_text(json.dumps(report, indent=2) + "\n")
    return int(report["status"] != "passed")


if __name__ == "__main__":
    sys.exit(main())
