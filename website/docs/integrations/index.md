# Integrations

Guides for integrating Oryn with AI agent frameworks and other tools.

## Overview

Oryn is designed to be integrated into AI agent systems. The CLI provides a simple interface that agents can control via standard input/output.

<div class="grid cards" markdown>

-   :material-google:{ .lg .middle } **Google ADK**

    ---

    Use Oryn with Google ADK (Agent Development Kit) agents.

    [:octicons-arrow-right-24: Google ADK Integration](google-adk.md)

-   :material-chart-line:{ .lg .middle } **IntentGym**

    ---

    Run reproducible Oryn-based benchmark evaluations for web agents.

    [:octicons-arrow-right-24: IntentGym Guide](intentgym.md)

-   :material-language-python:{ .lg .middle } **Python SDK**

    ---

    Drive Oryn from Python with sync/async pass-through clients.

    [:octicons-arrow-right-24: Python SDK Guide](python-sdk.md)

-   :material-connection:{ .lg .middle } **Remote Extension**

    ---

    Connect `oryn remote` to the browser extension in `extension/`.

    [:octicons-arrow-right-24: Remote Extension Guide](remote-extension.md)

-   :material-language-rust:{ .lg .middle } **WASM Extension**

    ---

    Build and run the standalone `extension-w/` browser extension.

    [:octicons-arrow-right-24: WASM Extension Guide](wasm-extension.md)

</div>

## Integration Approaches

### CLI Wrapper

The simplest approach is to wrap the `oryn` CLI as a subprocess:

```python
import subprocess

class OrynBrowser:
    def __init__(self, mode="headless"):
        self.process = subprocess.Popen(
            ["oryn", mode],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True
        )

    def execute(self, command: str) -> str:
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()
        return self._read_response()
```

### Tool Definition

For agent frameworks that support tools/functions:

```python
def browser_action(command: str) -> str:
    """
    Execute a browser action using Oryn Intent Language.

    Args:
        command: Intent command (e.g., 'goto google.com', 'click "Login"')

    Returns:
        Result of the action
    """
    return browser.execute(command)
```

## Supported Frameworks

| Framework | Status | Documentation |
|-----------|--------|---------------|
| IntentGym | Supported | [Guide](intentgym.md) |
| Python SDK (`oryn-python`) | Supported | [Guide](python-sdk.md) |
| Remote Extension (`extension`) | Supported | [Guide](remote-extension.md) |
| WASM Extension (`extension-w`) | Supported | [Guide](wasm-extension.md) |
| Google ADK | Supported | [Guide](google-adk.md) |
| LangChain | Planned | Coming soon |
| AutoGPT | Planned | Coming soon |
| CrewAI | Planned | Coming soon |

## Best Practices

### Tool Descriptions

Provide clear tool descriptions for the agent:

```python
TOOL_DESCRIPTION = """
Execute browser actions using Oryn Intent Language.

Available commands:
- goto <url>: Navigate to a page
- observe: List interactive elements
- click <target>: Click element by ID or text
- type <target> <text>: Type into input
- login <user> <pass>: Execute login intent
- search <query>: Execute search intent

Examples:
- goto google.com
- click "Sign in"
- type email "user@test.com"
- login "user@test.com" "password"
"""
```

### Session Management

Keep a single browser session per task:

```python
# Good: Reuse session
browser = OrynBrowser()
browser.execute("goto site1.com")
browser.execute("goto site2.com")

# Avoid: New session per command
```

### Error Handling

Parse Oryn responses to detect errors:

```python
def execute_with_retry(command: str, max_retries: int = 3) -> str:
    for attempt in range(max_retries):
        result = browser.execute(command)
        if not result.startswith("error"):
            return result
        if "element not found" in result:
            browser.execute("observe")  # Refresh element IDs
    return result
```
