//! Receipt-owned blank activation; publication follows actor adoption, never selection.
use super::prerender_policy::ActivationAck;
use super::*;
use crate::context_service::BlankPrerenderActivation;
type Key = (PageId, u64, u64);
pub(super) struct Record {
    pub(super) source: PageId,
    receipt: BlankPrerenderActivation,
    predecessor: Option<oryn_js::message_wire::ParkedDocumentToken>,
    activated: bool,
    initial_discarded: bool,
    published: bool,
    pub(super) acknowledged: bool,
    pub(super) destroyed: bool,
    busy: bool,
}
impl Record {
    pub(super) fn destroy(&mut self) {
        self.destroyed = true;
        self.acknowledged = true;
        self.predecessor = None;
    }
}
fn same(a: &BlankPrerenderActivation, b: &BlankPrerenderActivation) -> bool {
    a.page_id == b.page_id
        && a.token == b.token
        && a.generation == b.generation
        && a.epoch == b.epoch
        && a.url == b.url
}
struct Busy {
    context: BrowserContext,
    key: Key,
}
impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(record) = self
            .context
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .blank_activations
            .get_mut(&self.key)
        {
            record.busy = false;
        }
    }
}
impl BrowserContext {
    fn begin_blank_activation(
        &self,
        source: PageId,
        receipt: &BlankPrerenderActivation,
        create: bool,
    ) -> Result<Busy, RuntimeError> {
        let key = (receipt.page_id, receipt.generation, receipt.epoch);
        {
            let workers = self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("blank activation registry poisoned".into())
            })?;
            if let Some(record) = workers.blank_activations.get(&key) {
                if record.source != source || !same(&record.receipt, receipt) {
                    return Err(RuntimeError::ProtocolViolation(
                        "conflicting blank activation replay".into(),
                    ));
                }
                if record.destroyed {
                    return Err(RuntimeError::PageNotFound(receipt.page_id));
                }
            }
        }
        let pages =
            self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("blank activation pages poisoned".into())
            })?;
        let page = pages
            .get(&receipt.page_id)
            .filter(|page| page.is_live() && page.prerender_source == Some(source))
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation("foreign blank activation destination".into())
            })?;
        let mut workers = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("blank activation registry poisoned".into())
        })?;
        if !workers.blank_activations.contains_key(&key) {
            if !create || !page.hidden_prerender {
                return Err(RuntimeError::ProtocolViolation(
                    "blank activation not accepted".into(),
                ));
            }
            let scope = workers
                .prerenders
                .get(&key)
                .filter(|scope| {
                    scope.token == receipt.token
                        && scope.url == receipt.url
                        && scope.blank_selected
                        && !scope.canceling
                        && scope
                            .blank_source
                            .as_ref()
                            .is_some_and(|(owner, _)| *owner == source)
                })
                .ok_or_else(|| {
                    RuntimeError::ProtocolViolation("unselected blank activation receipt".into())
                })?;
            let _ = scope;
            if workers.blank_activations.len() >= 1024 {
                let completed = workers
                    .blank_activations
                    .iter()
                    .find(|(_, record)| !record.busy && (record.acknowledged || record.destroyed))
                    .map(|(key, _)| *key);
                if let Some(completed) = completed {
                    workers.blank_activations.remove(&completed);
                }
            }
            if workers.blank_activations.len() >= 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "blank activation receipts".into(),
                });
            }
            workers.blank_activations.insert(
                key,
                Record {
                    source,
                    receipt: receipt.clone(),
                    predecessor: None,
                    activated: false,
                    initial_discarded: false,
                    published: false,
                    acknowledged: false,
                    destroyed: false,
                    busy: false,
                },
            );
        }
        let record = workers
            .blank_activations
            .get_mut(&key)
            .ok_or(RuntimeError::NavigationCancelled)?;
        if record.source != source || !same(&record.receipt, receipt) {
            return Err(RuntimeError::ProtocolViolation(
                "conflicting blank activation replay".into(),
            ));
        }
        if record.busy {
            return Err(RuntimeError::ContextService(
                "blank activation already in progress".into(),
            ));
        }
        record.busy = true;
        Ok(Busy {
            context: self.clone(),
            key,
        })
    }
    pub(crate) fn activate_selected_blank_prerender(
        &self,
        source: PageId,
        receipt: &BlankPrerenderActivation,
    ) -> Result<ActivationAck, RuntimeError> {
        let guard = self.begin_blank_activation(source, receipt, true)?;
        let key = guard.key;
        let predecessor = self
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .blank_activations
            .get(&key)
            .ok_or(RuntimeError::NavigationCancelled)?
            .predecessor
            .clone();
        let predecessor = match predecessor {
            Some(token) => token,
            None => {
                let token = self.park_initial_document(receipt.page_id)?;
                // No fallible work separates the native park from retaining its token.
                self.inner
                    .worker_policies
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .blank_activations
                    .get_mut(&key)
                    .ok_or(RuntimeError::NavigationCancelled)?
                    .predecessor = Some(token.clone());
                token
            }
        };
        let ack = self.activate_native_prerender(
            receipt.page_id,
            &receipt.token,
            (receipt.generation, receipt.epoch),
            &predecessor,
        )?;
        #[cfg(test)]
        if FAIL_AFTER_PROMOTION.with(|flag| flag.replace(false)) {
            return Err(RuntimeError::ContextService(
                "injected blank activation ACK loss".into(),
            ));
        }
        if ack == ActivationAck::Current {
            self.inner
                .worker_policies
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .blank_activations
                .get_mut(&key)
                .ok_or(RuntimeError::NavigationCancelled)?
                .activated = true;
        }
        let discarded = self
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .blank_activations
            .get(&key)
            .ok_or(RuntimeError::NavigationCancelled)?
            .initial_discarded;
        if !discarded {
            self.finish_initial_document(receipt.page_id, predecessor, false)?;
            self.inner
                .worker_policies
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .blank_activations
                .get_mut(&key)
                .ok_or(RuntimeError::NavigationCancelled)?
                .initial_discarded = true;
        }
        Ok(ack)
    }
    pub(crate) fn acknowledge_selected_blank_prerender(
        &self,
        source: PageId,
        receipt: &BlankPrerenderActivation,
    ) -> Result<(), RuntimeError> {
        let guard = self.begin_blank_activation(source, receipt, false)?;
        let mut workers = self
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = workers
            .blank_activations
            .get_mut(&guard.key)
            .ok_or(RuntimeError::NavigationCancelled)?;
        if !record.published {
            return Err(RuntimeError::ProtocolViolation(
                "blank acknowledgment before publication".into(),
            ));
        }
        record.acknowledged = true;
        Ok(())
    }
    pub(crate) fn publish_selected_blank_prerender(
        &self,
        source: PageId,
        receipt: &BlankPrerenderActivation,
    ) -> Result<bool, RuntimeError> {
        let guard = self.begin_blank_activation(source, receipt, false)?;
        let key = guard.key;
        let predecessor = {
            let workers = self
                .inner
                .worker_policies
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let record = workers
                .blank_activations
                .get(&key)
                .ok_or(RuntimeError::NavigationCancelled)?;
            if record.published {
                return Ok(true);
            }
            if !record.activated || !record.initial_discarded {
                return Err(RuntimeError::ProtocolViolation(
                    "blank actor publication before activation ACK".into(),
                ));
            }
            record.predecessor.clone().unwrap()
        };
        if self.activate_native_prerender(
            receipt.page_id,
            &receipt.token,
            (receipt.generation, receipt.epoch),
            &predecessor,
        )? != ActivationAck::Current
        {
            return Ok(false);
        }
        self.publish_blank_prerender_page(receipt.page_id)?;
        self.inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .blank_activations
            .get_mut(&key)
            .ok_or(RuntimeError::NavigationCancelled)?
            .published = true;
        Ok(true)
    }
}
#[cfg(test)]
thread_local! {static FAIL_AFTER_PROMOTION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected() -> (BrowserContext, PageId, BlankPrerenderActivation) {
        let context = Browser::native_test_with_config(Default::default()).new_context();
        let source_page = context.new_page();
        let source = source_page.page_id();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                source_page.load_html("https://blank.test/source", "<!doctype html><p>source</p>"),
            )
            .unwrap();
        let creator = context.capture_command_navigation_source(source).unwrap();
        let (destination, _, scope) = context
            .reserve_blank_prerender(source, &creator, "https://blank.test/next?x=one")
            .unwrap();
        context
            .commit_prerender_response_policy(destination, &scope, &scope.url, &[])
            .unwrap();
        context
            .mark_native_prerender_loaded(destination, &scope.token, scope.lifetime)
            .unwrap();
        let receipt = context
            .claim_blank_prerender(source, &creator, Some(destination), &scope.url)
            .unwrap()
            .unwrap();
        (context, source, receipt)
    }
    #[test]
    fn promotion_ack_loss_keeps_one_park_and_publication_waits_for_actor() {
        let (context, source, receipt) = selected();
        let key = (receipt.page_id, receipt.generation, receipt.epoch);
        assert!(
            context
                .publish_selected_blank_prerender(source, &receipt)
                .is_err()
        );
        FAIL_AFTER_PROMOTION.with(|flag| flag.set(true));
        assert!(
            context
                .activate_selected_blank_prerender(source, &receipt)
                .is_err()
        );
        let predecessor = context
            .inner
            .worker_policies
            .lock()
            .unwrap()
            .blank_activations[&key]
            .predecessor
            .clone()
            .unwrap();
        assert!(context.hidden_prerender_page(receipt.page_id));
        assert!(
            context
                .publish_selected_blank_prerender(source, &receipt)
                .is_err()
        );
        // Accepted activation owns completion even when the initiating document retires.
        context
            .install_document_security(source, "https://blank.test/replacement".into(), &[])
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                context
                    .activate_selected_blank_prerender(source, &receipt)
                    .unwrap(),
                ActivationAck::Current
            );
        }
        assert_eq!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .blank_activations[&key]
                .predecessor
                .as_ref(),
            Some(&predecessor)
        );
        assert!(context.hidden_prerender_page(receipt.page_id));
        assert!(
            context
                .publish_selected_blank_prerender(source, &receipt)
                .unwrap()
        );
        assert!(
            context
                .publish_selected_blank_prerender(source, &receipt)
                .unwrap()
        );
        assert!(!context.hidden_prerender_page(receipt.page_id));
        assert_eq!(context.active_page().unwrap().page_id(), receipt.page_id);
        assert!(!context.navigation_resource_grants_drained(source).unwrap());
        context
            .acknowledge_selected_blank_prerender(source, &receipt)
            .unwrap();
        context
            .acknowledge_selected_blank_prerender(source, &receipt)
            .unwrap();
        assert!(context.navigation_resource_grants_drained(source).unwrap());
        context.activate_page(source).unwrap();
        assert!(
            context
                .publish_selected_blank_prerender(source, &receipt)
                .unwrap()
        );
        assert_eq!(
            context.active_page().unwrap().page_id(),
            source,
            "publication replay does not steal focus"
        );
        assert_eq!(
            context
                .pages()
                .iter()
                .filter(|page| page.page.page_id == receipt.page_id)
                .count(),
            1
        );
    }
    #[test]
    fn foreign_or_mutated_receipts_never_park_or_publish() {
        let (context, source, receipt) = selected();
        let other = context.new_page().page_id();
        assert!(
            context
                .activate_selected_blank_prerender(other, &receipt)
                .is_err()
        );
        for field in 0..4 {
            let mut changed = receipt.clone();
            match field {
                0 => changed.generation += 1,
                1 => changed.epoch += 1,
                2 => changed.url.push_str("&extra=yes"),
                _ => changed.page_id = source,
            }
            assert!(
                context
                    .activate_selected_blank_prerender(source, &changed)
                    .is_err()
            );
        }
        assert!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .blank_activations
                .is_empty()
        );
        assert!(context.hidden_prerender_page(receipt.page_id));
        assert_eq!(
            context
                .activate_selected_blank_prerender(source, &receipt)
                .unwrap(),
            ActivationAck::Current
        );
        let mut changed = receipt.clone();
        changed.url.push('#');
        assert!(
            context
                .publish_selected_blank_prerender(source, &changed)
                .is_err()
        );
        assert!(context.hidden_prerender_page(receipt.page_id));
    }
    #[test]
    fn destroyed_destination_tombstone_authenticates_before_terminal_ack() {
        let (context, source, receipt) = selected();
        context
            .activate_selected_blank_prerender(source, &receipt)
            .unwrap();
        context
            .publish_selected_blank_prerender(source, &receipt)
            .unwrap();
        context.clear_page_worker_policies(receipt.page_id);
        assert!(
            matches!(context.acknowledge_selected_blank_prerender(source,&receipt),Err(RuntimeError::PageNotFound(page)) if page==receipt.page_id)
        );
        let mut forged = receipt.clone();
        forged.url.push_str("?forged");
        assert!(matches!(
            context.acknowledge_selected_blank_prerender(source, &forged),
            Err(RuntimeError::ProtocolViolation(_))
        ));
        assert!(matches!(
            context.activate_selected_blank_prerender(PageId(source.0 + 1000), &receipt),
            Err(RuntimeError::ProtocolViolation(_))
        ));
        assert!(context.navigation_resource_grants_drained(source).unwrap());
    }
    #[test]
    fn destination_successor_makes_unpublished_activation_obsolete() {
        let (context, source, receipt) = selected();
        context
            .activate_selected_blank_prerender(source, &receipt)
            .unwrap();
        context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .park_document_authority(receipt.page_id.0, receipt.generation, receipt.epoch)
            .unwrap();
        assert!(
            !context
                .publish_selected_blank_prerender(source, &receipt)
                .unwrap()
        );
        assert!(context.hidden_prerender_page(receipt.page_id));
    }
}
