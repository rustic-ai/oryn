# Immutable G3 framework corpus

These eight applications use exact dependency versions recorded in
`pnpm-lock.yaml`. Their source is immutable gate input. Generated framework
output remains ignored and is copied into content-addressed raw artifacts by
`scripts/build-g3-framework-corpus.sh`.

The fixtures intentionally avoid layout assertions. Every app's declared OIL
task proves real boot/hydration, a framework-owned state transition, semantic
observation, routing markup, and storage without unexpected diagnostics. The
locked G3 corpus exercises the remaining application patterns across the eight
apps and controlled fixtures: modules/chunks, portals/modals, custom elements,
date/select controls, file/upload metadata, downloads, tables/filters,
pagination, commerce flows, and heavy SSR content. A feature is never inferred
from framework boot alone.
