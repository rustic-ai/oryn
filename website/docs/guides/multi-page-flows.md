# Multi-Page Flows

This guide covers how to orchestrate workflows that span multiple pages, such as checkout processes, wizards, and multi-step forms.

## Overview

Multi-page flows let you define intents that:

- Navigate across multiple pages automatically
- Extract data at each step
- Handle page transitions gracefully
- Resume from checkpoints on failure

## Flow Definition

### Basic Structure

```yaml
intent: complete_purchase
version: "1.0"
description: "Complete a multi-step purchase flow"

flow:
  start: cart
  pages:
    - name: cart
      url_pattern: ".*/cart.*"
      steps:
        - action: click
          target: { text: "Checkout" }
      next: shipping

    - name: shipping
      url_pattern: ".*/checkout/shipping.*"
      steps:
        - action: intent
          name: fill_shipping
      next: payment

    - name: payment
      url_pattern: ".*/checkout/payment.*"
      steps:
        - action: intent
          name: fill_payment
      next: confirmation

    - name: confirmation
      url_pattern: ".*/confirmation.*"
      extract:
        order_number:
          selector: "#order-number"
      next: end
```

### Page Definition

Each page in a flow has:

| Field | Description |
|-------|-------------|
| `name` | Unique identifier for the page |
| `url_pattern` | Regex pattern to match the page URL |
| `steps` | Actions to execute on this page |
| `extract` | Data to extract from this page |
| `next` | The next page in the flow (or `end`) |
| `on_error` | Error handling for this page |

## URL Pattern Matching

### Basic Patterns

```yaml
# Exact path
url_pattern: "/checkout/shipping"

# Any subdomain
url_pattern: ".*/checkout/shipping.*"

# Path with variable segment
url_pattern: ".*/orders/[0-9]+/confirm.*"

# Multiple paths
url_pattern: ".*/(checkout|cart)/.*"
```

### Waiting for Page

The flow engine automatically waits for the URL to match:

```yaml
- name: confirmation
  url_pattern: ".*/confirmation.*"
  # Engine waits until URL matches before executing steps
```

## Step Actions

### Navigation Actions

```yaml
# Navigate to URL
- action: navigate
  url: "https://example.com/checkout"

# Go back
- action: go_back

# Go forward
- action: go_forward

# Refresh
- action: refresh
```

### Standard Actions

```yaml
# Click
- action: click
  target: { text: "Continue" }

# Type
- action: type
  target: { role: email }
  text: $email

# Fill form
- action: fill_form
  data: $shipping_info
```

### Sub-Intent Execution

```yaml
# Execute another intent
- action: intent
  name: fill_shipping
  params:
    address: $shipping_address
```

## Data Extraction

### Extract from Page

```yaml
pages:
  - name: confirmation
    url_pattern: ".*/confirmation.*"
    extract:
      order_number:
        selector: "#order-number"
      total:
        selector: ".order-total"
      items:
        selector: ".order-item"
        multiple: true
```

### Extract from URL

```yaml
extract:
  order_id:
    from: url
    pattern: "/orders/([0-9]+)"
```

### Access Extracted Data

Extracted data is merged into the final result:

```
> complete_purchase --shipping {...} --payment {...}
ok complete_purchase

# result
order_number: "ORD-12345"
total: "$99.99"
items: ["Widget A", "Gadget B"]
```

## Passing Data Between Pages

### Parameters

```yaml
intent: checkout_flow
parameters:
  - name: shipping
    type: object
    required: true
  - name: payment
    type: object
    required: true

flow:
  pages:
    - name: shipping
      steps:
        - action: fill_form
          data: $shipping    # Use parameter

    - name: payment
      steps:
        - action: fill_form
          data: $payment     # Use parameter
```

### Extracted Data

Data extracted from earlier pages is available in later pages:

```yaml
flow:
  pages:
    - name: cart
      extract:
        cart_total:
          selector: ".cart-total"

    - name: payment
      steps:
        - action: verify
          condition:
            equals: [$cart_total, $expected_total]
```

## Error Handling

### Per-Page Error Handler

```yaml
pages:
  - name: payment
    url_pattern: ".*/checkout/payment.*"
    steps:
      - action: fill_form
        data: $payment
      - action: click
        target: { text: "Submit Payment" }
    on_error:
      - action: screenshot
        output: "payment_error.png"
      - action: observe
```

### Retry Logic

```yaml
pages:
  - name: shipping
    steps:
      - action: fill_form
        data: $shipping
    retry:
      max_attempts: 3
      delay: 2s
      on: [form_error, timeout]
```

