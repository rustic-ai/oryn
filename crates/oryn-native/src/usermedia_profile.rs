// Freeze native feature selection before generated realm constructors exist.
fn freeze_usermedia_profile(scope:&mut v8::PinScope,context:v8::Local<v8::Context>)->Result<(),HostError>{
    #[cfg(unix)] if let Some(endpoint)=current_context_endpoint(scope){
        let response=endpoint.client.request(ContextServiceRequest::WebFeatureProfile{page_id:endpoint.page_id}).map_err(|error|HostError::Execution(error.to_string()))?;
        let ContextServiceResponse::WebFeatureProfile{profile}=response else{return Err(HostError::Execution("native web feature profile response mismatch".into()))};
        context.set_slot(Rc::new(oryn_js::v8_bindings::UserMediaElementProfile{legacy:profile.usermedia_element==crate::feature_profile::UserMediaElementMode::Legacy}));return Ok(())
    }
    let profile=scope.get_slot::<oryn_js::v8_bindings::UserMediaElementProfile>().copied().unwrap_or_default();
    context.set_slot(Rc::new(profile));Ok(())
}
impl RawV8Host{
    /// Standalone hosts select features before installing any document bindings.
    /// Context-backed hosts always obtain their immutable BrowserContext profile.
    pub fn set_web_feature_profile(&mut self,profile:crate::feature_profile::NativeFeatureProfile)->Result<(),HostError>{
        if !self.binding_states.is_empty(){return Err(HostError::Execution("web feature profile must precede document bindings".into()))}
        self.isolate.set_slot(oryn_js::v8_bindings::UserMediaElementProfile{legacy:profile.usermedia_element==crate::feature_profile::UserMediaElementMode::Legacy});Ok(())
    }
}
#[cfg(test)]mod usermedia_profile_tests{
    use super::*;
    #[test]fn standalone_profile_is_selected_before_bindings_and_cannot_change_live_realms(){
        for legacy in [false,true]{let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);
            let profile=crate::feature_profile::NativeFeatureProfile{usermedia_element:if legacy{crate::feature_profile::UserMediaElementMode::Legacy}else{crate::feature_profile::UserMediaElementMode::Standard}};
            host.set_web_feature_profile(profile).unwrap();let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<usermedia id=u type=camera></usermedia>").into_runtime().unwrap()));host.install_dom(dom,1).unwrap();
            assert_eq!(host.evaluate_string("String(typeof HTMLUserMediaElement.isTypeSupported==='function')").unwrap(),legacy.to_string());
            assert!(host.set_web_feature_profile(crate::feature_profile::NativeFeatureProfile::default()).is_err());
        }
    }
}
