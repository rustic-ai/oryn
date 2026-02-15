# IntentGym

Benchmark harness for evaluating AI web agents using Oryn.

## Development Rules

**Strict Dependency Management**:
This project strictly uses **Poetry** for dependency management and execution.
-   **DO NOT** use `pip install`.
-   **ALWAYS** use `poetry add <package>` to install dependencies.
-   **ALWAYS** use `poetry run <command>` to execute scripts (e.g., `poetry run intentgym run ...`).

## Setup

1.  Install dependencies:
    ```bash
    poetry install
    ```

2.  Run a benchmark:
    ```bash
    poetry run intentgym run --config configs/miniwob_litellm.yaml
    ```

## MiniWoB Stability Notes

- Default MiniWoB configs run in headless mode without `--visible` for long-run stability.
- Use `configs/miniwob_100ep_visible_debug.yaml` only when you need a headed browser for local debugging.

## CI Context

- IntentGym runs are not part of required PR checks in `.github/workflows/`.
- Use local Poetry commands for benchmark validation.
- Core backend smoke coverage is handled separately by quick/nightly E2E workflows.
