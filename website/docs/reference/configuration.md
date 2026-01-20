# Configuration Reference

Complete reference for Oryn configuration options.

## Configuration File

Oryn reads configuration from `~/.oryn/config.yaml`:

```yaml
# ~/.oryn/config.yaml

intent_engine:
  resolution:
    tier_priority: [user, pack, core, builtin]
    allow_discovered: true
    strict_mode: false

  execution:
    default_timeout: 30s
    step_timeout: 10s
    max_retries: 3
    retry_delay: 1s

  verification:
    verify_success: true
    verify_failure: true
    rescan_after_action: auto

  logging:
    log_actions: true
    log_changes: true
    redact_sensitive: true

  learning:
    enabled: true
    min_observations: 3
    auto_propose: true

  packs:
    auto_load: true
    pack_paths:
      - ./intent-packs
      - ~/.oryn/packs
```

## Intent Engine

### resolution

Control how intents are resolved.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tier_priority` | array | [user, pack, core, builtin] | Order to search for intents |
| `allow_discovered` | boolean | true | Allow session-discovered intents |
| `strict_mode` | boolean | false | Fail on ambiguous targets |

### execution

Control intent execution behavior.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `default_timeout` | duration | 30s | Default command timeout |
| `step_timeout` | duration | 10s | Timeout per step |
| `max_retries` | number | 3 | Maximum retry attempts |
| `retry_delay` | duration | 1s | Delay between retries |

### verification

Control success/failure verification.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `verify_success` | boolean | true | Check success conditions |
| `verify_failure` | boolean | true | Check failure conditions |
| `rescan_after_action` | string | auto | When to rescan: auto, always, never |

### logging

Control action logging.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `log_actions` | boolean | true | Log executed actions |
| `log_changes` | boolean | true | Log page changes |
| `redact_sensitive` | boolean | true | Mask passwords in logs |

### learning

Control intent learning.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | true | Enable intent learning |
| `min_observations` | number | 3 | Observations before proposing |
| `auto_propose` | boolean | true | Automatically propose intents |

### packs

Control intent pack loading.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `auto_load` | boolean | true | Auto-load packs for visited sites |
| `pack_paths` | array | [...] | Directories to search for packs |

## Scanner Configuration

### scan

Default scan settings.

```yaml
scan:
  max_elements: 200
  include_hidden: false
  viewport_only: false
  pattern_detection: true
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `max_elements` | number | 200 | Maximum elements per scan |
| `include_hidden` | boolean | false | Include hidden elements |
| `viewport_only` | boolean | false | Only scan viewport |
| `pattern_detection` | boolean | true | Detect UI patterns |

## Backend Configuration

### Headless Mode

```yaml
headless:
  chrome_path: auto
  user_data_dir: null
  window_size: 1920x1080
  headless: true
  disable_gpu: false
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `chrome_path` | string | auto | Path to Chrome executable |
| `user_data_dir` | string | null | Chrome profile directory |
| `window_size` | string | 1920x1080 | Browser window size |
| `headless` | boolean | true | Run in headless mode |
| `disable_gpu` | boolean | false | Disable GPU acceleration |

### Embedded Mode

```yaml
embedded:
  driver_url: http://localhost:8080
  capabilities: {}
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `driver_url` | string | http://localhost:8080 | WebDriver server URL |
| `capabilities` | object | {} | Additional capabilities |

### Remote Mode

```yaml
remote:
  host: 127.0.0.1
  port: 9001
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `host` | string | 127.0.0.1 | WebSocket bind host |
| `port` | number | 9001 | WebSocket bind port |

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `RUST_LOG` | Log level | `RUST_LOG=debug` |
| `CHROME_PATH` | Chrome executable path | `/usr/bin/chromium` |
| `ORYN_CONFIG` | Config file path | `~/.oryn/config.yaml` |

## Command Line Options

Options can be set via command line, overriding config file:

```bash
# Headless mode options
oryn headless --chrome-path /path/to/chrome
oryn headless --no-headless
oryn headless --window-size 1280x720

# Embedded mode options
oryn embedded --driver-url http://localhost:9515

# Remote mode options
oryn remote --port 9001 --host 0.0.0.0

# Global options
oryn headless --config ./custom-config.yaml
oryn headless --log-level debug
```

## Per-Command Options

Many commands accept options that override defaults:

```
# Override timeout
goto example.com --timeout 60s
wait visible "Success" --timeout 120s
click 5 --timeout 10s

# Intent options
login "user" "pass" --wait 30s
login "user" "pass" --no-submit
```

## Sensitive Data

The following field types are automatically redacted:

```yaml
sensitive_fields:
  - password
  - credit_card
  - card_number
  - cvv
  - ssn
  - social_security
  - secret
  - token
  - api_key
```

## Example Configurations

### Minimal Configuration

```yaml
intent_engine:
  execution:
    default_timeout: 30s
```

### CI/CD Configuration

```yaml
intent_engine:
  execution:
    default_timeout: 60s
    max_retries: 5

  logging:
    log_actions: true
    log_changes: true

headless:
  headless: true
  disable_gpu: true
  window_size: 1920x1080
```

### IoT Configuration

```yaml
intent_engine:
  execution:
    default_timeout: 45s

scan:
  max_elements: 100
  viewport_only: true

embedded:
  driver_url: http://localhost:8080
```

### Development Configuration

```yaml
intent_engine:
  execution:
    default_timeout: 30s

  logging:
    log_actions: true
    log_changes: true
    redact_sensitive: false  # For debugging only!

headless:
  headless: false  # Visible browser
  window_size: 1280x720
```
