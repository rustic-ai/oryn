# WPT helper repair evidence

The shared-script syntax failure was reproduced as an actual HTTP 500 JSON
response from pinned wptserve. ConfigBuilder defaulted to SSL type `none`, which
silently removed HTTPS/WSS ports. Template expansion then raised IndexError.
The missing secondary ports and `www` template host were additional defects.

Changes:

- Enable pregenerated upstream TLS certificates in ConfigBuilder.
- Start two distinct public HTTP and HTTPS listeners; advertise their actual ports.
- Start real upstream WS/WSS handlers within the shared stash lifetime.
- Forward TLS requests to a private TLS wptserve listener, preserving request scheme.
- Provide `www`, `www1` and `www2` template entries.
- Serve static HTML through upstream MIME/charset/pipeline handling and insert the
  ASCII collector using a byte-preserving decode, rather than UTF-8 replacement.
- Select HTTPS for `.https.` source filenames.
- Fail preflight if five common scripts over both HTTP and HTTPS return an error,
  non-JavaScript MIME or unresolved host/port/location template tokens. Retain
  response hashes in each result.

Validation:

- 36 runner tests: 34 passed, two skipped (Node unavailable).
- Live upstream regression: ten shared-helper responses passed; WS and WSS echo
  endpoint handshakes passed; no stash handler import failures.
- Signed referrer canary: 3/3, previously 0/3 with syntax errors.
- Complete signed referrer-policy slice: **1,186/1,391**, compared with **11/1,391**
  in results-v2. Zero `Unexpected token ':'` harness failures remain. The 205
  failures comprise 199 assertion, five harness and one runtime outcome.
- Signed 20-execution CSP sample: 4/20; no shared-helper colon syntax errors remain.
  The sample is a smoke check, not a full CSP compatibility measurement.
- Python compilation and diff whitespace checks pass. Runtime binaries and their
  production limits were not changed.

Artifacts: `artifacts/wpt-full-baseline/repair-referrer-full.json`,
`repair-csp-sample.json`, `repair-canary.json` (failed intermediate attempt),
`repair-canary-v2.json` (passing canary). Original results-v2 remain untouched.
Each signed result records source, harness and binary fingerprints and dirty-tree
status. These are diagnostic results, not clean-tree release attestations.

The old 1,383 historical passes are not fully restored. Redirect/referrer assertion
mismatches, complete origin/DNS fidelity (including the legacy redirect mapping),
and remaining browser failures still require targeted diagnosis. Do not infer that
all previously affected Core cases now pass, or combine this slice with the old
baseline into a new aggregate. A full Core rerun is the next measurement.
