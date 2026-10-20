//! Captured standalone CSS resource loading. Only bounded IO lanes call load;
//! the page retains revision, cancellation and publication ownership.

/// Inputs retained together for expand.
struct StylesheetExpansionInput<'a> {
    url: &'a str,
    referrer: &'a str,
    policy: &'a str,
    css: &'a str,
    encoding: &'a str,
    depth: usize,
    ancestors: &'a mut Vec<String>,
}
use super::*;
use crate::cookies::{CookieCredentials, CookieRequestContext, CookieRequestKind};
use crate::network::{NetworkRequest, collect_operation_bounded_with_cancel};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

include!("standalone_retained_tree.rs");

pub(super) struct Cancel(
    pub Arc<AtomicBool>,
    #[cfg(unix)] Option<Arc<crate::context_service::ResourceCancellation>>,
    bool,
);
impl Cancel {
    pub(super) fn complete(mut self) {
        #[cfg(unix)]
        { self.2 = false; }
        #[cfg(not(unix))]
        { self.1 = false; }
    }
    pub(super) fn new(flag: Arc<AtomicBool>) -> Self {
        Self(
            flag,
            #[cfg(unix)]
            None,
            true,
        )
    }
    #[cfg(unix)]
    pub(super) fn native(scope: Arc<crate::context_service::ResourceCancellation>) -> Self {
        Self(scope.flag.clone(), Some(scope), true)
    }
}
impl Drop for Cancel {
    fn drop(&mut self) {
        #[cfg(unix)]
        let armed = self.2;
        #[cfg(not(unix))]
        let armed = self.1;
        if !armed { return; }
        self.0.store(true, Ordering::Release);
        #[cfg(unix)]
        if let Some(scope) = &self.1 {
            scope.cancel();
        }
    }
}
pub(super) struct Loader {
    render_blocking: bool,
    retained_fetch: bool,
    last_origin_clean: bool,
    // Native fetch origin, not the srcdoc/about:blank serialization or a base URL.
    source_origin: Option<oryn_js::origin::Origin>,
    preload_status: Option<crate::render_request_status::Status>,
    preload_trace: Vec<String>,
    quirks: bool,
    backend: module_graph_load::ModuleGraphBackend,
    policy: Option<crate::csp::CspPolicySet>,
    cancel: Arc<AtomicBool>,
    budget: Arc<std::sync::atomic::AtomicUsize>,
    retained: Vec<ScriptSourceReservation>,
    dependencies: CssDependencies,
    binary: Option<super::dynamic_stylesheet_preparation::CssBinary>,
    next: u64,
    end: u64,
    count: usize,
    deadline: std::time::Instant,
    initiator: String,
    referrer_policy: String,
    integrity: Option<String>,
    cross_origin: Option<String>,
    nonce: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use std::sync::{Mutex, mpsc};
    #[test]
    fn inline_style_policy_preserves_dom_but_suppresses_layout_and_cssom_cannot_enable_denied_sheet()
     {
        let mut page=RustExecutableDocument::load(r#"<meta http-equiv='Content-Security-Policy' content="style-src 'nonce-good'; style-src-attr 'none'"><style id=denied>#probe{margin-left:91px}</style><style nonce=good>#probe{margin-top:13px}</style><div id=probe style='margin-right:73px'></div>"#).unwrap();
        assert_eq!(page.evaluate(r#"const p=document.getElementById('probe'),s=document.getElementById('denied');s.disabled=false;[s.sheet===null,p.getAttribute('style').includes('73px'),getComputedStyle(p).marginLeft==='0px',getComputedStyle(p).marginRight==='0px',getComputedStyle(p).marginTop==='13px'].every(Boolean)"#).unwrap(),"true");
        assert_eq!(page.evaluate(r#"p.style.cssText='margin-left:2px';const first=getComputedStyle(p).marginLeft;p.setAttribute('style','margin-left:40px');[first==='2px',getComputedStyle(p).marginLeft==='0px',p.getAttribute('style')==='margin-left:40px'].every(Boolean)"#).unwrap(),"true");
        assert_eq!(page.evaluate(r#"s.setAttribute('nonce','good');s.textContent='#probe{margin-bottom:17px}';getComputedStyle(p).marginBottom==='17px'&&s.sheet!==null"#).unwrap(),"true");
    }
    #[test]
    fn header_nonce_slot_survives_hiding_and_authorizes_style_and_dynamic_script() {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[(
                "Content-Security-Policy".into(),
                "style-src 'nonce-abc'; script-src 'nonce-abc'".into(),
            )],
        );
        let resources = BTreeMap::from([(
            "oryn:csp-policy".into(),
            serde_json::to_string(&policy).unwrap(),
        )]);
        let mut page = RustExecutableDocument::load_with_environment(
            "<style id=sheet nonce=abc>#probe{margin-left:14px}</style><div id=probe></div>",
            &resources,
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"const sheet=document.getElementById('sheet');[sheet.nonce==='abc',sheet.getAttribute('nonce')==='',document.querySelector('[nonce=abc]')===null,getComputedStyle(document.getElementById('probe')).marginLeft==='14px',sheet.cloneNode(true).nonce==='abc'].every(Boolean)"#).unwrap(),"true");
        assert_eq!(page.evaluate(r#"const script=document.createElement('script');script.nonce='abc';script.textContent='globalThis.nonceExecuted=true';document.body.append(script);script.getAttribute('nonce')===null&&script.nonce==='abc'"#).unwrap(),"true");
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("nonceExecuted").unwrap(), "true");
        assert_eq!(page.evaluate(r#"sheet.setAttribute('nonce','visible');sheet.nonce='private';sheet.getAttribute('nonce')==='visible'&&sheet.nonce==='private'"#).unwrap(),"true");
    }
    #[test]
    fn standalone_inline_style_violation_is_deferred_trusted_and_not_repeated_by_layout_reads() {
        let mut page=RustExecutableDocument::load(r#"<meta http-equiv='Content-Security-Policy' content="style-src 'none'"><div id=probe></div>"#).unwrap();
        assert_eq!(page.agent.host.evaluate_string(r#"globalThis.violations=[];document.addEventListener('securitypolicyviolation',e=>violations.push([e.effectiveDirective,e.isTrusted,e.target===document]));const p=document.getElementById('probe');p.setAttribute('style','margin-left:73px');getComputedStyle(p).marginLeft;getComputedStyle(p).marginLeft;violations.length"#).unwrap(),"0");
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("JSON.stringify(violations)")
                .unwrap(),
            r#"[["style-src-attr",true,true]]"#
        );
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("violations.length")
                .unwrap(),
            "1"
        );
    }
    #[cfg(unix)]
    #[test]
    fn parent_css_expansion_retains_shared_bytes_and_cancellation_prevents_next_import() {
        let bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut expansion = StylesheetExpansionBudget::new(bytes.clone(), cancel.clone());
        expansion.retain(4096).unwrap();
        assert_eq!(bytes.load(Ordering::Acquire), 4096);
        cancel.store(true, Ordering::Release);
        assert!(expansion.request().is_err());
        drop(expansion);
        assert_eq!(bytes.load(Ordering::Acquire), 0);
        let mut expansion =
            StylesheetExpansionBudget::new(bytes.clone(), Arc::new(AtomicBool::new(false)));
        for _ in 0..256 {
            expansion.request().unwrap();
        }
        assert!(expansion.request().is_err());
        assert!(
            expansion
                .retain(MAX_QUEUED_SCRIPT_SOURCE_BYTES + 1)
                .is_err()
        );
        assert_eq!(bytes.load(Ordering::Acquire), 0);
    }
    #[test]
    fn later_meta_style_policy_does_not_revoke_already_prepared_sheet() {
        let mut page=RustExecutableDocument::load(r#"<style>#probe{margin-left:11px}</style><meta http-equiv='Content-Security-Policy' content="style-src 'none'"><div id=probe></div>"#).unwrap();
        assert_eq!(
            page.evaluate("getComputedStyle(document.getElementById('probe')).marginLeft")
                .unwrap(),
            "11px"
        );
        assert_eq!(page.evaluate("document.querySelector('style').textContent='#probe{margin-left:33px}';getComputedStyle(document.getElementById('probe')).marginLeft").unwrap(),"0px");
    }
    #[test]
    fn standalone_inline_policy_report_reaches_broker_and_native_event_independently_of_load() {
        struct Reports(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Reports {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let request_id = request.request_id;
                self.0.lock().unwrap().push(request);
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id,
                    status: 204,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }
        let observed = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Reports(observed.clone()));
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[(
                "Content-Security-Policy-Report-Only".into(),
                "style-src 'none'; report-uri /csp-report".into(),
            )],
        );
        let resources = BTreeMap::from([(
            "oryn:csp-policy".into(),
            serde_json::to_string(&policy).unwrap(),
        )]);
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><p id=probe></p>",
            &resources,
            Some(broker),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        page.execute("globalThis.policyEvents=[];document.addEventListener('securitypolicyviolation',e=>policyEvents.push(e.disposition));document.getElementById('probe').setAttribute('style','margin-left:9px')").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while observed.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            page.poll_context_events().unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("policyEvents.join(',')").unwrap(), "report");
        assert_eq!(
            page.evaluate("getComputedStyle(document.getElementById('probe')).marginLeft")
                .unwrap(),
            "9px"
        );
        let requests = observed.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://example.test/csp-report");
        assert_eq!(requests[0].method, "POST");
        assert!(!requests[0].credentials);
    }

    #[test]
    fn nonceable_algorithm_rejects_parser_duplicates_and_dangling_attribute_tokens() {
        let source = r#"<meta http-equiv=Content-Security-Policy content="script-src 'nonce-secret'"><script nonce=secret>globalThis.ran=[]</script><script nonce=secret data="aa<sCrIpTbb">ran.push('script')</script><script nonce=secret data="<StYlE">ran.push('style')</script><script nonce=secret data="<LiNk">ran.push('link')</script><script nonce=secret nonce=secret>ran.push('duplicate')</script><script nonce=secret>ran.push('allowed')</script>"#;
        let mut page = RustExecutableDocument::load(source).unwrap();
        assert_eq!(page.evaluate("ran.join(',')").unwrap(), "allowed");
        assert_eq!(
            page.evaluate("document.querySelector('[data]').nonce")
                .unwrap(),
            "secret"
        );
    }

    #[test]
    fn dangling_body_and_html_parser_attributes_cannot_publish_hidden_header_nonce() {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'nonce-secret'".into(),
            )],
        );
        let resources = BTreeMap::from([(
            "oryn:csp-policy".into(),
            serde_json::to_string(&policy).unwrap(),
        )]);
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><body <script nonce=secret></script><html <script nonce=secret></script>",
            &resources,
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate("document.body.getAttribute('nonce')===''&&document.documentElement.getAttribute('nonce')===''&&document.querySelector('[nonce*=secret]')===null").unwrap(),"true");
    }

    #[test]
    fn csp_meta_requires_head_insertion_and_content_changes_do_not_reprocess() {
        let mut page=RustExecutableDocument::load(r#"<body><meta http-equiv=Content-Security-Policy content="script-src 'none'"><script>globalThis.bodyPolicyIgnored=true</script>"#).unwrap();
        assert_eq!(page.evaluate("bodyPolicyIgnored").unwrap(), "true");
        page.execute(r#"const meta=document.createElement('meta');meta.httpEquiv='Content-Security-Policy';meta.content="script-src 'none'";document.head.append(meta);meta.content="script-src 'unsafe-inline'";const script=document.createElement('script');script.textContent='globalThis.mustRemainAbsent=true';document.body.append(script)"#).unwrap();
        assert_eq!(
            page.evaluate("typeof mustRemainAbsent").unwrap(),
            "undefined"
        );
    }

    #[test]
    fn standalone_script_policy_is_captured_before_later_meta_and_inline_insertion_is_checked() {
        let source = r#"<script>globalThis.order=[]</script><script type=module>order.push('earlier')</script><meta http-equiv=Content-Security-Policy content="script-src 'none'"><script>order.push('later')</script>"#;
        let mut page = RustExecutableDocument::load_parsed_with_limits(
            html::parse(source),
            &BTreeMap::new(),
            None,
            None,
            256 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(page.evaluate("order.join(',')").unwrap(), "earlier");
        page.execute(r#"const added=document.createElement('script');added.textContent="order.push('inserted')";document.body.append(added)"#).unwrap();
        assert_eq!(page.evaluate("order.join(',')").unwrap(), "earlier");
    }

    #[test]
    fn standalone_css_redirect_charset_import_and_cookie_destination_are_native() {
        struct Reply(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let (status, headers, body) = if request.url.ends_with("/start.css") {
                    (
                        302,
                        vec![
                            ("location".into(), "/final.css".into()),
                            ("referrer-policy".into(), "no-referrer".into()),
                        ],
                        Vec::new(),
                    )
                } else if request.url.ends_with("/final.css") {
                    (
                        200,
                        vec![(
                            "content-type".into(),
                            "text/css;charset=windows-1252".into(),
                        )],
                        b"@import url(child.css) layer(theme) screen;body::before{content:'\xe9'}"
                            .to_vec(),
                    )
                } else {
                    (
                        200,
                        vec![("content-type".into(), "text/css".into())],
                        b"body{width:42px}".to_vec(),
                    )
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status,
                    headers,
                    body,
                })
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut page = RustExecutableDocument::load_with_environment(
            "<link id=owner type=text/plain>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Reply(seen.clone()))),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let node = page.node_by_id("owner").unwrap();
        let fetch = ParserFetchContext::capture(&page.agent.host, &page.agent.dom, node).unwrap();
        let (loader, _cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            page.dynamic_scripts.source_bytes.clone(),
        )
        .unwrap();
        let (url, css, _) = loader
            .load(true, true, &fetch.base_url, "start.css", "", "UTF-8")
            .unwrap();
        assert_eq!(url, "https://example.test/final.css");
        assert!(css.contains('é'));
        assert!(css.contains("@layer theme"));
        assert!(css.contains("@media screen"));
        assert!(css.contains("42px"));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(
            seen.iter()
                .all(|request| request.security.kind == CookieRequestKind::Style)
        );
        assert!(seen[1..].iter().all(|request| {
            !request
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
        }));
        assert!(
            seen.windows(2)
                .all(|requests| requests[0].request_id != requests[1].request_id)
        );
    }
    #[test]
    fn stylesheet_without_waiting_script_allows_dom_content_loaded_before_response() {
        struct Held {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl NetworkBroker for Held {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/css".into())],
                    body: b"body{width:42px}".to_vec(),
                })
            }
        }
        let (started, observed) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let source = "<script>document.addEventListener('DOMContentLoaded',()=>globalThis.domReady=true);addEventListener('load',()=>globalThis.loaded=true)</script><link rel=stylesheet href=held.css>";
        let mut page = retained_fixture_stream(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            "https://example.test/",
            SharedNetworkBroker::new(Held {
                started,
                release: Mutex::new(blocked),
            }),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if observed.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        for _ in 0..8 {
            page.checkpoint().unwrap();
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("domReady+'|'+typeof loaded+'|'+document.readyState")
                .unwrap(),
            "true|undefined|interactive"
        );
        assert!(!page.parser_pending());
        assert!(!page.document_load_complete());
        release.send(()).unwrap();
        while !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(page.agent.host.evaluate_string("loaded").unwrap(), "true");
    }
    #[test]
    fn font_sources_fall_back_after_decode_failure_and_stop_after_success() {
        struct Reply(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url.clone());
                let body = if request.url.ends_with("bad.woff2") {
                    b"invalid font".to_vec()
                } else {
                    oryn_layout::DEFAULT_FONT_BYTES.to_vec()
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "font/ttf".into())],
                    body,
                })
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let source = "<style>@font-face{font-family:Fallback;src:url(bad.woff2),url(good.ttf),url(unused.ttf)}p{font-family:Fallback}</style><p>Text</p>";
        let mut page = RustExecutableDocument::load_stream_with_limits(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Reply(seen.clone()))),
            Some(Url::parse("https://example.test/").unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "https://example.test/bad.woff2".to_owned(),
                "https://example.test/good.ttf".to_owned()
            ]
        );
        assert_eq!(page.css_binary_resources.len(), 1);
    }
    #[test]
    fn css_resource_demand_waits_for_computed_use_and_rechecks_after_mutation() {
        struct Reply(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url.clone());
                let (mime, body) = if request.url.ends_with("theme.css") {
                    ("text/css",b"@font-face{font-family:Demand;src:url(font.ttf)}body.enabled{font-family:Demand;background-image:url(image.svg)}".to_vec())
                } else if request.url.ends_with("font.ttf") {
                    ("font/ttf", oryn_layout::DEFAULT_FONT_BYTES.to_vec())
                } else {
                    (
                        "image/svg+xml",
                        b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>".to_vec(),
                    )
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), mime.into())],
                    body,
                })
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let source = "<link rel=stylesheet href=theme.css><body><p>Visible text</p></body>";
        let mut page = RustExecutableDocument::load_stream_with_limits(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Reply(seen.clone()))),
            Some(Url::parse("https://example.test/").unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["https://example.test/theme.css".to_owned()]
        );
        page.agent
            .host
            .execute("document.body.className='enabled'")
            .unwrap();
        while page.css_binary_resources.is_empty()
            || page.css_paint_images(&BTreeSet::new()).unwrap().is_empty()
        {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        let requests = seen.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|url| url.ends_with("font.ttf"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|url| url.ends_with("image.svg"))
                .count(),
            1
        );
        drop(requests);
        for _ in 0..4 {
            page.checkpoint().unwrap();
        }
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
    #[test]
    fn unavailable_font_does_not_hold_parser_script_or_fail_stylesheet() {
        struct Reply {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            images: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let (status, mime, body) = if request.url.ends_with("/theme.css") {
                    (200,"text/css",b"@font-face{font-family:Demo;src:url(blocked.woff2)}body{font-family:Demo;background-image:url(image.png);width:42px}".to_vec())
                } else if request.url.ends_with("/blocked.woff2") {
                    self.started.send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                    (404, "font/woff2", Vec::new())
                } else {
                    self.images.fetch_add(1, Ordering::AcqRel);
                    (200, "image/png", Vec::new())
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status,
                    headers: vec![("content-type".into(), mime.into())],
                    body,
                })
            }
        }
        let (started, observed) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let images = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source = "<link id=sheet rel=stylesheet href=theme.css onload='globalThis.sheetLoaded=true' onerror='globalThis.sheetFailed=true'><script>globalThis.afterSheet=true;document.addEventListener('DOMContentLoaded',()=>globalThis.domReady=true)</script><p>Font demand</p>";
        let mut page = RustExecutableDocument::load_stream_with_limits(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Reply {
                started,
                release: Mutex::new(blocked),
                images: images.clone(),
            })),
            Some(Url::parse("https://example.test/").unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if observed.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        for _ in 0..8 {
            page.checkpoint().unwrap();
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("afterSheet+'|'+domReady+'|'+sheetLoaded+'|'+typeof sheetFailed")
                .unwrap(),
            "true|true|true|undefined"
        );
        while images.load(Ordering::Acquire) == 0 {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        while !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(images.load(Ordering::Acquire), 1);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof sheetFailed")
                .unwrap(),
            "undefined"
        );
    }
    #[test]
    fn standalone_snapshot_only_waits_for_sheets_before_the_current_script() {
        struct Held {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl NetworkBroker for Held {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/css".into())],
                    body: b"body{width:42px}".to_vec(),
                })
            }
        }
        let (started, observed) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let parsed = html::parse(
            "<script>globalThis.beforeSheet=true</script><link rel=stylesheet href=held.css><script>globalThis.afterSheet=true</script>",
        );
        let mut page = RustExecutableDocument::load_with_options(
            DocumentLoad::ScheduledSnapshot(Box::new(parsed)),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Held {
                started,
                release: Mutex::new(blocked),
            })),
            Some(Url::parse("https://example.test/").unwrap()),
            256 * 1024 * 1024,
            None,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if observed.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        for _ in 0..3 {
            page.checkpoint().unwrap();
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("beforeSheet+'|'+typeof afterSheet")
                .unwrap(),
            "true|undefined"
        );
        release.send(()).unwrap();
        while page.parser_pending() || !page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(
            page.agent.host.evaluate_string("afterSheet").unwrap(),
            "true"
        );
    }
    #[test]
    #[cfg(unix)]
    fn srcdoc_same_origin_broker_sheet_is_accessible_before_parser_script() {
        struct Reply;
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                assert_eq!(request.url,"https://example.test/child.css");
                Ok(NetworkResponse {
                    response_policy:None,timing:None,request_id:request.request_id,
                    status:200,headers:vec![("content-type".into(),"text/css".into())],
                    body:b"body{width:42px}".to_vec(),
                })
            }
        }
        let source="<iframe id=child srcdoc='<link id=owner rel=stylesheet href=https://example.test/child.css><script>try{const s=document.styleSheets[0];parent.brokerSheetReady=[s===document.getElementById(\"owner\").sheet,s.ownerNode===document.getElementById(\"owner\"),s.cssRules.length].join(\"|\")}catch(e){parent.brokerSheetReady=e.name+\":\"+e.message}</script><body></body>'></iframe>";
        let mut page=retained_fixture_stream(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            "https://example.test/",SharedNetworkBroker::new(Reply),
        ).unwrap();
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page.agent.host.evaluate_string("typeof brokerSheetReady").unwrap()=="string" {
                break;
            }
            assert!(std::time::Instant::now()<deadline,"broker srcdoc parser stylesheet pending");
            std::thread::yield_now();
        }
        assert_eq!(page.agent.host.evaluate_string("brokerSheetReady").unwrap(),"true|true|1");
        while page.parser_pending()||!page.document_load_complete() {
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now()<deadline,"broker srcdoc load pending");
        }
        assert_eq!(page.agent.host.evaluate_string("child.contentWindow.getComputedStyle(child.contentDocument.body).width").unwrap(),"42px");
    }
    #[test]
    fn standalone_srcdoc_stylesheet_preparation_precedes_child_parser_script() {
        let resources = BTreeMap::from([(
            "https://example.test/child.css".into(),
            "body{width:42px}".into(),
        )]);
        let mut page=RustExecutableDocument::load_with_environment("<iframe id=child srcdoc='<link id=owner rel=stylesheet href=https://example.test/child.css><script>try{parent.childSheetReady=document.styleSheets[0].cssRules.length}catch(error){parent.childSheetOutcome=JSON.stringify({name:error.name,message:error.message,hasSheet:document.styleSheets.length===1,hasBody:document.body!==null});parent.childSheetReady=-1}</script>'></iframe>",&resources,None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page
                .agent
                .host
                .evaluate_string("typeof childSheetReady")
                .unwrap()
                == "number"
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        let actual = page.agent.host.evaluate_string("childSheetReady").unwrap();
        let failure = if actual != "1" {page.agent.host.evaluate_string("childSheetOutcome").unwrap()} else {String::new()};
        assert_eq!(
            actual,
            "1", "original srcdoc stylesheet accessibility diagnostic: {failure}"
        );
    }
    #[test]
    fn standalone_srcdoc_cross_origin_source_keeps_owner_but_denies_css_rules() {
        let resources=BTreeMap::from([("https://other.test/child.css".into(),"body{width:42px}".into())]);
        let mut page=RustExecutableDocument::load_with_environment(
            "<iframe srcdoc='<link id=owner rel=stylesheet href=https://other.test/child.css><script>const s=document.styleSheets[0];parent.sourceOwner=s.ownerNode===document.getElementById(\"owner\");try{void s.cssRules;parent.sourceAccess=\"allowed\"}catch(e){parent.sourceAccess=e.name}</script>'></iframe>",
            &resources,None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        while page.agent.host.evaluate_string("typeof sourceAccess").unwrap()!="string" {
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now()<deadline,"cross-origin srcdoc preparation pending");
            std::thread::yield_now();
        }
        assert_eq!(page.agent.host.evaluate_string("sourceOwner").unwrap(),"true");
        assert_eq!(page.agent.host.evaluate_string("sourceAccess").unwrap(),"SecurityError");
    }
    #[test]
    fn standalone_stylesheet_rejects_nosniff_cors_and_integrity_before_publication() {
        struct Reply {
            headers: Vec<(String, String)>,
        }
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: self.headers.clone(),
                    body: b"body{width:42px}".to_vec(),
                })
            }
        }
        for (mime, nosniff, cors, integrity) in [
            ("text/plain", true, false, false),
            ("text/css", false, true, false),
            ("text/css", false, false, true),
        ] {
            let mut headers = vec![("content-type".into(), mime.into())];
            if nosniff {
                headers.push(("x-content-type-options".into(), "nosniff".into()));
            }
            let mut page = RustExecutableDocument::load_with_environment(
                "<link id=owner type=text/plain>",
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(Reply { headers })),
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            let node = page.node_by_id("owner").unwrap();
            let mut fetch =
                ParserFetchContext::capture(&page.agent.host, &page.agent.dom, node).unwrap();
            if cors {
                fetch.cross_origin = Some("anonymous".into());
            }
            if integrity {
                fetch.integrity =
                    Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into());
            }
            let budget = page.dynamic_scripts.source_bytes.clone();
            let before = budget.load(Ordering::Acquire);
            let (loader, _cancel) = Loader::capture(
                &mut page.agent.host,
                &page.resources,
                &fetch,
                budget.clone(),
            )
            .unwrap();
            assert!(
                loader
                    .load(
                        true,
                        true,
                        &fetch.base_url,
                        "https://other.test/blocked.css",
                        "",
                        "UTF-8"
                    )
                    .is_err()
            );
            assert_eq!(
                budget.load(Ordering::Acquire),
                before,
                "failed stylesheet releases its exact charge without discarding other page custody"
            );
            assert!(
                page.agent
                    .dom
                    .borrow()
                    .arena()
                    .stylesheet_text(node)
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn standalone_dynamic_stylesheet_uses_the_captured_native_css_loader() {
        let resources = BTreeMap::from([
            (
                "https://example.test/theme.css".into(),
                "@import url(child.css);body{width:42px}".into(),
            ),
            (
                "https://example.test/child.css".into(),
                "body{height:17px}".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(
            "<body>",
            &resources,
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        page.agent.host.execute("const sheet=document.createElement('link');sheet.id='dynamic';sheet.rel='stylesheet';sheet.href='theme.css';sheet.onload=()=>globalThis.sheetReady=true;document.head.append(sheet)").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page
                .agent
                .host
                .evaluate_string("typeof sheetReady")
                .unwrap()
                == "boolean"
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        let node = page.node_by_id("dynamic").unwrap();
        let css = page
            .agent
            .dom
            .borrow()
            .arena()
            .stylesheet_text(node)
            .unwrap();
        assert_eq!(css, "@import url(child.css);body{width:42px}");
        assert_eq!(
            page.evaluate(
                "const owner=document.getElementById('dynamic'),root=owner.sheet,list=root.cssRules,importRule=list[0],child=importRule.styleSheet;[root===document.styleSheets[0],root.ownerNode===owner,list.length===2&&importRule.parentStyleSheet===root&&child!==null&&child.ownerRule===importRule&&child.cssRules.length===1,getComputedStyle(document.body).width,getComputedStyle(document.body).height].join('|')",
            ).unwrap(),
            "true|true|true|42px|17px",
        );
        // The original same-origin Sources URL is readable under the captured
        // native origin; the separate cross-origin control still denies access.
        let realm = page.agent.host.active_realm();
        let provider = page.agent.host.native_layout_provider_in_realm(realm).unwrap();
        let backend = provider.native_backend().unwrap().downcast_ref::<LiveLayoutProvider>().unwrap();
        let allocation = backend.native_retained_allocation.borrow().as_ref().unwrap().clone();
        page.agent.host.refresh_native_retained_allocation(realm, &allocation).unwrap();
        let _scope = allocation.scope_guard();
        let admission = backend.retained_revision_admission().unwrap();
        let snapshot = RetainedPublicationSnapshot::capture(&backend.retained_stylesheets, admission.as_ref()).unwrap().unwrap();
        let record = snapshot.entries.iter().find(|entry| entry.record.owner.node == node).unwrap().record.clone();
        // Top-level capture has zero ancestor metadata. This test-local adapter
        // holds a real admitted lease and cannot invoke ledger/author callbacks
        // while the existing native rule_at holds its shared style read lock.
        struct PreadmittedTopLevel(Arc<dyn Send + Sync>);
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for PreadmittedTopLevel {
            fn checkpoint(&self) -> Result<(), String> { Ok(()) }
            fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
                if bytes != 0 { return Err("unexpected ancestor metadata".into()); }
                Ok(self.0.clone())
            }
        }
        let capture = PreadmittedTopLevel(admission.reserve(0).unwrap());
        let graph = record.graph.borrow();
        let root = graph.root();
        let import = graph.imports().find(|entry| entry.parent.same_sheet(&root)).unwrap();
        let child = import.child.as_ref().unwrap();
        assert!(import.parent.same_sheet(&root));
        assert!(graph.owner(child).unwrap().same_rule(import));
        let rule = graph.rule_at(&root, &[], 0, &capture).unwrap();
        assert!(graph.import_for_rule(&rule).unwrap().same_rule(import));
        assert_eq!(graph.rule_at(&root, &[], 1, &capture).unwrap().view().unwrap().declaration_text.as_deref(), Some("width: 42px;"));
        assert_eq!(graph.rule_at(child, &[], 0, &capture).unwrap().view().unwrap().declaration_text.as_deref(), Some("height: 17px;"));
    }
    #[test]
    fn brokerless_data_stylesheet_decodes_original_bytes_and_keeps_resource_guards() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><link id=owner type=text/plain>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let node = page.node_by_id("owner").unwrap();
        let fetch = ParserFetchContext::capture(&page.agent.host, &page.agent.dom, node).unwrap();
        let budget = page.dynamic_scripts.source_bytes.clone();
        let before = budget.load(Ordering::Acquire);
        let (mut loader, _cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            budget.clone(),
        )
        .unwrap();
        let (_, css, encoding, _) = loader
            .resource(
                "data:text/css;charset=windows-1252,body%3A%3Abefore%7Bcontent%3A%27%E9%27%7D",
                &fetch.base_url,
                "",
                "UTF-8",
                "stylesheet",
                true,
            )
            .unwrap();
        assert_eq!(css, "body::before{content:'é'}");
        assert_eq!(encoding, "windows-1252");
        drop(loader);
        assert_eq!(budget.load(Ordering::Acquire), before);
        let mut integrity_fetch = fetch.clone();
        integrity_fetch.integrity =
            Some("sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=".into());
        let (loader, _cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &integrity_fetch,
            budget.clone(),
        )
        .unwrap();
        assert!(
            loader
                .load(true, true, &fetch.base_url, "data:text/css,", "", "UTF-8")
                .unwrap()
                .1
                .is_empty()
        );
        assert_eq!(budget.load(Ordering::Acquire), before);
        for (href, integrity, expected) in [
            ("data:text/html,body%7Bcolor:red%7D", None, "MIME"),
            (
                "data:text/css,body%7Bcolor:red%7D",
                Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="),
                "integrity",
            ),
            (
                "data:text/css,%40import%20url(https://other.test/denied.css);",
                None,
                "resource unavailable",
            ),
        ] {
            let mut fetch = fetch.clone();
            fetch.integrity = integrity.map(str::to_owned);
            let (loader, _cancel) = Loader::capture(
                &mut page.agent.host,
                &page.resources,
                &fetch,
                budget.clone(),
            )
            .unwrap();
            let error = loader
                .load(true, true, &fetch.base_url, href, "", "UTF-8")
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert_eq!(budget.load(Ordering::Acquire), before);
        }
        let (loader, cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            budget.clone(),
        )
        .unwrap();
        drop(cancel);
        assert!(matches!(
            loader.load(true, true, &fetch.base_url, "data:text/css,", "", "UTF-8"),
            Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Cancelled
            ))
        ));
        page.agent
            .host
            .install_standalone_meta_policy("style-src 'none'")
            .unwrap();
        let (loader, _cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            budget.clone(),
        )
        .unwrap();
        assert!(
            loader
                .load(true, true, &fetch.base_url, "data:text/css,", "", "UTF-8")
                .unwrap_err()
                .to_string()
                .contains("Content-Security-Policy")
        );
        assert_eq!(budget.load(Ordering::Acquire), before);
        assert!(
            page.agent
                .host
                .network_broker_for_realm(page.top_realm)
                .is_none()
        );
    }
    #[test]
    fn brokerless_dynamic_data_link_publishes_its_sheet_only_after_load() {
        for (href, rules) in [
            ("data:text/css,", 0),
            ("data:text/css,body%7Bcolor:rgb(1,2,3)%7D", 1),
        ] {
            let mut page = RustExecutableDocument::load_with_environment(
                "<!doctype html><body>",
                &BTreeMap::new(),
                None,
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            assert_eq!(page.agent.host.evaluate_string(&format!("globalThis.sheetEvents=[];const link=document.createElement('link');link.rel='stylesheet';link.href={};link.onload=()=>sheetEvents.push(String(link.sheet!==null&&link.sheet.ownerNode===link&&link.sheet.cssRules.length==={rules}));link.onerror=()=>sheetEvents.push('error');document.head.append(link);String(link.sheet===null&&sheetEvents.length===0)",serde_json::to_string(href).unwrap())).unwrap(),"true");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                page.checkpoint().unwrap();
                let events = page
                    .agent
                    .host
                    .evaluate_string("sheetEvents.join(',')")
                    .unwrap();
                if !events.is_empty() {
                    assert_eq!(events, "true");
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "local stylesheet did not publish: {:?}",
                    page.diagnostics
                );
                std::thread::yield_now();
            }
            page.checkpoint().unwrap();
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("sheetEvents.join(',')")
                    .unwrap(),
                "true"
            );
            assert!(
                page.agent
                    .host
                    .network_broker_for_realm(page.top_realm)
                    .is_none()
            );
        }
    }
    #[test]
    fn standalone_stylesheet_mime_rejection_dispatches_error_for_empty_and_nonempty_bodies() {
        struct Sheet;
        impl NetworkBroker for Sheet {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    body: if request.url.ends_with("/empty") {
                        Vec::new()
                    } else {
                        b"body{margin-left:79px}".to_vec()
                    },
                })
            }
        }
        for (doctype, expected) in [("<!doctype html>", "error"), ("", "load")] {
            for suffix in ["empty", "nonempty"] {
                let mut page = RustExecutableDocument::load_with_environment(
                    &format!("{doctype}<body></body>"),
                    &BTreeMap::new(),
                    Some(SharedNetworkBroker::new(Sheet)),
                    Some(Url::parse("https://example.test/").unwrap()),
                )
                .unwrap();
                page.agent.host.execute(&format!("globalThis.mimeEvents=[];const link=document.createElement('link');link.rel='stylesheet';link.href='/{suffix}';link.onload=()=>mimeEvents.push('load');link.onerror=()=>mimeEvents.push('error');document.head.append(link)")).unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    page.checkpoint().unwrap();
                    let events = page
                        .agent
                        .host
                        .evaluate_string("mimeEvents.join(',')")
                        .unwrap();
                    if !events.is_empty() {
                        assert_eq!(events, expected, "{doctype} / {suffix}");
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "stylesheet event not published"
                    );
                    std::thread::yield_now();
                }
                page.checkpoint().unwrap();
                assert_eq!(
                    page.agent
                        .host
                        .evaluate_string("mimeEvents.join(',')")
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn standalone_css_cancel_and_meta_policy_reject_before_broker_admission() {
        struct Forbidden;
        impl NetworkBroker for Forbidden {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("denied CSS contacted broker")
            }
        }
        let mut page = RustExecutableDocument::load_with_environment(
            "<link id=owner type=text/plain>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Forbidden)),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let node = page.node_by_id("owner").unwrap();
        let fetch = ParserFetchContext::capture(&page.agent.host, &page.agent.dom, node).unwrap();
        let (loader, cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            page.dynamic_scripts.source_bytes.clone(),
        )
        .unwrap();
        drop(cancel);
        assert!(matches!(
            loader.load(true, true, &fetch.base_url, "blocked.css", "", "UTF-8"),
            Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Cancelled
            ))
        ));
        page.agent
            .host
            .install_standalone_meta_policy("style-src 'none'")
            .unwrap();
        let (loader, _cancel) = Loader::capture(
            &mut page.agent.host,
            &page.resources,
            &fetch,
            page.dynamic_scripts.source_bytes.clone(),
        )
        .unwrap();
        assert!(
            loader
                .load(true, true, &fetch.base_url, "blocked.css", "", "UTF-8")
                .is_err()
        );
    }
    #[test]
    fn native_stylesheet_applicability_gates_standalone_parser_without_blocking_tasks() {
        struct Held {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl NetworkBroker for Held {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/css".into())],
                    body: b"body{width:42px}".to_vec(),
                })
            }
        }
        for media in ["screen", "not all"] {
            let (started, observed) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let source = format!(
                "<link id=sheet rel=stylesheet href=held.css media='{media}'><script>globalThis.afterSheet=true</script>"
            );
            let mut page = retained_fixture_stream(
                DocumentStream::html(Box::new(std::iter::once(Ok(source)))),
                "https://example.test/",
                SharedNetworkBroker::new(Held {
                    started,
                    release: Mutex::new(blocked),
                }),
            )
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                page.poll_context_events().unwrap();
                if observed.try_recv().is_ok() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                    page.parser_pending(),
                    page.document_load_complete(),
                    page.frame_script_preparations.len(),
                    page.diagnostics
                );
                std::thread::yield_now();
            }
            for _ in 0..8 {
                page.checkpoint().unwrap();
            }
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("typeof afterSheet")
                    .unwrap(),
                if media == "screen" {
                    "undefined"
                } else {
                    "boolean"
                }
            );
            page.agent
                .host
                .execute("globalThis.unrelatedStyleTask=true")
                .unwrap();
            release.send(()).unwrap();
            while page.parser_pending() || !page.document_load_complete() {
                page.poll_context_events().unwrap();
                assert!(
                    std::time::Instant::now() < deadline,
                    "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                    page.parser_pending(),
                    page.document_load_complete(),
                    page.frame_script_preparations.len(),
                    page.diagnostics
                );
                std::thread::yield_now();
            }
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("afterSheet&&unrelatedStyleTask")
                    .unwrap(),
                "true"
            );
            let node = page.node_by_id("sheet").unwrap();
            assert!(
                page.agent
                    .dom
                    .borrow()
                    .arena()
                    .stylesheet_text(node)
                    .unwrap()
                    .contains("42px")
            );
        }
    }
}
impl Loader {
    pub(super) fn capture(
        host: &mut RawV8Host,
        resources: &BTreeMap<String, String>,
        fetch: &ParserFetchContext,
        budget: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(Self, Cancel), HostError> {
        let backend = host.module_fetch_backend()?;
        if !matches!(
            &backend,
            module_graph_load::ModuleGraphBackend::Broker { .. }
                | module_graph_load::ModuleGraphBackend::Sources(..)
        ) {
            return Err(HostError::Execution(
                "standalone stylesheet backend mismatch".into(),
            ));
        }
        let policy = host.standalone_resource_policy()?;
        let _ = resources; // Source bytes and policy are captured from this native realm.
        let next = if matches!(
            &backend,
            module_graph_load::ModuleGraphBackend::Broker { .. }
        ) {
            host.reserve_resource_request_ids(256 * 11)?
        } else {
            0
        };
        let cancel = Arc::new(AtomicBool::new(false));
        Ok((
            Self {
                preload_trace: Vec::new(),
                retained_fetch: false,
                last_origin_clean: false,
                source_origin: host.realm_descriptor(fetch.realm).map(|realm|realm.origin.clone()),
                preload_status: None,
                render_blocking: fetch.render_blocking,
                quirks: fetch.quirks,
                backend,
                policy,
                cancel: cancel.clone(),
                budget,
                retained: Vec::new(),
                dependencies: Vec::new(),
                binary: None,
                next,
                end: next + 256 * 11,
                count: 0,
                deadline: std::time::Instant::now() + Duration::from_secs(30),
                initiator: fetch.initiator_url.clone(),
                referrer_policy: fetch.referrer_policy.clone(),
                integrity: fetch.integrity.clone(),
                cross_origin: fetch.cross_origin.clone(),
                nonce: fetch.nonce.clone(),
            },
            Cancel::new(cancel),
        ))
    }
    fn check(&self) -> Result<(), HostError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Cancelled,
            ));
        }
        if std::time::Instant::now() >= self.deadline {
            return Err(HostError::Execution(
                "stylesheet resource deadline exceeded".into(),
            ));
        }
        Ok(())
    }
    pub(super) fn load_dependency(
        mut self,
        base: &str,
        referrer: &str,
        policy: &str,
        reference: oryn_layout::cssom::CssResourceReference,
    ) -> Result<super::dynamic_stylesheet_preparation::CssBinary, HostError> {
        self.load_dependency_next(base,referrer,policy,reference)
    }
    pub(super) fn load_dependency_next(
        &mut self,
        base: &str,
        referrer: &str,
        policy: &str,
        reference: oryn_layout::cssom::CssResourceReference,
    ) -> Result<super::dynamic_stylesheet_preparation::CssBinary, HostError> {
        self.check()?;
        let url = self.backend.resolve_request(base, &reference.url)?;
        let destination = match reference.destination {
            oryn_layout::cssom::CssResourceDestination::Font => "font",
            oryn_layout::cssom::CssResourceDestination::Image => "image",
        };
        self.resource(&url, referrer, policy, "UTF-8", destination, false)?;
        self.binary
            .take()
            .ok_or_else(|| HostError::Execution("CSS binary response absent".into()))
    }
    fn retain(&mut self, bytes: usize) -> Result<(), HostError> {
        self.retained.push(
            ScriptSourceReservation::reserve(&self.budget, bytes).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "stylesheet resource graph bytes".into(),
                }
            })?,
        );
        Ok(())
    }
    pub(super) fn load(
        mut self,
        linked: bool,
        install: bool,
        base: &str,
        href: &str,
        css: &str,
        encoding: &str,
    ) -> Result<StylesheetSource, HostError> {
        self.retain(
            base.len()
                .saturating_add(href.len())
                .saturating_add(css.len())
                .saturating_add(self.initiator.len()),
        )?;
        let policy = self.referrer_policy.clone();
        let initiator = self.initiator.clone();
        let (url, css) = if linked {
            let requested = self.backend.resolve_request(base, href)?;
            self.sheet(
                &requested,
                &initiator,
                &policy,
                encoding,
                0,
                &mut Vec::new(),
            )
        } else if install {
            self.expand(StylesheetExpansionInput {
                url: base,
                referrer: &initiator,
                policy: &policy,
                css,
                encoding,
                depth: 0,
                ancestors: &mut Vec::new(),
            })
            .map(|css| (base.into(), css))
        } else {
            collect_css_dependencies(&mut self.dependencies, base, &initiator, &policy, css)?;
            Ok((base.into(), css.into()))
        }?;
        Ok((url, css, std::mem::take(&mut self.dependencies)))
    }
    fn sheet(
        &mut self,
        requested: &str,
        referrer: &str,
        policy: &str,
        encoding: &str,
        depth: usize,
        ancestors: &mut Vec<String>,
    ) -> Result<(String, String), HostError> {
        self.check()?;
        if depth > 8 {
            return Err(HostError::LimitExceeded {
                resource: "stylesheet import depth".into(),
            });
        }
        with_stylesheet_ancestor(ancestors, requested, |ancestors| {
            let (url, css, encoding, policy) = self.resource(
                requested,
                referrer,
                policy,
                encoding,
                "stylesheet",
                depth == 0,
            )?;
            let expanded = if stylesheet_cycle_key(&url) == stylesheet_cycle_key(requested) {
                self.expand(StylesheetExpansionInput {
                    url: &url,
                    referrer: &url,
                    policy: &policy,
                    css: &css,
                    encoding: &encoding,
                    depth,
                    ancestors,
                })?
            } else {
                with_stylesheet_ancestor(ancestors, &url, |ancestors| {
                    self.expand(StylesheetExpansionInput {
                        url: &url,
                        referrer: &url,
                        policy: &policy,
                        css: &css,
                        encoding: &encoding,
                        depth,
                        ancestors,
                    })
                })?
                .unwrap_or_default()
            };
            Ok((url, expanded))
        })
        .map(|value| value.unwrap_or_else(|| (requested.into(), String::new())))
    }
    fn expand(&mut self, params: StylesheetExpansionInput<'_>) -> Result<String, HostError> {
        let StylesheetExpansionInput {
            url,
            referrer,
            policy,
            css,
            encoding,
            depth,
            ancestors,
        } = params;

        let rules = oryn_layout::cssom::read_stylesheet(css, url, false, true)
            .map_err(HostError::Execution)?;
        let mut output = String::new();
        for rule in rules {
            self.check()?;
            let text = if rule.rule_type == 3 {
                if rule
                    .properties
                    .get("supportsMatches")
                    .is_some_and(|value| value == "false")
                {
                    continue;
                }
                let Some(href) = rule.properties.get("href") else {
                    continue;
                };
                let href = self.backend.resolve_request(url, href)?;
                let dependency_start = self.dependencies.len();
                let (_, imported) =
                    self.sheet(&href, referrer, policy, encoding, depth + 1, ancestors)?;
                let mut growth = 0usize;
                for (_, _, _, css) in &mut self.dependencies[dependency_start..] {
                    let wrapped = wrap_imported_stylesheet(&rule, css);
                    growth = growth.saturating_add(wrapped.len().saturating_sub(css.len()));
                    *css = wrapped;
                }
                self.retain(growth)?;
                wrap_imported_stylesheet(&rule, &imported)
            } else {
                let previous = self.dependencies.len();
                collect_css_dependencies(
                    &mut self.dependencies,
                    url,
                    referrer,
                    policy,
                    &rule.css_text,
                )?;
                if self.dependencies.len() != previous {
                    self.retain(
                        url.len()
                            .saturating_add(referrer.len())
                            .saturating_add(policy.len())
                            .saturating_add(rule.css_text.len()),
                    )?;
                }
                rule.css_text
            };
            self.retain(text.len().saturating_add(1))?;
            output.push_str(&text);
            output.push('\n');
        }
        Ok(output)
    }
    fn resource(
        &mut self,
        requested: &str,
        referrer: &str,
        referrer_policy: &str,
        encoding: &str,
        destination: &str,
        root: bool,
    ) -> Result<(String, String, String, String), HostError> {
        // Preload receipts retain actual checked response classification;
        // retained consumers never infer clean status from a base URL.
        self.last_origin_clean = false;
        if root && destination == "stylesheet" && self.preload_status.is_none() {
            let key = crate::local_resource_preload::Key {
                destination: "stylesheet",
                url: requested.into(),
                policy: self
                    .policy
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| HostError::Execution(error.to_string()))?,
                initiator: self.initiator.clone(),
                referrer_policy: referrer_policy.into(),
                encoding: encoding.into(),
                nonce: self.nonce.clone(),
                integrity: self.integrity.clone(),
                cross_origin: self.cross_origin.clone(),
                parser_inserted: true,
                quirks: self.quirks,
            };
            let cached = self
                .backend
                .reports()
                .and_then(|reports| reports.3.claim(key.clone(), self.render_blocking));
            if let Some(cached) = cached {
                let response = cached.wait(&self.cancel);
                let reports = self.backend.reports().expect("native preload owner");
                if self.cancel.load(Ordering::Acquire) {
                    reports.3.finish_claim(&cached, false);
                    return Err(HostError::ScriptLoad(
                        crate::script_load_failure::ScriptLoadFailure::Cancelled,
                    ));
                }
                let authorized = if response.is_ok() {
                    self.backend.authorize_preload_reuse(&key, &cached)
                } else {
                    Ok(cached.same_request(&key))
                };
                match authorized {
                    Ok(false) => reports.3.finish_claim(&cached, false),
                    Ok(true) => {
                        reports.3.finish_claim(&cached, true);
                        let result = match response? {
                            crate::local_resource_preload::Response::Style(
                                url,
                                source,
                                encoding,
                                policy,
                                origin_clean,
                            ) => {
                                self.last_origin_clean = origin_clean;
                                (url, source, encoding, policy)
                            },
                            _ => {
                                return Err(HostError::Execution(
                                    "stylesheet preload response kind".into(),
                                ));
                            }
                        };
                        self.retain(result.0.len().saturating_add(result.1.len()))?;
                        return Ok(result);
                    }
                    Err(error) => {
                        reports.3.finish_claim(&cached, false);
                        return Err(error);
                    }
                }
            }
        }
        let mut _timing = match &self.backend {
            module_graph_load::ModuleGraphBackend::Broker { reports, .. } => {
                Some(reports.time_resource(
                    requested,
                    if destination == "stylesheet" && root {
                        "link"
                    } else {
                        "css"
                    },
                ))
            }
            _ => None,
        };
        if let (Some(timing), Some(status)) = (&mut _timing, &self.preload_status) {
            timing.set_shared_status(status.clone());
        }
        if let Some(timing) = &mut _timing {
            timing.set_render_blocking(self.render_blocking && destination == "stylesheet");
        }
        self.check()?;
        self.count += 1;
        if self.count > 256 {
            return Err(HostError::LimitExceeded {
                resource: "stylesheet resource graph requests".into(),
            });
        }
        let (broker, top, isolation) = match &self.backend {
            // Local response bytes use the same CSP, MIME, integrity, encoding
            // and cancellation path as a broker response. They do not acquire
            // a network endpoint, including for nested imports/dependencies.
            module_graph_load::ModuleGraphBackend::Sources(..)
                if Url::parse(requested).is_ok_and(|url| url.scheme() == "data") =>
            {
                (
                    None,
                    Url::parse(&self.initiator)
                        .map_err(|error| HostError::Execution(error.to_string()))?,
                    crate::agent_isolation::DocumentIsolationPolicy::default(),
                )
            }
            module_graph_load::ModuleGraphBackend::Sources(sources, _, _) => {
                if let Some(policy) = &self.policy {
                    let kind = match destination {
                        "font" => crate::csp::CspDestination::Font,
                        "image" => crate::csp::CspDestination::Image,
                        _ => crate::csp::CspDestination::Style,
                    };
                    let decision = policy.authorize(&crate::csp::CspRequest {
                        destination: kind,
                        url: requested.into(),
                        nonce: if root { self.nonce.clone() } else { None },
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    });
                    self.backend
                        .report_violations(policy, decision.violations)?;
                    if !decision.allowed {
                        return Err(self.web_failure(
                            "stylesheet resource blocked by Content-Security-Policy".into(),
                        ));
                    }
                }
                if root
                    && self
                        .integrity
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())
                {
                    return Err(self.web_failure(
                        "decoded stylesheet source cannot verify integrity".into(),
                    ));
                }
                let (url, source) = resource_entry(sources, requested).ok_or_else(|| {
                    self.web_failure("standalone stylesheet resource unavailable".into())
                })?;
                if destination != "stylesheet" {
                    self.binary = Some(super::dynamic_stylesheet_preparation::CssBinary {
                        url: url.to_owned(),
                        bytes: source.as_bytes().to_vec(),
                    });
                }
                if self.preload_status.is_some() {
                    self.preload_trace.push(requested.into());
                }
                let result = (
                    url.to_owned(),
                    if destination == "stylesheet" {
                        source.to_owned()
                    } else {
                        String::new()
                    },
                    encoding.into(),
                    referrer_policy.into(),
                );
                // Pinned decoded bytes have no CORS/redirect evidence. They
                // remain opaque across origins, but a same-origin original URL
                // is readable under the exact captured native document origin.
                self.last_origin_clean = self.source_origin.as_ref().is_some_and(|origin|
                    !origin.is_opaque() && Url::parse(url).is_ok_and(|url|
                        !matches!(url.origin(),url::Origin::Opaque(_))
                            && oryn_js::origin::Origin::from_url(url.as_str())==*origin));
                self.retain(result.0.len().saturating_add(result.1.len()))?;
                return Ok(result);
            }
            module_graph_load::ModuleGraphBackend::Broker {
                broker,
                base,
                isolation,
                ..
            } => (Some(broker.clone()), base.clone(), *isolation),
            _ => {
                return Err(HostError::Execution(
                    "standalone stylesheet backend mismatch".into(),
                ));
            }
        };
        let timing_started = std::time::Instant::now();
        let mut timing_hops = Vec::new();
        let initiator =
            Url::parse(&self.initiator).map_err(|error| HostError::Execution(error.to_string()))?;
        let origin = initiator.origin();
        let mut request_origin = origin.ascii_serialization();
        let mut opaque = false;
        let cors = destination == "font" || (root && self.cross_origin.is_some());
        let include = root
            && self
                .cross_origin
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"));
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            if cors { "cors" } else { "no-cors" },
            if destination == "stylesheet" {
                "style"
            } else {
                destination
            },
            matches!(origin, url::Origin::Opaque(_)),
        );
        let mut url =
            Url::parse(requested).map_err(|error| HostError::Execution(error.to_string()))?;
        let mut referrer = Url::parse(referrer).ok();
        let mut policy = crate::referrer::ReferrerPolicy::parse_list(referrer_policy);
        let mut eligible = true;
        for redirect in 0..=10 {
            self.check()?;
            if let Some(csp) = &self.policy {
                if let Some(upgraded) = csp.upgraded_url(&url) {
                    url = upgraded;
                }
                let destination = match destination {
                    "font" => crate::csp::CspDestination::Font,
                    "image" => crate::csp::CspDestination::Image,
                    _ => crate::csp::CspDestination::Style,
                };
                let decision = csp.authorize_redirect_from(
                    &crate::csp::CspRequest {
                        destination,
                        url: url.to_string(),
                        nonce: if root { self.nonce.clone() } else { None },
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    },
                    redirect != 0,
                    requested,
                );
                self.backend.report_violations(csp, decision.violations)?;
                if !decision.allowed {
                    return Err(self.web_failure(
                        "stylesheet resource blocked by Content-Security-Policy".into(),
                    ));
                }
            }
            if self.preload_status.is_some() {
                self.preload_trace.push(url.to_string());
            }
            fetch_metadata.observe(Some(&self.initiator), &url);
            crate::mixed_content::enforce(&self.initiator, &url)
                .map_err(|error| self.web_failure(error.to_string()))?;
            if url.scheme() != "data" && url.origin() != origin && !cors {
                eligible = false;
            }
            let (headers, body) = if url.scheme() == "data" {
                let (mime, body) = crate::network::decode_data_url(&url)
                    .map_err(|error| self.web_failure(error.to_string()))?;
                if let Some(timing) = &mut _timing {
                    timing.set_details(crate::network::local_resource_timing_metadata(
                        &origin.ascii_serialization(),
                        &body,
                        mime.as_deref(),
                        timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                    ));
                }
                (
                    mime.map(|mime| vec![("content-type".into(), mime)])
                        .unwrap_or_default(),
                    body,
                )
            } else {
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(self.web_failure(
                        "standalone CSS resource scheme unavailable".into(),
                    ));
                }
                let broker = broker.as_ref().ok_or_else(|| {
                    HostError::Execution(
                        "standalone stylesheet network endpoint unavailable".into(),
                    )
                })?;
                if self.next >= self.end {
                    return Err(HostError::LimitExceeded {
                        resource: "stylesheet request identities".into(),
                    });
                }
                let request_id = self.next;
                self.next += 1;
                let cross = cors && (opaque || url.origin() != origin);
                let omit = isolation.omit_credentials(cors, opaque || url.origin() != origin);
                let mut headers = vec![(
                    "accept".into(),
                    if destination == "stylesheet" {
                        "text/css,*/*;q=0.1"
                    } else {
                        "*/*"
                    }
                    .into(),
                )];
                if cross {
                    headers.push(("origin".into(), request_origin.clone()));
                }
                let sent_referrer = referrer
                    .as_ref()
                    .and_then(|source| policy.header(source, &url));
                referrer = sent_referrer
                    .as_ref()
                    .and_then(|value| Url::parse(value).ok());
                if let Some(value) = sent_referrer {
                    headers.push(("referer".into(), value));
                }
                let response = collect_operation_bounded_with_cancel(
                    broker,
                    NetworkRequest {
                        request_id,
                        method: "GET".into(),
                        url: url.to_string(),
                        headers,
                        body: Vec::new(),
                        credentials: (include || !cross) && !omit,
                        security: CookieRequestContext {
                            fetch_metadata: fetch_metadata.clone(),
                            dns_timing: None,
                            credentials: if omit {
                                CookieCredentials::Omit
                            } else if cors && !include {
                                CookieCredentials::SameOrigin
                            } else {
                                CookieCredentials::Include
                            },
                            kind: match destination {
                                "font" => CookieRequestKind::Font,
                                "image" => CookieRequestKind::Image,
                                _ => CookieRequestKind::Style,
                            },
                            initiator_url: Some(self.initiator.clone()),
                            top_level_url: Some(top.to_string()),
                            ..Default::default()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    self.deadline
                        .saturating_duration_since(std::time::Instant::now()),
                    16 * 1024 * 1024,
                    &self.cancel,
                )
                .map_err(|error| {
                    HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::network(
                        error,
                    ))
                })?;
                timing_hops.push(crate::network::timing_hop(&url, &response.headers));
                if cross {
                    let values = response
                        .headers
                        .iter()
                        .filter(|(name, _)| {
                            name.eq_ignore_ascii_case("access-control-allow-origin")
                        })
                        .map(|(_, value)| value.trim())
                        .collect::<Vec<_>>();
                    let credentials = response
                        .headers
                        .iter()
                        .filter(|(name, _)| {
                            name.eq_ignore_ascii_case("access-control-allow-credentials")
                        })
                        .map(|(_, value)| value.trim())
                        .collect::<Vec<_>>();
                    if values.len() != 1
                        || !((!include && values[0] == "*") || values[0] == request_origin)
                        || (include && credentials != vec!["true"])
                    {
                        return Err(self.web_failure(
                            "stylesheet resource failed CORS".into(),
                        ));
                    }
                }
                if let module_graph_load::ModuleGraphBackend::Broker { coep, reports, .. } =
                    &self.backend
                {
                    reports.push_coep(coep.response(crate::coep_reporting::ResponsePolicyInputs {
                        origin: &origin.ascii_serialization(),
                        opaque: matches!(origin, url::Origin::Opaque(_)),
                        target: &url,
                        cors,
                        credentials: (include || !cross) && !omit,
                        headers: &response.headers,
                        destination: if destination == "stylesheet" {
                            "style"
                        } else {
                            destination
                        },
                    }));
                }
                isolation
                    .authorize_response(
                        &origin.ascii_serialization(),
                        false,
                        &url,
                        cors,
                        (include || !cross) && !omit,
                        &response.headers,
                    )
                    .map_err(|message| self.web_failure(message))?;
                if oryn_net::fetch::is_redirect_status(response.status) {
                    if redirect == 10 {
                        return Err(self.web_failure(
                            "stylesheet redirect limit exceeded".into(),
                        ));
                    }
                    let next = response
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                        .and_then(|(_, location)| url.join(location).ok())
                        .ok_or_else(|| {
                            self.web_failure("invalid stylesheet redirect".into())
                        })?;
                    if cors && (!next.username().is_empty() || next.password().is_some()) {
                        return Err(self.web_failure(
                            "CORS stylesheet redirect contains credentials".into(),
                        ));
                    }
                    if cors && url.origin() != next.origin() && url.origin() != origin {
                        opaque = true;
                        request_origin = "null".into();
                    }
                    policy = policy.after_response(&response.headers);
                    url = next;
                    continue;
                }
                if !(200..300).contains(&response.status) {
                    if destination == "font" {
                        return Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(
                            format!("font response returned HTTP {}", response.status),
                        )));
                    }
                    return Err(self.web_failure(format!(
                        "CSS resource returned HTTP {}",
                        response.status
                    )));
                }
                if let Some(timing) = &mut _timing {
                    timing.set_details(crate::network::resource_timing_metadata(
                        &origin.ascii_serialization(),
                        matches!(origin, url::Origin::Opaque(_)),
                        timing_hops.clone(),
                        &response,
                        timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                    ));
                }
                (response.headers, response.body)
            };
            self.check()?;
            if body.len() > 16 * 1024 * 1024 {
                return Err(HostError::LimitExceeded {
                    resource: "stylesheet resource bytes".into(),
                });
            }
            self.retain(body.len())?;
            if destination == "stylesheet"
                && !oryn_net::stylesheet_response_allowed(&headers, self.quirks, cors || eligible)
            {
                return Err(self.web_failure(
                    "stylesheet response blocked by MIME type or nosniff".into(),
                ));
            }
            if root {
                crate::integrity::verify(
                    self.integrity.as_deref().unwrap_or(""),
                    &body,
                    cors || eligible,
                )
                .map_err(|error| {
                    if self.retained_fetch {
                        HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Integrity(format!("{error:?}")))
                    } else {
                        HostError::Execution(format!("stylesheet integrity failed: {error:?}"))
                    }
                })?;
            }
            let extracted_type = oryn_common::mime_type::MimeType::from_headers(&headers)
                .map(|mime| mime.serialize());
            let content_type = extracted_type.as_deref();
            if destination != "stylesheet" {
                self.binary = Some(super::dynamic_stylesheet_preparation::CssBinary {
                    url: url.to_string(),
                    bytes: body.clone(),
                });
            }
            let (css, encoding) = if destination == "stylesheet" {
                let (css, encoding) = oryn_net::decode_resource_text_with_fallback(
                    &body,
                    content_type,
                    oryn_net::TextResourceKind::Css,
                    Some(encoding),
                );
                (css, encoding.to_owned())
            } else {
                (String::new(), encoding.to_owned())
            };
            self.retain(css.len())?;
            self.last_origin_clean = cors || eligible;
            return Ok((
                url.to_string(),
                css,
                encoding,
                policy.after_response(&headers).as_str().into(),
            ));
        }
        Err(HostError::Execution(
            "stylesheet resource did not complete".into(),
        ))
    }
}

