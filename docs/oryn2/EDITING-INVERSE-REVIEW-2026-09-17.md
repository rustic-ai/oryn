# Editing inverse and continuation source review

Source inspection and code/regression generation only. No builds, formatters, generators or tests ran.

## Concrete findings and generated remedies

1. **Partial detach loses retry eligibility.** A node detached before an infrastructure error no longer has the original expected parent. Resource lane added shared per-segment detach/insert progress and late-commit observation. Actual detach also rechecks removable subtree witnesses and discounts the native detached sibling prefix. A rejected changed container disables its original descendant removal edges, preserving author-added children and the existing subtree.
2. **Control flags overwrite later author ownership.** Value CAS alone left dirty/user-edit/pending flags eligible even when the value was authored later. Resource lane groups these restorations under the original value guard and excludes duplicate generic node-value restoration. Regression sources change the author value and then its default-value attribute.
3. **Whole undo retry repeats admission after a committed prefix.** Original control value checks fail after an earlier segment or left document has already restored it. UndoUnit now shares native segment progress; paired Bridge owns its document cursor. Retry checks exact document/host and control marker identity, retains the admitted direction, and skips committed prefixes. History stacks advance only after the complete attempt. Regression sources fault after the first control segment and after the left document, then retry and redo.
4. **Paired terminal failure looks transient.** A Continuation can return a terminal error once and its changed result on subsequent polls. Transaction now latches the typed terminal error, cancels the unperformed other half, publishes committed native segments once, and only then returns the original error. Cross-frame drag skips the success-only input suffix after a failed transaction. Sources cover left and right terminal failures and no duplicate publication/recreated jobs.
5. **Smaller control mirror budget can retry forever.** Native segmented undo owns a bounded inverse independent of the smaller control-history mirror. `segmented_document_changes` admits the complete mirror group or omits it; it never records partial markers or retries an immutable oversized payload. Both Continuation and paired publication use it. Paired regression sources cover small mirrored controls and 300k-character controls using the native inverse alone.

## Remaining generation requirement

Direct native-only attribute/create/import/control blocks can still grow before the next Context await. If the subsequent snapshot exceeds capacity, the continuation's terminal quota path can lack an inverse for that most recent native mutation. The resource lane acknowledged this and owns tracing the production sites and adding pre-mutation growth admission or journaled Context operations. Arbitrary test futures are not themselves authored APIs, but actual production sites must be reconciled before source closure.

Whole-plan editing conformance and execution remain open. These findings do not establish test pass status.
