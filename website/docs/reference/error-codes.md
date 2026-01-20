# Error Codes Reference

Complete reference for Oryn error codes and recovery strategies.

## Error Response Format

Errors follow this format:

```
error <command>: <message>

# hint
<recovery suggestion>
```

## Scanner Errors

### ELEMENT_NOT_FOUND

**Description:** Element ID doesn't exist in the element map.

**Causes:**
- Element IDs changed after navigation
- Element was removed from page
- Wrong ID specified

**Recovery:**
```
# Refresh element map
observe
# Then retry with correct ID
click <new_id>
```

### ELEMENT_STALE

**Description:** Element reference is no longer valid (removed from DOM).

**Causes:**
- Page content was updated by JavaScript
- Element was replaced with new element
- Navigation occurred

**Recovery:**
```
# Re-scan the page
observe
# Use fresh element IDs
```

### ELEMENT_NOT_VISIBLE

**Description:** Element exists but is not visible.

**Causes:**
- CSS display:none or visibility:hidden
- Element outside viewport
- Covered by another element

**Recovery:**
```
# Try scrolling into view
scroll to <id>
click <id>

# Or wait for visibility
wait visible <id>
click <id>
```

### ELEMENT_DISABLED

**Description:** Element is disabled and cannot be interacted with.

**Causes:**
- Form validation hasn't passed
- Prerequisite action not completed
- Element intentionally disabled

**Recovery:**
```
# Complete required actions first
# Then wait for enabled state
wait enabled <id>
click <id>
```

### ELEMENT_NOT_INTERACTABLE

**Description:** Element cannot receive interaction (covered, etc.).

**Causes:**
- Modal/overlay covering element
- Cookie banner in the way
- Element in non-interactive state

**Recovery:**
```
# Dismiss overlays
dismiss_popups
accept_cookies

# Or force click
click <id> --force
```

### SELECTOR_INVALID

**Description:** CSS selector syntax error.

**Causes:**
- Malformed CSS selector
- Invalid characters in selector

**Recovery:**
```
# Fix selector syntax
click css(".valid-selector")
```

### TIMEOUT

**Description:** Operation exceeded timeout.

**Causes:**
- Slow network/page load
- Condition never met
- Element never appeared

**Recovery:**
```
# Increase timeout
wait visible "Element" --timeout 60s

# Or check if condition is correct
observe
```

### NAVIGATION_ERROR

**Description:** Page navigation failed or timed out.

**Causes:**
- Invalid URL
- Network error
- Page load timeout

**Recovery:**
```
# Verify URL is correct
# Check network connectivity
# Increase timeout
goto example.com --timeout 60s
```

### SCRIPT_ERROR

**Description:** JavaScript execution error.

**Causes:**
- Syntax error in script
- Runtime error
- Security restriction

**Recovery:**
```
# Check script syntax
# Verify script is allowed
```

### UNKNOWN_COMMAND

**Description:** Command not recognized.

**Causes:**
- Typo in command name
- Using unsupported command

**Recovery:**
```
# Check command spelling
help <command>
```

### INVALID_REQUEST

**Description:** Missing or malformed command parameters.

**Causes:**
- Required parameter missing
- Wrong parameter type

**Recovery:**
```
# Check command syntax
help <command>
```

### INVALID_ELEMENT_TYPE

**Description:** Element type doesn't match command.

**Causes:**
- Trying to type into a button
- Trying to check a text input
- Wrong element selected

**Recovery:**
```
# Verify element type in observe output
observe
# Use appropriate command for element type
```

### OPTION_NOT_FOUND

**Description:** Select option not found.

**Causes:**
- Value/text doesn't match any option
- Option was removed dynamically

**Recovery:**
```
# Check available options
observe --full
# Use correct option value or text
select <id> "Correct Option"
```

### INTERNAL_ERROR

**Description:** Unexpected internal error.

**Causes:**
- Bug in Oryn
- Unexpected browser state

**Recovery:**
```
# Report issue with reproduction steps
# Try restarting Oryn
```

## Intent Engine Errors

### INTENT_NOT_FOUND

**Description:** Intent name not recognized.

**Causes:**
- Typo in intent name
- Intent not loaded
- Intent file missing

**Recovery:**
```
# Check available intents
intents
# Verify spelling
```

### INTENT_UNAVAILABLE

**Description:** Intent triggers not satisfied.

**Causes:**
- Required pattern not detected
- Not on correct page
- Prerequisite state not met

**Recovery:**
```
# Check current page patterns
observe
# Navigate to correct page
goto <correct_page>
```

### PARAMETER_MISSING

**Description:** Required parameter not provided.

**Causes:**
- Forgot to provide parameter
- Wrong parameter name

**Recovery:**
```
# Provide all required parameters
login "username" "password"
```

### PARAMETER_INVALID

**Description:** Parameter type mismatch.

**Causes:**
- String instead of number
- Invalid JSON format

**Recovery:**
```
# Use correct parameter type
fill_form '{"name": "value"}'
```

### TARGET_NOT_FOUND

**Description:** Could not resolve target to element.

**Causes:**
- Text not found on page
- Role not present
- Pattern not detected

**Recovery:**
```
# Check available elements
observe
# Use ID targeting
click 5
```

### TARGET_AMBIGUOUS

**Description:** Multiple elements match target.

**Causes:**
- Multiple elements with same text
- Multiple elements with same role

**Recovery:**
```
# Be more specific
click "Sign in" near "Login Form"
# Or use ID
click 5
```

### STEP_FAILED

**Description:** Individual step execution failed.

**Causes:**
- Target not found
- Action failed
- Timeout

**Recovery:**
```
# Check intent logs for which step failed
# Fix the specific step issue
```

### VERIFICATION_FAILED

**Description:** Success conditions not met after execution.

**Causes:**
- Intent didn't achieve expected state
- Page didn't change as expected

**Recovery:**
```
# Check current page state
observe
# Verify manually or retry
```

### CHECKPOINT_INVALID

**Description:** Cannot resume from specified checkpoint.

**Causes:**
- Checkpoint name doesn't exist
- Page state doesn't match checkpoint

**Recovery:**
```
# Start from beginning
# Or use valid checkpoint name
```

### PACK_LOAD_FAILED

**Description:** Could not load intent pack.

**Causes:**
- Pack file not found
- Invalid YAML syntax
- Schema validation failed

**Recovery:**
```
# Check pack file exists
# Validate YAML syntax
# Check against schema
```

### DEFINITION_INVALID

**Description:** Intent definition schema violation.

**Causes:**
- Missing required fields
- Invalid field types
- Unknown fields

**Recovery:**
```
# Fix definition file
# Validate against schema
```

## Error Handling Best Practices

### 1. Re-scan After Errors

```
# If element not found
observe
click <new_id>
```

### 2. Use Semantic Targeting

```
# More robust than IDs
click "Sign in"
type email "user@test.com"
```

### 3. Add Wait Conditions

```
# Wait for elements to be ready
wait visible "Submit"
click "Submit"
```

### 4. Handle Popups First

```
# Clear overlays before interaction
dismiss_popups
accept_cookies
```

### 5. Increase Timeouts for Slow Operations

```
wait visible "Results" --timeout 60s
```

### 6. Check Debug Output

```bash
RUST_LOG=debug oryn headless
```
