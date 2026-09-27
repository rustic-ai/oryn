//! Incremental fragment tokenizer over an explicit live target. The synthetic
//! html root remains a tokenizer handle; its native target is retained by the
//! parser's migration-aware handle map, so publishing does not clone open nodes.
use super::*;
impl StreamingParser {
    /// Set a native insertion boundary before any tokenizer input. The existing
    /// parser handle map retains both the parent and anchor across callbacks;
    /// a removed/reparented anchor follows the stream-location append fallback.
    pub fn set_fragment_insertion_before(
        &mut self,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        let sink = &mut self.inner.tokenizer.sink.sink;
        let root = sink
            .fragment_root_key
            .get()
            .ok_or(DomError::StaleNode(self.document))?;
        let (owner, target) = sink.resolve(root);
        if let Some(before) = before
            && owner.borrow().parent(before)? != Some(target)
        {
            return Err(DomError::StaleNode(before));
        }
        let target_key = sink.key(&owner, target);
        let before_key = before.map(|node| sink.key(&owner, node));
        sink.stream_locations
            .borrow_mut()
            .insert(root, (target_key, before_key));
        Ok(())
    }
    pub fn with_fragment(
        dom: SharedDomRuntime,
        target: NodeId,
        context: crate::sanitizer::FragmentContext,
        policy: Option<(crate::sanitizer::Config, bool)>,
        run_scripts: bool,
    ) -> Result<Self, DomError> {
        let document = dom
            .borrow()
            .node_document(target)?
            .unwrap_or(dom.borrow().document());
        let staging = dom.borrow_mut().create(NodeKind::Document);
        let sink = OrynTreeSink::with_runtime(dom.clone(), staging);
        // Context construction is inert metadata, preceding token filtering.
        let attrs = context
            .attributes
            .into_iter()
            .map(|attribute| Attribute {
                name: dom_input::name(attribute.name, dom_input::encode),
                value: dom_input::encode(&attribute.value).into(),
            })
            .collect();
        let context_node = markup5ever::interface::tree_builder::create_element(
            &sink,
            dom_input::name(context.name, dom_input::encode),
            attrs,
        );
        if context.disable_context_shadow {
            dom.borrow_mut()
                .arena_mut()
                .get_mut(context_node)?
                .custom_element_shadow_disabled = Some(true);
        }
        let form = context.has_form_ancestor.then(|| {
            markup5ever::interface::tree_builder::create_element(
                &sink,
                QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "form".into()),
                Vec::new(),
            )
        });
        let mut inner = html5ever::parse_fragment_for_element(
            sink,
            decoded_parse_options(),
            context_node,
            true,
            form,
        );
        let root = dom
            .borrow()
            .children(staging)?
            .first()
            .copied()
            .ok_or(DomError::NotAnElement(staging))?;
        dom.borrow_mut().remove_child(staging, root)?;
        let sink = &mut inner.tokenizer.sink.sink;
        sink.parser_roots.replace(root, &dom, target);
        sink.fragment_root_key.set(Some(root));
        sink.document = document;
        sink.key(&dom, document);
        sink.patch_scope.set(Some(target));
        sink.fragment_run_scripts.set(Some(run_scripts));
        *sink.fragment_policy.borrow_mut() = policy;
        Ok(Self {
            inner,
            dom,
            document,
            bytes_seen: 0,
            pending_events: Vec::new(),
            end_started: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_keeps_open_element_identity_across_every_scalar_split() {
        let source = "<section id=s><span>one</span><b>two</b></section>";
        for split in 0..=source.len() {
            if !source.is_char_boundary(split) {
                continue;
            }
            let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let target = dom.borrow_mut().create_element("div");
            let document = dom.borrow().document();
            dom.borrow_mut().append_child(document, target).unwrap();
            let mut parser = StreamingParser::with_fragment(
                dom.clone(),
                target,
                crate::sanitizer::FragmentContext::html_body(),
                None,
                false,
            )
            .unwrap();
            let first = ParserInput::unicode(&source[..split]);
            let progress = parser.push_input_until_resource_or_script(&first);
            assert_eq!(progress.consumed_bytes, first.len());
            let identity = dom.borrow().find_by_id("s");
            let second = ParserInput::unicode(&source[split..]);
            let progress = parser.push_input_until_resource_or_script(&second);
            assert_eq!(progress.consumed_bytes, second.len());
            assert!(parser.prepare_finish());
            if let Some(identity) = identity {
                assert_eq!(dom.borrow().find_by_id("s"), Some(identity));
            }
            assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "onetwo");
        }
    }
    #[test]
    fn stream_policy_filters_customized_identity_before_constructor_and_patch() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = dom.borrow_mut().create_element("div");
        let document = dom.borrow().document();
        dom.borrow_mut().append_child(document, target).unwrap();
        let mut policy = crate::sanitizer::Config::default()
            .canonicalize(true)
            .unwrap();
        policy.remove_attributes = Some(vec![
            crate::sanitizer::Name::attribute("is"),
            crate::sanitizer::Name::attribute("for"),
        ]);
        let mut parser = StreamingParser::with_fragment(
            dom.clone(),
            target,
            crate::sanitizer::FragmentContext::html_body(),
            Some((policy, false)),
            false,
        )
        .unwrap();
        let mut constructions = 0;
        {
            let mut callback = |action| match action {
                ParserAction::Construct { target, .. } => {
                    constructions += 1;
                    Some(target)
                }
                ParserAction::Checkpoint => Some(ParserNode {
                    owner: dom.clone(),
                    node: document,
                }),
                ParserAction::Adopt { target, .. } => Some(target),
                ParserAction::Failed(_) => None,
            };
            let input = ParserInput::unicode(
                "<div is=x-custom><?start name=p>old<?end></div><template for=p>wrong</template>",
            );
            let progress = with_parser_actions(&dom, &mut callback, || {
                parser.push_input_until_resource_or_script(&input)
            });
            assert_eq!(progress.consumed_bytes, input.len());
        }
        assert_eq!(constructions, 0);
        assert!(parser.prepare_finish());
        assert!(
            dom.borrow()
                .arena()
                .text_content(target)
                .unwrap()
                .contains("old")
        );
    }
}

