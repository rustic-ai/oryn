# Guides

Practical guides for common Oryn use cases.

## Overview

These guides walk you through real-world scenarios and best practices for using Oryn effectively.

<div class="grid cards" markdown>

-   :material-navigation:{ .lg .middle } **Basic Navigation**

    ---

    Learn to navigate pages, observe elements, and interact with the web.

    [:octicons-arrow-right-24: Basic Navigation](basic-navigation.md)

-   :material-form-textbox:{ .lg .middle } **Form Interactions**

    ---

    Fill forms, handle dropdowns, checkboxes, and submit data.

    [:octicons-arrow-right-24: Form Interactions](form-interactions.md)

-   :material-puzzle:{ .lg .middle } **Custom Intents**

    ---

    Current status and practical alternatives for repeated workflows.

    [:octicons-arrow-right-24: Custom Intents](custom-intents.md)

-   :material-file-document-multiple:{ .lg .middle } **Multi-Page Flows**

    ---

    Orchestrate workflows that span multiple pages.

    [:octicons-arrow-right-24: Multi-Page Flows](multi-page-flows.md)

-   :material-bug:{ .lg .middle } **Troubleshooting**

    ---

    Common issues and how to resolve them.

    [:octicons-arrow-right-24: Troubleshooting](troubleshooting.md)

</div>

## Quick Tips

### Always Observe First

Before interacting with a page, run `observe` to get the current element IDs:

```
goto example.com
observe
click 1
```

### Re-scan After Navigation

Element IDs change when the page changes. Always re-scan:

```
click "Submit"
observe    # Re-scan to get new elements
click "Continue"
```

### Use Semantic Targeting

When possible, use text or role targeting for more robust scripts:

```
# Fragile: ID might change
click 5

# Robust: text is stable
click "Sign in"

# Robust: role is semantic
type email "user@test.com"
```

### Handle Popups Early

Many sites show cookie banners or modals. Dismiss them first:

```
goto example.com
accept_cookies
dismiss popups
observe
```
