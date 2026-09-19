//! Native select picker formatting is independent of the originating button.
//! Option IDs remain native; only their paint parent changes to the private box.
use super::*;
use oryn_common::v4::{PaintNodeSnapshot, TopLayerKind};

pub(super) fn append(
    input: &LayoutInput,
    limits: &LayoutLimits,
    output: &mut LayoutOutput,
) -> Result<(), LayoutError> {
    let sources = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut used = sources
        .keys()
        .copied()
        .chain(output.geometry.keys().copied())
        .collect::<BTreeSet<_>>();
    for (order, entry) in input.top_layer.iter().enumerate() {
        if entry.kind != TopLayerKind::SelectPicker {
            continue;
        }
        if used.len() >= limits.max_nodes {
            return Err(LayoutError::StyleTraversal(
                "picker exceeds native layout node budget".into(),
            ));
        }
        let Some(owner) = sources.get(&entry.node_id).copied() else {
            continue;
        };
        let Some(anchor) = output
            .geometry
            .get(&owner.id)
            .and_then(|geometry| geometry.box_model.as_ref())
            .map(|model| model.border_box)
        else {
            continue;
        };
        let mode = output
            .computed_styles
            .get(&owner.id)
            .map(|style| style_value(style, "writing-mode", "horizontal-tb"))
            .unwrap_or("horizontal-tb");
        let vertical = matches!(
            mode,
            "vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr"
        );
        let reverse = matches!(mode, "vertical-rl" | "sideways-rl");
        let mut id = u64::MAX;
        while used.contains(&id) {
            id = id
                .checked_sub(1)
                .ok_or_else(|| LayoutError::StyleTraversal("picker identity exhausted".into()))?;
        }
        used.insert(id);
        let roots = owner
            .children
            .iter()
            .copied()
            .filter(|id| {
                sources
                    .get(id)
                    .is_some_and(|node| node.tag != "button" && !node.tag.starts_with("::"))
            })
            .collect::<Vec<_>>();
        let mut included = BTreeSet::new();
        let mut pending = roots.clone();
        while let Some(id) = pending.pop() {
            if included.insert(id) {
                if let Some(node) = sources.get(&id) {
                    pending.extend(node.children.iter().copied());
                }
            }
        }
        let mut picker = input.clone();
        picker.nodes = input
            .nodes
            .iter()
            .filter(|node| included.contains(&node.id))
            .cloned()
            .collect();
        picker.top_layer.clear();
        picker.rendered_tree = None;
        picker.scroll = ScrollState::default();
        for node in &mut picker.nodes {
            if roots.contains(&node.id) {
                node.parent = Some(id);
            }
        }
        let mut wrapper = owner.clone();
        wrapper.id = id;
        wrapper.parent = None;
        wrapper.tag = "oryn-native-select-picker".into();
        wrapper.children = roots.clone();
        wrapper.text.clear();
        wrapper.content = Some(roots.iter().copied().map(LayoutContent::Element).collect());
        wrapper.checked = None;
        wrapper.disabled = None;
        wrapper.validity = None;
        picker.nodes.insert(0, wrapper);
        if let Some(state) = entry.picker {
            if !state.scroll.0.is_finite() || !state.scroll.1.is_finite() {
                return Err(LayoutError::StyleTraversal(
                    "non-finite native picker scroll".into(),
                ));
            }
            picker.element_scrolls.insert(id, state.scroll);
        }

        let mut styles = output.computed_styles.clone();
        let pseudo = output
            .pseudo_styles
            .get(&owner.id)
            .and_then(|pseudos| pseudos.get("picker"));
        let mut style = pseudo.map(|style| (**style).clone()).unwrap_or_else(|| {
            output
                .computed_styles
                .get(&owner.id)
                .map(|style| (**style).clone())
                .unwrap_or_default()
        });
        if pseudo.is_some() && style_value(&style, "display", "inline") == "none" {
            continue;
        }
        let area_positioned = style_value(&style, "position-area", "none") != "none";
        let automatic_x = !area_positioned
            && style_value(&style, "left", "auto") == "auto"
            && style_value(&style, "right", "auto") == "auto";
        let automatic_y = !area_positioned
            && style_value(&style, "top", "auto") == "auto"
            && style_value(&style, "bottom", "auto") == "auto";
        let display = match style_value(&style, "display", "inline") {
            "inline-flex" => "flex",
            "inline-grid" => "grid",
            "inline-table" => "table",
            "inline" | "inline-block" | "contents" => "block",
            value => value,
        }
        .to_owned();
        style.insert("display".into(), display);
        if !matches!(
            style_value(&style, "position", "static"),
            "fixed" | "absolute"
        ) {
            style.insert("position".into(), "absolute".into());
        }
        if automatic_x {
            style.insert(
                "left".into(),
                format!("{}px", if vertical { anchor.right() } else { anchor.x }),
            );
        }
        if automatic_y {
            style.insert(
                "top".into(),
                format!("{}px", if vertical { anchor.y } else { anchor.bottom() }),
            );
        }
        let min_inline = if vertical { "min-height" } else { "min-width" };
        if style_value(&style, min_inline, "auto") == "auto" {
            style.insert(
                min_inline.into(),
                format!(
                    "{}px",
                    if vertical {
                        anchor.height
                    } else {
                        anchor.width
                    }
                ),
            );
        }
        if pseudo.is_none() {
            style.insert(
                (if vertical { "height" } else { "width" }).into(),
                format!(
                    "{}px",
                    if vertical {
                        anchor.height
                    } else {
                        anchor.width
                    }
                ),
            );
            style.insert("background-color".into(), "white".into());
            style.insert("color".into(), "black".into());
            style.insert("overflow-y".into(), "auto".into());
        }
        if style_value(&style, "max-height", "none") == "none" {
            style.insert(
                "max-height".into(),
                format!("{}px", input.viewport.height.max(0.)),
            );
        }
        if style_value(&style, "max-width", "none") == "none" {
            style.insert(
                "max-width".into(),
                format!("{}px", input.viewport.width.max(0.)),
            );
        }
        if let Some(metadata) = output
            .computed_styles
            .get(&owner.id)
            .and_then(|style| style.get(text_pseudos::PICKER_KEY))
        {
            style.insert(text_pseudos::KEY.into(), metadata.clone());
        }
        for node in &picker.nodes {
            if let Some(style) = styles.get_mut(&node.id) {
                let style = Arc::make_mut(style);
                style.remove(text_pseudos::KEY);
                if let Some(metadata) = style.get(text_pseudos::PICKER_KEY).cloned() {
                    style.insert(text_pseudos::KEY.into(), metadata);
                }
            }
        }
        style.insert("-oryn-anchor-parent".into(), owner.id.to_string());
        styles.insert(id, Arc::new(style));
        let mut anchor_context = None;
        let position_visibility =
            style_value(&styles[&id], "position-visibility", "always").to_owned();
        if anchor_positioning::needed(&styles[&id]) {
            // Anchor selection uses the original document/scopes and current
            // transformed geometry. The isolated formatting tree intentionally
            // contains only picker content, so transport concrete candidates.
            let mut context = input.clone();
            context.nodes.push(picker.nodes[0].clone());
            context.top_layer.push(oryn_common::v4::TopLayerEntry {
                node_id: id,
                kind: TopLayerKind::Popover,
                anchor_source: Some(owner.id),
                picker: None,
            });
            if let Some(scope) = input.shadow_trees.node_scopes.get(&owner.id) {
                context.shadow_trees.node_scopes.insert(id, *scope);
            }
            let candidates = anchor_positioning::candidates(
                id,
                &context,
                &styles,
                &output.paint_projections,
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: input.viewport.width,
                    height: input.viewport.height,
                },
            );
            let mut base = candidates[0].as_ref().clone();
            let alternatives = candidates
                .iter()
                .skip(1)
                .map(|candidate| {
                    let mut candidate = candidate.as_ref().clone();
                    candidate.remove("-oryn-position-tries");
                    candidate.insert("position-visibility".into(), "always".into());
                    candidate
                })
                .collect::<Vec<_>>();
            let serialized = serde_json::to_string(&alternatives)
                .map_err(|error| LayoutError::StyleTraversal(error.to_string()))?;
            if serialized.len() > 4 * 1024 * 1024 {
                return Err(LayoutError::StyleTraversal(
                    "resolved picker fallback exceeds native bound".into(),
                ));
            }
            base.insert("-oryn-position-tries".into(), serialized);
            base.insert("position-visibility".into(), "always".into());
            styles.insert(id, Arc::new(base));
            anchor_context = Some(context);
        }

        // Materialize only the picker origin's nested pseudos. Option pseudos
        // were already materialized by the outer document pass.
        let nested = output
            .pseudo_styles
            .get(&owner.id)
            .map(|pseudos| {
                pseudos
                    .iter()
                    .filter_map(|(name, style)| {
                        name.strip_prefix("picker::")
                            .map(|name| (name.to_owned(), style.clone()))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let mut generated =
            generated::materialize(&picker, styles, &BTreeMap::from([(id, nested)]))?;
        if used
            .iter()
            .copied()
            .chain(generated.input.nodes.iter().map(|node| node.id))
            .collect::<BTreeSet<_>>()
            .len()
            > limits.max_nodes
        {
            return Err(LayoutError::StyleTraversal(
                "picker generated content exceeds native node budget".into(),
            ));
        }
        let mut styles = generated.styles.clone();
        let mut picker = generated.input.clone();
        let mut layout = compute_with_styles(&picker, limits, styles.clone())?;
        if let Some(rect) = layout
            .geometry
            .get(&id)
            .and_then(|geometry| geometry.box_model.as_ref())
            .map(|model| model.border_box)
        {
            let rtl = output
                .computed_styles
                .get(&owner.id)
                .is_some_and(|style| style_value(style, "direction", "ltr") == "rtl");
            let top = if !automatic_y {
                rect.y
            } else if vertical {
                (if rtl {
                    anchor.bottom() - rect.height
                } else {
                    anchor.y
                })
                .min((input.viewport.height - rect.height).max(0.))
                .max(0.)
            } else if rect.bottom() > input.viewport.height && anchor.y >= rect.height {
                anchor.y - rect.height
            } else {
                anchor
                    .bottom()
                    .min((input.viewport.height - rect.height).max(0.))
            };
            let left = if !automatic_x {
                rect.x
            } else if vertical {
                let preferred = if reverse {
                    anchor.x - rect.width
                } else {
                    anchor.right()
                };
                let alternate = if reverse {
                    anchor.right()
                } else {
                    anchor.x - rect.width
                };
                let chosen = if preferred < 0. || preferred + rect.width > input.viewport.width {
                    alternate
                } else {
                    preferred
                };
                chosen
                    .min((input.viewport.width - rect.width).max(0.))
                    .max(0.)
            } else {
                (if rtl {
                    anchor.right() - rect.width
                } else {
                    anchor.x
                })
                .min((input.viewport.width - rect.width).max(0.))
                .max(0.)
            };
            if (top - rect.y).abs() > 0.01 || (left - rect.x).abs() > 0.01 {
                let style = Arc::make_mut(styles.get_mut(&id).unwrap());
                if automatic_y {
                    style.insert("top".into(), format!("{top}px"));
                }
                if automatic_x {
                    style.insert("left".into(), format!("{left}px"));
                }
                layout = compute_with_styles(&picker, limits, styles.clone())?;
            }
        }
        if let Some(state) = entry.picker.filter(|state| state.reveal) {
            if let Some(active) = state.active.filter(|active| included.contains(active)) {
                if let (Some(projection), Some(target)) = (
                    layout.paint_projections.get(&id),
                    layout
                        .geometry
                        .get(&active)
                        .and_then(|geometry| geometry.box_model.as_ref())
                        .map(|model| model.border_box),
                ) {
                    if let Some(scroll) = &projection.scroll {
                        let mut bounds = CssRect {
                            x: f64::INFINITY,
                            y: f64::INFINITY,
                            width: 0.,
                            height: 0.,
                        };
                        let mut right = f64::NEG_INFINITY;
                        let mut bottom = f64::NEG_INFINITY;
                        for (x, y) in [
                            (target.x, target.y),
                            (target.right(), target.y),
                            (target.x, target.bottom()),
                            (target.right(), target.bottom()),
                        ] {
                            if let Some((x, y)) = projection.viewport_to_local(x, y) {
                                bounds.x = bounds.x.min(x);
                                bounds.y = bounds.y.min(y);
                                right = right.max(x);
                                bottom = bottom.max(y);
                            }
                        }
                        bounds.width = right - bounds.x;
                        bounds.height = bottom - bounds.y;
                        if bounds.x.is_finite() && bounds.y.is_finite() {
                            let mut port = scroll.scrollport;
                            if let Some(style) = styles.get(&active) {
                                let font = length(
                                    style_value(style, "font-size", "16px"),
                                    16.,
                                    16.,
                                    input.viewport,
                                );
                                let margin = edge(style, "scroll-margin", 0., font, input.viewport);
                                bounds.x -= margin.left;
                                bounds.y -= margin.top;
                                bounds.width += margin.left + margin.right;
                                bounds.height += margin.top + margin.bottom;
                            }
                            if let Some(style) = styles.get(&id) {
                                let font = length(
                                    style_value(style, "font-size", "16px"),
                                    16.,
                                    16.,
                                    input.viewport,
                                );
                                let used = |property: &str, basis| {
                                    length(
                                        style_value(style, property, "0px"),
                                        basis,
                                        font,
                                        input.viewport,
                                    )
                                    .max(0.)
                                };
                                let left = used("scroll-padding-left", port.width);
                                let right = used("scroll-padding-right", port.width);
                                let top = used("scroll-padding-top", port.height);
                                let bottom = used("scroll-padding-bottom", port.height);
                                port.x += left;
                                port.y += top;
                                port.width = (port.width - left - right).max(0.);
                                port.height = (port.height - top - bottom).max(0.);
                            }

                            let nearest = |start: f64, end: f64, lo: f64, hi: f64| {
                                if (start >= lo && end <= hi) || (start < lo && end > hi) {
                                    0.
                                } else if start < lo {
                                    start - lo
                                } else {
                                    end - hi
                                }
                            };
                            let next = (
                                scroll.offset_x
                                    + nearest(bounds.x, bounds.right(), port.x, port.right()),
                                scroll.offset_y
                                    + nearest(bounds.y, bounds.bottom(), port.y, port.bottom()),
                            );
                            if next != (scroll.offset_x, scroll.offset_y) {
                                picker.element_scrolls.insert(id, next);
                                layout = compute_with_styles(&picker, limits, styles.clone())?;
                            }
                        }
                    }
                }
            }
        }
        if let Some(mut context) = anchor_context {
            if position_visibility != "always" {
                // Conditional visibility also needs the outer anchor's clipped
                // projection; an isolated subtree cannot answer this question.
                let mut combined_styles = output.computed_styles.clone();
                combined_styles.extend(layout.computed_styles.clone());
                Arc::make_mut(combined_styles.get_mut(&id).unwrap())
                    .insert("position-visibility".into(), position_visibility);
                let authored = combined_styles.clone();
                let mut geometry = output.geometry.clone();
                geometry.extend(layout.geometry.clone());
                let mut projections = output.paint_projections.clone();
                projections.extend(layout.paint_projections.clone());
                let private_ids = picker
                    .nodes
                    .iter()
                    .map(|node| node.id)
                    .collect::<BTreeSet<_>>();
                context.nodes.retain(|node| !private_ids.contains(&node.id));
                context.nodes.extend(picker.nodes.iter().cloned());
                anchor_positioning::visibility(
                    &context,
                    &authored,
                    &mut combined_styles,
                    &mut geometry,
                    &projections,
                    EdgeSizes::default(),
                );
                for (node, value) in &mut layout.geometry {
                    if let Some(updated) = geometry.remove(node) {
                        *value = updated;
                    }
                }
                for (node, value) in &mut layout.computed_styles {
                    if let Some(updated) = combined_styles.remove(node) {
                        *value = updated;
                    }
                }
            }
        }
        generated.styles = styles;
        generated::finish(&generated, &mut layout);
        used.extend(layout.geometry.keys().copied());
        let Some(geometry) = layout.geometry.get(&id).cloned() else {
            continue;
        };
        output.generated_nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: output.control_states.get(&owner.id).cloned(),
            backdrop_style: None,
            top_layer: Some(order),
            node_id: id,
            parent_id: Some(owner.id),
            tag: "::picker(select)".into(),
            geometry,
            projection: layout.paint_projections.get(&id).cloned(),
            style: layout
                .computed_styles
                .get(&id)
                .map(|style| (**style).clone())
                .unwrap_or_default(),
            text: Vec::new(),
            image_resource: None,
        });
        for root in roots {
            output.paint_parent_overrides.insert(root, Some(id));
        }
        output.geometry.extend(layout.geometry);
        output.paint_projections.extend(layout.paint_projections);
        output.offset_dimensions.extend(layout.offset_dimensions);
        output.text_fragments.extend(layout.text_fragments);
        output.text_clusters.extend(layout.text_clusters);
        output.caret_stops.extend(layout.caret_stops);
        output.resolved_values.extend(layout.resolved_values);
        for (node, style) in layout.computed_styles {
            if included.contains(&node)
                || node == id
                || generated.input.nodes.iter().any(|entry| entry.id == node)
            {
                output.computed_styles.insert(node, style);
            }
        }
        for (family, characters) in layout.font_demands {
            output
                .font_demands
                .entry(family)
                .or_default()
                .extend(characters);
        }
        output.resolved_text.extend(layout.resolved_text);
        output.css_image_requests.extend(layout.css_image_requests);
        output
            .generated_image_urls
            .extend(layout.generated_image_urls);
        output.generated_nodes.extend(layout.generated_nodes);
        // These entries belong to the private picker, not the select button's
        // accessible name. Their paint/text nodes remain available to rendering.
        output
            .paint_parent_overrides
            .extend(layout.paint_parent_overrides);
        output.control_states.extend(layout.control_states);
        output.paint_fonts.extend(layout.paint_fonts);
        output.loaded_fonts.extend(layout.loaded_fonts);
        output.diagnostics.extend(layout.diagnostics);
        if output
            .paint_fonts
            .values()
            .fold(0usize, |sum, font| sum.saturating_add(font.bytes.len()))
            > limits.max_page_font_bytes
        {
            return Err(LayoutError::StyleTraversal(
                "picker exceeds native font budget".into(),
            ));
        }
    }
    Ok(())
}
