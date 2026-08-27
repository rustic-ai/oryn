use std::collections::BTreeSet;

use oryn_common::v2::{
    CONTRACT_VERSION, ExecutionDomain, NodeState, Observation, ObservationSource, PageInfo,
    ProjectionProfile, Provenance, Revision, SemanticAction, SemanticNodeView, SemanticRef,
};

use crate::dom::{DomArena, NodeId, NodeKind};

pub fn unpack_node_ref(node: u64) -> NodeId {
    NodeId {
        slot: node as u32,
        generation: (node >> 32) as u32,
    }
}

pub fn project_element(
    dom: &DomArena,
    id: NodeId,
    document_generation: u64,
    alias: u32,
) -> Option<SemanticNodeView> {
    let node = dom.get(id).ok()?;
    let NodeKind::Element { name } = &node.kind else {
        return None;
    };
    let local_name = name.local.as_ref();
    let parent = dom.ancestors(id).ok()?.into_iter().find_map(|ancestor| {
        matches!(dom.get(ancestor).ok()?.kind, NodeKind::Element { .. }).then_some(SemanticRef {
            document_generation,
            node: (u64::from(ancestor.generation) << 32) | u64::from(ancestor.slot),
        })
    });
    let role = node
        .attributes
        .get("role")
        .cloned()
        .unwrap_or_else(|| implicit_role(local_name, &node.attributes));
    let accessible_name = node
        .attributes
        .get("aria-label")
        .or_else(|| node.attributes.get("title"))
        .cloned()
        .or_else(|| {
            let id = node.attributes.get("id")?;
            dom.iter().find_map(|(label_id, label)| {
                matches!(&label.kind, NodeKind::Element { name } if name.local.as_ref() == "label")
                    .then(|| label.attributes.get("for"))
                    .flatten()
                    .filter(|target| *target == id)
                    .and_then(|_| dom.text_content(label_id).ok())
            })
        })
        .or_else(|| {
            dom.ancestors(id).ok()?.into_iter().find_map(|ancestor| {
                let ancestor_node = dom.get(ancestor).ok()?;
                matches!(
                    &ancestor_node.kind,
                    NodeKind::Element { name } if name.local.as_ref() == "label"
                )
                .then(|| dom.text_content(ancestor).ok())
                .flatten()
            })
        })
        .or_else(|| node.attributes.get("placeholder").cloned())
        .or_else(|| dom.text_content(id).ok())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let raw_text = dom
        .text_content(id)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut actions = BTreeSet::new();
    let mut states = BTreeSet::new();
    if is_hidden(dom, id) {
        states.insert(NodeState::Hidden);
    } else {
        states.insert(NodeState::Visible);
    }
    for (present, state) in [
        (
            node.attributes.contains_key("disabled"),
            NodeState::Disabled,
        ),
        (
            node.attributes.contains_key("required"),
            NodeState::Required,
        ),
        (
            node.attributes.contains_key("readonly"),
            NodeState::Readonly,
        ),
        (
            node.attributes.contains_key("data-oryn-focused"),
            NodeState::Focused,
        ),
        (node.attributes.contains_key("checked"), NodeState::Checked),
        (
            node.attributes.contains_key("selected"),
            NodeState::Selected,
        ),
    ] {
        if present {
            states.insert(state);
        }
    }
    if !states.contains(&NodeState::Disabled) && !states.contains(&NodeState::Hidden) {
        if matches!(local_name, "button" | "a") {
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "textarea" {
            if !states.contains(&NodeState::Readonly) {
                actions.extend([SemanticAction::Type, SemanticAction::Clear]);
            }
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "input" {
            match node.attributes.get("type").map(String::as_str) {
                Some("checkbox" | "radio") => actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                ]),
                Some("button" | "submit" | "reset") => {
                    actions.insert(SemanticAction::Click);
                }
                _ if !states.contains(&NodeState::Readonly) => {
                    actions.extend([SemanticAction::Type, SemanticAction::Clear]);
                }
                _ => {}
            }
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "select" {
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Select,
                SemanticAction::Focus,
            ]);
        }
        if local_name == "form"
            || matches!(
                node.attributes.get("type").map(String::as_str),
                Some("submit")
            )
        {
            actions.insert(SemanticAction::Submit);
        }
        if node.attributes.contains_key("onclick")
            || node.attributes.contains_key("data-oryn-click-listener")
        {
            actions.extend([SemanticAction::Click, SemanticAction::Focus]);
        }
        match role.as_str() {
            "button" | "tab" | "menuitem" | "treeitem" => {
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
            "checkbox" | "radio" => {
                actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                    SemanticAction::Focus,
                ]);
            }
            _ => {}
        }
    }
    let description = node
        .attributes
        .get("placeholder")
        .filter(|value| **value != accessible_name)
        .cloned()
        .or_else(|| {
            (!actions.is_empty() && !raw_text.is_empty() && raw_text != accessible_name)
                .then_some(raw_text)
        });
    Some(SemanticNodeView {
        semantic_ref: SemanticRef {
            document_generation,
            node: (u64::from(id.generation) << 32) | u64::from(id.slot),
        },
        parent,
        document_order: id.slot,
        alias,
        role,
        name: accessible_name,
        selector: node
            .attributes
            .get("id")
            .map(|id| format!("#{id}"))
            .or_else(|| {
                node.attributes
                    .get("data-testid")
                    .map(|value| format!(r#"[data-testid="{value}"]"#))
            }),
        value: node.attributes.get("value").cloned(),
        description,
        states,
        actions,
        provenance: Provenance::BrowserDeterministic,
    })
}

pub fn project_document(dom: &DomArena, document_generation: u64) -> Vec<SemanticNodeView> {
    dom.iter()
        .filter_map(|(id, _)| {
            let alias = id.slot.checked_add(1)?;
            project_element(dom, id, document_generation, alias).filter(is_compact_semantic)
        })
        .collect()
}

pub fn project_document_full(dom: &DomArena, document_generation: u64) -> Vec<SemanticNodeView> {
    dom.iter()
        .filter_map(|(id, node)| {
            let NodeKind::Element { name } = &node.kind else {
                return None;
            };
            if matches!(
                name.local.as_ref(),
                "html" | "head" | "body" | "meta" | "title" | "script" | "style"
            ) {
                return None;
            }
            let alias = id.slot.checked_add(1)?;
            project_element(dom, id, document_generation, alias)
        })
        .collect()
}

fn implicit_role(
    local_name: &str,
    attributes: &std::collections::BTreeMap<String, String>,
) -> String {
    match local_name {
        "a" if attributes.contains_key("href") => "link",
        "input" => match attributes.get("type").map(String::as_str) {
            Some("checkbox") => "checkbox",
            Some("radio") => "radio",
            Some("button" | "submit" | "reset") => "button",
            _ => "textbox",
        },
        "textarea" => "textbox",
        "select" => "combobox",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "output" => "status",
        other => other,
    }
    .into()
}

fn is_compact_semantic(node: &SemanticNodeView) -> bool {
    !node.states.contains(&NodeState::Hidden)
        && (!node.actions.is_empty()
            || matches!(
                node.role.as_str(),
                "heading"
                    | "status"
                    | "button"
                    | "link"
                    | "textbox"
                    | "checkbox"
                    | "radio"
                    | "combobox"
            ))
}

pub(crate) fn is_hidden(dom: &DomArena, id: NodeId) -> bool {
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
    let rules = dom
        .iter()
        .filter_map(|(style_id, node)| match &node.kind {
            NodeKind::Element { name } if name.local.as_ref() == "style" => {
                dom.text_content(style_id).ok()
            }
            _ => None,
        })
        .flat_map(|sheet| {
            strip_css_comments(&sheet)
                .split('}')
                .filter_map(|rule| rule.split_once('{'))
                .flat_map(|(selectors, declarations)| {
                    selectors
                        .split(',')
                        .map(str::trim)
                        .map(|selector| (selector.to_string(), declarations.to_string()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    lineage.iter().any(|candidate| {
        dom.get(*candidate).is_ok_and(|node| {
            if node.attributes.contains_key("hidden")
                || node
                    .attributes
                    .get("aria-hidden")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                return true;
            }
            let mut hidden = None;
            for (selector, declarations) in &rules {
                if matches_selector(dom, *candidate, selector) {
                    update_visibility(&mut hidden, declarations);
                }
            }
            if let Some(style) = node.attributes.get("style") {
                update_visibility(&mut hidden, style);
            }
            hidden.unwrap_or(false)
        })
    })
}

fn strip_css_comments(sheet: &str) -> String {
    let mut output = String::with_capacity(sheet.len());
    let mut remainder = sheet;
    while let Some(start) = remainder.find("/*") {
        output.push_str(&remainder[..start]);
        let Some(end) = remainder[start + 2..].find("*/") else {
            return output;
        };
        remainder = &remainder[start + 2 + end + 2..];
    }
    output.push_str(remainder);
    output
}

fn matches_selector(dom: &DomArena, id: NodeId, selector: &str) -> bool {
    let segments = selector
        .split_whitespace()
        .filter(|segment| *segment != ">")
        .collect::<Vec<_>>();
    let Some(subject) = segments.last() else {
        return false;
    };
    let Ok(node) = dom.get(id) else {
        return false;
    };
    if !matches_simple_selector(node, subject) {
        return false;
    }
    let ancestors = dom.ancestors(id).unwrap_or_default();
    let mut ancestor_index = 0;
    for segment in segments[..segments.len() - 1].iter().rev() {
        let Some(found) = ancestors[ancestor_index..].iter().position(|ancestor| {
            dom.get(*ancestor)
                .is_ok_and(|node| matches_simple_selector(node, segment))
        }) else {
            return false;
        };
        ancestor_index += found + 1;
    }
    true
}

fn update_visibility(hidden: &mut Option<bool>, declarations: &str) {
    for declaration in declarations.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        match property.trim().to_ascii_lowercase().as_str() {
            "display" => *hidden = Some(value.trim().eq_ignore_ascii_case("none")),
            "visibility" => {
                *hidden = Some(matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "hidden" | "collapse"
                ))
            }
            _ => {}
        }
    }
}

fn matches_simple_selector(node: &crate::dom::Node, selector: &str) -> bool {
    let NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let selector = selector.split(':').next().unwrap_or_default();
    let attribute_start = selector.find('[');
    let simple = &selector[..attribute_start.unwrap_or(selector.len())];
    if let Some(attribute) = attribute_start.and_then(|index| {
        selector
            .strip_suffix(']')
            .map(|selector| &selector[index + 1..])
    }) {
        let (key, expected) = attribute
            .split_once('=')
            .map(|(key, value)| (key, Some(value.trim_matches(['"', '\'']))))
            .unwrap_or((attribute, None));
        let Some(actual) = node.attributes.get(key) else {
            return false;
        };
        if expected.is_some_and(|expected| actual != expected) {
            return false;
        }
    }
    let mut tag_end = simple.len();
    for marker in ['#', '.'] {
        if let Some(index) = simple.find(marker) {
            tag_end = tag_end.min(index);
        }
    }
    let tag = &simple[..tag_end];
    if !tag.is_empty() && !name.local.as_ref().eq_ignore_ascii_case(tag) {
        return false;
    }
    if let Some(start) = simple.find('#') {
        let id = simple[start + 1..].split('.').next().unwrap_or_default();
        if node.attributes.get("id").is_none_or(|value| value != id) {
            return false;
        }
    }
    simple.split('.').skip(1).all(|class| {
        node.attributes
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class))
    })
}

pub fn empty_observation(url: String, title: String, document_generation: u64) -> Observation {
    Observation {
        contract_version: CONTRACT_VERSION,
        page: PageInfo {
            url,
            title,
            document_generation,
        },
        revision: Revision(0),
        profile: ProjectionProfile::Compact,
        source: ObservationSource::NativeDom,
        nodes: Vec::new(),
        capabilities: Vec::new(),
        diagnostics: Vec::new(),
        execution_domain: ExecutionDomain::Native,
    }
}
