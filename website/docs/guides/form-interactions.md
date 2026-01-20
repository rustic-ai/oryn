# Form Interactions

This guide covers how to interact with forms, including text inputs, dropdowns, checkboxes, and form submission.

## Text Inputs

### Basic Typing

```
> observe
[1] input/email "Email" {required}
[2] input/password "Password" {required}

> type 1 "user@example.com"
ok type [1] "user@example.com"

> type 2 "mypassword"
ok type [2] "••••••••"
```

Note: Password values are automatically masked in responses.

### Typing by Role

```
> type email "user@example.com"
ok type [1] "user@example.com"

> type password "mypassword"
ok type [2] "••••••••"
```

### Typing by Label

```
> type "Email" "user@example.com"
ok type [1] "user@example.com"
```

### Type Options

**Append without clearing:**
```
> type 1 "additional text" --append
```

**Type and press Enter:**
```
> type 1 "search query" --enter
```

**Type with delay between keys:**
```
> type 1 "slow typing" --delay 100
```

### Clearing Inputs

```
> clear 1
ok clear [1]
```

## Dropdowns (Select Elements)

### Observe Dropdown

```
> observe
[3] select "Country" ["United States", "Canada", "Mexico", ...]
```

### Select by Value

```
> select 3 "us"
ok select [3] "United States"
```

### Select by Visible Text

```
> select 3 "Canada"
ok select [3] "Canada"
```

### Select by Index

```
> select 3 --index 2
ok select [3] "Mexico"
```

## Checkboxes

### Observe Checkbox

```
> observe
[4] checkbox "Remember me" {unchecked}
[5] checkbox "Subscribe to newsletter" {checked}
```

### Check a Checkbox

```
> check 4
ok check [4]
```

### Uncheck a Checkbox

```
> uncheck 5
ok uncheck [5]
```

### Toggle by Label

```
> check "Remember me"
ok check [4]
```

## Radio Buttons

### Observe Radio Buttons

```
> observe
[6] radio "Standard Shipping" {checked}
[7] radio "Express Shipping" {unchecked}
[8] radio "Overnight Shipping" {unchecked}
```

### Select Radio Option

```
> click 7
ok click [7]
```

Or by label:

```
> click "Express Shipping"
ok click [7]
```

## Pressing Keys

### Common Keys

```
> press Enter
> press Tab
> press Escape
> press Space
> press Backspace
> press Delete
```

### Arrow Keys

```
> press ArrowUp
> press ArrowDown
> press ArrowLeft
> press ArrowRight
```

### Function Keys

```
> press F1
> press F5
```

### Key Combinations

```
> press Control+A    # Select all
> press Control+C    # Copy
> press Control+V    # Paste
> press Shift+Tab    # Previous field
```

## Form Submission

### Click Submit Button

```
> click submit
ok click [9] "Submit"
```

Or by text:

```
> click "Submit"
ok click [9] "Submit"
```

### Use the submit_form Intent

```
> submit_form
ok submit_form

# actions
click [9] "Submit"
wait navigation
```

### Type and Submit

```
> type 1 "search query" --enter
```

## The fill_form Intent

For filling multiple fields at once:

```
> fill_form {"name": "John Doe", "email": "john@example.com", "country": "us"}
ok fill_form

# filled
[1] input "Name" ← "John Doe"
[2] input "Email" ← "john@example.com"
[3] select "Country" ← "United States"
```

### Partial Filling

```
> fill_form {"name": "John"} --partial
ok fill_form

# filled
[1] input "Name" ← "John"

# skipped
"email": no value provided
"country": no value provided
```

## The login Intent

For login forms, use the built-in login intent:

```
> goto example.com/login
> login "user@example.com" "password123"
ok login

# actions
type [1] "user@example.com"
type [2] "••••••••"
click [3] "Sign in"
wait navigation

# changes
~ url: /login → /dashboard
- login_form
+ user_menu
```

### Login Without Submitting

```
> login "user@example.com" "password123" --no-submit
ok login (not submitted)

# actions
type [1] "user@example.com"
type [2] "••••••••"
```

## The search Intent

For search forms:

```
> goto google.com
> search "oryn browser automation"
ok search

# actions
clear [1]
type [1] "oryn browser automation"
press Enter
wait idle
```

## Relational Targeting

### Target Near Another Element

```
> click "Edit" near "Item 1"
ok click [5] "Edit"
```

### Target Inside Container

```
> type "Quantity" inside "Product Card" "5"
ok type [12] "5"
```

### Target After Element

```
> click button after "Product Description"
ok click [8]
```

## Waiting for Form Results

### Wait for Success Message

```
> click "Submit"
> wait visible "Thank you"
ok wait visible "Thank you" (2.3s)
```

### Wait for Error Message

```
> click "Submit"
> wait visible "Please fix the errors"
```

### Wait for Navigation

```
> click "Submit"
> wait url "/confirmation"
ok wait url "/confirmation" (3.1s)
```

## Error Handling

### Element Not Found

```
> type 99 "text"
error type: element [99] not found

# hint
Available elements: 1-8. Run 'observe' to refresh.
```

### Element Not Interactable

```
> type 5 "text"
error type: element [5] is not interactable (disabled)

# hint
Wait for element to be enabled or check if correct element.
```

### Select Option Not Found

```
> select 3 "Invalid Option"
error select: option "Invalid Option" not found in [3]

# hint
Available options: "United States", "Canada", "Mexico"
```

## Complete Form Example

```
# Navigate to signup form
goto example.com/signup

# Dismiss any popups
accept_cookies
dismiss_popups

# Observe the form
observe

@ example.com/signup "Sign Up"
[1] input "First Name" {required}
[2] input "Last Name" {required}
[3] input/email "Email" {required}
[4] input/password "Password" {required}
[5] input/password "Confirm Password" {required}
[6] select "Country" ["United States", "Canada", ...]
[7] checkbox "I agree to Terms" {unchecked, required}
[8] checkbox "Subscribe to newsletter" {unchecked}
[9] button/submit "Create Account" {primary}

# Fill the form
type 1 "John"
type 2 "Doe"
type email "john.doe@example.com"
type 4 "SecurePass123!"
type 5 "SecurePass123!"
select 6 "United States"
check 7
check 8

# Submit
click "Create Account"

# Wait for confirmation
wait url "/welcome"
observe
```

## Best Practices

1. **Use semantic targeting** — `type email "..."` is more robust than `type 1 "..."`

2. **Observe before interacting** — Get current element IDs

3. **Handle required fields** — Note the `{required}` modifier

4. **Wait for results** — Don't assume instant submission

5. **Use intents for common forms** — `login`, `search`, `fill_form`

6. **Check for errors** — Look for error patterns after submission