// A preload fetches and validates only the root stylesheet response. Parsing
// imports, loading dependencies and installing CSS remain consuming-link work.
pub(crate) fn preload(
    backend: module_graph_load::ModuleGraphBackend,
    key: crate::local_resource_preload::Key,
    next: u64,
    entry: &crate::local_resource_preload::Entry,
) -> Result<crate::local_resource_preload::Response, HostError> {
    let policy = backend.local_script_policy().cloned();
    let mut loader = Loader {
        preload_trace: Vec::new(),
        retained_fetch: false,
        last_origin_clean: false,
        source_origin: None,
        preload_status: Some(entry.status.clone()),
        render_blocking: false,
        quirks: key.quirks,
        backend,
        policy,
        cancel: entry.cancel.clone(),
        budget: Default::default(),
        retained: Vec::new(),
        dependencies: Vec::new(),
        binary: None,
        next,
        end: next.saturating_add(11),
        count: 0,
        deadline: std::time::Instant::now() + Duration::from_secs(30),
        initiator: key.initiator.clone(),
        referrer_policy: key.referrer_policy.clone(),
        integrity: key.integrity,
        cross_origin: key.cross_origin,
        nonce: key.nonce,
    };
    let (url, source, encoding, policy) = loader.resource(
        &key.url,
        &key.initiator,
        &key.referrer_policy,
        &key.encoding,
        "stylesheet",
        true,
    )?;
    entry.set_trace(std::mem::take(&mut loader.preload_trace));
    Ok(crate::local_resource_preload::Response::Style(
        url, source, encoding, policy, loader.last_origin_clean,
    ))
}

