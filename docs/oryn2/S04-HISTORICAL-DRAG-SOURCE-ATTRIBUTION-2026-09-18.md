# Historical drag source attribution — September 18

Source inspection and generation only. No build, binding regeneration, formatter,
test, WPT or service was run. This closes the historical **inventory**; it does
not declare 180 failures passing or change their classification/denominator.

## Exact population

The September 15 `artifacts/wpt-full-baseline/full-current-2026-09-15/cohort.json`
contains exactly **180 failed Core rows under `html/editing/dnd/`, across 72
unique paths**. They are not 180 distinct algorithms or files. The accompanying
[JSON map](wpt-procedures/s04-historical-drag-source-map-2026-09-18.json) preserves
every historical ID, query variant, diagnostic tag and failure domain, plus each
pinned file's hash, helper dependencies, assertion excerpts, production route,
regression names and source disposition. The upstream pin is
`719d5e38fdd0903a18ed9007aba816c98cc491e0`.

| Pinned group | Rows | Production/source obligation and regression attribution |
| --- | ---: | --- |
| drop/029 and drop/030 | 110 | `data_transfer::DragNegotiation` and retained begin/advance/drop receipts; new `pinned_drop_and_dragend_matrix_covers_all_110_variants` covers the helper's 11 effectAllowed × 5 dropEffect combinations at both endpoints, including invalid setter retention and denied-drop final effect. Existing event/default retry and native pointer tests cover transport. |
| interactiveelements | 24 | `native_drag::element_drag_source`, pointer drag admission and native pointer selection; new 24-shape matrix distinguishes button/image/reset/submit sources from select/text/read-only/disabled/editable selection surfaces. Selected-text payload admission remains first. |
| images | 16 | Native URI payload seeding, cross-frame retained transport, pointer scrollbar targeting, drag image snapshots and native image resources. Named cross-frame/feedback/scrollbar regressions are in the per-file map. Root scrollbar no-drop cases remain execution obligations under actual native scrollbar configuration. |
| canvas | 5 | Author canvas serialization feeds the same native data store and cross-frame transport; call-time drag snapshots own pixels through layout changes. Canvas raster and image decode remain rendering dependencies, not separate drag negotiation algorithms. |
| datastore | 5 | Constructor, mutable store, live item/FileList views, indexed properties, stable IDs and cached FrozenArray revisions. Native transfer and removed-item tests cover these paths; constructor default correction adds an explicit regression. |
| platform | 5 | Captured pointer press, layout recomputation and native selection routes; native admission retains original press through later layout changes. Existing pointer selection and canceled-start custody sources are named per file. |
| dom | 3 | Authored HTMLElement/Document/Window event members and HTMLElement.draggable reflection. New reflection matrix covers default image/link behavior, invalid and ASCII-insensitive tokens, and Web IDL boolean conversion. |
| events | 2 | Retained enter/leave related-target sequencing and pointer suppression/capture handoff; native cross-frame coordinate and action suppression sources. |
| the-draggable-attribute | 2 | Same new reflection implementation/matrix; arbitrary HTMLElement setter reflects literal true/false. |
| layoutshift, remove | 2 | Source Session keeps owning Document identity without requiring source connectivity; target still must be connected. Native drag snapshots and begin/advance retention preserve admitted data through layout change/removal. |
| synthetic, historical, JS File enabled | 3 | Native event constructors/store attachment, absent historical members, actual authorized File bytes. Synthetic events remain untrusted; callbacks remain queued. |
| Three contract conflicts below | 3 | Explicit unresolved pinned/profile disposition; no hidden exclusion or obsolete feature implementation. |

## Confirmed implementation gaps fixed

1. **DataTransfer constructor:** authored objects now start with effectAllowed
   `none`, while native drag seed stores retain `uninitialized`. Previously the
   binding used the native store default unchanged.
2. **Interactive element admission:** text control/select/editable surfaces no
   longer fall through to an explicit draggable ancestor. Existing selected-text
   admission remains available before element-source lookup. Buttons and input
   button/image/reset/submit retain element-drag behavior.
3. **HTMLElement.draggable:** the authored IDL member and native getter/setter
   were absent. Added CEReactions boolean reflection, native auto default for img
   and a[href], ASCII-only true/false recognition, and setter conversion without
   invoking author string conversion. Generated binding output must be regenerated
   when the global generation gate opens.

The 110-case negotiation regression is also new source. It deliberately checks
both valid/invalid setter values and terminal dragend behavior rather than only
checking a table of allowed bits.

## Three genuine pinned-contract conflicts

- `target-origin/202.html` invokes `initDragEvent` and `allowTargetOrigin`, while
  pinned `synthetic/001.html` explicitly asserts that `initDragEvent` is absent.
  A single conforming surface cannot satisfy both requirements as written.
- `the-dropzone-attribute/dropzone_attribute.html` requires obsolete dropzone IDL
  reflection. Current authored IDL does not expose that removed API. The original
  row remains failed; policy/profile disposition belongs to the coordinator.
- `js-file-fallback-text-only.html` explicitly tests a disabled JS File transport
  feature and demands zero files plus filename-only text. Its sibling
  `js-file-image-drag.html` requires actual binary File transport in the same
  profile. Native code supports actual File bytes; no per-URL fallback was added.

The coordinator must record their normative/profile disposition explicitly before
claiming complete conformance. The inventory is now concrete: **69 paths mapped
to generated native sources, 3 conflicting/obsolete configuration contracts**.
Regression names establish source attribution, not executed equivalence to every
upstream assertion. All 180 rows still require the agreed qualification process.
