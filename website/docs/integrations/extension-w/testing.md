# Oryn-W Testing

## Test Layers

`extension-w` uses three main test layers:

- Unit: `test/unit`
- Integration: `test/integration`
- E2E: `test/e2e`

There is also real WASM browser validation in `test/integration-real`.

## Core Commands

```bash
cd extension-w
npm install

npm run test:unit
npm run test:integration
npm run test:e2e
npm run test:all
```

Real WASM extension-context test:

```bash
npm run test:integration:real
```

## Scanner Package Checks

Because scanner behavior is shared across modes:

```bash
cd crates/oryn-scanner
npm install
npm run check
```

## CI Mapping

- `.github/workflows/ci-js.yml`
  - scanner-check
  - extension-test
- `.github/workflows/ci-e2e-quick.yml`
  - quick harness-backed smoke
- `.github/workflows/nightly-full-e2e.yml`
  - full backend variant matrix

## Coverage Expectations

`extension-w` Jest config defines thresholds:

- lines/statements: 80%
- functions: 75%
- branches: 70%

If coverage gates are not yet enabled in required checks, keep running local coverage to track drift:

```bash
cd extension-w
npm run test:coverage
```

## Suggested Validation Before Release

1. `./scripts/build-extension-w.sh`
2. `cd extension-w && npm run test:all`
3. `cd extension-w && npm run test:integration:real`
4. `./scripts/pack-extension-w.sh`
