# Core evidence qualification

The qualification overlay retains all 13,747 Core execution IDs and their raw
results. It does **not** turn unresolved failures into passes or a smaller headline
conformance denominator. This is a dirty-tree, multi-harness diagnostic composite,
not release evidence. No browser rerun or runtime change was made for this overlay.

## Current accounting

| Qualification | Executions | Meaning |
| --- | ---: | --- |
| Qualified | 1 | Explicit source/fixture/automation audit, bound to recorded evidence |
| Review candidate | 10,036 | Terminal assertion results; fixture and automation audit still required |
| Unresolved | 3,693 | Completion, automation, or evidence consistency requires investigation |
| Environment blocked | 17 | Distinct local/public IP spaces unavailable |

Raw outcomes remain 3,861 passed, 9,869 failed, and 17 environment blocked.
Candidates include 6,176 failed and 3,860 passed executions. Candidate does not
mean browser failure confirmed, and unresolved does not mean harness fault proven.
The single audited FileReader case is a qualification-workflow seed, not a
representative sample. Do not publish a qualified percentage from it.

## Qualification contract

`scripts/qualify-wpt-evidence.py` validates the composite ledger hash, unique IDs,
every referenced raw source hash, run fingerprint, parent and worker identities,
and agreement between ledger and source outcomes. It always uses replacement
evidence when present, otherwise the original source.

Qualification requires a completed harness with status zero, nonempty terminal
pass/fail subtests, a consistent aggregate outcome, no known timeout/crash or
unsupported-automation diagnostic, and an explicit review. The review binds the
exact evidence hash, test and audited dependency source hashes, expected subtest
count, fixture rationale, and automation rationale. Declared script dependencies
must have successful JavaScript preflight evidence for HTTP and HTTPS. A stale
review fails closed. Reviews cannot override incomplete execution.

The audit manifest is `benchmarks/conformance/wpt-qualification-reviews-v1.json`.
An audit author must enumerate all relevant dependencies and inspect dynamic
resource construction, module imports, origin/topology requirements and automation.
The tool checks these declarations; it does not prove their completeness. Static
dependency discovery and a passing result alone never qualify a test. Intentional
error fixtures need an appropriate explicit audit rather than a blanket 404/500
classification. The original literal-discovery limitations remain unchanged.

The overlay writes a new ledger and summary with classifier, diagnostic helper,
review and input hashes; existing output directories are rejected. The repair
merge also now rejects plans bound to a different baseline summary hash.

## Next audit/fix priorities

1. Resolve completion/evidence questions first: HTML has 1,326 unresolved cases,
   encoding 962, CSP 334 and WebSockets 181. Preserve timeout budgets and distinguish
   engine noncompletion from collector/fixture problems before attributing failures.
2. Audit compact assertion families containing both passing and failing cases:
   DOM, Fetch/XHR, then HTML parser and navigation groups. Do not audit only passes.
   Current review-candidate counts include DOM 619, Fetch 531 and XHR 373.
3. Audit the repaired WebSocket slice (552 candidates), keeping genuine H2 fixture
   support distinct from client support. Group confirmed failures by shared root
   cause only after checking each group's fixture/automation prerequisites.
4. Keep unsupported testdriver operations, the remaining syntax-error fixtures and
   dynamic-resource cases visible until independently resolved. Do not infer browser
   incompatibility from their headline failure counts.

## Reproduction

```sh
python3 scripts/qualify-wpt-evidence.py \
  --composite artifacts/wpt-full-baseline/fixture-repair-v5-composite \
  --checkout /Users/rohit/Work/oryn2/oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0 \
  --reviews benchmarks/conformance/wpt-qualification-reviews-v1.json \
  --output artifacts/wpt-full-baseline/qualification-v1
```

Choose a new output directory for subsequent runs. Generated evidence is in
`artifacts/wpt-full-baseline/qualification-v1/{summary.json,ledger.jsonl}`.

Tests cover pass/fail symmetry, unreviewed passes, noncompletion, timeout/crash and
automation blockers, empty/nonterminal subtests, outcome/count mismatches,
environment separation, stale source/review evidence and missing dependency proof.

Validation: 62 WPT-related tests ran, 60 passed and two Node-dependent tests were
skipped. This includes live HTTP/HTTPS, WebSocket/WSS and HTTP/2 fixture checks.
`git diff --check` passed. Rust tests were not rerun because no Rust or runtime
code changed.
