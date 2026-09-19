# Cache retirement custody — source review, 2026-09-16

Generated source only; no tests, builds, formatters, generators, services or
qualification were run. Whole-plan acceptance remains open.

## Generated paths

- `cache_storage::retire` latches retirement, takes pending tasks without holding
  the registry borrow across cleanup, and transfers captured body leases to their
  exact surviving source realm. Native HTTP stream abandonment starts immediately.
- Private `DeferredPlatformTask::CancelBodyLease` retains the original wrapper and
  source identity until the source checkpoint. `cancel_lease` enters the lease's
  captured source context and isolates cancellation exceptions. A destination
  object with the same numeric stream ID cannot receive that cancellation.
- `PendingBatch::drop` restores work only while the destination remains live.
  Retirement during conversion transfers current and unvisited body cleanup;
  both success and error exits avoid requeueing after retirement.
- The batch tracks its reserved task count. In-flight operations remain included
  in the 128-operation admission bound during reentrant author callbacks.
- Production polling stops after callback retirement, before later item conversion
  or PutAll. Cache calls check retirement before conversion, before each fetch,
  and before pending registration. Parent transport also checks before publishing
  secure context / dispatch.
- `cancel_items` isolates each signal and body cancellation independently, so one
  cleanup failure cannot poison later resource cleanup or the original rejection.

## Regression sources

`foreign_request_tests` now contains exact foreign lease cleanup with a same-ID
local decoy and restored caller context; callback retirement plus conversion error
covering current/unvisited cleanup, no revival, and released in-flight reservation;
and invalid signal/controller cleanup followed by successful later body cleanup.
The existing conversion-error RAII regression remains intact.

## Scope and validation still open

Retiring a realm does not run author code inline. Its own transport retirement
owns local native requests; surviving foreign realms receive deferred cancellation.
No new thread, broad authority, or synthetic completion is introduced. Physical
cleanup and all regression behavior still require execution after global generation.
