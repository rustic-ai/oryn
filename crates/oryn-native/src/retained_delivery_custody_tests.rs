#[cfg(all(test, unix))]
mod retained_delivery_custody_tests {
    use super::*;
    #[test]
    fn same_callback_resource_root_keeps_import_rule_identity_through_delivery() {
        let resources=BTreeMap::from([("https://example.test/child.css".into(),"p{height:23px}".into())]);
        let mut page=RustExecutableDocument::load_with_environment("<!doctype html><p id=p></p>",
            &resources,None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
        assert_eq!(page.agent.host.evaluate_string("globalThis.s=document.createElement('style');s.textContent='@import url(child.css);p{width:17px}';document.head.append(s);globalThis.root=s.sheet;globalThis.rule=root.cssRules[0];[root!==null,root===document.styleSheets[0],getComputedStyle(p).width].join('|')").unwrap(),"true|true|17px");
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        loop {
            page.checkpoint().unwrap();
            page.poll_context_events().unwrap();
            if page.evaluate("String(rule.styleSheet!==null&&getComputedStyle(p).height==='23px')").unwrap()=="true" {break;}
            assert!(std::time::Instant::now()<deadline,"same original import pending");
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("[s.sheet===root,root.cssRules[0]===rule,rule.styleSheet.parentStyleSheet===root,rule.styleSheet.ownerRule===rule,getComputedStyle(p).width,getComputedStyle(p).height].join('|')").unwrap(),"true|true|true|true|17px|23px");
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("String(s.sheet===root&&root.cssRules[0]===rule)").unwrap(),"true");
    }
    #[test]
    fn same_callback_inline_sheet_and_replacement_keep_original_native_identity() {
        let mut page=RustExecutableDocument::load("<!doctype html><p id=p></p>").unwrap();
        assert_eq!(page.agent.host.evaluate_string(
            "globalThis.s=document.createElement('style');s.textContent='p{width:17px}';document.head.append(s);globalThis.first=s.sheet;globalThis.rule=first.cssRules[0];[first!==null,first===document.styleSheets[0],getComputedStyle(p).width].join('|')",
        ).unwrap(),"true|true|17px");
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("[s.sheet===first,s.sheet.cssRules[0]===rule].join('|')").unwrap(),"true|true");
        assert_eq!(page.agent.host.evaluate_string(
            "s.firstChild.data='p{width:23px}';globalThis.second=s.sheet;[second!==first,first.ownerNode===null,getComputedStyle(p).width,second===document.styleSheets[0]].join('|')",
        ).unwrap(),"true|true|23px|true");
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("s.sheet===second").unwrap(),"true");
        assert_eq!(page.evaluate(
            "const wrong=document.createElement('style');wrong.type='text/plain';wrong.textContent='p{width:29px}';document.head.append(wrong);String(wrong.sheet===null&&getComputedStyle(p).width==='23px')",
        ).unwrap(),"true");
    }
    #[test]
    fn pending_import_operational_delivery_keeps_occurrence_and_primary_across_retries() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><style id=s>p{width:17px}</style><p></p>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        page.agent.host.execute("s.sheet.insertRule('@import url(pending.css)',0)").unwrap();
        let realm = page.agent.host.active_realm();
        let provider = page.agent.host.native_layout_provider_in_realm(realm).unwrap();
        let backend = provider.native_backend().unwrap().downcast_ref::<LiveLayoutProvider>().unwrap();
        let allocation = backend.native_retained_allocation.borrow().as_ref().unwrap().clone();
        page.agent.host.refresh_native_retained_allocation(realm,&allocation).unwrap();
        let _scope = allocation.scope_guard();
        let admission = backend.retained_revision_admission().unwrap();
        let snapshot = RetainedPublicationSnapshot::capture(&backend.retained_stylesheets,admission.as_ref()).unwrap().unwrap();
        let record = snapshot.entries.iter().find(|entry| !entry.record.pending_imports.borrow().is_empty()).unwrap().record.clone();
        let request = record.take_pending_import().unwrap().unwrap();
        let occurrence = request.occurrence;
        let key = NativePendingImportKey::for_request(&request,1);
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let request_allocation = admission.reserve(std::mem::size_of::<NativePendingImportJob>()).unwrap();
        let mut scheduler = ParserResourceScheduler::new();
        scheduler.native_pending_imports.insert(key,NativePendingImportJob {
            request,reply:None,failure:None,failed:false,prepared:None,
            _request_allocation:request_allocation,cancellation:cancellation.clone(),
            dependencies:None,committed:false,_dependency_allocation:None,
            settlement_failures:(None,None),
        });
        scheduler.receive_native_pending_import(key,Ok(NativePendingImportReply {
            key,document:record.owner.document,publication_revision:record.owner.revision,
            occurrence,result:Err(RetainedStylesheetFailure::Operational("pending transport infra".into())),
        })).unwrap();
        let job = scheduler.native_pending_imports.get_mut(&key).unwrap();
        for _ in 0..2 {
            let error = job.publish(&mut page.agent.host,&page.agent.dom).unwrap_err();
            assert_eq!(error.to_string(),HostError::Execution("pending transport infra".into()).to_string());
            assert!(!job.failed);
            assert!(job.failure.is_some());
            assert!(matches!(record.graph.borrow().occurrence(occurrence).unwrap().state,
                oryn_layout::cssom::native_import_graph::ImportState::Pending));
            assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        }
        let mut mismatched = key;
        mismatched.job_serial = mismatched.job_serial.checked_add(1).unwrap();
        assert!(scheduler.receive_native_pending_import(key,Ok(NativePendingImportReply {
            key:mismatched,document:record.owner.document,publication_revision:record.owner.revision,
            occurrence,result:Err(RetainedStylesheetFailure::Web),
        })).is_ok());
        let job = scheduler.native_pending_imports.get_mut(&key).unwrap();
        for _ in 0..2 {
            let error = job.publish(&mut page.agent.host,&page.agent.dom).unwrap_err();
            assert_eq!(error.to_string(),HostError::Execution("pending transport infra".into()).to_string());
            assert!(!job.failed);
            assert!(matches!(record.graph.borrow().occurrence(occurrence).unwrap().state,
                oryn_layout::cssom::native_import_graph::ImportState::Pending));
        }
        // A second exact occurrence receives a foreign first reply (not a
        // duplicate), and must retain operational custody without failing it.
        page.agent.host.execute("s.sheet.insertRule('@import url(other.css)',0)").unwrap();
        page.agent.host.refresh_native_retained_allocation(realm,&allocation).unwrap();
        let request2 = record.take_pending_import().unwrap().unwrap();
        let occurrence2 = request2.occurrence;
        let key2 = NativePendingImportKey::for_request(&request2,2);
        let job_charge2 = admission.reserve(std::mem::size_of::<NativePendingImportJob>()).unwrap();
        scheduler.native_pending_imports.insert(key2,NativePendingImportJob {
            request:request2,reply:None,failure:None,failed:false,prepared:None,
            _request_allocation:job_charge2,cancellation:Arc::new(crate::context_service::ResourceCancellation::default()),
            dependencies:None,committed:false,_dependency_allocation:None,settlement_failures:(None,None),
        });
        assert!(scheduler.receive_native_pending_import(key2,Ok(NativePendingImportReply {
            key,document:record.owner.document,publication_revision:record.owner.revision,
            occurrence:occurrence2,result:Err(RetainedStylesheetFailure::Web),
        })).is_err());
        let job2 = scheduler.native_pending_imports.get_mut(&key2).unwrap();
        for _ in 0..2 {
            assert_eq!(job2.publish(&mut page.agent.host,&page.agent.dom).unwrap_err().to_string(),
                HostError::Execution("pending import response owner mismatch".into()).to_string());
            assert!(!job2.failed);
            assert!(matches!(record.graph.borrow().occurrence(occurrence2).unwrap().state,
                oryn_layout::cssom::native_import_graph::ImportState::Pending));
        }
        let job = scheduler.native_pending_imports.get_mut(&key).unwrap();
        page.agent.host.execute("s.remove()").unwrap();
        assert!(job.publish(&mut page.agent.host,&page.agent.dom).unwrap());
        drop(scheduler);
        assert!(cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn operational_delivery_retries_preserve_primary_and_never_dispatch_web_error() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id=s>p{width:17px}</style><p></p>",
        ).unwrap();
        page.agent.host.execute("globalThis.webEvents=0;s.addEventListener('error',()=>webEvents++)").unwrap();
        let arena = page.agent.dom.clone();
        let realm = page.agent.host.active_realm();
        let provider = page.agent.host.native_layout_provider_in_realm(realm).unwrap();
        let backend = provider.native_backend().unwrap().downcast_ref::<LiveLayoutProvider>().unwrap();
        let identity = backend.native_document_identity.get().unwrap();
        let (node, revision) = {
            let dom = arena.borrow();
            let node = dom.get_element_by_id(identity.document,"s").unwrap().unwrap();
            (node,dom.arena().stylesheet_revision(node).unwrap())
        };
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let request = standalone_stylesheet::Cancel::native(cancellation.clone());
        let mut completion = RetainedStylesheetReady {
            identity,node,revision,cancellation:cancellation.clone(),
            result:Err(RetainedStylesheetFailure::Operational("injected transport custody failure".into())),
        }.into_completion(&arena);
        for _ in 0..2 {
            let error = publish_ready_retained_stylesheet(&mut completion,&mut page.agent.host,&arena).unwrap_err();
            assert!(error.to_string().contains("injected transport custody failure"));
            assert!(matches!(completion.phase,RetainedParserCompletionPhase::OperationalFailure { .. }));
            assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        }
        assert_eq!(page.evaluate("String(webEvents)").unwrap(),"0");
        page.agent.host.execute("s.remove()").unwrap();
        assert!(publish_ready_retained_stylesheet(&mut completion,&mut page.agent.host,&arena).unwrap());
        assert!(matches!(completion.phase,RetainedParserCompletionPhase::Retired));
        assert_eq!(page.evaluate("String(webEvents)").unwrap(),"0");
        assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        let custody = RetainedReadyCustody {
            completion,
            _request: (request, Arc::new(std::sync::Mutex::new(None))),
            _allocation: Arc::new(()),
        };
        custody.complete_request();
        assert!(cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn retained_failures_keep_web_outcomes_distinct_from_operational_custody() {
        use crate::script_load_failure::ScriptLoadFailure;
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let node = dom.create_element("style");
        dom.append_child(document, node).unwrap();
        let revision = dom.arena().stylesheet_revision(node).unwrap();
        let arena = Rc::new(RefCell::new(dom));
        let identity = ParserDocumentIdentity {
            realm: RealmId(1), document, document_generation: 1, stream_epoch: 0,
        };
        for error in [
            HostError::ScriptLoad(ScriptLoadFailure::Fetch("MIME denial".into())),
            HostError::ScriptLoad(ScriptLoadFailure::Integrity("digest denial".into())),
        ] {
            let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
            let completion = RetainedStylesheetReady {
                identity, node, revision, cancellation: cancellation.clone(),
                result: Err(RetainedStylesheetFailure::from(error)),
            }.into_completion(&arena);
            assert!(matches!(completion.phase,
                RetainedParserCompletionPhase::Failed { diagnostic: None, .. }));
            assert!(retained_operational_diagnostic(&completion.phase).is_none());
            assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        }
        for error in [
            HostError::CachedAuthorityExpired,
            HostError::LimitExceeded { resource: "source quota".into() },
            HostError::ScriptLoad(ScriptLoadFailure::Infrastructure("transport".into())),
            HostError::ScriptLoad(ScriptLoadFailure::Cancelled),
            HostError::Execution("native allocator".into()),
        ] {
            let expected = error.to_string();
            let mut completion = RetainedStylesheetReady {
                identity, node, revision,
                cancellation: Arc::new(crate::context_service::ResourceCancellation::default()),
                result: Err(RetainedStylesheetFailure::from(error)),
            }.into_completion(&arena);
            assert_eq!(retained_operational_diagnostic(&completion.phase), Some(expected.as_str()));
            assert_eq!(retained_operational_diagnostic(&completion.phase), Some(expected.as_str()));
            completion.cancel_pending();
            assert!(matches!(completion.phase, RetainedParserCompletionPhase::Retired));
        }
    }
    #[test]
    fn generated_owner_rule_getter_is_branded_and_null_for_roots() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id=s>p{width:17px}</style><p></p>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "globalThis.ownerGet=Object.getOwnPropertyDescriptor(CSSStyleSheet.prototype,'ownerRule').get;globalThis.constructed=new CSSStyleSheet();[typeof ownerGet,ownerGet.call(s.sheet)===null,ownerGet.call(constructed)===null].join('|')",
        ).unwrap(), "function|true|true");
        assert_eq!(page.evaluate(
            "let failures=0;for(const x of [{},null,document.querySelector('style')]){try{ownerGet.call(x)}catch(e){if(e instanceof TypeError)failures++}}String(failures)",
        ).unwrap(), "3");
        assert_eq!(page.evaluate(
            "let rejected=false;try{ownerGet.call(new Proxy(constructed,{}))}catch(e){rejected=e instanceof TypeError}String(rejected)",
        ).unwrap(), "true");
    }

    #[test]
    fn authored_source_revision_requeues_once_and_preserves_failed_head() {
        let mut page = RustExecutableDocument::load("<!doctype html><style id=s>p{width:17px}</style><p></p>").unwrap();
        page.agent.host.execute("globalThis.s=document.getElementById('s');s.textContent='p{width:19px}'").unwrap();
        let first = page.agent.host.peek_pending_dynamic_stylesheet().unwrap().unwrap();
        dynamic_stylesheet_preparation::fail_next_dynamic_retained_schedule();
        assert!(page.drain_dynamic_stylesheets().is_err());
        assert_eq!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap(), Some(first));
        // Revision changes while the older head remains unadmitted. Shared
        // preparation must update the marker, not suppress or duplicate it.
        page.agent.host.execute("s.replaceChildren(document.createTextNode('p{width:23px}'))").unwrap();
        assert_eq!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap(), Some(first));
        page.drain_dynamic_stylesheets().unwrap();
        assert!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap().is_none());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page.evaluate("String(s.sheet!==null)").unwrap() == "true" { break; }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("getComputedStyle(document.querySelector('p')).width").unwrap(), "23px");
        page.agent.host.execute("s.innerHTML='p{width:29px}'").unwrap();
        assert_eq!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap(), Some(first));
    }

    #[test]
    fn dynamic_inline_source_replacement_publishes_original_owner_generation() {
        let mut page = RustExecutableDocument::load("<!doctype html><p id=p></p>").unwrap();
        page.execute("globalThis.owner=document.createElement('style');owner.textContent='p{width:17px}';document.head.append(owner)").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            if page.evaluate("String(owner.sheet!==null)").unwrap()=="true" { break; }
            assert!(std::time::Instant::now()<deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("globalThis.old=owner.sheet;owner.textContent='p{width:23px}';String(old.ownerNode===null)").unwrap(), "true");
        loop {
            page.poll_context_events().unwrap();
            if page.evaluate("String(owner.sheet!==null)").unwrap()=="true" { break; }
            assert!(std::time::Instant::now()<deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.evaluate("[owner.sheet!==old,document.styleSheets[0]===owner.sheet,getComputedStyle(p).width].join('|')").unwrap(), "true|true|23px");
    }

    #[test]
    fn dynamic_schedule_preflight_failure_preserves_head_and_remaining_requests() {
        let mut page = RustExecutableDocument::load("<!doctype html><p></p>").unwrap();
        // Author mutation records are produced by the actual binding path.
        page.agent.host.execute("globalThis.a=document.createElement('style');globalThis.b=document.createElement('style');a.textContent='p{width:17px}';b.textContent='p{height:23px}';document.head.append(a,b)").unwrap();
        let head = page.agent.host.peek_pending_dynamic_stylesheet().unwrap().unwrap();
        dynamic_stylesheet_preparation::fail_next_dynamic_retained_schedule();
        assert!(page.drain_dynamic_stylesheets().is_err());
        assert_eq!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap(), Some(head));
        page.drain_dynamic_stylesheets().unwrap();
        assert!(page.agent.host.peek_pending_dynamic_stylesheet().unwrap().is_none());
    }

    #[test]
    fn standalone_inline_publication_needs_no_parent_resource_client() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id=s>p{width:17px}</style><p id=p></p>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "globalThis.sheet=s.sheet;[sheet!==null,document.styleSheets[0]===sheet,sheet.ownerNode===s,getComputedStyle(p).width].join('|')",
        ).unwrap(), "true|true|true|17px");
        assert_eq!(page.evaluate(
            "globalThis.rule=sheet.cssRules[0];sheet.insertRule('p{height:23px}',1);rule.style.width='19px';[sheet.cssRules[0]===rule,getComputedStyle(p).width,getComputedStyle(p).height].join('|')",
        ).unwrap(), "true|19px|23px");
    }
    #[test]
    fn successful_transport_release_does_not_retire_published_inline_authority() {
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let request = standalone_stylesheet::Cancel::native(cancellation.clone());
        request.complete();
        assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        let stale_request = standalone_stylesheet::Cancel::native(cancellation.clone());
        drop(stale_request);
        assert!(cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn dequeue_and_retry_do_not_cancel_the_retained_response_owner() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let node = dom.create_element("style");
        dom.append_child(document, node).unwrap();
        let revision = dom.arena().stylesheet_revision(node).unwrap();
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let mut scheduler = ParserResourceScheduler::new();
        scheduler.reserve_stylesheet(node, revision).unwrap();
        scheduler.local_stylesheets.insert((node, revision), (
            standalone_stylesheet::Cancel::native(cancellation.clone()),
            Arc::new(std::sync::Mutex::new(None)),
        ));
        scheduler.receive_completion(ParserResourceCompletion::RetainedStylesheet(RetainedStylesheetReady {
            identity: ParserDocumentIdentity { realm: RealmId(1), document, document_generation: 1, stream_epoch: 0 },
            node, revision, cancellation: cancellation.clone(), result: Err(RetainedStylesheetFailure::Operational("test response".into())),
        }));
        let completion = scheduler.take_ready_completion().unwrap();
        assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        assert!(scheduler.local_stylesheets.contains_key(&(node, revision)));
        scheduler.retry_completion(completion);
        let completion = scheduler.take_ready_completion().unwrap();
        assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        assert!(scheduler.local_stylesheets.contains_key(&(node, revision)));
        drop(completion);
        let request = scheduler.local_stylesheets.remove(&(node, revision));
        assert!(!cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
        drop(request);
        assert!(cancellation.flag.load(std::sync::atomic::Ordering::Acquire));
    }
    #[tokio::test(flavor = "current_thread")]
    async fn published_inline_sheet_keeps_authority_for_late_import_mutation() {
        struct Styles;
        impl crate::network::NetworkBroker for Styles {
            fn send(&self, request: crate::network::NetworkRequest) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                Ok(crate::network::NetworkResponse {
                    response_policy: None, timing: None, request_id: request.request_id,
                    status: 200, headers: vec![("content-type".into(), "text/css".into())],
                    body: b"p{height:23px}".to_vec(),
                })
            }
        }
        let context = crate::Browser::in_process_probe().new_context_with_broker(
            crate::ContextOptions::default(), crate::network::SharedNetworkBroker::new(Styles));
        let page = context.new_page();
        page.load_html("https://example.test/inline.html", "<!doctype html><style>p{width:17px}</style><p id=p></p>").await.unwrap();
        assert_eq!(page.evaluate("globalThis.root=document.styleSheets[0];globalThis.original=root.cssRules[0];getComputedStyle(document.getElementById('p')).width").await.unwrap(), "17px");
        assert_eq!(page.evaluate("[root===document.querySelector('style').sheet,document.styleSheets.item(0)===root].join('|')").await.unwrap(), "true|true");
        assert_eq!(page.evaluate("root.insertRule('@import url(late.css);',0);globalThis.imported=root.cssRules[0];String(root.cssRules[1]===original)").await.unwrap(), "true");
        assert_eq!(page.evaluate("[imported instanceof CSSImportRule,typeof imported.styleSheet!=='undefined',imported.parentStyleSheet===root].join('|')").await.unwrap(), "true|true|true");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let result = page.evaluate("String(imported.styleSheet!==null)").await.unwrap();
            if result == "true" { break; }
            assert!(std::time::Instant::now() < deadline, "late import completion stayed pending");
            tokio::task::yield_now().await;
        }
        assert_eq!(page.evaluate("original.style.width='19px';[root.cssRules[1]===original,imported.styleSheet.parentStyleSheet===root,getComputedStyle(document.getElementById('p')).width,getComputedStyle(document.getElementById('p')).height].join('|')").await.unwrap(), "true|true|19px|23px");
    }
}
