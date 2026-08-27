#!/usr/bin/env python3
"""Run the fingerprinted, cumulative-budget G2R MiniWoB model panel."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import statistics
import subprocess
import urllib.request
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from intentgym.core.config import AgentConfig, BenchmarkConfig, LLMConfig, RunConfig
from intentgym.core.runner import BenchmarkRunner

TASKS = (
    "click-button",
    "click-link",
    "click-option",
    "enter-text",
    "focus-text",
    "choose-date",
    "login-user",
    "search-engine",
)
SEEDS = (17, 42, 73)
MODEL_STAGES = (
    ("native", "hosted"),
    ("native", "local"),
    ("chromium", "hosted"),
    ("chromium", "local"),
)
HOSTED_MODEL = "gpt-5.6-terra"
LOCAL_MODEL = "qwen3:4b"
LOCAL_MAX_OUTPUT_TOKENS = 256
HOSTED_CAP_USD = 100.0
# Backwards-compatible symbol used by tests and downstream imports. For schema
# v4 it means all hosted spend accumulated before this G5A-bound rerun.
PRIOR_INVALID_HOSTED_SPEND_USD = 5.209632
HOSTED_INPUT_COST_PER_MILLION = 2.0
HOSTED_OUTPUT_COST_PER_MILLION = 12.0
EXPECTED_QWEN_DIGEST = (
    "359d7dd4bcdab3d86b87d73ac27966f4dbb9f5efdfcc75d34a8764a09474fae7"
)
EXPECTED_MINIWOB_COMMIT = "33c3b4ddef8c6eb67c57a29663d844b1eda7e614"
EXPECTED_MINIWOB_CONTENT_SHA256 = (
    "7bac5e8f28c40b8a44b3c915f98b26a8d3c494ecc1634e3d43839ed13598e386"
)


def _json(url: str) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=10) as response:
        return json.load(response)


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def _write_turn_jsonl(path: Path, turns: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as handle:
        for turn in turns:
            handle.write(json.dumps(turn, separators=(",", ":"), sort_keys=True) + "\n")
    temporary.replace(path)


def _percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))]


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _canonical_hash(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _files_hash(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def _deterministic_causal(
    evidence_path: Path, aggregate: dict[str, Any]
) -> dict[str, Any]:
    paths = sorted(evidence_path.parent.glob("*-native-oracle.json"))
    cells = [json.loads(path.read_text(encoding="utf-8")) for path in paths]
    aggregate_fingerprint = (aggregate.get("fingerprint") or {}).get("sha256")
    expected_ids = {f"{task}-s{seed}-native-oracle" for task in TASKS for seed in SEEDS}
    if len(cells) != 24 or {cell.get("cell_id") for cell in cells} != expected_ids:
        raise RuntimeError("deterministic native causal suite is not exactly 24 cells")
    if any(
        cell.get("fingerprint_sha256") != aggregate_fingerprint
        or cell.get("success") is not True
        for cell in cells
    ):
        raise RuntimeError(
            "deterministic native causal cells do not match the passing aggregate"
        )
    expected = sum(
        int((cell.get("causal") or {}).get("expected_effects") or 0) for cell in cells
    )
    observed_expected = sum(
        int((cell.get("causal") or {}).get("observed_expected_effects") or 0)
        for cell in cells
    )
    observed = sum(
        int((cell.get("causal") or {}).get("observed_effects") or 0) for cell in cells
    )
    if not expected or not observed or observed_expected > expected:
        raise RuntimeError("deterministic causal suite contains invalid effect counts")
    return {
        "cells_sha256": _files_hash(paths),
        "recall": _ratio(observed_expected, expected),
        "precision": _ratio(observed_expected, observed),
        "ledger": [
            {
                "cell_id": cell["cell_id"],
                "expected_effects": int(
                    (cell.get("causal") or {}).get("expected_effects") or 0
                ),
                "observed_expected_effects": int(
                    (cell.get("causal") or {}).get("observed_expected_effects") or 0
                ),
                "observed_effects": int(
                    (cell.get("causal") or {}).get("observed_effects") or 0
                ),
            }
            for cell in cells
        ],
    }


def _harness_hash(repo_root: Path) -> str:
    """Fingerprint the Python execution path that can change a panel cell."""
    candidates = [
        *sorted((repo_root / "intentgym/src").rglob("*.py")),
        *sorted((repo_root / "oryn-python/src").rglob("*.py")),
        repo_root / "intentgym/scripts/run_g2_model_panel.py",
    ]
    digest = hashlib.sha256()
    for path in candidates:
        digest.update(path.relative_to(repo_root).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def _source_tree_hash(repo_root: Path) -> str:
    return subprocess.check_output(
        [str(repo_root / "scripts/source-tree-hash.sh")], text=True
    ).strip()


def _codesign_value(details: str, name: str) -> str:
    prefix = f"{name}="
    return next(
        (
            line.removeprefix(prefix)
            for line in details.splitlines()
            if line.startswith(prefix)
        ),
        "",
    )


def _worker_bundle_fingerprint(
    args: argparse.Namespace, runtime: dict[str, Any], source_tree_sha256: str
) -> dict[str, Any]:
    configured = os.environ.get("ORYN_PAGE_WORKER")
    worker = (
        Path(configured)
        if configured
        else args.oryn_binary.parent
        / "OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
    )
    if not worker.is_file():
        raise RuntimeError(f"signed page worker is missing: {worker}")
    subprocess.run(
        ["/usr/bin/codesign", "--verify", "--strict", "--verbose=2", str(worker)],
        check=True,
        capture_output=True,
        text=True,
    )
    details_process = subprocess.run(
        ["/usr/bin/codesign", "-dvvv", str(worker)],
        check=True,
        capture_output=True,
        text=True,
    )
    details = details_process.stderr
    entitlements_process = subprocess.run(
        ["/usr/bin/codesign", "-d", "--entitlements", ":-", str(worker)],
        check=True,
        capture_output=True,
    )
    entitlements = plistlib.loads(entitlements_process.stdout)
    expected_entitlements = {
        "com.apple.security.app-sandbox": True,
        "com.apple.security.cs.allow-jit": True,
    }
    if entitlements != expected_entitlements:
        raise RuntimeError(
            f"worker entitlements are not the exact G5A set: {sorted(entitlements)}"
        )
    app = worker.parents[2]
    manifest_path = app / "Contents/Resources/worker-manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("source_tree_sha256") != source_tree_sha256:
        raise RuntimeError("signed worker manifest does not match the source tree")
    if manifest.get("protocol_version") != 2:
        raise RuntimeError("signed worker manifest does not require protocol v2")
    worker_sha256 = _sha256(worker)
    if runtime.get("worker_binary_sha256") != worker_sha256:
        raise RuntimeError("worker handshake hash differs from the signed artifact")
    if runtime.get("worker_bundle_id") != "ai.rustic.oryn.page-worker":
        raise RuntimeError("worker bundle identifier changed")
    if runtime.get("worker_team_id") != "5HVA9VFF8K":
        raise RuntimeError("worker Team ID changed")
    if (
        runtime.get("worker_signing_cert_sha1")
        != "9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90"
    ):
        raise RuntimeError("worker signing certificate changed")
    return {
        "path": str(worker),
        "sha256": worker_sha256,
        "bytes": worker.stat().st_size,
        "bundle_id": _codesign_value(details, "Identifier"),
        "team_id": _codesign_value(details, "TeamIdentifier"),
        "signing_cert_sha1": runtime.get("worker_signing_cert_sha1"),
        "cdhash": _codesign_value(details, "CDHash"),
        "hardened_runtime": "flags=0x10000(runtime)" in details,
        "signature_details_sha256": hashlib.sha256(details.encode()).hexdigest(),
        "entitlements": expected_entitlements,
        "entitlements_sha256": hashlib.sha256(
            plistlib.dumps(expected_entitlements, sort_keys=True)
        ).hexdigest(),
        "manifest_sha256": _sha256(manifest_path),
        "manifest": manifest,
    }


def _parse_json_lines(output: str) -> list[dict[str, Any]]:
    values = []
    for line in output.splitlines():
        start = line.find("{")
        if start < 0:
            continue
        try:
            value = json.loads(line[start:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            values.append(value)
    return values


def _runtime_smoke(binary: Path, fixture: Path) -> dict[str, Any]:
    completed = subprocess.run(
        [str(binary), "native", "--html", str(fixture)],
        input='observe\nclick "Mutate"\nobserve\nexit\n',
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "native V8 mutation smoke failed: "
            + (completed.stderr.strip() or f"exit {completed.returncode}")
        )
    values = _parse_json_lines(completed.stdout)
    actions = [
        value.get("result") or {} for value in values if value.get("kind") == "action"
    ]
    observations = [
        value.get("observation") or {}
        for value in values
        if value.get("kind") == "observation"
    ]
    if len(actions) != 1 or len(observations) != 2:
        raise RuntimeError(
            "native V8 mutation smoke returned an incomplete JSONL exchange: "
            f"actions={len(actions)}, observations={len(observations)}, "
            f"stderr={completed.stderr.strip()!r}"
        )
    action = actions[0]
    effect_kinds = {item.get("kind") for item in action.get("effects") or []}
    changed = any(
        node.get("name") == "changed" for node in observations[-1].get("nodes") or []
    )
    if "dom_mutation" not in effect_kinds or not changed:
        raise RuntimeError(
            "native V8 mutation smoke did not produce a measured DOM change"
        )
    return {
        "status": "passed",
        "revision_before": action.get("revision_before"),
        "revision_after": action.get("revision_after"),
        "effect_kinds": sorted(effect_kinds),
    }


def _preflight(args: argparse.Namespace) -> dict[str, Any]:
    if (
        float(getattr(args, "prior_hosted_spend", PRIOR_INVALID_HOSTED_SPEND_USD))
        != PRIOR_INVALID_HOSTED_SPEND_USD
    ):
        raise RuntimeError(
            f"schema-v4 budget accounting must start at USD {PRIOR_INVALID_HOSTED_SPEND_USD:.6f}"
        )
    if not os.environ.get("AZURE_OPENAI_API_KEY"):
        raise RuntimeError(
            "AZURE_OPENAI_API_KEY is required and must remain environment-only"
        )
    if not os.environ.get("AZURE_OPENAI_ENDPOINT"):
        raise RuntimeError("AZURE_OPENAI_ENDPOINT is required for the Azure G2R panel")
    if args.hosted_model != HOSTED_MODEL:
        raise RuntimeError(
            f"Azure deployment must be exactly {HOSTED_MODEL}, got {args.hosted_model}"
        )
    if args.local_model_digest != EXPECTED_QWEN_DIGEST:
        raise RuntimeError(
            "--local-model-digest must equal the pinned qwen3:4b immutable digest"
        )
    if not args.oryn_binary.is_file():
        raise RuntimeError(f"Oryn binary does not exist: {args.oryn_binary}")
    if not args.chrome_binary.is_file():
        raise RuntimeError(f"Chromium oracle does not exist: {args.chrome_binary}")

    runtime = json.loads(
        subprocess.check_output(
            [
                str(args.oryn_binary),
                "native",
                "--runtime-info",
                "--allow-loopback",
            ],
            text=True,
        )
    )
    if runtime.get("native_v8") is not True or not runtime.get("v8_version"):
        raise RuntimeError("production Oryn binary lacks functional native V8 metadata")
    if runtime.get("build_profile") != "release":
        raise RuntimeError(
            "G2R model evidence requires the release-profile Oryn binary"
        )
    if (
        runtime.get("execution_mode") != "sandboxed_worker"
        or runtime.get("sandboxed") is not True
        or runtime.get("sandbox_state") != "app_sandbox"
        or runtime.get("worker_protocol_version") != 2
    ):
        raise RuntimeError("G5A evidence requires the verified macOS sandbox worker")
    smoke = _runtime_smoke(
        args.oryn_binary,
        args.repo_root / "benchmarks/conformance/g2r-v8-mutation.html",
    )

    tags = _json(args.ollama_url.rstrip("/") + "/api/tags")
    model = next(
        (item for item in tags.get("models", []) if item.get("name") == LOCAL_MODEL),
        None,
    )
    if not model:
        raise RuntimeError(f"{LOCAL_MODEL} is unavailable from {args.ollama_url}")
    actual_digest = model.get("digest")
    if actual_digest != args.local_model_digest:
        raise RuntimeError(
            f"local model digest mismatch: expected {args.local_model_digest}, got {actual_digest}"
        )

    fixture = _json(args.miniwob_url.rstrip("/") + "/.well-known/oryn-g2-fixture.json")
    if fixture.get("kind") != "oryn_g2_seeded_miniwob":
        raise RuntimeError(
            "MiniWoB endpoint is not the benchmark-only seeded G2R server"
        )
    if fixture.get("allowed_seeds") != list(SEEDS):
        raise RuntimeError("MiniWoB server does not enforce the required seed set")
    if fixture.get("source_commit") != EXPECTED_MINIWOB_COMMIT:
        raise RuntimeError(
            "MiniWoB source commit does not match the pinned G2R fixture"
        )
    if fixture.get("content_sha256") != EXPECTED_MINIWOB_CONTENT_SHA256:
        raise RuntimeError(
            "MiniWoB served-content hash does not match the pinned fixture"
        )

    deterministic = json.loads(args.deterministic_evidence.read_text(encoding="utf-8"))
    schema_version = int(getattr(args, "schema_version", 4))
    if deterministic.get("schema_version") != schema_version:
        raise RuntimeError(f"deterministic evidence is not schema v{schema_version}")
    if deterministic.get("native") != {"completed": 24, "passed": 24}:
        raise RuntimeError(
            "deterministic native 24/24 must pass before any model request"
        )
    if deterministic.get("chromium") != {"completed": 24, "passed": 24}:
        raise RuntimeError(
            "deterministic Chromium comparison must pass before the model panel"
        )
    differential = deterministic.get("differential") or []
    if len(differential) != 24 or any(
        item.get("first_divergence") is not None for item in differential
    ):
        raise RuntimeError(
            "deterministic native/Chromium comparison contains a divergence"
        )
    if deterministic.get("outcome", {}).get("status") != "passed":
        raise RuntimeError("deterministic evidence outcome is not passed")
    deterministic_causal = _deterministic_causal(
        args.deterministic_evidence, deterministic
    )
    deterministic_fingerprint = deterministic.get("fingerprint") or {}
    deterministic_binary = deterministic_fingerprint.get("binary") or {}
    if deterministic_binary.get("sha256") != _sha256(args.oryn_binary):
        raise RuntimeError(
            "deterministic evidence was produced by a different Oryn binary"
        )
    if deterministic_binary.get("runtime") != runtime:
        raise RuntimeError("deterministic runtime metadata does not match preflight")
    if schema_version >= 4 and deterministic_fingerprint.get(
        "source_tree_sha256"
    ) != _source_tree_hash(args.repo_root):
        raise RuntimeError("deterministic evidence used a different source tree")
    if deterministic_fingerprint.get("miniwob") != fixture:
        raise RuntimeError("deterministic evidence used a different MiniWoB fixture")

    churn = json.loads(args.churn_evidence.read_text(encoding="utf-8"))
    if churn.get("schema_version") != schema_version or churn.get("status") != "passed":
        raise RuntimeError(
            f"controlled churn/recovery evidence is not passing schema v{schema_version}"
        )
    if churn.get("false_preservation", {}).get("numerator") != 0:
        raise RuntimeError(
            "churn evidence reports false semantic-reference preservation"
        )
    recovery = churn.get("recovery") or {}
    if recovery.get("denominator", 0) < 2 or recovery.get("value") != 1.0:
        raise RuntimeError("controlled stale/no-progress recovery suite did not pass")

    commit = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=args.repo_root, text=True
    ).strip()
    status = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=args.repo_root, text=True
    )
    if schema_version >= 4 and status.strip():
        raise RuntimeError("schema-v4 release evidence requires a clean source tree")
    source_tree_sha256 = _source_tree_hash(args.repo_root)
    worker_bundle = _worker_bundle_fingerprint(args, runtime, source_tree_sha256)
    chrome_version = subprocess.check_output(
        [str(args.chrome_binary), "--version"], text=True
    ).strip()
    if deterministic_fingerprint.get("chromium_version") != chrome_version:
        raise RuntimeError("deterministic evidence used a different Chromium version")
    fingerprint = {
        "source_commit": commit,
        "source_dirty": bool(status.strip()),
        "source_tree_sha256": source_tree_sha256,
        "harness_sha256": _harness_hash(args.repo_root),
        "deterministic_evidence_sha256": _sha256(args.deterministic_evidence),
        "deterministic_cells_sha256": deterministic_causal["cells_sha256"],
        "churn_evidence_sha256": _sha256(args.churn_evidence),
        "binary": {
            "sha256": _sha256(args.oryn_binary),
            "bytes": args.oryn_binary.stat().st_size,
            "runtime": runtime,
            "mutation_smoke": smoke,
        },
        "worker_bundle": worker_bundle,
        "policy": {
            "digest": runtime.get("policy_digest"),
            "network_mode": "benchmark_loopback",
            "parent_owned_cookies": True,
            "decoded_response_limit_bytes": 16 * 1024 * 1024,
        },
        "limits": runtime.get("limits"),
        "miniwob": fixture,
        "chromium_version": chrome_version,
        "models": {
            "hosted_provider": "azure_openai",
            "hosted_deployment": args.hosted_model,
            "local_model": LOCAL_MODEL,
            "local_model_digest": actual_digest,
        },
    }
    fingerprint["sha256"] = _canonical_hash(fingerprint)
    return {
        "fingerprint": fingerprint,
        "deterministic": deterministic,
        "deterministic_causal": deterministic_causal,
        "churn": churn,
    }


def _config(
    args: argparse.Namespace,
    task: str,
    seed: int,
    domain: str,
    model_kind: str,
    remaining_hosted_budget: float,
) -> RunConfig:
    run_id = f"g2r-{task}-s{seed}-{domain}-{model_kind}"
    if model_kind == "hosted":
        llm = LLMConfig(
            provider="openai",
            model=args.hosted_model,
            options={
                "budget_usd": remaining_hosted_budget,
                "max_output_tokens": 4096,
                "input_cost_per_million": args.hosted_input_cost_per_million,
                "output_cost_per_million": args.hosted_output_cost_per_million,
            },
        )
    else:
        llm = LLMConfig(
            provider="litellm",
            model=f"ollama/{LOCAL_MODEL}",
            options={
                "api_base": args.ollama_url.rstrip("/"),
                "temperature": 1.0,
                "max_tokens": LOCAL_MAX_OUTPUT_TOKENS,
                "think": False,
            },
        )
    if domain == "native":
        oryn_mode = "native"
        oryn_options = {
            "binary_path": str(args.oryn_binary),
            "cli_args": ["--allow-loopback"],
            "timeout": args.command_timeout,
        }
    else:
        oryn_mode = "headless"
        oryn_options = {
            "binary_path": str(args.oryn_binary),
            "env": {"CHROME_BIN": str(args.chrome_binary)},
            "timeout": args.command_timeout,
        }
    return RunConfig(
        run_id=run_id,
        seed=seed,
        benchmark=BenchmarkConfig(
            name="miniwob",
            options={"server_url": args.miniwob_url.rstrip("/"), "seed": seed},
        ),
        llm=llm,
        agent=AgentConfig(type="react", options={}),
        prompt_template="oil_standard",
        oryn_mode=oryn_mode,
        oryn_options=oryn_options,
        max_steps=12,
        timeout_seconds=args.task_timeout,
        save_transcript=True,
    )


def _ratio(numerator: int, denominator: int) -> dict[str, Any]:
    return {
        "numerator": numerator,
        "denominator": denominator,
        "value": numerator / denominator if denominator else None,
    }


def _role_is(element: dict[str, Any], expected: str) -> bool:
    element_type = str(element.get("type") or "").lower()
    role = str(element.get("role") or "").lower()
    if expected == "textbox":
        return element_type in {"input", "textarea", "textbox", "password"} or role in {
            "input",
            "textbox",
            "email",
            "password",
        }
    if expected == "button":
        return element_type == "button" or role in {"button", "primary"}
    if expected == "link":
        return element_type in {"a", "link"} or role == "link"
    if expected == "radio":
        return element_type == "radio" or role == "radio"
    return element_type == expected or role == expected


def _checkpoint_sufficient(task: str, turn: dict[str, Any]) -> bool:
    raw = str(turn.get("observation_raw") or "")
    elements = turn.get("observation_elements") or []

    def has_role(role: str, *, name: str | None = None, minimum: int = 1) -> bool:
        matches = [item for item in elements if _role_is(item, role)]
        if name is not None:
            matches = [
                item
                for item in matches
                if str(item.get("text") or item.get("name") or "") == name
            ]
        return len(matches) >= minimum

    if task == "click-button":
        match = re.search(r'Click on the "([^"]+)" button\.', raw)
        return bool(match and has_role("button", name=match.group(1)))
    if task == "click-link":
        match = re.search(r'Click on the link "([^"]+)"\.', raw)
        return bool(match and has_role("link", name=match.group(1)))
    if task == "click-option":
        match = re.search(r"Select (.+?) and click Submit\.", raw)
        if not match or not has_role("button", name="Submit"):
            return False
        target = match.group(1)
        return has_role("radio", name=target) or (
            target in {str(item.get("text") or "") for item in elements}
            and has_role("radio")
        )
    if task == "enter-text":
        return bool(
            re.search(r'Enter "[^"]+" into the text field and press Submit\.', raw)
            and has_role("textbox")
            and has_role("button", name="Submit")
        )
    if task == "focus-text":
        return "Focus into the textbox." in raw and has_role("textbox")
    if task == "choose-date":
        match = re.search(
            r"Select \d{2}/(\d{2})/\d{4} as the date and hit submit\.", raw
        )
        if not match:
            return False
        day = str(int(match.group(1)))
        months = (
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        )
        return (
            has_role("textbox")
            and has_role("button", name="Submit")
            and has_role("link", name=day)
            and any(month in raw for month in months)
        )
    if task == "login-user":
        return bool(
            re.search(r'username "[^"]+" and the password "[^"]+"', raw)
            and has_role("textbox", minimum=2)
            and has_role("button", name="Login")
        )
    if task == "search-engine":
        match = re.search(
            r'enter "[^"]+".*click the (\d+)(?:st|nd|rd|th) search result',
            raw,
            re.IGNORECASE | re.DOTALL,
        )
        if not match:
            return False
        result_links = [
            item
            for item in elements
            if _role_is(item, "link")
            and str(item.get("text") or "") not in {"", "<", ">", "1", "2", "3"}
        ]
        return (
            has_role("textbox")
            and has_role("button", name="Search")
            and len(result_links) >= min(3, int(match.group(1)))
        )
    return False


def _observation_sufficient(cell: dict[str, Any]) -> bool:
    return any(
        _checkpoint_sufficient(cell["task_id"], turn) for turn in cell.get("turns", [])
    )


def _measured_metrics(
    cells: list[dict[str, Any]],
    churn: dict[str, Any],
    deterministic_causal: dict[str, Any],
) -> dict[str, Any]:
    completed = [cell for cell in cells if cell.get("completed") is True]
    observation_numerator = sum(_observation_sufficient(cell) for cell in completed)
    action_latencies = []
    observation_bytes = []
    for cell in completed:
        for turn in cell.get("turns", []):
            action_latencies.append(float(turn.get("oryn_action_latency_ms") or 0.0))
            if turn.get("observation_bytes"):
                observation_bytes.append(int(turn["observation_bytes"]))
    survival = churn.get("reference_survival") or {}
    false_preservation = churn.get("false_preservation") or {}
    recovery = churn.get("recovery") or {}
    return {
        "task_success": _ratio(
            sum(bool(cell.get("success")) for cell in completed), len(completed)
        ),
        "observation_sufficiency": _ratio(observation_numerator, len(completed)),
        "recovery": {
            "numerator": int(recovery.get("numerator", 0)),
            "denominator": int(recovery.get("denominator", 0)),
            "value": recovery.get("value"),
        },
        "reference_survival": {
            "numerator": int(survival.get("numerator", 0)),
            "denominator": int(survival.get("denominator", 0)),
            "value": survival.get("value"),
        },
        "false_reference_preservation": {
            "numerator": int(false_preservation.get("numerator", 0)),
            "denominator": int(false_preservation.get("denominator", 0)),
            "value": false_preservation.get("value"),
        },
        "causal_recall": deterministic_causal["recall"],
        "causal_precision": deterministic_causal["precision"],
        "action_latency_ms": {
            "p50": statistics.median(action_latencies) if action_latencies else None,
            "p95": _percentile(action_latencies, 0.95),
            "samples": len(action_latencies),
        },
        "observation_utf8_bytes": {
            "p50": statistics.median(observation_bytes) if observation_bytes else None,
            "p95": _percentile(observation_bytes, 0.95),
            "samples": len(observation_bytes),
        },
    }


def _classifications_explained(cells: list[dict[str, Any]]) -> bool:
    allowed = {
        "failed",
        "unsupported",
        "event_only",
        "state_changed",
        "navigation",
        "request_effect",
    }
    for cell in cells:
        for turn in cell.get("turns", []):
            classification = turn.get("action_classification")
            if classification not in allowed:
                return False
            if classification in {"failed", "unsupported"} and not (
                turn.get("action_error")
                or turn.get("action_diagnostics")
                or turn.get("failure_domain_reason_code")
            ):
                return False
    return True


def _compact_cell_ledger(cells: list[dict[str, Any]]) -> list[dict[str, Any]]:
    ledger = []
    for index, cell in enumerate(cells):
        turns = cell.get("turns") or []
        ledger.append(
            {
                "cell_id": cell.get("cell_id", f"cell-{index}"),
                "fingerprint_sha256": cell.get("fingerprint_sha256"),
                "completed": bool(cell.get("completed")),
                "task_id": cell.get("task_id"),
                "seed": cell.get("seed"),
                "domain": cell.get("domain"),
                "model": cell.get("model"),
                "success": bool(cell.get("success")),
                "total_cost_usd": float(cell.get("total_cost_usd") or 0.0),
                "observation_sufficient": _observation_sufficient(cell),
                "classifications_explained": _classifications_explained([cell]),
                "action_latency_ms": [
                    float(turn.get("oryn_action_latency_ms") or 0.0) for turn in turns
                ],
                "observation_utf8_bytes": [
                    int(turn["observation_bytes"])
                    for turn in turns
                    if turn.get("observation_bytes")
                ],
            }
        )
    return ledger


def _aggregate(
    args: argparse.Namespace,
    preflight: dict[str, Any],
    cells: list[dict[str, Any]],
    started_at: str,
) -> dict[str, Any]:
    completed = [cell for cell in cells if cell.get("completed") is True]
    new_hosted_cost = round(
        sum(
            float(cell.get("total_cost_usd") or 0.0)
            for cell in completed
            if cell.get("model") == "hosted"
        ),
        6,
    )
    prior_hosted_spend = float(
        getattr(args, "prior_hosted_spend", PRIOR_INVALID_HOSTED_SPEND_USD)
    )
    cumulative_cost = round(prior_hosted_spend + new_hosted_cost, 6)
    azure_native = [
        cell
        for cell in completed
        if cell.get("domain") == "native" and cell.get("model") == "hosted"
    ]
    metrics = _measured_metrics(
        completed, preflight["churn"], preflight["deterministic_causal"]
    )
    deterministic = preflight["deterministic"]
    panel_complete = len(completed) == 96
    criteria = {
        "panel_complete": panel_complete,
        "source_tree_clean_and_bound": preflight["fingerprint"].get("source_dirty")
        is False
        and bool(preflight["fingerprint"].get("source_tree_sha256")),
        "sandboxed_worker_verified": (
            ((preflight["fingerprint"].get("binary") or {}).get("runtime") or {}).get(
                "execution_mode"
            )
            == "sandboxed_worker"
            and (
                (preflight["fingerprint"].get("binary") or {}).get("runtime") or {}
            ).get("sandboxed")
            is True
            and (
                (preflight["fingerprint"].get("binary") or {}).get("runtime") or {}
            ).get("worker_protocol_version")
            == 2
        ),
        "worker_limits_match_manifest": preflight["fingerprint"].get("limits")
        == (preflight["fingerprint"].get("worker_bundle") or {})
        .get("manifest", {})
        .get("limits"),
        "worker_replacement_passed": preflight["churn"]
        .get("worker_replacement", {})
        .get("status")
        == "passed",
        "native_deterministic_24_of_24": deterministic.get("native")
        == {"completed": 24, "passed": 24},
        "chromium_deterministic_24_of_24": deterministic.get("chromium")
        == {"completed": 24, "passed": 24},
        "deterministic_zero_divergences": len(deterministic.get("differential") or [])
        == 24
        and all(
            item.get("first_divergence") is None
            for item in deterministic.get("differential") or []
        ),
        "azure_native_at_least_15_of_24": len(azure_native) == 24
        and sum(bool(cell.get("success")) for cell in azure_native) >= 15,
        "budget_with_prior_spend_within_cap": cumulative_cost <= HOSTED_CAP_USD,
        "zero_false_reference_preservation": metrics["false_reference_preservation"][
            "numerator"
        ]
        == 0,
        "semantic_metrics_measured": all(
            metrics[name]["denominator"] > 0
            for name in (
                "observation_sufficiency",
                "recovery",
                "reference_survival",
                "false_reference_preservation",
                "causal_recall",
                "causal_precision",
            )
        )
        and metrics["action_latency_ms"]["samples"] > 0
        and metrics["observation_utf8_bytes"]["samples"] > 0,
        "all_action_classifications_explained": _classifications_explained(completed),
    }
    passed = all(criteria.values())
    reason_codes = [name for name, value in criteria.items() if not value]
    return {
        "schema_version": int(getattr(args, "schema_version", 4)),
        "run_id": "g2r-g5a-model-panel-v4",
        "started_at": started_at,
        "updated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "fingerprint": preflight["fingerprint"],
        "corpus": {
            "name": "oryn-g2r-native-proof",
            "revision": "2026-08-26",
            "entries": 30,
            "published_entries": 30,
        },
        "deterministic": deterministic,
        "panel": {
            "expected_runs": 96,
            "completed_runs": len(completed),
            "prior_hosted_spend_usd": prior_hosted_spend,
            "new_hosted_spend_usd": new_hosted_cost,
            "cumulative_hosted_spend_usd": cumulative_cost,
            "hosted_cost_cap_usd": HOSTED_CAP_USD,
            "hosted_provider": "azure_openai",
            "hosted_model": args.hosted_model,
            "azure_native_completed": len(azure_native),
            "azure_native_passed": sum(
                bool(cell.get("success")) for cell in azure_native
            ),
            "local_model": LOCAL_MODEL,
            "local_model_digest": args.local_model_digest,
            "local_model_max_output_tokens": LOCAL_MAX_OUTPUT_TOKENS,
        },
        "metrics": metrics,
        "cell_ledger": _compact_cell_ledger(completed),
        "causal_ledger": preflight["deterministic_causal"].get("ledger", []),
        "criteria": criteria,
        "outcome": {
            "panel_complete": panel_complete,
            "status": "passed" if passed else "failed",
            "reason_codes": reason_codes,
            "diagnostics": (
                []
                if passed
                else [f"unsatisfied G2R criteria: {', '.join(reason_codes)}"]
            ),
        },
    }


def _hosted_spend(cells: list[dict[str, Any]]) -> float:
    return sum(
        float(cell.get("total_cost_usd") or 0.0)
        for cell in cells
        if cell.get("completed") is True and cell.get("model") == "hosted"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    repo_root = Path(__file__).resolve().parents[2]
    parser.add_argument("--repo-root", type=Path, default=repo_root)
    parser.add_argument(
        "--oryn-binary", type=Path, default=repo_root / "target/release/oryn"
    )
    parser.add_argument(
        "--chrome-binary",
        type=Path,
        default=Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
    )
    parser.add_argument("--miniwob-url", default="http://127.0.0.1:8765")
    parser.add_argument("--ollama-url", default="http://127.0.0.1:11434")
    parser.add_argument(
        "--local-model-digest",
        default=os.environ.get("ORYN_QWEN3_DIGEST", EXPECTED_QWEN_DIGEST),
    )
    parser.add_argument(
        "--hosted-model",
        default=os.environ.get("AZURE_OPENAI_DEPLOYMENT", HOSTED_MODEL),
    )
    parser.add_argument(
        "--hosted-input-cost-per-million",
        type=float,
        default=float(
            os.environ.get(
                "ORYN_HOSTED_INPUT_COST_PER_MILLION", HOSTED_INPUT_COST_PER_MILLION
            )
        ),
    )
    parser.add_argument(
        "--hosted-output-cost-per-million",
        type=float,
        default=float(
            os.environ.get(
                "ORYN_HOSTED_OUTPUT_COST_PER_MILLION", HOSTED_OUTPUT_COST_PER_MILLION
            )
        ),
    )
    parser.add_argument("--command-timeout", type=float, default=120.0)
    parser.add_argument("--task-timeout", type=int, default=600)
    parser.add_argument("--schema-version", type=int, choices=(4,), default=4)
    parser.add_argument(
        "--prior-hosted-spend",
        type=float,
        default=PRIOR_INVALID_HOSTED_SPEND_USD,
    )
    parser.add_argument(
        "--deterministic-evidence",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/deterministic/aggregate.json",
    )
    parser.add_argument(
        "--churn-evidence",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/g2r-churn.json",
    )
    parser.add_argument(
        "--output", type=Path, default=repo_root / "artifacts/g2r-g5a-v4/model-panel"
    )
    parser.add_argument(
        "--evidence",
        type=Path,
        default=repo_root / "artifacts/g2r-g5a-v4/g2-model-panel-v4.json",
    )
    parser.add_argument("--preflight-only", action="store_true")
    args = parser.parse_args()

    preflight = _preflight(args)
    if args.preflight_only:
        print(
            json.dumps(
                {
                    "status": "passed",
                    "fingerprint_sha256": preflight["fingerprint"]["sha256"],
                    "binary": preflight["fingerprint"]["binary"],
                    "miniwob": preflight["fingerprint"]["miniwob"],
                    "chromium_version": preflight["fingerprint"]["chromium_version"],
                    "models": preflight["fingerprint"]["models"],
                },
                indent=2,
            )
        )
        return 0
    fingerprint = preflight["fingerprint"]["sha256"]
    started_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    cells: list[dict[str, Any]] = []
    for domain, model_kind in MODEL_STAGES:
        for task in TASKS:
            for seed in SEEDS:
                cell_id = f"{task}-s{seed}-{domain}-{model_kind}"
                path = args.output / f"{cell_id}.json"
                if path.exists():
                    existing = json.loads(path.read_text(encoding="utf-8"))
                    if existing.get("completed") is not True:
                        path.unlink()
                    elif existing.get("fingerprint_sha256") != fingerprint:
                        raise RuntimeError(
                            f"refusing to resume {path}: complete fingerprint mismatch"
                        )
                    else:
                        cells.append(existing)
                        continue

                remaining = (
                    HOSTED_CAP_USD - args.prior_hosted_spend - _hosted_spend(cells)
                )
                if model_kind == "hosted" and remaining <= 0:
                    _write_json(
                        args.evidence, _aggregate(args, preflight, cells, started_at)
                    )
                    raise RuntimeError(
                        "cumulative hosted-model ceiling reached before request"
                    )
                config = _config(args, task, seed, domain, model_kind, remaining)
                runner = BenchmarkRunner(config)
                try:
                    result = runner.run(subset=task)[0]
                except Exception:
                    _write_json(
                        args.evidence, _aggregate(args, preflight, cells, started_at)
                    )
                    raise
                finally:
                    runner.close()
                cell = {
                    "cell_id": cell_id,
                    "completed": True,
                    "fingerprint_sha256": fingerprint,
                    "task_id": task,
                    "seed": seed,
                    "domain": domain,
                    "model": model_kind,
                    "provider": "azure_openai" if model_kind == "hosted" else "ollama",
                    "model_id": (
                        args.hosted_model if model_kind == "hosted" else LOCAL_MODEL
                    ),
                    "local_model_digest": (
                        args.local_model_digest if model_kind == "local" else None
                    ),
                    **asdict(result),
                }
                cell.pop("config", None)
                _write_json(path, cell)
                _write_turn_jsonl(
                    args.output / "turns" / f"{cell_id}.jsonl", cell.get("turns", [])
                )
                cells.append(cell)
                _write_json(
                    args.evidence, _aggregate(args, preflight, cells, started_at)
                )
                print(
                    json.dumps(
                        {
                            "cell_id": cell_id,
                            "success": bool(cell.get("success")),
                            "steps": cell.get("total_steps"),
                            "cost_usd": cell.get("total_cost_usd"),
                            "completed_runs": len(cells),
                        }
                    ),
                    flush=True,
                )

        if domain == "native" and model_kind == "hosted":
            azure_native = [
                cell
                for cell in cells
                if cell.get("domain") == "native" and cell.get("model") == "hosted"
            ]
            if (
                len(azure_native) != 24
                or sum(cell.get("success", False) for cell in azure_native) < 15
            ):
                _write_json(
                    args.evidence, _aggregate(args, preflight, cells, started_at)
                )
                raise RuntimeError(
                    "Azure-native quality floor failed; full panel not started"
                )

    evidence = _aggregate(args, preflight, cells, started_at)
    _write_json(args.evidence, evidence)
    return 0 if evidence["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
