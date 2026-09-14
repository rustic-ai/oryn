# Option replacement mutation records: source follow-up

The native single-node replacement path used by indexed option assignment now
removes the incoming node first (the adoption removal remains observable), then
suppresses the replaced-child removal and incoming insertion beneath one
compound child-list record. Sibling identities are captured before adoption.
Self-replacement produces an adoption removal followed by a compound insertion
with no removed nodes, following the DOM replacement algorithm:
https://dom.spec.whatwg.org/#concept-node-replace

Regression coverage checks fresh, preceding-sibling, following-sibling,
self and other-select replacements, including record order, target, members
and previous/next siblings. Current source passes 227 JS binding tests (three
existing ignores), 81 DOM tests, 158 native executable tests (six existing
ignores), and targeted DOM/JS/native clippy with warnings denied. Formatting,
generated IDL consistency and diff checks pass. This does not establish general fragment replacement, custom-element
reaction or cross-document adoption conformance.

This change is **not** in the frozen `options-batch-v1` signed binaries. Those
125-file broad and 66-file regression runs were already active when the source
fix was made. Their results must not be attributed to this fix. A subsequent
signed build and verification remain required.
