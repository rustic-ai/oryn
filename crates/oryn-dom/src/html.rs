use std::borrow::Cow;
use std::cell::{Cell, RefCell};

use html5ever::driver::ParseOpts;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, Namespace, Parser, QualName, parse_document};
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
    ImportMap,
    SpeculationRules,
    Data,
}

/// JavaScript MIME type essence matches used by HTML script preparation.
/// Parameters are not accepted in a script type attribute.
pub fn is_javascript_mime_type(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
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
    )
}

pub fn normalize_script_type(value: &str) -> String {
    value
        .trim_matches(|character| matches!(character, '\t' | '\n' | '\u{000c}' | '\r' | ' '))
        .to_ascii_lowercase()
}

/// The legacy event-handler script filter applies only to classic scripts.
/// Missing either attribute leaves ordinary preparation unchanged. HTML ASCII
/// whitespace is stripped; non-ASCII spaces and Unicode case folding do not match.
pub fn classic_script_event_allowed(
    for_attribute: Option<&str>,
    event_attribute: Option<&str>,
) -> bool {
    let (Some(target), Some(event)) = (for_attribute, event_attribute) else {
        return true;
    };
    let target = normalize_script_type(target);
    let event = normalize_script_type(event);
    target == "window" && matches!(event.as_str(), "onload" | "onload()")
}

/// Resolve HTML's script block type before matching supported languages.
/// Explicit empty type selects JavaScript; whitespace-only type is a data
/// block. A language attribute is considered only when type is absent.
pub fn script_block_type(type_attribute: Option<&str>, language: Option<&str>) -> String {
    match type_attribute {
        Some("") => "text/javascript".into(),
        Some(value) => normalize_script_type(value),
        None => match language {
            Some(value) if !value.is_empty() => format!("text/{}", value.to_ascii_lowercase()),
            _ => "text/javascript".into(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserEvent {
    EncodingIndicator {
        label: String,
    },
    EncodingCommitted,
    ScriptReady {
        node: NodeId,
        scheduling: ScriptScheduling,
    },
    ModulePreloadDiscovered {
        node: NodeId,
    },
    ExternalTemplateDiscovered {
        node: NodeId,
    },
    StylesheetDiscovered {
        node: NodeId,
        href: String,
    },
    FrameDiscovered {
        node: NodeId,
        source: String,
    },
    FrameLocalDocument {
        node: NodeId,
    },
    EventHandlerAttributeChanged {
        node: NodeId,
        name: String,
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

#[cfg(test)]
mod resource_boundary_tests {
    use super::*;

    #[test]
    fn contextual_constructor_precedes_attributes_and_returns_actual_parser_handle() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let mut calls = Vec::new();
        let mut replaced = None;
        let mut original_lease = None;
        {
            let mut constructor = |node: NodeId, parent: NodeId| {
                let mut runtime = dom.borrow_mut();
                let record = runtime.arena().get(node).unwrap();
                assert!(record.attributes.is_empty());
                assert!(record.parent.is_none());
                calls.push((node, parent));
                original_lease =
                    Some(crate::TrackedNodeLease::parser_in(&dom, &mut runtime, node).unwrap());
                let fallback = runtime.create_element("x-context");
                let document = runtime.arena().node_document(parent).unwrap().unwrap();
                runtime.set_node_document(fallback, document).unwrap();
                replaced = Some(fallback);
                fallback
            };
            with_element_constructor(&dom, &mut constructor, || {
                parser.push("<body><section id=parent><x-context id=result><span id=child></span></x-context></section></body>")
            });
        }
        assert_eq!(calls.len(), 1);
        let runtime = dom.borrow();
        let result = runtime.find_by_id("result").unwrap();
        assert_eq!(Some(result), replaced);
        assert_eq!(runtime.parent(result).unwrap(), Some(calls[0].1));
        assert_eq!(
            runtime
                .parent(runtime.find_by_id("child").unwrap())
                .unwrap(),
            Some(result)
        );
        assert!(runtime.parent(calls[0].0).unwrap().is_none());
        drop(runtime);
        drop(original_lease);
        assert!(dom.borrow().arena().get(calls[0].0).is_err());
    }
    #[test]
    fn contextual_constructor_uses_foster_parent_and_does_not_escape_lexical_scope() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let mut parents = Vec::new();
        {
            let mut constructor = |node: NodeId, parent: NodeId| {
                parents.push(parent);
                node
            };
            with_element_constructor(&dom, &mut constructor, || {
                parser.push(
                "<body><section id=outer><table><x-foster id=foster></x-foster></table></section>",
            )
            });
        }
        assert_eq!(parents, vec![dom.borrow().find_by_id("outer").unwrap()]);
        parser.push("<x-after></x-after>");
        assert_eq!(parents.len(), 1);
    }
    #[test]
    fn resource_discovery_respects_type_and_srcdoc_precedence() {
        let parsed = parse(
            r#"
            <link rel=stylesheet href=invalid.css type=text/plain>
            <link rel=stylesheet href=''>
            <link rel='alternate stylesheet' href=valid.css type=TEXT/CSS>
            <style type=text/plain>@import 'invalid-inline.css';</style>
            <style type=TEXT/CSS>p { color: green }</style>
            <iframe src=ignored.html srcdoc=''></iframe>
            <iframe src=active.html></iframe>
        "#,
        );
        assert!(matches!(parsed.events.as_slice(), [
            ParserEvent::StylesheetDiscovered { href, .. },
            ParserEvent::InlineStyleReady { .. },
            ParserEvent::FrameLocalDocument { .. },
            ParserEvent::FrameDiscovered { source, .. },
        ] if href == "valid.css" && source == "active.html"));
    }

    #[test]
    fn yields_before_later_base_and_policy_tokens() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let source = "<link rel='alternate stylesheet' href='before.css'><base href='https://cdn.test/'><meta name='referrer' content='no-referrer'><link rel='stylesheet' href='after.css'><p id='later'>later</p>";
        let first = parser.push_until_resource_or_script(source);
        assert!(first.blocked_on.is_none());
        let events = parser.take_events();
        assert!(
            matches!(events.as_slice(), [ParserEvent::StylesheetDiscovered { href, .. }] if href == "before.css")
        );
        let document = dom.borrow().document();
        assert_eq!(
            dom.borrow()
                .arena()
                .resolve_document_base(document, "https://app.test/page", true)
                .unwrap(),
            "https://app.test/page"
        );
        let second = parser.push_until_resource_or_script(&source[first.consumed_bytes..]);
        assert!(
            matches!(parser.take_events().as_slice(), [ParserEvent::MetaReferrer { content }] if content == "no-referrer")
        );
        assert_eq!(
            dom.borrow()
                .arena()
                .resolve_document_base(document, "https://app.test/page", true)
                .unwrap(),
            "https://cdn.test/"
        );
        let offset = first.consumed_bytes + second.consumed_bytes;
        let third = parser.push_until_resource_or_script(&source[offset..]);
        assert!(
            matches!(parser.take_events().as_slice(), [ParserEvent::StylesheetDiscovered { href, .. }] if href == "after.css")
        );
        assert!(dom.borrow().find_by_id("later").is_none());
        parser.push(&source[offset + third.consumed_bytes..]);
        assert!(dom.borrow().find_by_id("later").is_some());
    }

    #[test]
    fn template_resources_do_not_emit_document_load_or_policy_events() {
        let parsed = parse(
            "<template><link rel=stylesheet href=inert.css><iframe src=inert.html></iframe><meta name=referrer content=unsafe-url><meta http-equiv=content-security-policy content=\"default-src 'none'\"></template><link rel=stylesheet href=active.css>",
        );
        assert!(
            matches!(parsed.events.as_slice(), [ParserEvent::StylesheetDiscovered { href, .. }] if href == "active.css")
        );
    }

    #[test]
    fn merged_body_handler_attributes_emit_a_boundary_only_when_added() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        parser.push("<body><p>existing</p>");
        parser.take_events();
        let source = "<body onload='first()'><p id=later>later</p>";
        let progress = parser.push_until_resource_or_script(source);
        assert!(matches!(parser.take_events().as_slice(),
            [ParserEvent::EventHandlerAttributeChanged { name, .. }] if name == "onload"));
        assert!(dom.borrow().find_by_id("later").is_none());
        parser.push(&source[progress.consumed_bytes..]);
        parser.push("<body onload='ignored()' onerror='added()'>");
        assert!(matches!(parser.take_events().as_slice(),
            [ParserEvent::EventHandlerAttributeChanged { name, .. }] if name == "onerror"));
    }
}

// Every entry point receives Unicode, after byte-level BOM processing by
// the resource decoder (or directly from a DOM string API). U+FEFF here is
// document content; removing it again loses a second BOM or author text.
fn decoded_parse_options() -> ParseOpts {
    let mut options = ParseOpts::default();
    options.tokenizer.discard_bom = false;
    options
}

pub fn parse(input: &str) -> ParsedDocument {
    let mut parser = StreamingParser::new();
    parser.push(input);
    parser.finish()
}

/// Parse a detached document with the HTML scripting flag disabled.
pub fn parse_inert(input: &str) -> ParsedDocument {
    let mut options = decoded_parse_options();
    options.tree_builder.scripting_enabled = false;
    let sink = OrynTreeSink::new_html_document();
    sink.allow_declarative.set(false);
    let source = dom_input::encode_unicode(input);
    parse_document(sink, options).one(source.as_str())
}

/// HTML unsafe parsing explicitly opts into declarative shadow construction.
pub fn parse_inert_with_declarative_shadow(input: &str) -> ParsedDocument {
    let mut options = decoded_parse_options();
    options.tree_builder.scripting_enabled = false;
    let source = dom_input::encode_unicode(input);
    parse_document(OrynTreeSink::new_html_document(), options).one(source.as_str())
}

#[cfg(test)]
mod declarative_shadow_tests {
    #[test]
    fn fragment_parser_persists_inert_scripts_in_template_and_shadow_trees() {
        let parsed = super::parse_fragment_with_shadow_context(
            "<script id=plain>0</script><template><script id=nested>1</script></template><section><template shadowrootmode=open><script id=shadow>2</script></template></section>",
            markup5ever::QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "div".into()),
            Vec::new(),
            false,
            true,
        );
        let scripts=parsed.dom.iter().filter(|(_,record)|matches!(&record.kind,crate::NodeKind::Element{name}if name.local.as_ref()=="script")).collect::<Vec<_>>();
        assert_eq!(scripts.len(), 3);
        assert!(
            scripts
                .iter()
                .all(|(_, record)| record.script_already_started)
        );
    }
    #[test]
    fn fragment_declarative_opt_in_preserves_nested_nonclonable_roots() {
        use super::*;
        let markup = "<section id=h><template shadowrootmode=closed shadowrootserializable><span id=n><template shadowrootmode=open><b>nested</b></template></span></template><i>light</i></section>";
        for enabled in [false, true] {
            let parsed = parse_fragment_with_shadow_context(
                markup,
                QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "div".into()),
                Vec::new(),
                false,
                enabled,
            );
            let mut dom = DomRuntime::new();
            let document = dom.import_subtree(&parsed.dom, parsed.document).unwrap();
            let host = dom.get_element_by_id(document, "h").unwrap().unwrap();
            let shadow = dom
                .arena()
                .get(host)
                .unwrap()
                .children
                .iter()
                .copied()
                .find(|node| {
                    matches!(
                        dom.arena().get(*node).unwrap().kind,
                        NodeKind::ShadowRoot { .. }
                    )
                });
            assert_eq!(shadow.is_some(), enabled);
            if let Some(shadow) = shadow {
                assert!(dom.arena().get(shadow).unwrap().shadow_serializable);
                assert!(matches!(
                    dom.arena().get(shadow).unwrap().kind,
                    NodeKind::ShadowRoot {
                        mode: crate::ShadowMode::Closed,
                        clonable: false
                    }
                ));
                let nested = dom.get_element_by_id(shadow, "n").unwrap().unwrap();
                assert!(
                    dom.arena()
                        .get(nested)
                        .unwrap()
                        .children
                        .iter()
                        .any(|node| matches!(
                            dom.arena().get(*node).unwrap().kind,
                            NodeKind::ShadowRoot {
                                mode: crate::ShadowMode::Open,
                                ..
                            }
                        ))
                );
            }
        }
    }
    use super::*;
    #[test]
    fn parser_respects_registered_custom_element_shadow_disable() {
        let runtime = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = runtime.borrow().document();
        runtime
            .borrow_mut()
            .set_parser_shadow_disabled(document, "x-blocked".into())
            .unwrap();
        let mut parser = StreamingParser::with_runtime(runtime.clone());
        parser.push("<x-blocked id=h><template shadowrootmode=open><span>inside</span></template></x-blocked>");
        let parsed = parser.finish();
        let dom = parsed.into_runtime().unwrap();
        let host = dom.get_element_by_id(dom.document(), "h").unwrap().unwrap();
        assert!(
            !dom.arena()
                .get(host)
                .unwrap()
                .children
                .iter()
                .any(|node| matches!(
                    dom.arena().get(*node).unwrap().kind,
                    NodeKind::ShadowRoot { .. }
                ))
        );
    }
    #[test]
    fn entrypoint_flags_and_parser_import_preserve_nonclonable_shadow() {
        let markup = "<div id=h><template shadowrootmode=closed shadowrootreferencetarget=t><span id=t>inside</span></template></div>";
        let inert = parse_inert(markup);
        let inert_host = inert
            .dom
            .descendants_inclusive(inert.document)
            .unwrap()
            .into_iter()
            .find(|node| {
                inert
                    .dom
                    .get(*node)
                    .unwrap()
                    .attributes
                    .get_null("id")
                    .is_some_and(|id| id == "h")
            })
            .unwrap();
        assert!(
            !inert
                .dom
                .get(inert_host)
                .unwrap()
                .children
                .iter()
                .any(|node| matches!(
                    inert.dom.get(*node).unwrap().kind,
                    NodeKind::ShadowRoot { .. }
                ))
        );
        let parsed = parse_inert_with_declarative_shadow(markup);
        let mut destination = DomRuntime::new();
        let imported = destination
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let host = destination
            .get_element_by_id(imported, "h")
            .unwrap()
            .unwrap();
        let shadow = destination
            .arena()
            .get(host)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|node| {
                matches!(
                    destination.arena().get(*node).unwrap().kind,
                    NodeKind::ShadowRoot {
                        mode: crate::ShadowMode::Closed,
                        clonable: false
                    }
                )
            })
            .unwrap();
        assert_eq!(
            destination.shadow_reference_target(shadow).unwrap(),
            Some("t")
        );
        assert!(
            destination
                .get_element_by_id(shadow, "t")
                .unwrap()
                .is_some()
        );
        let clone = destination.clone_node(host, true).unwrap();
        assert!(
            !destination
                .arena()
                .get(clone)
                .unwrap()
                .children
                .iter()
                .any(|node| matches!(
                    destination.arena().get(*node).unwrap().kind,
                    NodeKind::ShadowRoot { .. }
                ))
        );
    }
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
    parse_fragment_with_context(
        input,
        QualName::new(
            None,
            Namespace::from("http://www.w3.org/1999/xhtml"),
            LocalName::from(context_local_name),
        ),
        Vec::new(),
        false,
    )
}