#[cfg(test)]
mod preload_handoff_tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use std::sync::Mutex;
    struct Counted(Arc<Mutex<Vec<String>>>);
    impl NetworkBroker for Counted {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.0.lock().unwrap().push(request.url.clone());
            let (css, body) = if request.url.ends_with("root.css") {
                (true, "@import url(child.css);body{color:rgb(1,2,3)}")
            } else if request.url.ends_with("child.css") {
                (true, "body{margin-left:17px}")
            } else {
                (
                    false,
                    "globalThis.preloadedExecutions=(globalThis.preloadedExecutions||0)+1;",
                )
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![(
                    "content-type".into(),
                    if css { "text/css" } else { "text/javascript" }.into(),
                )],
                body: body.as_bytes().to_vec(),
            })
        }
    }
    #[test]
    fn parser_classic_and_style_preload_consumers_use_original_response_once() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let source = "<!doctype html><head><link rel=preload as=script href=app.js><link rel=preload as=style href=root.css><link rel=stylesheet href=root.css><script src=app.js></script></head><body>ready";
        let mut page = RustExecutableDocument::load_stream_with_limits(
            DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Counted(calls.clone()))),
            Some(Url::parse("https://example.test/").unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while page.parser_pending() {
            page.poll_context_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                page.parser_pending(),
                page.document_load_complete(),
                page.frame_script_preparations.len(),
                page.diagnostics
            );
            std::thread::yield_now();
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("preloadedExecutions")
                .unwrap(),
            "1"
        );
        let calls = calls.lock().unwrap();
        for name in ["app.js", "root.css", "child.css"] {
            assert_eq!(
                calls.iter().filter(|url| url.ends_with(name)).count(),
                1,
                "{name}: {calls:?}"
            );
        }
    }
}

