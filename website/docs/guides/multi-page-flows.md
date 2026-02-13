# Multi-Page Flows

How to run multi-step workflows across pages with current Oryn support.

## Current Status

Design docs and engine modules include richer flow/intents concepts, but the unified CLI does not currently expose full declarative flow YAML execution.

## Recommended Pattern Today

Model multi-page workflows as `.oil` scripts and run them with:

```bash
oryn --file flow.oil headless
```

## Example Checkout-Like Flow

```text
goto https://shop.example.com/cart
observe
click "Checkout"
wait navigation
observe

# shipping page
type "Address" "123 Main St"
type "City" "Austin"
select "Country" "United States"
click "Continue"
wait navigation
observe

# payment page
type "Card number" "4111111111111111"
type "Expiry" "12/30"
type "CVV" "123"
click "Pay"
wait navigation
observe
```

## Reliability Tips

1. Re-run `observe` after each navigation.
2. Prefer semantic targets (`"Continue"`, `email`, `submit`) over fragile numeric IDs.
3. Use explicit waits (`wait load`, `wait visible ...`, `wait url ...`) between transitions.
4. Capture evidence with screenshots:

```text
screenshot --output ./step-payment.png
```

## Recovery Pattern

When a step fails, split the flow into smaller files and rerun from the failed stage:

- `flow-01-cart.oil`
- `flow-02-shipping.oil`
- `flow-03-payment.oil`

This gives practical checkpointing without relying on unfinished flow DSL features.
