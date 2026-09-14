# HTMLOptionsCollection foundation

The next source candidate replaces select.options' sequence snapshot with a
live SameObject HTMLOptionsCollection, inheriting HTMLCollection. Native option
membership drives length, indexed reads, item, namedItem and named properties.
Weak owner caches and GC cleanup follow the existing collection ownership model.

Collection length, selectedIndex, add and remove are connected to native state.
Length expansion adds blank options to the select, not an existing optgroup;
truncation preserves optgroup elements. The specified growth ceiling of 100000
is observed. Argument conversion precedes live membership reads and exceptions
propagate. add shares the existing option/optgroup validation and insertion path.

Select now mirrors collection length, item and namedItem, and distinguishes
remove() (detach the select) from remove(index) (remove an option). Explicit
undefined is an index argument and converts to zero. Regression coverage checks
conversion-triggered mutations, required arguments and exception identity.
These methods follow the HTML Standard's select/options delegation rules:
https://html.spec.whatwg.org/multipage/form-elements.html#dom-select-remove

Indexed assignment/definition now operates on both HTMLOptionsCollection and
select. Nullable-option conversion precedes mutation; null/undefined removes
without padding; replacement preserves the existing option's parent; growth
pads before attempting insertion (including before a hierarchy failure).
Indexed padding does not borrow the length setter's 100000 ceiling and checks
V8 termination on each iteration. This does not establish acceptable large-
growth performance or resource qualification.

The WebIDL virtual data descriptors are writable, enumerable and configurable;
accessor and generic definitions are rejected, while data definitions invoke
the setter regardless of requested flags. Alternate Reflect.set receivers and
objects inheriting from a collection/select are tested without mutating the
original. Existing indexed deletion rules remain unchanged. Both surfaces share
the existing indexed callbacks; borrowed collection methods retain their own
collection receiver checks.

References:
https://html.spec.whatwg.org/multipage/common-dom-interfaces.html#htmloptionscollection
https://webidl.spec.whatwg.org/#legacy-platform-object-defineownproperty

Blank-option growth now uses a fresh-leaf batch insertion path. It preserves
structural journal entries but emits one observer child-list record with the
whole added-node list and correct previous/next siblings. Indexed growth emits
a separate insertion record for the supplied option after the padding batch.
Range boundaries, document ownership, option parentage, selection normalization
and no-op length assignment have explicit JS regression coverage. Creation can
be interrupted before the batch attaches. A native debug test created 100000
blank options in 123 ms (one sample, not a signed performance qualification),
verified the length ceiling and separately grew beyond it through the uncapped
batch API used by indexed assignment. Large truncation remains unqualified.

All 226 V8 binding tests pass with three existing ignores, including live
membership, mutations, branding, exceptions and detached owner/member retention.
The GC test collects 128 unreachable option-owner/list cycles and releases all
retained collections after deleting global references. Targeted JS/native clippy
passes with warnings denied. Native executable tests pass 158 cases with six
existing ignores after the select-method addition. All 81 DOM unit tests pass on
the current source; generated IDL consistency, formatting and diff checks pass.

This is not a completed collection or a qualified WPT gain. The release binary
remained frozen on the prior size/insertion candidate during development, then
was rebuilt and signed on September 12 at 23:29 local time. Signature/designated
requirement verification, protocol-v9 handshake, sandbox denial probes and the
independent inert-hit-testing probe passed. Current candidate identities:

- Parent SHA-256: `6b0b96afb705fabf743c1df7cf48951218833d28c81208af4ef30b89860106ff`
- Worker SHA-256: `ae23f30375d6dc841b7a5289812346d609c0c9238a22a4f63e97f9c33b280ba8`
- Code Directory hash: `35326ab918ff044eceb4d8bf89d98eff9fce82f0`
- WebIDL manifest: `c8bdbba1cd1337af320e75a8d35566893a83515c560b483b639eebff86c3c814`

The unchanged cohorts use `options-batch-v1-{broad,regression}-2026-09-12.json`.
The regression run completed all 66 files in 276.011 seconds: 43 raw passes,
23 failures, and no file-outcome changes against select-size-insertion-v1.
Subtests: 417 passes, 69 failures, one timeout. Fresh source/resource/probe-bound
reviews qualify 41 files / 336 subtests, leaving 16 review candidates and nine
unresolved. The two excluded raw passes remain radio-double-activate-pseudo
(missing fixture/automation audit) and clicking-noninteractive-unlabelable-content
(reference subtest identity unverified). Overlay/raw/review/helper hashes were
checked after generation. The broad run completed 125 files in 527.094 seconds:
41 raw passes and 84 failures, up from 30 raw passes in the previous candidate,
with no previous pass lost. Subtests: 250 passes, 198 failures, three timeouts,
four not-run. Eleven files improved. The initial 21-file reviewed subset
qualifies 20 files / 155 subtests, with 77 review candidates and 28 unresolved
across the cohort. common-HTMLOptionsCollection.html passed Oryn but its Chromium
reference prerequisites were unverified, so it is not qualified. Seven other
new raw passes remain outside this initial review set; no qualification is
implied. Overlay/raw/review/helper hashes were verified after generation.
The expanded immutable v2 review covers the remaining seven newly passing
files: wrapped optgroup and option-list exclusions, wrapper option membership,
optgroup insertion, multiple selection, absence of select named getters, and
selectedIndex. All sources and actual responses were rehashed. The v2 overlay
qualifies 27 files / 182 subtests, with 70 review candidates and 28 unresolved.
The original review and overlay remain unchanged; the reference-focus failure
still excludes common-HTMLOptionsCollection.html.
This remains a dirty-tree diagnostic build, not
clean-source attestation. Do not attribute prior raw WPT results to this source.

Required follow-up before claiming this surface complete:

- Qualify large growth/truncation under signed worker resource limits and audit
  custom-element/adoption behavior beyond the fresh blank-option batch.
- Audit adoption/realm behavior, attribute namespaces and large-index resources.
- Rebuild/sign and run the unchanged broad/regression cohorts, preserving every
  failure and requalifying source/resource/reference evidence.

Full Core conformance remains the objective; this foundation is not a substitute
for complete HTMLOptionsCollection behavior or the full Core acceptance gates.
