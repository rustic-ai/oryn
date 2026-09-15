//! Stylo selector matching over the canonical DOM, without a layout flush or mirror.
use oryn_dom::{DomRuntime, Node, NodeId, NodeKind, SelectorOperation};
use selectors::{
    Element, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    matching::{
        MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, SelectorCaches,
        matches_selector_list,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ptr::NonNull,
};
use style::{
    context::QuirksMode,
    selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl, SelectorParser},
    stylesheets::UrlExtraData,
    values::{AtomIdent, AtomString},
};

const HTML: &str = "http://www.w3.org/1999/xhtml";
#[derive(Clone, Copy)]
struct DomElement<'a> {
    id: NodeId,
    dom: &'a DomRuntime,
    states: &'a BTreeMap<NodeId, BTreeSet<String>>,
}
impl std::fmt::Debug for DomElement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.id.fmt(f)
    }
}
impl PartialEq for DomElement<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.dom, other.dom)
    }
}
impl<'a> DomElement<'a> {
    fn node(&self) -> &'a Node {
        self.dom
            .arena()
            .get(self.id)
            .expect("immutable live selector node")
    }
    fn element(&self, id: NodeId) -> Option<Self> {
        matches!(
            self.dom.arena().get(id).ok()?.kind,
            NodeKind::Element { .. }
        )
        .then_some(Self { id, ..*self })
    }
    fn tag(&self) -> &str {
        match &self.node().kind {
            NodeKind::Element { name } => name.local.as_ref(),
            _ => unreachable!(),
        }
    }
    fn ns(&self) -> &str {
        match &self.node().kind {
            NodeKind::Element { name } => name.ns.as_ref(),
            _ => unreachable!(),
        }
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.node()
            .attributes
            .namespaced(None, name)
            .map(|entry| entry.value.as_str())
    }
    fn html_tag(&self, tags: &[&str]) -> bool {
        self.ns() == HTML && tags.contains(&self.tag())
    }
    fn disabled(&self) -> bool {
        if !self.html_tag(&[
            "button", "input", "select", "textarea", "fieldset", "optgroup", "option",
        ]) {
            return false;
        }
        if self.attr("disabled").is_some() {
            return true;
        }
        if self.tag() == "option"
            && self
                .parent_element()
                .is_some_and(|p| p.html_tag(&["optgroup"]) && p.attr("disabled").is_some())
        {
            return true;
        }
        let mut child = *self;
        while let Some(parent) = child.parent_element() {
            if parent.html_tag(&["fieldset"]) && parent.attr("disabled").is_some() {
                let legend = parent
                    .node()
                    .children
                    .iter()
                    .filter_map(|id| self.element(*id))
                    .find(|e| e.html_tag(&["legend"]));
                if legend != Some(child) {
                    return true;
                }
            }
            child = parent;
        }
        false
    }
    fn editable(&self) -> bool {
        if self.html_tag(&["textarea", "input"]) {
            let text_input = self.tag() == "textarea"
                || matches!(
                    self.attr("type")
                        .unwrap_or("text")
                        .to_ascii_lowercase()
                        .as_str(),
                    "text"
                        | "search"
                        | "url"
                        | "tel"
                        | "email"
                        | "password"
                        | "date"
                        | "month"
                        | "week"
                        | "time"
                        | "datetime-local"
                        | "number"
                );
            return text_input && !self.disabled() && self.attr("readonly").is_none();
        }
        let mut current = Some(*self);
        while let Some(e) = current {
            if let Some(value) = e.attr("contenteditable") {
                if value.eq_ignore_ascii_case("false") {
                    return false;
                }
                if value.is_empty()
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("plaintext-only")
                {
                    return true;
                }
            }
            current = e.parent_element();
        }
        false
    }
}
impl Element for DomElement<'_> {
    type Impl = SelectorImpl;
    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::from_non_null_ptr(NonNull::from(self.node()).cast())
    }
    fn parent_element(&self) -> Option<Self> {
        self.element(self.node().parent?)
    }
    fn parent_node_is_shadow_root(&self) -> bool {
        self.node()
            .parent
            .and_then(|id| self.dom.arena().get(id).ok())
            .is_some_and(|n| matches!(n.kind, NodeKind::ShadowRoot { .. }))
    }
    fn containing_shadow_host(&self) -> Option<Self> {
        let mut id = self.node().parent;
        while let Some(i) = id {
            let n = self.dom.arena().get(i).ok()?;
            if matches!(n.kind, NodeKind::ShadowRoot { .. }) {
                return self.element(n.parent?);
            }
            id = n.parent;
        }
        None
    }
    fn is_pseudo_element(&self) -> bool {
        false
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        let children = &self.dom.arena().get(self.node().parent?).ok()?.children;
        let index = children.iter().position(|id| *id == self.id)?;
        children[..index]
            .iter()
            .rev()
            .find_map(|id| self.element(*id))
    }
    fn next_sibling_element(&self) -> Option<Self> {
        let children = &self.dom.arena().get(self.node().parent?).ok()?.children;
        let index = children.iter().position(|id| *id == self.id)?;
        children[index + 1..]
            .iter()
            .find_map(|id| self.element(*id))
    }
    fn first_element_child(&self) -> Option<Self> {
        self.node().children.iter().find_map(|id| self.element(*id))
    }
    fn is_html_element_in_html_document(&self) -> bool {
        self.ns() == HTML
    }
    fn has_local_name(
        &self,
        name: &<SelectorImpl as selectors::SelectorImpl>::BorrowedLocalName,
    ) -> bool {
        name.as_ref() == self.tag()
    }
    fn has_namespace(
        &self,
        ns: &<SelectorImpl as selectors::SelectorImpl>::BorrowedNamespaceUrl,
    ) -> bool {
        ns.as_ref() == self.ns()
    }
    fn is_same_type(&self, other: &Self) -> bool {
        self.ns() == other.ns() && self.tag() == other.tag()
    }
    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<SelectorImpl as selectors::SelectorImpl>::NamespaceUrl>,
        name: &<SelectorImpl as selectors::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        let local: &str = name.0.as_ref();
        self.node().attributes.records().any(|entry| {
            let matches_namespace = match ns {
                NamespaceConstraint::Any => true,
                NamespaceConstraint::Specific(wanted) => {
                    wanted.0.as_ref() == entry.namespace.as_deref().unwrap_or("")
                }
            };
            matches_namespace && entry.local_name == local && operation.eval_str(&entry.value)
        })
    }
    fn match_non_ts_pseudo_class(
        &self,
        pseudo: &NonTSPseudoClass,
        _: &mut MatchingContext<SelectorImpl>,
    ) -> bool {
        match pseudo {
            NonTSPseudoClass::AnyLink | NonTSPseudoClass::Link => self.is_link(),
            NonTSPseudoClass::Disabled => self.disabled(),
            NonTSPseudoClass::Enabled => {
                self.html_tag(&[
                    "button", "input", "select", "textarea", "fieldset", "optgroup", "option",
                ]) && !self.disabled()
            }
            NonTSPseudoClass::Checked => {
                (self.html_tag(&["input"])
                    && matches!(
                        self.attr("type")
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .as_str(),
                        "checkbox" | "radio"
                    )
                    && self.node().checked())
                    || (self.html_tag(&["option"]) && self.node().selected())
            }
            NonTSPseudoClass::Indeterminate => self.dom.is_indeterminate(self.id).unwrap_or(false),
            NonTSPseudoClass::Focus => self.dom.focused() == Some(self.id),
            NonTSPseudoClass::FocusVisible => {
                self.dom.focused() == Some(self.id) && self.dom.focus_visible()
            }
            NonTSPseudoClass::FocusWithin => {
                let mut id = self.dom.focused();
                while let Some(i) = id {
                    if i == self.id {
                        return true;
                    }
                    id = self.dom.arena().get(i).ok().and_then(|n| n.parent);
                }
                false
            }
            NonTSPseudoClass::ReadWrite => self.editable(),
            NonTSPseudoClass::ReadOnly => !self.editable(),
            NonTSPseudoClass::Required => {
                self.html_tag(&["input", "select", "textarea"]) && self.attr("required").is_some()
            }
            NonTSPseudoClass::Valid | NonTSPseudoClass::Invalid => self
                .dom
                .validation_selector_state(self.id)
                .is_ok_and(|state| {
                    state.is_some_and(|valid| valid == matches!(pseudo, NonTSPseudoClass::Valid))
                }),
            NonTSPseudoClass::Optional => {
                self.html_tag(&["input", "select", "textarea"]) && self.attr("required").is_none()
            }
            NonTSPseudoClass::Open => {
                self.html_tag(&["details", "dialog"]) && self.attr("open").is_some()
            }
            NonTSPseudoClass::Modal => {
                self.html_tag(&["dialog"]) && self.attr("data-oryn-modal").is_some()
            }
            NonTSPseudoClass::Defined => {
                !self.tag().contains('-') || self.states.contains_key(&self.id)
            }
            NonTSPseudoClass::CustomState(name) => self.has_custom_state(&name.0),
            NonTSPseudoClass::Dir(direction) => direction.eq_ignore_ascii_case(
                crate::directionality::directionality(self.dom.arena(), self.id),
            ),
            NonTSPseudoClass::Lang(language) => {
                let mut current = Some(*self);
                while let Some(element) = current {
                    if let Some(actual) = element.attr("xml:lang").or_else(|| element.attr("lang"))
                    {
                        return actual.eq_ignore_ascii_case(language)
                            || actual
                                .get(..language.len())
                                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(language))
                                && actual.as_bytes().get(language.len()) == Some(&b'-');
                    }
                    current = element
                        .parent_element()
                        .or_else(|| element.containing_shadow_host());
                }
                false
            }
            _ => false,
        }
    }
    fn match_pseudo_element(
        &self,
        _: &PseudoElement,
        _: &mut MatchingContext<SelectorImpl>,
    ) -> bool {
        false
    }
    fn apply_selector_flags(&self, _: selectors::matching::ElementSelectorFlags) {}
    fn is_link(&self) -> bool {
        self.html_tag(&["a", "area", "link"]) && self.attr("href").is_some()
    }
    fn is_html_slot_element(&self) -> bool {
        self.html_tag(&["slot"])
    }
    fn has_id(&self, id: &AtomIdent, case: CaseSensitivity) -> bool {
        self.attr("id")
            .is_some_and(|v| case.eq(v.as_bytes(), id.0.as_ref().as_bytes()))
    }
    fn has_class(&self, name: &AtomIdent, case: CaseSensitivity) -> bool {
        self.attr("class").is_some_and(|v| {
            v.split_ascii_whitespace()
                .any(|c| case.eq(c.as_bytes(), name.0.as_ref().as_bytes()))
        })
    }
    fn has_custom_state(&self, name: &AtomIdent) -> bool {
        self.states
            .get(&self.id)
            .is_some_and(|s| s.contains(name.0.as_ref()))
    }
    fn imported_part(&self, _: &AtomIdent) -> Option<AtomIdent> {
        None
    }
    fn is_part(&self, name: &AtomIdent) -> bool {
        self.attr("part")
            .is_some_and(|v| v.split_ascii_whitespace().any(|s| s == name.0.as_ref()))
    }
    fn is_empty(&self) -> bool {
        self.node().children.iter().all(|id| {
            self.dom.arena().get(*id).is_ok_and(|n| match &n.kind {
                NodeKind::Element { .. } => false,
                NodeKind::Text { data } | NodeKind::CDataSection { data } => data.is_empty(),
                _ => true,
            })
        })
    }
    fn is_root(&self) -> bool {
        self.node()
            .parent
            .and_then(|id| self.dom.arena().get(id).ok())
            .is_some_and(|n| matches!(n.kind, NodeKind::Document))
    }
    fn add_element_unique_hashes(&self, _: &mut selectors::bloom::BloomFilter) -> bool {
        false
    }
}

