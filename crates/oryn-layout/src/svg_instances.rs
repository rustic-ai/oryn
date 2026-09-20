//! SVG instance inheritance using the reference tree's native matched rules.
use super::*;

/// Immutable native style session sampled alongside one successful layout.
/// It is private embedding state, not serialized layout or authored style data.
#[derive(Debug, Clone)]
pub struct StyleSample {
    pub(super) now: f64,
    pub(super) continuity: blitz_dom::StyleContinuity,
    pub(super) effects: Vec<blitz_dom::NativeAnimationEffect>,
}

/// Instance identities occupy a separate namespace from source node identities.
#[derive(Debug, Clone, Copy)]
pub enum StyleParent {
    Source(u64),
    Instance(u64),
}

/// Requests are parent-first. A nested use or descendant inherits from the
/// previously resolved instance, while selectors still match `source`.
#[derive(Debug, Clone, Copy)]
pub struct StyleInstance {
    pub id: u64,
    pub source: u64,
    pub parent: StyleParent,
}

#[derive(Debug, Default)]
pub struct InstanceProjection {
    pub text_fonts: TextFonts,
    pub styles: ComputedStyles,
    pub pseudo_styles: BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
}

/// Shaped native SVG text contributes both font-loading demand and the exact
/// face bytes admitted by the ordinary text engine. Input identities must be
/// the rendered instance tree, so source defs do not hide a visible reference.
#[derive(Debug, Default, Clone)]
pub struct TextFonts {
    pub demands: BTreeMap<String, BTreeSet<u32>>,
    pub resources: BTreeMap<String, oryn_common::v4::PaintFontResource>,
    pub diagnostics: Vec<String>,
}

pub fn collect_text_fonts(
    input: &LayoutInput,
    styles: &ComputedStyles,
    limits: &LayoutLimits,
) -> Result<TextFonts, LayoutError> {
    validate(input, limits)?;
    let (fonts, _, diagnostics) = prepare_web_fonts(input, limits);
    let mut engine = TextEngine::new(&fonts);
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut output = TextFonts {
        diagnostics,
        ..Default::default()
    };
    let mut source_bytes = 0usize;
    for node in &input.nodes {
        if node.namespace != "http://www.w3.org/2000/svg"
            || !matches!(node.tag.as_str(), "text" | "tspan" | "textPath")
        {
            continue;
        }
        let mut current = Some(node.id);
        let mut seen = BTreeSet::new();
        let mut hidden = false;
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(LayoutError::StyleTraversal(
                    "SVG font ancestry cycle".into(),
                ));
            }
            let style = styles.get(&id).ok_or_else(|| {
                LayoutError::StyleTraversal("SVG font ancestry lacks computed style".into())
            })?;
            if style_value(style, "display", "inline") == "none"
                || style_value(style, "content-visibility", "visible") == "hidden"
            {
                hidden = true;
                break;
            }
            current = nodes.get(&id).and_then(|node| node.parent);
        }
        if hidden {
            continue;
        }
        let source = text_source::TextSource::direct(node);
        source_bytes = source_bytes
            .checked_add(source.text.len())
            .filter(|size| *size <= 16 * 1024 * 1024)
            .ok_or_else(|| {
                LayoutError::StyleTraversal("SVG font text exceeds native bound".into())
            })?;
        if source.text.is_empty() {
            continue;
        }
        let style = &styles[&node.id];
        let font = length(
            style_value(style, "font-size", "16px"),
            16.,
            16.,
            input.viewport,
        )
        .max(1.);
        let shaped = engine.shape_source(
            &source,
            input.viewport.width.max(1.),
            font,
            style,
            input.viewport,
            styles,
        );
        for (family, characters) in text_pseudos::font_demands(&shaped)? {
            output.demands.entry(family).or_default().extend(characters);
        }
        if text_pseudos::demand_bytes(&output.demands) > 64 * 1024 * 1024 {
            return Err(LayoutError::StyleTraversal(
                "SVG font demand exceeds native bound".into(),
            ));
        }
    }
    let total = engine
        .paint_fonts
        .values()
        .try_fold(0usize, |total, font| total.checked_add(font.bytes.len()))
        .ok_or_else(|| LayoutError::StyleTraversal("SVG captured font byte overflow".into()))?;
    if total > limits.max_page_font_bytes {
        return Err(LayoutError::StyleTraversal(
            "SVG captured fonts exceed page font budget".into(),
        ));
    }
    output.resources = engine.paint_fonts;
    Ok(output)
}

pub fn compute(
    input: &LayoutInput,
    instances: &[StyleInstance],
    limits: &LayoutLimits,
) -> Result<ComputedStyles, LayoutError> {
    compute_with_pseudos(input, instances, limits).map(|projection| projection.styles)
}

pub fn compute_with_pseudos(
    input: &LayoutInput,
    instances: &[StyleInstance],
    limits: &LayoutLimits,
) -> Result<InstanceProjection, LayoutError> {
    compute_with_pseudos_for_media(input, instances, limits, false)
}

pub fn compute_with_pseudos_for_media(
    input: &LayoutInput,
    instances: &[StyleInstance],
    limits: &LayoutLimits,
    print_media: bool,
) -> Result<InstanceProjection, LayoutError> {
    compute_with_style_sample(input, instances, limits, print_media, None)
}

pub fn compute_with_style_sample(
    input: &LayoutInput,
    instances: &[StyleInstance],
    limits: &LayoutLimits,
    print_media: bool,
    sample: Option<&StyleSample>,
) -> Result<InstanceProjection, LayoutError> {
    validate(input, limits)?;
    let total = input.nodes.len().saturating_add(instances.len());
    if total > limits.max_nodes {
        return Err(LayoutError::NodeLimit {
            actual: total,
            limit: limits.max_nodes,
        });
    }
    if instances.is_empty() {
        return Ok(InstanceProjection::default());
    }
    let mut output = InstanceProjection::default();
    // Restoring/sampling a projection must not advance or alter its owner.
    let mut continuity = sample.map(|sample| sample.continuity.clone());
    stylo_computed_values_with_instances(
        input,
        print_media,
        sample.map_or(0., |sample| sample.now),
        continuity.as_mut(),
        sample.map_or(&[][..], |sample| sample.effects.as_slice()),
        None,
        Some((instances, &mut output, limits)),
    )?;
    Ok(output)
}

