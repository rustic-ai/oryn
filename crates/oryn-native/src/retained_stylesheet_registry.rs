// Native publication roots. Parsing/admission never occur under this RefCell.
use oryn_layout::cssom::native_import_graph::{ImportGraph};
struct RetainedStylesheetOwner {
    arena: Weak<RefCell<oryn_dom::DomRuntime>>,
    document: ParserDocumentIdentity,
    node: NodeId,
    revision: u64,
    lifecycle: Rc<dyn oryn_js::v8_bindings::BindingStylesheetLifecycle>,
}
struct RetainedStylesheetRecord {
    owner: RetainedStylesheetOwner,
    graph: Rc<RefCell<ImportGraph>>,
    _source: Arc<str>,
    responses: RefCell<RetainedResponseStore>,
    admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    mutation_revision: Cell<u64>,
    pending_imports: RefCell<Vec<oryn_layout::cssom::native_import_graph::OccurrenceId>>,
    _publication_charge: Arc<dyn Send + Sync>,
}
#[derive(Default)]
struct RetainedStylesheets {
    records: Vec<Rc<RetainedStylesheetRecord>>,
    registry_charge: Option<Arc<dyn Send + Sync>>,
}
impl RetainedStylesheets {
    fn current(
        &self,
        arena: &SharedDomRuntime,
        document: ParserDocumentIdentity,
        node: NodeId,
        revision: u64,
    ) -> Option<Rc<RetainedStylesheetRecord>> {
        self.records
            .iter()
            .find(|record| {
                let owner = &record.owner;
                owner
                    .lifecycle
                    .current(oryn_js::v8_bindings::BindingStylesheetDocumentOwner {
                        realm: document.realm,
                        document: document.document,
                        document_generation: document.document_generation,
                        stream_epoch: document.stream_epoch,
                    })
                    && owner
                        .arena
                        .upgrade()
                        .is_some_and(|actual| Rc::ptr_eq(&actual, arena))
                    && owner.document.realm == document.realm
                    && owner.document.document == document.document
                    && owner.document.document_generation == document.document_generation
                    && owner.document.stream_epoch == document.stream_epoch
                    && owner.node == node
                    && owner.revision == revision
            })
            .cloned()
    }
    fn cascade(
        &self,
        arena: &SharedDomRuntime,
        document: ParserDocumentIdentity,
        owners: &BTreeMap<usize, u64>,
        lock: oryn_layout::cssom::NativeStyleLock,
        revision: u64,
        charge: Arc<dyn Send + Sync>,
    ) -> Result<oryn_layout::RetainedCascadeSheets, HostError> {
        let mut sheets = BTreeMap::new();
        let dom = arena.try_borrow().map_err(|_| {
            HostError::Execution("stylesheet cascade ownership borrow conflict".into())
        })?;
        for (index, id) in owners {
            let owner = NodeId {
                slot: *id as u32,
                generation: (*id >> 32) as u32,
            };
            let source_revision = dom
                .arena()
                .stylesheet_revision(owner)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            if let Some(record) = self.current(arena, document, owner, source_revision) {
                let sheet = record
                    .graph
                    .try_borrow()
                    .map_err(|_| {
                        HostError::Execution("stylesheet cascade graph borrow conflict".into())
                    })?
                    .root();
                sheets.insert(*index, sheet);
            }
        }
        Ok(oryn_layout::RetainedCascadeSheets::new(
            lock, sheets, revision, charge,
        ))
    }
}
