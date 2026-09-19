#!/usr/bin/env python3
"""Run the immutable workflow regressions together on one signed candidate.

Expanded-contract qualification remains separate: this controller cannot promote
controlled fixtures or framework probes into missing full-matrix outcomes.
"""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import signal
from generation_gate import check_execution_generation
import subprocess
import sys
import time

from workflow_contract import ROOT, load_contract, planned_attempts
from native_protocol_versions import versions as native_protocol_versions

FAMILIES = ("research", "forms", "shopping", "saas", "booking")
FRAMEWORKS = ("react", "preact", "vue", "angular", "svelte", "next", "nuxt", "sveltekit")
AUTH = ("popup-pkce", "redirect", "mfa-handoff", "popup-close-recovery", "existing-session-sso")


def validate_candidate_manifest(manifest, source, protocols):
    if (not isinstance(manifest, dict)
            or manifest.get("source_tree_sha256") != source
            or type(manifest.get("protocol_version")) is not int
            or manifest["protocol_version"] != protocols["worker"]
            or any(type(manifest.get(field)) is not int or manifest[field] != protocols[name]
                   for name, field in [("context", "context_service_protocol_version"), ("agent", "agent_ipc_protocol_version")])):
        raise ValueError("candidate manifest does not match current source/protocol")


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def candidate_identity(binary, worker, framework_build, manifest, source):
    return dict(parent_sha256=digest(binary), worker_sha256=digest(worker),
                source_tree_sha256=source, framework_build_sha256=digest(framework_build),
                worker_manifest_sha256=digest(manifest))


def validate_summary(summary, candidate, cases, samples, family):
    framework = family == "framework"
    errors = []
    for field in ("parent_sha256", "worker_sha256", "source_tree_sha256"):
        if summary.get(field) != candidate[field]:
            errors.append("candidate mismatch: " + field)
    if summary.get("source_matches_signed_build") is not True:
        errors.append("source does not match signed build")
    if framework and summary.get("build_manifest_sha256") != candidate["framework_build_sha256"]:
        errors.append("candidate mismatch: framework_build_sha256")
    if summary.get("artifact_identities_unchanged") is not True:
        errors.append("runner artifacts changed")
    cleanup = summary.get("fixture_cleanup")
    if cleanup is not True and (not isinstance(cleanup, dict)
            or set(cleanup) != {"returncode", "forced"}
            or type(cleanup.get("returncode")) is not int or cleanup["returncode"] != 0
            or cleanup.get("forced") is not False):
        errors.append("fixture cleanup was not successful")
    expected = Counter((case, sample) for case in cases for sample in range(1, samples + 1))
    records = summary.get("records", [])
    if not isinstance(records, list) or any(not isinstance(record, dict) for record in records):
        return errors + ["malformed records"]
    key = "framework" if framework else "case"
    if any(not isinstance(record.get(key), str) or type(record.get("sample")) is not int for record in records):
        return errors + ["malformed attempt identity"]
    actual = Counter((record[key], record["sample"]) for record in records)
    if actual != expected:
        errors.append("missing, duplicate or unexpected planned attempts")
    if summary.get("planned") != sum(expected.values()):
        errors.append("planned denominator mismatch")
    for record in records:
        if type(record.get("returncode")) is not int or record["returncode"] != 0:
            errors.append("attempt process did not exit successfully")
        if framework:
            cleanup = record.get('process_cleanup', {})
            if (type(record.get('returncode')) is not int or record.get('returncode') != 0
                    or not isinstance(cleanup, dict)
                    or cleanup.get('parent_reaped') is not True
                    or cleanup.get('descendants_clean') is not True
                    or cleanup.get('forced') is not False or cleanup.get('errors') != []):
                errors.append('framework process exit/descendant cleanup incomplete')
        if (record.get("passed") is not True or record.get("errors") != []
                or record.get("timed_out") is not False):
            errors.append("failed attempt retained")
        oracle_field = "final_oracle_verified" if family == "forms" else "final_database_verified"
        if not framework and record.get(oracle_field) is not True:
            errors.append("missing independent final oracle verification")
    if summary.get("passed") != sum(expected.values()) or summary.get("status") != "passed":
        errors.append("runner did not pass every planned attempt")
    if summary.get("infrastructure_error"):
        errors.append("runner infrastructure failed")
    return errors


def execute(command, directory, environment, timeout):
    directory.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    with (directory / "stdout.log").open("wb") as stdout, (directory / "stderr.log").open("wb") as stderr:
        check_execution_generation()
        process = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=stdout,
                                   stderr=stderr, start_new_session=True)
        timed_out = False
        try:
            returncode = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                # The group can exit between the deadline and the kill. Still
                # reap the child and retain the original timeout as a failure.
                pass
            returncode = process.wait()
        finally:
            # A runner must clean up its own fixture/worker descendants. Reap
            # escaped work in this launch's process group and retain the failure.
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                descendants_clean = True
            else:
                descendants_clean = False
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    # Preserve the observed leak even if it exits before kill.
                    pass
            process.wait()
    return dict(returncode=returncode, timed_out=timed_out,
                descendants_clean=descendants_clean, duration_seconds=time.monotonic() - started)


