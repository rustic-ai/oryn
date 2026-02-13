# Basic Navigation

This guide covers the fundamentals of navigating web pages with Oryn.

## Navigating to Pages

### Using `goto`

Navigate to any URL:

```
> goto google.com
ok goto https://google.com
```

The protocol (`https://`) is automatically added if omitted.

### Full URLs

You can specify the full URL:

```
> goto https://github.com/login
ok goto https://github.com/login
```

### URL Normalization Note

`goto` adds `https://` when protocol is omitted:

```
> goto example.com
ok goto https://example.com
```

Relative navigation like `goto /about` is not currently resolved against the active page URL in the unified translation path.

## History Navigation

### Go Back

```
> back
ok back
@ example.com "Previous Page"
```

### Go Forward

```
> forward
ok forward
@ example.com "Next Page"
```

### Refresh

```
> refresh
ok refresh
```

`refresh --hard` syntax:

```
> refresh --hard
ok refresh
```

`--hard` is parsed, but hard/soft distinction is not currently applied by executor/backend wiring.

## Observing Pages

The `observe` command (alias: `scan`) is fundamental to understanding what's on a page.

### Basic Observation

```
> observe

@ github.com/login "Sign in to GitHub"
[1] input/email "Username or email address" {required}
[2] input/password "Password" {required}
[3] link "Forgot password?"
[4] button/submit "Sign in" {primary}
[5] link "Create an account"

# patterns
- login_form: email=[1] password=[2] submit=[4]
```

### Understanding the Output

**Page Header:**
```
@ github.com/login "Sign in to GitHub"
```
Shows the current URL and page title.

**Elements:**
```
[1] input/email "Username or email address" {required}
```
- `[1]` — Numeric ID for targeting
- `input` — Element type
- `email` — Semantic role
- `"Username..."` — Visible text/label
- `{required}` — State modifier

**Patterns:**
```
# patterns
- login_form: email=[1] password=[2] submit=[4]
```
Detected UI patterns with element references.

### Observation Options

**Full details:**
```
> observe --full

@ github.com/login "Sign in to GitHub"
[1] input/email "Username or email address" {required}
    selector: #login_field
    xpath: //input[@id='login_field']
    rect: x=450 y=200 w=300 h=40
```

**Filter by proximity:**
```
> observe --near "Sign in"

[4] button/submit "Sign in" {primary}
[5] link "Create an account"
```

`observe --minimal` is parsed, but output is currently close to standard `observe` unless backend/scanner behavior changes.

## Clicking Elements

### By ID

The fastest and most precise method:

```
> click 4
ok click [4] "Sign in"
```

### By Text

More robust across page changes:

```
> click "Sign in"
ok click [4] "Sign in"
```

Text matching is case-insensitive and uses partial matching by default.

### By Role

For semantic targeting:

```
> click submit
ok click [4] "Sign in"
```

### Click Options

**Double-click:**
```
> click 5 --double
ok double-click [5]
```

**Right-click:**
```
> click 5 --right
ok right-click [5]
```

**Force click (even if covered):**
```
> click 5 --force
ok click [5] (forced)
```

## Scrolling

### Direction Scrolling

```
> scroll down
> scroll down --amount 500
> scroll up
> scroll left
> scroll right
```

### Page Scrolling

```
> scroll down --page   # One page height
> scroll up --page
```

### Scroll to Element

```
> observe
> scroll 15
ok scroll [15]
```

## Getting Page Information

### Current URL

```
> url
https://example.com/about
```

### Page Title

```
> title
About Us - Example Company
```

### Page Text

```
> text
[Full text content of the page]
```

### Extract Links

```
> extract links
[1] https://example.com/home "Home"
[2] https://example.com/about "About"
[3] https://example.com/contact "Contact"
```

## Waiting for Conditions

### Wait for Page Load

```
> wait load
ok wait load (1.2s)
```

### Wait for Network Idle

```
> wait idle
ok wait idle (2.3s)
```

### Wait for Element Visibility

```
> wait visible "Success"
ok wait visible "Success" (1.5s)
```

### Wait for Element to Disappear

```
> wait hidden "Loading..."
ok wait hidden "Loading..." (3.2s)
```

### Wait for URL Change

```
> wait url "/dashboard"
ok wait url "/dashboard" (2.1s)
```

### Custom Timeout

```
> wait visible "Results" --timeout 60s
ok wait visible "Results" (45.3s)
```

## Screenshots

### Capture Full Page

```
> screenshot
ok screenshot saved to ./screenshot.png
```

### Specify Output File

```
> screenshot --output ./my-screenshot.png
ok screenshot saved to ./my-screenshot.png
```

Element-target screenshots are currently limited in unified translation. Use full-page screenshots with explicit output paths.

## Common Patterns

### Basic Page Exploration

```
goto example.com
observe
click "About"
observe
back
observe
```

### Handling Cookie Banners

```
goto example.com
accept_cookies    # Built-in intent
observe
```

### Navigating with Pagination

```
goto example.com/products
observe

# Next page
click "Next"
observe

# Previous page
click "Previous"
observe
```

### Scrolling to Load Content

```
goto example.com/infinite-scroll
observe

scroll down --amount 500
wait idle
observe     # New elements loaded

scroll down --amount 500
wait idle
observe     # More elements loaded
```

## Best Practices

1. **Always observe after navigation** — Element IDs change between pages

2. **Use semantic targeting when possible** — More resilient to page changes

3. **Handle popups early** — Cookie banners and modals can block interaction

4. **Wait for conditions** — Don't assume instant page loads

5. **Re-scan after dynamic updates** — AJAX content changes the element map
