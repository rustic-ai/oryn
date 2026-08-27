# Oryn Benchmarks

- `schema/result.schema.json` preserves the per-run v1 record contract.
- `schema/result-v2.schema.json` preserves the original aggregate contract.
- `schema/result-v3.schema.json` is the fingerprinted G2R semantic, causal,
  recovery, reference-churn, and 96-cell model-panel contract.
- `corpus/g2.json` freezes the 30-task native-proof corpus.
- Generated measurements belong under ignored `artifacts/`.
- `evidence/g2-controlled-native.json` publishes the native 12-script harness,
  six-framework, and semantic-reference churn results. It is independently
  checked by `scripts/validate-g2-controlled.sh`.
- `evidence/g1-external-validation.json` publishes the pinned WPT/html5lib and
  native-versus-Chromium supported-scope validation.
- `evidence/g2-public-canaries.json` publishes all four read-only public canary
  outcomes, with availability kept separate from correctness.

Required harness and framework tasks block G2R. MiniWoB and public canaries
establish published baselines; public availability is reported separately from
browser correctness. Public canaries are read-only and must not authenticate,
message, upload, purchase, or cause another irreversible action.

The G2R model panel runs MiniWoB with seeds `17`, `42`, and `73` against Native
and Chromium. The hosted reference is the exact Azure deployment
`gpt-5.6-terra`; preflight rejects any other deployment before making a hosted
request.
The local reference is a pinned Qwen3 4B-class model served through
LiteLLM/Ollama; its immutable digest must be recorded in each result.
Run `intentgym/scripts/run_g2_model_panel.py` only after exporting the Azure
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