pub(super) fn resolve(
    document: &mut BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_dom::NodeId>,
    instances: &[StyleInstance],
    shadow_mirrors: &BTreeMap<u64, blitz_dom::NodeId>,
    limits: &LayoutLimits,
) -> Result<InstanceProjection, LayoutError> {
    let invalid = |message: &str| LayoutError::StyleTraversal(format!("SVG instance: {message}"));
    let mut native = BTreeMap::new();
    let mut native_parents = BTreeMap::new();
    let mut anchor_pseudos = Vec::new();
    let mut output = InstanceProjection::default();
    let mut shared = BTreeSet::new();
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    for instance in instances {
        if native.contains_key(&instance.id) {
            return Err(invalid("duplicate instance identity"));
        }
        let source = *mirrors
            .get(&instance.source)
            .ok_or_else(|| invalid("missing source"))?;
        let parent = match instance.parent {
            StyleParent::Source(id) => {
                let mirror = *mirrors
                    .get(&id)
                    .ok_or_else(|| invalid("missing source parent"))?;
                document
                    .resolved_primary_style(mirror)
                    .ok_or_else(|| invalid("source parent has no element style"))?
            }
            StyleParent::Instance(id) => native
                .get(&id)
                .cloned()
                .ok_or_else(|| invalid("instance parent is not earlier in the request"))?,
        };
        let computed = document
            .reparent_element_style(source, &parent)
            .ok_or_else(|| invalid("source has no element style"))?;
        let values = serialize(input, &computed, shadow_mirrors)?;
        output
            .styles
            .insert(instance.id, intern_computed_style(values, &mut shared));
        let node = nodes
            .get(&instance.source)
            .ok_or_else(|| invalid("missing source node"))?;
        let mut pseudo_native = BTreeMap::new();
        for (name, pseudo) in projected_pseudo_kinds(input, node) {
            let resolved = if name.starts_with("picker::") {
                let Some(origin) = pseudo_native.get("picker").cloned() else {
                    continue;
                };
                document.resolved_nested_pseudo_style_with_origin(source, &pseudo, origin)
            } else {
                document.resolved_instance_pseudo_style(source, &pseudo, computed.clone())
            };
            let Some(resolved) = resolved else { continue };
            let values = serialize(input, &resolved, shadow_mirrors)?;
            output
                .pseudo_styles
                .entry(instance.id)
                .or_default()
                .insert(name.into(), intern_computed_style(values, &mut shared));
            if matches!(name, "picker" | "interest-button") {
                anchor_pseudos.push((instance.id, name, resolved.clone()));
            }
            pseudo_native.insert(name, resolved);
        }
        native_parents.insert(instance.id, parent);
        native.insert(instance.id, computed);
    }
    // Formatting traverses instance identities, while native selector matching
    // continues to use the original source mirrors. Repeated references must
    // never overwrite one another's native primary style.
    let mut tree = input.clone();
    tree.nodes.clear();
    tree.top_layer.clear();
    let mut instance_mirrors = BTreeMap::new();
    let by_instance = instances
        .iter()
        .map(|instance| (instance.id, instance))
        .collect::<BTreeMap<_, _>>();
    let mut children = BTreeMap::<u64, Vec<u64>>::new();
    for instance in instances {
        if let StyleParent::Instance(parent) = instance.parent {
            children.entry(parent).or_default().push(instance.id);
        }
    }
    // A use expansion starts a distinct clone context even when it references
    // a source already present in an outer expansion. Text owners resolve in
    // that context, never by an unqualified source ID shared by other copies.
    let mut contexts = BTreeMap::new();
    let mut owners = BTreeMap::new();
    for instance in instances {
        let mut context = match instance.parent {
            StyleParent::Source(_) => instance.id,
            StyleParent::Instance(parent) => {
                let parent_source = nodes[&by_instance[&parent].source];
                if parent_source.namespace == "http://www.w3.org/2000/svg"
                    && parent_source.tag == "use"
                {
                    instance.id
                } else {
                    contexts[&parent]
                }
            }
        };
        // The public instance API also permits explicit nested copies without
        // a use source node. Their repeated source starts another context.
        if owners.contains_key(&(context, instance.source)) {
            context = instance.id;
        }
        contexts.insert(instance.id, context);
        owners.insert((context, instance.source), instance.id);
        let scope = context.to_string();
        Arc::make_mut(
            output
                .styles
                .get_mut(&instance.id)
                .ok_or_else(|| invalid("missing instance style"))?,
        )
        .insert(anchor_positioning::INSTANCE_CONTEXT.into(), scope.clone());
        if let Some(pseudos) = output.pseudo_styles.get_mut(&instance.id) {
            for values in pseudos.values_mut() {
                Arc::make_mut(values)
                    .insert(anchor_positioning::INSTANCE_CONTEXT.into(), scope.clone());
            }
        }
    }
    for instance in instances {
        let mut node = (*nodes[&instance.source]).clone();
        node.id = instance.id;
        node.parent = match instance.parent {
            StyleParent::Source(_) => None,
            StyleParent::Instance(parent) => Some(parent),
        };
        node.children = children.get(&instance.id).cloned().unwrap_or_default();
        if let Some(content) = &mut node.content {
            content.retain_mut(|item| {
                if let LayoutContent::Element(source) = item {
                    let Some(child) = node.children.iter().find(|child| {
                        by_instance
                            .get(*child)
                            .is_some_and(|instance| instance.source == *source)
                    }) else {
                        return false;
                    };
                    *source = *child;
                }
                true
            });
        }
        if let Some(content) = &mut node.content {
            for item in content {
                if let LayoutContent::StyledText { owner, .. } = item {
                    *owner =
                        instance_reference(instance.id, *owner, &contexts, &owners, &by_instance)
                            .ok_or_else(|| {
                            invalid("styled text owner is outside its instance context")
                        })?;
                }
            }
        }
        for entry in input
            .top_layer
            .iter()
            .filter(|entry| entry.node_id == instance.source)
        {
            let mut entry = *entry;
            entry.node_id = instance.id;
            tree.top_layer.push(entry);
        }
        tree.nodes.push(node);
        instance_mirrors.insert(instance.id, mirrors[&instance.source]);
    }
    project_formatting_metadata(input, &mut tree, &contexts, &owners, &by_instance);
    for (owner, name, computed) in anchor_pseudos {
        let values = output
            .pseudo_styles
            .get_mut(&owner)
            .and_then(|styles| styles.get_mut(name))
            .ok_or_else(|| invalid("missing generated anchor pseudo"))?;
        anchor_positioning::picker_cascade_with_native(
            crate::anchor_positioning::NativePickerCascadeInputs {
                document,
                input: &tree,
                mirrors: &instance_mirrors,
                styles: &output.styles,
                top_layer: name == "picker",
                owner,
                computed: &computed,
                values: Arc::make_mut(values),
                native: anchor_positioning::NativeValues(Some(&native), Some(&native_parents)),
            },
        )?;
    }
    text_pseudos::cascade_instances(
        document,
        &tree,
        &instance_mirrors,
        &mut output.styles,
        &mut output.pseudo_styles,
        &native,
    );
    permission_controls::adjust(&tree, &mut output.styles, &mut output.pseudo_styles);
    anchor_positioning::cascade_with_native(
        document,
        &tree,
        &instance_mirrors,
        &mut output.styles,
        anchor_positioning::NativeValues(Some(&native), Some(&native_parents)),
    )?;

    let mut font_tree = tree.clone();
    // Font shaping consumes already-resolved styles and the instance ancestry,
    // not original selector/shadow/scroll identities from the document clone.
    font_tree.rendered_tree = None;
    font_tree.shadow_trees = Default::default();
    font_tree.element_scrolls.clear();
    let mut font_styles = output.styles.clone();
    for instance in instances {
        let StyleParent::Source(parent) = instance.parent else {
            continue;
        };
        let mut current = Some(parent);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(invalid("source font ancestry cycle"));
            }
            let mirror = mirrors
                .get(&id)
                .ok_or_else(|| invalid("missing source font ancestor"))?;
            let computed = document
                .resolved_primary_style(*mirror)
                .ok_or_else(|| invalid("source font ancestor lacks style"))?;
            if computed.computed_value_to_string(
                style::properties::PropertyDeclarationId::Longhand(
                    style::properties::LonghandId::Display,
                ),
            ) == "none"
                || computed.computed_value_to_string(
                    style::properties::PropertyDeclarationId::Longhand(
                        style::properties::LonghandId::ContentVisibility,
                    ),
                ) == "hidden"
            {
                Arc::make_mut(
                    font_styles
                        .get_mut(&instance.id)
                        .ok_or_else(|| invalid("missing instance font root"))?,
                )
                .insert("display".into(), "none".into());
                break;
            }
            current = nodes.get(&id).and_then(|node| node.parent);
        }
    }
    output.text_fonts = collect_text_fonts(&font_tree, &font_styles, limits)?;

    // The renderer projects each instance back onto source-node geometry.
    // Relabel only generated origin metadata, retaining separate style maps.
    for style in output.styles.values_mut().chain(
        output
            .pseudo_styles
            .values_mut()
            .flat_map(|styles| styles.values_mut()),
    ) {
        remap_anchor_parents(Arc::make_mut(style), &by_instance)?;
        for key in [text_pseudos::KEY, text_pseudos::PICKER_KEY] {
            let Some(value) = style.get(key) else {
                continue;
            };
            let mut variants: text_pseudos::Variants = serde_json::from_str(value)
                .map_err(|_| invalid("invalid generated text pseudo metadata"))?;
            for origin in [&mut variants.line_origin, &mut variants.letter_origin] {
                if let Some(id) = *origin {
                    *origin = Some(
                        by_instance
                            .get(&id)
                            .ok_or_else(|| invalid("unknown generated text pseudo origin"))?
                            .source,
                    );
                }
            }
            let value = serde_json::to_string(&variants)
                .map_err(|_| invalid("cannot serialize text pseudo metadata"))?;
            Arc::make_mut(style).insert(key.into(), value);
        }
    }
    Ok(output)
}

