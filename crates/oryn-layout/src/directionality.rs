//! HTML directionality shared by DOM selector queries and the style snapshot.
use icu_properties::{CodePointMapData, props::BidiClass};
use oryn_dom::{DomArena, Node, NodeId, NodeKind};

fn html_tag(node: &Node) -> Option<&str> {
    match &node.kind {
        NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" => {
            Some(name.local.as_ref())
        }
        _ => None,
    }
}
fn dir_state(node: &Node) -> Option<&str> {
    html_tag(node)?;
    node.attributes
        .get("dir")
        .map(String::as_str)
        .filter(|value| {
            ["ltr", "rtl", "auto"]
                .iter()
                .any(|state| value.eq_ignore_ascii_case(state))
        })
}
fn first_strong(text: &str) -> Option<&'static str> {
    let classes = CodePointMapData::<BidiClass>::new();
    text.chars().find_map(|ch| match classes.get(ch) {
        BidiClass::LeftToRight => Some("ltr"),
        BidiClass::RightToLeft | BidiClass::ArabicLetter => Some("rtl"),
        _ => None,
    })
}
fn shadow_root(arena: &DomArena, mut id: NodeId) -> Option<NodeId> {
    loop {
        let node = arena.get(id).ok()?;
        if matches!(node.kind, NodeKind::ShadowRoot { .. }) {
            return Some(id);
        }
        id = node.parent?;
    }
}
fn excluded(node: &Node) -> bool {
    matches!(
        html_tag(node),
        Some("bdi" | "script" | "style" | "textarea")
    ) || dir_state(node).is_some()
}
fn contained_text(arena: &DomArena, root: NodeId, exclude_root: bool) -> Option<&'static str> {
    let node = arena.get(root).ok()?;
    if exclude_root && excluded(node) {
        return None;
    }
    let mut pending = node.children.iter().rev().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let node = arena.get(id).ok()?;
        if excluded(node) || matches!(node.kind, NodeKind::ShadowRoot { .. }) {
            continue;
        }
        if html_tag(node) == Some("slot")
            && let Some(root) = shadow_root(arena, id)
            && let Some(host) = arena.get(root).ok()?.parent
        {
            return Some(directionality(arena, host));
        }
        if let NodeKind::Text { data } | NodeKind::CDataSection { data } = &node.kind
            && let Some(direction) = first_strong(data)
        {
            return Some(direction);
        }
        pending.extend(node.children.iter().rev().copied());
    }
    None
}
fn auto_directionality(arena: &DomArena, id: NodeId) -> Option<&'static str> {
    let node = arena.get(id).ok()?;
    let tag = html_tag(node);
    let input_type = node
        .attributes
        .get("type")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "text".into());
    let value_input = tag == Some("input")
        && !matches!(
            input_type.as_str(),
            "date"
                | "datetime-local"
                | "month"
                | "week"
                | "time"
                | "number"
                | "range"
                | "color"
                | "checkbox"
                | "radio"
                | "file"
                | "image"
        );
    if value_input || tag == Some("textarea") {
        let fallback;
        let value = match node.value.as_deref() {
            Some(value) => value,
            None if tag == Some("textarea") => {
                fallback = arena.text_content(id).ok()?;
                &fallback
            }
            None => node
                .attributes
                .get("value")
                .map(String::as_str)
                .unwrap_or(""),
        };
        return first_strong(value).or_else(|| (!value.is_empty()).then_some("ltr"));
    }
    if tag == Some("slot")
        && let Some(root) = shadow_root(arena, id)
    {
        let host = arena.get(root).ok()?.parent?;
        let name = node
            .attributes
            .get("name")
            .map(String::as_str)
            .unwrap_or("");
        // Only the first slot with a given name receives automatically assigned nodes.
        let first = arena
            .descendants_inclusive(root)
            .ok()?
            .into_iter()
            .find(|candidate| {
                arena.get(*candidate).ok().is_some_and(|node| {
                    html_tag(node) == Some("slot")
                        && node
                            .attributes
                            .get("name")
                            .map(String::as_str)
                            .unwrap_or("")
                            == name
                })
            });
        if first == Some(id) {
            let assigned = arena
                .get(host)
                .ok()?
                .children
                .iter()
                .copied()
                .filter(|child| {
                    arena.get(*child).ok().is_some_and(|node| {
                        matches!(node.kind, NodeKind::Element { .. } | NodeKind::Text { .. })
                            && node
                                .attributes
                                .get("slot")
                                .map(String::as_str)
                                .unwrap_or("")
                                == name
                    })
                })
                .collect::<Vec<_>>();
            if !assigned.is_empty() {
                for child in assigned {
                    let result = match &arena.get(child).ok()?.kind {
                        NodeKind::Text { data } => first_strong(data),
                        _ => contained_text(arena, child, true),
                    };
                    if result.is_some() {
                        return result;
                    }
                }
                return None;
            }
        }
    }
    contained_text(arena, id, false)
}

/// Resolve semantic directionality independently of the CSS `direction` property.
pub fn directionality(arena: &DomArena, mut id: NodeId) -> &'static str {
    loop {
        let Ok(node) = arena.get(id) else {
            return "ltr";
        };
        match dir_state(node) {
            Some(value) if value.eq_ignore_ascii_case("ltr") => return "ltr",
            Some(value) if value.eq_ignore_ascii_case("rtl") => return "rtl",
            Some(_) => return auto_directionality(arena, id).unwrap_or("ltr"),
            None if html_tag(node) == Some("bdi") => {
                return auto_directionality(arena, id).unwrap_or("ltr");
            }
            None if html_tag(node) == Some("input")
                && node
                    .attributes
                    .get("type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("tel")) =>
            {
                return "ltr";
            }
            None => {}
        }
        let Some(parent) = node.parent else {
            return "ltr";
        };
        let Ok(parent_node) = arena.get(parent) else {
            return "ltr";
        };
        id = match parent_node.kind {
            NodeKind::Element { .. } => parent,
            NodeKind::ShadowRoot { .. } => match parent_node.parent {
                Some(host) => host,
                None => return "ltr",
            },
            _ => return "ltr",
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_direction_uses_first_strong_character() {
        assert_eq!(first_strong("123 \u{0301}"), None);
        assert_eq!(first_strong("123 \u{05ea}abc"), Some("rtl"));
        assert_eq!(first_strong("123 a\u{05ea}"), Some("ltr"));
        assert_eq!(first_strong("\u{1e900}"), Some("rtl"));
    }
    #[test]
    fn auto_direction_excludes_isolated_subtrees_and_css() {
        let parsed = oryn_dom::html::parse(
            "<div dir=rtl><div id=auto dir=auto style='direction:rtl'><bdi>ת</bdi><span dir=rtl>ת</span>A</div><bdi id=isolate>ת</bdi><input id=phone type=tel><div id=inherit dir=invalid></div></div>",
        );
        for (id, expected) in [
            ("auto", "ltr"),
            ("isolate", "rtl"),
            ("phone", "ltr"),
            ("inherit", "rtl"),
        ] {
            let node = parsed
                .dom
                .iter()
                .find(|(_, node)| node.attributes.get("id").is_some_and(|value| value == id))
                .unwrap()
                .0;
            assert_eq!(directionality(&parsed.dom, node), expected, "{id}");
        }
    }
}
