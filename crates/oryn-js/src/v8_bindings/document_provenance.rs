//! Document-owned source snapshots survive node migration and execution retirement.
use super::*;
pub type Capture = dyn Fn(
    &V8BindingState,
    Option<&oryn_dom::DocumentProvenance>,
    &str,
) -> Result<oryn_dom::DocumentProvenance, String>;
impl V8BindingState {
    pub fn has_document_provenance_capture(&self) -> bool {
        self.document_provenance_capture.borrow().is_some()
    }
    /// Native replacement preparation; the new authority installs its issuer before author execution.
    pub fn clear_document_provenance_capture(&self) {
        self.document_provenance_capture.borrow_mut().take();
    }
    pub fn document_source_ancestors(&self) -> Vec<u32> {
        let mut cursor = self
            .frame_viewport_parent
            .borrow()
            .as_ref()
            .and_then(|(parent, _, _)| parent.upgrade());
        let mut result = Vec::new();
        while let Some(parent) = cursor {
            if result.contains(&parent.realm().0) || result.len() >= 4096 {
                break;
            }
            result.push(parent.realm().0);
            cursor = parent
                .frame_viewport_parent
                .borrow()
                .as_ref()
                .and_then(|(parent, _, _)| parent.upgrade());
        }
        result
    }

    pub fn set_document_provenance_capture(&self, capture: Rc<Capture>) {
        *self.document_provenance_capture.borrow_mut() = Some(capture);
    }
    pub fn install_document_provenance(
        &self,
        source: oryn_dom::DocumentProvenance,
    ) -> Result<(), V8BindingError> {
        self.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(self.document_root())?
            .document_metadata
            .as_mut()
            .ok_or(V8BindingError::Allocation("document provenance metadata"))?
            .provenance = Some(std::sync::Arc::new(source));
        Ok(())
    }
    pub fn refresh_document_provenance(&self) -> Result<(), V8BindingError> {
        if self.binding_global != BindingGlobal::Window || self.execution_retired() {
            return Ok(());
        }
        let Some(capture) = self.document_provenance_capture.borrow().clone() else {
            return Ok(());
        };
        let url = self.request_initiator_url()?;
        let source = capture(self, None, &url).map_err(|error| {
            V8BindingError::Message(format!("document provenance capture: {error}"))
        })?;
        self.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(self.document_root())?
            .document_metadata
            .as_mut()
            .ok_or(V8BindingError::Allocation("document provenance metadata"))?
            .provenance = Some(std::sync::Arc::new(source));
        Ok(())
    }
    pub fn document_provenance(
        &self,
        document: NodeId,
    ) -> Result<Option<std::sync::Arc<oryn_dom::DocumentProvenance>>, V8BindingError> {
        if document == self.document_root() && !self.execution_retired() {
            self.refresh_document_provenance()?;
        }
        Ok(self
            .dom
            .borrow()
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|metadata| metadata.provenance.clone()))
    }
    pub(super) fn derive_document_provenance(
        &self,
        source: NodeId,
        destination: NodeId,
    ) -> Result<(), V8BindingError> {
        let provenance = self.document_provenance(source)?;
        let relaxed_domain = self
            .dom
            .borrow()
            .arena()
            .get(source)?
            .document_metadata
            .as_ref()
            .map(|metadata| metadata.relaxed_domain.clone())
            .unwrap_or_default();
        let Some(provenance) = provenance else {
            self.dom
                .borrow_mut()
                .arena_mut()
                .get_mut(destination)?
                .document_metadata
                .as_mut()
                .ok_or(V8BindingError::Allocation("inert document metadata"))?
                .relaxed_domain = relaxed_domain;
            return Ok(());
        };
        let url = self
            .dom
            .borrow()
            .arena()
            .get(destination)?
            .document_metadata
            .as_ref()
            .and_then(|metadata| metadata.url.clone())
            .unwrap_or_else(|| "about:blank".into());
        let capture =
            self.document_provenance_capture
                .borrow()
                .clone()
                .ok_or(V8BindingError::Allocation(
                    "document provenance issuer unavailable",
                ))?;
        let derived = capture(self, Some(&provenance), &url)
            .map_err(|_| V8BindingError::Allocation("inert document provenance"))?;
        let mut dom = self.dom.borrow_mut();
        let metadata = dom
            .arena_mut()
            .get_mut(destination)?
            .document_metadata
            .as_mut()
            .ok_or(V8BindingError::Allocation("inert document metadata"))?;
        metadata.provenance = Some(std::sync::Arc::new(derived));
        metadata.relaxed_domain = relaxed_domain;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructors_cloning_and_retirement_preserve_native_document_identity() {
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
        state.commit_location("https://source.test/path").unwrap();
        let failed = Rc::new(Cell::new(false));
        let fail = failed.clone();
        state.set_document_provenance_capture(Rc::new(move |state, source, _url| {
            if fail.get() {
                return Err("injected capture failure".into());
            }
            if let Some(source) = source {
                let mut source = source.clone();
                source.inert = true;
                source.capability = b"derived-native-source".to_vec();
                return Ok(source);
            }
            assert!(!state.execution_retired());
            Ok(oryn_dom::DocumentProvenance {
                capability: b"native-source".to_vec(),
                origin: "https://source.test".into(),
                origin_key: "tuple:https://source.test".into(),
                inert: false,
            })
        }));
        state.refresh_document_provenance().unwrap();
        let source=v8::String::new(scope,r#"
            globalThis.docs=[new Document(),document.implementation.createDocument(null,''),document.implementation.createHTMLDocument(''),new DOMParser().parseFromString('<p>x</p>','text/html'),document.cloneNode(true)];
            docs.forEach(doc=>{Object.defineProperty(doc,'origin',{value:'https://forged.test'});});
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let key = v8::String::new(scope, "docs").unwrap();
        let docs = context.global(scope).get(scope, key.into()).unwrap();
        let docs = v8::Local::<v8::Array>::try_from(docs).unwrap();
        let mut documents = Vec::new();
        for index in 0..docs.length() {
            let value = docs.get_index(scope, index).unwrap();
            let node = argument_node(scope, &state, value).unwrap();
            let snapshot = state.document_provenance(node).unwrap().unwrap();
            assert!(snapshot.inert);
            assert_eq!(snapshot.origin, "https://source.test");
            documents.push(node);
        }
        failed.set(true);
        assert!(
            state
                .commit_location("https://source.test/changed")
                .is_err()
        );
        assert_eq!(state.current_location_url(), "https://source.test/path");
        assert!(state.set_document_referrer_policy("no-referrer").is_err());
        assert_ne!(state.document_referrer_policy(), "no-referrer");
        failed.set(false);
        state.retire_execution();
        let copy = dom.borrow_mut().clone_node(documents[0], false).unwrap();
        state
            .derive_document_provenance(documents[0], copy)
            .unwrap();
        assert!(state.document_provenance(copy).unwrap().unwrap().inert);
    }
}
