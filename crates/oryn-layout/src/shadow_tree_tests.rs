//! Native shadow cascade regressions; generated during the all-source phase.
use super::*;
fn element(id: u64, parent: Option<u64>, tag: &str, children: Vec<u64>) -> LayoutNode {
    serde_json::from_value(serde_json::json!({"id":id,"parent":parent,"tag":tag,"namespace":"http://www.w3.org/1999/xhtml","children":children,"attributes":{},"text":""})).unwrap()
}
fn fixture() -> LayoutInput {
    let mut input:LayoutInput=serde_json::from_value(serde_json::json!({
        "nodes":[element(1,None,"html",vec![2,3]),element(2,Some(1),"x-host",vec![4]),element(3,Some(1),"p",vec![]),element(4,Some(2),"span",vec![]),element(5,None,"div",vec![6,7]),element(6,Some(5),"slot",vec![]),element(7,Some(5),"p",vec![])],
        "stylesheets":["p { width: 11px } x-host { display:block; color:red }",":host { width:200px; color:green } div {width:130px} p {width:23px} slot {color:blue} ::slotted(span) {height:17px}"],
        "viewport":ViewportState::default(),"scroll":ScrollState::default()
    })).unwrap();
    input.shadow_trees.roots.insert(
        100,
        ShadowTree {
            host: 2,
            content: vec![LayoutContent::Element(5)],
        },
    );
    input
        .shadow_trees
        .node_scopes
        .extend([(5, 100), (6, 100), (7, 100)]);
    input.shadow_trees.stylesheet_scopes.insert(1, 100);
    input.shadow_trees.assigned_slots.insert(4, 6);
    input
}
#[test]
fn shadow_selectors_host_slotted_inheritance_and_native_formatting_share_ids() {
    let input = fixture();
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&3], "width", ""),
        "11px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&7], "width", ""),
        "23px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&2], "width", ""),
        "200px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&4], "height", ""),
        "17px"
    );
    assert_eq!(
        output.computed_styles[&4].get("color"),
        output.computed_styles[&6].get("color")
    );
    let flat = project_rendered_tree(&input);
    assert_eq!(
        flat.nodes.iter().find(|node| node.id == 4).unwrap().parent,
        Some(6)
    );
    assert_eq!(
        input.nodes.iter().find(|node| node.id == 4).unwrap().parent,
        Some(2)
    );
}
#[test]
fn shadow_document_combinators_and_root_never_cross_boundary() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" x-host p {height:999px} div {width:999px}");
    input.stylesheets[1].push_str(" :root {height:777px} :host > div {height:31px}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&5], "width", ""),
        "130px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&5], "height", ""),
        "31px"
    );
    assert_ne!(
        style_value(&output.computed_styles[&7], "height", ""),
        "999px"
    );
    assert_ne!(
        style_value(&output.computed_styles[&5], "height", ""),
        "777px"
    );
}
#[test]
fn shadow_unassigned_light_dom_has_style_but_no_rendered_box() {
    let mut input = fixture();
    input.shadow_trees.assigned_slots.clear();
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert!(output.computed_styles.contains_key(&4));
    assert!(!output.geometry.contains_key(&4));
    assert!(
        !project_rendered_tree(&input)
            .nodes
            .iter()
            .any(|node| node.id == 4)
    );
}
#[test]
fn shadow_sheet_replacement_and_slot_reassignment_rebuild_native_cascade() {
    let mut input = fixture();
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    input.stylesheets[1] = "p {width:47px} slot{color:purple}".into();
    input.shadow_trees.assigned_slots.clear();
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        first.computed_styles[&7].get("width"),
        second.computed_styles[&7].get("width")
    );
    assert!(!second.geometry.contains_key(&4));
}
#[test]
fn shadow_identity_cycles_and_foreign_slot_assignments_are_rejected() {
    let mut input = fixture();
    input.shadow_trees.assigned_slots.insert(3, 6);
    assert!(shadow_tree::validate(&input).is_err());
    let mut input = fixture();
    input.shadow_trees.node_scopes.insert(2, 100);
    assert!(shadow_tree::validate(&input).is_err());
}
#[test]
fn shadow_implicit_scope_is_registered_per_live_host_and_sheet() {
    let mut input = fixture();
    input.stylesheets[1] = "@scope { p{width:37px} }".into();
    input
        .stylesheet_scope_roots
        .insert(1, StylesheetScopeRoot::Constructed);
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&first.computed_styles[&7], "width", ""), "37px");
    input.stylesheets[1] = "@scope { p{width:43px} }".into();
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&second.computed_styles[&7], "width", ""),
        "43px"
    );
}
#[test]
fn shadow_assigned_text_inherits_slot_and_fallback_stays_unrendered() {
    let mut input = fixture();
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 2)
        .unwrap()
        .content = Some(vec![
        LayoutContent::Text {
            node_id: 200,
            text: "assigned text".into(),
        },
        LayoutContent::Element(4),
    ]);
    input.shadow_trees.assigned_slots.insert(200, 6);
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 6)
        .unwrap()
        .content = Some(vec![LayoutContent::Text {
        node_id: 201,
        text: "fallback".into(),
    }]);
    let flat = project_rendered_tree(&input);
    let slot = flat.nodes.iter().find(|node| node.id == 6).unwrap();
    assert_eq!(slot.text, "assigned text");
    assert!(
        !slot
            .content
            .as_ref()
            .unwrap()
            .iter()
            .any(|item| matches!(item,LayoutContent::Text{text,..} if text=="fallback"))
    );
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        output.computed_styles[&4].get("color"),
        output.computed_styles[&6].get("color")
    );
}
#[test]
fn shadow_counter_names_are_local_and_parent_definitions_keep_their_scope() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" @counter-style local {system:cyclic;symbols:'D'} @counter-style inherited {system:extends local}");
    input.stylesheets[1].push_str(" @counter-style local {system:cyclic;symbols:'S'}");
    let document = generated_counter_styles::Registry::read_for_scope(&input, false, None).unwrap();
    let shadow =
        generated_counter_styles::Registry::read_for_scope(&input, false, Some(100)).unwrap();
    assert_eq!(document.format(1, "local"), "D");
    assert_eq!(shadow.format(1, "local"), "S");
    assert_eq!(shadow.format(1, "inherited"), "D");
}
#[test]
fn native_nested_shadow_part_export_preserves_selector_scope() {
    let mut input = fixture();
    input.nodes.push(element(8, Some(5), "inner-host", vec![]));
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 5)
        .unwrap()
        .children
        .push(8);
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 8)
        .unwrap()
        .attributes
        .insert("exportparts".into(), "inside: outside".into());
    let mut part = element(9, None, "span", vec![]);
    part.attributes.insert("part".into(), "inside".into());
    input.nodes.push(part);
    input.shadow_trees.node_scopes.extend([(8, 100), (9, 101)]);
    input.shadow_trees.roots.insert(
        101,
        ShadowTree {
            host: 8,
            content: vec![LayoutContent::Element(9)],
        },
    );
    input.stylesheets[0].push_str(" x-host::part(outside){width:61px}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&9], "width", ""),
        "61px"
    );
}
#[test]
fn inherited_list_style_keeps_declaration_tree_across_shadow_boundary() {
    let mut input = fixture();
    input.stylesheets[0]
        .push_str(" @counter-style local{system:cyclic;symbols:'D'} x-host{list-style-type:local}");
    input.stylesheets[1]
        .push_str(" @counter-style local{system:cyclic;symbols:'S'} p{display:list-item}");
    let inherited = compute(&input, &LayoutLimits::default()).unwrap();
    let marker = |output: &LayoutOutput| {
        output
            .generated_nodes
            .iter()
            .filter(|node| node.parent_id == Some(7))
            .flat_map(|node| node.text.iter())
            .map(|fragment| fragment.text.as_str())
            .collect::<String>()
    };
    assert!(marker(&inherited).contains('D'));
    input.stylesheets[1].push_str(" p{list-style-type:local}");
    let local = compute(&input, &LayoutLimits::default()).unwrap();
    assert!(marker(&local).contains('S'));
}
#[test]
fn resolved_native_text_preserves_expansion_and_erased_space_source_offsets() {
    let mut source = text_source::TextSource::default();
    source.append(1, Some(20), " straße  ");
    let style = BTreeMap::from([
        ("text-transform".into(), "uppercase".into()),
        ("white-space-collapse".into(), "collapse".into()),
    ]);
    let mut engine = TextEngine::new(&[]);
    let shaped = engine.shape_source(
        &source,
        300.,
        16.,
        &style,
        ViewportState::default(),
        &BTreeMap::new(),
    );
    let run = &shaped.resolved_text[0];
    assert!(run.text.contains("STRASSE"));
    assert!(run.spans.iter().any(|span| span.source_node == Some(20)
        && span.transformed
        && run.text.get(span.rendered_start..span.rendered_end) == Some("SS")));
    assert!(
        run.spans
            .iter()
            .chain(&run.aliases)
            .all(|span| span.source_end <= 9)
    );
    assert!(
        run.aliases
            .iter()
            .any(|span| span.source_node == Some(20) && span.source_end > span.source_start)
    );
}
#[test]
fn native_resolved_text_output_is_from_final_layout_with_dom_provenance() {
    let mut input = fixture();
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 7)
        .unwrap()
        .content = Some(vec![LayoutContent::Text {
        node_id: 300,
        text: "one  two".into(),
    }]);
    // Exercise collapsed source provenance without a soft line break first.
    input.stylesheets[1].push_str(" p {width:200px}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    let runs = output
        .resolved_text
        .iter()
        .filter(|run| run.spans.iter().any(|span| span.source_node == Some(300)))
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "one two");
    assert!(
        runs[0]
            .spans
            .iter()
            .all(|span| span.rendered_end <= runs[0].text.len())
    );
    // At a soft line edge the collapsed space has no painted advance, but
    // both original spaces must remain addressable through source aliases.
    input.stylesheets[1].push_str(" p {width:23px}");
    let wrapped = compute(&input, &LayoutLimits::default()).unwrap();
    let run = wrapped
        .resolved_text
        .iter()
        .find(|run| run.spans.iter().any(|span| span.source_node == Some(300)))
        .unwrap();
    assert_eq!(run.text, "onetwo");
    for source in [3, 4] {
        assert!(run.aliases.iter().any(|span| span.source_node == Some(300)
            && span.source_start <= source
            && span.source_end > source
            && span.rendered_start == 3
            && span.rendered_end == 3));
    }
    assert!(run.spans.iter().any(|span| span.source_node == Some(300)
        && span.source_start == 5
        && span.rendered_start == 3));
    assert!(
        run.spans
            .iter()
            .chain(&run.aliases)
            .all(|span| span.source_end <= 8 && span.rendered_end <= run.text.len())
    );
}
#[test]
fn inherited_counter_descriptor_fallback_uses_its_defining_tree() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" @counter-style label{system:cyclic;symbols:'D'} @counter-style restricted{system:fixed 1;symbols:'one';fallback:label}");
    input.stylesheets[1].push_str(" @counter-style label{system:cyclic;symbols:'S'} @counter-style inherited{system:extends restricted}");
    let registry =
        generated_counter_styles::Registry::read_for_scope(&input, false, Some(100)).unwrap();
    assert_eq!(registry.format(7, "inherited"), "D");
}
#[test]
fn slotted_generated_counter_content_resolves_in_its_declaration_tree() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" @counter-style label{system:cyclic;symbols:'D'}");
    input.stylesheets[1].push_str(" @counter-style label{system:cyclic;symbols:'S'} ::slotted(span)::before {content:counter(item,label)}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert!(
        output
            .generated_content
            .iter()
            .any(|content| content.owner == 4 && content.pseudo == "before" && content.text == "S")
    );
}
#[test]
fn native_selector_hooks_preserve_state_specificity_and_shadow_language() {
    let mut input = fixture();
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 2)
        .unwrap()
        .attributes
        .insert("lang".into(), "zh-Hant-TW".into());
    input.custom_states.insert(7, ["ready".into()].into());
    input.stylesheets[1]
        .push_str(" p:state(ready){height:29px} p:lang(\"de\", \"zh-*-TW\"){width:59px}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&7], "height", ""),
        "29px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&7], "width", ""),
        "59px"
    );
    input.custom_states.clear();
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        style_value(&output.computed_styles[&7], "height", ""),
        "29px"
    );
}
#[test]
fn qualified_attribute_selectors_keep_equal_local_names_separate() {
    let mut input = fixture();
    let node = input.nodes.iter_mut().find(|node| node.id == 3).unwrap();
    node.attributes.insert("flag".into(), "plain".into());
    node.namespaced_attributes.push(LayoutAttribute {
        namespace: "urn:case".into(),
        local_name: "flag".into(),
        prefix: Some("p".into()),
        value: "namespaced".into(),
    });
    input.stylesheets[0]="@namespace p url(urn:case); p[flag=plain]{width:31px} p[p|flag=namespaced]{height:37px} p[p|flag=plain]{width:999px}".into();
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&3], "width", ""),
        "31px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&3], "height", ""),
        "37px"
    );
}
#[test]
fn native_selector_state_changes_recompute_styles_without_authored_flags() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" :defined{height:5px} :placeholder-shown{height:7px} :read-write{width:41px} :read-only{width:43px} :target{height:13px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            fullscreen: false,
            media_state: 0,
            interest_source: false,
            interest_target: false,
            interest_invoker: false,
            inline_style_blocked: false,
            permission_granted: false,
            user_validity: None,
            default_control: false,
            in_range: None,
            required: None,
            defined: false,
            placeholder_shown: true,
            read_write: true,
            target: false,
        },
    );
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&first.computed_styles[&4], "height", ""), "7px");
    assert_eq!(style_value(&first.computed_styles[&4], "width", ""), "41px");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            fullscreen: false,
            media_state: 0,
            interest_source: false,
            interest_target: false,
            interest_invoker: false,
            inline_style_blocked: false,
            permission_granted: false,
            user_validity: None,
            default_control: false,
            in_range: None,
            required: None,
            defined: true,
            placeholder_shown: false,
            read_write: false,
            target: true,
        },
    );
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&second.computed_styles[&4], "height", ""),
        "13px"
    );
    assert_eq!(
        style_value(&second.computed_styles[&4], "width", ""),
        "43px"
    );
}

