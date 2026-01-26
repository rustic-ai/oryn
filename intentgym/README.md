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
