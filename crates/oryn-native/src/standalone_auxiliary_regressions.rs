// Production standalone Window.open source matrix. No parent BrowserContext,
// synthetic page endpoint, or legacy Fetch shim participates in these cases.
use super::*;
use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};
use std::sync::{Arc, Mutex};
struct Documents(Arc<Mutex<Vec<String>>>);
impl NetworkBroker for Documents {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        self.0.lock().unwrap().push(request.url.clone());
        let path = Url::parse(&request.url).unwrap().path().to_owned();
        let body = match path.as_str() {
            "/one" => {
                "<!doctype html><title>one</title><p id=value>one</p><script>globalThis.saved={value:73};opener.results.push('one');</script>"
            }
            "/two" => {
                "<!doctype html><title>two</title><p id=value>two</p><script>opener.results.push('two');</script>"
            }
            "/foreign" => "<!doctype html><title>foreign</title><p>foreign</p>",
            _ => "<!doctype html><title>empty</title>",
        };
        Ok(NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: request.request_id,
            status: 200,
            headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
            body: body.as_bytes().to_vec(),
        })
    }
}
fn page() -> (RustExecutableDocument, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let page = RustExecutableDocument::load_with_environment(
        "<!doctype html><title>opener</title><body></body>",
        &BTreeMap::new(),
        Some(SharedNetworkBroker::new(Documents(seen.clone()))),
        Some(Url::parse("https://example.test/base/").unwrap()),
    )
    .unwrap();
    (page, seen)
}
fn until(page: &mut RustExecutableDocument, expression: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        page.drain_tasks().unwrap();
        if page.evaluate(expression).unwrap() == "true" {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{expression}");
        std::thread::yield_now();
    }
}
#[test]
fn blank_popup_preserves_synchronous_dom_globals_named_identity_and_opener() {
    let (mut page, _) = page();
    assert_eq!(page.evaluate(r#"(()=>{globalThis.popup=open('','report');popup.document.body.innerHTML='<p id=written>already written</p>';popup.marker={value:73};return String(popup.opener===window&&popup.document.baseURI===document.baseURI&&open('','report')===popup)})()"#).unwrap(),"true");
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.marker.value===73&&popup.document.getElementById('written').textContent==='already written'&&popup!==window)").unwrap(),"true");
}
#[test]
fn noopener_creates_independent_root_even_when_return_is_null() {
    let (mut page, _) = page();
    assert_eq!(
        page.evaluate("String(open('about:blank','private','noopener')===null)")
            .unwrap(),
        "true"
    );
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        1
    );
    assert_eq!(
        page.evaluate("String(open('','private')!==null)").unwrap(),
        "true"
    );
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        2
    );
}
#[test]
fn native_http_popup_navigation_commits_inside_same_agent_then_restores_history() {
    let (mut page, seen) = page();
    let identity = page.agent_host_identity();
    page.execute(
        "globalThis.results=[];globalThis.popup=open('/one','report');globalThis.proxy=popup",
    )
    .unwrap();
    until(&mut page, "String(results.join()==='one')");
    assert!(Rc::ptr_eq(&identity, &page.agent_host_identity()));
    assert_eq!(
        page.evaluate("String(popup===proxy&&popup.document.title==='one'&&popup.opener===window)")
            .unwrap(),
        "true"
    );
    page.execute("popup.location='/two'").unwrap();
    until(&mut page, "String(results.join()==='one,two')");
    page.execute("popup.history.back()").unwrap();
    until(&mut page, "String(popup.document.title==='one')");
    assert_eq!(
        page.evaluate("String(popup===proxy&&popup.saved.value===73)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|url| url.ends_with("/one"))
            .count(),
        1
    );
}
#[test]
fn cross_origin_replacement_keeps_proxy_and_denies_dom_access() {
    let (mut page, seen) = page();
    page.execute("globalThis.popup=open('https://foreign.test/foreign');globalThis.proxy=popup")
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        page.drain_tasks().unwrap();
        let denied=page.evaluate("(()=>{try{void popup.document;return 'false'}catch(e){return String(e.name==='SecurityError')}})()").unwrap();
        if denied == "true" {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        page.evaluate("String(popup===proxy&&!popup.closed)")
            .unwrap(),
        "true"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|url| url == "https://foreign.test/foreign")
    );
}
#[test]
fn auxiliary_close_accepts_multiple_history_entries_and_retires_only_its_root() {
    let (mut page, _) = page();
    page.execute("globalThis.popup=open();popup.history.pushState({},'', 'about:blank#one');popup.history.pushState({},'', 'about:blank#two');globalThis.popupHistoryLength=popup.history.length;popup.close();globalThis.openerAlive=0;setTimeout(()=>openerAlive++,0)").unwrap();
    assert_eq!(page.evaluate("String(popupHistoryLength)").unwrap(), "3");
    until(&mut page, "String(popup.closed&&openerAlive===1)");
    assert_eq!(
        page.evaluate("String(!closed&&document.title==='opener')")
            .unwrap(),
        "true"
    );
}
#[test]
fn sandbox_without_popup_permission_does_not_allocate_native_root() {
    let (mut page, _) = page();
    assert!(!page.agent.host.has_unclaimed_standalone_roots());
    page.execute("globalThis.frame=document.createElement('iframe');frame.setAttribute('sandbox','allow-scripts allow-same-origin');document.body.append(frame)").unwrap();
    assert_eq!(
        page.evaluate("String(frame.contentWindow.open('about:blank')===null)")
            .unwrap(),
        "true"
    );
    assert!(!page.agent.host.has_unclaimed_standalone_roots());
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        0
    );
}
#[test]
fn initial_frame_popup_permission_uses_captured_container_policy() {
    let (mut page, _) = page();
    page.execute("globalThis.frame=document.createElement('iframe');frame.setAttribute('sandbox','allow-scripts allow-same-origin');document.body.append(frame);globalThis.child=frame.contentWindow;frame.removeAttribute('sandbox');globalThis.denied=child.open('about:blank')===null").unwrap();
    assert_eq!(page.evaluate("String(denied)").unwrap(), "true");
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        0
    );
    assert_eq!(
        page.evaluate("String(child.open('about:blank')===null)")
            .unwrap(),
        "true",
        "adopting the initial Document retains its captured sandbox"
    );
    page.execute("globalThis.allowed=document.createElement('iframe');allowed.setAttribute('sandbox','allow-scripts allow-same-origin allow-popups');document.body.append(allowed);globalThis.permitted=allowed.contentWindow.open('about:blank');globalThis.nestedDenied=permitted.open('about:blank')===null").unwrap();
    assert_eq!(
        page.evaluate("String(permitted!==null&&permitted.opener===allowed.contentWindow)")
            .unwrap(),
        "true"
    );
    // Inherited sandbox permits popups too; the new popup retains that policy.
    assert_eq!(page.evaluate("String(nestedDenied)").unwrap(), "false");
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        2
    );
}
#[test]
fn native_parked_popup_queue_cannot_run_while_sibling_remains_live() {
    let (mut page, _) = page();
    page.execute("globalThis.results=[];globalThis.popup=open('/one');")
        .unwrap();
    until(&mut page, "String(results.join()==='one')");
    page.execute(
        "popup.eval('setTimeout(()=>opener.results.push(\"parked\"),1000)');popup.location='/two'",
    )
    .unwrap();
    until(&mut page, "String(results.join()==='one,two')");
    assert_eq!(
        page.evaluate("String(!results.includes('parked')&&document.title==='opener')")
            .unwrap(),
        "true"
    );
}
#[test]
fn blob_popup_navigation_uses_native_blob_owner_and_survives_later_revocation() {
    let (mut page, seen) = page();
    page.execute("globalThis.results=[];globalThis.blobURL=URL.createObjectURL(new Blob(['<!doctype html><title>blob</title><script>opener.results.push(\"blob\")<\\/script>'],{type:'text/html'}));globalThis.popup=open(blobURL)").unwrap();
    until(&mut page, "String(results.join()==='blob')");
    page.execute("URL.revokeObjectURL(blobURL)").unwrap();
    assert_eq!(
        page.evaluate("String(popup.document.title==='blob'&&popup.opener===window)")
            .unwrap(),
        "true"
    );
    assert!(seen.lock().unwrap().is_empty());
}
#[test]
fn coop_response_severs_original_proxy_and_opener_without_stopping_document_driver() {
    struct Coop;
    impl NetworkBroker for Coop {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/html".into()),("cross-origin-opener-policy".into(),"same-origin".into())],body:b"<!doctype html><title>isolated</title><script>globalThis.hasOpener=opener!==null</script>".to_vec()})
        }
    }
    let mut page = RustExecutableDocument::load_with_environment(
        "<!doctype html><title>source</title>",
        &BTreeMap::new(),
        Some(SharedNetworkBroker::new(Coop)),
        Some(Url::parse("https://example.test/start").unwrap()),
    )
    .unwrap();
    page.execute("globalThis.popup=open('/isolated')").unwrap();
    until(&mut page, "String(popup.closed)");
    assert_eq!(
        page.evaluate("String(!closed&&document.title==='source')")
            .unwrap(),
        "true"
    );
    let original = page.top_realm;
    let roots = page.agent.host.standalone_root_realms();
    let active = roots
        .into_iter()
        .find(|realm| *realm != original && !page.agent.host.realm_is_parked(*realm))
        .unwrap();
    page.agent.host.activate_realm(active).unwrap();
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(opener===null&&!hasOpener&&document.title==='isolated')")
            .unwrap(),
        "true"
    );
    page.agent.host.activate_realm(original).unwrap();
}
#[test]
fn failed_initial_adoption_keeps_retry_wakeup_and_native_popup() {
    let (mut page, _) = page();
    page.agent
        .host
        .execute("globalThis.popup=open();popup.marker=73")
        .unwrap();
    crate::runtime::standalone_root_driver::FAIL_ADOPTION.with(|fault| fault.set(true));
    assert!(page.drain_tasks().is_err());
    assert!(page.has_agent_host());
    assert!(page.has_pending_page_work());
    assert!(page.next_timer_delay().is_some());
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.marker===73)").unwrap(), "true");
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        1
    );
}
#[test]
fn failed_turn_extraction_recovers_unique_native_host_before_returning_error() {
    let (mut page, _) = page();
    page.execute("globalThis.popup=open();popup.marker=73")
        .unwrap();
    let identity = page.agent_host_identity();
    crate::runtime::standalone_root_driver::FAIL_EXTRACTION.with(|fault| fault.set(true));
    assert!(page.drain_tasks().is_err());
    assert!(page.has_agent_host());
    assert!(Rc::ptr_eq(&identity, &page.agent_host_identity()));
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.marker===73)").unwrap(), "true");
}

