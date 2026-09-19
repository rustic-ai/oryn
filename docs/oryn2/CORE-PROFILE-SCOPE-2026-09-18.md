# Required Core profile scope

Coordinator decision for independent review, September 18, 2026. This defines the profile population before acceptance results exist. It does not create fixture endpoints, approve upstream dispositions, or claim qualification.

## Selected population

| Profile ID | UserMediaElement behavior | Third-party cookie policy | Role |
| --- | --- | --- | --- |
| standard-block | standard | block | Primary production profile and baseline for both comparisons |
| standard-allow | standard | allow | Complete cookie-policy comparison |
| legacy-block | legacy | block | Complete feature-profile comparison |

Each profile retains all **16,737** pinned Core obligations. The selected population is **50,211 profile-obligations**, comprising 47,955 automated obligations and 2,256 assessment obligations under the current dispatch inventory. These are required coverage counts, not results. Verify the inventory again before execution; no row may disappear if execution fails.

## Basis

- `crates/oryn-native/src/feature_profile.rs` declares `UserMediaElementMode::Standard` as the default. The official plugin README's Native feature profiles section also identifies standard as normal browser behavior and requires separate full standard/legacy runs.
- `crates/oryn-native/src/cookies.rs` declares `ThirdPartyCookiePolicy::Block` as the default; `ContextOptions` uses that default. `PARALLEL-AUTHORITY-STORAGE-2026-09-18.md` requires an allow/block pair with other fixture settings equal.
- `tools/wptrunner-oryn/fixture-contract.example.json` consistently selects standard/block. Its endpoint and TLS placeholders are not usable deployment evidence.
- The complete plan requires evidence for each claimed profile; the cited pair requirements do not independently require the legacy/allow combination. Choosing the actual production default as their shared baseline satisfies both explicit comparison populations without inventing another profile requirement. Any later additional policy claim requires its own explicit reviewed scope and evidence.

## Exact equality and claims

The allow/block pair changes only the cookie-policy field. The standard/legacy pair changes only the feature-profile field. All binary, source, signed worker, platform, fixture topology, TLS, device, renderer, timeout and other configuration identities remain equal within each pair. Use the new immutable profile registry and run envelopes tied to one combined release candidate; no URL-specific configuration or combining passing rows across profiles is permitted.

The five-family/eight-framework, reliability and performance production claim uses **standard-block**. It does not certify those non-Core outcomes under a different policy. Each acceptance component must bind its actual claimed profile; additional non-Core policy claims require additional evidence rather than inferred coverage.

Manual/platform evidence must be bound to each required profile and its actual procedures. Reusing an observation without independently validated applicability does not satisfy another profile's obligation. Known contradictory upstream assertions remain unresolved failures unless an actual independently reviewed, source/assertion-bound disposition satisfies the complete plan's existing rules. This scope decision creates no such disposition.

## Materialization and review gate

Independent review must verify this population against the pinned requirements and actual defaults. After that review, materialize the three real fixture contracts from the same verified deployment configuration, then the registry, scope-review records and immutable run envelopes. Do not manufacture placeholder fixtures or treat this Markdown document as an executed profile registry. Freeze all identities before any acceptance run and keep every original report in its own profile ledger.