## Checkpoints

### Defining Checkpoints

```yaml
flow:
  pages:
    - name: cart
      checkpoint: cart_verified
      steps:
        - action: verify_cart

    - name: shipping
      checkpoint: shipping_complete
      steps:
        - action: fill_shipping

    - name: payment
      checkpoint: payment_complete
      steps:
        - action: fill_payment
```

### Resuming from Checkpoint

If a flow fails, you can resume:

```
> complete_purchase --resume shipping_complete --payment {...}
ok complete_purchase (resumed from shipping_complete)
```

## Complete Example: E-commerce Checkout

```yaml
intent: ecommerce_checkout
version: "1.0"
description: "Complete e-commerce checkout flow"

parameters:
  - name: shipping_address
    type: object
    required: true
  - name: payment_method
    type: object
    required: true
  - name: promo_code
    type: string
    required: false

flow:
  start: cart

  pages:
    - name: cart
      url_pattern: ".*/cart.*"
      steps:
        # Apply promo code if provided
        - branch:
            if: $promo_code
            then:
              - action: type
                target: { placeholder_contains: "promo" }
                text: $promo_code
              - action: click
                target: { text: "Apply" }
              - action: wait
                condition: { visible: { text_contains: "discount" } }
                timeout: 5s

        # Proceed to checkout
        - action: click
          target: { text: "Proceed to Checkout" }

      extract:
        subtotal:
          selector: ".subtotal"
        discount:
          selector: ".discount"
          default: "$0.00"

      checkpoint: cart_complete
      next: shipping

    - name: shipping
      url_pattern: ".*/checkout/shipping.*"
      steps:
        - action: fill_form
          data:
            first_name: $shipping_address.first_name
            last_name: $shipping_address.last_name
            address: $shipping_address.street
            city: $shipping_address.city
            state: $shipping_address.state
            zip: $shipping_address.zip
            country: $shipping_address.country

        - action: click
          target: { text: "Continue to Payment" }

      checkpoint: shipping_complete
      next: payment

      on_error:
        - action: observe
          store: shipping_errors

    - name: payment
      url_pattern: ".*/checkout/payment.*"
      steps:
        # Select payment type
        - action: click
          target: { text: $payment_method.type }

        # Fill payment details
        - branch:
            if: { equals: [$payment_method.type, "Credit Card"] }
            then:
              - action: type
                target: { label: "Card Number" }
                text: $payment_method.card_number
              - action: type
                target: { label: "Expiration" }
                text: $payment_method.expiry
              - action: type
                target: { label: "CVV" }
                text: $payment_method.cvv

        # Place order
        - action: click
          target: { text: "Place Order" }

        - action: wait
          condition: { url_contains: "/confirmation" }
          timeout: 30s

      checkpoint: payment_complete
      next: confirmation

    - name: confirmation
      url_pattern: ".*/confirmation.*"
      extract:
        order_number:
          selector: "#order-number"
        estimated_delivery:
          selector: ".delivery-date"
        total:
          selector: ".order-total"
      next: end

success:
  conditions:
    - url_contains: "/confirmation"
    - visible: { text_contains: "Thank you" }
  extract:
    order_number:
      from: page
      selector: "#order-number"

failure:
  conditions:
    - text_contains: ["payment declined", "error", "failed"]
  recovery:
    - action: screenshot
    - action: observe
```

### Usage

```
> ecommerce_checkout --shipping_address '{
    "first_name": "John",
    "last_name": "Doe",
    "street": "123 Main St",
    "city": "Seattle",
    "state": "WA",
    "zip": "98101",
    "country": "United States"
  }' --payment_method '{
    "type": "Credit Card",
    "card_number": "4111111111111111",
    "expiry": "12/25",
    "cvv": "123"
  }' --promo_code "SAVE10"

ok ecommerce_checkout

# result
order_number: "ORD-789456"
estimated_delivery: "January 25, 2026"
total: "$89.99"
subtotal: "$99.99"
discount: "-$10.00"
```

## Best Practices

1. **Use descriptive page names** — Makes flow easier to understand

2. **Set appropriate URL patterns** — Be specific enough to avoid false matches

3. **Add checkpoints** — Enable resumption on failure

4. **Handle errors per-page** — Different pages may need different recovery

5. **Extract data progressively** — Capture important data at each step

6. **Use sub-intents** — Break complex steps into reusable intents

7. **Test incrementally** — Verify each page works before combining
