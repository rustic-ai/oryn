# Form static validation: ownership, ordering and event snapshots

Form checkValidity now enumerates built-in associated controls in tree order,
including external form-attributed controls, rather than querying descendants
once per tag name. Select controls share native eligibility and validity state;
the old generic empty-value check no longer misclassifies selected non-placeholder
empty options or ignores select custom errors.

The complete invalid-control list is captured before firing trusted, cancelable,
non-bubbling, non-composed invalid events. Handlers can repair, disable, detach or
reassign later controls without changing the current snapshot or its negative
result. Canceling an invalid event does not make checkValidity return true. New
invalid controls created by handlers participate only in a later validation.

Regression coverage includes mixed select/textarea/input tree order, external
controls before and after the form, another owner's descendant, detached forms,
shadow-tree exclusion, placeholder identity, custom errors, barred controls,
event metadata, cancellation and callback mutations.

Algorithm reference: [HTML static constraint validation](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#statically-validate-the-constraints).

## Boundaries still open

This is not complete form validation. Input/textarea retain their earlier limited
required/checked checks; complete eligibility, sanitization and constraint states
still need native integration. Built-in enumeration does not supply form-associated
custom-element internals. reportValidity shares the static event phase, but user
reporting and unhandled-invalid-control reporting remain unimplemented.
requestSubmit, submitter validation, submission reentrancy and interactive
submission validation remain separate work. No whole-Core gain is inferred.

The new source passes 233 JS binding tests (three existing ignores), 159 native
executable tests (six existing ignores), targeted DOM/JS/native clippy with
warnings denied, formatting and diff checks. It is newer
than the September 13 00:12 signed worker and is not verified by that worker's
focused probes or by the earlier select-validation-v1 WPT results. Historical
signed evidence remains unchanged.