/// Native fragment context carries namespace/integration-point attributes and
/// the ancestor form pointer; the resulting inert arena is imported by caller.
pub fn parse_fragment_with_context(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
) -> ParsedDocument {
    parse_fragment_with_shadow_context(input, context, attributes, has_form_ancestor, false)
}

/// Unsafe HTML setters opt into declarative shadow parsing explicitly; ordinary
/// innerHTML and contextual fragments continue to use the inert default above.
pub fn parse_fragment_with_shadow_context(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
    allow_declarative: bool,
) -> ParsedDocument {
    parse_fragment_with_retained_context(
        input,
        context,
        attributes,
        has_form_ancestor,
        allow_declarative,
        false,
    )
    .0
}
/// Retain the parser's real context handle: a top-level declarative template
/// attaches to this detached context rather than the synthetic fragment root.
pub fn parse_fragment_with_retained_context(
    input: &str,
    context: QualName,
    attributes: Vec<Attribute>,
    has_form_ancestor: bool,
    allow_declarative: bool,
    disable_context_shadow: bool,
) -> (ParsedDocument, NodeId) {
    let sink = OrynTreeSink::new();
    sink.allow_declarative.set(allow_declarative);
    let attributes = attributes
        .into_iter()
        .map(|attribute| Attribute {
            name: dom_input::name(attribute.name, dom_input::encode),
            value: dom_input::encode(&attribute.value).into(),
        })
        .collect();
    let context = markup5ever::interface::tree_builder::create_element(
        &sink,
        dom_input::name(context, dom_input::encode),
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
    parser.process(StrTendril::from(dom_input::encode_unicode(input).as_str()));
    (parser.finish(), context)
}

/// Lexically scoped native constructor callback. The parser owns no V8 state:
/// the embedding host installs this only while its entered scope calls push.
/// The callback is removed before returning or unwinding and is selected by
/// native arena identity, so nested unrelated parsing cannot borrow it.
pub struct ParserNode {
    pub owner: SharedDomRuntime,
    pub node: NodeId,
}
pub enum ParserAction {
    Checkpoint,
    Construct {
        target: ParserNode,
        parent: ParserNode,
    },
    Adopt {
        target: ParserNode,
        parent: ParserNode,
    },
    Failed(crate::DomError),
}
struct ConstructionHook {
    arena: *const RefCell<DomRuntime>,
    callback: *mut (),
    invoke: unsafe fn(*mut (), ParserAction) -> Option<ParserNode>,
    entered: bool,
    interrupted: bool,
}
thread_local! {static CONSTRUCTION_HOOKS:RefCell<Vec<ConstructionHook>>=const{RefCell::new(Vec::new())};}
pub fn with_element_constructor<F: FnMut(NodeId, NodeId) -> NodeId, R>(
    dom: &SharedDomRuntime,
    callback: &mut F,
    parse: impl FnOnce() -> R,
) -> R {
    let mut adapter = |action| match action {
        ParserAction::Construct { target, parent } => Some(ParserNode {
            owner: target.owner,
            node: callback(target.node, parent.node),
        }),
        ParserAction::Checkpoint => Some(ParserNode {
            owner: dom.clone(),
            node: dom.borrow().document(),
        }),
        _ => None,
    };
    with_parser_actions(dom, &mut adapter, parse)
}
pub fn with_parser_actions<F: FnMut(ParserAction) -> Option<ParserNode>, R>(
    dom: &SharedDomRuntime,
    callback: &mut F,
    parse: impl FnOnce() -> R,
) -> R {
    unsafe fn invoke<F: FnMut(ParserAction) -> Option<ParserNode>>(
        callback: *mut (),
        action: ParserAction,
    ) -> Option<ParserNode> {
        // SAFETY: lexical exclusive callback borrow; entered prevents reentry.
        unsafe { (&mut *callback.cast::<F>())(action) }
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CONSTRUCTION_HOOKS.with(|hooks| {
                hooks.borrow_mut().pop();
            });
        }
    }
    CONSTRUCTION_HOOKS.with(|hooks| {
        hooks.borrow_mut().push(ConstructionHook {
            arena: std::rc::Rc::as_ptr(dom),
            callback: (callback as *mut F).cast(),
            invoke: invoke::<F>,
            entered: false,
            interrupted: false,
        })
    });
    let guard = Guard;
    let result = parse();
    drop(guard);
    result
}

/// End the current tokenizer slice after its already-accepted token. This
/// preserves source suffixes when constructor infrastructure fails or its
/// native document is retired; a callback is never replayed to consume input.
pub fn interrupt_element_construction(dom: &SharedDomRuntime) {
    CONSTRUCTION_HOOKS.with(|hooks| {
        if let Some(hook) = hooks
            .borrow_mut()
            .iter_mut()
            .rev()
            .find(|hook| hook.arena == std::rc::Rc::as_ptr(dom))
        {
            hook.interrupted = true;
        }
    });
}
fn parser_actions_installed(dom: &SharedDomRuntime) -> bool {
    CONSTRUCTION_HOOKS.with(|hooks| {
        hooks
            .borrow()
            .iter()
            .any(|hook| hook.arena == std::rc::Rc::as_ptr(dom))
    })
}
fn element_construction_interrupted(dom: &SharedDomRuntime) -> bool {
    CONSTRUCTION_HOOKS.with(|hooks| {
        hooks
            .borrow()
            .iter()
            .rev()
            .find(|hook| hook.arena == std::rc::Rc::as_ptr(dom))
            .is_some_and(|hook| hook.interrupted)
    })
}
fn parser_action(dom: &SharedDomRuntime, action: ParserAction) -> Option<ParserNode> {
    let hook = CONSTRUCTION_HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        let (index, hook) = hooks
            .iter_mut()
            .enumerate()
            .rev()
            .find(|(_, hook)| hook.arena == std::rc::Rc::as_ptr(dom) && !hook.entered)?;
        hook.entered = true;
        Some((index, hook.callback, hook.invoke))
    });
    let (index, callback, invoke) = hook?;
    struct Entered(usize);
    impl Drop for Entered {
        fn drop(&mut self) {
            CONSTRUCTION_HOOKS.with(|hooks| {
                if let Some(hook) = hooks.borrow_mut().get_mut(self.0) {
                    hook.entered = false;
                }
            });
        }
    }
    let entered = Entered(index);
    // SAFETY: lexical installed callback and entered guard prevent aliasing.
    let result = unsafe { invoke(callback, action) };
    drop(entered);
    result
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
    end_started: bool,
}

impl Default for StreamingParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingParser {
    /// Currently buffered source custody. Completed DOM content is owned by
    /// native node accounting, not charged repeatedly as parser input.
    pub fn external_template_cleanup_started(&self) -> bool {
        self.inner
            .tokenizer
            .sink
            .sink
            .external_cleanup_started
            .get()
    }
    pub fn retained_input_bytes(&self) -> usize {
        let sink = &self.inner.tokenizer.sink.sink;
        let attributes = |attrs: &[Attribute]| {
            attrs
                .iter()
                .map(|attr| {
                    attr.name.local.len()
                        + attr.name.ns.len()
                        + attr.name.prefix.as_ref().map_or(0, |prefix| prefix.len())
                        + attr.value.len()
                })
                .sum::<usize>()
        };
        self.inner.input_buffer.retained_bytes()
            + self.inner.tokenizer.retained_input_bytes()
            + self.inner.tokenizer.sink.retained_input_bytes()
            + sink
                .pending_text
                .borrow()
                .as_ref()
                .map_or(0, |(_, text)| text.len())
            + sink
                .pending_mutations
                .borrow()
                .iter()
                .map(|mutation| match mutation {
                    ParserMutation::Text { data, .. } => data.len(),
                    ParserMutation::Initialize { attrs, .. }
                    | ParserMutation::Attributes { attrs, .. } => attributes(attrs),
                    _ => 0,
                })
                .sum::<usize>()
    }
    pub fn new() -> Self {
        Self::with_runtime(std::rc::Rc::new(RefCell::new(DomRuntime::new())))
    }

    /// Start a parser over the same live runtime exposed to V8.
    pub fn with_runtime(dom: SharedDomRuntime) -> Self {
        let document = dom.borrow().document();
        Self::with_document(dom, document)
    }

    /// Parse into a native child Document in a shared arena.
    pub fn with_document(dom: SharedDomRuntime, document: NodeId) -> Self {
        let sink = OrynTreeSink::with_runtime(dom.clone(), document);
        Self {
            inner: parse_document(sink, decoded_parse_options()),
            dom,
            document,
            bytes_seen: 0,
            pending_events: Vec::new(),
            end_started: false,
        }
    }

    /// Start a streaming text document in the same live DOM used by the host.
    pub fn with_runtime_text(dom: SharedDomRuntime, content_type: &str) -> Self {
        let document = dom.borrow().document();
        Self::with_document_text(dom, document, content_type)
    }

