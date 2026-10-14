struct OwnedOwnerStateInput {
    flags: (bool, bool, Option<bool>, bool, bool, bool),
    strings: Vec<String>,
    has_override: bool,
    has_title: bool,
    has_xml: bool,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl OwnedOwnerStateInput {
    fn capture(
        record: &RetainedStylesheetRecord,
    ) -> Result<Option<(oryn_dom::MutationCursor, Self)>, HostError> {
        let Some(arena) = record.owner.arena.upgrade() else {
            return Ok(None);
        };
        let (cursor, bytes) = {
            let dom = arena
                .try_borrow()
                .map_err(|_| HostError::Execution("owner input borrow conflict".into()))?;
            if !record.validate_owner_state_cursor(&dom, dom.mutation_cursor())? {
                return Ok(None);
            }
            let input = dom
                .arena()
                .stylesheet_owner_state_input(record.owner.node)
                .map_err(|e| HostError::Execution(e.to_string()))?;
            let bytes = [
                input.media_override,
                Some(input.html_media),
                input.html_title,
                input.selected_set,
                input.xml_pi,
            ]
            .into_iter()
            .try_fold(
                5 * std::mem::size_of::<String>() + std::mem::size_of::<Self>(),
                |bytes, value| bytes.checked_add(value.map_or(0, str::len)),
            )
            .ok_or_else(|| HostError::Execution("owner input size overflow".into()))?;
            (dom.mutation_cursor(), bytes)
        };
        let charge = record
            .admission
            .reserve(bytes)
            .map_err(HostError::Execution)?;
        let mut strings = Vec::new();
        strings
            .try_reserve_exact(5)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let dom = arena
            .try_borrow()
            .map_err(|_| HostError::Execution("owner input copy borrow conflict".into()))?;
        if dom.mutation_cursor() != cursor {
            return Ok(None);
        }
        if !record.validate_owner_state_cursor(&dom, cursor)? {
            return Ok(None);
        }
        let input = dom
            .arena()
            .stylesheet_owner_state_input(record.owner.node)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        for value in [
            input.media_override,
            Some(input.html_media),
            input.html_title,
            input.selected_set,
            input.xml_pi,
        ] {
            let value = value.unwrap_or("");
            let mut string = String::new();
            string
                .try_reserve_exact(value.len())
                .map_err(|e| HostError::Execution(e.to_string()))?;
            string.push_str(value);
            strings.push(string);
        }
        Ok(Some((
            cursor,
            Self {
                flags: (
                    input.parser_ready,
                    input.policy_allowed,
                    input.disabled_override,
                    input.disabled_attribute,
                    input.html_alternate,
                    input.in_document_tree,
                ),
                strings,
                has_override: input.media_override.is_some(),
                has_title: input.html_title.is_some(),
                has_xml: input.xml_pi.is_some(),
                _charge: charge,
            },
        )))
    }
    fn resolve(mut self) -> ResolvedOwnerState {
        let xml = self
            .has_xml
            .then(|| oryn_dom::parse_xml_stylesheet(&self.strings[4]))
            .flatten();
        let alternate = xml.as_ref().map_or(self.flags.4, |sheet| sheet.alternate);
        let mut xml = xml;
        let title = if let Some(title) = xml.as_mut().and_then(|sheet| sheet.title.take()) {
            Some(title)
        } else if self.has_title {
            Some(std::mem::take(&mut self.strings[2]))
        } else {
            None
        };
        let media = if self.has_override {
            std::mem::take(&mut self.strings[0])
        } else if let Some(xml) = xml {
            xml.media
        } else {
            std::mem::take(&mut self.strings[1])
        };
        ResolvedOwnerState {
            title,
            alternate,
            media,
            _input: self,
        }
    }
}
struct ResolvedOwnerState {
    title: Option<String>,
    alternate: bool,
    media: String,
    _input: OwnedOwnerStateInput,
}
impl RetainedStylesheetRecord {
    fn validate_owner_state_cursor(
        &self,
        dom: &oryn_dom::DomRuntime,
        cursor: oryn_dom::MutationCursor,
    ) -> Result<bool, HostError> {
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
            return Ok(false);
        }
        let Ok(node) = dom.arena().get(owner.node) else {
            return Ok(false);
        };
        if dom.mutation_cursor() != cursor
            || node.owner_document != Some(identity.document)
            || !dom
                .is_connected(owner.node)
                .map_err(|e| HostError::Execution(e.to_string()))?
            || dom
                .arena()
                .stylesheet_revision(owner.node)
                .map_err(|e| HostError::Execution(e.to_string()))?
                != owner.revision
        {
            return Ok(false);
        }
        Ok(true)
    }
    fn reconcile_owner_state_admitted(&self) -> Result<Option<bool>, HostError> {
        let Some((cursor, input)) = OwnedOwnerStateInput::capture(self)? else {
            return Ok(None);
        };
        // Private XML parsing happens here, with no DOM borrow held.
        let state = input.resolve();
        let Some(arena) = self.owner.arena.upgrade() else {
            return Ok(None);
        };
        let disabled = {
            let dom = arena
                .try_borrow()
                .map_err(|_| HostError::Execution("owner state read borrow conflict".into()))?;
            if !self.validate_owner_state_cursor(&dom, cursor)? {
                return Ok(None);
            }
            dom.arena()
                .stylesheet_disabled_with_metadata(
                    self.owner.node,
                    state.title.as_deref(),
                    state.alternate,
                )
                .map_err(|e| HostError::Execution(e.to_string()))?
        };
        let graph = self
            .graph
            .try_borrow_mut()
            .map_err(|_| HostError::Execution("owner reconcile graph borrow conflict".into()))?;
        let root = graph.root();
        let media_changed = root.media_text() != state.media;
        let disabled_changed = root.disabled() != disabled;
        if !media_changed && !disabled_changed {
            return Ok(Some(false));
        }
        let revision = self
            .mutation_revision
            .get()
            .checked_add(1)
            .ok_or_else(|| HostError::Execution("owner mutation revision exhausted".into()))?;
        let media = if media_changed {
            Some(
                graph
                    .prepare_sheet_media(
                        &root,
                        oryn_layout::cssom::MediaListEdit::Replace(&state.media),
                        self.admission.as_ref(),
                    )
                    .map_err(HostError::Execution)?,
            )
        } else {
            None
        };
        let disabled = if disabled_changed {
            Some(
                graph
                    .prepare_sheet_disabled(&root, disabled, self.admission.as_ref())
                    .map_err(HostError::Execution)?,
            )
        } else {
            None
        };
        // Admission may have changed DOM. Final read borrow pins it through
        // native commit; every fallible check precedes either state update.
        let dom = arena
            .try_borrow()
            .map_err(|_| HostError::Execution("owner state final borrow conflict".into()))?;
        if !self.validate_owner_state_cursor(&dom, cursor)? {
            return Ok(None);
        }
        if let Some(media) = media {
            media.commit();
        }
        if let Some(disabled) = disabled {
            disabled.commit();
        }
        self.mutation_revision.set(revision);
        Ok(Some(true))
    }
}
