# Declarative adopted stylesheet caller closure — September 18

Source-only. No builds, binding generators, tests, runtime or conformance runs.
Pinned WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`; all paths below are
under Core `shadow-dom` in the frozen manifest.

## Native semantics generated

- ShadowRoot retains `shadow_adopted_stylesheet_specifiers: Option<String>`.
  Parser capture, import, CloneCursor and getHTML serialization preserve absent,
  empty and raw whitespace/DOMString values. Live list edits never rewrite it.
- HTMLTemplateElement has the reflected DOMString `shadowRootAdoptedStyleSheets`.
- `declarative_stylesheets` resolves in the target's owner realm/document, with
  per-document authority epoch state. Inert Documents cannot use caller modules.
- A valid pending specifier creates a fresh constructed CSSStyleSheet placeholder
  per occurrence. Ready results use the actual module-map sheet. Invalid
  specifiers create no entry; failed fetches leave the original empty placeholder.
- Completion replaces all current occurrences of each original placeholder
  identity. Reordering, removing/reinserting, duplicate URLs, clearing, rule
  mutation and detached-but-retained roots preserve their intended behavior.
  Old placeholder objects remain unchanged. Removed entries are never restored.
- Initialization records a committed token cursor. A native list commit followed
  by cleanup/wrapper failure cannot append the same placeholder twice.
- The existing observable adoptedStyleSheets array keeps its identity; native
  completion updates its backing data without invoking author iteration or
  prototype setters. Style extraction now reads native sheet associations even
  when the weak observable-array wrapper has been collected.
- Native list replacement publishes StyleSheets invalidation at commit, before
  fallible cleanup. Failed old-sheet IDs survive for independent retry at native
  lifecycle checkpoints and subsequent setters. Native migration remaps them;
  runtime replacement discards old-arena IDs. No root decrement is replayed.

## Integration contract

The public binding API is `NativeDeclarativeStylesheetHook`,
`DeclarativeStylesheetResult::{Invalid,Pending,Ready,Failed}`,
`prepare_declarative_stylesheets` and `complete_declarative_stylesheet`.
Postconnection preparation and adoptedStyleSheets getters call the shared helper.
cloneNode, importNode and Range clone initialize declarative sheets after registry
cloning and before custom-element upgrades, using the same committed cursor.
The loading lane generated native hook installation, initial parser preparation,
typed CSS module fetch/cache reuse and static/import/modulepreload publication.
The bridge uses the existing typed module default object, preserves preparation
slots across nested loads, and retains publication-failed preload jobs for retry.

## Generated regression sources

- `declarative_stylesheets::tests`: distinct duplicate placeholders, shared final
  module identity, swap/clear/remove/mutate/disconnect, failed fetch retention,
  original source serialization, inert Document isolation and retired owners.
- Native sanitizer module sources: first/middle reclamation failure, one committed
  invalidation, successful suffix cleanup, failed-only retry, migration identity
  and runtime replacement with numerically reused NodeIds.
- `oryn-native/tests/set_html_unsafe_style_visibility.rs`: real-layout full
  replacement style visibility and data CSS module maps with nested independent
  roots, duplicate specifier identity, clone reuse, absent/empty/raw attributes,
  script-added sheets excluded from serialization and template IDL reflection.

- Loading lane native sources cover synchronous data module identity, injected
  preparation retry, asynchronous preload deduplication, reordered placeholders,
  failed fetch retention and parked completion authority custody.

These are unexecuted assertions. Source integration is generated; combined
asynchronous fetch/preload and pinned conformance execution await the global
generation-complete validation phase.

## Exact pinned fixture hashes

| Fixture | SHA-256 |
|---|---|
| `shadowrootadoptedstylesheets-async-fetch-disconnect-iframe.html` | `2176026538c343f3e351ef26cdd672c70e3804a4c488ac3632b61e404ad7dfa8` |
| `shadowrootadoptedstylesheets-async-fetch-disconnect.html` | `69e340fd782970fbda6950721bb59b01dbead36cc110b70292b478bb8c451f0f` |
| `shadowrootadoptedstylesheets-async-fetch-duplicate-bare-specifier.html` | `a66d6464afabe90999d29f086b4d77eb9904f038c60b30632a2d4480e02f490f` |
| `shadowrootadoptedstylesheets-async-fetch-duplicate-importmap.html` | `f083b0cf010489b8b75ffa94bf3a46a6322d876bffec7e505a8bca093159ccb7` |
| `shadowrootadoptedstylesheets-async-fetch-duplicate.html` | `dcb9a8bafcef1292bb34035c7e1d55809f7b04e29b1558b97f54acd705c4e257` |
| `shadowrootadoptedstylesheets-async-fetch-failure.html` | `cb533084adf3e72990c6b057ccfb41977a42ef4725e976c422d4fd567c49b84a` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-clear.html` | `4480dfe9d51875b3d790f535cbaa261c2e503cde26c8c6c0f6c192233d49689d` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-insertrule.html` | `864224a602a4540326d4ca25299c0117081a9bf8feec6fbe8c8306c4cf082249` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-reinsert-same.html` | `4c2e11ab683cb4d7956235a4f056f55bd1892a12960dad14905001e924f4b9a7` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-reinsert-swap.html` | `d35bdbe67f2139808d0156a5ea68c4db1d7ecb8783e9e77d571444be2abc45dc` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-remove-middle.html` | `9ee433f136f686340051a08e5359a2fd9a1e0efe5c5b43ee18f8f03dba90dac3` |
| `shadowrootadoptedstylesheets-async-fetch-mutation-replacesync.html` | `a3c261481fb08eef4be2d52903c173d04d3ceeb5b61a14a5f31554e400acf813` |
| `shadowrootadoptedstylesheets-async-fetch-ordering.html` | `27b761c4870fb7630147a55c347ac0274eb6a38388ead4ca616fdb342bde72ae` |
| `shadowrootadoptedstylesheets-async-fetch-shared.html` | `d69dd0baf5a97af22a7bfa9dc2e3e8fcdaa215683a9efa3cdc2ebe1f4f9417eb` |
| `shadowrootadoptedstylesheets-async-fetch.html` | `0f14b605fd881ab05a84f00b76a75826cb941a4eb1557f7312383a18264eb11e` |
| `shadowrootadoptedstylesheets-basic.html` | `9075631246c9f2644a19672122dd3d83dd614f384439d7d04e9f04387a8b450d` |
| `shadowrootadoptedstylesheets-clonenode-async-fetch.html` | `971b5f9751b561ef4e2b26f3892a847c6a3476f60054bb0fd673e7f56e510185` |
| `shadowrootadoptedstylesheets-clonenode.html` | `d2e2d3cb65b3c81b3732fdcf152caed002cf008790ac5e040099ea88fa2c63cf` |
| `shadowrootadoptedstylesheets-document-parse-crash.html` | `f93ae22232207ce240f5f4c1e90e3d54564f06b5c1663e6e9f877bcc4bb262a5` |
| `shadowrootadoptedstylesheets-fetched-module.html` | `966b9debcf79e8b51146cee74c3d648eea3cb51df0102b0eca3ef3b699367914` |
| `shadowrootadoptedstylesheets-idl-absent-and-empty.html` | `1bff0a1be00e9de465a75b6dcc7422901d4a44c0676166ffcad51cf6762f5a17` |
| `shadowrootadoptedstylesheets-idl-feature-detection.html` | `8320c339af840504f8a51071779009dea77f92ca28eae8101a47c7323bde95b2` |
| `shadowrootadoptedstylesheets-idl-reflection.html` | `0ded036bee201401438bfdb771ef3c984b59b9b5c83182e18e3d44b77db59dea` |
| `shadowrootadoptedstylesheets-invalid-specifier.html` | `cb112e2cf8c0247fafb1c434e35d36d621e351a78dbc02a02eca3b053f4843c1` |
| `shadowrootadoptedstylesheets-modulepreload-async.html` | `b49c0a71051f5f6a91cc3c3c5ae7ff2a2e1c9c4c667682843882e78b66435360` |
| `shadowrootadoptedstylesheets-modulepreload-basic.html` | `6835966de68613ff315274ea15f50dcab08d20c1a883fec47e21f6beab952956` |
| `shadowrootadoptedstylesheets-modulepreload-failure.html` | `b6f4f3dcaf27d4137aa128d4dce7eb6164b4deef3a99eb79e3955bd18bcd8775` |
| `shadowrootadoptedstylesheets-modulepreload-identity.html` | `9f3a69bc22e4590a842f46056572f467b4f9095fe5578b1b0cc7b72fee229b00` |
| `shadowrootadoptedstylesheets-modulepreload-nested.html` | `5f356724f8f78b42ea838945de785f42890eb7ff8d76a569790a6dbfe4f1aed3` |
| `shadowrootadoptedstylesheets-modulepreload-ordering.html` | `dffebda49b99cfa892d09008c3cf44af0601fb55b7f926ebc14b2b8afeba1030` |
| `shadowrootadoptedstylesheets-modulepreload-shared.html` | `7cf4df8aacc84c9621bcc61e53ab0ba9447848ff6875c4acf65658d3eb845136` |
| `shadowrootadoptedstylesheets-multiple.html` | `287b8644d4e7ccb8fc016edd00361a12e504297d9c67be81a535f09622bd0f86` |
| `shadowrootadoptedstylesheets-nested-async.html` | `6fea2741b478db41fab8f9d50c1509caf88658172405149860c7eb80dbb29c18` |
| `shadowrootadoptedstylesheets-nested-sync.html` | `138f873114ba0cf28f8d24c56457ed69adaf0da104d9c5214fc79215c231d8dd` |
| `shadowrootadoptedstylesheets-placeholder-identity.html` | `33abdd56adfafe45fc09fd530f35bff712f1414b0ae00235e24fd01ab4d2443e` |
| `shadowrootadoptedstylesheets-same-document.html` | `f18598320daa5982796f4a6b29f46e129f635d82f7ea58a43b82993415250c36` |
| `shadowrootadoptedstylesheets-serialization.html` | `97d7edb04291eeefcb5d1986dfcee8123df7c051f5b60405d1a536b2da96ebdd` |
