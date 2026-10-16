// Owning binding invokes this before an inline sheet/computed-style read.
// No host transport, author callback or synthetic owner participates.
impl LiveLayoutProvider {
    fn publish_inline_owner_now(
        &self,
        owner: NodeId,
        document: oryn_js::v8_bindings::BindingStylesheetDocumentOwner,
        lifecycle: Rc<dyn oryn_js::v8_bindings::BindingStylesheetLifecycle>,
        base: &str,
        referrer_policy: &str,
        encoding: &str,
    ) -> Result<bool, String> {
        let identity = ParserDocumentIdentity {
            realm: document.realm, document: document.document,
            document_generation: document.document_generation, stream_epoch: document.stream_epoch,
        };
        if self.native_document_identity.get() != Some(identity) || !lifecycle.current(document) {
            return Ok(false);
        }
        let admission = self.retained_revision_admission()?;
        let (revision,cursor,bytes,quirks) = {
            let dom = self.dom.try_borrow().map_err(|_| "inline publication DOM snapshot conflict")?;
            let node = dom.arena().get(owner).map_err(|e|e.to_string())?;
            if !matches!(&node.kind,NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref()=="style")
                || !node.stylesheet_policy_allowed || !node.stylesheet_parser_ready
                || !dom.arena().is_css_stylesheet_owner(owner).map_err(|e|e.to_string())?
                || !dom.is_connected(owner).map_err(|e|e.to_string())?
                || dom.node_document(owner).map_err(|e|e.to_string())? != Some(document.document)
            { return Ok(false); }
            let revision = dom.arena().stylesheet_revision(owner).map_err(|e|e.to_string())?;
            if self.retained_stylesheets.try_borrow().map_err(|_|"inline registry snapshot conflict")?
                .current(&self.dom,identity,owner,revision).is_some() { return Ok(true); }
            // Effective text uses the same light-tree domain and override
            // precedence as DomArena::stylesheet_text, not only direct children.
            let bytes = base.len().checked_mul(2)
                .and_then(|bytes|bytes.checked_add(referrer_policy.len()))
                .and_then(|bytes|bytes.checked_add(encoding.len()))
                .and_then(|bytes|bytes.checked_add(std::mem::size_of::<RetainedStylesheetResponse>()))
                .and_then(|bytes|bytes.checked_add(std::mem::size_of::<crate::context_service::ResourceCancellation>()))
                .and_then(|bytes|bytes.checked_add(std::mem::size_of::<(Arc<dyn Send+Sync>,Arc<dyn Send+Sync>,Arc<dyn Send+Sync>)>()))
                .and_then(|bytes|bytes.checked_add(8*std::mem::size_of::<usize>()))
                .ok_or("inline source scratch size overflow")?;
            let quirks = dom.arena().get(document.document).map_err(|e|e.to_string())?
                .document_metadata.as_ref().is_some_and(|m|m.quirks);
            (revision,dom.mutation_cursor(),bytes,quirks)
        };
        let metadata_charge = admission.reserve(bytes)?;
        let Some((source,text_charge))=capture_inline_effective_text(&self.dom,owner,cursor,admission.as_ref())?
            else {return Ok(false)};
        // Root is synchronously observable even while imports await the exact
        // already-scheduled owner fetch. Completion reuses these occurrences.
        let graph = ImportGraph::parse(&source,base,quirks,8,self.style_lock.clone(),admission.as_ref())?;
        let root = graph.root();
        let media_charge= {
            let dom=self.dom.try_borrow().map_err(|_|"inline media size conflict")?;
            if dom.mutation_cursor()!=cursor {return Ok(false);}
            let node=dom.arena().get(owner).map_err(|e|e.to_string())?;
            let value=node.stylesheet_media.as_deref().or_else(||node.attributes.get_null("media").map(String::as_str)).unwrap_or("");
            value.len().checked_add(std::mem::size_of::<String>()).ok_or("inline media size overflow")?
        };
        let media_charge=admission.reserve(media_charge)?;
        let (media,disabled) = {
            let dom = self.dom.try_borrow().map_err(|_|"inline root state conflict")?;
            if dom.mutation_cursor()!=cursor { return Ok(false); }
            (dom.arena().stylesheet_media(owner).map_err(|e|e.to_string())?,
             dom.arena().stylesheet_disabled(owner).map_err(|e|e.to_string())?)
        };
        graph.prepare_sheet_media(&root,oryn_layout::cssom::MediaListEdit::Replace(&media),admission.as_ref())?.commit();
        graph.prepare_sheet_disabled(&root,disabled,admission.as_ref())?.commit();
        let snapshot = RetainedPublicationSnapshot::capture(&self.retained_stylesheets,admission.as_ref())
            .map_err(|e|e.to_string())?.ok_or("inline registry changed")?;
        let storage = PreparedRetainedPublication::preflight(snapshot,admission.clone()).map_err(|e|e.to_string())?;
        let source_charge:Arc<dyn Send+Sync>=Arc::new((metadata_charge,text_charge,media_charge));
        let original: Arc<str> = Arc::from(source);
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let mut responses=Vec::new();
        responses.try_reserve_exact(1).map_err(|e|e.to_string())?;
        let response = RetainedStylesheetResponse {
            sheet:Some(root),final_url:base.to_owned(),source:original.clone(),
            referrer_policy:referrer_policy.to_owned(),encoding:encoding.to_owned(),guard:None,
            proof:NativeStylesheetResponseAuthority::Inline(NativeInlineStylesheetAuthority {
                document:identity,node:owner,source_revision:revision,cancellation,
            }),
        };
        responses.push(response);
        let source = RetainedStylesheetSource {
            final_url:base.to_owned(),original:original.clone(),graph,responses,
            dependencies:Vec::new(),retained_allocation:Some(source_charge.clone()),
        };
        let mut prepared = PreparedRetainedPublication::finish_preflight(storage,RetainedStylesheetOwner {
            arena:Rc::downgrade(&self.dom),document:identity,node:owner,revision,lifecycle:lifecycle.clone(),
        },source,admission.clone(),source_charge.clone());
        let stamp = self.dom.try_borrow().map_err(|_|"inline journal snapshot conflict")?.stylesheet_journal_stamp();
        let journal_bytes = stamp.requested_bytes().map_err(|e|e.to_string())?;
        let buffers = stamp.prepare_buffers(admission.reserve(journal_bytes)?,journal_bytes).map_err(|e|e.to_string())?;
        let mut dom_source = oryn_dom::PreparedSheetSourceReplacement::new(
            oryn_dom::OwnedNativeStylesheetSource::admitted(original,source_charge),None,true,buffers);
        admission.checkpoint()?;
        if !lifecycle.current(document) { return Ok(false); }
        {
            let mut registry = self.retained_stylesheets.try_borrow_mut().map_err(|_|"inline registry commit conflict")?;
            if !prepared.snapshot.matches(&registry) { return Ok(false); }
            let mut dom = self.dom.try_borrow_mut().map_err(|_|"inline DOM commit conflict")?;
            if dom.mutation_cursor()!=cursor || dom.node_document(owner).map_err(|e|e.to_string())?!=Some(document.document) {
                return Ok(false);
            }
            let transaction=match dom.prepare_stylesheet_source_replacement(owner,revision,&mut dom_source) {
                Ok(Some(transaction))=>transaction,
                Ok(None)|Err(oryn_dom::StylesheetMetadataPreparationError::Stale)=>return Ok(false),
                Err(oryn_dom::StylesheetMetadataPreparationError::Dom(error))=>return Err(error.to_string()),
            };
            if !prepared.commit(&mut registry) {return Ok(false);}
            transaction.commit();
        }
        // Retired records/journal/source charges drop outside both borrows.
        Ok(true)
    }
}
