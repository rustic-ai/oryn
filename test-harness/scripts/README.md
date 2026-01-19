# Oryn Test Scripts

This directory contains a suite of `.oil` scripts designed to exercise the Oryn backends against the [Test Harness](../index.html).

## Scripts Overview

| File                   | Scenario         | Key Commands Tested                                 |
| ---------------------- | ---------------- | --------------------------------------------------- |
| `01_static.oil`        | Static Content   | `goto`, `observe`, `extract text`, `extract tables` |
| `02_forms.oil`         | Forms            | `type`, `check`, `select`, `dismiss`                |
| `03_ecommerce.oil`     | E-commerce       | `near` modifier, `select`, `observe`                |
| `04_interactivity.oil` | UI Interactivity | `wait visible`, `click`, `check`, SPA navigation    |
| `05_dynamic.oil`       | Dynamic Content  | `scroll`, `wait visible`, `clear`                   |
| `06_edge_cases.oil`    | Edge Cases       | `shadow-dom`, `accessibility`, `role` targeting     |

## Running Scripts

To run these scripts, use any of the Oryn backends (`oryn-h`, `oryn-e`, or `oryn-r`).

### Headless Mode (oryn-h)
```bash
cargo run --bin oryn-h --file scripts/01_static.oil
```

### REPL Mode
You can also copy-paste commands from these scripts into the REPL:
```bash
cargo run --bin oryn-h
> goto "http://localhost:3000/static/article.html"
> observe
```

## Prerequisites
Ensure the test harness server is running:
```bash
cd test-harness
npm start
```
