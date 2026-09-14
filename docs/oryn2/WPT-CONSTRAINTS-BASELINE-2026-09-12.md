# Native constraint-validation baseline

Full Core conformance remains incomplete. This baseline includes every one of
the 28 mapped Core testharness executions under html/semantics/forms/constraints
in inventory-v3. No outcomes influenced selection. Unsupported/manual entries
remain unfinished in the full Core ledger, not exemptions.

The same signed fieldset candidate ran all 28 executions with a persistent
parent and isolated prewarmed workers: 217.843 seconds, no environment-blocked
executions, 0 passing files, 5 passing and 880 failing observed subtests.
The 885 observed subtests are not a complete denominator: six files abort early.

Raw artifact: `artifacts/wpt-full-baseline/form-constraints-v1-2026-09-12.json`.
SHA-256: `4f751f453262469df1a7cf0b4ad9a52e220f07aa63dc6f779936cde6ac467062`.
Candidate identities are in that artifact and the
[fieldset checkpoint](WPT-FIELDSET-COLLECTIONS-2026-09-12.md).
This is dirty-tree diagnostic evidence, not a release attestation.

## Evidence-driven repair order

1. **Prerequisite API completion:** implement native Option construction and
   option/select state integration. checkValidity, reportValidity, customError,
   valid, valueMissing and willValidate files all abort with `Option is not defined`.
   Preserve those failures; do not catch the exception in the harness or infer
   absent subtests. inputwillvalidate has a separate incomplete Chromium reference
   focus/completion observation and must be re-evaluated.
2. **Shared validity state:** expose live, branded, SameObject ValidityState and
   native custom-error state for relevant controls. Implement candidate eligibility,
   disabled/fieldset/datalist/readonly rules and current form ownership. Avoid
   presenting unimplemented flags as universally false.
3. **Validity algorithms:** implement required values and radio groups, type and
   pattern mismatch, numeric/date/week/month ranges and step bases, too-long/short
   user-edit semantics and bad input. Integrate sanitization and current values;
   do not calculate from stale attributes alone. Test dynamic mutation, disconnected
   trees and form-owner changes, and preserve UTF-16 versus scalar-value rules.
4. **Consumers and events:** replace descendant-only form validation with native
   form-owned membership; implement invalid events, cancellation, interactive
   reporting and :valid/:invalid from the shared state. Current form validation
   only performs limited required checks; exposed boolean return values do not
   prove the rest of the API.
5. **Verification:** rerun all 28 constraints entries and the prior 66-test cohort
   on a frozen signed candidate. Audit full source/helper/resource/automation and
   reference prerequisites before qualifying either passes or failures. Reconcile
   newly reachable subtests explicitly, without treating denominator growth as a
   regression. Full-Core and clean-source gates remain unchanged.

Other current failures include native maxlength editing, dynamic :invalid state
for pattern/number-step changes, and regex behavior. These are browser mechanisms,
not grounds to modify upstream assertions or weaken the runner.
