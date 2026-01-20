# Oryn Python Client

Python client library for [Oryn](https://github.com/dragonscale/oryn) browser automation via Intent Language pass-through.

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

The Python client is a **thin pass-through layer** that sends Intent Language commands directly to the oryn backend. Commands are sent as strings, exactly as you would type them in the oryn REPL or write in `.oil` scripts.

### Async Usage

```python
from oryn import OrynClient

async with OrynClient(mode="headless") as client:
    # All commands are Intent Language strings
    await client.execute('goto "https://example.com"')

    obs = await client.observe()
    print(f"Found {len(obs.elements)} elements")

    await client.execute('click "Sign in"')
    await client.execute('type email "user@example.com"')
```

### Sync Usage

```python
from oryn import OrynClientSync

with OrynClientSync(mode="headless") as client:
    client.execute('goto "https://example.com"')
    obs = client.observe()
    client.execute('click "Sign in"')
```

### Running .oil Scripts

You can run existing `.oil` scripts directly:

```python
from oryn import OrynClientSync, run_oil_file_sync

with OrynClientSync(mode="headless") as client:
    results = run_oil_file_sync(client, "path/to/script.oil")

    for cmd, result in results:
        print(f"{cmd}: {'OK' if result.success else 'FAIL'}")
```

## API Reference

### OrynClient / OrynClientSync

| Method | Description |
|--------|-------------|
| `execute(command)` | Execute an Intent Language command string |
| `observe(**options)` | Execute `observe` and return structured `OrynObservation` |

### run_oil_file_sync / run_oil_file_async

Run a `.oil` script file through the client:

```python
results = run_oil_file_sync(client, "script.oil")
# Returns: List[Tuple[str, OrynResult]]
```

### OrynObservation

Structured observation returned by `observe()`:

```python
obs = client.observe()

obs.url          # Current URL
obs.title        # Page title
obs.elements     # List of Element objects
obs.patterns     # DetectedPatterns (login, search, etc.)
obs.raw          # Raw response string

# Helper methods
obs.find_by_text("Sign In")     # Find element by text
obs.find_by_role("email")       # Find elements by role
obs.get_element(5)              # Get element by ID
obs.has_login_form()            # Check for login pattern
obs.has_search_box()            # Check for search pattern
```

## Intent Language Reference

Commands are passed through exactly as written. Examples:

```python
# Navigation
client.execute('goto "https://example.com"')
client.execute('back')
client.execute('forward')
client.execute('refresh')

# Actions
client.execute('click "Sign In"')
client.execute('click 5')  # By element ID
client.execute('type email "user@example.com"')
client.execute('type "Password" "secret"')
client.execute('select "Country" "USA"')
client.execute('check "Remember me"')

# Composite commands
client.execute('login "user" "pass"')
client.execute('search "query"')
client.execute('dismiss modal')
client.execute('accept cookies')

# Waiting
client.execute('wait load')
client.execute('wait visible "Success"')

# Scrolling
client.execute('scroll down')
client.execute('scroll until "Footer"')
```

See the [Intent Language documentation](../docs/intent-language.md) for the complete reference.

## Browser Modes

| Mode | Description |
|------|-------------|
| `headless` | Chromium via CDP (default) |
| `embedded` | WPE WebKit |
| `remote` | Browser extension |

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

## License

MIT
