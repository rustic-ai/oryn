"""Map retained selected execution IDs to pinned upstream URLs, never hooks.

Preparation/reconciliation only. Execution remains oryn-validate.py wpt-dev,
the upstream executor and authenticated WebDriver native actions.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def leaves(tree, prefix=""):
    for name, value in tree.items():
        path = prefix + name
        if isinstance(value, dict):
            yield from leaves(value, path + "/")
        else:
            yield path, value


def execution_url(path, global_name, variant):
    validate_source_path(path)
    if variant and (not variant.startswith("?") or "#" in variant or "\n" in variant or "\r" in variant):
        raise ValueError("unsafe inventory variant")
    if path.endswith(".any.js"):
        suffix = {"window": ".any.html", "dedicatedworker": ".any.worker.html",
                  "sharedworker": ".any.sharedworker.html", "serviceworker": ".any.serviceworker.html"}.get(global_name)
        if suffix is None:
            raise ValueError("unsupported inventory global: " + global_name)
        path = path[:-len(".any.js")] + suffix
    elif path.endswith(".window.js") and global_name == "window":
        path = path[:-len(".js")] + ".html"
    elif global_name != "window":
        raise ValueError("no exact upstream wrapper for " + path + "|" + global_name)
    return "/" + path + variant


def validate_source_path(path):
    if not isinstance(path, str) or not path or path.startswith("/") or "\\" in path or any(part in ("", ".", "..") for part in path.split("/")) or any(ch in path for ch in "?#\r\n\0"):
        raise ValueError("unsafe inventory source path")


def prepare(inventory_path, manifest_path, checkout, ledger_path=None):
    inventory = json.loads(Path(inventory_path).read_text())
    manifest = json.loads(Path(manifest_path).read_text())
    if manifest.get("version") != 9:
        raise ValueError("unexpected pinned manifest schema")
    indexed = {path: (kind,leaf) for kind,tree in manifest["items"].items() for path,leaf in leaves(tree)}
    rows, ids, urls = [], set(), set()
    for source in inventory["tests"]:
        path = source["path"]
        validate_source_path(path)
        root = Path(checkout).resolve()
        source_path = (root / path).resolve()
        if not source_path.is_relative_to(root):
            raise ValueError("inventory source escapes checkout")
        data = source_path.read_bytes()
        if hashlib.sha256(data).hexdigest() != source["sha256"]:
            raise ValueError("inventory source differs: " + path)
        # Retained export uses WebIDL/, upstream pinned URL/source key webidl/.
        # Verify identical Git blob before accepting this explicit alias.
        manifest_path_key = "webidl/"+path[len("WebIDL/"):] if path.startswith("WebIDL/") else path
        found = indexed.get(manifest_path_key)
        kind,leaf = found if found else (None,None)
        blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        source_matches = leaf is not None and leaf[0] == blob
        if leaf is None: leaf=[]
        manifest_urls = {"/" + (entry[0] or manifest_path_key).lstrip("/") for entry in leaf[1:] if entry and isinstance(entry[0],(str,type(None)))} if kind=="testharness" else set()
        for global_name in source["globals"]:
            for variant in source["variants"]:
                identifier = f"{path}|{global_name}|{variant}"
                url = execution_url(manifest_path_key, global_name, variant)
                if identifier in ids or url in urls:
                    raise ValueError("ambiguous or duplicate execution mapping: " + identifier)
                ids.add(identifier); urls.add(url)
                rows.append({"id": identifier, "url": url, "path": path,
                             "global": global_name, "variant": variant,
                             "test_type":kind,
                             "runnable":source_matches and kind=="testharness" and url in manifest_urls,
                             "mapping_error":("pinned_manifest_source_missing_or_changed" if not source_matches else None if kind=="testharness" and url in manifest_urls else "required_id_not_upstream_testharness")})
    if len(rows) != inventory["required_execution_total"]:
        raise ValueError("required execution total changed")
    if ledger_path:
        ledger = json.loads(Path(ledger_path).read_text())
        ledger_ids = [row["id"] for row in ledger["assertions"]]
        if len(ledger_ids) != len(set(ledger_ids)) or set(ledger_ids) != ids:
            raise ValueError("ledger population does not match exact mapped IDs")
    return {"schema_version": 1, "executor": "upstream-wptrunner",
            "inventory_sha256": digest(inventory_path), "manifest_sha256": digest(manifest_path),
            "upstream_revision": inventory["upstream_revision"], "execution_total": len(rows),
            "runnable_total":sum(row["runnable"] for row in rows), "executions": rows}


def reconcile(mapping, report, report_sha256=None):
    expected = {}
    ids = set()
    runnable = set()
    for row in mapping["executions"]:
        if row["url"] in expected or row["id"] in ids:
            raise ValueError("duplicate mapping URL or ID")
        expected[row["url"]] = row["id"]
        ids.add(row["id"])
        if row.get("runnable", True):
            runnable.add(row["url"])
    run_info = report.get("run_info", {})
    if not isinstance(run_info, dict):
        raise ValueError("invalid report run_info")
    revision = run_info.get("oryn_wpt_revision")
    if revision is not None and revision != mapping.get("upstream_revision"):
        raise ValueError("report pinned revision differs from mapping")
    executor = run_info.get("oryn_executor")
    if executor is not None and executor != "upstream-wptrunner":
        raise ValueError("report executor is not upstream wptrunner")
    observed = {}
    for row in report.get("results", []):
        url = row["test"]
        if url not in expected or url in observed:
            raise ValueError("extra or duplicate upstream execution: " + url)
        if url not in runnable:
            raise ValueError("non-testharness required ID cannot be observed by this mapper: " + url)
        observed[url] = dict(row, selected_id=expected[url])
    complete = len(observed) == len(expected)
    no_assertions = [row["selected_id"] for row in observed.values() if not row.get("subtests")]
    selected_passed = bool(observed) and not no_assertions and all(row.get("status") in ("OK", "PASS") and all(test.get("status") == "PASS" for test in row["subtests"]) for row in observed.values())
    return {"results": list(observed.values()),
            "missing_ids": [identifier for url, identifier in expected.items() if url not in observed],
            "complete": complete,
            "url_presence_complete": complete,
            "no_assertions_ids": no_assertions,
            "selected_results_passed": selected_passed,
            "qualification": "not_evaluated",
            "scope": "URL mapping only; assertion population, signatures and product gates require coordinator validation",
            "run_info": run_info,
            "report_sha256": report_sha256,
            "mapping_inventory_sha256": mapping.get("inventory_sha256"),
            "mapping_manifest_sha256": mapping.get("manifest_sha256"),
            "required_dispositions": [row for row in mapping["executions"] if not row.get("runnable", True)]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--ledger", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    mapping = prepare(args.inventory, args.manifest, args.checkout, args.ledger)
    args.output.mkdir(exist_ok=False, parents=True)
    (args.output / "selected-map.json").write_text(json.dumps(mapping, indent=2)+"\n")
    (args.output / "selected.include").write_text("\n".join(row["url"] for row in mapping["executions"] if row["runnable"])+"\n")
    if args.report:
        result = reconcile(mapping, json.loads(args.report.read_text()), digest(args.report))
        (args.output / "selected-reconciliation.json").write_text(json.dumps(result, indent=2)+"\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