    pub fn with_document_text(dom: SharedDomRuntime, document: NodeId, content_type: &str) -> Self {
        let mut parser = Self::with_document(dom, document);
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

    pub fn enable_encoding_restart_events(&self) {
        self.inner.tokenizer.sink.sink.encoding_events.set(true);
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

    /// Feed an inert/detached document's native DOMString storage. Active
    /// documents use typed input and resource boundaries to retain callbacks.
    pub fn push_dom_string(&mut self, input: &str) {
        let input = ParserInput::dom_string(input);
        self.bytes_seen = self.bytes_seen.saturating_add(input.len());
        self.inner.process(StrTendril::from(input.as_transport()));
        self.inner.tokenizer.sink.sink.flush_text();
    }

    fn process(&mut self, input: &str) {
        self.bytes_seen = self.bytes_seen.saturating_add(input.len());
        let source = dom_input::encode_unicode(input);
        self.inner.process(StrTendril::from(source.as_str()));
    }

    /// Feed input until a parser-blocking script closes. The unconsumed suffix
    /// remains with the caller, which executes the script and resumes parsing.
    pub fn push_until_blocked(&mut self, input: &str) -> ParseProgress {
        self.push_until_script_boundary(input, false, false)
    }

    /// Pause at every script end tag so the host can perform the parser's
    /// microtask checkpoint before preparing even deferred or data scripts.
    /// Nonblocking scripts remain in the event queue, preserving scheduling.
    pub fn push_until_script(&mut self, input: &str) -> ParseProgress {
        self.push_until_script_boundary(input, true, false)
    }

    /// Yield after resource and policy events as well as scripts, before later
    /// tokens can change the document base or policy observed by the host.
    pub fn push_until_resource_or_script(&mut self, input: &str) -> ParseProgress {
        self.push_until_script_boundary(input, true, true)
    }

    pub fn push_input_until_resource_or_script(&mut self, input: &ParserInput) -> ParseProgress {
        self.push_until_script_boundary_inner(input.as_transport(), true, true, true)
    }
    fn push_until_script_boundary(
        &mut self,
        input: &str,
        every_script: bool,
        resource_boundaries: bool,
    ) -> ParseProgress {
        self.push_until_script_boundary_inner(input, every_script, resource_boundaries, false)
    }
    fn push_until_script_boundary_inner(
        &mut self,
        input: &str,
        every_script: bool,
        resource_boundaries: bool,
        transport: bool,
    ) -> ParseProgress {
        let mut consumed = 0;
        if !self.inner.tokenizer.sink.sink.resume_pending_mutations() {
            return ParseProgress {
                consumed_bytes: 0,
                blocked_on: None,
            };
        }
        if let Some(index)=self.pending_events.iter().position(|event|matches!(event,ParserEvent::ScriptReady{scheduling,..} if every_script||*scheduling==ScriptScheduling::Blocking)){
            let blocking=matches!(self.pending_events[index],ParserEvent::ScriptReady{scheduling:ScriptScheduling::Blocking,..});
            return ParseProgress{consumed_bytes:0,blocked_on:blocking.then(||self.pending_events.remove(index))};
        }
        if resource_boundaries && !self.pending_events.is_empty() {
            return ParseProgress {
                consumed_bytes: 0,
                blocked_on: None,
            };
        }
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
            if transport {
                self.bytes_seen = self.bytes_seen.saturating_add(chunk.len());
                self.inner.process(StrTendril::from(chunk));
            } else {
                self.process(chunk);
            }
            consumed += chunk.len();
            self.capture_events();
            if self.inner.tokenizer.sink.sink.construction_interrupted() {
                return ParseProgress {
                    consumed_bytes: consumed,
                    blocked_on: None,
                };
            }
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
            if resource_boundaries && !self.pending_events.is_empty() {
                self.inner.tokenizer.sink.sink.flush_text();
                return ParseProgress {
                    consumed_bytes: consumed,
                    blocked_on: None,
                };
            }
        }
        self.inner.tokenizer.sink.sink.flush_text();
        ParseProgress {
            consumed_bytes: consumed,
            blocked_on: None,
        }
    }

    pub fn has_pending_events(&self) -> bool {
        !self.pending_events.is_empty()
    }
    pub fn prepend_events(&mut self, mut events: Vec<ParserEvent>) {
        events.append(&mut self.pending_events);
        self.pending_events = events;
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

    /// Emit EOF once while the owner still retains the parser. Failed native
    /// mutation remains queued and can be retried without repeating EOF tokens.
    pub fn prepare_finish(&mut self) -> bool {
        if !self.inner.tokenizer.sink.sink.resume_pending_mutations() {
            return false;
        }
        if !self.end_started {
            self.end_started = true;
            self.inner.tokenizer.end();
        }
        self.inner.tokenizer.sink.sink.flush_text();
        if self.inner.tokenizer.sink.sink.mutation_failed.get() {
            self.capture_events();
            return false;
        }
        if let Some(node) = self.inner.tokenizer.sink.sink.external_template.take() {
            self.inner
                .tokenizer
                .sink
                .sink
                .external_cleanup_started
                .set(true);
            self.inner
                .tokenizer
                .sink
                .sink
                .queue_mutation(ParserMutation::Pop(node));
        }
        if !self.inner.tokenizer.sink.sink.prepare_buffered_external() {
            self.capture_events();
            return false;
        }
        self.capture_events();
        !self.inner.tokenizer.sink.sink.mutation_failed.get()
    }
    pub fn has_pending_mutations(&self) -> bool {
        !self
            .inner
            .tokenizer
            .sink
            .sink
            .pending_mutations
            .borrow()
            .is_empty()
            || self.end_started
                && self
                    .inner
                    .tokenizer
                    .sink
                    .sink
                    .external_buffered
                    .borrow()
                    .as_ref()
                    .is_some_and(|state| !state.queued)
    }
    pub fn finish(mut self) -> ParsedDocument {
        assert!(
            self.prepare_finish(),
            "owner must retain interrupted parser before completion"
        );
        self.capture_events();
        let events = std::mem::take(&mut self.pending_events);
        let mut document = self.inner.tokenizer.sink.sink.finish();
        document.events = events;
        document
    }
}

mod fragment_stream;
use fragment_stream::BufferedExternal;
pub use fragment_stream::ExternalTemplateOptions;
mod partial_updates;
pub use partial_updates::apply_fragment_patches;
mod dom_input;
pub use dom_input::{
    ParserInput, parse_fragment_dom_string, parse_fragment_dom_string_with_context,
    parse_fragment_dom_string_with_retained_context, parse_inert_dom_string,
    parse_inert_dom_string_with_declarative_shadow,
    parse_inert_dom_string_with_declarative_shadow_unpatched,
};
mod handles;
use handles::ParserHandles;

#[cfg(test)]
thread_local! {pub(crate) static FAIL_PARSER_MUTATION:Cell<bool>=const{Cell::new(false)};}

#[derive(Clone)]
enum ParserMutation {
    Initialize {
        node: NodeId,
        parent: Option<NodeId>,
        attrs: Vec<Attribute>,
        template: bool,
        custom: bool,
        template_assignment: Option<(
            crate::NodeRef,
            NodeId,
            std::rc::Rc<RefCell<crate::insertion::DocumentAssignment>>,
        )>,
    },
    Insert {
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
        receipt: Option<std::rc::Rc<RefCell<crate::insertion::TreeInsertion>>>,
    },
    Foster {
        element: NodeId,
        previous: NodeId,
        node: NodeId,
    },
    Reparent {
        node: NodeId,
        parent: NodeId,
    },
    Detach {
        node: NodeId,
        receipt: Option<std::rc::Rc<RefCell<crate::insertion::TreeRemoval>>>,
    },
    Pop(NodeId),
    Patch {
        node: NodeId,
        receipt: std::rc::Rc<RefCell<partial_updates::PatchCursor>>,
    },
    StartStream {
        node: NodeId,
        content: NodeId,
        scope: NodeId,
    },
    StreamPatch {
        node: NodeId,
        receipt: std::rc::Rc<RefCell<partial_updates::StreamingPatch>>,
    },
    XmlPi(NodeId),
    BeginXml(String),
    Text {
        parent: NodeId,
        data: String,
        before: Option<NodeId>,
    },
    Attributes {
        node: NodeId,
        attrs: Vec<Attribute>,
    },
}

pub(crate) struct OrynTreeSink {
    dom: SharedDomRuntime,
    parser_roots: ParserHandles,
    pending_mutations: RefCell<std::collections::VecDeque<ParserMutation>>,
    mutation_failed: Cell<bool>,
    document: NodeId,
    errors: RefCell<Vec<String>>,
    quirks_mode: Cell<QuirksMode>,
    events: RefCell<Vec<ParserEvent>>,
    pending_text: RefCell<Option<(NodeId, String)>>,
    recorded_resources: RefCell<std::collections::BTreeSet<NodeId>>,
    open_elements: RefCell<Vec<NodeId>>,
    pending_open_elements: RefCell<Vec<NodeId>>,
    allow_declarative: Cell<bool>,
    encoding_events: Cell<bool>,
    dom_string_transport: Cell<bool>,
    xslt_source: Cell<bool>,
    patches_enabled: Cell<bool>,
    patch_scope: Cell<Option<NodeId>>,
    fragment_root_key: Cell<Option<NodeId>>,
    external_template: Cell<Option<NodeId>>,
    external_cleanup_started: Cell<bool>,
    external_buffered: RefCell<Option<BufferedExternal>>,
    fragment_run_scripts: Cell<Option<bool>>,
    fragment_policy: RefCell<Option<(crate::sanitizer::Config, bool)>>,
    fragment_extra_policy: RefCell<Option<(crate::sanitizer::Config, bool)>>,
    stream_patches: RefCell<
        std::collections::BTreeMap<NodeId, std::rc::Rc<RefCell<partial_updates::StreamingPatch>>>,
    >,
    stream_locations: RefCell<std::collections::BTreeMap<NodeId, (NodeId, Option<NodeId>)>>,
    stream_cleanup: RefCell<std::collections::BTreeMap<NodeId, Vec<NodeId>>>,
    stream_requested: RefCell<std::collections::BTreeSet<NodeId>>,
    filtered_nodes: RefCell<std::collections::BTreeSet<NodeId>>,
    unwrapped_nodes: RefCell<std::collections::BTreeMap<NodeId, NodeId>>,
}

impl OrynTreeSink {
    fn payload(&self, value: &str) -> String {
        if self.dom_string_transport.get() {
            dom_input::decode(value)
        } else {
            crate::dom_string::from_units(&value.encode_utf16().collect::<Vec<_>>())
        }
    }
    fn payload_name(&self, value: QualName) -> QualName {
        QualName::new(
            value
                .prefix
                .map(|value| self.payload(value.as_ref()).into()),
            self.payload(value.ns.as_ref()).into(),
            self.payload(value.local.as_ref()).into(),
        )
    }
    fn resolve(&self, key: NodeId) -> (SharedDomRuntime, NodeId) {
        self.parser_roots.resolve(key)
    }
    fn key(&self, owner: &SharedDomRuntime, node: NodeId) -> NodeId {
        self.parser_roots.register(owner, node)
    }
    fn parent_key(&self, key: NodeId) -> Option<NodeId> {
        let (owner, node) = self.resolve(key);
        let parent = owner.borrow().parent(node).ok().flatten();
        parent.map(|parent| self.key(&owner, parent))
    }
    fn queue_mutation(&self, mutation: ParserMutation) {
        self.pending_mutations.borrow_mut().push_back(mutation);
        if !self.mutation_failed.get() {
            self.drain_mutations();
        }
    }
    pub(crate) fn resume_pending_mutations(&self) -> bool {
        if parser_actions_installed(&self.dom)
            && parser_action(&self.dom, ParserAction::Checkpoint).is_none()
        {
            self.mutation_failed.set(true);
            return false;
        }
        self.mutation_failed.set(false);
        self.drain_mutations();
        !self.mutation_failed.get()
    }
    fn drain_mutations(&self) {
        loop {
            let pending = self.pending_mutations.borrow().front().cloned();
            let Some(pending) = pending else {
                return;
            };
            #[cfg(test)]
            if FAIL_PARSER_MUTATION.with(|fault| fault.replace(false)) {
                self.mutation_failed.set(true);
                interrupt_element_construction(&self.dom);
                parser_action(&self.dom, ParserAction::Failed(DomError::MigrationCapacity));
                return;
            }
            if let Err(error) = self.apply_mutation(pending) {
                self.mutation_failed.set(true);
                interrupt_element_construction(&self.dom);
                parser_action(&self.dom, ParserAction::Failed(error));
                return;
            }
            self.pending_mutations.borrow_mut().pop_front();
        }
    }
    fn apply_mutation(&self, mutation: ParserMutation) -> Result<(), DomError> {
        match mutation {
            ParserMutation::Initialize {
                node,
                parent,
                attrs,
                template,
                custom,
                template_assignment,
            } => {
                if custom && let Some(parent) = parent {
                    let (owner, actual) = self.resolve(node);
                    let (parent_owner, parent) = self.resolve(parent);
                    if parser_actions_installed(&self.dom) {
                        let result = parser_action(
                            &self.dom,
                            ParserAction::Construct {
                                target: ParserNode {
                                    owner,
                                    node: actual,
                                },
                                parent: ParserNode {
                                    owner: parent_owner,
                                    node: parent,
                                },
                            },
                        )
                        .ok_or(DomError::MigrationCapacity)?;
                        self.parser_roots.replace(node, &result.owner, result.node);
                    }
                    if let Some(ParserMutation::Initialize { custom, .. }) =
                        self.pending_mutations.borrow_mut().front_mut()
                    {
                        *custom = false;
                    }
                }
                let identity = crate::NodeRef {
                    arena: self.dom.borrow().arena().id(),
                    node: self.document,
                };
                let (owner, actual) = self.resolve(node);
                let mut dom = owner.borrow_mut();
                for attribute in attrs {
                    let qualified = attribute.name.prefix.as_ref().map_or_else(
                        || attribute.name.local.to_string(),
                        |prefix| format!("{prefix}:{}", attribute.name.local),
                    );
                    let namespace =
                        (!attribute.name.ns.is_empty()).then(|| attribute.name.ns.to_string());
                    dom.set_attribute_ns(
                        actual,
                        namespace.map(|value| self.payload(&value)),
                        self.payload(&qualified),
                        self.payload(&attribute.value),
                    )?;
                    if let Some(ParserMutation::Initialize { attrs, .. }) =
                        self.pending_mutations.borrow_mut().front_mut()
                    {
                        attrs.remove(0);
                    }
                }
                if template {
                    let content = dom
                        .arena()
                        .get(actual)?
                        .template_content
                        .ok_or(DomError::MigrationChanged)?;
                    let document = dom
                        .node_document(actual)?
                        .ok_or(DomError::MigrationChanged)?;
                    let inert = dom.arena_mut().template_contents_owner_document(document)?;
                    let identity = crate::NodeRef {
                        arena: dom.arena().id(),
                        node: content,
                    };
                    let assignment = match template_assignment {
                        Some((saved, owner, assignment)) if saved == identity && owner == inert => {
                            assignment
                        }
                        _ => {
                            let assignment = std::rc::Rc::new(RefCell::new(
                                crate::insertion::DocumentAssignment::new(
                                    identity.arena,
                                    content,
                                    inert,
                                ),
                            ));
                            if let Some(ParserMutation::Initialize {
                                template_assignment,
                                ..
                            }) = self.pending_mutations.borrow_mut().front_mut()
                            {
                                *template_assignment = Some((identity, inert, assignment.clone()));
                            }
                            assignment
                        }
                    };
                    assignment.borrow_mut().resume(dom.arena_mut())?;
                }
                dom.note_custom_registry_document(actual)?;
                if dom.is_element(actual, "http://www.w3.org/1999/xhtml", "input")
                    && dom
                        .attribute_in_null_namespace(actual, "type")?
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("range"))
                {
                    let value = dom
                        .attribute_in_null_namespace(actual, "value")?
                        .unwrap_or("")
                        .to_owned();
                    dom.reset_control_value(actual, value)?;
                }
                dom.arena_mut().get_mut(actual)?.parser_open_document = Some(identity);
                drop(dom);
                self.pending_open_elements.borrow_mut().push(node);
            }
            ParserMutation::Insert {
                parent,
                node,
                before,
                receipt,
            } => {
                if let Some(receipt) = receipt {
                    let (owner, _) = self.resolve(node);
                    let mut dom = owner.borrow_mut();
                    receipt.borrow_mut().resume(&mut dom)?;
                    receipt.borrow_mut().abort(&mut dom);
                    drop(dom);
                    self.record_connected_resource(node);
                    return Ok(());
                }
                if self.filtered_nodes.borrow().contains(&node)
                    || self.unwrapped_nodes.borrow().contains_key(&node)
                {
                    return Ok(());
                }
                let (parent, before) = self
                    .stream_location(parent)
                    .map_or((parent, before), |(parent, anchor)| {
                        (parent, before.or(anchor))
                    });
                let mut parent = parent;
                while let Some(next) = self.unwrapped_nodes.borrow().get(&parent).copied() {
                    parent = next;
                }
                let (parent_owner, parent_node) = self.resolve(parent);
                let (child_owner, child) = self.resolve(node);
                if let Some((policy, _)) = self.fragment_policy.borrow().as_ref()
                    && !policy.allows_parser_character_node(
                        &child_owner.borrow().arena().get(child)?.kind,
                    )
                {
                    return Ok(());
                }
                if let Some((policy, _)) = self.fragment_extra_policy.borrow().as_ref()
                    && !policy.allows_parser_character_node(
                        &child_owner.borrow().arena().get(child)?.kind,
                    )
                {
                    return Ok(());
                }
                let parent_document = parent_owner.borrow().node_document(parent_node)?;
                let child_document = child_owner.borrow().node_document(child)?;
                if !std::rc::Rc::ptr_eq(&parent_owner, &child_owner)
                    || parent_document != child_document
                {
                    if parser_actions_installed(&self.dom) {
                        let result = parser_action(
                            &self.dom,
                            ParserAction::Adopt {
                                target: ParserNode {
                                    owner: child_owner,
                                    node: child,
                                },
                                parent: ParserNode {
                                    owner: parent_owner.clone(),
                                    node: parent_node,
                                },
                            },
                        )
                        .ok_or(DomError::MigrationCapacity)?;
                        self.parser_roots.replace(node, &result.owner, result.node);
                    } else {
                        if !std::rc::Rc::ptr_eq(&parent_owner, &child_owner) {
                            let mut migration = DomRuntime::prepare_subtree_migration(
                                &child_owner,
                                &parent_owner,
                                child,
                                parent_document.unwrap_or(parent_node),
                            )?;
                            let moved = DomRuntime::commit_subtree_migration(
                                &child_owner,
                                &parent_owner,
                                &mut migration,
                            )?;
                            self.parser_roots.replace(node, &parent_owner, moved.root);
                        } else {
                            let mut dom = child_owner.borrow_mut();
                            let document = dom
                                .arena()
                                .document_id(parent_document.unwrap_or(parent_node))?;
                            let child = dom.arena().node_ref(child)?;
                            dom.adopt_node(document, child)?;
                        }
                    }
                }
                let (owner, actual) = self.resolve(node);
                let (parent_owner, parent_node) = self.resolve(parent);
                if !std::rc::Rc::ptr_eq(&owner, &parent_owner) {
                    return Err(DomError::MigrationCapacity);
                }
                let actual_before = before
                    .map(|key| self.resolve(key))
                    .map(|(before_owner, before)| {
                        if std::rc::Rc::ptr_eq(&owner, &before_owner) {
                            Ok(before)
                        } else {
                            Err(DomError::StaleNode(before))
                        }
                    })
                    .transpose()?;
                let receipt = std::rc::Rc::new(RefCell::new(crate::insertion::TreeInsertion::new(
                    Some(&owner),
                    &mut owner.borrow_mut(),
                    parent_node,
                    actual,
                    actual_before,
                    false,
                )?));
                // Install the native operation before its first structural
                // mutation. Foster/text entries become this exact receipt too.
                *self
                    .pending_mutations
                    .borrow_mut()
                    .front_mut()
                    .expect("retained parser insertion") = ParserMutation::Insert {
                    parent,
                    node,
                    before,
                    receipt: Some(receipt.clone()),
                };
                {
                    let mut dom = owner.borrow_mut();
                    receipt.borrow_mut().resume(&mut dom)?;
                    receipt.borrow_mut().abort(&mut dom);
                }
                self.record_connected_resource(node);
            }
            ParserMutation::Foster {
                element,
                previous,
                node,
            } => {
                let parent = self.parent_key(element);
                let insertion = ParserMutation::Insert {
                    parent: parent.unwrap_or(previous),
                    node,
                    before: parent.map(|_| element),
                    receipt: None,
                };
                *self
                    .pending_mutations
                    .borrow_mut()
                    .front_mut()
                    .expect("retained foster insertion") = insertion.clone();
                self.apply_mutation(insertion)?;
            }
            ParserMutation::Reparent { node, parent } => {
                // Resolve children only after preceding mutations commit. A
                // tokenizer token can enqueue this behind a failed insertion.
                let (owner, actual) = self.resolve(node);
                let children = owner.borrow().children(actual)?.to_vec();
                let insertions = children
                    .into_iter()
                    .map(|child| ParserMutation::Insert {
                        parent,
                        node: self.key(&owner, child),
                        before: None,
                        receipt: None,
                    })
                    .collect::<Vec<_>>();
                // Keep this no-op marker at the front for drain_mutations to
                // acknowledge; each child owns its own resumable insertion.
                let mut pending = self.pending_mutations.borrow_mut();
                for insertion in insertions.into_iter().rev() {
                    pending.insert(1, insertion);
                }
            }
            ParserMutation::BeginXml(content_type) => {
                let children = self.dom.borrow().children(self.document)?.to_vec();
                let removals = children
                    .into_iter()
                    .map(|node| ParserMutation::Detach {
                        node: self.key(&self.dom, node),
                        receipt: None,
                    })
                    .collect::<Vec<_>>();
                {
                    let mut dom = self.dom.borrow_mut();
                    let metadata = dom
                        .arena_mut()
                        .get_mut(self.document)?
                        .document_metadata
                        .get_or_insert_with(|| Box::new(crate::DocumentMetadata::default()));
                    metadata.content_type = content_type;
                    metadata.is_html = false;
                    metadata.quirks = false;
                }
                // Snapshot once, before the first removal. Each existing child
                // then owns the same resumable removal used by HTML parsing.
                let mut pending = self.pending_mutations.borrow_mut();
                for removal in removals.into_iter().rev() {
                    pending.insert(1, removal);
                }
            }
            ParserMutation::XmlPi(node) => self.publish_xml_pi_stylesheet(node),
            ParserMutation::StartStream {
                node,
                content,
                scope,
            } => {
                let (owner, actual) = self.resolve(node);
                let (scope_owner, scope) = self.resolve(scope);
                if !std::rc::Rc::ptr_eq(&owner, &scope_owner) {
                    return Err(DomError::MigrationChanged);
                }
                let patch = partial_updates::StreamingPatch::new(
                    &owner,
                    &mut owner.borrow_mut(),
                    scope,
                    actual,
                )?;
                if let Some(patch) = patch {
                    let parent = self.key(&owner, patch.parent);
                    let before = patch.before.map(|node| self.key(&owner, node));
                    let cleanup = patch
                        .cleanup_nodes()
                        .iter()
                        .map(|node| self.key(&owner, *node))
                        .collect();
                    self.stream_cleanup.borrow_mut().insert(node, cleanup);
                    let receipt = std::rc::Rc::new(RefCell::new(patch));
                    self.stream_locations
                        .borrow_mut()
                        .insert(content, (parent, before));
                    self.stream_patches
                        .borrow_mut()
                        .insert(node, receipt.clone());
                    let mutation = ParserMutation::StreamPatch { node, receipt };
                    *self
                        .pending_mutations
                        .borrow_mut()
                        .front_mut()
                        .expect("retained template activation") = mutation.clone();
                    return self.apply_mutation(mutation);
                }
            }
            ParserMutation::StreamPatch { node, receipt } => {
                let (owner, _) = self.resolve(node);
                receipt.borrow_mut().resume(&mut owner.borrow_mut())?;
            }
            ParserMutation::Pop(node) => {
                let streaming = self.stream_patches.borrow().get(&node).cloned();
                if let Some(receipt) = streaming {
                    // Marker custody follows each tracked handle even if author
                    // code moved the marker or target into another arena.
                    let cleanup = self
                        .stream_cleanup
                        .borrow_mut()
                        .remove(&node)
                        .unwrap_or_default();
                    self.stream_patches.borrow_mut().remove(&node);
                    let (owner, _) = self.resolve(node);
                    receipt.borrow_mut().abort(&mut owner.borrow_mut());
                    let mut queue = self.pending_mutations.borrow_mut();
                    for key in cleanup.into_iter().rev() {
                        queue.insert(
                            1,
                            ParserMutation::Detach {
                                node: key,
                                receipt: None,
                            },
                        );
                    }
                    return Ok(());
                }
                let (owner, actual) = self.resolve(node);
                if self.patches_enabled.get()
                    && owner
                        .borrow()
                        .is_element(actual, "http://www.w3.org/1999/xhtml", "template")
                {
                    if owner
                        .borrow()
                        .attribute_in_null_namespace(actual, "src")?
                        .is_some()
                    {
                        if owner.borrow().is_connected(actual)?
                            && std::rc::Rc::ptr_eq(&owner, &self.dom)
                        {
                            self.events
                                .borrow_mut()
                                .push(ParserEvent::ExternalTemplateDiscovered { node: actual });
                        }
                        return Ok(());
                    }
                    let mut scope = self.patch_scope.get().unwrap_or(self.document);
                    // A template inside inert template content cannot patch
                    // outside its enclosing template before sanitization.
                    let mut ancestor = actual;
                    while ancestor != scope {
                        if matches!(
                            owner.borrow().arena().get(ancestor)?.kind,
                            NodeKind::ShadowRoot { .. }
                        ) {
                            scope = ancestor;
                            break;
                        }
                        let Some(parent) = owner.borrow().parent(ancestor)? else {
                            break;
                        };
                        ancestor = parent;
                    }
                    if ancestor == scope {
                        let patch = partial_updates::PatchCursor::new(
                            Some(&owner),
                            &mut owner.borrow_mut(),
                            scope,
                            actual,
                        )?;
                        if let Some(patch) = patch {
                            let mutation = ParserMutation::Patch {
                                node,
                                receipt: std::rc::Rc::new(RefCell::new(patch)),
                            };
                            *self
                                .pending_mutations
                                .borrow_mut()
                                .front_mut()
                                .expect("retained template patch") = mutation.clone();
                            return self.apply_mutation(mutation);
                        }
                    }
                }
                self.finish_element(node);
            }
            ParserMutation::Patch { node, receipt } => {
                let (owner, _) = self.resolve(node);
                let mut dom = owner.borrow_mut();
                receipt.borrow_mut().resume(&mut dom)?;
                let roots = receipt.borrow().published().to_vec();
                // Nested templates become eligible only after the enclosing
                // template's sanitizer and structural publication complete.
                // Nested patch receipts are retained by the outer cursor.
                receipt.borrow_mut().abort(&mut dom);
                drop(dom);
                for root in roots {
                    let descendants = owner.borrow().arena().descendants_inclusive(root)?;
                    for actual in descendants {
                        let key = self.key(&owner, actual);
                        self.record_connected_resource(key);
                        self.finish_element(key);
                    }
                }
            }
            ParserMutation::Detach { node, receipt } => {
                let (owner, actual) = self.resolve(node);
                let receipt = match receipt {
                    Some(receipt) => receipt,
                    None => {
                        if owner.borrow().parent(actual)?.is_none() {
                            return Ok(());
                        }
                        let receipt =
                            std::rc::Rc::new(RefCell::new(crate::insertion::TreeRemoval::new(
                                Some(&owner),
                                &mut owner.borrow_mut(),
                                actual,
                            )?));
                        *self
                            .pending_mutations
                            .borrow_mut()
                            .front_mut()
                            .expect("retained parser removal") = ParserMutation::Detach {
                            node,
                            receipt: Some(receipt.clone()),
                        };
                        receipt
                    }
                };
                let mut dom = owner.borrow_mut();
                receipt.borrow_mut().resume(&mut dom)?;
                receipt.borrow_mut().abort(&mut dom);
            }
            ParserMutation::Text {
                parent,
                data,
                before,
            } => {
                let data = self.payload(&data);
                let (parent, before) = self
                    .stream_location(parent)
                    .map_or((parent, before), |(parent, anchor)| {
                        (parent, before.or(anchor))
                    });
                let mut parent = parent;
                while let Some(next) = self.unwrapped_nodes.borrow().get(&parent).copied() {
                    parent = next;
                }
                let (owner, parent_node) = self.resolve(parent);
                let before_node = before
                    .map(|key| self.resolve(key))
                    .map(|(before_owner, node)| {
                        if std::rc::Rc::ptr_eq(&owner, &before_owner) {
                            Ok(node)
                        } else {
                            Err(DomError::StaleNode(node))
                        }
                    })
                    .transpose()?;
                let mut dom = owner.borrow_mut();
                let children = dom.children(parent_node)?;
                let previous = match before_node {
                    None => children.last().copied(),
                    Some(before) => children
                        .iter()
                        .position(|node| *node == before)
                        .and_then(|position| position.checked_sub(1))
                        .map(|position| children[position]),
                };
                if let Some(last) = previous
                    && let NodeKind::Text { data: previous } = &dom.arena().get(last)?.kind
                {
                    let offset = crate::dom_string::len(previous) as u32;
                    dom.replace_character_data(last, offset, 0, data)?;
                    return Ok(());
                }
                let node = dom.create_text(data);
                let document = dom.node_document(parent_node)?.unwrap_or(parent_node);
                dom.set_node_document(node, document)?;
                drop(dom);
                let key = self.key(&owner, node);
                let insertion = ParserMutation::Insert {
                    parent,
                    node: key,
                    before,
                    receipt: None,
                };
                *self
                    .pending_mutations
                    .borrow_mut()
                    .front_mut()
                    .expect("retained parser text") = insertion.clone();
                self.apply_mutation(insertion)?;
            }
            ParserMutation::Attributes { node, attrs } => {
                let (owner, actual) = self.resolve(node);
                let mut dom = owner.borrow_mut();
                for attribute in attrs {
                    let raw_local = attribute.name.local.to_string();
                    let namespace = (!attribute.name.ns.is_empty())
                        .then(|| self.payload(attribute.name.ns.as_ref()));
                    let name = self.payload(&attribute.name.prefix.as_ref().map_or_else(
                        || raw_local.clone(),
                        |prefix| format!("{prefix}:{raw_local}"),
                    ));
                    let local = self.payload(&raw_local);
                    if dom
                        .attribute_ns(actual, namespace.as_deref(), &local)?
                        .is_none()
                    {
                        let window_handler = namespace.is_none()
                            && matches!(
                                local.as_str(),
                                "onload" | "onerror" | "onpageshow" | "onpagehide"
                            )
                            && (dom.is_element(actual, "http://www.w3.org/1999/xhtml", "body")
                                || dom.is_element(
                                    actual,
                                    "http://www.w3.org/1999/xhtml",
                                    "frameset",
                                ));
                        let nonce = namespace.is_none() && local == "nonce";
                        dom.set_attribute_ns(
                            actual,
                            namespace,
                            name,
                            self.payload(&attribute.value),
                        )?;
                        if nonce {
                            dom.hide_nonce_on_insertion(actual)?;
                        }
                        if window_handler && std::rc::Rc::ptr_eq(&owner, &self.dom) {
                            self.events.borrow_mut().push(
                                ParserEvent::EventHandlerAttributeChanged {
                                    node: actual,
                                    name: local,
                                },
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn finish_element(&self, key: NodeId) {
        let (owner, actual) = self.resolve(key);
        let node = &actual;
        let parser_identity = crate::NodeRef {
            arena: self.dom.borrow().arena().id(),
            node: self.document,
        };
        let mut dom = owner.borrow_mut();
        if let Ok(record) = dom.arena_mut().get_mut(*node) {
            record.stylesheet_parser_ready = true;
            if record.parser_open_document == Some(parser_identity) {
                record.parser_open_document = None;
            }
        }
        if !std::rc::Rc::ptr_eq(&owner, &self.dom)
            || dom.node_document(*node).ok().flatten() != Some(self.document)
        {
            return;
        }
        if self.fragment_run_scripts.get() == Some(false)
            && dom.is_element(*node, "http://www.w3.org/1999/xhtml", "script")
        {
            let _ = dom.mark_script_started(*node);
            return;
        }
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
            if !dom.arena().is_css_stylesheet_owner(*node).unwrap_or(false) {
                return;
            }
            self.events
                .borrow_mut()
                .push(ParserEvent::InlineStyleReady { node: *node });
            return;
        }
        if name.local.as_ref() != "script" {
            return;
        }
        let script_type = script_block_type(
            node_record.attributes.get_null("type").map(String::as_str),
            node_record
                .attributes
                .get_null("language")
                .map(String::as_str),
        );
        let scheduling = if script_type == "module" {
            ScriptScheduling::Module
        } else if script_type == "importmap" {
            ScriptScheduling::ImportMap
        } else if script_type == "speculationrules" {
            ScriptScheduling::SpeculationRules
        } else if !is_javascript_mime_type(&script_type) {
            ScriptScheduling::Data
        } else if dom.script_external_source(*node).ok().flatten().is_none() {
            // Async/defer do not change inline classic script scheduling.
            ScriptScheduling::Blocking
        } else if node_record.attributes.contains_null("async") {
            ScriptScheduling::Async
        } else if node_record.attributes.contains_null("defer") {
            ScriptScheduling::Defer
        } else {
            ScriptScheduling::Blocking
        };
        if let Ok(source) = dom.arena().child_text_dom_string(*node, 8 * 1024 * 1024)
            && let Ok(record) = dom.arena_mut().get_mut(*node)
        {
            record.script_trusted_text = source;
        }
        self.events.borrow_mut().push(ParserEvent::ScriptReady {
            node: *node,
            scheduling,
        });
    }

    fn create_element_contextual(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
        context: markup5ever::interface::tree_builder::ElementCreationContext<&NodeId>,
    ) -> NodeId {
        self.flush_text();
        let parent = match context {
            markup5ever::interface::tree_builder::ElementCreationContext::Unspecified => None,
            markup5ever::interface::tree_builder::ElementCreationContext::Parent(parent) => {
                Some(*parent)
            }
            markup5ever::interface::tree_builder::ElementCreationContext::BeforeSibling(
                sibling,
            ) => self.parent_key(*sibling),
            markup5ever::interface::tree_builder::ElementCreationContext::FosterParent {
                element,
                previous,
            } => self.parent_key(*element).or(Some(*previous)),
        };
        let parent = parent.map(|parent| {
            self.stream_locations
                .borrow()
                .get(&parent)
                .map_or(parent, |location| location.0)
        });
        let (owner, parent_node) = parent
            .map(|parent| self.resolve(parent))
            .unwrap_or_else(|| (self.dom.clone(), self.document));
        let original_attrs = attrs.clone();
        let decoded_name = self.payload_name(name.clone());
        let mut decoded_attrs = attrs
            .into_iter()
            .map(|attribute| Attribute {
                name: self.payload_name(attribute.name),
                value: self.payload(&attribute.value).into(),
            })
            .collect::<Vec<_>>();
        let mut disposition = crate::sanitizer::ParserElementDisposition::Keep;
        if let Some((policy, safe)) = self.fragment_policy.borrow().as_ref() {
            disposition = policy.filter_parser_element(&decoded_name, &mut decoded_attrs, *safe);
        }
        if let Some((policy, safe)) = self.fragment_extra_policy.borrow().as_ref() {
            let next = policy.filter_parser_element(&decoded_name, &mut decoded_attrs, *safe);
            if disposition != crate::sanitizer::ParserElementDisposition::Drop
                && next != crate::sanitizer::ParserElementDisposition::Keep
            {
                disposition = next;
            }
        }
        // Enclosing template-level sanitization applies before constructors,
        // nested template patching, or connection to any live destination.
        let mut ancestor = Some(parent_node);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(node) = ancestor {
            if !seen.insert(node) {
                break;
            }
            let dom = owner.borrow();
            let Ok(record) = dom.arena().get(node) else {
                break;
            };
            let host = record.template_host;
            if let Some(host) = host
                && dom
                    .attribute_in_null_namespace(host, "sanitize")
                    .ok()
                    .flatten()
                    .is_some_and(|value| value != "unsafe")
            {
                let next = crate::sanitizer::Config::default_preset().filter_parser_element(
                    &decoded_name,
                    &mut decoded_attrs,
                    true,
                );
                if disposition != crate::sanitizer::ParserElementDisposition::Drop
                    && next != crate::sanitizer::ParserElementDisposition::Keep
                {
                    disposition = next;
                }
            }
            ancestor = record.parent.or(host);
        }
        let attrs = if self.dom_string_transport.get() {
            decoded_attrs
                .into_iter()
                .map(|attribute| Attribute {
                    name: dom_input::name(attribute.name, dom_input::encode),
                    value: dom_input::encode(&attribute.value).into(),
                })
                .collect::<Vec<_>>()
        } else {
            original_attrs
        };
        let parser_identity = crate::NodeRef {
            arena: self.dom.borrow().arena().id(),
            node: self.document,
        };
        let mut dom = owner.borrow_mut();
        let element = dom.create(NodeKind::Element {
            name: self.payload_name(name),
        });
        let document = dom
            .node_document(parent_node)
            .expect("parser parent owner")
            .unwrap_or(parent_node);
        // Fresh allocation initialization has no adoption side effects. Keep
        // fallible template ownership publication in the retained mutation.
        dom.arena_mut()
            .get_mut(element)
            .expect("fresh parser element")
            .owner_document = Some(document);
        let registry = dom
            .arena()
            .get(parent_node)
            .expect("parser parent")
            .custom_element_registry;
        {
            let record = dom
                .arena_mut()
                .get_mut(element)
                .expect("new parser element");
            record.custom_element_registry = registry;
            record.parser_duplicate_attributes = flags.had_duplicate_attributes;
            if matches!(&record.kind,NodeKind::Element{name}if matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg")&&name.local.as_ref()=="script")
            {
                record.script_force_async = false;
                record.script_parser_document = Some(parser_identity);
            }
            if attrs.iter().any(|attribute| {
                attribute.name.ns.is_empty()
                    && attribute.name.local.as_ref() == "customelementregistry"
            }) {
                record.custom_element_registry = Some(None);
            }
            record.custom_element_is = attrs
                .iter()
                .find(|attribute| {
                    attribute.name.ns.is_empty() && attribute.name.local.as_ref() == "is"
                })
                .map(|attribute| self.payload(&attribute.value));
        }
        if flags.template {
            // html5ever may request this handle before a failed Initialize can
            // resume. Allocate and link it once, then retain its ownership tail.
            let content = dom.create_fragment();
            dom.arena_mut()
                .get_mut(element)
                .expect("fresh parser template")
                .template_content = Some(content);
            dom.arena_mut()
                .get_mut(content)
                .expect("fresh template fragment")
                .template_host = Some(element);
            dom.internal_targets.insert(content);
        }
        if dom.is_element(element, "http://www.w3.org/1999/xhtml", "style")
            || dom.is_element(element, "http://www.w3.org/2000/svg", "style")
        {
            dom.arena_mut()
                .get_mut(element)
                .expect("parser style")
                .stylesheet_parser_ready = false;
        }
        let custom = {
            let record = dom.arena().get(element).expect("parser element");
            matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&(crate::valid_custom_element_name(name.local.as_ref())||record.custom_element_is.is_some()))
        };
        drop(dom);
        let key = self.key(&owner, element);
        let inherited = parent.is_some_and(|parent| self.filtered_nodes.borrow().contains(&parent));
        let custom = custom
            && !self.xslt_source.get()
            && !inherited
            && disposition == crate::sanitizer::ParserElementDisposition::Keep;
        if inherited || disposition == crate::sanitizer::ParserElementDisposition::Drop {
            self.filtered_nodes.borrow_mut().insert(key);
        } else if disposition == crate::sanitizer::ParserElementDisposition::Unwrap
            && let Some(parent) = parent
        {
            self.unwrapped_nodes.borrow_mut().insert(key, parent);
        }
        self.queue_mutation(ParserMutation::Initialize {
            node: key,
            parent,
            attrs,
            template: flags.template,
            custom,
            template_assignment: None,
        });
        key
    }

    pub(crate) fn new_html_document() -> Self {
        Self::new()
    }

    fn new() -> Self {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        Self::with_runtime(dom, document)
    }

    fn with_runtime(dom: SharedDomRuntime, document: NodeId) -> Self {
        {
            let mut runtime = dom.borrow_mut();
            let metadata = runtime
                .arena_mut()
                .get_mut(document)
                .expect("parser document")
                .document_metadata
                .get_or_insert_with(|| Box::new(crate::DocumentMetadata::html()));
            // Attaching a parser changes parsing mode, not the identity or
            // loading state of the Document supplied by the embedding host.
            metadata.content_type = "text/html".into();
            metadata.is_html = true;
            metadata.quirks = false;
        }
        Self {
            parser_roots: ParserHandles::new(dom.clone(), document),
            pending_mutations: RefCell::new(std::collections::VecDeque::new()),
            mutation_failed: Cell::new(false),
            dom,
            document,
            errors: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            events: RefCell::new(Vec::new()),
            pending_text: RefCell::new(None),
            recorded_resources: RefCell::new(std::collections::BTreeSet::new()),
            open_elements: RefCell::new(Vec::new()),
            pending_open_elements: RefCell::new(Vec::new()),
            allow_declarative: Cell::new(true),
            encoding_events: Cell::new(false),
            dom_string_transport: Cell::new(true),
            xslt_source: Cell::new(false),
            patches_enabled: Cell::new(true),
            patch_scope: Cell::new(None),
            fragment_root_key: Cell::new(None),
            external_template: Cell::new(None),
            external_cleanup_started: Cell::new(false),
            external_buffered: RefCell::new(None),
            fragment_run_scripts: Cell::new(None),
            fragment_policy: RefCell::new(None),
            fragment_extra_policy: RefCell::new(None),
            stream_patches: RefCell::new(std::collections::BTreeMap::new()),
            stream_locations: RefCell::new(std::collections::BTreeMap::new()),
            stream_cleanup: RefCell::new(std::collections::BTreeMap::new()),
            stream_requested: RefCell::new(std::collections::BTreeSet::new()),
            filtered_nodes: RefCell::new(std::collections::BTreeSet::new()),
            unwrapped_nodes: RefCell::new(std::collections::BTreeMap::new()),
        }
    }

    pub(crate) fn with_xml_document(dom: SharedDomRuntime, document: NodeId) -> Self {
        let sink = Self::with_runtime(dom, document);
        sink.dom_string_transport.set(false);
        sink
    }
    pub(crate) fn suppress_xslt_source_execution(&self) {
        self.xslt_source.set(true);
    }
    pub(crate) fn begin_xml_input(&self, content_type: &str) -> bool {
        self.queue_mutation(ParserMutation::BeginXml(content_type.into()));
        !self.mutation_failed.get()
    }
    pub(crate) fn construction_interrupted(&self) -> bool {
        self.mutation_failed.get() || element_construction_interrupted(&self.dom)
    }
    pub(crate) fn prepare_xml_finish(&self) -> bool {
        self.flush_text();
        !self.mutation_failed.get()
    }
    pub(crate) fn take_xml_events(&self) -> Vec<ParserEvent> {
        self.flush_text();
        std::mem::take(&mut *self.events.borrow_mut())
    }
    pub(crate) fn has_xml_events(&self) -> bool {
        !self.events.borrow().is_empty()
    }
    pub(crate) fn take_xml_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.errors.borrow_mut())
    }
    pub(crate) fn record_xml_pi_stylesheet(&self, node: NodeId) {
        self.queue_mutation(ParserMutation::XmlPi(node));
    }
    fn publish_xml_pi_stylesheet(&self, node: NodeId) {
        let (owner, node) = self.resolve(node);
        if !std::rc::Rc::ptr_eq(&owner, &self.dom)
            || owner.borrow().node_document(node).ok().flatten() != Some(self.document)
        {
            return;
        }
        if let Some(sheet) = owner.borrow().arena().xml_stylesheet(node).ok().flatten() {
            self.events
                .borrow_mut()
                .push(ParserEvent::StylesheetDiscovered {
                    node,
                    href: sheet.href,
                });
        }
    }

    pub(crate) fn for_xml(content_type: &str) -> Self {
        let sink = Self::new();
        sink.dom_string_transport.set(false);
        sink.dom
            .borrow_mut()
            .arena_mut()
            .get_mut(sink.document)
            .unwrap()
            .document_metadata = Some(Box::new(crate::DocumentMetadata {
            content_type: content_type.into(),
            is_html: false,
            quirks: false,
            ..crate::DocumentMetadata::default()
        }));
        sink
    }

    pub(crate) fn xml_character_node(&self, data: String, target: Option<String>) -> NodeId {
        self.flush_text();
        let mut dom = self.dom.borrow_mut();
        let node = match target {
            Some(target) => dom.create_processing_instruction(target, data),
            None => dom.create_cdata_section(data),
        };
        drop(dom);
        self.key(&self.dom, node)
    }

    fn record_connected_resource(&self, key: NodeId) {
        let (owner, element) = self.resolve(key);
        if !std::rc::Rc::ptr_eq(&owner, &self.dom) {
            return;
        }
        let dom = owner.borrow();
        if dom.node_document(element).ok().flatten() != Some(self.document) {
            return;
        }
        if !dom.is_connected(element).unwrap_or(false)
            || !dom.arena().get(element).is_ok_and(|node| matches!(&node.kind,
                NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && matches!(name.local.as_ref(), "link" | "iframe" | "frame" | "object" | "embed" | "meta")))
            || !self.recorded_resources.borrow_mut().insert(key) {
            return;
        }
        if let Ok(node) = dom.arena().get(element)
            && let NodeKind::Element { name } = &node.kind
            && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        {
            if name.local.as_ref() == "link"
                && node.attributes.get_null("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace().any(|token| {
                        token.eq_ignore_ascii_case("modulepreload")
                            || token.eq_ignore_ascii_case("preload")
                    })
                })
            {
                self.events
                    .borrow_mut()
                    .push(ParserEvent::ModulePreloadDiscovered { node: element });
            }
            match name.local.as_ref() {
                "link"
                    if dom
                        .arena()
                        .is_css_stylesheet_owner(element)
                        .unwrap_or(false) =>
                {
                    if let Some(href) = node.attributes.get_null("href") {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::StylesheetDiscovered {
                                node: element,
                                href: href.clone(),
                            });
                    }
                }
                "iframe" | "frame" | "object" | "embed" => {
                    let kind = dom
                        .embedded_content_kind(element)
                        .expect("native embedded element");
                    if kind.supports_srcdoc() && node.attributes.contains_null("srcdoc")
                        || kind.creates_initial_document()
                            && node
                                .attributes
                                .get_null(kind.source_attribute())
                                .is_none_or(|source| source.trim().is_empty())
                    {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::FrameLocalDocument { node: element });
                    } else if let Some(source) = node
                        .attributes
                        .get_null(kind.source_attribute())
                        .filter(|source| kind.loads_source(Some(source.as_str())))
                    {
                        self.events.borrow_mut().push(ParserEvent::FrameDiscovered {
                            node: element,
                            source: source.clone(),
                        });
                    }
                }
                "meta" => {
                    let content = node.attributes.get_null("content").cloned();
                    if node
                        .attributes
                        .get_null("http-equiv")
                        .is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy"))
                        && dom.is_in_document_head(element).unwrap_or(false)
                        && let Some(content) = content.clone()
                    {
                        self.events
                            .borrow_mut()
                            .push(ParserEvent::MetaSecurity { content });
                    }
                    if node
                        .attributes
                        .get_null("name")
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
    }

    fn stream_location(&self, parent: NodeId) -> Option<(NodeId, Option<NodeId>)> {
        let (target, anchor) = self.stream_locations.borrow().get(&parent).copied()?;
        let (owner, node) = self.resolve(target);
        let anchor = anchor.filter(|key| {
            let (anchor_owner, anchor) = self.resolve(*key);
            std::rc::Rc::ptr_eq(&owner, &anchor_owner)
                && owner.borrow().parent(anchor).ok().flatten() == Some(node)
        });
        Some((target, anchor))
    }
    fn append_node_or_text(&self, parent: NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.flush_text();
                self.queue_mutation(ParserMutation::Insert {
                    parent,
                    node,
                    before: None,
                    receipt: None,
                });
            }
            NodeOrText::AppendText(text) => self.append_text(parent, text),
        }
    }
    fn append_text(&self, parent: NodeId, text: StrTendril) {
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
        let text = self.pending_text.borrow_mut().take();
        if let Some((parent, data)) = text {
            self.queue_mutation(ParserMutation::Text {
                parent,
                data,
                before: None,
            });
        }
    }
}

#[derive(Debug)]
pub(crate) struct OrynElemName {
    name: QualName,
}
impl ElemName for OrynElemName {
    fn ns(&self) -> &Namespace {
        &self.name.ns
    }
    fn local_name(&self) -> &LocalName {
        &self.name.local
    }
}

impl TreeSink for OrynTreeSink {
    fn character_reference_transport(&self, value: char) -> Option<StrTendril> {
        self.dom_string_transport
            .get()
            .then(|| dom_input::escape_scalar(value))
            .flatten()
            .map(|value| StrTendril::from(value.as_str()))
    }

