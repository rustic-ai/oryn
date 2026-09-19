//! XML events are projected into the same native tree sink as HTML.
use crate::html::{OrynTreeSink, ParsedDocument};
use html5ever::{Attribute, QualName};
use markup5ever::interface::{ElementFlags, NodeOrText, TreeSink};
use xml::name::OwnedName;
use xml::reader::{ParserConfig, XmlEvent};

/// Load a navigation response, including image documents backed by a resource URL.
pub fn parse_navigation(source: &str, content_type: Option<&str>, url: &str) -> ParsedDocument {
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    let media = media_document_tag(&essence);
    if media.is_some()
        || matches!(
            essence.as_str(),
            "image/bmp" | "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
    {
        let sink = OrynTreeSink::new_html_document();
        let document = sink.get_document();
        let mut parent = document;
        let resource_tag = media.unwrap_or("img");
        for tag in ["html", "body", resource_tag] {
            let element = sink.create_element(
                QualName::new(None, "http://www.w3.org/1999/xhtml".into(), tag.into()),
                if tag == resource_tag {
                    let mut attributes = vec![Attribute {
                        name: QualName::new(None, "".into(), "src".into()),
                        value: url.into(),
                    }];
                    if media.is_some() {
                        attributes.push(Attribute {
                            name: QualName::new(None, "".into(), "controls".into()),
                            value: "".into(),
                        });
                    }
                    attributes
                } else {
                    Vec::new()
                },
                ElementFlags::default(),
            );
            sink.append(&parent, NodeOrText::AppendNode(element));
            if tag == "html" {
                let head = sink.create_element(
                    QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "head".into()),
                    Vec::new(),
                    ElementFlags::default(),
                );
                sink.append(&element, NodeOrText::AppendNode(head));
            }
            parent = element;
        }
        let mut parsed = sink.finish();
        parsed
            .dom
            .get_mut(document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .content_type = essence;
        parsed
    } else {
        parse_by_mime(source, content_type)
    }
}

/// A media navigation is an HTML resource document retaining its response MIME.
/// The nested media loader owns range/decoder policy; no response byte is parsed
/// as authored markup. Playlist types are excluded until segment policy exists.
pub fn media_document_tag(essence: &str) -> Option<&'static str> {
    let essence = essence
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if essence.contains("mpegurl") {
        None
    } else if essence.starts_with("video/") {
        Some("video")
    } else if essence.starts_with("audio/") || essence == "application/ogg" {
        Some("audio")
    } else {
        None
    }
}

/// Dispatch a loaded document by its response MIME essence.
pub fn parse_by_mime(source: &str, content_type: Option<&str>) -> ParsedDocument {
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    if matches!(essence.as_str(), "application/xml" | "text/xml") || essence.ends_with("+xml") {
        parse(source, &essence)
    } else if is_text_mime(&essence) {
        crate::html::parse_text_document(source, &essence)
    } else {
        crate::html::parse_streamed(source, 4096)
    }
}

/// Whether a normalized MIME essence loads as a text document.
pub fn is_text_mime(essence: &str) -> bool {
    matches!(
        essence,
        "text/plain"
            | "text/css"
            | "text/vtt"
            | "text/javascript"
            | "text/ecmascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
            | "application/javascript"
            | "application/ecmascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "application/json"
            | "text/json"
    ) || essence.ends_with("+json")
}

fn qualified(name: OwnedName) -> QualName {
    QualName::new(
        name.prefix.map(Into::into),
        name.namespace.unwrap_or_default().into(),
        name.local_name.into(),
    )
}

/// Parses an already decoded Unicode string. XML encoding declarations do not
/// reinterpret the string; byte decoding belongs to the resource loader.
pub fn parse(source: &str, content_type: &str) -> ParsedDocument {
    let mut parser = XmlDocumentParser::new(source.to_owned(), content_type);
    loop {
        match parser.advance(256) {
            XmlParseStep::Pending(next) => parser = *next,
            XmlParseStep::Complete(document) => return *document,
            XmlParseStep::LiveComplete { .. } => {
                unreachable!("detached XML parser cannot produce live completion")
            }
        }
    }
}

/// XML fragment parsing retains in-scope namespace bindings but does not
/// insert the synthetic context wrapper into the destination document.
#[derive(Debug)]
pub enum FragmentError {
    Syntax(String),
    Dom(crate::DomError),
}
impl From<crate::DomError> for FragmentError {
    fn from(error: crate::DomError) -> Self {
        Self::Dom(error)
    }
}
pub fn parse_fragment(
    dom: &crate::DomRuntime,
    context: crate::NodeId,
    markup: &str,
) -> Result<(crate::DomRuntime, crate::NodeId), FragmentError> {
    use std::collections::{BTreeMap, BTreeSet};
    let markup = String::from_utf16(&crate::dom_string::units(markup))
        .map_err(|_| FragmentError::Syntax("Invalid XML character".into()))?;
    let mut namespaces = BTreeMap::<String, String>::new();
    let mut current = Some(context);
    let mut seen = BTreeSet::new();
    while let Some(node) = current {
        if !seen.insert(node) {
            return Err(crate::DomError::Cycle.into());
        }
        let record = dom.arena().get(node)?;
        if let crate::NodeKind::Element { name } = &record.kind {
            for attr in record.attributes.records() {
                if attr.namespace.as_deref() == Some("http://www.w3.org/2000/xmlns/") {
                    let prefix = if attr.prefix.as_deref() == Some("xmlns") {
                        attr.local_name.clone()
                    } else {
                        String::new()
                    };
                    namespaces
                        .entry(prefix)
                        .or_insert_with(|| attr.value.clone());
                }
            }
            namespaces
                .entry(
                    name.prefix
                        .as_ref()
                        .map(|prefix| prefix.to_string())
                        .unwrap_or_default(),
                )
                .or_insert_with(|| name.ns.to_string());
        }
        current = record.parent;
    }
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('\t', "&#x9;")
            .replace('\n', "&#xA;")
            .replace('\r', "&#xD;")
    }
    let mut source = String::from("<oryn-fragment");
    for (prefix, namespace) in namespaces {
        if prefix == "xml" || prefix == "xmlns" {
            continue;
        }
        let namespace = String::from_utf16(&crate::dom_string::units(&namespace))
            .map_err(|_| FragmentError::Syntax("Invalid XML namespace character".into()))?;
        source.push_str(" xmlns");
        if !prefix.is_empty() {
            source.push(':');
            source.push_str(&prefix);
        }
        source.push_str("=\"");
        source.push_str(&escape(&namespace));
        source.push('"');
    }
    source.push('>');
    source.push_str(&markup);
    source.push_str("</oryn-fragment>");
    let parsed = parse(&source, "application/xml");
    if !parsed.errors.is_empty() {
        return Err(FragmentError::Syntax(parsed.errors.join("; ")));
    }
    let mut parsed = parsed.into_runtime()?;
    let document = parsed.document();
    let root = parsed
        .arena()
        .get(document)?
        .children
        .iter()
        .copied()
        .find(|node| {
            parsed
                .arena()
                .get(*node)
                .is_ok_and(|record| matches!(record.kind, crate::NodeKind::Element { .. }))
        })
        .ok_or(crate::DomError::NotAnElement(document))?;
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let record = parsed.arena_mut().get_mut(node)?;
        if matches!(&record.kind,crate::NodeKind::Element{name}if name.local.as_ref()=="script"&&matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg"))
        {
            record.script_already_started = true;
        }
        pending.extend(record.children.iter().copied());
    }
    Ok((parsed, root))
}

