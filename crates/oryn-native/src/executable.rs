use std::collections::BTreeMap;

use markup5ever::{LocalName, QualName, ns};
use oryn_common::v3::{CapabilityDiagnostic, ExecutionDomain, SupportLevel};
use oryn_common::v3::{PageId, SemanticAction};
use serde::{Deserialize, Serialize};

use crate::{
    dom::{DomArena, NodeId, NodeKind},
    host::{HostError, JavaScriptHost, RawV8Host},
    html::{ParsedDocument, parse},
    network::SharedNetworkBroker,
};
use url::Url;

/// A thread-affine executable native document with a deliberately narrow DOM
/// compatibility layer for controlled Oryn fixtures.
pub struct ExecutableDocument {
    pub document: ParsedDocument,
    pub diagnostics: Vec<CapabilityDiagnostic>,
    host: RawV8Host,
    // Same-origin frames own independent V8 contexts. The top-level host keeps
    // a semantic snapshot for traversal, while these realms remain alive for
    // the lifetime of the page.
    _frame_documents: FrameDocuments,
}

/// V8 isolates on one thread must leave in strict reverse creation order.
/// `Vec` does not promise the reverse element drop order that nested frame
/// realms require, so explicitly retire the newest frame first.
struct FrameDocument {
    instance: String,
    source: String,
    document: ExecutableDocument,
}

struct FrameDocuments(Vec<FrameDocument>);

impl Drop for FrameDocuments {
    fn drop(&mut self) {
        while let Some(frame) = self.0.pop() {
            drop(frame);
        }
    }
}

