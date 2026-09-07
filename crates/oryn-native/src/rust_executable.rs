//! Production executable page backed by the Rust-owned live DOM.
//!
//! This module never exports or reconstructs a JavaScript-owned node graph.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Duration,
};

use base64::Engine as _;
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

const DOCUMENT_LIFECYCLE_SCRIPT: &str = "document.dispatchEvent(new Event('DOMContentLoaded',{bubbles:true}));if(document.body){if(typeof document.body.onload==='function')globalThis.onload=document.body.onload;else{const source=document.body.getAttribute('onload');if(source)globalThis.onload=Function(source)}}dispatchEvent(new Event('load'))";

struct LiveLayoutProvider {
    dom: SharedDomRuntime,
    viewport: oryn_common::v4::ViewportState,
}

impl BindingLayoutProvider for LiveLayoutProvider {
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
    for (node, resolved) in &output.resolved_values {
        if let Some(values) = styles.get_mut(node) {
            values.extend(
                resolved
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
    }
    BindingLayout {
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
            let base = Url::parse(&parent).ok();
            for dependency in module_dependencies(&source) {
                if resource_entry(resources, &dependency).is_some() {
                    continue;
                }
                let Some(url) = base
                    .as_ref()
                    .and_then(|base| base.join(&dependency).ok())
                    .or_else(|| Url::parse(&dependency).ok())
                else {
                    continue;
                };
                let canonical = url.to_string();
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
                let dependency_alias = dependency.clone();
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
                            } => Ok((dependency_alias, url, source)),
                            _ => Err("parent returned an invalid module response".into()),
                        });
                    let _ = sender.send(result);
                });
                scheduled = scheduled.saturating_add(1);
            }
        }
        for _ in 0..scheduled {
            let (alias, url, source) = receiver
                .recv()
                .map_err(|_| HostError::Execution("module scheduler disconnected".into()))?
                .map_err(HostError::Execution)?;
            resources.insert(format!("oryn:module-alias:{alias}"), url.clone());
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
    let parser_blocking = script_type.as_deref().is_none_or(|kind| {
        matches!(
            kind,
            "" | "text/javascript" | "application/javascript" | "text/babel"
        )
    }) && dom
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
    if *no_module {
        return Ok(());
    }
    if script_type.as_deref().is_some_and(|kind| {
        !matches!(
            kind,
            "" | "module" | "text/javascript" | "application/javascript" | "text/babel"
        )
    }) {
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
    host.execute("__oryn_refresh_window_named_properties()")?;
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
            preload_module_graph(client, *page_id, &name, &source, resources)?;
        }
        host.execute_module_script(&name, &source, resources, src.is_some())
            .map_err(|error| script_diagnostic_at(error, &name))
    } else {
        let source = src
            .as_ref()
            .and_then(|src| resource_entry(resources, src))
            .map(|(_, source)| source)
            .unwrap_or(inline);
        let transformed;
        let source = if script_type.as_deref() == Some("text/babel") {
            let encoded = serde_json::to_string(source)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            transformed = host.evaluate_string(&format!(
                "Babel.transform({encoded},{{presets:['react']}}).code"
            ))?;
            transformed.as_str()
        } else {
            source
        };
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
    host.execute("void 0")?;
    // The platform host receives Fetch/XHR/IndexedDB submissions immediately,
    // but their completions are page tasks, not parser microtasks. Polling
    // completions here can resume an `await` before html5ever has consumed the
    // markup after this parser-blocking script (and, for example, before
    // `document.body` exists). Completion tasks are drained only after parsing
    // resumes or at the document load barrier.
    host.execute("__oryn_run_custom_reactions()")?;
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
    Live(Box<dyn Iterator<Item = Result<String, HostError>>>),
    Snapshot(ParsedDocument),
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
                    },
                )
                .map_err(|error| error.to_string())
                .and_then(|response| match response {
                    crate::context_service::ContextServiceResponse::PageResource {
                        url,
                        source,
                        destination,
                        content_type,
                        ..
                    } if destination == "frame" => Ok(LoadedFrameResource {
                        url,
                        source,
                        content_type,
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
            && node.attributes.get("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            })
            && let Some(href) = node.attributes.get("href")
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

struct LiveParseResult {
    document: ParsedDocument,
    scripts: Vec<ScriptRecord>,
    frames: BTreeMap<String, LoadedFrameResource>,
}

fn parse_live_document(
    mut source: Box<dyn Iterator<Item = Result<String, HostError>>>,
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
    let mut parser = html::StreamingParser::with_runtime(live_dom.clone());
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
        let Some(mut chunk) = next else { break };
        if chunk.is_empty() {
            continue;
        }
        let progress = parser.push_until_blocked(&chunk);
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
                let record = script_record(&live_dom.borrow(), *node)?;
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
globalThis.window=globalThis;
globalThis.self=globalThis;
globalThis.global=globalThis;
// A top-level Window is its own parent and top. WPT's harness determines the
// browsing-context root before any test code runs, so leaving these absent
// prevents every generated-binding assertion from reaching the interface.
globalThis.parent=globalThis;
globalThis.top=globalThis;
globalThis.frames=globalThis;
globalThis.frameElement=null;globalThis.__oryn_document_detached=false;
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
globalThis.__oryn_custom_reactions=[];
globalThis.__oryn_run_custom_reactions=()=>{for(let rounds=0;__oryn_custom_reactions.length&&rounds<1024;rounds++){const reactions=__oryn_custom_reactions.splice(0);for(const reaction of reactions)reaction()}if(__oryn_custom_reactions.length)throw new Error('custom-element reaction limit exceeded')};
globalThis.__oryn_custom_definitions=new Map();
const __oryn_upgrade_custom_element=node=>{if(!node||node.namespaceURI!=='http://www.w3.org/1999/xhtml')return null;const definition=__oryn_custom_definitions.get(String(node.localName).toLowerCase());if(!definition||node.__orynCustomDefinition===definition)return null;try{__oryn_begin_custom_element_upgrade(node);const upgraded=Reflect.construct(definition,[],definition);upgraded.__orynCustomDefinition=definition;return upgraded}catch(error){__oryn_begin_custom_element_upgrade(null);__oryn_console.push({level:'error',message:'custom element constructor: '+String(error)});return null}};
const __oryn_queue_connected=node=>{if(node&&node.isConnected&&typeof node.connectedCallback==='function'&&!node.__orynConnected){node.__orynConnected=true;__oryn_custom_reactions.push(()=>node.connectedCallback())}};
globalThis.customElements={define(name,constructor){name=String(name).toLowerCase();if(!name.includes('-'))throw new Error('custom element name must contain a hyphen');if(__oryn_custom_definitions.has(name))throw new Error('custom element already defined');__oryn_custom_definitions.set(name,constructor);for(const node of Array.from(document.getElementsByTagName(name))){const upgraded=__oryn_upgrade_custom_element(node);if(upgraded)__oryn_queue_connected(upgraded)}__oryn_run_custom_reactions()},get(name){return __oryn_custom_definitions.get(String(name).toLowerCase())},whenDefined(name){name=String(name).toLowerCase();if(this.get(name))return Promise.resolve(this.get(name));return new Promise(resolve=>{const timer=setInterval(()=>{if(this.get(name)){clearInterval(timer);resolve(this.get(name))}},1)})},upgrade(root){const apply=node=>{const upgraded=__oryn_upgrade_custom_element(node);if(upgraded)__oryn_queue_connected(upgraded)};if(root&&root.tagName)apply(root);if(root&&root.querySelectorAll)for(const node of root.querySelectorAll('*'))apply(node);__oryn_run_custom_reactions()}};
for(const [owner,method] of [[Document.prototype,'createElement'],[Node.prototype,'appendChild'],[Node.prototype,'insertBefore'],[Node.prototype,'replaceChild']]){const native=owner[method];if(typeof native!=='function')continue;Object.defineProperty(owner,method,{configurable:true,writable:true,value:function(...args){const result=native.apply(this,args),node=method==='createElement'?result:args[0],upgraded=__oryn_upgrade_custom_element(node);if(method!=='createElement'){__oryn_queue_connected(upgraded||node);__oryn_run_custom_reactions()}return method==='createElement'?(upgraded||result):result}})}
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
Object.defineProperty(globalThis,'location',{value:{href:'about:blank'},writable:true,enumerable:true,configurable:true});
Object.defineProperty(globalThis,'origin',{configurable:true,enumerable:true,get(){return location.origin}});
globalThis.__oryn_layout={styles:{},geometry:{},viewport:{width:1280,height:720,device_pixel_ratio:1},scroll:{},media:{}};
globalThis.innerWidth=1280;globalThis.innerHeight=720;
globalThis.__oryn_install_layout=value=>{globalThis.__oryn_layout=value;globalThis.innerWidth=Number(value.viewport?.width||0);globalThis.innerHeight=Number(value.viewport?.height||0)};
globalThis.__oryn_window_named_properties=new Map();
globalThis.__oryn_refresh_window_named_properties=()=>{
  const bindings=__oryn_window_named_properties,next=new Set();
  for(const [name,binding] of bindings){
    const current=Object.getOwnPropertyDescriptor(globalThis,name);
    if(!current||current.get!==binding.get||current.set!==binding.set)bindings.delete(name);
  }
  for(const element of document.querySelectorAll('[id]')){
    const name=String(element.id||'');if(!name)continue;
    if(Object.prototype.hasOwnProperty.call(globalThis,name)&&!bindings.has(name))continue;
    next.add(name);
    if(bindings.has(name))continue;
    const binding={configurable:true,enumerable:true,
      get(){return document.getElementById(name)||undefined},
      set(value){
        Object.defineProperty(this,name,{value,writable:true,configurable:true,enumerable:true});
        if(this===globalThis)bindings.delete(name);
      }};
    Object.defineProperty(globalThis,name,binding);bindings.set(name,binding);
  }
  for(const name of bindings.keys())if(!next.has(name)){delete globalThis[name];bindings.delete(name)}
};
globalThis.scrollX=globalThis.pageXOffset=0;globalThis.scrollY=globalThis.pageYOffset=0;
globalThis.scrollTo=(x=0,y=0)=>{if(x&&typeof x==='object'){y=x.top??scrollY;x=x.left??scrollX}scrollX=pageXOffset=Number(x)||0;scrollY=pageYOffset=Number(y)||0};
globalThis.scrollBy=(x=0,y=0)=>{if(x&&typeof x==='object'){y=x.top??0;x=x.left??0}scrollTo(scrollX+(Number(x)||0),scrollY+(Number(y)||0))};
globalThis.__oryn_media={color_scheme:'light',reduced_motion:'no_preference'};
globalThis.matchMedia=query=>{query=String(query);const result={media:query,onchange:null,addEventListener(){},removeEventListener(){},addListener(){},removeListener(){}};Object.defineProperty(result,'matches',{get(){if(/prefers-color-scheme\s*:\s*dark/i.test(query))return __oryn_media.color_scheme==='dark';if(/prefers-color-scheme\s*:\s*light/i.test(query))return __oryn_media.color_scheme==='light';if(/prefers-reduced-motion\s*:\s*reduce/i.test(query))return __oryn_media.reduced_motion==='reduce';if(/prefers-reduced-motion\s*:\s*no-preference/i.test(query))return __oryn_media.reduced_motion==='no_preference';return true}});return result};
class OrynStorage {
  constructor(area,items=[]){this.area=area;this.fallback=new Map(items)}
  __response(operation,extra={}){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;return contextCall({operation:'storage',request:{operation,area:this.area,scope:{origin:location.origin,top_level_page_id:__oryn_page_id},...extra}}).response}
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
class OrynURLSearchParamsIterator {
  constructor(params,kind){__oryn_url_search_param_iterators.set(this,{params,kind,index:0})}
  next(){const record=__oryn_url_search_param_iterators.get(this);if(!record)throw new TypeError('Illegal invocation');if(record.index>=record.params.items.length)return {value:undefined,done:true};const item=record.params.items[record.index++];return {value:record.kind==='key'?item[0]:record.kind==='value'?item[1]:[item[0],item[1]],done:false}}
  [Symbol.iterator](){return this}
}
Object.setPrototypeOf(OrynURLSearchParamsIterator.prototype,Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
Object.defineProperty(OrynURLSearchParamsIterator.prototype,'next',{enumerable:true});
delete OrynURLSearchParamsIterator.prototype.constructor;
delete OrynURLSearchParamsIterator.prototype[Symbol.iterator];
Object.defineProperty(OrynURLSearchParamsIterator.prototype,Symbol.toStringTag,{value:'URLSearchParams Iterator',writable:false,enumerable:false,configurable:true});
class OrynURLSearchParams {
  constructor(init='',onChange=null){this.items=[];this.onChange=onChange;if(init&&typeof init==='object'){const entries=typeof init[Symbol.iterator]==='function'?[...init]:Object.entries(init);for(const entry of entries){if(!entry||entry.length!==2)throw new TypeError('URLSearchParams entry must contain two values');this.items.push([String(entry[0]),String(entry[1])])}return}for(const part of String(init).replace(/^\?/,'').split('&'))if(part){const [key,value='']=part.split('=');this.items.push([decodeURIComponent(key.replace(/\+/g,' ')),decodeURIComponent(value.replace(/\+/g,' '))])}}
  changed(){if(this.onChange)this.onChange()}
  append(key,value){this.items.push([String(key),String(value)]);this.changed()}
  set(key,value){this.delete(key);this.append(key,value)}
  get(key){const item=this.items.find(item=>item[0]===String(key));return item?item[1]:null}
  getAll(key){return this.items.filter(item=>item[0]===String(key)).map(item=>item[1])}
  has(key){return this.items.some(item=>item[0]===String(key))}
  delete(key){key=String(key);const length=this.items.length;for(let index=this.items.length-1;index>=0;index--)if(this.items[index][0]===key)this.items.splice(index,1);if(this.items.length!==length)this.changed()}
  get size(){return this.items.length}
  entries(){return new OrynURLSearchParamsIterator(this,'entry')}
  keys(){return new OrynURLSearchParamsIterator(this,'key')}
  values(){return new OrynURLSearchParamsIterator(this,'value')}
  forEach(callback,thisArg=undefined){for(let index=0;index<this.items.length;index++){const [key,value]=this.items[index];callback.call(thisArg,value,key,this)}}
  sort(){this.items=this.items.map((item,index)=>({item,index})).sort((left,right)=>left.item[0]<right.item[0]?-1:left.item[0]>right.item[0]?1:left.index-right.index).map(entry=>entry.item);this.changed()}
  [Symbol.iterator](){return this.entries()}
  toString(){return this.items.map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(value)).join('&')}
}
Object.defineProperty(OrynURLSearchParams.prototype,Symbol.toStringTag,{value:'URLSearchParams',writable:false,enumerable:false,configurable:true});
class OrynURL {
  constructor(input,base){input=String(input);if(base&&!/^[a-z][a-z0-9+.-]*:/i.test(input)){const parent=new OrynURL(base);const root=parent.host?parent.origin:parent.protocol;if(input.startsWith('#'))input=root+parent.pathname+parent.search+input;else if(input.startsWith('?'))input=root+parent.pathname+input;else input=input.startsWith('/')?root+input:root+parent.pathname.replace(/[^/]*$/,'')+input}const match=input.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i);if(!match)throw new TypeError('Invalid URL');this.protocol=match[1];this.hasAuthority=input.startsWith(this.protocol+'//');this.host=match[2]||'';this.hostname=this.host.split(':')[0];this.port=this.host.slice(this.hostname.length+1);this.pathname=match[3]||'/';this._search=match[4]||'';this._searchDirty=false;this.hash=match[5]||'';this.origin=this.host?this.protocol+'//'+this.host:'null';this.searchParams=new OrynURLSearchParams(this._search,()=>{this._searchDirty=true})}
  get search(){if(!this._searchDirty)return this._search;const query=this.searchParams.toString();return query?'?'+query:''}
  set search(value){this._search=String(value||'');this._searchDirty=false;this.searchParams=new OrynURLSearchParams(this._search,()=>{this._searchDirty=true})}
  get href(){const path=this.pathname||(this.host?'/':'');return (this.host?this.origin:(this.hasAuthority?this.protocol+'//':this.protocol))+path+this.search+this.hash}
  set href(value){Object.assign(this,new OrynURL(value,this.href))}
  toString(){return this.href}
  toJSON(){return this.href}
  static createObjectURL(blob){return __oryn_blob_url_create(blob)}
  static revokeObjectURL(url){__oryn_blob_url_revoke(String(url))}
}
globalThis.URL=OrynURL;globalThis.URLSearchParams=OrynURLSearchParams;
const __oryn_location=new OrynURL(location.href);
Object.defineProperty(globalThis,'location',{get(){return __oryn_location},set(value){__oryn_location.href=String(value)},enumerable:true,configurable:true});
Object.defineProperty(location,'assign',{configurable:true,enumerable:true,writable:true,value(url){this.href=new OrynURL(url,this.href).href}});
const __oryn_history_clone=value=>value===undefined?undefined:JSON.parse(JSON.stringify(value));
class OrynHistory {
  constructor(){this.entries=[{url:location.href,state:null}];this.index=0;this.scrollRestoration='auto'}
  get length(){return this.entries.length}
  get state(){return __oryn_history_clone(this.entries[this.index].state)}
  __entry(state,url){let next=new OrynURL(url===undefined||url===null||String(url)===''?location.href:url,location.href);if(location.origin!=='null'&&next.origin!==location.origin){const error=new Error('history URL must be same-origin');error.name='SecurityError';throw error}return {url:next.href,state:__oryn_history_clone(state)}}
  pushState(state,_unused,url){const entry=this.__entry(state,url);this.entries.splice(this.index+1);this.entries.push(entry);this.index=this.entries.length-1;Object.assign(location,new OrynURL(entry.url))}
  replaceState(state,_unused,url){const entry=this.__entry(state,url);this.entries[this.index]=entry;Object.assign(location,new OrynURL(entry.url))}
  go(delta=0){delta=Number(delta)||0;if(delta===0)return;const next=this.index+delta;if(next<0||next>=this.entries.length)return;const previous=location.href;this.index=next;const entry=this.entries[next];Object.assign(location,new OrynURL(entry.url));const pop=new Event('popstate');pop.state=__oryn_history_clone(entry.state);dispatchEvent(pop);if(new OrynURL(previous).hash!==location.hash){const hash=new Event('hashchange');hash.oldURL=previous;hash.newURL=location.href;dispatchEvent(hash)}}
  back(){this.go(-1)}
  forward(){this.go(1)}
}
if(globalThis.History&&globalThis.History.prototype)Object.setPrototypeOf(OrynHistory.prototype,globalThis.History.prototype);
// The generated Rust binding installs the production History object. Keep the
// compatibility implementation only for explicit legacy/no-binding probes.
if(!globalThis.history)globalThis.history=new OrynHistory();
const contextCall=request=>JSON.parse(globalThis.__oryn_context_call(JSON.stringify(request)));
Object.defineProperty(Document.prototype,'cookie',{configurable:true,enumerable:true,get(){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return '';const response=contextCall({operation:'get_document_cookie',page_id:__oryn_page_id});return response.result==='document_cookie'?response.value:''},set(value){if(typeof globalThis.__oryn_context_call==='function'&&__oryn_page_id!==null)contextCall({operation:'set_document_cookie',page_id:__oryn_page_id,value:String(value)})}});
Object.defineProperty(Document.prototype,'title',{configurable:true,enumerable:true,get(){const node=this.querySelector('title');return node?node.textContent:''},set(value){let node=this.querySelector('title');if(!node){node=this.createElement('title');const parent=this.querySelector('head')||this.documentElement;parent.appendChild(node)}node.textContent=String(value)}});
const __oryn_page_proxies=new Map();
const __oryn_page_proxy=pageId=>{if(pageId==null)return null;if(__oryn_page_proxies.has(pageId))return __oryn_page_proxies.get(pageId);const proxy={__orynPageId:pageId,__closed:false,get closed(){return this.__closed},get origin(){const response=contextCall({operation:'get_related_page_origin',source_page_id:__oryn_page_id,target_page_id:pageId});return response.result==='security_origin'?response.serialized:''},focus(){contextCall({operation:'activate_page',source_page_id:__oryn_page_id,target_page_id:pageId})},close(){contextCall({operation:'close_page',source_page_id:__oryn_page_id,target_page_id:pageId});this.__closed=true},postMessage(data,targetOrigin='*'){contextCall({operation:'post_message',source_page_id:__oryn_page_id,target_page_id:pageId,target_origin:String(targetOrigin),data})}};__oryn_page_proxies.set(pageId,proxy);return proxy};
globalThis.__oryn_configure_context=(pageId,openerPageId)=>{__oryn_page_id=pageId;globalThis.opener=__oryn_page_proxy(openerPageId);if(typeof globalThis.__oryn_context_call==='function'){const response=contextCall({operation:'get_security_origin',page_id:pageId});if(response.result==='security_origin'){location.origin=response.serialized;globalThis.__oryn_opaque_origin=!!response.opaque}}};
globalThis.open=(url='',name='')=>{url=String(url);name=String(name);if(name&&name!=='_blank'){const frame=[...document.querySelectorAll('iframe')].find(candidate=>candidate.name===name||candidate.getAttribute('name')===name);if(frame){if(url)frame.contentWindow.location.href=new OrynURL(url,location.href).href;return frame.contentWindow}}if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;const target=url?new OrynURL(url,location.href).href:null;const response=contextCall({operation:'open_page',opener_page_id:__oryn_page_id,url:target,name:name||null});if(response.result==='page_opened'){__oryn_context_trace.push({kind:'page_created',detail:{page_id:response.page_id,opener_page_id:__oryn_page_id}});return __oryn_page_proxy(response.page_id)}if(response.result==='page_denied')__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'popup.open',detail:response.reason}});return null};
globalThis.closed=false;
globalThis.close=()=>{if(closed||__oryn_page_id===null)return;globalThis.closed=true;__oryn_context_trace.push({kind:'page_closed',detail:{page_id:__oryn_page_id}})};
const __oryn_websockets=new Set();const __oryn_message_log=[];const __oryn_record_message=value=>{if(__oryn_message_log.length===128)__oryn_message_log.shift();__oryn_message_log.push(value)};let __oryn_context_event_id=0;
class SecurityPolicyViolationEvent extends Event {constructor(type,eventInitDict={}){super(type,{bubbles:true,cancelable:false});for(const [name,value] of Object.entries({documentURI:'',referrer:'',blockedURI:'',violatedDirective:'',effectiveDirective:'',originalPolicy:'',sourceFile:'',sample:'',disposition:'enforce',statusCode:0,lineNumber:0,columnNumber:0,...eventInitDict}))Object.defineProperty(this,name,{value,enumerable:true,writable:false,configurable:false})}}
globalThis.SecurityPolicyViolationEvent=SecurityPolicyViolationEvent;
const __oryn_structured_clone=globalThis.structuredClone;
globalThis.__oryn_deliver_message=(data,targetOrigin='*',source=null,sourceOrigin='')=>{targetOrigin=String(targetOrigin);if(targetOrigin!=='*'&&targetOrigin!=='/'&&targetOrigin!==location.origin)return;const cloned=__oryn_structured_clone(data);__oryn_record_message({origin:String(sourceOrigin),data:cloned});const event=new Event('message',{});event.data=cloned;event.origin=String(sourceOrigin);event.source=source;queueMicrotask(()=>dispatchEvent(event))};
globalThis.__oryn_make_window_proxy=(deliver,targetOrigin,sourceOrigin,sameOriginTarget=null)=>{const facade={__source:null,__closed:false,origin:String(targetOrigin),get closed(){return this.__closed},focus(){},close(){this.__closed=true},postMessage(data,requestedOrigin='*'){deliver(data,String(requestedOrigin),proxy.__source,String(sourceOrigin))}};const proxy=sameOriginTarget?new Proxy(facade,{get(target,key,receiver){if(Reflect.has(target,key))return Reflect.get(target,key,receiver);return Reflect.get(sameOriginTarget,key,sameOriginTarget)},set(target,key,value,receiver){if(Reflect.has(target,key))return Reflect.set(target,key,value,receiver);return Reflect.set(sameOriginTarget,key,value,sameOriginTarget)},has(target,key){return Reflect.has(target,key)||Reflect.has(sameOriginTarget,key)}}):facade;return proxy};
globalThis.__oryn_make_provisional_window_proxy=()=>{let target=null;const queued=[];const facade={__source:null,__closed:false,get closed(){return target?target.closed:this.__closed},get origin(){return target?target.origin:''},focus(){if(target)target.focus()},close(){this.__closed=true;if(target)target.close()},postMessage(data,requestedOrigin='*'){if(target)target.postMessage(data,requestedOrigin);else queued.push([data,String(requestedOrigin)])},__bind(committed){target=committed;this.__closed=false;this.__source=committed.__source;while(queued.length){const [data,origin]=queued.shift();target.postMessage(data,origin)}}};return new Proxy(facade,{get(object,key,receiver){if(Reflect.has(object,key))return Reflect.get(object,key,receiver);return target?Reflect.get(target,key,target):undefined},set(object,key,value,receiver){if(Reflect.has(object,key)||!target)return Reflect.set(object,key,value,receiver);return Reflect.set(target,key,value,target)},has(object,key){return Reflect.has(object,key)||(target&&Reflect.has(target,key))}})};
globalThis.postMessage=(data,targetOrigin='*')=>globalThis.__oryn_deliver_message(data,targetOrigin,globalThis,location.origin);
globalThis.__oryn_poll_context=()=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return;const response=contextCall({operation:'poll_events',page_id:__oryn_page_id,after_event_id:__oryn_context_event_id,max_events:32});if(response.result==='events')for(const record of response.events||[]){__oryn_context_event_id=Math.max(__oryn_context_event_id,record.event_id);const item=record.event;if(!item)continue;if(item.kind==='storage'){const change=item.change||{};const event=new Event('storage',{});event.key=change.key??null;event.oldValue=change.old_value??null;event.newValue=change.new_value??null;event.storageArea=change.area==='local'?localStorage:sessionStorage;event.url=location.href;dispatchEvent(event)}else if(item.kind==='message'){__oryn_record_message({origin:item.source_origin||'',data:item.data});__oryn_context_trace.push({kind:'opener_message',detail:{source_page_id:item.source_page_id,target_page_id:__oryn_page_id,origin:item.source_origin||''}});const event=new Event('message',{});event.data=item.data;event.origin=item.source_origin||'';event.source=__oryn_page_proxy(item.source_page_id);dispatchEvent(event)}else if(item.kind==='page_lifecycle'){const lifecycle=item.event||{};if(lifecycle.kind==='closed'){const proxy=__oryn_page_proxies.get(lifecycle.page_id);if(proxy)proxy.__closed=true}}else if(item.kind==='csp_violation'){const value=item.violation||{};const event=new SecurityPolicyViolationEvent('securitypolicyviolation',{documentURI:value.document_uri||'',referrer:value.referrer||'',blockedURI:value.blocked_uri||'',violatedDirective:value.effective_directive||'',effectiveDirective:value.effective_directive||'',originalPolicy:value.original_policy||'',sourceFile:value.source_file||'',sample:value.sample||'',disposition:value.disposition||'enforce',statusCode:value.status_code||0,lineNumber:value.line_number||0,columnNumber:value.column_number||0});document.dispatchEvent(event);dispatchEvent(event)}else if(item.kind==='csp_report'){__oryn_context_trace.push({kind:'csp_report',detail:{endpoint:item.endpoint||'',format:item.format||'',outcome:item.outcome||''}})}}for(const socket of [...__oryn_websockets])socket.__poll()};
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
class OrynTextEncoder {encode(value=''){const encoded=unescape(encodeURIComponent(String(value)));return Uint8Array.from(encoded,char=>char.charCodeAt(0))}encodeInto(value,destination){value=String(value);if(!(destination instanceof Uint8Array))throw new TypeError('destination must be a Uint8Array');let read=0,written=0;for(const scalar of value){const encoded=this.encode(scalar);if(written+encoded.length>destination.length)break;destination.set(encoded,written);written+=encoded.length;read+=scalar.length}return{read,written}}}
class OrynTextDecoder {decode(value=new Uint8Array()){let encoded='';for(const byte of new Uint8Array(value.buffer||value,value.byteOffset||0,value.byteLength??value.length))encoded+=String.fromCharCode(byte);return decodeURIComponent(escape(encoded))}}
globalThis.TextEncoder=OrynTextEncoder;globalThis.TextDecoder=OrynTextDecoder;
const b64chars='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const bytesToBase64=value=>{const bytes=value instanceof Uint8Array?value:new Uint8Array(value);let out='';for(let index=0;index<bytes.length;index+=3){const first=bytes[index],second=bytes[index+1],third=bytes[index+2];out+=b64chars[first>>2];out+=b64chars[((first&3)<<4)|((second??0)>>4)];out+=second===undefined?'=':b64chars[((second&15)<<2)|((third??0)>>6)];out+=third===undefined?'=':b64chars[third&63]}return out};
const base64ToBytes=value=>{value=String(value).replace(/\s/g,'');const out=[];for(let index=0;index<value.length;index+=4){const first=b64chars.indexOf(value[index]),second=b64chars.indexOf(value[index+1]),third=value[index+2]==='='?-1:b64chars.indexOf(value[index+2]),fourth=value[index+3]==='='?-1:b64chars.indexOf(value[index+3]);out.push((first<<2)|(second>>4));if(third>=0)out.push(((second&15)<<4)|(third>>2));if(fourth>=0)out.push(((third&3)<<6)|fourth)}return Uint8Array.from(out)};
globalThis.btoa=value=>bytesToBase64(Uint8Array.from(String(value),char=>char.charCodeAt(0)&255));globalThis.atob=value=>{let result='';for(const byte of base64ToBytes(value))result+=String.fromCharCode(byte);return result};
const bodyBytes=body=>{if(body==null)return new Uint8Array();if(body instanceof OrynBlob)return body.__bytes.slice();if(body instanceof ArrayBuffer)return new Uint8Array(body.slice(0));if(ArrayBuffer.isView(body))return new Uint8Array(body.buffer.slice(body.byteOffset,body.byteOffset+body.byteLength));return new TextEncoder().encode(String(body))};
class OrynBlob {constructor(parts=[],options={}){const chunks=parts.map(bodyBytes),length=chunks.reduce((sum,item)=>sum+item.length,0);this.__bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){this.__bytes.set(chunk,offset);offset+=chunk.length}this.type=String(options.type||'').toLowerCase()}get size(){return this.__bytes.length}slice(start=0,end=this.size,type=''){return new OrynBlob([this.__bytes.slice(start,end)],{type})}arrayBuffer(){return Promise.resolve(this.__bytes.buffer.slice(0))}text(){return Promise.resolve(new TextDecoder().decode(this.__bytes))}stream(){return new OrynReadableStream({start:controller=>{controller.enqueue(this.__bytes.slice());controller.close()}})}}
class OrynFile extends OrynBlob {constructor(parts,name,options={}){super(parts,options);this.name=String(name);this.lastModified=Number(options.lastModified??0);this.webkitRelativePath=''}}
globalThis.Blob=OrynBlob;globalThis.File=OrynFile;
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
const __orynInputFiles=node=>{const encoded=node.getAttribute('data-oryn-upload-files');if(!encoded)return [];try{return JSON.parse(encoded).map(file=>new File([base64ToBytes(file.body_base64)],file.filename,{type:file.media_type,lastModified:file.last_modified}))}catch(_){return []}};
const __oryn_testdriver_text_node=(node,last=false)=>{if(!node)return null;if(node.nodeType===3)return node;let fallback=null,children=[...(node.childNodes||[])];if(last)children.reverse();for(const child of children){const found=__oryn_testdriver_text_node(child,last);if(found&&!fallback)fallback=found;if(found&&String(found.data||'').trim())return found}return fallback};
const __oryn_testdriver_key_name=value=>({"\uE003":'Backspace',"\uE004":'Tab',"\uE007":'Enter',"\uE008":'Shift',"\uE009":'Control',"\uE00A":'Alt',"\uE010":'End',"\uE011":'Home',"\uE017":'Delete',"\uE03D":'Meta'}[value]||value);
const __oryn_testdriver_clipboard={text:'',html:''};
const __oryn_testdriver_data_transfer=()=>({getData(type){return type==='text/html'?__oryn_testdriver_clipboard.html:__oryn_testdriver_clipboard.text},setData(type,value){if(type==='text/html')__oryn_testdriver_clipboard.html=String(value);else __oryn_testdriver_clipboard.text=String(value)},types:['text/plain','text/html']});
const __oryn_testdriver_keyboard=(target,value,pressed)=>{
  const key=__oryn_testdriver_key_name(String(value));
  const event=new Event(pressed?'keydown':'keyup',{bubbles:true,cancelable:true});
  event.key=key;event.code=key.length===1?'Key'+key.toUpperCase():key;
  target.dispatchEvent(event);
  const modifiers=globalThis.__oryn_testdriver_modifiers||(globalThis.__oryn_testdriver_modifiers=new Set());
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
      if(controlPaste){const current=String(target.value||'');let start=Number(target.selectionStart??target.__oryn_selection_start??current.length),end=Number(target.selectionEnd??target.__oryn_selection_end??start);target.value=current.slice(0,start)+pasted+current.slice(end);target.selectionStart=target.selectionEnd=target.__oryn_selection_start=target.__oryn_selection_end=start+pasted.length;target.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertFromPaste',data:pasted,dataTransfer:null}))}
      else{const html=transfer.getData('text/html');globalThis.__oryn_pending_input_event={inputType:'insertFromPaste',data:null,dataTransfer:transfer};try{document.execCommand(html&&/<[a-z!/]/i.test(html)?'insertHTML':'insertText',false,html||pasted);const selection=document.getSelection();let host=selection?.anchorNode;while(host&&!(host.nodeType===1&&host.isContentEditable))host=host.parentElement;for(const styled of host?.querySelectorAll?.('[style]')||[]){const value=styled.getAttribute('style');if(value&&!value.trimEnd().endsWith(';'))styled.setAttribute('style',value.trimEnd()+';')}}finally{delete globalThis.__oryn_pending_input_event}}
    }return
  }
  if(command&&key.toLowerCase()==='a'){
    document.execCommand('selectAll',false,'');return
  }
  const tag=String(target.tagName||'').toLowerCase();
  const textControl=tag==='textarea'||(tag==='input'&&!['button','checkbox','file','radio','reset','submit'].includes(String(target.type||'').toLowerCase()));
  if(textControl&&(key==='Backspace'||key==='Delete'||key.length===1)){
    const unavailable=()=>!target.isConnected||target.getClientRects().length===0||getComputedStyle(target).visibility==='hidden';
    if(unavailable())return;
    if(key.length===1){
      const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);
      if(keypress.defaultPrevented||unavailable())return;
    }
    const inputType=key==='Backspace'?'deleteContentBackward':key==='Delete'?'deleteContentForward':'insertText';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType,data:key.length===1?key:null});target.dispatchEvent(before);
    if(before.defaultPrevented||unavailable())return;
    const current=String(target.value||'');
    let start=Number(target.selectionStart??target.__oryn_selection_start??current.length),end=Number(target.selectionEnd??target.__oryn_selection_end??start);
    start=Math.max(0,Math.min(current.length,start));end=Math.max(start,Math.min(current.length,end));
    let replacement='';
    if(key==='Backspace'&&start===end&&start>0)start--;else if(key==='Delete'&&start===end&&end<current.length)end++;else if(key.length===1)replacement=key;
    target.value=current.slice(0,start)+replacement+current.slice(end);
    target.selectionStart=target.selectionEnd=target.__oryn_selection_start=target.__oryn_selection_end=start+replacement.length;
    const input=new Event('input',{bubbles:true});input.inputType=inputType;input.data=replacement||null;target.dispatchEvent(input);return
  }
  if(key==='Enter'){
    const lineBreak=modifiers.has('Shift'),inputType=lineBreak?'insertLineBreak':'insertParagraph';
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
  }else if(key.length===1){
    const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);if(keypress.defaultPrevented)return;
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertText',data:key});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand('insertText',false,key)
  }
};
const __oryn_testdriver_key_target=target=>{const focused=document.activeElement;if(!focused||focused===target)return target;let current=focused;for(let depth=0;current&&depth<32;depth++){if(current===target)return focused;const root=current.getRootNode?.({composed:false});current=root&&root!==document?root.host:current.parentElement}return target};
globalThis.__oryn_testdriver_send_keys=async(target,keys)=>{if(!target)throw new TypeError('testdriver target is required');target=__oryn_testdriver_key_target(target);target.focus?.();keys=String(keys);if(String(target.tagName).toLowerCase()==='input'&&String(target.type).toLowerCase()==='file'){const response=contextCall({operation:'materialize_upload',page_id:__oryn_page_id,label:keys});if(response.result!=='upload_materialized')throw new Error('parent did not authorize the selected upload');target.setAttribute('data-oryn-upload-files',JSON.stringify([{filename:response.filename,media_type:response.media_type,body_base64:response.body_base64,last_modified:response.last_modified}]));target.value=response.filename;target.dispatchEvent(new Event('input',{bubbles:true}));target.dispatchEvent(new Event('change',{bubbles:true}));return}for(const key of [...keys]){__oryn_testdriver_keyboard(target,key,true);__oryn_testdriver_keyboard(target,key,false)}};
globalThis.__oryn_testdriver_click=async target=>{if(!target)throw new TypeError('testdriver target is required');target.focus?.();target.dispatchEvent(new Event('pointerdown',{bubbles:true,cancelable:true}));target.dispatchEvent(new Event('mousedown',{bubbles:true,cancelable:true}));target.dispatchEvent(new Event('pointerup',{bubbles:true,cancelable:true}));target.dispatchEvent(new Event('mouseup',{bubbles:true,cancelable:true}));target.click?.()};
globalThis.__oryn_testdriver_actions=async(sources,keyboardTarget=null)=>{let pointerTarget=null,pointerClicks=0;const modifiers=globalThis.__oryn_testdriver_modifiers||(globalThis.__oryn_testdriver_modifiers=new Set()),touchPointers=new Set(),dispatchInput=(target,type)=>__oryn_dispatch_user_input(target,type,false);const lists=(sources||[]).map(source=>source.actions||[]);const ticks=Math.max(0,...lists.map(actions=>actions.length));for(let tick=0;tick<ticks;tick++)for(let index=0;index<sources.length;index++){const source=sources[index],action=lists[index][tick];if(!action||action.type==='pause')continue;if(action.type==='scroll'){const target=document.elementFromPoint?.(Number(action.x)||0,Number(action.y)||0)||document.activeElement||document.body;for(const type of ['wheel','mousewheel'])dispatchInput(target,type);continue}if(action.type==='pointerMove'){if(action.origin&&typeof action.origin==='object')pointerTarget=action.origin;else if(!touchPointers.has(source.id))pointerTarget=document.elementFromPoint?.(Number(action.x)||0,Number(action.y)||0)||document.activeElement;if(touchPointers.has(source.id))dispatchInput(pointerTarget,'touchmove');continue}if(action.type==='pointerDown'){pointerTarget?.focus?.();pointerTarget?.dispatchEvent(new Event('pointerdown',{bubbles:true,cancelable:true}));pointerTarget?.dispatchEvent(new Event('mousedown',{bubbles:true,cancelable:true}));if(source.parameters?.pointerType==='touch'){touchPointers.add(source.id);dispatchInput(pointerTarget,'touchstart')}continue}if(action.type==='pointerUp'){pointerTarget?.dispatchEvent(new Event('pointerup',{bubbles:true,cancelable:true}));pointerTarget?.dispatchEvent(new Event('mouseup',{bubbles:true,cancelable:true}));if(touchPointers.delete(source.id))dispatchInput(pointerTarget,'touchend');pointerTarget?.click?.();pointerClicks++;if(pointerClicks===2){const text=__oryn_testdriver_text_node(pointerTarget);if(text){const selection=document.getSelection();selection.setBaseAndExtent(text,0,text,text.data.length)}}continue}const target=keyboardTarget||document.activeElement||document.body;if(action.type==='keyDown')__oryn_testdriver_keyboard(target,action.value,true);else if(action.type==='keyUp'){const key=__oryn_testdriver_key_name(String(action.value));__oryn_testdriver_keyboard(target,action.value,false);modifiers.delete(key)}}};
const __oryn_install_text_selection=prototype=>{if(!prototype)return;Object.defineProperties(prototype,{selectionStart:{configurable:true,get(){const length=String(this.value||'').length;return Math.max(0,Math.min(length,Number(this.__oryn_selection_start??length)))},set(value){const length=String(this.value||'').length;this.__oryn_selection_start=Math.max(0,Math.min(length,Number(value)||0));if(this.__oryn_selection_end===undefined||this.__oryn_selection_end<this.__oryn_selection_start)this.__oryn_selection_end=this.__oryn_selection_start}},selectionEnd:{configurable:true,get(){const length=String(this.value||'').length;return Math.max(0,Math.min(length,Number(this.__oryn_selection_end??this.selectionStart)))},set(value){const length=String(this.value||'').length;this.__oryn_selection_end=Math.max(0,Math.min(length,Number(value)||0));if(this.__oryn_selection_start===undefined||this.__oryn_selection_start>this.__oryn_selection_end)this.__oryn_selection_start=this.__oryn_selection_end}},selectionDirection:{configurable:true,get(){return this.__oryn_selection_direction||'none'},set(value){this.__oryn_selection_direction=['forward','backward'].includes(String(value))?String(value):'none'}},setSelectionRange:{configurable:true,value(start,end,direction='none'){this.__oryn_selection_start=Math.max(0,Math.min(String(this.value||'').length,Number(start)||0));this.__oryn_selection_end=Math.max(this.__oryn_selection_start,Math.min(String(this.value||'').length,Number(end)||0));this.selectionDirection=direction}},setRangeText:{configurable:true,value(replacement,start=this.selectionStart,end=this.selectionEnd,mode='preserve'){const value=String(this.value||''),text=String(replacement);start=Math.max(0,Math.min(value.length,Number(start)||0));end=Math.max(start,Math.min(value.length,Number(end)||0));this.value=value.slice(0,start)+text+value.slice(end);const finish=start+text.length;if(mode==='select')this.setSelectionRange(start,finish);else if(mode==='start')this.setSelectionRange(start,start);else if(mode==='end')this.setSelectionRange(finish,finish);else this.setSelectionRange(finish,finish)}}})};
if(globalThis.HTMLInputElement){Object.defineProperty(HTMLInputElement.prototype,'files',{configurable:true,get(){return __orynInputFiles(this)}});__oryn_install_text_selection(HTMLInputElement.prototype)}
if(globalThis.HTMLTextAreaElement)__oryn_install_text_selection(HTMLTextAreaElement.prototype);
class OrynFormData {constructor(form){this.items=[];if(form)for(const node of [...form.querySelectorAll('input'),...form.querySelectorAll('textarea'),...form.querySelectorAll('select')]){const name=node.getAttribute('name');if(!name||node.disabled)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;if(type==='file'){for(const file of __orynInputFiles(node))this.append(name,file);continue}this.append(name,node.value)}}append(name,value,filename){if(filename!==undefined&&value instanceof OrynBlob)value=new OrynFile([value.__bytes],filename,{type:value.type});this.items.push([String(name),value instanceof OrynBlob?value:String(value)])}set(name,value,filename){this.delete(name);this.append(name,value,filename)}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1])}has(name){return this.items.some(item=>item[0]===String(name))}delete(name){this.items=this.items.filter(item=>item[0]!==String(name))}entries(){return this.items[Symbol.iterator]()}keys(){return this.items.map(item=>item[0])[Symbol.iterator]()}values(){return this.items.map(item=>item[1])[Symbol.iterator]()}[Symbol.iterator](){return this.entries()}}
globalThis.FormData=OrynFormData;
globalThis.crypto={getRandomValues(array){const encoded=__oryn_crypto_sync('random',String(array.byteLength));array.set(base64ToBytes(encoded));return array},randomUUID(){const bytes=new Uint8Array(16);this.getRandomValues(bytes);bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`},subtle:{digest(algorithm,value){if(String(algorithm).toUpperCase()!=='SHA-256')return Promise.reject(new Error('unsupported digest'));const result=base64ToBytes(__oryn_crypto_sync('sha256',bytesToBase64(bodyBytes(value))));return Promise.resolve(result.buffer)}}};
const __oryn_idb_call=command=>{const outer=contextCall({operation:'storage',request:{operation:'indexed_db',origin:location.origin,command}});if(outer.result!=='storage'||outer.response.result!=='indexed_db')throw new Error('invalid IndexedDB response');return outer.response.response};
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
if(typeof globalThis.__oryn_context_call==='function')for(const [name,value] of Object.entries(__oryn_generated_platform))if(value!==undefined)globalThis[name]=value;
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
    live_dom: Rc<RefCell<DomRuntime>>,
    host: RawV8Host,
    top_realm: RealmId,
    node_realms: BTreeMap<NodeId, RealmId>,
    frame_realms: BTreeMap<RealmId, FrameRealm>,
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
            DocumentLoad::Live(Box::new(std::iter::once(Ok(html_source.to_owned())))),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            None,
        )
    }

    pub(crate) fn load_stream_with_limits(
        chunks: Box<dyn Iterator<Item = Result<String, HostError>>>,
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
            DocumentLoad::Snapshot(parsed),
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
            DocumentLoad::Live(Box::new(std::iter::once(Ok(html_source.to_owned())))),
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    #[cfg(unix)]
    pub(crate) fn load_stream_with_context_and_limits(
        chunks: Box<dyn Iterator<Item = Result<String, HostError>>>,
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
            DocumentLoad::Snapshot(parsed),
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
        host.execute(crate::observable::OBSERVABLE_GLOBALS)?;
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
        host.set_module_sources(&resources);
        if let Some(base_url) = &base_url {
            let url = serde_json::to_string(base_url.as_str())
                .map_err(|error| HostError::Execution(error.to_string()))?;
            // Seed the realm's committed URL without invoking Location's public
            // navigation path. Internal fixture URLs are valid document origins
            // but must never escape through the network broker as navigations.
            host.execute(&format!(
                "Object.assign(globalThis.location,new OrynURL({url}))"
            ))?;
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
                    )
                })
                .or_else(|| {
                    resources
                        .get(&source)
                        .cloned()
                        .map(|body| (requested_url.clone(), body, None::<String>))
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
                    },
                ) {
                    Ok(crate::context_service::ContextServiceResponse::PageResource {
                        url,
                        source: body,
                        destination,
                        content_type,
                        ..
                    }) if destination == "frame" => Some((
                        Url::parse(&url).ok().or(requested_url.clone()),
                        body,
                        content_type,
                    )),
                    _ => None,
                };
            }
            let Some((frame_url, frame_html, content_type)) = loaded_frame else {
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
            let child = oryn_dom::xml::parse_by_mime(&frame_html, content_type.as_deref());
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
                    .append_child(frame, document)
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
            host.execute(RUST_RUNTIME_GLOBALS)?;
            host.execute(crate::observable::OBSERVABLE_GLOBALS)?;
            host.install_layout_provider(Rc::new(LiveLayoutProvider {
                dom: frame_dom.clone(),
                viewport: oryn_common::v4::ViewportState::default(),
            }))?;
            host.set_module_sources(&resources);
            #[cfg(unix)]
            if let Some((page_id, _)) = context_identity {
                host.execute(&format!("__oryn_configure_context({}, null)", page_id.0))?;
            }
            if let Some(frame_url) = &frame_url {
                let url = serde_json::to_string(frame_url.as_str())
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                // Frame creation installs an already-authorized document URL;
                // assigning href here would incorrectly schedule navigation.
                host.execute(&format!(
                    "Object.assign(globalThis.location,new OrynURL({url}))"
                ))?;
            }
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
            live_dom,
            host,
            top_realm,
            node_realms,
            frame_realms: BTreeMap::new(),
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
            page.host.activate_realm(page.top_realm)?;
            page.host.ensure_provisional_frame_window(frame)?;
            page.host.schedule_dynamic_frame(frame)?;
        }
        // Parser-discovered frame scripts are prepared before the remaining
        // parent scripts, but their lifecycle fires only after the parent has
        // had a chance to install message/load observers. Each child realm gets
        // its own events and scheduler checkpoint before the top-level load
        // barrier is released.
        for realm in initial_frame_realms {
            page.host.activate_realm(realm)?;
            page.host.execute(DOCUMENT_LIFECYCLE_SCRIPT)?;
            // A frame's load task may schedule the zero-delay completion task
            // used by testharness and by application bootstraps. Run timers
            // that are due now while the child realm is active; otherwise the
            // top realm can reach its own load barrier with the child remote
            // context permanently awaiting completion.
            page.host.drain_scheduled_timers(false)?;
            page.checkpoint()?;
        }
        page.host.activate_realm(page.top_realm)?;
        // Dynamic external/module scripts inserted by parser-time JavaScript
        // are force-async but still participate in the document load barrier.
        page.drain_dynamic_frames()?;
        page.drain_dynamic_stylesheets()?;
        page.drain_dynamic_scripts()?;
        page.host.drain_async_fetches(true)?;
        // Zero-delay framework commits queued before load run at this
        // checkpoint. Future timers retain their deadlines and must not be
        // pulled forward merely because navigation reached the load barrier.
        page.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        page.host.execute(DOCUMENT_LIFECYCLE_SCRIPT)?;
        page.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        Ok(page)
    }

    pub fn execute(&mut self, source: &str) -> Result<(), HostError> {
        self.host.execute(source)?;
        self.checkpoint()
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, HostError> {
        if source.contains("__oryn_drain(true)") {
            self.drain_tasks()?;
        }
        let value = self.host.evaluate_string(source)?;
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
            let dom = self.live_dom.borrow();
            dom.query_selector(dom.document(), selector)
        }
        .map_err(|error| HostError::Execution(error.to_string()))?
        .ok_or_else(|| HostError::Execution("no matching element".into()))?;
        self.live_dom
            .borrow_mut()
            .set_value(node, value, false)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.execute_with_node(
            node,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        )?;
        self.checkpoint()
    }

    pub fn location(&mut self) -> Result<String, HostError> {
        self.host.evaluate_string("location.href")
    }

    pub fn set_committed_location(&mut self, url: &str) -> Result<(), HostError> {
        let url =
            serde_json::to_string(url).map_err(|error| HostError::Execution(error.to_string()))?;
        self.host
            .execute(&format!("Object.assign(location,new URL({url}))"))
    }

    pub fn take_history_traversal(&self) -> Result<Option<i64>, HostError> {
        self.host.take_history_traversal()
    }

    pub fn dispatch_page_transition(
        &mut self,
        event_type: &str,
        persisted: bool,
    ) -> Result<(), HostError> {
        let event_type = serde_json::to_string(event_type)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.execute(&format!(
            "{{const event=new Event({event_type});Object.defineProperty(event,'persisted',{{value:{persisted},enumerable:true}});dispatchEvent(event)}}"
        ))?;
        self.checkpoint()
    }

    pub fn suspend_for_back_forward_cache(&mut self) {
        self.host.suspend_page_isolate();
    }

    pub fn resume_from_back_forward_cache(&mut self) {
        self.host.resume_page_isolate();
    }

    pub fn has_pending_page_work(&mut self) -> bool {
        self.host.has_pending_page_work()
    }

    pub fn has_runnable_page_work(&mut self) -> bool {
        self.host.has_runnable_page_work()
    }

    pub fn storage_snapshot(&mut self) -> Result<(String, String), HostError> {
        Ok((
            self.host
                .evaluate_string("JSON.stringify([...localStorage.items])")?,
            self.host
                .evaluate_string("JSON.stringify([...sessionStorage.items])")?,
        ))
    }

    pub fn take_console_entries(&mut self) -> Result<Vec<ConsoleEntry>, HostError> {
        let value = self
            .host
            .evaluate_string("JSON.stringify(__oryn_take_console())")?;
        serde_json::from_str(&value).map_err(|error| HostError::Execution(error.to_string()))
    }

    pub fn take_context_trace_entries(&mut self) -> Result<Vec<ContextTraceEntry>, HostError> {
        let value = self
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
        self.host.drain_scheduled_timers(false)?;
        self.host.execute("__oryn_poll_context()")?;
        // Animation-frame callbacks are page tasks in their own right. They
        // must advance even when the preceding task changed only platform
        // state (for example scroll offsets) and therefore produced no
        // semantic/layout revision that would otherwise trigger a frame.
        self.host.execute("__oryn_run_animation_frames()")?;
        self.checkpoint()
    }

    pub fn poll_context_events(&mut self) -> Result<(), HostError> {
        self.host.execute("__oryn_poll_context()")?;
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
        #[derive(Serialize)]
        struct PageFile<'a> {
            filename: &'a str,
            media_type: &'a str,
            body_base64: String,
            last_modified: u64,
        }
        let is_file_input = {
            let dom = self.live_dom.borrow();
            dom.node_name(target)
                .is_ok_and(|name| name.eq_ignore_ascii_case("input"))
                && dom
                    .attribute(target, "type")
                    .ok()
                    .flatten()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
        };
        if !is_file_input {
            return Err(HostError::Execution(
                "file selection target is not an input[type=file]".into(),
            ));
        }
        let page_files = files
            .iter()
            .map(|file| PageFile {
                filename: &file.filename,
                media_type: &file.media_type,
                body_base64: base64::engine::general_purpose::STANDARD.encode(&file.bytes),
                last_modified: 0,
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&page_files)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        {
            let mut dom = self.live_dom.borrow_mut();
            dom.set_attribute(target, "data-oryn-upload-files", encoded)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            dom.set_value(
                target,
                files.first().map_or("", |file| file.filename.as_str()),
                false,
            )
            .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        let realm = self
            .node_realms
            .get(&target)
            .copied()
            .unwrap_or(self.top_realm);
        self.host.activate_realm(realm)?;
        let result = self.host.execute_with_node(
            target,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        );
        self.host.activate_realm(self.top_realm)?;
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
        let submission_form = self.submission_form(target, action)?;
        let submits_form = submission_form.is_some();
        let event_source = match action {
            SemanticAction::Click => {
                let input_kind = {
                    let dom = self.live_dom.borrow();
                    dom.node_name(target)
                        .is_ok_and(|name| name.eq_ignore_ascii_case("input"))
                        .then(|| {
                            dom.attribute(target, "type")
                                .ok()
                                .flatten()
                                .unwrap_or("")
                                .to_ascii_lowercase()
                        })
                };
                if matches!(input_kind.as_deref(), Some("checkbox" | "radio")) {
                    let mut dom = self.live_dom.borrow_mut();
                    if input_kind.as_deref() == Some("checkbox")
                        && dom
                            .attribute(target, "checked")
                            .map_err(|error| HostError::Execution(error.to_string()))?
                            .is_some()
                    {
                        dom.remove_attribute(target, "checked")
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    } else {
                        dom.set_attribute(target, "checked", "")
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    }
                }
                ""
            }
            SemanticAction::Type | SemanticAction::Clear => {
                self.live_dom
                    .borrow_mut()
                    .set_value(
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
                let mut dom = self.live_dom.borrow_mut();
                if action == SemanticAction::Check {
                    dom.set_attribute(target, "checked", "")
                } else {
                    dom.remove_attribute(target, "checked")
                }
                .map_err(|error| HostError::Execution(error.to_string()))?;
                "__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Select => {
                let requested = value.unwrap_or_default();
                let mut dom = self.live_dom.borrow_mut();
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
            SemanticAction::Submit => "",
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
        self.host.activate_realm(action_realm)?;
        let action_accepted = if action == SemanticAction::Click {
            self.host.evaluate_bool_with_node(
                target,
                "__oryn_action_target",
                "__oryn_action_target.dispatchEvent(new Event('click',{bubbles:true,cancelable:true}))",
            )
        } else {
            self.host
                .execute_with_node(target, "__oryn_action_target", event_source)
                .map(|()| true)
        };
        self.host.activate_realm(self.top_realm)?;
        let mut default_accepted = action_accepted?;
        if default_accepted && let Some(form) = submission_form {
            self.host.activate_realm(action_realm)?;
            let submit_accepted = self.host.evaluate_bool_with_node(
                form,
                "__oryn_submit_form",
                "__oryn_submit_form.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))",
            );
            self.host.activate_realm(self.top_realm)?;
            default_accepted = submit_accepted?;
        }
        if default_accepted && matches!(action, SemanticAction::Click | SemanticAction::Submit) {
            self.apply_default_navigation(target, action, submits_form, action_realm)?;
        }
        self.checkpoint()
    }

    fn submission_form(
        &self,
        target: NodeId,
        action: SemanticAction,
    ) -> Result<Option<NodeId>, HostError> {
        let dom = self.live_dom.borrow();
        let target_record = dom
            .arena()
            .get(target)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let submits = match (&target_record.kind, action) {
            (NodeKind::Element { name }, SemanticAction::Submit) => name.local.as_ref() == "form",
            (NodeKind::Element { name }, SemanticAction::Click)
                if name.local.as_ref() == "button" =>
            {
                !target_record.attributes.get("type").is_some_and(|value| {
                    value.eq_ignore_ascii_case("button") || value.eq_ignore_ascii_case("reset")
                })
            }
            (NodeKind::Element { name }, SemanticAction::Click)
                if name.local.as_ref() == "input" =>
            {
                target_record.attributes.get("type").is_some_and(|value| {
                    value.eq_ignore_ascii_case("submit") || value.eq_ignore_ascii_case("image")
                })
            }
            _ => false,
        };
        if !submits {
            return Ok(None);
        }
        let mut candidate = Some(target);
        while let Some(node) = candidate {
            let record = dom
                .arena()
                .get(node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            if matches!(&record.kind, NodeKind::Element { name } if name.local.as_ref() == "form") {
                return Ok(Some(node));
            }
            candidate = record.parent;
        }
        Ok(None)
    }

    pub(crate) fn focused_layout_node(&self) -> Option<u64> {
        self.live_dom
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
            .live_dom
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
        self.host
            .install_layout(binding_layout_from_output(output))?;
        self.host.execute(&format!(
            "globalThis.innerWidth={};globalThis.innerHeight={}",
            viewport.width, viewport.height
        ))?;
        self.host.execute("__oryn_run_geometry_observers()")?;
        self.checkpoint()
    }

    pub fn run_animation_frame_phase(&mut self) -> Result<(), HostError> {
        self.host.execute("__oryn_run_animation_frames()")?;
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
        self.host.execute(&format!(
            "__oryn_media={{color_scheme:{color_scheme},reduced_motion:{reduced_motion}}}"
        ))
    }

    fn checkpoint(&mut self) -> Result<(), HostError> {
        // Message delivery, promise jobs, default actions, and dynamic frame
        // loads can enqueue work in a different realm of the same page isolate.
        // Run a bounded page-wide fixed point instead of relying on an
        // unrelated future command to tick that realm.
        for _ in 0..8 {
            self.host.drain_async_fetches(true)?;
            self.drain_dynamic_referrer_policies()?;
            self.drain_default_actions()?;
            self.drain_frame_location_navigations()?;
            self.drain_dynamic_frames()?;
            self.drain_dynamic_stylesheets()?;
            self.drain_dynamic_scripts()?;
            self.host.drain_async_fetches(true)?;
            for realm in self.host.realm_ids() {
                self.host.activate_realm(realm)?;
                self.host.execute("__oryn_drain()")?;
                self.host.execute("__oryn_run_custom_reactions()")?;
                self.host.run_dom_observer_checkpoint()?;
                for report in self.host.take_module_error_reports() {
                    self.diagnostics
                        .push(script_diagnostic(HostError::Execution(report)));
                }
                for rejection in self.host.take_unhandled_promise_rejections() {
                    self.diagnostics
                        .push(script_diagnostic(HostError::Execution(format!(
                            "unhandled promise rejection: {rejection}"
                        ))));
                }
                self.host.process_binding_releases()?;
            }
        }
        self.host.activate_realm(self.top_realm)?;
        for report in self.host.take_module_error_reports() {
            self.diagnostics
                .push(script_diagnostic(HostError::Execution(report)));
        }
        for rejection in self.host.take_unhandled_promise_rejections() {
            self.diagnostics
                .push(script_diagnostic(HostError::Execution(format!(
                    "unhandled promise rejection: {rejection}"
                ))));
        }
        let (dom, document) = self.live_dom.borrow().snapshot();
        self.document.dom = dom;
        self.document.document = document;
        Ok(())
    }

    fn drain_dynamic_referrer_policies(&mut self) -> Result<(), HostError> {
        let pending = self.host.take_all_pending_referrer_policies()?;
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
        for (realm, owner) in self.frame_realms.clone() {
            self.host.activate_realm(realm)?;
            let href = self.host.evaluate_string("location.href")?;
            if let Some(source) = href.strip_prefix("javascript:") {
                execute_javascript_url(&mut self.host, source, &owner.committed_url)?;
                continue;
            }
            if href != owner.committed_url
                && Url::parse(&href).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            {
                let opaque = self
                    .host
                    .evaluate_string("String(globalThis.__oryn_opaque_origin===true)")?
                    == "true";
                let initiator_url = (!opaque)
                    .then(|| {
                        self.host
                            .evaluate_string("globalThis.__oryn_request_initiator_url")
                            .ok()
                    })
                    .flatten()
                    .filter(|value| !value.trim().is_empty());
                let referrer_policy = self
                    .host
                    .evaluate_string("globalThis.__oryn_referrer_policy")
                    .ok()
                    .filter(|value| !value.trim().is_empty());
                navigations.push((owner, href, initiator_url, referrer_policy));
            }
        }
        for (owner, href, initiator_url, referrer_policy) in navigations {
            self.host.activate_realm(owner.parent_realm)?;
            let dom = self.host.active_dom().ok_or_else(|| {
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
                },
            );
            self.host.reschedule_dynamic_frame(owner.frame)?;
        }
        self.host.activate_realm(self.top_realm)?;
        Ok(())
    }

    fn drain_default_actions(&mut self) -> Result<(), HostError> {
        use oryn_js::v8_bindings::PendingDefaultAction;

        self.host.activate_realm(self.top_realm)?;
        for _ in 0..1024 {
            let pending = self.host.take_pending_default_actions()?;
            if pending.is_empty() {
                return Ok(());
            }
            for (realm, action) in pending {
                self.host.activate_realm(realm)?;
                match action {
                    PendingDefaultAction::Click(node) => {
                        self.apply_default_navigation(node, SemanticAction::Click, false, realm)?;
                    }
                    PendingDefaultAction::Submit(node) => {
                        self.apply_default_navigation(node, SemanticAction::Submit, true, realm)?;
                    }
                }
            }
        }
        Err(HostError::LimitExceeded {
            resource: "default-action queue".into(),
        })
    }

    fn drain_dynamic_stylesheets(&mut self) -> Result<(), HostError> {
        let processing_instruction_href =
            regex::Regex::new(r#"(?i)href\s*=\s*[\"']([^\"']+)[\"']"#)
                .expect("xml-stylesheet href regex");
        for _ in 0..1024 {
            let pending = self.host.take_all_pending_dynamic_stylesheets()?;
            if pending.is_empty() {
                return Ok(());
            }
            for (realm, stylesheet) in pending {
                self.host.activate_realm(realm)?;
                let active_dom = self.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-stylesheet realm DOM is unavailable".into())
                })?;
                let (name, href, css, element_policy) = {
                    let dom = active_dom.borrow();
                    let record = dom
                        .arena()
                        .get(stylesheet)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if let NodeKind::ProcessingInstruction { target, data } = &record.kind {
                        let href = processing_instruction_href
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
                                .text_content(stylesheet)
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
                    let document_url = self.host.evaluate_string("location.href")?;
                    let policy = element_policy
                        .or_else(|| {
                            self.host
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
                            self.host.execute_with_node(
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
                        let _ = self.host.execute_with_node(
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
            let pending = self.host.take_all_pending_dynamic_scripts()?;
            if pending.is_empty() {
                return Ok(());
            }
            #[cfg(unix)]
            let (completion_sender, completion_receiver) = std::sync::mpsc::channel();
            #[cfg(unix)]
            let mut scheduled = 0usize;
            for (realm, script) in pending {
                self.host.activate_realm(realm)?;
                let active_dom = self.host.active_dom().ok_or_else(|| {
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
                        .host
                        .evaluate_string("globalThis.__oryn_request_initiator_url")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                        .or_else(|| self.host.evaluate_string("location.href").ok());
                    let referrer_policy = active_dom
                        .borrow()
                        .attribute(script, "referrerpolicy")
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                        .filter(|value| !value.trim().is_empty())
                        .or_else(|| {
                            self.host
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
                self.host.activate_realm(realm)?;
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
            let pending = self.host.take_all_pending_dynamic_frames()?;
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
                self.host.activate_realm(scheduled_realm)?;
                let scheduled_dom = self.host.active_dom().ok_or_else(|| {
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
                    let realm = ancestry
                        .iter()
                        .find_map(|node| self.node_realms.get(node).copied())
                        .unwrap_or(scheduled_realm);
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
                // Keep frame wrapper and WindowProxy bookkeeping in the realm
                // that scheduled the insertion. The document realm below is
                // used for URL/security inheritance only.
                let parent_realm = scheduled_realm;
                let replaced_realms = self
                    .frame_realms
                    .iter()
                    .filter_map(|(realm, owner)| {
                        (owner.parent_realm == parent_realm && owner.frame == frame)
                            .then_some(*realm)
                    })
                    .collect::<Vec<_>>();
                self.host.activate_realm(parent_realm)?;
                let parent_dom = self.host.active_dom().ok_or_else(|| {
                    HostError::Execution("dynamic-frame realm DOM is unavailable".into())
                })?;
                self.host.activate_realm(document_realm)?;
                let parent_url = self
                    .host
                    .evaluate_string("location.href")
                    .ok()
                    .and_then(|value| Url::parse(&value).ok())
                    .or_else(|| self.base_url.clone());
                let parent_security_url = self
                    .host
                    .evaluate_string("globalThis.__oryn_request_initiator_url")
                    .ok()
                    .and_then(|value| Url::parse(&value).ok())
                    .or_else(|| parent_url.clone());
                let mut parent_policy = document_referrer_policy.or_else(|| {
                    self.host
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
                self.host.activate_realm(parent_realm)?;
                let (requested, mut srcdoc, opaque_sandbox, element_referrer_policy) = {
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
                    let element_referrer_policy = dom
                        .attribute(frame, "referrerpolicy")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned)
                        .filter(|value| !value.trim().is_empty());
                    (requested, srcdoc, opaque_sandbox, element_referrer_policy)
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
                        }) if destination == "frame" => Some((
                            url,
                            source,
                            document_referrer,
                            referrer_policy,
                            refresh_url,
                            content_type,
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
                let network_cross_origin = parent_url
                    .as_ref()
                    .zip(frame_url.as_ref())
                    .is_some_and(|(parent, child)| parent.origin() != child.origin());
                let cross_origin = opaque_sandbox || network_cross_origin;
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
                let parsed = oryn_dom::xml::parse_by_mime(&source, content_type.as_deref());
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
                let realm = self.host.create_realm_with_dom_root(
                    frame_url
                        .as_ref()
                        .filter(|_| !opaque_sandbox)
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_else(|| "null".into()),
                    1,
                    Some(u64::try_from(self.node_realms.len()).unwrap_or(u64::MAX) + 1),
                    frame_dom.clone(),
                    child_document,
                )?;
                if !cross_origin && Rc::ptr_eq(&parent_dom, &self.live_dom) {
                    for node in subtree_nodes(&frame_dom.borrow(), child_document)? {
                        self.node_realms.insert(node, realm);
                    }
                }
                self.host.activate_realm(realm)?;
                self.host.execute(RUST_RUNTIME_GLOBALS).map_err(|error| {
                    HostError::Execution(format!("dynamic frame runtime bootstrap: {error}"))
                })?;
                self.host
                    .execute(crate::observable::OBSERVABLE_GLOBALS)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.host
                    .install_layout_provider(Rc::new(LiveLayoutProvider {
                        dom: frame_dom.clone(),
                        viewport: oryn_common::v4::ViewportState::default(),
                    }))?;
                self.host.set_module_sources(&self.resources);
                #[cfg(unix)]
                if let Some((_, page_id, _)) = &self.context {
                    self.host
                        .execute(&format!("__oryn_configure_context({}, null)", page_id.0))
                        .map_err(|error| {
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
                    let encoded = serde_json::to_string(&committed_url)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.host
                        .execute(&format!(
                            "Object.assign(globalThis.location,new OrynURL({encoded}))"
                        ))
                        .map_err(|error| {
                            HostError::Execution(format!("dynamic frame location: {error}"))
                        })?;
                }
                let referrer = serde_json::to_string(&document_referrer)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.host
                    .execute(&format!("globalThis.__oryn_document_referrer={referrer}"))?;
                let child_referrer_policy = serde_json::to_string(&child_referrer_policy)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                self.host.execute(&format!(
                    "globalThis.__oryn_referrer_policy={child_referrer_policy}"
                ))?;
                let request_initiator = serde_json::to_string(&if inherited_about_blank {
                    "about:blank".to_owned()
                } else {
                    frame_url.as_ref().map_or_else(String::new, Url::to_string)
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
                self.host.execute(&format!(
                    "globalThis.__oryn_request_initiator_url={request_initiator}"
                ))?;
                if opaque_sandbox {
                    self.host
                        .execute("location.origin='null';globalThis.__oryn_opaque_origin=true")
                        .map_err(|error| {
                            HostError::Execution(format!("dynamic frame opaque origin: {error}"))
                        })?;
                } else if (is_srcdoc || inherited_about_blank)
                    && let Some(frame_origin) = frame_url
                        .as_ref()
                        .map(|url| url.origin().ascii_serialization())
                {
                    let frame_origin = serde_json::to_string(&frame_origin)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.host
                        .execute(&format!(
                            "location.origin={frame_origin};globalThis.__oryn_opaque_origin=false"
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
                    self.host.remove_realm(realm)?;
                    self.node_realms
                        .retain(|_, node_realm| *node_realm != realm);
                }
                if replacing_document {
                    let old_documents = {
                        let dom = parent_dom.borrow();
                        dom.arena()
                            .get(frame)
                            .map_err(|error| HostError::Execution(error.to_string()))?
                            .children
                            .iter()
                            .copied()
                            .filter(|child| {
                                dom.arena()
                                    .get(*child)
                                    .is_ok_and(|record| matches!(record.kind, NodeKind::Document))
                            })
                            .collect::<Vec<_>>()
                    };
                    for document in old_documents {
                        parent_dom
                            .borrow_mut()
                            .remove_child(frame, document)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    }
                }
                if !cross_origin {
                    parent_dom
                        .borrow_mut()
                        .append_child(frame, child_document)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                // Establish parent/top and the frame's contentWindow before child
                // script execution. Frame scripts are allowed to synchronously
                // observe their parent relationship while booting.
                self.host.activate_realm(parent_realm)?;
                self.host.ensure_provisional_frame_window(frame)?;
                if cross_origin {
                    self.host.set_opaque_frame_realm(frame, realm)?;
                } else {
                    self.host.set_frame_realm(frame, child_document, realm)?;
                }
                self.host.activate_realm(realm)?;
                if let Some(source) = javascript_url.as_deref() {
                    execute_javascript_url(&mut self.host, source, &committed_url)?;
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
                                    record.attributes.get("src").cloned(),
                                    record.attributes.get("type").cloned(),
                                    dom.arena().text_content(script)?,
                                    false,
                                    record.attributes.contains_key("nomodule"),
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
                        &mut self.host,
                        &frame_dom,
                        &mut self.resources,
                        script,
                        &mut self.diagnostics,
                        #[cfg(unix)]
                        self.context.as_ref(),
                    )?;
                    self.host.refresh_named_window_properties()?;
                }
                self.host.activate_realm(parent_realm)?;
                self.host.activate_realm(realm)?;
                self.host
                    .execute(DOCUMENT_LIFECYCLE_SCRIPT)
                    .map_err(|error| {
                        HostError::Execution(format!("dynamic frame lifecycle: {error}"))
                    })?;
                self.host.activate_realm(parent_realm)?;
                self.frame_realms.insert(
                    realm,
                    FrameRealm {
                        parent_realm,
                        frame,
                        committed_url,
                    },
                );
                self.host
                    .execute_with_node(
                        frame,
                        "__oryn_dynamic_frame",
                        "__oryn_dynamic_frame.dispatchEvent(new Event('load'))",
                    )
                    .map_err(|error| {
                        HostError::Execution(format!("dynamic frame element load: {error}"))
                    })?;
                if let Some(refresh_url) = refresh_url {
                    self.host.activate_realm(realm)?;
                    let refresh_url = serde_json::to_string(&refresh_url)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.host
                        .execute(&format!("setTimeout(()=>location.href={refresh_url},0)"))?;
                    self.host.activate_realm(parent_realm)?;
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
                        client,
                        *page_id,
                        &resource_name,
                        &source,
                        &mut self.resources,
                    )?;
                }
                self.host
                    .execute_module_script(&resource_name, &source, &self.resources, true)
            } else {
                let previous_policy = self
                    .host
                    .evaluate_string("globalThis.__oryn_referrer_policy")
                    .unwrap_or_default();
                if let Some(policy) = &element_referrer_policy {
                    let policy = serde_json::to_string(policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.host
                        .execute(&format!("globalThis.__oryn_referrer_policy={policy}"))?;
                }
                let result = self.host.execute(&source);
                if element_referrer_policy.is_some() {
                    let previous = serde_json::to_string(&previous_policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    self.host
                        .execute(&format!("globalThis.__oryn_referrer_policy={previous}"))?;
                }
                result
            }
        });
        match result {
            Ok(()) => self.host.execute_with_node(
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
                let _ = self.host.execute_with_node(
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
                .host
                .evaluate_string("globalThis.__oryn_request_initiator_url")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| self.host.evaluate_string("location.href").ok());
            let referrer_policy = self
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
        self.live_dom.borrow().find_by_id(id)
    }

    fn apply_default_navigation(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        submits_form: bool,
        source_realm: oryn_js::realm::RealmId,
    ) -> Result<(), HostError> {
        self.host.activate_realm(source_realm)?;
        let source_dom = self.host.active_dom().ok_or_else(|| {
            HostError::Execution("default-action realm DOM is unavailable".into())
        })?;
        self.host.activate_realm(source_realm)?;
        let (href, target_name, navigation_referrer_policy) = {
            let dom = source_dom.borrow();
            let mut candidate = Some(target);
            let mut result = None;
            while let Some(node) = candidate {
                let record = dom
                    .arena()
                    .get(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let NodeKind::Element { name } = &record.kind {
                    if matches!(name.local.as_ref(), "a" | "area")
                        && action == SemanticAction::Click
                        && let Some(href) = record.attributes.get("href")
                    {
                        result = Some((
                            href.clone(),
                            record.attributes.get("target").cloned().unwrap_or_default(),
                            record
                                .attributes
                                .get("referrerpolicy")
                                .cloned()
                                .or_else(|| {
                                    record
                                        .attributes
                                        .get("rel")
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
                    if name.local.as_ref() == "form"
                        && (action == SemanticAction::Submit || submits_form)
                    {
                        result = Some((
                            record.attributes.get("action").cloned().unwrap_or_default(),
                            record.attributes.get("target").cloned().unwrap_or_default(),
                            None,
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
        if target_name.eq_ignore_ascii_case("_blank") {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &self.context {
                let response = client
                    .request(crate::context_service::ContextServiceRequest::OpenPage {
                        opener_page_id: *page_id,
                        url: Some(resolved),
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                match response {
                    crate::context_service::ContextServiceResponse::PageOpened {
                        page_id: opened_page_id,
                    } => {
                        self.host.execute(&format!(
                            "__oryn_context_trace.push({{kind:'page_created',detail:{{page_id:{},opener_page_id:{}}}}})",
                            opened_page_id.0, page_id.0
                        ))?;
                    }
                    crate::context_service::ContextServiceResponse::PageDenied { reason } => {
                        let reason = serde_json::to_string(&reason)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        self.host.execute(&format!(
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
                            .get("name")
                            .or_else(|| record.attributes.get("id"))
                            .is_some_and(|name| name == &target_name))
                    .then_some(frame)
                })
            };
            if let Some(frame) = target_frame {
                source_dom
                    .borrow_mut()
                    .set_attribute(frame, "src", &resolved)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let Some(policy) = navigation_referrer_policy {
                    source_dom
                        .borrow_mut()
                        .set_attribute(frame, "referrerpolicy", &policy)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                self.host.activate_realm(source_realm)?;
                self.host.reschedule_dynamic_frame(frame)?;
                return Ok(());
            }
        }
        let resolved = serde_json::to_string(&resolved)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.activate_realm(source_realm)?;
        self.host.execute(&format!("location.href={resolved}"))
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

fn execute_javascript_url(
    host: &mut RawV8Host,
    source: &str,
    committed_url: &str,
) -> Result<(), HostError> {
    let committed_url = serde_json::to_string(committed_url)
        .map_err(|error| HostError::Execution(error.to_string()))?;
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
        "{{const __origin=location.origin;Object.assign(location,new OrynURL({committed_url}));location.origin=__origin;{evaluation}if(typeof __html==='string'){{document.open();document.write(__html);document.close()}}}}"
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

#[cfg(test)]
mod tests {
    use super::*;

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
            .live_dom
            .borrow()
            .find_by_id("oryn-wpt-result")
            .and_then(|node| page.live_dom.borrow().arena().text_content(node).ok())
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
    fn script_assignments_shadow_named_window_properties() {
        let mut page = RustExecutableDocument::load(
            "<main id='n00'></main><div id='explicit'></div><i id='transient'></i><script>var n00=document.getElementById('n00');n00.id='changed';n00.setAttribute('kept','yes');Object.defineProperty(globalThis,'explicit',{value:42,writable:true,configurable:true});document.getElementById('transient').remove()</script><script>globalThis.namedAssignments=n00===document.getElementById('changed')&&n00.getAttribute('kept')==='yes'&&explicit===42&&typeof transient==='undefined'</script>",
        ).unwrap();
        assert_eq!(page.evaluate("namedAssignments").unwrap(), "true");
        assert_eq!(page.evaluate("__oryn_refresh_window_named_properties();n00.id='again';__oryn_refresh_window_named_properties();n00===document.getElementById('again')&&explicit===42").unwrap(), "true");
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
                    .get("id")
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
        let started = std::time::Instant::now();
        page.drain_tasks().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(10));
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
            let dom = page.live_dom.borrow();
            dom.arena()
                .iter()
                .find(|(id, node)| {
                    dom.is_connected(*id).unwrap_or(false)
                        && node
                            .attributes
                            .get("onclick")
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
        page.host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(page.diagnostics.is_empty(), "{:?}", page.diagnostics);
        let result = page
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
        page.host
            .execute("document.body.addEventListener('eval-probe',()=>{while(true){}},{once:true})")
            .unwrap();
        let result = page
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
        assert!(page.host.take_unhandled_promise_rejections().is_empty());
        assert!(page.host.take_module_error_reports().is_empty());
        page.host.execute("onerror=()=>false").unwrap();
        page.host
            .execute_module_script(
                "https://example.test/late.js",
                "await new Promise((resolve,reject)=>globalThis.rejectAgain=reject)",
                &BTreeMap::new(),
                true,
            )
            .unwrap();
        page.host.execute("rejectAgain(payload)").unwrap();
        assert_eq!(page.host.take_module_error_reports().len(), 1);
        assert!(page.host.take_unhandled_promise_rejections().is_empty());
    }

    #[test]
    fn module_error_native_api_propagation_and_shared_dependencies_are_preserved() {
        let mut page = RustExecutableDocument::load(
            "<script>globalThis.errors=0;onerror=()=>{errors++;return true}</script>",
        )
        .unwrap();
        assert!(
            page.host
                .execute_module(
                    "https://example.test/syntax.js",
                    "let = ;",
                    &BTreeMap::new()
                )
                .is_err()
        );
        assert!(
            page.host
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
            page.host
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
        page.host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(matches!(
            page.host.execute_module_script(
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
        page.host.execute("onerror=()=>{while(true){}}").unwrap();
        page.host
            .set_execution_wall_limit(std::time::Duration::from_millis(50));
        assert!(matches!(
            page.host
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
                "https://source.example.test/source".into(),
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
        page.host.activate_realm(old_realm).unwrap();
        assert_eq!(
            page.host
                .evaluate_string("document.querySelector('p').textContent")
                .unwrap(),
            "old"
        );
        page.host.activate_realm(RealmId(0)).unwrap();
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
            const child='<script>onmessage=()=>parent.postMessage(location.origin,"*")<\/script>';
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
    fn blob_url_worker_executes_registered_rust_owned_bytes() {
        let mut page = RustExecutableDocument::load_with_environment(
            r#"<output id=result>pending</output><script>
            const output=document.querySelector('#result');
            const dedicatedUrl=URL.createObjectURL(new Blob(['postMessage({value:"dedicated"})'],{type:'application/javascript'}));
            const dedicated=new Worker(dedicatedUrl);dedicated.onmessage=event=>output.textContent=event.data.value;
            </script>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        let value = page
            .evaluate("__oryn_drain(true);document.querySelector('#result').textContent+'|'+dedicated.__closed+'|'+JSON.stringify(__oryn_take_context_trace())")
            .unwrap();
        assert!(value.starts_with("dedicated|false|"), "{value}");
    }
}
