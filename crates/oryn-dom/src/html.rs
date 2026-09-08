use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};

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
    MetaSecurity {
        content: String,
    },
    MetaReferrer {
        content: String,
    },
    InlineStyleReady {
        node: NodeId,
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

/// Load a decoded text response without interpreting its contents as markup.
pub fn parse_text_document(input: &str, content_type: &str) -> ParsedDocument {
    let mut parser = StreamingParser::with_runtime_text(
        std::rc::Rc::new(RefCell::new(DomRuntime::new())),
        content_type,
    );
    parser.push(input);
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

    /// Start a streaming text document in the same live DOM used by the host.
    pub fn with_runtime_text(dom: SharedDomRuntime, content_type: &str) -> Self {
        let mut parser = Self::with_runtime(dom);
        parser.push("<pre>\n");
        parser.inner.tokenizer.set_plaintext_state();
        parser
            .inner
            .tokenizer
            .sink
            .sink
            .set_quirks_mode(QuirksMode::NoQuirks);
        parser
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(parser.document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .content_type = content_type.into();
        parser.bytes_seen = 0;
        parser
    }

    pub fn shared_dom(&self) -> SharedDomRuntime {
        self.dom.clone()
    }

    pub const fn document(&self) -> NodeId {
        self.document
    }

    pub fn push(&mut self, input: &str) {
        self.process(input);
        self.inner.tokenizer.sink.sink.flush_text();
    }

    fn process(&mut self, input: &str) {
        self.bytes_seen = self.bytes_seen.saturating_add(input.len());
        self.inner.process(StrTendril::from(input));
    }

    /// Feed input until a parser-blocking script closes. The unconsumed suffix
    /// remains with the caller, which executes the script and resumes parsing.
    pub fn push_until_blocked(&mut self, input: &str) -> ParseProgress {
        self.push_until_script_boundary(input, false)
    }

    /// Pause at every script end tag so the host can perform the parser's
    /// microtask checkpoint before preparing even deferred or data scripts.
    /// Nonblocking scripts remain in the event queue, preserving scheduling.
    pub fn push_until_script(&mut self, input: &str) -> ParseProgress {
        self.push_until_script_boundary(input, true)
    }

    fn push_until_script_boundary(&mut self, input: &str, every_script: bool) -> ParseProgress {
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
            self.process(chunk);
            consumed += chunk.len();
            self.capture_events();
            if let Some(index) = self.pending_events.iter().position(|event| {
                matches!(
                    event,
                    ParserEvent::ScriptReady { scheduling, .. }
                        if every_script || *scheduling == ScriptScheduling::Blocking
                )
            }) {
                let blocking = matches!(
                    self.pending_events[index],
                    ParserEvent::ScriptReady {
                        scheduling: ScriptScheduling::Blocking,
                        ..
                    }
                );
                let blocked_on = blocking.then(|| self.pending_events.remove(index));
                self.inner.tokenizer.sink.sink.flush_text();
                return ParseProgress {
                    consumed_bytes: consumed,
                    blocked_on,
                };
            }
        }
        self.inner.tokenizer.sink.sink.flush_text();
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

pub(crate) struct OrynTreeSink {
    dom: SharedDomRuntime,
    document: NodeId,
    errors: RefCell<Vec<String>>,
    quirks_mode: Cell<QuirksMode>,
    events: RefCell<Vec<ParserEvent>>,
    pending_text: RefCell<Option<(NodeId, String)>>,
}

impl OrynTreeSink {
    pub(crate) fn new_html_document() -> Self {
        Self::new()
    }

    fn new() -> Self {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        Self::with_runtime(dom, document)
    }

    fn with_runtime(dom: SharedDomRuntime, document: NodeId) -> Self {
        dom.borrow_mut()
            .arena_mut()
            .get_mut(document)
            .expect("parser document")
            .document_metadata = Some(Box::new(crate::DocumentMetadata::html()));
        Self {
            dom,
            document,
            errors: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            events: RefCell::new(Vec::new()),
            pending_text: RefCell::new(None),
        }
    }

    pub(crate) fn for_xml(content_type: &str) -> Self {
        let sink = Self::new();
        sink.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(sink.document)
            .unwrap()
            .document_metadata = Some(Box::new(crate::DocumentMetadata {
            content_type: content_type.into(),
            is_html: false,
            quirks: false,
        }));
        sink
    }

    pub(crate) fn xml_character_node(&self, data: String, target: Option<String>) -> NodeId {
        self.flush_text();
        let mut dom = self.dom.borrow_mut();
        match target {
            Some(target) => dom.create_processing_instruction(target, data),
            None => dom.create_cdata_section(data),
        }
    }

    fn append_node_or_text(&self, parent: NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.flush_text();
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
        // html5ever can emit many adjacent character tokens within one feed,
        // especially for raw script text. Insert the contiguous run together:
        // retaining old/new copies for every token grows quadratically.
        if let Some((pending_parent, data)) = self.pending_text.borrow_mut().as_mut()
            && *pending_parent == parent
        {
            data.push_str(&text);
            return;
        }
        self.flush_text();
        *self.pending_text.borrow_mut() = Some((parent, text.to_string()));
    }

    fn flush_text(&self) {
        let Some((parent, text)) = self.pending_text.borrow_mut().take() else {
            return;
        };
        let mut dom = self.dom.borrow_mut();
        let last_child = dom
            .arena()
            .get(parent)
            .ok()
            .and_then(|node| node.children.last().copied());
        if let Some(last_child) = last_child
            && let Ok(NodeKind::Text { data }) = dom.arena().get(last_child).map(|node| &node.kind)
        {
            let offset = data.encode_utf16().count() as u32;
            dom.replace_character_data(last_child, offset, 0, text)
                .expect("parser text node is live");
            return;
        }
        let text = dom.create_text(text.to_string());
        dom.append_child(parent, text)
            .expect("text parent is valid");
    }
}

pub(crate) struct OrynElemName<'a> {
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
        self.flush_text();
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
        self.flush_text();
        let is_template = flags.template;
        let mut dom = self.dom.borrow_mut();
        let element = dom.create(NodeKind::Element { name });
        for attribute in attrs {
            let qualified = attribute.name.prefix.as_ref().map_or_else(
                || attribute.name.local.to_string(),
                |prefix| format!("{prefix}:{}", attribute.name.local),
            );
            let namespace =
                (!attribute.name.ns.as_ref().is_empty()).then(|| attribute.name.ns.to_string());
            dom.set_attribute_ns(element, namespace, qualified, attribute.value.to_string())
                .expect("new element");
        }
        if is_template {
            dom.template_content(element).expect("new template");
        }
        if let Ok(node) = dom.arena().get(element)
            && let NodeKind::Element { name } = &node.kind
            && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
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
                "meta" => {
                    let content = node.attributes.get("content").cloned();
                    if node
                        .attributes
                        .get("http-equiv")
                        .is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy"))
                        && let Some(content) = content.clone()
                    {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::MetaSecurity { content });
                    }
                    if node
                        .attributes
                        .get("name")
                        .is_some_and(|value| value.eq_ignore_ascii_case("referrer"))
                        && let Some(content) = content
                    {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::MetaReferrer { content });
                    }
                }
                _ => {}
            }
        }
        element
    }

    fn pop(&self, node: &Self::Handle) {
        self.flush_text();
        let dom = self.dom.borrow();
        let Ok(node_record) = dom.arena().get(*node) else {
            return;
        };
        // Template contents are inert: closing an embedded script or style
        // must not schedule it as a document resource.
        if !dom.is_connected(*node).unwrap_or(false) {
            return;
        }
        let NodeKind::Element { name } = &node_record.kind else {
            return;
        };
        if !matches!(
            name.ns.as_ref(),
            "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
        ) {
            return;
        }
        if name.local.as_ref() == "style" {
            self.events
                .borrow_mut()
                .push(ParserEvent::InlineStyleReady { node: *node });
            return;
        }
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
        self.flush_text();
        self.dom.borrow_mut().create_comment(text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.flush_text();
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
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.flush_text();
        let doctype = self.dom.borrow_mut().create(NodeKind::DocumentType {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        self.dom
            .borrow_mut()
            .append_child(self.document, doctype)
            .expect("document is live");
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.dom
            .borrow()
            .arena()
            .get(*target)
            .expect("live template")
            .template_content
            .expect("template contents")
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
        self.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(self.document)
            .expect("parser document")
            .document_metadata
            .as_mut()
            .expect("HTML document metadata")
            .quirks = mode == QuirksMode::Quirks;
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        self.flush_text();
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
        self.flush_text();
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
        self.flush_text();
        self.dom
            .borrow_mut()
            .parser_detach(*target)
            .expect("target is live");
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.flush_text();
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
    fn foreign_attribute_namespaces_survive_parser_snapshot_and_import() {
        let mut dom = parse("<svg id='svg' xmlns:xlink='http://www.w3.org/1999/xlink' xlink:href='#target' xml:lang='en'></svg>").into_runtime().unwrap();
        let svg = dom.find_by_id("svg").unwrap();
        let href = dom.attribute_node(svg, "xlink:href").unwrap().unwrap();
        assert!(matches!(&dom.arena().get(href).unwrap().kind,
            NodeKind::Attribute { namespace: Some(namespace), prefix: Some(prefix), local_name, .. }
                if namespace == "http://www.w3.org/1999/xlink" && prefix == "xlink" && local_name == "href"));
        let (snapshot, _) = dom.snapshot();
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(&snapshot, svg).unwrap();
        let lang = other.attribute_node(imported, "xml:lang").unwrap().unwrap();
        assert!(matches!(&other.arena().get(lang).unwrap().kind,
            NodeKind::Attribute { namespace: Some(namespace), prefix: Some(prefix), local_name, value, .. }
                if namespace == "http://www.w3.org/XML/1998/namespace" && prefix == "xml" && local_name == "lang" && value == "en"));
    }

    #[test]
    fn parsed_template_scripts_and_styles_are_inert() {
        let parsed = parse(
            "<template id='t'><script>throw Error('inert')</script><style>body{display:none}</style></template><script>active()</script>",
        );
        assert_eq!(
            parsed
                .events
                .iter()
                .filter(|event| matches!(event, ParserEvent::ScriptReady { .. }))
                .count(),
            1
        );
        assert!(
            !parsed
                .events
                .iter()
                .any(|event| matches!(event, ParserEvent::InlineStyleReady { .. }))
        );
        let mut dom = parsed.into_runtime().unwrap();
        let template = dom.find_by_id("t").unwrap();
        let content = dom.template_content(template).unwrap();
        assert_eq!(dom.arena().get(content).unwrap().children.len(), 2);
    }

    #[test]
    fn raw_text_tokens_are_batched_before_observation() {
        let mut parser = StreamingParser::new();
        let source = "a < b && c > d; ".repeat(4096);
        let progress = parser.push_until_blocked(&format!("<script>{source}</script><p>later"));
        assert!(progress.blocked_on.is_some());
        assert_eq!(
            progress.consumed_bytes,
            "<script></script>".len() + source.len()
        );
        let dom = parser.shared_dom();
        let mut dom = dom.borrow_mut();
        let script = dom
            .query_selector(dom.document(), "script")
            .unwrap()
            .unwrap();
        assert_eq!(dom.arena().text_content(script).unwrap(), source);
        let mutations = dom.drain_mutations();
        let retained_text: usize = mutations
            .iter()
            .map(|mutation| match mutation {
                crate::Mutation::CharacterData {
                    old_value,
                    new_value,
                    ..
                } => old_value.len() + new_value.len(),
                _ => 0,
            })
            .sum();
        assert!(
            retained_text <= source.len() * 2,
            "retained {retained_text} bytes"
        );
    }

    #[test]
    fn streamed_text_append_preserves_existing_range_offsets() {
        let mut parser = StreamingParser::new();
        parser.push("<p>ab");
        let dom = parser.shared_dom();
        let (text, range) = {
            let mut dom = dom.borrow_mut();
            let p = dom.query_selector(dom.document(), "p").unwrap().unwrap();
            let text = dom.children(p).unwrap()[0];
            let range = dom
                .create_live_range(
                    crate::BoundaryPoint {
                        container: text,
                        offset: 1,
                    },
                    crate::BoundaryPoint {
                        container: text,
                        offset: 2,
                    },
                )
                .unwrap();
            dom.drain_mutations();
            (text, range)
        };
        parser.push("cd");
        let mut dom = dom.borrow_mut();
        assert_eq!(dom.arena().text_content(text).unwrap(), "abcd");
        let range = dom.live_range(range).unwrap();
        assert_eq!(range.start.offset, 1);
        assert_eq!(range.end.offset, 2);
        let mutations = dom.drain_mutations();
        assert!(mutations.iter().any(|mutation| matches!(mutation,
            crate::Mutation::CharacterData { old_value, new_value, .. }
                if old_value == "ab" && new_value == "abcd")));
    }

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
            matches!(&node.kind, NodeKind::DocumentType { name, .. } if name == "html")
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
