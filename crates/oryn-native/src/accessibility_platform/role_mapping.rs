//! Native AX role semantics shared by AppKit publication and source regressions.
pub fn role(value: &str) -> &'static str {
    match value {
        "button" => "AXButton",
        "checkbox" | "switch" => "AXCheckBox",
        "radio" | "tab" => "AXRadioButton",
        "radiogroup" => "AXRadioGroup",
        "textbox" | "searchbox" => "AXTextField",
        "combobox" => "AXComboBox",
        "list" | "listbox" => "AXList",
        "option" | "text" | "statictext" => "AXStaticText",
        "link" => "AXLink",
        "heading" => "AXHeading",
        "img" | "image" => "AXImage",
        "table" | "grid" | "treegrid" => "AXTable",
        "tree" => "AXOutline",
        "row" | "treeitem" => "AXRow",
        "cell" | "gridcell" | "columnheader" | "rowheader" => "AXCell",
        "slider" => "AXSlider",
        "scrollbar" => "AXScrollBar",
        "colorwell" => "AXColorWell",
        "spinbutton" => "AXIncrementor",
        "progressbar" | "meter" => "AXProgressIndicator",
        "tablist" => "AXTabGroup",
        "menu" => "AXMenu",
        "menubar" => "AXMenuBar",
        "menuitem" | "menuitemcheckbox" | "menuitemradio" => "AXMenuItem",
        "separator" => "AXSplitter",
        "toolbar" => "AXToolbar",
        "document" => "AXWebArea",
        _ => "AXGroup",
    }
}
pub fn subrole(value: &str, protected: bool) -> Option<&'static str> {
    if protected {
        return Some("AXSecureTextField");
    }
    match value {
        "searchbox" => Some("AXSearchField"),
        "switch" => Some("AXSwitch"),
        "treeitem" => Some("AXOutlineRow"),
        "article" => Some("AXDocumentArticle"),
        "region" => Some("AXLandmarkRegion"),
        "term" => Some("AXTerm"),
        "dialog" => Some("AXApplicationDialog"),
        "alertdialog" => Some("AXApplicationAlertDialog"),
        "application" => Some("AXWebApplication"),
        "group" | "feed" => Some("AXApplicationGroup"),
        _ => None,
    }
}
pub fn description(value: &str) -> &str {
    match value {
        "combobox" => "combo box",
        "searchbox" => "search text field",
        "textbox" => "text field",
        "listbox" => "list",
        "option" | "statictext" => "text",
        "grid" | "treegrid" => "table",
        "tree" => "outline",
        "treeitem" => "outline row",
        "radiogroup" => "radio group",
        "tablist" => "tab group",
        "menubar" => "menu bar",
        "menuitem" | "menuitemcheckbox" | "menuitemradio" => "menu item",
        "scrollbar" => "scroll bar",
        "spinbutton" => "stepper",
        "separator" => "splitter",
        "dialog" => "web dialog",
        "alertdialog" => "web alert dialog",
        "application" => "web application",
        "columnheader" | "rowheader" | "gridcell" => "cell",
        "generic" | "none" | "listitem" => "group",
        other => other,
    }
}
pub fn orientation(value: &str, explicit: Option<&str>) -> &'static str {
    match explicit {
        Some("horizontal") => "AXHorizontalOrientation",
        Some("vertical") => "AXVerticalOrientation",
        _ => match value {
            "slider" | "separator" | "tablist" | "toolbar" | "menubar" => "AXHorizontalOrientation",
            "listbox" | "menu" | "tree" | "scrollbar" => "AXVerticalOrientation",
            _ => "AXUnknownOrientation",
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_image_and_widget_roles_match_native_ax_contract() {
        for (input, output) in [
            ("image", "AXImage"),
            ("switch", "AXCheckBox"),
            ("tree", "AXOutline"),
            ("treegrid", "AXTable"),
            ("menubar", "AXMenuBar"),
            ("radiogroup", "AXRadioGroup"),
        ] {
            assert_eq!(role(input), output);
        }
        assert_eq!(subrole("switch", false), Some("AXSwitch"));
        assert_eq!(subrole("textbox", true), Some("AXSecureTextField"));
        assert_eq!(description("combobox"), "combo box");
        assert_eq!(orientation("combobox", None), "AXUnknownOrientation");
        assert_eq!(
            orientation("slider", Some("vertical")),
            "AXVerticalOrientation"
        );
    }
}

pub fn changed_notifications(old: &super::Node, node: &super::Node) -> Vec<&'static str> {
    let mut result = Vec::new();
    if old.value != node.value
        || [
            "checked",
            "pressed",
            "valuenow",
            "native_value_now",
            "valuetext",
        ]
        .iter()
        .any(|key| old.properties.get(*key) != node.properties.get(*key))
    {
        result.push("AXValueChanged");
    }
    if old.name != node.name {
        result.push("AXTitleChanged");
    }
    if old.text.as_ref().map(|text| (text.start, text.end))
        != node.text.as_ref().map(|text| (text.start, text.end))
    {
        result.push("AXSelectedTextChanged");
    }
    if old.properties.get("busy") != node.properties.get("busy") {
        result.push("AXElementBusyChanged");
    }
    if old.states.contains(&oryn_common::v3::NodeState::Selected)
        != node.states.contains(&oryn_common::v3::NodeState::Selected)
    {
        result.push("AXSelectedChildrenChanged");
    }
    if old.states.contains(&oryn_common::v3::NodeState::Expanded)
        != node.states.contains(&oryn_common::v3::NodeState::Expanded)
    {
        result.push("AXExpandedChanged");
    }
    result
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    #[test]
    fn property_and_selection_deltas_emit_exact_native_notifications() {
        let snapshot = super::super::tests::fixture();
        let old = snapshot.nodes.values().next().unwrap().clone();
        let mut node = old.clone();
        node.properties
            .insert("busy".into(), serde_json::json!(true));
        node.properties
            .insert("checked".into(), serde_json::json!(true));
        node.states.insert(oryn_common::v3::NodeState::Selected);
        assert_eq!(
            changed_notifications(&old, &node),
            vec![
                "AXValueChanged",
                "AXElementBusyChanged",
                "AXSelectedChildrenChanged"
            ]
        );
        assert!(changed_notifications(&node, &node).is_empty());
    }
}

