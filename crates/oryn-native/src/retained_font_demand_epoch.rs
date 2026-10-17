// Native-only original publication custody. Never use DOM mutation as a proxy
// for CSSOM descriptor edits or an Arc address without retaining its owner.
#[derive(Clone)]
pub(crate) struct RetainedFontDemandEpoch {
    publication: Rc<RetainedStylesheetRecord>,
    registry: Weak<RefCell<RetainedStylesheets>>,
    mutation_revision: u64,
    font_revision: u64,
}

impl RetainedFontDemandEpoch {
    pub(super) fn publication(&self)->&Rc<RetainedStylesheetRecord>{&self.publication}
    pub(super) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.publication, &other.publication)
            && self.mutation_revision == other.mutation_revision
            && self.font_revision == other.font_revision
    }
    pub(super) fn current(&self) -> bool {
        self.registry.upgrade().is_some_and(|registry| registry.try_borrow().is_ok_and(|registry|
            registry.records.iter().any(|record| Rc::ptr_eq(record, &self.publication))))
            && self.publication.mutation_revision.get() == self.mutation_revision
            && self
                .publication
                .graph
                .try_borrow()
                .is_ok_and(|graph| graph.font_revision() == self.font_revision)
    }
}
fn retained_font_epoch(
    host: &RawV8Host,
    identity: ParserDocumentIdentity,
    dom: &SharedDomRuntime,
    node: NodeId,
    revision: u64,
) -> Result<Option<RetainedFontDemandEpoch>, HostError> {
    let Some(provider) = host.native_layout_provider_in_realm(identity.realm) else {
        return Ok(None);
    };
    let Some(backend) = provider
        .native_backend()
        .and_then(|backend| backend.downcast_ref::<LiveLayoutProvider>())
    else {
        return Ok(None);
    };
    if !Rc::ptr_eq(&backend.dom, dom) || backend.native_document_identity.get() != Some(identity) {
        return Err(HostError::CachedAuthorityExpired);
    }
    let publication = backend
        .retained_stylesheets
        .try_borrow()
        .map_err(|_| HostError::Execution("font publication registry borrow conflict".into()))?
        .current(dom, identity, node, revision);
    publication
        .map(|publication| {
            let mutation_revision = publication.mutation_revision.get();
            let font_revision = publication
                .graph
                .try_borrow()
                .map_err(|_| HostError::Execution("font publication graph borrow conflict".into()))?
                .font_revision();
        Ok(RetainedFontDemandEpoch {
            publication,
            registry: Rc::downgrade(&backend.retained_stylesheets),
                mutation_revision,
                font_revision,
            })
        })
        .transpose()
}

pub(super) fn retained_font_evaluation_current(host: &RawV8Host, source: &CssDemandBatch) -> bool {
    match retained_font_epoch(
        host,
        source.identity,
        &source.dom,
        source.batch.node,
        source.batch.revision,
    ) {
        Ok(None) => source.evaluated_font.is_none(),
        Ok(Some(current)) => source
            .evaluated_font
            .as_ref()
            .is_some_and(|old| old.same(&current)),
        Err(_) => false, // Owning poll surfaces the error; this only wakes it.
    }
}

#[cfg(test)]
mod retained_font_epoch_controls {
    use super::*;
    #[test]
    fn original_descriptor_mutation_invalidates_broker_epoch_without_dom_mutation() {
        let mut page = RustExecutableDocument::load(
            "<style id=s>@font-face{font-family:Before;src:url(before.woff2)}</style><p>A</p>",
        )
        .unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        let dom = page.agent.dom.clone();
        let identity = ParserFetchContext::capture(&page.agent.host, &dom, node)
            .unwrap()
            .identity();
        let revision = dom.borrow().arena().stylesheet_revision(node).unwrap();
        let cursor = dom.borrow().mutation_cursor();
        let epoch = retained_font_epoch(&page.agent.host, identity, &dom, node, revision)
            .unwrap()
            .expect("original retained stylesheet publication");
        let _admission = page.agent.host.native_retained_admission_for_realm(identity.realm).unwrap().unwrap();
        let _scope = page.agent.host.retained_allocation_scope(identity.realm).unwrap();
        let rule = epoch
            .publication
            .graph
            .borrow()
            .rule_at(
                &epoch.publication.graph.borrow().root(),
                &[],
                0,
                epoch.publication.admission.as_ref(),
            )
            .unwrap();
        epoch
            .publication
            .graph
            .borrow_mut()
            .set_rule_property(
                &rule,
                "src",
                "url(after.woff2)",
                "",
                epoch.publication.admission.as_ref(),
            )
            .unwrap();
        // Graph revision alone rejects a late broker completion. The normal
        // native mapper additionally advances the publication mutation token.
        assert!(!epoch.current());
        assert_eq!(dom.borrow().mutation_cursor(), cursor);
        assert_eq!(
            dom.borrow().arena().stylesheet_revision(node).unwrap(),
            revision
        );
        let next = retained_font_epoch(&page.agent.host, identity, &dom, node, revision)
            .unwrap()
            .unwrap();
        assert!(!epoch.same(&next));
        assert!(next.current());
    }
}
