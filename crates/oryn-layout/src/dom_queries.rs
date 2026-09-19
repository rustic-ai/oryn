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

// RFC 4647 extended filtering used by Selectors 4, matching native paint.
fn language_range_matches(language: &str, range: &str) -> bool {
    if range.is_empty() {
        return language.is_empty();
    }
    if language.is_empty() {
        return false;
    }
    let language = language.split('-').collect::<Vec<_>>();
    let range = range.split('-').collect::<Vec<_>>();
    if range[0] != "*" && !range[0].eq_ignore_ascii_case(language[0]) {
        return false;
    }
    let mut position = 1;
    for subtag in &range[1..] {
        if *subtag == "*" {
            continue;
        }
        while position < language.len() && !subtag.eq_ignore_ascii_case(language[position]) {
            if language[position].len() == 1 {
                return false;
            }
            position += 1;
        }
        if position == language.len() {
            return false;
        }
        position += 1;
    }
    true
}

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
        if self.dom.is_form_associated_custom_element(self.id) {
            return self.dom.control_disabled(self.id).unwrap_or(false);
        }
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
                (self.html_tag(&[
                    "button", "input", "select", "textarea", "fieldset", "optgroup", "option",
                ]) || self.dom.is_form_associated_custom_element(self.id))
                    && !self.disabled()
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
                self.dom.element_focus_within(self.id).unwrap_or(false)
            }
            NonTSPseudoClass::ReadWrite => self.dom.element_read_write(self.id).unwrap_or(false),
            NonTSPseudoClass::ReadOnly => !self.dom.element_read_write(self.id).unwrap_or(false),
            NonTSPseudoClass::UserValid => {
                self.dom.element_user_validity(self.id).ok().flatten() == Some(true)
            }
            NonTSPseudoClass::UserInvalid => {
                self.dom.element_user_validity(self.id).ok().flatten() == Some(false)
            }
            NonTSPseudoClass::Default => self.dom.element_default_state(self.id).unwrap_or(false),
            NonTSPseudoClass::InRange => {
                self.dom.element_range_state(self.id).ok().flatten() == Some(true)
            }
            NonTSPseudoClass::OutOfRange => {
                self.dom.element_range_state(self.id).ok().flatten() == Some(false)
            }
            NonTSPseudoClass::Required => {
                self.dom.element_required_state(self.id).ok().flatten() == Some(true)
            }
            NonTSPseudoClass::Valid | NonTSPseudoClass::Invalid => self
                .dom
                .validation_selector_state(self.id)
                .is_ok_and(|state| {
                    state.is_some_and(|valid| valid == matches!(pseudo, NonTSPseudoClass::Valid))
                }),
            NonTSPseudoClass::Optional => {
                self.dom.element_required_state(self.id).ok().flatten() == Some(false)
            }
            NonTSPseudoClass::Open => {
                (self.html_tag(&["details", "dialog"]) && self.attr("open").is_some())
                    || (self.html_tag(&["select", "input"])
                        && self.dom.top_layer().any(|entry| {
                            entry.node == self.id
                                && matches!(
                                    entry.kind,
                                    oryn_dom::TopLayerKind::SelectPicker
                                        | oryn_dom::TopLayerKind::TemporalPicker
                                )
                        }))
            }
            NonTSPseudoClass::Modal => self.dom.is_modal_dialog(self.id),
            NonTSPseudoClass::PopoverOpen => self.dom.is_popover_open(self.id),
            NonTSPseudoClass::Granted => self
                .dom
                .element_permission_granted(self.id)
                .unwrap_or(false),
            NonTSPseudoClass::Fullscreen => self.dom.is_fullscreen(self.id),
            NonTSPseudoClass::Paused => {
                self.dom.media_selector_state(self.id).unwrap_or(0) & 2 != 0
            }
            NonTSPseudoClass::Playing => {
                self.dom.media_selector_state(self.id).unwrap_or(0) & 4 != 0
            }
            NonTSPseudoClass::Seeking => {
                self.dom.media_selector_state(self.id).unwrap_or(0) & 8 != 0
            }
            NonTSPseudoClass::Buffering => {
                self.dom.media_selector_state(self.id).unwrap_or(0) & 16 != 0
            }
            NonTSPseudoClass::Stalled => {
                self.dom.media_selector_state(self.id).unwrap_or(0) & 32 != 0
            }
            NonTSPseudoClass::InterestSource => {
                self.dom.element_interest_source(self.id).unwrap_or(false)
            }
            NonTSPseudoClass::InterestTarget => {
                self.dom.element_interest_target(self.id).unwrap_or(false)
            }
            NonTSPseudoClass::Defined => self.dom.element_defined(self.id).unwrap_or(false),
            NonTSPseudoClass::PlaceholderShown => {
                self.dom.element_placeholder_shown(self.id).unwrap_or(false)
            }
            NonTSPseudoClass::Target => {
                self.dom
                    .arena()
                    .node_document(self.id)
                    .ok()
                    .flatten()
                    .and_then(|document| self.dom.document_target(document).ok())
                    .flatten()
                    == Some(self.id)
            }
            NonTSPseudoClass::CustomState(name) => self.has_custom_state(&name.0),
            NonTSPseudoClass::Dir(direction) => direction.eq_ignore_ascii_case(
                crate::directionality::directionality(self.dom.arena(), self.id),
            ),
            NonTSPseudoClass::Lang(ranges) => {
                let actual = self
                    .dom
                    .element_language(self.id)
                    .ok()
                    .flatten()
                    .unwrap_or("");
                ranges
                    .iter()
                    .any(|range| language_range_matches(actual, range))
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

    #[test]
    fn registered_form_custom_elements_use_native_disabled_and_validity_state() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("fieldset");
        let legend = dom.create_element("legend");
        let exempt = dom.create_element("x-field");
        let control = dom.create_element("x-field");
        let ordinary = dom.create_element("x-other");
        dom.append_child(root, legend).unwrap();
        dom.append_child(legend, exempt).unwrap();
        dom.append_child(root, control).unwrap();
        dom.append_child(root, ordinary).unwrap();
        dom.register_form_associated_custom_element(exempt).unwrap();
        dom.register_form_associated_custom_element(control)
            .unwrap();
        dom.set_attribute(root, "disabled", "").unwrap();
        let states = BTreeMap::new();
        let matches = |dom: &DomRuntime, node, selector| {
            !query(dom, node, selector, SelectorOperation::Matches, &states)
                .unwrap()
                .is_empty()
        };
        assert!(matches(&dom, exempt, ":enabled"));
        assert!(!matches(&dom, exempt, ":disabled"));
        assert!(matches(&dom, control, ":disabled"));
        assert!(!matches(&dom, control, ":enabled"));
        assert!(!matches(&dom, ordinary, ":enabled"));
        assert!(!matches(&dom, ordinary, ":disabled"));
        dom.set_custom_element_validity(
            control,
            ["customError".to_owned()].into_iter().collect(),
            "invalid".into(),
        )
        .unwrap();
        assert!(
            !matches(&dom, control, ":invalid"),
            "disabled controls are barred from validation"
        );
        dom.remove_attribute(root, "disabled").unwrap();
        assert!(matches(&dom, control, ":enabled"));
        assert!(matches(&dom, control, ":invalid"));
        dom.set_custom_element_validity(control, Default::default(), String::new())
            .unwrap();
        assert!(matches(&dom, control, ":valid"));
        dom.unregister_form_associated_custom_element(control);
        assert!(!matches(&dom, control, ":enabled"));
        assert!(!matches(&dom, control, ":valid"));
    }
}

