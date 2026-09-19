# S-05 OIL identity and freshness source audit

Source generation only; no execution or acceptance claim.

## Confirmed gaps closed in this pass

- Numeric aliases are DOM slot-derived and previously resolved against the current
  graph. A document replacement could silently reuse an observed alias. Action
  resolution now replaces alias leaves, including relational anchors, with exact
  SemanticRef values from the retained observation before native resolution.
  Oversized alias values are checked before conversion to u32. Missing observations
  or missing aliases fail rather than using a current slot.
- Forced/simple geometry/frame selection aliases compare the current native
  observation against the original SemanticRef. Text extraction resolves a target
  reference first and reads only that reference. Wait polling freezes its alias
  reference before subsequent observations. Stale recovery excludes any target
  containing an alias, including nested relation anchors.
- Typed non-forced targets now preserve nested native Reference values directly
  rather than converting them through the OIL AST (whose atomic nodes lack a native
  reference variant).
- Title and URL query current page metadata. These metadata reads do not silently
  replace the observation used to bind numeric aliases.
- Composed AX frame capture previously rechecked only root revision and frame
  lifetimes. It now also rechecks each captured child generation and revision;
  unchanged frame inventory alone cannot certify same-document child freshness.

Sources: `oil.rs` resolution/action/text/wait/metadata callers and
`accessibility_platform/frames.rs::capture`. Removed superseded text target matcher.
Generated regression sources cover old-document alias rejection after replacement
and metadata read, retained nested reference anchors, declarative new-document
resolution, and child mutation/generation mismatch during composed capture.

## Native action-optional resolver integration

The confirmed forced/relational/selector gap above is now addressed by
`WorkerRequest::ResolveTarget` → worker dispatch / selected-frame read-only
transport → PageCommand actor → PageState::resolve_target →
SemanticGraph::resolve_identity. `action=None` selects an identity without
claiming actionability. Native action execution still validates the reference,
current document and actual permitted operation. Existing ResolveSemantic callers
retain action filtering and now share native selector evaluation.

CSS/XPath leaves, including relation anchors, are evaluated against the exact
selected native binding/DOM through RustExecutableDocument → RawV8Host →
V8BindingState::native_target_selector. CSS uses the existing Stylo provider,
preserving selector syntax, quoted commas, custom-element states and native focus.
XPath uses the native evaluator and requires a node set. Native source snapshots
provide a non-V8 fallback. No author JavaScript evaluates or mutates application
state. The old lossy typed-target-to-OIL conversion and observation-string matcher
were removed. Visibility waits use the same native resolver, preserving relational
constraints and bound aliases across polling.

Generated regression sources cover CSS selectors with quoted commas inside
relational scopes, equivalent XPath identity, typed nested native references in
forced resolution, malformed selectors/non-node XPath, no-action selection versus
real action rejection, selected child resolution, and wire round trips with an
optional action. The new request belongs to the existing **pending worker14**
batch; context45/agent14 are unchanged. All current exact-version consumers read
native contracts; no historical artifacts, locks or signed evidence were rewritten.
New candidate builds/source hashes must include this request before execution.

## Remaining requirement-specific boundary

No runtime qualification is claimed. Cross-call snapshots cannot promise an atomic
multi-frame view; this pass adds per-child revision validation, not a global
mutation lock. Existing selected-frame action/extraction and detached-frame
regressions remain source evidence. Full S-05 parent closure needs root's final
cross-package generation reconciliation, including active navigation/load changes;
actual five-family outcomes and acceptance remain unqualified.


Extraction integration follow-up: PageState::extract now obtains selector roots
through the same native selected-document CSS matcher. Invalid selectors return
errors instead of silently producing empty scopes. Scope assembly preserves
current document order and reduces overlapping roots to their outermost members,
so text/HTML extraction does not duplicate nested content. Added quoted-comma,
malformed-selector and overlapping-root extraction assertions to the native
identity regression. No runtime execution performed.
