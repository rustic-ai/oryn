# Quick Start

This guide will have you running your first web automation with Oryn in minutes.

## Starting Oryn

Launch Oryn in headless mode:

```bash
oryn headless
```

You'll see the Oryn REPL prompt:

```
Oryn v1.0.0 - Headless Mode
Browser ready.
>
```

## Basic Navigation

### Navigate to a Page

```
> goto example.com
ok goto https://example.com

# changes
@ example.com "Example Domain"
```

### Observe the Page

The `observe` command (or `scan`) shows all interactive elements:

```
> observe

@ example.com "Example Domain"
[1] link "More information..." {external}

# patterns
(none detected)
```

Each element has:
- **Numeric ID** (`[1]`) for targeting
- **Type** (`link`, `input`, `button`, etc.)
- **Text** (visible or accessible label)
- **Modifiers** (`{required}`, `{disabled}`, `{primary}`, etc.)

### Click an Element

```
> click 1
ok click [1]

# changes
~ url: example.com → www.iana.org/help/example-domains
```

## Form Interaction Example

Let's navigate to a login page and interact with it:

```
> goto github.com/login
ok goto https://github.com/login

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

### Type into Fields

```
> type 1 "myusername"
ok type [1] "myusername"

> type 2 "mypassword"
ok type [2] "••••••••"
```

Notice that password values are automatically masked in responses.

### Submit the Form

```
> click 4
ok click [4]
```

Or use semantic targeting:

```
> click "Sign in"
ok click [4]
```

## Using Intent Commands

Oryn provides high-level intent commands that encapsulate common workflows.

### Login Intent

Instead of individual commands:

```
> login "myusername" "mypassword"
ok login

# actions
type [1] "myusername"
type [2] "••••••••"
click [4] "Sign in"
wait navigation
```

### Search Intent

```
> goto google.com
> search "oryn browser automation"
ok search "oryn browser automation"

# actions
type [1] "oryn browser automation"
press Enter
wait idle
```

### Accept Cookies

```
> accept_cookies
ok accept_cookies

# actions
click [7] "Accept All"

# changes
- cookie_banner
```

## Semantic Targeting

You can target elements by text, role, or position instead of ID:

### By Text

```
> click "Sign in"
> type "Email" "user@example.com"
```

### By Role

```
> type email "user@example.com"
> type password "secret123"
> click submit
```

### Relational Targeting

```
> click "Remove" near "Item 1"
> type "Quantity" inside "Product Card" "5"
```

## Scrolling

```
> scroll down
> scroll down 500       # scroll 500 pixels
> scroll to "Footer"    # scroll element into view
```

## Waiting

```
> wait visible "Success"
> wait hidden "Loading..."
> wait idle             # wait for network to be idle
> wait load             # wait for page load
```

## Exiting

```
> exit
Goodbye!
```

## Running Scripts

You can run Oryn scripts from files:

```bash
# Create a script file
cat > my-script.oil << 'EOF'
goto example.com
observe
click "More information..."
observe
EOF

# Run it
oryn headless < my-script.oil
```

## Next Steps

- **[CLI Reference](cli-reference.md)** — All command-line options
- **[Intent Language](../concepts/intent-language.md)** — Complete command syntax
- **[Form Interactions](../guides/form-interactions.md)** — Advanced form handling
- **[Custom Intents](../guides/custom-intents.md)** — Define your own commands
