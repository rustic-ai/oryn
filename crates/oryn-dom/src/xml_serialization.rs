//! XML serialization from native node and namespace records.
use crate::{DomError, DomRuntime, NodeId, NodeKind, NodeRef, SerializationResolver};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
const HTML: &str = "http://www.w3.org/1999/xhtml";

fn escaped(value: &str, attribute: bool) -> String {
    crate::serialization::escaped(value, attribute, true)
}
use crate::serialization::{append, concat};

#[derive(Clone)]
struct Bindings {
    values: BTreeMap<String, String>,
    aliases: BTreeMap<String, Vec<String>>,
}
impl std::ops::Deref for Bindings {
    type Target = BTreeMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl Bindings {
    fn insert(&mut self, prefix: String, namespace: String) {
        // DOM Parsing's namespace-to-prefix lists retain earlier aliases even
        // when a descendant rebinds a prefix to another namespace.
        if !prefix.is_empty() {
            self.aliases
                .entry(namespace.clone())
                .or_default()
                .push(prefix.clone());
        }
        self.values.insert(prefix, namespace);
    }
    fn preferred(&self, namespace: &str, preferred: Option<&str>) -> Option<String> {
        let candidates = self.aliases.get(namespace)?;
        if let Some(prefix) =
            preferred.filter(|prefix| candidates.iter().any(|candidate| candidate == prefix))
        {
            return Some(prefix.into());
        }
        candidates.last().cloned()
    }
}

struct SerializedNodeBody {
    prefix: String,
    children: Vec<(NodeRef, Bindings)>,
    suffix: String,
}

struct Serializer<'a, R: SerializationResolver> {
    resolver: &'a R,
    active: BTreeSet<NodeRef>,
    next_prefix: usize,
}

// XML serialization follows HTML template contents, which may belong to a
// different native arena. Ordinary template children are not serialized.
fn serialization_children(
    node: NodeRef,
    record: &crate::Node,
    resolver: &impl SerializationResolver,
) -> Result<Vec<NodeRef>, DomError> {
    if matches!(&record.kind, NodeKind::Element { name } if name.ns.as_ref() == HTML && name.local.as_ref() == "template")
    {
        return Ok(resolver.template_content(node)?.into_iter().collect());
    }
    let mut children = Vec::new();
    for child in &record.children {
        let child = NodeRef {
            arena: node.arena,
            node: *child,
        };
        if !matches!(resolver.node(child)?.kind, NodeKind::ShadowRoot { .. }) {
            children.push(child);
        }
    }
    Ok(children)
}

impl DomRuntime {
    /// Serialize XML without interpreting HTML raw-text or void-element rules
    /// for foreign namespaces. Namespace scope is local to each subtree.
    pub fn serialize_xml(&self, node: NodeId) -> Result<String, DomError> {
        serialize_xml_with_resolver(
            NodeRef {
                arena: self.arena().id(),
                node,
            },
            &crate::serialization::LocalResolver(self),
        )
    }
}

