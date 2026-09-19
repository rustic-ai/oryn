#!/usr/bin/env python3
"""Recompute the compact application compatibility gate result from its manifest-bound case ledger."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from native_protocol_versions import versions as native_protocol_versions
from typing import Any

from evidence_protocols import validate_protocols, validate_protocol_pair

from jsonschema import Draft202012Validator, FormatChecker


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def source_tree_hash(repo_root: Path) -> str:
    return subprocess.check_output(
        [str(repo_root / "scripts/source-tree-hash.sh")], text=True
    ).strip()


def validate_open_status(status: dict[str, Any]) -> None:
    require(status.get("schema_version") == 1, "invalid open-status schema version")
    require(status.get("gate") == "G3", "invalid open-status gate")
    require(
        status.get("status") == "open", "application compatibility status is not open"
    )
    require(status.get("current_result") is None, "open status must not name a result")
    blockers = status.get("blocking_checks")
    require(
        isinstance(blockers, list) and blockers, "open status needs blocking checks"
    )


def load_json_lines(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for line_number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        if not line.strip():
            continue
        value = json.loads(line)
        require(
            isinstance(value, dict),
            f"{path}:{line_number} must contain a JSON object",
        )
        rows.append(value)
    require(bool(rows), f"raw JSONL artifact is empty: {path}")
    return rows


def trace_is_clean(rows: list[dict[str, Any]]) -> bool:
    for row in rows:
        if row.get("kind") != "trace":
            continue
        for event in row.get("events", []):
            kind = event.get("kind", {})
            if kind.get("kind") == "script_diagnostic":
                return False
            if kind.get("kind") == "console" and kind.get("level") == "error":
                return False
    return True


def raw_artifact(
    artifact_root: Path,
    artifacts: dict[str, dict[str, Any]],
    relative: str,
) -> Path:
    matches = [
        artifact for artifact in artifacts.values() if artifact["path"] == relative
    ]
    require(bool(matches), f"raw artifact is not ledgered: {relative}")
    require(len(matches) == 1, f"raw artifact is ledgered more than once: {relative}")
    path = artifact_root / relative
    require(path.is_file(), f"raw artifact is missing: {path}")
    return path


def require_test_markers(logs: str, case_id: str, markers: tuple[str, ...]) -> None:
    for marker in markers:
        require(
            f"test {marker} ... ok" in logs,
            f"raw Rust test evidence is missing for {case_id}: {marker}",
        )


def validate_raw_evidence(
    result: dict[str, Any],
    artifact_root: Path,
    artifacts: dict[str, dict[str, Any]],
    *,
    repo_root: Path = Path(__file__).resolve().parents[1],
    allow_historical: bool = False,
) -> None:
    """Recompute application compatibility claims from the content-addressed raw artifact tree."""

    production = load_json(
        raw_artifact(artifact_root, artifacts, "production-gate.json")
    )
    runtime = load_json(raw_artifact(artifact_root, artifacts, "runtime.json"))
    churn = load_json(raw_artifact(artifact_root, artifacts, "churn.json"))
    miniwob = load_json(
        raw_artifact(artifact_root, artifacts, "miniwob/aggregate.json")
    )
    benchmarks = load_json(
        raw_artifact(
            artifact_root, artifacts, "benchmarks/app-compatibility-benchmarks-v1.json"
        )
    )

    require(
        production.get("outcome", {}).get("status") == "passed",
        "raw production gate failed",
    )
    require(
        production.get("controlled_oil") == {"passed": 12, "total": 12},
        "raw controlled OIL total is not 12/12",
    )
    require(
        production.get("frameworks") == {"native": 8, "chromium": 8},
        "raw framework total is not 8/8 in both domains",
    )
    require(
        production.get("oauth") == {"native": 5, "chromium": 5},
        "raw OAuth total is not 5/5 in both domains",
    )
    require(
        production.get("source_commit") == result["fingerprint"]["source_commit"],
        "production source commit mismatch",
    )
    require(
        production.get("source_tree_sha256")
        == result["fingerprint"]["source_tree_sha256"],
        "production source-tree hash mismatch",
    )

    validate_protocol_pair(runtime, result["fingerprint"]["worker"], repo_root, historical=allow_historical)
    required_runtime = {
        "contract_version": result["fingerprint"]["worker"]["contract_version"] if allow_historical else 4,
        "native_v8": True,
        "build_profile": "release",
        "execution_mode": "sandboxed_worker",
        "sandboxed": True,
        "sandbox_state": "app_sandbox",
        "worker_protocol_version": result["fingerprint"]["worker"]["protocol_version"] if allow_historical else native_protocol_versions(repo_root)["worker"],
    }
    for key, expected in required_runtime.items():
        require(runtime.get(key) == expected, f"raw runtime {key} is not {expected!r}")
    worker = result["fingerprint"]["worker"]
    require(worker is not None, "raw validation requires a worker fingerprint")
    require(
        runtime.get("worker_binary_sha256") == worker["sha256"],
        "runtime worker hash mismatch",
    )
    require(
        runtime.get("v8_version") == worker["v8_version"], "runtime V8 version mismatch"
    )
    require(
        runtime.get("worker_team_id") == worker["team_id"],
        "runtime worker Team ID mismatch",
    )
    require(
        runtime.get("worker_signing_cert_sha1") == worker["signing_cert_sha1"],
        "runtime worker signing identity mismatch",
    )

    require(churn.get("status") == "passed", "raw churn suite failed")
    false_preservation = churn.get("false_preservation", {})
    require(
        false_preservation.get("numerator") == 0,
        "raw churn suite found false semantic-reference preservation",
    )
    require(
        churn.get("worker_replacement", {}).get("status") == "passed",
        "raw worker replacement suite failed",
    )
    require(
        result["metrics"]["false_semantic_reference_preservations"]
        == false_preservation.get("numerator"),
        "compact churn metric does not match raw evidence",
    )

    require(miniwob.get("cells") == 24, "raw native MiniWoB cell count is not 24")
    require(
        miniwob.get("native") == {"completed": 24, "passed": 24},
        "raw native MiniWoB is not 24/24",
    )
    require(
        miniwob.get("outcome", {}).get("status") == "passed",
        "raw native MiniWoB outcome failed",
    )

    require(
        benchmarks.get("outcome", {}).get("status") == "passed",
        "raw application compatibility benchmark failed",
    )
    page_group = benchmarks.get("multi_page_stream_websocket", {})
    pages = page_group.get("pages")
    require(
        isinstance(pages, int) and pages > 1,
        "raw multi-page benchmark did not exercise concurrent pages",
    )
    require(page_group.get("passed") is True, "raw multi-page benchmark failed")
    require(
        page_group.get("http_requests") == pages,
        "raw HTTP stream benchmark is incomplete",
    )
    require(
        page_group.get("websocket_sessions") == pages,
        "raw WebSocket benchmark is incomplete",
    )
    measurements = page_group.get("page_measurements", [])
    require(len(measurements) == pages, "raw per-page benchmark ledger is incomplete")
    require(
        all(
            item.get("stream_bytes", 0) > 0
            and item.get("stream_chunks", 0) > 1
            and item.get("websocket_echoed") is True
            for item in measurements
        ),
        "raw stream/WebSocket benchmark has a failed page",
    )

    controlled_names = (
        "01_static",
        "02_forms",
        "03_ecommerce",
        "04_interactivity",
        "05_dynamic",
        "06_edge_cases",
        "07_intents_builtin",
        "08_multipage_flows",
        "09_target_resolution",
        "10_react_spa",
        "11_angular_spa",
        "12_popup_detection",
    )
    for name in controlled_names:
        rows = load_json_lines(
            raw_artifact(artifact_root, artifacts, f"controlled/{name}.jsonl")
        )
        require(
            any(row.get("kind") == "observation" for row in rows),
            f"controlled case has no observation: {name}",
        )
        require(
            all(row.get("kind") != "unsupported" for row in rows),
            f"controlled case reported unsupported behavior: {name}",
        )

    framework_ids = {
        f"framework-{name}"
        for name in (
            "react",
            "next",
            "vue",
            "nuxt",
            "angular",
            "svelte",
            "sveltekit",
            "preact",
        )
    }
    for case_id in sorted(framework_ids):
        name = case_id.removeprefix("framework-")
        rows = load_json_lines(
            raw_artifact(artifact_root, artifacts, f"frameworks/{name}-native.jsonl")
        )
        require(
            any(
                row.get("kind") == "structured" and row.get("value") == "Items: 1"
                for row in rows
            ),
            f"native framework postcondition failed: {case_id}",
        )
        if case_id == "framework-react":
            require(
                any(
                    row.get("kind") == "structured"
                    and row.get("value")
                    == "Surface: oryn|tools|2026-08-27|2|1|fixture.txt:7|loaded|closed"
                    for row in rows
                ),
                "native React modern-app surface postcondition failed",
            )
        require(
            trace_is_clean(rows),
            f"native framework trace contains an unexpected diagnostic: {case_id}",
        )
    framework_chromium = load_json_lines(
        raw_artifact(artifact_root, artifacts, "frameworks/chromium.jsonl")
    )
    require(
        {row.get("case") for row in framework_chromium} == framework_ids,
        "Chromium framework case set drifted",
    )
    require(
        all(
            row.get("domain") == "chromium" and row.get("status") == "passed"
            for row in framework_chromium
        ),
        "Chromium framework differential failed",
    )

    oauth_expected = {
        "oauth-popup-code-pkce": ("popup-pkce", "Authenticated with PKCE"),
        "oauth-redirect": ("redirect", "Redirect authenticated"),
        "oauth-existing-session-sso": ("existing-session-sso", "SSO authenticated"),
        "oauth-mfa-handoff-return": ("mfa-handoff", "MFA authenticated"),
        "oauth-popup-denial-close-recovery": (
            "popup-close-recovery",
            "Popup closed; opener recovered",
        ),
    }
    for case_id, (name, expected) in oauth_expected.items():
        rows = load_json_lines(
            raw_artifact(artifact_root, artifacts, f"oauth/{name}-native.jsonl")
        )
        require(
            any(
                row.get("kind") == "structured" and row.get("value") == expected
                for row in rows
            ),
            f"native OAuth postcondition failed: {case_id}",
        )
        require(
            trace_is_clean(rows),
            f"native OAuth trace contains an unexpected diagnostic: {case_id}",
        )
    oauth_chromium = load_json_lines(
        raw_artifact(artifact_root, artifacts, "oauth/chromium.jsonl")
    )
    require(
        {row.get("case") for row in oauth_chromium} == set(oauth_expected),
        "Chromium OAuth case set drifted",
    )
    require(
        all(
            row.get("domain") == "chromium" and row.get("status") == "passed"
            for row in oauth_chromium
        ),
        "Chromium OAuth differential failed",
    )

    chromium_rows = framework_chromium + oauth_chromium
    chromium_versions = {row.get("chromium_version") for row in chromium_rows}
    require(
        chromium_versions == {result["fingerprint"]["chromium_version"]},
        "raw Chromium version does not match the fingerprint",
    )

    limitations = load_json_lines(
        raw_artifact(artifact_root, artifacts, "limitations/capabilities.jsonl")
    )
    unsupported = {
        capability.get("capability")
        for row in limitations
        if row.get("kind") == "observation"
        for capability in row.get("observation", {}).get("capabilities", [])
        if capability.get("support") == "unsupported"
    }
    expected_limitations = {
        "rendered_layout_and_paint",
        "cross_origin_dom",
        "service_worker_and_cache_api",
        "webrtc_and_media",
    }
    require(
        expected_limitations <= unsupported,
        "raw capability diagnostics do not cover every layout deferral",
    )

    rust_workspace = raw_artifact(
        artifact_root, artifacts, "software/rust-workspace.log"
    ).read_text(encoding="utf-8")
    rust_v8 = raw_artifact(artifact_root, artifacts, "software/rust-v8.log").read_text(
        encoding="utf-8"
    )
    rust_logs = rust_workspace + "\n" + rust_v8
    case_markers: dict[str, tuple[str, ...]] = {
        "contract-v3-roundtrip": ("v3::tests::native_refs_are_page_scoped",),
        "contract-v2-adapter": (
            "v3::tests::v2_observation_adapter_roundtrips_without_fabricating_identity",
        ),
        "page-list-active-wait-close": (
            "runtime::tests::page_group_shares_local_storage_clones_session_and_rejects_foreign_refs",
            "runtime::tests::page_close_does_not_invalidate_an_unrelated_page_reference",
        ),
        "popup-opener-focus-close": (
            "runtime::tests::popup_opener_post_message_and_storage_events_are_context_scoped",
            "runtime::tests::page_close_does_not_invalidate_an_unrelated_page_reference",
        ),
        "target-blank-form-target": (
            "runtime::tests::target_blank_links_and_forms_create_opener_scoped_pages",
        ),
        "postmessage-origin-check": (
            "runtime::tests::popup_opener_post_message_and_storage_events_are_context_scoped",
        ),
        "cross-page-ref-rejected": (
            "runtime::tests::page_group_shares_local_storage_clones_session_and_rejects_foreign_refs",
        ),
        "unrelated-page-ref-survives": (
            "runtime::tests::page_close_does_not_invalidate_an_unrelated_page_reference",
        ),
        "worker-replacement-invalidates-page": (
            "runtime::tests::worker_replacement_reopens_a_blank_generation_without_touching_other_pages",
        ),
        "localstorage-context-sharing": (
            "storage::tests::local_is_shared_and_session_is_page_scoped",
            "runtime::tests::page_group_shares_local_storage_clones_session_and_rejects_foreign_refs",
        ),
        "sessionstorage-popup-copy-diverge": (
            "runtime::tests::page_group_shares_local_storage_clones_session_and_rejects_foreign_refs",
        ),
        "storage-events-other-realms-only": (
            "runtime::tests::popup_opener_post_message_and_storage_events_are_context_scoped",
        ),
        "persistent-cookie-localstorage-restart": (
            "storage::tests::persistent_local_storage_survives_reopen",
        ),
        "persistent-profile-isolation": (
            "storage::tests::persistent_profiles_are_isolated_and_session_storage_is_not_persisted",
        ),
        "storage-quota-enforcement": (
            "storage::tests::storage_quotas_are_origin_scoped_and_failed_writes_are_atomic",
        ),
        "indexeddb-upgrade-transactions": (
            "runtime::tests::indexed_db_upgrade_transaction_and_cursor_subset_is_executable",
        ),
        "indexeddb-keypath-autoincrement-index-cursor": (
            "runtime::tests::indexed_db_upgrade_transaction_and_cursor_subset_is_executable",
            "storage::tests::indexed_db_batches_are_atomic_and_enforce_add",
        ),
        "indexeddb-structured-clone-subset": (
            "runtime::tests::indexed_db_structured_clone_roundtrips_binary_blob_and_file",
        ),
        "xhr-async-states-cors-cookies-abort-timeout": (
            "runtime::tests::asynchronous_xhr_reports_states_headers_and_json",
            "runtime::tests::cross_origin_credentials_obey_cookie_and_cors_rules",
            "runtime::tests::xhr_abort_and_timeout_are_observable",
        ),
        "xhr-sync-main-thread-diagnostic": (
            "runtime::tests::xhr_sync_main_thread_is_truthfully_unsupported",
        ),
        "fetch-byte-preserving-body": (
            "runtime::tests::fetch_preserves_binary_request_bytes_and_abort_prevents_dispatch",
        ),
        "streams-read-write-transform-pipe-cancel": (
            "runtime::tests::response_body_streams_in_bounded_parent_owned_chunks",
            "runtime::tests::streams_expose_cancellation_and_bounded_writer_readiness",
            "runtime::tests::web_crypto_file_and_stream_subset_is_executable",
        ),
        "websocket-policy-text-binary-close": (
            "runtime::tests::parent_brokered_websocket_delivers_text_binary_and_close_frames",
        ),
        "file-blob-formdata-request-response-headers": (
            "runtime::tests::file_formdata_request_response_and_headers_are_executable",
        ),
        "abortcontroller-propagation": (
            "runtime::tests::fetch_preserves_binary_request_bytes_and_abort_prevents_dispatch",
        ),
        "crypto-random-uuid-sha256": (
            "runtime::tests::web_crypto_file_and_stream_subset_is_executable",
        ),
        "dynamic-import-module-map-chunks": (
            "executable::tests::dynamic_import_uses_the_reusable_module_map",
            "host::tests::static_modules_and_import_map_aliases_share_one_module_instance",
        ),
        "webidl-partial-dictionary-callback-inheritance": (
            "tests::parses_and_merges_extended_binding_constructs",
        ),
        "range-selection-treewalker-nodeiterator": (
            "executable::tests::range_selection_traversal_and_contenteditable_are_available",
        ),
        "focus-contenteditable-form-custom-shadow-frame": (
            "executable::tests::range_selection_traversal_and_contenteditable_are_available",
            "executable::tests::form_constraint_validation_controls_default_submission",
            "executable::tests::select_options_expose_browser_style_collection_and_selection_state",
            "executable::tests::open_shadow_roots_are_semantic_and_closed_roots_are_isolated",
            "runtime::tests::same_origin_iframe_is_semantic_and_cross_origin_is_diagnostic",
        ),
    }
    for case_id, markers in case_markers.items():
        require_test_markers(rust_logs, case_id, markers)

    expected_software_artifacts = (
        "rust-fmt",
        "rust-clippy-default",
        "rust-clippy-v8",
        "rust-workspace",
        "rust-v8",
        "python-ruff",
        "python-format",
        "python-tests",
        "intentgym-black",
        "intentgym-isort",
        "intentgym-tests",
        "scanner-sync",
        "scanner-lint",
        "scanner-format",
        "scanner-tests",
        "extension-wasm",
        "extension-lint",
        "extension-unit",
        "extension-integration",
        "extension-e2e",
    )
    for name in expected_software_artifacts:
        raw_artifact(artifact_root, artifacts, f"software/{name}.log")

    cases = {case["id"]: case for case in result["case_ledger"]}
    for case_id in set(case_markers) | framework_ids | set(oauth_expected):
        require(
            cases[case_id]["status"] == "passed",
            f"compact case status disagrees with raw proof: {case_id}",
        )
    for case_id in (
        "pixel-geometry",
        "paint-screenshot",
        "cross-origin-frame-dom",
        "service-worker-cache-api",
        "webrtc-media-capture",
    ):
        require(
            cases[case_id]["status"] == "expected_limitation",
            f"compact deferral status disagrees with raw proof: {case_id}",
        )


def validate_result(
    repo_root: Path,
    result: dict[str, Any],
    artifact_root: Path | None,
    require_raw: bool,
    allow_historical: bool,
) -> None:
    validate_protocols(result.get("fingerprint", {}).get("worker"), repo_root, fingerprint=True, historical=allow_historical)
    if not allow_historical:
        require(result["fingerprint"]["worker"].get("contract_version") == 4, "current runtime contract must be v4")
    schema_path = (
        repo_root / "benchmarks/schema/app-compatibility-result-v1.schema.json"
    )
    manifest_path = (
        repo_root / "benchmarks/conformance/app-compatibility-manifest-v1.json"
    )
    fixture_lock_path = repo_root / "benchmarks/app-compatibility/fixtures.lock.json"
    wpt_path = repo_root / "benchmarks/app-compatibility/wpt-subset-v1.json"
    schema = load_json(schema_path)
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(
            result
        ),
        key=lambda error: list(error.absolute_path),
    )
    require(
        not errors,
        "schema validation failed: "
        + "; ".join(
            f"{'/'.join(map(str, error.absolute_path)) or '<root>'}: {error.message}"
            for error in errors
        ),
    )

    manifest = load_json(manifest_path)
    manifest_cases = {case["id"]: case for case in manifest["cases"]}
    artifact_ledger = result["artifact_ledger"]
    artifact_ids = [artifact["id"] for artifact in artifact_ledger]
    require(
        len(artifact_ids) == len(set(artifact_ids)),
        "artifact ledger contains duplicate IDs",
    )
    artifacts = {artifact["id"]: artifact for artifact in artifact_ledger}
    artifact_paths = [artifact["path"] for artifact in artifact_ledger]
    require(
        len(artifact_paths) == len(set(artifact_paths)),
        "artifact ledger contains duplicate paths",
    )
    for artifact in artifact_ledger:
        relative = Path(artifact["path"])
        require(
            not relative.is_absolute() and ".." not in relative.parts,
            f"artifact path is not safely relative: {relative}",
        )
        if artifact_root is not None:
            raw_path = artifact_root / relative
            require(raw_path.is_file(), f"raw artifact is missing: {raw_path}")
            require(
                raw_path.stat().st_size == artifact["bytes"],
                f"artifact size mismatch: {raw_path}",
            )
            require(
                sha256(raw_path) == artifact["sha256"],
                f"artifact hash mismatch: {raw_path}",
            )
    require(
        not require_raw or artifact_root is not None,
        "--require-raw requires --artifact-root",
    )
    if artifact_root is not None:
        validate_raw_evidence(result, artifact_root, artifacts, repo_root=repo_root,
                              allow_historical=allow_historical)
    ledger = result["case_ledger"]
    ledger_ids = [case["id"] for case in ledger]
    require(
        len(ledger_ids) == len(set(ledger_ids)), "case ledger contains duplicate IDs"
    )
    require(
        set(ledger_ids) == set(manifest_cases),
        "case ledger does not match manifest IDs",
    )

    required_passed = 0
    deferred_expected = 0
    for case in ledger:
        declared = manifest_cases[case["id"]]
        require(
            case["category"] == declared["category"], f"category drift: {case['id']}"
        )
        require(
            case["disposition"] == declared["disposition"],
            f"disposition drift: {case['id']}",
        )
        require(case["evidence"], f"case has no compact evidence: {case['id']}")
        require(
            all(evidence in artifacts for evidence in case["evidence"]),
            f"case references unknown evidence: {case['id']}",
        )
        native = case["domains"].get("native")
        chromium = case["domains"].get("chromium_differential")
        if declared["disposition"] == "required":
            if case["status"] == "passed":
                required_passed += 1
            require(
                case["status"] != "passed" or native == "passed",
                f"passed required case lacks native proof: {case['id']}",
            )
            if declared["category"] in {"framework", "oauth"}:
                require(
                    case["status"] != "passed" or chromium == "passed",
                    f"passed differential case lacks Chromium proof: {case['id']}",
                )
        else:
            expected = declared["expected_capability"]
            require(
                case.get("expected_capability") == expected,
                f"deferred capability drift: {case['id']}",
            )
            if case["status"] == "expected_limitation" and native == "passed":
                deferred_expected += 1

    required_total = sum(
        case["disposition"] == "required" for case in manifest_cases.values()
    )
    deferred_total = len(manifest_cases) - required_total
    metrics = result["metrics"]
    require(
        metrics["required_total"] == required_total, "required total was not recomputed"
    )
    require(metrics["required_passed"] == required_passed, "required passed mismatch")
    require(metrics["deferred_total"] == deferred_total, "deferred total mismatch")
    require(
        metrics["deferred_expected"] == deferred_expected,
        "expected deferred count mismatch",
    )

    fingerprint = result["fingerprint"]
    require(
        fingerprint["manifest_sha256"] == sha256(manifest_path),
        "manifest hash mismatch",
    )
    if not allow_historical:
        require(
            fingerprint["fixture_lock_sha256"] == sha256(fixture_lock_path),
            "fixture lock hash mismatch",
        )
    require(fingerprint["wpt_subset_sha256"] == sha256(wpt_path), "WPT hash mismatch")
    if not allow_historical:
        require(
            fingerprint["source_tree_sha256"] == source_tree_hash(repo_root),
            "source-tree hash mismatch",
        )

    outcome = result["outcome"]
    if outcome["status"] == "passed":
        require(fingerprint["source_dirty"] is False, "passed evidence must be clean")
        source_is_ancestor = (
            subprocess.run(
                [
                    "git",
                    "merge-base",
                    "--is-ancestor",
                    fingerprint["source_commit"],
                    "HEAD",
                ],
                cwd=repo_root,
                check=False,
            ).returncode
            == 0
        )
        require(
            source_is_ancestor,
            "tested source commit is not an ancestor of the evidence commit",
        )
        require(
            fingerprint["parent"] is not None, "passed evidence lacks parent binary"
        )
        require(
            fingerprint["worker"] is not None, "passed evidence lacks signed worker"
        )
        require(required_passed == required_total, "not every required case passed")
        require(deferred_expected == deferred_total, "not every deferral was truthful")
        require(
            metrics["false_semantic_reference_preservations"] == 0,
            "false semantic-reference preservation is nonzero",
        )
        require(
            result["regressions"]["native_miniwob_24"] == "passed",
            "native MiniWoB regression is incomplete",
        )
        require(
            result["regressions"]["signed_production_boundary"] == "passed",
            "signed production boundary is incomplete",
        )
        require(
            all(value == "passed" for value in result["regressions"].values()),
            "one or more regressions did not pass",
        )
        require(
            result["criteria"] and all(result["criteria"].values()), "criteria failed"
        )
        require(not outcome["reason_codes"], "passed evidence has failure reason codes")
    else:
        require(
            bool(outcome["reason_codes"]),
            "failed evidence must explain why the gate remains open",
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[1]
    )
    parser.add_argument("--result", type=Path)
    parser.add_argument("--artifact-root", type=Path)
    parser.add_argument("--require-raw", action="store_true")
    parser.add_argument("--allow-open", action="store_true")
    parser.add_argument("--allow-historical", action="store_true")
    args = parser.parse_args()
    repo_root = args.repo_root.resolve()
    result_path = (
        args.result
        or repo_root / "benchmarks/evidence/app-compatibility-v1/result-v1.json"
    )
    if not result_path.exists():
        if not args.allow_open:
            raise FileNotFoundError(
                f"application compatibility result is not published: {result_path}"
            )
        validate_open_status(
            load_json(
                repo_root / "benchmarks/evidence/app-compatibility-v1/STATUS.json"
            )
        )
        print(
            "application compatibility remains open; result validator and blockers are structurally valid"
        )
        return 0
    artifact_root = args.artifact_root.resolve() if args.artifact_root else None
    validate_result(
        repo_root,
        load_json(result_path),
        artifact_root,
        args.require_raw,
        args.allow_historical,
    )
    print(
        f"application compatibility result is schema-valid and recomputed from 48 manifest-bound cases: {result_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
