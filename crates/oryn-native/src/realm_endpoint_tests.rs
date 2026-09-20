use super::*;
use crate::network::NetworkBroker;

struct NoNetwork;
impl crate::network::NetworkBroker for NoNetwork {
    fn send(&self, _: crate::network::NetworkRequest) -> Result<crate::network::NetworkResponse,crate::network::NetworkError> {
        panic!("endpoint custody regression must not issue network requests")
    }
}
fn endpoints(url:&str) -> RealmEndpoints {
    RealmEndpoints { fetch:Some(FetchState {broker:SharedNetworkBroker::new(NoNetwork),base_url:Url::parse(url).unwrap(),next_script_request:Rc::new(std::cell::Cell::new(900_000))}),
        #[cfg(unix)] context:None, activity_parent:None }
}

#[cfg(unix)]
#[test]
fn child_endpoint_activity_tracks_parent_but_not_sibling_or_replacement(){
    let browser=crate::Browser::in_process_with_config(Default::default());
    let owner=browser.new_context();
    let page=owner.new_page();
    let client=SharedContextClient::direct(owner,page.page_id()).for_document_activity();
    let root=RealmEndpoints{fetch:None,context:Some(ContextState{client:client.clone(),page_id:page.page_id()}),activity_parent:None};
    let child=root.clone().for_child_document();
    let sibling=root.clone().for_child_document();
    let grandchild=child.clone().for_child_document();
    let child_client=&child.context.as_ref().unwrap().client;
    let grandchild_client=&grandchild.context.as_ref().unwrap().client;
    let job=grandchild_client.track_resource_job().unwrap();
    assert_eq!(client.resource_job_activity().0,1);
    assert_eq!(child_client.resource_job_activity().0,1);
    assert_eq!(grandchild_client.resource_job_activity().0,1);
    assert_eq!(sibling.context.as_ref().unwrap().client.resource_job_activity(),(0,None));
    let replacement=child.for_replacement_document().unwrap();
    assert_eq!(replacement.context.as_ref().unwrap().client.resource_job_activity(),(0,None));
    drop(child);
    assert_eq!(client.resource_job_activity().0,1,"realm endpoint release cannot release a descendant job");
    drop(job);
    assert_eq!(client.resource_job_activity().0,0);
    assert!(client.resource_job_activity().1.is_some());
}

#[test]
fn nested_context_entry_resolves_its_own_broker_and_restores_caller_endpoint() {
    let mut host=RawV8Host::new();
    v8::scope!(let scope,&mut host.isolate);
    let (_, first)=create_native_realm_context(scope,Origin::from_url("https://first.test/"),1,None,None,endpoints("https://first.test/base/")).unwrap();
    let (_, second)=create_native_realm_context(scope,Origin::from_url("https://second.test/"),1,None,None,endpoints("https://second.test/base/")).unwrap();
    let first=v8::Local::new(scope,&first);let second=v8::Local::new(scope,&second);
    let outer=&mut v8::ContextScope::new(scope,first);
    assert_eq!(current_fetch_endpoint(outer).unwrap().base_url.as_str(),"https://first.test/base/");
    {
        let inner=&mut v8::ContextScope::new(outer,second);
        assert_eq!(current_fetch_endpoint(inner).unwrap().base_url.as_str(),"https://second.test/base/");
        current_fetch_endpoint(inner).unwrap().next_script_request.set(77);
    }
    assert_eq!(current_fetch_endpoint(outer).unwrap().base_url.as_str(),"https://first.test/base/");
    assert_eq!(current_fetch_endpoint(outer).unwrap().next_script_request.get(),900_000);
}

#[cfg(unix)]
#[test]
fn active_frame_replacement_uses_embedding_activity_parent(){
    let browser=crate::Browser::in_process_with_config(Default::default());
    let owner=browser.new_context();
    let page=owner.new_page();
    let client=SharedContextClient::direct(owner,page.page_id()).for_document_activity();
    let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,None,
        client.clone(),page.page_id(),256*1024*1024).unwrap();
    let root=host.active_realm();
    let child=host.create_realm("https://child.test",1,Some(7)).unwrap();
    let old=host.context_endpoint_for_realm(child).unwrap().client;
    let old_job=old.track_resource_job().unwrap();
    host.activate_realm(child).unwrap();
    let replacement=host.replace_frame_realm(child,"https://new.test",2).unwrap();
    assert_eq!(host.active_realm(),replacement);
    assert!(host.resource_client_for_realm(child,page.page_id()).is_err(),"retired realm cannot recover its retained transport");
    assert!(host.resource_client_for_realm(replacement,oryn_common::v3::PageId(page.page_id().0+1)).is_err(),"a live realm cannot borrow another page's client");
    assert_eq!(host.resource_job_activity_for_realm(replacement),(0,None));
    assert_eq!(host.resource_job_activity_for_realm(root).0,1);
    let new=host.resource_client_for_realm(replacement,page.page_id()).unwrap();
    let new_job=new.track_resource_job().unwrap();
    assert_eq!(old.resource_job_activity().0,1,"new requests must not join the outgoing document");
    assert_eq!(host.resource_job_activity_for_realm(root).0,2);
    assert_eq!(host.resource_job_activity_for_realm(replacement).0,1);
    drop(old_job);
    assert_eq!(host.resource_job_activity_for_realm(root).0,1);
    assert_eq!(host.resource_job_activity_for_realm(replacement).0,1);
    drop(new_job);
    assert_eq!(host.resource_job_activity_for_realm(root).0,0);
}