#[cfg(test)]
mod partial_stream_tests {
    use super::*;
    fn feed(parser: &mut StreamingParser, source: &str) {
        let input = ParserInput::unicode(source);
        let progress = parser.push_input_until_resource_or_script(&input);
        assert_eq!(progress.consumed_bytes, input.len());
    }
    #[test]
    fn unbuffered_template_publishes_each_prefix_and_tracks_moved_end_marker() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let root = dom.borrow_mut().create_element("div");
        let document = dom.borrow().document();
        dom.borrow_mut().append_child(document, root).unwrap();
        let mut parser = StreamingParser::with_fragment(
            dom.clone(),
            root,
            crate::sanitizer::FragmentContext::html_body(),
            None,
            false,
        )
        .unwrap();
        feed(
            &mut parser,
            "<div id=p><?start name=p>old<?end></div><template for=p><b id=a>A</b>",
        );
        let target = dom.borrow().find_by_id("p").unwrap();
        let a = dom.borrow().find_by_id("a").unwrap();
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "A");
        let end = *dom.borrow().children(target).unwrap().last().unwrap();
        dom.borrow_mut()
            .insert_before(target, end, Some(a))
            .unwrap();
        feed(&mut parser, "<i>B</i>");
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "BA");
        dom.borrow_mut().remove(end).unwrap();
        feed(&mut parser, "<em>C</em>");
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "BAC");
        dom.borrow_mut().append_child(root, end).unwrap();
        feed(&mut parser, "</template>");
        assert!(parser.prepare_finish());
        assert_eq!(dom.borrow().parent(end).unwrap(), None);
    }
    #[test]
    fn streamed_target_migration_keeps_open_parser_identity_and_cleans_actual_owner_markers() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let destination = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("div");
        let document = source.borrow().document();
        source.borrow_mut().append_child(document, root).unwrap();
        let mut parser = StreamingParser::with_fragment(
            source.clone(),
            root,
            crate::sanitizer::FragmentContext::html_body(),
            None,
            false,
        )
        .unwrap();
        feed(
            &mut parser,
            "<div id=p><?start name=p>old<?end></div><template for=p><b>A</b>",
        );
        let target = source.borrow().find_by_id("p").unwrap();
        let destdoc = destination.borrow().document();
        let mut migration =
            DomRuntime::prepare_subtree_migration(&source, &destination, target, destdoc).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut migration).unwrap();
        destination
            .borrow_mut()
            .append_child(destdoc, moved.root)
            .unwrap();
        feed(&mut parser, "<i id=after>B</i></template>");
        assert!(parser.prepare_finish());
        assert!(source.borrow().find_by_id("after").is_none());
        let after = destination.borrow().find_by_id("after").unwrap();
        assert_eq!(
            destination.borrow().parent(after).unwrap(),
            Some(moved.root)
        );
        assert_eq!(
            destination
                .borrow()
                .arena()
                .text_content(moved.root)
                .unwrap(),
            "AB"
        );
        assert!(
            destination
                .borrow()
                .children(moved.root)
                .unwrap()
                .iter()
                .all(|node| !matches!(
                    destination.borrow().arena().get(*node).unwrap().kind,
                    NodeKind::ProcessingInstruction { .. }
                ))
        );
    }
    #[test]
    fn retained_input_accounting_releases_completed_tokens_and_counts_incomplete_attributes() {
        let mut parser = StreamingParser::new();
        parser.push("<body>");
        let baseline = parser.retained_input_bytes();
        parser.push(&format!("<div title='{}", "x".repeat(8192)));
        assert!(parser.retained_input_bytes() >= 8192);
        parser.push("'></div>");
        assert!(parser.retained_input_bytes() < baseline + 128);
        for _ in 0..128 {
            parser.push("<i>small</i>");
        }
        assert!(parser.retained_input_bytes() < baseline + 128);
    }
}

