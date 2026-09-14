# Select validity selectors (source follow-up)

Native select validity now drives DOM selector queries and both computed-style
input paths. LayoutNode carries an optional validity state rather than synthetic
HTML attributes. None means that neither validation pseudo-class applies.
The vendored Blitz Stylo adapter previously hardcoded both :valid and :invalid
to false; it now reads the native element-state bits.

Custom validity changes publish native FormState journal entries only when the
message changes, invalidating cached layout without exposing an attribute or
child-list mutation. ValidityState.valid shares the native validity helper.

The native regression checks query/matches and computed colors after required
selection, custom errors and clearing, disabling/re-enabling, explicit empty
selection and removing required. It also verifies no synthetic DOM mutation
record and no validation pseudo-class on body. Current validation: 230 JS tests
(three existing ignores), 82 DOM tests, 159 native executable tests (six existing
ignores), targeted clippy with warnings denied, formatting and diff checks.

Only select is integrated so far. Other controls, form/fieldset aggregation,
user-valid/user-invalid, form submission validation, namespace edge cases and
reportValidity remain open. This source change is newer than the frozen
select-validation-v1 signed candidate and is not covered by its WPT results.