#[test]
fn required_and_optional_state_changes_reach_style_cascade() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" :required{width:53px} :optional{width:59px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            required: Some(true),
            ..Default::default()
        },
    );
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&first.computed_styles[&4], "width", ""), "53px");
    input.selector_states.get_mut(&4).unwrap().required = Some(false);
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&second.computed_styles[&4], "width", ""),
        "59px"
    );
    input.selector_states.get_mut(&4).unwrap().required = None;
    let third = compute(&input, &LayoutLimits::default()).unwrap();
    assert!(!matches!(
        style_value(&third.computed_styles[&4], "width", ""),
        "53px" | "59px"
    ));
}

#[test]
fn range_states_use_distinct_inapplicable_and_in_range_cascade() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" :in-range{width:61px} :out-of-range{width:67px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            in_range: Some(true),
            ..Default::default()
        },
    );
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&first.computed_styles[&4], "width", ""), "61px");
    input.selector_states.get_mut(&4).unwrap().in_range = Some(false);
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&second.computed_styles[&4], "width", ""),
        "67px"
    );
}

#[test]
fn default_control_state_reaches_cascade() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" :default{width:71px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            default_control: true,
            ..Default::default()
        },
    );
    let first = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&first.computed_styles[&4], "width", ""), "71px");
    input.selector_states.get_mut(&4).unwrap().default_control = false;
    let second = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        style_value(&second.computed_styles[&4], "width", ""),
        "71px"
    );
}

