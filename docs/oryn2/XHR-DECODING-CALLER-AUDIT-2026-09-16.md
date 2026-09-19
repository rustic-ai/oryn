# XHR response decoding — source review, 2026-09-16

Generated source and regression sources only. No tests, builds, services,
formatters or generators run; runtime and conformance qualification remain open.

## Exact pinned obligations

Pinned WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`:

- `xhr/responsetext-decoding.htm`: default responseType sniffs XML declarations;
  explicit text does not; HTML responseText does not sniff meta declarations.
  Charset headers, UTF-8 fallback, and exactly one BOM removal apply.
- `xhr/responsedocument-decoding.htm`: document responses sniff XML declarations
  and HTML meta declarations when no recognized final charset determines encoding.
- `xhr/overridemimetype-edge-cases.window.js`: no override charset retains the
  server charset, while a bogus override charset replaces it (plain-text fallback
  becomes UTF-8). Open must not reset the override; existing open behavior unchanged.

## Generated production path

`xhr_responses::decode` selects `TextResourceKind::Xml`, `Html`, or `Fetch` from
final MIME and responseType, and calls the shared resource decoder. It forwards
only the selected recognized charset, preserving BOM precedence, and supplies
UTF-8 as the XHR HTML fallback instead of navigation's legacy default.

A present invalid override never falls back to the old server charset. Missing
or unrecognized final charset still permits the format-specific sniffing required
for XML/default or document responses; ordinary text falls back to UTF-8.

Both responseText and response/responseXML callers use this path. Parsed inert
response documents keep the selected encoding in document metadata; response
identity caching remains unchanged. JSON remains its independent UTF-8 path and
root's scalar-string handling remains intact.

## Regression source matrix

`xhr_responses::decoding_tests` exercises all three responseTypes against XML/HTML,
no charset/UTF-8/windows-1252, document text/characterSet/cache identity, invalid
and absent override charset, BOM precedence/removal, final MIME override/suffix,
missing MIME XML fallback, explicit text non-sniffing, and HTML UTF-8 fallback.

This closes the identified decoding caller gap at source level, not the whole XHR
algorithm inventory or an executed conformance result.