    type Handle = NodeId;
    type Output = ParsedDocument;
    type ElemName<'a> = OrynElemName;

    fn finish(self) -> Self::Output {
        self.flush_text();
        // Successful declarative templates are parser handles, never authored
        // light-tree nodes. Drop those handles before snapshotting the arena.
        for key in self.parser_roots.keys() {
            let (owner, node) = self.resolve(key);
            let declarative = {
                let dom = owner.borrow();
                dom.arena().get(node).is_ok_and(|record| {
                    record.parent.is_none()
                        && record.template_content.is_some_and(|content| {
                            dom.arena().get(content).is_ok_and(|record| {
                                matches!(record.kind, NodeKind::ShadowRoot { .. })
                            })
                        })
                })
            };
            if declarative {
                owner
                    .borrow_mut()
                    .arena_mut()
                    .get_mut(node)
                    .expect("declarative template")
                    .template_content = None;
                self.parser_roots.release(key);
            }
        }
        let (dom, _) = self.dom.borrow().snapshot();
        ParsedDocument {
            dom,
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
        if self.fragment_root_key.get() == Some(*target) {
            return OrynElemName {
                name: QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "html".into()),
            };
        }
        let (owner, node) = self.resolve(*target);
        let dom = owner.borrow();
        let NodeKind::Element { name } = &dom.arena().get(node).expect("live parser element").kind
        else {
            panic!("elem_name for non-element")
        };
        OrynElemName {
            name: if self.dom_string_transport.get() {
                dom_input::name(name.clone(), dom_input::encode)
            } else {
                name.clone()
            },
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        self.create_element_contextual(
            name,
            attrs,
            flags,
            markup5ever::interface::tree_builder::ElementCreationContext::Unspecified,
        )
    }
    fn create_element_with_context(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
        context: markup5ever::interface::tree_builder::ElementCreationContext<&NodeId>,
    ) -> NodeId {
        self.create_element_contextual(name, attrs, flags, context)
    }

    fn open_elements(&self, nodes: &[NodeId]) {
        let identity = crate::NodeRef {
            arena: self.dom.borrow().arena().id(),
            node: self.document,
        };
        for key in self
            .open_elements
            .borrow_mut()
            .drain(..)
            .chain(self.pending_open_elements.borrow_mut().drain(..))
        {
            let (owner, node) = self.resolve(key);
            if let Ok(record) = owner.borrow_mut().arena_mut().get_mut(node)
                && record.parser_open_document == Some(identity)
            {
                record.parser_open_document = None;
            }
        }
        for key in nodes {
            let (owner, node) = self.resolve(*key);
            if let Ok(record) = owner.borrow_mut().arena_mut().get_mut(node) {
                record.parser_open_document = Some(identity);
            }
        }
        self.open_elements.borrow_mut().extend_from_slice(nodes);
        self.dom.borrow_mut().refresh_render_expect_links();
    }

    fn pop(&self, node: &Self::Handle) {
        self.flush_text();
        self.queue_mutation(ParserMutation::Pop(*node));
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.flush_text();
        let node = self.dom.borrow_mut().create_comment(self.payload(&text));
        self.key(&self.dom, node)
    }
    fn processing_instructions_enabled(&self) -> bool {
        true
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.flush_text();
        let node = self
            .dom
            .borrow_mut()
            .create_processing_instruction(self.payload(&target), self.payload(&data));
        self.key(&self.dom, node)
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
        self.flush_text();
        match child {
            NodeOrText::AppendNode(node) => self.queue_mutation(ParserMutation::Foster {
                element: *element,
                previous: *prev_element,
                node,
            }),
            NodeOrText::AppendText(text) => {
                let parent = self.parent_key(*element);
                self.queue_mutation(ParserMutation::Text {
                    parent: parent.unwrap_or(*prev_element),
                    data: text.to_string(),
                    before: parent.map(|_| *element),
                });
            }
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.flush_text();
        let doctype = self.dom.borrow_mut().create(NodeKind::DocumentType {
            name: self.payload(&name),
            public_id: self.payload(&public_id),
            system_id: self.payload(&system_id),
        });
        let key = self.key(&self.dom, doctype);
        self.queue_mutation(ParserMutation::Insert {
            parent: self.document,
            node: key,
            before: None,
            receipt: None,
        });
    }

    fn allow_declarative_shadow_roots(&self, _parent: &Self::Handle) -> bool {
        self.allow_declarative.get()
    }
    fn attach_declarative_shadow(
        &self,
        host: &Self::Handle,
        template: &Self::Handle,
        _attrs: &[Attribute],
    ) -> bool {
        self.flush_text();
        let (attribute_owner, attribute_node) = self.resolve(*template);
        let attributes = attribute_owner
            .borrow()
            .arena()
            .get(attribute_node)
            .map(|record| {
                record
                    .attributes
                    .records()
                    .filter(|attribute| attribute.namespace.is_none())
                    .map(|attribute| (attribute.local_name.clone(), attribute.value.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let attribute = |name: &str| {
            attributes
                .iter()
                .find(|(local, _)| local == name)
                .map(|(_, value)| value.clone())
        };
        if attribute("for").is_some() {
            return false;
        }
        let mode = match attribute("shadowrootmode").as_deref() {
            Some("open") => crate::ShadowMode::Open,
            Some("closed") => crate::ShadowMode::Closed,
            _ => return false,
        };
        let (owner, host) = self.resolve(*host);
        let (template_owner, template) = self.resolve(*template);
        if !std::rc::Rc::ptr_eq(&owner, &template_owner) {
            return false;
        }
        let mut dom = owner.borrow_mut();
        let Ok(record) = dom.arena().get(host) else {
            return false;
        };
        let allowed = matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&(matches!(name.local.as_ref(),"article"|"aside"|"blockquote"|"body"|"div"|"footer"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"header"|"main"|"nav"|"p"|"section"|"span")||crate::valid_custom_element_name(name.local.as_ref())));
        if record.custom_element_shadow_disabled.unwrap_or_else(||matches!(&record.kind,NodeKind::Element{name}if dom.parser_shadow_disabled(dom.node_document(host).ok().flatten().unwrap_or(host),name.local.as_ref()))){return false;}
        if !allowed
            || record.children.iter().any(|child| {
                dom.arena()
                    .get(*child)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
            })
        {
            return false;
        }
        let document_registry = dom
            .node_document(host)
            .ok()
            .flatten()
            .and_then(|document| dom.arena().get(document).ok())
            .and_then(|document| document.custom_element_registry);
        let root = dom.create(NodeKind::ShadowRoot {
            mode,
            clonable: attribute("shadowrootclonable").is_some(),
        });
        {
            let record = dom.arena_mut().get_mut(root).expect("new declarative root");
            record.custom_element_registry = document_registry;
            record.declarative_shadow_root = true;
            record.shadow_serializable = attribute("shadowrootserializable").is_some();
            record.shadow_adopted_stylesheet_specifiers = attribute("shadowrootadoptedstylesheets");
            if attribute("shadowrootcustomelementregistry").is_some() {
                record.custom_element_registry = Some(None);
                record.custom_element_registry_keep_null = true;
            }
            record.delegates_focus = attribute("shadowrootdelegatesfocus").is_some();
            record.shadow_reference_target = attribute("shadowrootreferencetarget");
        }
        dom.append_child(host, root)
            .expect("declarative host is live");
        // html5ever inserts all following template tokens through this handle.
        // The template itself is not inserted into the light tree.
        let previous = dom
            .arena()
            .get(template)
            .expect("parser template")
            .template_content;
        dom.arena_mut()
            .get_mut(template)
            .expect("parser template")
            .template_content = Some(root);
        if let Some(previous) = previous {
            dom.arena_mut()
                .get_mut(previous)
                .expect("template fragment")
                .template_host = None;
            dom.retain_native(previous).expect("template fragment");
            dom.release_native(previous)
                .expect("obsolete template fragment");
        }
        true
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        let (owner, node) = self.resolve(*target);
        let content = owner
            .borrow()
            .arena()
            .get(node)
            .expect("live template")
            .template_content
            .expect("template contents");
        let key = self.key(&owner, content);
        if self.patches_enabled.get()
            && !self.stream_requested.borrow().contains(target)
            && owner
                .borrow()
                .attribute_in_null_namespace(node, "src")
                .ok()
                .flatten()
                .is_none()
        {
            let mut scope = self.patch_scope.get().unwrap_or(self.document);
            let mut ancestor = node;
            if owner.borrow().parent(node).ok().flatten().is_none() {
                let admitted =
                    self.pending_mutations
                        .borrow()
                        .iter()
                        .find_map(|mutation| match mutation {
                            ParserMutation::Insert { parent, node, .. } if node == target => {
                                Some(*parent)
                            }
                            _ => None,
                        });
                if let Some(parent) = admitted {
                    let (parent_owner, parent) = self.resolve(parent);
                    if std::rc::Rc::ptr_eq(&owner, &parent_owner) {
                        ancestor = parent;
                    }
                }
            }
            loop {
                if ancestor == scope {
                    break;
                }
                let dom = owner.borrow();
                if matches!(
                    dom.arena().get(ancestor).map(|node| &node.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                ) {
                    scope = ancestor;
                    break;
                }
                let Some(parent) = dom.parent(ancestor).ok().flatten() else {
                    break;
                };
                ancestor = parent;
            }
            if ancestor == scope {
                self.stream_requested.borrow_mut().insert(*target);
                let scope = self.key(&owner, scope);
                self.queue_mutation(ParserMutation::StartStream {
                    node: *target,
                    content: key,
                    scope,
                });
            }
        }
        key
    }
    fn mark_script_already_started(&self, node: &Self::Handle) {
        if self.fragment_run_scripts.get() == Some(true) {
            return;
        }
        let (owner, node) = self.resolve(*node);
        owner
            .borrow_mut()
            .mark_script_started(node)
            .expect("live parser script");
    }
    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        let (a, x) = self.resolve(*x);
        let (b, y) = self.resolve(*y);
        std::rc::Rc::ptr_eq(&a, &b) && x == y
    }

    fn encoding_indicator(&self, label: StrTendril) {
        if self.encoding_events.get() {
            self.events
                .borrow_mut()
                .push(ParserEvent::EncodingIndicator {
                    label: label.to_string(),
                });
        }
    }
    fn encoding_committed(&self) {
        if self.encoding_events.get() {
            self.events
                .borrow_mut()
                .push(ParserEvent::EncodingCommitted);
        }
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
        let Some(parent) = self.parent_key(*sibling) else {
            return;
        };
        match new_node {
            NodeOrText::AppendNode(node) => self.queue_mutation(ParserMutation::Insert {
                parent,
                node,
                before: Some(*sibling),
                receipt: None,
            }),
            NodeOrText::AppendText(text) => self.queue_mutation(ParserMutation::Text {
                parent,
                data: text.to_string(),
                before: Some(*sibling),
            }),
        }
    }
    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        self.flush_text();
        self.queue_mutation(ParserMutation::Attributes {
            node: *target,
            attrs,
        });
    }
    fn remove_from_parent(&self, target: &Self::Handle) {
        self.flush_text();
        self.queue_mutation(ParserMutation::Detach {
            node: *target,
            receipt: None,
        });
    }
    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.flush_text();
        self.queue_mutation(ParserMutation::Reparent {
            node: *node,
            parent: *new_parent,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_and_dom_string_parsers_preserve_leading_feff() {
        for text in ["\u{feff}", "\u{feff}\u{feff}", "\u{feff}<b>text</b>"] {
            let expected = text.replace("<b>", "").replace("</b>", "");
            for parsed in [
                parse(text),
                parse_inert(text),
                parse_fragment(text, "div"),
                parse_streamed(text, 1),
            ] {
                let actual: String = parsed
                    .dom
                    .iter()
                    .filter_map(|(_, node)| match &node.kind {
                        NodeKind::Text { data } => Some(data.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn foreign_attribute_namespaces_survive_parser_snapshot_and_import() {
        let mut dom = parse("<svg id='svg' xmlns:xlink='http://www.w3.org/1999/xlink' xlink:href='#target' xml:lang='en'></svg>").into_runtime().unwrap();
        let svg = dom.find_by_id("svg").unwrap();
        let href = dom.attribute_node(svg, "xlink:href").unwrap().unwrap();
        assert!(matches!(&dom.arena().get(href).unwrap().kind,
            NodeKind::Attribute { data, .. }
                if data.namespace.as_deref() == Some("http://www.w3.org/1999/xlink") && data.prefix.as_deref() == Some("xlink") && data.local_name == "href"));
        let (snapshot, _) = dom.snapshot();
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(&snapshot, svg).unwrap();
        let lang = other.attribute_node(imported, "xml:lang").unwrap().unwrap();
        assert!(matches!(&other.arena().get(lang).unwrap().kind,
            NodeKind::Attribute { data, .. }
                if data.namespace.as_deref() == Some("http://www.w3.org/XML/1998/namespace") && data.prefix.as_deref() == Some("xml") && data.local_name == "lang" && data.value == "en"));
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
            paragraphs[0].1.attributes.get_null("a").map(String::as_str),
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
                .map(|(_, node)| {
                    let mut node = node.clone();
                    if let Some(parser) = node.script_parser_document.as_mut()
                        && parser.arena == actual.dom.id()
                    {
                        parser.arena = expected.dom.id();
                    }
                    node
                })
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
    fn inline_classic_scripts_ignore_async_and_defer_attributes() {
        for attributes in ["async", "defer", "async defer"] {
            let source = format!("<script {attributes}>execute()</script><p>after</p>");
            let mut parser = StreamingParser::new();
            let progress = parser.push_until_blocked(&source);
            assert!(matches!(
                progress.blocked_on,
                Some(ParserEvent::ScriptReady {
                    scheduling: ScriptScheduling::Blocking,
                    ..
                })
            ));
            assert!(progress.consumed_bytes < source.len());
        }
    }

    #[test]
    fn data_and_policy_boundaries_preserve_preparation_checkpoint_insertion_points() {
        for (kind, scheduling) in [
            ("application/json", ScriptScheduling::Data),
            ("importmap", ScriptScheduling::ImportMap),
            ("speculationrules", ScriptScheduling::SpeculationRules),
        ] {
            let prefix = format!("<script type={kind}>{{}}</script>");
            let source = format!("{prefix}<p id=later>later</p>");
            let mut parser = StreamingParser::new();
            let progress = parser.push_until_resource_or_script(&source);
            assert_eq!(progress.consumed_bytes, prefix.len());
            assert!(progress.blocked_on.is_none());
            let events = parser.take_events();
            assert_eq!(events.len(), 1);
            assert!(
                matches!(events.last(),Some(ParserEvent::ScriptReady{scheduling:actual,..})if *actual==scheduling)
            );
        }
    }

    #[test]
    fn legacy_javascript_mime_types_block_but_parameters_and_unicode_space_do_not() {
        for kind in [
            "text/jscript",
            "application/x-ecmascript",
            " TEXT/JAVASCRIPT1.5\t",
        ] {
            let mut parser = StreamingParser::new();
            let progress =
                parser.push_until_blocked(&format!("<script type='{kind}'>execute()</script>"));
            assert!(matches!(
                progress.blocked_on,
                Some(ParserEvent::ScriptReady {
                    scheduling: ScriptScheduling::Blocking,
                    ..
                })
            ));
        }
        for kind in [
            "text/javascript; charset=utf-8",
            "\u{a0}text/javascript\u{a0}",
            "application/json",
        ] {
            let mut parser = StreamingParser::new();
            let progress =
                parser.push_until_blocked(&format!("<script type='{kind}'>data</script>"));
            assert!(progress.blocked_on.is_none());
            assert!(matches!(
                parser.take_events().as_slice(),
                [ParserEvent::ScriptReady {
                    scheduling: ScriptScheduling::Data,
                    ..
                }]
            ));
        }
    }

    #[test]
    fn script_language_fallback_does_not_override_explicit_type() {
        for (attributes, blocking) in [
            ("language=JavaScript1.5", true),
            ("type='' language=vbscript", true),
            ("type='  ' language=javascript", false),
            ("type=text/plain language=javascript", false),
            ("language=vbscript", false),
        ] {
            let mut parser = StreamingParser::new();
            let progress =
                parser.push_until_blocked(&format!("<script {attributes}>source</script>"));
            assert_eq!(progress.blocked_on.is_some(), blocking, "{attributes}");
        }
    }

    #[test]
    fn attaching_html_or_text_parser_preserves_host_document_identity() {
        for text in [false, true] {
            let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let document = dom.borrow().document();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata = Some(Box::new(crate::DocumentMetadata {
                url: Some("about:srcdoc".into()),
                inherited_base_url: Some("https://example.test/base/".into()),
                encoding: "windows-1252".into(),
                ready_state: crate::DocumentReadyState::Loading,
                ..crate::DocumentMetadata::default()
            }));
            let mut parser = if text {
                StreamingParser::with_runtime_text(dom.clone(), "text/plain")
            } else {
                StreamingParser::with_runtime(dom.clone())
            };
            parser.push("<!doctype html><p>content</p>");
            let parsed = parser.finish();
            let metadata = parsed
                .dom
                .get(parsed.document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap();
            assert_eq!(metadata.url.as_deref(), Some("about:srcdoc"));
            assert_eq!(
                metadata.inherited_base_url.as_deref(),
                Some("https://example.test/base/")
            );
            assert_eq!(metadata.encoding, "windows-1252");
            assert_eq!(metadata.ready_state, crate::DocumentReadyState::Loading);
            assert_eq!(
                metadata.content_type,
                if text { "text/plain" } else { "text/html" }
            );
            assert!(metadata.is_html);
            assert!(!metadata.quirks);
        }
    }

    #[test]
    fn parser_handles_survive_script_removal_until_parser_finishes() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        parser.push("<!doctype html><body><div id='open'>");
        let element = dom.borrow().find_by_id("open").unwrap();
        {
            let mut runtime = dom.borrow_mut();
            runtime.remove(element).unwrap();
            let mutations = runtime.drain_mutations();
            runtime.release_mutation_roots(&mutations).unwrap();
            assert!(runtime.arena().get(element).is_ok());
        }
        parser.push("<span>still parsing</span></div><p>connected</p>");
        let _ = parser.finish();
        let mut runtime = dom.borrow_mut();
        let mutations = runtime.drain_mutations();
        runtime.release_mutation_roots(&mutations).unwrap();
        assert!(runtime.arena().get(element).is_err());
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
                .get_null("data-script"),
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

#[cfg(test)]
mod encoding_indicator_tests {
    use super::*;
    fn events(source: &str) -> Vec<ParserEvent> {
        let mut parser = StreamingParser::new();
        parser.enable_encoding_restart_events();
        parser.push(source);
        parser.take_events()
    }
    #[test]
    fn encoding_indicator_uses_original_head_mode_and_not_fragment_or_template_delegation() {
        let source = format!(
            "<!doctype html><head>{}<meta charset=windows-1251></head><body>",
            " ".repeat(1100)
        );
        let found = events(&source);
        assert!(found.iter().any(
            |event| matches!(event,ParserEvent::EncodingIndicator{label}if label=="windows-1251")
        ));
        assert_eq!(
            found
                .iter()
                .filter(|event| matches!(event, ParserEvent::EncodingCommitted))
                .count(),
            1
        );
        for source in [
            "<head></head><meta charset=windows-1251>",
            "<head><template><meta charset=windows-1251></template>",
            "<head><script>'<meta charset=windows-1251>'</script>",
            "<head><noscript><meta charset=windows-1251></noscript>",
            "<head><link charset=windows-1251>",
        ] {
            assert!(
                !events(source)
                    .iter()
                    .any(|event| matches!(event, ParserEvent::EncodingIndicator { .. })),
                "{source}"
            );
        }
    }
    #[test]
    fn encoding_notifications_are_opt_in_and_invalid_labels_reach_native_decoder() {
        let mut parser = StreamingParser::new();
        parser.push("<meta charset=windows-1251><body>");
        assert!(!parser.take_events().iter().any(|event| matches!(
            event,
            ParserEvent::EncodingIndicator { .. } | ParserEvent::EncodingCommitted
        )));
        assert!(events("<head><meta charset=not-an-encoding><meta charset=windows-1251>").iter().any(|event|matches!(event,ParserEvent::EncodingIndicator{label}if label=="not-an-encoding")));
    }
    #[test]
    fn template_body_does_not_commit_the_outer_head_encoding() {
        let found = events(
            "<head><template><div>template body</div></template><meta charset=windows-1251></head>",
        );
        assert!(found.iter().any(
            |event| matches!(event,ParserEvent::EncodingIndicator{label}if label=="windows-1251")
        ));
    }
}

#[cfg(test)]
mod module_preload_parser_regressions {
    use super::*;
    #[test]
    fn preload_discovery_precedes_following_script_and_skips_inert_template() {
        let parsed = parse(
            "<!doctype html><head><link rel='modulepreload stylesheet' href='main.js'><script>after()</script><template><link rel=modulepreload href=inert.js></template></head>",
        );
        let preload = parsed
            .events
            .iter()
            .position(|event| matches!(event, ParserEvent::ModulePreloadDiscovered { .. }))
            .unwrap();
        let script = parsed
            .events
            .iter()
            .position(|event| matches!(event, ParserEvent::ScriptReady { .. }))
            .unwrap();
        assert!(preload < script);
        assert_eq!(
            parsed
                .events
                .iter()
                .filter(|event| matches!(event, ParserEvent::ModulePreloadDiscovered { .. }))
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod parser_script_document_tests {
    use super::*;
    #[test]
    fn parser_document_survives_adoption_and_prevents_move_back_execution() {
        for markup in [
            "<script id=s>globalThis.ran=true</script>",
            "<script id=s src=''></script>",
            "<script id=s type=module src='/value.js'></script>",
        ] {
            let mut dom = parse(markup).into_runtime().unwrap();
            let original = dom.document();
            let script = dom.find_by_id("s").unwrap();
            assert!(
                dom.script_parser_document_matches(script, original)
                    .unwrap()
            );
            let destination = dom.create(crate::NodeKind::Document);
            dom.set_node_document(script, destination).unwrap();
            assert!(dom.reject_moved_parser_script(script).unwrap());
            assert!(dom.script_already_started(script).unwrap());
            dom.set_node_document(script, original).unwrap();
            assert!(dom.script_already_started(script).unwrap());
        }
    }
    #[test]
    fn svg_script_source_prefers_href_and_preserves_empty_attribute() {
        let mut dom=parse("<script id=h src='html.js' href='ignored.js'></script><svg><script id=s xlink:href='legacy.js'></script></svg>").into_runtime().unwrap();
        let html = dom.find_by_id("h").unwrap();
        let svg = dom.find_by_id("s").unwrap();
        assert_eq!(dom.script_external_source(html).unwrap(), Some("html.js"));
        assert_eq!(dom.script_external_source(svg).unwrap(), Some("legacy.js"));
        dom.set_attribute(svg, "src", "ignored.js").unwrap();
        assert_eq!(dom.script_external_source(svg).unwrap(), Some("legacy.js"));
        dom.set_attribute(svg, "href", "modern.js").unwrap();
        assert_eq!(dom.script_external_source(svg).unwrap(), Some("modern.js"));
        dom.set_attribute(svg, "href", "").unwrap();
        assert_eq!(dom.script_external_source(svg).unwrap(), Some(""));
        dom.remove_attribute_ns(svg, None, "href").unwrap();
        assert_eq!(dom.script_external_source(svg).unwrap(), Some("legacy.js"));
    }

    #[test]
    fn parser_script_text_is_native_and_clones_do_not_copy_trust() {
        let mut dom = parse("<script id=s type=unknown>original</script>")
            .into_runtime()
            .unwrap();
        let script = dom.find_by_id("s").unwrap();
        assert_eq!(
            dom.arena().get(script).unwrap().script_trusted_text,
            "original"
        );
        let clone = dom.clone_node(script, true).unwrap();
        assert!(
            dom.arena()
                .get(clone)
                .unwrap()
                .script_trusted_text
                .is_empty()
        );
        assert_eq!(
            dom.arena().child_text_source(clone, 1024).unwrap(),
            "original"
        );
        dom.set_text_content(script, "changed").unwrap();
        assert!(!dom.reject_moved_parser_script(script).unwrap());
        assert_eq!(
            dom.arena().get(script).unwrap().script_trusted_text,
            "original",
            "preparation does not bless post-parser mutation"
        );
    }

    #[test]
    fn cloning_copies_started_state_without_copying_parser_identity() {
        let mut dom = parse("<script id=s src='/value.js'></script>")
            .into_runtime()
            .unwrap();
        let script = dom.find_by_id("s").unwrap();
        let clone = dom.clone_node(script, true).unwrap();
        assert!(
            dom.arena()
                .get(clone)
                .unwrap()
                .script_parser_document
                .is_none()
        );
        assert!(dom.script_async(clone).unwrap());
        dom.mark_script_started(script).unwrap();
        let clone = dom.clone_node(script, true).unwrap();
        assert!(dom.script_already_started(clone).unwrap());
        let snapshot = dom.arena().clone();
        let parser = snapshot
            .get(script)
            .unwrap()
            .script_parser_document
            .unwrap();
        assert_eq!(parser.arena, snapshot.id());
        assert_eq!(parser.node, dom.document());
    }
    #[test]
    fn empty_parser_script_remains_available_for_later_dynamic_preparation() {
        let mut dom = parse("<script id=s></script>").into_runtime().unwrap();
        let script = dom.find_by_id("s").unwrap();
        let destination = dom.create(crate::NodeKind::Document);
        dom.set_node_document(script, destination).unwrap();
        assert!(!dom.reject_moved_parser_script(script).unwrap());
        assert!(!dom.script_already_started(script).unwrap());
        assert!(
            dom.arena()
                .get(script)
                .unwrap()
                .script_parser_document
                .is_none()
        );
        assert!(dom.script_async(script).unwrap());
    }
    #[test]
    fn cross_arena_adoption_preserves_foreign_parser_identity() {
        let source = std::rc::Rc::new(std::cell::RefCell::new(
            parse("<script id=s src='/value.js'></script>")
                .into_runtime()
                .unwrap(),
        ));
        let destination = std::rc::Rc::new(std::cell::RefCell::new(
            parse("<body></body>").into_runtime().unwrap(),
        ));
        let script = source.borrow().find_by_id("s").unwrap();
        source.borrow_mut().retain_native(script).unwrap();
        let parser = source
            .borrow()
            .arena()
            .get(script)
            .unwrap()
            .script_parser_document;
        let document = destination.borrow().document();
        let mut prepared =
            crate::DomRuntime::prepare_subtree_migration(&source, &destination, script, document)
                .unwrap();
        let moved =
            crate::DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared)
                .unwrap();
        assert_eq!(
            destination
                .borrow()
                .arena()
                .get(moved.root)
                .unwrap()
                .script_parser_document,
            parser
        );
        let body = destination
            .borrow()
            .arena()
            .iter()
            .find_map(|(node, _)| {
                destination
                    .borrow()
                    .is_element(node, "http://www.w3.org/1999/xhtml", "body")
                    .then_some(node)
            })
            .unwrap();
        destination
            .borrow_mut()
            .append_child(body, moved.root)
            .unwrap();
        assert!(
            destination
                .borrow_mut()
                .reject_moved_parser_script(moved.root)
                .unwrap()
        );
        assert!(
            destination
                .borrow()
                .script_already_started(moved.root)
                .unwrap()
        );
    }
}

#[cfg(test)]
mod legacy_script_event_tests {
    #[test]
    fn classic_filter_uses_html_ascii_whitespace_and_case_only() {
        use super::classic_script_event_allowed as allowed;
        for (target, event) in [("window", "onload"), (" WINDOW\t", "\nOnLoad()\r")] {
            assert!(allowed(Some(target), Some(event)));
        }
        for (target, event) in [
            ("wİndow", "onload"),
            ("window", "onload x"),
            ("window", "onload(x"),
            ("window", "onload(x)"),
            ("window", "onclick"),
            ("", "onload"),
            ("window", ""),
            ("", ""),
            ("\u{a0}window", "onload"),
            ("window\u{a0}", "onload"),
            ("window", "\u{a0}onload"),
            ("window", "onload\u{a0}"),
            ("object", "handler"),
        ] {
            assert!(!allowed(Some(target), Some(event)), "{target:?} {event:?}");
        }
        assert!(allowed(None, Some("handler")));
        assert!(allowed(Some("object"), None));
    }
}

#[cfg(test)]
mod open_element_snapshot_tests {
    use super::*;
    #[test]
    fn target_closes_only_after_end_tag_and_abort_clears_stack() {
        let mut parser = StreamingParser::new();
        let dom = parser.dom.clone();
        let document = parser.document;
        parser.push("<body><div id=target><span>inside");
        let target = dom.borrow().find_by_id("target").unwrap();
        assert_eq!(
            dom.borrow()
                .arena()
                .get(target)
                .unwrap()
                .parser_open_document,
            Some(crate::NodeRef {
                arena: dom.borrow().arena().id(),
                node: document
            })
        );
        parser.push("</span></div>");
        assert!(
            dom.borrow()
                .arena()
                .get(target)
                .unwrap()
                .parser_open_document
                .is_none()
        );
        parser.push("<p id=unfinished>pending");
        let unfinished = dom.borrow().find_by_id("unfinished").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(unfinished)
                .unwrap()
                .parser_open_document
                .is_some()
        );
        drop(parser);
        assert!(
            dom.borrow()
                .arena()
                .get(unfinished)
                .unwrap()
                .parser_open_document
                .is_none()
        );
    }
    #[test]
    fn implied_end_tag_and_formatting_recovery_refresh_actual_stack() {
        let mut parser = StreamingParser::new();
        let dom = parser.dom.clone();
        parser.push("<body><p id=first>first<p id=second>second");
        let first = dom.borrow().find_by_id("first").unwrap();
        let second = dom.borrow().find_by_id("second").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(first)
                .unwrap()
                .parser_open_document
                .is_none()
        );
        assert!(
            dom.borrow()
                .arena()
                .get(second)
                .unwrap()
                .parser_open_document
                .is_some()
        );
        parser.push("<table><tr><td id=cell><b>bold</td></tr></table>");
        let cell = dom.borrow().find_by_id("cell").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(cell)
                .unwrap()
                .parser_open_document
                .is_none()
        );
    }
}

#[cfg(test)]
mod retained_parser_mutation_tests {
    use super::*;
    #[test]
    fn interrupted_template_initialization_retains_one_fragment_and_ownership_tail() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        parser.push("<body>");
        let input =
            ParserInput::unicode("<template><b id=child>kept</b></template><p id=after>suffix</p>");
        // The first fallible ownership step commits the fragment's inert owner
        // then interrupts. The same Initialize receipt must finish that suffix.
        crate::insertion::set_failure_after(Some(0));
        let progress = parser.push_input_until_resource_or_script(&input);
        crate::insertion::set_failure_after(None);
        assert!(parser.has_pending_mutations());
        let template = dom
            .borrow()
            .arena()
            .iter()
            .find_map(|(node, record)| {
                matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="template")
                    .then_some(node)
            })
            .unwrap();
        let content = dom
            .borrow()
            .arena()
            .get(template)
            .unwrap()
            .template_content
            .unwrap();
        let inert = dom.borrow().node_document(content).unwrap().unwrap();
        let suffix = input.clone().split_off(progress.consumed_bytes);
        assert_eq!(
            parser
                .push_input_until_resource_or_script(&suffix)
                .consumed_bytes,
            suffix.len()
        );
        assert!(parser.prepare_finish());
        assert_eq!(
            dom.borrow().arena().get(template).unwrap().template_content,
            Some(content)
        );
        assert_eq!(dom.borrow().node_document(content).unwrap(), Some(inert));
        assert_eq!(dom.borrow().arena().text_content(content).unwrap(), "kept");
        assert_eq!(dom.borrow().children(content).unwrap().len(), 1);
        assert!(dom.borrow().find_by_id("after").is_some());
        assert_eq!(dom.borrow().arena().iter().filter(|(_,record)|matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="template")).count(),1);
    }

    #[test]
    fn migrated_open_parent_receives_following_text_and_elements() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(source.clone());
        parser.push("<body><div id=parent>");
        let parent = source.borrow().find_by_id("parent").unwrap();
        let document = target.borrow().document();
        let mut migration =
            DomRuntime::prepare_subtree_migration(&source, &target, parent, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut migration).unwrap();
        let _observer = crate::TrackedNodeLease::new(&target, moved.root).unwrap();
        parser.push("<span id=child>kept</span>tail</div>");
        let child = target
            .borrow()
            .get_element_by_id(moved.root, "child")
            .unwrap()
            .unwrap();
        assert!(!target.borrow().is_connected(moved.root).unwrap());
        assert_eq!(target.borrow().parent(child).unwrap(), Some(moved.root));
        assert_eq!(
            target.borrow().arena().text_content(moved.root).unwrap(),
            "kepttail"
        );
        assert!(source.borrow().find_by_id("child").is_none());
    }
    #[test]
    fn failed_adoption_retains_token_prefix_and_retries_before_suffix() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(source.clone());
        let mut constructions = 0;
        let mut fail = true;
        {
            let mut callback = |action| match action {
                ParserAction::Checkpoint => Some(ParserNode {
                    owner: source.clone(),
                    node: source.borrow().document(),
                }),
                ParserAction::Construct {
                    target: element, ..
                } => {
                    constructions += 1;
                    let document = target.borrow().document();
                    let mut migration = DomRuntime::prepare_subtree_migration(
                        &element.owner,
                        &target,
                        element.node,
                        document,
                    )
                    .unwrap();
                    let moved = DomRuntime::commit_subtree_migration(
                        &element.owner,
                        &target,
                        &mut migration,
                    )
                    .unwrap();
                    Some(ParserNode {
                        owner: target.clone(),
                        node: moved.root,
                    })
                }
                ParserAction::Adopt {
                    target: element,
                    parent,
                } => {
                    if std::mem::take(&mut fail) {
                        return None;
                    }
                    let document = parent
                        .owner
                        .borrow()
                        .node_document(parent.node)
                        .unwrap()
                        .unwrap_or(parent.node);
                    let mut migration = DomRuntime::prepare_subtree_migration(
                        &element.owner,
                        &parent.owner,
                        element.node,
                        document,
                    )
                    .unwrap();
                    let moved = DomRuntime::commit_subtree_migration(
                        &element.owner,
                        &parent.owner,
                        &mut migration,
                    )
                    .unwrap();
                    Some(ParserNode {
                        owner: parent.owner,
                        node: moved.root,
                    })
                }
                ParserAction::Failed(_) => None,
            };
            let input = "<body><x-retained id=token><b id=suffix></b></x-retained>";
            let progress = with_parser_actions(&source, &mut callback, || {
                parser.push_until_resource_or_script(input)
            });
            assert!(parser.has_pending_mutations());
            assert!(source.borrow().find_by_id("suffix").is_none());
            let next = with_parser_actions(&source, &mut callback, || {
                parser.push_until_resource_or_script(&input[progress.consumed_bytes..])
            });
            assert_eq!(next.consumed_bytes, input.len() - progress.consumed_bytes);
        }
        assert_eq!(constructions, 1);
        assert!(!parser.has_pending_mutations());
        assert!(source.borrow().find_by_id("suffix").is_some());
    }
}

#[cfg(test)]
mod parser_eof_custody_tests {
    use super::*;
    #[test]
    fn eof_failure_retains_pending_text_and_never_repeats_eof() {
        let mut parser = StreamingParser::new();
        parser.process("<body>tail");
        FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        assert!(!parser.prepare_finish());
        assert!(parser.end_started);
        assert!(parser.has_pending_mutations());
        assert!(parser.prepare_finish());
        assert!(!parser.has_pending_mutations());
        assert!(parser.prepare_finish());
        let result = parser.finish();
        assert_eq!(result.dom.text_content(result.document).unwrap(), "tail");
    }
    #[test]
    fn cancellation_releases_pending_insert_from_its_current_owner() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let source_document = source.borrow().document();
        let sink = OrynTreeSink::with_runtime(source.clone(), source_document);
        let node = source.borrow_mut().create_text("pending");
        let key = sink.key(&source, node);
        let document = target.borrow().document();
        let mut migration =
            DomRuntime::prepare_subtree_migration(&source, &target, node, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut migration).unwrap();
        FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        sink.queue_mutation(ParserMutation::Insert {
            parent: sink.document,
            node: key,
            before: None,
            receipt: None,
        });
        assert!(sink.mutation_failed.get());
        drop(sink);
        drop(migration);
        crate::drain_node_migration_pins().unwrap();
        crate::drain_tracked_node_leases().unwrap();
        assert!(target.borrow().arena().get(moved.root).is_err());
    }
}

#[cfg(test)]
mod parser_postcommit_retry_tests {
    use super::*;
    #[test]
    fn committed_adoption_failure_retries_publication_before_insertion_without_readoption() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(source.clone());
        let mut constructions = 0;
        let mut adoptions = 0;
        let mut awaiting_publication = false;
        let mut publications = 0;
        {
            let mut callback = |action| match action {
                ParserAction::Checkpoint => {
                    if awaiting_publication {
                        awaiting_publication = false;
                        publications += 1;
                    }
                    Some(ParserNode {
                        owner: source.clone(),
                        node: source.borrow().document(),
                    })
                }
                ParserAction::Construct {
                    target: element, ..
                } => {
                    constructions += 1;
                    let document = target.borrow().document();
                    let mut migration = DomRuntime::prepare_subtree_migration(
                        &element.owner,
                        &target,
                        element.node,
                        document,
                    )
                    .unwrap();
                    let moved = DomRuntime::commit_subtree_migration(
                        &element.owner,
                        &target,
                        &mut migration,
                    )
                    .unwrap();
                    Some(ParserNode {
                        owner: target.clone(),
                        node: moved.root,
                    })
                }
                ParserAction::Adopt {
                    target: element,
                    parent,
                } => {
                    adoptions += 1;
                    let document = parent
                        .owner
                        .borrow()
                        .node_document(parent.node)
                        .unwrap()
                        .unwrap_or(parent.node);
                    let mut migration = DomRuntime::prepare_subtree_migration(
                        &element.owner,
                        &parent.owner,
                        element.node,
                        document,
                    )
                    .unwrap();
                    let _ = DomRuntime::commit_subtree_migration(
                        &element.owner,
                        &parent.owner,
                        &mut migration,
                    )
                    .unwrap();
                    awaiting_publication = true;
                    None
                }
                ParserAction::Failed(_) => None,
            };
            let source_text = "<body><x-postcommit id=token><span id=later></span></x-postcommit>";
            let progress = with_parser_actions(&source, &mut callback, || {
                parser.push_until_resource_or_script(source_text)
            });
            assert!(parser.has_pending_mutations());
            assert!(source.borrow().find_by_id("later").is_none());
            let _ = with_parser_actions(&source, &mut callback, || {
                parser.push_until_resource_or_script(&source_text[progress.consumed_bytes..])
            });
        }
        assert_eq!((constructions, adoptions, publications), (1, 1, 1));
        assert!(!awaiting_publication);
        assert!(!parser.has_pending_mutations());
        assert!(source.borrow().find_by_id("later").is_some());
    }
}

#[cfg(test)]
mod parser_admission_order_tests {
    use super::*;
    #[test]
    fn failure_without_v8_hook_preserves_html_suffix_until_retry() {
        let mut parser = StreamingParser::new();
        parser.push("<body>");
        let input = "<section id=first><span id=later>tail</span></section>";
        FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        let progress = parser.push_until_resource_or_script(input);
        assert_eq!(progress.consumed_bytes, "<section id=first>".len());
        assert!(parser.has_pending_mutations());
        assert!(parser.shared_dom().borrow().find_by_id("later").is_none());
        let next = parser.push_until_resource_or_script(&input[progress.consumed_bytes..]);
        assert_eq!(next.consumed_bytes, input.len() - progress.consumed_bytes);
        assert!(!parser.has_pending_mutations());
        let dom = parser.shared_dom();
        let dom = dom.borrow();
        let first = dom.find_by_id("first").unwrap();
        let later = dom.find_by_id("later").unwrap();
        assert_eq!(dom.parent(later).unwrap(), Some(first));
    }
    #[test]
    fn reparent_reads_children_after_failed_insertion_commits() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        let sink = OrynTreeSink::with_runtime(dom.clone(), document);
        let old = dom.borrow_mut().create_fragment();
        let new = dom.borrow_mut().create_fragment();
        let first = dom.borrow_mut().create_text("first");
        let second = dom.borrow_mut().create_text("second");
        let old = sink.key(&dom, old);
        let new = sink.key(&dom, new);
        let first = sink.key(&dom, first);
        let second = sink.key(&dom, second);
        FAIL_PARSER_MUTATION.with(|fault| fault.set(true));
        sink.append(&old, NodeOrText::AppendNode(first));
        sink.append(&old, NodeOrText::AppendNode(second));
        sink.reparent_children(&old, &new);
        assert!(sink.mutation_failed.get());
        assert!(sink.resume_pending_mutations());
        assert!(dom.borrow().children(old).unwrap().is_empty());
        assert_eq!(dom.borrow().children(new).unwrap(), [first, second]);
        assert!(sink.resume_pending_mutations());
        assert_eq!(dom.borrow().children(new).unwrap(), [first, second]);
    }
}

#[cfg(test)]
mod parser_native_receipt_tests {
    use super::*;
    #[test]
    fn insertion_failures_keep_one_native_edge_and_intervening_author_attribute() {
        for failure in 0..48 {
            let owner = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let document = owner.borrow().document();
            let sink = OrynTreeSink::with_runtime(owner.clone(), document);
            let parent = owner.borrow_mut().create_element("div");
            owner.borrow_mut().append_child(document, parent).unwrap();
            let child = owner.borrow_mut().create_element("span");
            let parent_key = sink.key(&owner, parent);
            let key = sink.key(&owner, child);
            let mutations = owner.borrow().mutation_cursor();
            crate::insertion::set_failure_after(Some(failure));
            sink.queue_mutation(ParserMutation::Insert {
                parent: parent_key,
                node: key,
                before: None,
                receipt: None,
            });
            crate::insertion::set_failure_after(None);
            owner
                .borrow_mut()
                .set_attribute(child, "data-author", "kept")
                .unwrap();
            assert!(sink.resume_pending_mutations());
            assert_eq!(owner.borrow().children(parent).unwrap(), &[child]);
            assert_eq!(
                owner
                    .borrow()
                    .attribute_in_null_namespace(child, "data-author")
                    .unwrap(),
                Some("kept")
            );
            assert!(sink.pending_mutations.borrow().is_empty());
            assert_eq!(
                owner
                    .borrow()
                    .mutations_since(mutations)
                    .mutations
                    .iter()
                    .filter(|row| matches!(row,crate::Mutation::Inserted{node,..}if *node==child))
                    .count(),
                1
            );
        }
    }
    #[test]
    fn removal_failures_complete_once_and_do_not_replay_after_author_reinsertion() {
        for failure in 0..36 {
            let owner = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
            let document = owner.borrow().document();
            let sink = OrynTreeSink::with_runtime(owner.clone(), document);
            let parent = owner.borrow_mut().create_element("div");
            owner.borrow_mut().append_child(document, parent).unwrap();
            let child = owner.borrow_mut().create_element("span");
            owner.borrow_mut().append_child(parent, child).unwrap();
            let key = sink.key(&owner, child);
            crate::insertion::set_failure_after(Some(failure));
            sink.queue_mutation(ParserMutation::Detach {
                node: key,
                receipt: None,
            });
            crate::insertion::set_failure_after(None);
            assert!(sink.resume_pending_mutations());
            assert_eq!(owner.borrow().parent(child).unwrap(), None);
            owner.borrow_mut().append_child(parent, child).unwrap();
            assert!(sink.resume_pending_mutations());
            assert_eq!(owner.borrow().parent(child).unwrap(), Some(parent));
        }
    }
}
