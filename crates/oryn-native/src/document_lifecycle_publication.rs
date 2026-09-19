//! Native lifecycle receipts retain callback progress across actor retries.
use super::*;
use std::time::Instant;

pub(super) struct PendingDocumentPublication {
    generation: u64,
    stream: u64,
    stage: Cell<u8>,
    receipt: Cell<Option<u64>>,
    retry: Cell<Option<Instant>>,
    active: Cell<bool>,
    parent: Cell<Option<RealmId>>,
}
impl PendingDocumentPublication {
    fn new(generation: u64, stream: u64, stage: u8) -> Self {
        Self {
            generation,
            stream,
            stage: Cell::new(stage),
            receipt: Cell::new(None),
            retry: Cell::new(None),
            active: Cell::new(false),
            parent: Cell::new(None),
        }
    }
}
impl RustExecutableDocument {
    pub(super) fn settle_frame_navigation_executions(&mut self) {
        let settled = self
            .frame_realms
            .iter()
            .filter_map(|(&realm, owner)| {
                let (_, identity) = owner.execution.as_ref()?;
                let live = self.agent.host.automation_realm_identity(realm) == Some(*identity)
                    && self.agent.host.automation_frame_child_live(
                        owner.parent_realm,
                        owner.frame,
                        realm,
                    )
                    && !self.agent.host.realm_document_load_stopped(realm);
                let outcome = if !live {
                    oryn_js::navigation::NavigationExecutionOutcome::Canceled
                } else if self
                    .document_publications
                    .get(&realm)
                    .is_some_and(|pending| {
                        pending.stage.get() == 7
                            && pending.generation == identity.1
                            && pending.stream == identity.2
                    })
                {
                    oryn_js::navigation::NavigationExecutionOutcome::Committed {
                        document_generation: identity.1,
                    }
                } else {
                    return None;
                };
                Some((realm, outcome))
            })
            .collect::<Vec<_>>();
        for (realm, outcome) in settled {
            if let Some((observer, _)) = self
                .frame_realms
                .get_mut(&realm)
                .and_then(|owner| owner.execution.take())
            {
                observer.complete(outcome);
            }
        }
    }
    fn document_publication_live(
        &self,
        realm: RealmId,
        pending: &PendingDocumentPublication,
    ) -> bool {
        self.agent.host.realm_execution_live(realm)
            && !self.agent.host.realm_document_load_stopped(realm)
            && self.agent.host.document_stream_epoch(realm) == Some(pending.stream)
            && self
                .agent
                .host
                .realm_descriptor(realm)
                .is_some_and(|current| current.document_generation == pending.generation)
    }
    pub(super) fn document_publications_runnable(&self) -> bool {
        self.document_publications.iter().any(|(&realm, pending)| {
            !pending.active.get()
                && (!self.document_publication_live(realm, pending)
                    || pending.stage.get() != 7
                        && (pending.stage.get() != 2 || !self.document_load_barrier(realm))
                        && pending
                            .retry
                            .get()
                            .is_none_or(|retry| retry <= Instant::now()))
        })
    }
    pub(super) fn document_publication_delay(&self) -> Option<Duration> {
        self.document_publications
            .values()
            .filter(|pending| !pending.active.get() && pending.stage.get() != 7)
            .filter_map(|pending| {
                pending
                    .retry
                    .get()
                    .map(|retry| retry.saturating_duration_since(Instant::now()))
            })
            .min()
    }
    pub(super) fn queue_document_load(&mut self, realm: RealmId) -> Result<(), HostError> {
        if self.agent.host.realm_document_load_stopped(realm) {
            return Ok(());
        }
        self.agent.host.activate_realm(realm)?;
        let generation = self
            .agent
            .host
            .realm_descriptor(realm)
            .ok_or_else(|| HostError::Execution("load document is unavailable".into()))?
            .document_generation;
        let stream = self
            .agent
            .host
            .document_stream_epoch(realm)
            .ok_or_else(|| HostError::Execution("load stream is unavailable".into()))?;
        let retired = self
            .document_publications
            .iter()
            .filter_map(|(&owner, pending)| {
                (pending.stage.get() == 7 && !self.document_publication_live(owner, pending))
                    .then_some(owner)
            })
            .collect::<Vec<_>>();
        for owner in retired {
            self.document_publications.remove(&owner);
        }
        if !self
            .document_publications
            .get(&realm)
            .is_some_and(|pending| pending.generation == generation && pending.stream == stream)
        {
            if !self.document_publications.contains_key(&realm)
                && self.document_publications.len() >= 1024
            {
                return Err(HostError::LimitExceeded {
                    resource: "document lifecycle publications".into(),
                });
            }
            if let Some(previous) = self.document_publications.get(&realm).cloned() {
                self.remove_document_publication(realm, &previous);
            }
            self.document_publications.insert(
                realm,
                Rc::new(PendingDocumentPublication::new(generation, stream, 0)),
            );
        }
        if self
            .document_publications
            .get(&realm)
            .is_some_and(|pending| pending.stage.get() == 7)
        {
            return Ok(());
        }
        // Native ownership precedes DOMContentLoaded and all author checkpoints.
        self.pending_document_loads
            .insert(realm, (generation, stream));
        if realm == self.top_realm {
            self.document_load_stopped = false;
        }
        self.agent.host.acknowledge_closed_document_stream(realm);
        self.flush_document_loads()
    }
    fn document_load_barrier(&self, realm: RealmId) -> bool {
        self.document_load_barrier_with_author_input(realm, true)
    }
    // Construction may return an incomplete document whose only remaining
    // dependency is authored document.write/close. Publication never uses this
    // exemption: its strict load barrier remains active until close/stop/remove.
    fn document_waits_only_for_author_input(&self, realm: RealmId) -> bool {
        if self
            .document_publications
            .get(&realm)
            .is_some_and(|pending| {
                self.document_publication_live(realm, pending)
                    && (pending.active.get()
                        || pending.receipt.get().is_some()
                        || !matches!(pending.stage.get(), 2 | 7))
            })
        {
            return false;
        }
        self.document_load_barrier_with_author_input(realm, true)
            && !self.document_load_barrier_with_author_input(realm, false)
    }
    pub(super) fn construction_document_publication_pending(&self, realm: RealmId) -> bool {
        let Some(pending) = self.document_publications.get(&realm) else {
            return self.pending_document_loads.contains_key(&realm);
        };
        if pending.stage.get() == 7 || !self.document_publication_live(realm, pending) {
            return false;
        }
        pending.stage.get() != 2 || !self.document_waits_only_for_author_input(realm)
    }
    fn document_load_barrier_with_author_input(
        &self,
        realm: RealmId,
        include_author_input: bool,
    ) -> bool {
        if self.frame_resources_pending(Some(realm)) {
            return true;
        }
        // A replacement child stream can still be loading before
        // document.close queues its lifecycle task. Include native
        // readiness, not just entries already in the completion map.
        let pending_child = self.frame_realms.keys().any(|child| {
            if self.agent.host.realm_document_load_stopped(*child) {
                return false;
            }
            let Some((document, _, _, true)) = self.agent.host.automation_realm_identity(*child)
            else {
                return false;
            };
            let loading = self.pending_document_loads.contains_key(child)
                || self.agent.host.automation_dom(*child).is_some_and(|dom| {
                    let dom = dom.borrow();
                    dom.arena()
                        .get(document)
                        .ok()
                        .and_then(|node| node.document_metadata.as_ref())
                        .is_some_and(|metadata| {
                            metadata.ready_state != oryn_dom::DocumentReadyState::Complete
                        })
                });
            if !loading {
                return false;
            }
            let mut ancestor = *child;
            let mut visited = std::collections::BTreeSet::new();
            while visited.insert(ancestor) {
                let Some(owner) = self.frame_realms.get(&ancestor) else {
                    return false;
                };
                if !self
                    .agent
                    .host
                    .automation_frame_live(owner.parent_realm, owner.frame)
                {
                    return false;
                }
                if owner.parent_realm == realm {
                    return include_author_input
                        || !self.document_waits_only_for_author_input(*child);
                }
                ancestor = owner.parent_realm;
            }
            false
        });
        if pending_child
            || self.dynamic_stylesheets_pending(Some(realm))
            || (if include_author_input {
                self.frame_scripts_pending(Some(realm))
            } else {
                self.ready_frame_jobs
                    .iter()
                    .any(|(parent, _, _)| *parent == realm)
                    || self
                        .frame_departures
                        .keys()
                        .any(|(parent, _)| *parent == realm)
                    || self.frame_script_preparations.iter().any(|owner| {
                        owner.load.realm == realm && owner.construction_pending(&self.agent.host)
                    })
            })
            || self.has_parser_async_modules(Some(realm))
            || self
                .dynamic_scripts
                .pending
                .values()
                .any(|(owner, prepared)| {
                    *owner == realm && self.agent.host.dynamic_script_is_current(*owner, prepared)
                })
            || self.agent.host.realm_has_pending_dynamic_scripts(realm)
            || self.agent.host.realm_has_pending_images(realm)
            || self.agent.host.realm_has_pending_frame_navigation(realm)
            || self.agent.host.realm_has_pending_stylesheet_events(realm)
        {
            return true;
        }
        false
    }
    pub(super) fn flush_document_loads(&mut self) -> Result<(), HostError> {
        self.poll_dynamic_stylesheets()?;
        // Preserve callers which already own a post-DOMContentLoaded barrier.
        for (&realm, &(generation, stream)) in &self.pending_document_loads {
            if !self
                .document_publications
                .get(&realm)
                .is_some_and(|pending| {
                    pending.generation == generation
                        && pending.stream == stream
                        && pending.stage.get() != 7
                })
            {
                self.document_publications.insert(
                    realm,
                    Rc::new(PendingDocumentPublication::new(generation, stream, 2)),
                );
            }
        }
        let previous = self.agent.host.active_realm();
        let result = (|| {
            loop {
                let snapshot = self
                    .document_publications
                    .iter()
                    .map(|(&realm, pending)| (realm, pending.clone()))
                    .collect::<Vec<_>>();
                let mut progressed = false;
                for (realm, pending) in snapshot {
                    if pending.active.get() {
                        continue;
                    }
                    if !self.document_publication_live(realm, &pending) {
                        self.remove_document_publication(realm, &pending);
                        progressed = true;
                        continue;
                    }
                    if pending.stage.get() == 7 {
                        continue;
                    }
                    if pending
                        .retry
                        .get()
                        .is_some_and(|retry| retry > Instant::now())
                    {
                        continue;
                    }
                    if pending.stage.get() == 2 && self.document_load_barrier(realm) {
                        continue;
                    }
                    pending.active.set(true);
                    let delivered = self.resume_document_publication(realm, &pending);
                    pending.active.set(false);
                    match delivered {
                        Ok(true) => {
                            self.remove_document_publication(realm, &pending);
                            progressed = true;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            pending
                                .retry
                                .set(Some(Instant::now() + Duration::from_millis(25)));
                            return Err(error);
                        }
                    }
                }
                if !progressed {
                    break;
                }
            }
            Ok(())
        })();
        let restored = self.agent.host.activate_realm(previous);
        result.and(restored)
    }
    fn remove_document_publication(
        &mut self,
        realm: RealmId,
        pending: &Rc<PendingDocumentPublication>,
    ) {
        if let Some(receipt) = pending.receipt.take() {
            if let Some(parent) = pending.parent.get() {
                self.agent.host.retire_native_event_receipt(parent, receipt);
            } else {
                self.agent
                    .host
                    .cancel_native_history_receipt(realm, receipt);
            }
        }
        if self
            .document_publications
            .get(&realm)
            .is_some_and(|current| Rc::ptr_eq(current, pending))
        {
            if self.document_publication_live(realm, pending) {
                pending.stage.set(7);
                pending.retry.set(None);
            } else {
                self.document_publications.remove(&realm);
            }
            if self.pending_document_loads.get(&realm)
                == Some(&(pending.generation, pending.stream))
            {
                self.pending_document_loads.remove(&realm);
            }
        }
    }
    fn resume_document_publication(
        &mut self,
        realm: RealmId,
        pending: &PendingDocumentPublication,
    ) -> Result<bool, HostError> {
        loop {
            if !self.document_publication_live(realm, pending) {
                return Ok(true);
            }
            self.agent.host.activate_realm(realm)?;
            match pending.stage.get() {
                0 | 3 => {
                    let content_loaded = pending.stage.get() == 0;
                    let receipt = self
                        .agent
                        .host
                        .start_native_document_lifecycle_publication(content_loaded)?;
                    pending.receipt.set(Some(receipt));
                    pending.stage.set(if content_loaded { 1 } else { 4 });
                    #[cfg(test)]
                    lifecycle_failure(if content_loaded {
                        LifecycleFailure::AfterDomReceipt
                    } else {
                        LifecycleFailure::AfterLoadReceipt
                    })?;
                }
                1 | 4 => {
                    if !self
                        .agent
                        .host
                        .resume_native_history_publication(pending.receipt.get().unwrap())?
                    {
                        pending
                            .retry
                            .set(Some(Instant::now() + Duration::from_millis(25)));
                        return Ok(false);
                    }
                    pending.receipt.set(None);
                    pending.retry.set(None);
                    pending
                        .stage
                        .set(if pending.stage.get() == 1 { 2 } else { 5 });
                }
                2 => {
                    if self.document_load_barrier(realm) {
                        return Ok(false);
                    }
                    pending.stage.set(3);
                }
                5 => {
                    let Some(owner) = self.frame_realms.get(&realm).cloned() else {
                        return Ok(true);
                    };
                    self.agent.host.activate_realm(owner.parent_realm)?;
                    let Some(receipt) = self.agent.host.start_native_frame_load_publication(
                        owner.frame,
                        realm,
                        pending.generation,
                    )?
                    else {
                        return Ok(true);
                    };
                    pending.parent.set(Some(owner.parent_realm));
                    pending.receipt.set(Some(receipt));
                    pending.stage.set(6);
                    #[cfg(test)]
                    lifecycle_failure(LifecycleFailure::AfterFrameReceipt)?;
                }
                6 => {
                    let parent = pending.parent.get().unwrap();
                    if !self.agent.host.realm_execution_live(parent) {
                        return Ok(true);
                    }
                    self.agent.host.activate_realm(parent)?;
                    if !self
                        .agent
                        .host
                        .resume_native_history_publication(pending.receipt.get().unwrap())?
                    {
                        pending
                            .retry
                            .set(Some(Instant::now() + Duration::from_millis(25)));
                        return Ok(false);
                    }
                    pending.receipt.set(None);
                    return Ok(true);
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
enum LifecycleFailure {
    AfterDomReceipt,
    AfterLoadReceipt,
    AfterFrameReceipt,
}
#[cfg(test)]
thread_local! {static LIFECYCLE_FAILURE:Cell<Option<LifecycleFailure>>=const{Cell::new(None)};}
#[cfg(test)]
fn lifecycle_failure(point: LifecycleFailure) -> Result<(), HostError> {
    if LIFECYCLE_FAILURE.with(|failure| {
        if failure.get() == Some(point) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        Err(HostError::Execution(
            "injected lifecycle receipt failure".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reset(page: &mut RustExecutableDocument, realm: RealmId) {
        page.pending_document_loads.remove(&realm);
        page.document_publications.remove(&realm);
        page.agent.host.activate_realm(realm).unwrap();
        page.agent
            .host
            .set_document_readiness(oryn_dom::DocumentReadyState::Loading)
            .unwrap();
    }
    fn prepare_images(page: &mut RustExecutableDocument, realm: RealmId) {
        let previous = page.agent.host.active_realm();
        page.agent.host.activate_realm(realm).unwrap();
        page.agent.host.prepare_document_images().unwrap();
        page.agent.host.activate_realm(previous).unwrap();
    }
    fn resume(page: &mut RustExecutableDocument) {
        for pending in page.document_publications.values() {
            pending.retry.set(None);
        }
        page.flush_document_loads().unwrap();
    }
    fn publication_state(page: &RustExecutableDocument, realm: RealmId) -> String {
        page.document_publications
            .get(&realm)
            .map(|pending| {
                format!(
                    "stage={}, receipt={:?}, retry={:?}, live={}, load_barrier={}",
                    pending.stage.get(),
                    pending.receipt.get(),
                    pending.retry.get(),
                    page.document_publication_live(realm, pending),
                    page.document_load_barrier(realm)
                )
            })
            .unwrap_or_else(|| "no retained publication".into())
    }
    #[test]
    fn native_dom_receipt_retry_preserves_order_and_does_not_duplicate() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        let realm = page.top_realm;
        reset(&mut page, realm);
        page.agent.host.execute("globalThis.calls=[];document.addEventListener('DOMContentLoaded',()=>calls.push('dom'));addEventListener('load',e=>calls.push(e.target===document&&e.currentTarget===window?'load':'bad-target'))").unwrap();
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterDomReceipt)));
        assert!(page.queue_document_load(realm).is_err());
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom"
        );
        assert!(page.pending_document_loads.contains_key(&realm));
        assert!(!page.document_publications_runnable());
        assert!(page.document_publication_delay().is_some());
        // Raw host execution does not run the image-selection phase after the
        // DOM listener registration mutation, unlike the parser/actor driver.
        assert!(page.agent.host.realm_has_pending_images(realm));
        prepare_images(&mut page, realm);
        assert!(!page.agent.host.realm_has_pending_images(realm));
        resume(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom,load",
            "{}",
            publication_state(&page, realm)
        );
        assert!(!page.pending_document_loads.contains_key(&realm));
        page.queue_document_load(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom,load"
        );
    }
    #[test]
    fn native_load_receipt_retry_does_not_repeat_load() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        let realm = page.top_realm;
        reset(&mut page, realm);
        page.agent
            .host
            .execute("globalThis.loads=0;addEventListener('load',()=>loads++)")
            .unwrap();
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterLoadReceipt)));
        assert!(page.queue_document_load(realm).is_err());
        assert_eq!(
            page.agent.host.evaluate_string("String(loads)").unwrap(),
            "1"
        );
        resume(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("String(loads)").unwrap(),
            "1"
        );
        assert!(page.pending_document_loads.is_empty());
    }
    #[test]
    fn parent_frame_receipt_retry_preserves_single_callback() {
        let mut page =
            RustExecutableDocument::load("<iframe id=f srcdoc='<p>child</p>'></iframe>").unwrap();
        let realm = *page.frame_realms.keys().next().unwrap();
        reset(&mut page, realm);
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("globalThis.frameLoads=0;document.getElementById('f').addEventListener('load',()=>frameLoads++)").unwrap();
        // The parent iframe listener mutates the arena shared with this child.
        assert!(page.agent.host.realm_has_pending_images(realm));
        prepare_images(&mut page, realm);
        assert!(!page.agent.host.realm_has_pending_images(realm));
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterFrameReceipt)));
        assert!(
            page.queue_document_load(realm).is_err(),
            "{}",
            publication_state(&page, realm)
        );
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(frameLoads)")
                .unwrap(),
            "1"
        );
        resume(&mut page);
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(frameLoads)")
                .unwrap(),
            "1"
        );
        assert!(!page.pending_document_loads.contains_key(&realm));
    }
    #[test]
    fn dom_navigation_finish_does_not_block_native_load_receipt() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body></body>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://history.test/start").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        reset(&mut page, realm);
        page.agent.host.execute("globalThis.calls=[];document.addEventListener('DOMContentLoaded',()=>{calls.push('dom');navigation.navigate('#during-dom').finished.catch(()=>{})},{once:true});addEventListener('load',()=>calls.push('load'),{once:true})").unwrap();
        page.queue_document_load(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom"
        );
        assert_eq!(page.document_publications[&realm].stage.get(), 2);
        assert!(page.agent.host.realm_has_pending_images(realm));
        // The once DOM listener removes itself during publication. Settle that
        // mutation's image selection without waiting on navigation.finished.
        prepare_images(&mut page, realm);
        assert!(!page.agent.host.realm_has_pending_images(realm));
        resume(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom,load",
            "{}",
            publication_state(&page, realm)
        );
        assert!(!page.pending_document_loads.contains_key(&realm));
        page.checkpoint().unwrap();
        page.queue_document_load(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "dom,load"
        );
    }
    #[test]
    fn construction_retains_child_load_receipt_when_handler_opens_descendant() {
        let mut page =
            RustExecutableDocument::load("<iframe id=f srcdoc='<iframe id=g></iframe>'></iframe>")
                .unwrap();
        let frame = page.agent.dom.borrow().find_by_id("f").unwrap();
        let child = *page
            .frame_realms
            .iter()
            .find(|(_, owner)| owner.frame == frame)
            .unwrap()
            .0;
        page.agent.host.activate_realm(child).unwrap();
        reset(&mut page, child);
        page.agent.host.execute("addEventListener('load',()=>{const document=globalThis.document.getElementById('g').contentDocument;document.open();document.write('<body>waiting')},{once:true})").unwrap();
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterLoadReceipt)));
        assert!(page.queue_document_load(child).is_err());
        let pending = page.document_publications[&child].clone();
        assert_eq!(pending.stage.get(), 4);
        assert!(pending.receipt.get().is_some());
        assert!(!page.document_waits_only_for_author_input(child));
        assert!(
            page.construction_document_publication_pending(child),
            "load callback custody is not an author-input-only wait"
        );
        assert!(pending.retry.get().is_some());
    }

    #[test]
    fn document_open_retires_old_native_lifecycle_receipt() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        let realm = page.top_realm;
        reset(&mut page, realm);
        page.agent
            .host
            .execute("globalThis.loads=0;addEventListener('load',()=>loads++)")
            .unwrap();
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterDomReceipt)));
        assert!(page.queue_document_load(realm).is_err());
        page.agent.host.execute("document.open()").unwrap();
        resume(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("String(loads)").unwrap(),
            "0"
        );
        assert!(!page.pending_document_loads.contains_key(&realm));
    }
    #[test]
    fn frame_execution_receipt_waits_for_parent_load_publication_retry() {
        let mut page =
            RustExecutableDocument::load("<iframe srcdoc='<p>child</p>'></iframe>").unwrap();
        let realm = *page.frame_realms.keys().next().unwrap();
        reset(&mut page, realm);
        let results = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = results.clone();
        let observer = oryn_js::navigation::NavigationExecutionObserver::new(move |value| {
            sink.lock().unwrap().push(value)
        });
        observer.loader_owned();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        page.frame_realms.get_mut(&realm).unwrap().execution = Some((observer, identity));
        LIFECYCLE_FAILURE.with(|failure| failure.set(Some(LifecycleFailure::AfterFrameReceipt)));
        assert!(page.queue_document_load(realm).is_err());
        page.settle_frame_navigation_executions();
        assert!(results.lock().unwrap().is_empty());
        resume(&mut page);
        page.settle_frame_navigation_executions();
        page.settle_frame_navigation_executions();
        assert_eq!(
            *results.lock().unwrap(),
            vec![oryn_js::navigation::NavigationExecutionOutcome::Committed {
                document_generation: identity.1
            }]
        );
    }
    #[test]
    fn replacement_stream_cancels_exact_frame_execution_receipt() {
        let mut page =
            RustExecutableDocument::load("<iframe srcdoc='<p>child</p>'></iframe>").unwrap();
        let realm = *page.frame_realms.keys().next().unwrap();
        let results = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = results.clone();
        let observer = oryn_js::navigation::NavigationExecutionObserver::new(move |value| {
            sink.lock().unwrap().push(value)
        });
        observer.loader_owned();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        page.frame_realms.get_mut(&realm).unwrap().execution = Some((observer, identity));
        page.agent.host.activate_realm(realm).unwrap();
        page.agent.host.execute("document.open()").unwrap();
        page.settle_frame_navigation_executions();
        assert_eq!(
            *results.lock().unwrap(),
            vec![oryn_js::navigation::NavigationExecutionOutcome::Canceled]
        );
    }
}
