# IntentGym User Guide

**IntentGym** is a benchmark harness for evaluating AI web agents. It provides a standardized interface for agents to interact with web environments (via Oryn) and measures their performance across various tasks.

## Installation

Prerequisites:
- Python 3.11+
- Poetry (recommended) or pip

```bash
git clone https://github.com/dragonscale/intentgym.git
cd intentgym
poetry install
```

## Quick Start

1. **Configure API Keys**: Ensure your LLM provider keys are set in your environment (e.g., `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`).

2. **Run a Mock Benchmark**:
   ```bash
   poetry run intentgym run --config configs/test_run.yaml
   ```

## Configuration

Runs are defined by YAML configuration files.

### Example Config (`configs/example.yaml`)

```yaml
run_id: my_experiment_v1
benchmark:
  name: miniwob
  options:
    subtasks: ["click-button", "login-user"]
    server_url: "http://localhost:8765"

llm:
  provider: openai
  model: gpt-4-turbo
  options:
    temperature: 0.0

agent:
  type: react
  options:
    max_steps: 10

prompt_template: verbose_cot
oryn_mode: headless
```

### Supported Agents
- `react`: Standard Reason+Act loop.
- `plan_act`: Generates a full plan, then executes it.
- `reflexion`: Retries failed tasks with verbal reinforcement learning.
- `ralph`: RAG-based agent using past successful trajectories.

### Supported Benchmarks
- **MiniWoB++**: Small web interaction tasks. Requires running MiniWoB server.
- **WebShop**: E-commerce shopping tasks.
- **WebArena**: Realistic web tasks (requires local setup).
- **Mock**: Simple test environment.

## CLI Commands

### `run`
Execute a benchmark run.
```bash
intentgym run --config <path_to_config> [--subset <task_class>]
```

### `inspect`
View detailed metrics for a specific run ID.
```bash
intentgym inspect <run_id>
```

### `compare`
Compare metrics across multiple runs.
```bash
intentgym compare run_v1 run_v2
```

### `download`
Download benchmark datasets.
```bash
intentgym download webarena
```

## Results
Results are saved to `results/<run_id>.json`. This file contains:
- **Summary**: Aggregate success rates, costs, and latency.
- **Task Traces**: Full history of every step, thought, and action.
- **Metrics**: Token usage breakdown per turnaround.
