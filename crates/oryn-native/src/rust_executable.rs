//! Production executable page backed by the Rust-owned live DOM.
//!
//! This module never exports or reconstructs a JavaScript-owned node graph.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Duration,
};

use html5ever::{QualName, local_name, ns};
use oryn_common::v3::SemanticAction;
use oryn_dom::{
    DomError, DomRuntime, Mutation, MutationCursor, NodeId, NodeKind, SharedDomRuntime, html,
};
use oryn_js::realm::RealmId;
use oryn_js::v8_bindings::{
    BindingAccessibility, BindingGeometry, BindingLayout, BindingLayoutProvider, BindingRect,
    ParserInputCommand,
};
use serde::{Deserialize, Serialize};

use crate::{
    host::{HostError, JavaScriptHost, RawV8Host},
    html::ParsedDocument,
    network::SharedNetworkBroker,
};
use url::Url;

const DOCUMENT_CONTENT_LOADED_SCRIPT: &str =
    "document.dispatchEvent(new Event('DOMContentLoaded',{bubbles:true}))";
const DOCUMENT_LOAD_SCRIPT: &str = "if(document.body){if(typeof document.body.onload==='function')globalThis.onload=document.body.onload;else{const source=document.body.getAttribute('onload');if(source)globalThis.onload=Function(source)}}dispatchEvent(new Event('load'))";

struct LiveLayoutProvider {
    dom: SharedDomRuntime,
    viewport: oryn_common::v4::ViewportState,
}

fn populate_validation_states(
    dom: &DomRuntime,
    input: &mut oryn_layout::LayoutInput,
) -> Result<(), DomError> {
    for node in &mut input.nodes {
        let id = NodeId {
            slot: node.id as u32,
            generation: (node.id >> 32) as u32,
        };
        node.validity = dom.validation_selector_state(id)?;
    }
    Ok(())
}

impl BindingLayoutProvider for LiveLayoutProvider {
    fn set_page_rule_selector(
        &self,
        source: &str,
        path: &[usize],
        selector: &str,
        base_url: &str,
        quirks: bool,
    ) -> Option<Result<Option<Vec<oryn_js::v8_bindings::BindingRuleView>>, String>> {
        Some(
            oryn_layout::cssom::set_page_rule_selector(source, path, selector, base_url, quirks)
                .map(|rules| rules.map(|rules| rules.into_iter().map(binding_rule_view).collect())),
        )
    }

    fn mutate_stylesheet_media(
        &self,
        text: &str,
        edit: oryn_js::v8_bindings::BindingMediaListEdit<'_>,
        base_url: &str,
        quirks: bool,
    ) -> Option<Result<oryn_js::v8_bindings::BindingStylesheetMediaMutation, String>> {
        use oryn_js::v8_bindings::{BindingMediaListEdit, BindingStylesheetMediaMutation};
        use oryn_layout::cssom::MediaListEdit;
        let edit = match edit {
            BindingMediaListEdit::Replace(text) => MediaListEdit::Replace(text),
            BindingMediaListEdit::Append(text) => MediaListEdit::Append(text),
            BindingMediaListEdit::Delete(text) => MediaListEdit::Delete(text),
        };
        Some(
            oryn_layout::cssom::mutate_stylesheet_media(text, edit, base_url, quirks).map(
                |result| BindingStylesheetMediaMutation {
                    queries: result.queries,
                    changed: result.changed,
                    not_found: result.not_found,
                },
            ),
        )
    }

    fn mutate_media_list(
        &self,
        source: &str,
        path: &[usize],
        edit: oryn_js::v8_bindings::BindingMediaListEdit<'_>,
        base_url: &str,
        quirks: bool,
    ) -> Option<Result<oryn_js::v8_bindings::BindingMediaListMutation, String>> {
        use oryn_js::v8_bindings::{BindingMediaListEdit, BindingMediaListMutation};
        use oryn_layout::cssom::MediaListEdit;
        let edit = match edit {
            BindingMediaListEdit::Replace(text) => MediaListEdit::Replace(text),
            BindingMediaListEdit::Append(text) => MediaListEdit::Append(text),
            BindingMediaListEdit::Delete(text) => MediaListEdit::Delete(text),
        };
        Some(
            oryn_layout::cssom::mutate_media_list(source, path, edit, base_url, quirks).map(
                |result| BindingMediaListMutation {
                    rules: result
                        .rules
                        .map(|rules| rules.into_iter().map(binding_rule_view).collect()),
                    not_found: result.not_found,
                },
            ),
        )
    }

    fn mutate_rule_list(
        &self,
        source: &str,
        parent_path: &[usize],
        edit: oryn_js::v8_bindings::BindingRuleListEdit<'_>,
        base_url: &str,
        quirks: bool,
        allow_imports: bool,
    ) -> Option<
        Result<
            Vec<oryn_js::v8_bindings::BindingRuleView>,
            oryn_js::v8_bindings::BindingRuleMutationError,
        >,
    > {
        use oryn_js::v8_bindings::{BindingRuleListEdit, BindingRuleMutationError as BindingError};
        use oryn_layout::cssom::{RuleListEdit, RuleMutationError};
        let edit = match edit {
            BindingRuleListEdit::Insert { index, text } => RuleListEdit::Insert { index, text },
            BindingRuleListEdit::Delete { index } => RuleListEdit::Delete { index },
        };
        Some(
            oryn_layout::cssom::mutate_rule_list(
                source,
                parent_path,
                edit,
                base_url,
                quirks,
                allow_imports,
            )
            .map(|rules| rules.into_iter().map(binding_rule_view).collect())
            .map_err(|error| match error {
                RuleMutationError::Syntax => BindingError::Syntax,
                RuleMutationError::IndexSize => BindingError::IndexSize,
                RuleMutationError::HierarchyRequest => BindingError::HierarchyRequest,
                RuleMutationError::InvalidState => BindingError::InvalidState,
                RuleMutationError::InvalidContext(message) => BindingError::InvalidContext(message),
            }),
        )
    }

    fn set_style_rule_selector(
        &self,
        source: &str,
        path: &[usize],
        selector: &str,
        base_url: &str,
        quirks: bool,
    ) -> Option<Result<Option<Vec<oryn_js::v8_bindings::BindingRuleView>>, String>> {
        Some(
            oryn_layout::cssom::set_style_rule_selector(source, path, selector, base_url, quirks)
                .map(|rules| rules.map(|rules| rules.into_iter().map(binding_rule_view).collect())),
        )
    }

    fn mutate_rule_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
        path: &[usize],
        replace: bool,
    ) -> Option<Result<oryn_js::v8_bindings::BindingRuleDeclarationMutation, String>> {
        Some(
            oryn_layout::cssom::mutate_rule_declaration(
                request.css_text,
                path,
                if replace {
                    None
                } else {
                    Some(request.property)
                },
                request.value,
                request.priority,
                request.base_url,
                request.quirks,
            )
            .map(
                |mutation| oryn_js::v8_bindings::BindingRuleDeclarationMutation {
                    previous_value: mutation.previous_value,
                    rules: mutation
                        .rules
                        .map(|rules| rules.into_iter().map(binding_rule_view).collect()),
                },
            ),
        )
    }

    fn read_font_face_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
    ) -> Option<Result<oryn_js::v8_bindings::BindingDeclarationView, String>> {
        Some(
            oryn_layout::cssom::read_font_face_declaration(
                request.css_text,
                request.property,
                request.base_url,
                request.quirks,
            )
            .map(|view| oryn_js::v8_bindings::BindingDeclarationView {
                css_text: view.css_text,
                names: view.names,
                value: view.value,
                important: view.important,
            }),
        )
    }

    fn read_page_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
    ) -> Option<Result<oryn_js::v8_bindings::BindingDeclarationView, String>> {
        Some(
            oryn_layout::cssom::read_page_declaration(
                request.css_text,
                request.property,
                request.base_url,
                request.quirks,
            )
            .map(|view| oryn_js::v8_bindings::BindingDeclarationView {
                css_text: view.css_text,
                names: view.names,
                value: view.value,
                important: view.important,
            }),
        )
    }

    fn mutate_page_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
        replace: bool,
    ) -> Option<Result<oryn_js::v8_bindings::BindingPageDeclarationMutation, String>> {
        let result = if replace {
            oryn_layout::cssom::replace_page_declarations(
                request.css_text,
                request.value,
                request.base_url,
                request.quirks,
            )
        } else {
            oryn_layout::cssom::mutate_page_declaration(
                request.css_text,
                request.property,
                request.value,
                request.priority,
                request.base_url,
                request.quirks,
            )
        };
        Some(result.map(
            |mutation| oryn_js::v8_bindings::BindingPageDeclarationMutation {
                previous_value: mutation.previous_value,
                rule: mutation.rule.map(binding_rule_view),
            },
        ))
    }

    fn set_page_selector(
        &self,
        source: &str,
        selector: &str,
        base_url: &str,
        quirks: bool,
    ) -> Option<Result<Option<oryn_js::v8_bindings::BindingRuleView>, String>> {
        Some(
            oryn_layout::cssom::set_page_selector(source, selector, base_url, quirks)
                .map(|rule| rule.map(binding_rule_view)),
        )
    }

    fn read_stylesheet(
        &self,
        source: &str,
        base_url: &str,
        quirks: bool,
        allow_imports: bool,
    ) -> Option<Result<Vec<oryn_js::v8_bindings::BindingRuleView>, String>> {
        Some(
            oryn_layout::cssom::read_stylesheet(source, base_url, quirks, allow_imports)
                .map(|rules| rules.into_iter().map(binding_rule_view).collect()),
        )
    }

    fn supported_css_properties(&self) -> Vec<(String, String)> {
        oryn_layout::cssom::supported_properties()
    }

    fn read_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
    ) -> Option<Result<oryn_js::v8_bindings::BindingDeclarationView, String>> {
        Some(
            oryn_layout::cssom::read_declaration(
                request.css_text,
                request.property,
                request.base_url,
                request.quirks,
            )
            .map(|result| oryn_js::v8_bindings::BindingDeclarationView {
                css_text: result.css_text,
                names: result.names,
                value: result.value,
                important: result.important,
            }),
        )
    }

    fn mutate_declaration(
        &self,
        request: &oryn_js::v8_bindings::BindingDeclarationRequest<'_>,
    ) -> Option<Result<oryn_js::v8_bindings::BindingDeclarationMutation, String>> {
        Some(
            oryn_layout::cssom::mutate_declaration(
                request.css_text,
                request.property,
                request.value,
                request.priority,
                request.base_url,
                request.quirks,
            )
            .map(|result| oryn_js::v8_bindings::BindingDeclarationMutation {
                previous_value: result.previous_value,
                css_text: result.css_text,
            }),
        )
    }

    fn layout_revision(&self) -> Option<u64> {
        // The viewport is immutable for this provider; replacement invalidates the
        // binding cache. DOM mutations include stylesheet, value, and focus changes.
        Some(self.dom.borrow().mutation_cursor().0)
    }

    fn compute_layout(
        &self,
        custom_states: &BTreeMap<NodeId, BTreeSet<String>>,
    ) -> Result<BindingLayout, String> {
        let (dom, document) = self.dom.borrow().snapshot();
        let parsed = ParsedDocument {
            dom,
            document,
            errors: Vec::new(),
            quirks_mode: html5ever::interface::QuirksMode::NoQuirks,
            events: Vec::new(),
        };
        let mut input = crate::runtime::layout_input(
            &parsed,
            self.viewport,
            oryn_common::v4::ScrollState::default(),
            oryn_common::v4::PreferredColorScheme::Light,
            oryn_common::v4::ReducedMotion::NoPreference,
            &[],
        );
        input.focused_node = self
            .dom
            .borrow()
            .focused()
            .map(|node| (u64::from(node.generation) << 32) | u64::from(node.slot));
        input.focus_visible = self.dom.borrow().focus_visible();
        populate_validation_states(&self.dom.borrow(), &mut input)
            .map_err(|error| error.to_string())?;
        {
            let dom = self.dom.borrow();
            for node in &mut input.nodes {
                let id = NodeId {
                    slot: node.id as u32,
                    generation: (node.id >> 32) as u32,
                };
                node.indeterminate = dom
                    .is_indeterminate(id)
                    .map_err(|error| error.to_string())?;
            }
        }
        input.custom_states = custom_states
            .iter()
            .map(|(node, values)| {
                (
                    (u64::from(node.generation) << 32) | u64::from(node.slot),
                    values.clone(),
                )
            })
            .collect();
        // Geometry is an optional capability and must never be able to tear
        // down the page worker. Stylo/Blitz can reject a transient DOM shape
        // while parser re-entry is replacing nested documents; translate an
        // upstream panic into the same explicit unavailable result as a
        // normal layout error.
        let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            oryn_layout::StyleEngine::default()
                .compute(&input, &oryn_layout::LayoutLimits::default())
        }))
        .map_err(|_| "synchronous layout flush panicked on a transient DOM shape".to_owned())?
        .map_err(|error| format!("synchronous layout flush failed: {error}"))?;
        Ok(binding_layout_from_output(&output))
    }

    fn query_selector(
        &self,
        node: NodeId,
        selector: &str,
        operation: oryn_dom::SelectorOperation,
        custom_states: &BTreeMap<NodeId, BTreeSet<String>>,
    ) -> Option<Result<Vec<NodeId>, String>> {
        Some(oryn_layout::dom_queries::query(
            &self.dom.borrow(),
            node,
            selector,
            operation,
            custom_states,
        ))
    }

    fn canonicalize_selector(&self, selector: &str) -> Result<String, String> {
        oryn_layout::canonicalize_selector(selector)
    }

    fn compute_accessibility(&self, node: NodeId) -> Result<BindingAccessibility, String> {
        let dom = self.dom.borrow();
        let (role, name) = crate::semantic::accessibility_for_node(dom.arena(), node)
            .ok_or_else(|| "target has no accessible semantic projection".to_string())?;
        Ok(BindingAccessibility { role, name })
    }
}

fn binding_layout_from_output(output: &oryn_layout::LayoutOutput) -> BindingLayout {
    let geometry = output
        .geometry
        .iter()
        .map(|(node, value)| {
            use oryn_common::v4::GeometryAvailability;
            let value = match value.availability {
                GeometryAvailability::Available => value.box_model.as_ref().map_or_else(
                    || {
                        BindingGeometry::Unsupported(
                            "layout reported available geometry without a box".into(),
                        )
                    },
                    |model| {
                        let rect = model.border_box;
                        BindingGeometry::Available(BindingRect {
                            x: rect.x,
                            y: rect.y,
                            width: rect.width,
                            height: rect.height,
                        })
                    },
                ),
                GeometryAvailability::Hidden => BindingGeometry::Hidden,
                GeometryAvailability::Unsupported => BindingGeometry::Unsupported(
                    value
                        .diagnostic
                        .clone()
                        .unwrap_or_else(|| "geometry is unsupported".into()),
                ),
            };
            (*node, value)
        })
        .collect();
    let mut styles = output.computed_styles.clone();
    let mut resolved_styles = BTreeSet::new();
    for (node, resolved) in &output.resolved_values {
        if let Some(values) = styles.get_mut(node) {
            if resolved
                .iter()
                .all(|(name, value)| values.get(name) == Some(value))
            {
                continue;
            }
            let mut combined = (**values).clone();
            combined.extend(
                resolved
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            *values = if let Some(shared) = resolved_styles.get(&combined) {
                std::sync::Arc::clone(shared)
            } else {
                let shared = std::sync::Arc::new(combined);
                resolved_styles.insert(shared.clone());
                shared
            };
        }
    }
    BindingLayout {
        author_whitespace: output.author_whitespace.clone(),
        offset_dimensions: output.offset_dimensions.clone(),
        styles,
        geometry,
    }
}

fn resource_entry<'a>(
    resources: &'a BTreeMap<String, String>,
    name: &str,
) -> Option<(&'a str, &'a str)> {
    if let Some(canonical) = resources.get(&format!("oryn:module-alias:{name}")) {
        return resources
            .get_key_value(canonical)
            .map(|(name, source)| (name.as_str(), source.as_str()));
    }
    resources
        .get_key_value(name)
        .map(|(name, source)| (name.as_str(), source.as_str()))
}

fn module_dependencies(source: &str) -> Vec<String> {
    let pattern = regex::Regex::new(
        r#"(?m)(?:\bimport|\bexport)[^;\n]*?\bfrom\s*["']([^"']+)["']|\bimport\s*(?:\(\s*)?["']([^"']+)["']"#,
    )
    .expect("module dependency regex");
    pattern
        .captures_iter(source)
        .filter_map(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|value| value.as_str().to_owned())
        .collect()
}

#[cfg(unix)]
fn preload_module_graph(
    host: &mut RawV8Host,
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    root_name: &str,
    root_source: &str,
    resources: &mut BTreeMap<String, String>,
) -> Result<(), HostError> {
    let mut pending = vec![(root_name.to_owned(), root_source.to_owned())];
    let mut visited = std::collections::BTreeSet::new();
    let mut loaded = 0usize;
    while !pending.is_empty() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut scheduled = 0usize;
        for (parent, source) in std::mem::take(&mut pending) {
            for dependency in module_dependencies(&source) {
                // Speculative loading does not record resolution and must not
                // turn a bare import or a fragment into a relative HTTP URL.
                let Ok(Some(canonical)) = host.resolve_module_specifier(&parent, &dependency)
                else {
                    continue;
                };
                if resources.contains_key(&canonical) {
                    continue;
                }
                if !visited.insert(canonical.clone()) {
                    continue;
                }
                loaded = loaded.saturating_add(1);
                if loaded > 256 {
                    return Err(HostError::LimitExceeded {
                        resource: "module dependency graph".into(),
                    });
                }
                let sender = sender.clone();
                let client = client.clone();
                let initiator_url = parent.clone();
                std::thread::spawn(move || {
                    let result = client
                        .request(crate::context_service::ContextServiceRequest::LoadScript {
                            page_id,
                            url: canonical,
                            module: true,
                            nonce: None,
                            initiator_url: Some(initiator_url),
                            referrer_policy: None,
                        })
                        .map_err(|error| error.to_string())
                        .and_then(|response| match response {
                            crate::context_service::ContextServiceResponse::ScriptSource {
                                url,
                                source,
                                ..
                            } => Ok((url, source)),
                            _ => Err("parent returned an invalid module response".into()),
                        });
                    let _ = sender.send(result);
                });
                scheduled = scheduled.saturating_add(1);
            }
        }
        for _ in 0..scheduled {
            let loaded = receiver
                .recv()
                .map_err(|_| HostError::Execution("module scheduler disconnected".into()))?;
            // A regex-discovered dependency may be an unused dynamic import.
            // Only V8's actual resolution may reject the importing script.
            let Ok((url, source)) = loaded else { continue };
            resources.insert(url.clone(), source.clone());
            pending.push((url, source));
        }
    }
    Ok(())
}

fn is_script_element(dom: &DomRuntime, node: NodeId) -> bool {
    dom.is_element(node, "http://www.w3.org/1999/xhtml", "script")
        || dom.is_element(node, "http://www.w3.org/2000/svg", "script")
}

fn collect_scripts(dom: &SharedDomRuntime) -> Result<Vec<ScriptRecord>, HostError> {
    let dom = dom.borrow();
    dom.query_selector_all(dom.document(), "script")
        .map_err(|error| HostError::Execution(error.to_string()))?
        .into_iter()
        .filter(|node| is_script_element(&dom, *node))
        .map(|node| script_record(&dom, node))
        .collect()
}

fn collect_frames(dom: &SharedDomRuntime) -> Result<Vec<(NodeId, Option<String>)>, HostError> {
    let dom = dom.borrow();
    Ok(dom
        .query_selector_all(dom.document(), "iframe")
        .map_err(|error| HostError::Execution(error.to_string()))?
        .into_iter()
        .filter(|node| dom.is_element(*node, "http://www.w3.org/1999/xhtml", "iframe"))
        .map(|node| {
            (
                node,
                dom.attribute(node, "src").ok().flatten().map(str::to_owned),
            )
        })
        .collect())
}

fn script_record(dom: &DomRuntime, node: NodeId) -> Result<ScriptRecord, HostError> {
    let script_type = dom
        .attribute(node, "type")
        .map_err(|error| HostError::Execution(error.to_string()))?
        .map(str::to_owned);
    let deferred = (script_type.as_deref() == Some("module")
        && dom
            .attribute(node, "async")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .is_none())
        || dom
            .attribute(node, "defer")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .is_some();
    let parser_blocking = script_type
        .as_deref()
        .is_none_or(|kind| matches!(kind, "" | "text/javascript" | "application/javascript"))
        && dom
            .attribute(node, "async")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .is_none()
        && dom
            .attribute(node, "defer")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .is_none();
    Ok((
        node,
        dom.attribute(node, "src")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .map(str::to_owned),
        script_type,
        dom.arena()
            .text_content(node)
            .map_err(|error| HostError::Execution(error.to_string()))?,
        parser_blocking,
        dom.attribute(node, "nomodule")
            .map_err(|error| HostError::Execution(error.to_string()))?
            .is_some(),
        deferred,
    ))
}

fn resolve_script_record(mut record: ScriptRecord, base_url: Option<&Url>) -> ScriptRecord {
    if let (Some(base_url), Some(source)) = (base_url, record.1.as_deref())
        && let Ok(absolute) = base_url.join(source)
    {
        record.1 = Some(absolute.to_string());
    }
    record
}

fn run_script_record(
    host: &mut RawV8Host,
    live_dom: &SharedDomRuntime,
    resources: &mut BTreeMap<String, String>,
    record: &ScriptRecord,
    diagnostics: &mut Vec<oryn_common::v3::CapabilityDiagnostic>,
    #[cfg(unix)] context: Option<&(
        crate::context_service::SharedContextClient,
        oryn_common::v3::PageId,
        Option<oryn_common::v3::PageId>,
    )>,
) -> Result<(), HostError> {
    let (script, src, script_type, inline, _parser_blocking, no_module, _) = record;
    let import_map = script_type.as_deref() == Some("importmap");
    if *no_module && !import_map {
        return Ok(());
    }
    if script_type.as_deref().is_some_and(|kind| {
        !matches!(
            kind,
            "" | "module" | "importmap" | "text/javascript" | "application/javascript"
        )
    }) {
        return Ok(());
    }
    if import_map && src.is_some() {
        diagnostics.push(script_diagnostic(HostError::Execution(
            "external import maps are not supported by HTML".into(),
        )));
        return Ok(());
    }
    #[cfg(unix)]
    let parent_authorizes = context.is_some();
    #[cfg(not(unix))]
    let parent_authorizes = false;
    if !parent_authorizes && let Some(serialized) = resources.get("oryn:csp-policy") {
        let policy: crate::csp::CspPolicySet = serde_json::from_str(serialized)
            .map_err(|error| HostError::Execution(format!("invalid CSP snapshot: {error}")))?;
        let request = crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Script,
            url: src.as_deref().unwrap_or("inline").to_owned(),
            nonce: live_dom
                .borrow()
                .attribute(*script, "nonce")
                .ok()
                .flatten()
                .map(str::to_owned),
            inline_source: src.is_none().then(|| inline.clone()),
            parser_inserted: true,
            initiated_by_trusted_script: false,
        };
        let decision = policy.authorize(&request);
        let sandbox_blocks = policy
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_scripts);
        if !decision.allowed || sandbox_blocks {
            diagnostics.push(script_diagnostic(HostError::Execution(
                "parser script blocked by Content-Security-Policy".into(),
            )));
            return Ok(());
        }
    }
    #[cfg(unix)]
    if let Some((client, page_id, _)) = context {
        let nonce = live_dom
            .borrow()
            .attribute(*script, "nonce")
            .ok()
            .flatten()
            .map(str::to_owned);
        let authorization = if let Some(src) = src
            && resource_entry(resources, src).is_none()
        {
            let initiator_url = host
                .evaluate_string("globalThis.__oryn_request_initiator_url")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| host.evaluate_string("location.href").ok());
            let referrer_policy = live_dom
                .borrow()
                .attribute(*script, "referrerpolicy")
                .ok()
                .flatten()
                .map(str::to_owned)
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    host.evaluate_string("globalThis.__oryn_referrer_policy")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                });
            client.request(crate::context_service::ContextServiceRequest::LoadScript {
                page_id: *page_id,
                url: src.clone(),
                module: script_type.as_deref() == Some("module"),
                nonce,
                initiator_url,
                referrer_policy,
            })
        } else if src.is_none() {
            client.request(
                crate::context_service::ContextServiceRequest::AuthorizeInlineScript {
                    page_id: *page_id,
                    source: inline.clone(),
                    nonce,
                },
            )
        } else {
            Ok(crate::context_service::ContextServiceResponse::ScriptAuthorized)
        };
        match authorization {
            Ok(crate::context_service::ContextServiceResponse::ScriptSource {
                url,
                source,
                ..
            }) => {
                if let Some(src) = src {
                    resources.insert(src.clone(), source.clone());
                    resources.insert(format!("oryn:module-alias:{src}"), url.clone());
                }
                resources.insert(url, source);
            }
            Ok(crate::context_service::ContextServiceResponse::ScriptAuthorized) => {}
            Ok(_) => {
                return Err(HostError::Execution(
                    "parent returned an invalid parser-script response".into(),
                ));
            }
            Err(error) => {
                diagnostics.push(script_diagnostic(HostError::Execution(error.to_string())));
                return Ok(());
            }
        }
    }
    if import_map {
        if let Err(error) = host.register_import_map(inline) {
            diagnostics.push(script_diagnostic(error));
        }
        return Ok(());
    }
    host.set_current_script((script_type.as_deref() != Some("module")).then_some(*script))?;
    if let Some(src) = src
        && resource_entry(resources, src).is_none()
    {
        host.set_current_script(None)?;
        diagnostics.push(script_diagnostic_at(
            HostError::Execution("external script source is unavailable".into()),
            src,
        ));
        return Ok(());
    }
    let result = if script_type.as_deref() == Some("module") {
        let resource = src.as_ref().and_then(|src| resource_entry(resources, src));
        let name = match resource {
            Some((name, _)) => name.to_owned(),
            None => host.evaluate_string("location.href")?,
        };
        let source = resource
            .map(|(_, source)| source.to_owned())
            .unwrap_or_else(|| inline.clone());
        #[cfg(unix)]
        if let Some((client, page_id, _)) = context {
            preload_module_graph(host, client, *page_id, &name, &source, resources)?;
        }
        host.execute_module_script(&name, &source, resources, src.is_some())
            .map_err(|error| script_diagnostic_at(error, &name))
    } else {
        let source = src
            .as_ref()
            .and_then(|src| resource_entry(resources, src))
            .map(|(_, source)| source)
            .unwrap_or(inline);
        let document_url = host.evaluate_string("location.href")?;
        let resource_name = match src.as_ref().and_then(|src| resource_entry(resources, src)) {
            Some((name, _)) => name.to_owned(),
            None => document_url.clone(),
        };
        let resource_name = Url::parse(&document_url)
            .ok()
            .and_then(|base| base.join(&resource_name).ok())
            .map(|url| url.to_string())
            .unwrap_or(resource_name);
        // The current resource table does not carry CORS response tainting. Until it does,
        // cross-origin resources must conservatively withhold exception details.
        let muted_errors = src.is_some()
            && Url::parse(&resource_name)
                .ok()
                .zip(Url::parse(&document_url).ok())
                .is_none_or(|(script, document)| script.origin() != document.origin());
        host.execute_parser_script(source, &resource_name, muted_errors)
            .map_err(|error| script_diagnostic_at(error, &resource_name))
    };
    host.set_current_script(None)?;
    if let Err(diagnostic) = result {
        diagnostics.push(diagnostic);
    }
    Ok(())
}

fn parser_checkpoint(
    host: &mut RawV8Host,
    diagnostics: &mut Vec<oryn_common::v3::CapabilityDiagnostic>,
) -> Result<(), HostError> {
    host.run_microtask_checkpoint()?;
    // The platform host receives Fetch/XHR/IndexedDB submissions immediately,
    // but their completions are page tasks, not parser microtasks. Polling
    // completions here can resume an `await` before html5ever has consumed the
    // markup after this parser-blocking script (and, for example, before
    // `document.body` exists). Completion tasks are drained only after parsing
    // resumes or at the document load barrier.
    host.run_custom_element_reactions()?;
    host.run_dom_observer_checkpoint()?;
    for report in host.take_module_error_reports() {
        diagnostics.push(script_diagnostic(HostError::Execution(report)));
    }
    for rejection in host.take_unhandled_promise_rejections() {
        diagnostics.push(script_diagnostic(HostError::Execution(format!(
            "unhandled promise rejection: {rejection}"
        ))));
    }
    Ok(())
}

enum DocumentLoad {
    Live(crate::host::DocumentStream),
    Snapshot(Box<ParsedDocument>),
}

type ScriptRecord = (
    NodeId,
    Option<String>,
    Option<String>,
    String,
    bool,
    bool,
    bool,
);

#[derive(Clone)]
struct LoadedFrameResource {
    url: String,
    source: String,
    encoding: String,
    content_type: Option<String>,
}

#[cfg(unix)]
enum ParserResourceCompletion {
    Script {
        order: u64,
        record: ScriptRecord,
        asynchronous: bool,
        result: Result<(String, String), String>,
    },
    Stylesheet {
        node: NodeId,
        result: Result<(String, String), String>,
    },
    Frame {
        requested_url: String,
        result: Result<LoadedFrameResource, String>,
    },
}

#[cfg(unix)]
struct ParserResourceScheduler {
    sender: std::sync::mpsc::Sender<ParserResourceCompletion>,
    receiver: std::sync::mpsc::Receiver<ParserResourceCompletion>,
    pending: usize,
    next_order: u64,
}

#[cfg(unix)]
impl ParserResourceScheduler {
    fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            sender,
            receiver,
            pending: 0,
            next_order: 0,
        }
    }

    fn next_order(&mut self) -> u64 {
        let order = self.next_order;
        self.next_order = order.saturating_add(1);
        order
    }

    fn schedule_script(
        &mut self,
        client: crate::context_service::SharedContextClient,
        page_id: oryn_common::v3::PageId,
        order: u64,
        record: ScriptRecord,
        asynchronous: bool,
        nonce: Option<String>,
    ) {
        let sender = self.sender.clone();
        let requested_url = record.1.clone().unwrap_or_default();
        let module = record.2.as_deref() == Some("module");
        self.pending = self.pending.saturating_add(1);
        std::thread::spawn(move || {
            let result = client
                .request(crate::context_service::ContextServiceRequest::LoadScript {
                    page_id,
                    url: requested_url,
                    module,
                    nonce,
                    initiator_url: None,
                    referrer_policy: None,
                })
                .map_err(|error| error.to_string())
                .and_then(|response| match response {
                    crate::context_service::ContextServiceResponse::ScriptSource {
                        url,
                        source,
                        ..
                    } => Ok((url, source)),
                    _ => Err("parent returned an invalid scheduled-script response".into()),
                });
            let _ = sender.send(ParserResourceCompletion::Script {
                order,
                record,
                asynchronous,
                result,
            });
        });
    }

    fn schedule_stylesheet(
        &mut self,
        client: crate::context_service::SharedContextClient,
        page_id: oryn_common::v3::PageId,
        node: NodeId,
        requested_url: String,
    ) {
        let sender = self.sender.clone();
        self.pending = self.pending.saturating_add(1);
        std::thread::spawn(move || {
            let result = load_parser_stylesheet(&client, page_id, &requested_url, None, None, 0)
                .map_err(|error| error.to_string());
            let _ = sender.send(ParserResourceCompletion::Stylesheet { node, result });
        });
    }

    fn schedule_frame(
        &mut self,
        client: crate::context_service::SharedContextClient,
        page_id: oryn_common::v3::PageId,
        requested_url: String,
    ) {
        let sender = self.sender.clone();
        self.pending = self.pending.saturating_add(1);
        std::thread::spawn(move || {
            let request_url = requested_url.clone();
            let result = client
                .request(
                    crate::context_service::ContextServiceRequest::LoadPageResource {
                        page_id,
                        url: request_url,
                        destination: "frame".into(),
                        initiator_url: None,
                        referrer_policy: None,
                        post: None,
                    },
                )
                .map_err(|error| error.to_string())
                .and_then(|response| match response {
                    crate::context_service::ContextServiceResponse::PageResource {
                        url,
                        source,
                        destination,
                        content_type,
                        encoding,
                        ..
                    } if destination == "frame" => Ok(LoadedFrameResource {
                        url,
                        source,
                        content_type,
                        encoding,
                    }),
                    _ => Err("parent returned an invalid frame response".into()),
                });
            let _ = sender.send(ParserResourceCompletion::Frame {
                requested_url,
                result,
            });
        });
    }

    fn try_next(&mut self) -> Option<ParserResourceCompletion> {
        self.receiver.try_recv().ok().inspect(|_| {
            self.pending = self.pending.saturating_sub(1);
        })
    }

    fn wait_next(&mut self) -> Result<ParserResourceCompletion, HostError> {
        let completion = self
            .receiver
            .recv()
            .map_err(|_| HostError::Execution("parser resource scheduler disconnected".into()))?;
        self.pending = self.pending.saturating_sub(1);
        Ok(completion)
    }
}

#[cfg(unix)]
fn load_parser_stylesheet(
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    requested_url: &str,
    initiator_url: Option<String>,
    referrer_policy: Option<String>,
    depth: usize,
) -> Result<(String, String), HostError> {
    if depth > 8 {
        return Err(HostError::LimitExceeded {
            resource: "stylesheet import depth".into(),
        });
    }
    let response = client
        .request(
            crate::context_service::ContextServiceRequest::LoadPageResource {
                page_id,
                url: requested_url.into(),
                destination: "stylesheet".into(),
                initiator_url,
                referrer_policy,
                post: None,
            },
        )
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let crate::context_service::ContextServiceResponse::PageResource {
        url,
        source,
        destination,
        referrer_policy,
        ..
    } = response
    else {
        return Err(HostError::Execution(
            "parent returned an invalid stylesheet response".into(),
        ));
    };
    if destination != "stylesheet" {
        return Err(HostError::Execution(
            "parent returned a mismatched parser resource".into(),
        ));
    }
    let import =
        regex::Regex::new(r#"(?is)@import\s+(?:url\(\s*)?[\"']?([^\"'\)\s;]+)[\"']?\s*\)?[^;]*;"#)
            .expect("stylesheet import regex");
    let base = Url::parse(&url).ok();
    let mut expanded = String::new();
    for capture in import.captures_iter(&source) {
        let child = base
            .as_ref()
            .and_then(|base| base.join(&capture[1]).ok())
            .map_or_else(|| capture[1].to_string(), |url| url.to_string());
        let (_, imported) = load_parser_stylesheet(
            client,
            page_id,
            &child,
            Some(url.clone()),
            Some(referrer_policy.clone()),
            depth + 1,
        )?;
        expanded.push_str(&imported);
    }
    let remaining = import.replace_all(&source, "");
    load_css_subresources(client, page_id, &url, &referrer_policy, remaining.as_ref())?;
    expanded.push_str(&remaining);
    Ok((url, expanded))
}

#[cfg(unix)]
fn load_css_subresources(
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    stylesheet_url: &str,
    referrer_policy: &str,
    css: &str,
) -> Result<(), HostError> {
    let urls = regex::Regex::new(r#"(?is)url\(\s*[\"']?([^\"')\s]+)[\"']?\s*\)"#)
        .expect("CSS resource URL regex");
    let destination = if css.to_ascii_lowercase().contains("@font-face") {
        "font"
    } else {
        "image"
    };
    let base = Url::parse(stylesheet_url).ok();
    for capture in urls.captures_iter(css) {
        let requested = capture[1].trim();
        if requested.is_empty() || requested.starts_with('#') {
            continue;
        }
        let requested = base
            .as_ref()
            .and_then(|base| base.join(requested).ok())
            .map_or_else(|| requested.to_owned(), |url| url.to_string());
        let response = client
            .request(
                crate::context_service::ContextServiceRequest::LoadPageResource {
                    page_id,
                    url: requested,
                    destination: destination.into(),
                    initiator_url: Some(stylesheet_url.into()),
                    referrer_policy: Some(referrer_policy.into()),
                    post: None,
                },
            )
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if !matches!(
            response,
            crate::context_service::ContextServiceResponse::PageResource { .. }
        ) {
            return Err(HostError::Execution(
                "parent returned an invalid CSS subresource response".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn load_inline_css_resources(
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    document_url: &str,
    referrer_policy: &str,
    css: &str,
) -> Result<(), HostError> {
    let imports =
        regex::Regex::new(r#"(?is)@import\s+(?:url\(\s*)?[\"']?([^\"'\)\s;]+)[\"']?\s*\)?[^;]*;"#)
            .expect("inline stylesheet import regex");
    let base = Url::parse(document_url).ok();
    for capture in imports.captures_iter(css) {
        let child = base
            .as_ref()
            .and_then(|base| base.join(&capture[1]).ok())
            .map_or_else(|| capture[1].to_string(), |url| url.to_string());
        load_parser_stylesheet(
            client,
            page_id,
            &child,
            Some(document_url.into()),
            Some(referrer_policy.into()),
            0,
        )?;
    }
    let remaining = imports.replace_all(css, "");
    load_css_subresources(
        client,
        page_id,
        document_url,
        referrer_policy,
        remaining.as_ref(),
    )
}

#[cfg(unix)]
fn load_frame_style_resources(
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    document_url: &str,
    referrer_policy: &str,
    source: &str,
    parsed: &ParsedDocument,
) -> Result<(), HostError> {
    for (_, node) in parsed.dom.iter() {
        let NodeKind::Element { name } = &node.kind else {
            continue;
        };
        if name.local.as_ref() == "style"
            && matches!(
                name.ns.as_ref(),
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            )
        {
            let css = node
                .children
                .iter()
                .filter_map(|child| parsed.dom.text_content(*child).ok())
                .collect::<String>();
            load_inline_css_resources(client, page_id, document_url, referrer_policy, &css)?;
        } else if name.local.as_ref() == "link"
            && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            && node.attributes.get_null("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            })
            && let Some(href) = node.attributes.get_null("href")
        {
            load_parser_stylesheet(
                client,
                page_id,
                href,
                Some(document_url.into()),
                Some(referrer_policy.into()),
                0,
            )?;
        }
    }
    // Attribute values commonly contain a query string. Stopping the PI body
    // at the first `?` drops exactly those browser resource URLs; terminate
    // only at the actual `?>` delimiter.
    let processing_instructions = regex::Regex::new(r#"(?is)<\?xml-stylesheet\s+(.+?)\?>"#)
        .expect("XML stylesheet processing-instruction regex");
    let href = regex::Regex::new(r#"(?i)href\s*=\s*[\"']([^\"']+)[\"']"#)
        .expect("XML stylesheet href regex");
    for instruction in processing_instructions.captures_iter(source) {
        let Some(href) = href.captures(&instruction[1]) else {
            continue;
        };
        let requested = href[1]
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&apos;", "'");
        load_parser_stylesheet(
            client,
            page_id,
            &requested,
            Some(document_url.into()),
            Some(referrer_policy.into()),
            0,
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn apply_parser_resource_completion(
    completion: ParserResourceCompletion,
    live_dom: &SharedDomRuntime,
    host: &mut RawV8Host,
    resources: &mut BTreeMap<String, String>,
    deferred_scripts: &mut BTreeMap<u64, ScriptRecord>,
    diagnostics: &mut Vec<oryn_common::v3::CapabilityDiagnostic>,
    context: Option<&(
        crate::context_service::SharedContextClient,
        oryn_common::v3::PageId,
        Option<oryn_common::v3::PageId>,
    )>,
) -> Result<Option<(String, LoadedFrameResource)>, HostError> {
    match completion {
        ParserResourceCompletion::Script {
            order,
            record,
            asynchronous,
            result,
        } => match result {
            Ok((url, source)) => {
                if let Some(requested) = &record.1 {
                    resources.insert(requested.clone(), source.clone());
                    resources.insert(format!("oryn:module-alias:{requested}"), url.clone());
                }
                resources.insert(url, source);
                if asynchronous {
                    run_script_record(host, live_dom, resources, &record, diagnostics, context)?;
                    parser_checkpoint(host, diagnostics)?;
                } else {
                    deferred_scripts.insert(order, record);
                }
            }
            Err(error) => diagnostics.push(script_diagnostic(HostError::Execution(format!(
                "scheduled script failed: {error}"
            )))),
        },
        ParserResourceCompletion::Stylesheet { node, result } => match result {
            Ok((url, css)) => {
                resources.insert(url, css.clone());
                let mut dom = live_dom.borrow_mut();
                let style = dom.create(NodeKind::Element {
                    name: QualName::new(None, ns!(html), local_name!("style")),
                });
                let text = dom.create_text(css);
                dom.append_child(style, text)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let Some(parent) = dom
                    .parent(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                {
                    dom.append_child(parent, style)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
            }
            Err(error) => diagnostics.push(script_diagnostic(HostError::Execution(format!(
                "scheduled stylesheet failed: {error}"
            )))),
        },
        ParserResourceCompletion::Frame {
            requested_url,
            result,
        } => {
            if let Ok(frame) = result {
                return Ok(Some((requested_url, frame)));
            }
            // Cross-origin, policy-denied, and failed frames remain opaque and
            // publish their capability diagnostics during realm construction.
        }
    }
    Ok(None)
}

fn set_parsed_encoding(parsed: &mut ParsedDocument, encoding: &str) -> Result<(), HostError> {
    if let Some(metadata) = parsed
        .dom
        .get_mut(parsed.document)
        .map_err(|error| HostError::Execution(error.to_string()))?
        .document_metadata
        .as_mut()
    {
        metadata.encoding = encoding.into();
    }
    Ok(())
}

struct LiveParseResult {
    document: ParsedDocument,
    scripts: Vec<ScriptRecord>,
    frames: BTreeMap<String, LoadedFrameResource>,
}

fn parse_live_document(
    input_stream: crate::host::DocumentStream,
    live_dom: &SharedDomRuntime,
    host: &mut RawV8Host,
    resources: &mut BTreeMap<String, String>,
    diagnostics: &mut Vec<oryn_common::v3::CapabilityDiagnostic>,
    #[cfg(unix)] context: Option<&(
        crate::context_service::SharedContextClient,
        oryn_common::v3::PageId,
        Option<oryn_common::v3::PageId>,
    )>,
) -> Result<LiveParseResult, HostError> {
    use std::collections::VecDeque;

    let mut loaded_frames = BTreeMap::new();
    let encoding = input_stream.encoding;
    let mut source = input_stream.chunks;
    let essence = input_stream
        .content_type
        .as_deref()
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    let mut parser = if oryn_dom::xml::is_text_mime(&essence) {
        html::StreamingParser::with_runtime_text(live_dom.clone(), &essence)
    } else {
        html::StreamingParser::with_runtime(live_dom.clone())
    };
    let mut input = VecDeque::new();
    #[cfg(unix)]
    let mut resource_scheduler = ParserResourceScheduler::new();
    #[cfg(unix)]
    let mut deferred_scripts = BTreeMap::new();
    #[cfg(not(unix))]
    let mut ordered_scripts = Vec::new();
    let mut all_events = Vec::new();
    #[cfg(unix)]
    let inline_import =
        regex::Regex::new(r#"(?is)@import\s+(?:url\(\s*)?[\"']?([^\"'\)\s;]+)[\"']?\s*\)?[^;]*;"#)
            .expect("stylesheet import regex");
    host.set_parser_input_active(true)?;

    loop {
        let next = match input.pop_front() {
            Some(chunk) => Some(chunk),
            None => source.next().transpose()?,
        };
        if let Some(encoding) = *encoding.lock().expect("navigation encoding") {
            let mut dom = live_dom.borrow_mut();
            let document = dom.document();
            if let Some(metadata) = dom
                .arena_mut()
                .get_mut(document)
                .map_err(|error| HostError::Execution(error.to_string()))?
                .document_metadata
                .as_mut()
            {
                metadata.encoding = encoding.into();
            }
        }
        let Some(mut chunk) = next else { break };
        if chunk.is_empty() {
            continue;
        }
        let progress = parser.push_until_script(&chunk);
        let suffix = chunk.split_off(progress.consumed_bytes);
        for event in parser.take_events() {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = context {
                match &event {
                    html::ParserEvent::StylesheetDiscovered { node, href } => {
                        resource_scheduler.schedule_stylesheet(
                            client.clone(),
                            *page_id,
                            *node,
                            href.clone(),
                        );
                    }
                    html::ParserEvent::InlineStyleReady { node } => {
                        let css = live_dom
                            .borrow()
                            .arena()
                            .text_content(*node)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        for capture in inline_import.captures_iter(&css) {
                            resource_scheduler.schedule_stylesheet(
                                client.clone(),
                                *page_id,
                                *node,
                                capture[1].to_string(),
                            );
                        }
                    }
                    html::ParserEvent::FrameDiscovered { source, .. } => {
                        resource_scheduler.schedule_frame(client.clone(), *page_id, source.clone());
                    }
                    _ => {}
                }
                let response = match &event {
                    html::ParserEvent::MetaSecurity { content } => Some(client.request(
                        crate::context_service::ContextServiceRequest::InstallMetaSecurity {
                            page_id: *page_id,
                            serialized: content.clone(),
                        },
                    )),
                    html::ParserEvent::MetaReferrer { content } => Some(client.request(
                        crate::context_service::ContextServiceRequest::InstallReferrerPolicy {
                            page_id: *page_id,
                            serialized: content.clone(),
                        },
                    )),
                    _ => None,
                };
                if let html::ParserEvent::MetaReferrer { content } = &event {
                    let policy = serde_json::to_string(content)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    host.execute(&format!("globalThis.__oryn_referrer_policy={policy}"))?;
                }
                if let Some(response) = response {
                    match response {
                        Ok(crate::context_service::ContextServiceResponse::DocumentSecurityInstalled) => {}
                        Ok(_) => {
                            return Err(HostError::Execution(
                                "parent returned an invalid parser policy response".into(),
                            ));
                        }
                        Err(error) => {
                            return Err(HostError::Execution(format!(
                                "parent rejected parser policy: {error}"
                            )));
                        }
                    }
                }
            }
            if let html::ParserEvent::FrameDiscovered { node, .. } = &event {
                // Browsers expose a stable WindowProxy as soon as the iframe
                // element is inserted. The child navigation may still be in
                // flight, but following parser-blocking scripts can already
                // retain the proxy and post messages through it.
                host.ensure_provisional_frame_window(*node)?;
            }
            if let html::ParserEvent::ScriptReady { node, scheduling } = &event {
                host.run_microtask_checkpoint()?;
                let record = script_record(&live_dom.borrow(), *node)?;
                if record.2.as_deref() == Some("importmap") {
                    run_script_record(
                        host,
                        live_dom,
                        resources,
                        &record,
                        diagnostics,
                        #[cfg(unix)]
                        context,
                    )?;
                }
                let asynchronous = live_dom
                    .borrow()
                    .attribute(*node, "async")
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .is_some();
                if asynchronous && *scheduling != html::ScriptScheduling::Data {
                    #[cfg(unix)]
                    if let (Some((client, page_id, _)), Some(_)) = (context, record.1.as_ref()) {
                        let nonce = live_dom
                            .borrow()
                            .attribute(*node, "nonce")
                            .ok()
                            .flatten()
                            .map(str::to_owned);
                        let order = resource_scheduler.next_order();
                        resource_scheduler.schedule_script(
                            client.clone(),
                            *page_id,
                            order,
                            record,
                            true,
                            nonce,
                        );
                    } else {
                        run_script_record(
                            host,
                            live_dom,
                            resources,
                            &record,
                            diagnostics,
                            context,
                        )?;
                        parser_checkpoint(host, diagnostics)?;
                    }
                    #[cfg(not(unix))]
                    {
                        run_script_record(host, live_dom, resources, &record, diagnostics)?;
                        parser_checkpoint(host, diagnostics)?;
                    }
                } else if matches!(
                    scheduling,
                    html::ScriptScheduling::Defer | html::ScriptScheduling::Module
                ) {
                    #[cfg(unix)]
                    {
                        let order = resource_scheduler.next_order();
                        if let (Some((client, page_id, _)), Some(_)) = (context, record.1.as_ref())
                        {
                            let nonce = live_dom
                                .borrow()
                                .attribute(*node, "nonce")
                                .ok()
                                .flatten()
                                .map(str::to_owned);
                            resource_scheduler.schedule_script(
                                client.clone(),
                                *page_id,
                                order,
                                record,
                                false,
                                nonce,
                            );
                        } else {
                            deferred_scripts.insert(order, record);
                        }
                    }
                    #[cfg(not(unix))]
                    ordered_scripts.push(record);
                }
            }
            all_events.push(event);
        }

        #[cfg(unix)]
        while let Some(completion) = resource_scheduler.try_next() {
            if let Some((requested, frame)) = apply_parser_resource_completion(
                completion,
                live_dom,
                host,
                resources,
                &mut deferred_scripts,
                diagnostics,
                context,
            )? {
                loaded_frames.insert(requested, frame);
            }
        }

        if let Some(event @ html::ParserEvent::ScriptReady { node, .. }) = progress.blocked_on {
            // HTML's script end-tag algorithm checkpoints before preparing the
            // script, independently of Window named-property publication.
            host.run_microtask_checkpoint()?;
            all_events.push(event);
            let record = script_record(&live_dom.borrow(), node)?;
            run_script_record(
                host,
                live_dom,
                resources,
                &record,
                diagnostics,
                #[cfg(unix)]
                context,
            )?;
            host.refresh_named_window_properties()?;
            parser_checkpoint(host, diagnostics)?;

            let commands = host.take_parser_input_commands()?;
            let mut inserted = String::new();
            let mut replace_parser = false;
            for command in commands {
                match command {
                    ParserInputCommand::Open => {
                        live_dom
                            .borrow_mut()
                            .clear_document()
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        inserted.clear();
                        input.clear();
                        replace_parser = true;
                    }
                    ParserInputCommand::Write(markup) => inserted.push_str(&markup),
                    ParserInputCommand::Close => {
                        // `close()` seals the replacement stream after all
                        // writes queued before it have reached html5ever.
                    }
                }
            }
            if replace_parser {
                parser = html::StreamingParser::with_runtime(live_dom.clone());
            } else if !suffix.is_empty() {
                input.push_front(suffix);
            }
            if !inserted.is_empty() {
                input.push_front(inserted);
            }
        } else if !suffix.is_empty() {
            input.push_front(suffix);
        }
    }

    #[cfg(unix)]
    while resource_scheduler.pending > 0 {
        let completion = resource_scheduler.wait_next()?;
        if let Some((requested, frame)) = apply_parser_resource_completion(
            completion,
            live_dom,
            host,
            resources,
            &mut deferred_scripts,
            diagnostics,
            context,
        )? {
            loaded_frames.insert(requested, frame);
        }
    }

    host.set_parser_input_active(false)?;
    let mut parsed = parser.finish();
    all_events.extend(std::mem::take(&mut parsed.events));
    parsed.events = all_events;
    #[cfg(unix)]
    let ordered_scripts = deferred_scripts.into_values().collect();
    Ok(LiveParseResult {
        document: parsed,
        scripts: ordered_scripts,
        frames: loaded_frames,
    })
}

const RUST_RUNTIME_GLOBALS: &str = r#"
globalThis.global=globalThis;
// A top-level Window is its own parent and top. WPT's harness determines the
// browsing-context root before any test code runs, so leaving these absent
// prevents every generated-binding assertion from reaching the interface.
globalThis.parent=globalThis;
if(globalThis.Document&&!globalThis.XMLDocument)globalThis.XMLDocument=globalThis.Document;
globalThis.focus=()=>{if(typeof globalThis.__oryn_context_call==='function'&&__oryn_page_id!==null)contextCall({operation:'activate_page',source_page_id:__oryn_page_id,target_page_id:__oryn_page_id})};
globalThis.blur=()=>{};
if(globalThis.CSSStyleDeclaration&&!CSSStyleDeclaration.prototype[Symbol.iterator])Object.defineProperty(CSSStyleDeclaration.prototype,Symbol.iterator,{configurable:true,writable:true,value:function*(){for(let i=0;i<this.length;i++)yield this.item(i)}});
if(!globalThis.CSS)globalThis.CSS={};
if(!CSS.escape)CSS.escape=value=>{const string=String(value),length=string.length;let result='';for(let index=0;index<length;index++){const code=string.charCodeAt(index),character=string.charAt(index);if(code===0){result+='\uFFFD';continue}if((code>=1&&code<=31)||code===127||(index===0&&code>=48&&code<=57)||(index===1&&code>=48&&code<=57&&string.charCodeAt(0)===45)){result+='\\'+code.toString(16)+' ';continue}if(index===0&&code===45&&length===1){result+='\\-';continue}if(code>=128||code===45||code===95||(code>=48&&code<=57)||(code>=65&&code<=90)||(code>=97&&code<=122)){result+=character;continue}result+='\\'+character}return result};
if(!CSS.supports)CSS.supports=(property,value)=>{if(value===undefined){const source=String(property).trim(),selector=source.match(/^selector\(([\s\S]*)\)$/);if(selector){try{document.querySelector(selector[1]);return true}catch(_){return false}}const match=source.match(/^\(\s*([\w-]+)\s*:\s*([^)]*)\)$/);return !!match&&CSS.supports(match[1],match[2])}const probe=document.createElement('div').style;try{probe.setProperty(String(property),String(value));return probe.getPropertyValue(String(property))!==''}catch(_){return false}};
// Generated Rust callbacks are the production web-platform authority. The
// compatibility bootstrap below still supplies unrelated utilities, but its
// legacy platform classes are restored out of the global namespace at the end
// of initialization and remain available only to the explicit probe build.
const __oryn_generated_platform={};
for(const name of ['Blob','File','FormData','Headers','AbortSignal','AbortController','ReadableStream','ReadableStreamDefaultReader','ReadableStreamDefaultController','WritableStream','WritableStreamDefaultWriter','WritableStreamDefaultController','TransformStream','Request','Response','XMLHttpRequest','WebSocket','IDBFactory','IDBRequest','IDBOpenDBRequest','IDBDatabase','IDBTransaction','IDBKeyRange','IDBCursor','IDBIndex','IDBObjectStore','fetch','indexedDB'])__oryn_generated_platform[name]=globalThis[name];
// DOM algorithms surface named exceptions. Keep their JavaScript identity and
// legacy numeric codes browser-compatible while the throwing algorithms stay
// in generated Rust callbacks.
globalThis.getSelection=()=>document.getSelection();
const __oryn_time_origin=__oryn_performance_time_origin;
const __oryn_now=__oryn_performance_now;
globalThis.performance={timeOrigin:__oryn_time_origin,now:()=>__oryn_now(),toJSON(){return {timeOrigin:this.timeOrigin}}};
globalThis.navigator=Object.freeze({userAgent:'Oryn/0.1',appVersion:'Oryn/0.1',platform:'MacIntel',language:'en-US',languages:Object.freeze(['en-US','en']),onLine:true,cookieEnabled:true,hardwareConcurrency:1,maxTouchPoints:0});
globalThis.queueMicrotask=callback=>Promise.resolve().then(callback);
globalThis.__oryn_console=[];
globalThis.console={
  log:(...values)=>__oryn_console.push({level:'log',message:values.map(String).join(' ')}),
  info:(...values)=>__oryn_console.push({level:'info',message:values.map(String).join(' ')}),
  warn:(...values)=>__oryn_console.push({level:'warn',message:values.map(String).join(' ')}),
  error:(...values)=>__oryn_console.push({level:'error',message:values.map(String).join(' ')})
};
globalThis.__oryn_take_console=()=>__oryn_console.splice(0);
globalThis.alert=value=>{__oryn_console.push({level:'info',message:'alert: '+String(value)})};
globalThis.confirm=value=>{__oryn_console.push({level:'info',message:'confirm dismissed: '+String(value)});return false};
globalThis.prompt=value=>{__oryn_console.push({level:'info',message:'prompt dismissed: '+String(value)});return null};
globalThis.__oryn_context_trace=[];
globalThis.__oryn_take_context_trace=()=>__oryn_context_trace.splice(0);
globalThis.__oryn_animation_frames=new Map();globalThis.__oryn_next_animation_frame=1;
globalThis.requestAnimationFrame=callback=>{const id=__oryn_next_animation_frame++;__oryn_animation_frames.set(id,callback);return id};
globalThis.cancelAnimationFrame=id=>__oryn_animation_frames.delete(Number(id));
globalThis.__oryn_run_animation_frames=()=>{const callbacks=[...__oryn_animation_frames.values()];__oryn_animation_frames.clear();const timestamp=Date.now();for(const callback of callbacks)callback(timestamp)};
globalThis.__oryn_resize_observers=new Set();globalThis.__oryn_intersection_observers=new Set();
class OrynResizeObserver{constructor(callback){if(typeof callback!=='function')throw new TypeError('callback must be callable');this.callback=callback;this.targets=new Map();__oryn_resize_observers.add(this)}observe(target){this.targets.set(target,null)}unobserve(target){this.targets.delete(target)}disconnect(){this.targets.clear();__oryn_resize_observers.delete(this)}}
class OrynIntersectionObserver{constructor(callback,options={}){if(typeof callback!=='function')throw new TypeError('callback must be callable');this.callback=callback;this.root=options.root||null;this.rootMargin=String(options.rootMargin||'0px');this.thresholds=Array.isArray(options.threshold)?options.threshold.map(Number):[Number(options.threshold||0)];this.targets=new Map();__oryn_intersection_observers.add(this)}observe(target){this.targets.set(target,null)}unobserve(target){this.targets.delete(target)}disconnect(){this.targets.clear();__oryn_intersection_observers.delete(this)}takeRecords(){return []}}
globalThis.ResizeObserver=OrynResizeObserver;globalThis.IntersectionObserver=OrynIntersectionObserver;
const __oryn_rect_key=rect=>[rect.x,rect.y,rect.width,rect.height].join(':');
globalThis.__oryn_run_geometry_observers=()=>{for(const observer of __oryn_resize_observers){const entries=[];for(const target of observer.targets.keys())try{const rect=target.getBoundingClientRect(),key=__oryn_rect_key(rect);if(observer.targets.get(target)!==key){observer.targets.set(target,key);entries.push({target,contentRect:rect,borderBoxSize:[{inlineSize:rect.width,blockSize:rect.height}],contentBoxSize:[{inlineSize:rect.width,blockSize:rect.height}]})}}catch(_){}if(entries.length)observer.callback(entries,observer)}for(const observer of __oryn_intersection_observers){const entries=[];for(const target of observer.targets.keys())try{const rect=target.getBoundingClientRect(),viewport={x:0,y:0,width:innerWidth||0,height:innerHeight||0},width=Math.max(0,Math.min(rect.right,viewport.width)-Math.max(rect.left,0)),height=Math.max(0,Math.min(rect.bottom,viewport.height)-Math.max(rect.top,0)),area=rect.width*rect.height,ratio=area>0?(width*height)/area:0,key=__oryn_rect_key(rect)+':'+ratio;if(observer.targets.get(target)!==key){observer.targets.set(target,key);entries.push({target,boundingClientRect:rect,intersectionRect:{x:Math.max(rect.x,0),y:Math.max(rect.y,0),width,height},rootBounds:viewport,isIntersecting:ratio>0,intersectionRatio:ratio,time:Date.now()})}}catch(_){}if(entries.length)observer.callback(entries,observer)}};
Object.defineProperty(globalThis,'origin',{configurable:true,enumerable:true,get(){return globalThis.__oryn_security_origin??location.origin}});
globalThis.__oryn_layout={styles:{},geometry:{},viewport:{width:1280,height:720,device_pixel_ratio:1},scroll:{},media:{}};
globalThis.innerWidth=1280;globalThis.innerHeight=720;
globalThis.__oryn_install_layout=value=>{globalThis.__oryn_layout=value;globalThis.innerWidth=Number(value.viewport?.width||0);globalThis.innerHeight=Number(value.viewport?.height||0)};
globalThis.scrollX=globalThis.pageXOffset=0;globalThis.scrollY=globalThis.pageYOffset=0;
globalThis.scrollTo=(x=0,y=0)=>{if(x&&typeof x==='object'){y=x.top??scrollY;x=x.left??scrollX}scrollX=pageXOffset=Number(x)||0;scrollY=pageYOffset=Number(y)||0};
globalThis.scrollBy=(x=0,y=0)=>{if(x&&typeof x==='object'){y=x.top??0;x=x.left??0}scrollTo(scrollX+(Number(x)||0),scrollY+(Number(y)||0))};
globalThis.__oryn_media={color_scheme:'light',reduced_motion:'no_preference'};
globalThis.matchMedia=query=>{query=String(query);const result={media:query,onchange:null,addEventListener(){},removeEventListener(){},addListener(){},removeListener(){}};Object.defineProperty(result,'matches',{get(){if(/prefers-color-scheme\s*:\s*dark/i.test(query))return __oryn_media.color_scheme==='dark';if(/prefers-color-scheme\s*:\s*light/i.test(query))return __oryn_media.color_scheme==='light';if(/prefers-reduced-motion\s*:\s*reduce/i.test(query))return __oryn_media.reduced_motion==='reduce';if(/prefers-reduced-motion\s*:\s*no-preference/i.test(query))return __oryn_media.reduced_motion==='no_preference';return true}});return result};
class OrynStorage {
  constructor(area,items=[]){this.area=area;this.fallback=new Map(items)}
  __response(operation,extra={}){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;return contextCall({operation:'storage',request:{operation,area:this.area,scope:{origin:globalThis.origin,top_level_page_id:__oryn_page_id},...extra}}).response}
  get items(){const response=this.__response('entries');return response&&response.result==='entries'?new Map(response.entries):this.fallback}
  get length(){return this.items.size}
  key(index){return [...this.items.keys()][index]??null}
  getItem(key){key=String(key);const response=this.__response('get',{key});return response&&response.result==='value'?response.value??null:(this.fallback.has(key)?this.fallback.get(key):null)}
  setItem(key,value){key=String(key);value=String(value);const response=this.__response('set',{key,value});if(response===null)this.fallback.set(key,value);else __oryn_context_trace.push({kind:'storage_changed',detail:{area:this.area,key}})}
  removeItem(key){key=String(key);const response=this.__response('remove',{key});if(response===null)this.fallback.delete(key)}
  clear(){const response=this.__response('clear');if(response===null)this.fallback.clear()}
}
if(globalThis.Storage&&globalThis.Storage.prototype)Object.setPrototypeOf(OrynStorage.prototype,globalThis.Storage.prototype);
globalThis.__oryn_page_id=null;
globalThis.__oryn_document_referrer='';
globalThis.__oryn_referrer_policy='';
globalThis.__oryn_request_initiator_url='';
if(!globalThis.localStorage)globalThis.localStorage=new OrynStorage('local');
if(!globalThis.sessionStorage)globalThis.sessionStorage=new OrynStorage('session');
if(!globalThis.__oryn_restore_storage)globalThis.__oryn_restore_storage=(localItems,sessionItems)=>{
  globalThis.localStorage=new OrynStorage('local',localItems||[]);
  globalThis.sessionStorage=new OrynStorage('session',sessionItems||[]);
};
globalThis.setTimeout=(callback,delay=0,...args)=>{if(typeof callback!=='function')callback=Function(String(callback));return __oryn_timer_schedule(callback,delay,false,args)};
globalThis.setInterval=(callback,delay=0,...args)=>{if(typeof callback!=='function')callback=Function(String(callback));return __oryn_timer_schedule(callback,delay,true,args)};
globalThis.clearTimeout=id=>{__oryn_timer_cancel(Number(id))};
globalThis.clearInterval=globalThis.clearTimeout;
globalThis.__oryn_drain=()=>{if(typeof __oryn_poll_context==='function')__oryn_poll_context()};
const __oryn_url_search_param_iterators=new WeakMap();
// URL, URLSearchParams, Location and History are installed by native bindings.
// Retain a lexical alias for bootstrap consumers without replacing constructors.
const OrynURL=globalThis.URL;
const contextCall=request=>JSON.parse(globalThis.__oryn_context_call(JSON.stringify(request)));
Object.defineProperty(Document.prototype,'cookie',{configurable:true,enumerable:true,get(){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return '';const response=contextCall({operation:'get_document_cookie',page_id:__oryn_page_id});return response.result==='document_cookie'?response.value:''},set(value){if(typeof globalThis.__oryn_context_call==='function'&&__oryn_page_id!==null)contextCall({operation:'set_document_cookie',page_id:__oryn_page_id,value:String(value)})}});
Object.defineProperty(Document.prototype,'title',{configurable:true,enumerable:true,get(){const node=this.querySelector('title');return node?node.textContent:''},set(value){let node=this.querySelector('title');if(!node){node=this.createElement('title');const parent=this.querySelector('head')||this.documentElement;parent.appendChild(node)}node.textContent=String(value)}});
const __oryn_page_proxies=new Map();
const __oryn_page_proxy=pageId=>{if(pageId==null)return null;if(__oryn_page_proxies.has(pageId))return __oryn_page_proxies.get(pageId);const proxy={__orynPageId:pageId,__closed:false,get closed(){return this.__closed},get origin(){const response=contextCall({operation:'get_related_page_origin',source_page_id:__oryn_page_id,target_page_id:pageId});return response.result==='security_origin'?response.serialized:''},focus(){contextCall({operation:'activate_page',source_page_id:__oryn_page_id,target_page_id:pageId})},close(){contextCall({operation:'close_page',source_page_id:__oryn_page_id,target_page_id:pageId});this.__closed=true},postMessage(data,targetOrigin='*'){contextCall({operation:'post_message',source_page_id:__oryn_page_id,target_page_id:pageId,target_origin:String(targetOrigin),data})}};__oryn_page_proxies.set(pageId,proxy);return proxy};
globalThis.__oryn_configure_context=(pageId,openerPageId,realmOrigin)=>{__oryn_page_id=pageId;globalThis.opener=__oryn_page_proxy(openerPageId);if(typeof globalThis.__oryn_context_call==='function'){const response=contextCall({operation:'get_security_origin',page_id:pageId});if(response.result==='security_origin'){globalThis.__oryn_security_origin=response.serialized;globalThis.__oryn_opaque_origin=!!response.opaque}}if(realmOrigin!==undefined){globalThis.__oryn_security_origin=String(realmOrigin);globalThis.__oryn_opaque_origin=realmOrigin==='null'}};
globalThis.open=(url='',name='')=>{url=String(url);name=String(name);if(name&&name!=='_blank'){const frame=[...document.querySelectorAll('iframe')].find(candidate=>candidate.name===name||candidate.getAttribute('name')===name);if(frame){if(url)frame.contentWindow.location.href=new OrynURL(url,location.href).href;return frame.contentWindow}}if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;const target=url?new OrynURL(url,location.href).href:null;const response=contextCall({operation:'open_page',opener_page_id:__oryn_page_id,url:target,name:name||null});if(response.result==='page_opened'){__oryn_context_trace.push({kind:'page_created',detail:{page_id:response.page_id,opener_page_id:__oryn_page_id}});return __oryn_page_proxy(response.page_id)}if(response.result==='page_denied')__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'popup.open',detail:response.reason}});return null};
globalThis.closed=false;
globalThis.close=()=>{if(closed||__oryn_page_id===null)return;globalThis.closed=true;__oryn_context_trace.push({kind:'page_closed',detail:{page_id:__oryn_page_id}})};
const __oryn_websockets=new Set();const __oryn_message_log=[];const __oryn_record_message=value=>{if(__oryn_message_log.length===128)__oryn_message_log.shift();__oryn_message_log.push(value)};let __oryn_context_event_id=0;
class SecurityPolicyViolationEvent extends Event {constructor(type,eventInitDict={}){super(type,{bubbles:true,cancelable:false});for(const [name,value] of Object.entries({documentURI:'',referrer:'',blockedURI:'',violatedDirective:'',effectiveDirective:'',originalPolicy:'',sourceFile:'',sample:'',disposition:'enforce',statusCode:0,lineNumber:0,columnNumber:0,...eventInitDict}))Object.defineProperty(this,name,{value,enumerable:true,writable:false,configurable:false})}}
globalThis.SecurityPolicyViolationEvent=SecurityPolicyViolationEvent;
const __oryn_structured_clone=globalThis.structuredClone;
globalThis.__oryn_deliver_message=(data,targetOrigin='*',source=null,sourceOrigin='')=>{targetOrigin=String(targetOrigin);if(targetOrigin!=='*'&&targetOrigin!=='/'&&targetOrigin!==globalThis.origin)return;const cloned=__oryn_structured_clone(data);__oryn_record_message({origin:String(sourceOrigin),data:cloned});const event=new Event('message',{});event.data=cloned;event.origin=String(sourceOrigin);event.source=source;queueMicrotask(()=>dispatchEvent(event))};
globalThis.__oryn_make_window_proxy=(deliver,targetOrigin,sourceOrigin,sameOriginTarget=null)=>{const facade={__source:null,__closed:false,origin:String(targetOrigin),get closed(){return this.__closed},focus(){},close(){this.__closed=true},postMessage(data,requestedOrigin='*'){deliver(data,String(requestedOrigin),proxy.__source,String(sourceOrigin))}};const proxy=sameOriginTarget?new Proxy(facade,{get(target,key,receiver){if(Reflect.has(target,key))return Reflect.get(target,key,receiver);return Reflect.get(sameOriginTarget,key,sameOriginTarget)},set(target,key,value,receiver){if(Reflect.has(target,key))return Reflect.set(target,key,value,receiver);return Reflect.set(sameOriginTarget,key,value,sameOriginTarget)},has(target,key){return Reflect.has(target,key)||Reflect.has(sameOriginTarget,key)}}):facade;return proxy};
globalThis.__oryn_make_provisional_window_proxy=()=>{let target=null;const queued=[];const facade={__source:null,__closed:false,get closed(){return target?target.closed:this.__closed},get origin(){return target?target.origin:''},focus(){if(target)target.focus()},close(){this.__closed=true;if(target)target.close()},postMessage(data,requestedOrigin='*'){if(target)target.postMessage(data,requestedOrigin);else queued.push([data,String(requestedOrigin)])},__bind(committed){target=committed;this.__closed=false;this.__source=committed.__source;while(queued.length){const [data,origin]=queued.shift();target.postMessage(data,origin)}}};return new Proxy(facade,{get(object,key,receiver){if(Reflect.has(object,key))return Reflect.get(object,key,receiver);return target?Reflect.get(target,key,target):undefined},set(object,key,value,receiver){if(Reflect.has(object,key)||!target)return Reflect.set(object,key,value,receiver);return Reflect.set(target,key,value,target)},has(object,key){return Reflect.has(object,key)||(target&&Reflect.has(target,key))}})};
globalThis.postMessage=(data,targetOrigin='*')=>globalThis.__oryn_deliver_message(data,targetOrigin,globalThis,globalThis.origin);
globalThis.__oryn_poll_context=()=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return;const response=contextCall({operation:'poll_events',page_id:__oryn_page_id,after_event_id:__oryn_context_event_id,max_events:32});if(response.result==='events')for(const record of response.events||[]){__oryn_context_event_id=Math.max(__oryn_context_event_id,record.event_id);const item=record.event;if(!item)continue;if(item.kind==='storage'){const change=item.change||{};const event=new Event('storage',{});event.key=change.key??null;event.oldValue=change.old_value??null;event.newValue=change.new_value??null;event.storageArea=change.area==='local'?localStorage:sessionStorage;event.url=location.href;dispatchEvent(event)}else if(item.kind==='message'){__oryn_record_message({origin:item.source_origin||'',data:item.data});__oryn_context_trace.push({kind:'opener_message',detail:{source_page_id:item.source_page_id,target_page_id:__oryn_page_id,origin:item.source_origin||''}});const event=new Event('message',{});event.data=item.data;event.origin=item.source_origin||'';event.source=__oryn_page_proxy(item.source_page_id);dispatchEvent(event)}else if(item.kind==='page_lifecycle'){const lifecycle=item.event||{};if(lifecycle.kind==='closed'){const proxy=__oryn_page_proxies.get(lifecycle.page_id);if(proxy)proxy.__closed=true}}else if(item.kind==='csp_violation'){const value=item.violation||{};const event=new SecurityPolicyViolationEvent('securitypolicyviolation',{documentURI:value.document_uri||'',referrer:value.referrer||'',blockedURI:value.blocked_uri||'',violatedDirective:value.effective_directive||'',effectiveDirective:value.effective_directive||'',originalPolicy:value.original_policy||'',sourceFile:value.source_file||'',sample:value.sample||'',disposition:value.disposition||'enforce',statusCode:value.status_code||0,lineNumber:value.line_number||0,columnNumber:value.column_number||0});document.dispatchEvent(event)}else if(item.kind==='csp_report'){__oryn_context_trace.push({kind:'csp_report',detail:{endpoint:item.endpoint||'',format:item.format||'',outcome:item.outcome||''}})}}for(const socket of [...__oryn_websockets])socket.__poll()};
class OrynWebSocket {constructor(url,protocols=[]){this.url=new OrynURL(url,location.href).href;this.protocol='';this.binaryType='blob';this.readyState=0;this.__listeners=new Map();if(typeof protocols==='string')protocols=[protocols];const response=contextCall({operation:'web_socket_open',page_id:__oryn_page_id,url:this.url,protocols:[...protocols].map(String)});if(response.result!=='web_socket_opened')throw new Error('WebSocket open failed');this.__id=response.socket_id;this.protocol=response.protocol;__oryn_websockets.add(this);queueMicrotask(()=>{this.readyState=1;this.__event('open')})}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}send(data){const frame=typeof data==='string'?{kind:'text',value:data}:{kind:'binary',base64:bytesToBase64(bodyBytes(data))};const bytes=typeof data==='string'?new TextEncoder().encode(data).length:bodyBytes(data).length;contextCall({operation:'web_socket_send',page_id:__oryn_page_id,socket_id:this.__id,frame});__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'outbound',frame_type:frame.kind,bytes,url:this.url}})}close(code=1000,reason=''){contextCall({operation:'web_socket_close',page_id:__oryn_page_id,socket_id:this.__id,code,reason:String(reason)});this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code,reason:String(reason),wasClean:true})}__poll(){if(this.readyState!==1)return;const response=contextCall({operation:'web_socket_poll',page_id:__oryn_page_id,socket_id:this.__id,max_frames:32});if(response.result!=='web_socket_events')return;for(const frame of response.frames||[]){let bytes=0;if(frame.kind==='text'){bytes=new TextEncoder().encode(frame.value).length;this.__event('message',{data:frame.value})}else if(frame.kind==='binary'){const value=base64ToBytes(frame.base64);bytes=value.length;this.__event('message',{data:this.binaryType==='arraybuffer'?value.buffer:new Blob([value])})}else if(frame.kind==='close'){this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code:frame.code??1005,reason:frame.reason||'',wasClean:true})}__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'inbound',frame_type:frame.kind,bytes,url:this.url}})}}}
Object.assign(OrynWebSocket,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});Object.assign(OrynWebSocket.prototype,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});globalThis.WebSocket=OrynWebSocket;
const __oryn_workers=new Set();
const __oryn_worker_clone=value=>{const encoded=JSON.stringify(value);if(encoded===undefined){const error=new TypeError('dedicated-worker messages must be structured-cloneable JSON values');error.name='DataCloneError';throw error}return encoded};
class OrynWorker {constructor(url,options={}){this.__onmessage=null;this.__onerror=null;this.__listeners=new Map();this.__pendingErrors=[];this.__closed=false;this.__global=String(options.orynGlobal||'dedicatedworker');const rawTarget=String(url),target=rawTarget.startsWith('data:')?rawTarget:new OrynURL(rawTarget,location.href).href;const opaquePrefix=`globalThis.indexedDB={open(){const error=new Error('opaque origins cannot access IndexedDB');error.name='SecurityError';throw error},deleteDatabase(){const error=new Error('opaque origins cannot access IndexedDB');error.name='SecurityError';throw error},databases(){const error=new Error('opaque origins cannot access IndexedDB');error.name='SecurityError';return Promise.reject(error)}};`;const blobSource=target.startsWith('blob:')&&typeof __oryn_blob_url_source==='function'?__oryn_blob_url_source(target):null;
const response=target.startsWith('data:,')?{result:'worker_script',source:opaquePrefix+decodeURIComponent(target.slice(6)),url:target,module:false,referrer_policy:''}:blobSource!==null?{result:'worker_script',source:blobSource,url:target,module:String(options.type||'classic')==='module',referrer_policy:globalThis.__oryn_referrer_policy||''}:contextCall({operation:'load_worker_script',page_id:__oryn_page_id,url:target,module:String(options.type||'classic')==='module'});if(response.result!=='worker_script')throw new Error('Worker script load failed');try{this.__id=__oryn_worker_create(response.source,response.url,!!response.module,this.__global,response.referrer_policy||'');__oryn_workers.add(this);queueMicrotask(()=>this.__poll())}catch(error){this.__closed=true;const message=String(error);__oryn_context_trace.push({kind:'capability_failure',detail:{capability:`worker.create: ${message}`,interface:'Worker',operation:'create',url:response.url||target,module:!!response.module,message}});this.__pendingErrors.push({error,message});queueMicrotask(()=>this.__flushErrors())}}get onmessage(){return this.__onmessage}set onmessage(callback){this.__onmessage=typeof callback==='function'?callback:null;if(this.__onmessage)queueMicrotask(()=>this.__poll())}get onerror(){return this.__onerror}set onerror(callback){this.__onerror=typeof callback==='function'?callback:null;if(this.__onerror)queueMicrotask(()=>this.__flushErrors())}addEventListener(type,callback){type=String(type);const list=this.__listeners.get(type)||[];list.push(callback);this.__listeners.set(type,list);if(type==='message')queueMicrotask(()=>this.__poll());if(type==='error')queueMicrotask(()=>this.__flushErrors())}removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=type==='message'?this.__onmessage:type==='error'?this.__onerror:this['on'+type];if(typeof handler==='function')handler.call(this,event)}__flushErrors(){if(!this.__onerror&&!(this.__listeners.get('error')||[]).length)return;while(this.__pendingErrors.length)this.__event('error',this.__pendingErrors.shift())}__poll(){if(this.__closed||!this.__id)return;if(!this.__onmessage&&!(this.__listeners.get('message')||[]).length)return;for(const encoded of JSON.parse(__oryn_worker_take(this.__id))){this.__event('message',{data:JSON.parse(encoded),source:null})}}postMessage(data){if(this.__closed||!this.__id)return;__oryn_worker_post(this.__id,__oryn_worker_clone(data));queueMicrotask(()=>this.__poll())}terminate(){if(this.__id)__oryn_worker_terminate(this.__id);this.__closed=true;this.__id=0;__oryn_workers.delete(this)}}
class OrynMessagePort {constructor(worker){this.__worker=worker;this.__onmessage=null;this.__listeners=new Map();this.__queue=[];this.__started=false;worker.addEventListener('message',event=>this.__enqueue(event.data))}get onmessage(){return this.__onmessage}set onmessage(callback){this.__onmessage=typeof callback==='function'?callback:null;if(this.__onmessage)this.start()}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=type==='message'?this.__onmessage:this['on'+type];if(typeof handler==='function')handler.call(this,event)}__enqueue(data){this.__queue.push(data);this.__flush()}__flush(){if(!this.__started)return;while(this.__queue.length)this.__event('message',{data:this.__queue.shift(),source:null})}postMessage(value){this.__worker.postMessage(value)}start(){this.__started=true;this.__flush()}close(){this.__queue.length=0;this.__worker.terminate()}}
class OrynSharedWorker {constructor(url,options={}){this.__listeners=new Map();this.__worker=new OrynWorker(url,{...options,orynGlobal:'sharedworker'});this.port=new OrynMessagePort(this.__worker);this.onerror=null;this.__worker.addEventListener('error',event=>this.__event('error',event))}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}__event(type,event){for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}}
class OrynServiceWorker {constructor(url,options={}){this.__worker=new OrynWorker(url,{...options,orynGlobal:'serviceworker'});this.state='activated';this.__onmessage=null;this.__listeners=new Map();this.__queue=[];this.__worker.addEventListener('message',event=>{this.__queue.push(event.data);this.__flush()})}get onmessage(){return this.__onmessage}set onmessage(callback){this.__onmessage=typeof callback==='function'?callback:null;this.__flush()}addEventListener(type,callback){type=String(type);const list=this.__listeners.get(type)||[];list.push(callback);this.__listeners.set(type,list);if(type==='message')this.__flush()}__flush(){if(!this.__onmessage&&!(this.__listeners.get('message')||[]).length)return;while(this.__queue.length)this.__event('message',{data:this.__queue.shift()})}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=type==='message'?this.__onmessage:this['on'+type];if(typeof handler==='function')handler.call(this,event)}postMessage(value){this.__worker.postMessage(value)}}
globalThis.Worker=OrynWorker;globalThis.SharedWorker=OrynSharedWorker;globalThis.ServiceWorker=OrynServiceWorker;
const __oryn_service_worker_container={controller:null,onmessage:null,__listeners:new Map(),addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)},register(url,options={}){const worker=new OrynServiceWorker(url,options);this.controller=worker;worker.addEventListener('message',event=>{for(const callback of this.__listeners.get('message')||[])callback.call(this,event);if(typeof this.onmessage==='function')this.onmessage(event)});return Promise.resolve({installing:null,waiting:null,active:worker,scope:new OrynURL('.',location.href).href,unregister(){worker.__worker.terminate();return Promise.resolve(true)}})},get ready(){return Promise.resolve({active:this.controller})}};
if(!globalThis.navigator)globalThis.navigator={};globalThis.navigator.serviceWorker=__oryn_service_worker_container;
const __oryn_context_poll_without_workers=globalThis.__oryn_poll_context;globalThis.__oryn_poll_context=()=>{__oryn_context_poll_without_workers();for(const worker of [...__oryn_workers])worker.__poll()};
const b64chars='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const bytesToBase64=value=>{const bytes=value instanceof Uint8Array?value:new Uint8Array(value);let out='';for(let index=0;index<bytes.length;index+=3){const first=bytes[index],second=bytes[index+1],third=bytes[index+2];out+=b64chars[first>>2];out+=b64chars[((first&3)<<4)|((second??0)>>4)];out+=second===undefined?'=':b64chars[((second&15)<<2)|((third??0)>>6)];out+=third===undefined?'=':b64chars[third&63]}return out};
const base64ToBytes=value=>{value=String(value).replace(/\s/g,'');const out=[];for(let index=0;index<value.length;index+=4){const first=b64chars.indexOf(value[index]),second=b64chars.indexOf(value[index+1]),third=value[index+2]==='='?-1:b64chars.indexOf(value[index+2]),fourth=value[index+3]==='='?-1:b64chars.indexOf(value[index+3]);out.push((first<<2)|(second>>4));if(third>=0)out.push(((second&15)<<4)|(third>>2));if(fourth>=0)out.push(((third&3)<<6)|fourth)}return Uint8Array.from(out)};
globalThis.btoa=value=>bytesToBase64(Uint8Array.from(String(value),char=>char.charCodeAt(0)&255));globalThis.atob=value=>{let result='';for(const byte of base64ToBytes(value))result+=String.fromCharCode(byte);return result};
const bodyBytes=body=>{if(body==null)return new Uint8Array();if(body instanceof OrynBlob)return body.__bytes.slice();if(body instanceof ArrayBuffer)return new Uint8Array(body.slice(0));if(ArrayBuffer.isView(body))return new Uint8Array(body.buffer.slice(body.byteOffset,body.byteOffset+body.byteLength));return new TextEncoder().encode(String(body))};
class OrynBlob {constructor(parts=[],options={}){const chunks=parts.map(bodyBytes),length=chunks.reduce((sum,item)=>sum+item.length,0);this.__bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){this.__bytes.set(chunk,offset);offset+=chunk.length}this.type=String(options.type||'').toLowerCase()}get size(){return this.__bytes.length}slice(start=0,end=this.size,type=''){return new OrynBlob([this.__bytes.slice(start,end)],{type})}arrayBuffer(){return Promise.resolve(this.__bytes.buffer.slice(0))}text(){return Promise.resolve(new TextDecoder().decode(this.__bytes))}stream(){return new OrynReadableStream({start:controller=>{controller.enqueue(this.__bytes.slice());controller.close()}})}}
class OrynFile extends OrynBlob {constructor(parts,name,options={}){super(parts,options);this.name=String(name);this.lastModified=Number(options.lastModified??0);this.webkitRelativePath=''}}
globalThis.Blob=OrynBlob;globalThis.File=OrynFile;
const __oryn_native_blob_url_source=globalThis.__oryn_blob_url_source;
const __oryn_script_blob_urls=new Map();let __oryn_next_script_blob_url=1;
const __oryn_native_create_object_url=URL.createObjectURL.bind(URL);
URL.createObjectURL=blob=>{if(blob instanceof OrynBlob){const url=`blob:${location.origin}/oryn-script-${__oryn_next_script_blob_url++}`;__oryn_script_blob_urls.set(url,new TextDecoder().decode(blob.__bytes));return url}return __oryn_native_create_object_url(blob)};
const __oryn_native_revoke_object_url=URL.revokeObjectURL.bind(URL);
URL.revokeObjectURL=url=>{__oryn_script_blob_urls.delete(String(url));return __oryn_native_revoke_object_url(url)};
globalThis.__oryn_blob_url_source=url=>__oryn_script_blob_urls.has(String(url))?__oryn_script_blob_urls.get(String(url)):__oryn_native_blob_url_source(String(url));
class OrynReadableStream {constructor(source={}){this.__queue=[];this.__closed=false;this.__waiting=[];this.__cancel=source.cancel;this.locked=false;const controller={enqueue:value=>{const waiter=this.__waiting.shift();waiter?waiter({done:false,value}):this.__queue.push(value)},close:()=>{this.__closed=true;for(const waiter of this.__waiting.splice(0))waiter({done:true})}};if(source.start)source.start(controller);this.__pull=source.pull?()=>source.pull(controller):null}getReader(){this.locked=true;const stream=this;return {read(){if(stream.__queue.length)return Promise.resolve({done:false,value:stream.__queue.shift()});if(stream.__closed)return Promise.resolve({done:true});return new Promise(resolve=>{stream.__waiting.push(resolve);if(stream.__pull)stream.__pull()})},cancel:reason=>stream.cancel(reason),releaseLock(){stream.locked=false}}}cancel(reason){this.__closed=true;for(const waiter of this.__waiting.splice(0))waiter({done:true});return Promise.resolve(this.__cancel?this.__cancel(reason):undefined)}pipeTo(destination){const reader=this.getReader(),writer=destination.getWriter();const pump=()=>reader.read().then(item=>item.done?writer.close():writer.write(item.value).then(pump));return pump()}pipeThrough(transform){this.pipeTo(transform.writable);return transform.readable}}
class OrynWritableStream {constructor(sink={}){this.__sink=sink;this.locked=false}getWriter(){this.locked=true;const stream=this;return {write:chunk=>Promise.resolve(stream.__sink.write?stream.__sink.write(chunk):undefined),close:()=>Promise.resolve(stream.__sink.close?stream.__sink.close():undefined),abort:reason=>Promise.resolve(stream.__sink.abort?stream.__sink.abort(reason):undefined),get ready(){return Promise.resolve()},get desiredSize(){return 1},releaseLock(){stream.locked=false}}}}
class OrynTransformStream {constructor(transformer={}){let controller;this.readable=new OrynReadableStream({start:value=>controller=value});this.writable=new OrynWritableStream({write:chunk=>transformer.transform?transformer.transform(chunk,controller):controller.enqueue(chunk),close:()=>{if(transformer.flush)return Promise.resolve(transformer.flush(controller)).then(()=>controller.close());controller.close()}})}}
globalThis.ReadableStream=OrynReadableStream;globalThis.WritableStream=OrynWritableStream;globalThis.TransformStream=OrynTransformStream;
class OrynAbortSignal {constructor(){this.aborted=false;this.reason=undefined;this.__listeners=[]}addEventListener(type,callback){if(type==='abort')this.__listeners.push(callback)}removeEventListener(type,callback){if(type==='abort')this.__listeners=this.__listeners.filter(item=>item!==callback)}throwIfAborted(){if(this.aborted)throw this.reason}}
class OrynAbortController {constructor(){this.signal=new OrynAbortSignal()}abort(reason=new Error('The operation was aborted')){if(this.signal.aborted)return;this.signal.aborted=true;this.signal.reason=reason;for(const callback of this.signal.__listeners.splice(0))callback.call(this.signal,new Event('abort'))}}
globalThis.AbortSignal=OrynAbortSignal;globalThis.AbortController=OrynAbortController;
class OrynHeaders {
  constructor(init=[]){this.items=[];const entries=init instanceof OrynHeaders?[...init]:(Array.isArray(init)||(init&&init[Symbol.iterator])?[...init]:Object.entries(init||{}));for(const [name,value] of entries)this.append(name,value)}
  set(name,value){name=String(name).toLowerCase();this.delete(name);this.items.push([name,String(value)])}
  append(name,value){name=String(name).toLowerCase();const old=this.get(name);this.set(name,old===null?value:`${old}, ${value}`)}
  get(name){name=String(name).toLowerCase();const item=this.items.find(([key])=>key===name);return item?item[1]:null}
  has(name){return this.get(name)!==null}
  delete(name){name=String(name).toLowerCase();this.items=this.items.filter(([key])=>key!==name)}
  [Symbol.iterator](){return this.items[Symbol.iterator]()}
}
class OrynResponse {
  constructor(body=null,{status=200,url='',headers=[],bodyChunks=[],streamId=null}={}){this.__bytes=streamId===null?bodyBytes(body):null;this.__chunks=bodyChunks.length?bodyChunks.map(bodyBytes):(this.__bytes?[this.__bytes.slice()]:[]);this.__streamId=streamId;this.__consumePromise=null;this.status=status;this.url=url;this.headers=new OrynHeaders(headers);let reading=false;this.body=new OrynReadableStream({start:controller=>{for(const chunk of this.__chunks)controller.enqueue(chunk.slice());if(streamId===null)controller.close()},pull:streamId===null?null:async controller=>{if(reading)return;reading=true;try{const result=JSON.parse(await globalThis.__oryn_stream_read_async(streamId));const chunk=base64ToBytes(result.body_base64||'');if(chunk.length){controller.enqueue(chunk);__oryn_context_trace.push({kind:'stream_chunk',detail:{direction:'response',bytes:chunk.length}})}if(result.done)controller.close()}finally{reading=false}},cancel:streamId===null?null:()=>{contextCall({operation:'http_stream_abort',page_id:__oryn_page_id,stream_id:streamId})}})}
  get ok(){return this.status>=200&&this.status<300}
  __consume(){if(this.__bytes)return Promise.resolve(this.__bytes);if(this.__consumePromise)return this.__consumePromise;const reader=this.body.getReader(),chunks=[];let length=0;this.__consumePromise=(async()=>{for(;;){const item=await reader.read();if(item.done)break;const chunk=bodyBytes(item.value);chunks.push(chunk);length+=chunk.length}const bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length}this.__bytes=bytes;return bytes})();return this.__consumePromise}
  text(){return this.__consume().then(bytes=>new TextDecoder().decode(bytes))}
  json(){return this.text().then(JSON.parse)}
  arrayBuffer(){return this.__consume().then(bytes=>bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength))}
  blob(){return this.__consume().then(bytes=>new OrynBlob([bytes],{type:this.headers.get('content-type')||''}))}
  static json(value,init={}){const headers=new OrynHeaders(init.headers||[]);if(!headers.has('content-type'))headers.set('content-type','application/json');return new OrynResponse(JSON.stringify(value),{...init,headers})}
}
globalThis.Headers=OrynHeaders;
globalThis.Response=OrynResponse;
class OrynRequest {constructor(input,init={}){this.url=new OrynURL(input&&input.url?input.url:String(input),location.href).href;this.method=String(init.method||'GET').toUpperCase();this.headers=new OrynHeaders(init.headers||[]);this.body=init.body??null;this.signal=init.signal||new OrynAbortSignal();this.credentials=init.credentials||'same-origin'}}
globalThis.Request=OrynRequest;
const __orynInputFiles=node=>Array.from(node.files||[]);
const __oryn_testdriver_text_node=(node,last=false)=>{if(!node)return null;if(node.nodeType===3)return node;let fallback=null,children=[...(node.childNodes||[])];if(last)children.reverse();for(const child of children){const found=__oryn_testdriver_text_node(child,last);if(found&&!fallback)fallback=found;if(found&&String(found.data||'').trim())return found}return fallback};
const __oryn_testdriver_key_name=value=>({"\uE003":'Backspace',"\uE004":'Tab',"\uE007":'Enter',"\uE008":'Shift',"\uE009":'Control',"\uE00A":'Alt',"\uE010":'End',"\uE011":'Home',"\uE012":'ArrowLeft',"\uE013":'ArrowUp',"\uE014":'ArrowRight',"\uE015":'ArrowDown',"\uE017":'Delete',"\uE03D":'Meta'}[value]||value);
const __oryn_testdriver_clipboard={text:'',html:''};
const __oryn_testdriver_data_transfer=()=>({getData(type){return type==='text/html'?__oryn_testdriver_clipboard.html:__oryn_testdriver_clipboard.text},setData(type,value){if(type==='text/html')__oryn_testdriver_clipboard.html=String(value);else __oryn_testdriver_clipboard.text=String(value)},types:['text/plain','text/html']});
const __oryn_testdriver_keyboard=(target,value,pressed,repeat=false)=>{
  target=__oryn_testdriver_key_target(target);
  const key=__oryn_testdriver_key_name(String(value));
  const modifiers=globalThis.__oryn_testdriver_modifiers||(globalThis.__oryn_testdriver_modifiers=new Set());
  if(pressed&&['Control','Meta','Shift','Alt'].includes(key))modifiers.add(key);
  if(!pressed)modifiers.delete(key);
  const code=/^[a-z]$/i.test(key)?'Key'+key.toUpperCase():/^[0-9]$/.test(key)?'Digit'+key:key===' '?'Space':key;
  const keyCode=({Backspace:8,Tab:9,Enter:13,Shift:16,Control:17,Alt:18,Pause:19,CapsLock:20,Escape:27,' ':32,PageUp:33,PageDown:34,End:35,Home:36,ArrowLeft:37,ArrowUp:38,ArrowRight:39,ArrowDown:40,Insert:45,Delete:46,Meta:91,NumLock:144,ScrollLock:145,';':186,':':186,'=':187,'+':187,',':188,'<':188,'-':189,'_':189,'.':190,'>':190,'/':191,'?':191,'`':192,'~':192,'[':219,'{':219,'\\':220,'|':220,']':221,'}':221,"'":222,'"':222})[key]??(/^[a-z0-9]$/i.test(key)?key.toUpperCase().charCodeAt(0):/^F(?:[1-9]|1[0-2])$/.test(key)?111+Number(key.slice(1)):0);
  const event=new KeyboardEvent(pressed?'keydown':'keyup',{bubbles:true,cancelable:true,composed:true,key,code,repeat,keyCode,charCode:0,
    ctrlKey:modifiers.has('Control'),metaKey:modifiers.has('Meta'),altKey:modifiers.has('Alt'),shiftKey:modifiers.has('Shift')});
  __oryn_dispatch_user_input(target,pressed?'keydown':'keyup',false,event);
  if(!pressed){modifiers.delete(key);return}
  if(event.defaultPrevented)return;
  if(key==='Control'||key==='Meta'||key==='Shift'||key==='Alt'){modifiers.add(key);return}
  const command=modifiers.has('Control')||modifiers.has('Meta');
  if(command&&key.toLowerCase()==='c'){
    const selection=document.getSelection();
    __oryn_testdriver_clipboard.text=selection?selection.toString():'';
    __oryn_testdriver_clipboard.html=__oryn_testdriver_clipboard.text;
    if(selection&&selection.rangeCount){try{const range=selection.getRangeAt(0),fragment=range.cloneContents(),container=document.createElement('div');container.appendChild(fragment);const sourceRoot=range.commonAncestorContainer?.nodeType===1?range.commonAncestorContainer:range.commonAncestorContainer?.parentElement;const source=sourceRoot?[...sourceRoot.querySelectorAll('*')]:[],clones=[...container.querySelectorAll('*')],properties=['color','background-color','font-family','font-size','font-style','font-weight','text-decoration','padding','margin','border'];for(let index=0;index<Math.min(source.length,clones.length);index++){const computed=getComputedStyle(source[index]),declarations=[];for(const property of properties){let resolved=computed.getPropertyValue(property);if(!resolved&&(property==='padding'||property==='margin')){const sides=['top','right','bottom','left'].map(side=>computed.getPropertyValue(`${property}-${side}`));if(sides[0]&&sides.every(value=>value===sides[0]))resolved=sides[0]}if(resolved&&resolved!=='normal'&&resolved!=='none'&&resolved!=='0px')declarations.push(`${property}: ${resolved}`)}if(declarations.length)clones[index].setAttribute('style',declarations.join('; ')+';')}__oryn_testdriver_clipboard.html=container.innerHTML}catch(_){}}
    const copy=new Event('copy',{bubbles:true,cancelable:true});
    copy.clipboardData=__oryn_testdriver_data_transfer();target.dispatchEvent(copy);return
  }
  if(command&&key.toLowerCase()==='v'){
    const tag=String(target.tagName||'').toLowerCase();
    const controlPaste=tag==='textarea'||tag==='input';
    const transfer=__oryn_testdriver_data_transfer();
    const pasted=transfer.getData('text/plain');
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertFromPaste',data:controlPaste?pasted:null,dataTransfer:controlPaste?null:transfer});
    target.dispatchEvent(before);
    if(!before.defaultPrevented){
      if(controlPaste){const current=String(target.value||'');let start=Number(target.selectionStart??target.__oryn_selection_start??current.length),end=Number(target.selectionEnd??target.__oryn_selection_end??start);const caret=__oryn_edit_control(target,pasted,start,end);target.__oryn_selection_start=target.__oryn_selection_end=caret;target.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertFromPaste',data:pasted,dataTransfer:null}))}
      else{const html=transfer.getData('text/html');globalThis.__oryn_pending_input_event={inputType:'insertFromPaste',data:null,dataTransfer:transfer};try{document.execCommand(html&&/<[a-z!/]/i.test(html)?'insertHTML':'insertText',false,html||pasted);const selection=document.getSelection();let host=selection?.anchorNode;while(host&&!(host.nodeType===1&&host.isContentEditable))host=host.parentElement;for(const styled of host?.querySelectorAll?.('[style]')||[]){const value=styled.getAttribute('style');if(value&&!value.trimEnd().endsWith(';'))styled.setAttribute('style',value.trimEnd()+';')}}finally{delete globalThis.__oryn_pending_input_event}}
    }return
  }
  if(command&&key.toLowerCase()==='a'){
    document.execCommand('selectAll',false,'');return
  }
  const tag=String(target.tagName||'').toLowerCase();
  const textControl=tag==='textarea'||(tag==='input'&&!['button','checkbox','file','radio','reset','submit'].includes(String(target.type||'').toLowerCase()));
  if(textControl&&(key==='Backspace'||key==='Delete'||[...key].length===1)){
    const unavailable=()=>!target.isConnected||target.getClientRects().length===0||getComputedStyle(target).visibility==='hidden';
    if(unavailable())return;
    if([...key].length===1){
      const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);
      if(keypress.defaultPrevented||unavailable())return;
    }
    const inputType=key==='Backspace'?'deleteContentBackward':key==='Delete'?'deleteContentForward':'insertText';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType,data:[...key].length===1?key:null});target.dispatchEvent(before);
    if(before.defaultPrevented||unavailable())return;
    const current=String(target.value||'');
    let start=Number(target.selectionStart??target.__oryn_selection_start??current.length),end=Number(target.selectionEnd??target.__oryn_selection_end??start);
    start=Math.max(0,Math.min(current.length,start));end=Math.max(start,Math.min(current.length,end));
    let replacement='';
    if(key==='Backspace'&&start===end&&start>0)start--;else if(key==='Delete'&&start===end&&end<current.length)end++;else if([...key].length===1)replacement=key;
    const caret=__oryn_edit_control(target,replacement,start,end);
    target.__oryn_selection_start=target.__oryn_selection_end=caret;
    const input=new Event('input',{bubbles:true});input.inputType=inputType;input.data=replacement||null;target.dispatchEvent(input);return
  }
  if(key==='Enter'){
    let plaintext=false;
    const selection=document.getSelection(),startNode=selection?.rangeCount?selection.getRangeAt(0).startContainer:target;
    for(let element=startNode?.nodeType===1?startNode:startNode?.parentElement; element; element=element.parentElement){
      const editable=element.getAttribute?.('contenteditable')?.toLowerCase();
      if(editable==='false')break;
      if(editable==='plaintext-only')plaintext=true;
      else if(editable===''||editable==='true')plaintext=false;
    }
    if(document.designMode.toLowerCase()==='on')plaintext=false;
    const lineBreak=modifiers.has('Shift')||plaintext,inputType=lineBreak?'insertLineBreak':'insertParagraph';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand(lineBreak?'insertLineBreak':'insertParagraph')
  }
  else if(key==='Backspace'||key==='Delete'){
    const inputType=key==='Backspace'?'deleteContentBackward':'deleteContentForward';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand(key==='Backspace'?'delete':'forwardDelete')
  }
  else if(key==='Home'||key==='End'){
    const text=__oryn_testdriver_text_node(target,key==='End');if(text){const selection=document.getSelection();selection.collapse(text,key==='Home'?0:text.data.length)}
  }else if([...key].length===1){
    const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);if(keypress.defaultPrevented)return;
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertText',data:key});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand('insertText',false,key)
  }
};
const __oryn_testdriver_key_target=target=>target?__oryn_resolve_keyboard_target(target):target;
globalThis.__oryn_testdriver_send_keys=async(target,keys)=>{await Promise.resolve();if(!target)throw new TypeError('testdriver target is required');target=__oryn_testdriver_key_target(target);target.focus?.();keys=String(keys);if(String(target.tagName).toLowerCase()==='input'&&String(target.type).toLowerCase()==='file'){const response=contextCall({operation:'materialize_upload',page_id:__oryn_page_id,label:keys});if(response.result!=='upload_materialized')throw new Error('parent did not authorize the selected upload');__oryn_select_file_bytes(target,JSON.stringify([{filename:response.filename,media_type:response.media_type,body_base64:response.body_base64,last_modified:response.last_modified}]));target.dispatchEvent(new Event('input',{bubbles:true}));target.dispatchEvent(new Event('change',{bubbles:true}));return}let currentTarget=target;for(const key of [...keys]){__oryn_testdriver_keyboard(currentTarget,key,true);currentTarget=__oryn_testdriver_key_target(document.activeElement||target);__oryn_testdriver_keyboard(currentTarget,key,false)}};
const __oryn_pointer_down=target=>{if(!target)return;if(__oryn_dispatch_user_input(target,'pointerdown',false)&&__oryn_dispatch_user_input(target,'mousedown',false))__oryn_focus_from_pointer(target)};
const __oryn_pointer_up=target=>{if(!target)return;__oryn_dispatch_user_input(target,'pointerup',false);__oryn_dispatch_user_input(target,'mouseup',false)};
class OrynFormData {constructor(form){this.items=[];if(form)for(const node of [...form.querySelectorAll('input'),...form.querySelectorAll('textarea'),...form.querySelectorAll('select')]){const name=node.getAttribute('name');if(!name||node.disabled)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;if(type==='file'){for(const file of __orynInputFiles(node))this.append(name,file);continue}this.append(name,node.value)}}append(name,value,filename){if(filename!==undefined&&value instanceof OrynBlob)value=new OrynFile([value.__bytes],filename,{type:value.type});this.items.push([String(name),value instanceof OrynBlob?value:String(value)])}set(name,value,filename){this.delete(name);this.append(name,value,filename)}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1])}has(name){return this.items.some(item=>item[0]===String(name))}delete(name){this.items=this.items.filter(item=>item[0]!==String(name))}entries(){return this.items[Symbol.iterator]()}keys(){return this.items.map(item=>item[0])[Symbol.iterator]()}values(){return this.items.map(item=>item[1])[Symbol.iterator]()}[Symbol.iterator](){return this.entries()}}
globalThis.FormData=OrynFormData;
globalThis.crypto={getRandomValues(array){const encoded=__oryn_crypto_sync('random',String(array.byteLength));array.set(base64ToBytes(encoded));return array},randomUUID(){const bytes=new Uint8Array(16);this.getRandomValues(bytes);bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`},subtle:{digest(algorithm,value){if(String(algorithm).toUpperCase()!=='SHA-256')return Promise.reject(new Error('unsupported digest'));const result=base64ToBytes(__oryn_crypto_sync('sha256',bytesToBase64(bodyBytes(value))));return Promise.resolve(result.buffer)}}};
const __oryn_idb_call=command=>{const outer=contextCall({operation:'storage',request:{operation:'indexed_db',origin:globalThis.origin,command}});if(outer.result!=='storage'||outer.response.result!=='indexed_db')throw new Error('invalid IndexedDB response');return outer.response.response};
const __oryn_clone_encode=value=>{if(value instanceof ArrayBuffer)return {__oryn_clone:'array_buffer',base64:bytesToBase64(value)};if(ArrayBuffer.isView(value))return {__oryn_clone:'typed_array',type:value.constructor.name,base64:bytesToBase64(new Uint8Array(value.buffer,value.byteOffset,value.byteLength))};if(value instanceof OrynFile)return {__oryn_clone:'file',base64:bytesToBase64(value.__bytes),type:value.type,name:value.name,last_modified:value.lastModified};if(value instanceof OrynBlob)return {__oryn_clone:'blob',base64:bytesToBase64(value.__bytes),type:value.type};if(Array.isArray(value))return value.map(__oryn_clone_encode);if(value&&typeof value==='object'){const result={};for(const [key,item] of Object.entries(value))result[key]=__oryn_clone_encode(item);return result}return value};
const __oryn_clone_decode=value=>{if(Array.isArray(value))return value.map(__oryn_clone_decode);if(value&&typeof value==='object'){if(value.__oryn_clone==='array_buffer')return base64ToBytes(value.base64).buffer;if(value.__oryn_clone==='typed_array'){const bytes=base64ToBytes(value.base64),ctor=globalThis[value.type];return new ctor(bytes.buffer)}if(value.__oryn_clone==='blob')return new Blob([base64ToBytes(value.base64)],{type:value.type});if(value.__oryn_clone==='file')return new File([base64ToBytes(value.base64)],value.name,{type:value.type,lastModified:value.last_modified});const result={};for(const [key,item] of Object.entries(value))result[key]=__oryn_clone_decode(item);return result}return value};
const __oryn_idb_slots=(target,values)=>{const descriptors={};for(const [name,value] of Object.entries(values))descriptors[name]={configurable:true,enumerable:true,writable:true,value};Object.defineProperties(target,descriptors)};
class OrynIDBKeyRange {constructor(lower,upper,lowerOpen=false,upperOpen=false){this.__oryn_key_range=true;__oryn_idb_slots(this,{lower,upper,lowerOpen:!!lowerOpen,upperOpen:!!upperOpen});this.lower_open=this.lowerOpen;this.upper_open=this.upperOpen}includes(key){const compare=(left,right)=>typeof left==='number'&&typeof right==='number'?left-right:JSON.stringify(left).localeCompare(JSON.stringify(right));return (this.lower===undefined||(this.lower_open?compare(key,this.lower)>0:compare(key,this.lower)>=0))&&(this.upper===undefined||(this.upper_open?compare(key,this.upper)<0:compare(key,this.upper)<=0))}static only(value){return new this(value,value)}static lowerBound(value,open=false){return new this(value,undefined,open,false)}static upperBound(value,open=false){return new this(undefined,value,false,open)}static bound(lower,upper,lowerOpen=false,upperOpen=false){return new this(lower,upper,lowerOpen,upperOpen)}}
globalThis.IDBKeyRange=OrynIDBKeyRange;
class OrynIDBRequest {constructor(){__oryn_idb_slots(this,{result:undefined,error:null,transaction:null,onsuccess:null,onerror:null,onupgradeneeded:null,onblocked:null})}__fire(type,extra={}){const event={type,target:this,...extra};const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}__success(value){queueMicrotask(()=>{this.result=value;this.__fire('success')})}__failure(error){queueMicrotask(()=>{this.error=error;this.__fire('error')})}}
class OrynIDBCursor {constructor(request,records,index=0){this.__request=request;this.__records=records;this.__index=index;__oryn_idb_slots(this,{key:undefined,primaryKey:undefined,value:undefined});this.__sync()}__sync(){const record=this.__records[this.__index];this.key=record?.key;this.primaryKey=record?.primary_key;this.value=__oryn_clone_decode(record?.value)}continue(key){let next=this.__index+1;if(key!==undefined)next=this.__records.findIndex((record,index)=>index>this.__index&&JSON.stringify(record.key)>=JSON.stringify(key));this.__index=next<0?this.__records.length:next;queueMicrotask(()=>{if(this.__index>=this.__records.length)this.__request.__success(null);else{this.__sync();this.__request.__success(this)}})}advance(count){this.__index+=Math.max(1,Number(count)||1);this.continue()}delete(){return this.__request.__store.delete(this.primaryKey)}update(value){return this.__request.__store.put(value,this.primaryKey)}}
class OrynIDBIndex {constructor(store,name){this.store=store;__oryn_idb_slots(this,{name:String(name),objectStore:store})}get(query){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query,reverse:false});request.__success(__oryn_clone_decode(response.records[0]?.value))}catch(error){request.__failure(error)}return request}getAll(query,count){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query:query??null,reverse:false});request.__success(response.records.slice(0,count??response.records.length).map(record=>__oryn_clone_decode(record.value)))}catch(error){request.__failure(error)}return request}count(query){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query:query??null,reverse:false});request.__success(response.records.length)}catch(error){request.__failure(error)}return request}openCursor(query,direction='next'){return this.store.__cursor(query,direction,this.name)}}
class OrynIDBObjectStore {constructor(transaction,name){__oryn_idb_slots(this,{transaction,name:String(name)})}put(value,key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:__oryn_clone_encode(value),add_only:false}]});request.__success(response.key)}catch(error){request.__failure(error)}return request}add(value,key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:__oryn_clone_encode(value),add_only:true}]});request.__success(response.key)}catch(error){request.__failure(error)}return request}get(key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'get',database:this.transaction.db.name,store:this.name,key});request.__success(__oryn_clone_decode(response.value))}catch(error){request.__failure(error)}return request}getAll(query,count){const request=new OrynIDBRequest();try{const response=query===undefined?__oryn_idb_call({command:'get_all',database:this.transaction.db.name,store:this.name}):__oryn_idb_call({command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query,reverse:false});const values=response.values?.length?response.values:response.records.map(record=>record.value);request.__success(values.slice(0,count??values.length).map(__oryn_clone_decode))}catch(error){request.__failure(error)}return request}delete(key){const request=new OrynIDBRequest();try{__oryn_idb_call({command:'delete',database:this.transaction.db.name,store:this.name,key});request.__success(undefined)}catch(error){request.__failure(error)}return request}clear(){const request=new OrynIDBRequest();try{__oryn_idb_call({command:'clear',database:this.transaction.db.name,store:this.name});request.__success(undefined)}catch(error){request.__failure(error)}return request}count(){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'count',database:this.transaction.db.name,store:this.name});request.__success(response.count||0)}catch(error){request.__failure(error)}return request}__cursor(query,direction='next',index=null){const request=new OrynIDBRequest();request.__store=this;try{const response=__oryn_idb_call({command:'cursor',database:this.transaction.db.name,store:this.name,index,query:query??null,reverse:String(direction).startsWith('prev')});queueMicrotask(()=>{if(!response.records.length)request.__success(null);else request.__success(new OrynIDBCursor(request,response.records))})}catch(error){request.__failure(error)}return request}openCursor(query,direction='next'){return this.__cursor(query,direction)}createIndex(name,keyPath,options={}){__oryn_idb_call({command:'create_index',database:this.transaction.db.name,store:this.name,definition:{name:String(name),key_path:String(keyPath),unique:!!options.unique}});return new OrynIDBIndex(this,name)}index(name){return new OrynIDBIndex(this,name)}}
class OrynIDBTransaction {constructor(db,names,mode='readonly'){this.db=db;this.names=Array.isArray(names)?names:[String(names)];__oryn_idb_slots(this,{mode,objectStoreNames:this.names.slice(),oncomplete:null,onabort:null,onerror:null});queueMicrotask(()=>{if(typeof this.oncomplete==='function')this.oncomplete({type:'complete',target:this})})}objectStore(name){if(!this.names.includes(String(name)))throw new Error('NotFoundError');return new OrynIDBObjectStore(this,name)}abort(){if(typeof this.onabort==='function')this.onabort({type:'abort',target:this})}}
class OrynIDBDatabase {constructor(name,version,stores=[]){__oryn_idb_slots(this,{name:String(name),version,objectStoreNames:stores})}createObjectStore(name,options={}){const response=__oryn_idb_call({command:'create_object_store',database:this.name,definition:{name:String(name),key_path:options.keyPath??null,auto_increment:!!options.autoIncrement}});this.objectStoreNames=response.object_stores;return new OrynIDBObjectStore(new OrynIDBTransaction(this,[name],'versionchange'),name)}transaction(names,mode='readonly'){return new OrynIDBTransaction(this,names,mode)}close(){}}
globalThis.indexedDB={open(name,version=0){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'open',name:String(name),version:Number(version)||0});const db=new OrynIDBDatabase(name,response.version,response.object_stores);queueMicrotask(()=>{request.result=db;if(response.upgrade_needed)request.__fire('upgradeneeded',{oldVersion:response.old_version,newVersion:response.version});request.__fire('success')})}catch(error){request.__failure(error)}return request},deleteDatabase(name){const request=new OrynIDBRequest();try{__oryn_idb_call({command:'delete_database',name:String(name)});request.__success(undefined)}catch(error){request.__failure(error)}return request}};
const __oryn_idb_event_target=prototype=>{prototype.addEventListener=function(type,callback){const listeners=this.__listeners||(this.__listeners=new Map()),list=listeners.get(String(type))||[];list.push(callback);listeners.set(String(type),list)};prototype.removeEventListener=function(type,callback){const listeners=this.__listeners;if(listeners)listeners.set(String(type),(listeners.get(String(type))||[]).filter(item=>item!==callback))};prototype.dispatchEvent=function(event){if(event.target!==this)try{Object.defineProperty(event,'target',{configurable:true,value:this})}catch(_){}for(const callback of this.__listeners?.get(event.type)||[])callback.call(this,event);const handler=this['on'+event.type];if(typeof handler==='function')handler.call(this,event);return !event.defaultPrevented}};
for(const [implementation,name] of [[OrynIDBRequest,'IDBRequest'],[OrynIDBCursor,'IDBCursor'],[OrynIDBIndex,'IDBIndex'],[OrynIDBObjectStore,'IDBObjectStore'],[OrynIDBTransaction,'IDBTransaction'],[OrynIDBDatabase,'IDBDatabase']])if(globalThis[name]?.prototype)Object.setPrototypeOf(implementation.prototype,globalThis[name].prototype);
for(const prototype of [OrynIDBRequest.prototype,OrynIDBTransaction.prototype,OrynIDBDatabase.prototype])__oryn_idb_event_target(prototype);
OrynIDBRequest.prototype.__fire=function(type,extra={}){this.dispatchEvent({type,target:this,defaultPrevented:false,...extra})};
OrynIDBObjectStore.prototype.getAllKeys=function(query,count){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query:query??null,reverse:false});request.__success(response.records.slice(0,count??response.records.length).map(record=>__oryn_clone_decode(record.primary_key)))}catch(error){request.__failure(error)}return request};
OrynIDBObjectStore.prototype.openKeyCursor=function(query,direction='next'){const request=this.__cursor(query,direction);const success=request.__success.bind(request);request.__success=value=>{if(value)value.value=undefined;success(value)};return request};
OrynIDBIndex.prototype.getAllKeys=function(query,count){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query:query??null,reverse:false});request.__success(response.records.slice(0,count??response.records.length).map(record=>__oryn_clone_decode(record.primary_key)))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.getKey=function(query){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query,reverse:false});request.__success(__oryn_clone_decode(response.records[0]?.primary_key))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.openKeyCursor=function(query,direction='next'){const request=this.store.__cursor(query,direction,this.name);const success=request.__success.bind(request);request.__success=value=>{if(value)value.value=undefined;success(value)};return request};
// Materialize an index snapshot once per script-visible index object. Besides
// matching IndexedDB's transaction snapshot semantics, this prevents large
// multiEntry lookup loops from crossing the typed parent boundary once per
// key. Mutating transactions create fresh object/index wrappers, so snapshots
// cannot leak across transaction boundaries.
OrynIDBIndex.prototype.__records=function(query){if(!this.__oryn_records)this.__oryn_records=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query:null,reverse:false}).records;const matches=record=>query===undefined||query===null||(query&&query.__oryn_key_range?query.includes(__oryn_clone_decode(record.key)):indexedDB.cmp(__oryn_clone_decode(record.key),query)===0);return this.__oryn_records.filter(matches)};
OrynIDBIndex.prototype.get=function(query){const request=new OrynIDBRequest();try{request.__success(__oryn_clone_decode(this.__records(query)[0]?.value))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.getKey=function(query){const request=new OrynIDBRequest();try{request.__success(__oryn_clone_decode(this.__records(query)[0]?.primary_key))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.getAll=function(query,count){const request=new OrynIDBRequest();try{const records=this.__records(query);request.__success(records.slice(0,count??records.length).map(record=>__oryn_clone_decode(record.value)))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.getAllKeys=function(query,count){const request=new OrynIDBRequest();try{const records=this.__records(query);request.__success(records.slice(0,count??records.length).map(record=>__oryn_clone_decode(record.primary_key)))}catch(error){request.__failure(error)}return request};
OrynIDBIndex.prototype.count=function(query){const request=new OrynIDBRequest();try{request.__success(this.__records(query).length)}catch(error){request.__failure(error)}return request};
globalThis.IDBRequest=OrynIDBRequest;globalThis.IDBOpenDBRequest=OrynIDBRequest;globalThis.IDBCursor=OrynIDBCursor;globalThis.IDBIndex=OrynIDBIndex;globalThis.IDBObjectStore=OrynIDBObjectStore;globalThis.IDBTransaction=OrynIDBTransaction;globalThis.IDBDatabase=OrynIDBDatabase;
indexedDB.cmp=(left,right)=>{if(typeof left==='number'&&typeof right==='number')return Math.sign(left-right);if(typeof left==='string'&&typeof right==='string')return left<right?-1:left>right?1:0;const a=JSON.stringify(left),b=JSON.stringify(right);return a<b?-1:a>b?1:0};
indexedDB.open=(name,version=0)=>{const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'open',name:String(name),version:Number(version)||0});const db=new OrynIDBDatabase(name,response.version,response.object_stores),transaction=new OrynIDBTransaction(db,response.object_stores,'versionchange');request.transaction=transaction;queueMicrotask(()=>{request.result=db;if(response.upgrade_needed)request.__fire('upgradeneeded',{oldVersion:response.old_version,newVersion:response.version});request.__fire('success')})}catch(error){request.__failure(error)}return request};
if(typeof globalThis.__oryn_fetch_async==='function')globalThis.fetch=(input,init={})=>{
  const request=input instanceof OrynRequest?new OrynRequest(input.url,{method:init.method||input.method,headers:init.headers||input.headers,body:init.body??input.body,signal:init.signal||input.signal,credentials:init.credentials||input.credentials}):new OrynRequest(input,init);
  const method=request.method,headers=[...request.headers],signal=request.signal||null;if(signal&&signal.aborted)return Promise.reject(signal.reason||new Error('AbortError'));
  const bytes=bodyBytes(request.body);
  __oryn_context_trace.push({kind:'request_started',detail:{method,url:request.url,bytes:bytes.length}});
  return globalThis.__oryn_fetch_async(request.url,method,bytesToBase64(bytes),JSON.stringify(headers),'base64',request.credentials).then(raw=>{
    if(signal&&signal.aborted)throw signal.reason||new Error('AbortError');
    const result=JSON.parse(raw);let encoded=atob(result.body_base64||'');const responseBytes=Uint8Array.from(encoded,value=>value.charCodeAt(0));
    const chunks=(result.body_chunks_base64||[]).map(base64ToBytes);__oryn_context_trace.push({kind:'response_received',detail:{url:result.url,status:result.status,bytes:result.stream_id===null?responseBytes.length:null}});for(const chunk of chunks)__oryn_context_trace.push({kind:'stream_chunk',detail:{direction:'response',bytes:chunk.length}});
    const response=new OrynResponse(responseBytes,{status:result.status,url:result.url,headers:result.headers,bodyChunks:chunks,streamId:result.stream_id});if(signal)signal.addEventListener('abort',()=>response.body.cancel(signal.reason));return response;
  });
};
class OrynXMLHttpRequest {
  constructor(){this.readyState=0;this.status=0;this.responseType='';this.response=null;this.responseText='';this.responseURL='';this.timeout=0;this.withCredentials=false;this.__headers=new OrynHeaders();this.__responseHeaders=new OrynHeaders();this.__listeners=new Map();this.__aborted=false;this.__timedOut=false}
  addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}
  removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}
  __event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}
  __state(value){this.readyState=value;this.__event('readystatechange')}
  open(method,url,async=true){if(async===false){__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'xhr.synchronous_main_thread',detail:'synchronous main-thread XHR is unsupported'}});throw new Error('synchronous main-thread XHR is unsupported')}this.__method=String(method).toUpperCase();this.__url=new OrynURL(url,location.href).href;this.__state(1)}
  setRequestHeader(name,value){this.__headers.append(name,value)}
  getResponseHeader(name){return this.__responseHeaders.get(name)}
  getAllResponseHeaders(){return [...this.__responseHeaders].map(([name,value])=>`${name}: ${value}\r\n`).join('')}
  send(body=null){if(this.readyState!==1)throw new Error('InvalidStateError');this.__event('loadstart');const controller=new AbortController();this.__controller=controller;const started=Date.now();const timeout=()=>{if(this.__aborted||this.__timedOut)return;this.__timedOut=true;controller.abort(new Error('TimeoutError'));this.status=0;this.__event('timeout');this.__event('loadend')};if(this.timeout>0)this.__timer=setTimeout(timeout,this.timeout);Promise.resolve().then(()=>{if(this.__aborted)return null;return fetch(this.__url,{method:this.__method,headers:this.__headers,body,credentials:this.withCredentials?'include':'same-origin',signal:controller.signal})}).then(async result=>{if(!result||this.__aborted||this.__timedOut)return;if(this.timeout>0&&Date.now()-started>=this.timeout){timeout();return}this.status=result.status;this.responseURL=result.url;this.__responseHeaders=result.headers;this.__state(2);const reader=result.body.getReader(),chunks=[],decoder=new TextDecoder();let loaded=0,sawChunk=false;while(true){const part=await reader.read();if(part.done)break;if(this.__aborted||this.__timedOut){await reader.cancel();return}const chunk=bodyBytes(part.value);chunks.push(chunk);loaded+=chunk.length;if(!sawChunk){sawChunk=true;this.__state(3)}this.responseText+=decoder.decode(chunk,{stream:true});this.__event('progress',{lengthComputable:false,loaded,total:0})}this.responseText+=decoder.decode();if(this.__timer)clearTimeout(this.__timer);const bytes=new Uint8Array(loaded);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length}if(!sawChunk)this.__state(3);this.response=this.responseType==='arraybuffer'?bytes.buffer.slice(0):this.responseType==='blob'?new Blob([bytes],{type:this.__responseHeaders.get('content-type')||''}):this.responseType==='json'?JSON.parse(this.responseText||'null'):this.responseText;this.__state(4);this.__event('load');this.__event('loadend')}).catch(error=>{if(this.__aborted||this.__timedOut)return;this.status=0;this.__event('error',{error});this.__event('loadend')})}
  abort(){if(this.__aborted)return;this.__aborted=true;if(this.__controller)this.__controller.abort(new Error('AbortError'));this.status=0;this.readyState=0;this.__event('abort');this.__event('loadend')}
}
Object.assign(OrynXMLHttpRequest,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});Object.assign(OrynXMLHttpRequest.prototype,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});globalThis.XMLHttpRequest=OrynXMLHttpRequest;
for(const [name,value] of Object.entries(__oryn_generated_platform))if(value!==undefined&&(typeof globalThis.__oryn_context_call==='function'||['Blob','File','FormData'].includes(name)))globalThis[name]=value;
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextTraceEntry {
    pub kind: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

pub struct RustExecutableDocument {
    pub document: ParsedDocument,
    pub diagnostics: Vec<oryn_common::v3::CapabilityDiagnostic>,
    agent: crate::agent::AgentRuntime,
    top_realm: RealmId,
    node_realms: BTreeMap<NodeId, RealmId>,
    frame_realms: BTreeMap<RealmId, FrameRealm>,
    pending_document_loads: BTreeSet<RealmId>,
    pending_frame_navigations: BTreeMap<(RealmId, NodeId), FrameNavigationContext>,
    base_url: Option<Url>,
    resources: BTreeMap<String, String>,
    broker: Option<SharedNetworkBroker>,
    next_dynamic_request_id: u64,
    semantic_mutation_cursor: MutationCursor,
    #[cfg(unix)]
    context: Option<(
        crate::context_service::SharedContextClient,
        oryn_common::v3::PageId,
        Option<oryn_common::v3::PageId>,
    )>,
}

#[derive(Debug, Clone)]
struct FrameRealm {
    parent_realm: RealmId,
    frame: NodeId,
    committed_url: String,
}

#[derive(Debug, Clone)]
struct FrameNavigationContext {
    initiator_url: Option<String>,
    referrer_policy: Option<String>,
    post: Option<oryn_js::navigation::PostResource>,
}

impl RustExecutableDocument {
    pub fn semantic_frame_ids(&self) -> BTreeMap<NodeId, u64> {
        self.node_realms
            .iter()
            .map(|(node, realm)| (*node, u64::from(realm.0)))
            .collect()
    }

    pub fn load(html_source: &str) -> Result<Self, HostError> {
        Self::load_with_resources(html_source, &BTreeMap::new())
    }

    pub fn load_with_resources(
        html_source: &str,
        resources: &BTreeMap<String, String>,
    ) -> Result<Self, HostError> {
        Self::load_with_environment(html_source, resources, None, None)
    }

    pub fn load_with_environment(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
    ) -> Result<Self, HostError> {
        Self::load_with_limits(html_source, resources, broker, base_url, 256 * 1024 * 1024)
    }

    pub fn load_with_limits(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Live(crate::host::DocumentStream::html(Box::new(
                std::iter::once(Ok(html_source.to_owned())),
            ))),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            None,
        )
    }

    pub(crate) fn load_stream_with_limits(
        chunks: crate::host::DocumentStream,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Live(chunks),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            None,
        )
    }

    pub(crate) fn load_parsed_with_limits(
        parsed: ParsedDocument,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Snapshot(Box::new(parsed)),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            None,
        )
    }

    #[cfg(unix)]
    pub(crate) fn load_with_context_and_limits(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        context: (
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        ),
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Live(crate::host::DocumentStream::html(Box::new(
                std::iter::once(Ok(html_source.to_owned())),
            ))),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    #[cfg(unix)]
    pub(crate) fn load_stream_with_context_and_limits(
        chunks: crate::host::DocumentStream,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        context: (
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        ),
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Live(chunks),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    #[cfg(unix)]
    pub(crate) fn load_parsed_with_context_and_limits(
        parsed: ParsedDocument,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        context: (
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        ),
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            DocumentLoad::Snapshot(Box::new(parsed)),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    fn load_with_options(
        input: DocumentLoad,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        #[cfg(unix)] context: Option<(
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        )>,
    ) -> Result<Self, HostError> {
        let mut resources = resources.clone();
        #[cfg(unix)]
        let context_identity = context
            .as_ref()
            .map(|(_, page_id, opener_page_id)| (*page_id, *opener_page_id));
        #[cfg(unix)]
        let context_handle = context.clone();
        let broker_handle = broker.clone();
        let input = match input {
            DocumentLoad::Live(stream) => {
                let essence = stream
                    .content_type
                    .as_deref()
                    .unwrap_or("text/html")
                    .split(';')
                    .next()
                    .unwrap_or("text/html")
                    .trim()
                    .to_ascii_lowercase();
                if essence == "text/html" || oryn_dom::xml::is_text_mime(&essence) {
                    DocumentLoad::Live(stream)
                } else {
                    let mut source = String::new();
                    for chunk in stream.chunks {
                        source.push_str(&chunk?);
                    }
                    let mut parsed = oryn_dom::xml::parse_navigation(
                        &source,
                        Some(&essence),
                        base_url.as_ref().map_or("about:blank", Url::as_str),
                    );
                    if let Some(encoding) = *stream.encoding.lock().expect("navigation encoding") {
                        set_parsed_encoding(&mut parsed, encoding)?;
                    }
                    DocumentLoad::Snapshot(Box::new(parsed))
                }
            }
            other => other,
        };
        let (live_dom, live_source, mut quirks_mode, mut parser_events) = match input {
            DocumentLoad::Live(source) => (
                Rc::new(RefCell::new(DomRuntime::new())),
                Some(source),
                html5ever::interface::QuirksMode::NoQuirks,
                Vec::new(),
            ),
            DocumentLoad::Snapshot(parsed) => {
                let quirks_mode = parsed.quirks_mode;
                let parser_events = parsed.events.clone();
                let runtime = parsed.into_runtime().map_err(|error| {
                    HostError::Execution(format!("failed to initialize Rust DOM: {error}"))
                })?;
                (
                    Rc::new(RefCell::new(runtime)),
                    None,
                    quirks_mode,
                    parser_events,
                )
            }
        };
        let mut scripts = if live_source.is_none() {
            collect_scripts(&live_dom)?
        } else {
            Vec::new()
        };
        let mut frame_sources = if live_source.is_none() {
            collect_frames(&live_dom)?
        } else {
            Vec::new()
        };
        #[cfg(unix)]
        let mut host = match context {
            Some((client, page_id, _opener_page_id)) => {
                RawV8Host::new_with_network_context_and_heap_limit(
                    broker.clone(),
                    base_url.clone(),
                    client,
                    page_id,
                    max_heap_bytes,
                )
            }
            None => match (broker, base_url.clone()) {
                (Some(broker), Some(base_url)) => {
                    RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
                }
                (None, Some(base_url)) => {
                    RawV8Host::new_with_base_url_and_heap_limit(base_url, max_heap_bytes)
                }
                _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
            },
        };
        #[cfg(not(unix))]
        let mut host = match (broker, base_url.clone()) {
            (Some(broker), Some(base_url)) => {
                RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
            }
            (None, Some(base_url)) => {
                RawV8Host::new_with_base_url_and_heap_limit(base_url, max_heap_bytes)
            }
            _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
        };
        if let Some(limit) = resources
            .get("oryn:execution-wall-ms")
            .and_then(|value| value.parse::<u64>().ok())
        {
            host.set_execution_wall_limit(Duration::from_millis(limit));
        }
        host.install_dom(live_dom.clone(), 1)?;
        host.install_layout_provider(Rc::new(LiveLayoutProvider {
            dom: live_dom.clone(),
            viewport: oryn_common::v4::ViewportState::default(),
        }))?;
        if let Some(config) = resources.get("oryn:trusted-types") {
            let config = serde_json::from_str(config).map_err(|error| {
                HostError::Execution(format!("invalid Trusted Types policy snapshot: {error}"))
            })?;
            let allow_eval = resources
                .get("oryn:csp-allow-eval")
                .is_none_or(|value| value == "true");
            host.configure_trusted_types(config, allow_eval)?;
        }
        let top_realm = host.active_realm();
        host.execute(RUST_RUNTIME_GLOBALS)?;
        host.execute(include_str!("native_actions.js"))?;
        host.execute(crate::observable::OBSERVABLE_GLOBALS)?;
        host.set_module_sources(&resources);
        if let Some(base_url) = &base_url {
            // Initial URL hydration is a native commit, not script navigation.
            host.commit_location(base_url.as_str())?;
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &context_handle {
                client
                    .request(crate::context_service::ContextServiceRequest::CommitPage {
                        page_id: *page_id,
                        url: base_url.to_string(),
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
        }
        #[cfg(unix)]
        if let Some((page_id, opener_page_id)) = context_identity {
            host.execute(&format!(
                "__oryn_configure_context({}, {})",
                page_id.0,
                opener_page_id
                    .map(|page_id| page_id.0.to_string())
                    .unwrap_or_else(|| "null".into())
            ))?;
        }
        let local_storage = resources
            .get("oryn:local-storage")
            .map(String::as_str)
            .unwrap_or("[]");
        let session_storage = resources
            .get("oryn:session-storage")
            .map(String::as_str)
            .unwrap_or("[]");
        let local_storage = serde_json::to_string(local_storage)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let session_storage = serde_json::to_string(session_storage)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        host.execute(&format!(
            "__oryn_restore_storage(JSON.parse({local_storage}),JSON.parse({session_storage}))"
        ))?;
        let mut diagnostics = Vec::new();
        let mut loaded_frames = BTreeMap::new();
        if let Some(source) = live_source {
            let LiveParseResult {
                document: parsed,
                scripts: ordered_scripts,
                frames,
            } = parse_live_document(
                source,
                &live_dom,
                &mut host,
                &mut resources,
                &mut diagnostics,
                #[cfg(unix)]
                context_handle.as_ref(),
            )?;
            loaded_frames = frames;
            quirks_mode = parsed.quirks_mode;
            parser_events = parsed.events;
            scripts = ordered_scripts;
            frame_sources = parser_events
                .iter()
                .filter_map(|event| match event {
                    html::ParserEvent::FrameDiscovered { node, source } => {
                        Some((*node, Some(source.clone())))
                    }
                    _ => None,
                })
                .collect();
        } else {
            // Compatibility snapshots cannot pause at their original parser
            // boundary. Keep their established document-order behavior while
            // production source loads use the live parser above.
            scripts.sort_by_key(|script| script.6);
        }
        let mut node_realms = BTreeMap::new();
        let mut frame_realms = BTreeMap::new();
        let mut initial_frame_realms = Vec::new();
        for (frame_index, (frame, source)) in frame_sources.into_iter().enumerate() {
            let Some(source) = source.filter(|source| !source.trim().is_empty()) else {
                continue;
            };
            let requested_url = base_url
                .as_ref()
                .and_then(|base| base.join(&source).ok())
                .or_else(|| Url::parse(&source).ok());
            let mut loaded_frame = loaded_frames
                .get(&source)
                .map(|frame| {
                    (
                        Url::parse(&frame.url).ok().or(requested_url.clone()),
                        frame.source.clone(),
                        frame.content_type.clone(),
                        frame.encoding.clone(),
                    )
                })
                .or_else(|| {
                    resources
                        .get(&source)
                        .cloned()
                        .map(|body| (requested_url.clone(), body, None::<String>, "UTF-8".into()))
                });
            #[cfg(unix)]
            if loaded_frame.is_none()
                && let Some((client, page_id, _)) = &context_handle
            {
                loaded_frame = match client.request(
                    crate::context_service::ContextServiceRequest::LoadPageResource {
                        page_id: *page_id,
                        url: source.clone(),
                        destination: "frame".into(),
                        initiator_url: base_url.as_ref().map(url::Url::to_string),
                        referrer_policy: None,
                        post: None,
                    },
                ) {
                    Ok(crate::context_service::ContextServiceResponse::PageResource {
                        url,
                        source: body,
                        destination,
                        content_type,
                        encoding,
                        ..
                    }) if destination == "frame" => Some((
                        Url::parse(&url).ok().or(requested_url.clone()),
                        body,
                        content_type,
                        encoding,
                    )),
                    _ => None,
                };
            }
            let Some((frame_url, frame_html, content_type, encoding)) = loaded_frame else {
                diagnostics.push(oryn_common::v3::CapabilityDiagnostic {
                    capability: "html.frame.load".into(),
                    support: oryn_common::v3::SupportLevel::Unsupported,
                    alternatives: vec![oryn_common::v3::ExecutionDomain::Chromium],
                    handoff_lossy: true,
                    detail: Some(format!(
                        "frame {source:?} could not be loaded by the parent"
                    )),
                });
                continue;
            };
            let same_origin = base_url
                .as_ref()
                .zip(frame_url.as_ref())
                .is_some_and(|(parent, child)| parent.origin() == child.origin());
            if !same_origin {
                diagnostics.push(oryn_common::v3::CapabilityDiagnostic {
                    capability: "html.frame.cross_origin".into(),
                    support: oryn_common::v3::SupportLevel::Partial,
                    alternatives: Vec::new(),
                    handoff_lossy: false,
                    detail: Some(format!(
                        "frame {source:?} executes in an opaque realm; DOM access and semantic traversal are denied"
                    )),
                });
            }
            let mut child = oryn_dom::xml::parse_navigation(
                &frame_html,
                content_type.as_deref(),
                frame_url.as_ref().map_or("about:blank", Url::as_str),
            );
            set_parsed_encoding(&mut child, &encoding)?;
            let child_referrer_policy = child
                .events
                .iter()
                .filter_map(|event| match event {
                    html::ParserEvent::MetaReferrer { content } => Some(content.as_str()),
                    _ => None,
                })
                .next_back()
                .map(crate::referrer::ReferrerPolicy::parse_list)
                .unwrap_or_default();
            #[cfg(unix)]
            if let (Some((client, page_id, _)), Some(document_url)) =
                (&context_handle, frame_url.as_ref())
            {
                load_frame_style_resources(
                    client,
                    *page_id,
                    document_url.as_str(),
                    child_referrer_policy.as_str(),
                    &frame_html,
                    &child,
                )?;
            }
            let (frame_dom, child_document) = if same_origin {
                let document = live_dom
                    .borrow_mut()
                    .import_subtree(&child.dom, child.document)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                live_dom
                    .borrow_mut()
                    .set_hosted_document(frame, Some(document))
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                (live_dom.clone(), document)
            } else {
                let document = child.document;
                let runtime = child
                    .into_runtime()
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                (Rc::new(RefCell::new(runtime)), document)
            };
            let realm = host.create_realm_with_dom_root(
                frame_url
                    .as_ref()
                    .map(|url| url.origin().ascii_serialization())
                    .unwrap_or_else(|| "null".into()),
                1,
                Some(frame_index as u64 + 1),
                frame_dom.clone(),
                child_document,
            )?;
            if same_origin {
                for node in subtree_nodes(&frame_dom.borrow(), child_document)? {
                    node_realms.insert(node, realm);
                }
            }
            host.activate_realm(realm)?;
            let forms_allowed = live_dom
                .borrow()
                .attribute_in_null_namespace(frame, "sandbox")
                .map_err(|error| HostError::Execution(error.to_string()))?
                .is_none_or(|tokens| {
                    tokens
                        .split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("allow-forms"))
                });
            host.set_forms_allowed(forms_allowed)?;
            host.execute(RUST_RUNTIME_GLOBALS)?;
            host.execute(include_str!("native_actions.js"))?;
            host.execute(crate::observable::OBSERVABLE_GLOBALS)?;
            host.install_layout_provider(Rc::new(LiveLayoutProvider {
                dom: frame_dom.clone(),
                viewport: oryn_common::v4::ViewportState::default(),
            }))?;
            host.set_module_sources(&resources);
            #[cfg(unix)]
            if let Some((page_id, _)) = context_identity {
                configure_frame_context(&mut host, page_id.0)?;
            }
            if let Some(frame_url) = &frame_url {
                host.commit_location(frame_url.as_str())?;
            }
            let committed_url = frame_url
                .as_ref()
                .map_or_else(|| "about:blank".to_owned(), Url::to_string);
            host.execute(&format!(
                "globalThis.__oryn_request_initiator_url={}",
                serde_json::to_string(&committed_url)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            ))?;
            frame_realms.insert(
                realm,
                FrameRealm {
                    parent_realm: top_realm,
                    frame,
                    committed_url,
                },
            );
            let referrer = serde_json::to_string(base_url.as_ref().map_or("", url::Url::as_str))
                .map_err(|error| HostError::Execution(error.to_string()))?;
            host.execute(&format!("globalThis.__oryn_document_referrer={referrer}"))?;
            host.execute(&format!(
                "globalThis.__oryn_referrer_policy={}",
                serde_json::to_string(child_referrer_policy.as_str())
                    .map_err(|error| HostError::Execution(error.to_string()))?
            ))?;
            host.activate_realm(top_realm)?;
            host.ensure_provisional_frame_window(frame)?;
            if same_origin {
                host.set_frame_realm(frame, child_document, realm)?;
            } else {
                host.set_opaque_frame_realm(frame, realm)?;
            }
            host.activate_realm(realm)?;
            let imported_scripts = {
                let dom = frame_dom.borrow();
                dom.query_selector_all(child_document, "script")
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .into_iter()
                    .filter(|script| is_script_element(&dom, *script))
                    .map(|script| {
                        script_record(&dom, script)
                            .map(|record| resolve_script_record(record, frame_url.as_ref()))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            for script in &imported_scripts {
                run_script_record(
                    &mut host,
                    &frame_dom,
                    &mut resources,
                    script,
                    &mut diagnostics,
                    #[cfg(unix)]
                    context_handle.as_ref(),
                )?;
                host.refresh_named_window_properties()?;
            }
            host.set_current_script(None)?;
            host.activate_realm(top_realm)?;
            initial_frame_realms.push(realm);
        }
        for script in &scripts {
            run_script_record(
                &mut host,
                &live_dom,
                &mut resources,
                script,
                &mut diagnostics,
                #[cfg(unix)]
                context_handle.as_ref(),
            )?;
            host.refresh_named_window_properties()?;
        }
        host.set_current_script(None)?;
        host.drain_async_fetches(true)?;
        let parser_local_frames = {
            let dom = live_dom.borrow();
            dom.query_selector_all(dom.document(), "iframe")
                .map_err(|error| HostError::Execution(error.to_string()))?
                .into_iter()
                .filter(|frame| {
                    dom.attribute(*frame, "srcdoc")
                        .is_ok_and(|value| value.is_some())
                        || dom
                            .attribute(*frame, "src")
                            .is_ok_and(|value| value.is_none_or(|src| src.trim().is_empty()))
                })
                .collect::<Vec<_>>()
        };
        let (dom, document) = live_dom.borrow().snapshot();
        let mut page = Self {
            document: ParsedDocument {
                dom,
                document,
                errors: Vec::new(),
                quirks_mode,
                events: parser_events,
            },
            diagnostics,
            agent: crate::agent::AgentRuntime::new(host, live_dom)?,
            top_realm,
            node_realms,
            frame_realms,
            pending_document_loads: BTreeSet::new(),
            pending_frame_navigations: BTreeMap::new(),
            base_url,
            resources,
            broker: broker_handle,
            next_dynamic_request_id: 1,
            semantic_mutation_cursor: MutationCursor::default(),
            #[cfg(unix)]
            context: context_handle,
        };
        for frame in parser_local_frames {
            page.agent.host.activate_realm(page.top_realm)?;
            page.agent.host.ensure_provisional_frame_window(frame)?;
            page.agent.host.schedule_dynamic_frame(frame)?;
        }
        // Parser-discovered frame scripts are prepared before the remaining
        // parent scripts, but their lifecycle fires only after the parent has
        // had a chance to install message/load observers. Each child realm gets
        // its own events and scheduler checkpoint before the top-level load
        // barrier is released.
        for realm in initial_frame_realms {
            // Parent scripts and an earlier child's load handlers may discard
            // any parser-created frame. This list is a scheduling snapshot,
            // not authority to revive a retired document or dispatch its load.
            if !page.agent.host.realm_execution_live(realm) {
                continue;
            }
            page.agent.host.activate_realm(realm)?;
            page.agent.host.prepare_document_images()?;
            page.agent.host.drain_async_fetches(true)?;
            page.queue_document_load(realm)?;
            // A frame's load task may schedule the zero-delay completion task
            // used by testharness and by application bootstraps. Run timers
            // that are due now while the child realm is active; otherwise the
            // top realm can reach its own load barrier with the child remote
            // context permanently awaiting completion.
            page.agent.host.drain_scheduled_timers(false)?;
            page.checkpoint()?;
        }
        page.agent.host.activate_realm(page.top_realm)?;
        // Dynamic external/module scripts inserted by parser-time JavaScript
        // are force-async but still participate in the document load barrier.
        // A parser-time submission may replace a just-inserted frame's initial
        // blank navigation. Resolve those planned actions before firing frame
        // load handlers, matching the ordering used by ordinary checkpoints.
        page.drain_default_actions()?;
        page.drain_frame_location_navigations()?;
        page.drain_dynamic_frames()?;
        page.drain_dynamic_stylesheets()?;
        page.drain_dynamic_scripts()?;
        page.agent.host.prepare_document_images()?;
        page.agent.host.drain_async_fetches(true)?;
        // Zero-delay framework commits queued before load run at this
        // checkpoint. Future timers retain their deadlines and must not be
        // pulled forward merely because navigation reached the load barrier.
        page.agent.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        page.queue_document_load(page.top_realm)?;
        page.agent.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        Ok(page)
    }

    pub fn execute(&mut self, source: &str) -> Result<(), HostError> {
        self.agent.host.execute(source)?;
        self.checkpoint()
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, HostError> {
        if source.contains("__oryn_drain(true)") {
            self.drain_tasks()?;
        }
        let value = self.agent.host.evaluate_string(source)?;
        self.checkpoint()?;
        Ok(value)
    }

    pub fn click(&mut self, selector: &str) -> Result<(), HostError> {
        let selector = serde_json::to_string(selector)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.execute(&format!(
            "const target=document.querySelector({selector});if(!target)throw new Error('no matching element');target.click()"
        ))
    }

    pub fn type_text(&mut self, selector: &str, value: &str) -> Result<(), HostError> {
        let node = {
            let dom = self.agent.dom.borrow();
            dom.query_selector(dom.document(), selector)
        }
        .map_err(|error| HostError::Execution(error.to_string()))?
        .ok_or_else(|| HostError::Execution("no matching element".into()))?;
        self.agent
            .dom
            .borrow_mut()
            .set_user_control_value(node, value, false)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.agent.host.execute_with_node(
            node,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        )?;
        self.checkpoint()
    }

    pub fn location(&mut self) -> Result<String, HostError> {
        self.agent.host.evaluate_string("location.href")
    }

    pub fn set_committed_location(&mut self, url: &str) -> Result<(), HostError> {
        self.agent.host.commit_location(url)
    }

    pub fn take_location_navigation(&self) -> Option<oryn_js::v8_bindings::LocationNavigation> {
        self.agent.host.take_location_navigation()
    }

    pub fn take_history_traversal(&self) -> Result<Option<i64>, HostError> {
        self.agent.host.take_history_traversal()
    }

    pub fn take_history_url_update(&self) -> Option<String> {
        self.agent.host.take_history_url_update()
    }

    pub fn dispatch_page_transition(
        &mut self,
        event_type: &str,
        persisted: bool,
    ) -> Result<(), HostError> {
        let event_type = serde_json::to_string(event_type)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.agent.host.execute(&format!(
            "{{const event=new Event({event_type});Object.defineProperty(event,'persisted',{{value:{persisted},enumerable:true}});dispatchEvent(event)}}"
        ))?;
        self.checkpoint()
    }

    pub fn suspend_for_back_forward_cache(&mut self) {
        self.agent.host.suspend_page_isolate();
    }

    pub fn resume_from_back_forward_cache(&mut self) {
        self.agent.host.resume_page_isolate();
    }

    pub fn has_pending_page_work(&mut self) -> bool {
        !self.pending_document_loads.is_empty() || self.agent.host.has_pending_page_work()
    }

    pub fn document_load_complete(&self) -> bool {
        !self.pending_document_loads.contains(&self.top_realm)
    }

    pub fn has_runnable_page_work(&mut self) -> bool {
        !self.pending_document_loads.is_empty() || self.agent.host.has_runnable_page_work()
    }

    pub fn next_timer_delay(&mut self) -> Option<Duration> {
        self.agent.host.next_timer_delay()
    }

    pub fn storage_snapshot(&mut self) -> Result<(String, String), HostError> {
        Ok((
            self.agent
                .host
                .evaluate_string("JSON.stringify([...localStorage.items])")?,
            self.agent
                .host
                .evaluate_string("JSON.stringify([...sessionStorage.items])")?,
        ))
    }

    pub fn take_console_entries(&mut self) -> Result<Vec<ConsoleEntry>, HostError> {
        let value = self
            .agent
            .host
            .evaluate_string("JSON.stringify(__oryn_take_console())")?;
        serde_json::from_str(&value).map_err(|error| HostError::Execution(error.to_string()))
    }

    pub fn take_context_trace_entries(&mut self) -> Result<Vec<ContextTraceEntry>, HostError> {
        let value = self
            .agent
            .host
            .evaluate_string("JSON.stringify(__oryn_take_context_trace())")?;
        serde_json::from_str(&value).map_err(|error| HostError::Execution(error.to_string()))
    }

    pub fn drain_tasks(&mut self) -> Result<(), HostError> {
        // A page task must never sleep until a future timer deadline. The page
        // actor owns waiting and continues servicing parent-owned context
        // events while timers remain scheduled. Blocking here can strand a
        // popup `postMessage` behind the test/application timeout which is
        // supposed to guard that very workflow.
        self.agent.host.drain_scheduled_timers(false)?;
        self.agent.host.execute("__oryn_poll_context()")?;
        // Animation-frame callbacks are page tasks in their own right. They
        // must advance even when the preceding task changed only platform
        // state (for example scroll offsets) and therefore produced no
        // semantic/layout revision that would otherwise trigger a frame.
        self.agent.host.execute("__oryn_run_animation_frames()")?;
        self.checkpoint()
    }

    pub fn poll_context_events(&mut self) -> Result<(), HostError> {
        self.agent.host.execute("__oryn_poll_context()")?;
        self.checkpoint()
    }

    pub fn apply_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        self.apply_action_with_sensitivity(target, action, value, false)
    }

    pub fn apply_sensitive_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        self.apply_action_with_sensitivity(target, action, value, true)
    }

    pub fn set_files(
        &mut self,
        target: NodeId,
        files: &[crate::upload::UploadFile],
    ) -> Result<(), HostError> {
        let selected = files
            .iter()
            .map(|file| oryn_dom::SelectedFile {
                name: file.filename.clone(),
                media_type: file.media_type.clone(),
                bytes: file.bytes.clone().into(),
                last_modified: 0,
            })
            .collect();
        self.agent
            .dom
            .borrow_mut()
            .set_selected_files(target, std::sync::Arc::new(selected))
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let realm = self
            .node_realms
            .get(&target)
            .copied()
            .unwrap_or(self.top_realm);
        self.agent.host.activate_realm(realm)?;
        let result = self.agent.host.execute_with_node(
            target,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        );
        self.agent.host.activate_realm(self.top_realm)?;
        result?;
        self.checkpoint()
    }

    fn apply_action_with_sensitivity(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
        sensitive: bool,
    ) -> Result<(), HostError> {
        if action == SemanticAction::Submit {
            let realm = self
                .node_realms
                .get(&target)
                .copied()
                .unwrap_or(self.top_realm);
            self.agent.host.activate_realm(realm)?;
            let result = self.agent.host.request_submit(target);
            self.agent.host.activate_realm(self.top_realm)?;
            result?;
            return self.checkpoint();
        }
        let event_source = match action {
            SemanticAction::Click => "",
            SemanticAction::Type | SemanticAction::Clear => {
                self.agent
                    .dom
                    .borrow_mut()
                    .set_user_control_value(
                        target,
                        if action == SemanticAction::Clear {
                            ""
                        } else {
                            value.unwrap_or_default()
                        },
                        sensitive,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Check | SemanticAction::Uncheck => {
                let mut dom = self.agent.dom.borrow_mut();
                dom.set_checked(target, action == SemanticAction::Check)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                "__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Select => {
                let requested = value.unwrap_or_default();
                let mut dom = self.agent.dom.borrow_mut();
                let options = dom
                    .query_selector_all(target, "option")
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let selected = options.iter().copied().find(|option| {
                    dom.attribute(*option, "value")
                        .ok()
                        .flatten()
                        .is_some_and(|candidate| candidate == requested)
                        || dom
                            .arena()
                            .text_content(*option)
                            .is_ok_and(|text| text.trim() == requested)
                });
                let selected =
                    selected.ok_or_else(|| HostError::Execution("no matching option".into()))?;
                for option in options {
                    if option == selected {
                        dom.set_attribute(option, "selected", "")
                    } else {
                        dom.remove_attribute(option, "selected")
                    }
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                "__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Focus => "__oryn_action_target.focus()",
            SemanticAction::Hover => {
                "__oryn_action_target.dispatchEvent(new Event('mouseover',{bubbles:true}))"
            }
            _ => {
                return Err(HostError::Execution(format!(
                    "unsupported native executable action {action:?}"
                )));
            }
        };
        let action_realm = self
            .node_realms
            .get(&target)
            .copied()
            .unwrap_or(self.top_realm);
        self.agent.host.activate_realm(action_realm)?;
        let action_accepted = if action == SemanticAction::Click {
            self.agent.host.evaluate_bool_with_node(
                target,
                "__oryn_action_target",
                "__oryn_action_target.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,composed:true}))",
            )
        } else {
            self.agent
                .host
                .execute_with_node(target, "__oryn_action_target", event_source)
                .map(|()| true)
        };
        self.agent.host.activate_realm(self.top_realm)?;
        let default_accepted = action_accepted?;
        // dispatchEvent already queues anchor/area activation in the generated
        // binding. The checkpoint owns that navigation; doing it here as well
        // creates two browsing contexts for a single target=_blank click.
        let queued_link_activation = action == SemanticAction::Click && {
            let dom = self.agent.dom.borrow();
            dom.node_name(target).is_ok_and(|name| {
                name.eq_ignore_ascii_case("a") || name.eq_ignore_ascii_case("area")
            })
        };
        if default_accepted && !queued_link_activation && action == SemanticAction::Click {
            self.apply_default_navigation(target, action, None, action_realm)?;
        }
        self.checkpoint()
    }

    pub(crate) fn focus_visible(&self) -> bool {
        self.agent.dom.borrow().focus_visible()
    }

    pub(crate) fn populate_semantic_control_state(
        &self,
        graph: &mut crate::semantic::SemanticGraph,
    ) -> Result<(), HostError> {
        graph
            .apply_native_control_state(&self.agent.dom.borrow())
            .map_err(|error| HostError::Execution(error.to_string()))
    }

    pub(crate) fn populate_validation_states(
        &self,
        input: &mut oryn_layout::LayoutInput,
    ) -> Result<(), HostError> {
        populate_validation_states(&self.agent.dom.borrow(), input)
            .map_err(|error| HostError::Execution(error.to_string()))
    }

    pub(crate) fn focused_layout_node(&self) -> Option<u64> {
        self.agent
            .dom
            .borrow()
            .focused()
            .map(|node| (u64::from(node.generation) << 32) | u64::from(node.slot))
    }

    pub fn layout_snapshot(&mut self) -> Result<ParsedDocument, HostError> {
        self.checkpoint()?;
        Ok(self.document.clone())
    }

    /// Return every live-DOM mutation not yet consumed by the semantic graph.
    /// MutationObserver delivery uses a separate journal cursor.
    pub fn take_semantic_mutations(&mut self) -> (Vec<Mutation>, bool) {
        let batch = self
            .agent
            .dom
            .borrow()
            .mutations_since(self.semantic_mutation_cursor);
        self.semantic_mutation_cursor = batch.next;
        (batch.mutations, batch.truncated)
    }

    pub fn install_layout(
        &mut self,
        output: &oryn_layout::LayoutOutput,
        viewport: oryn_common::v4::ViewportState,
    ) -> Result<(), HostError> {
        self.agent
            .host
            .install_layout(binding_layout_from_output(output))?;
        self.agent.host.execute(&format!(
            "globalThis.innerWidth={};globalThis.innerHeight={}",
            viewport.width, viewport.height
        ))?;
        self.agent.host.execute("__oryn_run_geometry_observers()")?;
        self.checkpoint()
    }

    pub fn run_animation_frame_phase(&mut self) -> Result<(), HostError> {
        self.agent.host.execute("__oryn_run_animation_frames()")?;
        self.checkpoint()
    }

    pub fn install_media(
        &mut self,
        color_scheme: &str,
        reduced_motion: &str,
    ) -> Result<(), HostError> {
        let color_scheme = serde_json::to_string(color_scheme)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let reduced_motion = serde_json::to_string(reduced_motion)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.agent.host.execute(&format!(
            "__oryn_media={{color_scheme:{color_scheme},reduced_motion:{reduced_motion}}}"
        ))
    }

    fn queue_document_load(&mut self, realm: RealmId) -> Result<(), HostError> {
        self.agent.host.activate_realm(realm)?;
        self.agent.host.execute(DOCUMENT_CONTENT_LOADED_SCRIPT)?;
        self.pending_document_loads.insert(realm);
        self.flush_document_loads()
    }

    fn flush_document_loads(&mut self) -> Result<(), HostError> {
        let previous_realm = self.agent.host.active_realm();
        let pending = self
            .pending_document_loads
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for realm in pending {
            if !self.agent.host.realm_execution_live(realm) {
                self.pending_document_loads.remove(&realm);
                continue;
            }
            let pending_child = self.pending_document_loads.iter().any(|child| {
                let mut ancestor = *child;
                while let Some(owner) = self.frame_realms.get(&ancestor) {
                    if owner.parent_realm == realm {
                        return true;
                    }
                    ancestor = owner.parent_realm;
                }
                false
            });
            if pending_child || self.agent.host.realm_has_pending_images(realm) {
                continue;
            }
            // Remove before authored handlers run; their navigations and newly
            // inserted frames must not redispatch this document's load event.
            self.pending_document_loads.remove(&realm);
            self.agent.host.activate_realm(realm)?;
            self.agent.host.execute(DOCUMENT_LOAD_SCRIPT)?;
            if let Some(owner) = self.frame_realms.get(&realm).cloned() {
                self.agent.host.activate_realm(owner.parent_realm)?;
                self.agent.host.execute_with_node(
                    owner.frame,
                    "__oryn_dynamic_frame",
                    "__oryn_dynamic_frame.dispatchEvent(new Event('load'))",
                )?;
            }
        }
        self.agent.host.activate_realm(previous_realm)
    }

    fn checkpoint(&mut self) -> Result<(), HostError> {
        // Message delivery, promise jobs, default actions, and dynamic frame
        // loads can enqueue work in a different realm of the same page isolate.
        // Run a bounded page-wide fixed point instead of relying on an
        // unrelated future command to tick that realm.
        for _ in 0..8 {
            self.agent.host.adopt_created_realms();
            for realm in self.agent.host.take_discarded_frame_realms() {
                self.frame_realms.remove(&realm);
                self.node_realms.retain(|_, owner| *owner != realm);
                self.pending_document_loads.remove(&realm);
                self.pending_frame_navigations
                    .retain(|(owner, _), _| *owner != realm);
                if self.agent.host.realm_descriptor(realm).is_some() {
                    self.agent.host.remove_realm(realm)?;
                }
            }
            self.agent.host.drain_async_fetches(false)?;
            self.drain_dynamic_referrer_policies()?;
            self.drain_default_actions()?;
            self.drain_frame_location_navigations()?;
            self.drain_dynamic_frames()?;
            self.drain_dynamic_stylesheets()?;
            self.drain_dynamic_scripts()?;
            self.agent.host.drain_async_fetches(false)?;
            self.flush_document_loads()?;
            for realm in self.agent.host.realm_ids() {
                if !self.agent.host.realm_execution_live(realm) {
                    continue;
                }
                self.agent.host.activate_realm(realm)?;
                self.agent.host.execute("__oryn_drain()")?;
                if !self.agent.host.realm_execution_live(realm) {
                    continue;
                }
                self.agent.host.run_custom_element_reactions()?;
                self.agent.host.run_dom_observer_checkpoint()?;
                for report in self.agent.host.take_module_error_reports() {
                    self.diagnostics
                        .push(script_diagnostic(HostError::Execution(report)));
                }
                for rejection in self.agent.host.take_unhandled_promise_rejections() {
                    self.diagnostics
                        .push(script_diagnostic(HostError::Execution(format!(
                            "unhandled promise rejection: {rejection}"
                        ))));
                }
                self.agent.host.process_binding_releases()?;
            }
        }
        self.agent.host.activate_realm(self.top_realm)?;
        for report in self.agent.host.take_module_error_reports() {
            self.diagnostics
                .push(script_diagnostic(HostError::Execution(report)));
        }
        for rejection in self.agent.host.take_unhandled_promise_rejections() {
            self.diagnostics
                .push(script_diagnostic(HostError::Execution(format!(
                    "unhandled promise rejection: {rejection}"
                ))));
        }
        let (dom, document) = self.agent.dom.borrow().snapshot();
        self.document.dom = dom;
        self.document.document = document;
        Ok(())
    }

    fn drain_dynamic_referrer_policies(&mut self) -> Result<(), HostError> {
        let pending = self.agent.host.take_all_pending_referrer_policies()?;
        #[cfg(unix)]
        if let Some((client, page_id, _)) = &self.context {
            for (realm, policy) in pending {
                if realm != self.top_realm {
                    continue;
                }
                let response = client
                    .request(
                        crate::context_service::ContextServiceRequest::InstallReferrerPolicy {
                            page_id: *page_id,
                            serialized: policy,
                        },
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if !matches!(
                    response,
                    crate::context_service::ContextServiceResponse::DocumentSecurityInstalled
                ) {
                    return Err(HostError::Execution(
                        "parent returned an invalid referrer-policy update response".into(),
                    ));
                }
            }
        }
        #[cfg(not(unix))]
        let _ = pending;
        Ok(())
    }

    fn drain_frame_location_navigations(&mut self) -> Result<(), HostError> {
        let mut navigations = Vec::new();
        for (realm, mut owner) in self.frame_realms.clone() {
            self.agent.host.activate_realm(realm)?;
            // History commits retain the frame's document. Consume these
            // before looking for a Location navigation, including when both
            // operations occurred in the same script task.
            if let Some(url) = self.agent.host.take_history_url_update() {
                owner.committed_url = url;
                self.frame_realms.insert(realm, owner.clone());
            }
            let request = self.agent.host.take_location_navigation();
            let href = request
                .as_ref()
                .map(|request| request.url.clone())
                .unwrap_or(self.agent.host.evaluate_string("location.href")?);
            if let Some(source) = href.strip_prefix("javascript:") {
                execute_javascript_url(&mut self.agent.host, source, &owner.committed_url)?;
                continue;
            }
            if (request.is_some() || href != owner.committed_url)
                && Url::parse(&href).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            {
                let opaque = self
                    .agent
                    .host
                    .evaluate_string("String(globalThis.__oryn_opaque_origin===true)")?
                    == "true";
                let initiator_url = (!opaque)
                    .then(|| {
                        self.agent
                            .host
                            .evaluate_string("globalThis.__oryn_request_initiator_url")
                            .ok()
                    })
                    .flatten()
                    .filter(|value| !value.trim().is_empty());
                let referrer_policy = self
                    .agent
                    .host
                    .evaluate_string("globalThis.__oryn_referrer_policy")
                    .ok()
                    .filter(|value| !value.trim().is_empty());
                let post = request.as_ref().and_then(|request| request.post.clone());
                let referrer_policy = request
                    .and_then(|request| request.referrer_policy)
                    .or(referrer_policy);
                navigations.push((owner, href, initiator_url, referrer_policy, post));
            }
        }
        for (owner, href, initiator_url, referrer_policy, post) in navigations {
            self.agent.host.activate_realm(owner.parent_realm)?;
            let dom = self.agent.host.active_dom().ok_or_else(|| {
                HostError::Execution("frame-navigation parent DOM is unavailable".into())
            })?;
            {
                let mut dom = dom.borrow_mut();
                dom.remove_attribute(owner.frame, "srcdoc")
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                dom.set_attribute(owner.frame, "src", href)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
            self.pending_frame_navigations.insert(
                (owner.parent_realm, owner.frame),
                FrameNavigationContext {
                    initiator_url,
                    referrer_policy,
                    post,
                },
            );
            self.agent.host.reschedule_dynamic_frame(owner.frame)?;
        }
        self.agent.host.activate_realm(self.top_realm)?;
        Ok(())
    }

    fn drain_default_actions(&mut self) -> Result<(), HostError> {
        use oryn_js::v8_bindings::PendingDefaultAction;

        self.agent.host.activate_realm(self.top_realm)?;
        for _ in 0..1024 {
            let pending = self.agent.host.take_pending_default_actions()?;
            if pending.is_empty() {
                return Ok(());
            }
            for (realm, action) in pending {
                self.agent.host.activate_realm(realm)?;
                match action {
                    PendingDefaultAction::Click(target) => {
                        self.apply_default_navigation(
                            target.node,
                            SemanticAction::Click,
                            None,
                            realm,
                        )?;
                    }
                    PendingDefaultAction::Submit(transaction) => {
                        let current_generation = self
                            .agent
                            .host
                            .realm_descriptor(realm)
                            .is_some_and(|descriptor| {
                                descriptor.document_generation == transaction.document_generation
                            });
                        let valid = current_generation
                            && self.agent.host.active_dom().is_some_and(|dom| {
                                let dom = dom.borrow();
                                dom.arena().resolve(transaction.document.root()).is_ok()
                                    && dom.is_connected(transaction.form).unwrap_or(false)
                            });
                        if valid {
                            self.apply_default_navigation(
                                transaction.form,
                                SemanticAction::Submit,
                                Some(&transaction),
                                realm,
                            )?;
                        }
                    }
                }
            }
        }
        Err(HostError::LimitExceeded {
            resource: "default-action queue".into(),
        })
    }

    fn drain_dynamic_stylesheets(&mut self) -> Result<(), HostError> {
        static PROCESSING_INSTRUCTION_HREF: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| {
                regex::Regex::new(r#"(?i)href\s*=\s*[\"']([^\"']+)[\"']"#)
                    .expect("xml-stylesheet href regex")
            });
        for _ in 0..1024 {
            let pending = self.agent.host.take_all_pending_dynamic_stylesheets()?;
            if pending.is_empty() {
                return Ok(());
            }
            for (realm, stylesheet) in pending {
                self.agent.host.activate_realm(realm)?;
                let active_dom = self.agent.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-stylesheet realm DOM is unavailable".into())
                })?;
                let (name, href, css, element_policy) = {
                    let dom = active_dom.borrow();
                    let record = dom
                        .arena()
                        .get(stylesheet)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if let NodeKind::ProcessingInstruction { target, data } = &record.kind {
                        let href = PROCESSING_INSTRUCTION_HREF
                            .captures(data)
                            .map(|capture| {
                                capture[1]
                                    .replace("&amp;", "&")
                                    .replace("&quot;", "\"")
                                    .replace("&apos;", "'")
                            })
                            .unwrap_or_default();
                        (target.clone(), href, String::new(), None)
                    } else {
                        let name = dom
                            .node_name(stylesheet)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        let href = dom
                            .attribute(stylesheet, "href")
                            .map_err(|error| HostError::Execution(error.to_string()))?
                            .map(str::to_owned)
                            .unwrap_or_default();
                        let css = if dom
                            .node_name(stylesheet)
                            .is_ok_and(|name| name.eq_ignore_ascii_case("style"))
                        {
                            dom.arena()
                                .stylesheet_text(stylesheet)
                                .map_err(|error| HostError::Execution(error.to_string()))?
                        } else {
                            [
                                "style",
                                "background",
                                "fill",
                                "stroke",
                                "filter",
                                "clip-path",
                                "mask",
                                "mask-image",
                                "marker-start",
                                "marker-mid",
                                "marker-end",
                            ]
                            .into_iter()
                            .filter_map(|attribute| {
                                dom.attribute(stylesheet, attribute)
                                    .ok()
                                    .flatten()
                                    .map(|value| {
                                        if attribute == "background" {
                                            format!("background-image:url({value})")
                                        } else {
                                            value.to_owned()
                                        }
                                    })
                            })
                            .collect::<Vec<_>>()
                            .join(";")
                        };
                        let element_policy = dom
                            .attribute(stylesheet, "referrerpolicy")
                            .map_err(|error| HostError::Execution(error.to_string()))?
                            .map(str::to_owned)
                            .filter(|value| !value.trim().is_empty());
                        (name, href, css, element_policy)
                    }
                };
                #[cfg(unix)]
                let loaded = if let Some((client, page_id, _)) = &self.context {
                    let document_url = self.agent.host.evaluate_string("location.href")?;
                    let policy = element_policy
                        .or_else(|| {
                            self.agent
                                .host
                                .evaluate_string("globalThis.__oryn_referrer_policy")
                                .ok()
                        })
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "strict-origin-when-cross-origin".into());
                    if name.eq_ignore_ascii_case("link")
                        || name.eq_ignore_ascii_case("xml-stylesheet")
                    {
                        load_parser_stylesheet(
                            client,
                            *page_id,
                            &href,
                            Some(document_url),
                            Some(policy),
                            0,
                        )
                    } else {
                        load_inline_css_resources(client, *page_id, &document_url, &policy, &css)?;
                        Ok((document_url, css))
                    }
                } else {
                    Err(HostError::Execution(
                        "dynamic stylesheet broker is unavailable".into(),
                    ))
                };
                #[cfg(not(unix))]
                let loaded: Result<(String, String), HostError> = Err(HostError::Execution(
                    "dynamic stylesheet broker is unavailable".into(),
                ));
                match loaded {
                    Ok((url, css)) => {
                        self.resources.insert(url, css.clone());
                        if name.eq_ignore_ascii_case("link") {
                            let mut dom = active_dom.borrow_mut();
                            let style = dom.create(NodeKind::Element {
                                name: QualName::new(None, ns!(html), local_name!("style")),
                            });
                            let text = dom.create_text(css);
                            dom.append_child(style, text)
                                .map_err(|error| HostError::Execution(error.to_string()))?;
                            if let Some(parent) = dom
                                .parent(stylesheet)
                                .map_err(|error| HostError::Execution(error.to_string()))?
                            {
                                dom.append_child(parent, style)
                                    .map_err(|error| HostError::Execution(error.to_string()))?;
                            }
                            drop(dom);
                            self.agent.host.execute_with_node(
                                stylesheet,
                                "__oryn_dynamic_stylesheet",
                                "__oryn_dynamic_stylesheet.dispatchEvent(new Event('load'))",
                            )?;
                        }
                    }
                    Err(error) => {
                        self.diagnostics
                            .push(script_diagnostic(HostError::Execution(format!(
                                "dynamic stylesheet failed: {error}"
                            ))));
                        let _ = self.agent.host.execute_with_node(
                            stylesheet,
                            "__oryn_dynamic_stylesheet",
                            "__oryn_dynamic_stylesheet.dispatchEvent(new Event('error'))",
                        );
                    }
                }
            }
        }
        Err(HostError::LimitExceeded {
            resource: "dynamic stylesheet preparation queue".into(),
        })
    }

    fn drain_dynamic_scripts(&mut self) -> Result<(), HostError> {
        // Script execution may itself connect another script. Keep one bounded
        // preparation loop so recursive insertion is deterministic and cannot
        // monopolize the page worker indefinitely.
        for _ in 0..1024 {
            let pending = self.agent.host.take_all_pending_dynamic_scripts()?;
            if pending.is_empty() {
                return Ok(());
            }
            #[cfg(unix)]
            let (completion_sender, completion_receiver) = std::sync::mpsc::channel();
            #[cfg(unix)]
            let mut scheduled = 0usize;
            for (realm, script) in pending {
                self.agent.host.activate_realm(realm)?;
                let active_dom = self.agent.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-script realm DOM is unavailable".into())
                })?;
                let (script_type, src, nonce, inline) = {
                    let dom = active_dom.borrow();
                    let script_type = dom
                        .attribute(script, "type")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .unwrap_or_default()
                        .to_owned();
                    let src = dom
                        .attribute(script, "src")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned);
                    let nonce = dom
                        .attribute(script, "nonce")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned);
                    let inline = dom
                        .arena()
                        .text_content(script)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    (script_type, src, nonce, inline)
                };
                let module = script_type.trim().eq_ignore_ascii_case("module");
                #[cfg(unix)]
                if let (Some(src), Some((client, page_id, _))) = (
                    src.as_deref().filter(|src| !src.trim().is_empty()),
                    &self.context,
                ) {
                    let sender = completion_sender.clone();
                    let client = client.clone();
                    let page_id = *page_id;
                    let src = src.to_owned();
                    let nonce = nonce.clone();
                    let initiator_url = self
                        .agent
                        .host
                        .evaluate_string("globalThis.__oryn_request_initiator_url")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                        .or_else(|| self.agent.host.evaluate_string("location.href").ok());
                    let referrer_policy = active_dom
                        .borrow()
                        .attribute(script, "referrerpolicy")
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                        .filter(|value| !value.trim().is_empty())
                        .or_else(|| {
                            self.agent
                                .host
                                .evaluate_string("globalThis.__oryn_referrer_policy")
                                .ok()
                                .filter(|value| !value.trim().is_empty())
                        });
                    scheduled = scheduled.saturating_add(1);
                    std::thread::spawn(move || {
                        let result = client
                            .request(crate::context_service::ContextServiceRequest::LoadScript {
                                page_id,
                                url: src,
                                module,
                                nonce,
                                initiator_url,
                                referrer_policy,
                            })
                            .map_err(|error| HostError::Execution(error.to_string()))
                            .and_then(|response| match response {
                                crate::context_service::ContextServiceResponse::ScriptSource {
                                    url,
                                    source,
                                    ..
                                } => Ok((url, source)),
                                _ => Err(HostError::Execution(
                                    "parent returned an invalid dynamic-script response".into(),
                                )),
                            });
                        let _ = sender.send((realm, script, module, result));
                    });
                    continue;
                }
                let loaded = self.load_dynamic_script_source(
                    src.as_deref(),
                    module,
                    nonce.as_deref(),
                    &inline,
                );
                self.finish_dynamic_script(script, module, loaded)?;
            }
            #[cfg(unix)]
            for _ in 0..scheduled {
                let (realm, script, module, loaded) = completion_receiver.recv().map_err(|_| {
                    HostError::Execution("dynamic-script scheduler disconnected".into())
                })?;
                self.agent.host.activate_realm(realm)?;
                self.finish_dynamic_script(script, module, loaded)?;
            }
        }
        Err(HostError::LimitExceeded {
            resource: "dynamic script preparation queue".into(),
        })
    }

    fn drain_dynamic_frames(&mut self) -> Result<(), HostError> {
        // A frame load handler can append another frame (the WPT opaque-origin
        // sequence does exactly this from a Promise continuation). Drain to a
        // fixed point in the same page task so its successor does not depend
        // on an unrelated future command ticking the page.
        for _ in 0..1024 {
            let pending = self.agent.host.take_all_pending_dynamic_frames()?;
            if pending.is_empty() {
                return Ok(());
            }
            for (scheduled_realm, frame) in pending {
                let navigation_context = self
                    .pending_frame_navigations
                    .remove(&(scheduled_realm, frame));
                // A same-origin child Document can be accessed through a
                // wrapper in its parent's realm. Nodes created through that
                // wrapper still belong to the child document, so derive the
                // browsing-context owner from the nearest mapped ancestor
                // instead of trusting the realm that happened to enqueue the
                // insertion callback.
                self.agent.host.activate_realm(scheduled_realm)?;
                let scheduled_dom = self.agent.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-frame scheduled DOM is unavailable".into())
                })?;
                let (document_realm, document_referrer_policy) = {
                    let dom = scheduled_dom.borrow();
                    let ancestry = std::iter::once(frame)
                        .chain(
                            dom.arena()
                                .ancestors(frame)
                                .map_err(|error| HostError::Execution(error.to_string()))?,
                        )
                        .collect::<Vec<_>>();
                    let Some(document) = dom
                        .arena()
                        .node_document(frame)
                        .map_err(|error| HostError::Execution(error.to_string()))?
                    else {
                        continue;
                    };
                    let Some(realm) = self.agent.host.realm_for_document(&scheduled_dom, document)
                    else {
                        continue;
                    };
                    let policy = ancestry
                        .iter()
                        .find(|node| {
                            dom.arena()
                                .get(**node)
                                .is_ok_and(|record| matches!(record.kind, NodeKind::Document))
                        })
                        .and_then(|document| dom.query_selector_all(*document, "meta").ok())
                        .and_then(|metas| {
                            metas.into_iter().rev().find_map(|meta| {
                                dom.attribute(meta, "name")
                                    .ok()
                                    .flatten()
                                    .is_some_and(|name| name.eq_ignore_ascii_case("referrer"))
                                    .then(|| {
                                        dom.attribute(meta, "content")
                                            .ok()
                                            .flatten()
                                            .map(str::to_owned)
                                    })
                                    .flatten()
                            })
                        });
                    (realm, policy)
                };
                // A wrapper's creation realm is not the embedding document.
                // Lifecycle, WindowProxy and security ownership all follow the
                // native ownerDocument, including calls through foreign wrappers.
                let parent_realm = document_realm;
                let parent_origin = self
                    .agent
                    .host
                    .realm_descriptor(parent_realm)
                    .ok_or_else(|| HostError::Execution("frame owner realm unavailable".into()))?
                    .origin
                    .clone();
                let replaced_realms = self
                    .frame_realms
                    .iter()
                    .filter_map(|(realm, owner)| {
                        (owner.parent_realm == parent_realm && owner.frame == frame)
                            .then_some(*realm)
                    })
                    .collect::<Vec<_>>();
                self.agent.host.activate_realm(parent_realm)?;
                let parent_dom = self.agent.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-frame realm DOM is unavailable".into())
                })?;
                self.agent.host.activate_realm(document_realm)?;
                let parent_url = self
                    .agent
                    .host
                    .evaluate_string("location.href")
                    .ok()
                    .and_then(|value| Url::parse(&value).ok())
                    .or_else(|| self.base_url.clone());
                let parent_base_url = self
                    .agent
                    .host
                    .evaluate_string("document.baseURI")
                    .ok()
                    .and_then(|value| Url::parse(&value).ok())
                    .or_else(|| parent_url.clone());
                let parent_security_url = self
                    .agent
                    .host
                    .evaluate_string("globalThis.__oryn_request_initiator_url")
                    .ok()
                    .and_then(|value| Url::parse(&value).ok())
                    .or_else(|| parent_url.clone());
                let mut parent_policy = document_referrer_policy.or_else(|| {
                    self.agent
                        .host
                        .evaluate_string("globalThis.__oryn_referrer_policy")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                });
                #[cfg(unix)]
                if parent_policy.is_none()
                    && let Some((client, page_id, _)) = &self.context
                    && let Ok(crate::context_service::ContextServiceResponse::ReferrerPolicy {
                        value,
                    }) = client.request(
                        crate::context_service::ContextServiceRequest::GetReferrerPolicy {
                            page_id: *page_id,
                        },
                    )
                {
                    parent_policy = Some(value);
                }
                self.agent.host.activate_realm(parent_realm)?;
                let (requested, mut srcdoc, opaque_sandbox, forms_allowed, element_referrer_policy) = {
                    let dom = parent_dom.borrow();
                    let requested = dom
                        .attribute(frame, "src")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned)
                        .filter(|value| !value.trim().is_empty());
                    let srcdoc = dom
                        .attribute(frame, "srcdoc")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned);
                    let sandbox = dom
                        .attribute(frame, "sandbox")
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    let opaque_sandbox = sandbox.is_some_and(|tokens| {
                        !tokens
                            .split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("allow-same-origin"))
                    });
                    let forms_allowed = sandbox.is_none_or(|tokens| {
                        tokens
                            .split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("allow-forms"))
                    });
                    let element_referrer_policy = dom
                        .attribute(frame, "referrerpolicy")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned)
                        .filter(|value| !value.trim().is_empty());
                    (
                        requested,
                        srcdoc,
                        opaque_sandbox,
                        forms_allowed,
                        element_referrer_policy,
                    )
                };
                let javascript_url = requested
                    .as_deref()
                    .and_then(|url| url.strip_prefix("javascript:"))
                    .map(str::to_owned);
                let request_referrer_policy = navigation_context
                    .as_ref()
                    .and_then(|context| context.referrer_policy.clone())
                    .or(element_referrer_policy)
                    .or(parent_policy.clone());
                let inherited_about_blank =
                    (requested.is_none() && srcdoc.is_none()) || javascript_url.is_some();
                if inherited_about_blank {
                    // An iframe without src creates an inherited-origin
                    // about:blank document and a fully usable child realm.
                    srcdoc = Some(String::new());
                }
                #[cfg(unix)]
                let fetched = if let Some(source) = srcdoc.clone() {
                    Some((
                        if inherited_about_blank {
                            "about:blank".to_owned()
                        } else {
                            "about:srcdoc".to_owned()
                        },
                        source,
                        parent_url.as_ref().map_or_else(String::new, Url::to_string),
                        parent_policy
                            .clone()
                            .unwrap_or_else(|| "strict-origin-when-cross-origin".into()),
                        None,
                        None::<String>,
                        "UTF-8".to_owned(),
                    ))
                } else if let (Some(requested), Some((client, page_id, _))) =
                    (requested.as_ref(), &self.context)
                {
                    let requested = parent_url
                        .as_ref()
                        .and_then(|base| base.join(requested).ok())
                        .map_or_else(|| requested.clone(), |url| url.to_string());
                    match client.request(
                        crate::context_service::ContextServiceRequest::LoadPageResource {
                            page_id: *page_id,
                            url: requested.clone(),
                            destination: "frame".into(),
                            initiator_url: navigation_context
                                .as_ref()
                                .map(|context| context.initiator_url.clone())
                                .unwrap_or_else(|| parent_url.as_ref().map(Url::to_string)),
                            referrer_policy: request_referrer_policy,
                            post: navigation_context
                                .as_ref()
                                .and_then(|context| context.post.clone()),
                        },
                    ) {
                        Ok(crate::context_service::ContextServiceResponse::PageResource {
                            url,
                            source,
                            destination,
                            document_referrer,
                            referrer_policy,
                            refresh_url,
                            content_type,
                            encoding,
                        }) if destination == "frame" => Some((
                            url,
                            source,
                            document_referrer,
                            referrer_policy,
                            refresh_url,
                            content_type,
                            encoding,
                        )),
                        Ok(_) => {
                            return Err(HostError::Execution(
                                "parent returned an invalid dynamic-frame response".into(),
                            ));
                        }
                        Err(error) => {
                            self.diagnostics
                                .push(script_diagnostic(HostError::Execution(format!(
                                    "dynamic frame {requested:?} failed: {error}"
                                ))));
                            None
                        }
                    }
                } else if let Some(requested) = requested.as_ref() {
                    self.resources.get(requested).cloned().map(|source| {
                        let url = parent_url
                            .as_ref()
                            .and_then(|base| base.join(requested).ok())
                            .map_or_else(|| requested.clone(), |url| url.to_string());
                        (
                            url,
                            source,
                            parent_url.as_ref().map_or_else(String::new, Url::to_string),
                            "strict-origin-when-cross-origin".into(),
                            None,
                            None::<String>,
                            "UTF-8".to_owned(),
                        )
                    })
                } else {
                    None
                };
                #[cfg(not(unix))]
                let fetched = if let Some(source) = srcdoc.clone() {
                    Some((
                        "about:srcdoc".to_owned(),
                        source,
                        parent_url.as_ref().map_or_else(String::new, Url::to_string),
                        parent_policy
                            .clone()
                            .unwrap_or_else(|| "strict-origin-when-cross-origin".into()),
                        None,
                        None::<String>,
                        "UTF-8".to_owned(),
                    ))
                } else if let Some(requested) = requested.as_ref() {
                    self.resources.get(requested).cloned().map(|source| {
                        let url = parent_url
                            .as_ref()
                            .and_then(|base| base.join(requested).ok())
                            .map_or_else(|| requested.clone(), |url| url.to_string());
                        (
                            url,
                            source,
                            parent_url.as_ref().map_or_else(String::new, Url::to_string),
                            "strict-origin-when-cross-origin".into(),
                            None,
                            None::<String>,
                            "UTF-8".to_owned(),
                        )
                    })
                } else {
                    None
                };
                let Some((
                    url,
                    source,
                    document_referrer,
                    mut child_referrer_policy,
                    refresh_url,
                    content_type,
                    encoding,
                )) = fetched
                else {
                    continue;
                };
                let is_srcdoc = srcdoc.is_some() && !inherited_about_blank;
                let frame_url = if inherited_about_blank {
                    parent_security_url.clone()
                } else if is_srcdoc {
                    parent_url.clone()
                } else {
                    Url::parse(&url)
                        .or_else(|_| {
                            parent_url
                                .as_ref()
                                .ok_or(url::ParseError::RelativeUrlWithoutBase)?
                                .join(&url)
                        })
                        .ok()
                };
                // about:blank/srcdoc inherit the owner's origin identity,
                // including an opaque identity; serializing and reparsing
                // `null` would create a different origin.
                let child_origin = if opaque_sandbox {
                    oryn_js::origin::Origin::opaque()
                } else if is_srcdoc || inherited_about_blank {
                    parent_origin.clone()
                } else {
                    oryn_js::origin::Origin::from_url(&url)
                };
                let cross_origin = child_origin != parent_origin;
                if cross_origin {
                    self.diagnostics
                    .push(oryn_common::v3::CapabilityDiagnostic {
                        capability: "html.frame.cross_origin".into(),
                        support: oryn_common::v3::SupportLevel::Partial,
                        alternatives: Vec::new(),
                        handoff_lossy: false,
                        detail: Some(format!(
                            "dynamic frame {url:?} executes in an opaque realm; DOM access and semantic traversal are denied"
                        )),
                    });
                }
                let mut parsed =
                    oryn_dom::xml::parse_navigation(&source, content_type.as_deref(), &url);
                set_parsed_encoding(&mut parsed, &encoding)?;
                if let Some(content) = parsed
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        html::ParserEvent::MetaReferrer { content } => Some(content.as_str()),
                        _ => None,
                    })
                    .next_back()
                {
                    child_referrer_policy = crate::referrer::ReferrerPolicy::parse_list(content)
                        .as_str()
                        .into();
                }
                if opaque_sandbox {
                    child_referrer_policy = "no-referrer".into();
                }
                #[cfg(unix)]
                if let Some((client, page_id, _)) = &self.context {
                    let document_url = frame_url
                        .as_ref()
                        .map_or_else(|| url.clone(), Url::to_string);
                    load_frame_style_resources(
                        client,
                        *page_id,
                        &document_url,
                        &child_referrer_policy,
                        &source,
                        &parsed,
                    )?;
                }
                let (frame_dom, child_document) = if cross_origin {
                    let document = parsed.document;
                    let runtime = parsed
                        .into_runtime()
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    (Rc::new(RefCell::new(runtime)), document)
                } else {
                    let document = parent_dom
                        .borrow_mut()
                        .import_subtree(&parsed.dom, parsed.document)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    (parent_dom.clone(), document)
                };
                let realm = self.agent.host.create_realm_with_dom_root(
                    child_origin.clone(),
                    1,
                    Some(u64::try_from(self.node_realms.len()).unwrap_or(u64::MAX) + 1),
                    frame_dom.clone(),
                    child_document,
                )?;
                if !cross_origin && Rc::ptr_eq(&parent_dom, &self.agent.dom) {
                    for node in subtree_nodes(&frame_dom.borrow(), child_document)? {
                        self.node_realms.insert(node, realm);
                    }
                }
                self.agent.host.activate_realm(realm)?;
                self.agent.host.set_forms_allowed(forms_allowed)?;
                if is_srcdoc || inherited_about_blank {
                    self.agent
                        .host
                        .set_inherited_base_url(parent_base_url.as_ref().map(Url::to_string))?;
                }
                self.agent
                    .host
                    .execute(RUST_RUNTIME_GLOBALS)
                    .map_err(|error| {
                        HostError::Execution(format!("dynamic frame runtime bootstrap: {error}"))
                    })?;
                self.agent.host.execute(include_str!("native_actions.js"))?;
                self.agent
                    .host
                    .execute(crate::observable::OBSERVABLE_GLOBALS)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.agent
                    .host
                    .install_layout_provider(Rc::new(LiveLayoutProvider {
                        dom: frame_dom.clone(),
                        viewport: oryn_common::v4::ViewportState::default(),
                    }))?;
                self.agent.host.set_module_sources(&self.resources);
                #[cfg(unix)]
                if let Some((_, page_id, _)) = &self.context {
                    configure_frame_context(&mut self.agent.host, page_id.0).map_err(|error| {
                        HostError::Execution(format!("dynamic frame context: {error}"))
                    })?;
                }
                let committed_url = if inherited_about_blank {
                    "about:blank".to_owned()
                } else if is_srcdoc {
                    "about:srcdoc".to_owned()
                } else {
                    frame_url
                        .as_ref()
                        .map_or_else(|| url.clone(), Url::to_string)
                };
                if frame_url.is_some() {
                    self.agent
                        .host
                        .commit_location(&committed_url)
                        .map_err(|error| {
                            HostError::Execution(format!("dynamic frame location: {error}"))
                        })?;
                }
                let referrer = serde_json::to_string(&document_referrer)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.agent
                    .host
                    .execute(&format!("globalThis.__oryn_document_referrer={referrer}"))?;
                let child_referrer_policy = serde_json::to_string(&child_referrer_policy)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.agent.host.execute(&format!(
                    "globalThis.__oryn_referrer_policy={child_referrer_policy}"
                ))?;
                let request_initiator = serde_json::to_string(&if inherited_about_blank {
                    "about:blank".to_owned()
                } else {
                    frame_url.as_ref().map_or_else(String::new, Url::to_string)
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
                self.agent.host.execute(&format!(
                    "globalThis.__oryn_request_initiator_url={request_initiator}"
                ))?;
                if opaque_sandbox {
                    self.agent.host
                        .execute("globalThis.__oryn_security_origin='null';globalThis.__oryn_opaque_origin=true")
                        .map_err(|error| {
                            HostError::Execution(format!("dynamic frame opaque origin: {error}"))
                        })?;
                } else if is_srcdoc || inherited_about_blank {
                    let frame_origin = serde_json::to_string(&child_origin.serialize())
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.agent.host
                        .execute(&format!(
                            "globalThis.__oryn_security_origin={frame_origin};globalThis.__oryn_opaque_origin={}",
                            child_origin.is_opaque()
                        ))
                        .map_err(|error| {
                            HostError::Execution(format!("dynamic srcdoc origin: {error}"))
                        })?;
                }
                // Retire the old document only after a replacement has loaded
                // and bootstrapped. A navigation with no response must leave the
                // active realm and its document intact.
                let replacing_document = !replaced_realms.is_empty();
                for realm in replaced_realms {
                    self.frame_realms.remove(&realm);
                    self.agent.host.remove_realm(realm)?;
                    self.node_realms
                        .retain(|_, node_realm| *node_realm != realm);
                }
                if replacing_document {
                    parent_dom
                        .borrow_mut()
                        .set_hosted_document(frame, None)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                if !cross_origin {
                    parent_dom
                        .borrow_mut()
                        .set_hosted_document(frame, Some(child_document))
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                // Establish parent/top and the frame's contentWindow before child
                // script execution. Frame scripts are allowed to synchronously
                // observe their parent relationship while booting.
                self.agent.host.activate_realm(parent_realm)?;
                self.agent.host.ensure_provisional_frame_window(frame)?;
                if cross_origin {
                    self.agent.host.set_opaque_frame_realm(frame, realm)?;
                } else {
                    self.agent
                        .host
                        .set_frame_realm(frame, child_document, realm)?;
                }
                self.agent.host.activate_realm(realm)?;
                if let Some(source) = javascript_url.as_deref() {
                    execute_javascript_url(&mut self.agent.host, source, &committed_url)?;
                }
                let child_scripts = if javascript_url.is_some() {
                    // `execute_javascript_url` commits its string completion
                    // through document.open/write/close; close synchronously
                    // executes parser-inserted scripts. Do not run the
                    // committed document a second time in the normal frame
                    // bootstrap scan.
                    Vec::new()
                } else {
                    let dom = frame_dom.borrow();
                    dom.query_selector_all(child_document, "script")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .into_iter()
                        .filter(|script| is_script_element(&dom, *script))
                        .map(|script| {
                            let record = dom.arena().get(script)?;
                            Ok(resolve_script_record(
                                (
                                    script,
                                    record.attributes.get_null("src").cloned(),
                                    record.attributes.get_null("type").cloned(),
                                    dom.arena().text_content(script)?,
                                    false,
                                    record.attributes.contains_null("nomodule"),
                                    false,
                                ),
                                frame_url.as_ref(),
                            ))
                        })
                        .collect::<Result<Vec<ScriptRecord>, DomError>>()
                        .map_err(|error| HostError::Execution(error.to_string()))?
                };
                for script in &child_scripts {
                    run_script_record(
                        &mut self.agent.host,
                        &frame_dom,
                        &mut self.resources,
                        script,
                        &mut self.diagnostics,
                        #[cfg(unix)]
                        self.context.as_ref(),
                    )?;
                    self.agent.host.refresh_named_window_properties()?;
                }
                self.agent.host.activate_realm(parent_realm)?;
                self.agent.host.activate_realm(realm)?;
                self.agent.host.prepare_document_images()?;
                self.agent.host.drain_async_fetches(true)?;
                self.agent.host.activate_realm(parent_realm)?;
                self.frame_realms.insert(
                    realm,
                    FrameRealm {
                        parent_realm,
                        frame,
                        committed_url,
                    },
                );
                self.queue_document_load(realm)?;
                self.agent.host.activate_realm(parent_realm)?;
                if let Some(refresh_url) = refresh_url {
                    self.agent.host.activate_realm(realm)?;
                    let refresh_url = serde_json::to_string(&refresh_url)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.agent
                        .host
                        .execute(&format!("setTimeout(()=>location.href={refresh_url},0)"))?;
                    self.agent.host.activate_realm(parent_realm)?;
                }
            }
        }
        Err(HostError::LimitExceeded {
            resource: "dynamic frame preparation queue".into(),
        })
    }

    fn finish_dynamic_script(
        &mut self,
        script: NodeId,
        module: bool,
        loaded: Result<(String, String), HostError>,
    ) -> Result<(), HostError> {
        let element_referrer_policy = self
            .agent
            .host
            .active_dom()
            .and_then(|dom| {
                dom.borrow()
                    .attribute(script, "referrerpolicy")
                    .ok()
                    .flatten()
                    .map(str::to_owned)
            })
            .filter(|value| !value.trim().is_empty());
        let result = loaded.and_then(|(resource_name, source)| {
            if module {
                #[cfg(unix)]
                if let Some((client, page_id, _)) = &self.context {
                    preload_module_graph(
                        &mut self.agent.host,
                        client,
                        *page_id,
                        &resource_name,
                        &source,
                        &mut self.resources,
                    )?;
                }
                self.agent.host.execute_module_script(
                    &resource_name,
                    &source,
                    &self.resources,
                    true,
                )
            } else {
                let previous_policy = self
                    .agent
                    .host
                    .evaluate_string("globalThis.__oryn_referrer_policy")
                    .unwrap_or_default();
                if let Some(policy) = &element_referrer_policy {
                    let policy = serde_json::to_string(policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.agent
                        .host
                        .execute(&format!("globalThis.__oryn_referrer_policy={policy}"))?;
                }
                let result = self.agent.host.execute(&source);
                if element_referrer_policy.is_some() {
                    let previous = serde_json::to_string(&previous_policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.agent
                        .host
                        .execute(&format!("globalThis.__oryn_referrer_policy={previous}"))?;
                }
                result
            }
        });
        match result {
            Ok(()) => self.agent.host.execute_with_node(
                script,
                "__oryn_dynamic_script",
                "__oryn_dynamic_script.dispatchEvent(new Event('load'))",
            ),
            Err(error) => {
                self.diagnostics
                    .push(script_diagnostic(HostError::Execution(format!(
                        "dynamic script failed: {error}"
                    ))));
                // Resource/script errors are reported to the element and
                // diagnostics but do not turn appendChild into an exception
                // after it has already returned.
                let _ = self.agent.host.execute_with_node(
                    script,
                    "__oryn_dynamic_script",
                    "__oryn_dynamic_script.dispatchEvent(new Event('error'))",
                );
                Ok(())
            }
        }
    }

    fn load_dynamic_script_source(
        &mut self,
        src: Option<&str>,
        module: bool,
        nonce: Option<&str>,
        inline: &str,
    ) -> Result<(String, String), HostError> {
        let Some(src) = src.filter(|src| !src.trim().is_empty()) else {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &self.context {
                client
                    .request(
                        crate::context_service::ContextServiceRequest::AuthorizeInlineScript {
                            page_id: *page_id,
                            source: inline.to_owned(),
                            nonce: nonce.map(str::to_owned),
                        },
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
            return Ok(("oryn:dynamic-inline-module".into(), inline.to_owned()));
        };

        #[cfg(unix)]
        if let Some((client, page_id, _)) = &self.context {
            let initiator_url = self
                .agent
                .host
                .evaluate_string("globalThis.__oryn_request_initiator_url")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| self.agent.host.evaluate_string("location.href").ok());
            let referrer_policy = self
                .agent
                .host
                .evaluate_string("globalThis.__oryn_referrer_policy")
                .ok()
                .filter(|value| !value.trim().is_empty());
            let response = client
                .request(crate::context_service::ContextServiceRequest::LoadScript {
                    page_id: *page_id,
                    url: src.to_owned(),
                    module,
                    nonce: nonce.map(str::to_owned),
                    initiator_url,
                    referrer_policy,
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let crate::context_service::ContextServiceResponse::ScriptSource {
                url, source, ..
            } = response
            else {
                return Err(HostError::Execution(
                    "parent returned an invalid dynamic-script response".into(),
                ));
            };
            return Ok((url, source));
        }

        let resolved = self
            .base_url
            .as_ref()
            .and_then(|base| base.join(src).ok())
            .or_else(|| Url::parse(src).ok());
        if let Some(source) = self.resources.get(src).cloned().or_else(|| {
            resolved
                .as_ref()
                .and_then(|url| self.resources.get(url.as_str()).cloned())
        }) {
            return Ok((
                resolved.map_or_else(|| src.to_owned(), |url| url.to_string()),
                source,
            ));
        }
        let url = resolved.ok_or_else(|| {
            HostError::Execution(format!("dynamic script URL {src:?} is invalid"))
        })?;
        let broker = self.broker.as_ref().ok_or_else(|| {
            HostError::Execution(format!("dynamic script resource {url} is unavailable"))
        })?;
        let request_id = self.next_dynamic_request_id;
        self.next_dynamic_request_id = request_id.saturating_add(1);
        let response = crate::network::collect_operation_bounded(
            broker,
            crate::network::NetworkRequest {
                request_id,
                method: "GET".into(),
                url: url.to_string(),
                headers: vec![(
                    "accept".into(),
                    "text/javascript,application/javascript,*/*;q=0.1".into(),
                )],
                body: Vec::new(),
                credentials: true,
                security: crate::cookies::CookieRequestContext {
                    kind: crate::cookies::CookieRequestKind::Script,
                    initiator_url: self.base_url.as_ref().map(ToString::to_string),
                    top_level_url: self.base_url.as_ref().map(ToString::to_string),
                    ..Default::default()
                },
                resolved_addrs: Vec::new(),
            },
            std::time::Duration::from_secs(30),
            8 * 1024 * 1024,
        )
        .map_err(|error| HostError::Execution(error.to_string()))?;
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        let (source, _) = oryn_net::decode_resource_text(
            &response.body,
            content_type,
            oryn_net::TextResourceKind::JavaScript,
        );
        Ok((url.to_string(), source))
    }

    pub fn node_by_id(&self, id: &str) -> Option<NodeId> {
        self.agent.dom.borrow().find_by_id(id)
    }

    fn apply_default_navigation(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        submission: Option<&oryn_js::v8_bindings::submissions::SubmissionTransaction>,
        source_realm: oryn_js::realm::RealmId,
    ) -> Result<(), HostError> {
        self.agent.host.activate_realm(source_realm)?;
        let source_dom = self.agent.host.active_dom().ok_or_else(|| {
            HostError::Execution("default-action realm DOM is unavailable".into())
        })?;
        self.agent.host.activate_realm(source_realm)?;
        let (href, target_name, navigation_referrer_policy) = if let Some(transaction) = submission
        {
            (
                transaction.navigation.url.clone(),
                transaction.target.clone(),
                transaction.navigation.referrer_policy.clone(),
            )
        } else {
            let dom = source_dom.borrow();
            let mut candidate = Some(target);
            let mut result = None;
            while let Some(node) = candidate {
                let record = dom
                    .arena()
                    .get(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let NodeKind::Element { name } = &record.kind {
                    // A label or checkable input owns its click activation.
                    // Forwarding a label click to its control does not also
                    // activate a surrounding hyperlink (for either dispatch).
                    if action == SemanticAction::Click
                        && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && (matches!(name.local.as_ref(), "label" | "button")
                            || (name.local.as_ref() == "input"
                                && record.attributes.get_null("type").is_some_and(|kind| {
                                    kind.eq_ignore_ascii_case("checkbox")
                                        || kind.eq_ignore_ascii_case("radio")
                                        || kind.eq_ignore_ascii_case("submit")
                                        || kind.eq_ignore_ascii_case("image")
                                })))
                    {
                        break;
                    }
                    if matches!(name.local.as_ref(), "a" | "area")
                        && action == SemanticAction::Click
                        && let Some(href) = record.attributes.get_null("href")
                    {
                        result = Some((
                            href.clone(),
                            record
                                .attributes
                                .get_null("target")
                                .cloned()
                                .unwrap_or_default(),
                            record
                                .attributes
                                .get_null("referrerpolicy")
                                .cloned()
                                .or_else(|| {
                                    record
                                        .attributes
                                        .get_null("rel")
                                        .is_some_and(|value| {
                                            value.split_ascii_whitespace().any(|token| {
                                                token.eq_ignore_ascii_case("noreferrer")
                                            })
                                        })
                                        .then(|| "no-referrer".into())
                                }),
                        ));
                        break;
                    }
                }
                candidate = record.parent;
            }
            let Some(result) = result else {
                return Ok(());
            };
            result
        };
        let resolved = self
            .agent
            .host
            .evaluate_string("location.href")
            .ok()
            .and_then(|value| Url::parse(&value).ok())
            .or_else(|| self.base_url.clone())
            .as_ref()
            .and_then(|base| base.join(&href).ok())
            .or_else(|| Url::parse(&href).ok())
            .map(|url| url.to_string())
            .unwrap_or(href);
        let post = submission.and_then(|transaction| transaction.navigation.post.clone());
        // GET form authorization is independent of target transport. POST
        // frames/popups are authorized by their parent-owned body loader.
        if submission.is_some()
            && post.is_none()
            && !self.authorize_form_navigation(&resolved, "GET")?
        {
            return Ok(());
        }
        if target_name.eq_ignore_ascii_case("_blank") {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &self.context {
                let response = client
                    .request(crate::context_service::ContextServiceRequest::OpenPage {
                        opener_page_id: *page_id,
                        url: Some(resolved),
                        post: submission
                            .and_then(|transaction| transaction.navigation.post.clone()),
                        referrer_policy: navigation_referrer_policy,
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                match response {
                    crate::context_service::ContextServiceResponse::PageOpened {
                        page_id: opened_page_id,
                    } => {
                        self.agent.host.execute(&format!(
                            "__oryn_context_trace.push({{kind:'page_created',detail:{{page_id:{},opener_page_id:{}}}}})",
                            opened_page_id.0, page_id.0
                        ))?;
                    }
                    crate::context_service::ContextServiceResponse::PageDenied { reason } => {
                        let reason = serde_json::to_string(&reason)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        self.agent.host.execute(&format!(
                            "__oryn_context_trace.push({{kind:'capability_failure',detail:{{capability:'popup.open',detail:{reason}}}}})"
                        ))?;
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        if !target_name.is_empty()
            && !matches!(
                target_name.to_ascii_lowercase().as_str(),
                "_self" | "_parent" | "_top"
            )
        {
            let target_frame = {
                let dom = source_dom.borrow();
                dom.arena().iter().find_map(|(frame, record)| {
                    let is_iframe = matches!(
                        &record.kind,
                        NodeKind::Element { name } if name.local.as_ref() == "iframe" && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    );
                    (is_iframe
                        && dom.is_connected(frame).unwrap_or(false)
                        && record
                            .attributes
                            .get_null("name")
                            .or_else(|| record.attributes.get_null("id"))
                            .is_some_and(|name| name == &target_name))
                    .then_some(frame)
                })
            };
            if let Some(frame) = target_frame {
                source_dom
                    .borrow_mut()
                    .remove_attribute(frame, "srcdoc")
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                source_dom
                    .borrow_mut()
                    .set_attribute(frame, "src", &resolved)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let Some(policy) = &navigation_referrer_policy {
                    source_dom
                        .borrow_mut()
                        .set_attribute(frame, "referrerpolicy", policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                self.agent.host.activate_realm(source_realm)?;
                self.pending_frame_navigations.insert(
                    (source_realm, frame),
                    FrameNavigationContext {
                        initiator_url: self.base_url.as_ref().map(ToString::to_string),
                        referrer_policy: navigation_referrer_policy,
                        post: submission
                            .and_then(|transaction| transaction.navigation.post.clone()),
                    },
                );
                self.agent.host.reschedule_dynamic_frame(frame)?;
                return Ok(());
            }
        }
        let destination_realm = match target_name.to_ascii_lowercase().as_str() {
            "_top" => self.top_realm,
            "_parent" => self
                .frame_realms
                .get(&source_realm)
                .map_or(source_realm, |owner| owner.parent_realm),
            _ => source_realm,
        };
        if post.is_some()
            && destination_realm == self.top_realm
            && !self.authorize_form_navigation(&resolved, "POST")?
        {
            return Ok(());
        }
        self.agent.host.activate_realm(destination_realm)?;
        self.agent
            .host
            .schedule_navigation(oryn_js::v8_bindings::LocationNavigation {
                url: resolved,
                replace: false,
                reload: false,
                post,
                referrer_policy: navigation_referrer_policy,
            })
    }

    fn authorize_form_navigation(&mut self, url: &str, method: &str) -> Result<bool, HostError> {
        #[cfg(unix)]
        if let Some((client, page_id, _)) = &self.context {
            match client
                .request(
                    crate::context_service::ContextServiceRequest::AuthorizeFormSubmission {
                        page_id: *page_id,
                        url: url.into(),
                        method: method.into(),
                    },
                )
                .map_err(|error| HostError::Execution(error.to_string()))?
            {
                crate::context_service::ContextServiceResponse::FormSubmissionAuthorized => {}
                crate::context_service::ContextServiceResponse::PageDenied { reason } => {
                    self.diagnostics
                        .push(script_diagnostic(HostError::Execution(reason)));
                    return Ok(false);
                }
                _ => {
                    return Err(HostError::Execution(
                        "invalid form authorization response".into(),
                    ));
                }
            }
        }
        #[cfg(not(unix))]
        let _ = (url, method);
        Ok(true)
    }
}

fn script_diagnostic(error: HostError) -> oryn_common::v3::CapabilityDiagnostic {
    oryn_common::v3::CapabilityDiagnostic {
        capability: "javascript.execution".into(),
        support: oryn_common::v3::SupportLevel::Partial,
        alternatives: vec![
            oryn_common::v3::ExecutionDomain::Chromium,
            oryn_common::v3::ExecutionDomain::Webkit,
            oryn_common::v3::ExecutionDomain::UserBrowser,
        ],
        handoff_lossy: true,
        detail: Some(error.to_string()),
    }
}

fn script_diagnostic_at(error: HostError, resource: &str) -> oryn_common::v3::CapabilityDiagnostic {
    script_diagnostic(HostError::Execution(format!("{resource}: {error}")))
}

/// Frame transport shares its page's context-service ID, but its security
/// origin is the native realm's origin, not the top-level page's origin.
#[cfg(unix)]
fn configure_frame_context(host: &mut RawV8Host, page_id: u64) -> Result<(), HostError> {
    let origin = &host
        .realm_descriptor(host.active_realm())
        .ok_or_else(|| HostError::Execution("frame realm metadata is unavailable".into()))?
        .origin;
    let origin = serde_json::to_string(&origin.serialize())
        .map_err(|error| HostError::Execution(error.to_string()))?;
    host.execute(&format!(
        "__oryn_configure_context({page_id},null,{origin})"
    ))
}

fn execute_javascript_url(
    host: &mut RawV8Host,
    source: &str,
    committed_url: &str,
) -> Result<(), HostError> {
    host.commit_location(committed_url)?;
    let source = source.trim();
    let evaluation = if let Some(literal) = source
        .strip_prefix('\'')
        .and_then(|literal| literal.strip_suffix('\''))
    {
        let literal = literal
            .replace("\\/", "/")
            .replace("\\'", "'")
            .replace("\\n", "\n")
            .replace("\\r", "\r");
        let literal = serde_json::to_string(&literal)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        format!("const __html={literal};")
    } else {
        let source = serde_json::to_string(source)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        format!("const __html=(0,eval)({source});")
    };
    host.execute(&format!(
        "{{{evaluation}if(typeof __html==='string'){{document.open();document.write(__html);document.close()}}}}"
    ))
}

fn subtree_nodes(dom: &DomRuntime, root: NodeId) -> Result<Vec<NodeId>, HostError> {
    let mut nodes = vec![root];
    let mut index = 0;
    while index < nodes.len() {
        let children = dom
            .arena()
            .get(nodes[index])
            .map_err(|error| HostError::Execution(error.to_string()))?
            .children
            .clone();
        nodes.extend(children);
        index += 1;
    }
    Ok(nodes)
}

fn binding_rule_view(rule: oryn_layout::cssom::RuleView) -> oryn_js::v8_bindings::BindingRuleView {
    oryn_js::v8_bindings::BindingRuleView {
        rule_type: rule.rule_type,
        css_text: rule.css_text,
        selector_text: rule.selector_text,
        declaration_text: rule.declaration_text,
        children: rule.children.into_iter().map(binding_rule_view).collect(),
        properties: rule.properties,
        conditions: rule.conditions,
        media_queries: rule.media_queries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_element_connections_follow_detached_subtrees_and_reconnection() {
        let mut page = RustExecutableDocument::load(r#"<body><script>
          globalThis.connections=[];
          customElements.define('connection-probe', class extends HTMLElement {
            connectedCallback(){connections.push('connected');if(!this.shadowRoot)this.attachShadow({mode:'open'}).innerHTML='<button>Ready</button>'}
            disconnectedCallback(){connections.push('disconnected')}
          });
          const container=document.createElement('section');
          const component=document.createElement('connection-probe');
          container.appendChild(component);
          if(connections.length)throw Error('connected while detached');
          document.body.appendChild(container);
          if(!component.shadowRoot)throw Error('missing subtree connection');
          container.remove();document.body.appendChild(container);
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(connections)").unwrap(),
            r#"["connected","disconnected","connected"]"#
        );
    }

    #[test]
    fn native_custom_element_registry_owns_callbacks_attributes_and_adoption() {
        let mut page = RustExecutableDocument::load(r#"<body><script>
          globalThis.events=[];
          class NativeLifecycle extends HTMLElement {
            static get observedAttributes(){return new Set(['value'])}
            connectedCallback(){events.push('connected')}
            disconnectedCallback(){events.push('disconnected')}
            attributeChangedCallback(name,oldValue,newValue,namespace){events.push([name,oldValue,newValue,namespace])}
            adoptedCallback(oldDocument,newDocument){events.push(['adopted',oldDocument===document,newDocument===other])}
          }
          customElements.define('native-lifecycle',NativeLifecycle);
          globalThis.other=document.implementation.createHTMLDocument('other');
          const component=new NativeLifecycle();
          if(component.localName!=='native-lifecycle')throw Error('construction identity');
          component.__orynConnected=true; component.__orynCustomDefinition=NativeLifecycle;
          NativeLifecycle.prototype.connectedCallback=()=>{throw Error('callbacks must be captured at define')};
          globalThis.__oryn_run_custom_reactions=()=>{throw Error('author checkpoint override')};
          component.setAttribute('value','a'); component.setAttribute('ignored','x');
          document.body.append(component);
          other.adoptNode(component);other.body.append(component);component.remove();
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["value",null,"a",null],"connected","disconnected",["adopted",true,true],"connected","disconnected"]"#
        );
    }

    #[test]
    fn custom_element_upgrade_preserves_existing_wrapper_and_private_fields() {
        let mut page = RustExecutableDocument::load(r#"<held-element></held-element><script>
            const held=document.querySelector('held-element');
            held.expando={retained:true};
            const map=new WeakMap([[held,42]]);
            class Held extends HTMLElement {
                #value=7;
                constructor(){super();this.seenSame=this===held}
                value(){return this.#value}
            }
            customElements.define('held-element',Held);
            globalThis.result=[held===document.querySelector('held-element'),held instanceof Held,
                held.seenSame,held.value(),held.expando.retained,map.get(document.querySelector('held-element'))];
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(result)").unwrap(),
            "[true,true,true,7,true,42]"
        );
    }

    #[test]
    fn custom_element_fragment_upgrades_interleave_with_parent_reactions() {
        let mut page = RustExecutableDocument::load(r#"<body><main></main><script>
            globalThis.events=[];
            class Parent extends HTMLElement {
                constructor(){super();events.push('parent-constructor')}
                connectedCallback(){
                    events.push('parent-connected');
                    const child=this.firstChild;
                    this.removeChild(child);
                    this.appendChild(child);
                }
            }
            class Child extends HTMLElement {
                constructor(){super();events.push('child-constructor')}
                connectedCallback(){events.push('child-connected')}
                disconnectedCallback(){events.push('child-disconnected')}
            }
            customElements.define('queued-parent',Parent);
            customElements.define('queued-child',Child);
            document.querySelector('main').innerHTML='<queued-parent><queued-child></queued-child></queued-parent>';
            events.push('returned');
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"["parent-constructor","parent-connected","child-constructor","child-connected","returned"]"#
        );
    }

    #[test]
    fn custom_element_nested_removal_flushes_only_enqueued_element_reactions() {
        let mut page = RustExecutableDocument::load(
            r#"<body><script>
          globalThis.events=[];
          class Parent extends HTMLElement {
            connectedCallback(){events.push('begin');this.firstChild.remove();events.push('end')}
          }
          class Child extends HTMLElement {
            connectedCallback(){events.push('connected')}
            disconnectedCallback(){events.push('disconnected')}
          }
          class QuietChild extends HTMLElement {
            connectedCallback(){events.push('connected')}
          }
          customElements.define('parent-element',Parent);
          customElements.define('child-element',Child);
          customElements.define('quiet-child',QuietChild);
          const first=new Parent();first.append(new Child());document.body.append(first);
          events.push('next');
          const second=new Parent();second.append(new QuietChild());document.body.append(second);
        </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"["begin","connected","disconnected","end","next","begin","end","connected"]"#
        );
    }

    #[test]
    fn custom_element_implicit_adoption_reports_descendants_before_connection() {
        let mut page = RustExecutableDocument::load(r#"<body><script>
          globalThis.events=[];
          globalThis.other=document.implementation.createHTMLDocument('other');
          class Adopted extends HTMLElement {
            adoptedCallback(oldDocument,newDocument){events.push([this.id,'adopted',oldDocument===document,newDocument===other])}
            connectedCallback(){events.push([this.id,'connected'])}
            disconnectedCallback(){events.push([this.id,'disconnected'])}
          }
          customElements.define('adopted-element',Adopted);
          const container=document.createElement('section');
          const a=document.createElement('adopted-element');a.id='a';container.append(a);
          const host=document.createElement('div');container.append(host);
          const b=document.createElement('adopted-element');b.id='b';host.attachShadow({mode:'closed'}).append(b);
          document.body.append(container);events=[];
          other.body.append(container);
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["a","disconnected"],["a","adopted",true,true],["a","connected"],["b","disconnected"],["b","adopted",true,true],["b","connected"]]"#
        );
    }

    #[test]
    fn native_custom_element_registry_rejects_invalid_names_and_reentrant_definition() {
        let mut page = RustExecutableDocument::load(
            r#"<script>
          globalThis.events=[];
          customElements.whenDefined('invalid').catch(error=>events.push(error.name));
          const pending=customElements.whenDefined('valid-element');
          if(pending!==customElements.whenDefined('valid-element'))throw Error('promise identity');
          class Valid extends HTMLElement {}
          Object.defineProperty(Valid.prototype,'connectedCallback',{get(){
            try { customElements.define('nested-element',class extends HTMLElement {}); }
            catch(error){events.push(error.name)}
            return undefined;
          }});
          customElements.define('valid-element',Valid);
          pending.then(constructor=>events.push(constructor===Valid));
          customElements.define('after-element',class extends HTMLElement {});
          events.push(customElements.get('nested-element')===undefined);
          events.push(!!customElements.get('after-element'));
        </script>"#,
        )
        .unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"["NotSupportedError",true,true,"SyntaxError",true]"#
        );
    }

    #[test]
    fn message_channel_delivers_cloned_fifo_tasks_with_microtask_checkpoints() {
        let mut page = RustExecutableDocument::load(r#"<script>
          globalThis.order = [];
          const channel = new MessageChannel();
          channel.port1.onmessage = event => {
            if (!(event instanceof MessageEvent) || !event.isTrusted || event.target !== channel.port1 || event.currentTarget !== channel.port1)
              throw Error('invalid native message event');
            if (event.origin !== '' || event.source !== null || event.ports.length !== 0) throw Error('message metadata');
            order.push(event.data.value);
            queueMicrotask(() => order.push('microtask'));
          };
          const value = {value: 'first'};
          channel.port2.postMessage(value); value.value = 'mutated';
          channel.port2.postMessage({value: 'second'});
          order.push('sync');
        </script>"#).unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(order)").unwrap(),
            r#"["sync","first","microtask","second","microtask"]"#
        );
    }

    #[test]
    fn message_channel_start_close_and_transfer_validation() {
        let mut page = RustExecutableDocument::load(
            r#"<script>
          globalThis.messages = [];
          globalThis.channel = new MessageChannel();
          channel.port1.addEventListener('message', event => messages.push(event.data));
          channel.port2.postMessage('queued');
        </script>"#,
        )
        .unwrap();
        assert_eq!(page.evaluate("JSON.stringify(messages)").unwrap(), "[]");
        page.evaluate("channel.port1.start(); 'started'").unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(messages)").unwrap(),
            r#"["queued"]"#
        );
        page.evaluate(r#"(() => {
          channel.port2.postMessage('inflight'); channel.port1.close(); channel.port1.start();
          channel.port2.postMessage('disconnected');
          let errors = 0;
          try { channel.port2.postMessage(()=>{}); } catch(e) { if(e.name==='DataCloneError')errors++; }
          const bytes = new ArrayBuffer(4);
          try { channel.port2.postMessage('bad', [bytes, bytes]); } catch(e) { if(e.name==='DataCloneError')errors++; }
          if(errors!==2 || bytes.byteLength!==4)throw Error('validation/atomicity');
          return 'ok';
        })()"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(messages)").unwrap(),
            r#"["queued","inflight"]"#
        );
    }

    #[test]
    fn message_channel_transfers_buffer_and_preserves_graphs() {
        let mut page = RustExecutableDocument::load(r#"<script>
          globalThis.result = 'pending';
          const channel = new MessageChannel();
          channel.port1.onmessage = event => {
            const value = event.data;
            result = String(value.self === value && value.map.get('bytes') === value.bytes && value.bytes[0] === 42);
          };
          const bytes = new Uint8Array([42]);
          const value = {bytes, map:new Map([['bytes',bytes]])}; value.self = value;
          channel.port2.postMessage(value, {transfer:[bytes.buffer]});
          if(bytes.byteLength!==0)throw Error('buffer not transferred');
        </script>"#).unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(page.evaluate("result").unwrap(), "true");
    }

    #[test]
    fn message_channel_bounds_paused_queue() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        assert_eq!(page.evaluate(r#"(() => {
          const channel = new MessageChannel();
          for(let i=0;i<1024;i++)channel.port2.postMessage(i);
          let bounded=false;
          try { channel.port2.postMessage(1024); } catch(e) { bounded=e.name==='QuotaExceededError'; }
          if(!bounded)throw Error('queue not bounded');
          channel.port1.close(); channel.port2.postMessage('closed');
          return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn message_channel_port_transfer_moves_queue_and_revokes_old_wrapper() {
        let mut page = RustExecutableDocument::load(r#"<script>
          globalThis.received = [];
          const channel = new MessageChannel();
          channel.port2.postMessage('before transfer');
          const moved = structuredClone({port:channel.port1}, {transfer:[channel.port1]}).port;
          channel.port1.close(); channel.port1.start(); channel.port1.postMessage('old authority');
          moved.onmessage = e => received.push(e.data);
          channel.port2.postMessage('after transfer');
          let rejected=false;
          try{structuredClone(channel.port1,{transfer:[channel.port1]})}catch(e){rejected=e.name==='DataCloneError'}
          if(!rejected)throw Error('detached port transferable');
        </script>"#).unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(received)").unwrap(),
            r#"["before transfer","after transfer"]"#
        );
    }

    #[test]
    fn message_channel_transferred_ports_are_native_event_capabilities() {
        let mut page = RustExecutableDocument::load(r#"<script>
          globalThis.received = [];
          const transport = new MessageChannel(), payload = new MessageChannel();
          payload.port2.postMessage('queued payload');
          transport.port1.onmessage = e => {
            if(e.ports.length!==1 || !Object.isFrozen(e.ports) || e.data.port!==e.ports[0])throw Error('transferred identity');
            e.ports[0].onmessage = event => received.push(event.data);
            e.ports[0].postMessage('reply');
          };
          payload.port2.onmessage = e => received.push(e.data);
          transport.port2.postMessage({port:payload.port1}, [payload.port1]);
        </script>"#).unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(received)").unwrap(),
            r#"["queued payload","reply"]"#
        );
    }

    #[test]
    fn message_channel_rejects_self_transfer_and_preserves_cross_channel_order() {
        let mut page = RustExecutableDocument::load(
            r#"<script>
          globalThis.received = [];
          const a=new MessageChannel(), b=new MessageChannel();
          a.port1.onmessage=e=>received.push(e.data); b.port1.onmessage=e=>received.push(e.data);
          a.port2.postMessage(1);a.port2.postMessage(2);b.port2.postMessage(3);
          let rejected=false;
          try{a.port2.postMessage(null,[a.port2])}catch(e){rejected=e.name==='DataCloneError'}
          if(!rejected)throw Error('self transfer permitted');
          const doomed=new MessageChannel();
          doomed.port2.postMessage(null,[doomed.port1]);
          doomed.port2.postMessage('lost');
        </script>"#,
        )
        .unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(received)").unwrap(),
            "[1,2,3]"
        );
    }

    #[test]
    fn script_text_uses_native_children_and_prepares_author_code_once() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"(() => {
            const script = document.createElement('script');
            script.appendChild(document.createTextNode('first'));
            const nested = document.createElement('span');
            nested.textContent = 'excluded';
            script.appendChild(nested);
            script.appendChild(document.createComment('excluded'));
            script.appendChild(document.createTextNode('last'));
            if (script.text !== 'firstlast') throw Error('not child text content');
            globalThis.scriptTextRuns = 0;
            script.text = 'globalThis.scriptTextRuns++';
            if (script.childNodes.length !== 1 || script.textContent !== script.text)
                throw Error('text did not replace children');
            if (script.getAttribute('text') !== null) throw Error('text reflected as attribute');
            document.body.appendChild(script);
            script.text = 'globalThis.scriptTextRuns += 100';
            script.remove(); document.body.appendChild(script);
            if (scriptTextRuns !== 1) throw Error('script execution count');
            return 'ok';
        })()"#
            )
            .unwrap(),
            "ok"
        );
    }

    #[test]
    fn script_text_enforces_trusted_script_before_mutating_children() {
        let config = oryn_js::trusted_types::TrustedTypesConfig {
            require_script: true,
            allow_any_policy: true,
            ..Default::default()
        };
        let resources = BTreeMap::from([(
            "oryn:trusted-types".into(),
            serde_json::to_string(&config).unwrap(),
        )]);
        let mut page =
            RustExecutableDocument::load_with_resources("<body></body>", &resources).unwrap();
        assert_eq!(
            page.evaluate(
                r#"(() => {
            const script = document.createElement('script');
            script.appendChild(document.createTextNode('original'));
            let rejected = false;
            try { script.text = 'untrusted'; } catch (_) { rejected = true; }
            if (!rejected || script.text !== 'original') throw Error('untrusted mutation');
            const policy = trustedTypes.createPolicy('author', {createScript: value => value});
            script.text = policy.createScript('globalThis.trustedScriptTextRan = true');
            document.body.appendChild(script);
            return String(globalThis.trustedScriptTextRan);
        })()"#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn author_prepares_script_from_unescaped_data_block_inner_html() {
        let mut page = RustExecutableDocument::load(r#"
            <body>
            <script id="source" type="text/author">globalThis.authored = (() => 1 < 2 && 3 > 2)();</script>
            <script>
              const source = document.getElementById('source');
              if (source.innerHTML !== source.textContent) throw Error('raw script text changed');
              const prepared = document.createElement('script');
              prepared.textContent = source.innerHTML;
              document.body.appendChild(prepared);
            </script>
        "#).unwrap();
        assert_eq!(
            page.evaluate("String(globalThis.authored)").unwrap(),
            "true"
        );
    }

    #[test]
    fn author_data_scripts_are_inert_until_author_prepares_javascript() {
        let mut page = RustExecutableDocument::load(r#"
            <script>
              globalThis.dataExecuted = 0;
              Object.defineProperty(globalThis, 'Babel', {get(){throw Error('browser must not invoke author transpiler')}});
            </script>
            <script type="text/babel">dataExecuted++</script>
            <script type="application/json">{"not":"javascript"}</script>
            <script type="text/javascript; charset=utf-8">dataExecuted++</script>
            <script>globalThis.classicExecuted = true;</script>
        "#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify([dataExecuted,classicExecuted])")
                .unwrap(),
            "[0,true]"
        );
        assert_eq!(
            page.evaluate(
                r#"(() => {
            const script = document.createElement('script');
            script.type = 'text/babel'; script.textContent = 'dataExecuted += 100';
            document.body.appendChild(script);
            if (dataExecuted !== 0) throw Error('dynamic data executed');
            script.remove(); script.type = 'text/javascript';
            script.textContent = 'dataExecuted += 1';
            document.body.appendChild(script);
            if (dataExecuted !== 1) throw Error('author-prepared classic did not execute');
            script.remove(); document.body.appendChild(script);
            if (dataExecuted !== 1) throw Error('classic script executed twice');
            return 'ok';
        })()"#
            )
            .unwrap(),
            "ok"
        );
    }

    #[test]
    #[ignore = "requires ORYN_WPT_ROOT pointing at the pinned WPT checkout"]
    fn pinned_wpt_range_clone_executes_with_resource_diagnostics() {
        let root = std::path::PathBuf::from(std::env::var("ORYN_WPT_ROOT").unwrap());
        let mut html = std::fs::read_to_string(root.join("dom/ranges/Range-cloneContents.html"))
            .unwrap()
            .replace("<script src=/resources/testharnessreport.js></script>", "");
        let completion = r#"<pre id="oryn-wpt-result"></pre><script>
            globalThis.__oryn_wpt_results=[];
            add_result_callback(test=>__oryn_wpt_results.push({name:String(test.name),status:Number(test.status),message:String(test.message||'')}));
            add_completion_callback((tests,harness)=>document.getElementById('oryn-wpt-result').textContent=JSON.stringify({harness:harness.status,tests:__oryn_wpt_results}));
            </script>"#;
        let harness_end =
            html.find("testharness.js></script>").unwrap() + "testharness.js></script>".len();
        html.insert_str(harness_end, completion);
        let mut resources = BTreeMap::from([
            (
                "/resources/testharness.js".into(),
                std::fs::read_to_string(root.join("resources/testharness.js")).unwrap(),
            ),
            (
                "../common.js".into(),
                std::fs::read_to_string(root.join("dom/common.js")).unwrap(),
            ),
            (
                "Range-test-iframe.html".into(),
                std::fs::read_to_string(root.join("dom/ranges/Range-test-iframe.html")).unwrap(),
            ),
            ("oryn:execution-wall-ms".into(), "5000".into()),
        ]);
        // Child-frame script records resolve against their document URL. Supply
        // canonical fixture keys as well as the parent's literal relative URLs.
        let base = Url::parse("https://wpt.test/dom/ranges/Range-cloneContents.html").unwrap();
        for relative in [
            "/resources/testharness.js",
            "../common.js",
            "Range-test-iframe.html",
        ] {
            resources.insert(
                base.join(relative).unwrap().to_string(),
                resources[relative].clone(),
            );
        }
        let page = RustExecutableDocument::load_with_environment(
            &html,
            &resources,
            None,
            Some(
                Url::parse("https://wpt.test/dom/ranges/Range-cloneContents.html?subtest=61")
                    .unwrap(),
            ),
        )
        .unwrap();
        assert!(page.diagnostics.is_empty(), "{:#?}", page.diagnostics);
        let result = page
            .agent
            .dom
            .borrow()
            .find_by_id("oryn-wpt-result")
            .and_then(|node| page.agent.dom.borrow().arena().text_content(node).ok())
            .unwrap_or_default();
        assert!(
            !result.is_empty(),
            "WPT did not publish a completion result"
        );
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["harness"], 0, "{result:#}");
        assert!(
            result["tests"]
                .as_array()
                .is_some_and(|tests| tests.iter().all(|test| test["status"] == 0)),
            "{result:#}"
        );
    }

    #[test]
    fn structured_clone_uses_native_serialization_in_page_runtime() {
        let mut page=RustExecutableDocument::load(r#"<output id='result'></output><script>
          const buffer=new ArrayBuffer(8), bytes=new Uint8Array(buffer);bytes[0]=42;
          const value={buffer,bytes};value.self=value;
          const copy=structuredClone(value,{transfer:[buffer]});
          let rejected=false;try{structuredClone(document)}catch(error){rejected=error instanceof DOMException&&error.name==='DataCloneError'}
          document.getElementById('result').textContent=[copy.self===copy,copy.buffer===copy.bytes.buffer,copy.bytes[0],buffer.byteLength,rejected].join('|');
        </script>"#).unwrap();
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate("document.getElementById('result').textContent")
                .unwrap(),
            "true|true|42|0|true"
        );
    }

    #[test]
    fn message_event_uses_native_payload_and_global_dispatch() {
        let mut page=RustExecutableDocument::load(r#"<output id='result'></output><script>
          const payload={value:42},port=new MessageChannel().port1;
          addEventListener('message',event=>{
            document.getElementById('result').textContent=[event instanceof MessageEvent,event instanceof Event,event.data===payload,event.source===window,event.ports[0]===port,Object.isFrozen(event.ports),event.isTrusted].join('|');
          });
          dispatchEvent(new MessageEvent('message',{data:payload,source:window,ports:[port]}));
        </script>"#).unwrap();
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate("document.getElementById('result').textContent")
                .unwrap(),
            "true|true|true|true|true|true|false"
        );
        assert_eq!(page.evaluate("(()=>{const e=document.createEvent('MessageEvent');e.initMessageEvent('legacy',false,false,1,'origin','id',window,[]);return [e.type,e.data,e.origin,e.lastEventId,e.source===window,e.ports.length].join('|')})()").unwrap(),"legacy|1|origin|id|true|0");
    }

    #[test]
    fn scripts_and_events_mutate_only_the_rust_dom() {
        let mut page = RustExecutableDocument::load(
            "<main id='root'></main><script>const button=document.createElement('button');button.id='go';button.addEventListener('click',()=>button.textContent='done');document.getElementById('root').appendChild(button)</script>",
        )
        .unwrap();
        page.click("#go").unwrap();
        let button = page.node_by_id("go").unwrap();
        assert_eq!(page.document.dom.text_content(button).unwrap(), "done");
    }

    #[test]
    fn nomodule_fallback_is_skipped_when_v8_supports_modules() {
        let mut page = RustExecutableDocument::load(
            "<output id='result'>modern</output><script nomodule>document.getElementById('result').textContent='legacy'</script>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.getElementById('result').textContent")
                .unwrap(),
            "modern"
        );
    }

    #[test]
    fn module_scripts_run_after_later_parser_time_classic_scripts() {
        let mut page = RustExecutableDocument::load(
            "<script type='module'>globalThis.moduleResult=globalThis.runtimeConfig</script><main></main><script>globalThis.runtimeConfig='ready'</script>",
        )
        .unwrap();
        assert_eq!(page.evaluate("moduleResult").unwrap(), "ready");
    }

    #[test]
    fn import_map_native_parser_aliases_dynamic_import_and_base() {
        let resources = BTreeMap::from([
            (
                "https://example.test/assets/shared.js".into(),
                "globalThis.instances=(globalThis.instances||0)+1;export const value=42".into(),
            ),
            (
                "https://example.test/assets/lazy.js".into(),
                "import {value} from '#shared';export default value+1".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(r##"<base href="/assets/">
          <script>Object.defineProperty(document,'baseURI',{value:'https://wrong.test/'});globalThis.answer='pending';</script>
          <script type="importmap">{"imports":{"#shared":"./shared.js","lazy":"./lazy.js"}}</script>
          <script type="module">import {value} from '#shared';globalThis.staticValue=value;import('lazy').then(m=>globalThis.answer=m.default)</script>"##,
            &resources, None, Some(Url::parse("https://example.test/page").unwrap())).unwrap();
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate("staticValue+'|'+answer+'|'+instances")
                .unwrap(),
            "42|43|1"
        );
    }

    #[test]
    fn import_map_native_does_not_alias_equal_relative_paths_between_directories() {
        let resources = BTreeMap::from([
            (
                "https://example.test/a/main.js".into(),
                "import {value} from './dep.js';globalThis.a=value".into(),
            ),
            (
                "https://example.test/b/main.js".into(),
                "import {value} from './dep.js';globalThis.b=value".into(),
            ),
            (
                "https://example.test/a/dep.js".into(),
                "export const value='a'".into(),
            ),
            (
                "https://example.test/b/dep.js".into(),
                "export const value='b'".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment("<script type=module src='https://example.test/a/main.js'></script><script type=module src='https://example.test/b/main.js'></script>", &resources, None, Some(Url::parse("https://example.test/").unwrap())).unwrap();
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        assert_eq!(page.evaluate("a+'|'+b").unwrap(), "a|b");
    }

    #[test]
    fn import_map_native_csp_denial_does_not_install_mapping() {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[("content-security-policy".into(), "script-src 'none'".into())],
        );
        let mut resources = BTreeMap::new();
        resources.insert(
            "oryn:csp-policy".into(),
            serde_json::to_string(&policy).unwrap(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            r##"<script type=importmap>{"imports":{"#secret":"./secret.js"}}</script>"##,
            &resources,
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        assert!(!page.diagnostics.is_empty());
        assert_eq!(
            page.agent
                .host
                .resolve_module_specifier("https://example.test/", "#secret")
                .unwrap(),
            None
        );
    }

    #[test]
    fn import_map_native_frame_maps_do_not_leak_to_parent() {
        let resources = BTreeMap::from([
            (
                "https://example.test/parent.js".into(),
                "export const value='parent'".into(),
            ),
            (
                "https://example.test/child.js".into(),
                "export const value='child'".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(r#"<script type=importmap>{"imports":{"shared":"https://example.test/parent.js"}}</script>
          <iframe srcdoc='<script type=importmap>{"imports":{"shared":"https://example.test/child.js"}}</script><script type=module>import {value} from "shared";globalThis.answer=value</script>'></iframe>
          <script type=module>import {value} from 'shared';globalThis.answer=value</script>"#, &resources, None, Some(Url::parse("https://example.test/").unwrap())).unwrap();
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate("answer+'|'+frames[0].answer").unwrap(),
            "parent|child"
        );
    }

    #[test]
    fn import_map_native_event_handler_resolution_and_typed_rejection() {
        let resources = BTreeMap::from([
            (
                "https://example.test/assets/shared.js".into(),
                "export const value=42".into(),
            ),
            (
                "https://example.test/assets/protected.js".into(),
                "globalThis.integrityBypassed=true;export const value=1".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(r#"<base href='/assets/'>
          <script type=importmap>{"imports":{"shared":"./shared.js"},"integrity":{"./protected.js":"sha256-required"}}</script>
          <button id=run onclick="import('shared').then(m=>globalThis.answer=m.value);import('./protected.js').catch(e=>globalThis.blocked=e instanceof TypeError);import('unmapped').catch(e=>globalThis.typed=e instanceof TypeError)">Run</button>"#, &resources, None, Some(Url::parse("https://example.test/").unwrap())).unwrap();
        page.click("#run").unwrap();
        assert_eq!(
            page.evaluate("answer+'|'+blocked+'|'+typed+'|'+typeof integrityBypassed")
                .unwrap(),
            "42|true|true|undefined"
        );
    }

    #[test]
    fn history_state_is_available_to_framework_routers() {
        let mut page = RustExecutableDocument::load("<main></main>").unwrap();
        assert_eq!(
            page.evaluate(
                "history.replaceState({route:'ready'},'', '?view=ready');history.pushState({route:'next'},'', '#next');history.length+'|'+history.state.route+'|'+location.search+'|'+location.hash"
            )
            .unwrap(),
            "2|next|?view=ready|#next"
        );
    }

    #[test]
    fn url_preserves_bare_query_until_search_params_are_mutated() {
        let mut page = RustExecutableDocument::load("<main></main>").unwrap();
        assert_eq!(
            page.evaluate(
                "const url=new URL('https://example.test/case?1-1');const before=url.search+'|'+url.href;url.searchParams.append('next','');before+'|'+url.search+'|'+url.href"
            )
            .unwrap(),
            "?1-1|https://example.test/case?1-1|?1-1=&next=|https://example.test/case?1-1=&next="
        );
    }

    #[test]
    fn editing_command_state_reports_paragraph_and_alignment_values() {
        let mut page = RustExecutableDocument::load(
            "<div id='editor' contenteditable>text</div><script>const editor=document.getElementById('editor');const selection=getSelection();selection.selectAllChildren(editor)</script>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                "const before=document.queryCommandValue('defaultParagraphSeparator')+'|'+document.queryCommandValue('justifyCenter')+'|'+document.queryCommandState('justifyLeft')+'|'+document.queryCommandValue('styleWithCSS');const paragraph=document.execCommand('defaultParagraphSeparator',false,'p');const centered=document.execCommand('justifyCenter',false,'');before+'|'+paragraph+'|'+centered+'|'+document.queryCommandValue('defaultParagraphSeparator')+'|'+document.queryCommandValue('justifyCenter')+'|'+document.queryCommandState('justifyCenter')"
            )
            .unwrap(),
            "div|left|true||true|true|p|center|true"
        );
    }

    #[test]
    fn justification_wraps_blocks_and_separates_merged_inline_runs() {
        let mut page =
            RustExecutableDocument::load("<div id=editor contenteditable><xmp>text</xmp></div>")
                .unwrap();
        assert_eq!(page.evaluate(
            "const editor=document.getElementById('editor');getSelection().selectAllChildren(editor.firstChild);document.execCommand('justifyCenter');[editor.firstChild.localName,editor.firstChild.style.textAlign,editor.firstChild.firstChild.localName,getSelection().toString()].join('|')"
        ).unwrap(), "div|center|xmp|text");
        assert_eq!(page.evaluate(
            "editor.innerHTML='<div style=\"text-align:center\">one</div>two<div style=\"text-align:center\">three</div>';getSelection().selectAllChildren(editor.childNodes[1]);document.execCommand('justifyCenter');[editor.children.length,editor.firstChild.innerHTML,document.queryCommandState('justifyCenter')].join('|')"
        ).unwrap(), "1|one<br>two<br>three|true");
    }

    #[test]
    fn justification_table_align_does_not_set_cell_text_alignment() {
        let mut page = RustExecutableDocument::load(
            "<div contenteditable><table id=table align=justify><tr><td id=cell>text</td></tr></table></div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const table=document.getElementById('table'),cell=document.getElementById('cell');getSelection().selectAllChildren(cell);const initial=document.queryCommandValue('justifyFull');table.style.textAlign='justify';[initial,document.queryCommandValue('justifyFull')].join('|')"
        ).unwrap(), "left|justify");
        assert_eq!(page.evaluate(
            "table.style.textAlign='';cell.setAttribute('align','JuStIfY');document.queryCommandValue('justifyFull')"
        ).unwrap(), "justify");
    }

    #[test]
    fn justification_queries_use_computed_block_alignment_and_direction() {
        let mut page = RustExecutableDocument::load(
            "<style>#editor {text-align:start;direction:rtl} #inline {text-align:center}</style><div id=editor contenteditable><span id=inline>text</span></div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const editor=document.getElementById('editor');getSelection().selectAllChildren(document.getElementById('inline').firstChild);const before=document.queryCommandValue('justifyLeft');editor.style.textAlign='end';const after=document.queryCommandValue('justifyRight');editor.style.direction='ltr';[before,after,document.queryCommandValue('justifyRight')].join('|')"
        ).unwrap(), "right|left|right");
    }

    #[test]
    fn justification_skips_aligned_content_and_updates_all_selected_blocks() {
        let mut page = RustExecutableDocument::load(
            "<div id=editor contenteditable> <span contenteditable=false>A</span> ; <span contenteditable=false>B</span> ; <span contenteditable=false>C</span> </div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const editor=document.getElementById('editor');getSelection().selectAllChildren(editor);const before=editor.innerHTML;document.execCommand('justifyLeft');editor.innerHTML===before"
        ).unwrap(), "true");
        assert_eq!(page.evaluate(
            "editor.innerHTML='<p id=left>one</p><p id=right style=\"text-align:right\">two</p>';const left=document.getElementById('left'),right=document.getElementById('right');getSelection().setBaseAndExtent(left.firstChild,0,right.firstChild,3);const beforeText=getSelection().toString();document.execCommand('justifyLeft');[left.hasAttribute('style'),right.hasAttribute('style'),getComputedStyle(right).textAlign,document.queryCommandValue('justifyLeft'),getSelection().toString()===beforeText].join('|')"
        ).unwrap(), "false|false|start|left|true");
    }

    #[test]
    fn justification_noop_succeeds_and_emits_input_without_an_undo_entry() {
        let mut page =
            RustExecutableDocument::load("<div id=editor contenteditable>text</div>").unwrap();
        assert_eq!(page.evaluate(
            "const editor=document.getElementById('editor');getSelection().selectAllChildren(editor);let events=[];editor.addEventListener('input',event=>events.push(event.inputType));const result=document.execCommand('justifyLeft');[result,events.join(','),document.queryCommandEnabled('undo'),editor.innerHTML].join('|')"
        ).unwrap(), "true|formatJustifyLeft|false|text");
    }

    #[test]
    fn justification_queries_cover_mixed_selection_and_target_table_cells() {
        let mut page = RustExecutableDocument::load(
            "<div id=editor contenteditable><p id=first style='text-align:center'>one</p><p id=last>two</p><table><tr><td id=cell>cell</td></tr></table></div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const first=document.getElementById('first'),last=document.getElementById('last');getSelection().setBaseAndExtent(last.firstChild,3,first.firstChild,0);[document.queryCommandState('justifyCenter'),document.queryCommandIndeterm('justifyCenter'),document.queryCommandValue('justifyCenter')].join('|')"
        ).unwrap(), "false|true|center");
        assert_eq!(page.evaluate(
            "const cell=document.getElementById('cell');getSelection().selectAllChildren(cell);document.execCommand('justifyCenter');[getComputedStyle(cell).textAlign,document.queryCommandState('justifyCenter')].join('|')"
        ).unwrap(), "center|true");
    }

    #[test]
    fn justification_block_extension_ignores_hidden_blocks() {
        let mut page = RustExecutableDocument::load(
            "<div id=editor contenteditable><p style='text-align:center'>one<span id=hidden style='display:none;text-align:right'>hidden</span><span id=caret>two</span></p><p id=next>next</p></div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const caret=document.getElementById('caret');getSelection().collapse(caret.firstChild,1);[document.queryCommandValue('justifyCenter'),document.queryCommandState('justifyCenter'),document.queryCommandIndeterm('justifyCenter')].join('|')"
        ).unwrap(), "center|true|false");
        assert_eq!(page.evaluate(
            "getSelection().setBaseAndExtent(caret.firstChild,1,document.getElementById('next').firstChild,1);[document.queryCommandState('justifyCenter'),document.queryCommandIndeterm('justifyCenter')].join('|')"
        ).unwrap(), "false|true");
    }

    #[test]
    fn justification_presentational_hints_inherit_and_yield_to_author_css() {
        let mut page = RustExecutableDocument::load(
            "<style>.override {text-align:right}</style><div id=editor contenteditable align=JuStIfY><p id=child>text</p></div>",
        ).unwrap();
        assert_eq!(page.evaluate(
            "const editor=document.getElementById('editor'),child=document.getElementById('child');getSelection().selectAllChildren(child);const initial=document.queryCommandValue('justifyFull');editor.className='override';[initial,getComputedStyle(child).textAlign,document.queryCommandValue('justifyRight')].join('|')"
        ).unwrap(), "justify|right|right");
        assert_eq!(page.evaluate(
            "editor.className='';editor.setAttribute('align','CENTER');document.queryCommandState('justifyCenter')"
        ).unwrap(), "true");
    }

    #[test]
    fn justification_preserves_backward_parent_offset_selection() {
        let mut page = RustExecutableDocument::load(
            "<div id='editor' contenteditable>one<span>two</span></div>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                "const editor=document.getElementById('editor');const selection=getSelection();selection.setBaseAndExtent(editor,2,editor,0);const before=selection.toString();document.execCommand('justifyRight');const wrapper=editor.firstChild;[before,selection.toString(),selection.anchorNode===wrapper,selection.anchorOffset,selection.focusNode===wrapper,selection.focusOffset,document.queryCommandValue('justifyRight')].join('|')"
            )
            .unwrap(),
            "onetwo|onetwo|true|2|true|0|right"
        );
    }

    #[test]
    fn navigator_and_performance_are_available_for_framework_feature_detection() {
        let mut page = RustExecutableDocument::load("<main></main>").unwrap();
        assert_eq!(
            page.evaluate(
                "navigator.language+'|'+navigator.languages.join(',')+'|'+navigator.onLine+'|'+(performance.now()>=0)"
            )
            .unwrap(),
            "en-US|en-US,en|true|true"
        );
    }

    #[test]
    fn window_scroll_offsets_track_scroll_operations() {
        let mut page = RustExecutableDocument::load("<main></main>").unwrap();
        assert_eq!(
            page.evaluate(
                "scrollTo({left:4,top:7});scrollBy(3,5);scrollX+'|'+scrollY+'|'+pageXOffset+'|'+pageYOffset"
            )
            .unwrap(),
            "7|12|7|12"
        );
    }

    #[test]
    fn initial_viewport_is_available_to_parser_time_scripts() {
        let mut page = RustExecutableDocument::load(
            "<script>globalThis.parserViewport=innerWidth+'|'+innerHeight</script>",
        )
        .unwrap();
        assert_eq!(page.evaluate("parserViewport").unwrap(), "1280|720");
    }

    #[test]
    fn native_focus_automation_defers_dispatch_until_promise_return() {
        let mut page = RustExecutableDocument::load(
            "<body><button id='click'>Click</button><input id='keys'><button id='actions'>Actions</button></body>"
        ).unwrap();
        page.execute(r#"
            globalThis.automationResult='pending';
            (async()=>{
                const checks=[];
                for(const id of ['click','keys','actions']) {
                    const target=document.getElementById(id);
                    let promise;
                    target.addEventListener('focus',()=>checks.push(!!promise&&typeof promise.then==='function'),{once:true});
                    if(id==='click')promise=__oryn_testdriver_click(target);
                    else if(id==='keys')promise=__oryn_testdriver_send_keys(target,'x');
                    else promise=__oryn_testdriver_actions([{type:'pointer',id:'mouse',actions:[
                        {type:'pointerMove',origin:target,x:0,y:0},{type:'pointerDown',button:0},{type:'pointerUp',button:0}
                    ]}]);
                    checks.push(promise instanceof Promise&&document.activeElement!==target);
                    await promise;
                    checks.push(document.activeElement===target);
                }
                checks.push(document.getElementById('keys').value==='x');
                return checks.length===10&&checks.every(Boolean)?'ok':JSON.stringify(checks);
            })().then(value=>globalThis.automationResult=value,error=>globalThis.automationResult=String(error));
        "#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);automationResult")
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_focus_visibility_uses_input_default_actions() {
        let mut page = RustExecutableDocument::load(
            "<style>*{outline-color:black}:focus-visible{outline-color:green}</style><body><button id='one'></button><button id='two'></button><input id='text'><div id='blank' style='height:100px;width:200px'></div></body>"
        ).unwrap();
        page.execute(r#"globalThis.focusTestResult="pending";(async () => {
            const one=document.getElementById('one'),two=document.getElementById('two'),text=document.getElementById('text');
            const visible=node=>{const css=getComputedStyle(node).outlineColor==='rgb(0, 128, 0)';if(node.matches(':focus-visible')!==css)throw new Error('selector/style mismatch');return css};
            one.focus();if(!visible(one))return 'initial script focus';
            one.blur();await __oryn_testdriver_click(one);if(visible(one))return 'pointer button';
            one.dispatchEvent(new Event('keydown',{bubbles:true}));if(visible(one))return 'synthetic keyboard';
            __oryn_testdriver_keyboard(one,'a',true);if(!visible(one))return 'native keyboard';
            await __oryn_testdriver_click(one);if(!visible(one))return 'same pointer target';
            two.addEventListener('mousedown',event=>event.preventDefault(),{once:true});
            await __oryn_testdriver_click(two);if(document.activeElement!==one)return 'canceled pointer default';
            await __oryn_testdriver_click(two);if(document.activeElement!==two||visible(two))return 'pointer focus transfer';
            await __oryn_testdriver_click(text);if(!visible(text))return 'text input';
            text.blur();one.focus();if(visible(one))return 'script focus after pointer text input';
            let bodyClick=false;document.body.addEventListener('click',()=>bodyClick=true,{once:true});await __oryn_testdriver_click(document.getElementById('blank'));if(!bodyClick||document.activeElement!==one)return 'nonfocusable pointer target';
            return 'ok';
        })().then(value=>globalThis.focusTestResult=value,error=>globalThis.focusTestResult=String(error))"#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);focusTestResult").unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_focus_default_outline_tracks_visibility_and_author_override() {
        let mut page = RustExecutableDocument::load(
            "<body><div id='target' tabindex='0'>Target</div><button id='button'>Button</button><input id='input'></body>"
        ).unwrap();
        page.execute(r#"globalThis.focusTestResult="pending";(async () => {
            const target=document.getElementById('target'),button=document.getElementById('button'),input=document.getElementById('input');
            const outline=node=>getComputedStyle(node).outlineStyle;
            if(outline(target)!=='none')return 'unfocused';
            target.focus();if(outline(target)!=='auto')return 'script focus';
            await __oryn_testdriver_click(button);if(outline(button)!=='none')return 'pointer button';
            __oryn_testdriver_keyboard(button,'a',true);if(outline(button)!=='auto')return 'keyboard button';
            button.style.outline='none';if(outline(button)!=='none')return 'author override';
            await __oryn_testdriver_click(input);if(outline(input)!=='auto')return 'text input';
            input.blur();if(outline(input)!=='none')return 'blur';
            return 'ok';
        })().then(value=>globalThis.focusTestResult=value,error=>globalThis.focusTestResult=String(error))"#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);focusTestResult").unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_focus_fixup_blurs_dynamic_inertness_and_preserves_reentrant_focus() {
        let mut page = RustExecutableDocument::load(
            "<body><input id='outside'><div id='outer'><input id='target'></div></body>",
        )
        .unwrap();
        page.execute(r#"
            globalThis.events=[];
            target.addEventListener('blur',e=>events.push('blur:'+e.isTrusted+':'+(e.relatedTarget===null)));
            target.addEventListener('focusout',e=>events.push('focusout:'+e.isTrusted));
            target.focus();outer.inert=true;
        "#).unwrap();
        assert_eq!(
            page.evaluate("(document.activeElement===document.body)+'|'+events.join(',')")
                .unwrap(),
            "true|blur:true:true,focusout:true"
        );
        page.execute(
            "outer.inert=false;target.focus();outer.inert=true;outer.inert=false;events=[]",
        )
        .unwrap();
        assert_eq!(
            page.evaluate("(document.activeElement===target)+'|'+events.length")
                .unwrap(),
            "true|0"
        );
        page.execute("target.addEventListener('blur',()=>outside.focus(),{once:true});outer.setAttribute('inert','')").unwrap();
        assert_eq!(
            page.evaluate("(document.activeElement===outside)+'|'+events.join(',')")
                .unwrap(),
            "true|blur:true:true,focusout:true"
        );
    }

    #[test]
    fn native_focus_fixup_tracks_live_slot_assignment() {
        let mut page = RustExecutableDocument::load(
            "<body><div id='host'><input id='target' slot='free'></div></body>",
        )
        .unwrap();
        page.execute(
            r#"
            const root=host.attachShadow({mode:'closed'});
            const free=document.createElement('slot'),blocked=document.createElement('slot');
            free.name='free';blocked.name='blocked';blocked.inert=true;
            root.appendChild(free);root.appendChild(blocked);
            target.focus();target.setAttribute('slot','blocked');
        "#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.activeElement===document.body")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn native_focus_respects_computed_display_and_visibility() {
        let mut page = RustExecutableDocument::load(
            "<style>.gone{display:none}.invisible{visibility:hidden}.visible{visibility:visible}</style><body><input id='outside'><section id='parent' class='gone'><input id='target'></section><section class='invisible'><input id='hidden'><input id='visible' class='visible'></section><canvas><input id='fallback'></canvas></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            outside.focus();target.focus();if(document.activeElement!==outside)return 'display:none';
            hidden.focus();if(document.activeElement!==outside)return 'visibility:hidden';
            visible.focus();if(document.activeElement!==visible)return 'visible override';
            fallback.focus();if(document.activeElement!==fallback)return 'canvas fallback';
            document.getElementById('parent').className='';target.focus();if(document.activeElement!==target)return 'restored display';
            return 'ok';
        })()"#).unwrap(), "ok");
        page.execute("globalThis.blurs=0;target.onblur=()=>blurs++;document.getElementById('parent').className='gone'")
            .unwrap();
        assert_eq!(
            page.evaluate("(document.activeElement===document.body)+'|'+blurs")
                .unwrap(),
            "true|1"
        );
        page.execute("document.getElementById('parent').className='';outside.focus();outside.onblur=()=>document.getElementById('parent').className='gone';target.focus()").unwrap();
        assert_eq!(
            page.evaluate("document.activeElement!==target").unwrap(),
            "true"
        );
    }

    #[test]
    fn native_interactive_validation_respects_cancellation_mutation_and_focus() {
        let mut page = RustExecutableDocument::load("<!doctype html><button id=away type=button>Away</button><form id=f><input id=a required><input id=b required><input id=c required></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const a=document.getElementById('a'),b=document.getElementById('b'),c=document.getElementById('c');
            const events=[];
            for(const control of [a,b,c]) control.oninvalid=e=>{
                check(e.isTrusted&&!e.bubbles&&e.cancelable,'event flags');events.push(control.id);
            };
            away.focus();check(!f.checkValidity()&&document.activeElement===away,'static does not focus');
            check(events.join(',')==='a,b,c','static snapshot');events.length=0;
            a.oninvalid=e=>{events.push('a');e.preventDefault()};
            check(!f.reportValidity()&&document.activeElement===b,'first unhandled');
            check(events.join(',')==='a,b,c','report snapshot');
            away.focus();check(!a.reportValidity()&&document.activeElement===away,'canceled control');
            a.oninvalid=()=>{a.remove();b.value='fixed'};
            away.focus();check(!f.reportValidity()&&document.activeElement===c,'removed and repaired skipped');
            document.body.appendChild(a);a.oninvalid=null;
            a.onfocus=()=>away.focus();check(!a.reportValidity()&&document.activeElement===away,'focus callback wins');
            a.onfocus=null;a.disabled=true;check(a.reportValidity(),'barred');a.disabled=false;
            a.oninvalid=()=>{a.value='fixed'};check(!a.reportValidity(),'snapshot result survives repair');
            check(a.reportValidity(),'subsequent valid result');
            let submits=0;f.onsubmit=e=>{e.preventDefault();submits++};
            away.focus();f.requestSubmit();check(submits===0&&document.activeElement===c,'submission reports');
            c.value='fixed';f.requestSubmit();check(submits===1,'submission recovery');
            for(const tag of ['input','textarea','select','button','fieldset','output','object']) {
                const el=document.createElement(tag);check(el.reportValidity(),'interface '+tag);
            }
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_text_editing_preserves_defaults_lengths_and_provenance() {
        let mut page = RustExecutableDocument::load("<style>:invalid{color:rgb(255,0,0)}</style><form id=f><textarea id=t minlength=5 maxlength=10 required></textarea><input id=i maxlength=10></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            t.defaultValue='abc';check(t.value==='abc'&&t.textContent==='abc'&&t.validity.valid,'default native text');
            t.replaceChildren('abcd');check(t.value==='abcd'&&t.validity.valid,'default children');
            t.value='';t.focus();t.oninput=()=>{t.defaultValue=t.value};
            for(const c of 'abc')__oryn_testdriver_keyboard(t,c,true);
            check(t.value==='abc'&&t.validity.tooShort&&!t.validity.valid,'user provenance');
            check(t.matches(':invalid')&&getComputedStyle(t).color==='rgb(255, 0, 0)','shared style');
            let submits=0;f.onsubmit=e=>{e.preventDefault();submits++};f.requestSubmit();check(submits===0,'short blocked');
            for(const c of 'de')__oryn_testdriver_keyboard(t,c,true);
            f.requestSubmit();check(submits===1&&t.validity.valid,'recovery');
            t.value='x';check(!t.validity.tooShort,'script exemption');
            t.setRangeText('y',0,1,'end');check(t.value==='y'&&!t.validity.tooShort,'range script exemption');
            t.defaultValue='fresh';check(t.value==='y','dirty default does not change value');
            f.reset();t.replaceChildren('new');check(t.value==='new'&&!t.validity.tooShort,'reset restores default tracking');
            check(t.minLength===5&&t.maxLength===10&&t.textLength===3,'reflected lengths');
            try{t.maxLength=-1;throw Error('missing exception')}catch(e){check(e.name==='IndexSizeError','negative maximum')}
            i.focus();check(document.execCommand('InsertHTML',false,'👨‍👩‍👧‍👦'),'insert command');
            check(i.value==='👨‍👩‍👧‍'&&i.value.length===9,'UTF16 scalar truncation');
            i.maxLength=2;check(i.validity.tooLong,'dynamic maximum');
            i.value='abc';check(!i.validity.tooLong,'script overlong exemption');
            i.value='';i.readOnly=true;document.execCommand('insertText',false,'a');check(i.value==='','readonly editing');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_control_events_are_tasks_and_changes_commit_once() {
        let mut page = RustExecutableDocument::load(
            "<form id=f><textarea id=t>abc</textarea><input id=i></form>",
        )
        .unwrap();
        page.execute(
            r#"
            globalThis.events=[];globalThis.setTimeout=()=>{throw Error('page timer called')};
            t.onselect=e=>events.push('select:'+e.isTrusted+':'+e.bubbles+':'+e.cancelable);
            t.setSelectionRange(0,3);events.push('sync');
        "#,
        )
        .unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("events.join('|')").unwrap(),
            "sync|select:true:true:false"
        );
        page.execute("events=[];t.setRangeText('foo',2,2);events.push('sync')")
            .unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(
            page.evaluate("events.join('|')").unwrap(),
            "sync|select:true:true:false"
        );
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};let changes=0;
            t.onchange=e=>{check(e.isTrusted&&e.bubbles&&!e.cancelable,'change flags');changes++};
            t.focus();t.select();__oryn_testdriver_keyboard(t,'Backspace',true);
            check(t.value===''&&changes===0,'not committed during input');t.blur();check(changes===1,'commit on blur');
            t.blur();check(changes===1,'no duplicate');
            t.value=null;i.value=null;check(t.value===''&&i.value==='','legacy null');
            t.value=undefined;check(t.value==='undefined','undefined remains DOMString');
            t.focus();__oryn_testdriver_keyboard(t,'x',true);f.reset();t.blur();check(changes===1,'reset clears pending commit');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_oil_typing_uses_user_length_constraints() {
        let mut page =
            RustExecutableDocument::load("<textarea id=t minlength=5 maxlength=10></textarea>")
                .unwrap();
        page.type_text("#t", "abc").unwrap();
        assert_eq!(
            page.evaluate("t.validity.tooShort+'|'+t.value").unwrap(),
            "true|abc"
        );
        page.type_text("#t", "👨‍👩‍👧‍👦").unwrap();
        assert_eq!(
            page.evaluate("t.validity.valid+'|'+t.value").unwrap(),
            "true|👨‍👩‍👧‍"
        );
        page.execute("t.value='abc';t.dispatchEvent(new Event('input'));t.dispatchEvent(new KeyboardEvent('keydown',{key:'x'}))").unwrap();
        assert_eq!(page.evaluate("t.validity.tooShort").unwrap(), "false");
    }

    #[test]
    fn native_pattern_constraints_share_validation_style_and_submission() {
        let mut page = RustExecutableDocument::load("<!doctype html><style>:invalid{color:rgb(255,0,0)}</style><form id=f><input id=p pattern='[A-Z]{3}' value=abc></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const state=p.validity;
            check(state.patternMismatch&&!state.valid,'native mismatch');
            check(p.matches(':invalid')&&getComputedStyle(p).color==='rgb(255, 0, 0)','shared style');
            let submits=0;f.onsubmit=e=>{e.preventDefault();submits++};
            f.requestSubmit();check(submits===0&&document.activeElement===p,'submission blocked');
            p.value='ABC';check(state.valid,'live recovery');f.requestSubmit();check(submits===1,'submission recovery');
            const original=RegExp,exec=RegExp.prototype.exec,test=RegExp.prototype.test;
            /sentinel/.test('page sentinel');const before=RegExp.input;
            RegExp.prototype.exec=RegExp.prototype.test=()=>{throw Error('authored regex called')};
            globalThis.RegExp=()=>{throw Error('authored constructor called')};
            p.value='private-subject';check(state.patternMismatch,'private engine');
            check(original.input===before,'no page regex statics leak');
            globalThis.RegExp=original;original.prototype.exec=exec;original.prototype.test=test;
            p.pattern='[\\p{ASCII}&&\\p{Letter}]+';p.value='ABC';check(state.valid,'Unicode intersection');
            p.value='é';check(state.patternMismatch,'Unicode exclusion');
            p.pattern='a)(b';p.value='ab';check(!state.patternMismatch,'invalid original syntax ignored');
            p.pattern='[(]';check(!state.patternMismatch,'v syntax enforced');
            p.pattern='';check(state.patternMismatch,'empty pattern');
            p.value='';check(!state.patternMismatch,'empty value exemption');
            for(const type of ['text','search','tel','url','email','password']) {
                p.type=type;p.pattern='ABC';p.value='xyz';check(state.patternMismatch,type);
            }
            p.type='email';p.multiple=true;p.pattern='[^@]+@example\\.test';
            p.value='a@example.test, b@example.test';check(!state.patternMismatch,'email tokens');
            p.value='a@example.test,b@else.test';check(state.patternMismatch,'email token mismatch');
            p.disabled=true;check(state.patternMismatch&&p.checkValidity(),'barred flag retained');p.disabled=false;
            p.type='text';p.pattern='(\\d+)*$';p.value='12345678901234567890123456789123456789z';
            check(state.patternMismatch,'bounded backtracking');
            for(let i=0;i<80;i++){p.pattern='x{'+(i+1)+'}';p.value='x';check(state.patternMismatch===(i!==0),'cache churn '+i)}
            p.removeAttribute('pattern');check(!state.patternMismatch,'attribute removal');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_input_numeric_apis_preserve_dom_state_and_error_semantics() {
        let mut page = RustExecutableDocument::load("<!doctype html><form id=f><input id=n type=number min=0 max=5 step=0.1 value=0.2><input id=d type=date><input id=t type=time></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const throws=(name,fn)=>{try{fn()}catch(e){check(e.name===name,'exception '+e.name);check(e instanceof (name==='TypeError'?TypeError:DOMException),'exception prototype');return}throw Error('missing '+name)};
            let events=0;n.oninput=n.onchange=()=>events++;
            n.stepUp();check(n.value==='0.3'&&n.valueAsNumber===0.3&&n.validity.valid,'precise native step');
            n.stepDown(undefined);check(n.value==='0.2'&&events===0,'default and no user events');
            n.stepDown(-2);check(n.value==='0.4','negative count reverses direction');
            n.stepUp(4294967297);check(n.value==='0.5','Web IDL long wrapping');
            n.stepDown(2.9);check(n.value==='0.3','Web IDL long truncation');
            throws('TypeError',()=>n.stepUp(1n));throws('TypeError',()=>n.valueAsNumber=1n);
            n.valueAsNumber={valueOf(){return 0.4}};check(n.value==='0.4','number conversion');
            n.valueAsNumber=NaN;check(n.value===''&&Number.isNaN(n.valueAsNumber),'NaN clear');
            throws('TypeError',()=>n.valueAsNumber=Infinity);
            n.step='any';throws('InvalidStateError',()=>n.stepUp());
            d.valueAsDate=new Date('2024-02-29T21:30:00Z');check(d.value==='2024-02-29','UTC date setter');
            check(d.valueAsDate instanceof Date&&d.valueAsDate.toISOString()==='2024-02-29T00:00:00.000Z','UTC getter');
            d.stepUp();check(d.value==='2024-03-01','calendar step');
            const poisoned=new Date(0);poisoned.valueOf=()=>{throw Error('called authored method')};
            d.valueAsDate=poisoned;check(d.value==='1970-01-01','Date internal slot');
            throws('TypeError',()=>d.valueAsDate={});d.valueAsDate=null;check(d.value==='','null clear');
            d.value='2024-02-29';d.valueAsDate=undefined;check(d.value==='','nullable undefined clear');
            d.valueAsDate=new Date(NaN);check(d.valueAsDate===null,'invalid date');
            d.type='datetime-local';check(d.valueAsDate===null,'local date does not apply');
            throws('InvalidStateError',()=>d.valueAsDate=new Date());
            d.type='text';throws('InvalidStateError',()=>d.valueAsNumber=1);throws('InvalidStateError',()=>d.stepDown());
            throws('TypeError',()=>d.valueAsDate=1);throws('InvalidStateError',()=>d.valueAsDate={});
            throws('TypeError',()=>d.valueAsNumber=Infinity);
            t.valueAsNumber=-3600000;check(t.value==='23:00','negative time wrap');
            t.valueAsNumber=2.7343337071894478e26;check(t.value==='10:54:10.944','large time wrap');
            t.value='23:59:59.999';t.step='0.001';t.stepUp();check(t.value==='00:00','midnight step');
            f.reset();check(n.value==='0.2','reset keeps authored default');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_numeric_constraints_share_submission_api_and_style() {
        let mut page = RustExecutableDocument::load("<!doctype html><style>:invalid{color:rgb(255,0,0)}</style><form id=f><input id=n type=number min=1 max=5 step=2 value=3><input id=d type=date><input id=t type=time></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const n=document.getElementById('n'),d=document.getElementById('d'),t=document.getElementById('t');
            const v=n.validity;check(n.min==='1'&&n.max==='5'&&n.step==='2','reflection');
            n.value='4';check(v.stepMismatch&&!v.valid&&!n.checkValidity(),'step');
            check(n.matches(':invalid')&&getComputedStyle(n).color==='rgb(255, 0, 0)','style');
            let submits=0;f.onsubmit=e=>{e.preventDefault();submits++};f.requestSubmit();check(submits===0,'blocked');
            n.step='any';n.value='6';check(v.rangeOverflow&&!v.rangeUnderflow,'overflow');
            n.value='0';check(v.rangeUnderflow&&!v.rangeOverflow,'underflow');
            n.value='3';check(v.valid,'recovery');
            for(const [type,value,min,max,step] of [
                ['date','2024-02-29','2024-03-01','2024-03-31','1'],
                ['month','2024-02','2024-03','2024-12','1'],
                ['week','2024-W01','2024-W02','2024-W52','1'],
                ['datetime-local','2024-02-29T12:00','2024-03-01T12:00','2024-12-31T12:00','60']
            ]) {d.type=type;d.min=min;d.max=max;d.step=step;d.value=value;check(d.validity.rangeUnderflow,type);d.value=min;check(d.validity.valid,type+' boundary')}
            d.value='';t.min='21:00';t.max='06:00';t.value='12:00';check(t.validity.rangeUnderflow&&t.validity.rangeOverflow,'reversed');
            t.value='23:00';check(t.validity.valid,'overnight');
            n.step='3e-15';n.min='';n.removeAttribute('value');n.value='17';check(n.validity.stepMismatch,'tiny step');
            n.step='0.003';n.value='3.6';check(n.validity.valid,'decimal multiple');
            n.step='any';n.value='1e999';check(n.value===''&&n.validity.valid,'sanitized invalid float');
            f.requestSubmit();check(submits===1,'successful recovery');return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_validation_candidates_include_submit_controls_and_barred_interfaces() {
        let mut page = RustExecutableDocument::load("<!doctype html><form id=f><button id=b>Submit</button><input id=i type=image><input id=s type=submit><input id=text required><textarea id=area required></textarea><input id=radio type=radio required><fieldset id=fs></fieldset><output id=out></output><object id=obj></object></form>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            let submits=0;f.onsubmit=event=>{event.preventDefault();submits++};
            for(const control of [text,area]) {
                check(control.validity.valueMissing,'initial missing');
                control.disabled=true;check(!control.validity.valueMissing&&control.validity.valid&&!control.willValidate,'disabled missing');
                control.disabled=false;check(control.validity.valueMissing,'restored missing');control.value='ok';
            }
            check(!radio.validity.valueMissing,'unnamed radio');
            radio.name='group';check(radio.validity.valueMissing,'named radio');
            radio.disabled=true;check(radio.validity.valueMissing&&!radio.willValidate&&radio.checkValidity(),'disabled radio retains group state');
            radio.disabled=false;radio.checked=true;
            for(const type of ['checkbox','file']) {
                const c=document.createElement('input');c.type=type;c.required=true;c.disabled=true;
                check(c.validity.valueMissing&&!c.willValidate&&c.checkValidity(),'disabled '+type+' retains validity');
            }
            for(const control of [b,i,s]) {
                const v=control.validity;let events=0;control.oninvalid=()=>events++;
                check(control.willValidate,'submit candidate');control.setCustomValidity('blocked');
                check(v.customError&&!v.valid&&!control.checkValidity()&&events===1,'submit custom error');
                f.requestSubmit();check(submits===0&&events===2,'form validates submit controls including images');
                control.setCustomValidity('');check(v.valid,'cleared');
            }
            b.type='button';b.setCustomValidity('still retained');check(!b.willValidate&&b.checkValidity()&&b.validity.customError,'button barred');
            b.type='reset';check(!b.willValidate,'reset barred');b.type='submit';b.setCustomValidity('');
            for(const control of [fs,out,obj]) {
                const v=control.validity;control.setCustomValidity('barred');
                check(!control.willValidate&&control.checkValidity()&&v.customError&&!v.valid&&control.validationMessage==='','barred state');
            }
            check(obj instanceof HTMLObjectElement&&obj.form===f,'object binding and owner');
            for(const type of ['color','file','submit','image','checkbox','radio']) {
                const c=document.createElement('input');c.type=type;c.readOnly=true;
                check(!c.willValidate,'readonly '+type);
            }
            b.setAttribute('command','--example');b.type='invalid';check(!b.willValidate,'auto command not submit');
            b.type='submit';check(b.willValidate,'explicit submit');
            f.requestSubmit();check(submits===1,'successful recovery');return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_control_validation_is_shared_by_submission_api_and_style() {
        let mut page = RustExecutableDocument::load(r#"<!doctype html><style>:invalid {color:rgb(255, 0, 0)} :valid {color:rgb(0, 128, 0)}</style><form id=f><fieldset id=fs><input id=e type=email required><textarea id=t required>initial</textarea></fieldset></form><input id=outside form=f required>"#).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const e=document.getElementById('e'),validity=e.validity;let invalid=0,submits=0;
            e.oninvalid=event=>{check(event.isTrusted&&!event.bubbles&&event.cancelable,'invalid event');invalid++};
            f.onsubmit=event=>{event.preventDefault();submits++};
            check(validity===e.validity&&validity instanceof ValidityState&&validity.valueMissing&&!validity.valid,'live state');
            check(e.matches(':invalid')&&f.matches(':invalid')&&fs.matches(':invalid'),'initial selectors');
            check(getComputedStyle(e).color==='rgb(255, 0, 0)','initial style');
            check(!e.checkValidity()&&invalid===1,'control check');f.requestSubmit();check(submits===0&&invalid===2,'submission check');
            e.value='a@b';check(validity.valid&&!validity.valueMissing&&e.matches(':valid'),'updated state');
            check(getComputedStyle(e).color==='rgb(0, 128, 0)','updated style');
            check(fs.matches(':valid')&&f.matches(':invalid'),'external form owner');
            outside.value='ok';check(f.matches(':valid')&&f.checkValidity(),'all valid');
            e.value='bad';check(validity.typeMismatch&&!validity.valid&&e.matches(':invalid'),'email mismatch');
            e.readOnly=true;check(!e.willValidate&&e.checkValidity()&&!e.matches(':invalid,:valid')&&f.matches(':valid'),'barred eligibility');
            e.readOnly=false;e.value='a@b';e.setCustomValidity('blocked');
            check(validity.customError&&!validity.valid&&e.validationMessage==='blocked'&&f.matches(':invalid'),'custom validity');
            e.setCustomValidity('');check(validity.valid&&e.validationMessage==='','custom clear');
            t.value='';check(t.validity.valueMissing&&t.matches(':invalid'),'textarea missing');
            t.value='ok';t.setCustomValidity('text error');check(t.validity.customError&&!f.checkValidity(),'textarea custom');
            t.setCustomValidity('');f.requestSubmit();check(submits===1,'recovery');
            e.remove();check(validity===e.validity&&validity.valid,'detached retained state');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_select_validity_drives_queries_and_computed_style() {
        let mut page = RustExecutableDocument::load("<style>select{color:black}select:valid{color:green}select:invalid{color:red}</style><body><select id=s required></select></body>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const verify=(valid,color)=>{
                check(s.matches(':valid')===(valid===true)&&s.matches(':invalid')===(valid===false),'matching');
                check((document.querySelector('select:invalid')===s)===(valid===false),'query');
                check(getComputedStyle(s).color===color,'computed '+getComputedStyle(s).color);
            };
            verify(false,'rgb(255, 0, 0)');s.add(new Option('chosen'));verify(true,'rgb(0, 128, 0)');
            const observer=new MutationObserver(()=>{});observer.observe(s,{attributes:true,childList:true,characterData:true,subtree:true});
            s.setCustomValidity('error');verify(false,'rgb(255, 0, 0)');
            check(observer.takeRecords().length===0,'custom error must not synthesize DOM mutations');
            s.setCustomValidity('');verify(true,'rgb(0, 128, 0)');
            s.disabled=true;verify(null,'rgb(0, 0, 0)');s.disabled=false;verify(true,'rgb(0, 128, 0)');
            s.selectedIndex=-1;verify(false,'rgb(255, 0, 0)');
            s.required=false;verify(true,'rgb(0, 128, 0)');
            check(!document.body.matches(':valid,:invalid'),'non-control');return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn attribute_map_is_live_and_records_use_local_names() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><body><div id=target></div>").unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            const e = document.getElementById('target');
            const attrs = e.attributes;
            if (!(attrs instanceof NamedNodeMap) || attrs !== e.attributes) return 'brand/identity';
            e.setAttribute('z', 'one');
            if (attrs.length !== 2 || attrs.item(1) !== e.getAttributeNode('z')) return 'live index';
            const observer = new MutationObserver(() => {});
            observer.observe(e, {attributes:true, attributeOldValue:true});
            e.setAttributeNS('urn:test', 'p:item', 'one');
            e.setAttributeNS('urn:test', 'q:item', 'two');
            const records = observer.takeRecords();
            if (records.length !== 2 || records.some(r => r.attributeName !== 'item' || r.attributeNamespace !== 'urn:test')) return 'mutation names';
            if (records[1].oldValue !== 'one') return 'old value';
            const a = attrs.getNamedItemNS('urn:test', 'item');
            if (attrs.removeNamedItemNS('urn:test', 'item') !== a || a.ownerElement !== null) return 'remove';
            a.value = 'detached';
            if (attrs.setNamedItemNS(a) !== null || a.ownerElement !== e || a.value !== 'detached') return 'reattach';
            const replacement = document.createAttributeNS('urn:test','r:item'); replacement.value = 'replacement';
            if (e.setAttributeNodeNS(replacement) !== a || a.ownerElement !== null || attrs.item(2) !== replacement) return 'replace';
            if (replacement.prefix !== 'r' || attrs.getNamedItem('r:item') !== replacement || a.prefix !== 'p') return 'replacement prefix';
            try { document.body.setAttributeNode(replacement); return 'in-use accepted'; } catch(error) { if(error.name !== 'InUseAttributeError') return error.name; }
            for (const receiver of [{}, new Proxy(attrs, {})]) {
                try { NamedNodeMap.prototype.item.call(receiver, 0); return 'forged receiver'; } catch(error) { if(!(error instanceof TypeError)) return 'receiver error'; }
            }
            if(Array.from(attrs).length !== 3 || Object.keys(attrs).some(key => key === 'id')) return 'enumeration';
            const differentOrder = document.createElement('div');
            for(const attribute of Array.from(attrs).reverse()) differentOrder.setAttributeNS(attribute.namespaceURI, attribute.name, attribute.value);
            if(!differentOrder.isEqualNode(e)) return 'order-dependent equality';
            differentOrder.removeAttributeNS('urn:test', 'item');
            differentOrder.setAttributeNS('urn:other', 'r:item', 'replacement');
            if(differentOrder.isEqualNode(e)) return 'namespace-blind equality';
            const input = document.createElement('input');
            input.setAttributeNS('urn:foreign','disabled','');
            input.disabled = true; input.disabled = false;
            if(input.disabled || input.getAttributeNS('urn:foreign','disabled') !== '') return 'boolean reflection';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn architecture_attribute_contracts_run_through_native_bindings() {
        let mut page = RustExecutableDocument::load(include_str!(
            "../../../test-harness/architecture-attribute-contracts.html"
        ))
        .unwrap();
        assert_eq!(
            page.evaluate(
                "JSON.stringify(architectureAttributeReport.cases.filter(c => !c.passed))"
            )
            .unwrap(),
            "[]"
        );
        assert_eq!(page.evaluate(r#"(() => {
            const e = document.createElement('div');
            e.setAttributeNS('urn:first', 'p:item', 'first');
            e.setAttributeNS('urn:second', 'p:item', 'second');
            const first = e.getAttributeNodeNS('urn:first', 'item');
            const second = e.getAttributeNodeNS('urn:second', 'item');
            if(e.attributes[0] !== first || e.attributes[1] !== second) return 'indices';
            const copy = e.cloneNode(true);
            if(copy.getAttributeNS('urn:first','item') !== 'first' || copy.getAttributeNS('urn:second','item') !== 'second') return 'clone';
            if(!e.matches('[*|item="second"]')) return 'selector';
            e.removeAttributeNS('urn:first', 'item');
            if(first.ownerElement !== null || second.ownerElement !== e || e.getAttributeNS('urn:second','item') !== 'second') return 'remove';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_form_activation_keeps_style_attributes_and_submission_consistent() {
        let mut page = RustExecutableDocument::load(
            "<style>input{color:black} input:checked{color:blue} input[checked]{background-color:red}</style><body><form id=f><label id=l><input id=c name=c type=checkbox checked></label></form></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            c.checked=false;
            if(c.matches(':checked') || getComputedStyle(c).color!=='rgb(0, 0, 0)')return 'live false';
            if(!c.matches('[checked]') || getComputedStyle(c).backgroundColor!=='rgb(255, 0, 0)')return 'default attribute';
            if(new FormData(f).has('c'))return 'unchecked submission';
            let events=[];c.onclick=()=>events.push('click');c.oninput=()=>events.push('input');c.onchange=()=>events.push('change');
            l.click();
            if(!c.checked || events.join()!=='click,input,change')return 'label activation';
            if(!c.matches(':checked') || getComputedStyle(c).color!=='rgb(0, 0, 255)')return 'live true';
            if(new FormData(f).get('c')!=='on')return 'default-on value';
            c.indeterminate=true;
            if(!c.matches(':indeterminate'))return 'indeterminate selector';
            c.onclick=e=>e.preventDefault();c.click();
            if(!c.checked || !c.indeterminate)return 'canceled restore';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_radio_arrow_actions_accept_webdriver_key_values() {
        let mut page = RustExecutableDocument::load("<!doctype html><body><input type=radio name=g id=a><input type=radio name=g id=b></body>").unwrap();
        page.execute(r#"a.focus();globalThis.arrowResult='pending';
            __oryn_testdriver_actions([{id:'keys',type:'key',actions:[
                {type:'keyDown',value:'\uE015'},{type:'keyUp',value:'\uE015'}
            ]}]).then(()=>arrowResult=document.activeElement===b&&b.checked?'ok':'wrong target',e=>arrowResult=e.message);
        "#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);arrowResult").unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_radio_arrows_use_tree_order_and_cancelable_activation() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><body><form id=f><input type=radio name=g id=a checked><button>skip</button><input type=radio name=g disabled><input type=radio name=g inert><input type=radio name=g style='display:none'></form><input type=radio name=g form=f id=b tabindex=-1><input type=radio name=g form=f id=c tabindex=1><input type=radio name=g id=other></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const arrow=(key='\uE014')=>{__oryn_testdriver_keyboard(document.activeElement,key,true);__oryn_testdriver_keyboard(document.activeElement,key,false)};
            const events=[];b.addEventListener('click',e=>{check(e.isTrusted&&e.pointerId===-1,'keyboard click metadata');events.push('click')});
            b.addEventListener('input',()=>events.push('input'));b.addEventListener('change',()=>events.push('change'));
            a.focus();arrow();check(document.activeElement===b&&b.checked&&!a.checked,'external negative tabindex peer');
            check(events.join()==='click,input,change','activation order');
            b.addEventListener('keydown',e=>e.preventDefault(),{once:true});arrow();check(document.activeElement===b,'canceled key');
            c.addEventListener('click',e=>e.preventDefault(),{once:true});arrow();check(document.activeElement===c&&b.checked&&!c.checked,'canceled activation restores selection');
            arrow();check(document.activeElement===a&&a.checked,'wrap excludes other form');
            arrow('\uE012');check(document.activeElement===c&&c.checked,'reverse tree order');
            c.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));check(document.activeElement===c,'synthetic key no default');
            __oryn_testdriver_keyboard(c,'Control',true);arrow();check(document.activeElement===c,'shortcut no default');__oryn_testdriver_keyboard(c,'Control',false);
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_radio_arrow_rechecks_focus_listener_mutations() {
        let mut page = RustExecutableDocument::load("<body><input type=radio name=g id=a checked><input type=radio name=g id=b><input id=other></body>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            let clicks=0;b.onclick=()=>clicks++;b.onfocus=()=>{b.disabled=true;other.focus()};
            a.focus();__oryn_testdriver_keyboard(a,'ArrowRight',true);
            if(clicks!==0||!a.checked||b.checked||document.activeElement!==other)throw Error('stale activation');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_radio_arrow_direction_follows_computed_rtl() {
        let mut page = RustExecutableDocument::load("<body style='direction:rtl'><input type=radio name=g id=a><input type=radio name=g id=b><input type=radio name=g id=c></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"(()=>{
            b.focus();__oryn_testdriver_keyboard(b,'ArrowRight',true);
            if(document.activeElement!==a||!a.checked)throw Error('rtl right');
            __oryn_testdriver_keyboard(a,'ArrowDown',true);
            if(document.activeElement!==b||!b.checked)throw Error('rtl down');
            return 'ok';
        })()"#
            )
            .unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_keyboard_tab_ignores_non_elements_and_reverses_radio_group() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html>\n<!-- before root --><html><head><title>Tab</title></head><body>\n<input id=start> text <!-- between -->\n<input type=radio name=g id=a><input type=radio name=g id=b>\n<input id=end></body></html>",
        ).unwrap();
        assert_eq!(
            page.evaluate(
                r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const tab=()=>__oryn_testdriver_keyboard(document.activeElement,'Tab',true);
            start.focus();tab();check(document.activeElement===a,'forward first radio');
            tab();check(document.activeElement===end,'group exit');
            __oryn_testdriver_keyboard(end,'Shift',true);
            tab();check(document.activeElement===a,'reverse first radio');
            tab();check(document.activeElement===start,'reverse group exit');
            __oryn_testdriver_keyboard(start,'Shift',false);
            return 'ok';
        })()"#
            )
            .unwrap(),
            "ok"
        );
    }

    #[test]
    fn native_keyboard_send_keys_retargets_keyup_after_tab_default() {
        let mut page =
            RustExecutableDocument::load("<body><input id=a><input id=b><input id=c></body>")
                .unwrap();
        page.execute(r#"
            globalThis.events=[];globalThis.finished=false;
            for(const kind of ['keydown','keyup'])document.addEventListener(kind,e=>events.push(kind+':'+e.target.id+':'+e.keyCode));
            __oryn_testdriver_send_keys(a,'\uE004\uE004').then(()=>finished=true);
        "#).unwrap();
        assert_eq!(
            page.evaluate(
                "__oryn_drain(true);finished+'|'+events.join(',')+'|'+document.activeElement.id"
            )
            .unwrap(),
            "true|keydown:a:9,keyup:b:9,keydown:b:9,keyup:c:9|c"
        );
    }

    #[test]
    fn native_keyboard_tab_orders_candidates_and_honors_cancellation() {
        let mut page = RustExecutableDocument::load(
            "<body><input id=z><input id=b tabindex=2><input id=a tabindex=1><input tabindex=-1><input disabled><input inert><input style='display:none'><input id=last></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const tab=()=>{const target=document.activeElement;__oryn_testdriver_keyboard(target,'Tab',true);__oryn_testdriver_keyboard(document.activeElement,'Tab',false)};
            let down=[];document.addEventListener('keydown',e=>{down.push([e.key,e.code,e.keyCode,e.charCode,e.isTrusted]);});
            tab();check(document.activeElement===a,'positive first');tab();check(document.activeElement===b,'positive second');
            b.addEventListener('keydown',e=>e.preventDefault(),{once:true});tab();check(document.activeElement===b,'canceled');
            tab();check(document.activeElement===z,'zero order');tab();check(document.activeElement===last,'skip negative disabled inert hidden');
            __oryn_testdriver_keyboard(last,'Shift',true);tab();check(document.activeElement===z,'reverse');__oryn_testdriver_keyboard(z,'Shift',false);
            z.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true}));check(document.activeElement===z,'synthetic no default');
            const negative=document.querySelector('[tabindex="-1"]');negative.focus();tab();check(document.activeElement===last,'negative starting point');
            check(down[0].join()==='Tab,Tab,9,0,true','metadata');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_keyboard_tab_enters_shadow_scopes_and_radio_groups() {
        let mut page = RustExecutableDocument::load(
            "<body><input id=before><div id=host><input id=assigned slot=x></div><input type=radio name=g id=r1><input type=radio name=g id=r2 checked><input id=after></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const root=host.attachShadow({mode:'closed'});root.innerHTML='<input id=inner tabindex=1><slot name=x><input id=fallback></slot>';
            const inner=root.getElementById('inner');
            const tab=()=>__oryn_testdriver_keyboard(document.activeElement,'Tab',true);
            before.focus();tab();check(root.activeElement===inner,'shadow entry');
            tab();check(document.activeElement===assigned,'assigned slot, not fallback');
            tab();check(document.activeElement===r2,'checked radio only');tab();check(document.activeElement===after,'radio exit');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_form_inert_label_click_does_not_hide_focus_failure() {
        let mut page = RustExecutableDocument::load(
            "<body><label id=l inert for=c>label</label><input id=c><input id=other></body>",
        )
        .unwrap();
        page.execute("other.focus();globalThis.result='pending';__oryn_testdriver_click(l).then(()=>result=document.activeElement===document.body?'body':'wrong focus',e=>result=e.message)").unwrap();
        let result = page.evaluate("__oryn_drain(true);result").unwrap();
        assert!(
            matches!(result.as_str(), "body" | "element click intercepted"),
            "{result}"
        );
    }

    #[test]
    fn native_form_label_focus_resolves_association_and_eligibility() {
        let mut page = RustExecutableDocument::load(
            "<body><label id=l for=c>label</label><input id=c><input id=other></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            l.focus();check(document.activeElement===c,'forward');
            l.tabIndex=0;l.focus();check(document.activeElement===l,'tabindex');
            l.tabIndex=-1;l.focus();check(document.activeElement===l,'negative');
            l.setAttribute('tabindex','');l.focus();check(document.activeElement===c,'empty tabindex');
            l.tabIndex=0;l.style.display='none';other.focus();l.focus();check(document.activeElement===c,'hidden label');
            l.style.display='';l.removeAttribute('tabindex');l.inert=true;other.focus();l.focus();check(document.activeElement===c,'inert label');
            c.inert=true;other.focus();l.focus();check(document.activeElement===other,'inert control');
            c.inert=false;c.disabled=true;l.focus();check(document.activeElement===other,'disabled control');
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_form_activation_does_not_activate_an_ancestor_link() {
        let mut page = RustExecutableDocument::load(
            "<body><a href='#unexpected'><label for=c><span id=s>choose</span></label><input id=c type=checkbox></a><a href='#expected'><span id=linktext>navigate</span></a></body>",
        ).unwrap();
        page.execute("document.getElementById('s').click()")
            .unwrap();
        page.drain_default_actions().unwrap();
        assert_eq!(
            page.evaluate("document.getElementById('c').checked")
                .unwrap(),
            "true"
        );
        assert_eq!(page.evaluate("location.hash").unwrap(), "");
        page.execute("document.getElementById('c').click()")
            .unwrap();
        page.drain_default_actions().unwrap();
        assert_eq!(page.evaluate("location.hash").unwrap(), "");
        page.execute("document.getElementById('linktext').click()")
            .unwrap();
        page.drain_default_actions().unwrap();
        assert_eq!(page.evaluate("location.hash").unwrap(), "#expected");
    }

    #[test]
    fn native_form_element_click_uses_trusted_pointer_activation() {
        let mut page = RustExecutableDocument::load(
            "<style>label{display:block;width:200px;height:30px}</style><body><label id=l for=c>choose</label><input id=c type=checkbox></body>",
        ).unwrap();
        page.execute(r#"
            globalThis.clickResult='pending';globalThis.clickLog=[];
            l.onclick=e=>clickLog.push([e.isTrusted,e.pointerType,e.pointerId,e.detail]);
            c.onclick=e=>clickLog.push([e.isTrusted,e.pointerType,e.pointerId,e.detail]);
            __oryn_testdriver_click(l).then(()=>clickResult=JSON.stringify([c.checked,clickLog]),e=>clickResult=String(e));
        "#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);clickResult").unwrap(),
            r#"[true,[[true,"mouse",1,1],[true,"mouse",1,1]]]"#
        );
    }

    #[test]
    fn native_focus_updates_stylo_and_selector_state_synchronously() {
        let mut page = RustExecutableDocument::load(
            "<style>* {color:black} :focus-within {color:red} :focus {color:blue}</style><body><div id='parent'><input id='target'></div><section></section></body>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            const parent = document.getElementById('parent'), target = document.getElementById('target');
            target.focus();
            if(getComputedStyle(parent).color !== 'rgb(255, 0, 0)' || getComputedStyle(target).color !== 'rgb(0, 0, 255)' || !parent.matches(':focus-within')) return 'focus';
            parent.remove(); target.focus();
            if(target.matches(':focus') || parent.querySelectorAll(':focus-within').length) return 'detached';
            document.body.appendChild(parent); target.focus();
            if(getComputedStyle(parent).color !== 'rgb(255, 0, 0)') return 'refocus';
            const section = document.querySelector('section');
            section.moveBefore(parent, null);
            if(document.activeElement !== target || getComputedStyle(parent).color !== 'rgb(255, 0, 0)') return 'preserving move';
            section.moveBefore(target, parent);
            if(document.activeElement !== target || getComputedStyle(parent).color !== 'rgb(0, 0, 0)') return 'preserving direct move';
            section.appendChild(target);
            if(target.matches(':focus') || getComputedStyle(parent).color !== 'rgb(0, 0, 0)') return 'move';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn custom_element_upgrade_uses_literal_names_and_connectedness() {
        let mut page = RustExecutableDocument::load("<body><ce-1.25></ce-1.25></body>").unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            globalThis.customConnections = 0;
            class DecimalName extends HTMLElement {connectedCallback(){customConnections++;}}
            customElements.define('ce-1.25', DecimalName);
            const existing = document.getElementsByTagName('ce-1.25')[0];
            if(!(existing instanceof DecimalName) || customConnections !== 1) return 'existing';
            const detached = document.createElement('div'), element = document.createElement('ce-1.25');
            detached.appendChild(element); customElements.upgrade(detached);
            if(!(element instanceof DecimalName) || customConnections !== 1) return 'detached';
            document.body.appendChild(element);
            // Explicit host reaction checkpoint; CEReactions timing is separate.
            __oryn_run_custom_reactions();
            if(customConnections !== 2) return 'connected';
            const foreign = document.createElementNS('urn:xml', 'ce-1.25');
            document.body.appendChild(foreign); customElements.upgrade(foreign);
            if(foreign instanceof DecimalName || customConnections !== 2) return 'namespace';
            const late = document.createElement('div'); late.innerHTML = '<ce-2.50></ce-2.50>';
            class LaterName extends HTMLElement {connectedCallback(){customConnections++;}}
            customElements.define('ce-2.50', LaterName); customElements.upgrade(late);
            if(!(late.firstChild instanceof LaterName) || customConnections !== 2) return 'late detached upgrade';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn parser_time_scripts_resolve_html_id_named_window_properties() {
        let mut page = RustExecutableDocument::load(
            "<main id='namedElement'></main><script>globalThis.namedElementResolved=namedElement===document.getElementById('namedElement')</script>",
        )
        .unwrap();
        assert_eq!(page.evaluate("namedElementResolved").unwrap(), "true");
    }

    #[test]
    fn frame_indices_are_native_live_own_properties() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe srcdoc='<input>'></iframe><iframe srcdoc='<p>second</p>'></iframe></body>",
            &BTreeMap::new(), None, Some(Url::parse("https://parent.example.test/").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const elements=document.querySelectorAll('iframe');
            const first=elements[0].contentWindow,second=elements[1].contentWindow;
            if(frames!==window||length!==2||frames[0]!==first||frames[1]!==second||frames[2]!==undefined)return 'lookup';
            const descriptor=Object.getOwnPropertyDescriptor(window,'0');
            if(!descriptor||descriptor.value!==first||descriptor.writable||!descriptor.enumerable)return 'descriptor';
            if(!frames[0].document.querySelector('input'))return 'document';
            elements[0].remove();
            if(length!==1||frames[0]!==second||frames[1]!==undefined)return 'removal';
            elements[1].remove();
            if(length!==0||0 in window)return 'empty';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn frame_named_window_properties_follow_child_and_attribute_names() {
        let mut page = RustExecutableDocument::load_with_environment(r#"<body><div id="originalName"></div><iframe id="theFrame" name="originalName" srcdoc="<script>window.name='childName'</script>"></iframe></body>"#, &BTreeMap::new(), None, Some(Url::parse("https://parent.example.test/").unwrap())).unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const frame=document.querySelector('iframe'),proxy=frame.contentWindow;
            if(childName!==proxy||proxy.name!=='childName')return 'child rename';
            if(originalName!==document.querySelector('div'))return 'old name';
            frame.name='originalName';
            if(originalName!==proxy||proxy.name!=='originalName'||typeof childName!=='undefined')return 'identical attribute';
            proxy.name='childAgain';
            frame.setAttribute('name','attributeName');
            if(attributeName!==proxy||proxy.name!=='attributeName'||typeof childAgain!=='undefined')return 'attribute write';
            frame.removeAttribute('name');
            if(typeof attributeName!=='undefined'||proxy.name!=='')return 'remove name';
            if(theFrame!==frame)return 'id';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn dynamic_named_window_properties_use_native_prototype_and_live_collections() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
            const a=document.createElement('div'),b=document.createElement('img');
            a.id='dynamicNamed'; b.name='dynamicNamed';
            document.body.appendChild(a);
            if(dynamicNamed!==a||Object.hasOwn(globalThis,'dynamicNamed'))return 'insertion';
            document.body.appendChild(b);const collection=dynamicNamed;
            if(!(collection instanceof HTMLCollection)||collection.length!==2)return 'duplicates';
            a.remove();b.name='changedNamed';
            if(collection.length!==0||typeof dynamicNamed!=='undefined'||changedNamed!==b)return 'mutation';
            if(Object.getPrototypeOf(Object.getPrototypeOf(Window.prototype))!==EventTarget.prototype)return 'prototype';
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn script_assignments_shadow_named_window_properties() {
        let mut page = RustExecutableDocument::load(
            "<main id='n00'></main><div id='explicit'></div><i id='transient'></i><script>var n00=document.getElementById('n00');n00.id='changed';n00.setAttribute('kept','yes');Object.defineProperty(globalThis,'explicit',{value:42,writable:true,configurable:true});document.getElementById('transient').remove()</script><script>globalThis.namedAssignments=n00===document.getElementById('changed')&&n00.getAttribute('kept')==='yes'&&explicit===42&&typeof transient==='undefined'</script>",
        ).unwrap();
        assert_eq!(page.evaluate("namedAssignments").unwrap(), "true");
        assert_eq!(
            page.evaluate("n00.id='again';n00===document.getElementById('again')&&explicit===42")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn connected_inline_dynamic_script_executes_before_append_child_returns() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>globalThis.order=[];const dynamic=document.createElement('script');dynamic.textContent=\"order.push('dynamic')\";document.querySelector('body').appendChild(dynamic);order.push('after')</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("order.join(',')").unwrap(), "dynamic,after");
    }

    #[test]
    fn connected_external_dynamic_script_uses_the_resource_scheduler_once() {
        let resources = BTreeMap::from([(
            "dynamic.js".into(),
            "globalThis.dynamicRuns=(globalThis.dynamicRuns||0)+1".into(),
        )]);
        let mut page = RustExecutableDocument::load_with_resources(
            "<body><main></main><script>const dynamic=document.createElement('script');dynamic.setAttribute('src','dynamic.js');dynamic.addEventListener('load',()=>globalThis.dynamicLoaded=true);const body=document.querySelector('body');body.appendChild(dynamic);body.removeChild(dynamic);body.appendChild(dynamic)</script></body>",
            &resources,
        )
        .unwrap();
        assert_eq!(page.evaluate("String(dynamicRuns)").unwrap(), "1");
        assert_eq!(page.evaluate("String(dynamicLoaded)").unwrap(), "true");
    }

    #[test]
    fn inner_html_scripts_remain_inert_when_the_subtree_is_connected() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>globalThis.ran=false;const holder=document.createElement('div');holder.innerHTML='<script>ran=true<\\/script>';document.querySelector('body').appendChild(holder)</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("String(ran)").unwrap(), "false");
    }

    #[test]
    fn connected_dynamic_module_uses_v8_module_evaluation() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>const dynamic=document.createElement('script');dynamic.setAttribute('type','module');dynamic.textContent='globalThis.dynamicModule=41+1;export const value=42';document.querySelector('body').appendChild(dynamic)</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("String(dynamicModule)").unwrap(), "42");
    }

    #[test]
    fn event_current_target_is_the_generated_rust_dom_wrapper() {
        let mut page = RustExecutableDocument::load(
            "<button id=go>Run</button><script>const button=document.getElementById('go');button.addEventListener('click',event=>globalThis.__sameTarget=event.currentTarget===button)</script>",
        )
        .unwrap();
        page.click("#go").unwrap();
        assert_eq!(page.evaluate("String(__sameTarget)").unwrap(), "true");
    }

    #[test]
    fn parser_inserted_document_write_reenters_the_live_tokenizer() {
        let page = RustExecutableDocument::load(
            "<main id=before>before</main><script>document.write('<section id=written>inserted</section>')</script><footer id=after>after</footer>",
        )
        .unwrap();
        let written = page.node_by_id("written").expect("written node");
        assert_eq!(page.document.dom.text_content(written).unwrap(), "inserted");
        let body = page
            .document
            .dom
            .iter()
            .find(|(_, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "body")
            })
            .map(|(node, _)| node)
            .unwrap();
        let ids = page
            .document
            .dom
            .get(body)
            .unwrap()
            .children
            .iter()
            .filter_map(|node| {
                page.document
                    .dom
                    .get(*node)
                    .ok()?
                    .attributes
                    .get_null("id")
                    .cloned()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["before", "written", "after"]);
    }

    #[test]
    fn parser_blocking_script_cannot_observe_later_source_markup() {
        let page = RustExecutableDocument::load(
            "<output id=result></output><script>document.getElementById('result').textContent=String(document.getElementById('later')===null)</script><aside id=later>later</aside>",
        )
        .unwrap();
        let result = page.node_by_id("result").expect("result node");
        assert_eq!(page.document.dom.text_content(result).unwrap(), "true");
        assert!(page.node_by_id("later").is_some());
    }

    #[test]
    fn parser_and_v8_preserve_wrapper_identity_across_pause_and_resume() {
        let page = RustExecutableDocument::load(
            "<main id=root></main><script>globalThis.saved=document.getElementById('root')</script><p id=later></p><script>document.getElementById('later').textContent=String(saved===document.getElementById('root'))</script>",
        )
        .unwrap();
        let later = page.node_by_id("later").expect("later parser node");
        assert_eq!(page.document.dom.text_content(later).unwrap(), "true");
    }

    #[test]
    fn observer_installed_by_blocking_script_receives_later_parser_mutations() {
        let page = RustExecutableDocument::load(
            "<output id=result>false</output><script>const observer=new MutationObserver(records=>{if(records.some(record=>[...record.addedNodes].some(node=>node.id==='later'))){observer.disconnect();document.getElementById('result').textContent='true'}});observer.observe(document.body,{childList:true,subtree:true})</script><p id=later>later</p>",
        )
        .unwrap();
        let result = page.node_by_id("result").expect("observer output");
        assert_eq!(page.document.dom.text_content(result).unwrap(), "true");
    }

    #[test]
    fn parser_inserted_scripts_can_nest_document_write_with_bounded_reentry() {
        let page = RustExecutableDocument::load(
            r#"<script>document.write('<script>document.write(\"<output id=nested>nested</output>\")<\/script>')</script>"#,
        )
        .unwrap();
        let nested = page.node_by_id("nested").expect("nested written node");
        assert_eq!(page.document.dom.text_content(nested).unwrap(), "nested");
    }

    #[test]
    fn document_open_write_close_replaces_contents_without_a_javascript_dom_copy() {
        let page = RustExecutableDocument::load(
            "<p id=old>old</p><script>document.open();document.write('<title>new</title><main id=fresh>fresh</main>');document.close()</script>",
        )
        .unwrap();
        assert!(page.node_by_id("old").is_none());
        let fresh = page.node_by_id("fresh").expect("replacement node");
        assert_eq!(page.document.dom.text_content(fresh).unwrap(), "fresh");
    }

    #[test]
    fn lifecycle_events_run_after_parser_inserted_scripts() {
        let page = RustExecutableDocument::load(
            "<output id=state>initial</output><script>document.addEventListener('DOMContentLoaded',()=>document.querySelector('#state').textContent+='|document-dom');addEventListener('DOMContentLoaded',()=>document.querySelector('#state').textContent+='|window-dom');addEventListener('load',()=>document.querySelector('#state').textContent+='|load')</script>",
        )
        .unwrap();
        let state = page.node_by_id("state").unwrap();
        assert_eq!(
            page.document.dom.text_content(state).unwrap(),
            "initial|document-dom|window-dom|load"
        );
    }

    #[test]
    fn timers_wait_for_deadlines_and_checkpoint_microtasks_between_tasks() {
        let mut page = RustExecutableDocument::load(
            "<output id=state>pending</output><script>globalThis.order=[];setTimeout(()=>{order.push('timer-1');Promise.resolve().then(()=>order.push('microtask-1'))},15);setTimeout(()=>order.push('timer-2'),15)</script>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#state').textContent")
                .unwrap(),
            "pending"
        );
        page.agent.host.drain_scheduled_timers(true).unwrap();
        page.checkpoint().unwrap();
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "timer-1,microtask-1,timer-2"
        );
    }

    #[test]
    fn interval_can_cancel_itself_without_becoming_an_infinite_drain() {
        let mut page = RustExecutableDocument::load(
            "<script>globalThis.count=0;const id=setInterval(()=>{count++;clearInterval(id)},1)</script>",
        )
        .unwrap();
        page.drain_tasks().unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(page.evaluate("String(count)").unwrap(), "1");
    }

    #[test]
    fn native_formdata_preserves_submitter_and_runs_guarded_callbacks() {
        let mut page = RustExecutableDocument::load("<!doctype html><body><form id=f><input name=a value=one><input type=file name=file><button id=b name=button value=yes>send</button><input id=image type=image name=pic><input type=reset name=reset value=no></form></body>").unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            let calls=0;
            f.onformdata = event => {
                calls++;
                if(!(event instanceof FormDataEvent) || event.target!==f || !event.bubbles || event.cancelable || !event.isTrusted) throw Error('event');
                try { new FormData(f); throw Error('recursion accepted'); } catch(error) { if(error.name!=='InvalidStateError') throw error; }
                event.formData.append('callback','added');
            };
            const data = new FormData(f,b);
            if(data.get('button')!=='yes' || data.has('reset') || data.has('pic.x') || data.get('callback')!=='added') return JSON.stringify({button:data.get('button'),reset:data.has('reset'),image:data.has('pic.x'),callback:data.get('callback'),calls});
            const file = data.get('file');
            if(!(file instanceof File) || file.name!=='' || file.size!==0 || file.type!=='application/octet-stream') return 'empty file';
            const imageData = new FormData(f,image);
            if(imageData.get('pic.x')!=='0' || imageData.get('pic.y')!=='0') return 'image';
            if(calls!==2) return 'callback count';
            const event = new FormDataEvent('formdata',{formData:data});
            if(event.formData!==data) return 'event identity';
            try { new FormDataEvent('formdata',{}); return 'missing accepted'; } catch(error) { if(!(error instanceof TypeError)) return 'missing error'; }
            f.onformdata = null;
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn native_file_selection_is_not_an_attribute_and_obeys_input_lifetime() {
        let mut page = RustExecutableDocument::load("<!doctype html><form id=f><input id=a type=file name=upload required value=forged><input id=b type=file></form>").unwrap();
        assert_eq!(page.evaluate("String(f.checkValidity())").unwrap(), "false");
        let a = page.node_by_id("a").unwrap();
        page.set_files(
            a,
            &[crate::upload::UploadFile {
                filename: "selected.txt".into(),
                media_type: "text/plain".into(),
                bytes: b"native bytes".to_vec(),
            }],
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          check(f.checkValidity(),'required selected file rejected');
          check(!a.hasAttribute('data-oryn-upload-files'),'attribute transport remains');
          const files=a.files, file=files[0];
          check(files instanceof FileList&&files===a.files&&files.length===1&&files.item(0)===file&&file.name==='selected.txt','native list');
          check(files.item(1)===null&&files[1]===undefined&&[...files][0]===file,'indexed list');
          check(a.value==='C:\\fakepath\\selected.txt','fakepath');
          a.setAttribute('data-oryn-upload-files','[{"filename":"forged","media_type":"text/plain","body_base64":"eA==","last_modified":0}]');
          check(a.files===files&&new FormData(f).get('upload')===file,'authored attribute changed selection or File identity');
          try { a.value='forged';return 'value accepted'; } catch(e) {check(e.name==='InvalidStateError','value error')}
          b.files=files; check(b.files===files,'assigned FileList identity');
          b.files=null;b.files=undefined;check(b.files===files,'nullable assignment changed files');
          for(const invalid of [[],[file],{},new Proxy(files,{})]) {
            try {b.files=invalid;return 'invalid FileList accepted'} catch(e) {check(e instanceof TypeError,'assignment brand error')}
          }
          a.type='FILE';check(a.files===files,'same type cleared files');
          const clone=a.cloneNode();check(clone.files[0].name==='selected.txt','clone files');
          a.value='';check(a.files.length===0&&files.length===1&&b.files.length===1,'clear snapshot');
          b.type='text';check(b.files===null,'non-file null');b.type='file';check(b.files.length===0,'type transition');
          a.files=files;f.reset();check(a.files.length===0&&!f.checkValidity(),'reset required file');
          try { FileList.prototype.item.call({},0);return 'forged receiver'; } catch(e) {check(e instanceof TypeError,'brand error')}
          return 'ok';
        })()"#).unwrap(),"ok");
    }

    #[test]
    fn native_form_navigation_owns_post_body_after_synchronous_formdata_callbacks() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<form id=f action=/old method=get><input id=a name=x value=initial><button id=b name=button value=yes></button></form>",
            &BTreeMap::new(), None, Some(Url::parse("https://forms.test/start").unwrap()),
        ).unwrap();
        page.execute("globalThis.events=[];f.onformdata=e=>{events.push('formdata');e.formData.append('extra','a\\nb');b.setAttribute('formaction','/accepted?keep=1');b.setAttribute('formmethod','post');f.requestSubmit();};f.requestSubmit(b);a.value='later';events.push('returned')").unwrap();
        assert_eq!(
            page.evaluate("events.join(',')").unwrap(),
            "formdata,returned"
        );
        let request = page.take_location_navigation().unwrap();
        assert_eq!(request.url, "https://forms.test/accepted?keep=1");
        let post = request.post.unwrap();
        assert_eq!(post.content_type, "application/x-www-form-urlencoded");
        assert_eq!(post.bytes, b"x=initial&button=yes&extra=a%0D%0Ab");
    }

    #[test]
    fn native_form_navigation_get_supersession_and_detachment_are_transactional() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<form id=f action='/send?old=discarded#keep'><input id=a name=x value=first></form>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://forms.test/start").unwrap()),
        )
        .unwrap();
        page.execute("f.submit();a.value='second';f.submit();a.value='third'")
            .unwrap();
        let request = page.take_location_navigation().unwrap();
        assert_eq!(request.url, "https://forms.test/send?x=second#keep");
        assert!(request.post.is_none());
        page.execute("f.onformdata=()=>f.remove();f.submit()")
            .unwrap();
        assert!(page.take_location_navigation().is_none());
    }

    #[test]
    fn native_form_data_uses_shared_directionality_and_case_insensitive_charset() {
        let mut page = RustExecutableDocument::load("<form id=f dir=rtl><input name=t value=abc dirname=t.dir><input name=a value='123 ת' dir=auto dirname=a.dir><textarea name=b dir=auto dirname=b.dir>123 A</textarea><input type=checkbox checked name=c dirname=ignored><input type=hidden name=_CHARSET_ value=wrong></form>").unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify([...new FormData(f)])")
                .unwrap(),
            r#"[["t","abc"],["t.dir","rtl"],["a","123 ת"],["a.dir","rtl"],["b","123 A"],["b.dir","ltr"],["c","on"],["_CHARSET_","UTF-8"]]"#
        );
    }

    #[test]
    fn sandboxed_frame_submission_is_blocked_before_events() {
        let mut page = RustExecutableDocument::load_with_environment("<iframe id=frame sandbox='allow-scripts allow-same-origin' srcdoc='<form id=f><input required></form><script>globalThis.seen=[];f.onsubmit=()=>seen.push(\"submit\");f.onformdata=()=>seen.push(\"formdata\");f.firstChild.oninvalid=()=>seen.push(\"invalid\");f.requestSubmit();f.submit();</script>'></iframe>", &BTreeMap::new(), None, Some(Url::parse("https://forms.test/start").unwrap())).unwrap();
        assert_eq!(
            page.evaluate("frame.contentWindow.seen.join(',')").unwrap(),
            ""
        );
        assert!(page.take_location_navigation().is_none());
    }

    #[test]
    fn dialog_submission_constructs_formdata_then_closes_without_navigation() {
        let mut page = RustExecutableDocument::load("<dialog id=d open><form id=f method=dialog><button id=b value=accepted>OK</button></form></dialog>").unwrap();
        page.execute("globalThis.seen=[];f.onformdata=()=>seen.push('data');d.onclose=()=>seen.push('close');f.requestSubmit(b)").unwrap();
        assert_eq!(
            page.evaluate("[d.open,d.returnValue,seen.join(',')].join('|')")
                .unwrap(),
            "false|accepted|data,close"
        );
        assert!(page.take_location_navigation().is_none());
    }

    #[test]
    fn native_semantic_submission_uses_shared_validation_without_duplicate_events() {
        let mut page = RustExecutableDocument::load(
            "<body><a href='#wrong'><form id=f action='#submitted'><select id=s required></select><button id=b>Submit</button></form></a><button id=external form=f>External</button></body>",
        ).unwrap();
        page.execute("globalThis.events=[];s.oninvalid=()=>events.push('invalid');f.onsubmit=e=>{events.push(e instanceof SubmitEvent?'submit':'untyped');e.preventDefault()}").unwrap();
        let button = page.node_by_id("b").unwrap();
        let form = page.node_by_id("f").unwrap();
        let external = page.node_by_id("external").unwrap();
        for (target, action) in [
            (button, SemanticAction::Click),
            (form, SemanticAction::Submit),
            (external, SemanticAction::Click),
        ] {
            page.apply_action(target, action, None).unwrap();
        }
        assert_eq!(
            page.evaluate("events.join()+'|'+location.hash").unwrap(),
            "invalid,invalid,invalid|"
        );
        page.execute("s.required=false;events=[];HTMLFormElement.prototype.requestSubmit=()=>{throw Error('page override reached')}").unwrap();
        for (target, action) in [
            (button, SemanticAction::Click),
            (form, SemanticAction::Submit),
            (external, SemanticAction::Click),
        ] {
            page.apply_action(target, action, None).unwrap();
        }
        assert_eq!(
            page.evaluate("events.join()+'|'+location.hash").unwrap(),
            "submit,submit,submit|"
        );
    }

    #[test]
    fn ordinary_button_click_does_not_schedule_document_navigation() {
        let mut page = RustExecutableDocument::load(
            "<button type=button id=go onclick=\"document.querySelector('#status').textContent='changed'\">Mutate</button><output id=status>before</output>",
        )
        .unwrap();
        let button = page.node_by_id("go").expect("button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("ordinary button click must not navigate");
        let status = page.node_by_id("status").expect("status");
        assert_eq!(page.document.dom.text_content(status).unwrap(), "changed");
        assert_eq!(
            page.evaluate("new URL('fixture:///tmp/page.html').href")
                .unwrap(),
            "fixture:///tmp/page.html"
        );
    }

    #[test]
    fn parsed_inline_location_assignment_updates_the_committed_url() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<button type=button id=go onclick=\"window.location.href='/next.html'\">Next</button>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/start.html").unwrap()),
        )
        .unwrap();
        let button = page.node_by_id("go").expect("button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("inline navigation handler");
        assert_eq!(page.location().unwrap(), "https://example.test/next.html");
    }

    #[cfg(unix)]
    #[test]
    fn location_frame_context_keeps_native_security_origin() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<p>frame origin</p>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://child.example.test/frame").unwrap()),
        )
        .unwrap();
        page.agent.host.execute("globalThis.__oryn_context_call=()=>JSON.stringify({result:'security_origin',serialized:'https://parent.example.test',opaque:false})").unwrap();
        configure_frame_context(&mut page.agent.host, 1).unwrap();
        assert_eq!(
            page.evaluate("[origin,location.origin,__oryn_opaque_origin].join('|')")
                .unwrap(),
            "https://child.example.test|https://child.example.test|false"
        );
        // An inherited srcdoc URL has an opaque URL origin but retains the
        // actual document realm's origin for messaging and access decisions.
        page.agent.host.commit_location("about:srcdoc").unwrap();
        configure_frame_context(&mut page.agent.host, 1).unwrap();
        assert_eq!(
            page.evaluate("[origin,location.origin,__oryn_opaque_origin].join('|')")
                .unwrap(),
            "https://child.example.test|null|false"
        );
    }

    #[test]
    fn window_location_assignment_uses_the_navigation_setter() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<p>remote executor</p>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/start.html").unwrap()),
        )
        .unwrap();
        page.evaluate("window.location='/next.html'")
            .expect("window.location assignment");
        assert_eq!(page.location().unwrap(), "https://example.test/next.html");
    }

    #[test]
    fn location_preserves_its_prototype_and_extensibility() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<p>Location invariants</p>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/start.html").unwrap()),
        )
        .unwrap();
        page.evaluate(
            r#"(() => {
              const target=location, prototype=Object.getPrototypeOf(target);
              const check=(value,message)=>{if(!value)throw new Error(message)};
              const throwsTypeError=callback=>{try{callback()}catch(error){return error instanceof TypeError}return false};
              check(Reflect.setPrototypeOf(target,prototype),'same prototype');
              check(Object.setPrototypeOf(target,prototype)===target,'same prototype identity');
              for(const value of [{},null]){
                check(!Reflect.setPrototypeOf(target,value),'Reflect must reject');
                check(throwsTypeError(()=>Object.setPrototypeOf(target,value)),'Object must throw');
                check(throwsTypeError(()=>{target.__proto__=value}),'legacy setter must throw');
              }
              check(Object.getPrototypeOf(target)===prototype,'prototype retained');
              check(!Reflect.preventExtensions(target),'preventExtensions must reject');
              check(throwsTypeError(()=>Object.preventExtensions(target)),'preventExtensions must throw');
              check(Object.isExtensible(target),'still extensible');
              target.extra=7;check(target.extra===7,'expando works');delete target.extra;
              target.assign('/next.html');
              check(location===target,'Location identity retained');
            })()"#,
        )
        .unwrap();
        assert_eq!(page.location().unwrap(), "https://example.test/next.html");
    }

    #[test]
    fn location_assign_resolves_against_the_committed_url() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<button type=button id=go onclick=\"location.assign('/oauth/callback?code=ready')\">Continue</button>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/oauth/start").unwrap()),
        )
        .unwrap();
        let button = page.node_by_id("go").expect("button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("location.assign navigation handler");
        assert_eq!(
            page.location().unwrap(),
            "https://example.test/oauth/callback?code=ready"
        );
    }

    #[test]
    fn shipping_fixture_back_button_updates_location() {
        let mut page = RustExecutableDocument::load_with_environment(
            include_str!("../../../test-harness/scenarios/intent-tests/flow-shipping.html"),
            &BTreeMap::new(),
            None,
            Some(Url::parse("http://localhost:3000/intent-tests/flow-shipping.html").unwrap()),
        )
        .unwrap();
        let button = {
            let dom = page.agent.dom.borrow();
            dom.arena()
                .iter()
                .find(|(id, node)| {
                    dom.is_connected(*id).unwrap_or(false)
                        && node
                            .attributes
                            .get_null("onclick")
                            .is_some_and(|source| source.contains("flow-cart.html"))
                })
                .map(|(node, _)| node)
                .expect("back button")
        };
        page.apply_action(button, SemanticAction::Click, None)
            .expect("inline back navigation");
        assert_eq!(
            page.location().unwrap(),
            "http://localhost:3000/intent-tests/flow-cart.html"
        );
    }

    #[test]
    fn document_title_reflects_the_rust_owned_title_element() {
        let mut page = RustExecutableDocument::load("<title>Initial</title><main>Body</main>")
            .expect("document");
        assert_eq!(page.evaluate("document.title").unwrap(), "Initial");
        page.execute("document.title='Updated'").unwrap();
        assert_eq!(page.evaluate("document.title").unwrap(), "Updated");
        assert_eq!(
            page.evaluate("document.querySelector('title').textContent")
                .unwrap(),
            "Updated"
        );
    }

    #[test]
    fn prevented_submit_event_keeps_the_document_and_publishes_handler_mutations() {
        let mut page = RustExecutableDocument::load(
            r#"<form id=form action=/charged><button id=submit type=submit>Sign In</button></form>
            <div id=modal style="display:none"></div>
            <script>document.getElementById('form').onsubmit=e=>{e.preventDefault();document.getElementById('modal').style.display='flex'}</script>"#,
        )
        .expect("document");
        let button = page.node_by_id("submit").expect("submit button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("prevented submit must not navigate");
        assert_eq!(page.evaluate("location.href").unwrap(), "about:blank");
        assert_eq!(
            page.evaluate("document.getElementById('modal').style.display")
                .unwrap(),
            "flex"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_owned_storage_is_visible_to_page_scripts() {
        let page = crate::runtime::Browser::new().new_context().new_page();
        page.load_html(
            "https://storage.example.test/app",
            "<script>localStorage.setItem('shared','value');sessionStorage.setItem('tab','one')</script>",
        )
        .await
        .unwrap();
        let values = page
            .evaluate("localStorage.getItem('shared')+'|'+sessionStorage.getItem('tab')")
            .await
            .unwrap();
        assert_eq!(values, "value|one", "trace: {:#?}", page.trace().await);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn denied_popup_is_reported_as_a_capability_failure() {
        let context = crate::runtime::Browser::new()
            .new_context_with_options(crate::runtime::ContextOptions::default().with_popups(false));
        let page = context.new_page();
        page.load_html("https://popup.example.test/app", "<main></main>")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("String(open('/denied'))").await.unwrap(),
            "null"
        );
        let trace = page.trace().await.unwrap();
        assert!(
            trace.iter().any(|entry| matches!(
                &entry.kind,
                crate::trace::TraceEventKind::PolicyDenied { capability }
                    if capability == "popup.open"
            )),
            "{trace:#?}"
        );
    }

    #[test]
    fn response_stream_delivers_every_buffered_chunk() {
        let mut page = RustExecutableDocument::load(
            "<output id=result>pending</output><script>const response=new Response(new Uint8Array([1,2,3]),{bodyChunks:[new Uint8Array([1]),new Uint8Array([2]),new Uint8Array([3])]});(async()=>{const reader=response.body.getReader();let chunks=0,total=0;for(;;){const item=await reader.read();if(item.done)break;chunks++;total+=item.value.length}document.querySelector('#result').textContent=chunks+':'+total})()</script>",
        )
        .unwrap();
        page.execute("__oryn_drain(true)").unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "3:3"
        );
    }

    #[test]
    fn node_error_native_passive_cleanup_and_property_cancellation() {
        let mut page=RustExecutableDocument::load(r#"<body><script>
          globalThis.order=[];const node=document.body;
          onerror=()=>{order.push('error');return true};
          node.addEventListener('probe',e=>{e.preventDefault();throw null},{passive:true});
          node.addEventListener('probe',e=>order.push('later:'+e.defaultPrevented));
          node.onprobe=()=>false;
          globalThis.event=new Event('probe',{cancelable:true});globalThis.accepted=node.dispatchEvent(event);
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate(
                "order.join('|')+':'+accepted+':'+event.defaultPrevented+':'+event.currentTarget"
            )
            .unwrap(),
            "error|later:false:false:true:null"
        );
    }

    #[test]
    fn node_error_native_termination_still_reaches_host_and_removes_once_listener() {
        let mut page=RustExecutableDocument::load("<body><script>document.body.addEventListener('probe',()=>{while(true){}},{once:true})</script>").unwrap();
        page.agent
            .host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        let result = page
            .agent
            .host
            .execute("document.body.dispatchEvent(new Event('probe'))");
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        assert_eq!(
            page.evaluate("document.body.dispatchEvent(new Event('probe'))")
                .unwrap(),
            "true"
        );
        page.agent
            .host
            .execute("document.body.addEventListener('eval-probe',()=>{while(true){}},{once:true})")
            .unwrap();
        let result = page
            .agent
            .host
            .evaluate_string("document.body.dispatchEvent(new Event('eval-probe'))");
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        assert_eq!(page.evaluate("1+1").unwrap(), "2");
    }

    #[test]
    #[ignore = "requires the pinned upstream WPT checkout in ORYN_WPT_ROOT"]
    fn node_error_native_pinned_testharness_rejects_unexpected_errors() {
        let root =
            std::path::PathBuf::from(std::env::var("ORYN_WPT_ROOT").expect("pinned WPT root"));
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://example.test/testharness.js".into(),
            std::fs::read_to_string(root.join("resources/testharness.js")).unwrap(),
        );
        for allow in [false, true] {
            let html = format!(
                r#"<body><script src="https://example.test/testharness.js"></script>
              <script>setup({{explicit_done:true,allow_uncaught_exception:{allow}}});
              globalThis.completion=null;add_completion_callback((tests,status)=>completion=[tests.length,status.status]);</script>
              <script>document.body.addEventListener('probe',()=>{{throw Error('node-contract')}});document.body.dispatchEvent(new Event('probe'));</script>
              <script>test(()=>assert_true(true),'later registration');done()</script>"#
            );
            let mut page = RustExecutableDocument::load_with_environment(
                &html,
                &resources,
                None,
                Some(Url::parse("https://example.test/page").unwrap()),
            )
            .unwrap();
            assert_eq!(
                page.evaluate("JSON.stringify(completion)").unwrap(),
                if allow { "[1,0]" } else { "[0,1]" },
                "allow_uncaught_exception={allow}"
            );
        }
    }

    #[test]
    fn module_error_native_inline_roots_preserve_values_and_identity() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<script>
          globalThis.errors=[];globalThis.conversions=0;globalThis.payload={toString(){conversions++;throw 'unexpected conversion'}};globalThis.order=[];
          onerror=(m,f,l,c,e)=>{errors.push(e);return true};
        </script><script type="module">throw payload</script>
        <script type="module">throw null</script><script type="module">throw undefined</script>
        <script type="module">order.push(import.meta.url,document.currentScript===null)</script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/page").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate("JSON.stringify([errors.length,errors[0]===payload,errors[1]===null,errors[2]===undefined,conversions,...order])").unwrap(),
          "[3,true,true,true,0,\"https://example.test/page\",true]");
    }

    #[test]
    fn module_error_native_compile_and_link_failures_report_syntax_errors() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://example.test/dep.js".into(),
            "export const available=1".into(),
        );
        let mut page=RustExecutableDocument::load_with_environment(r#"<script>
          globalThis.errors=[];onerror=(m,f,l,c,e)=>{errors.push(e instanceof SyntaxError);return true};
        </script><script type="module">let = ;</script>
        <script type="module">import {missing} from './dep.js'</script>"#,
          &resources,None,Some(Url::parse("https://example.test/page").unwrap())).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(errors)").unwrap(),
            "[true,true]"
        );
    }

    #[test]
    fn module_error_native_delayed_rejection_reports_once_and_is_handled() {
        let mut page=RustExecutableDocument::load(r#"<script>
          globalThis.errors=[];globalThis.payload={};onerror=(m,f,l,c,e)=>{errors.push(e);return true};
        </script><script type="module">await new Promise((resolve,reject)=>globalThis.rejectModule=reject)</script>"#).unwrap();
        assert_eq!(page.evaluate("errors.length").unwrap(), "0");
        page.evaluate("rejectModule(payload)").unwrap();
        assert_eq!(
            page.evaluate("errors.length+':'+(errors[0]===payload)")
                .unwrap(),
            "1:true"
        );
        assert!(
            page.agent
                .host
                .take_unhandled_promise_rejections()
                .is_empty()
        );
        assert!(page.agent.host.take_module_error_reports().is_empty());
        page.agent.host.execute("onerror=()=>false").unwrap();
        page.agent
            .host
            .execute_module_script(
                "https://example.test/late.js",
                "await new Promise((resolve,reject)=>globalThis.rejectAgain=reject)",
                &BTreeMap::new(),
                true,
            )
            .unwrap();
        page.agent.host.execute("rejectAgain(payload)").unwrap();
        assert_eq!(page.agent.host.take_module_error_reports().len(), 1);
        assert!(
            page.agent
                .host
                .take_unhandled_promise_rejections()
                .is_empty()
        );
    }

    #[test]
    fn module_error_native_api_propagation_and_shared_dependencies_are_preserved() {
        let mut page = RustExecutableDocument::load(
            "<script>globalThis.errors=0;onerror=()=>{errors++;return true}</script>",
        )
        .unwrap();
        assert!(
            page.agent
                .host
                .execute_module(
                    "https://example.test/syntax.js",
                    "let = ;",
                    &BTreeMap::new()
                )
                .is_err()
        );
        assert!(
            page.agent
                .host
                .execute_module(
                    "https://example.test/runtime.js",
                    "throw 42",
                    &BTreeMap::new()
                )
                .is_err()
        );
        assert_eq!(page.evaluate("errors").unwrap(), "0");
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://example.test/dep.js".into(),
            "globalThis.count=(globalThis.count||0)+1;export const value={}".into(),
        );
        for name in ["a", "b", "a"] {
            page.agent.host
                .execute_module_script(
                    &format!("https://example.test/{name}.js"),
                    "import {value} from './dep.js';globalThis.seen=value;globalThis.roots=(globalThis.roots||0)+1",
                    &resources,
                    true,
                )
                .unwrap();
        }
        assert_eq!(page.evaluate("count+':'+roots").unwrap(), "1:2");
    }

    #[test]
    fn module_error_native_reporting_obeys_wall_deadline() {
        let mut page =
            RustExecutableDocument::load("<script>onerror=()=>{while(true){}}</script>").unwrap();
        page.agent
            .host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(matches!(
            page.agent.host.execute_module_script(
                "https://example.test/fail.js",
                "throw 1",
                &BTreeMap::new(),
                true
            ),
            Err(HostError::LimitExceeded { .. })
        ));
        assert_eq!(page.evaluate("1+1").unwrap(), "2");
    }

    #[test]
    #[ignore = "requires the pinned upstream WPT checkout in ORYN_WPT_ROOT"]
    fn module_error_native_pinned_testharness_rejects_unexpected_errors() {
        let root =
            std::path::PathBuf::from(std::env::var("ORYN_WPT_ROOT").expect("pinned WPT root"));
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://example.test/testharness.js".into(),
            std::fs::read_to_string(root.join("resources/testharness.js")).unwrap(),
        );
        for allow in [false, true] {
            let html = format!(
                r#"<script src="https://example.test/testharness.js"></script>
              <script>setup({{explicit_done:true,allow_uncaught_exception:{allow}}});
              globalThis.completion=null;add_completion_callback((tests,status)=>completion=[tests.length,status.status]);</script>
              <script type="module">throw new Error('module-contract')</script>
              <script type="module">test(()=>assert_true(true),'later registration');done()</script>"#
            );
            let mut page = RustExecutableDocument::load_with_environment(
                &html,
                &resources,
                None,
                Some(Url::parse("https://example.test/page").unwrap()),
            )
            .unwrap();
            assert_eq!(
                page.evaluate("JSON.stringify(completion)").unwrap(),
                if allow { "[1,0]" } else { "[0,1]" },
                "allow_uncaught_exception={allow}"
            );
        }
    }

    #[test]
    fn parser_error_native_preserves_values_and_continues_scripts() {
        let mut page = RustExecutableDocument::load(
            r#"<body><script>
          globalThis.errors=[];globalThis.payload={};globalThis.after=false;
          onerror=function(m,f,l,c,error){errors.push([arguments.length,error,f,l,c]);return true};
        </script><script>throw payload</script><script>throw null</script>
        <script>throw undefined</script><script>globalThis.after=true</script>"#,
        )
        .unwrap();
        assert_eq!(page.evaluate("JSON.stringify([errors.length,errors[0][0],errors[0][1]===payload,errors[1][1]===null,errors[2][1]===undefined,errors.every(e=>e[2]===location.href&&e[3]>0&&e[4]>0),after])").unwrap(),
            "[3,5,true,true,true,true,true]");
        assert!(page.evaluate("throw payload").is_err());
        assert_eq!(page.evaluate("errors.length").unwrap(), "3");
    }

    #[test]
    fn parser_error_native_reports_syntax_and_runs_queued_jobs() {
        let mut page = RustExecutableDocument::load(r#"<body><script>
          globalThis.order=[];
          addEventListener('error',e=>{order.push(e.error instanceof SyntaxError?'syntax':'runtime');e.preventDefault()});
        </script><script>let = ;</script><script>
          Promise.resolve().then(()=>order.push('microtask'));throw 42;
        </script><script>order.push('after')</script>"#).unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "syntax|runtime|microtask|after"
        );
    }

    #[test]
    fn parser_error_native_external_source_location_and_cancellation() {
        let mut resources = BTreeMap::new();
        resources.insert("https://example.test/fail.js".into(), "\nthrow 7".into());
        resources.insert("/fail.js".into(), "\nthrow 7".into());
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<script>globalThis.seen=[];onerror=(m,f,l,c,e)=>{seen=[f,l,c,e];return true}</script>
            <script src="/fail.js"></script>"#,
            &resources,
            None,
            Some(Url::parse("https://example.test/page").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(seen)").unwrap(),
            "[\"https://example.test/fail.js\",2,1,7]"
        );
        assert!(
            !page
                .diagnostics
                .iter()
                .any(|d| format!("{d:?}").contains("Uncaught 7"))
        );
    }

    #[test]
    fn parser_error_native_cross_origin_details_are_muted() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://remote.test/fail.js".into(),
            "throw {secret:42}".into(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<script>globalThis.seen=[];onerror=(...args)=>{seen=args;return true}</script>
            <script src="https://remote.test/fail.js"></script>"#,
            &resources,
            None,
            Some(Url::parse("https://example.test/page").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(seen)").unwrap(),
            "[\"Script error.\",\"\",0,0,null]"
        );
    }

    #[test]
    #[ignore = "requires the pinned upstream WPT checkout in ORYN_WPT_ROOT"]
    fn parser_error_native_pinned_testharness_rejects_unexpected_errors() {
        let root =
            std::path::PathBuf::from(std::env::var("ORYN_WPT_ROOT").expect("pinned WPT root"));
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://example.test/testharness.js".into(),
            std::fs::read_to_string(root.join("resources/testharness.js")).unwrap(),
        );
        for allow in [false, true] {
            let html = format!(
                r#"<script src="https://example.test/testharness.js"></script>
              <script>setup({{explicit_done:true,allow_uncaught_exception:{allow}}});
              globalThis.completion=null;add_completion_callback((tests,status)=>completion=[tests.length,status.status]);</script>
              <script>throw new Error('parser-contract')</script>
              <script>test(()=>assert_true(true),'later registration');done()</script>"#
            );
            let mut page = RustExecutableDocument::load_with_environment(
                &html,
                &resources,
                None,
                Some(Url::parse("https://example.test/page").unwrap()),
            )
            .unwrap();
            assert_eq!(
                page.evaluate("JSON.stringify(completion)").unwrap(),
                if allow { "[1,0]" } else { "[0,1]" },
                "allow_uncaught_exception={allow}"
            );
        }
    }

    #[test]
    fn parser_error_native_reporting_keeps_wall_deadline() {
        let mut page = RustExecutableDocument::load("<body>").unwrap();
        page.agent
            .host
            .execute("onerror=()=>{while(true){}}")
            .unwrap();
        page.agent
            .host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(matches!(
            page.agent
                .host
                .execute_parser_script("throw 1", "https://example.test/fail.js", false),
            Err(HostError::LimitExceeded { .. })
        ));
        assert_eq!(page.evaluate("1+1").unwrap(), "2");
    }

    #[test]
    fn parser_error_native_handler_exceptions_do_not_recurse() {
        let mut page = RustExecutableDocument::load(
            r#"<body><script>
          globalThis.order=[];onerror=()=>{order.push('handler');throw 'nested'};
          addEventListener('error',e=>order.push('listener:'+e.error));
        </script><script>throw 'outer'</script><script>order.push('after')</script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "handler|listener:outer|after"
        );
    }

    #[test]
    fn global_event_native_observer_exceptions_reach_error_handlers() {
        let mut page=RustExecutableDocument::load(r#"<body><script>'use strict';
          globalThis.order=[];const payload={};
          onerror=function(m,f,l,c,error){order.push('handler:'+arguments.length+':'+(error===payload));return true};
          addEventListener('error',e=>order.push('listener:'+(e instanceof ErrorEvent&&e.isTrusted&&e.error===payload&&e.target===window&&e.defaultPrevented)));
          new MutationObserver(()=>{throw payload}).observe(document.body,{attributes:true});
          new MutationObserver(()=>order.push('after')).observe(document.body,{attributes:true});
          document.body.id='changed';
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "handler:5:true|listener:true|after"
        );
    }

    #[test]
    fn cssom_page_transactions_preserve_siblings_and_detached_context() {
        let mut page = RustExecutableDocument::load(r#"<!doctype html><style>@namespace svg url(http://www.w3.org/2000/svg); @page chapter { margin-top: 10px; @top-left { content: 'heading'; } } svg|rect { color: red; }</style>"#).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const sheet = document.styleSheets[0], list = sheet.cssRules;
            const pageRule = list[1], pageStyle = pageRule.style, sibling = list[2];
            const checks = [];
            sheet.insertRule('svg|circle {}', 1);
            pageRule.selectorText = 'chapter:left';
            pageStyle.marginTop = '20px';
            checks.push(list[2] === pageRule, list[3] === sibling,
                pageRule.selectorText === 'chapter:left', pageRule.style === pageStyle,
                pageStyle.marginTop === '20px', pageRule.cssText.includes('@top-left'),
                sibling.selectorText === 'svg|rect', sibling.style.color === 'red');
            sheet.deleteRule(2);
            pageRule.selectorText = 'appendix:right';
            pageStyle.cssText = 'margin-bottom: 30px';
            checks.push(pageRule.parentStyleSheet === null,
                pageRule.selectorText === 'appendix:right', pageStyle.marginBottom === '30px',
                pageRule.cssText.includes('@top-left'), list.length === 3, list[2] === sibling);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn cssom_batch_post_connection_skips_scripts_removed_by_an_earlier_script() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            globalThis.batchScriptCount = 0;
            for (const method of ['appendChild', 'append', 'prepend', 'replaceChildren']) {
                const host = document.createElement('div'); document.body.appendChild(host);
                const first = document.createElement('script');
                globalThis.batchRemovedScript = document.createElement('script');
                first.textContent = 'batchRemovedScript.remove(); batchScriptCount++;';
                batchRemovedScript.textContent = 'batchScriptCount += 100;';
                const fragment = document.createDocumentFragment();
                fragment.appendChild(first); fragment.appendChild(batchRemovedScript);
                if (method === 'appendChild') host.appendChild(fragment);
                else host[method](first, batchRemovedScript);
                host.remove();
            }
            batchScriptCount
        "#
            )
            .unwrap(),
            "4"
        );
    }

    #[test]
    fn implicit_scope_follows_stylesheet_owner_movement() {
        let mut page = RustExecutableDocument::load("<!doctype html><body><section id=left><button id=a>A</button></section><section id=right><button id=b>B</button></section></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const owner = document.createElement('style');
            owner.textContent = '@scope { button { z-index: 7 } :scope { z-index: 9 } }';
            const left = document.getElementById('left'), right = document.getElementById('right');
            const a = document.getElementById('a'), b = document.getElementById('b');
            left.appendChild(owner);
            const values = () => [left, a, right, b].map(n => getComputedStyle(n).zIndex).join(',');
            const results = [values()];
            right.appendChild(owner);
            results.push(values());
            owner.textContent = '@scope { button { z-index: 3 } }';
            results.push(values());
            owner.remove();
            results.push(values());
            results.join('|')
        "#
            )
            .unwrap(),
            "9,7,auto,auto|auto,auto,9,7|auto,auto,auto,3|auto,auto,auto,auto"
        );
    }

    #[test]
    fn stylesheet_media_is_live_and_obsolete_sheets_are_isolated() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const owner = document.createElement('style');
            owner.setAttribute('media', 'screen');
            owner.textContent = 'body { color: rgb(1, 2, 3) }';
            document.head.appendChild(owner);
            const sheet = owner.sheet, media = sheet.media;
            const checks = [media === sheet.media, sheet instanceof StyleSheet,
                media.mediaText === 'screen', media[0] === 'screen'];
            media.mediaText = 'not all';
            checks.push(owner.getAttribute('media') === 'screen');
            checks.push(getComputedStyle(document.body).color !== 'rgb(1, 2, 3)');
            owner.setAttribute('media', 'screen');
            checks.push(media.mediaText === 'screen');
            checks.push(getComputedStyle(document.body).color === 'rgb(1, 2, 3)');
            sheet.media = 'not all';
            checks.push(media.mediaText === 'not all');
            owner.textContent = 'body { color: rgb(4, 5, 6) }';
            const replacement = owner.sheet;
            media.mediaText = 'print';
            checks.push(replacement !== sheet, replacement.media.mediaText === 'screen');
            checks.push(getComputedStyle(document.body).color === 'rgb(4, 5, 6)');
            checks.every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn cssom_batch_insertion_prepares_later_stylesheets_before_scripts() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(page.evaluate(r#"
            const failures = [];
            for (const tag of ['style', 'link']) {
                for (const method of ['appendChild', 'insertBefore', 'replaceChild', 'append', 'prepend', 'replaceChildren', 'before', 'after', 'replaceWith']) {
                    const host = document.createElement('div'), marker = document.createElement('span');
                    host.appendChild(marker); document.body.appendChild(host);
                    globalThis.batchLater = document.createElement(tag);
                    globalThis.batchObserved = [];
                    if (tag === 'style') batchLater.textContent = 'body {}';
                    else { batchLater.rel = 'stylesheet'; batchLater.href = 'data:text/css,'; }
                    const script = document.createElement('script');
                    script.textContent = `
                        batchObserved.push(batchLater.isConnected, !!batchLater.sheet);
                        if (batchLater.tagName === 'STYLE') {
                            batchLater.appendChild(document.createTextNode('body {}'));
                            batchObserved.push(batchLater.sheet.cssRules.length === 2);
                        }
                    `;
                    const fragment = document.createDocumentFragment();
                    fragment.appendChild(script); fragment.appendChild(batchLater);
                    if (method === 'insertBefore') host.insertBefore(fragment, marker);
                    else if (method === 'replaceChild') host.replaceChild(fragment, marker);
                    else if (method === 'before' || method === 'after' || method === 'replaceWith') marker[method](fragment);
                    else if (method === 'append' || method === 'prepend' || method === 'replaceChildren') host[method](script, batchLater);
                    else host.appendChild(fragment);
                    if (batchObserved.length !== (tag === 'style' ? 3 : 2) || !batchObserved.every(Boolean)) failures.push(tag + ':' + method);
                    host.remove();
                }
            }
            failures.join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_media_lists_are_live_native_objects_with_retained_rules() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>@media screen { a { color: red; } }</style>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], rule = sheet.cssRules[0], media = rule.media, child = rule.cssRules[0];
            const checks = [media instanceof MediaList, media === rule.media, media.length === 1,
                media[0] === 'screen', media.item(1) === null, media[1] === undefined, String(media) === 'screen'];
            media.appendMedium('print');
            checks.push(media.length === 2, media[1] === 'print', rule.conditionText === 'screen, print', rule.cssRules[0] === child);
            media.deleteMedium('screen');
            checks.push(media.mediaText === 'print', media.length === 1, rule.cssText.includes('@media print'));
            try { media.deleteMedium('screen'); checks.push(false); } catch(e) { checks.push(e.name === 'NotFoundError'); }
            media.appendMedium('screen, print');
            checks.push(media.mediaText === 'print');
            rule.media = 'all';
            checks.push(media.mediaText === 'all', rule.media === media);
            media.mediaText = null;
            checks.push(media.length === 0, String(media) === '');
            media.mediaText = '???';
            checks.push(media[0] === 'not all');
            sheet.deleteRule(0);
            media.mediaText = 'screen';
            checks.push(rule.parentStyleSheet === null, rule.conditionText === 'screen',
                rule.cssRules[0] === child, child.parentRule === rule, child.parentStyleSheet === null);
            const getter = Object.getOwnPropertyDescriptor(MediaList.prototype, 'mediaText').get;
            for (const value of [{}, Object.create(MediaList.prototype), new Proxy(media, {})]) {
                try { getter.call(value); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
            }
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_conditional_rule_interfaces_expose_native_conditions() {
        let mut page = RustExecutableDocument::load("<!doctype html><style>@supports (color: red) { a {} } @supports (oryn-unknown: yes) { b {} } @container card (width > 1px) { c {} }</style>").unwrap();
        assert_eq!(page.evaluate(r#"
            const rules = document.styleSheets[0].cssRules, yes = rules[0], no = rules[1], container = rules[2];
            const checks = [yes instanceof CSSSupportsRule, yes instanceof CSSConditionRule,
                yes.matches === true, no.matches === false, yes.cssRules[0].parentRule === yes,
                container instanceof CSSContainerRule, container instanceof CSSGroupingRule,
                container.containerName === 'card', container.containerQuery === '(width > 1px)'];
            const conditions = container.conditions;
            checks.push(Object.isFrozen(conditions), conditions.length === 1,
                conditions[0].name === 'card', conditions[0].query === container.containerQuery);
            conditions[0].name = 'changed';
            checks.push(container.conditions[0].name === 'card');
            const matches = Object.getOwnPropertyDescriptor(CSSSupportsRule.prototype, 'matches').get;
            try { matches.call(container); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
            const name = Object.getOwnPropertyDescriptor(CSSContainerRule.prototype, 'containerName').get;
            try { name.call(yes); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_font_face_descriptors_keep_native_identity_and_mutations() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>@font-face { font-family: Example; src: url(a.woff2); font-weight: 100 900; } p { color: red; }</style>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], rule = sheet.cssRules[0], descriptor = rule.style;
            const checks = [rule instanceof CSSFontFaceRule, rule instanceof CSSRule,
                descriptor instanceof CSSFontFaceDescriptors, descriptor instanceof CSSStyleDeclaration,
                descriptor === rule.style, descriptor.parentRule === rule, rule.parentStyleSheet === sheet,
                descriptor.fontFamily === 'Example', descriptor.fontWeight === '100 900',
                descriptor.src.includes('a.woff2'), descriptor.getPropertyValue('color') === ''];
            descriptor.fontWeight = '400';
            checks.push(descriptor.fontWeight === '400', rule === sheet.cssRules[0], descriptor === rule.style);
            descriptor.fontWeight = '400 invalid';
            checks.push(descriptor.fontWeight === '400');
            checks.push(descriptor.removeProperty('font-weight') === '400', descriptor.fontWeight === '');
            rule.style = 'font-family: Second; src: url(b.woff2)';
            checks.push(descriptor.fontFamily === 'Second', descriptor.src.includes('b.woff2'));
            const family = Object.getOwnPropertyDescriptor(CSSFontFaceDescriptors.prototype, 'fontFamily');
            try { family.get.call(sheet.cssRules[1].style); checks.push(false); }
            catch (e) { checks.push(e instanceof TypeError); }
            sheet.deleteRule(0);
            descriptor.fontFamily = 'Detached';
            checks.push(rule.parentStyleSheet === null, descriptor.fontFamily === 'Detached', sheet.cssRules.length === 1);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn tree_walker_traverses_live_skipped_and_rejected_subtrees() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><main><section id='skip'><i id='visible'></i></section><section id='reject'><b id='hidden'></b></section><i id='tail'></i></main>",
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            const root = document.querySelector('main');
            let calls = 0;
            const filter = node => { ++calls; return node.id === 'skip' ? 3 : node.id === 'reject' ? 2 : 1; };
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, filter);
            const checks = [calls === 0, walker.root === root, walker.currentNode === root, walker.filter === filter];
            checks.push(walker.firstChild().id === 'visible', walker.nextSibling().id === 'tail',
                walker.previousSibling().id === 'visible', walker.parentNode() === root);
            checks.push(walker.lastChild().id === 'tail', walker.previousNode().id === 'visible');
            const added = document.createElement('i'); added.id = 'added';
            document.getElementById('visible').after(added);
            checks.push(walker.nextNode() === added, walker.nextNode().id === 'tail', walker.nextNode() === null);
            root.remove(); walker.currentNode = root;
            checks.push(walker.firstChild().id === 'visible');
            const outside = document.createElement('div'), child = document.createElement('b');
            outside.appendChild(child); walker.currentNode = outside;
            checks.push(walker.nextNode() === child, walker.nextNode() === null,
                walker.currentNode === child, walker.previousNode() === outside);
            const host = document.createElement('div'), shadow = host.attachShadow({mode:'open'});
            const inner = document.createElement('i'); shadow.appendChild(inner);
            walker.currentNode = inner;
            checks.push(walker.parentNode() === null, walker.currentNode === inner);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn tree_walker_callback_errors_use_the_filter_realm() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><iframe srcdoc='<!doctype html>'></iframe><main><i></i></main>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const foreign = document.querySelector('iframe').contentWindow;
            const filter = new foreign.Object();
            const walker = document.createTreeWalker(document.querySelector('main'), 1, filter);
            let error;
            try { walker.nextNode(); } catch (caught) { error = caught; }
            error instanceof foreign.TypeError && !(error instanceof TypeError)
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn tree_walker_filters_are_lazy_reentrant_safe_and_preserve_exceptions() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><main><i></i><b></b></main>").unwrap();
        assert_eq!(page.evaluate(r#"
            const root = document.querySelector('main'), sentinel = {};
            const filter = { get acceptNode() { throw sentinel; } };
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, filter);
            const checks = [walker.filter === filter];
            try { walker.nextNode(); checks.push(false); } catch (error) { checks.push(error === sentinel); }
            checks.push(walker.currentNode === root);
            Object.defineProperty(filter, 'acceptNode', {value: function(node) {
                checks.push(this === filter, walker.currentNode === root);
                try { walker.firstChild(); checks.push(false); }
                catch (error) { checks.push(error instanceof DOMException && error.name === 'InvalidStateError'); }
                return { valueOf() { throw sentinel; } };
            }, configurable:true});
            try { walker.nextNode(); checks.push(false); } catch (error) { checks.push(error === sentinel); }
            checks.push(walker.currentNode === root);
            Object.defineProperty(filter, 'acceptNode', {value: () => 65537});
            checks.push(walker.nextNode() === root.firstChild);
            const getter = Object.getOwnPropertyDescriptor(TreeWalker.prototype, 'currentNode').get;
            for (const receiver of [{}, Object.create(TreeWalker.prototype), new Proxy(walker, {}), document.createNodeIterator(root)]) {
                try { getter.call(receiver); checks.push(false); } catch (error) { checks.push(error instanceof TypeError); }
            }
            try { document.createTreeWalker(root, {valueOf() {throw sentinel}}); checks.push(false); }
            catch (error) { checks.push(error === sentinel); }
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn computed_custom_properties_use_stylo_cascade_and_live_cssom_updates() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html><style>
                @property --size { syntax: '<length>'; initial-value: 2px; inherits: false; }
                body { --Case: green; --case: blue; --resolved: var(--Case); --size: 4px; }
                #target { --cycle: var(--cycle); --fallback: var(--absent, orange); }
                .inactive { #target { @scope (&) { --scoped: 1; } } }
            </style><body class="active"><div id="target" style="--invalid: var(--missing)"></div></body>"#,
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            const target = document.getElementById('target'), computed = getComputedStyle(target);
            const read = name => computed.getPropertyValue(name);
            const checks = [read('--Case') === 'green', read('--case') === 'blue',
                read('--resolved') === 'green', read('--fallback') === 'orange',
                read('--cycle') === '', read('--invalid') === '', read('--scoped') === ''];
            checks.push(read('--size') === '2px',
                getComputedStyle(document.body).getPropertyValue('--size') === '4px');
            document.styleSheets[0].cssRules[3].selectorText = '.active';
            checks.push(read('--scoped') === '1');
            document.styleSheets[0].cssRules[1].style.setProperty('--Case', 'purple');
            checks.push(read('--Case') === 'purple', read('--resolved') === 'purple', read('--case') === 'blue');
            const names = Array.from({length: computed.length}, (_, i) => computed.item(i));
            checks.push(names.includes('--Case'), !names.includes('--cycle'), !names.includes('--invalid'));
            target.remove();
            checks.push(read('--Case') === '');
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_nested_declarations_preserve_native_styles_and_detached_identity() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>@scope (body) { color: red; span {} color: green; }</style><body></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], scope = sheet.cssRules[0];
            const rule = scope.cssRules[2], style = rule.style;
            const checks = [scope.cssRules.length === 3,
                scope.cssRules[0] instanceof CSSNestedDeclarations,
                rule instanceof CSSNestedDeclarations, rule instanceof CSSRule,
                !(rule instanceof CSSStyleRule), !(rule instanceof CSSGroupingRule),
                rule.type === 0, rule.parentRule === scope, rule.parentStyleSheet === sheet,
                style instanceof CSSStyleProperties, style instanceof CSSStyleDeclaration,
                style === rule.style, style.parentRule === rule,
                rule.cssText === 'color: green;', style.color === 'green'];
            style.color = 'purple';
            checks.push(rule.cssText === 'color: purple;', scope.cssRules[2] === rule,
                getComputedStyle(document.body).color === 'rgb(128, 0, 128)');
            rule.style = 'color: blue; width: 12px';
            checks.push(rule.style === style, style.width === '12px',
                getComputedStyle(document.body).color === 'rgb(0, 0, 255)');
            const getter = Object.getOwnPropertyDescriptor(CSSNestedDeclarations.prototype, 'style').get;
            for (const receiver of [{}, scope.cssRules[1], scope,
                Object.create(CSSNestedDeclarations.prototype), new Proxy(rule, {})]) {
                try { getter.call(receiver); checks.push(false); }
                catch (e) { checks.push(e instanceof TypeError); }
            }
            scope.deleteRule(2);
            style.setProperty('color', 'orange', 'important');
            checks.push(rule.parentRule === null, rule.parentStyleSheet === null,
                style.color === 'orange', style.getPropertyPriority('color') === 'important',
                scope.cssRules.length === 2, getComputedStyle(document.body).color === 'rgb(255, 0, 0)');
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_rule_declaration_subtypes_use_generated_prototype_accessors() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>a { color: red; } @page named { margin-top: 2px; }</style>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], style = sheet.cssRules[0], page = sheet.cssRules[1];
            const checks = [style instanceof CSSStyleRule, page instanceof CSSPageRule,
                style.style instanceof CSSStyleProperties, page.style instanceof CSSPageDescriptors,
                style.style instanceof CSSStyleDeclaration, page.style instanceof CSSStyleDeclaration];
            for (const rule of [style, page]) {
                checks.push(!Object.hasOwn(rule, 'style'), !Object.hasOwn(rule, 'selectorText'), rule.style === rule.style);
            }
            style.style = 'color: green; float: left;';
            checks.push(style.style.color === 'green', style.style.cssFloat === 'left', style.style[0] === 'color');
            page.style.marginTop = '5px';
            checks.push(page.style['margin-top'] === '5px');
            page.style['margin-top'] = '7px';
            checks.push(page.style.marginTop === '7px');
            CSSStyleDeclaration.prototype.setProperty.call(page.style, 'margin-bottom', '3px');
            checks.push(page.style.marginBottom === '3px');
            const selector = Object.getOwnPropertyDescriptor(CSSStyleRule.prototype, 'selectorText');
            const pageMargin = Object.getOwnPropertyDescriptor(CSSPageDescriptors.prototype, 'marginTop');
            selector.set.call(style, 'b');
            checks.push(selector.get.call(style) === 'b');
            for (const callback of [() => selector.get.call(page), () => selector.set.call(page, 'c'),
                () => pageMargin.get.call(style.style), () => pageMargin.set.call(style.style, '1px')]) {
                try { callback(); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
            }
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_nested_rules_preserve_live_identity_and_detached_context() {
        let mut page = RustExecutableDocument::load(r#"<!doctype html><style>@namespace svg url(http://www.w3.org/2000/svg); @media all { @layer theme { svg|rect { color: red; } } } aside { color: blue; }</style>"#).unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], media = sheet.cssRules[1];
            const layer = media.cssRules[0], list = layer.cssRules, rule = list[0];
            const checks = [];
            checks.push(media instanceof CSSGroupingRule, media instanceof CSSConditionRule,
                media.conditionText === 'all', layer instanceof CSSLayerBlockRule,
                layer.name === 'theme', list instanceof CSSRuleList,
                list === layer.cssRules, list.item(0) === rule,
                rule.parentRule === layer, layer.parentRule === media,
                media.parentRule === null, rule.parentStyleSheet === sheet);
            layer.insertRule('svg|circle { color: green; }', 0);
            checks.push(list.length === 2, list[1] === rule);
            rule.style.color = 'purple';
            rule.selectorText = 'svg|path';
            checks.push(list[1] === rule, rule.style.color === 'purple',
                media.cssText.includes('svg|path'), sheet.cssRules[2].selectorText === 'aside');
            layer.deleteRule(0);
            checks.push(list[0] === rule, list.length === 1);
            sheet.insertRule('div {}', 1);
            checks.push(sheet.cssRules[2] === media);
            sheet.deleteRule(2);
            checks.push(media.parentStyleSheet === null, layer.parentRule === media,
                rule.parentStyleSheet === null, rule.parentRule === layer);
            rule.selectorText = 'svg|circle';
            rule.style.color = 'orange';
            checks.push(rule.selectorText === 'svg|circle', media.cssText.includes('orange'));
            layer.deleteRule(0);
            checks.push(rule.parentRule === null, rule.parentStyleSheet === null, list.length === 0);
            rule.selectorText = 'svg|rect';
            rule.style.color = 'green';
            checks.push(rule.selectorText === 'svg|rect', rule.style.color === 'green',
                list.length === 0, sheet.cssRules.length === 3);
            layer.insertRule('svg|path {}');
            checks.push(list.length === 1, list[0].parentRule === layer,
                list[0].parentStyleSheet === null);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_grouping_mutations_validate_brands_and_conversion_order() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><style>@media all { a {} b {} }</style>")
                .unwrap();
        assert_eq!(page.evaluate(r#"
            const group = document.styleSheets[0].cssRules[0], list = group.cssRules;
            const checks = [], insert = CSSGroupingRule.prototype.insertRule;
            const getter = Object.getOwnPropertyDescriptor(CSSGroupingRule.prototype, 'cssRules').get;
            for (const value of [{}, Object.create(CSSGroupingRule.prototype), new Proxy(group, {})]) {
                try { getter.call(value); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
                try { insert.call(value, 'c {}'); checks.push(false); } catch(e) { checks.push(e.name === 'TypeError'); }
            }
            const sentinel = {};
            try { group.insertRule('c {}', {valueOf() {throw sentinel;}}); checks.push(false); }
            catch(e) { checks.push(e === sentinel); }
            checks.push(list.length === 2);
            try { group.insertRule('???', {valueOf() {group.deleteRule(1); return 2;}}); checks.push(false); }
            catch(e) { checks.push(e.name === 'IndexSizeError'); }
            checks.push(list.length === 1);
            const original = list[0];
            for (const [text, name] of [['???', 'SyntaxError'], ['@import url(x.css);', 'HierarchyRequestError']]) {
                try { group.insertRule(text); checks.push(false); } catch(e) { checks.push(e.name === name); }
                checks.push(list.length === 1, list[0] === original);
            }
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_stylesheet_mutations_use_native_hierarchy_and_preserve_rule_identity() {
        let mut page = RustExecutableDocument::load(r#"<!doctype html><style>@namespace svg url(http://www.w3.org/2000/svg); svg|rect { color: red; }</style>"#).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const sheet = document.styleSheets[0], list = sheet.cssRules, original = list[1];
            const checks = [];
            function rejects(name, fn) {
                const before = Array.from(list, r => r.cssText).join('\n');
                try { fn(); checks.push(false); }
                catch (error) { checks.push(error.name === name); }
                checks.push(Array.from(list, r => r.cssText).join('\n') === before);
            }
            rejects('IndexSizeError', () => sheet.insertRule('???', 3));
            rejects('SyntaxError', () => sheet.insertRule('svg|circle {} garbage', 2));
            rejects('SyntaxError', () => sheet.insertRule('svg|circle {} svg|path {}', 2));
            rejects('HierarchyRequestError', () => sheet.insertRule('@import url(other.css);', 2));
            rejects('InvalidStateError', () => sheet.deleteRule(0));
            rejects('InvalidStateError', () => sheet.removeRule(0));
            const inserted = sheet.insertRule('svg|circle { color: blue; }', 1);
            checks.push(inserted === 1, list === sheet.cssRules, list[2] === original,
                list[1].selectorText === 'svg|circle', list[1].style.color === 'blue');
            sheet.deleteRule(1);
            checks.push(list.length === 2, list[1] === original);
            original.style.color = 'green';
            checks.push(original.style.color === 'green', list[1] === original);
            const constructed = new CSSStyleSheet();
            try { constructed.insertRule('@import url(other.css);'); checks.push(false); }
            catch (error) { checks.push(error.name === 'SyntaxError'); }
            checks.push(constructed.cssRules.length === 0);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn cssom_stylesheet_index_conversion_precedes_native_validation() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><style>a {} b {}</style>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const sheet = document.styleSheets[0], list = sheet.cssRules, first = list[0];
            const sentinel = {}, checks = [];
            try { sheet.insertRule('c {}', { valueOf() { throw sentinel; } }); checks.push(false); }
            catch (error) { checks.push(error === sentinel); }
            checks.push(list.length === 2, list[0] === first);
            try { sheet.deleteRule({ valueOf() { throw sentinel; } }); checks.push(false); }
            catch (error) { checks.push(error === sentinel); }
            checks.push(list.length === 2);
            try {
                sheet.insertRule('???', { valueOf() { sheet.deleteRule(1); return 2; } });
                checks.push(false);
            } catch (error) { checks.push(error.name === 'IndexSizeError'); }
            checks.push(list.length === 1, list[0] === first);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn cssom_style_rule_selector_accepts_first_line_without_matching_the_host() {
        let mut page = RustExecutableDocument::load("<!doctype html><style>.unused { background-color: blue; } #probe { background-color: red; }</style><div id='probe'>text</div>").unwrap();
        assert_eq!(page.evaluate(r#"
            const rule = document.styleSheets[0].cssRules[0], target = document.getElementById('probe');
            const checks = [];
            for (const selector of ['::first-line', ':first-line', '#probe::first-line', '#probe:first-line']) {
                rule.selectorText = selector;
                const expected = selector.replace(/(?<!:):first-line$/, '::first-line');
                checks.push(rule.selectorText === expected, rule.style.backgroundColor === 'blue',
                    getComputedStyle(target).backgroundColor === 'rgb(255, 0, 0)');
            }
            checks.every(Boolean)
        "#).unwrap(), "true");
    }

    #[test]
    fn cssom_reconnecting_style_preserves_old_sheets_and_reparses_authored_text() {
        let mut page = RustExecutableDocument::load("<!doctype html><div id='host'><style id='styles'>#probe { width: 10px; }</style></div><div id='probe'></div>").unwrap();
        assert_eq!(page.evaluate(r#"
            const owner = document.getElementById('styles'), host = document.getElementById('host');
            const sheet = owner.sheet, rules = sheet.cssRules;
            sheet.insertRule('#probe { width: 20px; }', 1);
            host.remove();
            const checks = [owner.sheet === null, rules.length === 2];
            document.body.append(host);
            const next = owner.sheet;
            checks.push(next !== sheet, next.cssRules.length === 1, rules.length === 2,
                getComputedStyle(document.getElementById('probe')).width === '10px');
            rules[1].style.width = '40px';
            checks.push(rules[1].style.width === '40px', getComputedStyle(document.getElementById('probe')).width === '10px');
            checks.every(Boolean)
        "#).unwrap(), "true");
    }

    #[test]
    fn cssom_rule_lists_are_live_readonly_native_collections() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id='styles'>a { color: red; }</style>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const element = document.getElementById('styles'), sheet = element.sheet, list = sheet.cssRules, first = list[0];
            const checks = [list instanceof CSSRuleList, !Array.isArray(list), list === sheet.rules,
                list === sheet.cssRules, list.length === 1, list.item(0) === first, list.item(9) === null, list[9] === undefined];
            const descriptor = Object.getOwnPropertyDescriptor(list, '0');
            checks.push(descriptor.value === first, !descriptor.writable, descriptor.enumerable,
                !Reflect.set(list, '0', null), !Reflect.defineProperty(list, '0', {value: null}), !Reflect.deleteProperty(list, '0'), Reflect.deleteProperty(list, '9'));
            sheet.insertRule('b { color: blue; }', 0);
            checks.push(list.length === 2, list[1] === first, list.item(0).style.color === 'blue');
            sheet.deleteRule(0);
            checks.push(list.length === 1, list[0] === first, Object.keys(list).join(',') === '0');
            const getter = Object.getOwnPropertyDescriptor(CSSRuleList.prototype, 'length').get;
            for (const receiver of [{}, Object.create(list), new Proxy(list, {})]) {
                try { getter.call(receiver); checks.push(false); } catch (e) { checks.push(e instanceof TypeError); }
            }
            element.textContent = 'c { color: green; } d { color: purple; }';
            const nextSheet = element.sheet, nextList = nextSheet.cssRules;
            checks.push(nextSheet !== sheet, nextList !== list, list.length === 1, list[0] === first,
                first.parentStyleSheet === sheet, nextList.length === 2, nextList[0].selectorText === 'c');
            checks.push(nextList.item({valueOf() { nextSheet.deleteRule(0); return 0; }}).selectorText === 'd');
            sheet.insertRule('aside { color: orange; }', 0);
            checks.push(list.length === 2, nextList.length === 1, nextList[0].selectorText === 'd');
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_style_declarations_are_live_and_use_prototype_methods() {
        let mut page = RustExecutableDocument::load(r#"<!doctype html><style>#target { color: red; & span { opacity: .5; } }</style><div id="target"></div><div id="other"></div>"#).unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], rule = sheet.cssRules[0], style = rule.style;
            const proto = CSSStyleDeclaration.prototype;
            const checks = [style instanceof CSSStyleDeclaration, style.parentRule === rule, style === rule.style,
                !Object.hasOwn(style, 'color'), !Object.hasOwn(style, 'setProperty'), rule.type === 1];
            proto.setProperty.call(style, 'color', 'blue', 'important');
            checks.push(proto.getPropertyValue.call(style, 'color') === 'blue', style.color === 'blue',
                proto.getPropertyPriority.call(style, 'color') === 'important', getComputedStyle(document.getElementById('target')).color === 'rgb(0, 0, 255)');
            sheet.insertRule('aside { color: green; }', 0);
            style.color = 'purple';
            checks.push(rule === sheet.cssRules[1], sheet.cssRules[0].style.color === 'green');
            rule.selectorText = '#other';
            checks.push(rule.selectorText === '#other', rule.cssText.includes('opacity: 0.5'),
                getComputedStyle(document.getElementById('other')).color === 'rgb(128, 0, 128)');
            rule.selectorText = 'invalid {';
            checks.push(rule.selectorText === '#other');
            rule.style = 'color: orange; width: 10px';
            checks.push(rule.style === style, style[0] === 'color', style.length === 2, rule.cssText.includes('opacity: 0.5'));
            sheet.deleteRule(1);
            proto.setProperty.call(style, 'color', 'red');
            checks.push(rule.parentStyleSheet === null, style.parentRule === rule, style.color === 'red', sheet.cssRules.length === 1);
            checks.map((ok, i) => ok ? '' : i).filter(x => x !== '').join(',')
        "#).unwrap(), "");
    }

    #[test]
    fn cssom_namespace_rules_have_native_brands_and_parent_links() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html><style>@namespace svg url("http://www.w3.org/2000/svg");</style>"#,
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], rule = sheet.cssRules[0];
            const getter = Object.getOwnPropertyDescriptor(CSSNamespaceRule.prototype, 'prefix').get;
            const rejects = receiver => { try { getter.call(receiver); return false; } catch(e) { return e instanceof TypeError; } };
            const before = rule.cssText;
            rule.cssText = 'discarded';
            const checks = [rule instanceof CSSNamespaceRule, rule instanceof CSSRule, rule.type === 10,
                rule.prefix === 'svg', rule.namespaceURI === 'http://www.w3.org/2000/svg', rule.parentRule === null,
                rule.parentStyleSheet === sheet, rule === sheet.cssRules[0], rule.cssText === before,
                rejects({}), rejects(Object.create(rule)), rejects(new Proxy(rule, {})), !Object.hasOwn(rule, 'prefix')];
            sheet.deleteRule(0);
            checks.push(rule.parentStyleSheet === null, rule.prefix === 'svg');
            checks.every(Boolean)
        "#).unwrap(), "true");
    }

    #[test]
    fn cssom_rule_views_keep_selector_strings_and_namespace_context() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html><style>
            @namespace svg url("http://www.w3.org/2000/svg");
            svg|rect[data-token="{"] { color: red; & > svg|circle { color: blue; } }
        </style>"#,
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0];
            const rule = sheet.cssRules[1];
            [sheet.cssRules.length === 2, rule.selectorText === 'svg|rect[data-token="{"]', rule.style.color === 'red', rule === sheet.cssRules[1]].join('|')
        "#).unwrap(), "true|true|true|true");
    }

    #[test]
    fn cssom_page_declaration_indices_and_style_assignment_are_live() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>@page { margin-top: 1cm; }</style>",
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const rule = document.styleSheets[0].cssRules[0], style = rule.style;
            const initial = style[0] === 'margin-top';
            rule.style = 'margin-left: 2cm; margin-right: 3cm';
            const assigned = rule.style === style && style.length === 2 && style[0] === 'margin-left' && style[1] === 'margin-right';
            style.removeProperty('margin-left');
            const shifted = style[0] === 'margin-right' && style[1] === undefined;
            const descriptor = Object.getOwnPropertyDescriptor(style, '0');
            [initial, assigned, shifted, descriptor.value === 'margin-right', !descriptor.writable, Object.keys(style).includes('0'), !Reflect.set(style, '0', 'wrong')].join('|')
        "#).unwrap(), "true|true|true|true|true|true|true");
    }

    #[test]
    fn cssom_page_declarations_are_live_and_preserve_nested_rules() {
        let mut page = RustExecutableDocument::load("<!doctype html><style>@page named { margin: 1cm; @top-left { content: 'heading'; } }</style>").unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0], rule = sheet.cssRules[0], style = rule.style;
            const stable = style === rule.style && style.parentRule === rule;
            style.setProperty('margin-left', '2cm', 'important');
            const set = style.marginLeft === '2cm' && style.getPropertyPriority('margin-left') === 'important' && rule.cssText.includes('heading');
            const old = style.removeProperty('margin-left');
            style.cssText = 'margin-top: 3cm; invalid: value';
            const replaced = style.length === 1 && style.item(0) === 'margin-top' && style.marginTop === '3cm' && rule.cssText.includes('heading');
            sheet.deleteRule(0);
            style.marginTop = '4cm';
            [stable, set, old === '2cm', replaced, style.parentRule === rule, rule.parentStyleSheet === null, rule.cssText.includes('4cm'), sheet.cssRules.length === 0].join('|')
        "#).unwrap(), "true|true|true|true|true|true|true|true");
    }

    #[test]
    fn cssom_page_rule_replacement_detaches_identical_rules_immediately() {
        let mut page = RustExecutableDocument::load("<!doctype html><body>").unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = new CSSStyleSheet();
            sheet.replaceSync('@page {}');
            const list = sheet.cssRules, old = list[0];
            sheet.replaceSync('@page {}');
            const replaced = old.parentStyleSheet === null && list.length === 1 && list[0] !== old;
            old.selectorText = ':right';
            const independent = list[0].selectorText === '' && old.selectorText === ':right';
            const current = list[0];
            current.selectorText = { toString() { sheet.replaceSync('@page named {}'); return ':left'; } };
            [replaced, independent, current.parentStyleSheet === null,
                current.selectorText === ':left', list[0].selectorText === 'named'].join('|')
        "#).unwrap(), "true|true|true|true|true");
    }

    #[test]
    fn cssom_page_rule_mutation_preserves_identity_and_detaches() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><style>@page {} @page {}</style>")
                .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.styleSheets[0];
            const first = sheet.cssRules[0], second = sheet.cssRules[1];
            const same = sheet.cssRules[0] === first && sheet.cssRules[1] === second && first.type === CSSRule.PAGE_RULE;
            first.selectorText = 'named:First';
            const changed = first.cssText === '@page named:first { }' &&
                sheet.cssRules[0] === first && second.selectorText === '';
            first.selectorText = 'named :first';
            const invalid = first.selectorText === 'named:first';
            const parent = first.parentStyleSheet === sheet;
            sheet.deleteRule(0);
            const detached = first.parentStyleSheet === null && sheet.cssRules[0] === second;
            first.selectorText = ':left';
            [same, changed, invalid, parent, detached, first.cssText === '@page :left { }', second.selectorText === ''].join('|')
        "#).unwrap(), "true|true|true|true|true|true|true");
    }

    #[test]
    fn cssom_layer_edits_change_order_without_rewriting_style_text() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html>
            <style id='order'></style><style>
            @layer first { #probe { color: green; } }
            @layer second { #probe { color: red; } }
            </style><div id='probe'></div>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const owner = document.getElementById('order');
            const sheet = owner.sheet;
            const computed = getComputedStyle(document.getElementById('probe'));
            const colors = [computed.color];
            sheet.insertRule('@layer second {}', 0);
            colors.push(computed.color);
            sheet.deleteRule(0);
            colors.push(computed.color);
            colors.concat([owner.textContent === '']).join('|')
        "#
            )
            .unwrap(),
            "rgb(255, 0, 0)|rgb(0, 128, 0)|rgb(255, 0, 0)|true"
        );
    }

    #[test]
    fn cssom_import_rules_keep_their_indices_and_conditions() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><style id='sheet'></style><body>")
                .unwrap();
        assert_eq!(page.evaluate(r#"
            const sheet = document.getElementById('sheet').sheet;
            sheet.insertRule('@import url("unloaded.css") layer(theme) supports(display:block) screen;', 0);
            const rule = sheet.cssRules[0];
            const valid = !!rule && rule.cssText.includes('layer(theme)') &&
                rule.cssText.includes('supports(') && rule.cssText.includes('screen');
            sheet.deleteRule(0);
            [valid, sheet.cssRules.length].join('|')
        "#).unwrap(), "true|0");
    }

    #[test]
    fn cssom_svg_urls_follow_cascade_inheritance_and_live_updates() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html>
            <style>
            svg { marker-start: url(https://example.test/inherited.svg#start); }
            path { filter: url(https://example.test/filter.svg#effect);
                   marker-mid: url(https://example.test/mid.svg#marker);
                   marker-end: url(https://example.test/end.svg#marker); }
            </style><svg><path id='probe'></path></svg>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const probe = document.getElementById('probe');
            const computed = getComputedStyle(probe);
            const before = [computed.filter.includes('filter.svg#effect'),
                computed.markerStart.includes('inherited.svg#start'),
                computed.markerMid.includes('mid.svg#marker'),
                computed.markerEnd.includes('end.svg#marker')];
            probe.style.setProperty('marker-start', 'url(https://example.test/changed.svg#marker)');
            probe.style.setProperty('filter', 'url(https://example.test/changed.svg#effect)');
            before.concat([computed.markerStart.includes('changed.svg#marker'),
                computed.filter.includes('changed.svg#effect')]).join('|')
        "#
            )
            .unwrap(),
            "true|true|true|true|true|true"
        );
    }

    #[test]
    fn cssom_stylesheet_replacement_requires_a_constructed_sheet() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id='sheet'>div { color: red; }</style><body>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const sheet = document.getElementById('sheet').sheet;
            const before = sheet.cssRules[0].cssText;
            let syncError = '';
            try { sheet.replaceSync('div { color: blue; }'); }
            catch (error) { syncError = error.name + ':' + (error instanceof DOMException); }
            globalThis.replacementError = '';
            const result = sheet.replace('div { color: green; }');
            result.catch(error => { globalThis.replacementError = error.name; });
            [syncError, result instanceof Promise, sheet.cssRules[0].cssText === before].join('|')
        "#
            )
            .unwrap(),
            "NotAllowedError:true|true|true"
        );
        assert_eq!(
            page.evaluate("replacementError").unwrap(),
            "NotAllowedError"
        );
    }

    #[test]
    fn cssom_stylesheet_replacement_accepts_empty_parse_results() {
        let mut page = RustExecutableDocument::load("<!doctype html><body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const sheet = new CSSStyleSheet();
            sheet.replaceSync('div { color: red; }');
            const before = sheet.cssRules.length;
            sheet.replaceSync('@unknown { ignored: yes; }');
            const empty = sheet.cssRules.length;
            sheet.replaceSync('@import url("ignored.css"); div { color: blue; }');
            const mixed = sheet.cssRules.length;
            sheet.replaceSync('@import url("ignored.css");');
            [before, empty, mixed, sheet.cssRules.length].join('|')
        "#
            )
            .unwrap(),
            "1|0|1|0"
        );
    }

    #[test]
    fn cssom_stylesheet_indices_follow_the_parsed_rule_list() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id='sheet'>@unknown { ignored: yes; } #probe { width: 10px; } #probe { width: 20px; }</style><div id='probe'></div>",
        ).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const owner = document.getElementById('sheet');
            const sheet = owner.sheet;
            const before = sheet.cssRules.length;
            sheet.deleteRule(0);
            [before, sheet.cssRules.length,
             sheet.cssRules[0].cssText.includes('20px'),
             owner.textContent.includes('@unknown')].join('|')
        "#
            )
            .unwrap(),
            "2|1|true|true"
        );
    }

    #[test]
    fn cssom_stylesheet_edits_preserve_dom_children_and_update_rendering() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style id='sheet'>#probe { width: 10px; }</style><div id='probe'></div>",
        ).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const owner = document.getElementById('sheet');
            const probe = document.getElementById('probe');
            const sheet = owner.sheet;
            const child = owner.firstChild;
            const authored = owner.textContent;
            const observer = new MutationObserver(() => {});
            observer.observe(owner, { childList: true, characterData: true, subtree: true });
            const computed = getComputedStyle(probe);
            const widths = [computed.width];
            sheet.insertRule('#probe { width: 20px; }', 1);
            widths.push(computed.width);
            sheet.deleteRule(0);
            widths.push(computed.width);
            const unchanged = owner.firstChild === child && owner.textContent === authored;
            const records = observer.takeRecords().length;
            const clone = owner.cloneNode(true);
            child.data = '#probe { width: 30px; }';
            widths.push(computed.width);
            [widths.join(','), unchanged, records, clone.textContent === authored,
             sheet.cssRules.length, sheet.cssRules[0].cssText.includes('20px'),
             owner.sheet !== sheet, owner.sheet.cssRules[0].cssText.includes('30px')].join('|')
        "#
            )
            .unwrap(),
            "10px,20px,20px,30px|true|0|true|1|true|true|true"
        );
    }

    #[test]
    fn computed_cssom_insets_survive_shorthand_serialization() {
        let mut page = RustExecutableDocument::load(
            "<!doctype html><style>#probe { right: 9px !important; }</style><div id='probe'></div>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const probe=document.getElementById('probe');
            probe.style.cssText='display: none; top: 1px; right: 2px; bottom: 3px; left: 4px';
            const computed=getComputedStyle(probe);
            const before=[computed.top,computed.right,computed.bottom,computed.left].join(',');
            probe.style.setProperty('inset','5px 6px 7px 8px');
            const after=[computed.top,computed.right,computed.bottom,computed.left].join(',');
            before+'|'+after
        "#
            )
            .unwrap(),
            "1px,9px,3px,4px|5px,9px,7px,8px"
        );
    }

    #[test]
    fn cssom_property_catalogue_installs_native_accessors() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><body><div id='probe'></div>").unwrap();
        let catalogue = serde_json::to_string(&oryn_layout::cssom::supported_properties()).unwrap();
        let source = format!("globalThis.catalogue={catalogue};")
            + r#"
            const proto=CSSStyleDeclaration.prototype, style=document.getElementById('probe').style;
            const supported=catalogue.every(([name])=>{
                const descriptor=Object.getOwnPropertyDescriptor(proto,name);
                return descriptor&&typeof descriptor.get==='function'&&typeof descriptor.set==='function';
            });
            style.float='left';style.paddingTop='12px';style['margin-inline-start']='3px';
            const values=style.cssFloat==='left'&&style.getPropertyValue('float')==='left'
                &&style.getPropertyValue('padding-top')==='12px'&&style.marginInlineStart==='3px';
            const native=!Object.hasOwn(style,'float')&&!Object.hasOwn(style,'paddingTop');
            style.paddingTop=null;const removed=style.getPropertyValue('padding-top')==='';
            const computed=getComputedStyle(document.getElementById('probe'));let readonly=false;
            try{computed.marginInlineStart='4px'}catch(e){readonly=e.name==='NoModificationAllowedError'}
            [supported,values,native,removed,readonly].join('|')
        "#;
        assert_eq!(page.evaluate(&source).unwrap(), "true|true|true|true|true");
    }

    #[test]
    fn keyboard_editing_targets_focus_inside_nested_shadow_roots() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        page.execute(r#"
            globalThis.shadowKeyboardResult='pending';
            (async()=>{
                const checks=[];
                for(const mode of ['open','closed'])for(const tag of ['input','textarea']) {
                    const host=document.createElement('div');document.body.append(host);
                    const root=host.attachShadow({mode});
                    const nested=document.createElement('div');root.append(nested);
                    const inner=nested.attachShadow({mode});
                    const control=document.createElement(tag);inner.append(control);
                    control.value='text';control.focus();control.selectionStart=control.selectionEnd=4;
                    checks.push(document.activeElement===host&&root.activeElement===nested&&inner.activeElement===control);
                    await __oryn_testdriver_send_keys(document.body,'\uE003');
                    checks.push(control.value==='tex');
                    control.selectionStart=control.selectionEnd=0;
                    await __oryn_testdriver_send_keys(host,'\uE017');
                    checks.push(control.value==='ex');
                    const other=document.createElement('input');document.body.append(other);
                    await __oryn_testdriver_send_keys(other,'x');
                    checks.push(other.value==='x'&&control.value==='ex');
                    other.remove();host.remove();
                }
                globalThis.shadowKeyboardResult=checks.length===16&&checks.every(Boolean)?'ok':JSON.stringify(checks);
            })().catch(error=>{globalThis.shadowKeyboardResult=String(error)});
        "#).unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);shadowKeyboardResult")
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn keyboard_editing_enter_uses_selection_host_and_design_mode() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            const selection=document.getSelection();
            for(const [outer,inner,boundary,design,expected] of [
                ['true','plaintext-only',false,false,'insertParagraph'],
                ['plaintext-only','true',false,false,'insertLineBreak'],
                ['true','plaintext-only',true,false,'insertLineBreak'],
                ['plaintext-only','true',true,false,'insertParagraph'],
                ['plaintext-only','plaintext-only',false,true,'insertParagraph']
            ]) {
                const host=document.createElement('div'), nested=document.createElement('div');
                host.contentEditable=outer;nested.contentEditable=inner;
                if(boundary){const barrier=document.createElement('div');barrier.contentEditable='false';host.append(barrier);barrier.append(nested)}else host.append(nested);
                nested.innerHTML='<p>ab</p>';document.body.append(host);
                selection.collapse(nested.firstChild.firstChild,1);
                document.designMode=design?'on':'off';
                let observed='';
                host.addEventListener('beforeinput',event=>{observed=event.inputType;event.preventDefault()});
                __oryn_testdriver_keyboard(host,'Enter',true);
                __oryn_testdriver_keyboard(host,'Enter',false);
                if(observed!==expected)throw Error([outer,inner,boundary,design,observed,expected].join('|'));
                if(nested.innerHTML!=='<p>ab</p>')throw Error('cancelled beforeinput mutated content');
                document.designMode='off';selection.removeAllRanges();host.remove();
            }
            return 'ok';
        })()"#).unwrap(), "ok");
    }

    #[test]
    fn keyboard_editing_enter_uses_effective_editability() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const results=[];
            for(const [outer,inner,expected] of [
                ['plaintext-only',null,'insertLineBreak'],
                ['PLAINTEXT-ONLY','inherit','insertLineBreak'],
                ['plaintext-only','true','insertLineBreak'],
                ['true','plaintext-only','insertParagraph'],
                ['true',null,'insertParagraph']
            ]) {
                const host=document.createElement('div'),target=document.createElement('span');
                host.setAttribute('contenteditable',outer);
                if(inner!==null)target.setAttribute('contenteditable',inner);
                target.textContent='unchanged';host.append(target);document.body.append(host);
                let received=[];
                target.addEventListener('beforeinput',event=>{
                    received.push(event.inputType);event.preventDefault();
                });
                __oryn_testdriver_keyboard(target,'Enter',true);
                __oryn_testdriver_keyboard(target,'Enter',false);
                __oryn_testdriver_keyboard(target,'Shift',true);
                __oryn_testdriver_keyboard(target,'Enter',true);
                __oryn_testdriver_keyboard(target,'Enter',false);
                __oryn_testdriver_keyboard(target,'Shift',false);
                results.push(received.join('|')===expected+'|insertLineBreak');
                results.push(target.textContent==='unchanged');host.remove();
            }
            results.length===10&&results.every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn keyboard_editing_rechecks_native_geometry_after_handlers() {
        let mut page = RustExecutableDocument::load("<!doctype html><body></body>").unwrap();
        assert_eq!(page.evaluate(r#"
            const results=[];
            for(const tag of ['input','textarea'])for(const event of ['keydown','keypress','beforeinput']) {
                const parent=document.createElement('div'),control=document.createElement(tag);
                parent.append(control);document.body.append(parent);control.value='original';control.select();
                control.addEventListener(event,()=>parent.style.display='none',{once:true});
                __oryn_testdriver_keyboard(control,'a',true);
                __oryn_testdriver_keyboard(control,'a',false);
                results.push(control.value==='original');parent.remove();
            }
            const control=document.createElement('input');control.setAttribute('aria-hidden','true');
            document.body.append(control);control.value='original';control.select();
            __oryn_testdriver_keyboard(control,'a',true);__oryn_testdriver_keyboard(control,'a',false);
            results.push(control.value==='a');results.every(Boolean)
        "#).unwrap(),"true");
    }

    #[test]
    fn inline_cssom_converts_arguments_before_mutating() {
        let mut page =
            RustExecutableDocument::load("<!doctype html><body><div id='probe'></div>").unwrap();
        assert_eq!(page.evaluate(r#"
            const probe=document.getElementById('probe'), style=probe.style;
            style.width='10px'; const original=probe.getAttribute('style');
            const sentinel={};let caught=0;
            for(const action of [()=>style.width={toString(){throw sentinel}},
                ()=>style.cssText={toString(){throw sentinel}},
                ()=>style.setProperty('width','20px',{toString(){throw sentinel}})]) {
                try{action()}catch(e){if(e===sentinel&&probe.getAttribute('style')===original)caught++}
            }
            style.width=null;const removed=style.width==='';
            style.setProperty('width','20px',null);const priority=style.width==='20px';
            style.setProperty('width',null);const empty=style.width==='';
            [caught,removed,priority,empty].join('|')
        "#).unwrap(),"3|true|true|true");
    }

    #[test]
    fn computed_cssom_indices_follow_live_declarations() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html><body><div id="probe" style="--one: red"></div>"#,
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const probe=document.getElementById('probe'), computed=getComputedStyle(probe);
            const before=computed.length;
            const first=computed.item(before-1)==='--one'&&computed[before-1]==='--one';
            probe.style.setProperty('--two','blue');
            const added=computed.length===before+1&&computed.item(before)==='--two'&&computed[before]==='--two';
            probe.style.removeProperty('--one');
            const removed=computed.length===before&&computed.item(before-1)==='--two'&&computed[before]===undefined;
            const keys=Object.keys(computed).filter(key=>/^[0-9]+$/.test(key));
            [first,added,removed,keys.length===computed.length].join('|')
        "#).unwrap(), "true|true|true|true");
    }

    #[test]
    fn inline_cssom_uses_stylo_for_mutations_and_live_readback() {
        let mut page =
            RustExecutableDocument::load(r#"<!doctype html><body><div id="probe"></div>"#).unwrap();
        assert_eq!(page.evaluate(r#"
            const probe=document.getElementById('probe'), style=probe.style;
            const observer=new MutationObserver(()=>{});
            observer.observe(probe,{attributes:true,attributeOldValue:true});
            style.width='-100px'; style.removeProperty('width');
            const absent=observer.takeRecords().length===0&&!probe.hasAttribute('style');
            style.cssText='margin: 1px 2px !important; --Name: "a;b:c"; width: -1px';
            observer.takeRecords();
            const names=Array.from(style);
            const read=style.marginLeft==='2px'&&style.getPropertyPriority('margin')==='important'
                &&names.length===5&&names[0]==='margin-top'&&style[1]==='margin-right'
                &&style.item(4)==='--Name'&&style.item(99)===''&&style[99]===undefined;
            style.setProperty('--Name','three','important');
            const custom=style.getPropertyValue('--Name')==='three'&&style.getPropertyValue('--name')==='';
            const old=style.removeProperty('margin');
            const live=old==='1px 2px'&&style.length===1&&style[0]==='--Name'&&style[1]===undefined;
            observer.takeRecords();
            style.setProperty('--Name','three','important');
            const unchanged=observer.takeRecords().length===0;
            style.setProperty('--Name','','invalid');
            [absent,read,custom,live,unchanged,style.length===0].join('|')
        "#).unwrap(), "true|true|true|true|true|true");
    }

    #[test]
    fn global_event_native_phases_surround_dom_listeners() {
        let mut page = RustExecutableDocument::load(
            r#"<body><button></button><script>
          globalThis.order=[];const node=document.querySelector('button');
          addEventListener('native-probe',()=>order.push('capture'),true);
          node.addEventListener('native-probe',()=>order.push('node'));
          addEventListener('native-probe',()=>order.push('bubble'));
          node.dispatchEvent(new Event('native-probe',{bubbles:true}));
        </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "capture|node|bubble"
        );
    }

    #[test]
    fn global_event_native_error_handler_convention_depends_on_target() {
        let mut page=RustExecutableDocument::load(r#"<body><script>
          globalThis.order=[];
          onerror=function(){order.push(arguments.length);return true};
          order.push(dispatchEvent(new ErrorEvent('error',{cancelable:true})));
          document.onerror=function(){order.push(arguments.length);return false};
          order.push(document.dispatchEvent(new ErrorEvent('error',{cancelable:true})));
          document.onerror=null;onerror=function(){order.push(arguments.length);return false};
          order.push(document.dispatchEvent(new ErrorEvent('error',{bubbles:true,cancelable:true})));
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "5|false|1|false|1|false"
        );
    }

    #[test]
    fn mutation_observers_run_after_microtasks_and_reach_a_stable_checkpoint() {
        let mut page = RustExecutableDocument::load(
            r#"<div id=root></div><script>
            globalThis.order=[];
            const root=document.querySelector('#root');
            new MutationObserver(records=>{
              order.push('observer:'+records[0].type);
              if(!document.querySelector('#child')){
                const child=document.createElement('span');child.id='child';root.appendChild(child);
              }
            }).observe(root,{childList:true,subtree:true});
            Promise.resolve().then(()=>order.push('microtask'));
            root.appendChild(document.createElement('b'));
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "microtask,observer:childList,observer:childList"
        );
        assert_eq!(
            page.evaluate("String(!!document.querySelector('#child'))")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn parser_script_checkpoint_delivers_insertions_before_script_execution() {
        let mut page = RustExecutableDocument::load(
            r#"<body><script id=first>
            globalThis.order=[];
            const observer=new MutationObserver(records=>{
                order.push('observer:'+records.length);
                Promise.resolve().then(()=>order.push('promise'));
            });
            observer.observe(document,{childList:true,subtree:true});
            </script><p id=between></p><script id=second>
            order.push('second');
            const inserted=document.createElement('span');
            document.getElementById('between').appendChild(inserted);
            </script><script id=third>
            order.push('third');observer.disconnect();
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "observer:3|promise|second|observer:1|promise|observer:2|promise|third"
        );
    }

    #[test]
    fn parser_script_checkpoint_also_precedes_nonblocking_script_preparation() {
        for attributes in [
            "type=application/json",
            "type=module",
            "defer src=absent.js",
            "async src=absent.js",
        ] {
            let html = format!(
                r#"<body><script>
                globalThis.boundarySeen=false;
                const observer=new MutationObserver(records=>{{
                    if(records.some(record=>[...record.addedNodes].some(node=>node.id==='boundary'))) {{
                        boundarySeen=!document.getElementById('later');observer.disconnect();
                    }}
                }});
                observer.observe(document,{{childList:true,subtree:true}});
                </script><script id=boundary {attributes}></script><p id=later></p>"#
            );
            let mut page = RustExecutableDocument::load(&html).unwrap();
            assert_eq!(
                page.evaluate("boundarySeen").unwrap(),
                "true",
                "{attributes}"
            );
        }
    }

    #[test]
    fn observer_microtasks_and_connected_reactions_preserve_parser_script_order() {
        let mut page = RustExecutableDocument::load(
            r#"<body><script>
          globalThis.order=[];
          customElements.define('x-order',class extends HTMLElement {
            connectedCallback(){order.push('reaction');this.setAttribute('ready','')}
          });
          const node=document.createElement('x-order');
          new MutationObserver(records=>order.push('observer:'+records.length))
            .observe(document.body,{childList:true,attributes:true,subtree:true});
          Promise.resolve().then(()=>order.push('before'));
          document.body.appendChild(node);
          order.push('returned');
          Promise.resolve().then(()=>order.push('after'));
        </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join('|')").unwrap(),
            "reaction|returned|before|observer:2|after"
        );
    }

    #[test]
    fn custom_element_reactions_precede_mutation_observer_delivery() {
        let mut page = RustExecutableDocument::load(
            r#"<x-state></x-state><script>
            globalThis.order=[];
            const node=document.querySelector('x-state');
            new MutationObserver(()=>order.push('observer')).observe(node,{attributes:true});
            customElements.define('x-state',class extends HTMLElement{connectedCallback(){order.push('reaction');this.setAttribute('ready','')}});
            Promise.resolve().then(()=>order.push('microtask'));
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "reaction,observer,microtask"
        );
    }

    #[test]
    fn inner_html_upgrades_only_the_inserted_custom_element_subtree() {
        let mut page = RustExecutableDocument::load(
            r#"<main id="root"></main><script>
            customElements.define('x-state',class extends HTMLElement{
                connectedCallback(){this.setAttribute('ready','')}
            });
            document.querySelector('#root').innerHTML='<x-state></x-state>';
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("String(document.querySelector('x-state').hasAttribute('ready'))")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn source_less_dynamic_iframe_fires_load_with_same_origin_window() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const frame=document.createElement('iframe');
            frame.onload=()=>document.querySelector('#result').textContent=
              String(frame.contentDocument!==null)+'|'+typeof frame.contentWindow.IDBIndex;
            document.body.appendChild(frame);
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "true|function"
        );
    }

    #[test]
    #[ignore = "diagnostic only; requires ORYN_WPT_CHECKOUT"]
    fn pinned_range_insert_node_fixture_does_not_overflow_native_stack() {
        let root = std::path::PathBuf::from(std::env::var("ORYN_WPT_CHECKOUT").unwrap());
        let source =
            std::fs::read_to_string(root.join("dom/ranges/Range-insertNode.html")).unwrap();
        let mut resources = BTreeMap::new();
        for path in [
            "resources/testharness.js",
            "dom/common.js",
            "dom/ranges/Range-test-iframe.html",
        ] {
            resources.insert(
                format!("https://wpt.example/{path}"),
                std::fs::read_to_string(root.join(path)).unwrap(),
            );
        }
        resources.insert(
            "https://wpt.example/resources/testharnessreport.js".into(),
            include_str!("../../../scripts/wpt-report.js").into(),
        );
        for (alias, path) in [
            ("/resources/testharness.js", "resources/testharness.js"),
            ("../common.js", "dom/common.js"),
            (
                "Range-test-iframe.html",
                "dom/ranges/Range-test-iframe.html",
            ),
        ] {
            resources.insert(
                alias.into(),
                std::fs::read_to_string(root.join(path)).unwrap(),
            );
        }
        resources.insert(
            "/resources/testharnessreport.js".into(),
            include_str!("../../../scripts/wpt-report.js").into(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            &source,
            &resources,
            None,
            Some(Url::parse("https://wpt.example/dom/ranges/Range-insertNode.html").unwrap()),
        )
        .unwrap();
        eprintln!("diagnostics: {:?}", page.diagnostics);
        eprintln!(
            "completion: {:?}",
            page.evaluate("JSON.stringify(globalThis.__oryn_wpt_completion)")
                .map(|value| value.chars().take(500).collect::<String>())
        );
    }

    #[test]
    fn generated_click_default_action_navigates_a_named_iframe() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://frame.example.test/result".into(),
            "<script>parent.postMessage('navigated','*')</script>".into(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe name=result_frame></iframe><output id=result>pending</output>
            <script>
            addEventListener('message',event=>{
              if(event.source===document.querySelector('iframe').contentWindow)
                document.querySelector('#result').textContent=event.data;
            });
            const link=document.createElement('a');
            link.href='/result';
            link.target='result_frame';
            document.body.appendChild(link);
            link.click();
            </script>"#,
            &resources,
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "navigated"
        );
        assert_eq!(page.location().unwrap(), "https://frame.example.test/root");
    }

    #[test]
    fn nested_named_frame_navigation_reports_through_its_owner_realm() {
        let resources = BTreeMap::from([
            (
                "https://frame.example.test/source".into(),
                r#"<script>
                addEventListener('message',event=>{
                  if(event.data==='start'){
                    const target=document.createElement('iframe');
                    target.name='result_frame';
                    document.body.appendChild(target);
                    addEventListener('message',result=>{
                      if(result.source===target.contentWindow)
                        parent.postMessage({sourceContextUrl:location.toString(),value:result.data},'*');
                    });
                    const link=document.createElement('a');
                    link.href='/result';link.target='result_frame';
                    document.body.appendChild(link);link.click();
                  }
                });
                </script>"#
                    .into(),
            ),
            (
                "https://frame.example.test/result".into(),
                "<script>parent.postMessage('complete','*')</script>".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output>
            <script>
            const source=document.createElement('iframe');
            addEventListener('message',event=>{
              if(event.source===source.contentWindow)
                document.querySelector('#result').textContent=event.data.sourceContextUrl+'|'+event.data.value;
            });
            source.onload=()=>source.contentWindow.postMessage('start','*');
            source.src='https://frame.example.test/source';
            document.body.appendChild(source);
            </script>"#,
            &resources,
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        let actual = page
            .evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
            .unwrap();
        assert_eq!(actual, "https://frame.example.test/source|complete");
    }

    #[test]
    fn concurrent_nested_frame_messages_are_cloned_and_attributed_to_the_direct_source() {
        let resources = BTreeMap::from([
            (
                "https://source.example.test/source".into(),
                r#"<script>
                addEventListener('message',event=>{
                  if(!event.data || event.data.kind!=='start') return;
                  const target=document.createElement('iframe');
                  target.name=event.data.name;
                  document.body.appendChild(target);
                  addEventListener('message',result=>{
                    if(result.source===target.contentWindow)
                      parent.postMessage({sourceContextUrl:location.toString(),value:result.data,index:event.data.index},'*');
                  });
                  const link=document.createElement('a');
                  link.href='https://target.example.test/result?index='+event.data.index;link.target=event.data.name;
                  document.body.appendChild(link);link.click();
                });
                </script>"#
                    .into(),
            ),
            (
                "https://target.example.test/result?index=0".into(),
                "<script>const value={index:0,nested:{ok:true},referrer:document.referrer};parent.postMessage(value,'*');value.nested.ok=false</script>".into(),
            ),
            (
                "https://target.example.test/result?index=1".into(),
                "<script>const value={index:1,nested:{ok:true},referrer:document.referrer};parent.postMessage(value,'*');value.nested.ok=false</script>".into(),
            ),
        ]);
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result></output><script>
            const sources=[];const received=[];
            addEventListener('message',event=>{
              const sourceIndex=sources.findIndex(frame=>event.source===frame.contentWindow);
              received.push({sourceIndex,data:event.data});
              if(received.length===2)
                document.querySelector('#result').textContent=JSON.stringify(received.sort((a,b)=>a.data.index-b.data.index));
            });
            for(let index=0;index<2;index++){
              const source=document.createElement('iframe');sources.push(source);
              source.onload=()=>source.contentWindow.postMessage({kind:'start',index,name:'result-'+index},'*');
              source.src='https://source.example.test/source';document.body.appendChild(source);
            }
            </script>"#,
            &resources,
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        let actual = page
            .evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
            .unwrap();
        assert_eq!(
            actual,
            r#"[{"sourceIndex":0,"data":{"sourceContextUrl":"https://source.example.test/source","value":{"index":0,"nested":{"ok":true},"referrer":"https://source.example.test/source"},"index":0}},{"sourceIndex":1,"data":{"sourceContextUrl":"https://source.example.test/source","value":{"index":1,"nested":{"ok":true},"referrer":"https://source.example.test/source"},"index":1}}]"#
        );
    }

    #[test]
    fn parser_srcdoc_iframe_enters_the_frame_scheduler() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<p id=inside>ready</p>'></iframe>
            <output id=result>pending</output><script>
            const frame=document.querySelector('#frame');
            frame.addEventListener('load',()=>{
              const child=frame.contentDocument;
              const root=child.documentElement;
              document.querySelector('#result').textContent=
                child.querySelector('#inside').textContent+'|'+frame.contentWindow.origin+'|'+
                String(root.ownerDocument===child)+'|'+String(root.getRootNode()===child);
            });
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "ready|https://frame.example.test|true|true"
        );
    }

    #[test]
    fn custom_element_cloning_and_importing_upgrade_before_return() {
        let mut page = RustExecutableDocument::load(r#"<script>
            globalThis.events=[];
            class Copy extends HTMLElement {
                static observedAttributes=['title'];
                attributeChangedCallback(name,oldValue,newValue){events.push([name,oldValue,newValue])}
            }
            customElements.define('copy-element',Copy);
            const original=document.createElement('copy-element');original.title='copied';events=[];
            const clone=original.cloneNode(false);
            events.push(clone instanceof Copy);
            const imported=document.importNode(original,true);
            events.push(imported instanceof Copy);
        </script>"#).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["title",null,"copied"],true,["title",null,"copied"],true]"#
        );
    }

    #[test]
    fn custom_element_explicit_upgrade_uses_registry_for_inert_document() {
        let mut page = RustExecutableDocument::load(
            r#"<script>
            globalThis.events=[];
            class Explicit extends HTMLElement { constructor(){super();events.push('constructed')} }
            customElements.define('explicit-element',Explicit);
            const other=document.implementation.createHTMLDocument('inert');
            other.body.innerHTML='<explicit-element></explicit-element>';
            events.push(other.body.firstChild instanceof Explicit);
            customElements.upgrade(other.body);
            events.push(other.body.firstChild instanceof Explicit);
            events.push(other.body.firstChild.ownerDocument===other);
        </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[false,"constructed",true,true]"#
        );
    }

    #[test]
    fn custom_element_foreign_constructor_keeps_native_document_and_reactions() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<body><iframe id=frame srcdoc='<body></body>'></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const w=document.getElementById('frame').contentWindow;
            const events=[];
            class Foreign extends w.HTMLElement {
                connectedCallback(){events.push('connected')}
                disconnectedCallback(){events.push('disconnected')}
            }
            w.customElements.define('foreign-element',Foreign);
            const node=new Foreign();
            events.push(node.ownerDocument===w.document);
            w.document.body.appendChild(node);
            node.remove();
            class Local extends HTMLElement {
                adoptedCallback(){events.push('adopted')}
                connectedCallback(){events.push('local-connected')}
            }
            customElements.define('local-element',Local);
            w.customElements.define('local-element',class extends w.HTMLElement {
                constructor(){super();events.push('foreign-definition')}
            });
            const local=new Local();
            w.document.body.appendChild(local);
            JSON.stringify(events)
        "#
            )
            .unwrap(),
            r#"[true,"connected","disconnected","adopted","local-connected"]"#
        );
    }

    #[test]
    fn initial_frame_loads_skip_parser_removed_siblings() {
        let resources = BTreeMap::from([(
            "https://frame.example.test/blank".into(),
            "<body>child</body>".into(),
        )]);
        for (remove, expected) in [
            ("second.remove()", r#"[["first"],true,1]"#),
            ("first.remove();second.remove()", r#"[[],true,0]"#),
            (
                "first.addEventListener('load',()=>second.remove())",
                r#"[["first"],true,1]"#,
            ),
        ] {
            let source = r#"<iframe id=first src='https://frame.example.test/blank'></iframe><iframe id=second src='https://frame.example.test/blank'></iframe>
            <script>
            globalThis.loads=[];
            first.addEventListener('load',()=>loads.push('first'));
            second.addEventListener('load',()=>loads.push('second'));
            globalThis.removedWindow=second.contentWindow;
            REMOVAL;
            </script>"#.replace("REMOVAL", remove);
            let mut page = RustExecutableDocument::load_with_environment(
                &source,
                &resources,
                None,
                Some(Url::parse("https://frame.example.test/root").unwrap()),
            )
            .unwrap();
            assert_eq!(page.evaluate("JSON.stringify([loads,removedWindow.closed,document.querySelectorAll('iframe').length])").unwrap(),
            expected, "{remove}");
        }
    }

    #[test]
    fn same_origin_frame_eval_uses_child_realm() {
        for source in [
            "<iframe id=frame srcdoc='<p>child</p>'></iframe>",
            "<iframe id=frame src='https://frame.example.test/child'></iframe><iframe id=removed src='https://frame.example.test/child'></iframe><script>removed.remove()</script>",
        ] {
            let resources = BTreeMap::from([(
                "https://frame.example.test/child".into(),
                "<p>child</p><script>window.onload=function(){}</script>".into(),
            )]);
            let mut page = RustExecutableDocument::load_with_environment(
                source,
                &resources,
                None,
                Some(Url::parse("https://frame.example.test/root").unwrap()),
            )
            .unwrap();
            assert_eq!(
                page.evaluate(
                    r#"(()=>{
            const child=document.getElementById('frame').contentWindow;
            const evaluate=child.eval;
            evaluate('globalThis.childRealmMarker=17');
            return JSON.stringify([evaluate('document.querySelector("p").textContent'),
                child.childRealmMarker,typeof globalThis.childRealmMarker,
                evaluate('window===self && self===frames')]);
        })()"#
                )
                .unwrap(),
                r#"["child",17,"undefined",true]"#
            );
        }
    }

    #[test]
    fn same_origin_frame_adoption_preserves_cross_realm_identity_and_owner_document() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<section id=source><span data-kept="yes">text</span></section>'></iframe>"#,
            &BTreeMap::new(), None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            const frame=document.getElementById('frame'), childDocument=frame.contentDocument;
            const node=childDocument.getElementById('source'), descendant=node.firstChild;
            const attribute=descendant.getAttributeNode('data-kept');
            node.marker=73;
            const adopted=document.adoptNode(node);
            const first=[adopted===node,node.marker===73,node.ownerDocument===document,
                         descendant.ownerDocument===document,attribute.ownerDocument===document,
                         childDocument.getElementById('source')===null];
            document.body.append(node);
            first.push(node.parentNode===document.body);
            const returned=childDocument.adoptNode(node);
            first.concat([returned===node,node.ownerDocument===childDocument,
                          descendant.ownerDocument===childDocument,node.parentNode===null]).join('|')
        "#).unwrap(), "true|true|true|true|true|true|true|true|true|true|true");
    }

    #[test]
    fn frame_document_writes_preserve_parent_siblings_and_execution_realm() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<main id=parent>parent</main><iframe id=a srcdoc='<p>old a</p>'></iframe><iframe id=b srcdoc='<p>old b</p>'></iframe>"#,
            &BTreeMap::new(), None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            const parent=document.getElementById('parent');
            const a=document.getElementById('a'), b=document.getElementById('b');
            const ad=a.contentDocument, bd=b.contentDocument;
            ad.open();bd.open();
            ad.write('<p id="written">a</p><script>globalThis.writeMarker="a"</scr'+'ipt>');
            bd.write('<p id="written">b</p><script>globalThis.writeMarker="b"</scr'+'ipt>');
            bd.close();ad.close();
            [document.getElementById('parent')===parent, parent.textContent==='parent',
             a.contentDocument===ad,b.contentDocument===bd,
             ad.getElementById('written').textContent==='a',bd.getElementById('written').textContent==='b',
             a.contentWindow.writeMarker==='a',b.contentWindow.writeMarker==='b',
             typeof globalThis.writeMarker==='undefined',document.querySelector('#written')===null].join('|')
        "#).unwrap(), "true|true|true|true|true|true|true|true|true|true");
    }

    #[test]
    fn hosted_frame_focus_is_document_local_and_retargets_to_the_parent_frame() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<input id=shared><iframe id=frame srcdoc='<input id=shared>'></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const frame=document.getElementById('frame');
            const parentInput=document.getElementById('shared');
            const child=frame.contentDocument;
            const childInput=child.getElementById('shared');
            childInput.focus();
            const focused=document.activeElement===frame && child.activeElement===childInput;
            parentInput.focus();
            const restored=document.activeElement===parentInput && child.activeElement===child.body;
            childInput.focus();
            frame.remove();
            [focused,restored,document.activeElement===document.body,
             child.activeElement===null,document.getElementById('shared')===parentInput].join('|')
        "#
            )
            .unwrap(),
            "true|true|true|true|true"
        );
    }

    #[test]
    fn frame_clones_do_not_capture_the_live_child_document() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<p id=inside>initial</p>'></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            const frame=document.getElementById('frame');
            const child=frame.contentDocument;
            child.getElementById('inside').textContent='live-only';
            const copy=frame.cloneNode(true);
            const imported=document.importNode(frame,true);
            [copy.childNodes.length===0,imported.childNodes.length===0,
             frame.childNodes.length===0,child.parentNode===null,
             document.querySelector('#inside')===null,
             child.documentElement.getRootNode()===child,
             copy.contentDocument===null,imported.contentDocument===null,
             copy.ownerDocument===document,imported.ownerDocument===document,
             frame.contentDocument===child,child.getElementById('inside').textContent==='live-only'].every(Boolean)
        "#).unwrap(), "true");
    }

    #[test]
    fn native_frame_discard_covers_dom_removal_paths_and_retires_timers() {
        for removal in [
            "frame.remove()",
            "frame.parentNode.removeChild(frame)",
            "document.getElementById('container').remove()",
            "document.getElementById('container').innerHTML=''",
            "document.getElementById('container').replaceChildren()",
        ] {
            let mut page = RustExecutableDocument::load_with_environment(
                "<div id=container><iframe id=frame srcdoc='<p>kept</p>'></iframe></div><iframe id=sibling srcdoc='<p>live</p>'></iframe>",
                &BTreeMap::new(), None,
                Some(Url::parse("https://frame.example.test/root").unwrap()),
            ).unwrap();
            let source = format!(
                r#"
                const frame=document.getElementById('frame'), old=frame.contentWindow;
                const saved=frame.contentDocument;
                globalThis.fired=0;
                old.setTimeout(()=>fired++,0);
                {removal};
                const retired=old.setTimeout(()=>fired++,0)===0;
                [old.closed,frame.contentWindow===null,frame.contentDocument===null,
                 saved.body.textContent==='kept',retired,
                 document.getElementById('sibling').contentWindow.closed===false].join('|')
            "#
            );
            assert_eq!(
                page.evaluate(&source).unwrap(),
                "true|true|true|true|true|true",
                "{removal}"
            );
            assert_eq!(page.evaluate("String(fired)").unwrap(), "0", "{removal}");
            assert_eq!(
                page.frame_realms.len(),
                1,
                "discarded realm must be released"
            );
        }
    }

    #[test]
    fn borrowed_meta_setter_updates_only_owner_document_policy() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<meta name=referrer content=no-referrer><iframe id=frame srcdoc='<meta name=referrer content=same-origin><p>child</p>'></iframe>",
            &BTreeMap::new(), None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"
            document.querySelector('meta').content='no-referrer';
            const child=document.getElementById('frame').contentWindow;
            const meta=child.document.querySelector('meta');
            Object.getOwnPropertyDescriptor(HTMLMetaElement.prototype,'content').set.call(meta,'unsafe-url');
            JSON.stringify([__oryn_referrer_policy,child.__oryn_referrer_policy])
        "#).unwrap(), r#"["no-referrer","unsafe-url"]"#);
    }

    #[test]
    fn borrowed_meta_refresh_schedules_owner_document_navigation() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=frame src='https://frame.example.test/child/page'></iframe>",
            &BTreeMap::from([(
                "https://frame.example.test/child/page".into(),
                "<meta http-equiv=refresh><p>child</p>".into(),
            )]),
            None,
            Some(Url::parse("https://frame.example.test/parent/page").unwrap()),
        )
        .unwrap();
        let child = *page.frame_realms.keys().next().unwrap();
        page.agent
            .host
            .execute(
                r#"
            const child=document.getElementById('frame').contentWindow;
            Object.getOwnPropertyDescriptor(HTMLMetaElement.prototype,'content').set.call(
                child.document.querySelector('meta'),'0; url=next');
        "#,
            )
            .unwrap();
        page.agent.host.drain_scheduled_timers(false).unwrap();
        assert!(page.agent.host.take_location_navigation().is_none());
        page.agent.host.activate_realm(child).unwrap();
        let navigation = page.agent.host.take_location_navigation().unwrap();
        assert_eq!(navigation.url, "https://frame.example.test/child/next");
    }

    #[test]
    fn inserted_scripts_execute_in_native_owner_document_realm() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=frame srcdoc='<p>child</p>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const child=document.getElementById('frame').contentWindow;
            const foreign=document.createElement('script');
            foreign.textContent='globalThis.childScriptRan=true';
            Node.prototype.appendChild.call(child.document.body,foreign);
            const local=child.document.createElement('script');
            local.textContent='globalThis.parentScriptRan=true';
            child.Node.prototype.appendChild.call(document.body,local);
            child.childScriptRan===true && globalThis.childScriptRan===undefined &&
            globalThis.parentScriptRan===true && child.parentScriptRan===undefined
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn inserted_scripts_do_not_execute_in_inert_or_retired_documents() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=frame srcdoc='<p>child</p>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const frame=document.getElementById('frame');
            const retired=frame.contentDocument;
            const old=frame.contentWindow;
            frame.remove();
            const inert=document.implementation.createHTMLDocument('inert');
            globalThis.executed=0;
            for (const doc of [retired,inert]) {
                const script=document.createElement('script');
                script.textContent='globalThis.executed=99';
                Node.prototype.appendChild.call(doc.body,script);
            }
            executed===0 && old.executed===undefined
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn native_frame_discard_precedes_post_connection_script_execution() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<div id=slot><iframe id=frame srcdoc='<p>old</p>'></iframe></div>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate(r#"
            globalThis.old=document.getElementById('frame').contentWindow;
            globalThis.closedDuringScript=false;
            globalThis.timerDuringScript=-1;
            const script=document.createElement('script');
            script.textContent='closedDuringScript=old.closed;timerDuringScript=old.setTimeout(()=>{},0)';
            document.getElementById('slot').replaceChildren(script);
            closedDuringScript && timerDuringScript===0
        "#).unwrap(), "true");
        assert!(page.frame_realms.is_empty());
    }

    #[test]
    fn native_frame_discard_distinguishes_reparenting_from_state_preserving_moves() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<div id=one><iframe id=frame srcdoc='<p>kept</p>'></iframe></div><div id=two></div>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            globalThis.frame=document.getElementById('frame');globalThis.old=frame.contentWindow;
            old.close();
            document.getElementById('two').moveBefore(frame,null);
            [old.closed===false,frame.contentWindow===old].every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
        assert_eq!(
            page.evaluate(
                r#"
            document.getElementById('one').appendChild(frame);
            String(old.closed)
        "#
            )
            .unwrap(),
            "true"
        );
        assert_eq!(page.evaluate("[frame.contentWindow!==old,frame.contentWindow.closed===false,old.closed].every(Boolean)").unwrap(), "true");
        assert_eq!(page.frame_realms.len(), 1);
    }

    #[test]
    fn native_frame_discard_marks_retained_cross_origin_window_closed() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<div id=container><iframe id=frame src='https://foreign.example.test/child'></iframe></div>",
            &BTreeMap::from([("https://foreign.example.test/child".into(), "<p>foreign</p>".into())]),
            None, Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const frame=document.getElementById('frame'), old=frame.contentWindow;
            const before=old.closed===false;
            document.getElementById('container').remove();
            [before,old.closed,frame.contentWindow===null].every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
        assert!(page.frame_realms.is_empty());
    }

    #[test]
    fn native_frame_discard_inherits_nested_srcdoc_origin_but_respects_sandbox() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=outer srcdoc='<div id=slot></div>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        page.execute(
            r#"
            globalThis.outer=document.getElementById('outer');
            outer.contentDocument.getElementById('slot').innerHTML=
                '<iframe id=nested srcdoc="<p>inherited</p>"></iframe>'+
                '<iframe id=opaque sandbox="allow-scripts" srcdoc="<p>opaque</p>"></iframe>';
        "#,
        )
        .unwrap();
        assert_eq!(page.frame_realms.len(), 3, "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate(
                r#"
            const doc=outer.contentDocument;
            const nested=doc.getElementById('nested');
            const opaque=doc.getElementById('opaque');
            const retained=nested.contentWindow;
            const foreign=opaque.contentWindow;
            const accessible=nested.contentDocument.body.textContent==='inherited';
            const isolated=opaque.contentDocument===null;
            outer.remove();
            [accessible,isolated,retained.closed,foreign.closed].every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
        assert!(page.frame_realms.is_empty());
    }

    #[test]
    fn native_frame_element_identity_is_readonly_and_cleared_on_discard() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=frame srcdoc='<p>child</p>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            globalThis.frame=document.getElementById('frame');
            globalThis.child=frame.contentWindow.window;
            globalThis.descriptor=Object.getOwnPropertyDescriptor(child,'frameElement');
            [frameElement===null, child.frameElement===frame,
             descriptor.enumerable, descriptor.configurable,
             descriptor.set===undefined, typeof descriptor.get,
             Reflect.set(child,'frameElement',null), child.frameElement===frame].join('|')
        "#
            )
            .unwrap(),
            "true|true|true|true|true|function|false|true"
        );
        page.execute("frame.remove()").unwrap();
        assert_eq!(
            page.evaluate("String(descriptor.get.call(child)===null)")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn native_frame_link_uses_native_ancestry_without_author_top_getters() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=outer srcdoc='<div id=slot></div>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        page.execute(r#"
            globalThis.outer=document.getElementById('outer');
            globalThis.topReads=0;
            const replaced = Reflect.defineProperty(outer.contentWindow.window,'top',{
                configurable:true,get(){topReads++;throw Error('author top getter');}
            });
            if (replaced) throw Error('top must be unforgeable');
            const descriptor = Object.getOwnPropertyDescriptor(outer.contentWindow.window, 'top');
            if (descriptor.configurable || descriptor.set !== undefined || typeof descriptor.get !== 'function') throw Error('invalid top descriptor');
            outer.contentDocument.getElementById('slot').innerHTML='<iframe id=nested srcdoc="<p>nested</p>"></iframe>';
        "#).unwrap();
        assert_eq!(page.frame_realms.len(), 2, "{:?}", page.diagnostics);
        assert_eq!(
            page.evaluate(
                r#"
            const nested=outer.contentDocument.getElementById('nested').contentWindow;
            topReads===0 && nested.top===globalThis
        "#
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn native_frame_discard_retires_nested_foreign_contexts() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=outer srcdoc='<div id=slot></div>'></iframe>",
            &BTreeMap::from([(
                "https://foreign.example.test/child".into(),
                "<p>foreign</p>".into(),
            )]),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        page.execute(r#"
            globalThis.outer=document.getElementById('outer');
            outer.contentDocument.getElementById('slot').innerHTML='<iframe id=nested src="https://foreign.example.test/child"></iframe>';
        "#).unwrap();
        assert_eq!(
            page.frame_realms.len(),
            2,
            "frames={:?}; diagnostics={:?}; state={:?}",
            page.frame_realms,
            page.diagnostics,
            page.agent.host.realm_ids()
        );
        assert_eq!(
            page.evaluate(
                r#"
            const old=outer.contentWindow;
            const nested=outer.contentDocument.getElementById('nested').contentWindow;
            const before=!old.closed&&!nested.closed;
            outer.remove();
            [before,old.closed,nested.closed].every(Boolean)
        "#
            )
            .unwrap(),
            "true"
        );
        assert!(page.frame_realms.is_empty());
    }

    #[test]
    #[ignore = "requires the pinned upstream WPT checkout in ORYN_WPT_ROOT"]
    fn srcdoc_child_executes_pinned_testdriver_before_load() {
        let root =
            std::path::PathBuf::from(std::env::var("ORYN_WPT_ROOT").expect("pinned WPT root"));
        let source = std::fs::read_to_string(root.join("resources/testdriver.js")).unwrap();
        let resources = BTreeMap::from([(
            "https://frame.example.test/resources/testdriver.js".into(),
            source,
        )]);
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<script src="/resources/testdriver.js"></script>'></iframe>"#,
            &resources, None, Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(
            page.evaluate(
                "typeof document.querySelector('#frame').contentWindow.test_driver?.send_keys"
            )
            .unwrap(),
            "function",
            "{:?}",
            page.diagnostics
        );
        page.evaluate(r#"document.querySelector('#frame').srcdoc = '<head><script src="/resources/testdriver.js"></script></head><body>new</body>'"#).unwrap();
        assert_eq!(
            page.evaluate(
                "typeof document.querySelector('#frame').contentWindow.test_driver?.send_keys"
            )
            .unwrap(),
            "function",
            "{:?}",
            page.diagnostics
        );
    }

    #[test]
    fn srcdoc_inherits_parent_document_base_and_resolves_child_base_attributes() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<base href="https://assets.example.test/parent/"><iframe srcdoc='<a href="item">link</a>'></iframe>"#,
            &BTreeMap::new(), None, Some(Url::parse("https://frame.example.test/root").unwrap()),
        ).unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            globalThis.child = document.querySelector('iframe').contentDocument;
            [child.URL, child.baseURI, child.querySelector('a').href].join('|')
        "#
            )
            .unwrap(),
            "about:srcdoc|https://assets.example.test/parent/|https://assets.example.test/parent/item"
        );
        assert_eq!(page.evaluate(r#"
            const base = child.createElement('base'); base.setAttribute('href', '../child/'); child.head.appendChild(base);
            [child.baseURI, child.querySelector('a').href].join('|')
        "#).unwrap(), "https://assets.example.test/child/|https://assets.example.test/child/item");
        assert_eq!(
            page.evaluate(
                r#"
            child.querySelector('base').remove();
            document.querySelector('base').setAttribute('href', 'https://different.example/');
            child.querySelector('a').setAttribute('href', 'http://[');
            [child.baseURI, child.querySelector('a').href].join('|')
        "#
            )
            .unwrap(),
            "https://assets.example.test/parent/|http://["
        );
    }

    #[test]
    fn range_endpoint_reads_follow_mutations_without_changing_static_snapshots() {
        let mut page = RustExecutableDocument::load("<main>abcdef</main>").unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            const text = document.querySelector('main').firstChild;
            const live = document.createRange();
            live.setStart(text, 2); live.setEnd(text, 5);
            const snapshot = new StaticRange({startContainer:text, startOffset:2,
                endContainer:text, endOffset:5});
            text.insertData(0, 'xy');
            const first = [live.startOffset, live.endOffset,
                snapshot.startOffset, snapshot.endOffset];
            text.deleteData(3, 2);
            first.concat([live.startContainer === text, live.startOffset,
                live.endOffset, live.collapsed, snapshot.startOffset,
                snapshot.endOffset]).join('|')
        "#
            )
            .unwrap(),
            "4|7|2|5|true|3|5|false|2|5"
        );
    }

    #[test]
    fn selection_accepts_connected_shadow_ranges_and_rejects_detached_hosts() {
        let mut page = RustExecutableDocument::load("<main></main>").unwrap();
        assert_eq!(page.evaluate(r#"
            const host = document.querySelector('main');
            const root = host.attachShadow({mode:'closed'});
            const editor = document.createElement('span');
            editor.contentEditable = 'true'; editor.textContent = 'abc'; root.appendChild(editor);
            const range = document.createRange(); range.setStart(editor.firstChild, 1); range.setEnd(editor.firstChild, 2);
            const selection = document.getSelection(); selection.removeAllRanges(); selection.addRange(range);
            const connected = selection.getRangeAt(0) === range && selection.toString() === 'b';
            selection.removeAllRanges(); host.remove(); selection.addRange(range);
            [connected, selection.rangeCount].join('|')
        "#).unwrap(), "true|0");
    }

    #[test]
    fn frame_selection_retains_foreign_range_identity_and_live_boundaries() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<main contenteditable>abcdef</main>'></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"
            globalThis.child = document.querySelector('#frame').contentDocument;
            globalThis.text = child.querySelector('main').firstChild;
            globalThis.foreignRange = document.createRange();
            foreignRange.setStart(text, 1);
            foreignRange.setEnd(text, 4);
            globalThis.childSelection = child.getSelection();
            childSelection.removeAllRanges();
            childSelection.addRange(foreignRange);
            [childSelection.getRangeAt(0) === foreignRange,
             childSelection.toString(), childSelection.anchorOffset,
             childSelection.focusOffset].join('|')
        "#
            )
            .unwrap(),
            "true|bcd|1|4"
        );
        assert_eq!(
            page.evaluate(
                r#"
            foreignRange.setEnd(text, 5);
            text.insertData(0, 'X');
            [childSelection.getRangeAt(0) === foreignRange,
             foreignRange.startOffset, foreignRange.endOffset,
             childSelection.anchorOffset, childSelection.focusOffset,
             childSelection.toString()].join('|')
        "#
            )
            .unwrap(),
            "true|2|6|2|6|bcde"
        );
        assert_eq!(
            page.evaluate(
                r#"
            childSelection.setBaseAndExtent(text, 6, text, 2);
            const backward = childSelection.getRangeAt(0);
            backward.setEnd(text, 7);
            [childSelection.anchorOffset, childSelection.focusOffset,
             childSelection.toString()].join('|')
        "#
            )
            .unwrap(),
            "7|2|bcdef"
        );
        assert_eq!(
            page.evaluate(
                r#"
            childSelection.removeAllRanges();
            foreignRange.setEnd(text, 6);
            [childSelection.rangeCount, foreignRange.toString()].join('|')
        "#
            )
            .unwrap(),
            "0|bcde"
        );
    }

    #[test]
    fn frame_content_document_uses_native_committed_window() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<script>
              globalThis.documentProxyReads = 0;
              const factory = globalThis.__oryn_make_window_proxy;
              globalThis.__oryn_make_window_proxy = (...args) => new Proxy(factory(...args), {
                get(target, key, receiver) {
                  if (key === 'document') {
                    documentProxyReads++;
                    return {body: {textContent: 'forged'}};
                  }
                  return Reflect.get(target, key, receiver);
                }
              });
            </script><iframe id=frame srcdoc='<p>native child</p>'></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(page.evaluate("document.getElementById('frame').contentDocument.body.textContent + '|' + documentProxyReads").unwrap(), "native child|0");
    }

    #[test]
    fn frame_navigation_updates_content_document_origin_visibility() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<p>initial</p>'></iframe>"#,
            &BTreeMap::from([
                ("https://foreign.test/child".into(), "<p>foreign</p>".into()),
                ("/return".into(), "<p>returned</p>".into()),
            ]),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        page.evaluate("globalThis.frame=document.querySelector('#frame');globalThis.savedWindow=frame.contentWindow;globalThis.savedDocument=frame.contentDocument;frame.removeAttribute('srcdoc');frame.src='https://foreign.test/child'").unwrap();
        assert_eq!(page.evaluate("String(frame.contentDocument===null)+'|'+String(frame.contentWindow===savedWindow)+'|'+savedDocument.querySelector('p').textContent").unwrap(), "true|true|initial");
        page.evaluate("frame.src='/return'").unwrap();
        assert_eq!(page.evaluate("String(frame.contentDocument!==savedDocument)+'|'+String(frame.contentWindow===savedWindow)+'|'+frame.contentDocument.querySelector('p').textContent").unwrap(), "true|true|returned");
        page.evaluate("frame.setAttribute('sandbox','allow-scripts');frame.srcdoc='<p>opaque</p>'")
            .unwrap();
        assert_eq!(
            page.evaluate(
                "String(frame.contentDocument===null)+'|'+String(frame.contentWindow===savedWindow)"
            )
            .unwrap(),
            "true|true"
        );
    }

    #[test]
    fn uncommitted_frame_navigation_preserves_the_active_document() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame srcdoc='<p>old</p>'></iframe>"#,
            &BTreeMap::from([("/replacement".into(), "<p>new</p>".into())]),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        let old_realm = *page.frame_realms.keys().next().unwrap();
        page.evaluate("globalThis.frame=document.querySelector('#frame');globalThis.oldDocument=frame.contentDocument;globalThis.oldWindow=frame.contentWindow;frame.removeAttribute('srcdoc');frame.src='/unavailable'").unwrap();
        assert!(page.frame_realms.contains_key(&old_realm));
        assert_eq!(page.evaluate("String(frame.contentDocument===oldDocument)+'|'+String(frame.contentWindow===oldWindow)+'|'+oldDocument.querySelector('p').textContent").unwrap(), "true|true|old");
        page.agent.host.activate_realm(old_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("document.querySelector('p').textContent")
                .unwrap(),
            "old"
        );
        page.agent.host.activate_realm(RealmId(0)).unwrap();
        page.evaluate("frame.src='/replacement'").unwrap();
        assert!(!page.frame_realms.contains_key(&old_realm));
        assert_eq!(page.evaluate("String(frame.contentDocument!==oldDocument)+'|'+String(frame.contentWindow===oldWindow)+'|'+frame.contentDocument.querySelector('p').textContent").unwrap(), "true|true|new");
    }

    #[test]
    fn repeated_srcdoc_navigation_retargets_the_stable_window_proxy() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const frame=document.createElement('iframe');
            let retained=null;
            let loads=0;
            frame.onload=()=>{
              loads++;
              if(loads===1){
                retained=frame.contentWindow;
                frame.srcdoc='<script>globalThis.marker="second"<\/script><p>new</p>';
              }else{
                document.querySelector('#result').textContent=
                  String(frame.contentWindow===retained)+'|'+retained.marker+'|'+frame.contentDocument.querySelector('p').textContent;
              }
            };
            frame.srcdoc='<script>globalThis.marker="first"<\/script><p>old</p>';
            document.body.appendChild(frame);
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "true|second|new"
        );
    }

    #[test]
    fn parser_iframe_exposes_stable_window_proxy_before_child_commit() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://child.example.test/frame".into(),
            r#"<script>onmessage=event=>parent.postMessage('reply:'+event.data,'*')</script>"#
                .into(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<iframe id=frame src="https://child.example.test/frame"></iframe><output id=result>pending</output><script>
            const frame=document.querySelector('#frame');
            const retained=frame.contentWindow;
            addEventListener('message',event=>{
              document.querySelector('#result').textContent=
                String(event.source===retained)+'|'+String(frame.contentWindow===retained)+'|'+retained.origin+'|'+event.data;
            });
            retained.postMessage('early','*');
            </script>"#,
            &resources,
            None,
            Some(Url::parse("https://parent.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .unwrap(),
            "true|true|https://child.example.test|reply:early"
        );
    }

    #[test]
    fn srcdoc_window_proxies_preserve_message_source_for_same_and_opaque_origins() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const child='<script>onmessage=()=>{if(location.origin!=="null")throw Error("srcdoc URL origin");parent.postMessage(origin,"*")}<\/script>';
            const load=sandbox=>new Promise(resolve=>{
              const frame=document.createElement('iframe');
              if(sandbox) frame.sandbox='allow-scripts';
              frame.srcdoc=child;
              frame.onload=()=>{
                const listener=event=>{
                  if(event.source===frame.contentWindow){
                    removeEventListener('message',listener);
                    resolve(event.data);
                  }
                };
                addEventListener('message',listener);
                frame.contentWindow.postMessage({},'*');
              };
              document.documentElement.appendChild(frame);
            });
            Promise.all([load(false),load(true)]).then(values=>{
              document.querySelector('#result').textContent=values.join('|');
            });
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "https://frame.example.test|null"
        );
    }

    #[test]
    fn automation_trace_retains_cross_origin_realm_identity_without_exposing_reader() {
        let mut resources = BTreeMap::new();
        resources.insert("https://remote.example.test/child.html".into(),
            "<script>__oryn_record_automation_event(1,'click','started');__oryn_record_automation_event(1,'click','completed')</script>".into());
        let mut page = RustExecutableDocument::load_with_environment(
            "<script>__oryn_record_automation_event(1,'click','started');__oryn_record_automation_event(1,'click','completed')</script><iframe src='https://remote.example.test/child.html'></iframe>",
            &resources,None,Some(Url::parse("https://parent.example.test/").unwrap())).unwrap();
        assert_eq!(
            page.evaluate("typeof globalThis.__oryn_native_automation_trace")
                .unwrap(),
            "undefined"
        );
        let snapshot: serde_json::Value =
            serde_json::from_str(&page.evaluate("__oryn_native_automation_trace").unwrap())
                .unwrap();
        assert_eq!(snapshot["complete"], true);
        let events = snapshot["events"].as_array().unwrap();
        assert_eq!(events.len(), 4);
        assert_ne!(events[0]["context_id"], events[2]["context_id"]);
        assert_eq!(events[0]["id"], events[2]["id"]);
        page.execute("document.querySelector('iframe').remove()")
            .unwrap();
        let retained: serde_json::Value =
            serde_json::from_str(&page.evaluate("__oryn_native_automation_trace").unwrap())
                .unwrap();
        assert_eq!(retained, snapshot);
        let mut fresh = RustExecutableDocument::load("<body>").unwrap();
        let empty: serde_json::Value =
            serde_json::from_str(&fresh.evaluate("__oryn_native_automation_trace").unwrap())
                .unwrap();
        assert!(empty["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn frame_native_actions_preserve_realm_and_message_completion() {
        for origin in ["https://parent.example.test", "https://remote.example.test"] {
            let child = format!("{origin}/child.html");
            let mut resources = BTreeMap::new();
            resources.insert(
                child.clone(),
                r#"<!doctype html><input><script>
              addEventListener('load',async()=>{
                try {
                  const input=document.querySelector('input');
                  await __oryn_testdriver_actions([{type:'pointer',id:'mouse',actions:[
                    {type:'pointerMove',origin:input,x:0,y:0},
                    {type:'pointerDown',button:0},{type:'pointerUp',button:0}]}]);
                  await __oryn_testdriver_actions([{type:'key',id:'keyboard',actions:
                    [...'PASS'].flatMap(value=>[{type:'keyDown',value},{type:'keyUp',value}])}]);
                  parent.postMessage(input.value,'*');
                } catch(error) {parent.postMessage(String(error),'*');}
              });
            </script>"#
                    .into(),
            );
            let mut page = RustExecutableDocument::load_with_environment(
                &format!("<iframe src='{child}'></iframe><script>globalThis.result='waiting';addEventListener('message',event=>result=event.data)</script>"),
                &resources, None, Some(Url::parse("https://parent.example.test/").unwrap()),
            ).unwrap();
            assert_eq!(
                page.evaluate("__oryn_drain(true);result").unwrap(),
                "PASS",
                "{origin}"
            );
        }
    }

    #[test]
    fn cross_origin_frame_receives_global_load_and_can_complete_remote_protocol() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "https://remote.example.test/child.html".into(),
            "<!doctype html><script>addEventListener('load',()=>parent.postMessage({type:'complete',status:0},'*'))</script>".into(),
        );
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><iframe src='https://remote.example.test/child.html'></iframe><script>globalThis.remoteStatus='waiting';addEventListener('message',event=>{if(event.data&&event.data.type==='complete')remoteStatus='complete:'+event.data.status})</script>",
            &resources,
            None,
            Url::parse("https://parent.example.test/").ok(),
        )
        .expect("load cross-origin frame fixture");
        assert_eq!(
            page.evaluate("remoteStatus").expect("read remote status"),
            "complete:0"
        );
    }

    #[test]
    fn data_url_workers_report_opaque_indexed_db_security_errors() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const source=`
              const handler=reply=>{let result='none';try{indexedDB.open('blocked')}catch(error){result=error.name}reply({result})};
              addEventListener('message',()=>handler(postMessage));
              addEventListener('connect',event=>handler(value=>event.ports[0].postMessage(value)));
            `;
            const url='data:,'+encodeURIComponent(source);
            const output=document.querySelector('#result');
            output.textContent='script';
            const dedicated=new Worker(url);
            output.textContent=`dedicated:${dedicated.__closed}`;
            const shared=new SharedWorker(url,'opaque');
            output.textContent=`created:${dedicated.__closed}|${shared.__worker.__closed}`;
            const values=[];
            const done=value=>{values.push(value.result);if(values.length===2)output.textContent=values.join('|')};
            const failed=event=>output.textContent='error:'+event.message;
            dedicated.onerror=failed;
            shared.onerror=failed;
            dedicated.onmessage=event=>done(event.data);
            shared.port.onmessage=event=>done(event.data);
            dedicated.postMessage({});
            shared.port.postMessage({});
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://frame.example.test/root").unwrap()),
        )
        .unwrap();
        let value = page
            .evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
            .unwrap();
        assert_eq!(
            value, "SecurityError|SecurityError",
            "{:?}",
            page.diagnostics
        );
    }

    #[test]
    fn worker_messages_wait_for_a_registered_parent_handler() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const worker=new Worker('data:,'+encodeURIComponent('postMessage({value:"complete"})'));
            worker.onmessage=event=>document.querySelector('#result').textContent=event.data.value;
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .unwrap(),
            "complete"
        );
    }

    #[test]
    fn native_text_encoding_survives_page_bootstrap_and_worker_exposure() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            globalThis.pageEncoding=[new TextDecoder('latin1').encoding,
                new TextDecoder('latin1').decode(new Uint8Array([128])),
                [...new TextEncoder().encode('\ud800')].join(',')].join('|');
            const worker=new Worker('data:,'+encodeURIComponent(`
                const decoder=new TextDecoder('utf-8',{fatal:true});
                decoder.decode(new Uint8Array([0xe2]),{stream:true});
                postMessage([decoder.decode(new Uint8Array([0x82,0xac])),
                  [...new TextEncoder().encode('💡')].join(',')].join('|'));
            `));
            worker.onmessage=event=>document.querySelector('#result').textContent=event.data;
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                "__oryn_drain(true);pageEncoding+'|'+document.querySelector('#result').textContent"
            )
            .unwrap(),
            "windows-1252|€|239,191,189|€|240,159,146,161"
        );
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
    }

    #[test]
    fn blob_url_worker_executes_registered_rust_owned_bytes() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const output=document.querySelector('#result');
            const dedicatedUrl=URL.createObjectURL(new Blob(['postMessage({value:"dedicated"})'],{type:'application/javascript'}));
            globalThis.dedicated=new Worker(dedicatedUrl);dedicated.onmessage=event=>output.textContent=event.data.value;
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        let value = page
            .evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
            .unwrap();
        assert_eq!(value, "dedicated");
    }
}
