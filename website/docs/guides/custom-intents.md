# Custom Intents

This guide explains how to define your own intent commands for repeated workflows.

## Why Custom Intents?

Custom intents let you:

- Encapsulate repeated workflows into single commands
- Share automation patterns across sessions
- Build site-specific automation libraries
- Create semantic commands for your use case

## Session Intents

### Defining an Intent

Use the `define` command to create a session-scoped intent:

```
define add_to_cart:
  description: "Add current product to shopping cart"
  steps:
    - click "Add to Cart" or click "Add to Basket"
    - wait visible { text_contains: [added, cart] }
  success:
    - text_contains: [added, cart, basket]
```

### Using the Intent

```
> goto example.com/products/widget
> add_to_cart
ok add_to_cart

# actions
click [5] "Add to Cart"
wait visible "Added to cart"
```

### Step Syntax

Steps support a simplified syntax:

**Multiple fallback targets:**
```yaml
steps:
  - click "Add to Cart" or click "Buy Now" or click "Add"
```

**Wait conditions:**
```yaml
steps:
  - wait visible "Success"
  - wait hidden "Loading"
```

**Type shortcuts:**
```yaml
steps:
  - type email "user@test.com"
  - type password "secret"
```

## Parameterized Intents

### Defining with Parameters

```
define review_product:
  description: "Submit a product review"
  parameters:
    - rating: number, required
    - text: string, required
  steps:
    - click "Write a Review"
    - wait visible review_form
    - click star[$rating]
    - type review_text $text
    - click "Submit Review"
  success:
    - text_contains: "review submitted"
```

### Using Parameters

```
> review_product --rating 5 --text "Excellent product!"
ok review_product

# actions
click [7] "Write a Review"
wait visible review_form
click [12] (5 stars)
type [15] "Excellent product!"
click [18] "Submit Review"
```

### Parameter Types

| Type | Description | Example |
|------|-------------|---------|
| `string` | Text value | `--name "John"` |
| `number` | Numeric value | `--rating 5` |
| `boolean` | True/false | `--subscribe true` |
| `object` | JSON object | `--data '{"a": 1}'` |
| `array` | JSON array | `--items '["a", "b"]'` |

## YAML Intent Definitions

For persistent intents, create YAML files in `~/.oryn/intents/`:

### Basic Intent

```yaml
# ~/.oryn/intents/subscribe_newsletter.yaml

intent: subscribe_newsletter
version: "1.0"
description: "Subscribe to the site newsletter"

steps:
  - action: click
    target: { text_contains: ["Subscribe", "Newsletter"] }

  - action: wait
    condition: { visible: { text_contains: "email" } }

  - action: type
    target: { role: email }
    text: $email

  - action: click
    target: { role: submit }

success:
  conditions:
    - text_contains: ["thank you", "subscribed", "confirmed"]
```

### Intent with Fallbacks

```yaml
# ~/.oryn/intents/dismiss_cookie.yaml

intent: dismiss_cookie
version: "1.0"
description: "Dismiss cookie consent with fallbacks"

steps:
  - action: click
    target: { text: "Accept All" }
    on_error:
      - action: click
        target: { text: "Accept" }
      - action: click
        target: { text: "OK" }
      - action: click
        target: { text: "Got it" }
```

### Intent with Conditions

```yaml
intent: checkout_if_cart
version: "1.0"
description: "Proceed to checkout if cart has items"

steps:
  - action: observe
    store: page_state

  - branch:
      if: { pattern_exists: empty_cart }
      then:
        - action: error
          message: "Cart is empty"
      else:
        - action: click
          target: { text: "Checkout" }

        - action: wait
          condition: { url_contains: "/checkout" }
```

## Site-Specific Packs

Organize intents into site-specific packs:

### Pack Structure

```
~/.oryn/packs/
└── amazon.com/
    ├── pack.yaml
    ├── patterns.yaml
    └── intents/
        ├── add_to_cart.yaml
        ├── checkout.yaml
        └── track_order.yaml
```

### Pack Metadata

```yaml
# pack.yaml

pack: amazon.com
version: "1.0.0"
description: "Intent pack for Amazon"

domains:
  - amazon.com
  - www.amazon.com

auto_load:
  - "https://amazon.com/*"
  - "https://www.amazon.com/*"
```

### Site-Specific Patterns

