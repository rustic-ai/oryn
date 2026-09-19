# Native child-frame and late-upgrade form history generation

Implementation and regression source only. No build, formatter, generator, test or browser run occurred.

## Generated behavior

- Native custom-element registry retains bounded unmatched restoration obligations for definitions that upgrade later. Metadata includes full form owner identity and definition/name occurrence; descriptors count unupgraded candidates, preserving duplicate-name order.
- Restoration consumes one obligation before queueing its callback, then re-resolves subsequent targets after callback effects. Native stream replacement and retirement clear obligations. A second navigation before upgrade captures outstanding values rather than losing them.
- Child-frame snapshots carry complete native embedding paths (tag/id/name/src/srcdoc/occurrence), exact committed document URL, and typed saved control values. Matching requires a unique live native frame with that path and URL; redirects or mismatches do not receive private values.
- Native history stores child snapshots beside root snapshots and includes both in shared retention/control budgets. Root callback retirement prevents child restoration. Pending frame obligations survive until an eligible frame commits, with root realm/generation/epoch checks at every checkpoint.
- BFCache retains native documents/registries; existing cached-document restoration remains distinct from cross-document mapping.

## Regression sources

Added late-definition duplicate-name ordering, no replay, preservation while still unupgraded, child-frame late-definition restoration, mismatched frame URL withholding, and frame path/depth budget cases. None have run.

## Remaining limitations

Capture targets currently live native Window/frame realms; worker-owned auxiliary pages have their own history. Embedded metadata is bounded to 64 KiB per field and frame history to 32 levels/256 records with the page-wide control/byte limits. Direct low-level callers must still establish the history entry URL authority; the runtime does so before admitting restoration. Source review and execution may reveal integration failures; no conformance claim is made.
