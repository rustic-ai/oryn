# Intent Language

Oryn Intent Language (OIL) is the command language used by the unified CLI and adapters.

## Design Goals

- Compact commands for agents and operators
- Semantic targets (`"Sign in"`, `email`, CSS selectors)
- Human-readable interaction loop

## Command Shape

```text
command [target] [arguments] [--options]
```

## Targeting

- ID: `click 5`
- Text: `click "Sign in"`
- Semantic token: `type email "user@example.com"`
- Selector: `click css(".primary")`
- Relational: `click "Edit" near "Item 1"`

## Common Commands

### Navigation

```text
goto <url>
back
forward
refresh
url
```

### Observation

```text
observe
observe --full
text
title
screenshot
```

### Actions

```text
click <target>
type <target> "text"
clear <target>
select <target> <value-or-index>
check <target>
uncheck <target>
hover <target>
focus <target>
scroll ...
submit [target]
press <key>
```

### Wait

```text
wait load
wait idle
wait visible <target>
wait hidden <target>
wait exists "<selector>"
wait gone "<selector>"
wait url "<pattern>"
```

### Intent Commands (current)

```text
login "user" "pass"
search "query"
accept_cookies
dismiss popups
```

## Alias/Normalization Notes

- `scan` is normalized to `observe`.
- `accept cookies` is normalized to `accept_cookies`.

## Output Model

Oryn returns structured text responses including:

- current page header (`@ ...`),
- indexed elements (`[id] ...`),
- optional pattern section,
- error lines with hints when available.

See [Intent Commands Reference](../reference/intent-commands.md) for exact current syntax/support.
