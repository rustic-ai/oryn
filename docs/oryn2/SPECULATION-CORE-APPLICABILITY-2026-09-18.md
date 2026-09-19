# Speculation algorithm applicability to the frozen completion scope

Source review only, September 18, 2026. No build, test, WPT execution, lock
generation, tier edit, or denominator change was performed for this audit.

## Fixed scope and evidence

The completion plan section 2 requires all 16,737 Core entries, plus Necessary
and Extended capabilities when Core or the five-family contract depends on them.
It explicitly separates completing unrelated tiers. The saved
`artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl` contains 16,737 Core rows
mapping to 13,277 distinct source paths. The pinned checkout is revision
`719d5e38fdd0903a18ed9007aba816c98cc491e0` in the sibling
`oryn/artifacts/upstream-conformance` directory.

Inspection of those Core HTML/JS/XHTML/SVG sources for speculation script types,
speculation resource imports, `startPrerendering`, and `create_prerendered_page`
identified the following two direct callers. This is source reachability evidence,
not a claim that arbitrary dynamically computed imports were executed.

| Core caller | Saved source SHA-256 | Required behavior |
| --- | --- | --- |
| `html/browsers/browsing-the-web/history-traversal/pagereveal/order-in-prerender-activation.https.html` | `7ff7961801eef116f151b695aa35ec77ba47b5dd0b65c6963d73d0452e94ce7c` | Real hidden prerender, activation, and `pageshow,prerenderingchange,pagereveal,raf` ordering |
| `html/semantics/document-metadata/the-base-element/crashtests/base-with-speculation-rules-onerror.html` | `e2e13bdcf6d064b3a9c9bb1379ecc245aec32527bdf0cc62eadcda243c552d7b` | Safely reject top-level `[]` while base mutation and script error handling remain safe |

The first opens `resources/order-in-prerender-activation-popup.html?uid=...`.
That support page imports `/speculation-rules/prerender/resources/utils.js`,
calls `startPrerendering(prerendering_url)` without extras, waits on
`PrerenderChannel`, and activates using `location.replace`. The helper constructs
`{prerender:[{source:'list',urls:[url],...rule_extras}]}`. Thus its reachable rule
is an immediate list rule. The channel additionally requires ordinary Fetch
keepalive and the pinned broadcast-channel server resource.

The same utils file defines an optional list-prefetch branch inside
`create_prerendered_page`, guarded by `opt.prefetch`. Neither the Core parent nor
popup calls that function. A helper definition alone is not an executed branch.
The crashtest contains no candidate URL, document predicate, or prefetch rule.
Current five-family fixture and workflow recipe source searches found no
speculation-rule consumer. This does not assert compatibility with arbitrary
third-party sites beyond the versioned family contract.

## Document rules and speculation-prefetch disposition

The native parser currently admits list prerender rules, rejects unsupported
document sources, and does not schedule a speculation-prefetch operation. These
remain implementation limitations. The following concrete examples are already
recorded as **Extended** in the saved inventory:

| Example under `speculation-rules/` | Saved SHA-256 |
| --- | --- |
| `speculation-measurement/performance-speculations-navigations-pointerdown.tentative.https.html` | `0e868039ce00db170df269939be54e00203f4f2787fa4467066f634013d79677` |
| `speculation-measurement/performance-speculations-navigations-eagerness.tentative.https.html` | `08f4167673a0bcb506211ad033866b01346e301bf5a291a681845d3836a7738a` |
| `speculation-tags/prefetch-eagerness-pointer-down.https.html` | `f22012a68714c221efcd4e8db8129e7a1cf22ff81161eff8ff1c8af121f852ba` |

No Core/family dependency on document predicates or speculation-prefetch was
established by this review. Therefore this audit does not expand the fixed goal
to implement them or invent a new exclusion. If an actual Core/family caller is
found, implementation must include live anchor selection and invalidation,
predicate evaluation, eligibility and interest state, cancellation, frozen
authority and real network/cache custody; converting document rules to eager
lists or counting a rule as a successful prefetch would not satisfy it.

## Independent Core link-prefetch obligations remain

HTML `<link rel=prefetch>` is a separate native loading algorithm. It remains
required by these saved Core source groups, regardless of speculation-prefetch:

- Nine `content-security-policy/resource-hints/prefetch-*` files, including
  `prefetch-ignores-prefetch-src.sub.html` and conflicting-policy cases.
- `fetch/api/request/destination/fetch-destination-prefetch.https.html`.
- Two `fetch/metadata/generated/element-link-prefetch*` files.
- Twelve `mixed-content/gen/**/link-prefetch-tag.https.html` files.
- `referrer-policy/generic/link-rel-prefetch.html`.

These require the applicable actual destination, CSP, metadata, mixed-content,
referrer and completion-event behavior. Initial production source reconciliation
found a confirmed gap: the legacy `runtime.rs` resource scan classified prefetch
as a module, and no dedicated link-prefetch producer was present. The parent
assigned the complete producer, discovery and regression work to the parser
lane. In particular, the destination fixture appends a link inside a controlled
iframe and observes an empty service-worker `Request.destination`; loading it as
a script or ordinary connect request does not satisfy its security contract.
This document does not mark these obligations passed or waive
their optional/platform assertions. Speculative HTML preload-parser tests are
also a different algorithm and retain their existing obligations.

## Source regression and acceptance

`speculation_rules.rs` now includes
`pinned_core_pagereveal_helper_defaults_to_immediate_list_prerender`; the existing
malformed-rule test covers the invalid top-level array and invalid sibling rules.
The parser lane owns native interest and activation source regressions. All are
unexecuted. Full Core acceptance still requires the original pinned callers,
native event ordering, loading outcomes, and complete candidate-bound evidence.
