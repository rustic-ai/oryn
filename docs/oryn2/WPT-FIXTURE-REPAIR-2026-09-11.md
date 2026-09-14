# WPT fixture repair and versioned evidence

## Result

The affected-slice run completed 862 selected IDs across 50 shards in 520.6
seconds. It produced 288 passes, 557 failures and 17 environment-blocked records.
Compared with the same IDs in the completed Core baseline, 243 failures became
passes, 17 became environment-blocked, 557 remained failures, and all 45 passing
controls remained passing. No browser implementation, signed binary, worker mode,
or production limit changed.

The versioned composite accounts for all 13,747 original Core IDs: 3,861 pass,
9,869 fail and 17 environment-blocked. Attempted denominator: 13,730. Core's
2,990 unsupported runner entries remain separate, not removed from the overall
coverage inventory or counted as browser failures.

WebSockets improved from 0/733 to 240/733. The independently qualified HTTP/2
fixture does not imply that Oryn implements HTTP/2 WebSockets; transport and
browser failures remain visible. Colon-syntax harness errors fell from 589 to 37.

## Implemented repair

- Use pinned upstream subdomain and invalid-subdomain definitions, including
  internationalized names and their products, while retaining localhost routing.
- Serve actual TLS HTTP/2 directly through upstream's HTTP/2 and WebSocket
  extended-CONNECT handlers; advertise the real listener port, not an HTTPS alias.
- Qualify ALPN h2, ENABLE_CONNECT_PROTOCOL, extended CONNECT, text echo and close
  with an independent Python h2 client. The probe rejects an HTTP/1 HTTPS listener.
- Expand mandatory helper checks from five to eight resources over both HTTP and
  HTTPS, including the formerly broken WebSocket, secure-context and CSP helpers.
- Discover literal script-tag, META-script and importScripts dependencies
  recursively, preserving query strings. Record HTTP status, MIME, body hash and
  unresolved-token observations for candidate JavaScript dependencies.
- Block recognizable upstream template exceptions before browser execution;
  other unexpected responses remain observations, not automatic browser diagnoses.
  Expected HTTP errors, malformed JS and literal template text do not by themselves
  trigger the template-failure classifier. Malformed URLs do not abort discovery.
- Mark the 17 selected local-network-access cases environment-blocked because
  distinct local/public IP address spaces are unavailable. Do not fake those
  address spaces with loopback listeners.
- Capture recognizable template-error responses encountered during public proxy
  execution and attach them when diagnostic resource URLs support attribution.
- Record distinct diagnostic tags for harness deadlines, worker wall limits,
  generated-operation limits, outer timeouts, reported crashes and explicit
  unsupported testdriver operations. Extend timeout rerun selection accordingly.
- Preserve raw outcomes and diagnostic domains. Environment-blocked records have
  separate counters and do not increment the controller's browser-failure count.
- Register HTTP/TLS/H2/WS/WSS cleanup within a shared lifetime; retain the public
  request-thread drain introduced in the prior shutdown repair.

## Reproducible evidence

`scripts/wpt-repair-evidence.py prepare` derives an exact-ID repair inventory from
the baseline's recorded failures, identifiable dependency consumers, the complete
WebSocket slice and one passing control per available root/global pair.
`merge` rejects missing/extra replacement IDs, duplicate source IDs, changed
source hashes and changed parent/worker binaries. It never overwrites old results.

Artifacts under `artifacts/wpt-full-baseline/`:

- `fixture-repair-v5-inventory/plan.json`: selection reasons and affected resources.
- `fixture-repair-v5-results/summary.json`: complete affected-slice evidence.
- `fixture-repair-v5-composite/ledger.jsonl`: original and replacement source
  paths, hashes and fingerprints for each of the 13,747 IDs.
- `fixture-repair-v5-composite/summary.json`: reconciled totals and transitions.

All exact IDs and the ledger hash were checked after merge. This is an explicitly
multi-harness, dirty-tree diagnostic composite, not a single-version release
attestation or a claim of complete browser conformance. Old evidence remains intact.

## Validation and remaining limitations

55 harness tests ran: 53 passed, two Node-dependent tests skipped. Live tests cover
HTTP/TLS helper substitution, generated wrappers and variants, cross-host dispatcher
delivery, WS/WSS handshakes, genuine HTTP/2 echo/close, rejection of HTTP/1 as HTTP/2,
and waiting for an active public request handler during shutdown. Unit tests cover
timeout classification, expected-error preservation, recursive dependencies and
cycles, source-hash validation and duplicate-ID rejection. All 50 affected shard
processes exited normally; diff checks pass.

The remaining 37 colon errors are HTML 27, Trusted Types 3, import maps 2, and one
each in accessibility names, CSP, input events, Shadow DOM and XHR. Some scripts
return 404 JSON, including `/resources/wpt_test_util.js`, which is absent from the
pinned checkout. Do not fabricate substitute helpers or assume these are missing
browser APIs. Resource-resolution bugs, upstream fixture defects, intentional
errors and browser handling of failed script fetches require individual review.

Static discovery is best-effort, not a full JavaScript dependency graph. Dynamic
URLs, alternate execution paths and intentional invalid dependencies remain
limitations. The mandatory helper list is deliberately reviewed; incidental
candidate HTTP/MIME failures are retained for review rather than blindly blocked.
Only explicit testdriver failures are annotated, so automation limitations remain
a lower-bound count.

Composite diagnostic tags overlap outcome categories: 1,325 harness timeouts,
1,821 worker wall limits, 13 generated-operation limits, six outer timeouts,
26 explicit testdriver limitations and seven reported worker crashes. These are
not additive bug counts. Next priorities are the residual script-loading cases,
then browser transport/timeouts and the previously identified frame/navigation
compatibility work; no blanket deadline increase is justified by this repair.