#[test]
fn user_validity_state_changes_reach_native_style_and_reset() {
    let mut input = fixture();
    input.stylesheets[0].push_str(" :user-valid{width:73px} :user-invalid{width:79px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            permission_granted: false,
            user_validity: Some(false),
            ..Default::default()
        },
    );
    let invalid = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&invalid.computed_styles[&4], "width", ""),
        "79px"
    );
    input.selector_states.get_mut(&4).unwrap().user_validity = Some(true);
    let valid = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(style_value(&valid.computed_styles[&4], "width", ""), "73px");
    input.selector_states.get_mut(&4).unwrap().user_validity = None;
    let reset = compute(&input, &LayoutLimits::default()).unwrap();
    assert!(!matches!(
        style_value(&reset.computed_styles[&4], "width", ""),
        "73px" | "79px"
    ));
}

#[test]
fn granted_selector_uses_native_permission_state_and_restyles_on_revocation() {
    let mut input = fixture();
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 4)
        .unwrap()
        .tag = "geolocation".into();
    input.stylesheets[0]
        .push_str("geolocation:granted{width:157px} geolocation:not(:granted){width:123px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            permission_granted: true,
            ..Default::default()
        },
    );
    let granted = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&granted.computed_styles[&4], "width", ""),
        "157px"
    );
    input
        .selector_states
        .get_mut(&4)
        .unwrap()
        .permission_granted = false;
    let denied = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&denied.computed_styles[&4], "width", ""),
        "123px"
    );
}

