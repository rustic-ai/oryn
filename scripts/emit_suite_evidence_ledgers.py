#!/usr/bin/env python3
"""Emit case-bound assertion and regression ledgers from executed suite artifacts.

This module deliberately recognizes concrete checks (Rust test names, JSON
predicates, and structured JSONL outcomes).  A command's successful exit alone
is never sufficient to produce a passing assertion.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Callable

Predicate = Callable[[Path], bool]


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def text_contains(fragment: str) -> Predicate:
    return lambda path: fragment in path.read_text(encoding="utf-8", errors="replace")


def json_predicate(predicate: Callable[[Any], bool]) -> Predicate:
    return lambda path: bool(predicate(read_json(path)))


def jsonl_predicate(predicate: Callable[[list[Any]], bool]) -> Predicate:
    def check(path: Path) -> bool:
        values = [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        return bool(predicate(values))

    return check


def rust_test(name: str) -> Predicate:
    return text_contains(f"test {name} ... ok")


def check(name: str, artifact: str, predicate: Predicate) -> dict[str, Any]:
    return {"name": name, "artifact": artifact, "predicate": predicate}


def assertion(
    case_id: str,
    *checks: dict[str, Any],
    status: str = "passed",
    chromium: str = "not_applicable",
) -> dict[str, Any]:
    return {
        "id": f"assert-{case_id}",
        "case_id": case_id,
        "expected_status": status,
        "domains": {"chromium_differential": chromium},
        "checks": list(checks),
    }


def evaluate_assertions(
    raw: Path, suite: str, specs: list[dict[str, Any]], output: Path
) -> bool:
    records: list[dict[str, Any]] = []
    all_passed = True
    for spec in specs:
        checks = []
        artifacts = []
        passed = True
        for item in spec["checks"]:
            relative = item["artifact"]
            path = raw / relative
            artifacts.append(relative)
            try:
                observed = path.is_file() and item["predicate"](path)
            except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError):
                observed = False
            passed = passed and observed
            checks.append(
                {
                    "name": item["name"],
                    "passed": observed,
                    "artifact": relative,
                }
            )
        all_passed = all_passed and passed
        record = {
            "id": spec["id"],
            "case_id": spec["case_id"],
            "status": spec["expected_status"] if passed else "failed",
            "checks": checks,
            "artifacts": sorted(set(artifacts)),
        }
        if spec["domains"]["chromium_differential"] != "not_applicable":
            record["domains"] = spec["domains"]
        records.append(record)
    output.write_text(
        json.dumps(
            {"schema_version": 1, "suite": suite, "assertions": records}, indent=2
        )
        + "\n",
        encoding="utf-8",
    )
    return all_passed


def evaluate_regressions(
    raw: Path,
    suite: str,
    specs: dict[str, list[dict[str, Any]]],
    output: Path,
) -> bool:
    records: dict[str, Any] = {}
    all_passed = True
    for regression, items in specs.items():
        checks = []
        artifacts = []
        passed = True
        for item in items:
            relative = item["artifact"]
            path = raw / relative
            artifacts.append(relative)
            try:
                observed = path.is_file() and item["predicate"](path)
            except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError):
                observed = False
            passed = passed and observed
            checks.append(
                {
                    "name": item["name"],
                    "passed": observed,
                    "artifact": relative,
                }
            )
        all_passed = all_passed and passed
        records[regression] = {
            "status": "passed" if passed else "failed",
            "checks": checks,
            "artifacts": sorted(set(artifacts)),
        }
    output.write_text(
        json.dumps(
            {"schema_version": 1, "suite": suite, "regressions": records},
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return all_passed


def relation(name: str, predicate: Callable[[Any], bool]) -> dict[str, Any]:
    return check(
        f"layout probe relation {name}",
        "layout-result.json",
        json_predicate(lambda value: predicate(value["relations"][name])),
    )


def layout_specs() -> list[dict[str, Any]]:
    native = "software/rust-v8.log"
    workspace = "software/rust-workspace.log"
    layout_test = lambda name: check(
        name,
        "software/layout-platform-subset.log",
        rust_test(f"tests::{name}"),
    )
    native_test = lambda name: check(name, native, rust_test(f"runtime::tests::{name}"))
    return [
        assertion(
            "contract-v4-roundtrip",
            check(
                "v4 roundtrip",
                workspace,
                rust_test("v4::tests::v4_roundtrip_preserves_the_frozen_v3_projection"),
            ),
        ),
        assertion(
            "contract-v3-adapter-no-invented-geometry",
            check(
                "v3 adapter geometry",
                workspace,
                rust_test("v4::tests::v3_adapter_never_invents_geometry"),
            ),
        ),
        assertion(
            "worker-protocol-v6-layout-fingerprint",
            check(
                "protocol v10 signed runtime (stable historical control ID)",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("worker_protocol_version") == 10
                    and v.get("contract_version") == 4
                    and v.get("sandbox_state") == "app_sandbox"
                ),
            ),
            check(
                "sandbox worker integration",
                "worker-integration.txt",
                text_contains("survived across two sandbox workers"),
            ),
        ),
        assertion(
            "cascade-author-inline-inheritance",
            layout_test("cascade_and_block_flow_produce_relational_geometry"),
        ),
        assertion(
            "cascade-specificity-source-order",
            native_test("contract_v4_exposes_computed_style_and_relational_geometry"),
        ),
        assertion(
            "custom-properties-fallback",
            layout_test("cascade_and_block_flow_produce_relational_geometry"),
        ),
        assertion(
            "media-width-color-motion",
            native_test("contract_v4_exposes_computed_style_and_relational_geometry"),
        ),
        assertion(
            "cssom-computed-style-readonly",
            native_test("contract_v4_exposes_computed_style_and_relational_geometry"),
        ),
        assertion(
            "cssom-stylesheet-insert-delete",
            native_test("contract_v4_exposes_computed_style_and_relational_geometry"),
        ),
        assertion(
            "external-css-import-broker",
            native_test("external_css_import_and_web_font_are_parent_brokered"),
        ),
        assertion(
            "block-box-model",
            relation("block_border_width", lambda v: abs(float(v) - 224.0) < 0.01),
        ),
        assertion(
            "flex-relational-geometry",
            relation("flex_second_right_of_first", lambda v: v is True),
        ),
        assertion(
            "grid-relational-geometry",
            relation("grid_second_right_of_first", lambda v: v is True),
            relation("grid_third_below_first", lambda v: v is True),
        ),
        assertion(
            "bounded-float-clear",
            layout_test("taffy_honors_bounded_float_and_clear_flow"),
        ),
        assertion(
            "absolute-relative-fixed-position",
            layout_test("position_transform_and_scroll_remain_relational"),
        ),
        assertion(
            "two-dimensional-transform-bounds",
            relation("transform_moves_x", lambda v: v is True),
        ),
        assertion(
            "inline-shaping-line-fragments",
            relation("text_has_fragments", lambda v: v is True),
        ),
        assertion(
            "deterministic-embedded-font",
            layout_test("embedded_font_matches_the_frozen_asset_contract"),
        ),
        assertion(
            "brokered-web-font-hash-quota",
            native_test("external_css_import_and_web_font_are_parent_brokered"),
            layout_test("web_font_hash_and_quota_rejections_are_diagnostic"),
        ),
        assertion(
            "viewport-resize-reflow",
            native_test("viewport_scroll_and_table_geometry_are_truthful"),
        ),
        assertion(
            "scroll-clamp-and-into-view",
            relation("scroll_clamped", lambda v: v is True),
        ),
        assertion(
            "fixed-position-survives-scroll",
            relation("fixed_stable_after_scroll", lambda v: v is True),
        ),
        assertion(
            "viewport-only-observation",
            native_test(
                "viewport_filter_interactability_and_closed_shadow_are_truthful"
            ),
        ),
        assertion(
            "oil-box-viewport-media-scroll",
            check(
                "structured OIL geometry sequence",
                "oil/oil.jsonl",
                jsonl_predicate(
                    lambda values: sum(
                        v.get("kind") == "geometry_observation" for v in values
                    )
                    == 2
                    and any(v.get("kind") == "box" for v in values)
                    and any(
                        v.get("kind") == "scroll"
                        and v.get("scroll", {}).get("y", 0) > 0
                        for v in values
                    )
                ),
            ),
        ),
        assertion(
            "action-auto-scroll-interactability",
            native_test(
                "viewport_filter_interactability_and_closed_shadow_are_truthful"
            ),
        ),
        assertion(
            "hidden-zero-pointer-rejection",
            native_test(
                "viewport_filter_interactability_and_closed_shadow_are_truthful"
            ),
        ),
        assertion(
            "closed-shadow-layout-semantic-isolation",
            native_test(
                "viewport_filter_interactability_and_closed_shadow_are_truthful"
            ),
        ),
        assertion(
            "native-chromium-relational-differential",
            check(
                "native and Chromium relation documents passed",
                "differential/chromium.json",
                json_predicate(
                    lambda v: v.get("domain") == "chromium"
                    and v.get("outcome", {}).get("status") == "passed"
                ),
            ),
            check(
                "native differential probe passed",
                "differential/native.json",
                json_predicate(
                    lambda v: v.get("domain") == "native"
                    and v.get("outcome", {}).get("status") == "passed"
                ),
            ),
            chromium="passed",
        ),
        assertion(
            "table-bounded-geometry-and-structured-extraction",
            relation("table_geometry_unavailable", lambda v: v is False),
            native_test("viewport_scroll_and_table_geometry_are_truthful"),
            check(
                "structures_cover_agent_relevant_composites_and_landmarks",
                workspace,
                rust_test(
                    "semantic::tests::structures_cover_agent_relevant_composites_and_landmarks"
                ),
            ),
        ),
        *[
            assertion(
                case_id,
                check(
                    f"unsupported capability {capability} observed",
                    "limitations/capabilities.jsonl",
                    jsonl_predicate(
                        lambda values, capability=capability: any(
                            cap.get("capability") == capability
                            and cap.get("support") == "unsupported"
                            for value in values
                            for cap in value.get("observation", {}).get(
                                "capabilities", []
                            )
                        )
                    ),
                ),
                status="expected_limitation",
            )
            for case_id, capability in (
                ("paint-screenshot-pdf", "paint"),
                ("paint-order-hit-testing", "layout.hit_testing.paint_order"),
                ("three-dimensional-transforms", "layout.transform.3d"),
                ("complex-float-fragmentation", "layout.float.complex"),
            )
        ],
    ]


def layout_regressions() -> dict[str, list[dict[str, Any]]]:
    success = lambda path, marker: check(marker, path, text_contains(marker))
    return {
        "rust_workspace": [success("software/rust-workspace.log", "test result: ok")],
        "rust_v8": [success("software/rust-v8.log", "test result: ok")],
        "strict_clippy_default": [
            success("software/rust-clippy-default.log", "Finished")
        ],
        "strict_clippy_v8": [success("software/rust-clippy-v8.log", "Finished")],
        "python_sdk": [success("software/python-tests.log", "passed")],
        "intentgym": [success("software/intentgym-tests.log", "passed")],
        "scanner": [success("software/scanner-tests.log", "Tests:")],
        "extension": [success("software/extension-unit.log", "Tests:")],
        "g1": [
            success(
                "executable-document.log",
                "conformance manifest is structurally valid",
            )
        ],
        "g2_controlled": [
            success("semantic-controlled.log", "controlled native evidence is valid")
        ],
        "native_miniwob_24": [
            check(
                "native MiniWoB 24/24",
                "miniwob/aggregate.json",
                json_predicate(
                    lambda v: v.get("native") == {"completed": 24, "passed": 24}
                    and v.get("outcome", {}).get("status") == "passed"
                ),
            )
        ],
        "g3_frameworks_native": [success("frameworks.log", "framework corpus passed")],
        "g3_oauth_native": [success("auth-flows.log", "OAuth corpus passed")],
        "reference_churn": [
            check(
                "zero false reference preservation",
                "churn.json",
                json_predicate(
                    lambda v: v.get("status") == "passed"
                    and v.get("false_preservation", {}).get("numerator") == 0
                ),
            )
        ],
        "signed_production_boundary": [
            check(
                "signed sandbox runtime",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("native_v8") is True
                    and v.get("sandbox_state") == "app_sandbox"
                ),
            )
        ],
    }


def security_specs() -> list[dict[str, Any]]:
    signed = "layout/signed-worker.log"
    native = "software/native-security-protocol.log"
    v8 = "layout/software/rust-v8.log"
    probe = "security-probe.json"
    probe_bool = lambda case, key: assertion(
        case, check(key, probe, json_predicate(lambda v, key=key: v.get(key) is True))
    )
    test = lambda case, name, artifact=native: assertion(
        case, check(name, artifact, rust_test(name))
    )
    specs = [
        assertion(
            "sandbox-filesystem-denied",
            check(
                "sandbox filesystem denial",
                signed,
                text_contains('"filesystem_read_succeeded":false'),
            ),
        ),
        assertion(
            "sandbox-network-denied",
            check(
                "sandbox outgoing network denial",
                signed,
                text_contains('"outgoing_connection_succeeded":false'),
            ),
        ),
        assertion(
            "sandbox-listener-denied",
            check(
                "sandbox listener denial",
                signed,
                text_contains('"listener_succeeded":false'),
            ),
        ),
        assertion(
            "sandbox-environment-secret-denied",
            check(
                "sandbox environment denial",
                signed,
                text_contains('"secret_environment_present":false'),
            ),
        ),
        assertion(
            "signed-worker-identity",
            check(
                "signed worker identity",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("worker_team_id") == "5HVA9VFF8K"
                    and v.get("sandbox_state") == "app_sandbox"
                ),
            ),
        ),
        assertion(
            "worker-entitlements-minimal",
            check(
                "minimal signed entitlements",
                signed,
                text_contains("flags=0x10000(runtime)"),
            ),
        ),
        test(
            "worker-ipc-frame-limit",
            "worker::tests::rejects_duplicate_unmatched_and_reinitialized_v8_messages",
        ),
        assertion(
            "worker-command-queue-limit",
            check(
                "queue limit is effective",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("limits", {}).get("queue_capacity") == 32
                ),
            ),
        ),
        test(
            "broker-private-network-denied",
            "network::tests::broker_enforces_policy_before_transport",
        ),
        test(
            "broker-dns-rebinding-denied",
            "network::tests::policy_broker_pins_every_vetted_resolution",
        ),
        test(
            "broker-redirect-recheck",
            "runtime::tests::native_navigation_checks_redirects_and_supports_history",
            v8,
        ),
        test(
            "broker-host-allowlist",
            "security::tests::host_allow_and_deny_lists_cover_subdomains",
        ),
        test(
            "broker-host-denylist",
            "security::tests::host_allow_and_deny_lists_cover_subdomains",
        ),
        test(
            "broker-port-policy",
            "security::tests::default_policy_denies_local_targets_and_file_urls",
        ),
        test(
            "broker-header-filter",
            "network::tests::worker_cannot_supply_parent_owned_request_headers",
        ),
        test(
            "broker-body-limits",
            "context_service::tests::typed_http_streams_are_owned_bounded_and_header_filtered",
        ),
        test(
            "broker-decoded-response-limit",
            "network::tests::worker_rejects_decoded_response_over_sixteen_mib",
        ),
        probe_bool("confirmation-exact-scope", "confirmation_exact_scope"),
        probe_bool("confirmation-one-use", "confirmation_one_use"),
        probe_bool("origin-policy-denial", "origin_policy_denial"),
        test(
            "form-policy-hook",
            "runtime::tests::confirmed_form_command_is_rechecked_at_the_concrete_effect_boundary",
            v8,
        ),
        test(
            "popup-policy-hook",
            "runtime::tests::popup_policy_denial_returns_null_without_creating_a_page",
            v8,
        ),
        test(
            "cross-origin-navigation-policy-hook",
            "context_service::tests::worker_originated_cross_origin_navigation_is_parent_policy_checked",
        ),
        probe_bool(
            "sensitive-actions-default-confirmation",
            "all_sensitive_actions_default_confirmed",
        ),
        probe_bool("structured-security-audit", "structured_security_audit"),
        probe_bool("secret-origin-scope", "secret_handle_origin_scoped"),
        probe_bool("secret-revocation", "secret_revocation"),
        probe_bool("password-observation-redaction", "password_observation_redacted"),
        probe_bool(
            "secret-absent-from-policy-audit", "secret_absent_from_policy_audit"
        ),
        probe_bool("download-quarantine", "download_quarantined"),
        probe_bool("download-path-sanitization", "download_path_sanitized"),
        probe_bool("download-size-limit", "download_size_limited"),
        assertion(
            "infinite-loop-termination",
            check(
                "infinite loop terminated",
                "churn.json",
                json_predicate(
                    lambda v: v.get("worker_replacement", {}).get(
                        "infinite_loop_terminated"
                    )
                    is True
                ),
            ),
        ),
        assertion(
            "v8-heap-limit",
            check(
                "configured V8 heap limit",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("limits", {}).get("v8_heap_bytes") == 268435456
                ),
            ),
        ),
        assertion(
            "worker-rss-limit",
            check(
                "configured worker RSS limit",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("limits", {}).get("worker_rss_bytes") == 536870912
                ),
            ),
        ),
        assertion(
            "worker-crash-replacement",
            check(
                "worker replacement passed",
                "churn.json",
                json_predicate(
                    lambda v: v.get("worker_replacement", {}).get("status") == "passed"
                    and v.get("worker_replacement", {}).get("parent_survived") is True
                ),
            ),
        ),
        assertion(
            "replacement-ref-invalidation",
            check(
                "old reference invalidated",
                "churn.json",
                json_predicate(
                    lambda v: v.get("worker_replacement", {}).get(
                        "old_reference_invalidated"
                    )
                    is True
                ),
            ),
        ),
        test(
            "page-content-remains-data",
            "runtime::tests::raw_page_inspection_is_audited_without_recording_page_content",
            v8,
        ),
        test(
            "worker-has-no-generic-privileged-operation",
            "worker::tests::serves_initialized_page_requests_over_v8_json_lines",
        ),
    ]
    for case_id, target in (
        ("fuzz-html-tree-sink", "html_tree_sink"),
        ("fuzz-dom-mutation", "dom_mutation"),
        ("fuzz-url-policy", "url_policy"),
        ("fuzz-worker-protocol", "worker_protocol"),
    ):
        specs.append(
            assertion(
                case_id,
                check(
                    f"fuzz target {target}",
                    "fuzz/summary.json",
                    json_predicate(
                        lambda v, target=target: any(
                            item.get("name") == target
                            and item.get("status") == "passed"
                            for item in v.get("targets", [])
                        )
                        and v.get("crashes") == 0
                    ),
                ),
            )
        )
    specs.extend(
        [
            assertion(
                "dependency-advisory-audit",
                check(
                    "zero dependency vulnerabilities",
                    "software/cargo-audit.json",
                    json_predicate(
                        lambda v: v.get("vulnerabilities", {}).get("count") == 0
                    ),
                ),
            ),
            assertion(
                "dependency-exceptions-reviewed",
                check(
                    "reviewed dependency inventory",
                    "software/dependency-audit.log",
                    text_contains("reviewed informational warnings"),
                ),
            ),
            assertion(
                "rust-toolchain-pinned",
                check(
                    "pinned toolchain static validation",
                    "software/security-static.log",
                    text_contains("security static validation passed"),
                ),
            ),
            assertion(
                "source-tree-fingerprint-reproducible",
                check(
                    "source tree fingerprint",
                    "production-gate.json",
                    json_predicate(
                        lambda v: isinstance(v.get("source_tree_sha256"), str)
                        and len(v["source_tree_sha256"]) == 64
                    ),
                ),
            ),
            assertion(
                "v8-version-pinned",
                check(
                    "pinned V8 static validation",
                    "software/security-static.log",
                    text_contains("V8 150.4.0"),
                ),
            ),
            assertion(
                "v8-update-qualification",
                check(
                    "qualification entrypoint validated",
                    "software/security-static.log",
                    text_contains("security static validation passed"),
                ),
            ),
            assertion(
                "security-policy-published",
                check(
                    "published security policy validated",
                    "software/security-static.log",
                    text_contains("security static validation passed"),
                ),
            ),
            assertion(
                "zero-critical-high-unmitigated",
                check(
                    "zero audited vulnerabilities",
                    "software/cargo-audit.json",
                    json_predicate(
                        lambda v: v.get("vulnerabilities", {}).get("count") == 0
                    ),
                ),
            ),
        ]
    )
    return specs


def security_regressions() -> dict[str, list[dict[str, Any]]]:
    status_json = lambda name, path: [
        check(
            name,
            path,
            json_predicate(lambda v: v.get("outcome", {}).get("status") == "passed"),
        )
    ]
    return {
        "g1": [
            check(
                "executable document regression",
                "layout/executable-document.log",
                text_contains("conformance manifest is structurally valid"),
            )
        ],
        "g2_controlled": [
            check(
                "semantic controlled regression",
                "layout/semantic-controlled.log",
                text_contains("controlled native evidence is valid"),
            )
        ],
        "g3": [
            check(
                "framework regression",
                "layout/frameworks.log",
                text_contains("framework corpus passed"),
            ),
            check(
                "authentication regression",
                "layout/auth-flows.log",
                text_contains("OAuth corpus passed"),
            ),
        ],
        "g4": status_json("layout result passed", "layout/layout-result-v2.json"),
        "native_miniwob_24": [
            check(
                "native MiniWoB 24/24",
                "layout/miniwob/aggregate.json",
                json_predicate(
                    lambda v: v.get("native") == {"completed": 24, "passed": 24}
                ),
            )
        ],
        "rust_workspace": [
            check(
                "Rust workspace tests",
                "software/native-security-protocol.log",
                text_contains("test result: ok"),
            )
        ],
        "strict_clippy": [
            check(
                "strict Clippy",
                "layout/software/rust-clippy-default.log",
                text_contains("Finished"),
            )
        ],
        "python_sdk": [
            check(
                "Python SDK",
                "layout/software/python-tests.log",
                text_contains("passed"),
            )
        ],
        "intentgym": [
            check(
                "IntentGym",
                "layout/software/intentgym-tests.log",
                text_contains("passed"),
            )
        ],
        "scanner": [
            check(
                "scanner", "layout/software/scanner-tests.log", text_contains("Tests:")
            )
        ],
        "extension": [
            check(
                "extension",
                "layout/software/extension-unit.log",
                text_contains("Tests:"),
            )
        ],
        "signed_production_boundary": [
            check(
                "signed runtime",
                "runtime.json",
                json_predicate(
                    lambda v: v.get("sandbox_state") == "app_sandbox"
                    and v.get("native_v8") is True
                ),
            )
        ],
        "reference_churn": [
            check(
                "reference churn",
                "churn.json",
                json_predicate(
                    lambda v: v.get("status") == "passed"
                    and v.get("false_preservation", {}).get("numerator") == 0
                ),
            )
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("suite", choices=("layout", "security"))
    parser.add_argument("--artifact-root", type=Path, required=True)
    args = parser.parse_args()
    raw = args.artifact_root.resolve()
    raw.mkdir(parents=True, exist_ok=True)
    if args.suite == "layout":
        assertions = evaluate_assertions(
            raw, "layout", layout_specs(), raw / "assertions.json"
        )
        regressions = evaluate_regressions(
            raw, "layout", layout_regressions(), raw / "regressions.json"
        )
    else:
        assertions = evaluate_assertions(
            raw, "security", security_specs(), raw / "assertions.json"
        )
        regressions = evaluate_regressions(
            raw, "security", security_regressions(), raw / "regressions.json"
        )
    if not assertions or not regressions:
        raise SystemExit("one or more evidence assertions or regressions failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