/// Query tree order directly; shadow trees are separate query scopes.
pub fn query(
    dom: &DomRuntime,
    root: NodeId,
    selector: &str,
    operation: SelectorOperation,
    states: &BTreeMap<NodeId, BTreeSet<String>>,
) -> Result<Vec<NodeId>, String> {
    let url = UrlExtraData::from(url::Url::parse("about:blank").unwrap());
    let list = SelectorParser::parse_author_origin_no_namespace(selector, &url)
        .map_err(|e| format!("invalid selector: {e:?}"))?;
    let root_node = dom.arena().get(root).map_err(|e| e.to_string())?;
    let base = DomElement {
        id: root,
        dom,
        states,
    };
    if operation == SelectorOperation::Matches {
        return Ok(base
            .element(root)
            .filter(|e| style::dom_apis::element_matches(e, &list, QuirksMode::NoQuirks))
            .map(|e| vec![e.id])
            .unwrap_or_default());
    }
    if operation == SelectorOperation::Closest {
        return Ok(base
            .element(root)
            .and_then(|e| style::dom_apis::element_closest(e, &list, QuirksMode::NoQuirks))
            .map(|e| vec![e.id])
            .unwrap_or_default());
    }
    let scope = base.element(root).or_else(|| {
        if matches!(root_node.kind, NodeKind::Document) {
            root_node.children.iter().find_map(|id| base.element(*id))
        } else {
            None
        }
    });
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    context.scope_element = scope.map(|e| e.opaque());
    let mut pending: Vec<_> = root_node.children.iter().rev().copied().collect();
    let mut found = Vec::new();
    while let Some(id) = pending.pop() {
        let node = dom.arena().get(id).map_err(|e| e.to_string())?;
        if matches!(node.kind, NodeKind::ShadowRoot { .. }) {
            continue;
        }
        if let Some(element) = base.element(id)
            && matches_selector_list(&list, &element, &mut context)
        {
            found.push(id);
            if operation == SelectorOperation::First {
                break;
            }
        }
        pending.extend(node.children.iter().rev().copied());
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relational_and_filtered_nth_selectors_use_stylo_matching() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("main");
        let document = dom.document();
        dom.append_child(document, root).unwrap();
        let mut children = Vec::new();
        for class in ["hot", "cold", "hot"] {
            let node = dom.create_element("div");
            dom.set_attribute(node, "class", class).unwrap();
            dom.append_child(root, node).unwrap();
            children.push(node);
        }
        let states = BTreeMap::new();
        assert_eq!(
            query(
                &dom,
                document,
                "main:has(> .hot)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![root]
        );
        assert_eq!(
            query(
                &dom,
                root,
                ".cold:has(+ .hot)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![children[1]]
        );
        assert_eq!(
            query(
                &dom,
                root,
                ":nth-child(2 of .hot)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![children[2]]
        );
        assert!(
            query(
                &dom,
                document,
                "main:has(:has(div))",
                SelectorOperation::All,
                &states
            )
            .is_err()
        );
        assert!(
            query(
                &dom,
                document,
                "main:has(::before)",
                SelectorOperation::All,
                &states
            )
            .is_err()
        );
        dom.set_attribute(children[2], "class", "cold").unwrap();
        assert!(
            query(
                &dom,
                root,
                ".cold:has(+ .hot)",
                SelectorOperation::All,
                &states
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            query(
                &dom,
                root,
                ":nth-child(2 of .hot)",
                SelectorOperation::All,
                &states
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn focus_queries_distinguish_indicator_state_without_changing_focus_within() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let root = dom.create_element("main");
        let button = dom.create_element("button");
        dom.append_child(document, root).unwrap();
        dom.append_child(root, button).unwrap();
        dom.focus(Some(button)).unwrap();
        let states = BTreeMap::new();
        for visible in [true, false, true] {
            dom.set_focus_visible(visible);
            assert_eq!(
                query(&dom, document, ":focus", SelectorOperation::All, &states).unwrap(),
                vec![button]
            );
            assert_eq!(
                query(
                    &dom,
                    document,
                    ":focus-within",
                    SelectorOperation::All,
                    &states
                )
                .unwrap(),
                vec![root, button]
            );
            let expected = if visible { vec![button] } else { Vec::new() };
            assert_eq!(
                query(
                    &dom,
                    document,
                    ":focus-visible",
                    SelectorOperation::All,
                    &states
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                query(
                    &dom,
                    button,
                    ":focus-visible",
                    SelectorOperation::Matches,
                    &states
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn queries_use_stylo_grammar_tree_order_and_scopes() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let root = dom.create_element("main");
        let first = dom.create_element("div");
        let second = dom.create_element("div");
        dom.set_attribute(first, "id", "a:b").unwrap();
        dom.set_attribute(second, "class", "chosen").unwrap();
        dom.append_child(document, root).unwrap();
        dom.append_child(root, second).unwrap();
        dom.append_child(root, first).unwrap();
        let states = BTreeMap::new();
        let all =
            |node, selector| query(&dom, node, selector, SelectorOperation::All, &states).unwrap();
        assert_eq!(
            all(document, "main > :is(div.chosen, #a\\:b)"),
            vec![second, first]
        );
        assert_eq!(all(root, ":scope > div:nth-child(2)"), vec![first]);
        assert_eq!(all(root, "main"), vec![]);
        assert_eq!(all(root, ":scope"), vec![]);
        assert_eq!(all(document, ":scope"), vec![root]);
        assert_eq!(
            query(&dom, first, ":scope", SelectorOperation::Closest, &states).unwrap(),
            vec![first]
        );
        assert!(query(&dom, root, "div {}", SelectorOperation::All, &states).is_err());
        assert!(query(&dom, root, "div,", SelectorOperation::All, &states).is_err());
    }

    #[test]
    fn queries_respect_shadow_boundaries_empty_and_namespaces() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: oryn_dom::ShadowMode::Open,
            clonable: false,
        });
        let inner = dom.create_element("span");
        let svg = dom.create_element_ns("http://www.w3.org/2000/svg", "svg");
        dom.append_child(root, shadow).unwrap();
        dom.append_child(shadow, inner).unwrap();
        dom.append_child(root, svg).unwrap();
        let comment = dom.create_comment("ignored by :empty");
        dom.append_child(inner, comment).unwrap();
        let states = BTreeMap::new();
        assert_eq!(
            query(&dom, root, "*", SelectorOperation::All, &states).unwrap(),
            vec![svg]
        );
        assert_eq!(
            query(&dom, shadow, "span:empty", SelectorOperation::All, &states).unwrap(),
            vec![inner]
        );
        assert!(
            query(&dom, inner, "div", SelectorOperation::Closest, &states)
                .unwrap()
                .is_empty()
        );
        assert!(
            query(&dom, root, ":root", SelectorOperation::Matches, &states)
                .unwrap()
                .is_empty()
        );
        assert!(
            query(&dom, root, "|svg", SelectorOperation::All, &states)
                .unwrap()
                .is_empty()
        );
        let text = dom.create_text(" ");
        dom.append_child(inner, text).unwrap();
        assert!(
            query(&dom, shadow, ":empty", SelectorOperation::All, &states)
                .unwrap()
                .is_empty()
        );
    }
}