#[test]
fn focus_within_uses_flat_shadow_and_slot_ancestry_without_changing_selectors() {
    let mut input = fixture();
    input.stylesheets[0]
        .push_str(" html:focus-within{height:181px} x-host:focus-within{height:173px}");
    input.stylesheets[1]
        .push_str(" div:focus-within{height:167px} slot:focus-within{height:163px}");
    input.focused_node = Some(7);
    assert_eq!(
        shadow_tree::focus_within(&input, 7),
        BTreeSet::from([1, 2, 5, 7])
    );
    let inner = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&inner.computed_styles[&2], "height", ""),
        "173px"
    );
    assert_eq!(
        style_value(&inner.computed_styles[&5], "height", ""),
        "167px"
    );
    input.focused_node = Some(4);
    assert_eq!(
        shadow_tree::focus_within(&input, 4),
        BTreeSet::from([1, 2, 4, 5, 6])
    );
    let slotted = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&slotted.computed_styles[&6], "height", ""),
        "163px"
    );
    assert_eq!(
        input.nodes.iter().find(|node| node.id == 4).unwrap().parent,
        Some(2)
    );
    input.focused_node = Some(3);
    let moved = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        style_value(&moved.computed_styles[&2], "height", ""),
        "173px"
    );
    input.focused_node = None;
    let blurred = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        style_value(&blurred.computed_styles[&1], "height", ""),
        "181px"
    );
}

