# Document domain source audit — 2026-09-17

Status: source review and regression generation only. No build, test, service, or WPT execution was performed for this delivery. These entries do not establish conformance.

## Reviewed contracts

| Contract | Production source | Source status |
| --- | --- | --- |
| URL-host normalization, IDNA/IP equality, registrable-domain suffix, PSL boundaries, monotonic relaxation | `oryn-js/src/origin.rs` | Generated; strict Origin remains unchanged |
| Both sides must relax; scheme retained, ports ignored only for domain access | `DomainAccessIdentity` | Generated with pure regression sources |
| Document browsing-context check, sandbox flag, Permissions Policy, opaque origin, origin-keyed no-op | JS `document_domain.rs`; native `document_domain.rs` | Native setter policy and bilateral access are separate from network/storage/message authority |
| Prepare before allocation, commit with prior-domain and exact recipient roster checks | native domain module; host domain authorizer | Generated; final roster check uses page/policy/message locks |
| Initial and noninitial frame blank/srcdoc share origin-domain | `inherit_frame_worker_policy_at_url`; host inherited-context hook | Generated; sandbox origin replacement cannot alias |
| Initial popup and root blank navigation inherit domain | native popup policy/legacy opener path; `PreparedLocalDocument` | Reviewed and repaired in this pass |
| Parked alias state changes and restore-before-microtasks | parent alias registry; host restore/domain refresh | Generated; real parent-backed parked-popup fixture source added |
| Standalone aliases, initially relaxed tokens, callback publication | `local_document_provenance_v8.rs` | Generated with real-host frame regression sources |
| `window.originAgentCluster` uses admitted origin/group decision, opaque or isolated state | native domain module; JS `origin_agent_cluster.rs` | Generated; startup publishes secure-context authority before querying; retirement retains cached answer |
| `document.open` uses strict origin | JS domain strict-origin helper | Reviewed; domain access token is not used for strict check |
| Named familiarity, ancestor navigation, related Window URL visibility | `worker_service.rs` | Generated using separate domain identity |
| Navigation-event source identity, including retired source | `FormNavigationSource.source_domain_key`; signed form/document sources | Generated separately from strict origin key; consistent snapshot capture repaired in this pass |
| New/cloned/inert Document getter shares mutable inherited domain, including creator retirement | GC-owned DocumentDomainCell, native alias keys and weak host registry | Generated; signed navigation provenance remains frozen and inert setter still rejects |

## Required source evidence

Pinned WPT checkout: `wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0`.

- `html/browsers/origin/relaxing-the-same-origin-restriction/document_domain.html`
- `html/browsers/origin/relaxing-the-same-origin-restriction/document_domain_setter.html`
- `html/browsers/origin/relaxing-the-same-origin-restriction/document_domain_access_details.sub.html`
- `html/browsers/origin/relaxing-the-same-origin-restriction/document_domain_setter_srcdoc.html`
- `html/browsers/windows/document-domain-nested.window.js`
- `html/browsers/windows/document-domain-nested-set.window.js`
- `html/browsers/windows/document-domain-nested-navigate.window.js`
- Origin-keyed-agent-cluster getter special cases for removed/sandboxed frames and data-to-JavaScript navigation.

## Findings repaired in this review

1. Legacy root-opener popup creation now copies the authenticated opener's current domain group/value. Failed page admission removes pending domain records.
2. Root blank-navigation tickets retain the initiator's native domain snapshot and seed the next root after security commit. Data/blob navigation does not accidentally gain this blank-origin alias.
3. Signed Document capture takes the relaxed domain and navigation access key from one native identity snapshot.
4. The parked-popup regression now exercises explicit creator-authorized popup creation instead of an unrelated legacy fixture route.
5. Shared native domain cells replace copied metadata. Context-qualified alias keys join inherited root/frame/popup owners before exposure; inert/retired Documents retain the cell. A weak host map prunes expired cells, with no persistent parent capability records. Root blank handoffs preserve authenticated alias identity when the old owner has already retired. Regression sources cover changes after inert creation, parking, creator retirement and last-owner reclamation.

## Remaining gates and limits

- All generated source must still build and pass its native/JS regressions, complete WPT conformance, and five-family qualification after the global generation phase.
- Cross-context publication relies on resolving every active alias context before commit. The parent rejects a changed roster rather than publishing partial authority.
- Shared cells cover local same-agent inherited origins. Existing native admission requires all active alias recipients to resolve in the actor before commit. Cross-process network/storage/message identities remain strict; mutable metadata is not serialized as authority.
- This audit covers the named contracts and sources above, not every possible origin/agent-cluster case in the full HTML specification or the whole Core inventory.
