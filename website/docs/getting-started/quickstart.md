# Quick Start

Run your first Oryn automation in a few minutes.

## 1. Start Oryn

```bash
oryn headless
```

You will get the `>` REPL prompt.

## 2. Navigate and Observe

```text
> goto example.com
> observe
```

`observe` returns a structured page view with element IDs you can use in actions.

## 3. Interact with Elements

```text
> click 1
> back
> observe
```

You can also use semantic targets:

```text
> click "More information..."
```

## 4. Form Example

```text
> goto github.com/login
> observe
> type email "myusername"
> type password "mypassword"
> click submit
```

## 5. Intent Commands Available Today

```text
> login "myusername" "mypassword"
> search "oryn browser automation"
> accept_cookies
> dismiss popups
```

## 6. Waits and Screenshots

```text
> wait load
> wait visible "Success"
> screenshot --output ./page.png
```

## 7. Run a Script File

```bash
oryn --file test-harness/scripts/01_static.oil headless
```

## 8. Exit

```text
> exit
```

## Next Steps

- [CLI Reference](cli-reference.md)
- [Intent Commands](../reference/intent-commands.md)
- [Troubleshooting](../guides/troubleshooting.md)
