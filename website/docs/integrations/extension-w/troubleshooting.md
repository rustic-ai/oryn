# Oryn-W Troubleshooting

## 1) "Could not establish connection"

Typical cause: current page disallows content scripts or script not loaded.

Check:

- use `http://` or `https://` pages
- avoid `chrome://`, `chrome-extension://`, Web Store
- reload extension from `chrome://extensions`
- reload the target page

## 2) WASM status is error

Check:

- `extension-w/wasm/oryn_core_bg.wasm` exists
- rebuild with `./scripts/build-wasm.sh`
- review background service worker logs in `chrome://extensions`

## 3) Commands fail unexpectedly

Check:

- run `observe` first for element-targeted commands
- verify target text/ID is valid in current scan
- ensure you are on intended active tab

## 4) LLM status not configured

Check:

- open adapter config UI
- select available adapter
- add API key for remote adapters (OpenAI/Claude/Gemini)
- for Chrome AI, verify browser feature availability

## 5) Agent loops or stalls

Mitigations:

- give a more specific task
- reduce task scope
- clear prior history/trajectory data if needed
- switch to a stronger adapter/model

## 6) Extension loads, but E2E/CI-like local tests fail

When running with `act`, failures can come from container image parity (missing browser libs/tooling), not extension logic.

Use repo `act` defaults:

```bash
act -n pull_request -W .github/workflows/ci-js.yml
```

For runtime jobs, prefer full runner image mappings already configured in `.actrc`.

## 7) Scanner behavior mismatch

Source of truth is:

- `crates/oryn-scanner/src/scanner.js`

After scanner updates:

```bash
./scripts/sync-scanner.sh
./scripts/build-extension-w.sh
```

## Deep-Dive References

- `extension-w/TROUBLESHOOTING.md`
- `extension-w/DEV_GUIDE.md`
- `extension-w/LAUNCH_README.md`