#[cfg(test)]
mod streaming_admission_tests {
    use super::*;
    #[test]
    fn template_stream_admission_failure_keeps_token_suffix_and_one_publication() {
        for fail in 0..96 {
            let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let root = dom.borrow_mut().create_element("div");
            let document = dom.borrow().document();
            dom.borrow_mut().append_child(document, root).unwrap();
            let mut parser = StreamingParser::with_fragment(
                dom.clone(),
                root,
                crate::sanitizer::FragmentContext::html_body(),
                None,
                false,
            )
            .unwrap();
            let initial = ParserInput::unicode("<div id=p><?start name=p>old<?end></div>");
            assert_eq!(
                parser
                    .push_input_until_resource_or_script(&initial)
                    .consumed_bytes,
                initial.len()
            );
            let mut input = ParserInput::unicode(
                "<template for=p><b id=once>new</b></template><i id=suffix>end</i>",
            );
            crate::insertion::set_failure_after(Some(fail));
            let progress = parser.push_input_until_resource_or_script(&input);
            crate::insertion::set_failure_after(None);
            input = input.split_off(progress.consumed_bytes);
            let progress = parser.push_input_until_resource_or_script(&input);
            assert_eq!(progress.consumed_bytes, input.len());
            assert!(parser.prepare_finish());
            let target = dom.borrow().find_by_id("p").unwrap();
            let child = dom.borrow().find_by_id("once").unwrap();
            assert_eq!(dom.borrow().parent(child).unwrap(), Some(target));
            assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "new");
            assert!(dom.borrow().find_by_id("suffix").is_some());
        }
    }
}

