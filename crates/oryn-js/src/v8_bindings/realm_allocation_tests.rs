//! Allocation context is independent of the calling realm and mutable WindowProxy.
use super::*;
fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    super::test_eval::eval(scope, source)
}
fn runtime(markup: &str) -> SharedDom {
    Rc::new(RefCell::new(
        oryn_dom::html::parse(markup).into_runtime().unwrap(),
    ))
}

#[test]
fn foreign_html_base_uses_its_registry_and_document_for_direct_and_parsed_construction() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let handles,&mut isolate);
    let parent = v8::Context::new(handles, Default::default());
    let child = v8::Context::new(handles, Default::default());
    let token = v8::String::new(handles, "constructor-realms").unwrap();
    parent.set_security_token(token.into());
    child.set_security_token(token.into());
    {
        let scope = &mut v8::ContextScope::new(handles, parent);
        install_generated_bindings(scope, parent, runtime("<body></body>"), RealmId(950), 1)
            .unwrap();
    }
    {
        let scope = &mut v8::ContextScope::new(handles, child);
        install_generated_bindings(
            scope,
            child,
            runtime("<body><x-parsed-foreign></x-parsed-foreign></body>"),
            RealmId(951),
            1,
        )
        .unwrap();
    }
    let scope = &mut v8::ContextScope::new(handles, parent);
    let global = parent.global(scope);
    let foreign = child.global(scope);
    set_object_property(scope, global, "foreign", foreign.into()).unwrap();
    assert!(eval(scope,r#"(()=>{
        const log=[];class Foreign extends foreign.HTMLElement{constructor(){super();this.i=this.attachInternals();this.marker=23}connectedCallback(){log.push(this.ownerDocument===foreign.document)}disconnectedCallback(){log.push('removed')}}
        foreign.customElements.define('x-direct-foreign',Foreign);const element=new Foreign();foreign.document.body.append(element);element.remove();
        if(element.ownerDocument!==foreign.document||element.marker!==23||!(element.i instanceof foreign.ElementInternals)||log.join()!=='true,removed')return false;
        class Parsed extends foreign.HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals()}}
        foreign.customElements.define('x-parsed-foreign',Parsed);const parsed=foreign.document.querySelector('x-parsed-foreign');
        parsed.i.setFormValue('value');
        let mismatch=false;try{new(class extends HTMLElement{})()}catch(e){mismatch=e instanceof TypeError}
        return parsed instanceof Parsed&&parsed.ownerDocument===foreign.document&&parsed.i instanceof foreign.ElementInternals&&mismatch;
    })()"#).is_true());
}

#[test]
fn lazy_exception_keeps_its_original_realm_after_window_proxy_rebinding() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let handles,&mut isolate);
    let old = v8::Context::new(handles, Default::default());
    let (state, constructor) = {
        let scope = &mut v8::ContextScope::new(handles, old);
        let state =
            install_generated_bindings(scope, old, runtime("<body></body>"), RealmId(952), 1)
                .unwrap();
        let constructor = eval(scope, "DOMException");
        (state, constructor)
    };
    let proxy = old.global(handles);
    crate::v8_context_bridge::detach_global(handles, old);
    let next = v8::Context::new(
        handles,
        v8::ContextOptions {
            global_object: Some(proxy.into()),
            ..Default::default()
        },
    );
    let scope = &mut v8::ContextScope::new(handles, next);
    let _next =
        install_generated_bindings(scope, next, runtime("<body></body>"), RealmId(953), 2).unwrap();
    assert!(
        state
            .global_object
            .to_local(scope)
            .unwrap()
            .get_creation_context(scope)
            .unwrap()
            == next
    );
    let id = register_platform_object(
        &state,
        PlatformObjectRecord::DomException {
            name: "SecurityError".into(),
            message: "SecurityError".into(),
            code: 18,
        },
    );
    let exception = new_platform_object(scope, &state, "DOMException", id).unwrap();
    assert!(exception.get_creation_context(scope).unwrap() == old);
    let global = next.global(scope);
    set_object_property(scope, global, "originalException", constructor).unwrap();
    set_object_property(scope, global, "savedError", exception.into()).unwrap();
    assert!(eval(scope,"savedError instanceof originalException&&!(savedError instanceof DOMException)&&savedError.name==='SecurityError'").is_true());
}

