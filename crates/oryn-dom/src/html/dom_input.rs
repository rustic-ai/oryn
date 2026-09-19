//! Explicit DOMString and Unicode transport for native HTML parsing. html5ever consumes
//! Unicode scalars; this adapter preserves lone UTF-16 surrogates without
//! interpreting the private DOM storage envelope as authored markup.
use super::*;

const ESC: char = '\u{f1000}';
const BASE: u32 = 0xf2000;
fn reserved(value: char) -> bool {
    value == ESC || value == '\u{f0000}' || (BASE..BASE + 0x800).contains(&(value as u32))
}
pub(super) fn escape_scalar(value: char) -> Option<String> {
    reserved(value).then(|| format!("{ESC}{value}"))
}
pub(super) fn encode(value: &str) -> String {
    let mut output = String::new();
    for scalar in char::decode_utf16(crate::dom_string::units(value)) {
        match scalar {
            Ok(value) => {
                if reserved(value) {
                    output.push(ESC);
                }
                output.push(value);
            }
            Err(error) => output.push(
                char::from_u32(BASE + u32::from(error.unpaired_surrogate() - 0xd800)).unwrap(),
            ),
        }
    }
    output
}
pub(super) fn decode(value: &str) -> String {
    let scalar = String::from_utf16(&crate::dom_string::units(value))
        .expect("HTML transport contains Unicode scalars");
    let mut chars = scalar.chars();
    let mut units = Vec::new();
    while let Some(value) = chars.next() {
        if value == ESC {
            if let Some(literal) = chars.next() {
                units.extend(literal.encode_utf16(&mut [0; 2]).iter().copied());
            } else {
                units.extend(value.encode_utf16(&mut [0; 2]).iter().copied());
            }
        } else if (BASE..BASE + 0x800).contains(&(value as u32)) {
            units.push(0xd800 + (value as u32 - BASE) as u16);
        } else {
            units.extend(value.encode_utf16(&mut [0; 2]).iter().copied());
        }
    }
    crate::dom_string::from_units(&units)
}
pub(super) fn name(value: QualName, convert: fn(&str) -> String) -> QualName {
    QualName::new(
        value.prefix.map(|prefix| convert(prefix.as_ref()).into()),
        convert(value.ns.as_ref()).into(),
        convert(value.local.as_ref()).into(),
    )
}
/// Owned, explicitly attributed tokenizer input. Offsets refer to transport
/// bytes; no consumer can mistake a private DOMString envelope for markup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParserInput {
    transport: String,
}
impl ParserInput {
    pub fn unicode(value: &str) -> Self {
        Self {
            transport: encode_unicode(value),
        }
    }
    pub fn dom_string(value: &str) -> Self {
        Self {
            transport: encode(value),
        }
    }
    pub fn len(&self) -> usize {
        self.transport.len()
    }
    pub fn is_empty(&self) -> bool {
        self.transport.is_empty()
    }
    pub fn as_transport(&self) -> &str {
        &self.transport
    }
    pub fn is_char_boundary(&self, index: usize) -> bool {
        if !self.transport.is_char_boundary(index) {
            return false;
        }
        // A literal transport marker is an escape pair. Keep it in the same
        // tokenizer call, including when a resource slice hits its midpoint.
        self.transport[..index]
            .chars()
            .rev()
            .take_while(|value| *value == ESC)
            .count()
            % 2
            == 0
    }
    pub fn prefix(&self, end: usize) -> Self {
        assert!(self.is_char_boundary(end));
        Self {
            transport: self.transport[..end].into(),
        }
    }
    pub fn split_off(&mut self, index: usize) -> Self {
        assert!(self.is_char_boundary(index));
        Self {
            transport: self.transport.split_off(index),
        }
    }
}
pub(super) fn encode_unicode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for value in value.chars() {
        if reserved(value) {
            output.push(ESC);
        }
        output.push(value);
    }
    output
}