/// Attribute snapshot admitted before an external fetch starts.
#[derive(Clone, Debug)]
pub struct ExternalTemplateOptions {
    pub name: String,
    pub sanitize: bool,
}
impl ExternalTemplateOptions {
    pub fn capture(dom: &DomRuntime, template: NodeId) -> Result<Self, DomError> {
        Ok(Self {
            name: dom
                .attribute_in_null_namespace(template, "for")?
                .unwrap_or("")
                .to_owned(),
            sanitize: dom
                .attribute_in_null_namespace(template, "sanitize")?
                .is_some_and(|value| value != "unsafe"),
        })
    }
}
impl StreamingParser {
    /// Start after the resource owner admits the external response (and SRI for
    /// buffered delivery). Fetch policy, cancellation and byte limits remain
    /// with that owner; this continuation only owns native parsing/publication.
    pub fn with_external_template(
        dom: SharedDomRuntime,
        template: NodeId,
        policy: Option<(crate::sanitizer::Config, bool)>,
        run_scripts: bool,
    ) -> Result<Option<Self>, DomError> {
        let options = ExternalTemplateOptions::capture(&dom.borrow(), template)?;
        Self::with_external_template_captured(dom, template, options, policy, run_scripts)
    }
    pub fn with_external_template_captured(
        dom: SharedDomRuntime,
        template: NodeId,
        options: ExternalTemplateOptions,
        policy: Option<(crate::sanitizer::Config, bool)>,
        run_scripts: bool,
    ) -> Result<Option<Self>, DomError> {
        let mut scope = template;
        loop {
            let native = dom.borrow();
            if matches!(
                native.arena().get(scope)?.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                break;
            }
            let Some(parent) = native.parent(scope)? else {
                return Ok(None);
            };
            scope = parent;
        }
        let patch = partial_updates::StreamingPatch::external_named(
            &dom,
            &mut dom.borrow_mut(),
            scope,
            template,
            &options.name,
        )?;
        let Some(patch) = patch else {
            return Ok(None);
        };
        let parent = patch.parent;
        let before = patch.before;
        let context_node = {
            let native = dom.borrow();
            if matches!(
                native.arena().get(parent)?.kind,
                NodeKind::ShadowRoot { .. }
            ) {
                native.parent(parent)?.unwrap_or(parent)
            } else {
                parent
            }
        };
        let context =
            if dom
                .borrow()
                .is_element(context_node, "http://www.w3.org/1999/xhtml", "template")
            {
                crate::sanitizer::FragmentContext::html_body()
            } else {
                crate::sanitizer::FragmentContext::capture(&dom.borrow(), context_node)
                    .unwrap_or_else(|_| crate::sanitizer::FragmentContext::html_body())
            };
        let mut parser = Self::with_fragment(dom.clone(), parent, context, policy, run_scripts)?;
        let sink = &mut parser.inner.tokenizer.sink.sink;
        if options.sanitize {
            *sink.fragment_extra_policy.borrow_mut() =
                Some((crate::sanitizer::Config::default_preset(), true));
        }
        let root = sink
            .fragment_root_key
            .get()
            .expect("fragment tokenizer root");
        let parent_key = sink.key(&dom, parent);
        let template_key = sink.key(&dom, template);
        let before_key = before.map(|node| sink.key(&dom, node));
        let cleanup = patch
            .cleanup_nodes()
            .iter()
            .map(|node| sink.key(&dom, *node))
            .collect();
        let receipt = std::rc::Rc::new(RefCell::new(patch));
        sink.stream_locations
            .borrow_mut()
            .insert(root, (parent_key, before_key));
        sink.stream_cleanup
            .borrow_mut()
            .insert(template_key, cleanup);
        sink.stream_patches
            .borrow_mut()
            .insert(template_key, receipt.clone());
        sink.external_template.set(Some(template_key));
        sink.queue_mutation(ParserMutation::StreamPatch {
            node: template_key,
            receipt,
        });
        Ok(Some(parser))
    }
}

