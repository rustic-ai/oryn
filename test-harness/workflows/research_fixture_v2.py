"""Versioned hydrated research UI backed by the immutable research SQLite oracle."""
import argparse
import hashlib
import html
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("research_backend_v1", HERE / "research_fixture.py")
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)
snapshot = legacy.snapshot
ASSETS = {"research-client.mjs", "research-search.mjs"}


class Fixture(legacy.Fixture):
    def get(self, case_id, path, query):
        with self.lock:
            if path in ASSETS:
                with self.db:
                    if not self.db.execute("SELECT 1 FROM cases WHERE case_id=?", (case_id,)).fetchone():
                        return 404, "Unknown case", []
                    body = (HERE / "v2" / path).read_text()
                    self.db.execute("INSERT INTO visits VALUES(?,?,?,?,?)", (
                        case_id, path, json.dumps(query, sort_keys=True), 200,
                        hashlib.sha256(body.encode()).hexdigest()))
                return 200, body, [("Content-Type", "text/javascript; charset=utf-8")]
            status, body, headers = super().get(case_id, path, query)
            if status != 200 or path not in ("start", "search"):
                return status, body, headers
            base = f"/research/{case_id}"
            term = query.get("q", ["energy"])[0]
            page = query.get("page", ["1"])[0]
            results = "" if path == "start" else self.results(base, term, page)
            body = self.page(
                f'<section id="research-app" data-base="{base}" data-state="hydrating">'
                f'<form id="search-form" action="{base}/search">'
                f'<label>Search sources<input name="q" value="{html.escape(term, quote=True)}" required></label>'
                '<button>Search reports</button></form>'
                f'<div id="results">{results}</div>'
                '<p id="search-status" role="status"></p></section>'
                f'<script type="module" src="{base}/research-client.mjs"></script>')
            # Replace the just-recorded representation hash while the same fixture
            # lock is held; the independent oracle describes the delivered bytes.
            with self.db:
                self.db.execute("UPDATE visits SET body_sha256=? WHERE rowid=(SELECT MAX(rowid) FROM visits WHERE case_id=?)",
                                (hashlib.sha256(body.encode()).hexdigest(), case_id))
            return status, body, headers


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    legacy.serve(parser.parse_args().database, fixture_type=Fixture,
                 extra_paths=tuple(ASSETS))
