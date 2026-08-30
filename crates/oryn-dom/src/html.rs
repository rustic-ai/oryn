use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeMap;

use html5ever::driver::ParseOpts;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{
    Attribute, LocalName, Namespace, Parser, QualName, parse_document,
    parse_fragment as html5_parse_fragment,
};
use markup5ever::interface::tree_builder::ElemName;
use markup5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};

use crate::{DomArena, NodeId, NodeKind};
use crate::{DomError, DomRuntime};

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub dom: DomArena,
    pub document: NodeId,
    pub errors: Vec<String>,
    pub quirks_mode: QuirksMode,
    pub events: Vec<ParserEvent>,
}

impl ParsedDocument {
    pub fn into_runtime(self) -> Result<DomRuntime, DomError> {
        DomRuntime::from_parsed(self.dom, self.document)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptScheduling {
    Blocking,
    Async,
    Defer,
    Module,
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserEvent {
    ScriptReady {
        node: NodeId,
        scheduling: ScriptScheduling,
    },
    StylesheetDiscovered {
        node: NodeId,
        href: String,
    },
    FrameDiscovered {
        node: NodeId,
        source: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseProgress {
    pub consumed_bytes: usize,
    pub blocked_on: Option<ParserEvent>,
}

pub fn parse(input: &str) -> ParsedDocument {
    let mut parser = StreamingParser::new();
    parser.push(input);
    parser.finish()
}

/// Parse a decoded navigation body through the incremental production driver.
/// Chunk boundaries are kept on UTF-8 scalar boundaries, as they are after the
/// network decoder has completed each incremental decode step.
pub fn parse_streamed(input: &str, max_chunk_bytes: usize) -> ParsedDocument {
    let max_chunk_bytes = max_chunk_bytes.max(1);
    let mut parser = StreamingParser::new();
    let mut start = 0;
    while start < input.len() {
        let mut end = start.saturating_add(max_chunk_bytes).min(input.len());
        while end > start && !input.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = input[start..]
                .char_indices()
                .nth(1)
                .map_or(input.len(), |(offset, _)| start + offset);
        }
        parser.push(&input[start..end]);
        start = end;
    }
    parser.finish()
}

pub fn parse_fragment(input: &str, context_local_name: &str) -> ParsedDocument {
    let context = QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from(context_local_name),
    );
    let mut parser = html5_parse_fragment(
        OrynTreeSink::new(),
        ParseOpts::default(),
        context,
        Vec::new(),
        true,
    );
    parser.process(StrTendril::from(input));
    parser.finish()
}

/// Incremental html5ever driver over Oryn's authoritative Rust DOM.
///
/// Navigation feeds decoded response chunks as they arrive. Script pause and
/// re-entry are coordinated by the page scheduler while tokenizer and tree
/// builder state remain live in this object.
pub struct StreamingParser {
    inner: Parser<OrynTreeSink>,
    bytes_seen: usize,
    pending_events: Vec<ParserEvent>,
}

impl Default for StreamingParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingParser {
    pub fn new() -> Self {
        Self {
            inner: parse_document(OrynTreeSink::new(), ParseOpts::default()),
            bytes_seen: 0,
            pending_events: Vec::new(),
        }
    }

    pub fn push(&mut self, input: &str) {
        self.bytes_seen = self.bytes_seen.saturating_add(input.len());
        self.inner.process(StrTendril::from(input));
    }

    /// Feed input until a parser-blocking script closes. The unconsumed suffix
    /// remains with the caller, which executes the script and resumes parsing.
    pub fn push_until_blocked(&mut self, input: &str) -> ParseProgress {
        let mut consumed = 0;
        for character in input.chars() {
            let mut encoded = [0_u8; 4];
            let chunk = character.encode_utf8(&mut encoded);
            self.push(chunk);
            consumed += chunk.len();
            self.capture_events();
            if let Some(index) = self.pending_events.iter().position(|event| {
                matches!(
                    event,
                    ParserEvent::ScriptReady {
                        scheduling: ScriptScheduling::Blocking,
                        ..
                    }
                )
            }) {
                let event = self.pending_events.remove(index);
                return ParseProgress {
                    consumed_bytes: consumed,
                    blocked_on: Some(event),
                };
            }
        }
        ParseProgress {
            consumed_bytes: consumed,
            blocked_on: None,
        }
    }

    pub fn take_events(&mut self) -> Vec<ParserEvent> {
        self.capture_events();
        std::mem::take(&mut self.pending_events)
    }

    fn capture_events(&mut self) {
        self.pending_events.extend(std::mem::take(
            &mut *self.inner.tokenizer.sink.sink.events.borrow_mut(),
        ));
    }

    pub const fn bytes_seen(&self) -> usize {
        self.bytes_seen
    }

    pub fn finish(mut self) -> ParsedDocument {
        self.capture_events();
        let events = std::mem::take(&mut self.pending_events);
        let mut document = self.inner.finish();
        document.events = events;
        document
    }
}

struct OrynTreeSink {
    dom: RefCell<DomArena>,
    document: NodeId,
    errors: RefCell<Vec<String>>,
    templates: RefCell<BTreeMap<NodeId, NodeId>>,
    quirks_mode: Cell<QuirksMode>,
    events: RefCell<Vec<ParserEvent>>,
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
            events: RefCell::new(Vec::new()),
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
    node: Ref<'a, crate::Node>,
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
            events: self.events.into_inner(),
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
        if let Ok(node) = dom.get(element)
            && let NodeKind::Element { name } = &node.kind
        {
            match name.local.as_ref() {
                "link"
                    if node
                        .attributes
                        .get("rel")
                        .is_some_and(|value| value.eq_ignore_ascii_case("stylesheet")) =>
                {
                    if let Some(href) = node.attributes.get("href") {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::StylesheetDiscovered {
                                node: element,
                                href: href.clone(),
                            });
                    }
                }
                "iframe" => {
                    if let Some(source) = node.attributes.get("src") {
                        self.events.borrow_mut().push(ParserEvent::FrameDiscovered {
                            node: element,
                            source: source.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        element
    }

    fn pop(&self, node: &Self::Handle) {
        let dom = self.dom.borrow();
        let Ok(node_record) = dom.get(*node) else {
            return;
        };
        let NodeKind::Element { name } = &node_record.kind else {
            return;
        };
        if name.local.as_ref() != "script" {
            return;
        }
        let script_type = node_record
            .attributes
            .get("type")
            .map(String::as_str)
            .unwrap_or("");
        let scheduling = if script_type == "module" {
            ScriptScheduling::Module
        } else if !matches!(
            script_type,
            "" | "text/javascript" | "application/javascript" | "text/babel"
        ) {
            ScriptScheduling::Data
        } else if node_record.attributes.contains_key("async") {
            ScriptScheduling::Async
        } else if node_record.attributes.contains_key("defer") {
            ScriptScheduling::Defer
        } else {
            ScriptScheduling::Blocking
        };
        self.events.borrow_mut().push(ParserEvent::ScriptReady {
            node: *node,
            scheduling,
        });
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

    #[test]
    fn incremental_chunks_match_one_shot_tree() {
        let source = "<!doctype html><main><p>one</p><script>0</script><p>two</p></main>";
        let expected = parse(source);
        for split in 0..=source.len() {
            if !source.is_char_boundary(split) {
                continue;
            }
            let mut parser = StreamingParser::new();
            parser.push(&source[..split]);
            parser.push(&source[split..]);
            let actual = parser.finish();
            let expected_nodes = expected
                .dom
                .iter()
                .map(|(_, node)| node.clone())
                .collect::<Vec<_>>();
            let actual_nodes = actual
                .dom
                .iter()
                .map(|(_, node)| node.clone())
                .collect::<Vec<_>>();
            assert_eq!(actual_nodes, expected_nodes, "split at {split}");
        }
    }

    #[test]
    fn parser_can_pause_at_a_blocking_script_and_resume() {
        let source = "<main>before<script>window.x=1</script><p>after</p></main>";
        let mut parser = StreamingParser::new();
        let progress = parser.push_until_blocked(source);
        assert!(matches!(
            progress.blocked_on,
            Some(ParserEvent::ScriptReady {
                scheduling: ScriptScheduling::Blocking,
                ..
            })
        ));
        assert!(progress.consumed_bytes < source.len());
        parser.push(&source[progress.consumed_bytes..]);
        let document = parser.finish();
        assert!(document.dom.iter().any(|(_, node)| matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "p")));
    }

    #[test]
    fn parser_pause_preserves_prior_resource_and_deferred_script_events() {
        let source = "<link rel=stylesheet href=app.css><script defer src=app.js></script><script>block()</script><p>after</p>";
        let mut parser = StreamingParser::new();
        let progress = parser.push_until_blocked(source);
        assert!(progress.blocked_on.is_some());
        let events = parser.take_events();
        assert!(events.iter().any(|event| matches!(event, ParserEvent::StylesheetDiscovered { href, .. } if href == "app.css")));
        assert!(events.iter().any(|event| matches!(
            event,
            ParserEvent::ScriptReady {
                scheduling: ScriptScheduling::Defer,
                ..
            }
        )));
    }

    #[test]
    fn parsed_tree_becomes_the_live_indexed_dom_without_reconstruction() {
        let parsed = parse("<main id=app><button id=go>Go</button></main>");
        let runtime = parsed.into_runtime().unwrap();
        let target = runtime.find_by_id("go").expect("indexed parser node");
        assert_eq!(runtime.arena().text_content(target).unwrap(), "Go");
    }
}