#[cfg(test)]
mod external_template_tests {
    use super::*;
    #[test]
    fn external_template_discovery_preserves_old_target_and_main_parser_progress() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let mut input = ParserInput::unicode(
            "<body><div id=p><?start name=p>old<?end></div><template id=t for=p src=remote.html></template><i id=later>main</i>",
        );
        let mut discovered = Vec::new();
        while !input.is_empty() {
            let progress = parser.push_input_until_resource_or_script(&input);
            input = input.split_off(progress.consumed_bytes);
            discovered.extend(parser.take_events());
        }
        let target = dom.borrow().find_by_id("p").unwrap();
        let template = dom.borrow().find_by_id("t").unwrap();
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "old");
        assert!(dom.borrow().is_connected(template).unwrap());
        assert!(dom.borrow().find_by_id("later").is_some());
        assert_eq!(discovered.iter().filter(|event|matches!(event,ParserEvent::ExternalTemplateDiscovered{node}if *node==template)).count(),1);
    }
    #[test]
    fn external_response_keeps_identity_across_chunks_and_cleans_only_on_eof() {
        let dom=std::rc::Rc::new(RefCell::new(parse_inert_dom_string_with_declarative_shadow_unpatched("<div id=p><?start name=p>old<?end></div><template id=t for=p src=remote.html></template>").into_runtime().unwrap()));
        let target = dom.borrow().find_by_id("p").unwrap();
        let template = dom.borrow().find_by_id("t").unwrap();
        let mut parser =
            StreamingParser::with_external_template(dom.clone(), template, None, false)
                .unwrap()
                .unwrap();
        let first = ParserInput::unicode("<b id=first>A");
        assert_eq!(
            parser
                .push_input_until_resource_or_script(&first)
                .consumed_bytes,
            first.len()
        );
        let first_node = dom.borrow().find_by_id("first").unwrap();
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "A");
        assert!(dom.borrow().is_connected(template).unwrap());
        let last = ParserInput::unicode("B</b><i>C</i>");
        assert_eq!(
            parser
                .push_input_until_resource_or_script(&last)
                .consumed_bytes,
            last.len()
        );
        assert_eq!(dom.borrow().find_by_id("first"), Some(first_node));
        assert!(parser.prepare_finish());
        assert!(!dom.borrow().is_connected(template).unwrap());
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "ABC");
    }
}

