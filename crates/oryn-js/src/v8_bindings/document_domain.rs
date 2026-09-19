//! Document-owned DOM access domain, separate from strict origin authority.
use super::*;

pub type AuthorizeDomainChange = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    &Rc<V8BindingState>,
    &str,
) -> Result<Option<String>, V8BindingError>;

pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let candidate = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    if document != state.document_root()
        || state.execution_retired()
        || state.binding_global != BindingGlobal::Window
    {
        return Err(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "SecurityError".into(),
        });
    }
    // Ensure storage exists before the native authorizer publishes a token.
    if state
        .dom
        .borrow()
        .arena()
        .get(document)?
        .document_metadata
        .is_none()
    {
        return Err(V8BindingError::Allocation("document domain metadata"));
    }
    let authorize = state
        .document_domain_authorizer
        .get()
        .ok_or(V8BindingError::Allocation(
            "native document domain authority",
        ))?;
    if let Some(domain) = authorize(scope, state, &candidate)? {
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(document)?
            .document_metadata
            .as_mut()
            .ok_or(V8BindingError::Allocation("document domain metadata"))?
            .relaxed_domain
            .set(Some(domain));
    }
    Ok(())
}

/// document.open uses strict origin equality even when DOM access was granted
/// through bilateral domain relaxation. Never compare the V8 access token here.
pub(super) fn require_strict_origin(
    scope: &mut v8::PinScope,
    target: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<(), V8BindingError> {
    let denied = || V8BindingError::Unsupported {
        interface: "Document".into(),
        member: "SecurityError".into(),
    };
    let caller =
        crate::v8_context_bridge::incumbent_slot::<V8BindingState>(scope).ok_or_else(denied)?;
    let source = caller.document_provenance(caller.document_root())?;
    let destination = target.document_provenance(document)?;
    let same = match (source, destination) {
        (Some(source), Some(destination)) => source.origin_key == destination.origin_key,
        (None, None) if Rc::ptr_eq(&caller, target) => true,
        (None, None) => {
            let source = crate::origin::Origin::from_url(&caller.current_location_url());
            let destination = crate::origin::Origin::from_url(&target.current_location_url());
            if !source.is_opaque() && !destination.is_opaque() {
                source == destination
            } else {
                // Bare binding embedders may inherit about:blank through a
                // native token before installing document provenance. Such a
                // token is strict only while neither Document has relaxed.
                let relaxed = caller
                    .dom
                    .borrow()
                    .arena()
                    .get(caller.document_root())?
                    .document_metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.relaxed_domain.is_some())
                    || target
                        .dom
                        .borrow()
                        .arena()
                        .get(document)?
                        .document_metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.relaxed_domain.is_some());
                !relaxed
                    && target.global_object.to_local(scope).is_some_and(|window| {
                        crate::v8_context_bridge::can_access_window(scope, window)
                    })
            }
        }
        _ => false,
    };
    if same { Ok(()) } else { Err(denied()) }
}

