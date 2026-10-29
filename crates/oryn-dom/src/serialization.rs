//! Read-only serializers consume arena-qualified records and template edges.
use crate::*;
/// Implementations must return stable native records without running script or
/// mutating DOM. A bridge may capture external edges before entering serialization.
pub trait SerializationResolver {
    fn node(&self, node: NodeRef) -> Result<Node, DomError>;
    /// Override when registry classification is supplied by a binding realm.
    fn non_global_shadow_registry(&self, node: NodeRef) -> Result<bool, DomError> {
        Ok(self.node(node)?.custom_element_registry.is_some())
    }
    fn template_content(&self, node: NodeRef) -> Result<Option<NodeRef>, DomError> {
        Ok(self.node(node)?.template_content.map(|content| NodeRef {
            arena: node.arena,
            node: content,
        }))
    }
}
pub(crate) struct LocalResolver<'a>(pub &'a DomRuntime);
impl SerializationResolver for LocalResolver<'_> {
    fn non_global_shadow_registry(&self, node: NodeRef) -> Result<bool, DomError> {
        self.node(node)?;
        self.0.has_non_global_shadow_registry(node.node)
    }
    fn node(&self, node: NodeRef) -> Result<Node, DomError> {
        if node.arena != self.0.arena().id() {
            return Err(DomError::WrongArena {
                expected: self.0.arena().id(),
                actual: node.arena,
            });
        }
        Ok(self.0.arena().get(node.node)?.clone())
    }
}
pub(crate) fn concat(parts: &[&str]) -> String {
    dom_string::from_units(
        &parts
            .iter()
            .flat_map(|part| dom_string::units(part))
            .collect::<Vec<_>>(),
    )
}
pub(crate) fn append(output: &mut String, value: &str) {
    if !output.starts_with(dom_string::PREFIX) && !value.starts_with(dom_string::PREFIX) {
        output.push_str(value)
    } else {
        *output = concat(&[output, value]);
    }
}
pub(crate) fn escaped(value: &str, attribute: bool, xml: bool) -> String {
    let mut units = Vec::new();
    for unit in dom_string::units(value) {
        let replacement = match unit {
            38 => Some("&amp;"),
            60 if xml || !attribute => Some("&lt;"),
            62 if xml || !attribute => Some("&gt;"),
            34 if attribute => Some("&quot;"),
            160 if !xml => Some("&nbsp;"),
            9 if xml && attribute => Some("&#x9;"),
            10 if xml && attribute => Some("&#xA;"),
            13 if xml => Some("&#xD;"),
            _ => None,
        };
        if let Some(value) = replacement {
            units.extend(value.encode_utf16())
        } else {
            units.push(unit)
        }
    }
    dom_string::from_units(&units)
}
/// Serialize with an explicit traversal stack: valid DOM depth is not limited
/// by the Rust call stack. The active path detects malformed ownership cycles
/// while allowing the same record in independent serialization roots.
#[derive(Default)]
pub struct HtmlSerializationOptions {
    pub serializable_shadow_roots: bool,
    pub shadow_roots: BTreeSet<NodeRef>,
}
pub fn serialize_html_with_resolver(
    root: NodeRef,
    resolver: &impl SerializationResolver,
) -> Result<String, DomError> {
    serialize_html_with_options(root, resolver, &HtmlSerializationOptions::default(), false)
}
pub fn serialize_html_with_options(
    root: NodeRef,
    resolver: &impl SerializationResolver,
    options: &HtmlSerializationOptions,
    children_only: bool,
) -> Result<String, DomError> {
    enum Task {
        Visit(NodeRef, bool),
        Leave(NodeRef),
        Open(String),
        Close(String),
    }
    fn emit(output: &mut Vec<u16>, value: &str) {
        output.extend(dom_string::units(value));
    }
    let mut output = Vec::new();
    let mut active = BTreeSet::new();
    let mut pending = vec![Task::Visit(root, !children_only)];
    while let Some(task) = pending.pop() {
        let (node, emit_self) = match task {
            Task::Leave(node) => {
                active.remove(&node);
                continue;
            }
            Task::Close(tag) => {
                emit(&mut output, "</");
                emit(&mut output, &tag);
                emit(&mut output, ">");
                continue;
            }
            Task::Open(text) => {
                emit(&mut output, &text);
                continue;
            }
            Task::Visit(node, emit_self) => (node, emit_self),
        };
        if !active.insert(node) {
            return Err(DomError::Cycle);
        }
        pending.push(Task::Leave(node));
        let record = resolver.node(node)?;
        match &record.kind {
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => {
                pending.extend(record.children.iter().rev().map(|child| {
                    Task::Visit(
                        NodeRef {
                            arena: node.arena,
                            node: *child,
                        },
                        true,
                    )
                }));
            }
            NodeKind::DocumentType { name, .. } => {
                emit(&mut output, "<!DOCTYPE ");
                emit(&mut output, name);
                emit(&mut output, ">");
            }
            NodeKind::Text { data } => {
                let raw=record.parent.map(|parent|resolver.node(NodeRef{arena:node.arena,node:parent})).transpose()?
                    .is_some_and(|parent|matches!(&parent.kind,NodeKind::Element{name}
                        if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"
                        &&matches!(name.local.as_ref(),"script"|"style"|"xmp"|"iframe"|"noembed"|"noframes"|"plaintext")));
                emit(
                    &mut output,
                    &if raw {
                        data.clone()
                    } else {
                        escaped(data, false, false)
                    },
                );
            }
            NodeKind::CDataSection { data } => {
                emit(&mut output, &concat(&["<![CDATA[", data, "]]>"]))
            }
            NodeKind::ProcessingInstruction { target, data } => {
                emit(&mut output, &concat(&["<?", target, " ", data, "?>"]))
            }
            NodeKind::Comment { data } => emit(&mut output, &concat(&["<!--", data, "-->"])),
            NodeKind::Attribute { data, .. } => {
                emit(&mut output, &escaped(&data.value, true, false))
            }
            NodeKind::Element { name } => {
                let tag = if matches!(
                    name.ns.as_ref(),
                    "http://www.w3.org/1999/xhtml"
                        | "http://www.w3.org/2000/svg"
                        | "http://www.w3.org/1998/Math/MathML"
                ) {
                    name.local.to_string()
                } else {
                    name.prefix.as_ref().map_or_else(
                        || name.local.to_string(),
                        |prefix| format!("{prefix}:{}", name.local),
                    )
                };
                if emit_self {
                    emit(&mut output, "<");
                    emit(&mut output, &tag);
                    // The creation-time value survives removal of the content
                    // attribute. An authored attribute, including an empty one,
                    // takes precedence and retains its ordinary attribute order.
                    if !record.attributes.contains_null("is")
                        && let Some(value) = &record.custom_element_is
                    {
                        emit(&mut output, " is=\"");
                        emit(&mut output, &escaped(value, true, false));
                        emit(&mut output, "\"");
                    }
                    for attribute in record.attributes.records() {
                        let key = match attribute.namespace.as_deref() {
                            Some("http://www.w3.org/XML/1998/namespace") => {
                                format!("xml:{}", attribute.local_name)
                            }
                            Some("http://www.w3.org/2000/xmlns/")
                                if attribute.local_name == "xmlns" =>
                            {
                                "xmlns".into()
                            }
                            Some("http://www.w3.org/2000/xmlns/") => {
                                format!("xmlns:{}", attribute.local_name)
                            }
                            Some("http://www.w3.org/1999/xlink") => {
                                format!("xlink:{}", attribute.local_name)
                            }
                            _ => attribute.name.clone(),
                        };
                        emit(&mut output, " ");
                        emit(&mut output, &key);
                        emit(&mut output, "=\"");
                        emit(&mut output, &escaped(&attribute.value, true, false));
                        emit(&mut output, "\"");
                    }
                    emit(&mut output, ">");
                }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && is_void_html_element(&name.local)
                {
                    continue;
                }
                if emit_self {
                    pending.push(Task::Close(tag));
                }
                let content = if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && name.local.as_ref() == "template"
                {
                    resolver.template_content(node)?
                } else {
                    None
                };
                if let Some(content) = content {
                    if !matches!(resolver.node(content)?.kind, NodeKind::DocumentFragment) {
                        return Err(DomError::Cycle);
                    }
                    pending.push(Task::Visit(content, true));
                } else {
                    for child in record.children.iter().rev() {
                        let child = NodeRef {
                            arena: node.arena,
                            node: *child,
                        };
                        if !matches!(resolver.node(child)?.kind, NodeKind::ShadowRoot { .. }) {
                            pending.push(Task::Visit(child, true));
                        }
                    }
                    // A selected shadow tree precedes all light children,
                    // irrespective of its native child-list position.
                    for child in &record.children {
                        let shadow = NodeRef {
                            arena: node.arena,
                            node: *child,
                        };
                        let shadow_record = resolver.node(shadow)?;
                        let NodeKind::ShadowRoot { mode, clonable } = shadow_record.kind else {
                            continue;
                        };
                        if !options.shadow_roots.contains(&shadow)
                            && !(options.serializable_shadow_roots
                                && shadow_record.shadow_serializable)
                        {
                            continue;
                        }
                        let mut opening = format!(
                            "<template shadowrootmode=\"{}\"",
                            if mode == ShadowMode::Open {
                                "open"
                            } else {
                                "closed"
                            }
                        );
                        if shadow_record.delegates_focus {
                            opening.push_str(" shadowrootdelegatesfocus=\"\"");
                        }
                        if shadow_record.manual_slot_assignment {
                            opening.push_str(" shadowrootslotassignment=\"manual\"");
                        }
                        if shadow_record.shadow_serializable {
                            opening.push_str(" shadowrootserializable=\"\"");
                        }
                        if clonable {
                            opening.push_str(" shadowrootclonable=\"\"");
                        }
                        if let Some(target) = &shadow_record.shadow_reference_target {
                            append(
                                &mut opening,
                                &concat(&[
                                    " shadowrootreferencetarget=\"",
                                    &escaped(target, true, false),
                                    "\"",
                                ]),
                            );
                        }
                        if let Some(specifiers) =
                            &shadow_record.shadow_adopted_stylesheet_specifiers
                        {
                            append(
                                &mut opening,
                                &concat(&[
                                    " shadowrootadoptedstylesheets=\"",
                                    &escaped(specifiers, true, false),
                                    "\"",
                                ]),
                            );
                        }
                        if resolver.non_global_shadow_registry(shadow)? {
                            append(&mut opening, " shadowrootcustomelementregistry=\"\"");
                        }
                        append(&mut opening, ">");
                        pending.push(Task::Close("template".into()));
                        pending.push(Task::Visit(shadow, true));
                        pending.push(Task::Open(opening));
                        break;
                    }
                }
            }
        }
    }
    Ok(dom_string::from_units(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_shadow_serialization_precedes_light_children_and_preserves_flags() {
        let mut dom = DomRuntime::new();
        let host = dom.create_element("div");
        let light = dom.create_text("light<&");
        dom.append_child(host, light).unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: true,
        });
        dom.append_child(host, shadow).unwrap();
        {
            let record = dom.arena_mut().get_mut(shadow).unwrap();
            record.shadow_serializable = true;
            record.delegates_focus = true;
            record.shadow_reference_target = Some("target\"&".into());
        }
        let text = dom.create_text("shadow");
        dom.append_child(shadow, text).unwrap();
        let options = HtmlSerializationOptions {
            serializable_shadow_roots: true,
            ..Default::default()
        };
        let expected = "<template shadowrootmode=\"closed\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootclonable=\"\" shadowrootreferencetarget=\"target&quot;&amp;\">shadow</template>light&lt;&amp;";
        assert_eq!(
            serialize_html_with_options(identity(&dom, host), &LocalResolver(&dom), &options, true)
                .unwrap(),
            expected
        );
        assert_eq!(dom.serialize(host).unwrap(), "<div>light&lt;&amp;</div>");
        dom.arena_mut().get_mut(shadow).unwrap().shadow_serializable = false;
        assert_eq!(
            serialize_html_with_options(identity(&dom, host), &LocalResolver(&dom), &options, true)
                .unwrap(),
            "light&lt;&amp;"
        );
        let explicit = HtmlSerializationOptions {
            shadow_roots: BTreeSet::from([identity(&dom, shadow)]),
            ..Default::default()
        };
        assert_eq!(
            serialize_html_with_options(
                identity(&dom, host),
                &LocalResolver(&dom),
                &explicit,
                true
            )
            .unwrap(),
            expected.replace(" shadowrootserializable=\"\"", "")
        );
        assert_eq!(
            serialize_html_with_options(
                identity(&dom, shadow),
                &LocalResolver(&dom),
                &explicit,
                true
            )
            .unwrap(),
            "shadow"
        );
    }
    struct Resolver<'a> {
        arenas: BTreeMap<ArenaId, &'a DomArena>,
        templates: BTreeMap<NodeRef, NodeRef>,
    }
    impl SerializationResolver for Resolver<'_> {
        fn node(&self, node: NodeRef) -> Result<Node, DomError> {
            let arena = self.arenas.get(&node.arena).ok_or(DomError::Cycle)?;
            Ok(arena.get(node.node)?.clone())
        }
        fn template_content(&self, node: NodeRef) -> Result<Option<NodeRef>, DomError> {
            Ok(self
                .templates
                .get(&node)
                .copied()
                .or(self.node(node)?.template_content.map(|content| NodeRef {
                    arena: node.arena,
                    node: content,
                })))
        }
    }
    fn identity(dom: &DomRuntime, node: NodeId) -> NodeRef {
        NodeRef {
            arena: dom.arena().id(),
            node,
        }
    }
    #[test]
    fn html_serialization_preserves_creation_is_without_overwriting_authored_attributes() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("p");
        dom.arena_mut().get_mut(node).unwrap().custom_element_is = Some("my-p".into());
        dom.set_attribute(node, "class", "foo").unwrap();
        assert_eq!(
            dom.serialize(node).unwrap(),
            "<p is=\"my-p\" class=\"foo\"></p>"
        );
        dom.set_attribute(node, "is", "authored\"&\n").unwrap();
        assert_eq!(
            dom.serialize(node).unwrap(),
            "<p class=\"foo\" is=\"authored&quot;&amp;\n\"></p>"
        );
        dom.set_attribute(node, "is", "").unwrap();
        assert_eq!(
            dom.serialize(node).unwrap(),
            "<p class=\"foo\" is=\"\"></p>"
        );
        dom.remove_attribute(node, "is").unwrap();
        assert_eq!(
            dom.serialize(node).unwrap(),
            "<p is=\"my-p\" class=\"foo\"></p>"
        );
        assert!(
            !dom.arena()
                .get(node)
                .unwrap()
                .attributes
                .contains_null("is")
        );
        assert_eq!(
            dom.arena().get(node).unwrap().custom_element_is.as_deref(),
            Some("my-p")
        );
    }
    #[test]
    fn html_serialization_handles_deep_trees_and_rejects_cycles() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let mut parent = root;
        for _ in 0..20_000 {
            let child = dom.create_element("div");
            dom.arena_mut()
                .get_mut(parent)
                .unwrap()
                .children
                .push(child);
            dom.arena_mut().get_mut(child).unwrap().parent = Some(parent);
            parent = child;
        }
        let text = dom.create_text("<&");
        dom.arena_mut().get_mut(parent).unwrap().children.push(text);
        dom.arena_mut().get_mut(text).unwrap().parent = Some(parent);
        assert_eq!(
            dom.serialize(root).unwrap(),
            format!(
                "{}&lt;&amp;{}",
                "<div>".repeat(20_001),
                "</div>".repeat(20_001)
            )
        );
        dom.arena_mut().get_mut(parent).unwrap().children.push(root);
        assert_eq!(dom.serialize(root), Err(DomError::Cycle));
    }
    #[test]
    fn html_serialization_combines_utf16_without_exposing_internal_markers() {
        let mut dom = DomRuntime::new();
        let root = dom.create_fragment();
        for data in [
            dom_string::from_units(&[0xd83d]),
            dom_string::from_units(&[0xde00]),
            "<&".into(),
        ] {
            let text = dom.create_text(&data);
            dom.append_child(root, text).unwrap();
        }
        assert_eq!(dom.serialize(root).unwrap(), "😀&lt;&amp;");
    }
    #[test]
    fn external_nested_template_serialization_preserves_code_units_and_native_records() {
        let mut host = DomRuntime::new();
        let outer = host.create_element("template");
        let mut middle = DomRuntime::new();
        let content = middle.create_fragment();
        let nested = middle.create_element("template");
        middle.append_child(content, nested).unwrap();
        let mut leaf = DomRuntime::new();
        let nested_content = leaf.create_fragment();
        let child = leaf.create_element_ns("urn:foreign", "p:item");
        leaf.append_child(nested_content, child).unwrap();
        let data = dom_string::from_units(&[0xd800, 38, 60, 0xdc00]);
        let text = leaf.create_text(&data);
        leaf.append_child(child, text).unwrap();
        leaf.set_attribute(child, "data-x", dom_string::from_units(&[0xdfff, 34, 38]))
            .unwrap();
        let records = [
            host.arena().get(outer).unwrap().clone(),
            middle.arena().get(nested).unwrap().clone(),
            leaf.arena().get(child).unwrap().clone(),
        ];
        let resolver = Resolver {
            arenas: BTreeMap::from([
                (host.arena().id(), host.arena()),
                (middle.arena().id(), middle.arena()),
                (leaf.arena().id(), leaf.arena()),
            ]),
            templates: BTreeMap::from([
                (identity(&host, outer), identity(&middle, content)),
                (identity(&middle, nested), identity(&leaf, nested_content)),
            ]),
        };
        let result = serialize_html_with_resolver(identity(&host, outer), &resolver).unwrap();
        let expected = concat(&[
            "<template><template><p:item data-x=\"",
            &dom_string::from_units(&[0xdfff]),
            "&quot;&amp;\">",
            &dom_string::from_units(&[0xd800]),
            "&amp;&lt;",
            &dom_string::from_units(&[0xdc00]),
            "</p:item></template></template>",
        ]);
        assert_eq!(dom_string::units(&result), dom_string::units(&expected));
        let xml =
            crate::serialize_xml_with_resolver(identity(&leaf, nested_content), &resolver).unwrap();
        assert_eq!(
            dom_string::units(&xml),
            dom_string::units(&concat(&[
                "<p:item xmlns:p=\"urn:foreign\" data-x=\"",
                &dom_string::from_units(&[0xdfff]),
                "&quot;&amp;\">",
                &dom_string::from_units(&[0xd800]),
                "&amp;&lt;",
                &dom_string::from_units(&[0xdc00]),
                "</p:item>"
            ]))
        );
        assert_eq!(
            records,
            [
                host.arena().get(outer).unwrap().clone(),
                middle.arena().get(nested).unwrap().clone(),
                leaf.arena().get(child).unwrap().clone()
            ]
        );
    }
    #[test]
    fn external_template_cycle_is_rejected_and_literal_storage_marker_is_preserved() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("template");
        let content = dom.create_fragment();
        dom.append_child(content, root).unwrap();
        let resolver = Resolver {
            arenas: BTreeMap::from([(dom.arena().id(), dom.arena())]),
            templates: BTreeMap::from([(identity(&dom, root), identity(&dom, content))]),
        };
        assert_eq!(
            serialize_html_with_resolver(identity(&dom, root), &resolver),
            Err(DomError::Cycle)
        );
        let literal = format!("{}[55296]", dom_string::PREFIX);
        let encoded = dom_string::from_units(&literal.encode_utf16().collect::<Vec<_>>());
        let node = dom.create_text(&encoded);
        assert_eq!(
            dom_string::units(&dom.serialize(node).unwrap()),
            literal.encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(
            dom_string::units(&dom.serialize_xml(node).unwrap()),
            literal.encode_utf16().collect::<Vec<_>>()
        );
    }
}
