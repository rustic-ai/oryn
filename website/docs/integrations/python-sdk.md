# Python SDK (`oryn-python`)

Use the Python client when you want to drive Oryn from Python code while sending raw OIL commands.

## What It Is

`oryn-python` is a thin pass-through client:

- starts/connects to Oryn runtime,
- sends OIL command strings,
- returns raw responses.

## Install

From package index:

```bash
pip install oryn
```

From this repository:

```bash
cd oryn-python
pip install -e .
```

## Requirements

- Python 3.10+
- Oryn binary available in `PATH` (or explicit binary path in client config)

## Sync Example

```python
from oryn import OrynClientSync

with OrynClientSync(mode="headless") as client:
    client.execute('goto "https://example.com"')
    obs = client.execute('observe')
    print(obs)
    client.execute('click "More information..."')
```

## Async Example

```python
from oryn import OrynClient

async with OrynClient(mode="headless") as client:
    await client.execute('goto "https://example.com"')
    obs = await client.execute('observe')
    print(obs)
```

## Run `.oil` Files

```python
from oryn import OrynClientSync, run_oil_file_sync

with OrynClientSync(mode="headless") as client:
    results = run_oil_file_sync(client, "test-harness/scripts/01_static.oil")
    for cmd, result in results:
        print(cmd, result)
```

## Modes

```python
OrynClientSync(mode="headless")
OrynClientSync(mode="embedded", driver_url="http://localhost:4444")
OrynClientSync(mode="remote", port=9001)
```

## Typical Use Cases

- orchestrating Oryn from backend services,
- experiment loops and benchmark harnesses,
- agent-framework wrappers in Python.

## Related

- [Intent Commands](../reference/intent-commands.md)
- [IntentGym](intentgym.md)
