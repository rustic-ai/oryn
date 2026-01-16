# Lemmascope Test Scripts

This directory contains a suite of `.lemma` scripts designed to exercise the Lemmascope backends against the [Test Harness](../index.html).

## Scripts Overview

| File                     | Scenario         | Key Commands Tested                                 |
| ------------------------ | ---------------- | --------------------------------------------------- |
| `01_static.lemma`        | Static Content   | `goto`, `observe`, `extract text`, `extract tables` |
| `02_forms.lemma`         | Forms            | `type`, `check`, `select`, `dismiss`                |
| `03_ecommerce.lemma`     | E-commerce       | `near` modifier, `select`, `observe`                |
| `04_interactivity.lemma` | UI Interactivity | `wait visible`, `click`, `check`, SPA navigation    |
| `05_dynamic.lemma`       | Dynamic Content  | `scroll`, `wait visible`, `clear`                   |
| `06_edge_cases.lemma`    | Edge Cases       | `shadow-dom`, `accessibility`, `role` targeting     |

## Running Scripts

To run these scripts, use any of the Lemmascope backends (`lscope-h`, `lscope-e`, or `lscope-r`).

### Headless Mode (lscope-h)
```bash
cargo run --bin lscope-h --file scripts/01_static.lemma
```

### REPL Mode
You can also copy-paste commands from these scripts into the REPL:
```bash
cargo run --bin lscope-h
> goto "http://localhost:3000/static/article.html"
> observe
```

## Prerequisites
Ensure the test harness server is running:
```bash
cd test-harness
npm start
```