/// Parse collision-safe native DOMString storage, with scripting disabled.
pub fn parse_inert_dom_string(input: &str) -> ParsedDocument {
    parse_document_dom_string(input, false)
}
pub fn parse_inert_dom_string_with_declarative_shadow(input: &str) -> ParsedDocument {
    parse_document_dom_string(input, true)
}
pub fn parse_inert_dom_string_with_declarative_shadow_unpatched(input: &str) -> ParsedDocument {
    parse_document_dom_string_options(input, true, false)
}
fn parse_document_dom_string(input: &str, allow_declarative: bool) -> ParsedDocument {
    parse_document_dom_string_options(input, allow_declarative, true)
}
fn parse_document_dom_string_options(
    input: &str,
    allow_declarative: bool,
    patches: bool,
) -> ParsedDocument {
    let mut options = decoded_parse_options();
    options.tree_builder.scripting_enabled = false;
    let sink = OrynTreeSink::new_html_document();
    sink.allow_declarative.set(allow_declarative);
    sink.dom_string_transport.set(true);
    sink.patches_enabled.set(patches);
    let source = encode(input);
    parse_document(sink, options).one(source.as_str())
}
pub fn parse_fragment_dom_string(input: &str, context_local_name: &str) -> ParsedDocument {
    parse_fragment_dom_string_with_context(
        input,
        QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            context_local_name.into(),
        ),
        Vec::new(),
        false,
    )
}
pub fn parse_fragment_dom_string_with_context(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
) -> ParsedDocument {
    parse_fragment_dom_string_options(
        input,
        context,
        attributes,
        has_form_ancestor,
        false,
        false,
        true,
    )
    .0
}
pub fn parse_fragment_dom_string_with_retained_context(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
    allow_declarative: bool,
    disable_context_shadow: bool,
) -> (ParsedDocument, NodeId) {
    parse_fragment_dom_string_options(
        input,
        context,
        attributes,
        has_form_ancestor,
        allow_declarative,
        disable_context_shadow,
        false,
    )
}
fn parse_fragment_dom_string_options(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
    allow_declarative: bool,
    disable_context_shadow: bool,
    patches: bool,
) -> (ParsedDocument, NodeId) {
    let sink = OrynTreeSink::new();
    sink.allow_declarative.set(allow_declarative);
    sink.dom_string_transport.set(true);
    sink.patches_enabled.set(patches);
    let attributes = attributes
        .into_iter()
        .map(|attribute| Attribute {
            name: name(attribute.name, encode),
            value: encode(&attribute.value).into(),
        })
        .collect();
    let context = markup5ever::interface::tree_builder::create_element(
        &sink,
        name(context, encode),
        attributes,
    );
    if disable_context_shadow {
        sink.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(context)
            .expect("fragment context")
            .custom_element_shadow_disabled = Some(true);
    }
    let form = has_form_ancestor.then(|| {
        markup5ever::interface::tree_builder::create_element(
            &sink,
            QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "form".into()),
            Vec::new(),
        )
    });
    let mut parser =
        html5ever::parse_fragment_for_element(sink, decoded_parse_options(), context, true, form);
    parser.process(StrTendril::from(encode(input).as_str()));
    (parser.finish(), context)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lone(source: &str) -> String {
        let mut units = Vec::new();
        for (index, part) in source.split("LONE").enumerate() {
            if index > 0 {
                units.push(0xd800);
            }
            units.extend(part.encode_utf16());
        }
        crate::dom_string::from_units(&units)
    }
    #[test]
    fn inert_transport_preserves_lone_units_names_comments_and_reference_provenance() {
        let input = lone(
            "<p id=p data-LONE='LONE' title='&#xF2000;&#xF1000;&#xF0000;'>LONE\u{f2000}\u{f1000}&#xF2000;&#xD800;<!--LONE--></p><script>LONE&#xF2000;\u{f2000}</script><x-LONE></x-LONE>",
        );
        let parsed = parse_inert_dom_string(&input);
        let dom = parsed.into_runtime().unwrap();
        let p = dom.find_by_id("p").unwrap();
        let expected_name = lone("data-LONE");
        assert_eq!(
            dom.attribute_in_null_namespace(p, &expected_name).unwrap(),
            Some(lone("LONE").as_str())
        );
        assert_eq!(
            crate::dom_string::units(
                dom.attribute_in_null_namespace(p, "title")
                    .unwrap()
                    .unwrap()
            ),
            "\u{f2000}\u{f1000}\u{f0000}"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        let text = dom.arena().child_text_dom_string(p, 4096).unwrap();
        let mut expected = vec![0xd800];
        expected.extend("\u{f2000}\u{f1000}\u{f2000}\u{fffd}".encode_utf16());
        assert_eq!(crate::dom_string::units(&text), expected);
        let nodes = dom.arena().descendants_inclusive(dom.document()).unwrap();
        assert!(nodes.iter().any(|node|matches!(&dom.arena().get(*node).unwrap().kind,NodeKind::Comment{data}if crate::dom_string::units(data)==[0xd800])));
        assert!(nodes.iter().any(|node|matches!(&dom.arena().get(*node).unwrap().kind,NodeKind::Element{name}if name.local.as_ref()==lone("x-LONE"))));
        let script = nodes
            .into_iter()
            .find(|node| dom.is_element(*node, "http://www.w3.org/1999/xhtml", "script"))
            .unwrap();
        assert_eq!(
            crate::dom_string::units(&dom.arena().child_text_dom_string(script, 4096).unwrap()),
            crate::dom_string::units(&lone("LONE&#xF2000;\u{f2000}"))
        );
    }
    #[test]
    fn literal_storage_prefix_and_tokenizer_escape_never_become_surrogates() {
        let literal = format!("{}[65]\u{f1000}\u{f2000}", crate::dom_string::PREFIX);
        let input = crate::dom_string::from_units(
            format!("<p>{literal}</p>")
                .encode_utf16()
                .collect::<Vec<_>>()
                .as_slice(),
        );
        for parsed in [
            parse_inert_dom_string(&input),
            parse_fragment_dom_string(&input, "div"),
        ] {
            let dom = parsed.into_runtime().unwrap();
            let units = dom
                .arena()
                .descendants_inclusive(dom.document())
                .unwrap()
                .into_iter()
                .filter_map(|node| match &dom.arena().get(node).unwrap().kind {
                    NodeKind::Text { data } => Some(crate::dom_string::units(data)),
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(units, literal.encode_utf16().collect::<Vec<_>>());
        }
    }
    #[test]
    fn fragment_context_and_foster_text_decode_complete_escape_pairs() {
        let input = lone("<table>LONE&#xF2000;\u{f1000}<tr><td>LONE</td></tr></table>");
        let parsed = parse_fragment_dom_string(&input, "div");
        let dom = parsed.into_runtime().unwrap();
        let text = dom
            .arena()
            .descendants_inclusive(dom.document())
            .unwrap()
            .into_iter()
            .filter_map(|node| match &dom.arena().get(node).unwrap().kind {
                NodeKind::Text { data } => Some(crate::dom_string::units(data)),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        let mut expected = vec![0xd800];
        expected.extend("\u{f2000}\u{f1000}".encode_utf16());
        expected.push(0xd800);
        assert_eq!(text, expected);
    }
    #[test]
    fn typed_stream_boundaries_preserve_unicode_literals_and_dom_units() {
        let literal = format!("{}[65]\u{f1000}\u{f2000}", crate::dom_string::PREFIX);
        let mut input = ParserInput::unicode(&format!("<body>{literal}"));
        let mut parser = StreamingParser::new();
        while !input.is_empty() {
            let mut end = input.len().min(5);
            while !input.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                end = input.len().min(8);
                while !input.is_char_boundary(end) {
                    end += 1;
                }
            }
            let suffix = input.split_off(end);
            let progress = parser.push_input_until_resource_or_script(&input);
            assert_eq!(progress.consumed_bytes, input.len());
            input = suffix;
        }
        let parsed = parser.finish();
        let units = parsed
            .dom
            .iter()
            .filter_map(|(_, node)| match &node.kind {
                NodeKind::Text { data } => Some(crate::dom_string::units(data)),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(units, literal.encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn typed_construction_retry_keeps_transport_suffix_and_decoded_identity() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let mut input = ParserInput::dom_string(&lone(
            "<body><x-LONE data-v='LONE'><span id=tail></span></x-LONE>",
        ));
        let mut attempts = 0;
        {
            let mut action = |action| match action {
                ParserAction::Checkpoint => Some(ParserNode {
                    owner: dom.clone(),
                    node: dom.borrow().document(),
                }),
                ParserAction::Construct { target, .. } => {
                    attempts += 1;
                    let native = target.owner.borrow();
                    let NodeKind::Element { name } = &native.arena().get(target.node).unwrap().kind
                    else {
                        panic!("native token element")
                    };
                    assert_eq!(name.local.as_ref(), lone("x-LONE"));
                    assert!(
                        native
                            .arena()
                            .get(target.node)
                            .unwrap()
                            .attributes
                            .is_empty()
                    );
                    drop(native);
                    if attempts == 1 { None } else { Some(target) }
                }
                ParserAction::Failed(_) => None,
                ParserAction::Adopt { target, .. } => Some(target),
            };
            let progress = with_parser_actions(&dom, &mut action, || {
                parser.push_input_until_resource_or_script(&input)
            });
            assert!(parser.has_pending_mutations());
            assert!(dom.borrow().find_by_id("tail").is_none());
            input = input.split_off(progress.consumed_bytes);
            let progress = with_parser_actions(&dom, &mut action, || {
                parser.push_input_until_resource_or_script(&input)
            });
            assert_eq!(progress.consumed_bytes, input.len());
        }
        assert_eq!(attempts, 2);
        let tail = dom.borrow().find_by_id("tail").unwrap();
        let parent = dom.borrow().parent(tail).unwrap().unwrap();
        assert_eq!(
            crate::dom_string::units(
                dom.borrow()
                    .attribute_in_null_namespace(parent, "data-v")
                    .unwrap()
                    .unwrap()
            ),
            [0xd800]
        );
    }
}