#[cfg(unix)]
#[test]
fn frame_commit_retains_prepared_response_activity_in_reused_and_replaced_realms(){
    struct Buffered;
    impl crate::network::NetworkBroker for Buffered{
        fn send(&self,request:crate::network::NetworkRequest)->Result<crate::network::NetworkResponse,crate::network::NetworkError>{
            Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![],body:vec![1]})
        }
    }
    let browser=crate::Browser::in_process_with_config(Default::default());
    let owner=browser.new_context();let page=owner.new_page();
    owner.install_document_security(page.page_id(),"https://parent.test/".into(),&[]).unwrap();
    let client=SharedContextClient::direct(owner.clone(),page.page_id()).for_document_activity();
    let mut host=RawV8Host::new_with_network_context_and_heap_limit(Some(SharedNetworkBroker::new(Buffered)),Some(Url::parse("https://parent.test/").unwrap()),
        client.clone(),page.page_id(),256*1024*1024).unwrap();
    let root=host.active_realm();
    let generation=host.realm_descriptor(root).unwrap().document_generation;
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
    host.install_dom(dom.clone(),generation).unwrap();
    host.execute("globalThis.frame=document.createElement('iframe');document.body.append(frame);void frame.contentWindow").unwrap();
    host.adopt_created_realms();
    let frame=dom.borrow().query_selector(dom.borrow().document(),"iframe").unwrap().unwrap();
    let initial=host.realm_ids().into_iter().find(|realm|*realm!=root).unwrap();
    let initial_client=host.resource_client_for_realm(initial,page.page_id()).unwrap();
    let initial_job=initial_client.track_resource_job().unwrap();
    let claimed=host.claim_initial_frame_document(frame).unwrap().unwrap();
    assert_eq!(claimed.0,initial);
    assert_eq!(host.resource_job_activity_for_realm(initial).0,1,"claiming an existing about:blank document preserves its activity view");
    assert!(host.claim_initial_frame_document(frame).unwrap().is_none());
    drop(initial_job);
    let origin=host.realm_descriptor(root).unwrap().origin.clone();
    for (index,origin) in [origin,Origin::from_url("https://replacement.test/")].into_iter().enumerate(){
        let prepared=host.child_resource_client(root).unwrap();
        let response=prepared.reserve_frame_response(page.page_id()).unwrap();
        let broker=host.network_broker_for_realm(root).unwrap().new_request_scope();
        let mut body=broker.start_stream(crate::network::NetworkRequest{request_id:1,method:"GET".into(),
            url:"https://replacement.test/".into(),headers:vec![],body:vec![],credentials:false,
            resolved_addrs:vec![],security:Default::default()}).unwrap();
        let parsed=oryn_dom::html::parse("<p>committed</p>");
        let document=dom.borrow_mut().import_subtree(&parsed.dom,parsed.document).unwrap();
        let realm=host.commit_frame_document_with_resource_owners(crate::host::FrameResourceCommitInput { frame, origin, dom: dom.clone(), document, committed_url: Some("https://replacement.test/"), resource_broker: Some(broker.clone()), resource_client: Some(prepared.clone()), ancestor_capture: None }).unwrap();
        if index==0{assert_eq!(realm,initial,"first same-origin commit reuses its Window");}
        else{assert_ne!(realm,initial);}
        assert_eq!(host.resource_job_activity_for_realm(realm).0,1,"unread response remains owned after commit");
        assert_eq!(host.resource_job_activity_for_realm(root).0,1);
        assert_eq!(host.network_broker_for_realm(realm).unwrap().owned_operation_activity().0,1);
        let document_job=host.context_endpoint_for_realm(realm).unwrap().client.track_resource_job().unwrap();
        assert_eq!(prepared.resource_job_activity().0,2,"subresources share the prepared document view");
        response.finish();
        assert_eq!(host.resource_job_activity_for_realm(realm).0,1);
        drop(document_job);
        assert_eq!(host.resource_job_activity_for_realm(root).0,0);
        let mut bytes=Vec::new();std::io::Read::read_to_end(&mut body.body,&mut bytes).unwrap();
        assert_eq!(bytes,vec![1]);
        assert_eq!(host.network_broker_for_realm(realm).unwrap().owned_operation_activity().0,0);
    }
}

