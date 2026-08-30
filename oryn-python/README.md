# Oryn Python Client

Python client library for [Oryn](https://github.com/dragonscale/oryn) browser automation through the shared typed agent contract or the compatible Intent Language pass-through.

## Installation

```bash
pip install oryn
```

Or install from source:

```bash
cd oryn-python
pip install -e .
```

## Requirements

- Python 3.10+
- Oryn binary installed and in PATH (or set `ORYN_BINARY` env var)

## Quick Start

The Python client exposes the same versioned `Intent`/`Target` contract as the
Rust API and MCP server. Existing OIL strings and `.oil` scripts remain
supported through `execute()`.

### Async Usage

```python
from oryn import OrynClient, OrynInput, OrynIntent, OrynTarget

async with OrynClient() as client:
    await client.perform(OrynIntent.navigate("https://example.com"))
    observed = await client.perform(OrynIntent.observe(include_structures=True))
    print(observed.semantic_observation.structures)

    await client.perform(OrynIntent.click(OrynTarget.text("Sign in")))
    await client.perform(
        OrynIntent.type(
            OrynTarget.role("textbox"),
            OrynInput.text("user@example.com"),
        )
    )
```

Secret input uses a parent-owned label and never places the secret value in the
intent, trace, observation, Python result, argv, environment, or MCP request.
Standalone clients provision values through an anonymous inherited descriptor:

```python
async with OrynClient(secret_values={
    "https://example.com": {"account-password": "value stays in parent memory"}
}) as client:
    await client.perform(
        OrynIntent.type(
            OrynTarget.role("textbox"),
            OrynInput.secret("account-password"),
        )
    )
```

Native upload paths are imported by the parent and selected by label:

```python
async with OrynClient(upload_paths={"proof": "/private/path/proof.txt"}) as client:
    await client.perform(OrynIntent.upload(OrynTarget.css("input[type=file]"), ["proof"]))
```

### Sync Usage

```python
from oryn import OrynClientSync

with OrynClientSync() as client:
    client.execute('goto "https://example.com"')
    obs = client.execute('observe')
    print(obs)
    client.execute('click "Sign in"')
```

### Running .oil Scripts

You can run existing `.oil` scripts directly:

```python
from oryn import OrynClientSync, run_oil_file_sync

with OrynClientSync() as client:
    results = run_oil_file_sync(client, "path/to/script.oil")

    for cmd, result in results:
        print(f"{cmd}: {result}")
```

## API Reference

### OrynClient / OrynClientSync

| Method             | Description                                                                    |
| ------------------ | ------------------------------------------------------------------------------ |
| `perform(intent)`  | Execute the shared typed agent contract and return an `OrynIntentResult`.       |
| `execute(command)` | Execute an Intent Language command string and return the raw string response.  |

### run_oil_file_sync / run_oil_file_async

Run a `.oil` script file through the client:

```python
results = run_oil_file_sync(client, "script.oil")
# Returns: List[Tuple[str, str]] (command, response)
```

See the [Intent Language documentation](../docs/SPEC-INTENT-LANGUAGE.md) for the complete reference.

## Browser Modes

| Mode       | Description                               |
| ---------- | ----------------------------------------- |
| `native`   | Sandboxed Oryn native runtime (default)   |
| `headless` | Chromium via CDP compatibility domain     |
| `embedded` | WPE WebKit                                |
| `remote`   | Browser extension                         |

```python
OrynClient(mode="headless")
OrynClient(mode="remote", port=9001)
OrynClient(mode="embedded", driver_url="http://localhost:4444")
```

## Running Tests

### Unit Tests

```bash
pytest tests/test_*.py -v
```

### E2E Tests (runs .oil scripts)

Requires oryn binary and test harness running:

```bash
# Start test harness
cd test-harness && npm start &

# Run E2E tests
pytest tests/e2e/ -v -m oil
```

### Docker E2E

```bash
./scripts/run-python-e2e-tests.sh --quick  # oryn-h only
./scripts/run-python-e2e-tests.sh          # All variants
```

## CI Note

Repository-level GitHub workflows currently focus Rust, JS, E2E harness, preview packaging, and docs publishing under `.github/workflows/`.
For Python client validation, run the test commands above locally (or in your own Python-specific CI job).

## License

MIT