// Source IDs and instance IDs may have the same integer value. Resolve a
// source reference through its clone context before considering real ancestors.
fn instance_reference(
    owner: u64,
    source: u64,
    contexts: &BTreeMap<u64, u64>,
    owners: &BTreeMap<(u64, u64), u64>,
    instances: &BTreeMap<u64, &StyleInstance>,
) -> Option<u64> {
    if let Some(id) = owners.get(&(*contexts.get(&owner)?, source)) {
        return Some(*id);
    }
    let mut current = Some(owner);
    while let Some(id) = current {
        let instance = instances.get(&id)?;
        if instance.source == source {
            return Some(id);
        }
        current = match instance.parent {
            StyleParent::Instance(parent) => Some(parent),
            StyleParent::Source(_) => None,
        };
    }
    None
}

fn project_formatting_metadata(
    input: &LayoutInput,
    tree: &mut LayoutInput,
    contexts: &BTreeMap<u64, u64>,
    owners: &BTreeMap<(u64, u64), u64>,
    instances: &BTreeMap<u64, &StyleInstance>,
) {
    // Native selectors have already resolved on the source document. Formatting
    // consumes the generated node forest, not the source's flat tree or slots.
    tree.rendered_tree = None;
    tree.shadow_trees = ShadowTrees::default();
    tree.element_scrolls.clear();
    for instance in instances.values() {
        if let Some(scope) = input.shadow_trees.node_scopes.get(&instance.source) {
            tree.shadow_trees.node_scopes.insert(instance.id, *scope);
        }
    }
    for entry in &mut tree.top_layer {
        entry.anchor_source = entry.anchor_source.and_then(|source| {
            instance_reference(entry.node_id, source, contexts, owners, instances)
        });
        if let Some(picker) = &mut entry.picker {
            picker.active = picker.active.and_then(|source| {
                instance_reference(entry.node_id, source, contexts, owners, instances)
            });
        }
    }
}