#[test]
fn captured_popup_permission_survives_later_container_restriction() {
    let (mut page, _) = page();
    page.execute("globalThis.frame=document.createElement('iframe');frame.setAttribute('sandbox','allow-scripts allow-same-origin allow-popups');document.body.append(frame);globalThis.child=frame.contentWindow;frame.setAttribute('sandbox','allow-scripts allow-same-origin');globalThis.popup=child.open('about:blank')").unwrap();
    assert_eq!(
        page.evaluate("String(popup!==null&&popup.opener===child)")
            .unwrap(),
        "true"
    );
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        1
    );
}

#[test]
fn sandbox_popup_permission_survives_standalone_frame_adoption_and_srcdoc_commit() {
    for srcdoc in [false, true] {
        for popups in [false, true] {
            let (mut page, _) = page();
            let tokens = if popups {
                "allow-scripts allow-same-origin allow-popups"
            } else {
                "allow-scripts allow-same-origin"
            };
            page.execute(&format!("globalThis.frame=document.createElement('iframe');frame.setAttribute('sandbox','{tokens}');{}document.body.append(frame);globalThis.initialResult=frame.contentWindow.open('','initial');",
                if srcdoc { "frame.srcdoc='<p>child</p>';" } else { "" })).unwrap();
            assert_eq!(
                page.evaluate("String(initialResult!==null)").unwrap(),
                popups.to_string()
            );
            until(
                &mut page,
                if srcdoc {
                    "String(frame.contentWindow.location.href==='about:srcdoc')"
                } else {
                    "String(frame.contentWindow.document.readyState==='complete')"
                },
            );
            assert_eq!(
                page.evaluate("String(frame.contentWindow.open('','after')!==null)")
                    .unwrap(),
                popups.to_string()
            );
            page.drain_tasks().unwrap();
            assert_eq!(
                page.standalone_roots
                    .upgrade()
                    .unwrap()
                    .borrow()
                    .drivers_len(),
                if popups { 2 } else { 0 }
            );
        }
    }
}
#[test]
fn parser_created_standalone_frame_keeps_container_sandbox() {
    for source in ["", " srcdoc='<p>child</p>'"] {
        let mut page = RustExecutableDocument::load_with_environment(
            &format!(
                "<body><iframe id=child sandbox='allow-scripts allow-same-origin'{source}></iframe>"
            ),
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("String(child.contentWindow.open('about:blank')===null)")
                .unwrap(),
            "true"
        );
        page.drain_tasks().unwrap();
        assert_eq!(
            page.standalone_roots
                .upgrade()
                .unwrap()
                .borrow()
                .drivers_len(),
            0
        );
    }
}

