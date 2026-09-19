

## September 17 — streamed document fallback encoding reconciliation

`IncrementalTextDecoder::with_fallback` previously accepted parent encodings that the buffered `decode_resource_text_with_fallback` path rejected. Both now use `resource_fallback`, preserving HTML's ASCII-compatible parent constraint and CSS's separate environment fallback behavior. Added split-boundary source regression for UTF-16LE/BE, replacement, windows-1251/1252 and UTF-8 fallback labels, including empty transport chunks and independently expected decoded text. No execution.

Remaining source-confirmed L-01/L-02 dependency: classic external script decoding receives transport charset but current `ScriptFetch` has no captured script-element/document fallback encoding, and parent `decode_page_script` selects the shared JavaScript decoder without one. Complete this across prepared parser/dynamic requests and parent/standalone consumers; module and worker scripts must retain UTF-8-only decoding. This is an open implementation requirement, not covered by the document decoder correction.


### Classic-script fallback implementation in progress

Generated shared classic-script fallback selection below BOM/transport charset, with buffered/streamed regression sources and module UTF-8 controls. `PreparedDynamicScript` now freezes a canonical fallback label from the script charset attribute or native document encoding. The standalone dynamic-script producer passes it through `ScriptFetch`; module/worker callers explicitly supply no document fallback. Parent `LoadScript`, parser preparation and direct classic-host callers still need the same captured field propagated before this requirement is source-complete. No wire contract change or execution in this subgroup yet.


### Captured classic-script encoding through parent and parser

The `LoadScript` wire request now carries optional `fallback_encoding`; parent cancellation/classification and network/data decode paths pass it into classic decoding. Typed/module routes keep UTF-8 semantics. Parser preparation captures the label alongside policy/base/referrer identity, and both scheduled parent loading and standalone parser loading consume that saved value. Dynamic parent loading now sends its already-captured label too. Added parent decode precedence/module controls and extended prepared-script mutation regression to assert charset freezing. The wire field is serde-defaulted; final combined protocol/source/artifact reconciliation remains required. No build or tests run. Parent transport regression and remaining producer inventory review are still pending.


### Classic-script fallback source caller reconciliation

Generated `captured_script_encoding_survives_parent_transport_redirect_and_lease` exercises actual parent IPC request serialization, direct and leased requests, redirects, data URLs, response charset precedence, and module UTF-8 controls. Generated `parser_script_encoding_is_frozen_from_attribute_or_native_document` checks both preparation sources and later mutation. Source inspection traced all production `LoadScript` constructors: parser and dynamic classic routes carry captured fallback; module graph routes explicitly use none. All production parser ScriptRecord preparation routes pass through `capture_parser_script_identity`, and standalone parser fetch receives that saved label. Direct decoder tests use a test-only compatibility wrapper; production uses the fallback-aware function. These tests are source only and have not executed. Final combined protocol/source qualification remains open.


### HTML charset prescan attribute ordering

Replaced the sorted attribute map in the byte prescan with source-ordered first occurrences. The charset/content/pragma state now follows encounter order: an explicit charset replaces the earlier content-derived encoding even when its label is invalid; later content only fills a missing encoding and requires the content-type pragma. Duplicate attributes retain their first occurrence. Generated buffered and every-split streamed regression sources for both invalid-label orders, direct override, duplicates and absent/duplicate pragmas. No execution; broader HTML prescan/reparse qualification remains open.


### Prescan text versus tag boundaries; remaining reparse requirement

The byte prescan now treats a stray less-than sign followed by ordinary text as text instead of letting a fake attribute scan consume a later real meta element. Added positive stray-prefix and negative comment/declaration/end-tag/quoted-attribute sources. No execution.

The pinned `html/syntax/charset` README explicitly describes its interoperable browser behavior and links HTML issue 6962; source inspection found no encoding-confidence/reparse machinery in native document loading after the initial 1,024-byte sniff. Late declarations therefore require a separate parser-aware investigation and complete implementation where the pinned Core cases require them. Do not treat prescan fixes as closing the full L-01 HTML encoding requirement.


### Shared Fetch MIME extraction in progress

Added `MimeType::from_headers`/`extract` to combine Content-Type field values before splitting quoted commas, select the last valid non-wildcard MIME record, and carry charset across matching essence values. Pinned `fetch/content-type/resources/content-types.json` provides regression expectations, including quoted strings continued across field occurrences. Script/stylesheet MIME blocking and XHR response MIME now use the shared path. Remaining navigation/script decode, body Blob/FormData, module and resource consumers still require reconciliation; this is not full MIME caller closure. Source only; no execution.