#[test]
fn foreign_node_allocation_retains_exact_owner_through_adoption_then_collects() {
    super::tests::initialize_v8();
    for same_arena in [true, false] {
        let mut isolate = v8::Isolate::new(Default::default());
        let (source_context, target_context, source, target, object) = {
            v8::scope!(let handles,&mut isolate);
            let first = v8::Context::new(handles, Default::default());
            let second = v8::Context::new(handles, Default::default());
            let token = v8::String::new(handles, "node-realms").unwrap();
            first.set_security_token(token.into());
            second.set_security_token(token.into());
            let dom = runtime("<body><section><b>kept</b></section></body>");
            let target_dom = if same_arena {
                dom.clone()
            } else {
                runtime("<body></body>")
            };
            let target_document = if same_arena {
                let parsed = oryn_dom::html::parse("<body></body>");
                dom.borrow_mut()
                    .import_subtree(&parsed.dom, parsed.document)
                    .unwrap()
            } else {
                target_dom.borrow().document()
            };
            let source = {
                let scope = &mut v8::ContextScope::new(handles, first);
                install_generated_bindings(scope, first, dom, RealmId(954), 1).unwrap()
            };
            let target = {
                let scope = &mut v8::ContextScope::new(handles, second);
                install_generated_bindings_with_root(
                    scope,
                    second,
                    target_dom,
                    RealmId(955),
                    1,
                    target_document,
                )
                .unwrap()
            };
            let scope = &mut v8::ContextScope::new(handles, second);
            let node = source
                .dom
                .borrow()
                .query_selector(source.document_root(), "section")
                .unwrap()
                .unwrap();
            let object = wrap_node(scope, &source, node).unwrap();
            assert!(object.get_creation_context(scope).unwrap() == first);
            let dataset = associated_wrapper(scope, &source, node, "DOMStringMap")
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            assert!(dataset.get_creation_context(scope).unwrap() == first);
            let global = second.global(scope);
            set_object_property(scope, global, "kept", object.into()).unwrap();
            assert!(eval(scope,"kept.marker=81;document.adoptNode(kept)===kept&&kept.ownerDocument===document&&kept.firstChild.textContent==='kept'").is_true());
            let (owner, actual) = node_identity::argument(scope, object.into())
                .unwrap()
                .unwrap();
            assert_eq!(
                owner.dom.borrow().node_document(actual).unwrap(),
                Some(target_document)
            );
            assert!(Rc::ptr_eq(
                &owner,
                if same_arena { &source } else { &target }
            ));
            eval(scope, "delete globalThis.kept");
            // The host consumes native mutation journals after publication.
            // Their removal leases are independent roots until that checkpoint.
            for dom in [&source.dom, &target.dom] {
                let mut dom = dom.borrow_mut();
                let mutations = dom.drain_mutations();
                dom.release_mutation_roots(&mutations).unwrap();
            }
            source.retire_execution();
            source.refresh_connected_wrapper_roots(scope);
            target.retire_execution();
            target.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, first),
                v8::Weak::new(scope, second),
                Rc::downgrade(&source),
                Rc::downgrade(&target),
                v8::Global::new(scope, object),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = source_context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let object = v8::Local::new(scope, &object);
            let (owner, node) = node_identity::argument(scope, object.into())
                .unwrap()
                .unwrap();
            assert!(source.upgrade().is_some());
            assert_eq!(
                owner.dom.borrow().arena().text_content(node).unwrap(),
                "kept"
            );
            let marker = v8::String::new(scope, "marker").unwrap();
            assert_eq!(
                object.get(scope, marker.into()).unwrap().int32_value(scope),
                Some(81)
            );
        }
        drop(object);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles,&mut isolate);
        assert!(
            source_context.to_local(handles).is_none(),
            "same_arena={same_arena}"
        );
        assert!(target_context.to_local(handles).is_none());
        assert!(source.upgrade().is_none());
        assert!(target.upgrade().is_none());
    }
}

#[test]
fn cached_html_templates_do_not_keep_an_empty_retired_realm_alive() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (context, state) = {
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let state =
            install_generated_bindings(scope, context, runtime("<body></body>"), RealmId(956), 1)
                .unwrap();
        assert!(
            state
                .instance_templates
                .borrow()
                .contains_key("HTMLElement")
        );
        state.retire_execution();
        state.refresh_connected_wrapper_roots(scope);
        (v8::Weak::new(scope, context), state)
    };
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    let collected = {
        v8::scope!(let handles,&mut isolate);
        context.to_local(handles).is_none()
    };
    if !collected {
        // Drop the observation handle before taking the snapshot, so the
        // diagnostic does not itself add a strong Context root. Preserve the
        // pre-snapshot verdict even if snapshot collection changes the heap.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oryn-retired-realm-{}-{stamp}.heapsnapshot",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let mut write_error = None;
                isolate.take_heap_snapshot(|bytes| {
                    match std::io::Write::write_all(&mut file, bytes) {
                        Ok(()) => true,
                        Err(error) => {
                            write_error = Some(error);
                            false
                        }
                    }
                });
                if let Some(error) = write_error {
                    eprintln!(
                        "retired realm heap snapshot incomplete at {}: {error}",
                        path.display()
                    );
                } else {
                    eprintln!("retired realm heap snapshot: {}", path.display());
                }
            }
            Err(error) => eprintln!("retired realm heap snapshot could not be created: {error}"),
        }
    }
    assert!(
        collected,
        "native cached templates and primitive callback data must not root their retired Context"
    );
    assert!(
        state
            .instance_templates
            .borrow()
            .contains_key("HTMLElement")
    );
    drop(state);
}