pub(super) struct BufferedExternal {
    template: NodeId,
    staging: NodeId,
    scope: NodeId,
    name: String,
    pub(super) queued: bool,
}
impl StreamingParser {
    /// Buffered responses parse off-tree. No target prefix is observable between
    /// bounded input slices; EOF admits one retained marker publication.
    pub fn with_external_template_buffered(
        dom: SharedDomRuntime,
        template: NodeId,
        policy: Option<(crate::sanitizer::Config, bool)>,
        run_scripts: bool,
    ) -> Result<Option<Self>, DomError> {
        let options = ExternalTemplateOptions::capture(&dom.borrow(), template)?;
        Self::with_external_template_buffered_captured(dom, template, options, policy, run_scripts)
    }
    pub fn with_external_template_buffered_captured(
        dom: SharedDomRuntime,
        template: NodeId,
        options: ExternalTemplateOptions,
        policy: Option<(crate::sanitizer::Config, bool)>,
        run_scripts: bool,
    ) -> Result<Option<Self>, DomError> {
        let mut scope = template;
        loop {
            let native = dom.borrow();
            if matches!(
                native.arena().get(scope)?.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                break;
            }
            let Some(parent) = native.parent(scope)? else {
                return Ok(None);
            };
            scope = parent;
        }
        let preview = partial_updates::StreamingPatch::external_named(
            &dom,
            &mut dom.borrow_mut(),
            scope,
            template,
            &options.name,
        )?;
        let Some(mut preview) = preview else {
            return Ok(None);
        };
        let parent = preview.parent;
        preview.abort(&mut dom.borrow_mut());
        let context_node = {
            let native = dom.borrow();
            if matches!(
                native.arena().get(parent)?.kind,
                NodeKind::ShadowRoot { .. }
            ) {
                native.parent(parent)?.unwrap_or(parent)
            } else {
                parent
            }
        };
        let context = crate::sanitizer::FragmentContext::capture(&dom.borrow(), context_node)
            .unwrap_or_else(|_| crate::sanitizer::FragmentContext::html_body());
        let staging = dom.borrow_mut().create_fragment();
        let document = dom
            .borrow()
            .node_document(template)?
            .ok_or(DomError::MigrationChanged)?;
        dom.borrow_mut().set_node_document(staging, document)?;
        let mut parser = Self::with_fragment(dom.clone(), staging, context, policy, run_scripts)?;
        let sink = &mut parser.inner.tokenizer.sink.sink;
        sink.patches_enabled.set(false);
        if options.sanitize {
            *sink.fragment_extra_policy.borrow_mut() =
                Some((crate::sanitizer::Config::default_preset(), true));
        }
        let name = options.name;
        let template = sink.key(&dom, template);
        let staging = sink.key(&dom, staging);
        let scope = sink.key(&dom, scope);
        *sink.external_buffered.borrow_mut() = Some(BufferedExternal {
            template,
            staging,
            scope,
            name,
            queued: false,
        });
        Ok(Some(parser))
    }
}
impl OrynTreeSink {
    pub(super) fn prepare_buffered_external(&self) -> bool {
        let pending = self
            .external_buffered
            .borrow()
            .as_ref()
            .filter(|state| !state.queued)
            .map(|state| {
                (
                    state.template,
                    state.staging,
                    state.scope,
                    state.name.clone(),
                )
            });
        let Some((template, staging, scope, name)) = pending else {
            return true;
        };
        let result = (|| -> Result<(), DomError> {
            let (owner, node) = self.resolve(template);
            let (staging_owner, content) = self.resolve(staging);
            let (scope_owner, scope) = self.resolve(scope);
            if !std::rc::Rc::ptr_eq(&owner, &staging_owner)
                || !std::rc::Rc::ptr_eq(&owner, &scope_owner)
                || !owner.borrow().is_connected(node)?
            {
                return Err(DomError::MigrationChanged);
            }
            let patch = partial_updates::PatchCursor::external_content(
                &owner,
                &mut owner.borrow_mut(),
                scope,
                node,
                content,
                &name,
            )?;
            self.external_buffered
                .borrow_mut()
                .as_mut()
                .expect("buffered source custody")
                .queued = true;
            if let Some(patch) = patch {
                self.external_cleanup_started.set(true);
                self.queue_mutation(ParserMutation::Patch {
                    node: template,
                    receipt: std::rc::Rc::new(RefCell::new(patch)),
                });
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.mutation_failed.set(true);
            parser_action(&self.dom, ParserAction::Failed(error));
            false
        } else {
            !self.mutation_failed.get()
        }
    }
}

#[cfg(test)]
mod buffered_external_tests {
    use super::*;
    fn fixture() -> (SharedDomRuntime, NodeId, NodeId) {
        let dom=std::rc::Rc::new(RefCell::new(parse_inert_dom_string_with_declarative_shadow_unpatched("<div id=p><?start name=p>old<?end></div><template id=t for=p src=remote.html buffer></template>").into_runtime().unwrap()));
        let target = dom.borrow().find_by_id("p").unwrap();
        let template = dom.borrow().find_by_id("t").unwrap();
        (dom, target, template)
    }
    #[test]
    fn buffered_external_response_is_invisible_until_complete_native_publication() {
        let (dom, target, template) = fixture();
        let mut parser =
            StreamingParser::with_external_template_buffered(dom.clone(), template, None, true)
                .unwrap()
                .unwrap();
        for chunk in ["<b id=first>A", "B</b><script>void 0</script>", "<i>C</i>"] {
            let input = ParserInput::unicode(chunk);
            let progress = parser.push_input_until_resource_or_script(&input);
            assert_eq!(progress.consumed_bytes, input.len());
            assert!(parser.take_events().is_empty());
            assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "old");
            assert!(!parser.external_template_cleanup_started());
        }
        assert!(parser.prepare_finish());
        assert!(parser.external_template_cleanup_started());
        assert!(!dom.borrow().is_connected(template).unwrap());
        let first = dom.borrow().find_by_id("first").unwrap();
        assert_eq!(dom.borrow().parent(first).unwrap(), Some(target));
        assert!(
            parser
                .take_events()
                .iter()
                .any(|event| matches!(event, ParserEvent::ScriptReady { .. }))
        );
    }
    #[test]
    fn buffered_external_publication_failures_keep_staged_identity_and_do_not_reparse() {
        for failure in 0..96 {
            crate::insertion::set_failure_after(None);
            let (dom, target, template) = fixture();
            let mut parser = StreamingParser::with_external_template_buffered(
                dom.clone(),
                template,
                None,
                false,
            )
            .unwrap()
            .unwrap();
            let input = ParserInput::unicode("<b>new</b>");
            assert_eq!(
                parser
                    .push_input_until_resource_or_script(&input)
                    .consumed_bytes,
                input.len()
            );
            let staging = parser
                .inner
                .tokenizer
                .sink
                .sink
                .external_buffered
                .borrow()
                .as_ref()
                .unwrap()
                .staging;
            let (_, staging) = parser.inner.tokenizer.sink.sink.resolve(staging);
            let child = dom.borrow().children(staging).unwrap()[0];
            crate::insertion::set_failure_after(Some(failure));
            let completed = parser.prepare_finish();
            crate::insertion::set_failure_after(None);
            if !completed {
                assert!(parser.has_pending_mutations());
                assert!(parser.prepare_finish());
            }
            assert_eq!(dom.borrow().children(target).unwrap(), &[child]);
            assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "new");
            assert!(parser.external_template_cleanup_started());
        }
    }
    #[test]
    fn removed_buffered_template_cancels_without_publishing_staged_prefix() {
        let (dom, target, template) = fixture();
        let mut parser =
            StreamingParser::with_external_template_buffered(dom.clone(), template, None, false)
                .unwrap()
                .unwrap();
        let input = ParserInput::unicode("<b>unpublished</b>");
        assert_eq!(
            parser
                .push_input_until_resource_or_script(&input)
                .consumed_bytes,
            input.len()
        );
        dom.borrow_mut().remove(template).unwrap();
        assert!(!parser.prepare_finish());
        assert!(!parser.external_template_cleanup_started());
        drop(parser);
        crate::drain_tracked_node_leases().unwrap();
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "old");
    }
}