#[test]
fn retirement_and_environment_replacement_never_recover_an_old_endpoint() {
    let mut host=RawV8Host::new();
    v8::scope!(let scope,&mut host.isolate);
    let (realm,context)=create_native_realm_context(scope,Origin::from_url("https://first.test/"),1,None,None,endpoints("https://first.test/")).unwrap();
    let context=v8::Local::new(scope,&context);
    let scope=&mut v8::ContextScope::new(scope,context);
    let old=current_environment(scope).unwrap();
    bind_document_environment(scope,realm,2).unwrap();
    assert!(endpoints_for_environment(scope,old).is_none());
    assert_eq!(current_fetch_endpoint(scope).unwrap().base_url.as_str(),"https://first.test/");
    let current=current_environment(scope).unwrap();
    // Rolling back a never-installed replacement must not erase the active
    // environment's endpoint merely because the realm number matches.
    let uninstalled=scope.get_slot_mut::<AgentEnvironments>().unwrap().create(realm,3,None).unwrap();
    retire_environment(scope,uninstalled.id);
    assert!(endpoints_for_environment(scope,current).is_some());
    retire_environment(scope,current.id);
    assert!(current_fetch_endpoint(scope).is_none());
    assert!(!scope.get_slot::<RealmEndpointRegistry>().unwrap().0.contains_key(&realm));
}

#[cfg(unix)]
#[test]
fn current_parent_service_endpoint_is_bound_to_page_not_last_entered_root() {
    let browser=crate::Browser::in_process_with_config(Default::default());
    let context=browser.new_context();let first=context.new_page();let second=context.new_page();
    let mut host=RawV8Host::new();
    v8::scope!(let scope,&mut host.isolate);
    let endpoint=|page|RealmEndpoints{fetch:None,context:Some(ContextState{client:SharedContextClient::direct(context.clone(),page),page_id:page}),activity_parent:None};
    let (_, a)=create_native_realm_context(scope,Origin::from_url("https://same.test/"),1,None,None,endpoint(first.page_id())).unwrap();
    let (_, b)=create_native_realm_context(scope,Origin::from_url("https://same.test/"),1,None,None,endpoint(second.page_id())).unwrap();
    let a=v8::Local::new(scope,&a);let b=v8::Local::new(scope,&b);
    let outer=&mut v8::ContextScope::new(scope,a);
    assert_eq!(current_context_endpoint(outer).unwrap().page_id,first.page_id());
    {
        let inner=&mut v8::ContextScope::new(outer,b);
        assert_eq!(current_context_endpoint(inner).unwrap().page_id,second.page_id());
        let environment=current_environment(inner).unwrap();retire_environment(inner,environment.id);
        assert!(current_context_endpoint(inner).is_none());
    }
    assert_eq!(current_context_endpoint(outer).unwrap().page_id,first.page_id());
}

