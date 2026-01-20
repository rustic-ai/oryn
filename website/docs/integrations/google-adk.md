# Google ADK Integration

This guide demonstrates how to give Google ADK (Agent Development Kit) agents the ability to browse the web using Oryn.

## Overview

By wrapping the unified `oryn` CLI as a custom tool, your agents can perform semantic web interactions without dealing with raw HTML or complex browser automation scripts.

## Prerequisites

1. **Oryn**: Installed and available in your PATH
2. **Google ADK**: Python environment with `google-adk` or Gen AI SDK
3. **Chromium**: Installed (for headless mode)

## Step 1: Create the Oryn Tool Wrapper

Create a file named `oryn_tool.py`:

```python
import subprocess
import threading


class OrynTool:
    """Wrapper for the Oryn browser automation CLI."""

    def __init__(self, mode="headless", port=None, driver_url=None):
        self.mode = mode
        self.process = None
        self._lock = threading.Lock()

        # Build command
        cmd = ["oryn", mode]
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
            bufsize=0
        )

        # Wait for startup
        self._read_until_prompt()

    def _read_until_prompt(self) -> str:
        """Read output until the REPL prompt '>' is found."""
        output = []
        while True:
            char = self.process.stdout.read(1)
            if not char:
                break
            output.append(char)
            # Check for prompt
            if len(output) >= 2 and "".join(output[-2:]) == "> ":
                break
        return "".join(output[:-2]).strip()

    def execute(self, command: str) -> str:
        """Send an Intent Command to Oryn and return the response."""
        with self._lock:
            if not self.process or self.process.poll() is not None:
                return "Error: Browser process not running"

            # Send command
            self.process.stdin.write(command + "\n")
            self.process.stdin.flush()

            # Read response
            return self._read_until_prompt()

    def close(self):
        """Terminate the browser process."""
        if self.process:
            self.process.stdin.write("exit\n")
            self.process.stdin.flush()
            self.process.terminate()
            self.process = None


# Global browser instance
_browser = None


def get_browser() -> OrynTool:
    """Get or create the global browser instance."""
    global _browser
    if _browser is None:
        _browser = OrynTool(mode="headless")
    return _browser
```

## Step 2: Define the Agent Tool

Create the tool interface for Google ADK:

```python
from google.generativeai import tools


def browser_action(command: str) -> str:
    """
    Execute a browser action using Oryn Intent Language.

    This tool allows the agent to interact with web pages semantically.

    Args:
        command: The intent command to execute. Available commands:
            - goto <url>: Navigate to a page (e.g., 'goto google.com')
            - observe: Scan page and list interactive elements
            - click <target>: Click element by ID or text (e.g., 'click 5', 'click "Login"')
            - type <target> <text>: Type into input (e.g., 'type 1 "hello"', 'type email "user@test.com"')
            - scroll [direction] [amount]: Scroll the page
            - wait <condition>: Wait for condition (e.g., 'wait visible "Success"')
            - login <user> <pass>: Execute login workflow
            - search <query>: Execute search workflow
            - accept_cookies: Dismiss cookie consent banner
            - back/forward/refresh: History navigation

    Returns:
        The result of the browser action, including:
        - Page observations with labeled elements
        - Action confirmations
        - Error messages with recovery hints

    Examples:
        browser_action('goto github.com')
        browser_action('observe')
        browser_action('click "Sign in"')
        browser_action('type email "user@example.com"')
        browser_action('login "user@example.com" "password123"')
    """
    browser = get_browser()
    return browser.execute(command)


# Create the tool for Google ADK
oryn_tool = tools.Tool.from_function(browser_action)
```

## Step 3: Create the Agent

Configure your Google ADK agent to use the tool:

