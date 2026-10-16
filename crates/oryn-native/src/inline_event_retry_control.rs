#[cfg(test)]
mod inline_event_retry_control {
    use super::*;
    #[test]
    fn inline_admission_new_owner_is_not_cached_as_already_published() {
        struct IntroduceOwner {
            delegate: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
            dom: SharedDomRuntime,
            parent: NodeId,
            fired: Cell<bool>,
        }
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for IntroduceOwner {
            fn checkpoint(&self)->Result<(),String> {self.delegate.checkpoint()}
            fn reserve(&self,bytes:usize)->Result<Arc<dyn Send+Sync>,String> {
                let charge=self.delegate.reserve(bytes)?;
                if !self.fired.replace(true) {
                    let mut dom=self.dom.try_borrow_mut().map_err(|_|"allocation callback held DOM borrow")?;
                    let owner=dom.create_element("style");
                    let text=dom.create_text("p{width:29px}");
                    dom.append_child(owner,text).map_err(|e|e.to_string())?;
                    dom.append_child(self.parent,owner).map_err(|e|e.to_string())?;
                }
                Ok(charge)
            }
        }
        let mut page=RustExecutableDocument::load_with_environment("<!doctype html><p id=p></p>",
            &BTreeMap::new(),None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
        page.agent.host.execute("globalThis.s=document.createElement('style');s.textContent='p{width:17px}';document.head.append(s)").unwrap();
        let realm=page.agent.host.active_realm();
        let provider=page.agent.host.native_layout_provider_in_realm(realm).unwrap();
        let backend=provider.native_backend().unwrap().downcast_ref::<LiveLayoutProvider>().unwrap();
        let delegate=backend.retained_revision_admission().unwrap();
        let parent={let dom=page.agent.dom.borrow();
            dom.query_selector(dom.document(),"head").unwrap().expect("actual document head")};
        let callback=Rc::new(IntroduceOwner {delegate:delegate.clone(),dom:page.agent.dom.clone(),
            parent,fired:Cell::new(false)});
        *backend.retained_admission.borrow_mut()=Some(callback.clone());
        // The first read can report a stale snapshot after the callback. A
        // second read must discover its new owner, never cache the new cursor.
        let first=page.agent.host.evaluate_string("getComputedStyle(p).width");
        assert!(callback.fired.get());
        let second=page.agent.host.evaluate_string("getComputedStyle(p).width");
        *backend.retained_admission.borrow_mut()=Some(delegate);
        if let Err(error)=&first {
            assert!(!error.to_string().contains("allocation callback held DOM borrow"),
                "callback isolation failed: {error}");
        }
        assert_eq!(second.unwrap(),"29px","first admission-raced read: {first:?}");
    }
    #[test]
    fn incumbent_inline_event_failure_keeps_original_graph_and_retries_once() {
        let mut page=RustExecutableDocument::load_with_environment("<!doctype html><p id=p></p>",
            &BTreeMap::new(),None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
        page.agent.host.execute("globalThis.s=document.createElement('style');s.textContent='p{width:17px}';s.onload=()=>globalThis.loads=(globalThis.loads||0)+1;document.head.append(s);globalThis.original=s.sheet;globalThis.rule=original.cssRules[0]").unwrap();
        frame_script_preparation::inject_stylesheet_event_failure();
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        let mut fault=false;
        loop {
            match page.checkpoint() {
                Err(error) => {
                    assert!(error.to_string().contains("injected XML publication failure"));
                    fault=true;
                }
                Ok(())=>{}
            }
            match page.poll_context_events() {
                Err(error)=>{
                    assert!(error.to_string().contains("injected XML publication failure"));
                    fault=true;
                }
                Ok(())=>{}
            }
            assert_eq!(page.evaluate("String(s.sheet===original&&original.cssRules[0]===rule&&getComputedStyle(p).width==='17px')").unwrap(),"true");
            if page.evaluate("String(globalThis.loads===1)").unwrap()=="true" {break;}
            assert!(std::time::Instant::now()<deadline,"native event retry pending");
            std::thread::yield_now();
        }
        assert!(fault,"actual native BeforeStyleEvent injection was exercised");
        page.checkpoint().unwrap();
        assert_eq!(page.evaluate("String(loads===1&&s.sheet===original&&original.cssRules[0]===rule)").unwrap(),"true");
    }
}
