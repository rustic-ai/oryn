# Troubleshooting

Common issues and concrete recovery steps for current Oryn behavior.

## Headless Browser Launch Issues

### Chromium executable not found

Set `CHROME_BIN`:

```bash
export CHROME_BIN=/path/to/chrome
oryn headless
```

Optional isolated profile:

```bash
export ORYN_USER_DATA_DIR=/tmp/oryn-profile
oryn headless
```

## Embedded Mode WebDriver Issues

```bash
oryn embedded --driver-url http://localhost:8080
```

If connection fails, verify your WebDriver endpoint is running and reachable.

## Remote Mode Connection Issues

```bash
oryn remote --port 9001
```

Checklist:

1. Load `extension/` in browser dev mode.
2. Connect extension to `localhost:9001`.
3. Keep Oryn process running.

## Element Not Found / Stale IDs

IDs can change after navigation or DOM updates.

```text
observe
click "Sign in"
```

Prefer semantic targeting (`"Sign in"`, `email`, `submit`) when possible.

## Element Not Visible / Covered

```text
wait visible "Submit"
accept_cookies
dismiss popups
click "Submit"
```

If needed:

```text
click "Submit" --force
```

## Slow or Flaky Navigation

```text
goto example.com
wait navigation --timeout 60s
wait load
wait idle
observe
```

## Wait Condition Never Resolves

Check that the condition type is supported:

- `load`, `idle`, `navigation`
- `visible <target>`, `hidden <target>`
- `exists "<selector>"`, `gone "<selector>"`
- `url "<pattern>"`, `until "<expr>"`, `items "<selector>" <count>`

`wait ready` is currently parsed but not translated to a supported scanner wait condition.

`wait enabled ...` is not currently part of supported grammar.

## Script Mode Errors

Run scripts with:

```bash
oryn --file path/to/script.oil headless
```

The runner skips blank lines and `#` comments; stop-on-error is enabled in unified CLI file mode.

## Useful Logging

```bash
RUST_LOG=debug oryn headless
```

For headless network debugging:

```bash
ORYN_ENABLE_NETWORK_LOG=1 oryn headless
```
