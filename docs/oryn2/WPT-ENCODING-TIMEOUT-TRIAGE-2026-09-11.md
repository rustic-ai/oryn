# Encoding form timeout: bounded diagnostic

This is an engine-investigation lead, not a qualified encoding conformance result.
The full audited baseline is separate and must finish before acceptance.

## Observations

Evidence: `artifacts/wpt-full-baseline/audited-retirement-encoding-chromium-1.json`.
The selected canonical execution is
`encoding/legacy-mb-japanese/euc-jp/eucjp-encode-form-cseucpkdfmtjapanese.html|window|?1-1000`.

- Oryn reached its declared long outer deadline after 120,003 ms without a
  completion report. Cleanup and worker replacement succeeded.
- Chromium completed with harness status 0 and all 1,000 subtests passing on
  the same served fixture. This does not measure isolated browser performance.
- Oryn recorded complete HTTP observations for the document, seven scripts,
  and one `/common/blank.html` request. That final request was GET with no query.
- `Content-Type: text/html; charset=cseucpkdfmtjapanese` comes from the pinned
  test's `.html.headers` sidecar. It is not a charset invented by the harness.

## What the fixture exercises

The reviewed pinned `encoding/resources/encode-form-common.js` helper creates
two named iframes and GET forms, sets `acceptCharset`, and submits code-point
chunks of 400 characters to `/common/blank.html`. Each form targets an iframe.
After the top-level load event starts submission, iframe load handlers inspect
their resulting URL queries and complete the corresponding asynchronous tests.

Consequently, completion depends on form serialization, named-target navigation,
iframe location visibility and load-event delivery, in addition to the encoder.
The current evidence does **not** isolate an incorrect encoding algorithm or
prove that 1,000 encoding assertions failed: Oryn produced no terminal subtest
results. Investigate that form/frame/load-event path first, using a small
standalone regression before changing codec behavior.

No engine change or upstream fixture modification was made for this diagnostic.
Do not substitute this separate comparison for any full-cohort execution.