#[cfg(test)]
mod stack_close_notification_regressions {
    use super::*;

    #[test]
    fn external_template_close_and_eof_publish_exactly_one_discovery() {
        for explicit_close in [false, true] {
            let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let mut parser = StreamingParser::with_runtime(dom.clone());
            let suffix = if explicit_close {
                "</template><p id=after>tail</p>"
            } else {
                ""
            };
            let mut input = ParserInput::unicode(&format!(
                "<body><div><?marker name=p></div><template id=t for=p src=fragment.html>{suffix}"
            ));
            let mut events = Vec::new();
            while !input.is_empty() {
                let progress = parser.push_input_until_resource_or_script(&input);
                input = input.split_off(progress.consumed_bytes);
                events.extend(parser.take_events());
            }
            let count = |events: &Vec<ParserEvent>| {
                events
                    .iter()
                    .filter(|event| matches!(event, ParserEvent::ExternalTemplateDiscovered { .. }))
                    .count()
            };
            assert_eq!(
                count(&events),
                usize::from(explicit_close),
                "explicit close must notify before EOF"
            );
            assert!(parser.prepare_finish());
            events.extend(parser.take_events());
            assert_eq!(
                count(&events),
                1,
                "EOF must neither miss nor repeat template close"
            );
            assert!(parser.prepare_finish());
            assert!(parser.take_events().is_empty());
            assert!(
                dom.borrow().find_by_id("t").is_some(),
                "discovery preserves the template until its resource is admitted"
            );
        }
    }

