# Troubleshooting

Common issues and how to resolve them when using Oryn.

## Connection Issues

### Browser Not Found (Headless Mode)

**Error:**
```
error: Could not find Chrome executable
```

**Solution:**

1. Install Chrome/Chromium:
   ```bash
   # Ubuntu/Debian
   sudo apt install chromium-browser

   # macOS
   brew install --cask google-chrome
   ```

2. Or specify the path explicitly:
   ```bash
   oryn headless --chrome-path /path/to/chrome
   ```

3. Or set the environment variable:
   ```bash
   export CHROME_PATH=/path/to/chrome
   oryn headless
   ```

### WebDriver Connection Failed (Embedded Mode)

**Error:**
```
error: Could not connect to WebDriver at http://localhost:8080
```

**Solution:**

1. Ensure COG is running in WebDriver mode:
   ```bash
   cog --webdriver --port 8080
   ```

2. Check if port is in use:
   ```bash
   lsof -i :8080
   ```

3. Try a different port:
   ```bash
   cog --webdriver --port 9515
   oryn embedded --driver-url http://localhost:9515
   ```

### Extension Not Connected (Remote Mode)

**Error:**
```
Waiting for extension connection...
```

**Solution:**

1. Verify the extension is installed and enabled
2. Click the Oryn extension icon in your browser
3. Enter the correct server address (e.g., `localhost:9001`)
4. Check that the port matches what you started:
   ```bash
   oryn remote --port 9001
   ```

## Element Interaction Issues

### Element Not Found

**Error:**
```
error click: element [99] not found

# hint
Available elements: 1-8. Run 'observe' to refresh.
```

**Causes:**
- Element IDs changed after navigation or DOM update
- The element doesn't exist on the page

**Solution:**

1. Re-scan the page:
   ```
   observe
   ```

2. Check if the element exists:
   ```
   observe --full
   ```

3. Use text or role targeting instead of ID:
   ```
   click "Sign in"    # Instead of click 5
   ```

### Element Stale

**Error:**
```
error click: element [5] is stale (removed from DOM)
```

**Causes:**
- Page content was updated via JavaScript
- Element was replaced with a new element

**Solution:**

1. Re-scan before interacting:
   ```
   observe
   click 5    # Now with fresh ID
   ```

2. Wait for the page to stabilize:
   ```
   wait idle
   observe
   ```

### Element Not Visible

**Error:**
```
error click: element [5] is not visible
```

**Causes:**
- Element is hidden (CSS display:none or visibility:hidden)
- Element is outside the viewport

**Solution:**

1. Scroll the element into view:
   ```
   scroll to 5
   click 5
   ```

2. Wait for visibility:
   ```
   wait visible 5
   click 5
   ```

3. Use force click (not recommended):
   ```
   click 5 --force
   ```

### Element Disabled

**Error:**
```
error click: element [5] is disabled
```

**Causes:**
- Button/input is disabled until a condition is met
- Form validation hasn't passed

**Solution:**

1. Complete required fields first
2. Wait for element to be enabled:
   ```
   wait enabled 5
   click 5
   ```

### Element Covered

**Error:**
```
error click: element [5] is covered by another element
```

**Causes:**
- Modal/overlay is covering the element
- Cookie banner is in the way

**Solution:**

1. Dismiss popups first:
   ```
   dismiss_popups
   accept_cookies
   click 5
   ```

2. Use force click:
   ```
   click 5 --force
   ```

## Navigation Issues

### Timeout Waiting for Page

**Error:**
```
error goto: timeout waiting for page load
```

**Causes:**
- Slow network connection
- Page takes long to load
- Redirect loop

**Solution:**

1. Increase timeout:
   ```
   goto example.com --timeout 60s
   ```

2. Check network connectivity
3. Try accessing the URL directly in a browser

### Unexpected Redirect

**Error:**
```
error: navigated to unexpected URL
expected: example.com/dashboard
actual: example.com/login
```

**Causes:**
- Authentication required
- Session expired

**Solution:**

1. Check if login is required
2. Handle the redirect:
   ```
   goto example.com/dashboard
   observe    # See where you actually landed
   ```

## Intent Issues

### Intent Not Found

**Error:**
```
error: unknown intent 'my_custom_intent'
```

**Solution:**

1. Check if intent is defined:
   ```
   intents
   ```

2. Check spelling and case

3. For file-based intents, verify the file exists:
   ```bash
   ls ~/.oryn/intents/
   ```

### Intent Parameters Missing

**Error:**
```
error login: required parameter 'username' not provided
```

**Solution:**

Provide all required parameters:
```
login "user@example.com" "password"
```

### Pattern Not Detected

**Error:**
```
error login: login_form pattern not detected
```

**Causes:**
- Page structure doesn't match expected pattern
- Not on a login page

**Solution:**

1. Observe the page to see what's available:
   ```
   observe --full
   ```

2. Use fallback targeting:
   ```
   type email "user@example.com"
   type password "secret"
   click submit
   ```

## Timeout Issues

### Command Timeout

**Error:**
```
error: command timed out after 30s
```

**Solution:**

1. Increase timeout:
   ```
   click 5 --timeout 60s
   wait visible "Success" --timeout 120s
   ```

2. Configure default timeout:
   ```yaml
   # ~/.oryn/config.yaml
   intent_engine:
     default_timeout: 60s
   ```

### Wait Condition Never Met

**Error:**
```
error wait: condition not met within 30s
```

**Causes:**
- Element never appeared
- Wrong selector/text
- Page state doesn't change

**Solution:**

1. Verify the expected state:
   ```
   observe
   ```

2. Check your condition:
   ```
   wait visible "Success"    # Is "Success" the exact text?
   ```

3. Increase timeout or check if condition is correct:
   ```
   wait visible "success" --timeout 60s
   ```

## Performance Issues

### Slow Observations

**Causes:**
- Too many elements on page
- Complex page structure

**Solution:**

1. Limit scan scope:
   ```
   observe --viewport-only
   observe --near "Login"
   ```

2. Use max elements:
   ```yaml
   # Default is 200
   scan:
     max_elements: 100
   ```

### High Memory Usage

**Causes:**
- Running headless mode with many tabs
- Memory leaks in long sessions

**Solution:**

1. Use embedded mode for lower memory (~50MB)
2. Restart Oryn periodically
3. Close unused tabs:
   ```
   tabs
   tab close 2
   ```

## Debug Mode

### Enable Verbose Logging

```bash
RUST_LOG=debug oryn headless
```

### Logging Levels

| Level | Description |
|-------|-------------|
| `error` | Only errors |
| `warn` | Warnings and errors |
| `info` | General information |
| `debug` | Detailed debugging |
| `trace` | Very verbose |

### Log to File

```bash
RUST_LOG=debug oryn headless 2>&1 | tee oryn.log
```

## Getting Help

### Check Documentation

- [Intent Language Reference](../reference/intent-commands.md)
- [Scanner Protocol](../concepts/scanner-protocol.md)
- [Error Codes](../reference/error-codes.md)

### Report Issues

If you've found a bug:

1. Reproduce with debug logging enabled
2. Capture the error message and context
3. Report at: https://github.com/dragonscale/oryn/issues
