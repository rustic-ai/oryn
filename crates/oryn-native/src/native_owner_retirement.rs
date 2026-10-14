impl RetainedStylesheetRecord {
    /// Proven publication retirement, not an ordinary journal cursor race.
    /// Host lifecycle authority is queried before borrowing the DOM; no
    /// admission/native parse callback occurs within the read-only borrow.
    fn owner_publication_retired(&self) -> Result<bool, HostError> {
        let owner = &self.owner;
        let identity = owner.document;
        if !owner
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
        let Some(arena) = owner.arena.upgrade() else {
            return Ok(true);
        };
        let dom = arena.try_borrow().map_err(|_| {
            HostError::Execution("stylesheet retirement DOM borrow conflict".into())
        })?;
        let Ok(node) = dom.arena().get(owner.node) else {
            return Ok(true);
        };
        if node.owner_document != Some(identity.document) {
            return Ok(true);
        }
        if !dom
            .is_connected(owner.node)
            .map_err(|e| HostError::Execution(e.to_string()))?
        {
            return Ok(true);
        }
        Ok(dom
            .arena()
            .stylesheet_revision(owner.node)
            .map_err(|e| HostError::Execution(e.to_string()))?
            != owner.revision)
    }
}
struct PreparedOwnerRetirements {
    decisions: Vec<bool>,
    removed: Vec<Rc<RetainedStylesheetRecord>>,
    count: usize,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl PreparedOwnerRetirements {
    /// Caller snapshots count, releases registry borrow, then calls prepare.
    fn prepare(
        count: usize,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<Self, HostError> {
        let bytes = count
            .checked_mul(
                std::mem::size_of::<bool>() + std::mem::size_of::<Rc<RetainedStylesheetRecord>>(),
            )
            .ok_or_else(|| HostError::Execution("retirement capacity overflow".into()))?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let mut decisions = Vec::new();
        decisions
            .try_reserve_exact(count)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let mut removed = Vec::new();
        removed
            .try_reserve_exact(count)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        Ok(Self {
            decisions,
            removed,
            count,
            _charge: charge,
        })
    }
}
impl RetainedStylesheets {
    /// All fallible checks precede moving any Rc. No destructor runs during
    /// registry mutation: returned custody MUST drop after releasing borrow.
    fn take_proven_retired_owner_publications(
        &mut self,
        prepared: &mut PreparedOwnerRetirements,
        snapshot: &RetainedPublicationSnapshot,
    ) -> Result<bool, HostError> {
        if !snapshot.matches(self)
            || self.records.len() != prepared.count
            || prepared.decisions.capacity() < self.records.len()
            || prepared.removed.capacity() < self.records.len()
            || !prepared.removed.is_empty()
            || prepared.decisions.len() != prepared.count
        {
            return Ok(false);
        }
        let mut decisions = prepared.decisions.iter();
        self.records.retain(|record| {
            let retired = *decisions
                .next()
                .expect("validated retirement decision count");
            if retired {
                prepared.removed.push(record.clone());
            }
            !retired
        });
        Ok(true)
    }
}
#[cfg(test)]
mod retirement_custody_tests {
    use std::{cell::Cell, rc::Rc};
    struct Probe(Rc<Cell<usize>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    #[test]
    fn moving_retirement_custody_does_not_run_destructor() {
        let drops = Rc::new(Cell::new(0));
        let mut registry = vec![Rc::new(Probe(drops.clone()))];
        let mut removed = Vec::new();
        removed.try_reserve_exact(registry.len()).unwrap();
        removed.push(registry.remove(0));
        assert_eq!(drops.get(), 0);
        drop(registry);
        assert_eq!(drops.get(), 0);
        drop(removed);
        assert_eq!(drops.get(), 1);
    }
}

fn sweep_retired_stylesheet_records(
    registry: &RefCell<RetainedStylesheets>,
    admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
) -> Result<(), HostError> {
    let Some(snapshot) = RetainedPublicationSnapshot::capture(registry, admission)? else {
        return Ok(());
    };
    let mut prepared = PreparedOwnerRetirements::prepare(snapshot.entries.len(), admission)?;
    admission.checkpoint().map_err(HostError::Execution)?;
    for entry in &snapshot.entries {
        prepared
            .decisions
            .push(entry.record.owner_publication_retired()?);
    }
    {
        let mut current = registry.try_borrow_mut().map_err(|_| {
            HostError::Execution("stylesheet retirement registry borrow conflict".into())
        })?;
        current.take_proven_retired_owner_publications(&mut prepared, &snapshot)?;
    }
    drop(prepared);
    drop(snapshot);
    Ok(())
}

#[cfg(test)]
mod retained_record_sweep_tests {
    use super::*;
    struct Admission;
    impl oryn_layout::cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            Ok(Arc::new(()))
        }
    }
    struct Lifecycle(Cell<bool>);
    impl oryn_js::v8_bindings::BindingStylesheetLifecycle for Lifecycle {
        fn current(&self, _: oryn_js::v8_bindings::BindingStylesheetDocumentOwner) -> bool {
            self.0.get()
        }
    }
    #[test]
    fn sweep_preserves_retained_graph_and_rejects_stale_registry_snapshot() {
        let admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission> =
            Rc::new(Admission);
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let node = dom.create_element("style");
        dom.append_child(document, node).unwrap();
        let revision = dom.arena().stylesheet_revision(node).unwrap();
        let arena = Rc::new(RefCell::new(dom));
        let lifecycle = Rc::new(Lifecycle(Cell::new(true)));
        let graph = ImportGraph::parse(
            "div{color:red}",
            "https://example.test/s.css",
            false,
            8,
            oryn_layout::cssom::new_native_style_lock(),
            admission.as_ref(),
        )
        .unwrap();
        let retained_root = graph.root();
        let record = Rc::new(RetainedStylesheetRecord {
            owner: RetainedStylesheetOwner {
                arena: Rc::downgrade(&arena),
                document: ParserDocumentIdentity {
                    realm: RealmId(1),
                    document,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                node,
                revision,
                lifecycle: lifecycle.clone(),
            },
            graph: Rc::new(RefCell::new(graph)),
            _source: Arc::from("div{color:red}"),
            responses: RefCell::new(RetainedResponseStore::new(Vec::new())),
            admission: admission.clone(),
            mutation_revision: Cell::new(0),
            pending_imports: RefCell::new(Vec::new()),
            _publication_charge: Arc::new(()),
        });
        let registry = RefCell::new(RetainedStylesheets {
            records: vec![record.clone()],
            registry_charge: None,
        });
        sweep_retired_stylesheet_records(&registry, admission.as_ref()).unwrap();
        assert_eq!(registry.borrow().records.len(), 1);
        let snapshot = RetainedPublicationSnapshot::capture(&registry, admission.as_ref())
            .unwrap()
            .unwrap();
        let mut prepared = PreparedOwnerRetirements::prepare(1, admission.as_ref()).unwrap();
        prepared.decisions.push(true);
        record.mutation_revision.set(1);
        assert!(
            !registry
                .borrow_mut()
                .take_proven_retired_owner_publications(&mut prepared, &snapshot)
                .unwrap()
        );
        assert_eq!(registry.borrow().records.len(), 1);
        struct RestoreLifecycle(Rc<Lifecycle>);
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for RestoreLifecycle {
            fn checkpoint(&self) -> Result<(), String> {
                self.0.0.set(true);
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
                Ok(Arc::new(()))
            }
        }
        lifecycle.0.set(false);
        sweep_retired_stylesheet_records(&registry, &RestoreLifecycle(lifecycle.clone())).unwrap();
        assert_eq!(registry.borrow().records.len(), 1);
        struct Reattach {
            arena: SharedDomRuntime,
            document: NodeId,
            node: NodeId,
        }
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for Reattach {
            fn checkpoint(&self) -> Result<(), String> {
                if !self
                    .arena
                    .borrow()
                    .is_connected(self.node)
                    .map_err(|e| e.to_string())?
                {
                    self.arena
                        .borrow_mut()
                        .append_child(self.document, self.node)
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
                Ok(Arc::new(()))
            }
        }
        arena.borrow_mut().remove_child(document, node).unwrap();
        let reattach = Reattach {
            arena: arena.clone(),
            document,
            node,
        };
        sweep_retired_stylesheet_records(&registry, &reattach).unwrap();
        assert!(arena.borrow().is_connected(node).unwrap());
        assert_ne!(
            arena.borrow().arena().stylesheet_revision(node).unwrap(),
            revision
        );
        assert!(registry.borrow().records.is_empty());
        lifecycle.0.set(false);
        sweep_retired_stylesheet_records(&registry, admission.as_ref()).unwrap();
        assert!(registry.borrow().records.is_empty());
        assert!(retained_root.same_sheet(&record.graph.borrow().root()));
        assert_eq!(record.graph.borrow().rule_count(&retained_root).unwrap(), 1);
    }
}
