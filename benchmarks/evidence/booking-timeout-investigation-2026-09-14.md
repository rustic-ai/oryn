# Unresolved signed booking timeout

Status: **open; not reproduced in the separate recheck, not fixed**.

Candidate source SHA-256: `cdfdfe1ad8331cb75299cdd49336d270a24469f2eb277a8829bcf9588aa563de`.
Parent SHA-256: `93bb3c721e86aeb051c1a51d31887025c66508ffb574871a5f82fa15686330ae`.
Signed worker SHA-256: `ad7cc562866a2df0f09dd9a7a1cdf7565af8893344ac19b837a766162fb9fc70`.

The original [run](booking-signed-2026-09-14/summary.json) passed 17/18
planned attempts. `required_blocks`, sample 1, exited 1 after 30.120 seconds:
`Error: Runtime(WorkerLimitExceeded { resource: "wall time" })`.
The runner's separate 60-second deadline did not fire. Standard output was empty.
The persisted oracle recorded zero reviews, offers, reservation requests,
reservations, or authenticated-page views for that case. This does not prove
that the initial `/start` request was never received: that redirect is not logged
in the views table. OIL file outputs are buffered until execution completes, so
empty output alone also does not identify the failing command.

The CLI first loads an initial about:blank document, then executes the OIL file.
The existing error does not distinguish an IPC response deadline from a
worker-reported execution deadline, nor identify the command. The exact failing
phase and root cause remain unknown. Other workflow suites were running
concurrently, but resource contention has not been established as the cause.

A separate [recheck](booking-signed-isolated-recheck-2026-09-14/summary.json)
passed 18/18 with three concurrent booking jobs on the unchanged signed candidate.
Both runs match the signed source identity, preserve all planned cases, verify
artifact stability and independently reopen the final SQLite database. Fixture
shutdown was clean. The recheck does not replace the original denominator:
across these two runs the result is **35/36**, not failure-free qualification.

Next: distinguish transport deadlines from worker execution deadlines and attach
only a fixed command label/correlation number (never payloads, URLs, credentials,
or nonces). Reproduce under bounded mixed-suite concurrency and capture the
failing phase before proposing a runtime fix. Do not increase deadlines or
reclassify this failure as an environment block without evidence.