#[derive(Serialize)]
struct FrameSnapshot {
    source: String,
    document: JsNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextTraceEntry {
    pub kind: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl ExecutableDocument {
    pub fn load(html: &str) -> Result<Self, HostError> {
        Self::load_with_resources(html, &BTreeMap::new())
    }

    pub fn load_with_resources(
        html: &str,
        resources: &BTreeMap<String, String>,
    ) -> Result<Self, HostError> {
        Self::load_with_environment(html, resources, None, None)
    }

    pub fn load_with_environment(
        html: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
    ) -> Result<Self, HostError> {
        Self::load_with_limits(html, resources, broker, base_url, 256 * 1024 * 1024)
    }

    pub fn load_with_limits(
        html: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
    ) -> Result<Self, HostError> {
        Self::load_with_context_options(
            html,
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
        html: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        context: (
            crate::context_service::SharedContextClient,
            PageId,
            Option<PageId>,
        ),
    ) -> Result<Self, HostError> {
        Self::load_with_context_options(
            html,
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    fn load_with_context_options(
        html: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        #[cfg(unix)] context: Option<(
            crate::context_service::SharedContextClient,
            PageId,
            Option<PageId>,
        )>,
    ) -> Result<Self, HostError> {
        Self::load_with_context_options_at_depth(
            html,
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            context,
            0,
        )
    }

    fn load_with_context_options_at_depth(
        html: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        #[cfg(unix)] context: Option<(
            crate::context_service::SharedContextClient,
            PageId,
            Option<PageId>,
        )>,
        frame_depth: usize,
    ) -> Result<Self, HostError> {
        let mut document = parse(html);
        let frame_nodes = document
            .dom
            .iter()
            .filter_map(|(id, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "iframe")
                    .then_some(id)
            })
            .collect::<Vec<_>>();
        let mut frame_sources = Vec::new();
        for (index, id) in frame_nodes.into_iter().enumerate() {
            let instance = format!("frame-{frame_depth}-{index}");
            let node = document
                .dom
                .get_mut(id)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            node.attributes
                .insert("data-oryn-frame-instance".into(), instance.clone());
            if let Some(source) = node.attributes.get("src").cloned() {
                frame_sources.push((instance, source));
            }
        }
        let initial = serde_json::to_string(&JsNode::from_dom(&document.dom, document.document)?)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        #[cfg(unix)]
        let context_identity = context
            .as_ref()
            .map(|(_, page_id, opener_page_id)| (*page_id, *opener_page_id));
        // Build child realms before entering the top-level isolate. Raw V8
        // hosts are thread-affine, and no isolate scope may be interleaved
        // while another realm is being initialized.
        let mut frame_documents = Vec::new();
        let mut frame_snapshots = BTreeMap::<String, FrameSnapshot>::new();
        let mut frame_diagnostics = Vec::new();
        if frame_depth < 4 {
            for (instance, source) in frame_sources {
                let Some(frame_html) = resources.get(&source) else {
                    continue;
                };
                let frame_url = base_url
                    .as_ref()
                    .and_then(|base| base.join(&source).ok())
                    .or_else(|| Url::parse(&source).ok());
                let frame = Self::load_with_context_options_at_depth(
                    frame_html,
                    resources,
                    broker.clone(),
                    frame_url,
                    max_heap_bytes,
                    #[cfg(unix)]
                    context.clone(),
                    frame_depth + 1,
                )?;
                let mut snapshot = JsNode::from_dom(&frame.document.dom, frame.document.document)?;
                snapshot.mark_frame_nodes(&instance);
                snapshot.strip_ids();
                let frame_source = source.clone();
                frame_snapshots.insert(
                    instance.clone(),
                    FrameSnapshot {
                        source,
                        document: snapshot,
                    },
                );
                frame_diagnostics.extend(frame.diagnostics.clone());
                frame_documents.push(FrameDocument {
                    instance,
                    source: frame_source,
                    document: frame,
                });
            }
        }
        #[cfg(unix)]
        let mut host = match context.clone() {
            Some((context_client, page_id, _)) => {
                RawV8Host::new_with_network_context_and_heap_limit(
                    broker.clone(),
                    base_url.clone(),
                    context_client,
                    page_id,
                    max_heap_bytes,
                )
            }
            None => match (broker.clone(), base_url.clone()) {
                (Some(broker), Some(base_url)) => {
                    RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
                }
                _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
            },
        };
        #[cfg(not(unix))]
        let mut host = match (broker.clone(), base_url.clone()) {
            (Some(broker), Some(base_url)) => {
                RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
            }
            _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
        };
        host.execute(DOM_BOOTSTRAP)?;
        host.set_module_sources(resources);
        host.execute(&format!("__oryn_install({initial})"))?;
        if let Some(base_url) = &base_url {
            host.execute(&format!(
                "__oryn_set_location({})",
                json_string(base_url.as_str())?
            ))?;
        }
        #[cfg(unix)]
        if let Some((page_id, opener_page_id)) = context_identity {
            if frame_depth == 0 {
                host.execute(&format!(
                    "__oryn_configure_context({}, {})",
                    page_id.0,
                    opener_page_id
                        .map(|page_id| page_id.0.to_string())
                        .unwrap_or_else(|| "null".into())
                ))?;
            } else {
                host.execute(&format!("__oryn_configure_frame_context({})", page_id.0))?;
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
        let local_storage = json_string(local_storage)?;
        let session_storage = json_string(session_storage)?;
        host.execute(&format!(
            "__oryn_restore_storage(JSON.parse({local_storage}),JSON.parse({session_storage}))"
        ))?;
        let frame_resources = serde_json::to_string(&frame_snapshots)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        host.execute(&format!(
            "__oryn_install_frame_documents({frame_resources})"
        ))?;

        let mut diagnostics = frame_diagnostics;
        for (_, node) in document.dom.iter() {
            if matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "iframe")
                && node
                    .attributes
                    .get("src")
                    .is_some_and(|src| !resources.contains_key(src))
            {
                diagnostics.push(unsupported("html.frame.cross_origin"));
            }
        }
        let mut scripts = document
            .dom
            .iter()
            .filter_map(|(id, node)| {
                let NodeKind::Element { name } = &node.kind else {
                    return None;
                };
                (name.local.as_ref() == "script").then(|| {
                    (
                        id,
                        node.attributes.clone(),
                        document.dom.text_content(id).unwrap_or_default(),
                    )
                })
            })
            .collect::<Vec<_>>();
        // Browser modules are deferred until parsing and parser-blocking classic
        // scripts complete. Keep relative order within each class.
        scripts.sort_by_key(|(_, attributes, _)| {
            attributes.get("type").map(String::as_str) == Some("module")
        });
        for (script_id, attributes, source) in scripts {
            let script_id = serde_json::to_string(&script_id)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            host.execute(&format!("document.currentScript=__oryn_get({script_id})"))?;
            if attributes.get("type").map(String::as_str) == Some("module") {
                let resource_name = attributes
                    .get("src")
                    .map(String::as_str)
                    .unwrap_or("oryn:inline-module");
                let module_source = attributes
                    .get("src")
                    .and_then(|src| resources.get(src))
                    .map(String::as_str)
                    .unwrap_or(source.as_str());
                capture_script_result(
                    host.execute_module(resource_name, module_source, resources),
                    "module",
                    &mut diagnostics,
                )?;
                host.execute("document.currentScript=null")?;
                continue;
            }
            if let Some(script_type) = attributes.get("type")
                && !matches!(
                    script_type.as_str(),
                    "" | "text/javascript" | "application/javascript" | "text/babel"
                )
            {
                if !matches!(
                    script_type.as_str(),
                    "importmap" | "application/importmap+json" | "application/json"
                ) {
                    diagnostics.push(unsupported(&format!("html.script.type.{script_type}")));
                }
                host.execute("document.currentScript=null")?;
                continue;
            }
            if attributes.get("type").map(String::as_str) == Some("text/babel") {
                let source = json_string(&source)?;
                match host.evaluate_string(&format!(
                    "Babel.transform({source},{{presets:['react']}}).code"
                )) {
                    Ok(transformed) => {
                        capture_script_result(
                            host.execute(&transformed),
                            "babel",
                            &mut diagnostics,
                        )?;
                    }
                    Err(error) => {
                        capture_script_result(Err(error), "babel", &mut diagnostics)?;
                    }
                }
            } else if let Some(src) = attributes.get("src") {
                match resources.get(src) {
                    Some(source) => {
                        capture_script_result(
                            host.execute(source),
                            "classic_external",
                            &mut diagnostics,
                        )?;
                    }
                    None => diagnostics.push(unsupported("html.script.external.unavailable")),
                }
            } else {
                capture_script_result(host.execute(&source), "classic_inline", &mut diagnostics)?;
            }
            host.execute("document.currentScript=null")?;
        }
        host.execute("__oryn_install_inline_handlers()")?;
        host.execute("document.__dispatchReady()")?;
        host.execute("__oryn_dispatch_load()")?;
        drain_host(&mut host)?;
        host.execute("if(globalThis.ReactDOM&&ReactDOM.flushSync)ReactDOM.flushSync(()=>{})")?;
        drain_host(&mut host)?;
        for rejection in host.take_unhandled_promise_rejections() {
            diagnostics.push(script_failure(
                "promise",
                &HostError::Execution(format!("unhandled promise rejection: {rejection}")),
            ));
        }
        for failure in host.take_module_resolution_failures() {
            diagnostics.push(script_failure(
                "module_resolution",
                &HostError::Execution(failure),
            ));
        }

        let mut executable = Self {
            document,
            diagnostics,
            host,
            _frame_documents: FrameDocuments(frame_documents),
        };
        executable.sync_dom()?;
        Ok(executable)
    }

    pub fn execute(&mut self, source: &str) -> Result<(), HostError> {
        self.host.execute(source)?;
        self.sync_dom()
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, HostError> {
        let value = self.host.evaluate_string(source)?;
        self.sync_dom()?;
        Ok(value)
    }

    pub fn location(&mut self) -> Result<String, HostError> {
        self.host.evaluate_string("location.href")
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

    pub fn click(&mut self, selector: &str) -> Result<(), HostError> {
        let selector = json_string(selector)?;
        self.host.execute(&format!(
            "{{const node=document.querySelector({selector});if(!node)throw new Error('no matching element');node.click();}}"
        ))?;
        drain_host(&mut self.host)?;
        self.sync_dom()
    }

    pub fn type_text(&mut self, selector: &str, value: &str) -> Result<(), HostError> {
        let selector = json_string(selector)?;
        let value = json_string(value)?;
        self.host.execute(&format!(
            "{{const node=document.querySelector({selector});if(!node)throw new Error('no matching element');node.value={value};node.dispatchEvent(new Event('input',{{bubbles:true}}));node.dispatchEvent(new Event('change',{{bubbles:true}}));}}"
        ))?;
        drain_host(&mut self.host)?;
        self.sync_dom()
    }

    pub fn drain_tasks(&mut self) -> Result<(), HostError> {
        self.host.execute("__oryn_drain(true)")?;
        self.sync_frame_documents()?;
        self.sync_dom()
    }

    pub fn apply_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        let frame_target = self.document.dom.get(target).ok().and_then(|node| {
            Some((
                node.attributes.get("data-oryn-frame-owner")?.clone(),
                NodeId {
                    slot: node.attributes.get("data-oryn-frame-slot")?.parse().ok()?,
                    generation: node
                        .attributes
                        .get("data-oryn-frame-generation")?
                        .parse()
                        .ok()?,
                },
            ))
        });
        if let Some((instance, child_target)) = frame_target {
            let frame = self
                ._frame_documents
                .0
                .iter_mut()
                .find(|frame| frame.instance == instance)
                .ok_or_else(|| HostError::Execution("frame realm is unavailable".into()))?;
            frame.document.host.enter_realm();
            let action_result = frame.document.apply_action(child_target, action, value);
            frame.document.host.exit_realm();
            action_result?;
            self.sync_frame_documents()?;
            return self.sync_dom();
        }
        let target = serde_json::to_string(&target)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let value = json_string(value.unwrap_or_default())?;
        let operation = match action {
            SemanticAction::Click => "node.click()".into(),
            SemanticAction::Type => format!("node.value={value};node.dispatchEvent(new Event('input',{{bubbles:true}}));node.dispatchEvent(new Event('change',{{bubbles:true}}))"),
            SemanticAction::Clear => "node.value='';node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Check => "node.checked=true;node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Uncheck => "node.checked=false;node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Select => format!(
                "{{const requested={value};const option=[...(node.options||[])].find(item=>item.value===requested||item.textContent.trim()===requested);if(!option)throw new Error('no matching option');node.value=option.value;node.dispatchEvent(new Event('change',{{bubbles:true}}));}}"
            ),
            SemanticAction::Focus => "node.focus()".into(),
            SemanticAction::Hover => "node.dispatchEvent(new Event('mouseover',{bubbles:true}))".into(),
            SemanticAction::Submit => "node.dispatchEvent(new Event('submit',{bubbles:true}))".into(),
            _ => return Err(HostError::Execution(format!("unsupported native executable action {action:?}"))),
        };
        self.host.execute(&format!("{{const node=__oryn_get({target});if(!node)throw new Error('stale node');{operation};}}"))?;
        drain_host(&mut self.host)?;
        self.sync_dom()
    }

    fn sync_frame_documents(&mut self) -> Result<(), HostError> {
        let mut updates = Vec::with_capacity(self._frame_documents.0.len());
        for frame in &mut self._frame_documents.0 {
            frame.document.host.enter_realm();
            let drain_result = frame.document.drain_tasks();
            frame.document.host.exit_realm();
            drain_result?;
            let mut snapshot = JsNode::from_dom(
                &frame.document.document.dom,
                frame.document.document.document,
            )?;
            snapshot.mark_frame_nodes(&frame.instance);
            snapshot.strip_ids();
            updates.push((
                frame.instance.clone(),
                FrameSnapshot {
                    source: frame.source.clone(),
                    document: snapshot,
                },
            ));
        }
        for (instance, snapshot) in updates {
            self.host.execute(&format!(
                "__oryn_update_frame_document({}, {})",
                json_string(&instance)?,
                serde_json::to_string(&snapshot)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            ))?;
        }
        Ok(())
    }

    fn sync_dom(&mut self) -> Result<(), HostError> {
        let exported = self
            .host
            .evaluate_string("JSON.stringify(__oryn_export())")?;
        let root: JsNode = serde_json::from_str(&exported)
            .map_err(|error| HostError::Execution(format!("invalid DOM export: {error}")))?;
        let (dom, document) = root.into_dom(&self.document.dom)?;
        self.document.dom = dom;
        self.document.document = document;
        Ok(())
    }
}

fn drain_host(host: &mut RawV8Host) -> Result<(), HostError> {
    for _ in 0..4 {
        host.execute("__oryn_drain(false)")?;
    }
    Ok(())
}

fn json_string(value: &str) -> Result<String, HostError> {
    serde_json::to_string(value).map_err(|error| HostError::Execution(error.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsNode {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    attrs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<JsNode>,
}

impl JsNode {
    fn from_dom(dom: &DomArena, id: NodeId) -> Result<Self, HostError> {
        let node = dom
            .get(id)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let (kind, tag, data) = match &node.kind {
            NodeKind::Document => ("document", None, None),
            NodeKind::DocumentType { name } => ("doctype", None, Some(name.clone())),
            NodeKind::Element { name } => ("element", Some(name.local.to_string()), None),
            NodeKind::Text { data } => ("text", None, Some(data.clone())),
            NodeKind::Comment { data } => ("comment", None, Some(data.clone())),
            NodeKind::DocumentFragment => ("fragment", None, None),
        };
        let children = node
            .children
            .iter()
            .map(|child| Self::from_dom(dom, *child))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            kind: kind.into(),
            id: Some(id),
            tag,
            data,
            attrs: node.attributes.clone(),
            children,
        })
    }

    fn into_dom(self, previous: &DomArena) -> Result<(DomArena, NodeId), HostError> {
        let mut retained = Vec::new();
        self.collect_ids(&mut retained);
        let mut dom = DomArena::prepare_rebuild(previous, &retained);
        let root = self.insert_into(&mut dom)?;
        Ok((dom, root))
    }

    fn collect_ids(&self, retained: &mut Vec<NodeId>) {
        if let Some(id) = self.id {
            retained.push(id);
        }
        for child in &self.children {
            child.collect_ids(retained);
        }
    }

    fn strip_ids(&mut self) {
        self.id = None;
        for child in &mut self.children {
            child.strip_ids();
        }
    }

    fn mark_frame_nodes(&mut self, instance: &str) {
        if self.kind == "element"
            && let Some(id) = self.id
        {
            self.attrs
                .insert("data-oryn-frame-owner".into(), instance.into());
            self.attrs
                .insert("data-oryn-frame-slot".into(), id.slot.to_string());
            self.attrs.insert(
                "data-oryn-frame-generation".into(),
                id.generation.to_string(),
            );
        }
        for child in &mut self.children {
            child.mark_frame_nodes(instance);
        }
    }

    fn insert_into(self, dom: &mut DomArena) -> Result<NodeId, HostError> {
        let kind = match self.kind.as_str() {
            "document" => NodeKind::Document,
            "doctype" => NodeKind::DocumentType {
                name: self.data.unwrap_or_else(|| "html".into()),
            },
            "element" => NodeKind::Element {
                name: QualName::new(
                    None,
                    ns!(html),
                    LocalName::from(self.tag.as_deref().unwrap_or("div")),
                ),
            },
            "text" => NodeKind::Text {
                data: self.data.unwrap_or_default(),
            },
            "comment" => NodeKind::Comment {
                data: self.data.unwrap_or_default(),
            },
            "fragment" => NodeKind::DocumentFragment,
            other => {
                return Err(HostError::Execution(format!(
                    "unknown exported node kind {other}"
                )));
            }
        };
        let id = if let Some(id) = self.id {
            dom.restore(id, kind)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            id
        } else {
            dom.insert(kind)
        };
        dom.get_mut(id)
            .map_err(|error| HostError::Execution(error.to_string()))?
            .attributes = self.attrs;
        for child in self.children {
            let child = child.insert_into(dom)?;
            dom.append_child(id, child)
                .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        Ok(id)
    }
}

fn unsupported(capability: &str) -> CapabilityDiagnostic {
    CapabilityDiagnostic {
        capability: capability.into(),
        support: SupportLevel::Unsupported,
        alternatives: vec![
            ExecutionDomain::Chromium,
            ExecutionDomain::Webkit,
            ExecutionDomain::UserBrowser,
        ],
        handoff_lossy: true,
        detail: Some("native script loader does not implement this script class yet".into()),
    }
}

fn script_failure(script_class: &str, error: &HostError) -> CapabilityDiagnostic {
    CapabilityDiagnostic {
        capability: format!("html.script.{script_class}.execution"),
        support: SupportLevel::Partial,
        alternatives: vec![
            ExecutionDomain::Chromium,
            ExecutionDomain::Webkit,
            ExecutionDomain::UserBrowser,
        ],
        handoff_lossy: true,
        detail: Some(format!(
            "page-authored script failed without aborting navigation: {error}"
        )),
    }
}

fn capture_script_result(
    result: Result<(), HostError>,
    script_class: &str,
    diagnostics: &mut Vec<CapabilityDiagnostic>,
) -> Result<(), HostError> {
    match result {
        Ok(()) => Ok(()),
        Err(error @ HostError::LimitExceeded { .. }) => Err(error),
        Err(error) => {
            diagnostics.push(script_failure(script_class, &error));
            Ok(())
        }
    }
}

const DOM_BOOTSTRAP: &str = r#"
(() => {
  let nextNodeSlot=0;
  class OrynEvent {
    constructor(type, init = {}) { this.type=String(type); this.bubbles=!!init.bubbles; this.cancelable=!!init.cancelable;this.composed=!!init.composed;this.detail=init.detail;this.defaultPrevented=false;this.cancelBubble=false;this.__immediateStopped=false; this.target=null; this.currentTarget=null;this.eventPhase=0;this.timeStamp=0;this.isTrusted=false; }
    preventDefault() { if(this.cancelable)this.defaultPrevented=true; }
    stopPropagation() { this.cancelBubble=true; }
    stopImmediatePropagation() { this.__immediateStopped=true;this.cancelBubble=true; }
    composedPath() { const path=[];for(let node=this.target;node;node=node.parentNode)path.push(node);path.push(globalThis);return path; }
  }
  class OrynStyleDeclaration {
    constructor(node) { this.__node=node; }
    __values() { return Object.fromEntries((this.__node.getAttribute('style')||'').split(';').map(item=>item.split(':')).filter(item=>item.length===2).map(([key,value])=>[key.trim(),value.trim()])); }
    __write(values) { this.__node.setAttribute('style',Object.entries(values).map(([name,value])=>name+': '+value).join('; ')); }
    getPropertyValue(name) { return this.__values()[String(name)]||''; }
    setProperty(name,value,_priority='') { const values=this.__values();values[String(name)]=String(value);this.__write(values); }
    removeProperty(name) { const values=this.__values();const previous=values[String(name)]||'';delete values[String(name)];this.__write(values);return previous; }
    get cssText() { return this.__node.getAttribute('style')||''; }
    set cssText(value) { this.__node.setAttribute('style',String(value)); }
  }
  class OrynNode {
    constructor(raw={kind:'element',tag:'unknown'}, allocate=false) {
      this.__kind=raw.kind; this.__id=raw.id||(allocate?{slot:nextNodeSlot++,generation:0}:null); this.__tag=raw.tag||null; this.__data=raw.data||''; this.__attrs={...(raw.attrs||{})};
      this.childNodes=[]; this.parentNode=null; this.__listeners=new Map(); this.__shadow=null; this.__shadowMode=null;this.__style=null;
      for (const child of raw.children||[]) this.appendChild(new OrynNode(child));
    }
    appendChild(node) { if(node.parentNode)node.remove(); node.parentNode=this; this.childNodes.push(node); if(node.connectedCallback&&!node.__connected){node.__connected=true;node.connectedCallback();} notifyMutation(this); return node; }
    removeChild(node) { const index=this.childNodes.indexOf(node);if(index<0)throw new Error('node is not a child');this.childNodes.splice(index,1);node.parentNode=null;notifyMutation(this);return node; }
    insertBefore(node,reference) { if(reference===null)return this.appendChild(node);const index=this.childNodes.indexOf(reference);if(index<0)throw new Error('reference is not a child');if(node.parentNode)node.remove();node.parentNode=this;this.childNodes.splice(index,0,node);if(node.connectedCallback&&!node.__connected){node.__connected=true;node.connectedCallback();}notifyMutation(this);return node; }
    replaceChild(node,old) { this.insertBefore(node,old);this.removeChild(old);return old; }
    append(...nodes) { for(const node of nodes)this.appendChild(typeof node==='string'?new OrynNode({kind:'text',data:node},true):node); }
    prepend(...nodes) { for(const node of [...nodes].reverse())this.insertBefore(typeof node==='string'?new OrynNode({kind:'text',data:node},true):node,this.firstChild); }
    replaceChildren(...nodes) { for(const child of [...this.childNodes])this.removeChild(child);this.append(...nodes); }
    before(...nodes) { if(!this.parentNode)return;for(const node of nodes)this.parentNode.insertBefore(typeof node==='string'?new OrynNode({kind:'text',data:node},true):node,this); }
    after(...nodes) { if(!this.parentNode)return;const reference=this.nextSibling;for(const node of nodes)this.parentNode.insertBefore(typeof node==='string'?new OrynNode({kind:'text',data:node},true):node,reference); }
    replaceWith(...nodes) { const parent=this.parentNode;if(!parent)return;this.before(...nodes);parent.removeChild(this); }
    remove() { if(!this.parentNode)return; const parent=this.parentNode; parent.childNodes=parent.childNodes.filter(node=>node!==this); this.parentNode=null; notifyMutation(parent); }
    addEventListener(type, callback, options=false) { if(!callback)return;type=String(type);const capture=options===true||!!options?.capture;const list=this.__listeners.get(type)||[];if(!list.some(item=>item.callback===callback&&item.capture===capture))list.push({callback,capture,once:!!options?.once});this.__listeners.set(type,list);if(type==='click'&&this.__kind==='element')this.__attrs['data-oryn-click-listener']=''; }
    removeEventListener(type, callback, options=false) { const capture=options===true||!!options?.capture;this.__listeners.set(type,(this.__listeners.get(type)||[]).filter(item=>item.callback!==callback||item.capture!==capture)); }
    dispatchEvent(event) { if(!(event instanceof OrynEvent))event=new OrynEvent(event.type||event); if(!event.target)event.target=this; event.currentTarget=this;event.eventPhase=this===event.target?2:3;const list=this.__listeners.get(event.type)||[];for(const item of [...list]){const fn=item.callback;if(typeof fn==='function')fn.call(this,event);else if(fn&&typeof fn.handleEvent==='function')fn.handleEvent(event);if(item.once)this.removeEventListener(event.type,item.callback,{capture:item.capture});if(event.__immediateStopped)break;} const handler=this['on'+event.type];if(typeof handler==='function'&&!event.__immediateStopped)handler.call(this,event); if(event.bubbles&&!event.cancelBubble&&this.parentNode)this.parentNode.dispatchEvent(event); return !event.defaultPrevented; }
    click() { if(this.disabled)return;if(['input','textarea','select','button','a'].includes(this.__tag))this.focus();const allowed=this.dispatchEvent(new OrynEvent('click',{bubbles:true,cancelable:true}));if(!allowed)return;if(this.__tag==='label'){const control=descendants(this).find(node=>node.__tag==='input')||(this.getAttribute('for')&&document.getElementById(this.getAttribute('for')));if(control)control.click();return;}if(this.__tag==='input'){const type=(this.getAttribute('type')||'text').toLowerCase();if(type==='checkbox')this.checked=!this.checked;if(type==='radio'){const name=this.getAttribute('name');for(const node of descendants(document))if(node!==this&&node.__tag==='input'&&(node.getAttribute('type')||'').toLowerCase()==='radio'&&node.getAttribute('name')===name)node.checked=false;this.checked=true;}if(type==='checkbox'||type==='radio')this.dispatchEvent(new OrynEvent('change',{bubbles:true}));if(type==='submit'){const form=this.closest('form');if(form)submitForm(form);}}if(this.__tag==='button'&&(this.getAttribute('type')||'submit')==='submit'){const form=this.closest('form');if(form)submitForm(form);}if(this.__tag==='a'&&this.getAttribute('href'))navigateTarget(this.getAttribute('href'),this.getAttribute('target')||'_self'); }
    focus() { const active=document&&document.activeElement;if(active&&active!==this&&active!==document.body)active.blur();this.setAttribute('data-oryn-focused','');this.dispatchEvent(new OrynEvent('focus')); }
    blur() { if(!this.hasAttribute('data-oryn-focused'))return;this.removeAttribute('data-oryn-focused');this.dispatchEvent(new OrynEvent('blur')); }
    setAttribute(name,value) { name=String(name);value=String(value);const old=this.getAttribute(name);this.__attrs[name]=value;if(/^on[a-z]+$/i.test(name)){try{this[name.toLowerCase()]=new Function('event',value);}catch(error){console.error(error);}}if(old!==value&&this.attributeChangedCallback&&(this.constructor.observedAttributes||[]).includes(name))queueMicrotask(()=>this.attributeChangedCallback(name,old,value));notifyMutation(this,{type:'attributes',attributeName:name,oldValue:old}); }
    setAttributeNS(_namespace,name,value) { this.setAttribute(name,value); }
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(this.__attrs,name)?this.__attrs[name]:null; }
    getAttributeNS(_namespace,name) { return this.getAttribute(name); }
    hasAttribute(name) { return Object.prototype.hasOwnProperty.call(this.__attrs,name); }
    toggleAttribute(name,force) { const present=this.hasAttribute(name),next=force===undefined?!present:!!force;if(next)this.setAttribute(name,'');else this.removeAttribute(name);return next; }
    getAttributeNames() { return Object.keys(this.__attrs); }
    get attributes() { const node=this;const entries=Object.entries(this.__attrs).map(([name,value])=>({name,nodeName:name,value,nodeValue:value,specified:true,ownerElement:node,expando:false}));const named=Object.fromEntries(entries.map(attribute=>[attribute.name,attribute]));return Object.assign(entries,{item:index=>entries[index]||null,getNamedItem:name=>named[String(name)]||null,...named}); }
    removeAttribute(name) { const old=this.getAttribute(name);delete this.__attrs[name];if(old!==null&&this.attributeChangedCallback&&(this.constructor.observedAttributes||[]).includes(String(name)))queueMicrotask(()=>this.attributeChangedCallback(String(name),old,null));notifyMutation(this,{type:'attributes',attributeName:String(name),oldValue:old}); }
    removeAttributeNS(_namespace,name) { this.removeAttribute(name); }
    querySelector(selector) { return query(this,selector,false)[0]||null; }
    querySelectorAll(selector) { return query(this,selector,false); }
    getElementById(id) { return this.querySelector('#'+id); }
    closest(selector) { let node=this;while(node){if(matches(node,selector))return node;node=node.parentNode;}return null; }
    reset() { for(const node of descendants(this)){if(['input','textarea','select'].includes(node.__tag)){node.value='';node.checked=false;}}this.dispatchEvent(new OrynEvent('reset',{bubbles:true})); }
    checkValidity() { if(this.__tag==='form')return descendants(this).filter(node=>['input','textarea','select'].includes(node.__tag)).every(node=>node.checkValidity());if(!['input','textarea','select'].includes(this.__tag)||this.disabled)return true;const value=this.value;this.validationMessage='';if(this.hasAttribute('required')&&!value&&!this.checked)this.validationMessage='Please fill out this field.';else if(this.getAttribute('type')==='email'&&value&&!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(value))this.validationMessage='Please enter an email address.';else if(this.hasAttribute('minlength')&&value.length<Number(this.getAttribute('minlength')))this.validationMessage='Value is too short.';return !this.validationMessage; }
    reportValidity() { if(this.__tag==='form'){let valid=true;for(const node of descendants(this).filter(node=>['input','textarea','select'].includes(node.__tag)))if(!node.reportValidity())valid=false;return valid;}const valid=this.checkValidity();if(!valid)this.dispatchEvent(new OrynEvent('invalid',{cancelable:true}));return valid; }
    submit() { if(this.__tag!=='form')throw new TypeError('submit called on non-form');navigateTarget(this.getAttribute('action')||location.href,this.getAttribute('target')||'_self'); }
    requestSubmit(submitter=null) { if(this.__tag!=='form')throw new TypeError('requestSubmit called on non-form');if(this.hasAttribute('novalidate')||this.reportValidity()){const event=new OrynEvent('submit',{bubbles:true,cancelable:true});event.submitter=submitter;if(this.dispatchEvent(event))this.submit();} }
    get textContent() { return this.__kind==='text'||this.__kind==='comment'?this.__data:this.childNodes.map(node=>node.textContent).join(''); }
    set textContent(value) { if(this.__kind==='text'||this.__kind==='comment')this.__data=String(value); else { const string=String(value??'');this.childNodes=string?[new OrynNode({kind:'text',data:string},true)]:[];for(const child of this.childNodes)child.parentNode=this; } notifyMutation(this); }
    get innerText() { return this.textContent; }
    set innerText(value) { this.textContent=value; }
    get innerHTML() { const children=this.__tag==='template'&&this.content?this.content.childNodes:this.childNodes;return children.map(serializeNode).join(''); }
    set innerHTML(value) { const target=this.__tag==='template'&&this.content?this.content:this;target.childNodes=parseFragment(String(value));for(const child of target.childNodes)child.parentNode=target;upgradeTree(target); }
    get tagName() { return this.__tag?this.__tag.toUpperCase():undefined; }
    get nodeType() { return this.__kind==='element'?1:this.__kind==='text'?3:this.__kind==='comment'?8:this.__kind==='document'?9:11; }
    get nodeName() { return this.__kind==='text'?'#text':this.__kind==='comment'?'#comment':this.__kind==='document'?'#document':this.tagName||'#document-fragment'; }
    get nodeValue() { return this.__kind==='text'||this.__kind==='comment'?this.__data:null; } set nodeValue(value) { if(this.__kind==='text'||this.__kind==='comment'){this.__data=String(value??'');notifyMutation(this);} }
    get data() { return this.__data; } set data(value) { this.nodeValue=value; }
    get ownerDocument() { return this.__kind==='document'?null:globalThis.document||null; }
    get firstChild() { return this.childNodes[0]||null; }
    get lastChild() { return this.childNodes[this.childNodes.length-1]||null; }
    get nextSibling() { if(!this.parentNode)return null;const index=this.parentNode.childNodes.indexOf(this);return this.parentNode.childNodes[index+1]||null; }
    get previousSibling() { if(!this.parentNode)return null;const index=this.parentNode.childNodes.indexOf(this);return index>0?this.parentNode.childNodes[index-1]:null; }
    get children() { return this.childNodes.filter(node=>node.__kind==='element'); }
    get childElementCount() { return this.children.length; }
    hasChildNodes() { return this.childNodes.length>0; }
    get firstElementChild() { return this.children[0]||null; }
    get lastElementChild() { const children=this.children;return children[children.length-1]||null; }
    get nextElementSibling() { let node=this.nextSibling;while(node&&node.__kind!=='element')node=node.nextSibling;return node; }
    get previousElementSibling() { let node=this.previousSibling;while(node&&node.__kind!=='element')node=node.previousSibling;return node; }
    get parentElement() { return this.parentNode&&this.parentNode.__kind==='element'?this.parentNode:null; }
    get isConnected() { for(let node=this;node;node=node.parentNode)if(node===document)return true;return false; }
    getRootNode(options={}) { let node=this;while(node.parentNode)node=node.parentNode;return options.composed&&node.__kind==='fragment'&&node.parentNode?node.parentNode.getRootNode(options):node; }
    normalize() { for(let index=this.childNodes.length-1;index>=0;index--){const child=this.childNodes[index];if(child.__kind==='text'){if(!child.__data)this.removeChild(child);else if(index>0&&this.childNodes[index-1].__kind==='text'){this.childNodes[index-1].__data+=child.__data;this.removeChild(child);}}else child.normalize();} }
    getElementsByClassName(name) { name=String(name);return descendants(this).filter(node=>node.__kind==='element'&&node.className.split(/\s+/).includes(name)); }
    getElementsByTagName(name) { name=String(name).toLowerCase();return descendants(this).filter(node=>node.__kind==='element'&&(name==='*'||node.__tag===name)); }
    contains(other) { for(let node=other;node;node=node.parentNode)if(node===this)return true;return false; }
    isSameNode(other) { return this===other; }
    isEqualNode(other) { if(!other||this.nodeType!==other.nodeType||this.nodeName!==other.nodeName||this.nodeValue!==other.nodeValue)return false;if(this.__kind==='element'){const left=Object.entries(this.__attrs),right=Object.entries(other.__attrs||{});if(left.length!==right.length||left.some(([name,value])=>other.getAttribute(name)!==value))return false;}if(this.childNodes.length!==other.childNodes.length)return false;return this.childNodes.every((child,index)=>child.isEqualNode(other.childNodes[index])); }
    compareDocumentPosition(other) { if(this===other)return 0;if(this.contains(other))return 20;if(other&&other.contains&&other.contains(this))return 10;const nodes=[document,...descendants(document)];return nodes.indexOf(this)<nodes.indexOf(other)?4:2; }
    getBoundingClientRect() { const nodes=[document,...descendants(document)];const order=Math.max(0,nodes.indexOf(this));const hidden=this.hasAttribute('hidden')||this.getAttribute('aria-hidden')==='true'||this.style.display==='none'||this.style.visibility==='hidden';const size=hidden?0:1;return {x:0,y:order,top:order,left:0,width:size,height:size,right:size,bottom:order+size,toJSON(){return {x:this.x,y:this.y,top:this.top,left:this.left,width:this.width,height:this.height,right:this.right,bottom:this.bottom};}}; }
    getClientRects() { const rect=this.getBoundingClientRect();return rect.width&&rect.height?Object.assign([rect],{item:index=>index===0?rect:null}):Object.assign([],{item:()=>null}); }
    get offsetWidth() { return this.getBoundingClientRect().width; }
    get offsetHeight() { return this.getBoundingClientRect().height; }
    get clientWidth() { return this.offsetWidth; }
    get clientHeight() { return this.offsetHeight; }
    get scrollWidth() { return this.clientWidth; }
    get scrollHeight() { return this.clientHeight; }
    matches(selector) { return matches(this,selector); }
    cloneNode(deep=false) { const copy=new OrynNode({kind:this.__kind,tag:this.__tag,data:this.__data,attrs:{...this.__attrs}},true);if(this.__tag==='template'){copy.content=new OrynNode({kind:'fragment'},true);if(deep)for(const child of this.content?.childNodes||[])copy.content.appendChild(child.cloneNode(true));}else if(deep)for(const child of this.childNodes)copy.appendChild(child.cloneNode(true));return copy; }
    get id() { return this.getAttribute('id')||''; } set id(value) { this.setAttribute('id',value); }
    get src() { const value=this.getAttribute('src');return value===null?'':new OrynURL(value,location.href).href; } set src(value) { this.setAttribute('src',value); }
    get href() { const value=this.getAttribute('href');return value===null?'':new OrynURL(value,location.href).href; } set href(value) { this.setAttribute('href',value); }
    get localName() { return this.__tag||null; }
    get namespaceURI() { return this.__namespaceURI||(this.__kind==='element'?'http://www.w3.org/1999/xhtml':null); } set namespaceURI(value) { this.__namespaceURI=value; }
    get baseURI() { return location.href; }
    get className() { return this.getAttribute('class')||''; } set className(value) { this.setAttribute('class',value); }
    get dataset() { const node=this;return new Proxy({}, {get(_target,key){const name='data-'+String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase());return node.getAttribute(name)??undefined;},set(_target,key,value){const name='data-'+String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase());node.setAttribute(name,value);return true;}}); }
    get classList() { const node=this;return {contains:name=>node.className.split(/\s+/).includes(String(name)),add(...names){const set=new Set(node.className.split(/\s+/).filter(Boolean));names.forEach(name=>set.add(String(name)));node.className=[...set].join(' ');},remove(...names){const remove=new Set(names.map(String));node.className=node.className.split(/\s+/).filter(name=>name&&!remove.has(name)).join(' ');},toggle(name,force){const has=this.contains(name);const add=force===undefined?!has:!!force;add?this.add(name):this.remove(name);return add;}}; }
    get style() { if(!this.__style){const declaration=new OrynStyleDeclaration(this);this.__style=new Proxy(declaration,{get(target,key){if(key in target){const value=target[key];return typeof value==='function'?value.bind(target):value;}return target.getPropertyValue(String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase()));},set(target,key,value){if(key in target){target[key]=value;return true;}target.setProperty(String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase()),value);return true;}});}return this.__style; }
    get options() { if(this.__tag!=='select')return undefined;const entries=descendants(this).filter(node=>node.__tag==='option');return Object.assign(entries,{item:index=>entries[index]||null,namedItem:name=>entries.find(option=>option.id===String(name)||option.getAttribute('name')===String(name))||null}); }
    get selectedOptions() { if(this.__tag!=='select')return undefined;const entries=this.options.filter(option=>option.selected);return Object.assign(entries,{item:index=>entries[index]||null}); }
    get selectedIndex() { if(this.__tag!=='select')return -1;return this.options.findIndex(option=>option.selected); }
    set selectedIndex(value) { if(this.__tag!=='select')return;const index=Number(value);for(let optionIndex=0;optionIndex<this.options.length;optionIndex++)this.options[optionIndex].selected=optionIndex===index; }
    get value() { if(this.__tag==='option')return this.getAttribute('value')??this.textContent;if(this.__tag==='select'){const selected=this.options.find(option=>option.selected)||(!this.multiple?this.options[0]:null);return selected?selected.value:'';}return this.getAttribute('value')||''; }
    set value(value) { value=String(value);if(this.__tag==='select'){let matched=false;for(const option of this.options){const selected=!matched&&option.value===value;option.selected=selected;matched=matched||selected;}return;}this.setAttribute('value',value); }
    get selected() { return this.__tag==='option'&&this.hasAttribute('selected'); } set selected(value) { if(this.__tag!=='option')return;value?this.setAttribute('selected',''):this.removeAttribute('selected'); }
    get defaultSelected() { return this.__tag==='option'&&this.hasAttribute('selected'); } set defaultSelected(value) { this.selected=value; }
    get multiple() { return this.__tag==='select'&&this.hasAttribute('multiple'); } set multiple(value) { value?this.setAttribute('multiple',''):this.removeAttribute('multiple'); }
    get disabled() { return this.hasAttribute('disabled'); } set disabled(value) { value?this.setAttribute('disabled',''):this.removeAttribute('disabled'); }
    get checked() { return this.hasAttribute('checked'); } set checked(value) { value?this.setAttribute('checked',''):this.removeAttribute('checked'); }
    get contentEditable() { return this.getAttribute('contenteditable')??'inherit'; } set contentEditable(value) { this.setAttribute('contenteditable',String(value)); }
    get isContentEditable() { let node=this;while(node&&node.__kind==='element'){const value=node.getAttribute('contenteditable');if(value!==null)return value===''||String(value).toLowerCase()==='true';node=node.parentNode;}return false; }
    get onclick() { return this.__onclick||null; } set onclick(value) { this.__onclick=typeof value==='function'?value:null;this.__onclick?this.setAttribute('data-oryn-click-listener',''):this.removeAttribute('data-oryn-click-listener'); }
    get onfocus() { return this.__onfocus||null; } set onfocus(value) { this.__onfocus=typeof value==='function'?value:null;this.__onfocus?this.setAttribute('data-oryn-focus-listener',''):this.removeAttribute('data-oryn-focus-listener'); }
    get oninput() { return this.__oninput||null; } set oninput(value) { this.__oninput=typeof value==='function'?value:null; }
    get onchange() { return this.__onchange||null; } set onchange(value) { this.__onchange=typeof value==='function'?value:null; }
    get onsubmit() { return this.__onsubmit||null; } set onsubmit(value) { this.__onsubmit=typeof value==='function'?value:null; }
    getContext(kind) { if(this.__tag!=='canvas'||kind!=='2d')return null;return {canvas:this,beginPath(){},closePath(){},clearRect(){},fillRect(){},strokeRect(){},moveTo(){},lineTo(){},arc(){},fill(){},stroke(){},save(){},restore(){},translate(){},scale(){},rotate(){},setTransform(){},fillText(){},strokeText(){},measureText(text){return {width:String(text).length*8};}}; }
    attachShadow(init={mode:'open'}) { if(this.__shadow)throw new Error('shadow root already attached');this.__shadow=new OrynNode({kind:'fragment'},true);this.__shadow.parentNode=this;this.__shadowMode=init.mode==='closed'?'closed':'open';return this.__shadow; }
    get shadowRoot() { return this.__shadowMode==='open'?this.__shadow:null; }
    export() { const children=this.childNodes.concat(this.__shadowMode==='open'&&this.__shadow?this.__shadow.childNodes:[]).concat(this.__tag==='iframe'&&this.contentDocument?this.contentDocument.childNodes:[]);return {kind:this.__kind,...(this.__id?{id:this.__id}:{}),...(this.__tag?{tag:this.__tag}:{}),...(this.__data?{data:this.__data}:{}),...(Object.keys(this.__attrs).length?{attrs:{...this.__attrs}}:{}),...(children.length?{children:children.map(node=>node.export())}:{})}; }
  }
  function descendants(root) { const out=[]; const children=root.childNodes.concat(root.__shadowMode==='open'&&root.__shadow?[root.__shadow]:[]);for(const child of children){out.push(child);out.push(...descendants(child));} return out; }
  globalThis.__oryn_get=id=>[document,...descendants(document)].find(node=>node.__id&&node.__id.slot===id.slot&&node.__id.generation===id.generation)||null;
  function matches(node, selector) {
    if(node.__kind!=='element')return false; selector=selector.trim();
    if(selector.includes(' '))selector=selector.split(/\s+/).pop();
    const not=selector.match(/:not\(\.([\w-]+)\)$/);if(not){if(node.className.split(/\s+/).includes(not[1]))return false;selector=selector.slice(0,not.index);}
    const id=selector.match(/#([\w-]+)/);if(id&&node.id!==id[1])return false;
    const classes=[...selector.matchAll(/\.([\w-]+)/g)].map(match=>match[1]);if(classes.some(name=>!node.className.split(/\s+/).includes(name)))return false;
    const attr=selector.match(/^([\w-]+)?\[([\w-]+)(?:=['\"]?([^'\"]+)['\"]?)?\]$/);
    if(attr)return(!attr[1]||node.__tag===attr[1].toLowerCase())&&node.hasAttribute(attr[2])&&(attr[3]===undefined||node.getAttribute(attr[2])===attr[3]);
    const tag=selector.split(/[.#\[]/)[0];return !tag||node.__tag===tag.toLowerCase();
  }
  function query(root, selector, includeRoot) { return (includeRoot?[root]:[]).concat(descendants(root)).filter(node=>matches(node,selector)); }
  const mutationObservers=[];
  function notifyMutation(target,record={type:'childList'}){for(const item of mutationObservers){if(!item.active)continue;const observed=item.targets.find(entry=>entry.target===target||(entry.options.subtree&&entry.target.contains(target)));if(!observed)continue;if(record.type==='attributes'&&!observed.options.attributes)continue;if(record.type==='characterData'&&!observed.options.characterData)continue;if(record.type==='childList'&&!observed.options.childList)continue;const value={type:record.type,target,addedNodes:record.addedNodes||[],removedNodes:record.removedNodes||[],previousSibling:record.previousSibling||null,nextSibling:record.nextSibling||null,attributeName:record.attributeName||null,attributeNamespace:null,oldValue:record.oldValue??null};item.records.push(value);queueMicrotask(()=>{if(!item.active||!item.records.length)return;const records=item.records.splice(0);item.callback(records,item.observer);});}}
  class OrynMutationObserver { constructor(callback){this.callback=callback;mutationObservers.push({observer:this,callback,active:false,targets:[],records:[]});}observe(target,options={}){const item=mutationObservers.find(item=>item.observer===this);item.active=true;item.targets.push({target,options:{childList:!!options.childList,attributes:!!options.attributes,characterData:!!options.characterData,subtree:!!options.subtree}});}disconnect(){const item=mutationObservers.find(item=>item.observer===this);item.active=false;item.targets=[];item.records=[];}takeRecords(){const item=mutationObservers.find(item=>item.observer===this);return item.records.splice(0);} }
  const customRegistry=new Map();
  function upgradeTree(root){for(const node of [root,...descendants(root)]){if(node.__kind!=='element')continue;const ctor=customRegistry.get(node.__tag);if(ctor&&!(node instanceof ctor)){const fresh=new ctor();for(const key of Object.keys(fresh)){if(!['__kind','__id','__tag','__data','__attrs','childNodes','parentNode','__listeners','__connected'].includes(key))node[key]=fresh[key];}if(node.__shadow)node.__shadow.parentNode=node;Object.setPrototypeOf(node,ctor.prototype);if(node.connectedCallback&&!node.__connected){node.__connected=true;node.connectedCallback();}}}}
  function serializeNode(node){if(node.__kind==='text')return node.__data;if(node.__kind==='comment')return '<!--'+node.__data+'-->';if(node.__kind!=='element')return node.childNodes.map(serializeNode).join('');const attrs=Object.entries(node.__attrs).map(([key,value])=>' '+key+'="'+String(value).replace(/"/g,'&quot;')+'"').join('');return '<'+node.__tag+attrs+'>'+node.childNodes.map(serializeNode).join('')+'</'+node.__tag+'>';}
  function parseFragment(html){const root=new OrynNode({kind:'fragment'},true);const stack=[root];const tokens=html.match(/<!--[\s\S]*?-->|<![^>]*>|<\/?[A-Za-z][^>]*>|[^<]+/g)||[];for(const token of tokens){if(token.startsWith('<!--')||token.startsWith('<!'))continue;if(token.startsWith('</')){if(stack.length>1)stack.pop();continue;}if(token.startsWith('<')){const match=token.match(/^<\s*([\w-]+)([\s\S]*?)\/?\s*>$/);if(!match)continue;const node=document.createElement(match[1]);for(const attr of match[2].matchAll(/([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g))node.setAttribute(attr[1],attr[2]??attr[3]??attr[4]??'');stack[stack.length-1].appendChild(node);if(!/\/>$/.test(token)&&!['input','img','br','hr','meta','link'].includes(node.__tag))stack.push(node);}else stack[stack.length-1].appendChild(new OrynNode({kind:'text',data:token},true));}return root.childNodes;}
  class OrynDocument extends OrynNode {
    constructor(raw) { super(raw); this.__ready=[];this.readyState='loading';this.currentScript=null; }
    createElement(tag) { tag=String(tag).toLowerCase();const ctor=customRegistry.get(tag);const node=ctor?new ctor():new OrynNode({kind:'element',tag},true);node.__kind='element';node.__tag=tag;if(!node.__id)node.__id={slot:nextNodeSlot++,generation:0};if(tag==='template'&&!node.content)node.content=new OrynNode({kind:'fragment'},true);return node; }
    createElementNS(namespace,tag) { const node=this.createElement(String(tag).split(':').pop());node.namespaceURI=namespace;return node; }
    createTextNode(data) { return new OrynNode({kind:'text',data:String(data)},true); }
    createComment(data) { return new OrynNode({kind:'comment',data:String(data)},true); }
    createDocumentFragment() { return new OrynNode({kind:'fragment'},true); }
    createEvent(type) { const event=new OrynEvent(type);event.initEvent=(name,bubbles=false,cancelable=false)=>{event.type=String(name);event.bubbles=!!bubbles;event.cancelable=!!cancelable;};return event; }
    importNode(node,deep=false) { return node.cloneNode(deep); }
    adoptNode(node) { node.remove();return node; }
    createRange() { return new OrynRange(); }
    createTreeWalker(root,whatToShow=0xFFFFFFFF,filter=null) { return new OrynTraversal(root,whatToShow,filter,false); }
    createNodeIterator(root,whatToShow=0xFFFFFFFF,filter=null) { return new OrynTraversal(root,whatToShow,filter,true); }
    getElementById(id) { return this.querySelector('#'+id); }
    addEventListener(type, callback) { if(type==='DOMContentLoaded')this.__ready.push(callback); else super.addEventListener(type,callback); }
    __dispatchReady() { this.readyState='interactive';for(const fn of this.__ready.splice(0))fn.call(this,new OrynEvent('DOMContentLoaded')); }
    get body() { return this.querySelector('body'); }
    get head() { return this.querySelector('head'); }
    get documentElement() { return this.querySelector('html'); }
    get activeElement() { return this.querySelector('[data-oryn-focused]')||this.body; }
    get defaultView() { return globalThis; }
    get title() { const node=this.querySelector('title'); return node?node.textContent:''; }
    set title(value) { let node=this.querySelector('title'); if(!node){node=this.createElement('title');this.documentElement.append(node);}node.textContent=value; }
  }
  const nodeType=node=>node.__kind==='element'?1:node.__kind==='text'?3:node.__kind==='document'?9:node.__kind==='fragment'?11:8;
  class OrynRange { constructor(){this.startContainer=document;this.startOffset=0;this.endContainer=document;this.endOffset=0;this.collapsed=true;}setStart(node,offset){this.startContainer=node;this.startOffset=Number(offset);this.collapsed=this.startContainer===this.endContainer&&this.startOffset===this.endOffset;}setEnd(node,offset){this.endContainer=node;this.endOffset=Number(offset);this.collapsed=this.startContainer===this.endContainer&&this.startOffset===this.endOffset;}setStartBefore(node){this.setStart(node.parentNode,node.parentNode.childNodes.indexOf(node));}setStartAfter(node){this.setStart(node.parentNode,node.parentNode.childNodes.indexOf(node)+1);}setEndBefore(node){this.setEnd(node.parentNode,node.parentNode.childNodes.indexOf(node));}setEndAfter(node){this.setEnd(node.parentNode,node.parentNode.childNodes.indexOf(node)+1);}selectNode(node){this.setStartBefore(node);this.setEndAfter(node);}selectNodeContents(node){this.setStart(node,0);this.setEnd(node,node.__kind==='text'?node.__data.length:node.childNodes.length);}collapse(toStart=false){if(toStart){this.endContainer=this.startContainer;this.endOffset=this.startOffset;}else{this.startContainer=this.endContainer;this.startOffset=this.endOffset;}this.collapsed=true;}cloneRange(){const range=new OrynRange();Object.assign(range,this);return range;}toString(){if(this.startContainer===this.endContainer&&this.startContainer.__kind==='text')return this.startContainer.__data.slice(this.startOffset,this.endOffset);return this.commonAncestorContainer.textContent;}get commonAncestorContainer(){let node=this.startContainer;while(node){let end=this.endContainer;while(end){if(node===end)return node;end=end.parentNode;}node=node.parentNode;}return document;}cloneContents(){const fragment=document.createDocumentFragment();if(this.startContainer===this.endContainer){const node=this.startContainer;if(node.__kind==='text')fragment.appendChild(document.createTextNode(node.__data.slice(this.startOffset,this.endOffset)));else for(const child of node.childNodes.slice(this.startOffset,this.endOffset))fragment.appendChild(child.cloneNode(true));}return fragment;}deleteContents(){if(this.startContainer===this.endContainer){const node=this.startContainer;if(node.__kind==='text'){node.__data=node.__data.slice(0,this.startOffset)+node.__data.slice(this.endOffset);}else for(const child of node.childNodes.slice(this.startOffset,this.endOffset))child.remove();this.collapse(true);}}insertNode(node){const container=this.startContainer;if(container.__kind==='text'){const parent=container.parentNode;parent.insertBefore(node,container);return;}container.insertBefore(node,container.childNodes[this.startOffset]||null);} }
  const selection=new (class { constructor(){this.__ranges=[];}get rangeCount(){return this.__ranges.length;}get isCollapsed(){return !this.__ranges.length||this.__ranges.every(range=>range.collapsed);}get anchorNode(){return this.__ranges[0]?.startContainer??null;}get anchorOffset(){return this.__ranges[0]?.startOffset??0;}get focusNode(){return this.__ranges[0]?.endContainer??null;}get focusOffset(){return this.__ranges[0]?.endOffset??0;}addRange(range){this.__ranges=[range];}removeAllRanges(){this.__ranges=[];}getRangeAt(index){if(!this.__ranges[index])throw new Error('IndexSizeError');return this.__ranges[index];}collapse(node,offset=0){const range=new OrynRange();range.setStart(node,offset);range.collapse(true);this.addRange(range);}toString(){return this.__ranges.map(String).join('');}})();
  class OrynTraversal { constructor(root,whatToShow,filter,iterator){this.root=root;this.whatToShow=whatToShow;this.filter=filter;this.__nodes=[root,...descendants(root)].filter(node=>this.__accept(node));this.__index=iterator?-1:0;this.currentNode=root;}__accept(node){const mask=1<<(nodeType(node)-1);if((this.whatToShow&mask)===0)return false;const result=typeof this.filter==='function'?this.filter(node):this.filter&&this.filter.acceptNode?this.filter.acceptNode(node):1;return result===1;}nextNode(){if(this.__index+1>=this.__nodes.length)return null;this.currentNode=this.__nodes[++this.__index];return this.currentNode;}previousNode(){if(this.__index-1<0)return null;this.currentNode=this.__nodes[--this.__index];return this.currentNode;}parentNode(){let node=this.currentNode.parentNode;while(node&&node!==this.root.parentNode){if(this.__accept(node)){this.currentNode=node;return node;}node=node.parentNode;}return null;}firstChild(){const node=[...this.currentNode.childNodes].find(item=>this.__accept(item))||null;if(node)this.currentNode=node;return node;}nextSibling(){if(!this.currentNode.parentNode)return null;const siblings=this.currentNode.parentNode.childNodes;const node=siblings.slice(siblings.indexOf(this.currentNode)+1).find(item=>this.__accept(item))||null;if(node)this.currentNode=node;return node;}detach(){} }
  const microtasks=[]; const timers=[];
  globalThis.Event=OrynEvent;
  globalThis.CustomEvent=OrynEvent;
  globalThis.CSSStyleDeclaration=OrynStyleDeclaration;
  const consoleEntries=[];
  const contextTraceEntries=[];const traceContext=(kind,detail={})=>contextTraceEntries.push({kind,detail});globalThis.__oryn_take_context_trace=()=>contextTraceEntries.splice(0);
  const formatConsoleItem=item=>item&&typeof item==='object'&&typeof item.stack==='string'?item.stack:String(item);
  const captureConsole=(level,items)=>{let message=items.map(formatConsoleItem).join(' ');if(level==='error'&&message==='Hydration completed but contains mismatches.')message+='\n'+new Error('hydration mismatch call site').stack;consoleEntries.push({level,message});};
  globalThis.console={log:(...items)=>captureConsole('log',items),info:(...items)=>captureConsole('info',items),warn:(...items)=>captureConsole('warn',items),error:(...items)=>captureConsole('error',items)};
  globalThis.__oryn_take_console=()=>consoleEntries.splice(0);
  globalThis.alert=()=>{};globalThis.confirm=()=>true;globalThis.prompt=()=>null;
  globalThis.HTMLElement=OrynNode;
  globalThis.Node=OrynNode;globalThis.Element=OrynNode;globalThis.Document=OrynDocument;globalThis.DocumentFragment=OrynNode;globalThis.HTMLIFrameElement=OrynNode;globalThis.HTMLInputElement=OrynNode;globalThis.HTMLSelectElement=OrynNode;globalThis.HTMLTextAreaElement=OrynNode;
  globalThis.Text=class Text extends OrynNode { constructor(data=''){super({kind:'text',data:String(data)},true);}static [Symbol.hasInstance](value){return value&&value.__kind==='text';} };
  globalThis.Comment=class Comment extends OrynNode { constructor(data=''){super({kind:'comment',data:String(data)},true);}static [Symbol.hasInstance](value){return value&&value.__kind==='comment';} };
  const htmlElementClass=(...tags)=>class extends OrynNode { static [Symbol.hasInstance](value){return value&&value.__kind==='element'&&tags.includes(value.__tag);} };
  globalThis.SVGElement=class SVGElement extends OrynNode { static [Symbol.hasInstance](value){return value&&value.__kind==='element'&&String(value.namespaceURI).includes('svg');} };globalThis.SVGAElement=class SVGAElement extends globalThis.SVGElement { static [Symbol.hasInstance](value){return value&&value.__tag==='a'&&String(value.namespaceURI).includes('svg');} };
  globalThis.HTMLMediaElement=htmlElementClass('audio','video');globalThis.HTMLAudioElement=htmlElementClass('audio');globalThis.HTMLVideoElement=htmlElementClass('video');globalThis.HTMLButtonElement=htmlElementClass('button');globalThis.HTMLFormElement=htmlElementClass('form');globalThis.HTMLAnchorElement=htmlElementClass('a');globalThis.HTMLOptionElement=htmlElementClass('option');globalThis.HTMLCanvasElement=htmlElementClass('canvas');globalThis.HTMLTemplateElement=htmlElementClass('template');globalThis.HTMLScriptElement=htmlElementClass('script');globalThis.HTMLLinkElement=htmlElementClass('link');
  globalThis.EventTarget=OrynNode;globalThis.MouseEvent=OrynEvent;globalThis.KeyboardEvent=OrynEvent;globalThis.InputEvent=OrynEvent;globalThis.FocusEvent=OrynEvent;
  globalThis.Range=OrynRange;globalThis.Selection=selection.constructor;globalThis.TreeWalker=OrynTraversal;globalThis.NodeIterator=OrynTraversal;globalThis.NodeFilter={SHOW_ALL:0xFFFFFFFF,SHOW_ELEMENT:1,SHOW_TEXT:4,FILTER_ACCEPT:1,FILTER_REJECT:2,FILTER_SKIP:3};globalThis.getSelection=()=>selection;
  globalThis.MutationObserver=OrynMutationObserver;
  class OrynIntersectionObserver { constructor(callback,options={}){this.callback=callback;this.root=options.root||null;this.rootMargin=options.rootMargin||'0px';this.thresholds=Array.isArray(options.threshold)?options.threshold:[options.threshold??0];this.__targets=new Set();}observe(target){this.__targets.add(target);}unobserve(target){this.__targets.delete(target);}disconnect(){this.__targets.clear();}takeRecords(){return [];} }
  class OrynResizeObserver { constructor(callback){this.callback=callback;this.__targets=new Set();}observe(target){this.__targets.add(target);}unobserve(target){this.__targets.delete(target);}disconnect(){this.__targets.clear();} }
  globalThis.IntersectionObserver=OrynIntersectionObserver;globalThis.ResizeObserver=OrynResizeObserver;
  const globalListeners=new Map();
  globalThis.addEventListener=(type,callback)=>{type=String(type);const list=globalListeners.get(type)||[];list.push(callback);globalListeners.set(type,list);};
  globalThis.removeEventListener=(type,callback)=>globalListeners.set(String(type),(globalListeners.get(String(type))||[]).filter(item=>item!==callback));
  globalThis.dispatchEvent=event=>{if(!(event instanceof OrynEvent))event=new OrynEvent(event.type||event);event.target=globalThis;event.currentTarget=globalThis;for(const fn of [...(globalListeners.get(event.type)||[])])fn.call(globalThis,event);const handler=globalThis['on'+event.type];if(typeof handler==='function')handler.call(globalThis,event);return !event.defaultPrevented;};
  globalThis.pageXOffset=0;globalThis.pageYOffset=0;globalThis.scrollX=0;globalThis.scrollY=0;globalThis.scrollTo=(x=0,y=0)=>{if(typeof x==='object'){y=x.top??0;x=x.left??0;}globalThis.pageXOffset=globalThis.scrollX=Number(x)||0;globalThis.pageYOffset=globalThis.scrollY=Number(y)||0;};globalThis.scrollBy=(x=0,y=0)=>globalThis.scrollTo(globalThis.scrollX+(Number(x)||0),globalThis.scrollY+(Number(y)||0));
  globalThis.getComputedStyle=node=>node&&node.style?node.style:new OrynStyleDeclaration({getAttribute:()=>'',setAttribute:()=>{}});
  globalThis.customElements={define(name,ctor){name=String(name).toLowerCase();customRegistry.set(name,ctor);if(globalThis.document)upgradeTree(document);},get:name=>customRegistry.get(String(name).toLowerCase())};
  class OrynURLSearchParams { constructor(init=''){this.items=[];const value=String(init).replace(/^\?/,'');for(const part of value.split('&'))if(part){const [key,item='']=part.split('=');this.append(decodeURIComponent(key.replace(/\+/g,' ')),decodeURIComponent(item.replace(/\+/g,' ')));}}append(key,value){this.items.push([String(key),String(value)]);}set(key,value){this.delete(key);this.append(key,value);}get(key){const item=this.items.find(item=>item[0]===String(key));return item?item[1]:null;}getAll(key){return this.items.filter(item=>item[0]===String(key)).map(item=>item[1]);}has(key){return this.items.some(item=>item[0]===String(key));}delete(key){this.items=this.items.filter(item=>item[0]!==String(key));}toString(){return this.items.map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(value)).join('&');}}
  class OrynURL { constructor(input,base){input=String(input);if(base&&!/^[a-z][a-z0-9+.-]*:/i.test(input)){const parent=new OrynURL(base);const opaque=['about:','data:','javascript:'].includes(parent.protocol);const root=parent.host?parent.origin:(opaque?parent.protocol:parent.protocol+'//');if(input.startsWith('#'))input=root+parent.pathname+parent.search+input;else if(input.startsWith('?'))input=root+parent.pathname+input;else input=input.startsWith('/')?root+input:root+parent.pathname.replace(/[^/]*$/,'')+input;}const match=input.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i);if(!match)throw new TypeError('Invalid URL');this.protocol=match[1];this.host=match[2]||'';this.hostname=this.host.split(':')[0];this.port=this.host.slice(this.hostname.length+1);this.pathname=match[3]||'/';this.search=match[4]||'';this.hash=match[5]||'';this.origin=this.host?this.protocol+'//'+this.host:'null';this.searchParams=new OrynURLSearchParams(this.search);}get href(){const query=this.searchParams.toString();const opaque=['about:','data:','javascript:'].includes(this.protocol);const prefix=this.host?this.origin:(opaque?this.protocol:this.protocol+'//');return prefix+this.pathname+(query?'?'+query:'')+this.hash;}set href(value){const next=new OrynURL(value,this.href);this.protocol=next.protocol;this.host=next.host;this.hostname=next.hostname;this.port=next.port;this.pathname=next.pathname;this.search=next.search;this.hash=next.hash;this.origin=next.origin;this.searchParams=next.searchParams;}assign(value){navigateTarget(value,'_self');}replace(value){navigateTarget(value,'_self');}reload(){navigateTarget(this.href,'_self');}toString(){return this.href;}toJSON(){return this.href;}}
  class OrynTextEncoder { encode(value=''){const bytes=unescape(encodeURIComponent(String(value)));return Uint8Array.from(bytes,char=>char.charCodeAt(0));} }
  class OrynTextDecoder { decode(value=new Uint8Array()){let bytes='';for(const byte of value)bytes+=String.fromCharCode(byte);return decodeURIComponent(escape(bytes));} }
  globalThis.URL=OrynURL;globalThis.URLSearchParams=OrynURLSearchParams;globalThis.TextEncoder=OrynTextEncoder;globalThis.TextDecoder=OrynTextDecoder;
  class OrynHeaders { constructor(entries=[]){this.items=new Map();if(entries instanceof OrynHeaders)entries=entries.entries();else if(!Array.isArray(entries)&&!(entries&&entries[Symbol.iterator]))entries=Object.entries(entries||{});for(const [key,value] of entries)this.append(key,value);}get(key){return this.items.get(String(key).toLowerCase())||null;}has(key){return this.items.has(String(key).toLowerCase());}set(key,value){this.items.set(String(key).toLowerCase(),String(value));}append(key,value){key=String(key).toLowerCase();value=String(value);this.items.set(key,this.items.has(key)?this.items.get(key)+', '+value:value);}delete(key){this.items.delete(String(key).toLowerCase());}forEach(callback,thisArg){for(const [key,value] of this.items)callback.call(thisArg,value,key,this);}keys(){return this.items.keys();}values(){return this.items.values();}entries(){return this.items.entries();}[Symbol.iterator](){return this.entries();} }
  globalThis.Headers=OrynHeaders;
  const b64chars='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const bytesToBase64=value=>{const bytes=value instanceof Uint8Array?value:new Uint8Array(value);let out='';for(let index=0;index<bytes.length;index+=3){const first=bytes[index],second=bytes[index+1],third=bytes[index+2];out+=b64chars[first>>2];out+=b64chars[((first&3)<<4)|((second??0)>>4)];out+=second===undefined?'=':b64chars[((second&15)<<2)|((third??0)>>6)];out+=third===undefined?'=':b64chars[third&63];}return out;};
  const base64ToBytes=value=>{value=String(value).replace(/\s/g,'');const out=[];for(let index=0;index<value.length;index+=4){const first=b64chars.indexOf(value[index]),second=b64chars.indexOf(value[index+1]),third=value[index+2]==='='?-1:b64chars.indexOf(value[index+2]),fourth=value[index+3]==='='?-1:b64chars.indexOf(value[index+3]);out.push((first<<2)|(second>>4));if(third>=0)out.push(((second&15)<<4)|(third>>2));if(fourth>=0)out.push(((third&3)<<6)|fourth);}return Uint8Array.from(out);};
  globalThis.btoa=value=>bytesToBase64(Uint8Array.from(String(value),char=>char.charCodeAt(0)&255));globalThis.atob=value=>String.fromCharCode(...base64ToBytes(value));
  const bodyBytes=body=>{if(body===undefined||body===null)return new Uint8Array();if(body instanceof OrynBlob)return body.__bytes.slice();if(body instanceof ArrayBuffer)return new Uint8Array(body.slice(0));if(ArrayBuffer.isView(body))return new Uint8Array(body.buffer.slice(body.byteOffset,body.byteOffset+body.byteLength));return new TextEncoder().encode(String(body));};
  class OrynBlob { constructor(parts=[],options={}){const chunks=parts.map(bodyBytes);const length=chunks.reduce((sum,item)=>sum+item.length,0);this.__bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){this.__bytes.set(chunk,offset);offset+=chunk.length;}this.type=String(options.type||'').toLowerCase();}get size(){return this.__bytes.length;}slice(start=0,end=this.size,type=''){return new OrynBlob([this.__bytes.slice(start<0?Math.max(this.size+start,0):start,end<0?Math.max(this.size+end,0):end)],{type});}arrayBuffer(){return Promise.resolve(this.__bytes.buffer.slice(0));}text(){return Promise.resolve(new TextDecoder().decode(this.__bytes));}stream(){return new OrynReadableStream({start:controller=>{controller.enqueue(this.__bytes.slice());controller.close();}});} }
  class OrynFile extends OrynBlob { constructor(parts,name,options={}){super(parts,options);this.name=String(name);this.lastModified=Number(options.lastModified??0);this.webkitRelativePath='';} }
  globalThis.Blob=OrynBlob;globalThis.File=OrynFile;
  class OrynReadableStream { constructor(source={}){this.__queue=[];this.__closed=false;this.__error=null;this.__waiting=[];this.locked=false;const controller={enqueue:value=>{if(this.__closed)return;const waiter=this.__waiting.shift();waiter?waiter.resolve({done:false,value}):this.__queue.push(value);},close:()=>{this.__closed=true;for(const waiter of this.__waiting.splice(0))waiter.resolve({done:true,value:undefined});},error:error=>{this.__error=error;for(const waiter of this.__waiting.splice(0))waiter.reject(error);}};this.__cancel=source.cancel;if(source.start)source.start(controller);this.__pull=source.pull?()=>source.pull(controller):null;}getReader(){if(this.locked)throw new TypeError('stream is locked');this.locked=true;const stream=this;return {read(){if(stream.__error)return Promise.reject(stream.__error);if(stream.__queue.length)return Promise.resolve({done:false,value:stream.__queue.shift()});if(stream.__closed)return Promise.resolve({done:true,value:undefined});return new Promise((resolve,reject)=>{stream.__waiting.push({resolve,reject});if(stream.__pull)Promise.resolve(stream.__pull()).catch(error=>{const index=stream.__waiting.findIndex(item=>item.resolve===resolve);if(index>=0)stream.__waiting.splice(index,1);reject(error);});});},cancel(reason){stream.__closed=true;return Promise.resolve(stream.__cancel?stream.__cancel(reason):undefined);},releaseLock(){stream.locked=false;}};}cancel(reason){this.__closed=true;return Promise.resolve(this.__cancel?this.__cancel(reason):undefined);}pipeTo(destination){const reader=this.getReader(),writer=destination.getWriter();const pump=()=>reader.read().then(item=>item.done?writer.close():writer.write(item.value).then(pump));return pump();}pipeThrough(transform){this.pipeTo(transform.writable);return transform.readable;} }
  class OrynWritableStream { constructor(sink={}){this.__sink=sink;this.locked=false;}getWriter(){if(this.locked)throw new TypeError('stream is locked');this.locked=true;const stream=this;return {write:chunk=>Promise.resolve(stream.__sink.write?stream.__sink.write(chunk):undefined),close:()=>Promise.resolve(stream.__sink.close?stream.__sink.close():undefined),abort:reason=>Promise.resolve(stream.__sink.abort?stream.__sink.abort(reason):undefined),releaseLock(){stream.locked=false;},get ready(){return Promise.resolve();},get desiredSize(){return 1;}};}abort(reason){return Promise.resolve(this.__sink.abort?this.__sink.abort(reason):undefined);}close(){return Promise.resolve(this.__sink.close?this.__sink.close():undefined);} }
  class OrynTransformStream { constructor(transformer={}){let readableController;this.readable=new OrynReadableStream({start:controller=>{readableController=controller;}});this.writable=new OrynWritableStream({write:chunk=>transformer.transform?transformer.transform(chunk,{enqueue:value=>readableController.enqueue(value),error:error=>readableController.error(error),terminate:()=>readableController.close()}):readableController.enqueue(chunk),close:()=>{if(transformer.flush)return Promise.resolve(transformer.flush({enqueue:value=>readableController.enqueue(value)})).then(()=>readableController.close());readableController.close();}});} }
  globalThis.ReadableStream=OrynReadableStream;globalThis.WritableStream=OrynWritableStream;globalThis.TransformStream=OrynTransformStream;
  class OrynAbortSignal { constructor(){this.aborted=false;this.reason=undefined;this.__listeners=[];}addEventListener(type,callback){if(type==='abort')this.__listeners.push(callback);}removeEventListener(type,callback){if(type==='abort')this.__listeners=this.__listeners.filter(item=>item!==callback);}throwIfAborted(){if(this.aborted)throw this.reason;} }
  class OrynAbortController { constructor(){this.signal=new OrynAbortSignal();}abort(reason=new Error('The operation was aborted')){if(this.signal.aborted)return;this.signal.aborted=true;this.signal.reason=reason;for(const callback of this.signal.__listeners.splice(0))callback.call(this.signal,new OrynEvent('abort'));} }
  globalThis.AbortSignal=OrynAbortSignal;globalThis.AbortController=OrynAbortController;
  class OrynRequest { constructor(input,init={}){const source=input instanceof OrynRequest?input:null;this.url=new OrynURL(source?source.url:input&&input.url?input.url:String(input),location.href).href;this.method=String(init.method??source?.method??'GET').toUpperCase();this.headers=new OrynHeaders(init.headers??source?.headers??[]);this.body=init.body??source?.body??null;this.signal=init.signal??source?.signal??new OrynAbortSignal();this.credentials=init.credentials??source?.credentials??'same-origin';this.mode=init.mode??source?.mode??'cors';}clone(){return new OrynRequest(this);} }
  class OrynResponse { constructor(body=null,init={}){this.__bytes=bodyBytes(body);this.__chunks=Array.isArray(init.bodyChunks)&&init.bodyChunks.length?init.bodyChunks.map(bodyBytes):[this.__bytes.slice()];this.status=Number(init.status??200);this.statusText=String(init.statusText??'');this.headers=new OrynHeaders(init.headers||[]);this.url=String(init.url||'');this.redirected=!!init.redirected;this.type='basic';this.bodyUsed=false;this.body=new OrynReadableStream({start:controller=>{for(const chunk of this.__chunks){traceContext('stream_chunk',{direction:'response',bytes:chunk.length});controller.enqueue(chunk.slice());}controller.close();}});}get ok(){return this.status>=200&&this.status<300;}arrayBuffer(){this.bodyUsed=true;return Promise.resolve(this.__bytes.buffer.slice(0));}text(){this.bodyUsed=true;return Promise.resolve(new TextDecoder().decode(this.__bytes));}json(){return this.text().then(JSON.parse);}blob(){this.bodyUsed=true;return Promise.resolve(new OrynBlob([this.__bytes],{type:this.headers.get('content-type')||''}));}clone(){if(this.bodyUsed)throw new TypeError('body already used');return new OrynResponse(this.__bytes,{status:this.status,statusText:this.statusText,headers:this.headers,url:this.url,redirected:this.redirected,bodyChunks:this.__chunks});}static json(value,init={}){const headers=new OrynHeaders(init.headers||[]);if(!headers.has('content-type'))headers.set('content-type','application/json');return new OrynResponse(JSON.stringify(value),{...init,headers});}static redirect(url,status=302){return new OrynResponse(null,{status,headers:[['location',String(url)]]});} }
  globalThis.Request=OrynRequest;globalThis.Response=OrynResponse;
  let contextPageId=null;let contextOpenerPageId=null;let contextEventId=0;
  const contextCall=request=>{if(typeof globalThis.__oryn_context_sync!=='function')return null;return JSON.parse(globalThis.__oryn_context_sync(JSON.stringify(request)));};
  const storageScope=()=>({origin:location.origin,top_level_page_id:contextPageId});
  class OrynStorage {
    constructor(area){this.area=area;this.fallback=new Map();}
    __response(operation,extra={}){const outer=contextCall({operation:'storage',request:{operation,area:this.area,scope:storageScope(),...extra}});return outer&&outer.result==='storage'?outer.response:null;}
    get items(){const response=this.__response('entries');return response&&response.result==='entries'?new Map(response.entries):this.fallback;}
    get length(){return this.items.size;}
    key(index){return [...this.items.keys()][index]??null;}
    getItem(key){key=String(key);const response=this.__response('get',{key});if(response&&response.result==='value')return response.value??null;return this.fallback.has(key)?this.fallback.get(key):null;}
    setItem(key,value){key=String(key);value=String(value);const response=this.__response('set',{key,value});if(response===null)this.fallback.set(key,value);else traceContext('storage_changed',{area:this.area,origin:location.origin,key});}
    removeItem(key){key=String(key);const response=this.__response('remove',{key});if(response===null)this.fallback.delete(key);else traceContext('storage_changed',{area:this.area,origin:location.origin,key});}
    clear(){const response=this.__response('clear');if(response===null)this.fallback.clear();else traceContext('storage_changed',{area:this.area,origin:location.origin,key:null});}
  }
  globalThis.localStorage=new OrynStorage('local');globalThis.sessionStorage=new OrynStorage('session');
  globalThis.__oryn_restore_storage=(localItems,sessionItems)=>{if(typeof globalThis.__oryn_context_sync==='function')return;for(const [key,value] of localItems||[])localStorage.setItem(key,value);for(const [key,value] of sessionItems||[])sessionStorage.setItem(key,value);};
  const idbCall=command=>{const outer=contextCall({operation:'storage',request:{operation:'indexed_db',origin:location.origin,command}});if(!outer||outer.result!=='storage'||outer.response.result!=='indexed_db')throw new Error('IndexedDB context response was invalid');return outer.response.response;};
  const idbCloneEncode=value=>{if(value instanceof ArrayBuffer)return {__oryn_clone:'array_buffer',base64:bytesToBase64(value)};if(ArrayBuffer.isView(value))return {__oryn_clone:'typed_array',type:value.constructor.name,base64:bytesToBase64(new Uint8Array(value.buffer,value.byteOffset,value.byteLength))};if(value instanceof OrynFile)return {__oryn_clone:'file',base64:bytesToBase64(value.__bytes),type:value.type,name:value.name,last_modified:value.lastModified};if(value instanceof OrynBlob)return {__oryn_clone:'blob',base64:bytesToBase64(value.__bytes),type:value.type};if(Array.isArray(value))return value.map(idbCloneEncode);if(value&&typeof value==='object'){const result={};for(const [key,item] of Object.entries(value))result[key]=idbCloneEncode(item);return result;}if(typeof value==='function'||typeof value==='symbol')throw new Error('DataCloneError');return value;};
  const idbCloneDecode=value=>{if(Array.isArray(value))return value.map(idbCloneDecode);if(value&&typeof value==='object'){if(value.__oryn_clone==='array_buffer')return base64ToBytes(value.base64).buffer;if(value.__oryn_clone==='typed_array'){const bytes=base64ToBytes(value.base64),constructor=globalThis[value.type];return typeof constructor==='function'?new constructor(bytes.buffer):bytes;}if(value.__oryn_clone==='blob')return new OrynBlob([base64ToBytes(value.base64)],{type:value.type});if(value.__oryn_clone==='file')return new OrynFile([base64ToBytes(value.base64)],value.name,{type:value.type,lastModified:value.last_modified});const result={};for(const [key,item] of Object.entries(value))result[key]=idbCloneDecode(item);return result;}return value;};
  class OrynIDBRequest { constructor(){this.result=undefined;this.error=null;this.readyState='pending';this.onsuccess=null;this.onerror=null;this.onupgradeneeded=null;this.__listeners=new Map();}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list);}__fire(type,extra={}){const event=new OrynEvent(type);Object.assign(event,{target:this,...extra});for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event);}__success(value){queueMicrotask(()=>{this.result=value;this.readyState='done';this.__fire('success');});}__failure(error){queueMicrotask(()=>{this.error=error;this.readyState='done';this.__fire('error');});} }
  class OrynIDBCursor { constructor(request,records,index){this.__request=request;this.__records=records;this.__index=index;this.__set();}__set(){const record=this.__records[this.__index];this.key=record&&record.key;this.primaryKey=record&&record.primary_key;this.value=record&&idbCloneDecode(record.value);}continue(){this.__index++;this.__set();this.__request.result=this.__index<this.__records.length?this:null;queueMicrotask(()=>this.__request.__fire('success'));}advance(count){this.__index+=Math.max(1,Number(count)||1);this.__set();this.__request.result=this.__index<this.__records.length?this:null;queueMicrotask(()=>this.__request.__fire('success'));}update(value){return this.__request.__store.put(value,this.primaryKey);}delete(){return this.__request.__store.delete(this.primaryKey);} }
  class OrynIDBIndex { constructor(store,name){this.objectStore=store;this.name=String(name);}__cursor(query,direction){const request=new OrynIDBRequest();request.__store=this.objectStore;try{const response=idbCall({command:'cursor',database:this.objectStore.transaction.db.name,store:this.objectStore.name,index:this.name,query:query&&query.__orynRange?query.value:query??null,reverse:String(direction||'next').startsWith('prev')});const cursor=response.records.length?new OrynIDBCursor(request,response.records,0):null;request.__success(cursor);}catch(error){request.__failure(error);}return request;}openCursor(query,direction){return this.__cursor(query,direction);}get(query){const request=new OrynIDBRequest();try{const response=idbCall({command:'cursor',database:this.objectStore.transaction.db.name,store:this.objectStore.name,index:this.name,query:query??null,reverse:false});request.__success(idbCloneDecode(response.records[0]?.value));}catch(error){request.__failure(error);}return request;}getAll(query){const request=new OrynIDBRequest();try{const response=idbCall({command:'cursor',database:this.objectStore.transaction.db.name,store:this.objectStore.name,index:this.name,query:query??null,reverse:false});request.__success(response.records.map(record=>idbCloneDecode(record.value)));}catch(error){request.__failure(error);}return request;} }
  class OrynIDBObjectStore { constructor(transaction,name){this.transaction=transaction;this.name=String(name);}__request(command,project=response=>response){const request=new OrynIDBRequest();this.transaction.__pending++;try{const response=idbCall(command);request.__success(project(response));queueMicrotask(()=>this.transaction.__finish());}catch(error){request.__failure(error);this.transaction.error=error;queueMicrotask(()=>this.transaction.__finish(error));}return request;}add(value,key){return this.__request({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:idbCloneEncode(value),add_only:true}]},response=>response.key);}put(value,key){return this.__request({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:idbCloneEncode(value),add_only:false}]},response=>response.key);}get(key){return this.__request({command:'get',database:this.transaction.db.name,store:this.name,key},response=>idbCloneDecode(response.value));}getAll(){return this.__request({command:'get_all',database:this.transaction.db.name,store:this.name},response=>response.values.map(idbCloneDecode));}delete(key){return this.__request({command:'delete',database:this.transaction.db.name,store:this.name,key},()=>undefined);}clear(){return this.__request({command:'clear',database:this.transaction.db.name,store:this.name},()=>undefined);}count(){return this.__request({command:'count',database:this.transaction.db.name,store:this.name},response=>response.count);}createIndex(name,keyPath,options={}){idbCall({command:'create_index',database:this.transaction.db.name,store:this.name,definition:{name:String(name),key_path:String(keyPath),unique:!!options.unique}});return new OrynIDBIndex(this,name);}index(name){return new OrynIDBIndex(this,name);}openCursor(query,direction){const request=new OrynIDBRequest();request.__store=this;this.transaction.__pending++;try{const response=idbCall({command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query:query&&query.__orynRange?query.value:query??null,reverse:String(direction||'next').startsWith('prev')});request.__success(response.records.length?new OrynIDBCursor(request,response.records,0):null);queueMicrotask(()=>this.transaction.__finish());}catch(error){request.__failure(error);queueMicrotask(()=>this.transaction.__finish(error));}return request;} }
  class OrynIDBTransaction { constructor(db,names,mode='readonly'){this.db=db;this.mode=mode;this.objectStoreNames=Array.isArray(names)?names:[String(names)];this.error=null;this.oncomplete=null;this.onerror=null;this.onabort=null;this.__pending=0;this.__completed=false;}objectStore(name){if(!this.objectStoreNames.includes(String(name)))throw new Error('NotFoundError');return new OrynIDBObjectStore(this,name);}__finish(error){if(this.__pending>0){this.__pending--;if(this.__pending>0)return;}if(this.__completed)return;this.__completed=true;const event=new OrynEvent(error?'error':'complete');event.target=this;const handler=error?this.onerror:this.oncomplete;if(typeof handler==='function')handler.call(this,event);}abort(){this.error=new Error('AbortError');if(this.onabort)this.onabort.call(this,new OrynEvent('abort'));} }
  class OrynIDBDatabase { constructor(name,version,stores=[]){this.name=String(name);this.version=version;this.objectStoreNames=stores.slice();this.onversionchange=null;}createObjectStore(name,options={}){const response=idbCall({command:'create_object_store',database:this.name,definition:{name:String(name),key_path:options.keyPath??null,auto_increment:!!options.autoIncrement}});this.objectStoreNames=response.object_stores;const transaction=new OrynIDBTransaction(this,[name],'versionchange');return new OrynIDBObjectStore(transaction,name);}deleteObjectStore(name){idbCall({command:'delete_object_store',database:this.name,store:String(name)});this.objectStoreNames=this.objectStoreNames.filter(item=>item!==String(name));}transaction(names,mode='readonly'){return new OrynIDBTransaction(this,names,mode);}close(){} }
  globalThis.IDBKeyRange={only:value=>({__orynRange:true,value}),lowerBound:value=>({__orynRange:true,value,lower:true}),upperBound:value=>({__orynRange:true,value,upper:true}),bound:(lower,upper)=>({__orynRange:true,lower,upper})};
  globalThis.indexedDB={open(name,version=0){const request=new OrynIDBRequest();try{const response=idbCall({command:'open',name:String(name),version:Number(version)||0});const db=new OrynIDBDatabase(name,response.version,response.object_stores);queueMicrotask(()=>{request.result=db;if(response.upgrade_needed)request.__fire('upgradeneeded',{oldVersion:response.old_version,newVersion:response.version});request.readyState='done';request.__fire('success');});}catch(error){request.__failure(error);}return request;},deleteDatabase(name){const request=new OrynIDBRequest();try{idbCall({command:'delete_database',name:String(name)});request.__success(undefined);}catch(error){request.__failure(error);}return request;}};
  class OrynFormData { constructor(form){this.items=[];if(form){for(const node of descendants(form)){if(!['input','textarea','select'].includes(node.__tag)||node.disabled)continue;const name=node.getAttribute('name');if(!name)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;this.append(name,node.value);}}}append(name,value,filename){if(filename!==undefined&&value instanceof OrynBlob)value=new OrynFile([value.__bytes],filename,{type:value.type});this.items.push([String(name),value instanceof OrynBlob?value:String(value)]);}set(name,value,filename){this.delete(name);this.append(name,value,filename);}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null;}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1]);}has(name){return this.items.some(item=>item[0]===String(name));}delete(name){this.items=this.items.filter(item=>item[0]!==String(name));}entries(){return this.items[Symbol.iterator]();}keys(){return this.items.map(item=>item[0])[Symbol.iterator]();}values(){return this.items.map(item=>item[1])[Symbol.iterator]();}forEach(callback,thisArg){for(const [key,value] of this.items)callback.call(thisArg,value,key,this);}[Symbol.iterator](){return this.entries();}toString(){return this.items.map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(value instanceof OrynBlob?value.name||'blob':value)).join('&');} }
  globalThis.FormData=OrynFormData;
  globalThis.location=new OrynURL('about:blank');
  globalThis.__oryn_set_location=value=>{globalThis.location=new OrynURL(value);document.location=globalThis.location;historyEntries[0]={url:globalThis.location.href,state:null};historyIndex=0;};
  const historyEntries=[{url:'about:blank',state:null}];let historyIndex=0;
  const updateLocation=href=>{globalThis.location.href=href;document.location=globalThis.location;};
  globalThis.history={scrollRestoration:'auto',get length(){return historyEntries.length;},get state(){return historyEntries[historyIndex]?.state??null;},pushState(state,_title,url){const href=url===undefined||url===null?location.href:new OrynURL(url,location.href).href;historyEntries.splice(historyIndex+1);historyEntries.push({url:href,state});historyIndex=historyEntries.length-1;updateLocation(href);if(contextPageId!==null)contextCall({operation:'commit_page',page_id:contextPageId,url:href});},replaceState(state,_title,url){const href=url===undefined||url===null?location.href:new OrynURL(url,location.href).href;historyEntries[historyIndex]={url:href,state};updateLocation(href);if(contextPageId!==null)contextCall({operation:'commit_page',page_id:contextPageId,url:href});},go(delta=0){const next=Math.max(0,Math.min(historyEntries.length-1,historyIndex+Number(delta||0)));if(next===historyIndex)return;historyIndex=next;const entry=historyEntries[historyIndex];updateLocation(entry.url);const event=new OrynEvent('popstate');event.state=entry.state;globalThis.dispatchEvent(event);},back(){this.go(-1);},forward(){this.go(1);}};
  const navigateTarget=(url,target='_self')=>{url=new OrynURL(url,location.href).href;if(String(target).toLowerCase()==='_blank')return globalThis.open(url,'_blank');const oldURL=location.href;updateLocation(url);if(oldURL.split('#')[0]===url.split('#')[0]&&oldURL!==url){const event=new OrynEvent('hashchange');event.oldURL=oldURL;event.newURL=url;globalThis.dispatchEvent(event);}return null;};
  const submitForm=form=>{if(!(form.hasAttribute('novalidate')||form.reportValidity()))return false;const event=new OrynEvent('submit',{bubbles:true,cancelable:true});if(!form.dispatchEvent(event))return false;let url=new OrynURL(form.getAttribute('action')||location.href,location.href);const method=(form.getAttribute('method')||'get').toLowerCase();if(method==='get'){const values=new OrynFormData(form);for(const [key,value] of values)url.searchParams.append(key,value instanceof OrynBlob?value.name||'blob':value);}navigateTarget(url.href,form.getAttribute('target')||'_self');return true;};
  const pageProxies=new Map();let contextSelfClosed=false;
  const pageProxy=pageId=>{if(pageId===null||pageId===undefined)return null;if(pageProxies.has(pageId))return pageProxies.get(pageId);const proxy={__orynPageId:pageId,__closed:false,get closed(){return this.__closed;},focus(){contextCall({operation:'activate_page',source_page_id:contextPageId,target_page_id:pageId});traceContext('page_activated',{page_id:pageId});},close(){contextCall({operation:'close_page',source_page_id:contextPageId,target_page_id:pageId});this.__closed=true;traceContext('page_closed',{page_id:pageId});},postMessage(data,targetOrigin='*'){contextCall({operation:'post_message',source_page_id:contextPageId,target_page_id:pageId,target_origin:String(targetOrigin),data});traceContext('opener_message',{source_page_id:contextPageId,target_page_id:pageId,origin:String(targetOrigin)});}};pageProxies.set(pageId,proxy);return proxy;};
  globalThis.__oryn_configure_context=(pageId,openerPageId)=>{contextPageId=pageId;contextOpenerPageId=openerPageId;globalThis.opener=pageProxy(openerPageId);contextCall({operation:'commit_page',page_id:contextPageId,url:location.href});};
  globalThis.__oryn_configure_frame_context=pageId=>{contextPageId=pageId;contextOpenerPageId=null;globalThis.opener=null;};
  globalThis.open=(url='',_target='_blank',_features='')=>{const response=contextCall({operation:'open_page',opener_page_id:contextPageId,url:url?new OrynURL(url,location.href).href:null});if(response&&response.result==='page_opened'){traceContext('page_created',{page_id:response.page_id,opener_page_id:contextPageId});return pageProxy(response.page_id);}if(response&&response.result==='page_denied')traceContext('capability_failure',{capability:'popup.open',detail:response.reason});return null;};
  globalThis.close=()=>{contextSelfClosed=true;if(contextPageId!==null)traceContext('page_closed',{page_id:contextPageId});};
  Object.defineProperty(globalThis,'closed',{get:()=>contextSelfClosed});
  const webSockets=new Set();
  class OrynWebSocket {
    constructor(url,protocols=[]){this.url=new OrynURL(url,location.href).href;this.protocol='';this.extensions='';this.binaryType='blob';this.bufferedAmount=0;this.readyState=OrynWebSocket.CONNECTING;this.__listeners=new Map();if(typeof protocols==='string')protocols=[protocols];const response=contextCall({operation:'web_socket_open',page_id:contextPageId,url:this.url,protocols:[...protocols].map(String)});if(!response||response.result!=='web_socket_opened')throw new Error('WebSocket open failed');this.__socketId=response.socket_id;this.protocol=response.protocol;webSockets.add(this);queueMicrotask(()=>{this.readyState=OrynWebSocket.OPEN;this.__event('open');});}
    addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list);}
    removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback));}
    __event(type,values={}){const event=new OrynEvent(type);Object.assign(event,values);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event);}
    send(data){if(this.readyState!==OrynWebSocket.OPEN)throw new Error('InvalidStateError');let frame;if(typeof data==='string')frame={kind:'text',value:data};else frame={kind:'binary',base64:bytesToBase64(bodyBytes(data))};const bytes=typeof data==='string'?new TextEncoder().encode(data).length:bodyBytes(data).length;this.bufferedAmount+=bytes;contextCall({operation:'web_socket_send',page_id:contextPageId,socket_id:this.__socketId,frame});traceContext('web_socket_frame',{socket_id:this.__socketId,direction:'outbound',frame_type:frame.kind,bytes});this.bufferedAmount=0;}
    close(code=1000,reason=''){if(this.readyState>=OrynWebSocket.CLOSING)return;this.readyState=OrynWebSocket.CLOSING;contextCall({operation:'web_socket_close',page_id:contextPageId,socket_id:this.__socketId,code:Number(code),reason:String(reason)});this.readyState=OrynWebSocket.CLOSED;webSockets.delete(this);this.__event('close',{code:Number(code),reason:String(reason),wasClean:true});}
    __poll(){if(this.readyState!==OrynWebSocket.OPEN)return;const response=contextCall({operation:'web_socket_poll',page_id:contextPageId,socket_id:this.__socketId,max_frames:32});if(!response||response.result!=='web_socket_events')return;for(const frame of response.frames){const bytes=frame.kind==='text'?new TextEncoder().encode(frame.value).length:frame.kind==='binary'?base64ToBytes(frame.base64).length:0;traceContext('web_socket_frame',{socket_id:this.__socketId,direction:'inbound',frame_type:frame.kind,bytes});if(frame.kind==='text')this.__event('message',{data:frame.value,origin:new OrynURL(this.url).origin});else if(frame.kind==='binary'){const data=base64ToBytes(frame.base64);this.__event('message',{data:this.binaryType==='arraybuffer'?data.buffer:new OrynBlob([data]),origin:new OrynURL(this.url).origin});}else if(frame.kind==='close'){this.readyState=OrynWebSocket.CLOSED;webSockets.delete(this);this.__event('close',{code:frame.code??1005,reason:frame.reason||'',wasClean:true});}}}
  }
  Object.assign(OrynWebSocket,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});Object.assign(OrynWebSocket.prototype,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});globalThis.WebSocket=OrynWebSocket;
  globalThis.__oryn_poll_context=()=>{if(contextPageId===null)return;const response=contextCall({operation:'poll_events',page_id:contextPageId,after_event_id:contextEventId,max_events:32});if(response&&response.result==='events')for(const record of response.events){contextEventId=Math.max(contextEventId,record.event_id);const item=record.event;if(item.kind==='storage'){const event=new OrynEvent('storage');event.key=item.change.key;event.oldValue=item.change.old_value;event.newValue=item.change.new_value;event.storageArea=item.change.area==='local'?localStorage:sessionStorage;event.url=location.href;globalThis.dispatchEvent(event);}else if(item.kind==='message'){const event=new OrynEvent('message');event.data=item.data;event.origin=item.source_origin;event.source=pageProxy(item.source_page_id);globalThis.dispatchEvent(event);}else if(item.kind==='page_lifecycle'&&item.event?.kind==='closed'){const proxy=pageProxies.get(item.event.page_id);if(proxy)proxy.__closed=true;}}for(const socket of [...webSockets])socket.__poll();};
  if(typeof globalThis.__oryn_fetch_sync==='function')globalThis.fetch=(input,init={})=>new Promise((resolve,reject)=>queueMicrotask(()=>{try{const request=new OrynRequest(input,init);if(request.signal.aborted)throw request.signal.reason;const requestHeaders=[...request.headers];const bytes=bodyBytes(request.body);traceContext('request_started',{method:request.method,url:request.url,bytes:bytes.length});const raw=globalThis.__oryn_fetch_sync(request.url,request.method,bytesToBase64(bytes),JSON.stringify(requestHeaders),'base64',request.credentials);if(request.signal.aborted)throw request.signal.reason;const response=JSON.parse(raw);traceContext('response_received',{url:response.url,status:response.status,bytes:base64ToBytes(response.body_base64).length});resolve(new OrynResponse(base64ToBytes(response.body_base64),{status:response.status,url:response.url,redirected:response.url!==request.url,headers:response.headers,bodyChunks:(response.body_chunks_base64||[]).map(base64ToBytes)}));}catch(error){reject(error);}}));
  class OrynXMLHttpRequest {
    constructor(){this.readyState=0;this.status=0;this.statusText='';this.responseType='';this.response=null;this.responseText='';this.responseURL='';this.timeout=0;this.withCredentials=false;this.upload={};this.__headers=new OrynHeaders();this.__responseHeaders=new OrynHeaders();this.__listeners=new Map();this.__aborted=false;}
    addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list);}
    removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback));}
    __event(type,init={}){const event=new OrynEvent(type);Object.assign(event,init);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event);}
    __state(value){this.readyState=value;traceContext('xhr_state_changed',{url:this.__url||'',ready_state:value});this.__event('readystatechange');}
    open(method,url,async=true,_user,_password){if(async===false){traceContext('capability_failure',{capability:'xhr.synchronous_main_thread',detail:'synchronous main-thread XHR is unsupported'});throw new Error('synchronous main-thread XHR is unsupported');}this.__method=String(method).toUpperCase();this.__url=new OrynURL(url,location.href).href;this.__aborted=false;this.__state(1);}
    setRequestHeader(name,value){if(this.readyState!==1)throw new Error('InvalidStateError');this.__headers.append(name,value);}
    getResponseHeader(name){return this.__responseHeaders.get(name);}
    getAllResponseHeaders(){return [...this.__responseHeaders].map(([name,value])=>name+': '+value+'\r\n').join('');}
    overrideMimeType(value){this.__mime=String(value);}
    abort(){this.__aborted=true;this.status=0;this.response=null;this.responseText='';this.__state(0);this.__event('abort');this.__event('loadend');}
    send(body=null){if(this.readyState!==1)throw new Error('InvalidStateError');this.__event('loadstart');const started=Date.now();queueMicrotask(()=>{if(this.__aborted)return;try{const bytes=bodyBytes(body),raw=globalThis.__oryn_fetch_sync(this.__url,this.__method,bytesToBase64(bytes),JSON.stringify([...this.__headers]),'base64',this.withCredentials?'include':'same-origin');if(this.__aborted)return;if(this.timeout>0&&Date.now()-started>this.timeout){this.status=0;this.__event('timeout');this.__event('loadend');return;}const result=JSON.parse(raw),responseBytes=base64ToBytes(result.body_base64);this.status=result.status;this.responseURL=result.url;this.__responseHeaders=new OrynHeaders(result.headers);this.__state(2);this.__state(3);this.responseText=new TextDecoder().decode(responseBytes);this.response=this.responseType==='arraybuffer'?responseBytes.buffer.slice(0):this.responseType==='blob'?new OrynBlob([responseBytes],{type:this.__responseHeaders.get('content-type')||''}):this.responseType==='json'?JSON.parse(this.responseText||'null'):this.responseText;this.__event('progress',{lengthComputable:true,loaded:responseBytes.length,total:responseBytes.length});this.__state(4);this.__event('load');this.__event('loadend');}catch(error){this.status=0;this.__event('error',{error});this.__event('loadend');}});}
  }
  Object.assign(OrynXMLHttpRequest,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});Object.assign(OrynXMLHttpRequest.prototype,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});globalThis.XMLHttpRequest=OrynXMLHttpRequest;
  if(typeof globalThis.__oryn_crypto_sync==='function')globalThis.crypto={getRandomValues(array){if(!ArrayBuffer.isView(array)||array instanceof DataView)throw new TypeError('expected integer typed array');const bytes=base64ToBytes(globalThis.__oryn_crypto_sync('random',String(array.byteLength)));new Uint8Array(array.buffer,array.byteOffset,array.byteLength).set(bytes);return array;},randomUUID(){const bytes=this.getRandomValues(new Uint8Array(16));bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');return hex.slice(0,8)+'-'+hex.slice(8,12)+'-'+hex.slice(12,16)+'-'+hex.slice(16,20)+'-'+hex.slice(20);},subtle:{digest(algorithm,data){const name=typeof algorithm==='string'?algorithm:algorithm&&algorithm.name;if(String(name).toUpperCase().replace('-','')!=='SHA256')return Promise.reject(new Error('unsupported digest'));const bytes=bodyBytes(data);return Promise.resolve(base64ToBytes(globalThis.__oryn_crypto_sync('sha256',bytesToBase64(bytes))).buffer);}}};
  globalThis.queueMicrotask=fn=>microtasks.push(fn);
  globalThis.setTimeout=(fn,delay=0,...args)=>{const id=timers.length+1;timers.push({id,fn,args,delay:Math.max(0,Number(delay)||0),cancelled:false});return id;};
  globalThis.clearTimeout=id=>{const timer=timers.find(item=>item.id===id);if(timer)timer.cancelled=true;};
  globalThis.setInterval=(fn,_delay=0,...args)=>globalThis.setTimeout(fn,0,...args);globalThis.clearInterval=globalThis.clearTimeout;
  globalThis.performance={now:()=>0};globalThis.requestAnimationFrame=fn=>globalThis.setTimeout(()=>fn(performance.now()),0);globalThis.cancelAnimationFrame=globalThis.clearTimeout;
  globalThis.MessageChannel=class { constructor(){const first={onmessage:null};const second={onmessage:null};first.postMessage=data=>globalThis.setTimeout(()=>second.onmessage&&second.onmessage({data}),0);second.postMessage=data=>globalThis.setTimeout(()=>first.onmessage&&first.onmessage({data}),0);this.port1=first;this.port2=second;} };
  globalThis.__oryn_drain=(advance=false)=>{__oryn_poll_context();let guard=0;while(guard++<10000){while(microtasks.length)microtasks.shift()();let index=timers.findIndex(item=>!item.cancelled&&item.delay<=0);if(index<0&&advance)index=timers.findIndex(item=>!item.cancelled);if(index<0)break;const [timer]=timers.splice(index,1);if(timer&&!timer.cancelled)timer.fn(...timer.args);}if(guard>=10000)throw new Error('task drain limit exceeded');__oryn_poll_context();};
  globalThis.__oryn_install=raw=>{const ids=[];(function collect(node){if(node.id)ids.push(node.id.slot);for(const child of node.children||[])collect(child);})(raw);nextNodeSlot=(ids.length?Math.max(...ids)+1:0);globalThis.document=new OrynDocument(raw);globalThis.window=globalThis;globalThis.self=globalThis;globalThis.global=globalThis;globalThis.navigator={userAgent:'Oryn Native/0.1'};upgradeTree(document);};
  globalThis.__oryn_dispatch_load=()=>{document.readyState='complete';globalThis.dispatchEvent(new OrynEvent('load'));};
  globalThis.__oryn_install_inline_handlers=()=>{for(const node of descendants(document)){for(const [name,source] of Object.entries(node.__attrs||{})){if(/^on[a-z]+$/i.test(name)){try{node[name.toLowerCase()]=new Function('event',String(source));}catch(error){console.error(error);}}}}};
  const installFrameDocument=(frame,record)=>{const retained=new Map();if(frame.contentDocument)for(const node of [frame.contentDocument,...descendants(frame.contentDocument)]){const owner=node.getAttribute&&node.getAttribute('data-oryn-frame-owner'),slot=node.getAttribute&&node.getAttribute('data-oryn-frame-slot'),generation=node.getAttribute&&node.getAttribute('data-oryn-frame-generation');if(owner&&slot!==null&&generation!==null&&node.__id)retained.set(owner+':'+slot+':'+generation,node.__id);}const child=new OrynDocument(record.document);const assignIds=node=>{const owner=node.getAttribute&&node.getAttribute('data-oryn-frame-owner'),slot=node.getAttribute&&node.getAttribute('data-oryn-frame-slot'),generation=node.getAttribute&&node.getAttribute('data-oryn-frame-generation'),key=owner&&slot!==null&&generation!==null?owner+':'+slot+':'+generation:null;if(key&&retained.has(key))node.__id=retained.get(key);else if(!node.__id)node.__id={slot:nextNodeSlot++,generation:0};for(const item of node.childNodes)assignIds(item);};assignIds(child);frame.contentDocument=child;frame.contentWindow={document:child,frameElement:frame,location:new OrynURL(record.source,location.href)};};
  globalThis.__oryn_install_frame_documents=resources=>{for(const frame of descendants(document).filter(node=>node.__tag==='iframe')){const instance=frame.getAttribute('data-oryn-frame-instance');if(!instance||!Object.prototype.hasOwnProperty.call(resources,instance)){frame.contentDocument=null;frame.contentWindow=null;continue;}installFrameDocument(frame,resources[instance]);}};
  globalThis.__oryn_update_frame_document=(instance,record)=>{const frame=descendants(document).find(node=>node.__tag==='iframe'&&node.getAttribute('data-oryn-frame-instance')===instance);if(!frame)throw new Error('frame instance is unavailable');installFrameDocument(frame,record);};
  globalThis.__oryn_export=()=>document.export();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_classic_inline_scripts_in_a_persistent_realm() {
        let mut document = ExecutableDocument::load(
            "<!doctype html><script>globalThis.answer = 40</script><script>answer += 2</script>",
        )
        .expect("load executable document");
        document
            .execute("if (answer !== 42) throw new Error('realm was not persistent')")
            .expect("persistent realm");
    }

    #[test]
    fn reports_only_unavailable_external_scripts_as_unsupported() {
        let document = ExecutableDocument::load(
            "<!doctype html><script src=app.js></script><script type=module>export {}</script>",
        )
        .expect("load document");
        assert_eq!(document.diagnostics.len(), 1);
        assert!(
            document
                .diagnostics
                .iter()
                .all(|item| item.support == SupportLevel::Unsupported)
        );
    }

    #[test]
    fn page_script_failure_is_diagnostic_and_does_not_abort_navigation() {
        let mut document = ExecutableDocument::load(
            "<!doctype html><title>Still loaded</title><button>Continue</button><script>const = invalid</script>",
        )
        .expect("page-authored syntax failure must not abort the document");
        assert_eq!(
            document
                .host
                .evaluate_string("document.title")
                .expect("evaluate title"),
            "Still loaded"
        );
        assert!(document.diagnostics.iter().any(|item| {
            item.capability == "html.script.classic_inline.execution"
                && item.support == SupportLevel::Partial
        }));
    }

    #[test]
    fn open_shadow_roots_are_semantic_and_closed_roots_are_isolated() {
        let document = ExecutableDocument::load(
            r#"<open-box></open-box><closed-box></closed-box><script>
              class OpenBox extends HTMLElement { connectedCallback(){ const root=this.attachShadow({mode:'open'});root.innerHTML='<button id="open-action">Open action</button>'; } }
              class ClosedBox extends HTMLElement { connectedCallback(){ const root=this.attachShadow({mode:'closed'});root.innerHTML='<button id="closed-action">Closed action</button>'; } }
              customElements.define('open-box',OpenBox);customElements.define('closed-box',ClosedBox);
            </script>"#,
        )
        .expect("load shadow fixture");
        assert!(node_by_id(&document, "open-action").is_some());
        assert!(node_by_id(&document, "closed-action").is_none());
    }

    #[test]
    fn class_style_mutation_and_platform_primitives_execute() {
        let mut document = ExecutableDocument::load(
            r#"<div id=result></div><script>
              const result=document.querySelector('#result');let mutations=0;
              new MutationObserver(records=>mutations+=records.length).observe(result,{attributes:true});
              result.classList.add('ready');result.style.display='block';
              const url=new URL('/items?q=one','https://example.test/base');url.searchParams.set('q','two');
              const value=new TextDecoder().decode(new TextEncoder().encode('Oryn ✓'));
              queueMicrotask(()=>result.textContent=url.href+' '+value+' '+mutations);
            </script>"#,
        )
        .expect("load platform primitives");
        document.drain_tasks().expect("drain mutation callbacks");
        let text = text_by_id(&document, "result").expect("result text");
        assert!(text.contains("https://example.test/items?q=two Oryn ✓"));
        assert!(text.ends_with(" 2"), "{text}");
    }

    #[test]
    fn document_fragments_support_browser_style_batch_insertion() {
        let mut document = ExecutableDocument::load(
            r#"<main id=result></main><script>
              const fragment=document.createDocumentFragment();
              const first=document.createElementNS('http://www.w3.org/1999/xhtml','button');
              first.textContent='First';
              fragment.appendChild(first);
              fragment.appendChild(document.createTextNode('Second'));
              document.querySelector('#result').appendChild(fragment);
            </script>"#,
        )
        .expect("load document fragment fixture");
        document.drain_tasks().expect("drain fragment mutations");
        assert_eq!(
            text_by_id(&document, "result").as_deref(),
            Some("FirstSecond")
        );
    }

    #[test]
    fn wpt_pinned_supported_url_and_encoding_vectors() {
        // WPT 719d5e38fdd0903a18ed9007aba816c98cc491e0:
        // url/urlsearchparams-constructor.any.js (basic, leading '?', '+') and
        // encoding/api-basics.any.js (UTF-8 default input and round trip).
        let mut document = ExecutableDocument::load("<!doctype html>").expect("load document");
        let values = document
            .evaluate(
                r#"JSON.stringify([
                  new URLSearchParams().toString(),
                  new URLSearchParams('?a=b').toString(),
                  new URLSearchParams('a=b+c').get('a'),
                  new TextEncoder().encode().length,
                  new TextDecoder().decode(new TextEncoder().encode('z¢水𝄞'))
                ])"#,
            )
            .expect("execute supported WPT vectors");
        assert_eq!(values, r#"["","a=b","b c",0,"z¢水𝄞"]"#);
    }

    #[test]
    fn hostless_hierarchical_urls_retain_their_scheme() {
        let expected = "fixture:///tmp/oryn-g2r-smoke.html";
        let mut document = ExecutableDocument::load_with_environment(
            r#"<button type=button id=go onclick="document.querySelector('#status').textContent='changed'">Mutate</button><output id=status>before</output>"#,
            &BTreeMap::new(),
            None,
            Some(Url::parse(expected).expect("fixture URL")),
        )
        .expect("load document with hostless hierarchical URL");

        assert_eq!(document.location().expect("initial location"), expected);
        document.click("#go").expect("click mutation control");
        assert_eq!(document.location().expect("retained location"), expected);
        assert_eq!(text_by_id(&document, "status").as_deref(), Some("changed"));
    }

    #[test]
    fn event_handlers_mutate_the_oryn_owned_dom() {
        let mut document = ExecutableDocument::load(
            r#"<output id=count>0</output><button id=add>Add</button><script>
              let count=0; document.querySelector('#add').addEventListener('click',()=>{
                document.querySelector('#count').textContent=String(++count);
              });
            </script>"#,
        )
        .expect("load document");
        document.click("#add").expect("click");
        assert_eq!(text_by_id(&document, "count").as_deref(), Some("1"));
    }

    #[test]
    fn forms_microtasks_and_timers_are_deterministic() {
        let mut document = ExecutableDocument::load(
            r#"<input id=name><output id=result></output><script>
              const input=document.querySelector('#name');
              input.addEventListener('input',()=>queueMicrotask(()=>setTimeout(()=>{
                document.querySelector('#result').textContent=input.value;
              },10)));
            </script>"#,
        )
        .expect("load document");
        document.type_text("#name", "Oryn").expect("type");
        document.drain_tasks().expect("drain");
        assert_eq!(text_by_id(&document, "result").as_deref(), Some("Oryn"));
    }

    #[test]
    fn form_constraint_validation_controls_default_submission() {
        let mut document = ExecutableDocument::load(
            r#"<form id=form><input id=email type=email required><button id=submit>Submit</button></form><output id=count>0</output><script>
              let submits=0;document.querySelector('#form').addEventListener('submit',event=>{event.preventDefault();document.querySelector('#count').textContent=String(++submits);});
            </script>"#,
        )
        .expect("load validated form");
        document.click("#submit").expect("invalid activation");
        assert_eq!(text_by_id(&document, "count").as_deref(), Some("0"));
        document
            .type_text("#email", "agent@example.test")
            .expect("type valid email");
        document.click("#submit").expect("valid activation");
        assert_eq!(text_by_id(&document, "count").as_deref(), Some("1"));
    }

    #[test]
    fn select_options_expose_browser_style_collection_and_selection_state() {
        let mut document = ExecutableDocument::load(
            r#"<select id=category><option value=all>All</option><option id=tools value=tools>Tools</option></select><output id=result></output><script>
              const select=document.querySelector('#category');
              select.value='tools';
              document.querySelector('#result').textContent=[
                select.options.length,
                select.options.item(1).textContent,
                select.options.namedItem('tools').value,
                select.selectedIndex,
                select.selectedOptions.length,
                select.value
              ].join('|');
            </script>"#,
        )
        .expect("load select fixture");
        assert_eq!(
            text_by_id(&document, "result").as_deref(),
            Some("2|Tools|tools|1|1|tools")
        );
        let category = node_by_id(&document, "category").expect("category select");
        document
            .apply_action(category, SemanticAction::Select, Some("All"))
            .expect("select option by displayed label");
        assert_eq!(
            document
                .evaluate("document.querySelector('#category').value")
                .expect("read selected option value"),
            "all"
        );
    }

    #[test]
    fn executes_vue_style_controlled_fixture() {
        let mut document = ExecutableDocument::load(include_str!(
            "../../../test-harness/scenarios/spa/vue-tasks.html"
        ))
        .expect("load Vue-style fixture");
        document.type_text("#task", "Ship G1").expect("type task");
        document.click("#add").expect("add task");
        assert!(text_by_id(&document, "tasks").is_some_and(|text| text.contains("Ship G1")));
    }

    #[test]
    fn executes_svelte_style_controlled_fixture() {
        let mut document = ExecutableDocument::load(include_str!(
            "../../../test-harness/scenarios/spa/svelte-tasks.html"
        ))
        .expect("load Svelte-style fixture");
        document.click("#increment").expect("increment");
        assert_eq!(text_by_id(&document, "count").as_deref(), Some("Items: 1"));
        let decrement = document
            .document
            .dom
            .iter()
            .find(|(_, node)| node.attributes.get("id").map(String::as_str) == Some("decrement"))
            .map(|(_, node)| node);
        assert!(decrement.is_some_and(|node| !node.attributes.contains_key("disabled")));
    }

    #[test]
    fn structural_churn_never_preserves_a_removed_node_identity() {
        let mut document = ExecutableDocument::load(
            r#"<main><button id=replace>Replace</button><span id=old>Old</span></main><script>
              document.querySelector('#replace').addEventListener('click',()=>{
                document.querySelector('#old').remove();
                const replacement=document.createElement('span'); replacement.id='new';
                replacement.textContent='New'; document.querySelector('main').append(replacement);
              });
            </script>"#,
        )
        .expect("load document");
        let old = node_by_id(&document, "old").expect("old node");
        let retained = node_by_id(&document, "replace").expect("retained node");
        document.click("#replace").expect("replace");
        let new = node_by_id(&document, "new").expect("new node");
        assert!(document.document.dom.get(old).is_err());
        assert_eq!(node_by_id(&document, "replace"), Some(retained));
        assert_ne!(old, new);
    }

    fn text_by_id(document: &ExecutableDocument, id: &str) -> Option<String> {
        document
            .document
            .dom
            .iter()
            .find(|(_, node)| node.attributes.get("id").map(String::as_str) == Some(id))
            .map(|(node, _)| document.document.dom.text_content(node).expect("text"))
    }

    fn node_by_id(document: &ExecutableDocument, id: &str) -> Option<NodeId> {
        document
            .document
            .dom
            .iter()
            .find(|(_, node)| node.attributes.get("id").map(String::as_str) == Some(id))
            .map(|(node, _)| node)
    }

    #[test]
    fn dynamic_import_uses_the_reusable_module_map() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "./chunk.js".into(),
            "export const label='Chunk loaded'".into(),
        );
        let mut document = ExecutableDocument::load_with_resources(
            "<output id=result>pending</output><script type=module>import('./chunk.js').then(module=>document.querySelector('#result').textContent=module.label)</script>",
            &resources,
        )
        .expect("load dynamic module fixture");
        document.drain_tasks().expect("drain module promise");
        assert_eq!(
            document
                .evaluate("document.querySelector('#result').textContent")
                .expect("read dynamic module result"),
            "Chunk loaded"
        );
    }

    #[test]
    fn range_selection_traversal_and_contenteditable_are_available() {
        let mut document = ExecutableDocument::load(
            r#"<main contenteditable><p>Alpha</p><p>Beta</p></main><output id=result></output><script>
              const main=document.querySelector('main');
              const walker=document.createTreeWalker(main,NodeFilter.SHOW_ELEMENT);
              const names=[];let node;while(node=walker.nextNode())names.push(node.tagName);
              const range=document.createRange();range.selectNodeContents(main.querySelector('p'));
              getSelection().addRange(range);
              document.querySelector('#result').textContent=main.isContentEditable+'|'+getSelection().toString()+'|'+names.join(',');
            </script>"#,
        )
        .expect("load editing fixture");
        assert_eq!(
            document
                .evaluate("document.querySelector('#result').textContent")
                .expect("read editing APIs"),
            "true|Alpha|P,P"
        );
    }
}
