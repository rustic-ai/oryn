# Intent Commands Reference

Current command support in the unified `oryn` CLI.

## Command Form

```text
command [target] [arguments] [--options]
```

## Target Forms

- Numeric ID: `click 5`
- Text: `click "Sign in"`
- Role-like token: `type email "user@example.com"`
- Selector: `click css(".btn")`
- Relational: `click "Edit" near "Item 1"`

## Navigation

### `goto`

```text
goto <url> [--headers "<json>"] [--timeout <duration>]
```

Note: `--headers` and `--timeout` parse but are currently not applied in unified translation.

### `back`

```text
back
```

### `forward`

```text
forward
```

### `refresh`

```text
refresh [--hard]
```

Note: `--hard` is parsed, but current executor/backend wiring does not distinguish hard vs soft refresh.

### `url`

```text
url
```

## Observation

### `observe` (alias: `scan`)

```text
observe [--full] [--minimal] [--viewport] [--hidden] [--positions] [--diff] [--near <text>] [--timeout <duration>]
```

Notes:

- `scan` is normalized to `observe`.
- `--minimal`, `--positions`, and `--timeout` are parsed but currently have limited/no translation effect.

### `html`

```text
html [--selector "<css>"]
```

### `text`

```text
text [--selector "<css>"] [<target>]
```

Note: current translation uses selector-based extraction; target support is limited.

### `title`

```text
title
```

### `screenshot`

```text
screenshot [--output <path>] [--format png|jpeg|webp] [--fullpage] [<target>]
```

Note: target parsing exists, but element-target capture is currently limited in translation.

## Actions

### `click`

```text
click <target> [--double] [--right] [--middle] [--force] [--ctrl] [--shift] [--alt] [--timeout <duration>]
```

Note: `--ctrl/--shift/--alt` and `--timeout` are parsed, but currently not applied by translation/execution.

### `type`

```text
type <target> "<text>" [--append] [--enter] [--delay <ms>] [--clear] [--timeout <duration>]
```

Note: `--append` and `--timeout` parse but are currently not applied in unified translation.

### `clear`

```text
clear <target>
```

### `press`

```text
press <key-or-combo>
```

Examples:

```text
press enter
press control+a
press shift+tab
```

### `select`

```text
select <target> <value-or-index>
```

Notes:

- String argument selects by label/value semantics.
- Numeric argument is translated as index.

### `check`

```text
check <target>
```

### `uncheck`

```text
uncheck <target>
```

### `hover`

```text
hover <target>
```

### `focus`

```text
focus <target>
```

### `scroll`

```text
scroll [up|down|left|right] [<target>] [--amount <n>] [--page] [--timeout <duration>]
```

Examples:

```text
scroll down
scroll --amount 300
scroll down --amount 500
scroll 12
```

Note: `--timeout` is parsed but currently not applied in unified translation.

### `submit`

```text
submit [<target>]
```

## Wait

### `wait`

```text
wait load|idle|navigation [--timeout <duration>]
wait visible <target> [--timeout <duration>]
wait hidden <target> [--timeout <duration>]
wait exists "<selector>" [--timeout <duration>]
wait gone "<selector>" [--timeout <duration>]
wait url "<pattern>" [--timeout <duration>]
wait until "<expression>" [--timeout <duration>]
wait items "<selector>" <count> [--timeout <duration>]
```

Notes:

- `ready` is parsed in grammar but not currently mapped to a supported scanner wait condition.
- `wait url "<pattern>"` currently waits for generic navigation and does not apply URL pattern matching in translation.
- `wait enabled` is not currently part of supported grammar.

## Extraction

### `extract`

```text
extract links|images|tables|meta|text|css("<selector>") [--selector "<css>"] [--format json|csv|text]
```

## Intent Commands (Current)

### `login`

```text
login "<username>" "<password>" [--no-submit] [--wait <duration>] [--timeout <duration>]
```

Note: `--no-submit`, `--wait`, and `--timeout` parse but are currently not applied in unified translation.

### `search`

```text
search "<query>" [--submit enter|click|auto] [--wait <duration>] [--timeout <duration>]
```

Note: `--submit`, `--wait`, and `--timeout` parse but are currently not applied in unified translation.

### `dismiss`

```text
dismiss popups|modals|modal|banner|"<target>"
```

### `accept_cookies`

```text
accept_cookies
```

## Session / Tabs / Utility

### Cookies

```text
cookies list
cookies get <name>
cookies set <name> "<value>"
cookies delete <name>
cookies clear
```

Note: `cookies clear` is currently limited in executor support.

### Tabs

```text
tabs
tab new <url>
tab switch <index>
tab close [<index>]
```

Note: `tabs` listing is implemented; `tab new/switch/close` are currently limited in executor support.

### PDF

```text
pdf <path> [--format A4|Letter|...] [--landscape] [--margin <value>]
```

Note: backend support is primarily headless mode.

### Exit

```text
exit
quit
```

## Not Currently Available in Unified CLI

These appear in older docs/spec drafts but are not currently end-to-end available in the unified command path:

- `logout`
- `dismiss_popups`
- `fill_form`
- `submit_form`
- `scroll_to`
- `version`
- `wait enabled ...`
- intent-management commands (`intents`, `define`, `undefine`, `export`, `run`) are present in grammar/parser surface but currently incomplete/stubbed in execution.
