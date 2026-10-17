// Native auxiliary-window structured-message transport. Included on Unix.
use oryn_js::message_wire::{MessageRequest,MessageResponse,WindowTarget,TargetOrigin};
fn native_message_request(scope:&mut v8::PinScope,request:MessageRequest)->Result<MessageResponse,ScriptError>{
    if matches!(&request,MessageRequest::PostWindow{..}){publish_local_window_inventory(scope).map_err(|error|remote_window_error(error.to_string()))?;}
    current_environment(scope).map_err(remote_window_error)?;
    let (request, endpoint)=if matches!(&request,MessageRequest::PostWindow{..}){
        let source=oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope).or_else(||scope.get_current_context().get_slot::<V8BindingState>()).ok_or_else(ScriptError::retired)?;
        let endpoint=context_endpoint_for_binding(scope,&source).ok_or_else(ScriptError::retired)?;
        (source.scoped_message_request(request).map_err(|error|remote_window_error(error.to_string()))?, endpoint)
    }else{(request,current_context_endpoint(scope).ok_or_else(||remote_window_error("message context unavailable"))?)};
    match endpoint.client.request(ContextServiceRequest::MessageWire{source_page_id:endpoint.page_id,request}){
        Ok(ContextServiceResponse::MessageWire{result})=>result.map_err(remote_window_error),
        _=>Err(remote_window_error("message transport failed")),
    }
}
fn publish_native_message_frame(scope:&mut v8::PinScope,context:v8::Local<v8::Context>,frame:&FrameLifetime)->Result<(),ScriptError>{
    let scope=&mut v8::ContextScope::new(scope,context);
    if current_context_endpoint(scope).is_none(){return Ok(())}
    let state=context.get_slot::<V8BindingState>().ok_or_else(ScriptError::retired)?;
    let generation=(state.document_generation(),state.document_authority_epoch());
    if frame.message_generation.get()==Some(generation){return Ok(())}
    let origin=context.get_slot::<RealmDescriptor>().ok_or_else(ScriptError::retired)?.origin.serialize();
    let child_origin=context.get_slot::<RealmDescriptor>().ok_or_else(ScriptError::retired)?.origin.clone();
    let parent=scope.get_slot::<NativeRealmIdentities>().and_then(|registry|registry.contexts.get(&frame.parent_environment.realm)).cloned().and_then(|weak|weak.to_local(scope));
    let inherit_origin=parent.and_then(|parent|{
        if parent.get_slot::<RealmDescriptor>().is_some_and(|descriptor|descriptor.origin==child_origin){
            Some(parent.get_slot::<FrameLifetime>().map(|frame|WindowTarget::Frame(frame.message_token.clone())).unwrap_or(WindowTarget::Root))
        }else{None}
    });
    let private_parent=parent.and_then(|context|context.get_slot::<V8BindingState>()).filter(|parent|parent.document_prerendering());
    if let Some(parent)=private_parent{
        let endpoint=current_context_endpoint(scope).ok_or_else(ScriptError::retired)?;
        let creator=parent.native_worker_creator().map_err(|error|remote_window_error(error.to_string()))?;
        match endpoint.client.request(ContextServiceRequest::RegisterPrerenderFrame{page_id:endpoint.page_id,parent:creator,frame:frame.message_token.clone(),generation:generation.0,epoch:generation.1,origin,inherit_origin:inherit_origin.is_some()}){
            Ok(ContextServiceResponse::PrerenderFrameRegistered)=>{},
            Ok(_)=>return Err(remote_window_error("private frame registration acknowledgement mismatch")),
            Err(error)=>return Err(remote_window_error(error.to_string())),
        }
    }else{
        native_message_request(scope,MessageRequest::RegisterWindow{frame:frame.message_token.clone(),document_generation:generation.0,stream_epoch:generation.1,origin,inherit_origin})?;
    }
    state.set_message_target(WindowTarget::Frame(frame.message_token.clone()));
    frame.message_generation.set(Some(generation));Ok(())
}
fn retire_native_message_frame(scope:&mut v8::PinScope,frame:&FrameLifetime){
    if let Some(owner)=scope.get_slot::<Arc<crate::runtime::deferred_fetch::StandaloneOwner>>(){owner.remove(&frame.message_token);}
    frame.deferred_cleanup.borrow_mut().take();

    if let Some((document_generation,stream_epoch))=frame.message_generation.take()
        && let Some(endpoint)=endpoints_for_environment(scope,frame.parent_environment).and_then(|entry|entry.context){
            let _=endpoint.client.request(ContextServiceRequest::MessageWire{source_page_id:endpoint.page_id,request:MessageRequest::RetireWindow{frame:frame.message_token.clone(),document_generation,stream_epoch}});
        }
}
fn native_message_source(scope:&mut v8::PinScope)->Result<WindowTarget,ScriptError>{
    let source=oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope).or_else(||scope.get_current_context().get_slot::<V8BindingState>()).ok_or_else(ScriptError::retired)?;
    if source.execution_retired(){return Err(ScriptError::retired())}
    let contexts=scope.get_slot::<NativeRealmIdentities>().map(|registry|registry.contexts.values().cloned().collect::<Vec<_>>()).unwrap_or_default();
    for weak in contexts {
        let Some(context)=weak.to_local(scope)else{continue};
        if !context.get_slot::<V8BindingState>().is_some_and(|state|Rc::ptr_eq(&state,&source)){continue}
        if let Some(frame)=context.get_slot::<FrameLifetime>() {
            if frame.closed.get(){return Err(ScriptError::retired())}
            publish_native_message_frame(scope,context,&frame)?;
            return Ok(WindowTarget::Frame(frame.message_token.clone()));
        }
        if context.get_slot::<RealmDescriptor>().is_some_and(|realm|realm.frame_id.is_none()){return Ok(WindowTarget::Root)}
    }Err(ScriptError::retired())
}
fn native_window_target_object<'s>(scope:&mut v8::PinScope<'s,'_>,page:PageId,target:WindowTarget,allow_closed:bool)->Result<v8::Local<'s,v8::Object>,ScriptError>{
    if current_context_endpoint(scope).is_some_and(|state|state.page_id==page){
        let contexts=scope.get_slot::<NativeRealmIdentities>().map(|registry|registry.contexts.values().cloned().collect::<Vec<_>>()).unwrap_or_default();
        for weak in contexts {
            let Some(context)=weak.to_local(scope)else{continue};
            let matched=match &target {
                WindowTarget::Root=>context.get_slot::<RealmDescriptor>().is_some_and(|realm|realm.frame_id.is_none())&&context.get_slot::<V8BindingState>().is_some_and(|state|state.binding_global()==BindingGlobal::Window),
                WindowTarget::Frame(token)=>context.get_slot::<FrameLifetime>().is_some_and(|frame|frame.message_token==*token&&!frame.closed.get()),
            };
            if matched&&context.get_slot::<V8BindingState>().is_some_and(|state|!state.execution_retired()){return Ok(context.global(scope))}
        }
        if !allow_closed{return Err(remote_window_error("local browsing context lifetime is retired"))}
    }
    remote_window_object(scope,page,target,true,allow_closed)
}
fn remote_window_post_message(scope:&mut v8::PinScope,args:&v8::FunctionCallbackArguments,entry:&RemoteWindow,live:bool){
    if args.length()==0{throw_type_error(scope,"postMessage requires a message");return}
    let options=args.get(1);let dictionary=options.is_object()||options.is_null_or_undefined();
    let include_activation=if dictionary{let Some(include)=oryn_js::v8_bindings::structured_clone::include_user_activation(scope,options)else{return};include}else{false};
    let origin_value=if dictionary {
        if let Ok(object)=v8::Local::<v8::Object>::try_from(options){let key=v8::String::new(scope,"targetOrigin").unwrap();let Some(value)=object.get(scope,key.into())else{return};value}else{v8::undefined(scope).into()}
    }else{options};
    let origin=if dictionary&&origin_value.is_undefined(){"/".to_string()}else{let Some(value)=origin_value.to_string(scope)else{return};value.to_rust_string_lossy(scope)};
    let transfer_options=if dictionary{options}else{let object=v8::Object::new(scope);let key=v8::String::new(scope,"transfer").unwrap();object.create_data_property(scope,key.into(),args.get(2));object.into()};
    let Some(transfers)=oryn_js::v8_bindings::structured_clone::transfer_list(scope,transfer_options)else{return};
    let source=match native_message_source(scope){Ok(source)=>source,Err(error)=>{throw_type_error(scope,&error.message);return}};
    let target_origin=match origin.as_str(){
        "*"=>TargetOrigin::Any,
        "/"=>TargetOrigin::SameOrigin,
        text=>{let Ok(url)=Url::parse(text)else{oryn_js::v8_bindings::window_messages::throw_external_message_error(scope,"SyntaxError");return};match url.origin(){url::Origin::Tuple(..)=>TargetOrigin::Tuple(url.origin().ascii_serialization()),_=>TargetOrigin::Never}},
    };
    let Some(payload)=oryn_js::v8_bindings::structured_clone::serialize_wire_with_activation(scope,args.get(0),&transfers,|scope|if include_activation{let state=oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope).or_else(||scope.get_current_context().get_slot::<V8BindingState>());Some(state.map_or([false,false],|state|[state.has_transient_user_activation(),state.has_sticky_user_activation()]))}else{None})else{return};
    let _shared_send_guard = oryn_js::v8_bindings::structured_clone::shared_send_guard(scope,&payload);
    let transferred=payload.ports.clone();
    // A retained closed or severed Window still converts and serializes the
    // message. Transfer side effects occur, but the target must not be revived
    // or receive a queued message. Release native transferred-port custody
    // through the current source's endpoint, never the stale destination.
    if !live {
        if !transferred.is_empty(){
            let source=oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope).or_else(||scope.get_current_context().get_slot::<V8BindingState>());
            let endpoint=source.as_ref().and_then(|source|context_endpoint_for_binding(scope,source));
            let released=endpoint.is_some_and(|endpoint|matches!(endpoint.client.request(ContextServiceRequest::MessageWire{source_page_id:endpoint.page_id,request:MessageRequest::DiscardTransfers{ports:transferred}}),Ok(ContextServiceResponse::MessageWire{result:Ok(MessageResponse::Done)})));
            if !released{oryn_js::v8_bindings::window_messages::throw_external_message_error(scope,"QuotaExceededError");}
        }
        return;
    }
    if native_message_request(scope,MessageRequest::PostWindow{target_page:entry.page.0,target:entry.target.clone(),source,target_origin,payload}).is_err(){
        if let Some(endpoint)=current_context_endpoint(scope){let _=endpoint.client.request(ContextServiceRequest::MessageWire{source_page_id:endpoint.page_id,request:MessageRequest::DiscardTransfers{ports:transferred}});}
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope,"QuotaExceededError");
    }
}
impl RawV8Host {
    fn poll_remote_message_tasks(&mut self)->Result<usize,HostError>{
        let contexts=self.contexts.iter().filter(|(realm,_)| !self.realm_is_parked(**realm)).map(|(_,context)|context.clone()).collect::<Vec<_>>();let mut delivered=0;
        for context in contexts {
            let deadline=ExecutionDeadline::arm(&self.isolate,self.execution_wall_limit);
            v8::scope!(let handles,&mut self.isolate);let context=v8::Local::new(handles,&context);let scope=&mut v8::ContextScope::new(handles,context);
            // Script close commits parent retirement before actor cleanup drops
            // the retained contexts. Never register/poll that closed root or its
            // descendants during an unrelated live root's shared-host turn.
            let root=context.get_slot::<FrameLifetime>().and_then(|frame|self.contexts.get(&frame.root_realm))
                .map(|root|v8::Local::new(scope,root)).unwrap_or(context);
            if root.get_slot::<NativeWindowLifecycle>().is_some_and(|window|window.closed.get()){continue}
            checkpoint_frame_lifetimes(scope).map_err(|error|HostError::Execution(error.to_string()))?;
            let Some(state)=context.get_slot::<V8BindingState>()else{continue};if state.execution_retired(){continue}
            // This host can retain standalone and parent-backed realms together.
            // Admit the visited context, not whichever realm the caller selected.
            // Connected realms still register and poll their exact native creator.
            if current_context_endpoint(scope).is_none(){continue}
            delivered+=state.poll_external_ports(scope).map_err(|e|HostError::Execution(e.to_string()))?;
            let target=if let Some(frame)=context.get_slot::<FrameLifetime>(){
                if frame.closed.get(){continue}publish_native_message_frame(scope,context,&frame).map_err(|e|HostError::Execution(e.message))?;WindowTarget::Frame(frame.message_token.clone())
            }else if context.get_slot::<RealmDescriptor>().is_some_and(|realm|realm.frame_id.is_none())&&state.binding_global()==BindingGlobal::Window{WindowTarget::Root}else{continue};
            if !state.has_pending_window_message(){
                let response=native_message_request(scope,state.scoped_message_receive(MessageRequest::PollWindow{target,max:1}).map_err(|error|HostError::Execution(error.to_string()))?).map_err(|e|HostError::Execution(e.message))?;
                let MessageResponse::WindowMessages(messages)=response else{return Err(HostError::Execution("native message poll response mismatch".into()))};
                for message in messages {
                    let source_page=PageId(message.source_page);let source_target=message.source;
                    let source=Rc::new(move|scope:&mut v8::PinScope|{
                        let source=native_window_target_object(scope,source_page,source_target.clone(),true).map_err(|error|oryn_js::v8_bindings::V8BindingError::Message(error.message))?;
                        Ok(v8::Global::new(scope,v8::Local::<v8::Value>::from(source)))
                    });
                    oryn_js::v8_bindings::window_messages::queue_external_window(&state,message.payload,&message.source_origin,source).map_err(|error|HostError::Execution(error.to_string()))?;
                }
            }
            if state.has_pending_window_message(){
                delivered+=usize::from(oryn_js::v8_bindings::window_messages::deliver(scope,&state).map_err(|error|HostError::Execution(error.to_string()))?);
                scope.perform_microtask_checkpoint();
            }
            if deadline.expired()||scope.is_execution_terminating(){scope.cancel_terminate_execution();return Err(HostError::LimitExceeded{resource:"external message task wall time".into()})}
        }Ok(delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // OwnedIsolate enters on construction. These wire fixtures retain two hosts
    // on one thread, so every operation must temporarily select its own isolate
    // and restore the most recently constructed host before ordinary field drop.
    // This is an isolate entry only: it must not suspend/resume page history or
    // retire input defaults, broadcast subscriptions, or pending message tasks.
    struct MessageHost(RawV8Host);
    struct MessageHostEntry<'a>(&'a mut RawV8Host);
    impl MessageHost {
        fn entered(&mut self) -> MessageHostEntry<'_> {
            self.0.enter_realm();
            MessageHostEntry(&mut self.0)
        }
    }
    impl std::ops::Deref for MessageHostEntry<'_> {
        type Target = RawV8Host;
        fn deref(&self) -> &RawV8Host { self.0 }
    }
    impl std::ops::DerefMut for MessageHostEntry<'_> {
        fn deref_mut(&mut self) -> &mut RawV8Host { self.0 }
    }
    impl Drop for MessageHostEntry<'_> {
        fn drop(&mut self) { self.0.exit_realm(); }
    }
    fn host(context:&crate::BrowserContext,page:PageId)->MessageHost{
        let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,None,SharedContextClient::direct(context.clone(),page),page,256*1024*1024).unwrap();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())),1).unwrap();MessageHost(host)
    }
    fn checkpoint(host:&mut MessageHost){
        let mut host=host.entered();
        host.drain_scheduled_timers(false).unwrap();host.drain_async_fetches(false).unwrap();
    }
    struct BroadcastBackend {
        hub: std::sync::Arc<std::sync::Mutex<oryn_js::message_wire::MessageHub>>,
        page: u64,
    }
    impl oryn_js::message_wire::MessageTransport for BroadcastBackend {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            let origins = BTreeMap::from([(1, "https://a.test".into()), (2, "https://a.test".into())]);
            let mut hub = self.hub.lock().unwrap();
            hub.set_partition(oryn_js::message_wire::Principal::Window(self.page), "context/site".into());
            hub.handle(self.page, request, &origins)
        }
    }
    fn broadcast_host(hub: std::sync::Arc<std::sync::Mutex<oryn_js::message_wire::MessageHub>>, page: u64) -> MessageHost {
        let mut host = RawV8Host::new_with_heap_limit(256 * 1024 * 1024);
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())), 1).unwrap();
        {
            let context = host.contexts[&host.active_realm()].clone();
            v8::scope!(let handles, &mut host.isolate);
            let context = v8::Local::new(handles, &context);
            let state = context.get_slot::<V8BindingState>().unwrap();
            state.set_message_transport(std::sync::Arc::new(BroadcastBackend { hub, page }));
        }
        MessageHost(host)
    }
    fn poll_broadcast(host: &mut MessageHost) {
        let mut host = host.entered();
        let context = host.contexts[&host.active_realm()].clone();
        v8::scope!(let handles, &mut host.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let state = context.get_slot::<V8BindingState>().unwrap();
        state.poll_external_ports(scope).unwrap();
    }
    #[test]
    fn message_fixture_restores_current_isolate_after_nested_entry_and_unwind() {
        let hub = Arc::new(Mutex::new(oryn_js::message_wire::MessageHub::default()));
        let mut source = broadcast_host(hub.clone(), 1);
        source.entered().execute("globalThis.fixtureOwner='source'").unwrap();
        let mut target = broadcast_host(hub, 2);
        // Deliberately use the ambient target without a guard below: these
        // operations prove that the guard restored the previous isolate.
        target.0.execute("globalThis.fixtureOwner='target'").unwrap();
        {
            let mut entered_source = source.entered();
            assert_eq!(entered_source.evaluate_string("fixtureOwner").unwrap(), "source");
            {
                let mut entered_target = target.entered();
                assert_eq!(entered_target.evaluate_string("fixtureOwner").unwrap(), "target");
                entered_target.execute("globalThis.targetOnly=1").unwrap();
            }
            assert_eq!(entered_source.evaluate_string("typeof targetOnly").unwrap(), "undefined");
        }
        assert_eq!(target.0.evaluate_string("fixtureOwner").unwrap(), "target");
        assert!(source.entered().execute("throw Error('fixture script failure')").is_err());
        assert_eq!(target.0.evaluate_string("fixtureOwner").unwrap(), "target");
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut entered_source = source.entered();
            entered_source.execute("globalThis.beforeUnwind=17").unwrap();
            panic!("fixture entry unwind control");
        }));
        assert!(unwind.is_err());
        assert_eq!(target.0.evaluate_string("fixtureOwner").unwrap(), "target");
        assert_eq!(target.0.evaluate_string("typeof beforeUnwind").unwrap(), "undefined");
        assert_eq!(source.entered().evaluate_string("String(beforeUnwind)").unwrap(), "17");
        // Normal reverse creation order drops target, then source. The guards
        // must leave both original OwnedIsolate construction entries balanced.
    }
    #[test]
    fn document_open_preserves_broadcast_subscription_and_object_listener(){
        let hub=std::sync::Arc::new(std::sync::Mutex::new(oryn_js::message_wire::MessageHub::default()));
        let mut source=broadcast_host(hub.clone(),1);let mut target=broadcast_host(hub,2);
        source.entered().execute("globalThis.channel=new BroadcastChannel('open-survival')").unwrap();
        target.entered().execute("globalThis.seen=[];globalThis.channel=new BroadcastChannel('open-survival');channel.onmessage=e=>seen.push(e.data)").unwrap();
        source.entered().execute("channel.postMessage('before')").unwrap();
        let (before,authority)={let target=target.entered();let realm=target.active_realm();(target.document_stream_epoch(realm),target.document_authority_epoch(realm))};
        target.entered().execute("document.open();document.write('<body>new input</body>');document.close()").unwrap();
        {let target=target.entered();let realm=target.active_realm();assert_ne!(target.document_stream_epoch(realm),before);assert_eq!(target.document_authority_epoch(realm),authority);}
        source.entered().execute("channel.postMessage('after')").unwrap();poll_broadcast(&mut target);poll_broadcast(&mut target);
        assert_eq!(target.entered().evaluate_string("seen.join(',')").unwrap(),"before,after");
        target.entered().execute("channel.postMessage('reply');channel.close()").unwrap();
    }

    #[test]
    fn broadcast_native_clone_trusted_events_and_close_ordering() {
        let hub = std::sync::Arc::new(std::sync::Mutex::new(oryn_js::message_wire::MessageHub::default()));
        let mut source = broadcast_host(hub.clone(), 1);
        let mut target = broadcast_host(hub, 2);
        target.entered().execute(r#"
            globalThis.valid=false;globalThis.channel=new BroadcastChannel('\ud800');
            channel.onmessage=e=>{const d=e.data;valid=e.isTrusted&&e.target===channel&&e.source===null
                &&e.origin==='https://a.test'&&e.ports.length===0&&d.self===d
                &&d.map instanceof Map&&d.map.get(d)===123n&&d.text.charCodeAt(0)===0xd800};
            JSON.parse=JSON.stringify=()=>{throw Error('author JSON')};
            globalThis.MessageEvent=()=>{throw Error('author constructor')};
        "#).unwrap();
        source.entered().execute(r#"
            globalThis.channel=new BroadcastChannel('\ud800');globalThis.selfMessages=0;
            channel.onmessage=()=>selfMessages++;
            const d={text:'\ud800'};d.self=d;d.map=new Map([[d,123n]]);channel.postMessage(d);
        "#).unwrap();
        assert_eq!(target.entered().evaluate_string("String(valid)").unwrap(), "false");
        poll_broadcast(&mut target); poll_broadcast(&mut source);
        assert_eq!(target.entered().evaluate_string("String(valid)").unwrap(), "true");
        assert_eq!(source.entered().evaluate_string("String(selfMessages)").unwrap(), "0");
        assert_eq!(source.entered().evaluate_string(r#"(()=>{
            channel.close();channel.close();let read=false;const d={get value(){read=true;throw Error('getter')}};
            try{channel.postMessage(d)}catch(e){return e.name==='InvalidStateError'&&!read}return false;
        })()"#).unwrap(), "true");
        target.entered().execute("channel.close();valid=false").unwrap();
        source.entered().execute("new BroadcastChannel('\\ud800').postMessage('after-close')").unwrap();
        poll_broadcast(&mut target);
        assert_eq!(target.entered().evaluate_string("String(valid)").unwrap(), "false");
    }
    #[tokio::test]
    async fn auxiliary_clone_preserves_graph_types_transfers_and_trusted_destination_events(){
        let context=crate::Browser::new().new_context();let parent=context.new_page();let popup=context.new_popup(parent.page_id(),None);
        let mut source=host(&context,parent.page_id());let mut target=host(&context,popup.page_id());
        target.entered().configure_native_page_opener(Some(parent.page_id())).unwrap();
        target.entered().execute(r#"
            globalThis.valid=false;globalThis.receivedPort=null;
            addEventListener('message',event=>{
                const d=event.data;
                valid=event.isTrusted && event.source===opener && event.ports.length===1 && d.port===event.ports[0]
                    && d.self===d && d.map instanceof Map && d.map.get(d)===d.set && d.set instanceof Set && d.set.has(d)
                    && d.bytes instanceof Uint8Array && d.bytes.buffer===d.buffer && d.bytes[0]===17
                    && d.big===12345678901234567890n && d.text.charCodeAt(0)===0xd800
                    && d.date instanceof Date && d.date.getTime()===1234 && d.regex instanceof RegExp
                    && d.error instanceof TypeError && d.error.message==='typed'
                    && d.blob instanceof Blob && d.file instanceof File && d.file.name==='x.bin';
                receivedPort=event.ports[0];receivedPort.postMessage(new Map([['reply',91]]));
            });
            JSON.parse=JSON.stringify=()=>{throw Error('poisoned JSON')};
            globalThis.MessageEvent=()=>{throw Error('poisoned constructor')};
            globalThis.dispatchEvent=()=>{throw Error('poisoned dispatch')};
        "#).unwrap();
        source.entered().execute(&format!("globalThis.remote=__oryn_native_page_proxy({})",popup.page_id().0)).unwrap();
        source.entered().execute(r#"
            globalThis.answer=0;const channel=new MessageChannel();channel.port1.onmessage=e=>answer=e.data.get('reply');
            const buffer=new ArrayBuffer(8);const bytes=new Uint8Array(buffer);bytes[0]=17;
            const d={buffer,bytes,port:channel.port2,big:12345678901234567890n,text:'\ud800',date:new Date(1234),regex:/x/gi,error:new TypeError('typed'),blob:new Blob(['a']),file:new File(['b'],'x.bin')};
            d.self=d;d.set=new Set([d]);d.map=new Map([[d,d.set]]);
            JSON.parse=JSON.stringify=()=>{throw Error('poisoned JSON')};
            remote.postMessage(d,{targetOrigin:'*',transfer:[buffer,channel.port2]});
            globalThis.detached=buffer.byteLength===0;
        "#).unwrap();
        assert_eq!(source.entered().evaluate_string("String(detached)").unwrap(),"true");
        assert_eq!(target.entered().evaluate_string("String(valid)").unwrap(),"false");
        checkpoint(&mut target);checkpoint(&mut source);
        assert_eq!(target.entered().evaluate_string("String(valid)").unwrap(),"true");
        assert_eq!(source.entered().evaluate_string("String(answer)").unwrap(),"91");
        popup.close().await.unwrap();parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn auxiliary_transfer_failures_are_atomic_and_closed_target_still_detaches(){
        let context=crate::Browser::new().new_context();let parent=context.new_page();let popup=context.new_popup(parent.page_id(),None);
        let mut source=host(&context,parent.page_id());
        source.entered().execute(&format!("globalThis.remote=__oryn_native_page_proxy({})",popup.page_id().0)).unwrap();
        assert_eq!(source.entered().evaluate_string(r#"(()=>{
            const b=new ArrayBuffer(4);let errors=0;
            for(const [value,transfer] of [[()=>{},[b]],[{},[b,b]],[new Proxy({},{}),[]],[remote,[]]]){
                try{remote.postMessage(value,'*',transfer)}catch(e){if(e.name==='DataCloneError')errors++}
            }
            return errors===4&&b.byteLength===4;
        })()"#).unwrap(),"true");
        source.entered().execute("globalThis.receivedRAB=false;globalThis.buffer=new ArrayBuffer(4,{maxByteLength:16});new Uint8Array(buffer)[1]=9").unwrap();
        let mut target=host(&context,popup.page_id());
        target.entered().execute("addEventListener('message',e=>{globalThis.receivedRAB=e.data.resizable&&e.data.maxByteLength===16&&new Uint8Array(e.data)[1]===9;e.data.resize(8)})").unwrap();
        source.entered().execute("remote.postMessage(buffer,'*',[buffer])").unwrap();checkpoint(&mut target);
        assert_eq!(target.entered().evaluate_string("String(receivedRAB)").unwrap(),"true");
        popup.close().await.unwrap();
        assert_eq!(source.entered().evaluate_string("(()=>{const b=new ArrayBuffer(2);remote.postMessage(b,'*',[b]);return b.byteLength})()").unwrap(),"0");
        assert_eq!(source.entered().evaluate_string(r#"(()=>{
            const b=new ArrayBuffer(3),pair=new MessageChannel();let reads=0;
            try{remote.postMessage(()=>{},'*',[b]);return false}catch(e){if(e.name!=='DataCloneError'||b.byteLength!==3)return false}
            try{remote.postMessage({},'not a URL',[b]);return false}catch(e){if(e.name!=='SyntaxError'||b.byteLength!==3)return false}
            globalThis.closedPeer=false;globalThis.closedPeerCalls=0;pair.port1.onclose=()=>{closedPeer=true;closedPeerCalls++};pair.port1.start();globalThis.retainedClosedPeer=pair.port1;
            remote.postMessage({get data(){reads++;return b}},{targetOrigin:'*',transfer:[b,pair.port2]});
            return reads===1&&b.byteLength===0;
        })()"#).unwrap(),"true");
        checkpoint(&mut source);
        assert_eq!(source.entered().evaluate_string("String(closedPeer)").unwrap(),"false","the port poll queues a separate close-event task");
        checkpoint(&mut source);
        assert_eq!(source.entered().evaluate_string("String(closedPeer)").unwrap(),"true");
        assert_eq!(source.entered().evaluate_string("String(closedPeerCalls)").unwrap(),"1");
        checkpoint(&mut source);
        assert_eq!(source.entered().evaluate_string("String(closedPeerCalls)").unwrap(),"1","a consumed peer-close notification must not replay");
        parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn severed_proxy_serializes_but_cannot_deliver_to_an_installed_live_document(){
        let context=crate::Browser::new().new_context();let parent=context.new_page();
        context.install_document_security(parent.page_id(),"https://a.test/".into(),&[("Cross-Origin-Opener-Policy".into(),"same-origin".into())]).unwrap();
        let popup=context.new_popup(parent.page_id(),None);let mut source=host(&context,parent.page_id());
        source.entered().execute(&format!("globalThis.saved=__oryn_native_page_proxy({})",popup.page_id().0)).unwrap();
        assert_eq!(source.entered().evaluate_string("String(saved.closed)").unwrap(),"false");
        context.install_document_security(popup.page_id(),"https://b.test/".into(),&[]).unwrap();
        let mut target=host(&context,popup.page_id());
        target.entered().execute("globalThis.deliveries=0;addEventListener('message',()=>deliveries++)").unwrap();
        assert_eq!(source.entered().evaluate_string("String(saved.closed)").unwrap(),"true");
        assert!(context.native_window_group(popup.page_id(),popup.page_id(),None).unwrap().2,"session target is still live");
        assert_eq!(source.entered().evaluate_string("(()=>{const bytes=new ArrayBuffer(4);let reads=0;saved.postMessage({get bytes(){reads++;return bytes}},'*',[bytes]);return String(reads===1&&bytes.byteLength===0)})()").unwrap(),"true");
        // Poll the exact already-installed destination before any replacement
        // could discard an incorrectly queued predecessor message.
        let request={let target=target.entered();target.binding_states[&target.active_realm()].scoped_message_receive(MessageRequest::PollWindow{target:WindowTarget::Root,max:32}).unwrap()};
        let response=context.handle_message_wire(popup.page_id(),request).unwrap();
        assert!(matches!(response,MessageResponse::WindowMessages(messages)if messages.is_empty()),"the severed author proxy cannot send into a live session window");
        checkpoint(&mut target);assert_eq!(target.entered().evaluate_string("String(deliveries)").unwrap(),"0");
        popup.close().await.unwrap();parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn reply_to_frame_source_targets_native_frame_lifetime_not_top_level(){
        let context=crate::Browser::new().new_context();let parent=context.new_page();let popup=context.new_popup(parent.page_id(),None);
        let mut source=host(&context,parent.page_id());let mut target=host(&context,popup.page_id());let root=source.entered().active_realm();
        source.entered().execute("globalThis.rootMessages=0;addEventListener('message',()=>rootMessages++);const f=document.createElement('iframe');document.body.appendChild(f);globalThis.frame=f;void f.contentWindow").unwrap();
        checkpoint(&mut source);
        let (token,frame_realm)={let source=source.entered();let frame=source.isolate.get_slot::<FrameLifetimes>().unwrap().live.values().next().unwrap();(frame.message_token.clone(),frame.environment.get().realm)};
        source.entered().activate_realm(frame_realm).unwrap();
        source.entered().execute(&format!("globalThis.remote=__oryn_native_page_proxy({});globalThis.reply='';addEventListener('message',e=>reply=e.data);remote.postMessage('frame','*')",popup.page_id().0)).unwrap();
        target.entered().execute("globalThis.sourceWasFrame=false;addEventListener('message',e=>{sourceWasFrame=e.source!==opener;e.source.postMessage('reply','*')})").unwrap();
        checkpoint(&mut target);checkpoint(&mut source);
        assert_eq!(source.entered().evaluate_string("reply").unwrap(),"reply");source.entered().activate_realm(root).unwrap();
        assert_eq!(source.entered().evaluate_string("String(rootMessages)").unwrap(),"0");
        source.entered().execute("frame.remove()").unwrap();checkpoint(&mut source);
        assert!(matches!(SharedContextClient::direct(context.clone(),popup.page_id()).request(ContextServiceRequest::MessageWire{source_page_id:popup.page_id(),request:MessageRequest::WindowStatus{page:parent.page_id().0,target:WindowTarget::Frame(token)}}).unwrap(),ContextServiceResponse::MessageWire{result:Ok(MessageResponse::WindowLive(false))}));
        popup.close().await.unwrap();parent.close().await.unwrap();
    }
}


#[cfg(test)]
mod realm_endpoint_poll_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use oryn_js::worker_wire::Creator;

    struct PollTransport {
        route: crate::worker_route::WorkerRoute,
        page: PageId,
        creator: Creator,
        polls: AtomicUsize,
        registrations: AtomicUsize,
        reject_root_registration: bool,
    }
    impl crate::context_service::ContextTransport for PollTransport {
        fn request_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            request: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, crate::RuntimeError> {
            if route != Some(&self.route) {
                return Err(crate::RuntimeError::ProtocolViolation(
                    "endpoint poll route authority mismatch".into(),
                ));
            }
            self.request(request)
        }
        fn request(&self, request: ContextServiceRequest) -> Result<ContextServiceResponse, crate::RuntimeError> {
            let ContextServiceRequest::MessageWire { source_page_id, request } = request else {
                return Err(crate::RuntimeError::ContextService("unexpected endpoint poll request".into()));
            };
            if source_page_id != self.page {
                return Err(crate::RuntimeError::ContextService("wrong endpoint poll page".into()));
            }
            let Creator::Window { target, document_generation, stream_epoch } = &self.creator else { unreachable!() };
            let result = match request {
                MessageRequest::RegisterRootDocument { document_generation: generation, stream_epoch: epoch }
                    if *target == WindowTarget::Root && (generation, epoch) == (*document_generation, *stream_epoch) => {
                    self.registrations.fetch_add(1, Ordering::SeqCst);
                    if self.reject_root_registration { Err("fixture root registration denied".into()) } else { Ok(MessageResponse::Done) }
                }
                MessageRequest::RegisterWindow { frame, document_generation: generation, stream_epoch: epoch, .. }
                    if *target == WindowTarget::Frame(frame.clone()) && (generation, epoch) == (*document_generation, *stream_epoch) => Ok(MessageResponse::Done),
                MessageRequest::InDocument { target: owner, document_generation: generation, stream_epoch: epoch, request }
                    if owner == *target && (generation, epoch) == (*document_generation, *stream_epoch) => {
                    match *request {
                        MessageRequest::PollWindow { target: destination, max: 1 } if destination == *target => {
                            self.polls.fetch_add(1, Ordering::SeqCst);
                            Ok(MessageResponse::WindowMessages(Vec::new()))
                        }
                        _ => Err("unexpected scoped endpoint poll request".into()),
                    }
                }
                MessageRequest::RetireWindow { frame, document_generation: generation, stream_epoch: epoch }
                    if *target == WindowTarget::Frame(frame.clone()) && (generation, epoch) == (*document_generation, *stream_epoch) => Ok(MessageResponse::Done),
                MessageRequest::RetireRootDocument { document_generation: generation, stream_epoch: epoch }
                    if *target == WindowTarget::Root && (generation, epoch) == (*document_generation, *stream_epoch) => Ok(MessageResponse::Done),
                _ => Err("endpoint poll creator mismatch".into()),
            };
            Ok(ContextServiceResponse::MessageWire { result })
        }
    }
    fn local_host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(), 256 * 1024 * 1024);
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())), 1).unwrap();
        host
    }
    fn client(transport: Arc<PollTransport>) -> SharedContextClient {
        let route = transport.route.clone();
        SharedContextClient::for_test_transport(transport, route)
    }
    #[test]
    fn endpoint_poll_fixture_rejects_missing_and_foreign_route_grants() {
        use crate::context_service::ContextTransport;
        let transport = PollTransport {
            route: crate::worker_route::WorkerRoute::issue(814, 3, 0).unwrap(),
            page: PageId(3),
            creator: Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            polls: AtomicUsize::new(0),
            registrations: AtomicUsize::new(0),
            reject_root_registration: false,
        };
        let foreign = crate::worker_route::WorkerRoute::issue(814, 3, 0).unwrap();
        let request = || ContextServiceRequest::MessageWire {
            source_page_id: PageId(3),
            request: MessageRequest::RegisterRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
        };
        for route in [None, Some(&foreign)] {
            assert!(transport.request_routed(route, request()).is_err());
        }
        assert_eq!(transport.registrations.load(Ordering::SeqCst), 0);
        assert!(matches!(
            transport.request_routed(Some(&transport.route), request()).unwrap(),
            ContextServiceResponse::MessageWire { result: Ok(MessageResponse::Done) }
        ));
        assert_eq!(transport.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(transport.polls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn mixed_endpoint_poll_uses_each_realm_and_ignores_active_realm_selection() {
        let mut host = local_host();
        let root = host.active_realm();
        host.execute("globalThis.child=document.createElement('iframe');document.body.append(child)").unwrap();
        host.adopt_created_realms();
        let child = host.realm_ids().into_iter().find(|realm| *realm != root).unwrap();
        let (token, generation, epoch) = {
            v8::scope!(let scope, &mut host.isolate);
            let context = v8::Local::new(scope, &host.contexts[&child]);
            let frame = context.get_slot::<FrameLifetime>().unwrap();
            let state = context.get_slot::<V8BindingState>().unwrap();
            (frame.message_token.clone(), state.document_generation(), state.document_authority_epoch())
        };
        let transport = Arc::new(PollTransport { route: crate::worker_route::WorkerRoute::issue(814, 1, 0).unwrap(), page: PageId(1), creator: Creator::Window { target: WindowTarget::Frame(token), document_generation: generation, stream_epoch: epoch }, polls: AtomicUsize::new(0), registrations: AtomicUsize::new(0), reject_root_registration: false });
        host.install_test_resource_endpoint(child, client(transport.clone()), PageId(1));
        assert!(host.context_endpoint_for_realm(root).is_none());
        for active in [root, child, root] {
            host.activate_realm(active).unwrap();
            let before = transport.polls.load(Ordering::SeqCst);
            assert_eq!(host.poll_remote_message_tasks().unwrap(), 0);
            assert_eq!(transport.polls.load(Ordering::SeqCst), before + 1);
            assert_eq!(host.active_realm(), active);
            assert_eq!(transport.registrations.load(Ordering::SeqCst), 0, "the disconnected root must never register with the child's endpoint");
        }
        host.activate_realm(root).unwrap();
        assert_eq!(host.evaluate_string("String(child.contentWindow.parent===window)").unwrap(), "true");
    }
    #[test]
    fn connected_root_poll_keeps_missing_and_rejected_creator_registration_fail_closed() {
        for install_messages in [false, true] {
            let mut host = local_host();
            let root = host.active_realm();
            let state = host.binding_states[&root].clone();
            let transport = Arc::new(PollTransport { route: crate::worker_route::WorkerRoute::issue(814, 2, 0).unwrap(), page: PageId(2), creator: state.native_local_creator_identity().unwrap(), polls: AtomicUsize::new(0), registrations: AtomicUsize::new(0), reject_root_registration: true });
            let endpoint = client(transport.clone());
            if install_messages {
                host.install_test_resource_endpoint(root, endpoint, PageId(2));
            } else {
                // Deliberately incomplete connected fixture: presence of an
                // endpoint must not hide missing lifetime-registration transport.
                host.isolate.get_slot_mut::<RealmEndpointRegistry>().unwrap().0.get_mut(&root).unwrap().1.context = Some(ContextState { client: endpoint, page_id: PageId(2) });
            }
            let error = host.poll_remote_message_tasks().unwrap_err().to_string();
            assert!(error.contains("native creator lifetime registration"), "{error}");
            assert_eq!(error.contains("fixture root registration denied"), install_messages);
            assert_eq!(transport.polls.load(Ordering::SeqCst), 0);
            assert_eq!(transport.registrations.load(Ordering::SeqCst), usize::from(install_messages));
            assert_eq!(host.active_realm(), root);
        }
    }
}
