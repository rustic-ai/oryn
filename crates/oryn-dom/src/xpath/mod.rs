//! XPath 1.0 over native DOM identities. Evaluation never clones the DOM or
//! consults author properties. Bindings resolve namespace callbacks before borrowing.
mod parser;
use crate::{DomError, DomRuntime, NodeId, NodeKind};
use parser::{Axis, Expr, Op, Step, Test};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid XPath expression: {0}")]
    Syntax(String),
    #[error("unresolved XPath namespace prefix: {0}")]
    Namespace(String),
    #[error("XPath type error")]
    Type,
    #[error("invalid XPath context node")]
    Context,
    #[error("XPath evaluation limit exceeded")]
    Limit,
    #[error(transparent)]
    Dom(#[from] DomError),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Node {
    Dom(NodeId),
    Attribute {
        owner: NodeId,
        namespace: String,
        local: String,
    },
    Namespace {
        owner: NodeId,
        prefix: String,
        uri: String,
    },
}
impl Node {
    pub fn owner(&self) -> NodeId {
        match self {
            Self::Dom(n) => *n,
            Self::Attribute { owner, .. } | Self::Namespace { owner, .. } => *owner,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nodes(Vec<Node>),
    String(String),
    Number(f64),
    Boolean(bool),
}
#[derive(Clone, Debug)]
pub struct Expression(Expr);
impl Expression {
    pub fn compile(source: &str, resolver: &mut dyn FnMut(&str) -> Result<String>) -> Result<Self> {
        Ok(Self(parser::parse(source, resolver)?))
    }
    /// Retained native AST storage, excluding temporary evaluator indexes.
    pub fn retained_bytes(&self) -> usize {
        let mut bytes = 0usize;
        let mut pending = vec![&self.0];
        while let Some(expr) = pending.pop() {
            bytes = bytes.saturating_add(std::mem::size_of::<Expr>());
            match expr {
                Expr::String(s) => bytes = bytes.saturating_add(s.capacity()),
                Expr::Function(name, args) => {
                    bytes = bytes.saturating_add(name.capacity()).saturating_add(
                        (args.capacity() - args.len()).saturating_mul(std::mem::size_of::<Expr>()),
                    );
                    pending.extend(args);
                }
                Expr::Neg(e) => pending.push(e),
                Expr::Binary(_, a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                Expr::Filter(e, p) => {
                    bytes = bytes.saturating_add(
                        (p.capacity() - p.len()).saturating_mul(std::mem::size_of::<Expr>()),
                    );
                    pending.push(e);
                    pending.extend(p);
                }
                Expr::Path(e, steps) => {
                    pending.push(e);
                    bytes = bytes.saturating_add(
                        steps.capacity().saturating_mul(std::mem::size_of::<Step>()),
                    );
                    for step in steps {
                        bytes = bytes.saturating_add(
                            (step.predicates.capacity() - step.predicates.len())
                                .saturating_mul(std::mem::size_of::<Expr>()),
                        );
                        pending.extend(&step.predicates);
                        match &step.test {
                            Test::Name {
                                namespace, local, ..
                            } => {
                                bytes = bytes
                                    .saturating_add(namespace.capacity())
                                    .saturating_add(local.as_ref().map_or(0, String::capacity));
                            }
                            Test::Pi(Some(s)) => bytes = bytes.saturating_add(s.capacity()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        bytes
    }
    pub fn evaluate(&self, dom: &DomRuntime, context: NodeId) -> Result<Value> {
        self.evaluate_node(dom, Node::Dom(context))
    }
    pub fn evaluate_node(&self, dom: &DomRuntime, context: Node) -> Result<Value> {
        let mut e = Evaluation::new(dom, context.owner())?;
        let node = e.canonical(context)?;
        e.expr(
            &self.0,
            &Context {
                node,
                position: 1,
                size: 1,
            },
        )
    }
}
impl Value {
    pub fn boolean(&self) -> bool {
        match self {
            Self::Nodes(v) => !v.is_empty(),
            Self::String(v) => !v.is_empty(),
            Self::Number(v) => *v != 0. && !v.is_nan(),
            Self::Boolean(v) => *v,
        }
    }
    pub fn string(&self, dom: &DomRuntime) -> Result<String> {
        Ok(match self {
            Self::String(v) => v.clone(),
            Self::Boolean(v) => if *v { "true" } else { "false" }.into(),
            Self::Number(v) => number_string(*v),
            Self::Nodes(v) => v
                .first()
                .map(|n| string_value(dom, n))
                .transpose()?
                .unwrap_or_default(),
        })
    }
    pub fn number(&self, dom: &DomRuntime) -> Result<f64> {
        Ok(match self {
            Self::Number(v) => *v,
            Self::Boolean(v) => {
                if *v {
                    1.
                } else {
                    0.
                }
            }
            _ => number(&self.string(dom)?),
        })
    }
    pub fn node_slice(&self) -> Result<&[Node]> {
        match self {
            Self::Nodes(nodes) => Ok(nodes),
            _ => Err(Error::Type),
        }
    }
    pub fn nodes(self) -> Result<Vec<Node>> {
        match self {
            Self::Nodes(v) => Ok(v),
            _ => Err(Error::Type),
        }
    }
}
// Unicode scalar pairs count once; unmatched UTF-16 units remain distinct values.
// One flat u32 per character avoids a heap allocation for every character.
fn characters(value: &str) -> Vec<u32> {
    let units = crate::dom_string::units(value);
    let mut output = Vec::with_capacity(units.len());
    let mut at = 0;
    while at < units.len() {
        let first = units[at];
        if (0xd800..=0xdbff).contains(&first)
            && units
                .get(at + 1)
                .is_some_and(|u| (0xdc00..=0xdfff).contains(u))
        {
            output.push(
                0x10000 + (((first as u32 - 0xd800) << 10) | (units[at + 1] as u32 - 0xdc00)),
            );
            at += 2;
        } else {
            output.push(first as u32);
            at += 1;
        }
    }
    output
}
fn from_characters(parts: impl IntoIterator<Item = u32>) -> String {
    let mut units = Vec::new();
    for point in parts {
        if point <= 0xffff {
            units.push(point as u16);
        } else {
            let point = point - 0x10000;
            units.push(0xd800 + (point >> 10) as u16);
            units.push(0xdc00 + (point & 1023) as u16);
        }
    }
    crate::dom_string::from_units(&units)
}
fn append_string(target: &mut String, part: &str) -> Result<()> {
    if !target.starts_with(crate::dom_string::PREFIX)
        && !part.starts_with(crate::dom_string::PREFIX)
    {
        if target.len().saturating_add(part.len()) > 32 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        target.push_str(part);
        if target.starts_with(crate::dom_string::PREFIX) {
            *target = crate::dom_string::from_units(&target.encode_utf16().collect::<Vec<_>>());
        }
        return Ok(());
    }
    let mut units = crate::dom_string::units(target);
    let next = crate::dom_string::units(part);
    if units.len().saturating_add(next.len()) > 16 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    units.extend(next);
    *target = crate::dom_string::from_units(&units);
    Ok(())
}
fn normalized_space(value: &str) -> String {
    let mut out = Vec::new();
    let mut space = false;
    for unit in crate::dom_string::units(value) {
        if matches!(unit, 9 | 10 | 13 | 32) {
            space = !out.is_empty();
        } else {
            if space {
                out.push(32);
            }
            out.push(unit);
            space = false;
        }
    }
    crate::dom_string::from_units(&out)
}
fn whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}
fn number(s: &str) -> f64 {
    let s = s.trim_matches(whitespace);
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    if unsigned.is_empty()
        || unsigned.chars().any(|c| !c.is_ascii_digit() && c != '.')
        || unsigned.chars().filter(|c| *c == '.').count() > 1
        || !unsigned.chars().any(|c| c.is_ascii_digit())
    {
        return f64::NAN;
    }
    s.parse().unwrap_or(f64::NAN)
}
fn number_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "Infinity".into()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".into()
    } else if n == 0. {
        "0".into()
    } else {
        n.to_string()
    }
}
fn round(n: f64) -> f64 {
    if !n.is_finite() || n.fract() == 0. {
        return n;
    }
    if (-0.5..0.).contains(&n) {
        -0.
    } else {
        (n + 0.5).floor()
    }
}
fn text(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Text { .. } | NodeKind::CDataSection { .. })
}
fn tree_node(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Document
            | NodeKind::Element { .. }
            | NodeKind::Text { .. }
            | NodeKind::CDataSection { .. }
            | NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
    )
}
fn raw_parent(dom: &DomRuntime, node: &Node) -> Result<Option<Node>> {
    Ok(match node {
        Node::Attribute { owner, .. } | Node::Namespace { owner, .. } => Some(Node::Dom(*owner)),
        Node::Dom(id) => match &dom.arena().get(*id)?.kind {
            NodeKind::Attribute { owner, .. } => owner.map(Node::Dom),
            _ => dom.arena().get(*id)?.parent.map(Node::Dom),
        },
    })
}
fn copy_string(value: &str) -> Result<String> {
    if value.len() > 32 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    Ok(value.into())
}
pub fn string_value(dom: &DomRuntime, node: &Node) -> Result<String> {
    match node {
        Node::Namespace { uri, .. } => copy_string(uri),
        Node::Attribute {
            owner,
            namespace,
            local,
        } => copy_string(
            dom.attribute_ns(
                *owner,
                (!namespace.is_empty()).then_some(namespace.as_str()),
                local,
            )?
            .unwrap_or(""),
        ),
        Node::Dom(id) => match &dom.arena().get(*id)?.kind {
            NodeKind::Attribute { data, .. } => copy_string(&data.value),
            NodeKind::Comment { data } | NodeKind::ProcessingInstruction { data, .. } => {
                copy_string(data)
            }
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
                let mut result = String::new();
                let record = dom.arena().get(*id)?;
                if let Some(parent) = record.parent {
                    let children = dom.children(parent)?;
                    let at = children
                        .iter()
                        .position(|n| n == id)
                        .ok_or(Error::Context)?;
                    let mut start = at;
                    while start > 0 && text(&dom.arena().get(children[start - 1])?.kind) {
                        start -= 1;
                    }
                    for child in &children[start..] {
                        match &dom.arena().get(*child)?.kind {
                            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                                append_string(&mut result, data)?
                            }
                            _ => break,
                        }
                    }
                } else if let NodeKind::Text { data } | NodeKind::CDataSection { data } =
                    &record.kind
                {
                    append_string(&mut result, data)?;
                }
                Ok(result)
            }
            NodeKind::Document
            | NodeKind::Element { .. }
            | NodeKind::ShadowRoot { .. }
            | NodeKind::DocumentFragment => {
                let mut output = String::new();
                let mut pending: Vec<NodeId> = dom.children(*id)?.iter().rev().copied().collect();
                let mut visited = 0;
                while let Some(id) = pending.pop() {
                    visited += 1;
                    if visited > 1_000_000 || output.len() > 32 * 1024 * 1024 {
                        return Err(Error::Limit);
                    }
                    let node = dom.arena().get(id)?;
                    match &node.kind {
                        NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                            append_string(&mut output, data)?
                        }
                        NodeKind::Element { .. } => {
                            pending.extend(node.children.iter().rev().copied())
                        }
                        _ => {}
                    }
                }
                Ok(output)
            }
            _ => Err(Error::Context),
        },
    }
}
#[derive(Clone)]
struct Context {
    node: Node,
    position: usize,
    size: usize,
}
struct Evaluation<'a> {
    dom: &'a DomRuntime,
    root: NodeId,
    order: BTreeMap<NodeId, usize>,
    work: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
    html: bool,
}
impl<'a> Evaluation<'a> {
    fn new(dom: &'a DomRuntime, context: NodeId) -> Result<Self> {
        let record = dom.arena().get(context)?;
        if !tree_node(&record.kind) && !matches!(record.kind, NodeKind::Attribute { .. }) {
            return Err(Error::Context);
        }
        let mut root = match &record.kind {
            NodeKind::Attribute {
                owner: Some(owner), ..
            } => *owner,
            NodeKind::Attribute { owner: None, .. } => context,
            _ => context,
        };
        while let Some(parent) = dom.arena().get(root)?.parent {
            root = parent;
        }
        if !tree_node(&dom.arena().get(root)?.kind)
            && !matches!(
                dom.arena().get(root)?.kind,
                NodeKind::ShadowRoot { .. }
                    | NodeKind::DocumentFragment
                    | NodeKind::Attribute { .. }
            )
        {
            return Err(Error::Context);
        }
        let document = record.owner_document.unwrap_or(root);
        let html = dom
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|m| m.is_html);
        let mut order = BTreeMap::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if order.len() > 1_000_000 {
                return Err(Error::Limit);
            }
            let node = dom.arena().get(id)?;
            if id != root && !tree_node(&node.kind) {
                continue;
            }
            order.insert(id, order.len());
            stack.extend(node.children.iter().rev().copied());
        }
        Ok(Self {
            dom,
            root,
            order,
            work: std::cell::Cell::new(0),
            bytes: std::cell::Cell::new(0),
            html,
        })
    }
    fn charge(&self, n: usize) -> Result<()> {
        let next = self.work.get().checked_add(n).ok_or(Error::Limit)?;
        if next > 4_000_000 {
            Err(Error::Limit)
        } else {
            self.work.set(next);
            Ok(())
        }
    }
    fn charge_bytes(&self, n: usize) -> Result<()> {
        let next = self.bytes.get().checked_add(n).ok_or(Error::Limit)?;
        if next > 64 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        self.bytes.set(next);
        Ok(())
    }
    fn account_value(&self, value: &Value) -> Result<()> {
        self.charge_bytes(match value {
            Value::String(s) => s.capacity(),
            Value::Nodes(nodes) => nodes
                .capacity()
                .saturating_mul(std::mem::size_of::<Node>())
                .saturating_add(
                    nodes
                        .iter()
                        .map(|node| match node {
                            Node::Attribute {
                                namespace, local, ..
                            } => namespace.capacity().saturating_add(local.capacity()),
                            Node::Namespace { prefix, uri, .. } => {
                                prefix.capacity().saturating_add(uri.capacity())
                            }
                            _ => 0,
                        })
                        .fold(0usize, usize::saturating_add),
                ),
            _ => 0,
        })
    }
    fn canonical(&self, node: Node) -> Result<Node> {
        if let Node::Dom(id) = node {
            match &self.dom.arena().get(id)?.kind {
                NodeKind::Attribute {
                    data,
                    owner: Some(owner),
                } => {
                    return Ok(Node::Attribute {
                        owner: *owner,
                        namespace: data.namespace.clone().unwrap_or_default(),
                        local: data.local_name.clone(),
                    });
                }
                kind if text(kind) => {
                    if let Some(parent) = self.dom.arena().get(id)?.parent {
                        let children = self.dom.children(parent)?;
                        let mut at = children
                            .iter()
                            .position(|child| *child == id)
                            .ok_or(Error::Context)?;
                        while at > 0 && text(&self.dom.arena().get(children[at - 1])?.kind) {
                            self.charge(1)?;
                            at -= 1;
                        }
                        let first = children[at];
                        for child in &children[at..] {
                            self.charge(1)?;
                            match &self.dom.arena().get(*child)?.kind {
                                NodeKind::Text { data } | NodeKind::CDataSection { data }
                                    if !data.is_empty() =>
                                {
                                    return Ok(Node::Dom(*child));
                                }
                                kind if text(kind) => {}
                                _ => break,
                            }
                        }
                        return Ok(Node::Dom(first));
                    }
                }
                _ => {}
            }
            Ok(Node::Dom(id))
        } else {
            Ok(node)
        }
    }
    fn ordered(&self, mut nodes: Vec<Node>) -> Vec<Node> {
        let rank = |node: &Node| match node {
            Node::Dom(_) => 0,
            Node::Namespace { .. } => 1,
            Node::Attribute { .. } => 2,
        };
        nodes.sort_by(|a, b| {
            self.order
                .get(&a.owner())
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&self.order.get(&b.owner()).copied().unwrap_or(usize::MAX))
                .then_with(|| rank(a).cmp(&rank(b)))
                .then_with(|| a.cmp(b))
        });
        nodes.dedup();
        nodes
    }
    fn descendants(&mut self, node: &Node, include: bool) -> Result<Vec<Node>> {
        let mut result = Vec::new();
        if include {
            result.push(node.clone());
        }
        let Node::Dom(id) = node else {
            return Ok(result);
        };
        let mut pending: Vec<NodeId> = self.dom.children(*id)?.iter().rev().copied().collect();
        while let Some(id) = pending.pop() {
            self.charge(1)?;
            let native = self.dom.arena().get(id)?;
            if !tree_node(&native.kind) {
                continue;
            }
            let node = self.canonical(Node::Dom(id))?;
            if (!text(&native.kind)
                || !matches!(&self.dom.arena().get(node.owner())?.kind,NodeKind::Text{data}|NodeKind::CDataSection{data} if data.is_empty()))
                && result.last() != Some(&node)
            {
                self.charge_bytes(std::mem::size_of::<Node>())?;
                result.push(node);
            }
            pending.extend(native.children.iter().rev().copied());
        }
        Ok(result)
    }
    fn axis(&mut self, node: &Node, axis: Axis) -> Result<Vec<Node>> {
        self.charge(1)?;
        Ok(match axis {
            Axis::SelfNode => vec![node.clone()],
            Axis::Parent => raw_parent(self.dom, node)?.into_iter().collect(),
            Axis::Descendant => return self.descendants(node, false),
            Axis::DescendantOrSelf => return self.descendants(node, true),
            Axis::Ancestor | Axis::AncestorOrSelf => {
                let mut result = Vec::new();
                let mut at = if axis == Axis::AncestorOrSelf {
                    Some(node.clone())
                } else {
                    raw_parent(self.dom, node)?
                };
                while let Some(n) = at {
                    self.charge(1)?;
                    at = raw_parent(self.dom, &n)?;
                    result.push(n);
                }
                result
            }
            Axis::Child => {
                let mut result = Vec::new();
                if let Node::Dom(id) = node {
                    for child in self.dom.children(*id)? {
                        self.charge(1)?;
                        let kind = &self.dom.arena().get(*child)?.kind;
                        if !tree_node(kind) {
                            continue;
                        }
                        let n = self.canonical(Node::Dom(*child))?;
                        if text(kind)
                            && matches!(&self.dom.arena().get(n.owner())?.kind,NodeKind::Text{data}|NodeKind::CDataSection{data} if data.is_empty())
                        {
                            continue;
                        }
                        if result.last() != Some(&n) {
                            self.charge_bytes(std::mem::size_of::<Node>())?;
                            result.push(n);
                        }
                    }
                }
                result
            }
            Axis::Attribute => {
                let mut result = Vec::new();
                if let Node::Dom(id) = node {
                    let record = self.dom.arena().get(*id)?;
                    if matches!(record.kind, NodeKind::Element { .. }) {
                        for a in record.attributes.records() {
                            self.charge(1)?;
                            if a.namespace.as_deref() != Some("http://www.w3.org/2000/xmlns/") {
                                self.charge_bytes(
                                    a.namespace
                                        .as_ref()
                                        .map_or(0, String::len)
                                        .saturating_add(a.local_name.len())
                                        .saturating_add(std::mem::size_of::<Node>()),
                                )?;
                                result.push(Node::Attribute {
                                    owner: *id,
                                    namespace: a.namespace.clone().unwrap_or_default(),
                                    local: a.local_name.clone(),
                                });
                            }
                        }
                    }
                }
                result
            }
            Axis::Namespace => {
                let mut result = Vec::new();
                if let Node::Dom(id) = node
                    && matches!(self.dom.arena().get(*id)?.kind, NodeKind::Element { .. })
                {
                    let mut bindings = BTreeMap::new();
                    bindings.insert(
                        "xml".to_string(),
                        "http://www.w3.org/XML/1998/namespace".to_string(),
                    );
                    let mut at = Some(*id);
                    while let Some(element) = at {
                        let record = self.dom.arena().get(element)?;
                        for a in record.attributes.records() {
                            self.charge(1)?;
                            if a.namespace.as_deref() == Some("http://www.w3.org/2000/xmlns/") {
                                self.charge_bytes(
                                    a.value.len().saturating_add(a.local_name.len()),
                                )?;
                                let prefix = if a.name == "xmlns" { "" } else { &a.local_name };
                                bindings
                                    .entry(prefix.into())
                                    .or_insert_with(|| a.value.clone());
                            }
                        }
                        if let NodeKind::Element { name } = &record.kind
                            && !name.ns.is_empty()
                        {
                            self.charge_bytes(name.ns.len())?;
                            bindings
                                .entry(name.prefix.as_ref().map_or("", |p| p.as_ref()).into())
                                .or_insert_with(|| name.ns.to_string());
                        }
                        at = record.parent;
                        self.charge(1)?;
                    }
                    result = bindings
                        .into_iter()
                        .filter(|(_, uri)| !uri.is_empty())
                        .map(|(prefix, uri)| Node::Namespace {
                            owner: *id,
                            prefix,
                            uri,
                        })
                        .collect();
                }
                result
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                let mut result = Vec::new();
                if matches!(node, Node::Dom(_))
                    && let Some(parent) = raw_parent(self.dom, node)?
                {
                    let siblings = self.axis(&parent, Axis::Child)?;
                    if let Some(at) = siblings.iter().position(|n| n == node) {
                        result = if axis == Axis::FollowingSibling {
                            siblings[at + 1..].to_vec()
                        } else {
                            siblings[..at].iter().rev().cloned().collect()
                        };
                    }
                }
                result
            }
            Axis::Following | Axis::Preceding => {
                let all = self.descendants(&Node::Dom(self.root), true)?;
                let anchor = Node::Dom(node.owner());
                let at = all
                    .iter()
                    .position(|n| n == &anchor)
                    .ok_or(Error::Context)?;
                if axis == Axis::Following {
                    let descendants = if matches!(node, Node::Dom(_)) {
                        self.descendants(node, true)?
                            .into_iter()
                            .collect::<BTreeSet<_>>()
                    } else {
                        BTreeSet::new()
                    };
                    all[at + 1..]
                        .iter()
                        .filter(|n| !descendants.contains(*n))
                        .cloned()
                        .collect()
                } else {
                    let ancestors = self
                        .axis(node, Axis::AncestorOrSelf)?
                        .into_iter()
                        .collect::<BTreeSet<_>>();
                    all[..at]
                        .iter()
                        .rev()
                        .filter(|n| !ancestors.contains(*n))
                        .cloned()
                        .collect()
                }
            }
        })
    }
    fn names(&self, node: &Node) -> Result<(String, String, String)> {
        Ok(match node {
            Node::Attribute {
                owner,
                namespace,
                local,
            } => {
                let name = self
                    .dom
                    .arena()
                    .get(*owner)?
                    .attributes
                    .records()
                    .find(|a| {
                        a.local_name == *local && a.namespace.as_deref().unwrap_or("") == namespace
                    })
                    .map(|a| a.name.clone())
                    .unwrap_or_default();
                (namespace.clone(), local.clone(), name)
            }
            Node::Namespace { prefix, .. } => (String::new(), prefix.clone(), prefix.clone()),
            Node::Dom(id) => match &self.dom.arena().get(*id)?.kind {
                NodeKind::Element { name } => (
                    name.ns.to_string(),
                    name.local.to_string(),
                    name.prefix
                        .as_ref()
                        .map_or_else(|| name.local.to_string(), |p| format!("{p}:{}", name.local)),
                ),
                NodeKind::ProcessingInstruction { target, .. } => {
                    (String::new(), target.clone(), target.clone())
                }
                NodeKind::Attribute { data, .. } => (
                    data.namespace.clone().unwrap_or_default(),
                    data.local_name.clone(),
                    data.name.clone(),
                ),
                _ => (String::new(), String::new(), String::new()),
            },
        })
    }
    fn matches(&self, node: &Node, step: &Step) -> Result<bool> {
        let kind = if let Node::Dom(id) = node {
            Some(&self.dom.arena().get(*id)?.kind)
        } else {
            None
        };
        Ok(match &step.test {
            Test::Node => true,
            Test::Text => kind.is_some_and(text),
            Test::Comment => matches!(kind, Some(NodeKind::Comment { .. })),
            Test::Pi(target) => {
                matches!(kind,Some(NodeKind::ProcessingInstruction{target:name,..}) if target.as_ref().is_none_or(|t|t==name))
            }
            Test::Any | Test::Name { .. } => {
                let principal = match step.axis {
                    Axis::Attribute => matches!(node, Node::Attribute { .. }),
                    Axis::Namespace => matches!(node, Node::Namespace { .. }),
                    _ => matches!(kind, Some(NodeKind::Element { .. })),
                };
                if !principal {
                    return Ok(false);
                }
                match &step.test {
                    Test::Any => true,
                    Test::Name {
                        namespace,
                        local,
                        unprefixed,
                    } => {
                        let (ns, name, _) = self.names(node)?;
                        let html = self.html
                            && step.axis != Axis::Attribute
                            && ns == "http://www.w3.org/1999/xhtml";
                        (ns == *namespace || (html && *unprefixed && namespace.is_empty()))
                            && local.as_ref().is_none_or(|wanted| {
                                if html {
                                    name.eq_ignore_ascii_case(wanted)
                                } else {
                                    &name == wanted
                                }
                            })
                    }
                    _ => false,
                }
            }
        })
    }
    fn filter(&mut self, mut nodes: Vec<Node>, predicates: &[Expr]) -> Result<Vec<Node>> {
        for predicate in predicates {
            let size = nodes.len();
            let mut kept = Vec::new();
            for (i, node) in nodes.into_iter().enumerate() {
                let value = self.expr(
                    predicate,
                    &Context {
                        node: node.clone(),
                        position: i + 1,
                        size,
                    },
                )?;
                let keep = match value {
                    Value::Number(n) => n == (i + 1) as f64,
                    _ => value.boolean(),
                };
                if keep {
                    kept.push(node);
                }
            }
            nodes = kept;
        }
        Ok(nodes)
    }
    fn expr(&mut self, expr: &Expr, context: &Context) -> Result<Value> {
        self.charge(1)?;
        let value = match expr {
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::Context => Ok(Value::Nodes(vec![context.node.clone()])),
            Expr::Root => Ok(Value::Nodes(vec![Node::Dom(self.root)])),
            Expr::Neg(e) => Ok(Value::Number(-self.expr(e, context)?.number(self.dom)?)),
            Expr::Function(name, args) => self.function(name, args, context),
            Expr::Filter(base, predicates) => {
                let nodes = self.expr(base, context)?.nodes()?;
                let nodes = self.ordered(nodes);
                Ok(Value::Nodes(self.filter(nodes, predicates)?))
            }
            Expr::Path(base, steps) => {
                let mut nodes = self.expr(base, context)?.nodes()?;
                for step in steps {
                    let mut combined = Vec::new();
                    for node in nodes {
                        let candidates = self.axis(&node, step.axis)?;
                        let mut matched = Vec::new();
                        for n in candidates {
                            if self.matches(&n, step)? {
                                matched.push(n);
                            }
                        }
                        let filtered = self.filter(matched, &step.predicates)?;
                        self.charge_bytes(
                            filtered.len().saturating_mul(std::mem::size_of::<Node>()),
                        )?;
                        combined.extend(filtered);
                        self.charge(combined.len())?;
                    }
                    nodes = self.ordered(combined);
                }
                Ok(Value::Nodes(nodes))
            }
            Expr::Binary(op, left, right) => {
                let left = self.expr(left, context)?;
                if matches!(op, Op::Or) && left.boolean() {
                    return Ok(Value::Boolean(true));
                }
                if matches!(op, Op::And) && !left.boolean() {
                    return Ok(Value::Boolean(false));
                }
                let right = self.expr(right, context)?;
                match op {
                    Op::Or | Op::And => Ok(Value::Boolean(right.boolean())),
                    Op::Union => {
                        let mut nodes = left.nodes()?;
                        nodes.extend(right.nodes()?);
                        Ok(Value::Nodes(self.ordered(nodes)))
                    }
                    Op::Equal
                    | Op::NotEqual
                    | Op::Less
                    | Op::LessEqual
                    | Op::Greater
                    | Op::GreaterEqual => {
                        let a = if let Value::Nodes(n) = &left {
                            n.len()
                        } else {
                            1
                        };
                        let b = if let Value::Nodes(n) = &right {
                            n.len()
                        } else {
                            1
                        };
                        self.charge(a.checked_mul(b).ok_or(Error::Limit)?)?;
                        Ok(Value::Boolean(compare(
                            self.dom,
                            *op,
                            &left,
                            &right,
                            &|n| self.charge(n),
                        )?))
                    }
                    _ => {
                        let a = left.number(self.dom)?;
                        let b = right.number(self.dom)?;
                        Ok(Value::Number(match op {
                            Op::Add => a + b,
                            Op::Subtract => a - b,
                            Op::Multiply => a * b,
                            Op::Divide => a / b,
                            Op::Modulo => a % b,
                            _ => unreachable!(),
                        }))
                    }
                }
            }
        }?;
        self.account_value(&value)?;
        Ok(value)
    }
    fn function(&mut self, name: &str, args: &[Expr], context: &Context) -> Result<Value> {
        let mut values = Vec::new();
        for arg in args {
            values.push(self.expr(arg, context)?);
        }
        let context_value = Value::Nodes(vec![context.node.clone()]);
        let first = values.first().unwrap_or(&context_value);
        Ok(match name {
            "last" => Value::Number(context.size as f64),
            "position" => Value::Number(context.position as f64),
            "true" => Value::Boolean(true),
            "false" => Value::Boolean(false),
            "count" => Value::Number(first.node_slice()?.len() as f64),
            "boolean" => Value::Boolean(first.boolean()),
            "not" => Value::Boolean(!first.boolean()),
            "string" => Value::String(first.string(self.dom)?),
            "number" => Value::Number(first.number(self.dom)?),
            "local-name" | "namespace-uri" | "name" => {
                let nodes = first.node_slice()?;
                let tuple = nodes
                    .first()
                    .map(|n| self.names(n))
                    .transpose()?
                    .unwrap_or_default();
                Value::String(match name {
                    "namespace-uri" => tuple.0,
                    "local-name" => tuple.1,
                    _ => tuple.2,
                })
            }
            "concat" => {
                let mut result = String::new();
                for v in &values {
                    let part = v.string(self.dom)?;
                    if result.len().saturating_add(part.len()) > 32 * 1024 * 1024 {
                        return Err(Error::Limit);
                    }
                    append_string(&mut result, &part)?;
                }
                Value::String(result)
            }
            "starts-with" | "contains" | "substring-before" | "substring-after" => {
                let a = characters(&first.string(self.dom)?);
                let b = characters(&values[1].string(self.dom)?);
                self.charge(a.len().checked_mul(b.len().max(1)).ok_or(Error::Limit)?)?;
                let found = if b.is_empty() {
                    Some(0)
                } else {
                    a.windows(b.len()).position(|slice| slice == b.as_slice())
                };
                match name {
                    "starts-with" => Value::Boolean(found == Some(0)),
                    "contains" => Value::Boolean(found.is_some()),
                    "substring-before" => Value::String(
                        found
                            .map(|i| from_characters(a[..i].iter().cloned()))
                            .unwrap_or_default(),
                    ),
                    _ => Value::String(
                        found
                            .map(|i| from_characters(a[i + b.len()..].iter().cloned()))
                            .unwrap_or_default(),
                    ),
                }
            }
            "substring" => {
                let text = first.string(self.dom)?;
                let start = round(values[1].number(self.dom)?);
                let end = if values.len() == 3 {
                    start + round(values[2].number(self.dom)?)
                } else {
                    f64::INFINITY
                };
                Value::String(from_characters(
                    characters(&text)
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| (*i + 1) as f64 >= start && ((*i + 1) as f64) < end)
                        .map(|(_, c)| c),
                ))
            }
            "string-length" => Value::Number(characters(&first.string(self.dom)?).len() as f64),
            "normalize-space" => Value::String(normalized_space(&first.string(self.dom)?)),
            "translate" => {
                let source = characters(&first.string(self.dom)?);
                let from = characters(&values[1].string(self.dom)?);
                let to = characters(&values[2].string(self.dom)?);
                self.charge(
                    source
                        .len()
                        .saturating_add(from.len())
                        .saturating_add(to.len()),
                )?;
                let mut mapping = BTreeMap::new();
                for (i, c) in from.into_iter().enumerate() {
                    mapping.entry(c).or_insert_with(|| to.get(i).copied());
                }
                Value::String(from_characters(
                    source
                        .into_iter()
                        .filter_map(|c| mapping.get(&c).copied().unwrap_or(Some(c))),
                ))
            }
            "sum" => {
                let nodes = first.node_slice()?;
                let mut result = 0.;
                for n in nodes {
                    self.charge(1)?;
                    result += number(&string_value(self.dom, n)?);
                }
                Value::Number(result)
            }
            "floor" => Value::Number(first.number(self.dom)?.floor()),
            "ceiling" => Value::Number(first.number(self.dom)?.ceil()),
            "round" => Value::Number(round(first.number(self.dom)?)),
            "lang" => {
                let wanted = first.string(self.dom)?.to_ascii_lowercase();
                let mut at = Some(context.node.clone());
                let mut found = false;
                while let Some(n) = at {
                    self.charge(1)?;
                    if let Node::Dom(id) = &n
                        && matches!(self.dom.arena().get(*id)?.kind, NodeKind::Element { .. })
                        && let Some(lang) = self.dom.attribute_ns(
                            *id,
                            Some("http://www.w3.org/XML/1998/namespace"),
                            "lang",
                        )?
                    {
                        let lang = lang.to_ascii_lowercase();
                        found = lang == wanted
                            || lang
                                .strip_prefix(&wanted)
                                .is_some_and(|tail| tail.starts_with('-'));
                        break;
                    }
                    at = raw_parent(self.dom, &n)?;
                }
                Value::Boolean(found)
            }
            "id" => {
                let texts = match first {
                    Value::Nodes(nodes) => nodes
                        .iter()
                        .map(|n| string_value(self.dom, n))
                        .collect::<Result<Vec<_>>>()?,
                    _ => vec![first.string(self.dom)?],
                };
                let mut ids = texts
                    .iter()
                    .flat_map(|s| s.split(whitespace))
                    .filter(|s| !s.is_empty())
                    .collect::<BTreeSet<_>>();
                let mut result = Vec::new();
                for n in self.descendants(&Node::Dom(self.root), true)? {
                    if let Node::Dom(id) = n
                        && matches!(self.dom.arena().get(id)?.kind, NodeKind::Element { .. })
                    {
                        let value = self.dom.attribute_in_null_namespace(id, "id")?.or(self
                            .dom
                            .attribute_ns(
                                id,
                                Some("http://www.w3.org/XML/1998/namespace"),
                                "id",
                            )?);
                        if value.is_some_and(|v| ids.remove(v)) {
                            result.push(Node::Dom(id));
                        }
                    }
                }
                Value::Nodes(self.ordered(result))
            }
            _ => return Err(Error::Syntax(format!("unknown function {name}"))),
        })
    }
}
fn compare(
    dom: &DomRuntime,
    op: Op,
    left: &Value,
    right: &Value,
    charge: &dyn Fn(usize) -> Result<()>,
) -> Result<bool> {
    let equality = matches!(op, Op::Equal | Op::NotEqual);
    let numeric = |a: f64, b: f64| match op {
        Op::Equal => a == b,
        Op::NotEqual => a != b,
        Op::Less => a < b,
        Op::LessEqual => a <= b,
        Op::Greater => a > b,
        Op::GreaterEqual => a >= b,
        _ => false,
    };
    let strings = |a: &str, b: &str| {
        if matches!(op, Op::Equal) {
            a == b
        } else {
            a != b
        }
    };
    match (left, right) {
        (Value::Nodes(a), Value::Nodes(b)) => {
            for x in a {
                for y in b {
                    let x = string_value(dom, x)?;
                    charge(x.len())?;
                    let y = string_value(dom, y)?;
                    charge(y.len())?;
                    if if equality {
                        strings(&x, &y)
                    } else {
                        numeric(number(&x), number(&y))
                    } {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        (Value::Nodes(nodes), other) | (other, Value::Nodes(nodes)) => {
            if equality && matches!(other, Value::Boolean(_)) {
                return Ok(if matches!(op, Op::Equal) {
                    left.boolean() == right.boolean()
                } else {
                    left.boolean() != right.boolean()
                });
            }
            let left_nodes = matches!(left, Value::Nodes(_));
            for node in nodes {
                let text = string_value(dom, node)?;
                charge(text.len())?;
                let result = if equality && matches!(other, Value::String(_)) {
                    strings(&text, &other.string(dom)?)
                } else {
                    let a = number(&text);
                    let b = other.number(dom)?;
                    if left_nodes {
                        numeric(a, b)
                    } else {
                        numeric(b, a)
                    }
                };
                if result {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ if equality && matches!(left, Value::Boolean(_))
            || equality && matches!(right, Value::Boolean(_)) =>
        {
            Ok(if matches!(op, Op::Equal) {
                left.boolean() == right.boolean()
            } else {
                left.boolean() != right.boolean()
            })
        }
        _ if !equality || matches!(left, Value::Number(_)) || matches!(right, Value::Number(_)) => {
            Ok(numeric(left.number(dom)?, right.number(dom)?))
        }
        _ => Ok(strings(&left.string(dom)?, &right.string(dom)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(dom: &DomRuntime, expression: &str) -> Value {
        Expression::compile(expression, &mut |prefix| {
            if prefix == "p" {
                Ok("urn:p".into())
            } else {
                Err(Error::Namespace(prefix.into()))
            }
        })
        .unwrap()
        .evaluate(dom, dom.document())
        .unwrap()
    }
    fn fixture() -> DomRuntime {
        crate::html::parse("<body><section><i id=a n=1>A</i><i id=b n=2>B</i><i id=c n=3>C</i></section><i id=d n=4>D</i></body>").into_runtime().unwrap()
    }
    #[test]
    fn axes_and_predicates_have_distinct_reverse_axis_and_filter_order() {
        let dom = fixture();
        for (expr, expected) in [
            ("string(//i[@id='c']/preceding-sibling::i[1]/@id)", "b"),
            ("string((//i[@id='c']/preceding-sibling::i)[1]/@id)", "a"),
            ("string(//i[1]/@id)", "a"),
            ("string((//i)[last()]/@id)", "d"),
            ("string(//i[@id='a']/following::i[1]/@id)", "b"),
        ] {
            assert_eq!(eval(&dom, expr), Value::String(expected.into()), "{expr}");
        }
    }
    #[test]
    fn comparison_is_existential_and_not_negated_set_equality() {
        let dom = fixture();
        assert_eq!(
            eval(&dom, "//i/@n = 2 and //i/@n != 2"),
            Value::Boolean(true)
        );
        assert_eq!(eval(&dom, "//missing = false()"), Value::Boolean(true));
        assert_eq!(eval(&dom, "//i/@n > 3"), Value::Boolean(true));
        assert_eq!(eval(&dom, "sum(//i/@n)"), Value::Number(10.));
    }
    #[test]
    fn xpath_numeric_string_functions_handle_nan_rounding_and_unicode() {
        let dom = fixture();
        for (expr, expected) in [
            ("substring('12345',1.5,2.6)", "234"),
            ("substring('12345',0,3)", "12"),
            ("substring('12345',0 div 0,3)", ""),
            ("translate('bar','abc','ABC')", "BAr"),
            ("normalize-space('  a  b  ')", "a b"),
            ("substring('A😀B',2,1)", "😀"),
        ] {
            assert_eq!(eval(&dom, expr), Value::String(expected.into()), "{expr}");
        }
        assert_eq!(
            eval(&dom, "string(1 div 0)"),
            Value::String("Infinity".into())
        );
        assert_eq!(
            eval(&dom, "number('+1') = number('+1')"),
            Value::Boolean(false)
        );
    }
    #[test]
    fn parser_rejects_unknown_functions_variables_namespaces_and_wrong_arity() {
        for source in [
            "$x",
            "unknown()",
            "count()",
            "//p:item",
            "1e3",
            "//a[",
            "//a::b",
            "concat('a')",
        ] {
            assert!(
                Expression::compile(source, &mut |p| Err(Error::Namespace(p.into()))).is_err(),
                "{source}"
            );
        }
    }
    #[test]
    fn native_mutations_are_visible_without_a_shadow_tree_copy() {
        let mut dom = fixture();
        let expression =
            Expression::compile("count(//i)", &mut |p| Err(Error::Namespace(p.into()))).unwrap();
        assert_eq!(
            expression.evaluate(&dom, dom.document()).unwrap(),
            Value::Number(4.)
        );
        let id = dom.get_element_by_id(dom.document(), "a").unwrap().unwrap();
        dom.remove(id).unwrap();
        assert_eq!(
            expression.evaluate(&dom, dom.document()).unwrap(),
            Value::Number(3.)
        );
    }
    #[test]
    fn utf16_literals_translate_and_substring_keep_unpaired_units() {
        let dom = fixture();
        let units = [39, 0xd800, 0x61, 0xdc00, 39];
        let literal = crate::dom_string::from_units(&units);
        assert_eq!(
            eval(&dom, &literal),
            Value::String(crate::dom_string::from_units(&units[1..4]))
        );
        let expression = crate::dom_string::from_units(
            &"substring("
                .encode_utf16()
                .chain(units)
                .chain(", 1, 1)".encode_utf16())
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            eval(&dom, &expression),
            Value::String(crate::dom_string::from_units(&[0xd800]))
        );
        assert_eq!(
            eval(&dom, "translate('banana','ana','XYZ')"),
            Value::String("bXYXYX".into())
        );
    }
    #[test]
    fn bounded_ast_rejects_flat_operator_chains_before_recursive_evaluation() {
        for source in [
            std::iter::repeat_n("1", 1024).collect::<Vec<_>>().join("+"),
            std::iter::repeat_n("*", 1024).collect::<Vec<_>>().join("/"),
        ] {
            assert!(matches!(
                Expression::compile(&source, &mut |_| Ok(String::new())),
                Err(Error::Limit)
            ));
        }
        let short = Expression::compile("1+2*3", &mut |_| Ok(String::new())).unwrap();
        assert!(short.retained_bytes() > 0);
        let dom = fixture();
        assert_eq!(
            short.evaluate(&dom, dom.document()).unwrap(),
            Value::Number(7.)
        );
    }

    #[test]
    fn all_axes_keep_attribute_namespace_and_tree_node_principal_types() {
        let dom = crate::xml::parse(
            "<r xmlns:p='urn:p'><a id='a'><p:b id='b'/><c id='c'/></a><d id='d'/></r>",
            "text/xml",
        )
        .into_runtime()
        .unwrap();
        for (query, count) in [
            ("//p:b/ancestor::*", 2.),
            ("//p:b/ancestor-or-self::*", 3.),
            ("//p:b/parent::*", 1.),
            ("//p:b/self::*", 1.),
            ("/r/descendant::*", 4.),
            ("/r/descendant-or-self::*", 5.),
            ("/r/child::*", 2.),
            ("//p:b/following-sibling::*", 1.),
            ("//c/preceding-sibling::*", 1.),
            ("//p:b/following::*", 2.),
            ("//d/preceding::*", 3.),
            ("//p:b/attribute::*", 1.),
            ("//p:b/namespace::*", 2.),
            ("//p:b/@id/parent::*", 1.),
            ("//p:b/@id/child::node()", 0.),
        ] {
            assert_eq!(
                eval(&dom, &format!("count({query})")),
                Value::Number(count),
                "{query}"
            );
        }
    }

    #[test]
    fn cumulative_namespace_and_temporary_string_budgets_reject_before_unbounded_growth() {
        let path = std::iter::repeat_n("p:a", 40).collect::<Vec<_>>().join("/");
        assert!(matches!(
            Expression::compile(&path, &mut |_| Ok("x".repeat(1024 * 1024))),
            Err(Error::Limit)
        ));
        let dom = crate::html::parse(&format!("<body>{}</body>", "x".repeat(1024 * 1024)))
            .into_runtime()
            .unwrap();
        let expression = format!(
            "concat({})",
            std::iter::repeat_n("string(.)", 80)
                .collect::<Vec<_>>()
                .join(",")
        );
        let compiled = Expression::compile(&expression, &mut |_| Ok(String::new())).unwrap();
        assert!(matches!(
            compiled.evaluate(&dom, dom.document()),
            Err(Error::Limit)
        ));
    }
}
