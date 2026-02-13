# IntentGym

IntentGym is Oryn's benchmark harness for evaluating web agents across task suites like MiniWoB.

## Overview

IntentGym runs benchmark episodes by combining:

- an Oryn browser backend (`headless`, `embedded`, or `remote`),
- an LLM provider (`openai`, `anthropic`, `litellm`, `mock`),
- an agent strategy (`react`, `plan_act`, `reflexion`, `ralph`, `swarm`, `adk`),
- a benchmark loader (`miniwob`, `webshop`, `webarena`, `mock`).

## Prerequisites

1. Build the Oryn CLI binary:

```bash
cargo build --release -p oryn
```

2. Install IntentGym dependencies with Poetry:

```bash
cd intentgym
poetry install
```

3. Set credentials for your chosen LLM provider (for example `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`).

!!! note
    IntentGym is managed with Poetry in this repository.  
    Use `poetry run ...` for commands.

## MiniWoB Quick Start

Start the MiniWoB server in one terminal:

```bash
cd intentgym
./scripts/run_miniwob.sh
```

Run a benchmark in a second terminal:

```bash
cd intentgym
poetry run intentgym run --config configs/miniwob_litellm.yaml
```

Run only one task:

```bash
poetry run intentgym run --config configs/miniwob_litellm.yaml --subset click-button
```

Run multiple tasks:

```bash
poetry run intentgym run --config configs/miniwob_litellm.yaml --subset click-button,enter-text
```

## Config Structure

IntentGym config files use a flat schema like this:

```yaml
run_id: miniwob_litellm
seed: 42
benchmark: miniwob
benchmark_options:
  server_url: http://localhost:8765
  episodes_per_task: 5
llm_provider: litellm
llm_model: vertex_ai/gemini-2.5-flash-lite
llm_options:
  temperature: 1.0
agent_type: react
agent_options: {}
prompt_template: oil_instructions_v2
oryn_mode: headless
oryn_options:
  binary_path: ../target/release/oryn
  timeout: 60.0
  log_file: oryn_debug.log
  cli_args: ["--visible"]
max_steps: 50
timeout_seconds: 300
save_transcript: true
```

Working examples live in `intentgym/configs/`.

## CLI Commands

Run benchmark:

```bash
poetry run intentgym run --config configs/miniwob_5ep.yaml
```

Run with browser log redirection:

```bash
poetry run intentgym run --config configs/miniwob_5ep.yaml --oryn-log-file logs/oryn.log
```

Override Oryn options from CLI (repeatable):

```bash
poetry run intentgym run --config configs/miniwob_5ep.yaml --oryn-opt timeout=90 --oryn-opt log_file=oryn_debug.log
```

Inspect a run:

```bash
poetry run intentgym inspect miniwob_5ep
```

Compare runs:

```bash
poetry run intentgym compare miniwob_5ep miniwob_100ep
```

Initialize benchmark resources:

```bash
poetry run intentgym download miniwob
poetry run intentgym download webshop
poetry run intentgym download webarena
```

## Outputs

- JSON reports: `intentgym/results/<run_id>.json`
- Markdown transcripts (when enabled): `intentgym/transcripts/`
- Optional Oryn subprocess logs: path set by `oryn_options.log_file` or `--oryn-log-file`

## Notes

- `configs/miniwob_100ep_visible_debug.yaml` enables a visible browser and is intended for local debugging.
- Long MiniWoB runs are usually more stable in default headless mode configs.
