# Document language pipeline — source review, 2026-09-16

Source generation only. No builds, tests, services, formatters, generators or
qualification have run. Whole V/L/Core acceptance remains open.

## Pinned evidence

WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`:
`html/dom/elements/global-attributes/the-lang-attribute-003/004/005/006/007/009/010`
and corresponding header fixtures cover HTTP fallback, meta precedence, explicit
language precedence and explicit empty lang suppressing fallback. The `011`
header fixture contains `ko,zh,ja`; a multiple-language response cannot select a
single document default. `css/selectors/invalidation/lang-pseudo-class-in-has-document-element.html`
requires meta language plus subsequent explicit document-element invalidation.

## Producers and ownership

- DOM DocumentMetadata now distinguishes response `content_language` from processed
  `pragma_language`. Single HTTP field values are accepted; comma lists and multiple
  fields select no default. Values remain language data, never authored attributes.
- DomArena insertion processes connected HTML meta Content-Language pragmas. The
  content comma guard precedes first-token extraction using HTML ASCII whitespace.
  Empty/invalid directives leave the previous pragma intact. Relevant attribute
  changes reprocess; removal does not undo an already processed directive. Shadow
  and template trees are excluded. Existing mutation publication invalidates live
  layout; explicit native header updates publish native state mutation.
- Native navigation_actor/runtime/navigation_runtime propagate response headers into
  private DocumentStream metadata before parser/author execution. Buffered/XML
  document handoff preserves the header. Non-V8 buffered/native paths also populate it.
- Parent PageResource carries optional content_language; both static prepared frames
  and dynamic frame completion install their own response language before DOM import
  and parser continuation. Local blank/srcdoc/fixture streams default to no response
  language; another document's header or explicit lang is not inherited across a
  browsing-context boundary. Fresh documents start clear; same-Document metadata and
  cached/imported document identity preserve their own language state.
- XHR inert response documents also receive their own Content-Language header.

## Consumers

Shared DomArena/DomRuntime `element_language` applies XML namespace precedence,
ordinary/shadow ancestry, explicit empty language, then document pragma/header.
Native selector queries and `:has(:lang(...))` consume it. LayoutInput transports the
current document fallback into Stylo native language state, shaping and text transforms.
Temporal/numeric picker locale selection and native permission labels use the same
language chain; native UI locale remains the final control fallback only when no
language was specified. Explicit empty language blocks that fallback.

## Regression sources and contracts

Generated DOM tests cover header lists, meta updates/removal/invalid values, empty
language, XML precedence, inert template and independent imported documents, and
control locale. Selector tests cover header-to-meta changes and relational matching.
Layout tests cover shaping fallback, empty inheritance and serialization. Native
stream tests cover pre-parser installation and a fresh document reset; the existing
parent frame-header fixture now asserts Content-Language before body EOF.

`PageResource` remains in the shared pending context45 batch with a defaulted optional
field. LayoutInput adds a defaulted document_language field; existing literal consumers
were reconciled. No compiled artifact or historical signed evidence was rewritten.
