# Oryn Test Harness

A Node.js/Express application serving multiple test scenarios for comprehensive Oryn testing.

## Quick Start

1. **Install dependencies**:
   ```bash
   npm install
   ```

2. **Start the server**:
   ```bash
   npm start
   ```
   The harness will be available at `http://localhost:3000`.

## Testing with Scripts

A collection of pre-defined Oryn scripts is available in the `scripts/` directory.

```bash
# Example: Run the e-commerce test
./target/debug/oryn-h --file scripts/03_ecommerce.oil
```

## Scenario Categories

- **Static Content**: Text extraction, table parsing.
- **Forms**: Input, validation, radio/checkbox interaction.
- **E-commerce**: Dynamic catalogs, cart state, checkout flows.
- **Interactivity**: Modals, React SPA, toasts.
- **Dynamic Content**: Infinite scroll, live search (debounced).
- **Edge Cases**: Iframes, Shadow DOM, Accessibility (ARIA).

## CI Usage

- PR/main smoke runs use `./scripts/run-e2e-tests.sh --quick` via `.github/workflows/ci-e2e-quick.yml`.
- Scheduled/manual full backend matrix runs via `.github/workflows/nightly-full-e2e.yml`.
