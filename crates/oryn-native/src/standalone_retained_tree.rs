// Native standalone response discovery. No flattening, synthesized wrappers,
// parent receipt fabrication, or networking outside the captured Loader.
impl Loader {
    fn web_failure(&self, message: String) -> HostError {
        if self.retained_fetch {
            HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(message))
        } else {
            HostError::Execution(message)
        }
    }
    #[cfg(unix)]
    // Keep the exact captured document, source and cancellation inputs explicit;
    // these borrowed fields cross no new allocation or authority boundary.
    #[expect(clippy::too_many_arguments, reason = "explicit retained source and owner inputs preserve the captured loader contract")]
    pub(super) fn load_retained_import_tree(
        mut self, document: ParserDocumentIdentity, node: NodeId, revision: u64,
        cancellation: Arc<crate::context_service::ResourceCancellation>,
        requested: &str, parent_url: &str, policy: &str, encoding: &str, depth: usize,
    ) -> Result<RetainedStylesheetSourceEnvelope, HostError> {
        self.bind_retained_cancellation(cancellation.clone());
        let response = self.retained_response(requested, parent_url, policy, encoding,
            false, document, node, revision, &cancellation)?;
        let mut ancestors = Vec::new();
        ancestors.try_reserve_exact(9).map_err(|e| HostError::Execution(e.to_string()))?;
        ancestors.push(stylesheet_cycle_key(parent_url));
        ancestors.push(stylesheet_cycle_key(&response.final_url));
        let tree = self.retained_tree(response, depth, &mut ancestors,
            document, node, revision, &cancellation)?;
        self.check()?;
        let bytes = tree.queued_source_bytes()?;
        let held = self.retained.iter().try_fold(0usize, |sum, entry| sum.checked_add(entry.bytes))
            .ok_or_else(|| HostError::Execution("standalone source bytes overflow".into()))?;
        if bytes > held { self.retain(bytes - held)?; }
        Ok(RetainedStylesheetSourceEnvelope::new(tree, std::mem::take(&mut self.retained)))
    }
    #[cfg(unix)]
    pub(super) fn bind_retained_cancellation(
        &mut self,
        cancellation: Arc<crate::context_service::ResourceCancellation>,
    ) {
        self.cancel = cancellation.flag.clone();
        self.retained_fetch = true;
    }

    #[cfg(unix)]
    #[expect(clippy::too_many_arguments, reason = "explicit retained source and owner inputs preserve the captured loader contract")]
    pub(super) fn load_retained_tree(
        mut self,
        document: ParserDocumentIdentity,
        node: NodeId,
        revision: u64,
        cancellation: Arc<crate::context_service::ResourceCancellation>,
        href: Option<&str>,
        base: &str,
        css: &str,
        encoding: &str,
    ) -> Result<RetainedStylesheetSourceEnvelope, HostError> {
        self.check()?;
        let policy = self.referrer_policy.clone();
        let initiator = self.initiator.clone();
        let response = if let Some(href) = href {
            let requested = self.backend.resolve_stylesheet_request(base, href)?;
            self.retained_response(&requested, &initiator, &policy, encoding, true,
                document, node, revision, &cancellation)?
        } else {
            // Policy was reconciled on the owning DOM before dispatch. Inline
            // root authority is checked again before the graph is published.
            self.retain(css.len())?;
            RetainedStylesheetResponse {
                sheet: None,
                final_url: base.to_owned(), source: Arc::from(css),
                referrer_policy: policy, encoding: encoding.to_owned(),
                proof: NativeStylesheetResponseAuthority::Inline(NativeInlineStylesheetAuthority {
                    document, node, source_revision: revision,
                    cancellation: cancellation.clone(),
                }), guard: None,
            }
        };
        let mut ancestors = Vec::new();
        ancestors.try_reserve_exact(9).map_err(|e| HostError::Execution(e.to_string()))?;
        ancestors.push(stylesheet_cycle_key(&response.final_url));
        let tree = self.retained_tree(response, 0, &mut ancestors,
            document, node, revision, &cancellation)?;
        self.check()?;
        // Existing reservations cover original response text; charge only the
        // metadata delta, not a second whole tree body quota.
        let bytes = tree.queued_source_bytes()?;
        let held = self.retained.iter().try_fold(0usize, |sum, entry| sum.checked_add(entry.bytes))
            .ok_or_else(|| HostError::Execution("standalone retained bytes overflow".into()))?;
        if bytes > held { self.retain(bytes - held)?; }
        Ok(RetainedStylesheetSourceEnvelope::new(tree,
            std::mem::take(&mut self.retained)))
    }

    #[cfg(unix)]
    #[expect(clippy::too_many_arguments, reason = "response authorization keeps exact owner, cancellation and request metadata explicit")]
    fn retained_response(
        &mut self, requested: &str, referrer: &str, policy: &str, encoding: &str,
        root: bool, document: ParserDocumentIdentity, node: NodeId, revision: u64,
        cancellation: &Arc<crate::context_service::ResourceCancellation>,
    ) -> Result<RetainedStylesheetResponse, HostError> {
        let (url, source, encoding, policy) = self.resource(
            requested, referrer, policy, encoding, "stylesheet", root)?;
        self.check()?;
        let origin_clean = self.last_origin_clean;
        // The map-backed Sources backend has decoded text, not HTTP provenance:
        // its resource() branch deliberately leaves classification false.
        Ok(RetainedStylesheetResponse {
            sheet: None, final_url: url, source: Arc::from(source),
            referrer_policy: policy, encoding,
            proof: NativeStylesheetResponseAuthority::Standalone(StandaloneStylesheetAuthority {
                document, node, revision, cancellation: cancellation.clone(), origin_clean,
            }), guard: None,
        })
    }

    #[cfg(unix)]
    #[expect(clippy::too_many_arguments, reason = "recursive retained walk shares exact owner and cancellation without allocating a second context")]
    fn retained_tree(
        &mut self, response: RetainedStylesheetResponse, depth: usize,
        ancestors: &mut Vec<String>, document: ParserDocumentIdentity,
        node: NodeId, revision: u64,
        cancellation: &Arc<crate::context_service::ResourceCancellation>,
    ) -> Result<StylesheetSourceTree, HostError> {
        self.check()?;
        if depth > 8 { return Err(HostError::LimitExceeded {
            resource: "stylesheet import depth".into() }); }
        // Discovery parser remains the existing native parser; opaque parser
        // allocator physical/RSS bounds are not claimed by source reservations.
        let rules = oryn_layout::cssom::read_stylesheet(
            &response.source, &response.final_url, false, true).map_err(HostError::Execution)?;
        let mut imports = Vec::new();
        for (index, rule) in rules.into_iter().enumerate() {
            self.check()?;
            if rule.rule_type != 3 { continue; }
            let Some(href) = rule.properties.get("href") else { continue; };
            let requested_url = match self.backend.resolve_stylesheet_request(&response.final_url, href) {
                Ok(url) => url,
                Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(message))) => {
                    // Original parser href remains the occurrence identity even
                    // when the actual URL parser refuses a network request.
                    self.retain(href.len().checked_add(message.len())
                        .and_then(|n| n.checked_add(std::mem::size_of::<StylesheetSourceEdge>()))
                        .ok_or_else(|| HostError::Execution("failed URL metadata overflow".into()))?)?;
                    imports.try_reserve(1).map_err(|e| HostError::Execution(e.to_string()))?;
                    imports.push(StylesheetSourceEdge { index, requested_url: href.clone(),
                        response: StylesheetSourceOutcome::Failed {
                            _diagnostic: crate::RuntimeError::Network(message),
                        },
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            let key = stylesheet_cycle_key(&requested_url);
            let outcome = if rule.properties.get("supportsMatches").is_some_and(|s| s == "false") {
                StylesheetSourceOutcome::Refused
            } else if ancestors.contains(&key) {
                StylesheetSourceOutcome::Cycle
            } else {
                let fetched = self.retained_response(&requested_url, &response.final_url,
                    &response.referrer_policy, &response.encoding, false,
                    document, node, revision, cancellation);
                let child = match fetched {
                    Ok(child) => child,
                    Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(message)
                        | crate::script_load_failure::ScriptLoadFailure::Integrity(message))) => {
                        self.retain(requested_url.len().checked_add(message.len())
                            .and_then(|n| n.checked_add(std::mem::size_of::<StylesheetSourceEdge>()))
                            .ok_or_else(|| HostError::Execution("failed import metadata overflow".into()))?)?;
                        imports.try_reserve(1).map_err(|e| HostError::Execution(e.to_string()))?;
                        imports.push(StylesheetSourceEdge {
                            index, requested_url,
                            response: StylesheetSourceOutcome::Failed {
                                _diagnostic: crate::RuntimeError::Network(message),
                            },
                        });
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let final_key = stylesheet_cycle_key(&child.final_url);
                if ancestors.contains(&final_key) {
                    StylesheetSourceOutcome::Cycle
                } else {
                    if depth == 8 { return Err(HostError::LimitExceeded {
                        resource: "stylesheet import depth".into() }); }
                    ancestors.push(final_key);
                    let child = self.retained_tree(child, depth + 1, ancestors,
                        document, node, revision, cancellation)?;
                    ancestors.pop();
                    StylesheetSourceOutcome::Loaded(Box::new(child))
                }
            };
            self.retain(requested_url.len().checked_add(std::mem::size_of::<StylesheetSourceEdge>())
                .ok_or_else(|| HostError::Execution("standalone import metadata overflow".into()))?)?;
            imports.try_reserve(1).map_err(|e| HostError::Execution(e.to_string()))?;
            imports.push(StylesheetSourceEdge { index, requested_url, response: outcome });
        }
        Ok(StylesheetSourceTree { response, imports })
    }
}
#[cfg(all(test, unix))]
mod retained_standalone_controls {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    struct Broker;
    impl NetworkBroker for Broker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let source = if request.url.ends_with("/bad.css") {
                "not CSS"
            } else if request.url.ends_with("/root.css") {
                "@import url(child.css);p{width:17px}"
            } else if request.url.ends_with("/child.css") {
                "@import url(grandchild.css);p{height:23px}"
            } else { "p{margin-left:29px}" };
            Ok(NetworkResponse {
                request_id: request.request_id, status: 200,
                headers: vec![("content-type".into(), if request.url.ends_with("/bad.css") {
                    "text/plain".into()
                } else { "text/css".into() }), ("x-content-type-options".into(), "nosniff".into())],
                body: source.as_bytes().to_vec(), response_policy: None, timing: None,
            })
        }
    }
    #[test]
    fn standalone_broker_original_import_tree_and_owner_identity() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><link id=s rel=stylesheet href=root.css><p id=p></p>",
            &BTreeMap::new(), Some(SharedNetworkBroker::new(Broker)),
            Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while page.parser_pending() || !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("const s=document.getElementById('s').sheet,r=s.cssRules[0],p=document.getElementById('p');[s===document.styleSheets[0],r.styleSheet.parentStyleSheet===s,r.styleSheet.ownerRule===r,getComputedStyle(p).width,getComputedStyle(p).height].join('|')").unwrap(), "true|true|true|17px|23px");
        assert!(page.diagnostics.iter().all(|message|
            !message.detail.as_deref().is_some_and(|detail|
                detail.contains("retained stylesheet broker context absent"))));
    }
    #[test]
    fn standalone_late_import_keeps_original_rule_and_failed_mime_is_null() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><style id=s>p{width:17px}</style><p id=p></p>",
            &BTreeMap::new(), Some(SharedNetworkBroker::new(Broker)),
            Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        page.execute("globalThis.s=document.getElementById('s').sheet;s.insertRule('@import url(child.css)',0);globalThis.r=s.cssRules[0]").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page.evaluate("String(r.styleSheet!==null)").unwrap() == "true" { break; }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("[s.cssRules[0]===r,r.styleSheet.parentStyleSheet===s,getComputedStyle(document.getElementById('p')).height].join('|')").unwrap(), "true|true|23px");
        assert_eq!(page.evaluate("[r.styleSheet.cssRules[0].styleSheet.parentStyleSheet===r.styleSheet,r.styleSheet.cssRules[0].styleSheet.ownerRule===r.styleSheet.cssRules[0],getComputedStyle(document.getElementById('p')).marginLeft].join('|')").unwrap(), "true|true|29px");
        page.execute("s.insertRule('@import url(bad.css)',0);globalThis.bad=s.cssRules[0]").unwrap();
        let failure_deadline = std::time::Instant::now() + Duration::from_secs(5);
        // Execute first owning checkpoint so the exact pending occurrence is
        // extracted; then await native async scheduler work, not parser EOF.
        page.poll_context_events().unwrap();
        while page.has_parser_async_modules(Some(page.top_realm)) || page.dynamic_stylesheets_pending(None) {
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now() < failure_deadline);
        }
        assert_eq!(page.evaluate("String(bad.styleSheet===null&&s.cssRules[1]===r&&r.styleSheet!==null)").unwrap(), "true");
    }
    #[test]
    fn standalone_missing_mapped_child_preserves_native_root() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><style id=s>@import url(missing.css);p{width:17px}</style><p id=p></p>",
            &BTreeMap::new(), None, Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while page.parser_pending() || !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now() < deadline);
        }
        assert_eq!(page.evaluate("const s=document.getElementById('s').sheet;[s!==null,s.cssRules[0].styleSheet===null,getComputedStyle(document.getElementById('p')).width].join('|')").unwrap(), "true|true|17px");
    }
}
