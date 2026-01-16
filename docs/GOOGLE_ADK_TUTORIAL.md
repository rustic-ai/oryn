# Integrating Lemmascope with Google ADK

This tutorial demonstrates how to give **Google ADK Agents** the ability to browse the web using **Lemmascope**.

By wrapping the unified `lscope` CLI as a custom tool, your agents can perform semantic web interactions (navigation, reading, clicking) without dealing with raw HTML or complex browser automation scripts.

## Prerequisites

1.  **Lemmascope**: Installed and available in your PATH as `lscope`.
    - Follow the [User Guide](USER_GUIDE.md) to build and install.
2.  **Google ADK**: Python environment set up with `google-adk` (or equivalent Gen AI SDK).
3.  **Headless Browser**: Chromium installed (for `lscope headless`).

## Concept

We will create a **Python Tool** that wraps the `lscope` binary. The agent will send natural language Intent Commands (e.g., `goto google.com`, `click "Search"`) to this tool, and the tool will return the semantic response from Lemmascope.

## Step 1: Create the Lemmascope Tool Wrapper

Create a file named `lemmascope_tool.py`. This class manages a persistent subprocess for `lscope`.

```python
import subprocess
import time
import threading

class LemmascopeTool:
    def __init__(self, mode="headless", port=None, driver_url=None):
        self.mode = mode
        self.process = None
        
        # Build command
        cmd = ["lscope", mode]
        if mode == "embedded" and driver_url:
            cmd.extend(["--driver-url", driver_url])
        elif mode == "remote" and port:
            cmd.extend(["--port", str(port)])
            
        # Start persistent process
        self.process = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=0 # Unbuffered
        )
        
        # Consume initial startup logs
        self._read_until_prompt()

    def _read_until_prompt(self):
        """Reads output until the interaction prompt '>' is found."""
        output = []
        while True:
            char = self.process.stdout.read(1)
            if not char:
                break
            output.append(char)
            if "".join(output[-2:]) == "> ":
                break
        return "".join(output[:-2]).strip()

    def execute(self, command: str) -> str:
        """Sends an Intent Command to Lemmascope and returns the response."""
        if not self.process:
            return "Error: Browser not running."
            
        # Send command
        self.process.stdin.write(command + "\n")
        
        # Read response until next prompt
        return self._read_until_prompt()

    def close(self):
        if self.process:
            self.process.terminate()
```

## Step 2: Define the Agent Tool Interface

Now, expose this class as a function or tool definition that your ADK agent can call.

```python
from google.generativeai import tools

# Initialize the global browser instance
browser = LemmascopeTool(mode="headless")

def browser_action(command: str) -> str:
    """
    Executes a browser action using Lemmascope Intent Language.
    
    Args:
        command: The intent command (e.g., 'goto google.com', 'scan', 'click "Login"').
        
    Returns:
        The result of the action (semantic observation of the page).
    """
    return browser.execute(command)

# Create the tool definition
lemmascope_tool = tools.Tool.from_function(browser_action)
```

## Step 3: Create the Agent

Configure your Google ADK agent to use the tool.

```python
import google.generativeai as genai

model = genai.GenerativeModel(
    model_name='gemini-pro',
    tools=[lemmascope_tool]
)

chat = model.start_chat()

# Task: Research
prompt = """
You are a research agent. Use the browser_action tool to find information.
Go to 'wikipedia.org', search for 'Lemmascope', and summarize the first paragraph if found.
Start by navigating to the page.
"""

response = chat.send_message(prompt)
print(response.text)
```

## Supported Commands

The agent can now use the full Lemmascope Intent Language:

*   **Navigation**: `goto <url>`
*   **Observation**: `scan` (Returns list of interactive elements)
*   **Interaction**: `click <id>`, `type <id> "text"`, `scroll`
*   **Waiting**: `wait visible "Result"`

## Best Practices for Agents

1.  **Always Scan First**: Before clicking or typing, the agent should issue `scan` (or `observe`) to get the current IDs of elements. IDs are dynamic and change after navigation.
2.  **Use Semantic Targets**: "click 'Submit'" is robust, but "click 42" requires a fresh scan. Encourage the agent to rely on the IDs returned by the most recent scan.
3.  **Handle Errors**: If Lemmascope returns `ELEMENT_NOT_FOUND`, the agent should retry with a new `scan` to refresh its view of the page.
