# Configuration Reference

Current configuration and runtime environment settings for Oryn.

## Status

Oryn includes a YAML config schema and loader in `oryn-engine`, but the unified `oryn` CLI currently does not expose a `--config` flag.

The loader searches these paths when used by engine integrations:

1. `./oryn.yaml`
2. `~/.oryn/config.yaml`

## YAML Schema

Top-level schema (`oryn-engine`):

```yaml
intent_engine:
  default_timeout_ms: 30000
  step_timeout_ms: 10000
  max_retries: 3
  retry_delay_ms: 1000
  strict_mode: false

packs:
  auto_load: true
  pack_paths:
    - ~/.oryn/packs
    - ./packs

learning:
  enabled: false
  min_observations: 3
  min_confidence: 0.75
  min_pattern_length: 2
  auto_accept: false

security:
  sensitive_fields:
    - password
    - token
    - card_number
    - cvv
    - ssn
    - secret
  redact_in_logs: true
```

## Field Reference

### `intent_engine`

| Field | Type | Default | Description |
|------|------|---------|-------------|
| `default_timeout_ms` | integer | `30000` | Default command timeout in ms |
| `step_timeout_ms` | integer | `10000` | Per-step timeout in ms |
| `max_retries` | integer | `3` | Retry attempts |
| `retry_delay_ms` | integer | `1000` | Delay between retries |
| `strict_mode` | boolean | `false` | Stricter resolution/validation behavior |

### `packs`

| Field | Type | Default | Description |
|------|------|---------|-------------|
| `auto_load` | boolean | `true` | Auto-load intent packs |
| `pack_paths` | array[path] | `~/.oryn/packs`, `./packs` | Pack search paths |

### `learning`

| Field | Type | Default | Description |
|------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable learning pipeline |
| `min_observations` | integer | `3` | Minimum observations before proposing |
| `min_confidence` | float | `0.75` | Confidence threshold |
| `min_pattern_length` | integer | `2` | Minimum sequence length |
| `auto_accept` | boolean | `false` | Auto-accept learned patterns |

### `security`

| Field | Type | Default | Description |
|------|------|---------|-------------|
| `sensitive_fields` | array[string] | built-in list | Field names to redact |
| `redact_in_logs` | boolean | `true` | Enable log redaction |

## Environment Variables

### Core

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Set Rust logging level |

### Headless backend (`oryn-h`)

| Variable | Description |
|----------|-------------|
| `CHROME_BIN` | Chromium/Chrome executable path |
| `ORYN_USER_DATA_DIR` | Use a fixed browser profile directory |
| `ORYN_ENABLE_NETWORK_LOG` | Enable network logging (`1/true/yes/on`) |

## Notes

- `CHROME_PATH` and `ORYN_CONFIG` are not current runtime variables.
- Unified CLI options such as `--chrome-path`, `--host`, `--log-level`, and `--config` are not currently available.
