use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeMap;

use html5ever::driver::ParseOpts;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, Namespace, QualName, parse_document};
use markup5ever::interface::tree_builder::ElemName;
use markup5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};

use crate::dom::{DomArena, NodeId, NodeKind};

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub dom: DomArena,
    pub document: NodeId,
    pub errors: Vec<String>,
    pub quirks_mode: QuirksMode,
}

pub fn parse(input: &str) -> ParsedDocument {
    parse_document(OrynTreeSink::new(), ParseOpts::default()).one(input)
}

struct OrynTreeSink {
    dom: RefCell<DomArena>,
    document: NodeId,
    errors: RefCell<Vec<String>>,
    templates: RefCell<BTreeMap<NodeId, NodeId>>,
    quirks_mode: Cell<QuirksMode>,
}

impl OrynTreeSink {
    fn new() -> Self {
        let mut dom = DomArena::default();
        let document = dom.insert(NodeKind::Document);
        Self {
            dom: RefCell::new(dom),
            document,
            errors: RefCell::new(Vec::new()),
            templates: RefCell::new(BTreeMap::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
        }
    }

    fn append_node_or_text(&self, parent: NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.dom
                    .borrow_mut()
                    .append_child(parent, node)
                    .expect("html5ever emitted a valid append");
            }
            NodeOrText::AppendText(text) => self.append_text(parent, text),
        }
    }

    fn append_text(&self, parent: NodeId, text: StrTendril) {
        let mut dom = self.dom.borrow_mut();
        let last_child = dom
            .get(parent)
            .ok()
            .and_then(|node| node.children.last().copied());
        if let Some(last_child) = last_child
            && let Ok(node) = dom.get_mut(last_child)
            && let NodeKind::Text { data } = &mut node.kind
        {
            data.push_str(&text);
            return;
        }
        let text = dom.insert(NodeKind::Text {
            data: text.to_string(),
        });
        dom.append_child(parent, text)
            .expect("text parent is valid");
    }
}

struct OrynElemName<'a> {
    node: Ref<'a, crate::dom::Node>,
}

impl std::fmt::Debug for OrynElemName<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OrynElemName")
            .finish_non_exhaustive()
    }
}

impl ElemName for OrynElemName<'_> {
    fn ns(&self) -> &Namespace {
        match &self.node.kind {
            NodeKind::Element { name } => &name.ns,
            _ => panic!("elem_name called for non-element"),
        }
    }

    fn local_name(&self) -> &LocalName {
        match &self.node.kind {
            NodeKind::Element { name } => &name.local,
            _ => panic!("elem_name called for non-element"),
        }
    }
}

impl TreeSink for OrynTreeSink {
    type Handle = NodeId;
    type Output = ParsedDocument;
    type ElemName<'a> = OrynElemName<'a>;

    fn finish(self) -> Self::Output {
        ParsedDocument {
            dom: self.dom.into_inner(),
            document: self.document,
            errors: self.errors.into_inner(),
            quirks_mode: self.quirks_mode.get(),
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg.into_owned());
    }

    fn get_document(&self) -> Self::Handle {
        self.document
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        OrynElemName {
            node: Ref::map(self.dom.borrow(), |dom| {
                dom.get(*target).expect("element handle must be live")
            }),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        let is_template = flags.template;
        let mut dom = self.dom.borrow_mut();
        let element = dom.insert(NodeKind::Element { name });
        for attribute in attrs {
            dom.get_mut(element)
                .expect("new element")
                .attributes
                .insert(
                    attribute.name.local.to_string(),
                    attribute.value.to_string(),
                );
        }
        if is_template {
            let contents = dom.insert(NodeKind::DocumentFragment);
            self.templates.borrow_mut().insert(element, contents);
        }
        element
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.dom.borrow_mut().insert(NodeKind::Comment {
            data: text.to_string(),
        })
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.dom.borrow_mut().insert(NodeKind::Comment {
            data: format!("?{} {}?", target, data),
        })
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.append_node_or_text(*parent, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        let parent = self
            .dom
            .borrow()
            .get(*element)
            .ok()
            .and_then(|node| node.parent);
        self.append_node_or_text(parent.unwrap_or(*prev_element), child);
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let doctype = self.dom.borrow_mut().insert(NodeKind::DocumentType {
            name: name.to_string(),
        });
        self.dom
            .borrow_mut()
            .append_child(self.document, doctype)
            .expect("document is live");
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.templates.borrow()[target]
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        match new_node {
            NodeOrText::AppendNode(node) => self
                .dom
                .borrow_mut()
                .insert_before(*sibling, node)
                .expect("valid sibling insertion"),
            NodeOrText::AppendText(text) => {
                let parent = self
                    .dom
                    .borrow()
                    .get(*sibling)
                    .expect("sibling is live")
                    .parent
                    .expect("sibling has parent");
                let node = self.dom.borrow_mut().insert(NodeKind::Text {
                    data: text.to_string(),
                });
                self.dom
                    .borrow_mut()
                    .insert_before(*sibling, node)
                    .expect("valid text insertion");
                debug_assert_eq!(
                    self.dom.borrow().get(node).expect("text").parent,
                    Some(parent)
                );
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut dom = self.dom.borrow_mut();
        let attributes = &mut dom.get_mut(*target).expect("element is live").attributes;
        for attribute in attrs {
            attributes
                .entry(attribute.name.local.to_string())
                .or_insert_with(|| attribute.value.to_string());
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.dom
            .borrow_mut()
            .detach(*target)
            .expect("target is live");
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let children = self
            .dom
            .borrow()
            .get(*node)
            .expect("node is live")
            .children
            .clone();
        for child in children {
            self.dom
                .borrow_mut()
                .append_child(*new_parent, child)
                .expect("valid reparent");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_html_into_oryn_owned_dom() {
        let parsed = parse(
            "<!doctype html><title>Oryn</title><main><button aria-label='Go'>Run</button></main>",
        );
        let document = parsed.dom.get(parsed.document).expect("document");
        assert!(!document.children.is_empty());
        assert_eq!(parsed.quirks_mode, QuirksMode::NoQuirks);
    }

    #[test]
    fn records_parse_errors_without_aborting() {
        let parsed = parse("<table><p>foster parented</table>");
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn html5lib_pinned_tokenizer_and_tree_vectors() {
        // html5lib-tests 224991ec10db04f056a89eed8b0bd8695fd2950e,
        // tokenizer/test1.test: mixed-case doctype, repeated attributes,
        // unclosed paragraphs, and unfinished comments.
        let parsed = parse("<!DOCTYPE HtMl><p a='b' a='d'>One<p>Two<!--comment");
        assert!(parsed.dom.iter().any(|(_, node)| {
            matches!(&node.kind, NodeKind::DocumentType { name } if name == "html")
        }));
        let paragraphs = parsed
            .dom
            .iter()
            .filter(|(_, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "p")
            })
            .collect::<Vec<_>>();
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(
            paragraphs[0].1.attributes.get("a").map(String::as_str),
            Some("b")
        );
        assert!(parsed.dom.iter().any(|(_, node)| {
            matches!(&node.kind, NodeKind::Comment { data } if data == "comment")
        }));
        assert!(!parsed.errors.is_empty());
    }
}
