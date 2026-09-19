# S requirement/source audit — generation in progress

This maps source ownership and remaining work. No build/test/generator/conformance execution has occurred in this generation phase.

| Requirement | Authoritative source paths | Generation obligations |
|---|---|---|
| S-01 WebIDL/DOM/Range/Selection/XPath | `oryn-webidl/idl/browser.webidl`; `oryn-js/src/v8_bindings.rs`, `selection_movement.rs`, `range_geometry.rs`; `oryn-dom/src/lib.rs`, `xpath/` | XPath was absent and is now being implemented natively. Native parser/evaluator, V8 interfaces, namespace-node wrappers, per-document iterator tokens, resolver reentrancy, detached Attr contexts and regression sources are generated. Namespace EventTarget, reverse Node comparison and first-nonempty logical text mapping are also generated. Remaining source review includes temporary resource accounting, result-reuse allocation failures and adversarial realm retirement; see S-XPATH-GENERATION-NOTES-2026-09-16.md. Global WebIDL regeneration intentionally deferred. Native Range crash/resource reproductions remain qualification obligations. |
| S-02 components/shadow/slots | `v8_bindings/custom_elements.rs`, `oryn-dom/src/slots.rs`, native FACE registry, native accessibility projection | Definition/upgrade/reaction/adoption, FACE state and default ARIA generated; history/late-upgrade restoration integrated. Full external Core/custom-element matrix remains unexecuted. |
| S-03 form submission | `v8_bindings/form_activation.rs`, `form_data.rs`, `custom_elements.rs`; DOM form controls; native history/form state | Native ownership/reentrancy/files/image coordinates and FACE submission/restoration generated; end-to-end persisted family oracles unrun. Check XML typed ID declarations as XPath evaluation integration proceeds. |
| S-04 trusted input/editing | native keyboard/pointer/drag/clipboard modules; native action scheduler and host | Production authority separated from historical helpers. Nineteen native host regression sources and scheduler release/admission timing sources cover historical behavior families; native pointer/touch owner continues pan/gesture defaults. Historical Promise helper is diagnostic only: production admission returns before dispatch, stale admitted work rejects before focus. |
| S-05 OIL semantics | native OIL apply-action/semantic paths and context/frame identity | Parent owns complete native OIL action integration and freshness. Five-family workflow sources use genuine OIL targets and durable SQLite oracles, not page-script shortcuts. Remaining end-to-end tests are deferred. |

## Five-family overlap

- Forms: real file bytes, constraint/default-value provenance, custom controls/FACE, keyboard activation, native frame focus and persisted submission.
- Shopping: selection/editor behavior, framed payment input, trusted pointer defaults and idempotent durable outcomes.
- Booking: native focus/selection, timezone fixture contracts, lifecycle cancellation/restart and unknown-commit outcome reconciliation.
- Research/extraction: namespace-aware querying, native extraction, document/selection ownership, observation freshness and heterogeneous concurrent contexts.
- Authenticated SaaS: portal/custom-element ownership, rich editable input/clipboard/drag, file fidelity, auth recovery and cross-context isolation.

Workflow versions, behavior-cell adapters, durable gates, SQLite evidence and mixed native context accounting remain in the existing F/Q generation notes. Source generation is not evidence that any family or Core cohort currently passes.