#[cfg(test)]
mod dynamic_preload_handoff_tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use std::sync::Mutex;
    struct Counted(Arc<Mutex<usize>>);
    impl NetworkBroker for Counted {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            *self.0.lock().unwrap() += 1;
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/javascript".into())],
                body: b"globalThis.dynamicPreloaded=(globalThis.dynamicPreloaded||0)+1".to_vec(),
            })
        }
    }
    #[test]
    fn dynamic_classic_default_uses_retained_preload_after_link_removal() {
        for parser_preload in [false, true] {
            let calls = Arc::new(Mutex::new(0));
            let mut page=RustExecutableDocument::load_stream_with_limits(DocumentStream::html(Box::new(std::iter::once(Ok(if parser_preload{"<!doctype html><head><link rel=preload as=script href=/app.js></head><body>ready".into()}else{"<!doctype html><head></head><body>ready".into()})))),&BTreeMap::new(),Some(SharedNetworkBroker::new(Counted(calls.clone()))),Some(Url::parse("https://example.test/").unwrap()),256*1024*1024,true).unwrap();
            while page.parser_pending() {
                page.poll_context_events().unwrap();
            }
            page.agent.host.execute("let preload=document.querySelector('link[rel=preload]');if(!preload){preload=document.createElement('link');preload.rel='preload';preload.as='script';preload.href='/app.js';document.head.append(preload)}preload.remove();let consumer=document.createElement('script');consumer.src='/app.js';document.head.append(consumer)").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                page.poll_context_events().unwrap();
                if page
                    .agent
                    .host
                    .evaluate_string("typeof dynamicPreloaded==='number'")
                    .unwrap()
                    == "true"
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "stylesheet wait: parser_pending={}, load_complete={}, parser_owners={}, diagnostics={:?}",
                    page.parser_pending(),
                    page.document_load_complete(),
                    page.frame_script_preparations.len(),
                    page.diagnostics
                );
                std::thread::yield_now();
            }
            assert_eq!(
                page.agent.host.evaluate_string("dynamicPreloaded").unwrap(),
                "1"
            );
            assert_eq!(*calls.lock().unwrap(), 1);
        }
    }
}
