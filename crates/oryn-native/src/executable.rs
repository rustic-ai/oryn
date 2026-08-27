use std::collections::BTreeMap;

use markup5ever::{LocalName, QualName, ns};
use oryn_common::v2::SemanticAction;
use oryn_common::v2::{CapabilityDiagnostic, ExecutionDomain, SupportLevel};
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub message: String,
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
        let document = parse(html);
        let initial = serde_json::to_string(&JsNode::from_dom(&document.dom, document.document)?)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let mut host = match (broker, base_url.clone()) {
            (Some(broker), Some(base_url)) => RawV8Host::new_with_network(broker, base_url),
            _ => RawV8Host::new(),
        };
        host.execute(DOM_BOOTSTRAP)?;
        host.execute(&format!("__oryn_install({initial})"))?;
        if let Some(base_url) = base_url {
            host.execute(&format!(
                "__oryn_set_location({})",
                json_string(base_url.as_str())?
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
        let local_storage = json_string(local_storage)?;
        let session_storage = json_string(session_storage)?;
        host.execute(&format!(
            "__oryn_restore_storage(JSON.parse({local_storage}),JSON.parse({session_storage}))"
        ))?;
        let frame_resources = serde_json::to_string(resources)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        host.execute(&format!("__oryn_install_frames({frame_resources})"))?;

        let mut diagnostics = Vec::new();
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
        let scripts = document
            .dom
            .iter()
            .filter_map(|(id, node)| {
                let NodeKind::Element { name } = &node.kind else {
                    return None;
                };
                (name.local.as_ref() == "script").then(|| {
                    (
                        node.attributes.clone(),
                        document.dom.text_content(id).unwrap_or_default(),
                    )
                })
            })
            .collect::<Vec<_>>();
        for (attributes, source) in scripts {
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
                if let Err(error) = host.execute_module(resource_name, module_source, resources) {
                    diagnostics.push(script_failure("module", &error));
                }
                continue;
            }
            if let Some(script_type) = attributes.get("type")
                && !matches!(
                    script_type.as_str(),
                    "" | "text/javascript" | "application/javascript" | "text/babel"
                )
            {
                diagnostics.push(unsupported(&format!("html.script.type.{script_type}")));
                continue;
            }
            if attributes.get("type").map(String::as_str) == Some("text/babel") {
                let source = json_string(&source)?;
                match host.evaluate_string(&format!(
                    "Babel.transform({source},{{presets:['react']}}).code"
                )) {
                    Ok(transformed) => {
                        if let Err(error) = host.execute(&transformed) {
                            diagnostics.push(script_failure("babel", &error));
                        }
                    }
                    Err(error) => diagnostics.push(script_failure("babel", &error)),
                }
            } else if let Some(src) = attributes.get("src") {
                match resources.get(src) {
                    Some(source) => {
                        if let Err(error) = host.execute(source) {
                            diagnostics.push(script_failure("classic_external", &error));
                        }
                    }
                    None => diagnostics.push(unsupported("html.script.external.unavailable")),
                }
            } else {
                if let Err(error) = host.execute(&source) {
                    diagnostics.push(script_failure("classic_inline", &error));
                }
            }
        }
        host.execute("__oryn_install_inline_handlers()")?;
        host.execute("document.__dispatchReady()")?;
        host.execute("__oryn_dispatch_load()")?;
        drain_host(&mut host)?;
        host.execute("if(globalThis.ReactDOM&&ReactDOM.flushSync)ReactDOM.flushSync(()=>{})")?;
        drain_host(&mut host)?;

        let mut executable = Self {
            document,
            diagnostics,
            host,
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
        self.sync_dom()
    }

    pub fn apply_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        let target = serde_json::to_string(&target)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let value = json_string(value.unwrap_or_default())?;
        let operation = match action {
            SemanticAction::Click => "node.click()".into(),
            SemanticAction::Type => format!("node.value={value};node.dispatchEvent(new Event('input',{{bubbles:true}}));node.dispatchEvent(new Event('change',{{bubbles:true}}))"),
            SemanticAction::Clear => "node.value='';node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Check => "node.checked=true;node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Uncheck => "node.checked=false;node.dispatchEvent(new Event('change',{bubbles:true}))".into(),
            SemanticAction::Select => format!("node.value={value};node.dispatchEvent(new Event('change',{{bubbles:true}}))"),
            SemanticAction::Focus => "node.focus()".into(),
            SemanticAction::Hover => "node.dispatchEvent(new Event('mouseover',{bubbles:true}))".into(),
            SemanticAction::Submit => "node.dispatchEvent(new Event('submit',{bubbles:true}))".into(),
            _ => return Err(HostError::Execution(format!("unsupported native executable action {action:?}"))),
        };
        self.host.execute(&format!("{{const node=__oryn_get({target});if(!node)throw new Error('stale node');{operation};}}"))?;
        drain_host(&mut self.host)?;
        self.sync_dom()
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

const DOM_BOOTSTRAP: &str = r#"
(() => {
  let nextNodeSlot=0;
  class OrynEvent {
    constructor(type, init = {}) { this.type=String(type); this.bubbles=!!init.bubbles; this.cancelable=!!init.cancelable;this.composed=!!init.composed;this.detail=init.detail;this.defaultPrevented=false; this.target=null; this.currentTarget=null; }
    preventDefault() { this.defaultPrevented=true; }
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
    remove() { if(!this.parentNode)return; const parent=this.parentNode; parent.childNodes=parent.childNodes.filter(node=>node!==this); this.parentNode=null; notifyMutation(parent); }
    addEventListener(type, callback) { type=String(type);const list=this.__listeners.get(type)||[];list.push(callback);this.__listeners.set(type,list);if(type==='click'&&this.__kind==='element')this.__attrs['data-oryn-click-listener']=''; }
    removeEventListener(type, callback) { this.__listeners.set(type,(this.__listeners.get(type)||[]).filter(item=>item!==callback)); }
    dispatchEvent(event) { if(!(event instanceof OrynEvent))event=new OrynEvent(event.type||event); if(!event.target)event.target=this; event.currentTarget=this; for(const fn of [...(this.__listeners.get(event.type)||[])])fn.call(this,event); const handler=this['on'+event.type];if(typeof handler==='function')handler.call(this,event); if(event.bubbles&&this.parentNode)this.parentNode.dispatchEvent(event); return !event.defaultPrevented; }
    click() { if(this.disabled)return;if(['input','textarea','select','button','a'].includes(this.__tag))this.focus();const allowed=this.dispatchEvent(new OrynEvent('click',{bubbles:true,cancelable:true}));if(!allowed)return;if(this.__tag==='label'){const control=descendants(this).find(node=>node.__tag==='input')||(this.getAttribute('for')&&document.getElementById(this.getAttribute('for')));if(control)control.click();return;}if(this.__tag==='input'){const type=(this.getAttribute('type')||'text').toLowerCase();if(type==='checkbox')this.checked=!this.checked;if(type==='radio'){const name=this.getAttribute('name');for(const node of descendants(document))if(node!==this&&node.__tag==='input'&&(node.getAttribute('type')||'').toLowerCase()==='radio'&&node.getAttribute('name')===name)node.checked=false;this.checked=true;}if(type==='checkbox'||type==='radio')this.dispatchEvent(new OrynEvent('change',{bubbles:true}));}if(this.__tag==='button'&&(this.getAttribute('type')||'submit')==='submit'){const form=this.closest('form');if(form&&(form.hasAttribute('novalidate')||form.reportValidity()))form.dispatchEvent(new OrynEvent('submit',{bubbles:true,cancelable:true}));} }
    focus() { const active=document&&document.activeElement;if(active&&active!==this&&active!==document.body)active.blur();this.setAttribute('data-oryn-focused','');this.dispatchEvent(new OrynEvent('focus')); }
    blur() { if(!this.hasAttribute('data-oryn-focused'))return;this.removeAttribute('data-oryn-focused');this.dispatchEvent(new OrynEvent('blur')); }
    setAttribute(name,value) { name=String(name);value=String(value);const old=this.getAttribute(name);this.__attrs[name]=value;if(/^on[a-z]+$/i.test(name)){try{this[name.toLowerCase()]=new Function('event',value);}catch(error){console.error(error);}}if(old!==value&&this.attributeChangedCallback&&(this.constructor.observedAttributes||[]).includes(name))queueMicrotask(()=>this.attributeChangedCallback(name,old,value));notifyMutation(this); }
    setAttributeNS(_namespace,name,value) { this.setAttribute(name,value); }
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(this.__attrs,name)?this.__attrs[name]:null; }
    getAttributeNS(_namespace,name) { return this.getAttribute(name); }
    hasAttribute(name) { return Object.prototype.hasOwnProperty.call(this.__attrs,name); }
    get attributes() { const node=this;const entries=Object.entries(this.__attrs).map(([name,value])=>({name,nodeName:name,value,nodeValue:value,specified:true,ownerElement:node,expando:false}));const named=Object.fromEntries(entries.map(attribute=>[attribute.name,attribute]));return Object.assign(entries,{item:index=>entries[index]||null,getNamedItem:name=>named[String(name)]||null,...named}); }
    removeAttribute(name) { const old=this.getAttribute(name);delete this.__attrs[name];if(old!==null&&this.attributeChangedCallback&&(this.constructor.observedAttributes||[]).includes(String(name)))queueMicrotask(()=>this.attributeChangedCallback(String(name),old,null));notifyMutation(this); }
    removeAttributeNS(_namespace,name) { this.removeAttribute(name); }
    querySelector(selector) { return query(this,selector,false)[0]||null; }
    querySelectorAll(selector) { return query(this,selector,false); }
    getElementById(id) { return this.querySelector('#'+id); }
    closest(selector) { let node=this;while(node){if(matches(node,selector))return node;node=node.parentNode;}return null; }
    reset() { for(const node of descendants(this)){if(['input','textarea','select'].includes(node.__tag)){node.value='';node.checked=false;}}this.dispatchEvent(new OrynEvent('reset',{bubbles:true})); }
    checkValidity() { if(this.__tag==='form')return descendants(this).filter(node=>['input','textarea','select'].includes(node.__tag)).every(node=>node.checkValidity());if(!['input','textarea','select'].includes(this.__tag)||this.disabled)return true;const value=this.value;this.validationMessage='';if(this.hasAttribute('required')&&!value&&!this.checked)this.validationMessage='Please fill out this field.';else if(this.getAttribute('type')==='email'&&value&&!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(value))this.validationMessage='Please enter an email address.';else if(this.hasAttribute('minlength')&&value.length<Number(this.getAttribute('minlength')))this.validationMessage='Value is too short.';return !this.validationMessage; }
    reportValidity() { if(this.__tag==='form'){let valid=true;for(const node of descendants(this).filter(node=>['input','textarea','select'].includes(node.__tag)))if(!node.reportValidity())valid=false;return valid;}const valid=this.checkValidity();if(!valid)this.dispatchEvent(new OrynEvent('invalid',{cancelable:true}));return valid; }
    get textContent() { return this.__kind==='text'?this.__data:this.childNodes.map(node=>node.textContent).join(''); }
    set textContent(value) { if(this.__kind==='text')this.__data=String(value); else { const text=new OrynNode({kind:'text',data:String(value)},true); text.parentNode=this; this.childNodes=[text]; } notifyMutation(this); }
    get innerText() { return this.textContent; }
    set innerText(value) { this.textContent=value; }
    get innerHTML() { return this.childNodes.map(serializeNode).join(''); }
    set innerHTML(value) { this.childNodes=parseFragment(String(value));for(const child of this.childNodes)child.parentNode=this;upgradeTree(this); }
    get tagName() { return this.__tag?this.__tag.toUpperCase():undefined; }
    get nodeType() { return this.__kind==='element'?1:this.__kind==='text'?3:this.__kind==='document'?9:11; }
    get nodeName() { return this.__kind==='text'?'#text':this.__kind==='document'?'#document':this.tagName||'#document-fragment'; }
    get ownerDocument() { return this.__kind==='document'?null:globalThis.document||null; }
    get firstChild() { return this.childNodes[0]||null; }
    get lastChild() { return this.childNodes[this.childNodes.length-1]||null; }
    get nextSibling() { if(!this.parentNode)return null;const index=this.parentNode.childNodes.indexOf(this);return this.parentNode.childNodes[index+1]||null; }
    get previousSibling() { if(!this.parentNode)return null;const index=this.parentNode.childNodes.indexOf(this);return index>0?this.parentNode.childNodes[index-1]:null; }
    get children() { return this.childNodes.filter(node=>node.__kind==='element'); }
    get firstElementChild() { return this.children[0]||null; }
    get lastElementChild() { const children=this.children;return children[children.length-1]||null; }
    get nextElementSibling() { let node=this.nextSibling;while(node&&node.__kind!=='element')node=node.nextSibling;return node; }
    get previousElementSibling() { let node=this.previousSibling;while(node&&node.__kind!=='element')node=node.previousSibling;return node; }
    get parentElement() { return this.parentNode&&this.parentNode.__kind==='element'?this.parentNode:null; }
    getElementsByClassName(name) { name=String(name);return descendants(this).filter(node=>node.__kind==='element'&&node.className.split(/\s+/).includes(name)); }
    getElementsByTagName(name) { name=String(name).toLowerCase();return descendants(this).filter(node=>node.__kind==='element'&&(name==='*'||node.__tag===name)); }
    contains(other) { for(let node=other;node;node=node.parentNode)if(node===this)return true;return false; }
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
    cloneNode(deep=false) { const copy=new OrynNode({kind:this.__kind,tag:this.__tag,data:this.__data,attrs:{...this.__attrs}},true);if(deep)for(const child of this.childNodes)copy.appendChild(child.cloneNode(true));return copy; }
    get id() { return this.getAttribute('id')||''; } set id(value) { this.setAttribute('id',value); }
    get className() { return this.getAttribute('class')||''; } set className(value) { this.setAttribute('class',value); }
    get dataset() { const node=this;return new Proxy({}, {get(_target,key){const name='data-'+String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase());return node.getAttribute(name)??undefined;},set(_target,key,value){const name='data-'+String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase());node.setAttribute(name,value);return true;}}); }
    get classList() { const node=this;return {contains:name=>node.className.split(/\s+/).includes(String(name)),add(...names){const set=new Set(node.className.split(/\s+/).filter(Boolean));names.forEach(name=>set.add(String(name)));node.className=[...set].join(' ');},remove(...names){const remove=new Set(names.map(String));node.className=node.className.split(/\s+/).filter(name=>name&&!remove.has(name)).join(' ');},toggle(name,force){const has=this.contains(name);const add=force===undefined?!has:!!force;add?this.add(name):this.remove(name);return add;}}; }
    get style() { if(!this.__style){const declaration=new OrynStyleDeclaration(this);this.__style=new Proxy(declaration,{get(target,key){if(key in target){const value=target[key];return typeof value==='function'?value.bind(target):value;}return target.getPropertyValue(String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase()));},set(target,key,value){if(key in target){target[key]=value;return true;}target.setProperty(String(key).replace(/[A-Z]/g,char=>'-'+char.toLowerCase()),value);return true;}});}return this.__style; }
    get value() { return this.getAttribute('value')||''; } set value(value) { this.setAttribute('value',value); }
    get disabled() { return this.hasAttribute('disabled'); } set disabled(value) { value?this.setAttribute('disabled',''):this.removeAttribute('disabled'); }
    get checked() { return this.hasAttribute('checked'); } set checked(value) { value?this.setAttribute('checked',''):this.removeAttribute('checked'); }
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
  function notifyMutation(target){for(const item of mutationObservers)if(item.active)queueMicrotask(()=>item.callback([{type:'childList',target}],item.observer));}
  class OrynMutationObserver { constructor(callback){this.callback=callback;mutationObservers.push({observer:this,callback,active:false});}observe(){mutationObservers.find(item=>item.observer===this).active=true;}disconnect(){mutationObservers.find(item=>item.observer===this).active=false;}takeRecords(){return [];} }
  const customRegistry=new Map();
  function upgradeTree(root){for(const node of [root,...descendants(root)]){if(node.__kind!=='element')continue;const ctor=customRegistry.get(node.__tag);if(ctor&&!(node instanceof ctor)){const fresh=new ctor();for(const key of Object.keys(fresh)){if(!['__kind','__id','__tag','__data','__attrs','childNodes','parentNode','__listeners','__connected'].includes(key))node[key]=fresh[key];}if(node.__shadow)node.__shadow.parentNode=node;Object.setPrototypeOf(node,ctor.prototype);if(node.connectedCallback&&!node.__connected){node.__connected=true;node.connectedCallback();}}}}
  function serializeNode(node){if(node.__kind==='text')return node.__data;if(node.__kind!=='element')return node.childNodes.map(serializeNode).join('');const attrs=Object.entries(node.__attrs).map(([key,value])=>' '+key+'="'+String(value).replace(/"/g,'&quot;')+'"').join('');return '<'+node.__tag+attrs+'>'+node.childNodes.map(serializeNode).join('')+'</'+node.__tag+'>';}
  function parseFragment(html){const root=new OrynNode({kind:'fragment'},true);const stack=[root];const tokens=html.match(/<!--[\s\S]*?-->|<![^>]*>|<\/?[A-Za-z][^>]*>|[^<]+/g)||[];for(const token of tokens){if(token.startsWith('<!--')||token.startsWith('<!'))continue;if(token.startsWith('</')){if(stack.length>1)stack.pop();continue;}if(token.startsWith('<')){const match=token.match(/^<\s*([\w-]+)([\s\S]*?)\/?\s*>$/);if(!match)continue;const node=document.createElement(match[1]);for(const attr of match[2].matchAll(/([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g))node.setAttribute(attr[1],attr[2]??attr[3]??attr[4]??'');stack[stack.length-1].appendChild(node);if(!/\/>$/.test(token)&&!['input','img','br','hr','meta','link'].includes(node.__tag))stack.push(node);}else stack[stack.length-1].appendChild(new OrynNode({kind:'text',data:token},true));}return root.childNodes;}
  class OrynDocument extends OrynNode {
    constructor(raw) { super(raw); this.__ready=[];this.readyState='loading'; }
    createElement(tag) { tag=String(tag).toLowerCase();const ctor=customRegistry.get(tag);const node=ctor?new ctor():new OrynNode({kind:'element',tag},true);node.__kind='element';node.__tag=tag;if(!node.__id)node.__id={slot:nextNodeSlot++,generation:0};return node; }
    createElementNS(namespace,tag) { const node=this.createElement(String(tag).split(':').pop());node.namespaceURI=namespace;return node; }
    createTextNode(data) { return new OrynNode({kind:'text',data:String(data)},true); }
    createComment(data) { return new OrynNode({kind:'comment',data:String(data)},true); }
    createDocumentFragment() { return new OrynNode({kind:'fragment'},true); }
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
  const microtasks=[]; const timers=[];
  globalThis.Event=OrynEvent;
  globalThis.CustomEvent=OrynEvent;
  globalThis.CSSStyleDeclaration=OrynStyleDeclaration;
  const consoleEntries=[];
  globalThis.console={log:(...items)=>consoleEntries.push({level:'log',message:items.map(String).join(' ')}),info:(...items)=>consoleEntries.push({level:'info',message:items.map(String).join(' ')}),warn:(...items)=>consoleEntries.push({level:'warn',message:items.map(String).join(' ')}),error:(...items)=>consoleEntries.push({level:'error',message:items.map(String).join(' ')})};
  globalThis.__oryn_take_console=()=>consoleEntries.splice(0);
  globalThis.alert=()=>{};globalThis.confirm=()=>true;globalThis.prompt=()=>null;
  globalThis.HTMLElement=OrynNode;
  globalThis.Node=OrynNode;globalThis.Element=OrynNode;globalThis.Document=OrynDocument;globalThis.DocumentFragment=OrynNode;globalThis.HTMLIFrameElement=OrynNode;globalThis.HTMLInputElement=OrynNode;globalThis.HTMLSelectElement=OrynNode;globalThis.HTMLTextAreaElement=OrynNode;globalThis.SVGElement=OrynNode;
  globalThis.MutationObserver=OrynMutationObserver;
  const globalListeners=new Map();
  globalThis.addEventListener=(type,callback)=>{type=String(type);const list=globalListeners.get(type)||[];list.push(callback);globalListeners.set(type,list);};
  globalThis.removeEventListener=(type,callback)=>globalListeners.set(String(type),(globalListeners.get(String(type))||[]).filter(item=>item!==callback));
  globalThis.dispatchEvent=event=>{if(!(event instanceof OrynEvent))event=new OrynEvent(event.type||event);event.target=globalThis;event.currentTarget=globalThis;for(const fn of [...(globalListeners.get(event.type)||[])])fn.call(globalThis,event);const handler=globalThis['on'+event.type];if(typeof handler==='function')handler.call(globalThis,event);return !event.defaultPrevented;};
  globalThis.pageXOffset=0;globalThis.pageYOffset=0;globalThis.scrollX=0;globalThis.scrollY=0;
  globalThis.getComputedStyle=node=>node&&node.style?node.style:new OrynStyleDeclaration({getAttribute:()=>'',setAttribute:()=>{}});
  globalThis.customElements={define(name,ctor){name=String(name).toLowerCase();customRegistry.set(name,ctor);if(globalThis.document)upgradeTree(document);},get:name=>customRegistry.get(String(name).toLowerCase())};
  class OrynURLSearchParams { constructor(init=''){this.items=[];const value=String(init).replace(/^\?/,'');for(const part of value.split('&'))if(part){const [key,item='']=part.split('=');this.append(decodeURIComponent(key.replace(/\+/g,' ')),decodeURIComponent(item.replace(/\+/g,' ')));}}append(key,value){this.items.push([String(key),String(value)]);}set(key,value){this.delete(key);this.append(key,value);}get(key){const item=this.items.find(item=>item[0]===String(key));return item?item[1]:null;}getAll(key){return this.items.filter(item=>item[0]===String(key)).map(item=>item[1]);}has(key){return this.items.some(item=>item[0]===String(key));}delete(key){this.items=this.items.filter(item=>item[0]!==String(key));}toString(){return this.items.map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(value)).join('&');}}
  class OrynURL { constructor(input,base){input=String(input);if(base&&!/^[a-z][a-z0-9+.-]*:/i.test(input)){const parent=new OrynURL(base);const root=parent.host?parent.origin:parent.protocol+'//';input=input.startsWith('/')?root+input:root+parent.pathname.replace(/[^/]*$/,'')+input;}const match=input.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i);if(!match)throw new TypeError('Invalid URL');this.protocol=match[1];this.host=match[2]||'';this.hostname=this.host.split(':')[0];this.port=this.host.slice(this.hostname.length+1);this.pathname=match[3]||'/';this.search=match[4]||'';this.hash=match[5]||'';this.origin=this.host?this.protocol+'//'+this.host:'null';this.searchParams=new OrynURLSearchParams(this.search);}get href(){const query=this.searchParams.toString();const opaque=['about:','data:','javascript:'].includes(this.protocol);const prefix=this.host?this.origin:(opaque?this.protocol:this.protocol+'//');return prefix+this.pathname+(query?'?'+query:'')+this.hash;}set href(value){const next=new OrynURL(value,this.href);this.protocol=next.protocol;this.host=next.host;this.hostname=next.hostname;this.port=next.port;this.pathname=next.pathname;this.search=next.search;this.hash=next.hash;this.origin=next.origin;this.searchParams=next.searchParams;}toString(){return this.href;}toJSON(){return this.href;}}
  class OrynTextEncoder { encode(value=''){const bytes=unescape(encodeURIComponent(String(value)));return Uint8Array.from(bytes,char=>char.charCodeAt(0));} }
  class OrynTextDecoder { decode(value=new Uint8Array()){let bytes='';for(const byte of value)bytes+=String.fromCharCode(byte);return decodeURIComponent(escape(bytes));} }
  globalThis.URL=OrynURL;globalThis.URLSearchParams=OrynURLSearchParams;globalThis.TextEncoder=OrynTextEncoder;globalThis.TextDecoder=OrynTextDecoder;
  class OrynHeaders { constructor(entries=[]){this.items=new Map(entries.map(([key,value])=>[String(key).toLowerCase(),String(value)]));}get(key){return this.items.get(String(key).toLowerCase())||null;}has(key){return this.items.has(String(key).toLowerCase());}entries(){return this.items.entries();}[Symbol.iterator](){return this.entries();} }
  globalThis.Headers=OrynHeaders;
  class OrynStorage { constructor(){this.items=new Map();}get length(){return this.items.size;}key(index){return [...this.items.keys()][index]??null;}getItem(key){return this.items.has(String(key))?this.items.get(String(key)):null;}setItem(key,value){this.items.set(String(key),String(value));}removeItem(key){this.items.delete(String(key));}clear(){this.items.clear();} }
  globalThis.localStorage=new OrynStorage();globalThis.sessionStorage=new OrynStorage();
  globalThis.__oryn_restore_storage=(localItems,sessionItems)=>{for(const [key,value] of localItems||[])localStorage.setItem(key,value);for(const [key,value] of sessionItems||[])sessionStorage.setItem(key,value);};
  class OrynFormData { constructor(form){this.items=[];if(form){for(const node of descendants(form)){if(!['input','textarea','select'].includes(node.__tag)||node.disabled)continue;const name=node.getAttribute('name');if(!name)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;this.append(name,node.value);}}}append(name,value){this.items.push([String(name),String(value)]);}set(name,value){this.delete(name);this.append(name,value);}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null;}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1]);}has(name){return this.items.some(item=>item[0]===String(name));}delete(name){this.items=this.items.filter(item=>item[0]!==String(name));}entries(){return this.items[Symbol.iterator]();}[Symbol.iterator](){return this.entries();} }
  globalThis.FormData=OrynFormData;
  globalThis.location=new OrynURL('about:blank');
  globalThis.__oryn_set_location=value=>{globalThis.location=new OrynURL(value);document.location=globalThis.location;};
  if(typeof globalThis.__oryn_fetch_sync==='function')globalThis.fetch=(input,init={})=>new Promise((resolve,reject)=>{try{const requested=input&&input.url?input.url:String(input);const url=new OrynURL(requested,location.href).href;const requestHeaders=init.headers instanceof OrynHeaders?[...init.headers]:Array.isArray(init.headers)?init.headers:Object.entries(init.headers||{});const raw=globalThis.__oryn_fetch_sync(url,String(init.method||'GET').toUpperCase(),init.body===undefined?'':String(init.body),JSON.stringify(requestHeaders));const response=JSON.parse(raw);const headers=new OrynHeaders(response.headers);resolve({ok:response.status>=200&&response.status<300,status:response.status,statusText:'',url:response.url,redirected:response.url!==url,headers,text:()=>Promise.resolve(response.body),json:()=>Promise.resolve(JSON.parse(response.body)),arrayBuffer:()=>Promise.resolve(new TextEncoder().encode(response.body).buffer)});}catch(error){reject(error);}});
  globalThis.queueMicrotask=fn=>microtasks.push(fn);
  globalThis.setTimeout=(fn,delay=0,...args)=>{const id=timers.length+1;timers.push({id,fn,args,delay:Math.max(0,Number(delay)||0),cancelled:false});return id;};
  globalThis.clearTimeout=id=>{const timer=timers.find(item=>item.id===id);if(timer)timer.cancelled=true;};
  globalThis.setInterval=(fn,_delay=0,...args)=>globalThis.setTimeout(fn,0,...args);globalThis.clearInterval=globalThis.clearTimeout;
  globalThis.performance={now:()=>0};globalThis.requestAnimationFrame=fn=>globalThis.setTimeout(()=>fn(performance.now()),0);globalThis.cancelAnimationFrame=globalThis.clearTimeout;
  globalThis.MessageChannel=class { constructor(){const first={onmessage:null};const second={onmessage:null};first.postMessage=data=>globalThis.setTimeout(()=>second.onmessage&&second.onmessage({data}),0);second.postMessage=data=>globalThis.setTimeout(()=>first.onmessage&&first.onmessage({data}),0);this.port1=first;this.port2=second;} };
  globalThis.__oryn_drain=(advance=false)=>{let guard=0;while(guard++<10000){while(microtasks.length)microtasks.shift()();let index=timers.findIndex(item=>!item.cancelled&&item.delay<=0);if(index<0&&advance)index=timers.findIndex(item=>!item.cancelled);if(index<0)break;const [timer]=timers.splice(index,1);if(timer&&!timer.cancelled)timer.fn(...timer.args);}if(guard>=10000)throw new Error('task drain limit exceeded');};
  globalThis.__oryn_install=raw=>{const ids=[];(function collect(node){if(node.id)ids.push(node.id.slot);for(const child of node.children||[])collect(child);})(raw);nextNodeSlot=(ids.length?Math.max(...ids)+1:0);globalThis.document=new OrynDocument(raw);globalThis.window=globalThis;globalThis.self=globalThis;globalThis.global=globalThis;globalThis.navigator={userAgent:'Oryn Native/0.1'};upgradeTree(document);};
  globalThis.__oryn_dispatch_load=()=>{document.readyState='complete';globalThis.dispatchEvent(new OrynEvent('load'));};
  globalThis.__oryn_install_inline_handlers=()=>{for(const node of descendants(document)){for(const [name,source] of Object.entries(node.__attrs||{})){if(/^on[a-z]+$/i.test(name)){try{node[name.toLowerCase()]=new Function('event',String(source));}catch(error){console.error(error);}}}}};
  globalThis.__oryn_install_frames=resources=>{for(const frame of descendants(document).filter(node=>node.__tag==='iframe')){const source=frame.getAttribute('src');if(!source||!Object.prototype.hasOwnProperty.call(resources,source)){frame.contentDocument=null;frame.contentWindow=null;continue;}const child=new OrynDocument({kind:'document'});child.childNodes=parseFragment(resources[source]);for(const node of child.childNodes)node.parentNode=child;frame.contentDocument=child;frame.contentWindow={document:child,frameElement:frame,location:new OrynURL(source,location.href)};}};
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
              new MutationObserver(()=>mutations++).observe(result,{attributes:true});
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
}
