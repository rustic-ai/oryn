# Form Interactions

Practical form automation patterns using currently supported Oryn commands.

## Observe First

```text
> observe
[1] input/email "Email" {required}
[2] input/password "Password" {required}
[3] select "Country"
[4] checkbox "Remember me"
[5] button/submit "Sign in"
```

## Typing

### By ID

```text
> type 1 "user@example.com"
> type 2 "mypassword"
```

### By semantic target

```text
> type email "user@example.com"
> type password "mypassword"
```

### Useful options

```text
> type 1 "search query" --enter
> type 1 "more" --append
> type 1 "slow" --delay 100
```

`--append` is parsed, but currently not applied in unified translation.

## Clear Inputs

```text
> clear 1
```

## Select Dropdown Values

```text
> select 3 "Canada"
> select 3 2
```

Notes:

- string argument: label/value style selection
- numeric argument: translated as index

## Checkboxes

```text
> check 4
> uncheck 4
```

## Submit Forms

```text
> click submit
```

or

```text
> click "Sign in"
```

## Login Shortcut

```text
> login "user@example.com" "password123"
```

Optional no-submit mode:

```text
> login "user@example.com" "password123" --no-submit
```

`--no-submit` is parsed, but currently not applied in unified translation.

## Common Failure Recovery

### Element IDs changed

```text
> observe
> type email "user@example.com"
```

### Button disabled or hidden

```text
> wait visible "Sign in"
> click "Sign in"
```

### Popup blocks interaction

```text
> accept_cookies
> dismiss popups
> observe
```

## Recommended Flow

```text
goto https://example.com/login
observe
type email "user@example.com"
type password "password123"
click submit
wait navigation
observe
```
