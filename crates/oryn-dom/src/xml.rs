//! XML events are projected into the same native tree sink as HTML.
use crate::html::{OrynTreeSink, ParsedDocument};
use html5ever::{Attribute, QualName};
use markup5ever::interface::{ElementFlags, NodeOrText, TreeSink};
use xml::name::OwnedName;
use xml::reader::{ParserConfig, XmlEvent};

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
    } else if matches!(
        essence.as_str(),
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
    {
        crate::html::parse_text_document(source, &essence)
    } else {
        crate::html::parse_streamed(source, 4096)
    }
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
    let sink = OrynTreeSink::for_xml(content_type);
    let mut stack = vec![sink.get_document()];
    let mut reader = ParserConfig::new()
        .preserve_namespace_attributes(true)
        .allow_multiple_root_elements(false)
        .ignore_comments(false)
        .cdata_to_characters(false)
        .ignore_root_level_whitespace(true)
        .override_encoding(Some(xml::Encoding::Utf8))
        .ignore_invalid_encoding_declarations(true)
        .create_reader(source.as_bytes());
    loop {
        let event = match reader.next() {
            Ok(event) => event,
            Err(error) => return error_document(content_type, error.to_string()),
        };
        let parent = *stack.last().expect("document stack");
        match event {
            XmlEvent::StartDocument { .. } => {}
            XmlEvent::EndDocument => return sink.finish(),
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
                let element = sink.create_element(qualified(name), attrs, ElementFlags::default());
                sink.append(&parent, NodeOrText::AppendNode(element));
                stack.push(element);
            }
            XmlEvent::EndElement { .. } => {
                let element = stack.pop().expect("validated XML nesting");
                sink.pop(&element);
            }
            XmlEvent::Characters(text) | XmlEvent::Whitespace(text) => {
                sink.append(&parent, NodeOrText::AppendText(text.into()))
            }
            XmlEvent::Comment(text) => {
                let node = sink.create_comment(text.into());
                sink.append(&parent, NodeOrText::AppendNode(node));
            }
            XmlEvent::CData(data) => {
                let node = sink.xml_character_node(data, None);
                sink.append(&parent, NodeOrText::AppendNode(node));
            }
            XmlEvent::ProcessingInstruction { name, data } => {
                let node = sink.xml_character_node(data.unwrap_or_default(), Some(name));
                sink.append(&parent, NodeOrText::AppendNode(node));
            }
            XmlEvent::Doctype { .. } => {
                if let Some(ids) = reader.doctype_ids() {
                    sink.append_doctype_to_document(
                        ids.name().into(),
                        ids.public_id().unwrap_or_default().into(),
                        ids.system_id().unwrap_or_default().into(),
                    );
                }
            }
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
