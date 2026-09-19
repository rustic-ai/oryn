# S-05 OIL caller and recovery source audit

September 17, 2026. Source generation only; no build, test, WPT or qualification executed. Broad S-05 remains open pending complete reconciliation and execution. Network-idle ownership remains with the root integration lane.

## Shared production paths reviewed

| Caller | Authority / outcome path | Source review result |
| --- | --- | --- |
| NativeOilSession text script | parse whole script, native target resolution/actions, completed outputs | Later failures now retain successful prefix, failed command index and cause; no automatic prefix replay |
| NativeContextOilSession | active native page, per-page session/frame selection, typed envelope/upload/page/text | All command classes pass through a shared receipt wrapper; upload failed attempts now receive an audit record |
| WorkerRequest::Oil | page session -> typed worker error | Appended OilExecutionFailed preserves outputs as JSON strings and native cause; deadline/root-cause descent retains nested action identity |
| CLI file/interactive | NativeContextOilSession -> JSONL output | Successful prefix prints before the error; no retry of completed actions |
| MCP oryn_perform | versioned IntentRequest -> same context/session | Structured native error accompanies diagnostic text; action IDs survive the adapter |
| Full-contract five-family executable | single-command recipe steps -> context session | Ordinary/recovery/duplicate outcomes retain native_error and prefix outputs; existing expected-error and timeout policy remains |
| Legacy forms/shopping/booking/SaaS/research runners | CLI JSONL + persisted stdout/stderr | Nonzero outcomes remain failures and raw successful prefix evidence is persisted; adapters do not turn partial completion into success |
| Native frame commands | typed selected-frame identity -> native frame algorithm | OIL uses native_frame_command to retain typed transport errors; existing WebDriver caller still receives its prescribed error projection |
| Target read / observation diff | shared native resolver -> current observation | Implicit refreshed observation and delta cursor now advance together |

## Existing source protections retained

Declarative action targets retry once only for a pre-dispatch StaleSemanticRef; aliases never re-resolve across replacement. ActionExecutionFailed wraps post-dispatch failures so they cannot masquerade as safe stale retries. Native batch errors retain successful action receipts. Completed action receipts survive failed follow-up observations; alias caches clear for uncertain outcomes. Root navigation clears old frame scope/aliases before admission. Extraction uses the shared native structured/text/HTML serializer, not authored scripts.

## Five-family routing reviewed

`workflow_family_recipes.py` supplies forms, shopping, authenticated SaaS and booking recipes. `workflow_native_adapter.py` supplies research recipes and shared mixed-family execution; it builds single-line native steps and consumes successful output records from primary/recovery/replay evidence. `run-oil-full-contract.py` freezes candidate/generation identity and uses that adapter. `oryn-workflow-qualification.rs` validates single-line steps, retains original native references for stale-identity cases, and now retains typed failures. Individual `run-oil-*-outcomes.py` tools preserve stdout on failure and do not qualify failed runs from prefix outputs.

## Generated regressions

- multiline_failure_preserves_prefix_and_does_not_replay_actions
- context_prefix_covers_page_typed_and_upload_parse_failures
- first_command_failure_preserves_native_classification
- oil_deadline_preserves_completed_output_and_typed_cause
- frame_transport_failure_keeps_native_terminal_cause
- target_read_refresh_keeps_alias_snapshot_and_delta_cursor_together
- native_failure_evidence_retains_partial_receipts_and_action_identity

## Remaining gates

This audit does not claim exhaustive recipe-by-recipe recovery qualification or signed worker compatibility. Combined protocol reconciliation, compilation and all execution remain deferred by the global generation directive. Full S-05 includes network-idle ownership and dynamic frame replacement/observation freshness across the complete frozen attempt inventory; existing bounded sources and these additions must be assessed together before closing the parent row.

## Legacy dismissal scope finding

`Command::Dismiss` still searches visible close/cancel/dismiss controls globally and does not constrain candidates to its authored target. Static source search found callers in legacy `test-harness/scripts/{02_forms,07_intents_builtin,09_target_resolution,12_popup_detection}.oil`, including geometry-based popup detection cases. No `dismiss` command occurs in the current frozen five-family runner/recipe sources inspected here. This behavior is not claimed complete by this audit; resolving legacy arbitrary visual popup detection requires the native overlay/geometry target contract, rather than silently treating every visible cancel button as the requested popup. The remaining source review must decide whether that legacy contract is a required supported target before broad S-05 closure.
