impl RetainedStylesheetRecord {
    /// Retained CSSOM objects outlive DOM attachment and source-owner revision.
    /// Only originating execution/document identity retirement forbids access;
    /// resource fetch cancellation uses owner_publication_retired separately.
    fn cssom_execution_retired(&self) -> Result<bool, HostError> {
        let identity = self.owner.document;
        if !self
            .owner
            .lifecycle
            .current(oryn_js::v8_bindings::BindingStylesheetDocumentOwner {
                realm: identity.realm,
                document: identity.document,
                document_generation: identity.document_generation,
                stream_epoch: identity.stream_epoch,
            })
        {
            return Ok(true);
        }
        let Some(arena) = self.owner.arena.upgrade() else {
            return Ok(true);
        };
        let dom = arena.try_borrow().map_err(|_| {
            HostError::Execution("CSSOM execution authority DOM borrow conflict".into())
        })?;
        // Adoption/detachment of owner node is not retirement of its original
        // retained native stylesheet. Keep its captured document alive/valid.
        Ok(dom.arena().get(identity.document).is_err())
    }
}
#[cfg(test)]
mod cssom_execution_authority_tests {
    use super::*;
    struct Lifecycle(Cell<bool>);
    impl oryn_js::v8_bindings::BindingStylesheetLifecycle for Lifecycle {
        fn current(&self, _: oryn_js::v8_bindings::BindingStylesheetDocumentOwner) -> bool {
            self.0.get()
        }
    }
    struct Admission;
    impl oryn_layout::cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            Ok(Arc::new(()))
        }
    }
    #[test]
    fn detached_reconnected_and_adopted_owner_do_not_retire_cssom_execution() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let owner = dom.create_element("style");
        dom.append_child(document, owner).unwrap();
        let arena = Rc::new(RefCell::new(dom));
        let lifecycle = Rc::new(Lifecycle(Cell::new(true)));
        let admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission> =
            Rc::new(Admission);
        let graph = ImportGraph::parse(
            "p{}",
            "https://example.test/",
            false,
            8,
            oryn_layout::cssom::new_native_style_lock(),
            admission.as_ref(),
        )
        .unwrap();
        let sheet = graph.root();
        let record = Rc::new(RetainedStylesheetRecord {
            owner: RetainedStylesheetOwner {
                arena: Rc::downgrade(&arena),
                document: ParserDocumentIdentity {
                    realm: RealmId(1),
                    document,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                node: owner,
                revision: 0,
                lifecycle: lifecycle.clone(),
            },
            graph: Rc::new(RefCell::new(graph)),
            _source: Arc::from("p{}"),
            responses: RefCell::new(RetainedResponseStore::new(vec![
                RetainedStylesheetResponse {
                    sheet: Some(sheet.clone()),
                    final_url: "https://example.test/".into(),
                    source: Arc::from("p{}"),
                    referrer_policy: String::new(),
                    encoding: "utf-8".into(),
                    guard: None,
                    proof: NativeStylesheetResponseAuthority::Fetched(
                        crate::stylesheet_response_proof::StylesheetResponseProof {
                            id: 1,
                            origin_clean: true,
                            body_sha256: [0; 32],
                        },
                    ),
                },
            ])),
            admission,
            mutation_revision: Cell::new(0),
            pending_imports: RefCell::new(Vec::new()),
            _publication_charge: Arc::new(()),
        });
        let handle = NativeSheetHandle {
            identity: 1,
            publication: record.clone(),
            sheet: sheet.clone(),
            registry: Rc::new(RefCell::new(NativeCssHandleRegistry::default())),
            _charge: Arc::new(()),
        };
        use oryn_js::v8_bindings::BindingNativeStylesheet;
        arena.borrow_mut().remove_child(document, owner).unwrap();
        assert!(!record.cssom_execution_retired().unwrap());
        assert!(record.owner_publication_retired().unwrap());
        assert!(handle.insert_rule(1, "q{}").is_ok());
        assert_eq!(record.graph.borrow().rule_count(&sheet).unwrap(), 2);
        arena.borrow_mut().append_child(document, owner).unwrap();
        assert!(!record.cssom_execution_retired().unwrap());
        assert!(handle.insert_rule(2, "r{}").is_ok());
        let destination = arena
            .borrow_mut()
            .arena_mut()
            .insert(oryn_dom::NodeKind::Document);
        let (document_ref, node_ref) = {
            let dom = arena.borrow();
            (
                dom.arena().document_id(destination).unwrap(),
                dom.arena().node_ref(owner).unwrap(),
            )
        };
        arena
            .borrow_mut()
            .adopt_node(document_ref, node_ref)
            .unwrap();
        assert!(!record.cssom_execution_retired().unwrap());
        assert!(handle.insert_rule(3, "s{}").is_ok());
        assert_eq!(record.graph.borrow().rule_count(&sheet).unwrap(), 4);
        lifecycle.0.set(false);
        assert!(record.cssom_execution_retired().unwrap());
        assert!(handle.insert_rule(4, "t{}").is_err());
        assert_eq!(record.graph.borrow().rule_count(&sheet).unwrap(), 4);
    }
}