#[test]
fn native_language_cascade_uses_xml_namespace_not_lookalike_attribute() {
    let mut input = fixture();
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 2)
        .unwrap()
        .attributes
        .insert("lang".into(), "fr-Latn-FR".into());
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 2)
        .unwrap()
        .attributes
        .insert("xml:lang".into(), "de".into());
    input.stylesheets[1]
        .push_str("p:lang(fr){height:151px} p:lang(de){height:157px} p:lang(zh){height:163px}");
    let lookalike = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&lookalike.computed_styles[&7], "height", ""),
        "151px"
    );
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 2)
        .unwrap()
        .namespaced_attributes
        .push(LayoutAttribute {
            namespace: "http://www.w3.org/XML/1998/namespace".into(),
            local_name: "lang".into(),
            prefix: Some("xml".into()),
            value: "zh".into(),
        });
    let namespaced = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&namespaced.computed_styles[&7], "height", ""),
        "163px"
    );
}

#[test]
fn denied_inline_declarations_preserve_style_attribute_selector_matching() {
    let mut input = fixture();
    let node = input.nodes.iter_mut().find(|node| node.id == 4).unwrap();
    node.attributes.insert("style".into(), "width:999px".into());
    input.stylesheets[0].push_str(" [style]{width:29px;height:31px}");
    input.selector_states.insert(
        4,
        LayoutSelectorState {
            fullscreen: false,
            media_state: 0,
            interest_source: false,
            interest_target: false,
            interest_invoker: false,
            inline_style_blocked: true,
            ..Default::default()
        },
    );
    let denied = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&denied.computed_styles[&4], "width", ""),
        "29px"
    );
    assert_eq!(
        style_value(&denied.computed_styles[&4], "height", ""),
        "31px"
    );
    input
        .selector_states
        .get_mut(&4)
        .unwrap()
        .inline_style_blocked = false;
    let allowed = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&allowed.computed_styles[&4], "width", ""),
        "999px"
    );
}

#[test]
fn exported_part_enumeration_keeps_all_aliases_and_ignores_malformed_mappings() {
    let mut input = fixture();
    let mut host = element(8, Some(5), "inner-host", vec![]);
    host.attributes.insert(
        "exportparts".into(),
        "inside: outside, bad:too:many, inside: alias".into(),
    );
    input.nodes.push(host);
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 5)
        .unwrap()
        .children
        .push(8);
    let mut part = element(9, None, "span", vec![]);
    part.attributes.insert("part".into(), "inside extra".into());
    input.nodes.push(part);
    input.shadow_trees.node_scopes.extend([(8, 100), (9, 101)]);
    input.shadow_trees.roots.insert(
        101,
        ShadowTree {
            host: 8,
            content: vec![LayoutContent::Element(9)],
        },
    );
    input.stylesheets[0].push_str(" x-host::part(outside){width:61px} x-host::part(alias){height:37px} x-host::part(inside){width:99px}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        style_value(&output.computed_styles[&9], "width", ""),
        "61px"
    );
    assert_eq!(
        style_value(&output.computed_styles[&9], "height", ""),
        "37px"
    );
    input
        .nodes
        .iter_mut()
        .find(|node| node.id == 8)
        .unwrap()
        .attributes
        .remove("exportparts");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_ne!(
        style_value(&output.computed_styles[&9], "width", ""),
        "61px"
    );
    assert_ne!(
        style_value(&output.computed_styles[&9], "height", ""),
        "37px"
    );
}
