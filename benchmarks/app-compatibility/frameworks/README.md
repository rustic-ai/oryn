# Immutable application-compatibility framework corpus

These eight applications use exact dependency versions recorded in
`pnpm-lock.yaml`. Their source is immutable gate input. Generated framework
output remains ignored and is copied into content-addressed raw artifacts by
`scripts/build-app-compatibility-framework-corpus.sh`.

Build and capture all eight applications with:

```sh
scripts/build-app-compatibility-framework-corpus.sh --node /absolute/path/to/node --output benchmarks/evidence/framework-build-UNIQUE
python3 scripts/run-oil-framework-outcomes.py --build benchmarks/evidence/framework-build-UNIQUE/build.json --output benchmarks/evidence/framework-run-UNIQUE
```

The builder verifies the fixture lock before and after building, uses the pinned
package manager and frozen dependency lock, and captures logs plus a hashed
copy of each served output. Framework-generated build IDs mean this is not a
claim of byte-identical output across builds. The runner checks those copies
before and after use and retains every failure in the eight-framework denominator.
The tasks prove their declared state changes, not persisted server outcomes or
complete routing/storage compatibility.

The fixtures intentionally avoid layout assertions. Every app's declared OIL
task proves real boot/hydration, a framework-owned state transition, semantic
observation, routing markup, and storage without unexpected diagnostics. The
locked application-compatibility corpus exercises the remaining application patterns across the eight
apps and controlled fixtures: modules/chunks, portals/modals, custom elements,
date/select controls, file/upload metadata, downloads, tables/filters,
pagination, commerce flows, and heavy SSR content. A feature is never inferred
from framework boot alone.