#[test]
fn standalone_frame_container_permission_cannot_relax_response_sandbox() {
    struct SandboxedResponse;
    impl NetworkBroker for SandboxedResponse {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("Content-Type".into(), "text/html; charset=utf-8".into()),
                    (
                        "Content-Security-Policy".into(),
                        "sandbox allow-scripts allow-same-origin".into(),
                    ),
                ],
                body: b"<!doctype html><title>response</title><body>child</body>".to_vec(),
            })
        }
    }
    let mut page = RustExecutableDocument::load_with_environment(
        "<body></body>",
        &BTreeMap::new(),
        Some(SharedNetworkBroker::new(SandboxedResponse)),
        Some(Url::parse("https://example.test/").unwrap()),
    )
    .unwrap();
    page.execute("globalThis.frame=document.createElement('iframe');frame.setAttribute('sandbox','allow-scripts allow-same-origin allow-popups');frame.src='/child';document.body.append(frame)").unwrap();
    until(
        &mut page,
        "String(frame.contentWindow.location.href==='https://example.test/child'&&frame.contentDocument.readyState==='complete')",
    );
    assert_eq!(
        page.evaluate("String(frame.contentWindow.open('about:blank')===null)")
            .unwrap(),
        "true"
    );
    page.drain_tasks().unwrap();
    assert_eq!(
        page.standalone_roots
            .upgrade()
            .unwrap()
            .borrow()
            .drivers_len(),
        0
    );
}