pub fn serialize_xml_with_resolver(
    root: NodeRef,
    resolver: &impl SerializationResolver,
) -> Result<String, DomError> {
    let namespaces = Bindings {
        values: BTreeMap::from([("xml".into(), XML.into()), (String::new(), String::new())]),
        aliases: BTreeMap::from([(XML.into(), vec!["xml".into()])]),
    };
    Serializer {
        resolver,
        active: BTreeSet::new(),
        next_prefix: 1,
    }
    .node(root, &namespaces)
}
/// Legacy XML markup getters require well-formed output. XMLSerializer keeps
/// its separate non-strict behavior, including lossless DOMString emission.
#[derive(Debug)]
pub enum WellFormedError {
    Dom(DomError),
    InvalidState,
}
impl From<DomError> for WellFormedError {
    fn from(error: DomError) -> Self {
        Self::Dom(error)
    }
}
// Validate native character records before markup parsing. A permissive XML
// tokenizer may accept forbidden characters or interpret a delimiter inside a
// comment/CDATA/PI as different nodes; that cannot establish well-formedness of
// the original DOM record. Keep non-strict XMLSerializer behavior separate.
fn well_formed_characters(value: &str) -> Result<String, WellFormedError> {
    let value = String::from_utf16(&crate::dom_string::units(value))
        .map_err(|_| WellFormedError::InvalidState)?;
    if !value
        .chars()
        .all(|ch| matches!(ch as u32,0x9|0xa|0xd|0x20..=0xd7ff|0xe000..=0xfffd|0x10000..=0x10ffff))
    {
        return Err(WellFormedError::InvalidState);
    }
    Ok(value)
}
pub fn serialize_xml_well_formed_with_resolver(
    root: NodeRef,
    resolver: &impl SerializationResolver,
) -> Result<String, WellFormedError> {
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            return Err(DomError::Cycle.into());
        }
        let record = resolver.node(node)?;
        match &record.kind {
            NodeKind::Text { data } => {
                well_formed_characters(data)?;
            }
            NodeKind::Comment { data } => {
                let data = well_formed_characters(data)?;
                if data.contains("--") || data.ends_with('-') {
                    return Err(WellFormedError::InvalidState);
                }
            }
            NodeKind::CDataSection { data } => {
                if well_formed_characters(data)?.contains("]]>") {
                    return Err(WellFormedError::InvalidState);
                }
            }
            NodeKind::ProcessingInstruction { target, data } => {
                well_formed_characters(target)?;
                if well_formed_characters(data)?.contains("?>") {
                    return Err(WellFormedError::InvalidState);
                }
            }
            _ => {}
        }
        if let NodeKind::Element { name } = &record.kind {
            if name.local.contains(':') {
                return Err(WellFormedError::InvalidState);
            }
            for attr in record.attributes.records() {
                well_formed_characters(&attr.value)?;
                if attr.local_name.contains(':')
                    || (attr.namespace.is_none() && attr.local_name == "xmlns")
                {
                    return Err(WellFormedError::InvalidState);
                }
                if attr.namespace.as_deref() == Some(XMLNS) {
                    let prefix = if attr.prefix.as_deref() == Some("xmlns") {
                        attr.local_name.as_str()
                    } else {
                        ""
                    };
                    if prefix == "xmlns"
                        || attr.value == XMLNS
                        || (prefix == "xml" && attr.value != XML)
                        || (!prefix.is_empty() && attr.value.is_empty())
                    {
                        return Err(WellFormedError::InvalidState);
                    }
                }
            }
        }
        if matches!(&record.kind,NodeKind::ProcessingInstruction{target,..}if target.contains(':')||target.eq_ignore_ascii_case("xml"))
        {
            return Err(WellFormedError::InvalidState);
        }
        pending.extend(serialization_children(node, &record, resolver)?);
    }
    let serialized = serialize_xml_with_resolver(root, resolver)?;
    let unicode = String::from_utf16(&crate::dom_string::units(&serialized))
        .map_err(|_| WellFormedError::InvalidState)?;
    // The parser checks XML Char, names, comments, PI data, CDATA boundaries,
    // namespace declarations and duplicate expanded attribute names together.
    let parsed = crate::xml::parse(
        &format!("<oryn-fragment>{unicode}</oryn-fragment>"),
        "application/xml",
    );
    if !parsed.errors.is_empty() {
        return Err(WellFormedError::InvalidState);
    }
    Ok(serialized)
}
impl<R: SerializationResolver> Serializer<'_, R> {
    fn fresh_prefix(&mut self, _bindings: &Bindings) -> String {
        // The pinned DOM Parsing algorithm increments without skipping an
        // authored nsN collision (tracked by specification issue 44).
        let prefix = format!("ns{}", self.next_prefix);
        self.next_prefix += 1;
        prefix
    }

    fn node(&mut self, node: NodeRef, inherited: &Bindings) -> Result<String, DomError> {
        enum Task {
            Visit(NodeRef, Bindings),
            Leave(NodeRef),
            Close(String),
        }
        let mut pending = vec![Task::Visit(node, inherited.clone())];
        let mut output = Vec::new();
        while let Some(task) = pending.pop() {
            match task {
                Task::Leave(node) => {
                    self.active.remove(&node);
                }
                Task::Close(value) => output.extend(crate::dom_string::units(&value)),
                Task::Visit(node, bindings) => {
                    if !self.active.insert(node) {
                        return Err(DomError::Cycle);
                    }
                    pending.push(Task::Leave(node));
                    let SerializedNodeBody {
                        prefix,
                        children,
                        suffix,
                    } = self.node_body(node, &bindings)?;
                    output.extend(crate::dom_string::units(&prefix));
                    if !suffix.is_empty() {
                        pending.push(Task::Close(suffix));
                    }
                    pending.extend(
                        children
                            .into_iter()
                            .rev()
                            .map(|(node, bindings)| Task::Visit(node, bindings)),
                    );
                }
            }
        }
        Ok(crate::dom_string::from_units(&output))
    }
    fn node_body(
        &mut self,
        node: NodeRef,
        inherited: &Bindings,
    ) -> Result<SerializedNodeBody, DomError> {
        let record = self.resolver.node(node)?;
        let prefix = match &record.kind {
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => {
                let mut children = Vec::new();
                for child in &record.children {
                    let child = NodeRef {
                        arena: node.arena,
                        node: *child,
                    };
                    if !matches!(self.resolver.node(child)?.kind, NodeKind::ShadowRoot { .. }) {
                        children.push((child, inherited.clone()));
                    }
                }
                return Ok(SerializedNodeBody {
                    prefix: String::new(),
                    children,
                    suffix: String::new(),
                });
            }
            NodeKind::Text { data } => escaped(data, false),
            NodeKind::CDataSection { data } => concat(&["<![CDATA[", data, "]]>"]),
            NodeKind::Comment { data } => concat(&["<!--", data, "-->"]),
            NodeKind::ProcessingInstruction { target, data } => {
                concat(&["<?", target, " ", data, "?>"])
            }
            // Attributes are serialized only as part of their owner element.
            // The XML serialization dispatch returns empty for an Attr root.
            NodeKind::Attribute { .. } => String::new(),
            NodeKind::DocumentType {
                name,
                public_id,
                system_id,
            } => {
                let mut output = concat(&["<!DOCTYPE ", name]);
                if !public_id.is_empty() {
                    append(&mut output, &concat(&[" PUBLIC \"", public_id, "\""]));
                }
                if !system_id.is_empty() {
                    append(
                        &mut output,
                        if public_id.is_empty() { " SYSTEM" } else { "" },
                    );
                    append(&mut output, &concat(&[" \"", system_id, "\""]));
                }
                append(&mut output, ">");
                output
            }
            NodeKind::Element { name } => {
                let mut bindings = inherited.clone();
                let mut declarations = BTreeMap::new();
                for attribute in record.attributes.records() {
                    if attribute.namespace.as_deref() == Some(XMLNS) {
                        let prefix = if attribute.name == "xmlns" {
                            ""
                        } else {
                            &attribute.local_name
                        };
                        // For an unprefixed element its native namespace owns
                        // the default binding. Redundant or inconsistent
                        // authored defaults must not change that binding.
                        if prefix.is_empty()
                            && name.prefix.as_ref().is_none_or(|prefix| prefix.is_empty())
                            && name.ns.as_ref() != XML
                            && (inherited
                                .get("")
                                .is_some_and(|value| value == name.ns.as_ref())
                                || inherited.preferred(name.ns.as_ref(), None).is_none())
                        {
                            continue;
                        }
                        if prefix != "xml" && prefix != "xmlns" {
                            bindings.insert(prefix.into(), attribute.value.clone());
                            declarations.insert(prefix.to_string(), attribute.value.clone());
                        }
                    }
                }
                let namespace = name.ns.as_ref();
                let mut prefix = name
                    .prefix
                    .as_ref()
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                let mut reused_prefix = false;
                if namespace.is_empty() || inherited.get("").is_some_and(|value| value == namespace)
                {
                    prefix.clear();
                } else if namespace == XML {
                    prefix = "xml".into();
                } else if let Some(existing) = bindings.preferred(namespace, Some(&prefix)) {
                    prefix = existing;
                    reused_prefix = true;
                } else if !prefix.is_empty()
                    && declarations
                        .get(&prefix)
                        .is_some_and(|value| value != namespace)
                {
                    prefix = self.fresh_prefix(&bindings);
                }
                let mut attributes = Vec::new();
                if !reused_prefix && bindings.get(&prefix).map(String::as_str) != Some(namespace) {
                    bindings.insert(prefix.clone(), namespace.into());
                    declarations.remove(&prefix);
                    attributes.push((
                        if prefix.is_empty() {
                            "xmlns".into()
                        } else {
                            format!("xmlns:{prefix}")
                        },
                        namespace.to_owned(),
                    ));
                }
                let tag = if prefix.is_empty() {
                    name.local.to_string()
                } else {
                    format!("{prefix}:{}", name.local)
                };
                // Namespace attributes keep their authored positions. A
                // generated attribute binding is emitted immediately before
                // the attribute that first needs it, not in a sorted prelude.
                for attribute in record.attributes.records() {
                    if attribute.namespace.as_deref() == Some(XMLNS) {
                        let prefix = if attribute.name == "xmlns" {
                            ""
                        } else {
                            &attribute.local_name
                        };
                        if declarations.get(prefix) == Some(&attribute.value) {
                            attributes.push((
                                if prefix.is_empty() {
                                    "xmlns".into()
                                } else {
                                    format!("xmlns:{prefix}")
                                },
                                attribute.value.clone(),
                            ));
                        }
                        continue;
                    }
                    if attribute.namespace.is_none() && attribute.local_name == "xmlns" {
                        continue;
                    }
                    let namespace = attribute.namespace.as_deref().unwrap_or("");
                    let qualified = if namespace.is_empty() {
                        attribute.local_name.clone()
                    } else if namespace == XML {
                        format!("xml:{}", attribute.local_name)
                    } else {
                        let preferred = attribute.prefix.as_deref().filter(|value| {
                            !value.is_empty() && *value != "xml" && *value != "xmlns"
                        });
                        let existing = bindings.preferred(namespace, preferred);
                        let reused = existing.is_some();
                        let prefix = existing
                            .or_else(|| {
                                preferred
                                    .filter(|prefix| bindings.get(*prefix).is_none())
                                    .map(str::to_owned)
                            })
                            .unwrap_or_else(|| self.fresh_prefix(&bindings));
                        if !reused {
                            bindings.insert(prefix.clone(), namespace.into());
                            attributes.push((format!("xmlns:{prefix}"), namespace.to_owned()));
                        }
                        format!("{prefix}:{}", attribute.local_name)
                    };
                    attributes.push((qualified, attribute.value.clone()));
                }
                let mut output = format!("<{tag}");
                for (name, value) in attributes {
                    append(
                        &mut output,
                        &concat(&[" ", &name, "=\"", &escaped(&value, true), "\""]),
                    );
                }
                let children = &record.children;
                if children.is_empty() && namespace != HTML {
                    append(&mut output, "/>");
                } else if children.is_empty() && crate::is_void_html_element(&name.local) {
                    append(&mut output, " />");
                } else {
                    append(&mut output, ">");
                    let descendants = serialization_children(node, &record, self.resolver)?
                        .into_iter()
                        .map(|child| (child, bindings.clone()))
                        .collect();
                    return Ok(SerializedNodeBody {
                        prefix: output,
                        children: descendants,
                        suffix: format!("</{tag}>"),
                    });
                }
                output
            }
        };
        Ok(SerializedNodeBody {
            prefix,
            children: Vec::new(),
            suffix: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_xml_rejects_forbidden_attribute_characters_before_serialization() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("div");
        dom.set_attribute(element, "title", "before\u{c}after")
            .unwrap();
        let original = dom.serialize_xml(element).unwrap();
        let node = NodeRef {
            arena: dom.arena().id(),
            node: element,
        };
        assert!(matches!(
            serialize_xml_well_formed_with_resolver(
                node,
                &crate::serialization::LocalResolver(&dom)
            ),
            Err(WellFormedError::InvalidState)
        ));
        assert_eq!(dom.serialize_xml(element).unwrap(), original);
    }
    #[test]
    fn strict_xml_character_nodes_reject_invalid_records_without_changing_xmlserializer() {
        let mut dom = DomRuntime::new();
        let invalid = [
            NodeKind::Text {
                data: "\u{c}".into(),
            },
            NodeKind::Text {
                data: crate::dom_string::from_units(&[0xd800]),
            },
            NodeKind::Comment {
                data: "two--hyphens".into(),
            },
            NodeKind::Comment {
                data: "--><injected/><!--".into(),
            },
            NodeKind::Comment {
                data: "last-".into(),
            },
            NodeKind::CDataSection { data: "]]>".into() },
            NodeKind::ProcessingInstruction {
                target: "xml".into(),
                data: "data".into(),
            },
            NodeKind::ProcessingInstruction {
                target: "x:y".into(),
                data: "data".into(),
            },
            NodeKind::ProcessingInstruction {
                target: "target".into(),
                data: "?>".into(),
            },
        ];
        for kind in invalid {
            let node = dom.create(kind);
            let original = dom.serialize_xml(node).unwrap();
            let identity = NodeRef {
                arena: dom.arena().id(),
                node,
            };
            assert!(
                matches!(
                    serialize_xml_well_formed_with_resolver(
                        identity,
                        &crate::serialization::LocalResolver(&dom)
                    ),
                    Err(WellFormedError::InvalidState)
                ),
                "invalid native XML record: {:?}",
                dom.arena().get(node).unwrap().kind
            );
            assert_eq!(dom.serialize_xml(node).unwrap(), original);
        }
        for kind in [
            NodeKind::Text {
                data: "<&>\t\n\r\u{10000}".into(),
            },
            NodeKind::Comment {
                data: "valid".into(),
            },
            NodeKind::CDataSection {
                data: "valid <&".into(),
            },
            NodeKind::ProcessingInstruction {
                target: "target".into(),
                data: "valid".into(),
            },
        ] {
            let node = dom.create(kind);
            let identity = NodeRef {
                arena: dom.arena().id(),
                node,
            };
            assert_eq!(
                serialize_xml_well_formed_with_resolver(
                    identity,
                    &crate::serialization::LocalResolver(&dom)
                )
                .unwrap(),
                dom.serialize_xml(node).unwrap()
            );
        }
    }
    #[test]
    fn pinned_non_strict_namespace_aliases_and_generated_collisions() {
        let mut dom = crate::xml::parse(
            r#"<el1 xmlns:p="u1" xmlns:q="u1"><el2 xmlns:q="u2"/></el1>"#,
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        let root = dom.arena().get(dom.document()).unwrap().children[0];
        let child = dom.arena().get(root).unwrap().children[0];
        dom.set_attribute_ns(child, Some("u1".into()), "name", "v")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<el1 xmlns:p="u1" xmlns:q="u1"><el2 xmlns:q="u2" q:name="v"/></el1>"#
        );
        let mut dom = crate::xml::parse(
            r#"<root xmlns:ns2="uri2"><child xmlns:ns1="uri1"/></root>"#,
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        let root = dom.arena().get(dom.document()).unwrap().children[0];
        let child = dom.arena().get(root).unwrap().children[0];
        dom.set_attribute_ns(child, Some("uri3".into()), "attr1", "value1")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<root xmlns:ns2="uri2"><child xmlns:ns1="uri1" xmlns:ns1="uri3" ns1:attr1="value1"/></root>"#
        );
    }
    #[test]
    fn namespace_declarations_follow_attribute_order_and_generated_use() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element_ns("", "root");
        dom.set_attribute_ns(root, Some("uri1".into()), "p:foobar", "value1")
            .unwrap();
        dom.set_attribute_ns(root, Some(XMLNS.into()), "xmlns:p", "uri2")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<root xmlns:ns1="uri1" ns1:foobar="value1" xmlns:p="uri2"/>"#
        );
        let root = dom.create_element_ns("", "root");
        dom.set_attribute_ns(root, None, "before", "v").unwrap();
        dom.set_attribute_ns(root, Some(XMLNS.into()), "xmlns:z", "urn:z")
            .unwrap();
        dom.set_attribute_ns(root, None, "middle", "v").unwrap();
        dom.set_attribute_ns(root, Some(XMLNS.into()), "xmlns:a", "urn:a")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<root before="v" xmlns:z="urn:z" middle="v" xmlns:a="urn:a"/>"#
        );
        let root = dom.create_element_ns("", "root");
        let first = dom.create_element_ns("", "child1");
        let second = dom.create_element_ns("", "child2");
        dom.append_child(root, first).unwrap();
        dom.append_child(root, second).unwrap();
        dom.set_attribute_ns(first, Some("uri1".into()), "attr1", "value1")
            .unwrap();
        dom.set_attribute_ns(first, Some("uri2".into()), "attr2", "value2")
            .unwrap();
        dom.set_attribute_ns(second, Some("uri3".into()), "attr3", "value3")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<root><child1 xmlns:ns1="uri1" ns1:attr1="value1" xmlns:ns2="uri2" ns2:attr2="value2"/><child2 xmlns:ns3="uri3" ns3:attr3="value3"/></root>"#
        );
    }
    #[test]
    fn xml_serialization_reuses_nearest_declared_prefix() {
        let mut dom = crate::xml::parse(
            r#"<r xmlns:x0="uri" xmlns:x2="uri"><b xmlns:x1="uri"/></r>"#,
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        let root = dom.arena().get(dom.document()).unwrap().children[0];
        let child = dom.arena().get(root).unwrap().children[0];
        dom.set_attribute_ns(child, Some("uri".into()), "unused:name", "v")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<r xmlns:x0="uri" xmlns:x2="uri"><b xmlns:x1="uri" x1:name="v"/></r>"#
        );
        let nested = dom.create_element_ns("uri", "nested");
        dom.append_child(child, nested).unwrap();
        assert!(dom.serialize_xml(root).unwrap().contains("<x1:nested/>"));
        let dom = crate::xml::parse(
            r#"<root xmlns="u1"><p:child xmlns:p="u1"/></root>"#,
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        assert_eq!(
            dom.serialize_xml(dom.document()).unwrap(),
            r#"<root xmlns="u1"><child xmlns:p="u1"/></root>"#
        );
        let dom = crate::xml::parse(
            r#"<root xmlns:x="uri1"><table xmlns="uri1"/></root>"#,
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        assert_eq!(
            dom.serialize_xml(dom.document()).unwrap(),
            r#"<root xmlns:x="uri1"><x:table xmlns="uri1"/></root>"#
        );
    }
    #[test]
    fn strict_xml_checks_original_namespace_declarations_before_normalization() {
        for (prefix, value) in [
            ("xmlns:xml", "urn:wrong"),
            ("xmlns:xmlns", XMLNS),
            ("xmlns:p", ""),
        ] {
            let mut dom = DomRuntime::new();
            let root = dom.create_element_ns("", "root");
            // Populate native records through the regular attribute setter;
            // namespace URI values are not constrained by DOM setAttributeNS.
            dom.set_attribute_ns(root, Some(XMLNS.into()), prefix, value)
                .unwrap();
            let identity = NodeRef {
                arena: dom.arena().id(),
                node: root,
            };
            assert!(
                matches!(
                    serialize_xml_well_formed_with_resolver(
                        identity,
                        &crate::serialization::LocalResolver(&dom)
                    ),
                    Err(WellFormedError::InvalidState)
                ),
                "{prefix}={value}"
            );
        }
    }
    #[test]
    fn default_namespace_serialization_uses_native_names_and_omits_redundancy() {
        for (source, expected) in [
            (
                r#"<root><child xmlns=""/></root>"#,
                r#"<root><child/></root>"#,
            ),
            (
                r#"<root xmlns=""><child xmlns=""/></root>"#,
                r#"<root><child/></root>"#,
            ),
            (
                r#"<root xmlns="u1"><child xmlns="u1"/></root>"#,
                r#"<root xmlns="u1"><child/></root>"#,
            ),
        ] {
            let dom = crate::xml::parse(source, "application/xml")
                .into_runtime()
                .unwrap();
            assert_eq!(dom.serialize_xml(dom.document()).unwrap(), expected);
        }
        let mut dom = crate::xml::parse(r#"<root xmlns="uri1"/>"#, "application/xml")
            .into_runtime()
            .unwrap();
        let root = dom.arena().get(dom.document()).unwrap().children[0];
        for (namespace, name, declared) in [
            ("", "child", "FAIL1"),
            ("uri2", "child2", "FAIL2"),
            ("uri1", "child3", "FAIL3"),
            ("uri4", "child4", "uri4"),
            ("", "child5", ""),
        ] {
            let child = dom.create_element_ns(namespace, name);
            dom.set_attribute_ns(child, Some(XMLNS.into()), "xmlns", declared)
                .unwrap();
            dom.append_child(root, child).unwrap();
        }
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            r#"<root xmlns="uri1"><child xmlns=""/><child2 xmlns="uri2"/><child3/><child4 xmlns="uri4"/><child5 xmlns=""/></root>"#
        );
        let null = dom.create_element_ns("", "package");
        dom.set_attribute_ns(null, None, "xmlns", "urn:wrong")
            .unwrap();
        assert_eq!(dom.serialize_xml(null).unwrap(), "<package/>");
        assert_eq!(
            dom.attribute_in_null_namespace(null, "xmlns").unwrap(),
            Some("urn:wrong")
        );
    }
    #[test]
    fn doctype_public_and_system_literals_follow_pinned_xml_serialization() {
        let mut dom = DomRuntime::new();
        for (public, system, expected) in [
            ("", "", r#"<!DOCTYPE html>"#),
            ("a", "", r#"<!DOCTYPE html PUBLIC "a">"#),
            ("", "a", r#"<!DOCTYPE html SYSTEM "a">"#),
            ("a", "b", r#"<!DOCTYPE html PUBLIC "a" "b">"#),
            (r#"""#, r#"""#, r#"<!DOCTYPE html PUBLIC """ """>"#),
        ] {
            let node = dom.create(NodeKind::DocumentType {
                name: "html".into(),
                public_id: public.into(),
                system_id: system.into(),
            });
            assert_eq!(dom.serialize_xml(node).unwrap(), expected);
        }
    }

    #[test]
    fn xml_serialization_handles_deep_trees_and_preserves_namespace_scope() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element_ns("urn:tree", "p:node");
        let mut parent = root;
        for _ in 0..20_000 {
            let child = dom.create_element_ns("urn:tree", "p:node");
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
            dom.serialize_xml(root).unwrap(),
            format!(
                "<p:node xmlns:p=\"urn:tree\">{}&lt;&amp;{}",
                "<p:node>".repeat(20_000),
                "</p:node>".repeat(20_001)
            )
        );
        dom.arena_mut().get_mut(parent).unwrap().children.push(root);
        assert_eq!(dom.serialize_xml(root), Err(DomError::Cycle));
    }

    #[test]
    fn xml_round_trip_preserves_namespaces_and_character_data() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element_ns("urn:root", "root");
        let child = dom.create_element_ns("", "child");
        let text = dom.create_text("<&>\u{a0}");
        dom.append_child(root, child).unwrap();
        dom.append_child(child, text).unwrap();
        dom.set_attribute_ns(root, Some("urn:attr".into()), "flag", "\t\n\r\"&")
            .unwrap();
        let serialized = dom.serialize_xml(root).unwrap();
        assert_eq!(
            serialized,
            "<root xmlns=\"urn:root\" xmlns:ns1=\"urn:attr\" ns1:flag=\"&#x9;&#xA;&#xD;&quot;&amp;\"><child xmlns=\"\">&lt;&amp;&gt;\u{a0}</child></root>"
        );
        let parsed = crate::xml::parse(&serialized, "application/xml");
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    }

    #[test]
    fn prefix_collisions_do_not_change_expanded_names() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element_ns("urn:element", "p:root");
        dom.set_attribute_ns(root, Some(XMLNS.into()), "xmlns:p", "urn:declared")
            .unwrap();
        dom.set_attribute_ns(root, Some("urn:attribute".into()), "p:value", "yes")
            .unwrap();
        assert_eq!(
            dom.serialize_xml(root).unwrap(),
            "<ns1:root xmlns:ns1=\"urn:element\" xmlns:p=\"urn:declared\" xmlns:ns2=\"urn:attribute\" ns2:value=\"yes\"/>"
        );
    }

    #[test]
    fn xml_does_not_apply_html_raw_text_or_named_entities() {
        let mut dom = DomRuntime::new();
        let script = dom.create_element("script");
        let text = dom.create_text("a < b && \u{a0}");
        dom.append_child(script, text).unwrap();
        assert_eq!(
            dom.serialize_xml(script).unwrap(),
            "<script xmlns=\"http://www.w3.org/1999/xhtml\">a &lt; b &amp;&amp; \u{a0}</script>"
        );
        let foreign = dom.create_element_ns("urn:foreign", "br");
        assert_eq!(
            dom.serialize_xml(foreign).unwrap(),
            "<br xmlns=\"urn:foreign\"/>"
        );
    }
}

#[cfg(test)]
mod template_content_regressions {
    use super::*;
    struct Resolver<'a> {
        host: &'a DomRuntime,
        content: &'a DomRuntime,
        template: NodeRef,
        fragment: NodeRef,
    }
    impl SerializationResolver for Resolver<'_> {
        fn node(&self, node: NodeRef) -> Result<crate::Node, DomError> {
            let arena = if node.arena == self.host.arena().id() {
                self.host.arena()
            } else {
                self.content.arena()
            };
            if node.arena != arena.id() {
                return Err(DomError::WrongArena {
                    expected: arena.id(),
                    actual: node.arena,
                });
            }
            Ok(arena.get(node.node)?.clone())
        }
        fn template_content(&self, node: NodeRef) -> Result<Option<NodeRef>, DomError> {
            if node == self.template {
                Ok(Some(self.fragment))
            } else {
                Ok(self.node(node)?.template_content.map(|content| NodeRef {
                    arena: node.arena,
                    node: content,
                }))
            }
        }
    }
    #[test]
    fn xml_template_contents_follow_external_identity_and_namespace_scope() {
        let mut host = DomRuntime::new();
        let template = host.create_element("template");
        let ignored = host.create_text("ignored ordinary child");
        host.append_child(template, ignored).unwrap();
        let mut content = DomRuntime::new();
        let fragment = content.create(NodeKind::DocumentFragment);
        let child = content.create_element_ns("urn:content", "child");
        content.append_child(fragment, child).unwrap();
        let text = content.create_text("<&");
        content.append_child(child, text).unwrap();
        let template = NodeRef {
            arena: host.arena().id(),
            node: template,
        };
        let fragment = NodeRef {
            arena: content.arena().id(),
            node: fragment,
        };
        let expected = "<template xmlns=\"http://www.w3.org/1999/xhtml\"><child xmlns=\"urn:content\">&lt;&amp;</child></template>";
        {
            let resolver = Resolver {
                host: &host,
                content: &content,
                template,
                fragment,
            };
            assert_eq!(
                serialize_xml_with_resolver(template, &resolver).unwrap(),
                expected
            );
            assert_eq!(
                serialize_xml_well_formed_with_resolver(template, &resolver).unwrap(),
                expected
            );
        }
        let invalid = content.create(NodeKind::Comment {
            data: "invalid--comment".into(),
        });
        content.append_child(fragment.node, invalid).unwrap();
        let resolver = Resolver {
            host: &host,
            content: &content,
            template,
            fragment,
        };
        assert!(matches!(
            serialize_xml_well_formed_with_resolver(template, &resolver),
            Err(WellFormedError::InvalidState)
        ));
        assert!(
            serialize_xml_with_resolver(template, &resolver)
                .unwrap()
                .contains("<!--invalid--comment-->")
        );
    }
    #[test]
    fn xml_nested_templates_use_local_content_and_ignore_ordinary_children() {
        let mut dom = DomRuntime::new();
        let outer = dom.create_element("template");
        let outer_content = dom.template_content(outer).unwrap();
        let nested = dom.create_element("template");
        dom.append_child(outer_content, nested).unwrap();
        let nested_content = dom.template_content(nested).unwrap();
        let text = dom.create_text("nested");
        dom.append_child(nested_content, text).unwrap();
        let ignored = dom.create(NodeKind::Comment {
            data: "ignored--invalid".into(),
        });
        dom.append_child(nested, ignored).unwrap();
        let reference = NodeRef {
            arena: dom.arena().id(),
            node: outer,
        };
        let expected = "<template xmlns=\"http://www.w3.org/1999/xhtml\"><template>nested</template></template>";
        assert_eq!(dom.serialize_xml(outer).unwrap(), expected);
        assert_eq!(
            serialize_xml_well_formed_with_resolver(
                reference,
                &crate::serialization::LocalResolver(&dom)
            )
            .unwrap(),
            expected
        );
        let foreign = dom.create_element_ns("urn:foreign", "template");
        let text = dom.create_text("ordinary");
        dom.append_child(foreign, text).unwrap();
        assert_eq!(
            dom.serialize_xml(foreign).unwrap(),
            "<template xmlns=\"urn:foreign\">ordinary</template>"
        );
    }
    #[test]
    fn xml_template_content_cycles_are_rejected() {
        let mut host = DomRuntime::new();
        let node = host.create_element("template");
        let template = NodeRef {
            arena: host.arena().id(),
            node,
        };
        let content = DomRuntime::new();
        let resolver = Resolver {
            host: &host,
            content: &content,
            template,
            fragment: template,
        };
        assert_eq!(
            serialize_xml_with_resolver(template, &resolver),
            Err(DomError::Cycle)
        );
        assert!(matches!(
            serialize_xml_well_formed_with_resolver(template, &resolver),
            Err(WellFormedError::Dom(DomError::Cycle))
        ));
    }
}
