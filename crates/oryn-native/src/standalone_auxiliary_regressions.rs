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
fn close_diagnostic_scope(label: &'static str) -> crate::native_timing::CommandScope {
    // Test-only explicit inherited capability; no file is opened by this test.
    if let Ok(value) = std::env::var("ORYN_CLOSE_DIAGNOSTIC_FD") {
        let fd = value.parse::<i32>().expect("numeric inherited diagnostic fd");
        crate::native_timing::install_inherited(fd).expect("valid private diagnostic capability");
    }
    crate::native_timing::CommandScope::start(label, serde_json::Value::Null)
}
#[test]
fn blank_popup_preserves_synchronous_dom_globals_named_identity_and_opener() {
    let (mut page, _) = page();
    assert_eq!(page.evaluate(r#"(()=>{globalThis.popup=open('','report');popup.document.body.innerHTML='<p id=written>already written</p>';popup.marker={value:73};return String(popup.opener===window&&popup.document.baseURI===document.baseURI&&open('','report')===popup)})()"#).unwrap(),"true");
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.marker.value===73&&popup.document.getElementById('written').textContent==='already written'&&popup!==window)").unwrap(),"true");
}
#[test]
fn explicit_blank_popup_preserves_initial_document_but_existing_target_navigates() {
    let (mut page, _) = page();
    page.execute("globalThis.popup=open('about:blank','explicit');globalThis.initial=popup.document;popup.document.body.innerHTML='<p id=written>saved</p>';popup.marker=73").unwrap();
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.document===initial&&popup.document.getElementById('written').textContent==='saved'&&popup.marker===73)").unwrap(), "true");
    page.execute("globalThis.sameTarget=open('about:blank','explicit')").unwrap();
    until(&mut page, "String(popup.document!==initial)");
    assert_eq!(page.evaluate("String(sameTarget===popup&&popup.document.getElementById('written')===null&&popup.opener===window)").unwrap(), "true");
}
#[test]
fn adopted_auxiliary_driver_uses_its_exact_native_terminal_controller() {
    let (mut page, _) = page();
    page.execute("globalThis.popup=open('about:blank');globalThis.popupOwnerDocument=popup.document").unwrap();
    let opener = page.agent.host.retained_binding_state(page.top_realm).unwrap().prompt_controller();
    let popup_realm = page.agent.host.standalone_root_realms().into_iter()
        .find(|realm| *realm != page.top_realm).unwrap();
    let popup_controller = page.agent.host.retained_binding_state(popup_realm).unwrap().prompt_controller();
    assert!(!Arc::ptr_eq(&opener, &popup_controller), "auxiliary root cannot inherit opener terminal authority");
    page.drain_tasks().unwrap();
    let roots = page.standalone_roots.upgrade().unwrap();
    assert_eq!(roots.borrow().drivers_len(), 1);
    let current = page.agent.host.standalone_navigation_root(popup_realm).unwrap();
    let page_controller = page.agent.host.standalone_page_controller(current).unwrap();
    assert!(Arc::ptr_eq(&page_controller, &roots.borrow().driver_page_controller(popup_realm).unwrap()));
    assert!(!Arc::ptr_eq(&page_controller, &popup_controller));
    page.execute("popup.document.body.innerHTML='<p id=ready>ready</p>';popup.marker=17").unwrap();
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(popup.document===popupOwnerDocument&&popup.marker===17&&popup.document.getElementById('ready').textContent==='ready'&&!closed&&!popup.closed)").unwrap(), "true");
}
#[test]
fn auxiliary_page_controller_survives_successor_and_old_document_retirement() {
    let _diagnostic = close_diagnostic_scope("native_auxiliary_successor_close_control");
    let (mut page, _) = page();
    page.execute("globalThis.results=[];globalThis.popup=open('about:blank')").unwrap();
    page.drain_tasks().unwrap();
    let initial = page.agent.host.standalone_root_realms().into_iter()
        .find(|realm| *realm != page.top_realm).unwrap();
    let current_root = page.agent.host.standalone_navigation_root(initial).unwrap();
    let controller = page.agent.host.standalone_page_controller(current_root).unwrap();
    let drivers = page.standalone_roots.upgrade().unwrap();
    assert!(Arc::ptr_eq(&controller, &drivers.borrow().driver_page_controller(initial).unwrap()));
    let old = page.agent.host.retained_binding_state(current_root).unwrap();
    assert!(!Arc::ptr_eq(&controller, &old.prompt_controller()));
    page.execute("popup.location.href='https://example.test/one'").unwrap();
    until(&mut page, "String(popup.document.title==='one')");
    let current = page.agent.host.standalone_navigation_root(initial).unwrap();
    assert!(Arc::ptr_eq(&controller, &page.agent.host.standalone_page_controller(current).unwrap()));
    page.execute("popup.document.body.innerHTML='<p>active</p>'").unwrap();
    page.drain_tasks().unwrap();
    page.execute("popup.location.href='https://example.test/two'").unwrap();
    until(&mut page, "String(popup.document.title==='two')");
    let second = page.agent.host.standalone_navigation_root(initial).unwrap();
    assert!(Arc::ptr_eq(&controller, &page.agent.host.standalone_page_controller(second).unwrap()));
    page.execute("popup.history.back()").unwrap();
    until(&mut page, "String(popup.document.title==='one')");
    let restored = page.agent.host.standalone_navigation_root(initial).unwrap();
    assert!(Arc::ptr_eq(&controller, &page.agent.host.standalone_page_controller(restored).unwrap()));
    assert!(controller.inspect().is_ok());
    // Complete the ordinary history control before deliberately retiring an
    // old owner. Initial about:blank may reuse its binding, so never treat its
    // retained Rc as proof of a distinct replaced Document.
    let replaced = page.agent.host.retained_binding_state(restored).unwrap();
    page.execute("popup.location.href='https://example.test/two'").unwrap();
    until(&mut page, "String(popup.document.title==='two')");
    let active = page.agent.host.standalone_navigation_root(initial).unwrap();
    let active_state = page.agent.host.retained_binding_state(active).unwrap();
    assert_ne!(restored, active);
    assert!(!Rc::ptr_eq(&replaced, &active_state));
    replaced.retire_execution();
    assert!(!active_state.execution_retired());
    assert!(controller.inspect().is_ok(), "distinct old Document cannot retire page publication");
    page.execute("popup.document.body.innerHTML='<p>still active</p>'").unwrap();
    page.drain_tasks().unwrap();
    page.execute("popup.close()").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while controller.inspect().is_ok() {
        page.drain_tasks().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(matches!(controller.inspect(), Err(oryn_js::prompts::PromptError::Retired)));
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
    let _diagnostic = close_diagnostic_scope("native_auxiliary_close_control");
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
fn borrowed_auxiliary_history_projects_only_its_receiver_document() {
    let _diagnostic = close_diagnostic_scope("native_auxiliary_history_close_control");
    let (mut page, _) = page();
    page.execute("globalThis.popup=open();globalThis.originalURL=location.href;globalThis.originalLength=history.length;globalThis.otherHistory=popup.history;otherHistory.pushState({entry:1},'', 'about:blank#one');otherHistory.replaceState({entry:2},'', 'about:blank#two')").unwrap();
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("String(location.href===originalURL&&history.length===originalLength&&history.state===null&&popup.location.href==='about:blank#two'&&popup.history.length===2&&popup.history.state.entry===2&&popup.document!==document)").unwrap(), "true");
    assert_eq!(page.evaluate("(()=>{let reads=0;const data={get entry(){reads++;if(window!==popup.opener)throw Error('getter realm');return 4}};otherHistory.replaceState(data,'','about:blank#getter');return String(reads===1&&popup.history.state.entry===4&&popup.location.href==='about:blank#getter'&&location.href===originalURL&&history.length===originalLength)})()").unwrap(), "true");
    assert_eq!(page.evaluate("(()=>{let conversions=0;const unused={toString(){conversions++;if(window!==popup.opener)throw Error('wrong caller realm');return ''}};otherHistory.replaceState({entry:3},unused,'about:blank#three');const sentinel={};let caught;try{otherHistory.pushState({}, {toString(){throw sentinel}}, 'about:blank#bad')}catch(error){caught=error}return String(conversions===1&&caught===sentinel&&popup.location.href==='about:blank#three'&&location.href===originalURL&&popup.history.length===2)})()").unwrap(), "true");
    page.execute("popup.close()").unwrap();
    until(&mut page, "String(popup.closed)");
    page.drain_tasks().unwrap();
    assert_eq!(page.evaluate("(()=>{let rejected=false;try{otherHistory.pushState({},'', 'about:blank#retired')}catch(error){rejected=true}return String(rejected&&location.href===originalURL&&history.length===originalLength)})()").unwrap(), "true");
}
#[test]
fn discarded_auxiliary_checkpoint_does_not_poll_retired_execution_and_keeps_dom() {
    let (mut page, _) = page();
    page.execute("globalThis.popup=open();globalThis.savedDocument=popup.document;popup.document.body.innerHTML='<p id=saved>kept</p>'").unwrap();
    let root = page.agent.host.standalone_root_realms().into_iter().find(|realm| *realm != page.top_realm).unwrap();
    let current = page.agent.host.standalone_navigation_root(root).unwrap();
    let owner = page.agent.host.retained_binding_state(current).unwrap();
    page.execute("popup.close();globalThis.openerTick=0;setTimeout(()=>openerTick++,0)").unwrap();
    until(&mut page, "String(popup.closed&&openerTick===1)");
    assert!(owner.execution_retired());
    for _ in 0..3 { page.drain_tasks().unwrap(); }
    assert_eq!(page.evaluate("String(!closed&&savedDocument.getElementById('saved').textContent==='kept'&&openerTick===1)").unwrap(), "true");
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
    // Synchronize native creation inventory without consuming the driver's
    // pending adoption queue. The injected failure remains before adoption.
    page.agent.host.adopt_created_realms();
    let popup_realm = page.agent.host.standalone_root_realms().into_iter()
        .find(|realm| *realm != page.top_realm).unwrap();
    let before = page.agent.host.standalone_page_controller(popup_realm).unwrap();
    crate::runtime::standalone_root_driver::FAIL_ADOPTION.with(|fault| fault.set(true));
    assert!(page.drain_tasks().is_err());
    assert!(page.has_agent_host());
    assert!(page.has_pending_page_work());
    assert!(page.next_timer_delay().is_some());
    page.drain_tasks().unwrap();
    assert!(Arc::ptr_eq(&before, &page.agent.host.standalone_page_controller(popup_realm).unwrap()));
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
