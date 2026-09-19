//! Native sanitizer configuration, independent of author-visible dictionaries.
//! Bindings must finish Web IDL conversion before validating or installing it.
use crate::{DomError, DomRuntime, NodeId, NodeKind};
use std::collections::BTreeSet;
mod default_policy;

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Name {
    pub name: String,
    pub namespace: Option<String>,
}
impl Name {
    pub fn new(name: String, namespace: Option<String>) -> Self {
        Self {
            name,
            namespace: namespace.filter(|value| !value.is_empty()),
        }
    }
    pub fn element(name: &str) -> Self {
        Self::new(name.into(), Some(HTML_NAMESPACE.into()))
    }
    pub fn attribute(name: &str) -> Self {
        Self::new(name.into(), None)
    }
    fn data_attribute(&self) -> bool {
        self.namespace.is_none()
            && crate::dom_string::units(&self.name).starts_with(&[100, 97, 116, 97, 45])
    }
    fn non_replaceable(&self) -> bool {
        matches!(
            (self.namespace.as_deref(), self.name.as_str()),
            (Some(HTML_NAMESPACE), "html")
                | (Some("http://www.w3.org/2000/svg"), "svg")
                | (Some("http://www.w3.org/1998/Math/MathML"), "math")
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub name: Name,
    pub attributes: Option<Vec<Name>>,
    pub remove_attributes: Option<Vec<Name>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub elements: Option<Vec<Element>>,
    pub remove_elements: Option<Vec<Name>>,
    pub replace_with_children_elements: Option<Vec<Name>>,
    pub processing_instructions: Option<Vec<String>>,
    pub remove_processing_instructions: Option<Vec<String>>,
    pub attributes: Option<Vec<Name>>,
    pub remove_attributes: Option<Vec<Name>>,
    pub comments: Option<bool>,
    pub data_attributes: Option<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParserElementDisposition {
    Keep,
    Drop,
    Unwrap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    ConflictingLists,
    Duplicate,
    IntersectingLists,
    InvalidAttributeOverride,
    NonReplaceable,
    DataAttributeConflict,
}
#[derive(Debug)]
pub enum SanitizeError {
    Configuration(ConfigError),
    Dom(DomError),
}
impl From<DomError> for SanitizeError {
    fn from(value: DomError) -> Self {
        Self::Dom(value)
    }
}
fn unique<'a, T: Ord + 'a>(values: impl IntoIterator<Item = &'a T>) -> bool {
    let mut seen = BTreeSet::new();
    values.into_iter().all(|value| seen.insert(value))
}
fn names(values: &Option<Vec<Name>>) -> &[Name] {
    values.as_deref().unwrap_or(&[])
}
fn overlap(a: &[Name], b: &[Name]) -> bool {
    a.iter().any(|name| b.contains(name))
}
fn remove_item<T: PartialEq>(list: &mut Option<Vec<T>>, item: &T) -> bool {
    let Some(list) = list else { return false };
    let before = list.len();
    list.retain(|value| value != item);
    list.len() != before
}
fn append_item<T: PartialEq>(list: &mut Option<Vec<T>>, item: T) -> bool {
    let list = list.get_or_insert_with(Vec::new);
    if list.contains(&item) {
        false
    } else {
        list.push(item);
        true
    }
}
/// Match qualified names exactly; neither attribute names nor animation
/// attributeName values undergo HTML-style case folding here.
fn unsafe_url_attribute(element: &Name, attribute: &Name, value: &str) -> bool {
    let ns = element.namespace.as_deref();
    if ns == Some(SVG_NAMESPACE)
        && matches!(
            element.name.as_str(),
            "animate" | "animateTransform" | "set"
        )
        && attribute.namespace.is_none()
        && attribute.name == "attributeName"
        && matches!(value, "href" | "xlink:href")
    {
        return true;
    }
    let navigates = match ns {
        Some(HTML_NAMESPACE) if attribute.namespace.is_none() => matches!(
            (element.name.as_str(), attribute.name.as_str()),
            ("a" | "area", "href") | ("button" | "input", "formaction") | ("form", "action")
        ),
        Some(SVG_NAMESPACE) => {
            element.name == "a"
                && attribute.name == "href"
                && matches!(attribute.namespace.as_deref(), None | Some(XLINK_NAMESPACE))
        }
        Some(MATHML_NAMESPACE) => {
            attribute.name == "href"
                && matches!(attribute.namespace.as_deref(), None | Some(XLINK_NAMESPACE))
        }
        _ => false,
    };
    navigates
        && url::Url::parse(&String::from_utf16_lossy(&crate::dom_string::units(value)))
            .is_ok_and(|url| url.scheme() == "javascript")
}
fn compare_names(a: &Name, b: &Name) -> std::cmp::Ordering {
    let namespace = match (&a.namespace, &b.namespace) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => crate::dom_string::units(a).cmp(&crate::dom_string::units(b)),
    };
    namespace
        .then_with(|| crate::dom_string::units(&a.name).cmp(&crate::dom_string::units(&b.name)))
}
impl Config {
    pub fn default_preset() -> Self {
        default_policy::configuration()
    }
    /// Web IDL get() exposes sorted copies, independent of native storage
    /// order. Comparisons use UTF-16 code units, not UTF-8 or scalar order.
    pub fn sorted_snapshot(&self) -> Self {
        let mut config = self.clone();
        if let Some(elements) = &mut config.elements {
            for element in elements.iter_mut() {
                for list in [&mut element.attributes, &mut element.remove_attributes]
                    .into_iter()
                    .flatten()
                {
                    list.sort_by(compare_names);
                }
            }
            elements.sort_by(|a, b| compare_names(&a.name, &b.name));
        }
        for list in [
            &mut config.remove_elements,
            &mut config.replace_with_children_elements,
            &mut config.attributes,
            &mut config.remove_attributes,
        ]
        .into_iter()
        .flatten()
        {
            list.sort_by(compare_names);
        }
        for list in [
            &mut config.processing_instructions,
            &mut config.remove_processing_instructions,
        ]
        .into_iter()
        .flatten()
        {
            list.sort_by_key(|value| crate::dom_string::units(value));
        }
        config
    }

    /// Apply the pinned safe baseline without changing allow/remove list mode.
    /// URL-value filtering belongs to safe tree sanitization, not this mutation.
    pub fn remove_unsafe(&mut self) -> bool {
        let mut changed = false;
        for name in ["base", "embed", "frame", "iframe", "object", "script"] {
            changed = self.remove_element(Name::element(name)) || changed;
        }
        for name in ["script", "use"] {
            changed =
                self.remove_element(Name::new(name.into(), Some(SVG_NAMESPACE.into()))) || changed;
        }
        for name in crate::event_handlers::CONTENT_ATTRIBUTES {
            changed = self.remove_attribute(Name::attribute(name)) || changed;
        }
        changed
    }
    /// Raw native configurations are copied before imposing the safe baseline.
    /// Bindings separately mutate a reusable Sanitizer at sanitization time.
    pub fn filter_tree_safe(
        &self,
        dom: &mut DomRuntime,
        root: NodeId,
    ) -> Result<(), SanitizeError> {
        self.validate().map_err(SanitizeError::Configuration)?;
        let mut safe = self.clone();
        safe.remove_unsafe();
        safe.filter_tree_inner(dom, root, true)
    }
    pub fn set_comments(&mut self, allow: bool) -> bool {
        if self.comments == Some(allow) {
            false
        } else {
            self.comments = Some(allow);
            true
        }
    }
    pub fn allow_attribute(&mut self, attribute: Name) -> bool {
        if let Some(attributes) = &mut self.attributes {
            if (self.data_attributes == Some(true) && attribute.data_attribute())
                || attributes.contains(&attribute)
            {
                return false;
            }
            for element in self.elements.iter_mut().flatten() {
                remove_item(&mut element.attributes, &attribute);
            }
            attributes.push(attribute);
            true
        } else {
            remove_item(&mut self.remove_attributes, &attribute)
        }
    }
    pub fn remove_attribute(&mut self, attribute: Name) -> bool {
        if self.attributes.is_some() {
            let mut changed = remove_item(&mut self.attributes, &attribute);
            for element in self.elements.iter_mut().flatten() {
                changed = remove_item(&mut element.attributes, &attribute) || changed;
                remove_item(&mut element.remove_attributes, &attribute);
            }
            changed
        } else {
            if names(&self.remove_attributes).contains(&attribute) {
                return false;
            }
            for element in self.elements.iter_mut().flatten() {
                remove_item(&mut element.attributes, &attribute);
                remove_item(&mut element.remove_attributes, &attribute);
            }
            append_item(&mut self.remove_attributes, attribute)
        }
    }
    pub fn set_data_attributes(&mut self, allow: bool) -> bool {
        let Some(attributes) = &mut self.attributes else {
            return false;
        };
        if self.data_attributes == Some(allow) {
            return false;
        }
        if allow {
            attributes.retain(|attribute| !attribute.data_attribute());
            for element in self.elements.iter_mut().flatten() {
                if let Some(attributes) = &mut element.attributes {
                    attributes.retain(|attribute| !attribute.data_attribute());
                }
            }
        }
        self.data_attributes = Some(allow);
        true
    }
    pub fn allow_processing_instruction(&mut self, target: String) -> bool {
        if self.processing_instructions.is_some() {
            append_item(&mut self.processing_instructions, target)
        } else {
            remove_item(&mut self.remove_processing_instructions, &target)
        }
    }
    pub fn remove_processing_instruction(&mut self, target: String) -> bool {
        if self.processing_instructions.is_some() {
            remove_item(&mut self.processing_instructions, &target)
        } else {
            append_item(&mut self.remove_processing_instructions, target)
        }
    }
    pub fn remove_element(&mut self, name: Name) -> bool {
        let changed = remove_item(&mut self.replace_with_children_elements, &name);
        if let Some(elements) = &mut self.elements {
            let before = elements.len();
            elements.retain(|element| element.name != name);
            changed || elements.len() != before
        } else {
            append_item(&mut self.remove_elements, name) || changed
        }
    }
    pub fn allow_element(&mut self, mut element: Element) -> bool {
        if element.attributes.is_none() && element.remove_attributes.is_none() {
            element.remove_attributes = Some(Vec::new());
        }
        if let Some(elements) = &mut self.elements {
            let changed = remove_item(&mut self.replace_with_children_elements, &element.name);
            if let Some(global) = &self.attributes {
                if let Some(allowed) = &mut element.attributes {
                    let mut seen = BTreeSet::new();
                    allowed.retain(|name| {
                        seen.insert(name.clone())
                            && !global.contains(name)
                            && !(self.data_attributes == Some(true) && name.data_attribute())
                    });
                }
                if let Some(removed) = &mut element.remove_attributes {
                    let mut seen = BTreeSet::new();
                    removed.retain(|name| seen.insert(name.clone()) && global.contains(name));
                }
            } else {
                if let Some(allowed) = &mut element.attributes {
                    let removed = element.remove_attributes.take().unwrap_or_default();
                    let mut seen = BTreeSet::new();
                    allowed.retain(|name| {
                        seen.insert(name.clone())
                            && !removed.contains(name)
                            && !names(&self.remove_attributes).contains(name)
                    });
                }
                if let Some(removed) = &mut element.remove_attributes {
                    let mut seen = BTreeSet::new();
                    removed.retain(|name| {
                        seen.insert(name.clone()) && !names(&self.remove_attributes).contains(name)
                    });
                }
            }
            if let Some(index) = elements
                .iter()
                .position(|current| current.name == element.name)
            {
                if elements[index] == element {
                    return changed;
                }
                elements.remove(index);
            }
            elements.push(element);
            true
        } else {
            if element.attributes.is_some() || !names(&element.remove_attributes).is_empty() {
                return false;
            }
            let changed = remove_item(&mut self.replace_with_children_elements, &element.name);
            remove_item(&mut self.remove_elements, &element.name) || changed
        }
    }
    pub fn replace_element_with_children(&mut self, name: Name) -> bool {
        if name.non_replaceable() {
            return false;
        }
        let mut changed = false;
        if let Some(elements) = &mut self.elements {
            let before = elements.len();
            elements.retain(|element| element.name != name);
            changed = elements.len() != before;
        }
        changed = remove_item(&mut self.remove_elements, &name) || changed;
        append_item(&mut self.replace_with_children_elements, name) || changed
    }
    /// Filter an inert native tree. Callers must not expose intermediate
    /// mutations or run author code while this operation owns the tree.
    pub fn filter_tree(&self, dom: &mut DomRuntime, root: NodeId) -> Result<(), SanitizeError> {
        self.validate().map_err(SanitizeError::Configuration)?;
        self.filter_tree_inner(dom, root, false)
    }
    /// Only for a snapshot of a natively branded, already-configured Sanitizer.
    /// Modifiers operate on its admitted state; they do not reconstruct an
    /// authored dictionary. Raw configuration callers must use filter_tree.
    pub fn filter_instance_snapshot(
        &self,
        dom: &mut DomRuntime,
        root: NodeId,
        safe: bool,
    ) -> Result<(), SanitizeError> {
        if safe {
            let mut config = self.clone();
            config.remove_unsafe();
            config.filter_tree_inner(dom, root, true)
        } else {
            self.filter_tree_inner(dom, root, false)
        }
    }
    fn element_disposition(&self, name: &Name, safe: bool) -> ParserElementDisposition {
        if safe
            && matches!(
                (name.namespace.as_deref(), name.name.as_str()),
                (
                    Some(HTML_NAMESPACE),
                    "base" | "embed" | "frame" | "iframe" | "object" | "script"
                ) | (Some(SVG_NAMESPACE), "script" | "use")
            )
        {
            return ParserElementDisposition::Drop;
        }
        if names(&self.replace_with_children_elements).contains(name) {
            return ParserElementDisposition::Unwrap;
        }
        if self
            .elements
            .as_ref()
            .is_some_and(|elements| !elements.iter().any(|element| element.name == *name))
            || names(&self.remove_elements).contains(name)
        {
            ParserElementDisposition::Drop
        } else {
            ParserElementDisposition::Keep
        }
    }
    fn keeps_attribute(&self, name: &Name, attr: &Name, value: &str, safe: bool) -> bool {
        let local = self
            .elements
            .as_ref()
            .and_then(|elements| elements.iter().find(|element| element.name == *name));
        let locally_removed =
            local.is_some_and(|element| names(&element.remove_attributes).contains(attr));
        let keep = if locally_removed {
            false
        } else if let Some(global) = &self.attributes {
            global.contains(attr)
                || local.is_some_and(|element| names(&element.attributes).contains(attr))
                || (self.data_attributes == Some(true) && attr.data_attribute())
        } else {
            !names(&self.remove_attributes).contains(attr)
                && local
                    .and_then(|element| element.attributes.as_ref())
                    .is_none_or(|allowed| allowed.contains(attr))
        };
        keep && !(safe
            && (unsafe_url_attribute(name, attr, value)
                || (attr.namespace.is_none()
                    && crate::event_handlers::CONTENT_ATTRIBUTES.contains(&attr.name.as_str()))))
    }
    /// Token admission before parser constructors/publication. Names and values
    /// use decoded native DOMString storage. The caller owns configuration
    /// conversion/admission; safe URL and mandatory baseline checks stay native.
    pub fn filter_parser_element(
        &self,
        name: &markup5ever::QualName,
        attrs: &mut Vec<markup5ever::Attribute>,
        safe: bool,
    ) -> ParserElementDisposition {
        let name = Name::new(name.local.to_string(), Some(name.ns.to_string()));
        let disposition = self.element_disposition(&name, safe);
        if disposition == ParserElementDisposition::Keep {
            attrs.retain(|attribute| {
                let attr = Name::new(
                    attribute.name.local.to_string(),
                    Some(attribute.name.ns.to_string()),
                );
                self.keeps_attribute(&name, &attr, &attribute.value, safe)
            });
        } else {
            attrs.clear();
        }
        disposition
    }
    pub fn allows_parser_character_node(&self, node: &NodeKind) -> bool {
        match node {
            NodeKind::Comment { .. } => self.comments == Some(true),
            NodeKind::ProcessingInstruction { target, .. } => {
                if let Some(allowed) = &self.processing_instructions {
                    allowed.contains(target)
                } else {
                    !self
                        .remove_processing_instructions
                        .as_ref()
                        .is_some_and(|removed| removed.contains(target))
                }
            }
            _ => true,
        }
    }
    fn filter_tree_inner(
        &self,
        dom: &mut DomRuntime,
        root: NodeId,
        safe: bool,
    ) -> Result<(), SanitizeError> {
        enum Task {
            Children(NodeId),
            Visit(NodeId),
            Unwrap(NodeId),
        }
        let mut tasks = vec![Task::Children(root)];
        let mut visited = BTreeSet::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Children(node) => {
                    for child in dom.arena().get(node)?.children.iter().rev() {
                        tasks.push(Task::Visit(*child));
                    }
                }
                Task::Unwrap(node) => {
                    let Some(parent) = dom.parent(node)? else {
                        continue;
                    };
                    let children = dom.arena().get(node)?.children.clone();
                    for child in children {
                        if !matches!(dom.arena().get(child)?.kind, NodeKind::ShadowRoot { .. }) {
                            dom.insert_before(parent, child, Some(node))?;
                        }
                    }
                    dom.remove(node)?;
                }
                Task::Visit(node) => {
                    if !visited.insert(node) {
                        return Err(DomError::Cycle.into());
                    }
                    let record = dom.arena().get(node)?.clone();
                    match &record.kind {
                        NodeKind::Comment { .. } => {
                            if !self.allows_parser_character_node(&record.kind) {
                                dom.remove(node)?;
                            }
                        }
                        NodeKind::ProcessingInstruction { .. } => {
                            let keep = self.allows_parser_character_node(&record.kind);
                            if !keep {
                                dom.remove(node)?;
                            }
                        }
                        NodeKind::Element { name } => {
                            let name = Name::new(name.local.to_string(), Some(name.ns.to_string()));
                            if self.element_disposition(&name, safe)
                                == ParserElementDisposition::Unwrap
                            {
                                tasks.push(Task::Unwrap(node));
                                for child in record.children.iter().rev() {
                                    if !matches!(
                                        dom.arena().get(*child)?.kind,
                                        NodeKind::ShadowRoot { .. }
                                    ) {
                                        tasks.push(Task::Visit(*child));
                                    }
                                }
                                continue;
                            }
                            if self.element_disposition(&name, safe)
                                == ParserElementDisposition::Drop
                            {
                                dom.remove(node)?;
                                continue;
                            }
                            for attribute in record.attributes.records() {
                                let attr = Name::new(
                                    attribute.local_name.clone(),
                                    attribute.namespace.clone(),
                                );
                                if !self.keeps_attribute(&name, &attr, &attribute.value, safe) {
                                    dom.remove_attribute_ns(
                                        node,
                                        attr.namespace.as_deref(),
                                        &attr.name,
                                    )?;
                                    // Sanitization precedes custom-element construction. Unlike
                                    // ordinary removeAttribute, removing the parsed is attribute
                                    // also cancels its pending customized-built-in definition.
                                    if attr.namespace.is_none() && attr.name == "is" {
                                        dom.arena_mut().get_mut(node)?.custom_element_is = None;
                                    }
                                }
                            }
                            tasks.push(Task::Children(node));
                            if let Some(content) = record.template_content {
                                tasks.push(Task::Children(content));
                            }
                        }
                        NodeKind::ShadowRoot { .. } | NodeKind::DocumentFragment => {
                            tasks.push(Task::Children(node))
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
    /// Complete omitted fields after Web IDL conversion. Safe sinks use
    /// restrictive defaults; the unsafe empty configuration is permissive.
    /// This is separate from selecting the built-in default preset.
    pub fn canonicalize(mut self, permissive_defaults: bool) -> Result<Self, ConfigError> {
        if self.elements.is_none() && self.remove_elements.is_none() {
            self.remove_elements = Some(Vec::new());
        }
        if self.attributes.is_none() && self.remove_attributes.is_none() {
            self.remove_attributes = Some(Vec::new());
        }
        if self.processing_instructions.is_none() && self.remove_processing_instructions.is_none() {
            if permissive_defaults {
                self.remove_processing_instructions = Some(Vec::new());
            } else {
                self.processing_instructions = Some(Vec::new());
            }
        }
        for element in self.elements.iter_mut().flatten() {
            if element.attributes.is_none() && element.remove_attributes.is_none() {
                element.remove_attributes = Some(Vec::new());
            }
        }
        self.comments.get_or_insert(permissive_defaults);
        if self.attributes.is_some() {
            self.data_attributes.get_or_insert(permissive_defaults);
        }
        self.validate()?;
        Ok(self)
    }
    /// Validate presence as well as contents: two explicitly empty mutually
    /// exclusive lists are invalid. Never modify the caller's candidate.
    pub fn validate(&self) -> Result<(), ConfigError> {
        use ConfigError::*;
        if (self.elements.is_some() && self.remove_elements.is_some())
            || (self.attributes.is_some() && self.remove_attributes.is_some())
            || (self.processing_instructions.is_some()
                && self.remove_processing_instructions.is_some())
        {
            return Err(ConflictingLists);
        }
        if !unique(self.elements.iter().flatten().map(|element| &element.name)) {
            return Err(Duplicate);
        }
        for values in [
            &self.remove_elements,
            &self.replace_with_children_elements,
            &self.attributes,
            &self.remove_attributes,
        ] {
            if !unique(names(values)) {
                return Err(Duplicate);
            }
        }
        for values in [
            &self.processing_instructions,
            &self.remove_processing_instructions,
        ] {
            if !unique(values.iter().flatten()) {
                return Err(Duplicate);
            }
        }
        let replacements = names(&self.replace_with_children_elements);
        if replacements.iter().any(Name::non_replaceable) {
            return Err(NonReplaceable);
        }
        if overlap(replacements, names(&self.remove_elements))
            || self
                .elements
                .iter()
                .flatten()
                .any(|element| replacements.contains(&element.name))
        {
            return Err(IntersectingLists);
        }
        if self.remove_attributes.is_some() && self.data_attributes.is_some() {
            return Err(DataAttributeConflict);
        }
        if self.data_attributes == Some(true)
            && names(&self.attributes).iter().any(Name::data_attribute)
        {
            return Err(DataAttributeConflict);
        }
        for element in self.elements.iter().flatten() {
            let allowed = names(&element.attributes);
            let removed = names(&element.remove_attributes);
            if !unique(allowed) || !unique(removed) {
                return Err(Duplicate);
            }
            if let Some(global) = &self.attributes {
                if overlap(global, allowed) {
                    return Err(IntersectingLists);
                }
                if removed.iter().any(|name| !global.contains(name)) {
                    return Err(InvalidAttributeOverride);
                }
                if self.data_attributes == Some(true) && allowed.iter().any(Name::data_attribute) {
                    return Err(DataAttributeConflict);
                }
            }
            if let Some(global) = &self.remove_attributes {
                if element.attributes.is_some() && element.remove_attributes.is_some() {
                    return Err(ConflictingLists);
                }
                if overlap(global, allowed) || overlap(global, removed) {
                    return Err(IntersectingLists);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn configuration_snapshot_sorts_namespace_then_utf16_without_mutating_policy() {
        let qualified = |name: &str, namespace: Option<&str>| {
            Name::new(name.into(), namespace.map(str::to_owned))
        };
        let config = Config {
            remove_elements: Some(vec![
                qualified("a", Some("z")),
                qualified("z", None),
                qualified("a", None),
                qualified("b", Some("a")),
            ]),
            remove_attributes: Some(vec![
                Name::attribute("\u{e000}"),
                Name::attribute("\u{10000}"),
            ]),
            remove_processing_instructions: Some(vec!["z".into(), "a".into()]),
            comments: Some(true),
            ..Default::default()
        };
        let original = config.clone();
        let sorted = config.sorted_snapshot();
        assert_eq!(config, original);
        assert_eq!(
            sorted.remove_elements,
            Some(vec![
                qualified("a", None),
                qualified("z", None),
                qualified("b", Some("a")),
                qualified("a", Some("z"))
            ])
        );
        assert_eq!(
            sorted.remove_attributes,
            Some(vec![
                Name::attribute("\u{10000}"),
                Name::attribute("\u{e000}")
            ])
        );
        assert_eq!(
            sorted.remove_processing_instructions,
            Some(vec!["a".into(), "z".into()])
        );
        let mut local = Config::default_preset();
        local.elements.as_mut().unwrap().reverse();
        let sorted = local.sorted_snapshot();
        assert!(
            sorted
                .elements
                .unwrap()
                .windows(2)
                .all(|pair| compare_names(&pair[0].name, &pair[1].name).is_le())
        );
    }
    #[test]
    fn safe_baseline_preserves_configuration_modes_namespaces_and_idempotence() {
        let mut config = Config::default().canonicalize(true).unwrap();
        assert!(config.remove_unsafe());
        assert!(!config.remove_unsafe());
        assert_eq!(config.validate(), Ok(()));
        let expected = ["base", "embed", "frame", "iframe", "object", "script"]
            .into_iter()
            .map(Name::element)
            .chain(
                ["script", "use"]
                    .into_iter()
                    .map(|name| Name::new(name.into(), Some(SVG_NAMESPACE.into()))),
            )
            .collect::<Vec<_>>();
        assert_eq!(config.remove_elements, Some(expected));
        assert!(config.attributes.is_none());
        assert_eq!(
            names(&config.remove_attributes).len(),
            crate::event_handlers::CONTENT_ATTRIBUTES.len()
        );
        let mut preset = Config::default_preset();
        let before = preset.clone();
        assert!(!preset.remove_unsafe());
        assert_eq!(preset, before);
        let mut dom=crate::html::parse("<div id=root><use>keep</use><script>drop</script><svg><use></use><script>drop</script></svg><math><script>keep</script></math><b onclick=bad onmadeup=keep>text</b></div>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        config.filter_tree(&mut dom, root).unwrap();
        assert_eq!(
            dom.serialize(root).unwrap(),
            "<div id=\"root\"><use>keep</use><svg></svg><math><script>keep</script></math><b onmadeup=\"keep\">text</b></div>"
        );
    }
    #[test]
    fn safe_url_filter_uses_basic_url_parsing_and_exact_qualified_names() {
        let href = Name::attribute("href");
        let anchor = Name::element("a");
        let mut units = "javascript:".encode_utf16().collect::<Vec<_>>();
        units.push(0xd800);
        assert!(unsafe_url_attribute(
            &anchor,
            &href,
            &crate::dom_string::from_units(&units)
        ));
        let mut units = "data-".encode_utf16().collect::<Vec<_>>();
        units.push(0xdfff);
        assert!(Name::attribute(&crate::dom_string::from_units(&units)).data_attribute());
        for value in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "\t java\nscript:alert(1)\r",
            "\u{0000}javascript:alert(1)",
        ] {
            assert!(unsafe_url_attribute(&anchor, &href, value), "{value:?}");
        }
        for value in [
            "/javascript:alert(1)",
            "https://example.test/javascript:alert(1)",
            "data:text/html,foo",
            "java script:alert(1)",
            "javascript%3Aalert(1)",
        ] {
            assert!(!unsafe_url_attribute(&anchor, &href, value), "{value:?}");
        }
        for namespace in [SVG_NAMESPACE, MATHML_NAMESPACE] {
            let element = Name::new(
                if namespace == SVG_NAMESPACE {
                    "a"
                } else {
                    "mrow"
                }
                .into(),
                Some(namespace.into()),
            );
            for namespace in [None, Some(XLINK_NAMESPACE)] {
                assert!(unsafe_url_attribute(
                    &element,
                    &Name::new("href".into(), namespace.map(str::to_owned)),
                    "javascript:0"
                ));
            }
        }
        assert!(!unsafe_url_attribute(
            &anchor,
            &Name::new("href".into(), Some(XLINK_NAMESPACE.into())),
            "javascript:0"
        ));
        assert!(!unsafe_url_attribute(
            &Name::element("img"),
            &Name::attribute("src"),
            "javascript:0"
        ));
        for name in ["animate", "animateTransform", "set"] {
            let element = Name::new(name.into(), Some(SVG_NAMESPACE.into()));
            for value in ["href", "xlink:href"] {
                assert!(unsafe_url_attribute(
                    &element,
                    &Name::attribute("attributeName"),
                    value
                ));
            }
            for value in [" href ", "HREF", "xlink:Href"] {
                assert!(!unsafe_url_attribute(
                    &element,
                    &Name::attribute("attributeName"),
                    value
                ));
            }
        }
        assert!(!unsafe_url_attribute(
            &Name::new("animateMotion".into(), Some(SVG_NAMESPACE.into())),
            &Name::attribute("attributeName"),
            "href"
        ));
    }
    #[test]
    fn safe_tree_filter_copies_policy_and_filters_template_and_shadow_urls() {
        let config = Config::default().canonicalize(true).unwrap();
        let before = config.clone();
        let input = "<div id=root><a href='java&#10;script:0' onload=bad>link</a><template><form action='javascript:0'></form></template><section><template shadowrootmode=open shadowrootserializable><svg><a href='javascript:0'></a><set attributeName=href></set></svg></template></section></div>";
        let mut dom = crate::html::parse_inert_with_declarative_shadow(input)
            .into_runtime()
            .unwrap();
        let root = dom.find_by_id("root").unwrap();
        config.filter_tree_safe(&mut dom, root).unwrap();
        assert_eq!(config, before);
        let options = crate::HtmlSerializationOptions {
            serializable_shadow_roots: true,
            ..Default::default()
        };
        let markup = crate::serialize_html_with_options(
            dom.arena().node_ref(root).unwrap(),
            &crate::serialization::LocalResolver(&dom),
            &options,
            false,
        )
        .unwrap();
        assert!(!markup.contains("javascript"));
        assert!(!markup.contains("href="));
        assert!(!markup.contains("action="));
        assert!(!markup.contains("attributeName="));
        assert!(!markup.contains("onload="));
        let mut unsafe_dom =
            crate::html::parse("<div id=root><a href='javascript:0' onclick=bad>keep</a></div>")
                .into_runtime()
                .unwrap();
        let root = unsafe_dom.find_by_id("root").unwrap();
        config.filter_tree(&mut unsafe_dom, root).unwrap();
        assert!(unsafe_dom.serialize(root).unwrap().contains("javascript:0"));
        assert!(unsafe_dom.serialize(root).unwrap().contains("onclick="));
    }

    use super::*;
    #[test]
    fn allow_element_normalizes_overrides_and_moves_updated_rules_to_the_end() {
        let mut config = Config {
            elements: Some(vec![]),
            attributes: Some(vec![Name::attribute("class")]),
            data_attributes: Some(true),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        let plain = |tag: &str| Element {
            name: Name::element(tag),
            attributes: None,
            remove_attributes: None,
        };
        assert!(config.allow_element(plain("div")));
        assert!(config.allow_element(plain("span")));
        assert!(!config.allow_element(plain("div")));
        let rule = Element {
            name: Name::element("div"),
            attributes: Some(vec![
                Name::attribute("title"),
                Name::attribute("class"),
                Name::attribute("title"),
                Name::attribute("data-x"),
            ]),
            remove_attributes: Some(vec![Name::attribute("class"), Name::attribute("unknown")]),
        };
        assert!(config.allow_element(rule));
        let elements = config.elements.as_ref().unwrap();
        assert_eq!(elements[0].name, Name::element("span"));
        assert_eq!(names(&elements[1].attributes), &[Name::attribute("title")]);
        assert_eq!(
            names(&elements[1].remove_attributes),
            &[Name::attribute("class")]
        );
        assert_eq!(config.validate(), Ok(()));
        let mut config = Config::default().canonicalize(true).unwrap();
        config.remove_element(Name::element("div"));
        let before = config.clone();
        assert!(!config.allow_element(Element {
            name: Name::element("div"),
            attributes: Some(vec![]),
            remove_attributes: None
        }));
        assert_eq!(config, before);
        assert!(config.allow_element(plain("div")));
        assert!(!config.allow_element(plain("div")));
    }
    #[test]
    fn attribute_modifiers_reconcile_global_and_local_rules() {
        let mut config = Config {
            attributes: Some(vec![Name::attribute("class")]),
            elements: Some(vec![Element {
                name: Name::element("div"),
                attributes: Some(vec![
                    Name::attribute("title"),
                    Name::attribute("data-local"),
                ]),
                remove_attributes: Some(vec![Name::attribute("class")]),
            }]),
            data_attributes: Some(false),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        assert!(config.allow_attribute(Name::attribute("title")));
        assert!(!config.allow_attribute(Name::attribute("title")));
        assert_eq!(
            names(&config.elements.as_ref().unwrap()[0].attributes),
            &[Name::attribute("data-local")]
        );
        assert!(config.remove_attribute(Name::attribute("class")));
        assert!(!config.remove_attribute(Name::attribute("class")));
        assert!(names(&config.elements.as_ref().unwrap()[0].remove_attributes).is_empty());
        assert!(config.allow_attribute(Name::attribute("data-global")));
        assert!(config.set_data_attributes(true));
        assert!(!config.set_data_attributes(true));
        assert!(!config.allow_attribute(Name::attribute("data-new")));
        assert_eq!(names(&config.attributes), &[Name::attribute("title")]);
        assert!(names(&config.elements.as_ref().unwrap()[0].attributes).is_empty());
        assert_eq!(config.validate(), Ok(()));
        let mut removal = Config::default().canonicalize(true).unwrap();
        let before = removal.clone();
        assert!(!removal.set_data_attributes(false));
        assert_eq!(removal, before);
        assert!(removal.remove_attribute(Name::attribute("title")));
        assert!(!removal.remove_attribute(Name::attribute("title")));
        assert!(removal.allow_attribute(Name::attribute("title")));
        assert!(!removal.allow_attribute(Name::attribute("title")));
        assert_eq!(removal.validate(), Ok(()));
    }
    #[test]
    fn modifiers_preserve_list_mode_order_and_report_actual_changes() {
        for permissive in [false, true] {
            let mut config = Config::default().canonicalize(permissive).unwrap();
            assert_eq!(
                config.allow_processing_instruction("one".into()),
                !permissive
            );
            assert!(!config.allow_processing_instruction("one".into()));
            assert!(config.remove_processing_instruction("one".into()));
            assert!(!config.remove_processing_instruction("one".into()));
            assert!(config.allow_processing_instruction("one".into()));
            assert!(!config.set_comments(permissive));
            assert!(config.set_comments(!permissive));
            assert_eq!(config.validate(), Ok(()));
        }
        let mut config = Config::default_preset();
        assert!(config.remove_element(Name::element("div")));
        assert!(!config.remove_element(Name::element("div")));
        assert!(config.replace_element_with_children(Name::element("span")));
        assert!(!config.replace_element_with_children(Name::element("span")));
        assert!(config.remove_element(Name::element("span")));
        assert!(names(&config.replace_with_children_elements).is_empty());
        let before = config.clone();
        assert!(!config.replace_element_with_children(Name::element("html")));
        assert_eq!(config, before);
        assert_eq!(config.validate(), Ok(()));
        let mut config = Config::default().canonicalize(true).unwrap();
        assert!(config.remove_element(Name::element("a")));
        assert!(config.remove_element(Name::element("b")));
        assert!(config.replace_element_with_children(Name::element("a")));
        assert_eq!(config.remove_elements, Some(vec![Name::element("b")]));
        assert!(config.remove_element(Name::element("a")));
        assert_eq!(
            config.remove_elements,
            Some(vec![Name::element("b"), Name::element("a")])
        );
        assert_eq!(config.validate(), Ok(()));
    }
    #[test]
    fn default_preset_is_valid_and_preserves_safe_namespace_specific_markup() {
        let config = Config::default_preset();
        assert_eq!(config.validate(), Ok(()));
        let mut dom=crate::html::parse("<div id=root><script>bad</script><b class=ok dir=ltr onclick=bad()>bold</b><svg><circle cx=2 cy=3 r=4 onload=bad()></circle></svg><!--comment--></div>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        config.filter_tree(&mut dom, root).unwrap();
        let markup = dom.serialize(root).unwrap();
        assert!(
            markup.contains("<b dir=\"ltr\">bold</b>"),
            "safe formatting and allowed direction: {markup}"
        );
        assert!(
            !markup.contains("class="),
            "default preset excludes authored class: {markup}"
        );
        assert!(markup.contains("<circle cx=\"2\" cy=\"3\" r=\"4\"></circle>"));
        assert!(!markup.contains("script"));
        assert!(!markup.contains("onclick"));
        assert!(!markup.contains("onload"));
        assert!(!markup.contains("comment"));
        // Explicitly permissive configuration and the default preset are
        // different choices; an empty dictionary must not select this preset.
        assert_ne!(config, Config::default().canonicalize(true).unwrap());
    }
    #[test]
    fn native_filter_prunes_unwraps_and_filters_attributes_without_promoting_shadow_children() {
        let mut dom=crate::html::parse("<div id=root><section><script>drop</script><span class=c title=t data-x=x>kept</span></section><!--drop--></div>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        let config = Config {
            remove_elements: Some(vec![Name::element("script")]),
            replace_with_children_elements: Some(vec![Name::element("section")]),
            attributes: Some(vec![Name::attribute("class")]),
            data_attributes: Some(false),
            comments: Some(false),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        config.filter_tree(&mut dom, root).unwrap();
        assert_eq!(
            dom.serialize(root).unwrap(),
            "<div id=\"root\"><span class=\"c\">kept</span></div>"
        );
        let host = dom.create_element("section");
        dom.append_child(root, host).unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: crate::ShadowMode::Closed,
            clonable: false,
        });
        dom.append_child(host, shadow).unwrap();
        let secret = dom.create_text("secret");
        dom.append_child(shadow, secret).unwrap();
        let light = dom.create_text("light");
        dom.append_child(host, light).unwrap();
        config.filter_tree(&mut dom, root).unwrap();
        assert_eq!(dom.parent(shadow).unwrap(), Some(host));
        assert_eq!(dom.parent(light).unwrap(), Some(root));
        assert!(!dom.serialize(root).unwrap().contains("secret"));
    }
    #[test]
    fn native_filter_visits_template_and_shadow_contents_and_processing_instructions() {
        let mut dom=crate::html::parse_inert_with_declarative_shadow("<div id=root><section><template shadowrootmode=open shadowrootserializable><script>bad</script><b>good</b></template></section><template><script>bad</script><i>good</i></template></div>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        let pi = dom.create(NodeKind::ProcessingInstruction {
            target: "drop".into(),
            data: "x".into(),
        });
        dom.append_child(root, pi).unwrap();
        let config = Config {
            remove_elements: Some(vec![Name::element("script")]),
            processing_instructions: Some(vec![]),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        config.filter_tree(&mut dom, root).unwrap();
        assert_eq!(dom.parent(pi).unwrap(), None);
        let options = crate::HtmlSerializationOptions {
            serializable_shadow_roots: true,
            ..Default::default()
        };
        assert!(
            !crate::serialize_html_with_options(
                dom.arena().node_ref(root).unwrap(),
                &crate::serialization::LocalResolver(&dom),
                &options,
                false
            )
            .unwrap()
            .contains("bad")
        );
        assert!(!dom.serialize(root).unwrap().contains("script"));
    }
    #[test]
    fn canonicalization_distinguishes_empty_configuration_and_explicit_allowlists() {
        let unsafe_empty = Config::default().canonicalize(true).unwrap();
        assert_eq!(unsafe_empty.comments, Some(true));
        assert_eq!(unsafe_empty.data_attributes, None);
        assert_eq!(unsafe_empty.remove_elements, Some(vec![]));
        assert_eq!(unsafe_empty.remove_attributes, Some(vec![]));
        assert_eq!(unsafe_empty.remove_processing_instructions, Some(vec![]));
        assert_eq!(unsafe_empty.processing_instructions, None);
        let safe_empty = Config::default().canonicalize(false).unwrap();
        assert_eq!(safe_empty.comments, Some(false));
        assert_eq!(safe_empty.processing_instructions, Some(vec![]));
        for permissive in [false, true] {
            let value = Config {
                attributes: Some(vec![]),
                elements: Some(vec![Element {
                    name: Name::element("div"),
                    attributes: None,
                    remove_attributes: None,
                }]),
                ..Default::default()
            }
            .canonicalize(permissive)
            .unwrap();
            assert_eq!(value.data_attributes, Some(permissive));
            assert_eq!(value.elements.unwrap()[0].remove_attributes, Some(vec![]));
        }
        assert_eq!(
            Config {
                data_attributes: Some(true),
                ..Default::default()
            }
            .canonicalize(true),
            Err(ConfigError::DataAttributeConflict)
        );
        let explicit = Config {
            comments: Some(true),
            attributes: Some(vec![]),
            data_attributes: Some(true),
            ..Default::default()
        }
        .canonicalize(false)
        .unwrap();
        assert_eq!(explicit.comments, Some(true));
        assert_eq!(explicit.data_attributes, Some(true));
    }
    #[test]
    fn namespace_canonicalization_preserves_case_and_distinguishes_element_defaults() {
        assert_eq!(
            Name::new("x".into(), Some(String::new())),
            Name::attribute("x")
        );
        assert_ne!(Name::element("x"), Name::attribute("x"));
        assert_ne!(Name::element("DIV"), Name::element("div"));
    }
    #[test]
    fn configuration_presence_duplicates_and_overrides_are_validated_without_mutation() {
        let mut config = Config {
            elements: Some(vec![]),
            remove_elements: Some(vec![]),
            ..Default::default()
        };
        assert_eq!(config.validate(), Err(ConfigError::ConflictingLists));
        config.elements = None;
        config.remove_elements = Some(vec![Name::element("div"), Name::element("div")]);
        assert_eq!(config.validate(), Err(ConfigError::Duplicate));
        config.remove_elements = None;
        config.attributes = Some(vec![Name::attribute("class")]);
        config.elements = Some(vec![Element {
            name: Name::element("div"),
            attributes: Some(vec![Name::attribute("title")]),
            remove_attributes: Some(vec![Name::attribute("class")]),
        }]);
        assert_eq!(config.validate(), Ok(()));
        let saved = config.clone();
        config.elements.as_mut().unwrap()[0].remove_attributes =
            Some(vec![Name::attribute("unknown")]);
        let invalid = config.clone();
        assert_eq!(
            config.validate(),
            Err(ConfigError::InvalidAttributeOverride)
        );
        assert_eq!(config, invalid);
        config = saved;
        config.data_attributes = Some(true);
        config
            .attributes
            .as_mut()
            .unwrap()
            .push(Name::attribute("data-x"));
        assert_eq!(config.validate(), Err(ConfigError::DataAttributeConflict));
    }
    #[test]
    fn replacement_and_processing_instruction_constraints_are_namespace_aware() {
        for name in [
            Name::element("html"),
            Name::new("svg".into(), Some("http://www.w3.org/2000/svg".into())),
            Name::new(
                "math".into(),
                Some("http://www.w3.org/1998/Math/MathML".into()),
            ),
        ] {
            assert_eq!(
                Config {
                    replace_with_children_elements: Some(vec![name]),
                    ..Default::default()
                }
                .validate(),
                Err(ConfigError::NonReplaceable)
            );
        }
        assert!(
            Config {
                replace_with_children_elements: Some(vec![Name::attribute("html")]),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            Config {
                processing_instructions: Some(vec!["target".into(), "target".into()]),
                ..Default::default()
            }
            .validate(),
            Err(ConfigError::Duplicate)
        );
    }
}

/// The real fragment context, captured from the receiver after author
/// conversion. Template contents may live in a different arena from this
/// context, so preparation does not require target/context arena equality.
pub struct FragmentContext {
    pub name: markup5ever::QualName,
    pub attributes: Vec<markup5ever::Attribute>,
    pub has_form_ancestor: bool,
    pub disable_context_shadow: bool,
}
impl FragmentContext {
    pub fn html_body() -> Self {
        Self {
            name: markup5ever::QualName::new(None, HTML_NAMESPACE.into(), "body".into()),
            attributes: Vec::new(),
            has_form_ancestor: false,
            disable_context_shadow: false,
        }
    }

    pub fn capture(dom: &DomRuntime, node: NodeId) -> Result<Self, DomError> {
        let record = dom.arena().get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Err(DomError::NotAnElement(node));
        };
        let attributes = record
            .attributes
            .records()
            .map(|attribute| markup5ever::Attribute {
                name: markup5ever::QualName::new(
                    attribute.prefix.as_deref().map(Into::into),
                    attribute.namespace.as_deref().unwrap_or("").into(),
                    attribute.local_name.as_str().into(),
                ),
                value: attribute.value.clone().into(),
            })
            .collect();
        let mut current = Some(node);
        let mut seen = BTreeSet::new();
        let mut has_form_ancestor = false;
        while let Some(node) = current {
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            has_form_ancestor |= dom.is_element(node, HTML_NAMESPACE, "form");
            current = dom.arena().get(node)?.parent;
        }
        let disable_context_shadow = record.custom_element_shadow_disabled == Some(true)
            || record.children.iter().any(|child| {
                dom.arena()
                    .get(*child)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
            });
        Ok(Self {
            name: name.clone(),
            attributes,
            has_form_ancestor,
            disable_context_shadow,
        })
    }
    pub fn safe_sink_is_noop(&self) -> bool {
        self.name.local.as_ref() == "script"
            && matches!(self.name.ns.as_ref(), HTML_NAMESPACE | SVG_NAMESPACE)
    }
}
/// Build and filter off-tree before replacing any target children. The parser's
/// synthetic root is retained only as the fragment's child container; it is
/// never itself checked against the sanitizer's element allowlist.
pub fn prepare_fragment(
    markup: &str,
    context: FragmentContext,
    config: &Config,
    safe: bool,
) -> Result<(DomRuntime, NodeId), SanitizeError> {
    let (mut parsed, root) = parse_fragment_tree(markup, context, true)?;
    if safe {
        config.filter_tree_safe(&mut parsed, root)?
    } else {
        config.filter_tree(&mut parsed, root)?
    }
    crate::html::apply_fragment_patches(&mut parsed, root)?;
    Ok((parsed, root))
}
/// Parse the safe/unsafe sink fragment before its acquired Sanitizer is used.
/// The binding retains instance identity and applies mandatory mutations only
/// after this parse succeeds, immediately before filtering the detached tree.
pub fn prepare_unfiltered_fragment(
    markup: &str,
    context: FragmentContext,
) -> Result<(DomRuntime, NodeId), DomError> {
    parse_fragment_tree(markup, context, true)
}
/// Legacy HTML sinks use contextual parsing without declarative shadow roots
/// or sanitizer filtering. Parser-created script flags remain inert.
pub fn prepare_inert_fragment(
    markup: &str,
    context: FragmentContext,
) -> Result<(DomRuntime, NodeId), DomError> {
    let (mut parsed, root) = parse_fragment_tree(markup, context, false)?;
    crate::html::apply_fragment_patches(&mut parsed, root)?;
    Ok((parsed, root))
}
fn parse_fragment_tree(
    markup: &str,
    context: FragmentContext,
    allow_shadow: bool,
) -> Result<(DomRuntime, NodeId), DomError> {
    let (parsed, context_node) = crate::html::parse_fragment_dom_string_with_retained_context(
        markup,
        context.name,
        context.attributes,
        context.has_form_ancestor,
        allow_shadow,
        context.disable_context_shadow,
    );
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
                .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
        })
        .ok_or(DomError::NotAnElement(document))?;
    let shadow = parsed
        .arena()
        .get(context_node)?
        .children
        .iter()
        .copied()
        .find(|node| {
            parsed
                .arena()
                .get(*node)
                .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
        });
    if let Some(shadow) = shadow {
        let first = parsed.arena().get(root)?.children.first().copied();
        parsed.insert_before(root, shadow, first)?;
    }
    Ok((parsed, root))
}
impl DomRuntime {
    /// Import the complete filtered fragment before replacement, and publish
    /// the same single ChildrenReplaced mutation used by native innerHTML.
    /// Existing shadow roots on an Element remain attached.
    pub fn import_fragment_children(
        &mut self,
        target: NodeId,
        source: &DomRuntime,
        root: NodeId,
    ) -> Result<Vec<NodeId>, DomError> {
        if !matches!(
            self.arena().get(target)?.kind,
            NodeKind::Element { .. } | NodeKind::ShadowRoot { .. } | NodeKind::DocumentFragment
        ) {
            return Err(DomError::NotAnElement(target));
        }
        let document = self
            .node_document(target)?
            .ok_or(DomError::StaleNode(target))?;
        let mut children = Vec::new();
        for child in source.arena().get(root)?.children.iter().copied() {
            let child = self.import_subtree(source.arena(), child)?;
            self.initialize_cloned_node_document(child, document)?;
            children.push(child);
        }
        Ok(children)
    }
    /// Declarative roots are attachment edges and cannot enter a public host
    /// childList record, including append/prepend and sibling HTML insertions.
    pub fn attach_imported_fragment_shadows(
        &mut self,
        target: NodeId,
        children: &[NodeId],
    ) -> Result<(), DomError> {
        let cursor = self.mutation_cursor();
        let result = (|| {
            for &child in children {
                if matches!(self.arena().get(child)?.kind, NodeKind::ShadowRoot { .. }) {
                    self.append_child(target, child)?;
                }
            }
            Ok(())
        })();
        self.mutations.suppress_tree_observers_since(cursor, target);
        result
    }
    pub fn replace_imported_fragment_children(
        &mut self,
        target: NodeId,
        children: Vec<NodeId>,
    ) -> Result<(), DomError> {
        self.replace_with_new_children(target, children)
    }
    pub fn replace_with_sanitized_fragment(
        &mut self,
        target: NodeId,
        source: &DomRuntime,
        root: NodeId,
    ) -> Result<Vec<NodeId>, DomError> {
        let children = self.import_fragment_children(target, source, root)?;
        self.replace_imported_fragment_children(target, children.clone())?;
        Ok(children)
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    #[test]
    fn stylesheet_cleanup_failures_publish_once_and_preserve_only_failed_suffix() {
        for broken in [0, 1] {
            let mut dom = DomRuntime::new();
            let root = dom.create_element("section");
            dom.retain_native(root).unwrap();
            let old = vec![
                dom.create_element("style"),
                dom.create_element("style"),
                dom.create_element("style"),
            ];
            let new = dom.create_element("style");
            dom.set_adopted_stylesheets(root, old.clone()).unwrap();
            dom.drain_mutations();
            dom.arena_mut().get_mut(old[broken]).unwrap().parent = Some(old[broken]);
            assert!(matches!(
                dom.set_adopted_stylesheets(root, vec![new]),
                Err(DomError::Cycle)
            ));
            assert_eq!(
                dom.arena().get(root).unwrap().adopted_stylesheets,
                vec![new]
            );
            assert_eq!(dom.pending_stylesheet_reclaims, [old[broken]].into());
            for (index, node) in old.iter().enumerate() {
                assert_eq!(dom.arena().get(*node).is_ok(), index == broken);
            }
            assert_eq!(dom.drain_mutations().iter().filter(|mutation|matches!(mutation,crate::Mutation::StyleSheets{node}if *node==root)).count(),1);
            dom.arena_mut().get_mut(old[broken]).unwrap().parent = None;
            dom.set_adopted_stylesheets(root, vec![new]).unwrap();
            assert!(dom.pending_stylesheet_reclaims.is_empty());
            assert!(dom.arena().get(old[broken]).is_err());
            assert!(
                !dom.drain_mutations()
                    .iter()
                    .any(|mutation| matches!(mutation, crate::Mutation::StyleSheets { .. }))
            );
        }
    }
    #[test]
    fn stylesheet_cleanup_identity_moves_and_runtime_replacement_discards_old_ids() {
        let source = std::rc::Rc::new(std::cell::RefCell::new(DomRuntime::new()));
        let destination = std::rc::Rc::new(std::cell::RefCell::new(DomRuntime::new()));
        let sheet = source.borrow_mut().create_element("style");
        source.borrow_mut().retain_native(sheet).unwrap();
        source
            .borrow_mut()
            .pending_stylesheet_reclaims
            .insert(sheet);
        let document = destination.borrow().document();
        let mut transfer =
            DomRuntime::prepare_subtree_migration(&source, &destination, sheet, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut transfer).unwrap();
        assert!(source.borrow().pending_stylesheet_reclaims.is_empty());
        assert!(
            destination
                .borrow()
                .pending_stylesheet_reclaims
                .contains(&moved.root)
        );
        destination
            .borrow_mut()
            .retry_pending_stylesheet_reclaims()
            .unwrap();
        assert!(destination.borrow().arena().get(moved.root).is_ok());
        let mut old = DomRuntime::new();
        let node = old.create_element("style");
        old.pending_stylesheet_reclaims.insert(node);
        let mut replacement = DomRuntime::new();
        let fresh = replacement.create_element("style");
        assert_eq!(node, fresh);
        let (arena, document) = replacement.snapshot();
        old.replace_parsed_document(arena, document).unwrap();
        old.retry_pending_stylesheet_reclaims().unwrap();
        assert!(old.arena().get(fresh).is_ok());
    }
    #[test]
    fn fragment_context_keeps_domstring_attribute_units() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("section");
        let value = crate::dom_string::from_units(&[0xd800, 65, 0xdc00]);
        dom.set_attribute(element, "title", value.clone()).unwrap();
        let context = FragmentContext::capture(&dom, element).unwrap();
        assert_eq!(context.attributes[0].value.as_ref(), value);
        let (mut fragment, root) = prepare_unfiltered_fragment("<b>text</b>", context).unwrap();
        Config::default()
            .canonicalize(true)
            .unwrap()
            .filter_tree(&mut fragment, root)
            .unwrap();
        assert!(fragment.serialize(root).unwrap().contains("text"));
    }
    #[test]
    fn parser_token_filter_matches_tree_policy_before_publication() {
        let name = markup5ever::QualName::new(None, HTML_NAMESPACE.into(), "a".into());
        let attr = |name: &str, value: &str| markup5ever::Attribute {
            name: markup5ever::QualName::new(None, "".into(), name.into()),
            value: value.into(),
        };
        let units = crate::dom_string::from_units(&[0xd800, 120, 0xdc00]);
        let mut attrs = vec![
            attr("href", "java\tscript:alert(1)"),
            attr("onclick", "run()"),
            attr("title", &units),
            attr("data-extra", "value"),
        ];
        let config = Config::default().canonicalize(true).unwrap();
        assert_eq!(
            config.filter_parser_element(&name, &mut attrs, true),
            ParserElementDisposition::Keep
        );
        assert_eq!(
            attrs
                .iter()
                .map(|attr| attr.name.local.as_ref())
                .collect::<Vec<_>>(),
            vec!["title", "data-extra"]
        );
        assert_eq!(attrs[0].value.as_ref(), units);
        let mut script_attrs = vec![attr("id", "script")];
        let script = markup5ever::QualName::new(None, HTML_NAMESPACE.into(), "script".into());
        let unwrap = Config {
            replace_with_children_elements: Some(vec![Name::element("script")]),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        assert_eq!(
            unwrap.filter_parser_element(&script, &mut script_attrs, true),
            ParserElementDisposition::Drop
        );
        let div = markup5ever::QualName::new(None, HTML_NAMESPACE.into(), "div".into());
        let unwrap = Config {
            replace_with_children_elements: Some(vec![Name::element("div")]),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap();
        assert_eq!(
            unwrap.filter_parser_element(&div, &mut Vec::new(), false),
            ParserElementDisposition::Unwrap
        );
        let local = Config {
            attributes: Some(vec![Name::attribute("title"), Name::attribute("class")]),
            elements: Some(vec![Element {
                name: Name::element("a"),
                attributes: None,
                remove_attributes: Some(vec![Name::attribute("class")]),
            }]),
            ..Default::default()
        }
        .canonicalize(false)
        .unwrap();
        let mut attrs = vec![
            attr("title", "kept"),
            attr("class", "removed"),
            attr("id", "removed"),
        ];
        assert_eq!(
            local.filter_parser_element(&name, &mut attrs, false),
            ParserElementDisposition::Keep
        );
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].name.local.as_ref(), "title");
        assert!(!local.allows_parser_character_node(&NodeKind::Comment {
            data: "discard".into()
        }));
        assert!(
            !Config {
                processing_instructions: Some(vec![]),
                ..Default::default()
            }
            .allows_parser_character_node(&NodeKind::ProcessingInstruction {
                target: "start".into(),
                data: "name='x'".into()
            })
        );
    }
    #[test]
    fn removed_is_attribute_clears_only_pending_customization() {
        let context = FragmentContext::html_body();
        let(mut dom,root)=prepare_unfiltered_fragment("<div is='x-div'></div><x-auto is='x-other'></x-auto><template><button is='x-button'></button></template>",context).unwrap();
        let children = dom.arena().get(root).unwrap().children.clone();
        assert_eq!(
            dom.arena()
                .get(children[0])
                .unwrap()
                .custom_element_is
                .as_deref(),
            Some("x-div")
        );
        Config {
            remove_attributes: Some(vec![Name::attribute("is")]),
            ..Default::default()
        }
        .canonicalize(true)
        .unwrap()
        .filter_tree(&mut dom, root)
        .unwrap();
        for node in children.iter().take(2) {
            assert!(dom.arena().get(*node).unwrap().custom_element_is.is_none());
            assert!(
                dom.attribute_in_null_namespace(*node, "is")
                    .unwrap()
                    .is_none()
            );
        }
        let content = dom
            .arena()
            .get(children[2])
            .unwrap()
            .template_content
            .unwrap();
        let button = dom.arena().get(content).unwrap().children[0];
        assert!(dom.arena().get(button).unwrap().custom_element_is.is_none());
        let (mut allowed, root) =
            prepare_unfiltered_fragment("<div is='x-div'></div>", FragmentContext::html_body())
                .unwrap();
        Config::default()
            .canonicalize(true)
            .unwrap()
            .filter_tree(&mut allowed, root)
            .unwrap();
        let div = allowed.arena().get(root).unwrap().children[0];
        assert_eq!(
            allowed
                .arena()
                .get(div)
                .unwrap()
                .custom_element_is
                .as_deref(),
            Some("x-div")
        );
    }

    #[test]
    fn declarative_fragment_context_is_distinct_from_full_document_topmost_root() {
        let markup = "<template shadowrootmode=open><b>shadow</b></template>";
        for local in ["div", "span"] {
            let context = FragmentContext {
                name: markup5ever::QualName::new(None, HTML_NAMESPACE.into(), local.into()),
                attributes: Vec::new(),
                has_form_ancestor: false,
                disable_context_shadow: false,
            };
            let (fragment, root) = prepare_unfiltered_fragment(markup, context).unwrap();
            let shadow = fragment
                .children(root)
                .unwrap()
                .iter()
                .copied()
                .find(|node| {
                    matches!(
                        fragment.arena().get(*node).unwrap().kind,
                        NodeKind::ShadowRoot { .. }
                    )
                })
                .expect("real fragment context accepts top-level declarative template");
            assert_eq!(fragment.arena().text_content(shadow).unwrap(), "shadow");
        }
        let document=crate::html::parse_inert_dom_string_with_declarative_shadow("<!doctype html><html><template shadowrootmode=open><b>head template</b></template><body><div id=allowed><template shadowrootmode=open><b>nested shadow</b></template></div></body></html>").into_runtime().unwrap();
        let html = document
            .children(document.document())
            .unwrap()
            .iter()
            .copied()
            .find(|node| document.is_element(*node, HTML_NAMESPACE, "html"))
            .unwrap();
        assert!(
            !document.children(html).unwrap().iter().any(|node| matches!(
                document.arena().get(*node).unwrap().kind,
                NodeKind::ShadowRoot { .. }
            )),
            "full-document topmost html must not acquire a shadow root"
        );
        let allowed = document.find_by_id("allowed").unwrap();
        assert!(
            document
                .children(allowed)
                .unwrap()
                .iter()
                .any(|node| matches!(
                    document.arena().get(*node).unwrap().kind,
                    NodeKind::ShadowRoot { .. }
                )),
            "full-document nested eligible host still attaches"
        );
    }
    #[test]
    fn top_level_declarative_root_survives_fragment_context_and_is_filtered() {
        let mut dom = crate::html::parse("<div id=host>old</div>")
            .into_runtime()
            .unwrap();
        let host = dom.find_by_id("host").unwrap();
        let config = Config::default().canonicalize(true).unwrap();
        let(fragment,root)=prepare_fragment("<template shadowrootmode=closed shadowrootserializable><a href='javascript:0'>shadow</a><script>bad</script></template><b>light</b>",FragmentContext::capture(&dom,host).unwrap(),&config,true).unwrap();
        let prepared_children = fragment
            .arena()
            .get(root)
            .unwrap()
            .children
            .iter()
            .map(|child| (*child, fragment.arena().get(*child).unwrap().kind.clone()))
            .collect::<Vec<_>>();
        assert!(
            prepared_children
                .iter()
                .any(|(_, kind)| matches!(kind, NodeKind::ShadowRoot { .. })),
            "prepared filtered fragment lost declarative root: {prepared_children:?}; markup={}",
            fragment.serialize(root).unwrap()
        );
        let imported = dom
            .replace_with_sanitized_fragment(host, &fragment, root)
            .unwrap();
        let published_children = dom
            .arena()
            .get(host)
            .unwrap()
            .children
            .iter()
            .map(|child| (*child, dom.arena().get(*child).unwrap().kind.clone()))
            .collect::<Vec<_>>();
        let shadow=published_children.iter().find_map(|(child,kind)|matches!(kind,NodeKind::ShadowRoot{..}).then_some(*child)).unwrap_or_else(||panic!("imported declarative root not attached: imported={imported:?}, children={published_children:?}"));
        assert!(dom.arena().get(shadow).unwrap().shadow_serializable);
        assert!(!dom.serialize(shadow).unwrap().contains("javascript"));
        assert!(!dom.serialize(shadow).unwrap().contains("script"));
        let (fragment, root) = prepare_fragment(
            "<template shadowrootmode=open><i>fallback</i></template>",
            FragmentContext::capture(&dom, host).unwrap(),
            &config,
            false,
        )
        .unwrap();
        dom.replace_with_sanitized_fragment(host, &fragment, root)
            .unwrap();
        assert_eq!(dom.parent(shadow).unwrap(), Some(host));
        assert!(
            dom.serialize(host)
                .unwrap()
                .contains("<template shadowrootmode=\"open\"><i>fallback</i></template>")
        );
    }
    #[test]
    fn sanitized_fragment_uses_table_svg_and_form_context() {
        let mut dom=crate::html::parse("<table><tbody id=rows></tbody></table><svg id=vector></svg><form><div id=inside></div></form>").into_runtime().unwrap();
        let config = Config::default().canonicalize(true).unwrap();
        let rows = dom.find_by_id("rows").unwrap();
        let context = FragmentContext::capture(&dom, rows).unwrap();
        let (fragment, root) =
            prepare_fragment("<tr><td onclick=bad>cell</td></tr>", context, &config, true).unwrap();
        dom.replace_with_sanitized_fragment(rows, &fragment, root)
            .unwrap();
        assert_eq!(
            dom.serialize(rows).unwrap(),
            "<tbody id=\"rows\"><tr><td>cell</td></tr></tbody>"
        );
        let vector = dom.find_by_id("vector").unwrap();
        let context = FragmentContext::capture(&dom, vector).unwrap();
        let (fragment, root) = prepare_fragment(
            "<circle/><a href='javascript:0'>x</a>",
            context,
            &config,
            true,
        )
        .unwrap();
        let children = dom
            .replace_with_sanitized_fragment(vector, &fragment, root)
            .unwrap();
        assert!(dom.is_element(children[0], SVG_NAMESPACE, "circle"));
        assert!(!dom.serialize(vector).unwrap().contains("href="));
        let inside = dom.find_by_id("inside").unwrap();
        let context = FragmentContext::capture(&dom, inside).unwrap();
        assert!(context.has_form_ancestor);
        let (fragment, root) =
            prepare_fragment("<form><input></form>", context, &config, false).unwrap();
        dom.replace_with_sanitized_fragment(inside, &fragment, root)
            .unwrap();
        assert_eq!(
            dom.serialize(inside).unwrap(),
            "<div id=\"inside\"><input></div>"
        );
    }
    #[test]
    fn filtered_fragment_preserves_existing_shadow_and_does_not_remove_old_content_on_invalid_policy()
     {
        let mut dom = crate::html::parse("<div id=host><b>old</b></div>")
            .into_runtime()
            .unwrap();
        let host = dom.find_by_id("host").unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: crate::ShadowMode::Closed,
            clonable: false,
        });
        dom.append_child(host, shadow).unwrap();
        let config = Config {
            elements: Some(vec![]),
            remove_elements: Some(vec![]),
            ..Default::default()
        };
        assert!(
            prepare_fragment(
                "new",
                FragmentContext::capture(&dom, host).unwrap(),
                &config,
                true
            )
            .is_err()
        );
        assert!(dom.serialize(host).unwrap().contains("old"));
        let config = Config::default().canonicalize(true).unwrap();
        let (fragment, root) = prepare_fragment(
            "<i>new</i>",
            FragmentContext::capture(&dom, host).unwrap(),
            &config,
            false,
        )
        .unwrap();
        dom.replace_with_sanitized_fragment(host, &fragment, root)
            .unwrap();
        assert_eq!(dom.parent(shadow).unwrap(), Some(host));
        assert_eq!(
            dom.serialize(host).unwrap(),
            "<div id=\"host\"><i>new</i></div>"
        );
        for namespace in [HTML_NAMESPACE, SVG_NAMESPACE] {
            let context = FragmentContext {
                name: markup5ever::QualName::new(None, namespace.into(), "script".into()),
                attributes: vec![],
                has_form_ancestor: false,
                disable_context_shadow: false,
            };
            assert!(context.safe_sink_is_noop());
        }
    }
}