#[cfg(unix)]
#[test]
fn two_native_roots_park_restore_and_discard_their_own_parent_capability() {
    let browser=crate::Browser::in_process_with_config(Default::default());
    // A single agent may host related roots; unrelated new_page roots belong
    // to separate browsing-context groups and cannot publish one local tree.
    let owner=browser.new_context();let first=owner.new_page();let second=owner.new_popup(first.page_id(),None);
    let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,None,
        SharedContextClient::direct(owner.clone(),first.page_id()),first.page_id(),256*1024*1024).unwrap();
    host.install_dom(Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),1).unwrap();
    let first_realm=host.active_realm();
    // Mint the second root's native document authority through the production
    // constructor, then attach an independent realm in the first isolate. The
    // temporary authority host has no bindings/tasks to retire on drop.
    let authority=RawV8Host::new_with_network_context_and_heap_limit(None,None,
        SharedContextClient::direct(owner.clone(),second.page_id()),second.page_id(),256*1024*1024).unwrap();
    let descriptor=authority.realm_descriptor(authority.active_realm()).unwrap().clone();
    let second_endpoints=authority.endpoints_for_realm(authority.active_realm()).unwrap();
    drop(authority);
    let (second_realm,context,state)={
        v8::scope!(let scope,&mut host.isolate);
        let (realm,context)=create_native_realm_context(scope,descriptor.origin,descriptor.document_generation,None,None,second_endpoints).unwrap();
        let local=v8::Local::new(scope,&context);
        let dom=Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));let document=dom.borrow().document();
        let state=install_native_document(scope,local,dom,realm,descriptor.document_generation,document,None).unwrap();
        (realm,context,state)
    };
    host.contexts.insert(second_realm,context);host.binding_states.insert(second_realm,state);
    host.isolate.get_slot_mut::<NativeRealmIdentities>().unwrap().pending_contexts.remove(&second_realm);
    // Both roots share the completion channel. A completed response must stay
    // owned by its suspended environment while its sibling can make progress.
    for (operation_id, realm) in [(101, first_realm), (102, second_realm)] {
        let context=host.contexts[&realm].clone();
        v8::scope!(let handles,&mut host.isolate);
        let local=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,local);
        let resolver=v8::PromiseResolver::new(scope).unwrap();
        let environment=current_environment(scope).unwrap();
        let resolver=v8::Global::new(scope,resolver);
        let fetches=scope.get_slot_mut::<AsyncFetches>().unwrap();
        fetches.pending.insert(operation_id,PendingFetch{environment,context,resolver});
        fetches.sender.send(FetchCompletion{operation_id,result:Ok("{}".into()),network:None,stream_terminal:None}).unwrap();
    }
    let first_token=host.park_document_authority(first_realm).unwrap().unwrap();
    let second_token=host.park_document_authority(second_realm).unwrap().unwrap();
    assert_ne!(first_token,second_token);
    assert_eq!(host.parked_documents.len(),2);
    assert!(host.park_document_authority(first_realm).is_err());
    assert!(!host.has_pending_async_fetches());
    assert_eq!(host.drain_async_fetches(true).unwrap(),0);
    assert_eq!(host.isolate.get_slot::<AsyncFetches>().unwrap().deferred.len(),2);
    host.restore_document_authority(first_realm).unwrap();
    assert_eq!(host.parked_documents.len(),1);
    assert!(host.has_pending_async_fetches());
    assert_eq!(host.drain_async_fetches(false).unwrap(),1);
    assert_eq!(host.isolate.get_slot::<AsyncFetches>().unwrap().deferred.len(),1);
    assert!(!host.has_pending_async_fetches());
    assert_eq!(host.parked_documents[&second_realm].token,second_token);
    let second_environment=host.isolate.get_slot::<RealmEndpointRegistry>().unwrap().0[&second_realm].0;
    host.discard_cached_authority(second_realm).unwrap();
    retire_environment(&mut host.isolate,second_environment.id);
    let fetches=host.isolate.get_slot::<AsyncFetches>().unwrap();
    assert!(fetches.deferred.is_empty());
    assert!(fetches.pending.is_empty());
    assert!(!fetches.retired.contains(&102));
    assert_eq!(host.drain_async_fetches(false).unwrap(),0);
    assert!(host.parked_documents.is_empty());
    assert!(host.context_endpoint_for_realm(first_realm).is_some());
}