#[cfg(test)]
mod permission_query_tests {
    use super::*;
    #[test]
    fn granted_query_uses_native_state_with_stylo_negation_and_lists() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let a = dom.create_element("geolocation");
        let b = dom.create_element("usermedia");
        dom.append_child(root, a).unwrap();
        dom.append_child(root, b).unwrap();
        dom.set_permission_granted(a, true).unwrap();
        let states = BTreeMap::new();
        assert_eq!(
            query(
                &dom,
                root,
                ":is(geolocation,usermedia):granted",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![a]
        );
        assert_eq!(
            query(
                &dom,
                root,
                "usermedia:not(:granted)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![b]
        );
        dom.set_permission_granted(a, false).unwrap();
        assert!(
            query(&dom, root, ":granted", SelectorOperation::All, &states)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod flat_focus_regressions {
    use super::*;
    #[test]
    fn shadow_host_and_slot_queries_track_native_focus_ancestry() {
        let mut dom = oryn_dom::html::parse(
            "<main><x-host id=host><input id=light></x-host><input id=outside></main>",
        )
        .into_runtime()
        .unwrap();
        let host = dom.find_by_id("host").unwrap();
        let light = dom.find_by_id("light").unwrap();
        let outside = dom.find_by_id("outside").unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: oryn_dom::ShadowMode::Open,
            clonable: false,
        });
        dom.append_child(host, shadow).unwrap();
        let slot = dom.create_element("slot");
        dom.append_child(shadow, slot).unwrap();
        let inner = dom.create_element("input");
        dom.append_child(shadow, inner).unwrap();
        let states = BTreeMap::new();
        dom.focus(Some(inner)).unwrap();
        assert_eq!(
            query(
                &dom,
                host,
                ":focus-within",
                SelectorOperation::Matches,
                &states
            )
            .unwrap(),
            vec![host]
        );
        assert_eq!(
            query(
                &dom,
                slot,
                ":focus-within",
                SelectorOperation::Matches,
                &states
            )
            .unwrap(),
            vec![]
        );
        dom.focus(Some(light)).unwrap();
        assert_eq!(
            query(
                &dom,
                slot,
                ":focus-within",
                SelectorOperation::Matches,
                &states
            )
            .unwrap(),
            vec![slot]
        );
        assert_eq!(
            query(
                &dom,
                host,
                ":focus-within",
                SelectorOperation::Matches,
                &states
            )
            .unwrap(),
            vec![host]
        );
        dom.focus(Some(outside)).unwrap();
        assert!(
            query(
                &dom,
                host,
                ":focus-within",
                SelectorOperation::Matches,
                &states
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[cfg(test)]
mod language_selector_regressions {
    use super::*;
    #[test]
    fn document_language_meta_and_http_drive_relational_queries() {
        let mut dom = oryn_dom::html::parse("<section id=owner><span id=child></span></section>")
            .into_runtime()
            .unwrap();
        let root = dom.document();
        let owner = dom.find_by_id("owner").unwrap();
        let child = dom.find_by_id("child").unwrap();
        let states = BTreeMap::new();
        dom.set_document_content_language(root, Some("fr".into()))
            .unwrap();
        assert_eq!(
            query(
                &dom,
                root,
                "section:has(:lang(fr))",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![owner]
        );
        let meta = dom.create_element("meta");
        dom.set_attribute(meta, "http-equiv", "content-language")
            .unwrap();
        dom.set_attribute(meta, "content", "tr").unwrap();
        dom.append_child(owner, meta).unwrap();
        assert_eq!(
            query(
                &dom,
                root,
                "#child:lang(tr)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
        dom.set_attribute(child, "lang", "").unwrap();
        assert!(
            query(
                &dom,
                root,
                "#child:lang(tr)",
                SelectorOperation::All,
                &states
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            query(
                &dom,
                root,
                r#"#child:lang("")"#,
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
    }
    #[test]
    fn native_lang_queries_use_namespace_precedence_lists_and_extended_ranges() {
        let mut dom=oryn_dom::html::parse("<html lang=en-US><body><section id=owner lang=fr-Latn-FR><span id=child></span></section></body></html>").into_runtime().unwrap();
        let root = dom.document();
        let owner = dom.find_by_id("owner").unwrap();
        let child = dom.find_by_id("child").unwrap();
        let states = BTreeMap::new();
        assert_eq!(
            query(
                &dom,
                root,
                "#child:lang(de,nl,fr)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
        assert_eq!(
            query(
                &dom,
                root,
                r#"#child:lang("fr-*-FR")"#,
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
        dom.set_attribute(owner, "xml:lang", "de").unwrap();
        assert_eq!(
            query(
                &dom,
                root,
                "#child:lang(fr)",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
        dom.set_attribute_ns(
            owner,
            Some("http://www.w3.org/XML/1998/namespace".into()),
            "xml:lang",
            "zh-Hant-TW",
        )
        .unwrap();
        assert_eq!(
            query(
                &dom,
                root,
                "section:has(#child:lang(zh-TW))",
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![owner]
        );
        assert!(
            query(
                &dom,
                root,
                "#child:lang(fr)",
                SelectorOperation::All,
                &states
            )
            .unwrap()
            .is_empty()
        );
        dom.set_attribute_ns(
            owner,
            Some("http://www.w3.org/XML/1998/namespace".into()),
            "xml:lang",
            "",
        )
        .unwrap();
        assert_eq!(
            query(
                &dom,
                root,
                r#"#child:lang("")"#,
                SelectorOperation::All,
                &states
            )
            .unwrap(),
            vec![child]
        );
        assert!(!language_range_matches("de-x-DE", "de-DE"));
        assert!(language_range_matches("de-Latn-DE", "de-DE"));
    }
}