def run_family(name, mode, args, candidate, frontend_v2=False, frontend_version=2):
    directory = args.output / mode / name
    command = [sys.executable, str(ROOT / f"scripts/run-oil-{name}-outcomes.py"),
               "--binary", str(args.binary), "--worker", str(args.worker),
               "--output", str(directory / "raw"), "--samples", "3", "--jobs", "1"]
    if name == "framework":
        command.extend(["--build", str(args.framework_build)])
        cases = FRAMEWORKS
    else:
        suffix = f"-v{frontend_version}" if frontend_v2 else ""
        if frontend_v2:
            command.append(f"--frontend-v{frontend_version}")
        cases = [case["id"] for case in json.loads(
            (ROOT / f"test-harness/workflows/{name}-cases{suffix}.json").read_text())["cases"]]
    result = dict(family=name, mode=mode, errors=[])
    try:
        result.update(execute(command, directory, dict(os.environ), args.timeout))
        summary_path = directory / "raw/summary.json"
        result["raw_summary"] = str(summary_path.relative_to(args.output))
        result["raw_summary_sha256"] = digest(summary_path)
        summary = json.loads(summary_path.read_text())
        result["errors"] = validate_summary(summary, candidate, cases, 3, name)
    except Exception as error:
        result["errors"].append(str(error))
    result["passed"] = (not result["errors"] and result.get("returncode") == 0
                        and not result.get("timed_out") and result.get("descendants_clean") is True)
    return result


def run_auth(sample, args):
    directory = args.output / "auth" / str(sample)
    environment = dict(os.environ, ORYN_BIN=str(args.binary), ORYN_PAGE_WORKER=str(args.worker),
                       ORYN_APP_COMPATIBILITY_DOMAIN="native", ORYN_APP_COMPATIBILITY_EXTERNAL_SERVER="0",
                       ORYN_APP_COMPATIBILITY_PORT=str(args.auth_port),
                       ORYN_APP_COMPATIBILITY_ARTIFACT_DIR=str(directory / "raw"))
    result = dict(family="auth_mechanics", sample=sample, errors=[])
    try:
        result.update(execute(["bash", str(ROOT / "scripts/validate-auth-flows.sh")],
                              directory, environment, args.timeout))
        result["artifacts"] = {}
        for case in AUTH:
            for suffix in (".jsonl", ".oil"):
                path = directory / "raw" / (case + "-native" + suffix)
                result["artifacts"][str(path.relative_to(args.output))] = digest(path)
    except Exception as error:
        result["errors"].append(str(error))
    result["passed"] = (not result["errors"] and result.get("returncode") == 0
                        and not result.get("timed_out") and result.get("descendants_clean") is True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--framework-build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=7200)
    parser.add_argument("--auth-port", type=int, default=3104)
    args = parser.parse_args()
    from generation_gate import inherited_generation_lease
    admission = inherited_generation_lease()
    if args.timeout <= 0 or not 1024 <= args.auth_port <= 65535:
        parser.error("invalid timeout or fixture port")
    for name in ("binary", "worker", "framework_build", "output"):
        setattr(args, name, getattr(args, name).resolve())
    contract = load_contract()
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(schema_version=1, scope="integrated_existing_regressions", family_qualified=False,
                  goal_complete=False, results=[], status="failed", errors=[],
                  expanded_contract=contract["contract_id"],
                  expanded_unexecuted_attempts=planned_attempts(contract))
    try:
        source = subprocess.check_output([str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True).strip()
        manifest_path = args.worker.parent.parent / "Resources/worker-manifest.json"
        candidate = candidate_identity(args.binary, args.worker, args.framework_build, manifest_path, source)
        report["candidate"] = candidate
        manifest = json.loads(manifest_path.read_text())
        protocols = native_protocol_versions(ROOT)
        validate_candidate_manifest(manifest, source, protocols)
        report["native_protocol_versions"] = protocols
        for family in FAMILIES:
            report["results"].append(run_family(family, "serial", args, candidate))
        with ThreadPoolExecutor(max_workers=5) as executor:
            report["results"].extend(executor.map(lambda family: run_family(family, "mixed_family", args, candidate), FAMILIES))
        report["results"].append(run_family("framework", "serial", args, candidate))
        report["results"].append(run_family("forms", "expanded_frontend", args, candidate, frontend_v2=True))
        report["results"].append(run_family("shopping", "expanded_frontend", args, candidate, frontend_v2=True))
        report["results"].append(run_family("research", "expanded_frontend", args, candidate, frontend_v2=True))
        report["results"].append(run_family("research", "expanded_frames", args, candidate, frontend_v2=True,frontend_version=3))
        report["results"].append(run_family("booking", "expanded_frontend", args, candidate, frontend_v2=True))
        report["results"].append(run_family("saas", "expanded_frontend", args, candidate, frontend_v2=True))
        report["results"].append(run_family("saas", "expanded_expiry", args, candidate, frontend_v2=True, frontend_version=3))
        report["results"].extend(run_auth(sample, args) for sample in range(1, 4))
        after_source = subprocess.check_output([str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True).strip()
        after = candidate_identity(args.binary, args.worker, args.framework_build, manifest_path, after_source)
        if after != candidate:
            raise ValueError("candidate changed during integrated run")
        if len(report["results"]) == 21 and all(result["passed"] for result in report["results"]):
            report["status"] = "passed"
    except Exception as error:
        report["errors"].append(str(error))
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("status", "scope", "family_qualified")}))
    return int(report["status"] != "passed")


if __name__ == "__main__":
    sys.exit(main())