pub(super) fn get(state: &V8BindingState, document: NodeId) -> Result<String, V8BindingError> {
    if let Some(domain) = state
        .dom
        .borrow()
        .arena()
        .get(document)?
        .document_metadata
        .as_ref()
        .and_then(|metadata| metadata.relaxed_domain.get())
    {
        return Ok(domain);
    }
    let origin = state
        .document_provenance(document)?
        .map(|source| source.origin.clone())
        .unwrap_or_else(|| state.current_location_url());
    Ok(
        match url::Url::parse(&origin).ok().map(|url| url.origin()) {
            Some(url::Origin::Tuple(_, host, _)) => host.to_string(),
            _ => String::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relaxed_dom_access_does_not_authorize_cross_origin_document_open() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_context = v8::Context::new(scope, Default::default());
        let target_context = v8::Context::new(scope, Default::default());
        let install = |scope: &mut v8::PinScope<'_, '_, ()>,
                       context: v8::Local<v8::Context>,
                       realm: RealmId,
                       url: &str,
                       key: &str| {
            let context = v8::Local::new(scope, context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>Retained</body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom.clone(), realm, 1).unwrap();
            state.commit_location(url).unwrap();
            state
                .install_document_provenance(oryn_dom::DocumentProvenance {
                    capability: vec![1],
                    origin: url.into(),
                    origin_key: key.into(),
                    inert: false,
                })
                .unwrap();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(state.document_root())
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .relaxed_domain
                .set(Some("example.test".into()));
            state
        };
        let _source = install(
            scope,
            source_context,
            RealmId(0),
            "https://a.example.test",
            "strict-a",
        );
        let target = install(
            scope,
            target_context,
            RealmId(1),
            "https://b.example.test",
            "strict-b",
        );
        let token = v8::String::new(scope, "shared-domain-access-only").unwrap();
        source_context.set_security_token(token.into());
        target_context.set_security_token(token.into());
        let scope = &mut v8::ContextScope::new(scope, source_context);
        let document = wrap_node(scope, &target, target.document_root()).unwrap();
        let key = v8::String::new(scope, "foreignDocument").unwrap();
        source_context
            .global(scope)
            .set(scope, key.into(), document.into());
        let script=v8::String::new(scope,r#"(()=>{
            foreignDocument.body.textContent='Visible through relaxed DOM access';
            const original=foreignDocument.documentElement;
            try{foreignDocument.open();return false}catch(e){if(e.name!=='SecurityError')return false}
            return foreignDocument.documentElement===original&&foreignDocument.body.textContent==='Visible through relaxed DOM access';
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn setter_converts_once_rejects_inert_documents_and_preserves_ignored_or_denied_state() {
        fn authorize(
            _scope: &mut v8::PinScope,
            _state: &Rc<V8BindingState>,
            candidate: &str,
        ) -> Result<Option<String>, V8BindingError> {
            match candidate {
                "ignored" => Ok(None),
                "example.test" => Ok(Some(candidate.into())),
                _ => Err(V8BindingError::Unsupported {
                    interface: "Document".into(),
                    member: "SecurityError".into(),
                }),
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state
            .commit_location("https://sub.example.test/path")
            .unwrap();
        state.set_document_domain_authorizer(authorize);
        let script=v8::String::new(scope,r#"(()=>{
            let conversions=0;document.domain={toString(){conversions++;return 'example.test'}};
            if(conversions!==1||document.domain!=='example.test')return false;
            document.domain='ignored';if(document.domain!=='example.test')return false;
            try{document.domain='denied';return false}catch(e){if(e.name!=='SecurityError')return false}
            const inert=new Document();try{inert.domain='example.test';return false}catch(e){if(e.name!=='SecurityError')return false}
            return document.domain==='example.test'&&document.URL==='https://sub.example.test/path';
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn domain_getter_uses_document_origin_without_changing_serialized_authority() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state
            .commit_location("https://sub.example.test:8443/path")
            .unwrap();
        state
            .install_document_provenance(oryn_dom::DocumentProvenance {
                capability: vec![1],
                origin: "https://sub.example.test:8443".into(),
                origin_key: "strict-native-identity".into(),
                inert: false,
            })
            .unwrap();
        assert_eq!(
            get(&state, state.document_root()).unwrap(),
            "sub.example.test"
        );
        dom.borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .relaxed_domain
            .set(Some("example.test".into()));
        assert_eq!(get(&state, state.document_root()).unwrap(), "example.test");
        let source = state
            .document_provenance(state.document_root())
            .unwrap()
            .unwrap();
        assert_eq!(source.origin, "https://sub.example.test:8443");
        assert_eq!(source.origin_key, "strict-native-identity");
        assert_eq!(
            state.current_location_url(),
            "https://sub.example.test:8443/path"
        );
    }
}