fn remap_anchor_parents(
    values: &mut BTreeMap<String, String>,
    instances: &BTreeMap<u64, &StyleInstance>,
) -> Result<(), LayoutError> {
    if let Some(parent) = values.get_mut("-oryn-anchor-parent")
        && parent != "none"
    {
        let id = parent
            .parse::<u64>()
            .ok()
            .and_then(|id| instances.get(&id))
            .ok_or_else(|| {
                LayoutError::StyleTraversal("unknown SVG instance anchor parent".into())
            })?;
        *parent = id.source.to_string();
    }
    if let Some(serialized) = values.get_mut("-oryn-position-tries") {
        let mut candidates: Vec<BTreeMap<String, String>> = serde_json::from_str(serialized)
            .map_err(|error| LayoutError::StyleTraversal(error.to_string()))?;
        for candidate in &mut candidates {
            remap_anchor_parents(candidate, instances)?;
        }
        *serialized = serde_json::to_string(&candidates)
            .map_err(|error| LayoutError::StyleTraversal(error.to_string()))?;
    }
    Ok(())
}

fn serialize(
    input: &LayoutInput,
    computed: &style::properties::ComputedValues,
    shadow_mirrors: &BTreeMap<u64, blitz_dom::NodeId>,
) -> Result<BTreeMap<String, String>, LayoutError> {
    let mut values = BTreeMap::new();
    for name in COMPUTED_PROPERTIES {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            continue;
        };
        let value = match property.as_shorthand() {
            Ok(shorthand) => {
                let declarations = shorthand
                    .longhands()
                    .map(|id| computed.computed_or_resolved_declaration(id, None))
                    .collect::<Vec<_>>();
                let references = declarations.iter().collect::<Vec<_>>();
                let mut value = String::new();
                shorthand
                    .longhands_to_css(&references, &mut value)
                    .map_err(|_| {
                        LayoutError::StyleTraversal(
                            "SVG instance cannot serialize computed shorthand".into(),
                        )
                    })?;
                value
            }
            Err(longhand) => computed.computed_value_to_string(longhand),
        };
        values.insert((*name).into(), value);
    }
    let custom = computed.custom_properties();
    for (name, value) in custom.inherited.iter().chain(custom.non_inherited.iter()) {
        if let Some(value) = value {
            values.insert(format!("--{name}"), value.to_css_string());
        }
    }
    values.insert(
        "oryn-effective-zoom".into(),
        computed.effective_zoom.value().to_string(),
    );
    values.insert(
        "oryn-preferred-color-scheme".into(),
        if input.color_scheme == ColorScheme::Dark {
            "dark"
        } else {
            "light"
        }
        .into(),
    );
    values.insert(
        "oryn-reduced-motion".into(),
        if input.reduced_motion {
            "reduce"
        } else {
            "no-preference"
        }
        .into(),
    );
    let scope_name = |scope: usize| -> Result<String, LayoutError> {
        if scope == 0 {
            return Ok("document".into());
        }
        shadow_mirrors
            .iter()
            .find_map(|(root, mirror)| {
                (mirror.as_u64() as usize == scope).then(|| root.to_string())
            })
            .ok_or_else(|| LayoutError::StyleTraversal("unknown SVG instance cascade scope".into()))
    };
    values.insert(
        "oryn-list-style-tree".into(),
        scope_name(computed.get_list().clone_list_style_type().1)?,
    );
    if let style::values::generics::counters::Content::Items(items) =
        &computed.get_counters().content
        && let Some(scope) = items.native_scope
    {
        values.insert("oryn-content-tree".into(), scope_name(scope)?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(reference: &str, sheet: &str) -> LayoutInput {
        serde_json::from_value(serde_json::json!({
            "nodes": [
                {"id":1,"parent":null,"tag":"svg","namespace":"http://www.w3.org/2000/svg","children":[2,4,5]},
                {"id":2,"parent":1,"tag":"defs","namespace":"http://www.w3.org/2000/svg","children":[3]},
                {"id":3,"parent":2,"tag":"path","namespace":"http://www.w3.org/2000/svg","attributes":{"id":"shape","style":reference}},
                {"id":4,"parent":1,"tag":"use","namespace":"http://www.w3.org/2000/svg","attributes":{"style":"fill:red;color:red;--tone:red"}},
                {"id":5,"parent":1,"tag":"use","namespace":"http://www.w3.org/2000/svg","attributes":{"style":"fill:blue;color:blue;--tone:blue"}}
            ],
            "stylesheets":[sheet],
            "viewport":{"width":6,"height":2,"device_pixel_ratio":1},
            "scroll":{"x":0,"y":0,"content_width":0,"content_height":0}
        })).unwrap()
    }

    #[test]
    fn formatting_metadata_maps_source_references_without_cross_clone_id_aliases() {
        use oryn_common::v4::{SelectPickerState, TopLayerEntry, TopLayerKind};
        let mut input = input("", "");
        input.shadow_trees.node_scopes = BTreeMap::from([(3, 700), (6, 701), (10, 999)]);
        input.shadow_trees.assigned_slots.insert(6, 3);
        input.shadow_trees.stylesheet_scopes.insert(0, 700);
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 20,
                source: 6,
                parent: StyleParent::Instance(10),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 21,
                source: 6,
                parent: StyleParent::Instance(11),
            },
            // This numeric source ID deliberately aliases another instance ID.
            StyleInstance {
                id: 30,
                source: 20,
                parent: StyleParent::Source(5),
            },
        ];
        let instances = requests.iter().map(|item| (item.id, item)).collect();
        let contexts = BTreeMap::from([(10, 10), (20, 10), (11, 11), (21, 11), (30, 30)]);
        let owners = BTreeMap::from([
            ((10, 3), 10),
            ((10, 6), 20),
            ((11, 3), 11),
            ((11, 6), 21),
            ((30, 20), 30),
        ]);
        let mut tree = input.clone();
        tree.top_layer = [20, 21]
            .into_iter()
            .map(|id| TopLayerEntry {
                node_id: id,
                anchor_source: Some(3),
                kind: TopLayerKind::SelectPicker,
                picker: Some(SelectPickerState {
                    scroll: (0., 0.),
                    active: Some(6),
                    reveal: true,
                }),
            })
            .collect();
        tree.top_layer.push(TopLayerEntry {
            node_id: 10,
            anchor_source: Some(20),
            kind: TopLayerKind::Popover,
            picker: None,
        });
        project_formatting_metadata(&input, &mut tree, &contexts, &owners, &instances);
        assert_eq!(
            tree.shadow_trees.node_scopes,
            BTreeMap::from([(10, 700), (11, 700), (20, 701), (21, 701)])
        );
        assert!(tree.shadow_trees.assigned_slots.is_empty());
        assert!(tree.shadow_trees.stylesheet_scopes.is_empty());
        assert!(tree.rendered_tree.is_none());
        assert_eq!(tree.top_layer[0].anchor_source, Some(10));
        assert_eq!(tree.top_layer[1].anchor_source, Some(11));
        assert_eq!(tree.top_layer[0].picker.unwrap().active, Some(20));
        assert_eq!(tree.top_layer[1].picker.unwrap().active, Some(21));
        assert_eq!(tree.top_layer[2].anchor_source, None);
        let nested_contexts = BTreeMap::from([(10, 10), (20, 20)]);
        let nested_owners = BTreeMap::from([((10, 3), 10), ((20, 6), 20)]);
        assert_eq!(
            instance_reference(20, 3, &nested_contexts, &nested_owners, &instances),
            Some(10)
        );
        assert_eq!(
            instance_reference(20, 20, &nested_contexts, &nested_owners, &instances),
            None
        );
    }

    #[test]
    fn captured_animation_sample_matches_owner_and_survives_later_layouts() {
        let mut input = input(
            "",
            "use { animation: grow 2s linear both } @keyframes grow { from { font-size:10px } to { font-size:30px } }",
        );
        input.nodes[2].tag = "text".into();
        input.nodes[2].text = "A".into();
        let limits = LayoutLimits::default();
        let mut engine = StyleEngine::default();
        assert!(engine.instance_style_sample().is_none());
        engine.compute_at(&input, &limits, 0.).unwrap();
        let first = engine.compute_at(&input, &limits, 0.5).unwrap();
        let sample = engine.instance_style_sample().unwrap();
        let requests = [StyleInstance {
            id: 10,
            source: 3,
            parent: StyleParent::Source(4),
        }];
        let projection =
            compute_with_style_sample(&input, &requests, &limits, false, Some(&sample)).unwrap();
        assert_eq!(
            projection.styles[&10]["font-size"],
            first.computed_styles[&4]["font-size"]
        );
        let second = engine.compute_at(&input, &limits, 1.5).unwrap();
        assert_ne!(
            first.computed_styles[&4]["font-size"],
            second.computed_styles[&4]["font-size"]
        );
        let current = engine.instance_style_sample().unwrap();
        let later =
            compute_with_style_sample(&input, &requests, &limits, false, Some(&current)).unwrap();
        assert_eq!(
            later.styles[&10]["font-size"],
            second.computed_styles[&4]["font-size"]
        );
        let replay =
            compute_with_style_sample(&input, &requests, &limits, false, Some(&sample)).unwrap();
        assert_eq!(replay.styles, projection.styles);
        assert_eq!(replay.text_fonts.demands, projection.text_fonts.demands);
    }

    #[test]
    fn instance_cascade_uses_selected_media() {
        let input = input("", "#shape{fill:red}@media print{#shape{fill:blue}}");
        let requests = [StyleInstance {
            id: 10,
            source: 3,
            parent: StyleParent::Source(4),
        }];
        let limits = LayoutLimits::default();
        let screen = compute_with_pseudos_for_media(&input, &requests, &limits, false).unwrap();
        let print = compute_with_pseudos_for_media(&input, &requests, &limits, true).unwrap();
        assert_eq!(screen.styles[&10]["fill"], "rgb(255, 0, 0)");
        assert_eq!(print.styles[&10]["fill"], "rgb(0, 0, 255)");
    }

    #[test]
    fn native_instance_cascade_preserves_matches_and_recomputes_inheritance() {
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 12,
                source: 3,
                parent: StyleParent::Instance(10),
            },
        ];
        for (inline, sheet, colors) in [
            ("", "", ["rgb(255, 0, 0)", "rgb(0, 0, 255)"]),
            ("fill:inherit", "", ["rgb(255, 0, 0)", "rgb(0, 0, 255)"]),
            ("fill:var(--tone)", "", ["rgb(255, 0, 0)", "rgb(0, 0, 255)"]),
            ("fill:black", "", ["rgb(0, 0, 0)"; 2]),
            (
                "fill:black",
                "defs > #shape {fill:lime!important}",
                ["rgb(0, 255, 0)"; 2],
            ),
        ] {
            let result =
                compute(&input(inline, sheet), &requests, &LayoutLimits::default()).unwrap();
            assert_eq!(result[&10]["fill"], colors[0], "{inline}; {sheet}");
            assert_eq!(result[&11]["fill"], colors[1], "{inline}; {sheet}");
            assert_eq!(result[&12]["fill"], colors[0], "nested: {inline}; {sheet}");
            assert_eq!(result[&10]["--tone"], "red");
            assert_eq!(result[&11]["--tone"], "blue");
        }
    }

    #[test]
    fn instance_pseudos_match_hidden_sources_and_inherit_each_instance_values() {
        let mut input = input(
            "display:block",
            "defs > #shape::before { content:var(--label); display:block; color:var(--tone); width:2em } defs > #shape::after { content:'tail'; color:black!important }",
        );
        input.nodes[1]
            .attributes
            .insert("style".into(), "display:none".into());
        input.nodes[2].tag = "div".into();
        input.nodes[2].namespace = "http://www.w3.org/1999/xhtml".into();
        input.nodes[3].attributes.insert(
            "style".into(),
            "color:red;--tone:red;--label:'R';font-size:12px".into(),
        );
        input.nodes[4].attributes.insert(
            "style".into(),
            "color:blue;--tone:blue;--label:'B';font-size:24px".into(),
        );
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
        ];
        let result = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (id, color, label, width) in [
            (10, "rgb(255, 0, 0)", "\"R\"", "24px"),
            (11, "rgb(0, 0, 255)", "\"B\"", "48px"),
        ] {
            let pseudos = &result.pseudo_styles[&id];
            assert_eq!(pseudos["before"]["color"], color);
            assert_eq!(pseudos["before"]["content"], label);
            assert_eq!(pseudos["before"]["width"], width);
            assert_eq!(pseudos["after"]["color"], "rgb(0, 0, 0)");
            assert_eq!(pseudos["after"]["content"], "\"tail\"");
            assert!(!pseudos.contains_key("picker"));
            assert!(!pseudos.contains_key("backdrop"));
        }
    }

    #[test]
    fn first_line_and_letter_metadata_use_instance_styles_and_source_origins() {
        let mut input = input(
            "display:block",
            "#shape::first-line {color:var(--tone)} #shape::first-letter {font-weight:bold}",
        );
        input.nodes[2].tag = "div".into();
        input.nodes[2].namespace = "http://www.w3.org/1999/xhtml".into();
        input.nodes[2].text = "First line".into();
        input.nodes[2].children.push(6);
        input.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":6,"parent":3,"tag":"span","namespace":"http://www.w3.org/1999/xhtml",
                "attributes":{"style":"color:black"},"text":" explicit child"
            }))
            .unwrap(),
        );
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 20,
                source: 6,
                parent: StyleParent::Instance(10),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 21,
                source: 6,
                parent: StyleParent::Instance(11),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (id, color) in [(10, "rgb(255, 0, 0)"), (11, "rgb(0, 0, 255)")] {
            let variants = text_pseudos::read(&output.styles[&id]);
            assert_eq!(variants.line.as_ref().unwrap()["color"], color);
            assert_eq!(variants.letter.as_ref().unwrap()["font-weight"], "700");
            assert_eq!(variants.line_origin, Some(3));
            assert_eq!(variants.letter_origin, Some(3));
            assert!(variants.active);
        }
        for id in [20, 21] {
            let variants = text_pseudos::read(&output.styles[&id]);
            assert_eq!(
                variants.line.as_ref().unwrap()["color"],
                "rgb(0, 0, 0)",
                "explicit child color must win over inherited instance line color"
            );
            assert_eq!(variants.line_origin, Some(3));
        }
    }

    #[test]
    fn permission_instances_constrain_icons_and_hide_author_children() {
        let mut input = input(
            "display:block;font-size:inherit;transform:translateX(20px);letter-spacing:100px",
            "#shape::permission-icon {height:100px;fill:currentColor}",
        );
        input.nodes[2].tag = "geolocation".into();
        input.nodes[2].namespace = "http://www.w3.org/1999/xhtml".into();
        input.nodes[2].children.push(6);
        input.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":6,"parent":3,"tag":"span","namespace":"http://www.w3.org/1999/xhtml",
                "attributes":{"style":"display:block"},"text":"author child"
            }))
            .unwrap(),
        );
        input.nodes[3]
            .attributes
            .insert("style".into(), "color:red;font-size:12px".into());
        input.nodes[4]
            .attributes
            .insert("style".into(), "color:blue;font-size:24px".into());
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 20,
                source: 6,
                parent: StyleParent::Instance(10),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 21,
                source: 6,
                parent: StyleParent::Instance(11),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (id, font) in [(10, 12.0), (11, 24.0)] {
            let style = &output.styles[&id];
            assert_eq!(style["transform"], "none");
            assert_eq!(style["user-select"], "none");
            let spacing = style["letter-spacing"]
                .trim_end_matches("px")
                .parse::<f64>()
                .unwrap();
            assert!((spacing - font * 0.2).abs() < 0.000001);
            let icon = &output.pseudo_styles[&id]["permission-icon"];
            assert_eq!(icon["height"], format!("{}px", font * 1.5));
            assert_eq!(icon["width"], icon["height"]);
            assert_eq!(icon["opacity"], "1");
        }
        for id in [20, 21] {
            assert_eq!(output.styles[&id]["display"], "none");
        }
    }

    #[test]
    fn anchor_fallbacks_preserve_instance_inheritance_and_source_parent_identity() {
        let mut input = input(
            "display:block;position:relative",
            "#target{position:absolute;position-anchor:--a;position-try-fallbacks:--shift} @position-try --shift{top:var(--offset);width:2em}",
        );
        input.nodes[2].tag = "div".into();
        input.nodes[2].namespace = "http://www.w3.org/1999/xhtml".into();
        input.nodes[2].children.push(6);
        input.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":6,"parent":3,"tag":"div","namespace":"http://www.w3.org/1999/xhtml",
                "attributes":{"id":"target"}
            }))
            .unwrap(),
        );
        input.nodes[3].attributes.insert(
            "style".into(),
            "color:red;font-size:12px;--offset:3px".into(),
        );
        input.nodes[4].attributes.insert(
            "style".into(),
            "color:blue;font-size:24px;--offset:7px".into(),
        );
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 20,
                source: 6,
                parent: StyleParent::Instance(10),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 21,
                source: 6,
                parent: StyleParent::Instance(11),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (id, top, width, color) in [
            (20, "3px", "24px", "rgb(255, 0, 0)"),
            (21, "7px", "48px", "rgb(0, 0, 255)"),
        ] {
            let style = &output.styles[&id];
            assert_eq!(style["-oryn-anchor-parent"], "3");
            let candidates: Vec<BTreeMap<String, String>> =
                serde_json::from_str(&style["-oryn-position-tries"]).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0]["top"], top);
            assert_eq!(candidates[0]["width"], width);
            assert_eq!(candidates[0]["color"], color);
            assert_eq!(candidates[0]["-oryn-anchor-parent"], "3");
            let scope = if id == 20 { "10" } else { "11" };
            assert_eq!(style[anchor_positioning::INSTANCE_CONTEXT], scope);
            assert_eq!(candidates[0][anchor_positioning::INSTANCE_CONTEXT], scope);
        }
    }

    #[test]
    fn nested_use_anchor_contexts_preserve_each_clone_fallback_inheritance() {
        let mut input = input(
            "display:block",
            "#target{position:absolute;position-anchor:--a;position-try-fallbacks:--shift} @position-try --shift{top:var(--offset)}",
        );
        input.nodes[1].children.push(7);
        input.nodes[2].tag = "g".into();
        input.nodes[2].children.push(6);
        input.nodes[3]
            .attributes
            .insert("style".into(), "--offset:3px".into());
        input.nodes[4]
            .attributes
            .insert("style".into(), "--offset:7px".into());
        for value in [
            serde_json::json!({"id":6,"parent":3,"tag":"use","namespace":"http://www.w3.org/2000/svg","attributes":{"href":"#inner"}}),
            serde_json::json!({"id":7,"parent":2,"tag":"foreignObject","namespace":"http://www.w3.org/2000/svg","attributes":{"id":"inner","width":"2","height":"2","style":"display:block;position:relative"},"children":[8]}),
            serde_json::json!({"id":8,"parent":7,"tag":"div","namespace":"http://www.w3.org/1999/xhtml","attributes":{"id":"target"}}),
        ] {
            input.nodes.push(serde_json::from_value(value).unwrap());
        }
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 20,
                source: 6,
                parent: StyleParent::Instance(10),
            },
            StyleInstance {
                id: 30,
                source: 7,
                parent: StyleParent::Instance(20),
            },
            StyleInstance {
                id: 40,
                source: 8,
                parent: StyleParent::Instance(30),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
            StyleInstance {
                id: 21,
                source: 6,
                parent: StyleParent::Instance(11),
            },
            StyleInstance {
                id: 31,
                source: 7,
                parent: StyleParent::Instance(21),
            },
            StyleInstance {
                id: 41,
                source: 8,
                parent: StyleParent::Instance(31),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (outer, use_id, root, child, top) in [(10, 20, 30, 40, "3px"), (11, 21, 31, 41, "7px")]
        {
            let outer_scope = outer.to_string();
            let nested_scope = root.to_string();
            assert_eq!(
                output.styles[&outer][anchor_positioning::INSTANCE_CONTEXT],
                outer_scope
            );
            assert_eq!(
                output.styles[&use_id][anchor_positioning::INSTANCE_CONTEXT],
                outer_scope
            );
            assert_eq!(
                output.styles[&root][anchor_positioning::INSTANCE_CONTEXT],
                nested_scope
            );
            let style = &output.styles[&child];
            assert_eq!(style[anchor_positioning::INSTANCE_CONTEXT], nested_scope);
            assert_eq!(style["-oryn-anchor-parent"], "7");
            let candidates: Vec<BTreeMap<String, String>> =
                serde_json::from_str(&style["-oryn-position-tries"]).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0][anchor_positioning::INSTANCE_CONTEXT],
                nested_scope
            );
            assert_eq!(candidates[0]["top"], top);
            assert_eq!(candidates[0]["-oryn-anchor-parent"], "7");
        }
    }

    #[test]
    fn picker_anchor_fallbacks_use_each_instance_native_pseudo() {
        let mut input = input(
            "appearance:base-select",
            "#shape::picker(select){appearance:base-select;position:fixed;position-try-fallbacks:--shift} @position-try --shift{top:var(--offset);width:2em}",
        );
        input.nodes[2].tag = "select".into();
        input.nodes[2].namespace = "http://www.w3.org/1999/xhtml".into();
        input.nodes[3]
            .attributes
            .insert("style".into(), "font-size:12px;--offset:3px".into());
        input.nodes[4]
            .attributes
            .insert("style".into(), "font-size:24px;--offset:7px".into());
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for (id, top, width) in [(10, "3px", "24px"), (11, "7px", "48px")] {
            let picker = &output.pseudo_styles[&id]["picker"];
            let candidates: Vec<BTreeMap<String, String>> =
                serde_json::from_str(&picker["-oryn-position-tries"]).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0]["top"], top);
            assert_eq!(candidates[0]["width"], width);
        }
    }

    #[test]
    fn svg_text_font_projection_records_transformed_demand_and_exact_faces() {
        let input: LayoutInput = serde_json::from_value(serde_json::json!({
            "nodes":[{"id":1,"parent":null,"tag":"svg","namespace":"http://www.w3.org/2000/svg","children":[2,3]},
                {"id":2,"parent":1,"tag":"text","namespace":"http://www.w3.org/2000/svg","text":"az"},
                {"id":3,"parent":1,"tag":"text","namespace":"http://www.w3.org/2000/svg","text":"hidden"}],
            "stylesheets":[],
            "viewport":{"width":200,"height":40,"device_pixel_ratio":1},"scroll":{"x":0,"y":0,"content_width":0,"content_height":0}
        })).unwrap();
        let styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("display".into(), "block".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([
                    ("font-family".into(), "InstanceFace, sans-serif".into()),
                    ("font-size".into(), "16px".into()),
                    ("text-transform".into(), "uppercase".into()),
                ])),
            ),
            (
                3,
                Arc::new(BTreeMap::from([
                    ("display".into(), "none".into()),
                    ("font-family".into(), "HiddenFace".into()),
                ])),
            ),
        ]);
        let output = collect_text_fonts(&input, &styles, &LayoutLimits::default()).unwrap();
        assert!(output.demands["instanceface"].contains(&('A' as u32)));
        assert!(output.demands["instanceface"].contains(&('Z' as u32)));
        assert!(!output.demands["instanceface"].contains(&('a' as u32)));
        assert!(!output.demands.contains_key("hiddenface"));
        assert!(!output.resources.is_empty());
        for (key, face) in &output.resources {
            assert_eq!(
                key,
                &format!(
                    "{:x}:{}",
                    Sha256::digest(&face.bytes),
                    face.collection_index
                )
            );
        }
    }

    #[test]
    fn native_instance_fonts_ignore_source_defs_but_honor_hidden_use_ancestry() {
        let mut input = input("text-transform:uppercase", "defs {display:none}");
        input.nodes[2].tag = "text".into();
        input.nodes[2].text = "az".into();
        input.nodes[3].attributes.insert(
            "style".into(),
            "font-family:VisibleInstance,sans-serif".into(),
        );
        input.nodes[4].attributes.insert(
            "style".into(),
            "display:none;font-family:HiddenInstance,sans-serif".into(),
        );
        let requests = [
            StyleInstance {
                id: 10,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        assert!(output.text_fonts.demands["visibleinstance"].contains(&('A' as u32)));
        assert!(output.text_fonts.demands["visibleinstance"].contains(&('Z' as u32)));
        assert!(!output.text_fonts.demands.contains_key("hiddeninstance"));
        assert!(!output.text_fonts.resources.is_empty());
        assert_ne!(
            output.styles[&11]["display"], "none",
            "font visibility filtering must not rewrite instance styles"
        );
    }

    #[test]
    fn styled_text_owners_resolve_to_each_instance_font_style() {
        let mut input = input("", "defs{display:none}");
        input.nodes[2].tag = "text".into();
        input.nodes[2].content = Some(vec![LayoutContent::StyledText {
            owner: 3,
            node_id: Some(90),
            text: "P".into(),
        }]);
        input.nodes[3].attributes.insert(
            "style".into(),
            "font-family:FirstInstance,sans-serif".into(),
        );
        input.nodes[4].attributes.insert(
            "style".into(),
            "font-family:SecondInstance,sans-serif".into(),
        );
        // The instance ID 3 intentionally collides with the original source ID.
        // Unqualified owner lookup would silently select the first copy's font.
        let requests = [
            StyleInstance {
                id: 3,
                source: 3,
                parent: StyleParent::Source(4),
            },
            StyleInstance {
                id: 11,
                source: 3,
                parent: StyleParent::Source(5),
            },
        ];
        let output = compute_with_pseudos(&input, &requests, &LayoutLimits::default()).unwrap();
        for family in ["firstinstance", "secondinstance"] {
            assert!(output.text_fonts.demands[family].contains(&('P' as u32)));
        }
    }

    #[test]
    fn styled_text_cannot_borrow_a_sibling_instances_owner() {
        let mut input = input("", "");
        input.nodes[2].tag = "text".into();
        input.nodes[2].content = Some(vec![LayoutContent::StyledText {
            owner: 4,
            node_id: None,
            text: "P".into(),
        }]);
        let requests = [StyleInstance {
            id: 10,
            source: 3,
            parent: StyleParent::Source(5),
        }];
        assert!(
            matches!(compute_with_pseudos(&input, &requests, &LayoutLimits::default()), Err(LayoutError::StyleTraversal(detail)) if detail.contains("styled text owner"))
        );
    }

    #[test]
    fn instance_graph_rejects_missing_parents_sources_duplicates_and_excess_size() {
        let input = input("", "");
        let valid = StyleInstance {
            id: 10,
            source: 3,
            parent: StyleParent::Source(4),
        };
        for requests in [
            vec![StyleInstance {
                source: 99,
                ..valid
            }],
            vec![StyleInstance {
                parent: StyleParent::Source(99),
                ..valid
            }],
            vec![StyleInstance {
                parent: StyleParent::Instance(10),
                ..valid
            }],
            vec![
                StyleInstance {
                    parent: StyleParent::Instance(11),
                    ..valid
                },
                StyleInstance {
                    id: 11,
                    parent: StyleParent::Instance(10),
                    ..valid
                },
            ],
            vec![valid, valid],
        ] {
            assert!(matches!(
                compute(&input, &requests, &LayoutLimits::default()),
                Err(LayoutError::StyleTraversal(_))
            ));
        }
        let limits = LayoutLimits {
            max_nodes: input.nodes.len(),
            ..Default::default()
        };
        assert!(matches!(
            compute(&input, &[valid], &limits),
            Err(LayoutError::NodeLimit { .. })
        ));
    }
}