pub fn popup(properties: &std::collections::BTreeMap<String, serde_json::Value>) -> bool {
    properties.get("haspopup").is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_str().is_some_and(|value| {
                matches!(
                    value,
                    "true" | "menu" | "listbox" | "tree" | "grid" | "dialog"
                )
            })
    })
}

pub fn toggle_value(
    properties: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<usize> {
    for key in ["checked", "pressed"] {
        match properties.get(key) {
            Some(serde_json::Value::Bool(value)) => return Some(usize::from(*value)),
            Some(serde_json::Value::String(value)) if value == "mixed" => return Some(2),
            _ => {}
        }
    }
    None
}
pub fn numeric_bound(
    role: &str,
    minimum: bool,
    properties: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<f64> {
    properties
        .get(if minimum {
            "native_value_min"
        } else {
            "native_value_max"
        })
        .or_else(|| properties.get(if minimum { "valuemin" } else { "valuemax" }))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| match role {
            "slider" | "scrollbar" | "separator" | "progressbar" => {
                Some(if minimum { 0. } else { 100. })
            }
            "spinbutton" => Some(if minimum {
                -9007199254740992.
            } else {
                9007199254740992.
            }),
            _ => None,
        })
}

/// Author role descriptions require a meaningful computed role. A generic
/// container cannot gain semantics merely by changing its spoken description.
pub fn authored_description<'a>(
    role: &str,
    properties: &'a std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<&'a str> {
    if matches!(role, "generic" | "none" | "presentation") {
        return None;
    }
    properties
        .get("roledescription")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
}
#[cfg(test)]
mod description_tests {
    use super::*;
    #[test]
    fn generic_role_rejects_authored_description_and_whitespace_is_missing() {
        let mut properties = std::collections::BTreeMap::from([(
            "roledescription".into(),
            serde_json::json!("SassyButton"),
        )]);
        assert_eq!(
            authored_description("button", &properties),
            Some("SassyButton")
        );
        assert_eq!(authored_description("generic", &properties), None);
        properties.insert("roledescription".into(), serde_json::json!(" \t\n"));
        assert_eq!(authored_description("button", &properties), None);
    }
}

pub fn offers_popup(
    role: &str,
    properties: &std::collections::BTreeMap<String, serde_json::Value>,
) -> bool {
    popup(properties) || (role == "combobox" && !properties.contains_key("haspopup"))
}
#[cfg(test)]
mod popup_tests {
    use super::*;
    #[test]
    fn combobox_implicit_popup_does_not_override_explicit_false() {
        let mut properties = std::collections::BTreeMap::new();
        assert!(offers_popup("combobox", &properties));
        assert!(!offers_popup("button", &properties));
        properties.insert("haspopup".into(), serde_json::json!(false));
        assert!(!offers_popup("combobox", &properties));
        properties.insert("haspopup".into(), serde_json::json!("dialog"));
        assert!(offers_popup("combobox", &properties));
    }
}

pub fn value_description<'a>(
    role: &str,
    focusable: bool,
    properties: &'a std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<&'a str> {
    if role == "separator" && !focusable {
        None
    } else {
        properties
            .get("valuetext")
            .and_then(serde_json::Value::as_str)
    }
}
pub fn is_active_descendant(owner: &super::Node, target: oryn_common::v3::SemanticRef) -> bool {
    owner.states.contains(&oryn_common::v3::NodeState::Focused)
        && owner
            .relations
            .get(&oryn_common::v5::AccessibilityRelation::ActiveDescendant)
            .and_then(|targets| targets.first())
            == Some(&target)
}
#[cfg(test)]
mod manual_context_tests {
    use super::*;
    #[test]
    fn unfocusable_separator_does_not_expose_range_value_text() {
        let properties =
            std::collections::BTreeMap::from([("valuetext".into(), serde_json::json!("Bonaire"))]);
        assert_eq!(value_description("separator", false, &properties), None);
        assert_eq!(
            value_description("separator", true, &properties),
            Some("Bonaire")
        );
        assert_eq!(orientation("menubar", None), "AXHorizontalOrientation");
    }
    #[test]
    fn virtual_focus_requires_current_owner_focus_and_selected_relation() {
        let mut owner = super::super::tests::fixture()
            .nodes
            .into_values()
            .next()
            .unwrap();
        let target = oryn_common::v3::SemanticRef {
            node: owner.id.node + 1,
            ..owner.id
        };
        owner.states.remove(&oryn_common::v3::NodeState::Focused);
        owner.relations.insert(
            oryn_common::v5::AccessibilityRelation::ActiveDescendant,
            vec![target],
        );
        assert!(!is_active_descendant(&owner, target));
        owner.states.insert(oryn_common::v3::NodeState::Focused);
        assert!(is_active_descendant(&owner, target));
        assert!(!is_active_descendant(&owner, owner.id));
    }
}

/// Reverse label references in native paint order, including multiple controls
/// that legitimately share one label. Uses identities, never author ID strings.
pub fn labelled_controls<'a>(
    label: oryn_common::v3::SemanticRef,
    nodes: impl Iterator<Item = &'a super::Node>,
) -> Vec<oryn_common::v3::SemanticRef> {
    let mut values = nodes
        .filter(|node| {
            node.relations
                .get(&oryn_common::v5::AccessibilityRelation::LabelledBy)
                .is_some_and(|targets| targets.contains(&label))
        })
        .collect::<Vec<_>>();
    values.sort_by(|a, b| {
        a.paint_order
            .cmp(&b.paint_order)
            .then_with(|| a.id.cmp(&b.id))
    });
    values.into_iter().map(|node| node.id).collect()
}

#[cfg(test)]
mod label_relation_tests {
    use super::*;
    #[test]
    fn reverse_labels_follow_published_identity_and_document_paint_order() {
        let snapshot = super::super::tests::fixture();
        let label = *snapshot.nodes.keys().next().unwrap();
        let mut first = snapshot.nodes[&label].clone();
        first.id.node += 1;
        first.paint_order = vec![2];
        first.relations.insert(
            oryn_common::v5::AccessibilityRelation::LabelledBy,
            vec![label],
        );
        let mut second = first.clone();
        second.id.node += 1;
        second.paint_order = vec![1];
        assert_eq!(
            labelled_controls(label, [&first, &second].into_iter()),
            vec![second.id, first.id]
        );
        second.relations.clear();
        assert_eq!(
            labelled_controls(label, [&first, &second].into_iter()),
            vec![first.id]
        );
        let mut retired = label;
        retired.document_generation += 1;
        assert!(labelled_controls(retired, [&first, &second].into_iter()).is_empty());
    }
}