```yaml
# patterns.yaml

patterns:
  product_page:
    detection:
      url_contains: "/dp/"
    elements:
      title:
        selector: "#productTitle"
      price:
        selector: ".a-price-whole"
      add_to_cart:
        selector: "#add-to-cart-button"
      buy_now:
        selector: "#buy-now-button"

  cart:
    detection:
      url_contains: "/cart"
    elements:
      checkout:
        selector: "#sc-buy-box-ptc-button"
      quantity_inputs:
        selector: ".sc-quantity-textfield"
```

### Site Intent

```yaml
# intents/add_to_cart.yaml

intent: add_to_cart
version: "1.0"
pack: amazon.com

triggers:
  patterns:
    - product_page
  urls:
    - "*/dp/*"

parameters:
  - name: quantity
    type: number
    default: 1

steps:
  - branch:
      if: { gt: [$quantity, 1] }
      then:
        - action: select
          target: { id_contains: "quantity" }
          value: $quantity

  - action: click
    target: { pattern: product_page.add_to_cart }

  - action: wait
    condition:
      any:
        - text_contains: "Added to Cart"
        - url_contains: "/cart"

success:
  conditions:
    - text_contains: ["Added to Cart", "added to your"]
```

## Managing Intents

### List All Intents

```
> intents
Built-in intents:
  - login
  - logout
  - search
  - accept_cookies
  - dismiss_popups
  - fill_form
  - submit_form
  - scroll_to

Loaded intents:
  - subscribe_newsletter (user)
  - add_to_cart (amazon.com)

Session intents:
  - review_product
```

### List Session Intents

```
> intents --session
Session intents:
  - review_product (defined 5 mins ago)
  - quick_checkout (defined 2 mins ago)
```

### Remove Session Intent

```
> undefine review_product
ok undefine review_product
```

### Clear All Session Intents

```
> intents --clear-session
ok cleared 2 session intents
```

## Exporting Intents

### Export to File

```
> export review_product --out ~/.oryn/intents/review_product.yaml
ok export review_product

# written
~/.oryn/intents/review_product.yaml
```

The exported intent will be automatically loaded on future sessions.

## Best Practices

### 1. Use Descriptive Names

```yaml
# Good
intent: add_product_to_wishlist

# Avoid
intent: apw
```

### 2. Include Descriptions

```yaml
intent: checkout_with_express
description: "Complete checkout using express/saved payment method"
```

### 3. Provide Fallback Targets

```yaml
- action: click
  target: { text: "Submit Order" }
  on_error:
    - action: click
      target: { text: "Place Order" }
    - action: click
      target: { role: submit }
```

### 4. Verify Success

```yaml
success:
  conditions:
    - url_contains: "/confirmation"
    - text_contains: "order confirmed"
```

### 5. Handle Errors Gracefully

```yaml
failure:
  conditions:
    - text_contains: ["error", "failed", "invalid"]
  recovery:
    - action: screenshot
    - action: observe
```

### 6. Document Parameters

```yaml
parameters:
  - name: shipping_speed
    type: string
    required: false
    default: "standard"
    description: "Shipping speed: standard, express, or overnight"
```

## Example: Complete Checkout Intent

```yaml
intent: complete_checkout
version: "1.0"
description: "Complete checkout with shipping and payment"

parameters:
  - name: shipping
    type: object
    required: true
    description: "Shipping address object"
  - name: payment
    type: object
    required: true
    description: "Payment details object"

flow:
  start: cart
  pages:
    - name: cart
      url_pattern: ".*/cart.*"
      steps:
        - action: click
          target: { text: "Proceed to Checkout" }
      next: shipping

    - name: shipping
      url_pattern: ".*/checkout/shipping.*"
      steps:
        - action: fill_form
          data: $shipping
        - action: click
          target: { text: "Continue to Payment" }
      next: payment

    - name: payment
      url_pattern: ".*/checkout/payment.*"
      steps:
        - action: fill_form
          data: $payment
        - action: click
          target: { text: "Place Order" }
      next: confirmation

    - name: confirmation
      url_pattern: ".*/confirmation.*"
      extract:
        order_number:
          selector: "#order-number"
        total:
          selector: ".order-total"
      next: end

success:
  conditions:
    - url_contains: "/confirmation"
  extract:
    order_number:
      from: page
      selector: "#order-number"

failure:
  conditions:
    - text_contains: ["payment declined", "error"]
```
