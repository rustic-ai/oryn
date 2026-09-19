// Standalone embedders have no context RPC. Their isolate owns the signer and
// immutable policy records; these capabilities never validate in a browser parent.
#[derive(serde::Serialize,serde::Deserialize)]
struct LocalDocumentSource {version:u8,#[serde(default)]relaxed_domain:Option<String>,origin:String,origin_key:String,url:String,referrer:String,inert:bool,realm:u32,ancestors:Vec<u32>,policy:crate::csp::CspPolicySet}
#[derive(serde::Serialize,serde::Deserialize)]
struct LocalDocumentSeal {payload:Vec<u8>,mac:[u8;32]}
struct LocalDocumentSigner([u8;32]);
struct LocalDocumentProvenanceInstalled;
#[derive(Default)]struct LocalOriginClusters(BTreeMap<String,bool>);
struct LocalOriginCluster(bool);
fn admit_local_origin_cluster(scope:&mut v8::PinScope,context:v8::Local<v8::Context>,state:&Rc<V8BindingState>,headers:&[(String,String)])->Result<(),HostError>{
    #[cfg(unix)] if endpoints_for_context(scope,context).and_then(|endpoints|endpoints.context).is_some(){return Ok(())}
    if let Some(admitted)=context.get_slot::<LocalOriginCluster>(){state.set_origin_agent_cluster(admitted.0);return Ok(())}
    let origin=context.get_slot::<RealmDescriptor>().ok_or_else(||HostError::Execution("standalone cluster origin missing".into()))?.origin.clone();
    let secure=context.get_slot::<oryn_js::realm::SecureContextState>().is_some_and(|value|value.0);
    let isolation=crate::agent_isolation::DocumentIsolationPolicy::from_headers(headers);
    let mut fields=headers.iter().filter(|(name,_)|name.eq_ignore_ascii_case("origin-agent-cluster"));
    let requested=fields.next().is_some_and(|(_,value)|value.trim_start_matches(' ').strip_prefix("?1").is_some_and(|tail|crate::agent_isolation::structured_token(&format!("enabled{tail}"))==Some("enabled")))&&fields.next().is_none();
    if scope.get_slot::<LocalOriginClusters>().is_none(){scope.set_slot(LocalOriginClusters::default());}
    let registry=scope.get_slot_mut::<LocalOriginClusters>().expect("installed cluster registry");
    let key=origin.security_token_key();
    if registry.0.len()>=4096&&!registry.0.contains_key(&key){return Err(HostError::Execution("standalone origin cluster capacity".into()))}
    let keyed=origin.is_opaque()||isolation.top_level_isolated(secure,true)||*registry.0.entry(key).or_insert(secure&&requested);
    context.set_slot(Rc::new(LocalOriginCluster(keyed)));state.set_origin_agent_cluster(keyed);Ok(())
}

impl LocalDocumentSigner {
    fn mac(&self,payload:&[u8])->[u8;32]{
        use sha2::{Digest,Sha256};let mut inner=[0x36u8;64];let mut outer=[0x5cu8;64];for(index,byte)in self.0.iter().enumerate(){inner[index]^=byte;outer[index]^=byte;}
        let mut hash=Sha256::new();hash.update(inner);hash.update(b"oryn.standalone-document-source.v1\0");hash.update(payload);let digest=hash.finalize();let mut hash=Sha256::new();hash.update(outer);hash.update(digest);hash.finalize().into()
    }
    fn seal(&self,source:&LocalDocumentSource)->Result<oryn_dom::DocumentProvenance,String>{let payload=serde_json::to_vec(source).map_err(|error|error.to_string())?;if payload.len()>65536{return Err("standalone document provenance capacity".into());}let capability=serde_json::to_vec(&LocalDocumentSeal{mac:self.mac(&payload),payload}).map_err(|error|error.to_string())?;if capability.len()>262144{return Err("sealed standalone provenance capacity".into());}Ok(oryn_dom::DocumentProvenance{capability,origin:source.origin.clone(),origin_key:source.origin_key.clone(),inert:source.inert})}
    fn open(&self,source:&oryn_dom::DocumentProvenance)->Result<LocalDocumentSource,String>{if source.capability.len()>262144{return Err("standalone provenance capacity".into());}let sealed:LocalDocumentSeal=serde_json::from_slice(&source.capability).map_err(|error|error.to_string())?;let expected=self.mac(&sealed.payload);let mut difference=0;for(a,b)in expected.iter().zip(sealed.mac){difference|=*a^b;}if difference!=0{return Err("invalid standalone document capability".into());}let value:LocalDocumentSource=serde_json::from_slice(&sealed.payload).map_err(|error|error.to_string())?;if value.version!=1||value.origin!=source.origin||value.origin_key!=source.origin_key||value.inert!=source.inert{return Err("invalid standalone document metadata".into());}Ok(value)}
}
/// Only native inherited-origin creation calls this. A response-created same-
/// origin Document intentionally receives a separate mutable domain identity.
struct LocalDomainAccess(Rc<RefCell<oryn_js::origin::DomainAccessIdentity>>,oryn_dom::DocumentDomainCell);
fn inherit_local_document_domain(creator:v8::Local<v8::Context>,child:v8::Local<v8::Context>){
    let(Some(parent),Some(target))=(creator.get_slot::<RealmDescriptor>(),child.get_slot::<RealmDescriptor>())else{return};
    if parent.origin!=target.origin{return} // Opaque sandbox replacement is distinct.
    if let Some(access)=creator.get_slot::<LocalDomainAccess>(){if access.0.borrow().origin()==&target.origin{child.set_slot(access);}}
}
fn authorize_local_document_domain(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,candidate:&str)->Result<Option<String>,oryn_js::v8_bindings::V8BindingError>{
    use oryn_js::v8_bindings::V8BindingError;
    let denied=||V8BindingError::Unsupported{interface:"Document".into(),member:"SecurityError".into()};
    let weak=scope.get_slot::<NativeRealmIdentities>().and_then(|registry|registry.contexts.get(&state.realm())).cloned().ok_or_else(denied)?;
    let context=weak.to_local(scope).ok_or_else(denied)?;
    if !context.get_slot::<V8BindingState>().is_some_and(|owner|Rc::ptr_eq(&owner,state))||state.execution_retired(){return Err(denied())}
    let access=context.get_slot::<LocalDomainAccess>().ok_or_else(denied)?;
    if access.0.borrow().origin().is_opaque()||context.get_slot::<RefCell<crate::csp::CspPolicySet>>().is_some_and(|policy|policy.borrow().sandbox().is_some()){return Err(denied())}
    if !state.native_permissions_policy_allows("document-domain"){return Err(denied())}
    if state.origin_agent_cluster(){return Ok(None)}
    let mut next=access.0.borrow().clone();next.set_domain(candidate).map_err(|_|denied())?;
    let token_key=next.security_token_key();let domain=next.domain();
    let token=if let Some(token)=scope.get_slot::<NativeRealmIdentities>().and_then(|registry|registry.security_tokens.get(&token_key)).cloned(){v8::Local::new(scope,&token)}else{
        let token=v8::String::new(scope,&token_key).ok_or(V8BindingError::Allocation("standalone domain token"))?;let global=v8::Global::new(scope,token);scope.get_slot_mut::<NativeRealmIdentities>().ok_or_else(denied)?.security_tokens.insert(token_key,global);token
    };
    let contexts=scope.get_slot::<NativeRealmIdentities>().ok_or_else(denied)?.contexts.values().cloned().collect::<Vec<_>>();let mut recipients=Vec::new();
    for weak in contexts{let Some(context)=weak.to_local(scope)else{continue};let Some(alias)=context.get_slot::<LocalDomainAccess>()else{continue};if !Rc::ptr_eq(&alias.0,&access.0){continue}let Some(owner)=context.get_slot::<V8BindingState>()else{continue};if owner.execution_retired(){continue}
        if owner.dom().borrow().arena().get(owner.document_root())?.document_metadata.is_none(){return Err(V8BindingError::Allocation("standalone alias metadata"))}
        recipients.push((context,owner,domain.clone()));
    }
    // No author callback or fallible V8 allocation follows this publication.
    *access.0.borrow_mut()=next;
    for(context,owner,domain)in recipients{owner.dom().borrow_mut().arena_mut().get_mut(owner.document_root()).expect("preflighted alias document").document_metadata.as_mut().expect("preflighted alias metadata").relaxed_domain.set(Some(domain));context.set_security_token(token.into());}
    Ok(Some(domain))
}
fn install_local_document_provenance(scope:&mut v8::PinScope,context:v8::Local<v8::Context>,state:&Rc<V8BindingState>)->Result<(),HostError>{
    #[cfg(unix)] if endpoints_for_context(scope,context).and_then(|endpoints|endpoints.context).is_some(){return Ok(());}
    if state.has_document_provenance_capture()&&context.get_slot::<LocalDocumentProvenanceInstalled>().is_none(){return Ok(());}
    context.set_slot(Rc::new(LocalDocumentProvenanceInstalled));
    let signer=if let Some(signer)=scope.get_slot::<Rc<LocalDocumentSigner>>(){signer.clone()}else{let mut key=[0;32];getrandom::fill(&mut key).map_err(|error|HostError::Execution(error.to_string()))?;let signer=Rc::new(LocalDocumentSigner(key));scope.set_slot(signer.clone());signer};
    let origin=context.get_slot::<RealmDescriptor>().ok_or_else(||HostError::Execution("standalone document origin missing".into()))?.origin.clone();
    let policy=context.get_slot::<RefCell<crate::csp::CspPolicySet>>();
    let access=if let Some(access)=context.get_slot::<LocalDomainAccess>().filter(|access|access.0.borrow().origin()==&origin){access}else{let access=Rc::new(LocalDomainAccess(Rc::new(RefCell::new(oryn_js::origin::DomainAccessIdentity::new(origin.clone()))),oryn_dom::DocumentDomainCell::default()));context.set_slot(access.clone());access};
    let isolated=context.get_slot::<crate::agent_isolation::DocumentIsolationPolicy>().is_some_and(|policy|policy.top_level_isolated(context.get_slot::<oryn_js::realm::SecureContextState>().is_some_and(|state|state.0),true));
    state.set_origin_agent_cluster(context.get_slot::<LocalOriginCluster>().map_or(origin.is_opaque()||isolated,|admitted|admitted.0));
    state.set_document_domain_authorizer(authorize_local_document_domain);
    state.dom().borrow_mut().arena_mut().get_mut(state.document_root()).map_err(|error|HostError::Execution(error.to_string()))?.document_metadata.as_mut().ok_or_else(||HostError::Execution("standalone domain metadata missing".into()))?.relaxed_domain=access.1.clone();
    if let Some(domain)=access.0.borrow().relaxed_domain(){
        let key=access.0.borrow().security_token_key();
        let token=if let Some(token)=scope.get_slot::<NativeRealmIdentities>().and_then(|registry|registry.security_tokens.get(&key)).cloned(){v8::Local::new(scope,&token)}else{let token=v8::String::new(scope,&key).ok_or_else(||HostError::Execution("standalone inherited domain token".into()))?;let global=v8::Global::new(scope,token);scope.get_slot_mut::<NativeRealmIdentities>().ok_or_else(||HostError::Execution("standalone realm registry".into()))?.security_tokens.insert(key,global);token};
        state.dom().borrow_mut().arena_mut().get_mut(state.document_root()).map_err(|error|HostError::Execution(error.to_string()))?.document_metadata.as_mut().ok_or_else(||HostError::Execution("standalone domain metadata missing".into()))?.relaxed_domain.set(Some(domain.to_owned()));context.set_security_token(token.into());
    }

    state.set_document_provenance_capture(Rc::new(move|state,source,url|{
        let record=if let Some(source)=source{let mut record=signer.open(source)?;if url!="about:blank"&&url!=record.url{return Err("inert document URL differs from source".into());}record.inert=true;record.url=url.into();record.referrer=crate::referrer::ReferrerPolicy::default().as_str().into();record.policy=crate::csp::CspPolicySet::new(url);record}
            else{LocalDocumentSource{version:1,relaxed_domain:access.0.borrow().relaxed_domain().map(str::to_owned),origin:origin.serialize(),origin_key:origin.security_token_key(),url:url.into(),referrer:state.document_referrer_policy(),inert:false,realm:state.realm().0,ancestors:state.document_source_ancestors(),policy:policy.as_ref().map(|policy|policy.borrow().clone()).unwrap_or_else(||crate::csp::CspPolicySet::new(url))}};
        signer.seal(&record)
    }));Ok(())
}
impl RawV8Host {
    pub fn local_document_form_allowed(&self,source:&oryn_dom::DocumentProvenance,url:&str,target:RealmId)->Result<bool,HostError>{
        let signer=self.isolate.get_slot::<Rc<LocalDocumentSigner>>().ok_or_else(||HostError::Execution("standalone document signer missing".into()))?;
        let source=signer.open(source).map_err(HostError::Execution)?;
        if source.policy.sandbox().is_some_and(|flags|!flags.allow_forms){return Ok(false);}
        if !source.policy.authorize(&crate::csp::CspRequest{destination:crate::csp::CspDestination::Form,url:url.into(),nonce:None,inline_source:None,parser_inserted:false,initiated_by_trusted_script:false}).allowed{return Ok(false);}
        if source.inert||source.realm==target.0{return Ok(true);}
        let target_state=self.binding_states.get(&target).ok_or_else(||HostError::Execution("standalone navigation target missing".into()))?;
        if target_state.document_source_ancestors().contains(&source.realm){return Ok(true);}
        if source.ancestors.contains(&target.0){if !target_state.document_source_ancestors().is_empty(){return Ok(true);}return Ok(source.policy.sandbox().is_none_or(|flags|flags.allow_top_navigation));}
        Ok(source.policy.sandbox().is_none())
    }
}

fn prepare_local_document_policy(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,policy:&crate::csp::CspPolicySet)->Result<Option<oryn_dom::DocumentProvenance>,HostError>{
    let Some(signer)=scope.get_slot::<Rc<LocalDocumentSigner>>()else{return Ok(None);};
    let Some(source)=state.document_provenance(state.document_root()).map_err(|error|HostError::Execution(error.to_string()))?else{return Ok(None);};
    let mut record=signer.open(&source).map_err(HostError::Execution)?;record.policy=policy.clone();Ok(Some(signer.seal(&record).map_err(HostError::Execution)?))
}

#[cfg(test)]
mod local_document_source_tests {
    use super::*;
    #[test]
    fn standalone_initial_frames_inherit_relaxed_tokens_and_share_domain_updates(){
        let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://www.example.test/").unwrap(),256*1024*1024);
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())),1).unwrap();
        assert_eq!(host.evaluate_string(r#"(()=>{
            globalThis.beforeRelaxation=new Document();document.domain='www.example.test';
            const blank=document.createElement('iframe');document.body.append(blank);
            const srcdoc=document.createElement('iframe');srcdoc.srcdoc='<body>srcdoc</body>';document.body.append(srcdoc);
            if(blank.contentDocument.domain!=='www.example.test'||srcdoc.contentDocument.domain!=='www.example.test')return 'initial token failed';
            blank.contentDocument.domain='example.test';
            return [document.domain,blank.contentDocument.domain,srcdoc.contentDocument.domain,originAgentCluster].join(',');
        })()"#).unwrap(),"example.test,example.test,example.test,false");
        assert_eq!(host.evaluate_string("[beforeRelaxation.domain,document.implementation.createHTMLDocument().domain,document.cloneNode(false).domain].join(',')").unwrap(),"example.test,example.test,example.test");
    }
    #[test]
    fn local_document_tokens_bind_origin_policy_and_isolate_signer(){
        let signer=LocalDocumentSigner([7;32]);let other=LocalDocumentSigner([8;32]);
        let mut policy=crate::csp::CspPolicySet::new("https://source.test/");policy.install_meta("form-action 'none'");
        let record=LocalDocumentSource{version:1,relaxed_domain:Some("source.test".into()),origin:"https://source.test".into(),origin_key:"native-source-key".into(),url:"https://source.test/".into(),referrer:"origin".into(),inert:false,realm:2,ancestors:vec![0],policy};
        let token=signer.seal(&record).unwrap();assert_eq!(signer.open(&token).unwrap().ancestors,vec![0]);assert!(other.open(&token).is_err());
        let mut forged=token.clone();forged.origin_key="other-origin".into();assert!(signer.open(&forged).is_err());
        let mut forged=token;forged.inert=true;assert!(signer.open(&forged).is_err());
    }
}