    #[test]
    fn unbuffered_template_eof_cleans_markers_after_publishing_prefix() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let root = dom.borrow_mut().create_element("div");
        let document = dom.borrow().document();
        dom.borrow_mut().append_child(document, root).unwrap();
        let mut parser = StreamingParser::with_fragment(
            dom.clone(),
            root,
            crate::sanitizer::FragmentContext::html_body(),
            None,
            false,
        )
        .unwrap();
        let input = ParserInput::unicode(
            "<div id=p><?start name=p>old<?end></div><template for=p><b>new</b>",
        );
        assert_eq!(
            parser
                .push_input_until_resource_or_script(&input)
                .consumed_bytes,
            input.len()
        );
        let target = dom.borrow().find_by_id("p").unwrap();
        assert_eq!(dom.borrow().arena().text_content(target).unwrap(), "new");
        assert!(parser.prepare_finish());
        assert!(
            dom.borrow()
                .children(target)
                .unwrap()
                .iter()
                .all(|node| !matches!(
                    dom.borrow().arena().get(*node).unwrap().kind,
                    NodeKind::ProcessingInstruction { .. }
                ))
        );
        assert!(
            dom.borrow()
                .query_selector_all(root, "template")
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod insertion_boundary_tests {
    use super::*;
    #[test]
    fn fragment_insertion_boundary_orders_text_and_open_nodes_across_chunks() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let root = dom.borrow_mut().create_element("div");
        let tail = dom.borrow_mut().create_element("i");
        dom.borrow_mut().append_child(root, tail).unwrap();
        let mut parser = StreamingParser::with_fragment(
            dom.clone(),
            root,
            crate::sanitizer::FragmentContext::html_body(),
            None,
            false,
        )
        .unwrap();
        parser.set_fragment_insertion_before(Some(tail)).unwrap();
        for chunk in ["A<b>", "B", "C</b>D"] {
            let input = ParserInput::unicode(chunk);
            assert_eq!(
                parser
                    .push_input_until_resource_or_script(&input)
                    .consumed_bytes,
                input.len()
            );
        }
        assert!(parser.prepare_finish());
        let native = dom.borrow();
        assert_eq!(native.children(root).unwrap().last(), Some(&tail));
        assert_eq!(native.arena().text_content(root).unwrap(), "ABCD");
        assert_eq!(native.children(root).unwrap().len(), 4);
    }
    #[test]
    fn fragment_insertion_rejects_foreign_anchor_before_input() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let root = dom.borrow_mut().create_element("div");
        let other = dom.borrow_mut().create_element("div");
        let anchor = dom.borrow_mut().create_element("i");
        dom.borrow_mut().append_child(other, anchor).unwrap();
        let mut parser = StreamingParser::with_fragment(
            dom.clone(),
            root,
            crate::sanitizer::FragmentContext::html_body(),
            None,
            false,
        )
        .unwrap();
        assert!(parser.set_fragment_insertion_before(Some(anchor)).is_err());
        assert!(dom.borrow().children(root).unwrap().is_empty());
        assert_eq!(dom.borrow().parent(anchor).unwrap(), Some(other));
    }
}