```python
import google.generativeai as genai

# Configure the API
genai.configure(api_key="YOUR_API_KEY")

# Create model with Oryn tool
model = genai.GenerativeModel(
    model_name='gemini-pro',
    tools=[oryn_tool]
)

# Start a chat session
chat = model.start_chat()

# Give the agent a task
prompt = """
You are a research agent with web browsing capabilities.

Use the browser_action tool to:
1. Go to wikipedia.org
2. Search for 'Artificial Intelligence'
3. Read the first paragraph and summarize it

Always use 'observe' after navigation to see the available elements.
"""

response = chat.send_message(prompt)
print(response.text)
```

## Step 4: Run the Agent

```bash
python your_agent_script.py
```

## Complete Example: Research Agent

```python
import google.generativeai as genai
from google.generativeai import tools
from oryn_tool import get_browser, browser_action

# Configure
genai.configure(api_key="YOUR_API_KEY")

# Create tool
oryn_tool = tools.Tool.from_function(browser_action)

# Create agent
model = genai.GenerativeModel(
    model_name='gemini-pro',
    tools=[oryn_tool],
    system_instruction="""
    You are a helpful research agent that can browse the web.

    When using the browser:
    1. Always run 'observe' after navigating to see available elements
    2. Use element IDs from the most recent observation
    3. Handle cookie banners with 'accept_cookies'
    4. Use semantic targeting when possible (e.g., click "Login" not click 5)

    Common patterns:
    - Navigation: goto <url>
    - See elements: observe
    - Click: click <target>
    - Type: type <target> <text>
    - Login: login <user> <pass>
    - Search: search <query>
    """
)

chat = model.start_chat()

# Interactive loop
while True:
    user_input = input("You: ")
    if user_input.lower() in ['quit', 'exit']:
        break

    response = chat.send_message(user_input)
    print(f"Agent: {response.text}")

# Cleanup
get_browser().close()
```

## Best Practices

### 1. Always Observe First

Instruct the agent to run `observe` after navigation:

```python
system_instruction = """
After using 'goto', always run 'observe' to see the current page elements.
Element IDs change between pages, so you must refresh your view.
"""
```

### 2. Use Semantic Targeting

Encourage text-based targeting for robustness:

```python
# Tell agent to prefer:
click "Sign in"       # More robust

# Over:
click 5              # ID might change
```

### 3. Handle Errors Gracefully

Add retry logic for common errors:

```python
def browser_action_with_retry(command: str) -> str:
    result = browser_action(command)

    if "element not found" in result.lower():
        # Refresh and retry
        browser_action("observe")
        result = browser_action(command)

    return result
```

### 4. Session Cleanup

Always close the browser when done:

```python
try:
    # Agent work here
    pass
finally:
    get_browser().close()
```

## Troubleshooting

### Agent Can't Find Elements

**Problem:** Agent uses old element IDs.

**Solution:** Remind agent to observe after navigation:

```python
response = chat.send_message("""
The element wasn't found. Please:
1. Run 'observe' to refresh the element list
2. Look at the current elements
3. Try the action again with the correct target
""")
```

### Agent Gets Stuck on Popups

**Problem:** Cookie banners or modals block interaction.

**Solution:** Add popup handling to system instructions:

```python
system_instruction = """
If you encounter cookie banners or popups, dismiss them first:
- accept_cookies: Dismiss cookie consent
- dismiss_popups: Close modal dialogs
"""
```

### Slow Response Times

**Problem:** Agent takes long to respond.

**Solution:** Use headless mode and keep the session alive:

```python
# Reuse the same browser instance
browser = OrynTool(mode="headless")

# Don't create new instances per command
```

## Example Use Cases

### Price Monitoring

```python
prompt = """
Monitor the price of a product:
1. Go to amazon.com
2. Search for 'wireless mouse'
3. Find the first product
4. Report the price
"""
```

### Form Automation

```python
prompt = """
Fill out a contact form:
1. Go to example.com/contact
2. Fill in: name="John Doe", email="john@example.com", message="Hello!"
3. Submit the form
4. Confirm success
"""
```

### Research Task

```python
prompt = """
Research a topic:
1. Go to wikipedia.org
2. Search for 'Climate Change'
3. Read the introduction section
4. Provide a summary
"""
```
