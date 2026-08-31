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

use crate::{DomArena, DomError, DomRuntime, NodeId, NodeKind, SharedDomRuntime};

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
/// The executable loader feeds decoded source segments here. Script pause and
/// re-entry are coordinated by the page scheduler while tokenizer and tree
/// builder state remain live in this object.
pub struct StreamingParser {
    inner: Parser<OrynTreeSink>,
    dom: SharedDomRuntime,
    document: NodeId,
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
        Self::with_runtime(std::rc::Rc::new(RefCell::new(DomRuntime::new())))
    }

    /// Start a parser over the same live runtime exposed to V8.
    pub fn with_runtime(dom: SharedDomRuntime) -> Self {
        let document = dom.borrow().document();
        let sink = OrynTreeSink::with_runtime(dom.clone(), document);
        Self {
            inner: parse_document(sink, ParseOpts::default()),
            dom,
            document,
            bytes_seen: 0,
            pending_events: Vec::new(),
        }
    }

    pub fn shared_dom(&self) -> SharedDomRuntime {
        self.dom.clone()
    }

    pub const fn document(&self) -> NodeId {
        self.document
    }

    pub fn push(&mut self, input: &str) {
        self.bytes_seen = self.bytes_seen.saturating_add(input.len());
        self.inner.process(StrTendril::from(input));
    }

    /// Feed input until a parser-blocking script closes. The unconsumed suffix
    /// remains with the caller, which executes the script and resumes parsing.
    pub fn push_until_blocked(&mut self, input: &str) -> ParseProgress {
        let mut consumed = 0;
        while consumed < input.len() {
            // A parser-inserted script cannot become ready before its closing
            // tag's `>`. Feeding through the next delimiter preserves the
            // exact pause boundary without paying one html5ever call per
            // Unicode scalar. Extra delimiters in text/raw-text are harmless.
            let remaining = &input[consumed..];
            let length = remaining
                .find('>')
                .map_or(remaining.len(), |index| index + 1);
            let chunk = &remaining[..length];
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
    dom: SharedDomRuntime,
    document: NodeId,
    errors: RefCell<Vec<String>>,
    templates: RefCell<BTreeMap<NodeId, NodeId>>,
    quirks_mode: Cell<QuirksMode>,
    events: RefCell<Vec<ParserEvent>>,
}

impl OrynTreeSink {
    fn new() -> Self {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        Self::with_runtime(dom, document)
    }

    fn with_runtime(dom: SharedDomRuntime, document: NodeId) -> Self {
        Self {
            dom,
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
                let mut dom = self.dom.borrow_mut();
                dom.parser_detach(node)
                    .expect("html5ever emitted a valid reparent");
                dom.append_child(parent, node)
                    .expect("html5ever emitted a valid append");
            }
            NodeOrText::AppendText(text) => self.append_text(parent, text),
        }
    }

    fn append_text(&self, parent: NodeId, text: StrTendril) {
        let mut dom = self.dom.borrow_mut();
        let last_child = dom
            .arena()
            .get(parent)
            .ok()
            .and_then(|node| node.children.last().copied());
        if let Some(last_child) = last_child
            && let Ok(NodeKind::Text { data }) =
                dom.arena().get(last_child).map(|node| node.kind.clone())
        {
            dom.set_text(last_child, format!("{data}{text}"))
                .expect("parser text node is live");
            return;
        }
        let text = dom.create_text(text.to_string());
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
        let (dom, document) = self.dom.borrow().snapshot();
        ParsedDocument {
            dom,
            document,
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
                dom.arena()
                    .get(*target)
                    .expect("element handle must be live")
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
        let element = dom.create(NodeKind::Element { name });
        for attribute in attrs {
            dom.set_attribute(
                element,
                attribute.name.local.to_string(),
                attribute.value.to_string(),
            )
            .expect("new element");
        }
        if is_template {
            let contents = dom.create_fragment();
            self.templates.borrow_mut().insert(element, contents);
        }
        if let Ok(node) = dom.arena().get(element)
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
        let Ok(node_record) = dom.arena().get(*node) else {
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
        self.dom.borrow_mut().create_comment(text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.dom
            .borrow_mut()
            .create_comment(format!("?{} {}?", target, data))
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
            .arena()
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
        let doctype = self.dom.borrow_mut().create(NodeKind::DocumentType {
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
            NodeOrText::AppendNode(node) => {
                let parent = self
                    .dom
                    .borrow()
                    .parent(*sibling)
                    .expect("sibling is live")
                    .expect("sibling has parent");
                let mut dom = self.dom.borrow_mut();
                dom.parser_detach(node).expect("valid parser reparent");
                dom.insert_before(parent, node, Some(*sibling))
                    .expect("valid sibling insertion");
            }
            NodeOrText::AppendText(text) => {
                let parent = self
                    .dom
                    .borrow()
                    .parent(*sibling)
                    .expect("sibling is live")
                    .expect("sibling has parent");
                let node = self.dom.borrow_mut().create_text(text.to_string());
                self.dom
                    .borrow_mut()
                    .insert_before(parent, node, Some(*sibling))
                    .expect("valid text insertion");
                debug_assert_eq!(self.dom.borrow().parent(node).expect("text"), Some(parent));
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut dom = self.dom.borrow_mut();
        for attribute in attrs {
            let name = attribute.name.local.to_string();
            if dom
                .attribute(*target, &name)
                .expect("element is live")
                .is_none()
            {
                dom.set_attribute(*target, name, attribute.value.to_string())
                    .expect("element is live");
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.dom
            .borrow_mut()
            .parser_detach(*target)
            .expect("target is live");
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let children = self
            .dom
            .borrow()
            .children(*node)
            .expect("node is live")
            .to_vec();
        for child in children {
            let mut dom = self.dom.borrow_mut();
            dom.parser_detach(child).expect("valid parser reparent");
            dom.append_child(*new_parent, child)
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

    #[test]
    fn parser_and_script_side_share_node_identity_and_mutation_sequence() {
        let source = "<main id=before><script>pause()</script><p id=after>after</p></main>";
        let mut parser = StreamingParser::new();
        let dom = parser.shared_dom();
        let progress = parser.push_until_blocked(source);
        let before = dom.borrow().find_by_id("before").expect("live parser node");
        assert!(dom.borrow().find_by_id("after").is_none());
        let script_cursor = dom.borrow().mutation_cursor();
        dom.borrow_mut()
            .set_attribute(before, "data-script", "ran")
            .unwrap();
        parser.push(&source[progress.consumed_bytes..]);
        let parsed = parser.finish();
        assert_eq!(
            parsed
                .dom
                .get(before)
                .unwrap()
                .attributes
                .get("data-script"),
            Some(&"ran".to_owned())
        );
        assert!(dom.borrow().find_by_id("after").is_some());
        let later = dom.borrow().mutations_since(script_cursor);
        assert!(later.mutations.iter().any(|mutation| matches!(
            mutation,
            crate::Mutation::Attribute { node, name, .. }
                if *node == before && name == "data-script"
        )));
        assert!(later.mutations.iter().any(|mutation| matches!(
            mutation,
            crate::Mutation::Inserted {
                parent: Some(_),
                ..
            }
        )));
    }
}