#[test]
fn cross_arena_owner_survives_adoption_and_releases_after_last_wrapper() {
    super::tests::initialize_v8();
    for same_arena in [false] {
        let mut isolate = v8::Isolate::new(Default::default());
        let (source_context, target_context, source, target, object) = {
            v8::scope!(let handles,&mut isolate);
            let first = v8::Context::new(handles, Default::default());
            let second = v8::Context::new(handles, Default::default());
            let token = v8::String::new(handles, "node-realms").unwrap();
            first.set_security_token(token.into());
            second.set_security_token(token.into());
            let dom = runtime("<body><section><b>kept</b></section></body>");
            let target_dom = if same_arena {
                dom.clone()
            } else {
                runtime("<body></body>")
            };
            let target_document = if same_arena {
                let parsed = oryn_dom::html::parse("<body></body>");
                dom.borrow_mut()
                    .import_subtree(&parsed.dom, parsed.document)
                    .unwrap()
            } else {
                target_dom.borrow().document()
            };
            let source = {
                let scope = &mut v8::ContextScope::new(handles, first);
                install_generated_bindings(scope, first, dom, RealmId(954), 1).unwrap()
            };
            let target = {
                let scope = &mut v8::ContextScope::new(handles, second);
                install_generated_bindings_with_root(
                    scope,
                    second,
                    target_dom,
                    RealmId(955),
                    1,
                    target_document,
                )
                .unwrap()
            };
            let scope = &mut v8::ContextScope::new(handles, second);
            let node = source
                .dom
                .borrow()
                .query_selector(source.document_root(), "section")
                .unwrap()
                .unwrap();
            let object = wrap_node(scope, &source, node).unwrap();
            assert!(object.get_creation_context(scope).unwrap() == first);
            let dataset = associated_wrapper(scope, &source, node, "DOMStringMap")
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            assert!(dataset.get_creation_context(scope).unwrap() == first);
            let global = second.global(scope);
            set_object_property(scope, global, "kept", object.into()).unwrap();
            assert!(eval(scope,"kept.marker=81;document.adoptNode(kept)===kept&&kept.ownerDocument===document&&kept.firstChild.textContent==='kept'").is_true());
            let (owner, actual) = node_identity::argument(scope, object.into())
                .unwrap()
                .unwrap();
            assert_eq!(
                owner.dom.borrow().node_document(actual).unwrap(),
                Some(target_document)
            );
            assert!(Rc::ptr_eq(
                &owner,
                if same_arena { &source } else { &target }
            ));
            eval(scope, "delete globalThis.kept");
            // The host consumes native mutation journals after publication.
            // Their removal leases are independent roots until that checkpoint.
            for dom in [&source.dom, &target.dom] {
                let mut dom = dom.borrow_mut();
                let mutations = dom.drain_mutations();
                dom.release_mutation_roots(&mutations).unwrap();
            }
            source.retire_execution();
            source.refresh_connected_wrapper_roots(scope);
            target.retire_execution();
            target.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, first),
                v8::Weak::new(scope, second),
                Rc::downgrade(&source),
                Rc::downgrade(&target),
                v8::Global::new(scope, object),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = source_context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let object = v8::Local::new(scope, &object);
            let (owner, node) = node_identity::argument(scope, object.into())
                .unwrap()
                .unwrap();
            assert!(source.upgrade().is_some());
            assert_eq!(
                owner.dom.borrow().arena().text_content(node).unwrap(),
                "kept"
            );
            let marker = v8::String::new(scope, "marker").unwrap();
            assert_eq!(
                object.get(scope, marker.into()).unwrap().int32_value(scope),
                Some(81)
            );
        }
        drop(object);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles,&mut isolate);
        assert!(source_context.to_local(handles).is_none());
        assert!(target_context.to_local(handles).is_none());
        assert!(source.upgrade().is_none());
        assert!(target.upgrade().is_none());
    }
}

#[test]
fn retired_document_allocates_original_interfaces_after_public_constructor_deletion() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (context, document, state) = {
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let state =
            install_generated_bindings(scope, context, runtime("<body></body>"), RealmId(957), 1)
                .unwrap();
        let source = v8::String::new(scope, "document.savedNavigationEntry=navigation.currentEntry; delete globalThis.navigation; delete globalThis.HTMLDivElement; delete globalThis.HTMLSpanElement; document.querySelectorAll('body'); document").unwrap();
        let document = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        state.retire_execution();
        state.refresh_connected_wrapper_roots(scope);
        (
            v8::Weak::new(scope, context),
            v8::Global::new(scope, document),
            state,
        )
    };
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    {
        v8::scope!(let handles, &mut isolate);
        let context = context
            .to_local(handles)
            .expect("retained document must preserve its owner");
        let scope = &mut v8::ContextScope::new(handles, context);
        let source = v8::String::new(scope, "(()=>{const a=document.createElement('div'),b=document.createElement('span');return a.constructor.name==='HTMLDivElement'&&b.constructor.name==='HTMLSpanElement'&&a instanceof HTMLElement&&document.querySelectorAll('body').item(0)===document.body&&document.savedNavigationEntry.getState()===undefined})()").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        state.refresh_connected_wrapper_roots(scope);
    }
    drop(document);
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    v8::scope!(let handles, &mut isolate);
    assert!(
        context.to_local(handles).is_none(),
        "Rust template caches must not retain the owner after its last document wrapper is gone"
    );
}