### MIME extraction consumer integration

Standalone and parent script source decoding now consume the serialized extracted MIME record, matching their policy gate instead of decoding with the first header's charset. Typed JSON/CSS module validation uses the shared extraction path. Native Request/Response Body operations now derive Blob type and FormData decoder choice from all Content-Type values. Added module positive/negative multi-header controls and native Body regression sources for charset carry, a quoted string spanning header values, and form decoding. Navigation/resource consumers remain under review; no execution.

### Response MIME integration continuation — September 17

Root actor navigation, serialized navigation, parent frame/style/binary loading, standalone external text/image loading, frame paint images, download metadata and resource timing now extract MIME from all Content-Type field values through the shared Fetch algorithm. Parser selection and byte decoding consume the same extracted value. The existing native document encoding regression now supplies an initial conflicting binary MIME, mixed-case repeated headers, inherited charset and wildcard/invalid tails; it covers root, static frame, dynamic frame and BOM precedence. Regression sources only; no build, tests or conformance executed. Full remaining response-consumer audit and late HTML encoding restart remain open.

### Remaining native MIME consumer audit — September 17

Shared extraction now also supplies standalone frame navigation, standalone CSS/font/image loading, worker/service script metadata, agent Fetch timing, document-security preparation and the bounded resource audit record. A search of native Content-Type comparisons leaves header replacement and test assertions, not production first-field response MIME readers. JS remaining comparisons concern request header creation/replacement or request assertions; direct MIME parsers retain their role for authored override values and data/blob MIME records.

Generated `extracted_response_mime_aligns_document_decoder_and_timing` covers repeated-header HTML/XML/plain-text parser/decoder selection and every byte split; `audit_records_extracted_mime_without_unrelated_headers` covers effective MIME evidence. Existing root/static/dynamic-frame encoding regression now exercises conflicting/repeated response fields. These are source-only claims. Missing-type MIME sniffing, late HTML encoding restart, exhaustive pinned inventory and allocation/runtime qualification still require completion.

The frame consumer review found and removed a separate HTML/XML-only decoder selection in `frame_response_stream.rs`: streamed plain-text frames now use the same non-HTML decoder as root and buffered navigation, so literal meta markup cannot change decoding. The root/frame regression adds a UTF-8 plain-text frame containing a misleading windows-1252 meta string. Standalone frame preparation now captures its native parent encoding and applies the same-origin-only fallback at the final response URL, matching parent-owned frame decoding; opaque/cross-origin destinations cannot inherit it. Source generation only; complete standalone fallback regression review remains pending.

### Standalone fallback regression source review — September 17

`standalone_frame_encoding_is_captured_and_scoped_to_final_origin` now exercises the actual standalone loader with captured windows-1251 parent metadata, later parent mutation, same/cross-origin final responses, redirects away and back, explicit UTF-8 transport charset, plain-text UTF-8 and an opaque parent. Expected decoded text distinguishes inherited windows-1251 from HTML default windows-1252 and UTF-8. This completes the named standalone fallback regression generation; no execution or broader encoding closure is claimed.

### Linked stylesheet MIME admission — September 17

Pinned `html/semantics/document-metadata/the-link-element/stylesheet-bad-mime-type.html` requires error rather than load for both empty and nonempty text/html responses. Native loaders previously checked only Fetch nosniff. Shared `stylesheet_response_allowed` now applies the linked-sheet text/css requirement after nosniff, permitting the quirks-mode exception only for CORS-same-origin responses. `ParserFetchContext` captures native document quirks; the serde-default field in `StylesheetRequest` carries it through parent IPC. Imported sheets drop root nonce/integrity/crossorigin but preserve document quirks. Standalone loader and parent HTTP/data/blob paths share the rule; the legacy test-only stylesheet fetch helper applies strict admission.

Generated regression sources: `linked_stylesheet_mime_requires_css_except_cors_same_origin_quirks`, `parent_stylesheet_mime_checks_empty_network_and_local_responses`, and `standalone_stylesheet_mime_rejection_dispatches_error_for_empty_and_nonempty_bodies`. These cover repeated/invalid MIME, nosniff, same/cross-origin and successful CORS, local responses, and public load/error completion. No execution. Final protocol/fixture lock reconciliation remains required.
