# Oryn Benchmarks

- `schema/result.schema.json` preserves the per-run v1 record contract.
- `schema/result-v2.schema.json` preserves the original aggregate contract.
- `schema/result-v3.schema.json` is the fingerprinted semantic recovery semantic, causal,
  recovery, reference-churn, and 96-cell model-panel contract.
- `schema/result-v5.schema.json` corrects the proposal KPIs: failed-task
  compact-versus-full attribution, independently labelled direct/derived/
  correlated causality, and exact-identity churn. Historical schemas remain
  immutable.
- `corpus/semantic-loop.json` freezes the 30-task native-proof corpus.
- Generated measurements belong under ignored `artifacts/`.
- `evidence/semantic-loop-controlled-native.json` publishes the native 12-script harness,
  six-framework, and semantic-reference churn results. It is independently
  checked by `scripts/validate-semantic-controlled.sh`.
- `evidence/executable-document-external-validation.json` publishes the pinned WPT/html5lib and
  native-versus-Chromium supported-scope validation.
- `evidence/semantic-public-canaries.json` publishes all four read-only public canary
  outcomes, with availability kept separate from correctness.
- `evidence/semantic-public-canaries-v2.json` corrects historical native
  failures that were labelled as site challenges. It uses `not_assessed` where
  retained data cannot support retrospective detection.
- `evidence/semantic-validation-v5/` contains controlled causal/challenge
  evidence and a machine-readable attestation status. The final 96-cell result
  is intentionally absent until it can be bound to the final signed candidate.

Required harness and framework tasks block semantic recovery. MiniWoB and public canaries
establish published baselines; public availability is reported separately from
browser correctness. Public canaries are read-only and must not authenticate,
message, upload, purchase, or cause another irreversible action.

The semantic recovery model panel runs MiniWoB with seeds `17`, `42`, and `73` against Native
and Chromium. The hosted reference is the exact Azure deployment
`gpt-5.6-terra`; preflight rejects any other deployment before making a hosted
request.
The local reference is a pinned Qwen3 4B-class model served through
LiteLLM/Ollama; its immutable digest must be recorded in each result.
Run `intentgym/scripts/run_semantic_model_panel.py` only after exporting the Azure
variables (`AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_API_KEY`, and
`AZURE_OPENAI_DEPLOYMENT`), starting the seeded MiniWoB server and Ollama, and
confirming that Ollama serves the pinned digest. The runner enforces that digest
and includes the prior invalid-panel spend in its cumulative $100 guard before
every Azure request. The default GPT-5.6 Terra cost guard uses $2/M input and
$12/M output; deployment-specific rates can be supplied with
`ORYN_HOSTED_INPUT_COST_PER_MILLION` and
`ORYN_HOSTED_OUTPUT_COST_PER_MILLION`.
Local Qwen requests are bounded to 256 output tokens and request non-thinking
mode so every panel cell remains within its task timeout.

Historical result payloads and their versioned schema fields remain immutable
for auditability. Filesystem paths, executable identifiers, scripts, fixtures,
and current documentation use capability-based names.