/// Owned decoded XML input. Byte and event slices suspend with WouldBlock,
/// preserving parser state without reporting premature network EOF.
struct XmlInput {
    chunks: std::collections::VecDeque<Vec<u8>>,
    offset: usize,
    closed: bool,
    waiting: bool,
    slice_bytes: usize,
}
impl std::io::Read for XmlInput {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self
            .chunks
            .front()
            .is_some_and(|chunk| self.offset == chunk.len())
        {
            self.chunks.pop_front();
            self.offset = 0;
        }
        let Some(chunk) = self.chunks.front() else {
            if self.closed {
                return Ok(0);
            }
            self.waiting = true;
            return Err(std::io::ErrorKind::WouldBlock.into());
        };
        // Suspend only at a UTF-8 scalar boundary; xml-rs's scalar decoder
        // keeps its partial bytes on the stack rather than in the reader.
        if self.slice_bytes >= 64 * 1024 && chunk[self.offset] & 0xc0 != 0x80 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let count = output.len().min(chunk.len() - self.offset).min(1);
        self.slice_bytes += count;
        output[..count].copy_from_slice(&chunk[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
pub struct XmlDocumentParser {
    sink: OrynTreeSink,
    stack: Vec<crate::NodeId>,
    reader: xml::reader::EventReader<XmlInput>,
    content_type: String,
    yield_resources: bool,
    input_failure: Option<String>,
    end_started: bool,
    input_started: bool,
}

pub enum XmlParseStep {
    Pending(Box<XmlDocumentParser>),
    Complete(Box<ParsedDocument>),
    LiveComplete {
        document: crate::NodeId,
        events: Vec<crate::html::ParserEvent>,
        errors: Vec<String>,
    },
}

impl XmlDocumentParser {
    pub fn new(source: String, content_type: &str) -> Self {
        let sink = OrynTreeSink::for_xml(content_type);
        let stack = vec![sink.get_document()];
        let reader = ParserConfig::new()
            .preserve_namespace_attributes(true)
            .allow_multiple_root_elements(false)
            .ignore_comments(false)
            .cdata_to_characters(false)
            .ignore_root_level_whitespace(true)
            .override_encoding(Some(xml::Encoding::Utf8))
            .ignore_invalid_encoding_declarations(true)
            .ignore_end_of_stream(true)
            .create_reader(XmlInput {
                chunks: std::collections::VecDeque::from([source.into_bytes()]),
                offset: 0,
                closed: true,
                waiting: false,
                slice_bytes: 0,
            });
        Self {
            sink,
            stack,
            reader,
            content_type: content_type.into(),
            yield_resources: false,
            input_failure: None,
            end_started: false,
            input_started: true,
        }
    }

    /// Live XML shares the selected native Document and yields at each resource
    /// boundary so script close tags execute before later nodes are visible.
    pub fn with_document(
        source: String,
        content_type: &str,
        dom: crate::SharedDomRuntime,
        document: crate::NodeId,
    ) -> Self {
        let mut parser = Self::new(source, content_type);
        parser.sink = OrynTreeSink::with_xml_document(dom, document);
        parser.stack = vec![document];
        parser.yield_resources = true;
        parser.input_started = false;
        parser
    }
    /// Pending decoded input is an IO suspension, never a premature XML EOF.
    /// String chunk boundaries cannot split the decoder's UTF-8 scalar read.
    pub fn streaming_with_document(
        content_type: &str,
        dom: crate::SharedDomRuntime,
        document: crate::NodeId,
    ) -> Self {
        let mut parser = Self::with_document(String::new(), content_type, dom, document);
        parser.reader.source_mut().closed = false;
        parser
    }
    pub fn push_decoded(&mut self, chunk: String) {
        let input = self.reader.source_mut();
        if !chunk.is_empty() {
            input.chunks.push_back(chunk.into_bytes());
        }
        input.waiting = false;
    }
    pub fn fail_input(&mut self, message: String) {
        self.input_failure = Some(message);
    }
    pub fn finish_input(&mut self) {
        let input = self.reader.source_mut();
        input.closed = true;
        input.waiting = false;
    }
    pub fn needs_input(&self) -> bool {
        self.reader.source().waiting && !self.reader.source().closed
    }
    pub fn take_events(&mut self) -> Vec<crate::html::ParserEvent> {
        self.sink.take_xml_events()
    }

    /// Consumes at most event_budget events, retaining all parse state on yield.
    /// Ownership makes terminal completion and cancellation release the reader.
    pub fn advance(mut self, event_budget: usize) -> XmlParseStep {
        if !self.sink.resume_pending_mutations() {
            return XmlParseStep::Pending(Box::new(self));
        }
        if let Some(error) = self.input_failure.take() {
            return XmlParseStep::Complete(Box::new(error_document(&self.content_type, error)));
        }
        if !self.input_started && event_budget > 0 {
            let input = self.reader.source_mut();
            if !input.closed
                && !input
                    .chunks
                    .iter()
                    .enumerate()
                    .any(|(index, chunk)| chunk.len() > if index == 0 { input.offset } else { 0 })
            {
                input.waiting = true;
                return XmlParseStep::Pending(Box::new(self));
            }
            // Reader construction and IO suspension cannot replace authored
            // nodes. Admit replacement once input (or real EOF) can be parsed.
            self.input_started = true;
            if !self.sink.begin_xml_input(&self.content_type) {
                return XmlParseStep::Pending(Box::new(self));
            }
        }
        if self.end_started {
            return self.complete();
        }
        self.reader.source_mut().slice_bytes = 0;
        for _ in 0..event_budget {
            let event = match self.reader.next() {
                Ok(event) => event,
                Err(error) if matches!(error.kind(),xml::reader::ErrorKind::Io(error) if error.kind()==std::io::ErrorKind::WouldBlock) =>
                {
                    return XmlParseStep::Pending(Box::new(self));
                }
                Err(error) => {
                    return XmlParseStep::Complete(Box::new(error_document(
                        &self.content_type,
                        error.to_string(),
                    )));
                }
            };
            let parent = *self.stack.last().expect("document stack");
            match event {
                XmlEvent::StartDocument { .. } => {}
                XmlEvent::EndDocument => {
                    self.end_started = true;
                    return self.complete();
                }
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let attrs = attributes
                        .into_iter()
                        .map(|attr| Attribute {
                            name: qualified(attr.name),
                            value: attr.value.into(),
                        })
                        .collect();
                    let element = self.sink.create_element_with_context(
                        qualified(name),
                        attrs,
                        ElementFlags::default(),
                        markup5ever::interface::tree_builder::ElementCreationContext::Parent(
                            &parent,
                        ),
                    );
                    self.sink.append(&parent, NodeOrText::AppendNode(element));
                    self.stack.push(element);
                }
                XmlEvent::EndElement { .. } => {
                    let element = self.stack.pop().expect("validated XML nesting");
                    self.sink.pop(&element);
                }
                XmlEvent::Characters(text) | XmlEvent::Whitespace(text) => self
                    .sink
                    .append(&parent, NodeOrText::AppendText(text.into())),
                XmlEvent::Comment(text) => {
                    let node = self.sink.create_comment(text.into());
                    self.sink.append(&parent, NodeOrText::AppendNode(node));
                }
                XmlEvent::CData(data) => {
                    let node = self.sink.xml_character_node(data, None);
                    self.sink.append(&parent, NodeOrText::AppendNode(node));
                }
                XmlEvent::ProcessingInstruction { name, data } => {
                    let node = self
                        .sink
                        .xml_character_node(data.unwrap_or_default(), Some(name));
                    self.sink.append(&parent, NodeOrText::AppendNode(node));
                    self.sink.record_xml_pi_stylesheet(node);
                }
                XmlEvent::Doctype { .. } => {
                    if let Some(ids) = self.reader.doctype_ids() {
                        self.sink.append_doctype_to_document(
                            ids.name().into(),
                            ids.public_id().unwrap_or_default().into(),
                            ids.system_id().unwrap_or_default().into(),
                        );
                    }
                }
            }
            if self.sink.construction_interrupted()
                || (self.yield_resources && self.sink.has_xml_events())
            {
                return XmlParseStep::Pending(Box::new(self));
            }
        }
        XmlParseStep::Pending(Box::new(self))
    }

    fn complete(self) -> XmlParseStep {
        // EndDocument is already consumed. Retain the parser and its tracked
        // nodes if final text publication fails; do not ask xml-rs for EOF again.
        if !self.sink.prepare_xml_finish() {
            return XmlParseStep::Pending(Box::new(self));
        }
        if self.yield_resources {
            XmlParseStep::LiveComplete {
                document: self.sink.get_document(),
                events: self.sink.take_xml_events(),
                errors: self.sink.take_xml_errors(),
            }
        } else {
            XmlParseStep::Complete(Box::new(self.sink.finish()))
        }
    }
}

fn error_document(content_type: &str, error: String) -> ParsedDocument {
    let sink = OrynTreeSink::for_xml(content_type);
    sink.parse_error(error.clone().into());
    let element = sink.create_element(
        QualName::new(
            None,
            "http://www.mozilla.org/newlayout/xml/parsererror.xml".into(),
            "parsererror".into(),
        ),
        vec![],
        ElementFlags::default(),
    );
    sink.append(&sink.get_document(), NodeOrText::AppendNode(element));
    sink.append(&element, NodeOrText::AppendText(error.into()));
    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    #[test]
    fn streaming_xml_resumes_every_scalar_split_without_premature_eof() {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let mut parser =
            XmlDocumentParser::streaming_with_document("application/xml", dom.clone(), document);
        for character in
            "<root xmlns='urn:test'><item a='x&amp;y'>雪&amp;<![CDATA[a<b]]><!--c--></item></root>"
                .chars()
        {
            parser.push_decoded(character.to_string());
            loop {
                match parser.advance(4) {
                    XmlParseStep::Pending(next) => {
                        let waiting = next.needs_input();
                        parser = *next;
                        if waiting {
                            break;
                        }
                    }
                    _ => panic!("XML must await native EOF"),
                }
            }
        }
        parser.finish_input();
        loop {
            match parser.advance(4) {
                XmlParseStep::Pending(next) => parser = *next,
                XmlParseStep::LiveComplete { errors, .. } => {
                    assert!(errors.is_empty());
                    break;
                }
                _ => panic!("unexpected detached error"),
            }
        }
        assert_eq!(
            dom.borrow().arena().text_content(document).unwrap(),
            "雪&a<b"
        );
    }
    #[test]
    fn streamed_xml_rejects_real_eof_inside_tokens_after_suspension() {
        for source in [
            "<root><child",
            "<root a='unfinished",
            "<root></root",
            "<root><!-- unfinished",
            "<root><!-- unfinished--",
            "<root><![CDATA[unfinished",
            "<root><![CDATA[unfinished]]",
            "<root>unfinished&amp",
            "<root>unfinished&#12",
            "<root/> <!-- unfinished",
            "<root/> <?trailing unfinished",
            "<!DOCTYPE root [<!ELEMENT root ANY>",
        ] {
            for split in 0..=source.len() {
                let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
                let document = dom.borrow().document();
                let mut parser =
                    XmlDocumentParser::streaming_with_document("application/xml", dom, document);
                for chunk in [&source[..split], &source[split..]] {
                    parser.push_decoded(chunk.into());
                    let mut slices = 0;
                    loop {
                        slices += 1;
                        assert!(
                            slices <= source.len() + 16,
                            "input suspension failed to make progress: {source}, split {split}"
                        );
                        match parser.advance(4) {
                            XmlParseStep::Pending(next) => {
                                let waiting = next.needs_input();
                                parser = *next;
                                if waiting {
                                    break;
                                }
                            }
                            _ => panic!(
                                "input suspension is not terminal EOF: {source}, split {split}"
                            ),
                        }
                    }
                }
                parser.finish_input();
                let mut slices = 0;
                loop {
                    slices += 1;
                    assert!(
                        slices <= source.len() + 16,
                        "real EOF never completed: {source}, split {split}"
                    );
                    match parser.advance(4) {
                        XmlParseStep::Pending(next) => parser = *next,
                        XmlParseStep::Complete(parsed) => {
                            assert!(
                                !parsed.errors.is_empty(),
                                "truncated token accepted: {source}, split {split}"
                            );
                            break;
                        }
                        XmlParseStep::LiveComplete { .. } => {
                            panic!("truncated XML published as complete: {source}, split {split}")
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn streamed_xml_accepts_complete_trailing_markup_after_every_scalar_split() {
        let source = "<?xml version='1.0'?><root a='x&amp;y'>雪\r\n&amp;<![CDATA[a<b]]></root><!--tail--><?done ok?>\r\n";
        for split in (0..=source.len()).filter(|&index| source.is_char_boundary(index)) {
            let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
            let document = dom.borrow().document();
            let mut parser = XmlDocumentParser::streaming_with_document(
                "application/xml",
                dom.clone(),
                document,
            );
            for chunk in [&source[..split], &source[split..]] {
                parser.push_decoded(chunk.into());
                let mut slices = 0;
                loop {
                    slices += 1;
                    assert!(
                        slices <= source.len() + 16,
                        "suspended XML made no progress at {split}"
                    );
                    match parser.advance(4) {
                        XmlParseStep::Pending(next) => {
                            let waiting = next.needs_input();
                            parser = *next;
                            if waiting {
                                break;
                            }
                        }
                        _ => panic!("well-formed XML completed before native EOF at {split}"),
                    }
                }
            }
            parser.finish_input();
            let mut slices = 0;
            loop {
                slices += 1;
                assert!(
                    slices <= source.len() + 16,
                    "finished XML made no progress at {split}"
                );
                match parser.advance(4) {
                    XmlParseStep::Pending(next) => parser = *next,
                    XmlParseStep::LiveComplete { errors, .. } => {
                        assert!(errors.is_empty(), "split {split}: {errors:?}");
                        break;
                    }
                    XmlParseStep::Complete(parsed) => {
                        panic!("well-formed XML rejected at {split}: {:?}", parsed.errors)
                    }
                }
            }
            assert_eq!(
                dom.borrow().arena().text_content(document).unwrap(),
                "雪\n&a<b",
                "split {split}"
            );
        }
    }
    #[test]
    fn xml_large_single_text_event_yields_by_bytes_without_losing_unicode() {
        let text = "雪".repeat(65536);
        let source = format!("<root>{text}</root>");
        let mut parser = XmlDocumentParser::new(source, "application/xml");
        let mut yields = 0;
        loop {
            match parser.advance(256) {
                XmlParseStep::Pending(next) => {
                    assert!(!next.needs_input());
                    parser = *next;
                    yields += 1;
                }
                XmlParseStep::Complete(parsed) => {
                    assert!(parsed.errors.is_empty());
                    assert_eq!(parsed.dom.text_content(parsed.document).unwrap(), text);
                    break;
                }
                _ => panic!("detached parser"),
            }
        }
        assert!(yields >= 2);
    }

    #[test]
    fn live_xml_yields_at_script_close_and_completes_without_tree_snapshot() {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let parser=XmlDocumentParser::with_document("<html xmlns='http://www.w3.org/1999/xhtml'><script>void 0</script><p id='later'/></html>".into(),"application/xhtml+xml",dom.clone(),document);
        let XmlParseStep::Pending(parser) = parser.advance(256) else {
            panic!("script close must yield")
        };
        let mut parser = *parser;
        assert!(
            parser
                .take_events()
                .iter()
                .any(|event| matches!(event, crate::html::ParserEvent::ScriptReady { .. }))
        );
        assert!(
            dom.borrow()
                .query_selector(document, "#later")
                .unwrap()
                .is_none()
        );
        let XmlParseStep::LiveComplete {
            document: completed,
            errors,
            ..
        } = parser.advance(256)
        else {
            panic!("live completion must retain native Document identity")
        };
        assert_eq!(completed, document);
        assert!(errors.is_empty());
        assert!(
            dom.borrow()
                .query_selector(document, "#later")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn xml_script_close_remembers_parser_text_before_author_mutation() {
        for (namespace, mime) in [
            ("http://www.w3.org/1999/xhtml", "application/xhtml+xml"),
            ("http://www.w3.org/2000/svg", "image/svg+xml"),
        ] {
            let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
            let document = dom.borrow().document();
            let source = format!(
                "<root xmlns='{namespace}'><script type='unknown'>original</script><later/></root>"
            );
            let parser = XmlDocumentParser::with_document(source, mime, dom.clone(), document);
            let XmlParseStep::Pending(parser) = parser.advance(256) else {
                panic!("script close must yield")
            };
            let mut parser = *parser;
            let script = parser
                .take_events()
                .into_iter()
                .find_map(|event| match event {
                    crate::html::ParserEvent::ScriptReady { node, .. } => Some(node),
                    _ => None,
                })
                .expect("XML parser publishes script close");
            assert_eq!(
                dom.borrow()
                    .arena()
                    .get(script)
                    .unwrap()
                    .script_trusted_text,
                "original"
            );
            dom.borrow_mut()
                .set_text_content(script, "changed")
                .unwrap();
            let XmlParseStep::LiveComplete { errors, .. } = parser.advance(256) else {
                panic!("XML completion")
            };
            assert!(errors.is_empty());
            assert_eq!(
                dom.borrow()
                    .arena()
                    .get(script)
                    .unwrap()
                    .script_trusted_text,
                "original",
                "parser completion must not bless later author mutation"
            );
        }
    }

    #[test]
    fn owned_xml_parser_yields_and_preserves_late_errors() {
        for malformed in [false, true] {
            let mut source = String::from("<root xmlns='urn:fixture'>");
            for _ in 0..600 {
                source.push_str("<item><![CDATA[value]]></item>");
            }
            source.push_str(if malformed { "</wrong>" } else { "</root>" });
            let parser = XmlDocumentParser::new(source, "application/xml");
            let XmlParseStep::Pending(parser) = parser.advance(0) else {
                panic!("zero event budget must yield")
            };
            let mut parser = *parser;
            assert_eq!(parser.reader.source().offset, 0);
            let mut slices = 0;
            let parsed = loop {
                slices += 1;
                match parser.advance(64) {
                    XmlParseStep::Pending(next) => parser = *next,
                    XmlParseStep::Complete(parsed) => break parsed,
                    XmlParseStep::LiveComplete { .. } => {
                        panic!("detached test parser produced live completion")
                    }
                }
            };
            assert!(slices > 1);
            let document = parsed.dom.get(parsed.document).unwrap();
            assert_eq!(document.children.len(), 1);
            let root = parsed.dom.get(document.children[0]).unwrap();
            if malformed {
                assert!(!parsed.errors.is_empty());
                assert!(parsed.events.is_empty());
                assert!(
                    matches!(&root.kind,NodeKind::Element{name} if name.local.as_ref()=="parsererror")
                );
            } else {
                assert!(parsed.errors.is_empty());
                assert_eq!(root.children.len(), 600);
                assert!(
                    matches!(&root.kind,NodeKind::Element{name} if name.ns.as_ref()=="urn:fixture")
                );
                let item = parsed.dom.get(root.children[599]).unwrap();
                assert!(
                    matches!(&parsed.dom.get(item.children[0]).unwrap().kind,NodeKind::CDataSection{data} if data=="value")
                );
            }
        }
    }

    #[test]
    fn namespaces_and_distinct_character_nodes_survive() {
        let parsed = parse(
            "<?before data?><p:Root xmlns:p='urn:test' p:Attr='v'>a<![CDATA[b]]>c<!--d--></p:Root>",
            "application/xml",
        );
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let doc = parsed.dom.get(parsed.document).unwrap();
        assert!(!doc.document_metadata.as_ref().unwrap().is_html);
        assert!(matches!(
            parsed.dom.get(doc.children[0]).unwrap().kind,
            NodeKind::ProcessingInstruction { .. }
        ));
        let root = parsed.dom.get(doc.children[1]).unwrap();
        assert!(
            matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "Root" && name.ns.as_ref() == "urn:test")
        );
        assert_eq!(root.attributes.get("xmlns:p").unwrap(), "urn:test");
        assert_eq!(root.attributes.get("p:Attr").unwrap(), "v");
        assert_eq!(root.children.len(), 4);
        assert!(matches!(
            parsed.dom.get(root.children[1]).unwrap().kind,
            NodeKind::CDataSection { .. }
        ));
    }

    #[test]
    fn malformed_input_discards_partial_tree_and_resource_events() {
        for source in [
            "<a><b></a>",
            "<a/><b/>",
            "<p:a/>",
            "<a xmlns:p='x' xmlns:p='y'/>",
            "<a x='1' x='2'/>",
            "<a>&missing;</a>",
            "",
        ] {
            let parsed = parse(source, "text/xml");
            assert!(!parsed.errors.is_empty(), "{source}");
            assert!(parsed.events.is_empty());
            let doc = parsed.dom.get(parsed.document).unwrap();
            assert_eq!(doc.children.len(), 1);
            assert!(
                matches!(&parsed.dom.get(doc.children[0]).unwrap().kind, NodeKind::Element { name } if name.local.as_ref() == "parsererror")
            );
        }
        let parsed = parse(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script>test()</script><broken></html>",
            "application/xhtml+xml",
        );
        assert!(!parsed.errors.is_empty());
        assert!(parsed.events.is_empty());
    }

    #[test]
    fn resource_discovery_observes_namespaces() {
        assert!(
            parse(
                "<root><script>test()</script><style>a{}</style><iframe src='x'/></root>",
                "application/xml"
            )
            .events
            .is_empty()
        );
        let parsed = parse(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script>test()</script><style>a{}</style></html>",
            "application/xhtml+xml",
        );
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn mime_dispatch_uses_essence_and_keeps_xml_doctype() {
        let parsed = parse_by_mime(
            "<!DOCTYPE Root PUBLIC 'public-id' 'system-id' [<!ENTITY word 'hello'>]><Root>&word;</Root>",
            Some("Application/XHTML+XML; charset=utf-8"),
        );
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let doc = parsed.dom.get(parsed.document).unwrap();
        assert_eq!(
            doc.document_metadata.as_ref().unwrap().content_type,
            "application/xhtml+xml"
        );
        assert!(
            matches!(&parsed.dom.get(doc.children[0]).unwrap().kind, NodeKind::DocumentType { name, public_id, system_id } if name == "Root" && public_id == "public-id" && system_id == "system-id")
        );
        assert_eq!(
            parsed.dom.text_content(parsed.document).unwrap(),
            "hello",
            "internal subset expansion preserves document text"
        );
        let html = parse_by_mime("<p>hello", None);
        assert!(
            html.dom
                .get(html.document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .is_html
        );
    }

    #[test]
    fn text_navigation_is_literal_across_stream_boundaries() {
        let source = "\n<meta charset=windows-1252>&amp;\r\né<script>run()</script>";
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let mut parser = crate::html::StreamingParser::with_runtime_text(dom.clone(), "text/plain");
        for character in source.chars() {
            parser.push(&character.to_string());
        }
        assert_eq!(parser.bytes_seen(), source.len());
        assert_eq!(
            dom.borrow()
                .arena()
                .text_content(parser.document())
                .unwrap(),
            source.replace("\r\n", "\n")
        );
        let parsed = parser.finish();
        assert!(parsed.events.is_empty());
        assert_eq!(
            parsed.quirks_mode,
            markup5ever::interface::QuirksMode::NoQuirks
        );
        assert_eq!(
            parsed.dom.text_content(parsed.document).unwrap(),
            source.replace("\r\n", "\n")
        );
    }

    #[test]
    fn image_navigation_builds_a_media_host_without_parsing_response_bytes() {
        let url = "https://example.test/picture?quote=\"&tag=<script>";
        for mime in [
            "Image/PNG; charset=binary",
            "image/jpeg",
            "image/gif",
            "image/bmp",
            "image/webp",
        ] {
            let parsed = parse_navigation("<script>unexpected()</script>", Some(mime), url);
            let document = parsed.dom.get(parsed.document).unwrap();
            let metadata = document.document_metadata.as_ref().unwrap();
            assert!(metadata.is_html && !metadata.quirks);
            assert_eq!(
                metadata.content_type,
                mime.split(';').next().unwrap().to_ascii_lowercase()
            );
            assert_eq!(document.children.len(), 1);
            let html = parsed.dom.get(document.children[0]).unwrap();
            assert_eq!(html.children.len(), 2);
            let body = parsed.dom.get(html.children[1]).unwrap();
            assert_eq!(body.children.len(), 1);
            let image = parsed.dom.get(body.children[0]).unwrap();
            assert!(
                matches!(&image.kind, NodeKind::Element { name } if name.local.as_ref() == "img" && name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
            );
            assert_eq!(image.attributes.get("src").map(String::as_str), Some(url));
            assert!(parsed.events.is_empty());
            assert_eq!(parsed.dom.text_content(parsed.document).unwrap(), "");
        }
        let svg = parse_navigation(
            "<svg xmlns='http://www.w3.org/2000/svg'/>",
            Some("image/svg+xml"),
            url,
        );
        assert!(
            !svg.dom
                .get(svg.document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .is_html
        );
    }

    #[test]
    fn text_responses_preserve_mime_and_never_discover_markup_resources() {
        let source = "\n<script src='x'>run()</script>&amp;\r\n<style>x{}</style>";
        for mime in ["Text/Plain; charset=utf-8", "text/css", "application/json"] {
            let parsed = parse_by_mime(source, Some(mime));
            let metadata = parsed
                .dom
                .get(parsed.document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap();
            assert_eq!(
                metadata.content_type,
                mime.split(';').next().unwrap().to_ascii_lowercase()
            );
            assert!(metadata.is_html);
            assert!(!metadata.quirks);
            assert_eq!(
                parsed.quirks_mode,
                markup5ever::interface::QuirksMode::NoQuirks
            );
            assert!(parsed.events.is_empty());
            let pre = parsed
                .dom
                .iter()
                .find_map(|(id, node)| {
                    matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "pre")
                        .then_some(id)
                })
                .unwrap();
            assert_eq!(
                parsed.dom.text_content(pre).unwrap(),
                source.replace("\r\n", "\n")
            );
            assert_eq!(parsed.dom.get(pre).unwrap().children.len(), 1);
        }
    }

    #[test]
    fn unicode_string_does_not_follow_encoding_declaration() {
        let parsed = parse(
            "<?xml version='1.0' encoding='windows-1252'?><a>\u{20ac}\u{6c34}</a>",
            "application/xml",
        );
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    }
}

#[cfg(test)]
mod media_document_tests {
    use super::*;
    #[test]
    fn media_navigation_creates_native_resource_element_with_response_identity() {
        for (mime, tag) in [
            ("video/mp4", "video"),
            ("audio/ogg; codecs=vorbis", "audio"),
        ] {
            let parsed = parse_navigation(
                "<script>untrusted binary must not parse</script>",
                Some(mime),
                "https://media.test/movie?x=1&y=2",
            );
            let dom = parsed.into_runtime().unwrap();
            let root = dom.document();
            let nodes = dom.arena().descendants_inclusive(root).unwrap();
            let media = nodes
                .iter()
                .copied()
                .find(|id| dom.is_element(*id, "http://www.w3.org/1999/xhtml", tag))
                .unwrap();
            assert_eq!(
                dom.attribute_in_null_namespace(media, "src").unwrap(),
                Some("https://media.test/movie?x=1&y=2")
            );
            assert!(
                dom.attribute_in_null_namespace(media, "controls")
                    .unwrap()
                    .is_some()
            );
            assert!(!nodes.iter().any(|id| dom.is_element(
                *id,
                "http://www.w3.org/1999/xhtml",
                "script"
            )));
            assert_eq!(
                dom.arena()
                    .get(root)
                    .unwrap()
                    .document_metadata
                    .as_ref()
                    .unwrap()
                    .content_type,
                mime.split(';').next().unwrap()
            );
        }
    }
}

#[cfg(test)]
mod fragment_regressions {
    use super::*;
    #[test]
    fn fragment_wrapper_cannot_be_closed_by_markup_and_failure_keeps_context() {
        let dom = parse(
            "<root xmlns='urn:root' xmlns:p='urn:parent'><context xmlns:p='urn:child'/></root>",
            "application/xml",
        )
        .into_runtime()
        .unwrap();
        let context = dom
            .arena()
            .descendants_inclusive(dom.document())
            .unwrap()
            .into_iter()
            .find(|node| dom.is_element(*node, "urn:root", "context"))
            .unwrap();
        let before = dom.serialize_xml(dom.document()).unwrap();
        for markup in [
            "x</context><!--y",
            "</oryn-fragment><!--",
            "</oryn-fragment><other/>",
            "<p:x>",
            "<missing:x/>",
            "<!DOCTYPE root><root/>",
        ] {
            assert!(
                matches!(
                    parse_fragment(&dom, context, markup),
                    Err(FragmentError::Syntax(_))
                ),
                "{markup}"
            );
            assert_eq!(dom.serialize_xml(dom.document()).unwrap(), before);
        }
        let (fragment, root) = parse_fragment(
            &dom,
            context,
            "before<p:x/><![CDATA[mid]]><?pi value?><!--tail-->after",
        )
        .unwrap();
        let children = &fragment.arena().get(root).unwrap().children;
        assert_eq!(children.len(), 6);
        assert!(fragment.is_element(children[1], "urn:child", "x"));
        assert!(matches!(
            fragment.arena().get(children[2]).unwrap().kind,
            crate::NodeKind::CDataSection { .. }
        ));
        assert_eq!(dom.serialize_xml(dom.document()).unwrap(), before);
    }
    #[test]
    fn fragment_namespace_values_use_lossless_domstrings() {
        let mut dom = parse("<root/>", "application/xml").into_runtime().unwrap();
        let root = dom.arena().get(dom.document()).unwrap().children[0];
        dom.set_attribute_ns(
            root,
            Some("http://www.w3.org/2000/xmlns/".into()),
            "xmlns:p",
            crate::dom_string::from_units(&[0xd800]),
        )
        .unwrap();
        assert!(matches!(
            parse_fragment(&dom, root, "<p:x/>"),
            Err(FragmentError::Syntax(_))
        ));
    }
}

#[cfg(test)]
mod migrated_parser_handles_tests {
    use super::*;
    #[test]
    fn xml_open_parent_and_character_handles_follow_migration() {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let target = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let mut parser =
            XmlDocumentParser::streaming_with_document("application/xml", dom.clone(), document);
        parser.push_decoded("<root id='parent'>".into());
        let mut parser = match parser.advance(32) {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("open XML input"),
        };
        let parent = dom.borrow().find_by_id("parent").unwrap();
        let target_document = target.borrow().document();
        let mut migration =
            crate::DomRuntime::prepare_subtree_migration(&dom, &target, parent, target_document)
                .unwrap();
        let moved =
            crate::DomRuntime::commit_subtree_migration(&dom, &target, &mut migration).unwrap();
        let _observer = crate::TrackedNodeLease::new(&target, moved.root).unwrap();
        parser.push_decoded(
            "<child id='child'/><![CDATA[text]]><!--comment--><?test value?></root>".into(),
        );
        parser.finish_input();
        loop {
            match parser.advance(32) {
                XmlParseStep::Pending(next) => parser = *next,
                XmlParseStep::LiveComplete { errors, .. } => {
                    assert!(errors.is_empty(), "{errors:?}");
                    break;
                }
                XmlParseStep::Complete(_) => panic!("live completion"),
            }
        }
        let child = target
            .borrow()
            .get_element_by_id(moved.root, "child")
            .unwrap()
            .unwrap();
        assert!(!target.borrow().is_connected(moved.root).unwrap());
        assert_eq!(target.borrow().parent(child).unwrap(), Some(moved.root));
        assert_eq!(
            target.borrow().arena().text_content(moved.root).unwrap(),
            "text"
        );
        assert!(dom.borrow().find_by_id("child").is_none());
        assert_eq!(target.borrow().children(moved.root).unwrap().len(), 4);
    }
}

#[cfg(test)]
mod retained_mutation_failure_tests {
    use super::*;
    #[test]
    fn native_mutation_failure_stops_xml_without_a_v8_hook() {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let parser = XmlDocumentParser::with_document(
            "<root id='first'><child id='later'/></root>".into(),
            "application/xml",
            dom.clone(),
            document,
        );
        crate::html::FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        let mut parser = match parser.advance(128) {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("retain failed root initialization"),
        };
        assert!(parser.sink.construction_interrupted());
        assert!(dom.borrow().find_by_id("later").is_none());
        loop {
            match parser.advance(128) {
                XmlParseStep::Pending(next) => parser = *next,
                XmlParseStep::LiveComplete { errors, .. } => {
                    assert!(errors.is_empty());
                    break;
                }
                _ => panic!("live XML completion"),
            }
        }
        let root = dom.borrow().find_by_id("first").unwrap();
        let child = dom.borrow().find_by_id("later").unwrap();
        assert_eq!(dom.borrow().parent(child).unwrap(), Some(root));
    }
    #[test]
    fn eof_publication_failure_retains_owner_and_completes_once() {
        // Stage the same final-text custody that completion must drain after
        // xml-rs reports EndDocument, independently of reader chunking.
        let mut parser = XmlDocumentParser::new("<root/>".into(), "application/xml");
        parser.end_started = true;
        let document = parser.sink.get_document();
        let root = parser.sink.create_element(
            QualName::new(None, "".into(), "root".into()),
            vec![],
            ElementFlags::default(),
        );
        parser.sink.append(&document, NodeOrText::AppendNode(root));
        parser
            .sink
            .append(&root, NodeOrText::AppendText("retained".into()));
        crate::html::FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        let parser = match parser.complete() {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("failed EOF publication retained"),
        };
        assert!(parser.end_started);
        assert!(parser.sink.construction_interrupted());
        let parsed = match parser.advance(1) {
            XmlParseStep::Complete(parsed) => parsed,
            _ => panic!("retry completes without reading another XML event"),
        };
        assert_eq!(
            parsed.dom.text_content(parsed.document).unwrap(),
            "retained"
        );
    }
    #[test]
    fn xml_end_tag_clears_open_marker_before_terminal_completion() {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let parser = XmlDocumentParser::with_document(
            "<root id='root'/>".into(),
            "application/xml",
            dom.clone(),
            document,
        );
        let parser = match parser.advance(2) {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("document and start element only"),
        };
        let root = dom.borrow().find_by_id("root").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(root)
                .unwrap()
                .parser_open_document
                .is_some()
        );
        let _parser = match parser.advance(1) {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("end tag before EndDocument"),
        };
        assert!(
            dom.borrow()
                .arena()
                .get(root)
                .unwrap()
                .parser_open_document
                .is_none()
        );
    }
}

#[cfg(test)]
mod live_input_admission_tests {
    use super::*;
    fn live() -> (crate::SharedDomRuntime, crate::NodeId, crate::NodeId) {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(crate::DomRuntime::new()));
        let document = dom.borrow().document();
        let retained = dom.borrow_mut().create_element("retained");
        dom.borrow_mut().append_child(document, retained).unwrap();
        (dom, document, retained)
    }
    fn pending(parser: XmlDocumentParser, budget: usize) -> XmlDocumentParser {
        match parser.advance(budget) {
            XmlParseStep::Pending(parser) => *parser,
            _ => panic!("expected retained live parser"),
        }
    }
    #[test]
    fn pending_input_and_buffering_leave_authored_document_untouched_until_parse_admission() {
        let (dom, document, retained) = live();
        let before = dom.borrow().mutation_cursor();
        let content_type = dom
            .borrow()
            .arena()
            .get(document)
            .unwrap()
            .document_metadata
            .clone();
        let mut parser =
            XmlDocumentParser::streaming_with_document("application/xml", dom.clone(), document);
        for _ in 0..3 {
            parser = pending(parser, 32);
            assert!(parser.needs_input());
            assert_eq!(dom.borrow().children(document).unwrap(), &[retained]);
            assert_eq!(dom.borrow().mutation_cursor(), before);
        }
        assert_eq!(
            dom.borrow()
                .arena()
                .get(document)
                .unwrap()
                .document_metadata,
            content_type
        );
        parser.push_decoded("<root xmlns='urn:ready'><child/></root>".into());
        parser = pending(parser, 0);
        assert_eq!(
            dom.borrow().children(document).unwrap(),
            &[retained],
            "buffering and zero parse budget are not publication"
        );
        parser = pending(parser, 32);
        assert!(parser.needs_input());
        assert_eq!(dom.borrow().parent(retained).unwrap(), None);
        let root = dom.borrow().children(document).unwrap()[0];
        assert!(
            matches!(&dom.borrow().arena().get(root).unwrap().kind,crate::NodeKind::Element{name}if name.ns.as_ref()=="urn:ready")
        );
        parser.finish_input();
        match parser.advance(32) {
            XmlParseStep::LiveComplete {
                document: finished,
                errors,
                ..
            } => {
                assert_eq!(finished, document);
                assert!(errors.is_empty());
            }
            _ => panic!("live completion"),
        }
        assert_eq!(dom.borrow().children(document).unwrap(), &[root]);
    }
    #[test]
    fn cancellation_and_input_failure_before_admission_preserve_document_but_real_empty_eof_replaces_it()
     {
        for failure in [false, true] {
            let (dom, document, retained) = live();
            let mut parser = XmlDocumentParser::streaming_with_document(
                "application/xml",
                dom.clone(),
                document,
            );
            parser = pending(parser, 8);
            if failure {
                parser.fail_input("transport failed".into());
                match parser.advance(8) {
                    XmlParseStep::Complete(error) => assert!(
                        error
                            .errors
                            .iter()
                            .any(|error| error.contains("transport failed"))
                    ),
                    _ => panic!("transport error document"),
                }
            } else {
                drop(parser)
            }
            assert_eq!(dom.borrow().children(document).unwrap(), &[retained]);
            assert!(dom.borrow().is_connected(retained).unwrap());
        }
        let (dom, document, retained) = live();
        let mut parser =
            XmlDocumentParser::streaming_with_document("application/xml", dom.clone(), document);
        parser.finish_input();
        match parser.advance(32) {
            XmlParseStep::Complete(error) => assert!(!error.errors.is_empty()),
            _ => panic!("empty XML retains its existing syntax-error behavior"),
        }
        assert!(dom.borrow().children(document).unwrap().is_empty());
        assert_eq!(dom.borrow().parent(retained).unwrap(), None);
    }
    #[test]
    fn admitted_replacement_retries_removal_without_resnapshotting_author_insertions() {
        for failure in 0..36 {
            let (dom, document, retained) = live();
            let observer = crate::TrackedNodeLease::new(&dom, retained).unwrap();
            let mut parser = XmlDocumentParser::streaming_with_document(
                "application/xml",
                dom.clone(),
                document,
            );
            parser.push_decoded("<root/>".into());
            crate::insertion::set_failure_after(Some(failure));
            parser = pending(parser, 32);
            crate::insertion::set_failure_after(None);
            // The input admission has already captured its original children.
            // A node inserted while removal is suspended is outside that set.
            let authored = dom.borrow_mut().create_comment("authored during retry");
            dom.borrow_mut().append_child(document, authored).unwrap();
            assert!(parser.sink.resume_pending_mutations());
            assert_eq!(dom.borrow().parent(retained).unwrap(), None);
            parser = pending(parser, 32);
            assert_eq!(dom.borrow().parent(authored).unwrap(), Some(document));
            parser.finish_input();
            loop {
                match parser.advance(32) {
                    XmlParseStep::Pending(next) => parser = *next,
                    XmlParseStep::LiveComplete { errors, .. } => {
                        assert!(errors.is_empty());
                        break;
                    }
                    _ => panic!("replacement replay corrupted XML"),
                }
            }
            assert_eq!(dom.borrow().parent(authored).unwrap(), Some(document));
            assert_eq!(dom.borrow().parent(retained).unwrap(), None);
            drop(observer);
        }
    }
}
