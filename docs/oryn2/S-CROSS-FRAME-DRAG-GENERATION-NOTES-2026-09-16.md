# Cross-frame native drag generation

Source generation only; no build, tests, formatter or generator ran.

## Implementation

- Native endpoint objects carry exact document/generation/stream-epoch and V8 context ownership. Parent-provided embedding links additionally validate connected native frame ownership and removal/reinsertion journals during callbacks.
- The original source Session remains native. Destination events get event-scoped DataTransfer wrappers with protected/read-only modes, native trusted UIEvent slots and the destination Window as `view`; no serialized author drag capability is exposed.
- Drag/enter/leave/over/drop/end ordering transitions from an existing local target to child/sibling frames and back. Previous-target coordinates come from the host's current transformed realm mapping. Escape uses the saved target context for leave before source end.
- Same-origin selected control/editor moves validate the original source after each beforeinput callback, then mutate both documents without callbacks inside a paired transaction. Failure restores both native snapshots. Paired undo/redo works from either document/control and rejects retired/adopted owners or missing companion history.
- Cross-origin copy/link delivery is allowed; native cross-origin Move is rejected and never deletes original source. Related target is null across origins. Native editing HTML resolves against the source URL and uses the shared sanitizer.
- Payload item/byte and editing snapshot budgets are enforced. Drop/cancel/error/retirement detach payload and clear raster feedback; host owns terminal pointer/capture cleanup.

## Regression source

Native tests cover event scope/readability, target realm view/trust, payload detachment, paired two-document rollback/undo/redo, retired companion rejection, ancestor removal/reinsertion, and explicit/native draggable ancestry. Parent generated physical sibling-frame routing/coordinate and terminal-pointer tests.

## Remaining integration scope

Host/router changes are parent-owned and unverified. Auxiliary browsing contexts in separate worker processes are distinct from same-page frame transport. Native cross-origin Move restriction is explicit runtime policy; conformance outcomes require execution. Legacy author-visible trusted-input hooks are the next migration task; they must not be enabled in default production. Custom canceled drop handlers retain responsibility for their own application edits; native selected-source deletion runs only for an accepted native move default.
