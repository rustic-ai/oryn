//! Break tokens retain source identity while each variable fragmentainer formats
//! its remaining content anew. Selector matching uses the original document;
//! hiding consumed boxes here cannot change :nth-child or sibling selectors.
use super::*;
#[derive(Default)]
struct BreakToken {
    finished: BTreeSet<u64>,
    started: BTreeSet<u64>,
    remaining_height: BTreeMap<u64, f64>,
    next_break: Option<String>,
    consumed_rows: BTreeSet<u64>,
}

/// Consumed rows remain structural carriers for rowspan cells, but must not
/// repeat their authored block size through the ordinary layout box path.
fn suppress_consumed_row_block(style: &mut BTreeMap<String, String>, vertical: bool) {
    let property = if vertical { "width" } else { "height" };
    for name in [
        property.to_owned(),
        format!("min-{property}"),
        format!("max-{property}"),
    ] {
        style.insert(name, "0px".into());
    }
    for side in if vertical {
        ["left", "right"]
    } else {
        ["top", "bottom"]
    } {
        for name in [
            format!("margin-{side}"),
            format!("padding-{side}"),
            format!("border-{side}-width"),
        ] {
            style.insert(name, "0px".into());
        }
    }
}
fn text_mut(node: &mut LayoutNode, mut f: impl FnMut(u64, &mut String)) {
    if let Some(content) = &mut node.content {
        for item in content {
            match item {
                LayoutContent::Text { node_id, text } => f(*node_id, text),
                LayoutContent::StyledText {
                    owner,
                    node_id,
                    text,
                } => f(node_id.unwrap_or(*owner), text),
                LayoutContent::Element(_) => {}
            }
        }
    } else {
        f(node.id, &mut node.text);
    }
}
fn has_text(node: &LayoutNode) -> bool {
    node.content
        .as_ref()
        .map_or(!node.text.trim().is_empty(), |items| {
            items.iter().any(|item| match item {
                LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                    !text.trim().is_empty()
                }
                _ => false,
            })
        })
}
fn drop_utf16(text: &mut String, units: usize) {
    let mut consumed = 0;
    let mut bytes = 0;
    for ch in text.chars() {
        if consumed + ch.len_utf16() > units {
            break;
        }
        consumed += ch.len_utf16();
        bytes += ch.len_utf8();
    }
    text.drain(..bytes);
}
fn ancestors(id: u64, nodes: &BTreeMap<u64, &LayoutNode>) -> Vec<u64> {
    let mut ids = Vec::new();
    let mut current = Some(id);
    while let Some(id) = current {
        ids.push(id);
        current = nodes.get(&id).and_then(|node| node.parent);
    }
    ids
}
fn page_name(id: u64, nodes: &BTreeMap<u64, &LayoutNode>, styles: &ComputedStyles) -> String {
    ancestors(id, nodes)
        .into_iter()
        .find_map(|id| {
            styles.get(&id).and_then(|s| {
                let name = style_value(s, "page", "auto");
                (name != "auto").then(|| name.to_owned())
            })
        })
        .unwrap_or_default()
}
fn order(input: &LayoutInput) -> Vec<u64> {
    let nodes = input
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<BTreeMap<_, _>>();
    let mut stack = input
        .nodes
        .iter()
        .filter(|n| n.parent.is_none())
        .rev()
        .map(|n| n.id)
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    while let Some(id) = stack.pop() {
        output.push(id);
        if let Some(node) = nodes.get(&id) {
            stack.extend(node.children.iter().rev());
        }
    }
    output
}
fn repeating_group(id: u64, nodes: &BTreeMap<u64, &LayoutNode>, styles: &ComputedStyles) -> bool {
    ancestors(id, nodes).iter().any(|id| {
        styles.get(id).is_some_and(|s| {
            matches!(
                style_value(s, "display", "block"),
                "table-header-group" | "table-footer-group"
            )
        })
    })
}
fn inactive(
    id: u64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    token: &BreakToken,
) -> bool {
    repeating_group(id, nodes, styles)
        || ancestors(id, nodes).iter().any(|id| {
            token.finished.contains(id)
                || styles.get(id).is_some_and(|style| {
                    matches!(
                        style_value(style, "display", "block"),
                        "none" | "table-column" | "table-column-group"
                    ) || style_value(style, "position", "static") == "fixed"
                })
        })
}
fn first_pending(input: &LayoutInput, styles: &ComputedStyles, token: &BreakToken) -> Option<u64> {
    let nodes = input
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<BTreeMap<_, _>>();
    order(input).into_iter().find(|id| {
        let node = nodes[id];
        !inactive(*id, &nodes, styles, token) && (has_text(node) || node.children.is_empty())
    })
}
pub(super) fn compute(
    input: &LayoutInput,
    limits: &LayoutLimits,
    options: &PrintOptions,
    base: PrintDimensions,
    rules: &[page_rules::Rule],
    styles: ComputedStyles,
    whitespace: BTreeSet<u64>,
    scale: f64,
) -> Result<PrintLayoutOutput, PrintError> {
    let mut remaining = input.clone();
    // Give legacy element-only text a stable native source identity so the
    // same UTF16 token mechanism applies to legacy and DOM-backed snapshots.
    for node in &mut remaining.nodes {
        if node.content.is_none() && !node.text.is_empty() {
            let mut content = vec![LayoutContent::Text {
                node_id: node.id,
                text: node.text.clone(),
            }];
            content.extend(node.children.iter().copied().map(LayoutContent::Element));
            node.content = Some(content);
        }
    }
    let spans = tables::rowspan_coverage(input, &styles);
    let mut token = BreakToken::default();
    let mut pages = Vec::new();
    let mut names = Vec::new();
    let mut dimensions = Vec::new();
    let mut blanks = Vec::new();
    let mut table_width_cache = None::<((u64, u64), BTreeMap<u64, f64>)>;
    let mut document = None::<LayoutOutput>;
    let mut total_fragments = 0usize;
    loop {
        let pending = first_pending(&remaining, &styles, &token);
        if pending.is_none() && !pages.is_empty() {
            break;
        }
        if pages.len() >= MAX_PRINT_PAGES {
            return Err(PrintError::PageLimit);
        }
        let nodes = remaining
            .nodes
            .iter()
            .map(|n| (n.id, n))
            .collect::<BTreeMap<_, _>>();
        let name = pending
            .map(|id| page_name(id, &nodes, &styles))
            .unwrap_or_default();
        let request = pending
            .and_then(|id| {
                ancestors(id, &nodes).into_iter().find_map(|id| {
                    if token.started.contains(&id) {
                        return None;
                    }
                    let kind = style_value(&styles[&id], "break-before", "auto");
                    matches!(
                        kind,
                        "page" | "always" | "left" | "right" | "recto" | "verso"
                    )
                    .then(|| kind.to_owned())
                })
            })
            .or_else(|| token.next_break.clone());
        if request.as_deref().is_some_and(|kind| {
            matches!(kind, "left" | "right" | "recto" | "verso")
                && (pages.len() % 2 == 0) != matches!(kind, "right" | "recto")
        }) {
            let size = page_rules::dimensions(
                base,
                &page_rules::cascade(rules, &name, pages.len(), true),
            )?;
            pages.push(PrintPageLayout {
                content_box: size.content,
                scale,
                backdrops: BTreeMap::new(),
                generated_nodes: Vec::new(),
                viewport: ViewportState {
                    width: size.width,
                    height: size.height,
                    ..input.viewport
                },
                geometry: BTreeMap::new(),
                paint_projections: BTreeMap::new(),
                text_fragments: BTreeMap::new(),
            });
            names.push(name);
            dimensions.push(size);
            blanks.push(true);
            continue;
        }
        token.next_break = None;
        let size =
            page_rules::dimensions(base, &page_rules::cascade(rules, &name, pages.len(), false))?;
        remaining.viewport.width = size.content.width / scale;
        remaining.viewport.height = size.content.height / scale;
        let page_axis = PageAxis::new(&nodes, &styles, &BTreeMap::new());
        let mut page_styles = styles.clone();
        if styles
            .values()
            .any(|s| matches!(style_value(s, "display", "block"), "table" | "inline-table"))
        {
            let key = (
                remaining.viewport.width.to_bits(),
                remaining.viewport.height.to_bits(),
            );
            if table_width_cache
                .as_ref()
                .is_none_or(|(cached, _)| *cached != key)
            {
                let mut original = input.clone();
                original.viewport = remaining.viewport;
                let measure = compute_with_styles(&original, limits, styles.clone())?;
                let widths = measure
                    .paint_projections
                    .iter()
                    .filter(|(id, _)| {
                        styles
                            .get(id)
                            .is_some_and(|s| style_value(s, "display", "block") == "table-cell")
                    })
                    .map(|(id, p)| {
                        (*id, {
                            let rect = if style_value(&styles[id], "box-sizing", "content-box")
                                == "border-box"
                            {
                                p.box_model.border_box
                            } else {
                                p.box_model.content_box
                            };
                            if tables::containing_axes(*id, &nodes, &styles).vertical {
                                rect.height
                            } else {
                                rect.width
                            }
                        })
                    })
                    .collect();
                table_width_cache = Some((key, widths));
            }
            for (id, width) in &table_width_cache.as_ref().unwrap().1 {
                if let Some(style) = page_styles.get_mut(id) {
                    let vertical = tables::containing_axes(*id, &nodes, &styles).vertical;
                    Arc::make_mut(style).insert(
                        if vertical { "height" } else { "width" }.into(),
                        format!("{width}px"),
                    );
                }
            }
        }

        for id in &token.finished {
            if let Some(style) = page_styles.get_mut(id) {
                Arc::make_mut(style).insert("display".into(), "none".into());
            }
        }
        for id in &token.started {
            if let Some(style) = page_styles.get_mut(id) {
                let style = Arc::make_mut(style);
                style.insert("break-before".into(), "auto".into());
                // A continuation has no new leading block edge under slice.
                if style_value(style, "box-decoration-break", "slice") == "slice" {
                    let side = page_axis.leading_side();
                    for property in [
                        format!("margin-{side}"),
                        format!("padding-{side}"),
                        format!("border-{side}-width"),
                    ] {
                        style.insert(property, "0px".into());
                    }
                }
            }
        }
        for (id, height) in &token.remaining_height {
            if let Some(style) = page_styles.get_mut(id) {
                let style = Arc::make_mut(style);
                style.insert(page_axis.block_property().into(), format!("{height}px"));
                style.insert(format!("min-{}", page_axis.block_property()), "0px".into());
            }
        }
        for id in &token.consumed_rows {
            if let Some(style) = page_styles.get_mut(id) {
                suppress_consumed_row_block(
                    Arc::make_mut(style),
                    tables::containing_axes(*id, &nodes, &styles).vertical,
                );
            }
        }
        if let Some(first) = pending {
            // The first remaining box starts the requested page; preserve its
            // original named-page ancestry but don't reapply a satisfied break.
            for id in ancestors(first, &nodes) {
                if let Some(style) = page_styles.get_mut(&id) {
                    Arc::make_mut(style).insert("break-before".into(), "auto".into());
                }
            }
            let leading = ancestors(first, &nodes)
                .into_iter()
                .collect::<BTreeSet<_>>();
            for id in order(&remaining) {
                if inactive(id, &nodes, &styles, &token) || leading.contains(&id) {
                    continue;
                }
                if page_name(id, &nodes, &styles) != name {
                    if let Some(style) = page_styles.get_mut(&id) {
                        Arc::make_mut(style).insert("break-before".into(), "page".into());
                    }
                    break;
                }
            }
        }
        let height = page_axis.extent(remaining.viewport);
        let probe = compute_with_styles_fragmented_with_rows(
            &remaining,
            limits,
            page_styles.clone(),
            None,
            &token.consumed_rows,
        )?;
        let reserve = footer_reserve(&remaining, &probe, height);
        let flow_height = (height - reserve).max(1.);
        let mut layout = compute_with_styles_fragmented_with_rows(
            &remaining,
            limits,
            page_styles,
            Some(flow_height),
            &token.consumed_rows,
        )?;
        repeat_footers(&remaining, &mut layout, height, flow_height);
        layout.author_whitespace = whitespace.clone();
        total_fragments = total_fragments
            .checked_add(layout.paint_projections.len())
            .ok_or(PrintError::PageLimit)?;
        if total_fragments > 1_000_000 {
            return Err(PrintError::Invalid(
                "print fragment metadata exceeds the one-million-fragment bound".into(),
            ));
        }
        let axis = PageAxis::new(&nodes, &styles, &layout.geometry);
        let mut page = project_page(&layout, &remaining, size, scale, 0.);
        if reserve > 0. {
            let rect = if axis.vertical {
                CssRect {
                    x: if axis.reverse {
                        size.content.right() - flow_height * scale
                    } else {
                        size.content.x
                    },
                    width: flow_height * scale,
                    ..size.content
                }
            } else {
                CssRect {
                    height: flow_height * scale,
                    ..size.content
                }
            };
            let clip = PaintClip {
                shape: None,
                owner: None,
                scroll_x: false,
                scroll_y: false,
                matrix: transforms::Projection::IDENTITY.matrix(),
                rect,
                clip_x: true,
                clip_y: true,
            };
            for (id, projection) in &mut page.paint_projections {
                let footer = ancestors(*id, &nodes).iter().any(|id| {
                    styles
                        .get(id)
                        .is_some_and(|s| style_value(s, "display", "block") == "table-footer-group")
                });
                if !footer {
                    projection.clips.push(clip.clone());
                    projection.text_clips.push(clip.clone());
                }
            }
        }
        let before = (
            token.finished.len() + token.consumed_rows.len(),
            remaining
                .nodes
                .iter()
                .map(|n| {
                    n.content.as_ref().map_or(n.text.len(), |c| {
                        c.iter()
                            .map(|item| match item {
                                LayoutContent::Text { text, .. }
                                | LayoutContent::StyledText { text, .. } => text.len(),
                                _ => 0,
                            })
                            .sum()
                    })
                })
                .sum::<usize>(),
            token.remaining_height.values().sum::<f64>(),
        );
        // Fragmentation consumes source in the native layout plane, before CSS transforms.
        let line_flows = layout
            .paint_projections
            .iter()
            .flat_map(|(owner, projection)| {
                projection.text.iter().filter_map(|fragment| {
                    Some(((*owner, fragment.flow_line?), fragment.flow_rect?))
                })
            })
            .collect::<BTreeMap<_, _>>();
        let consumed = layout
            .text_clusters
            .iter()
            .filter_map(|(id, clusters)| {
                let end = clusters
                    .iter()
                    .filter(|c| {
                        let flow = axis.rect(
                            line_flows
                                .get(&(c.layout_node, c.line))
                                .copied()
                                .unwrap_or(c.rect),
                        );
                        !repeating_group(c.layout_node, &nodes, &styles)
                            && flow.y >= -0.01
                            && flow.bottom() <= flow_height + 0.01
                    })
                    .map(|c| c.end)
                    .max()?;
                // A line can straddle the page edge with neither its selection
                // bottom inside this page nor its selection top on the next page.
                // Advancing to the next wholly off-page cluster would discard that
                // line's unpainted source. Retain every byte after the last fully
                // consumed cluster; the next page normalizes leading whitespace.
                Some((*id, end))
            })
            .collect::<BTreeMap<_, _>>();
        // Only source prefix tokens are removed. Each new fragment is shaped
        // afresh, so line breaking can change when the page width changes.
        for node in &mut remaining.nodes {
            text_mut(node, |id, text| {
                if let Some(end) = consumed.get(&id) {
                    drop_utf16(text, *end);
                }
            });
        }
        let nodes = remaining
            .nodes
            .iter()
            .map(|n| (n.id, n))
            .collect::<BTreeMap<_, _>>();
        for id in order(&remaining).into_iter().rev() {
            if inactive(id, &nodes, &styles, &token) {
                continue;
            }
            let Some(model) = layout.paint_projections.get(&id).map(|p| &p.box_model) else {
                continue;
            };
            let rect = axis.rect(model.border_box);
            if rect.y >= flow_height - 0.01 {
                continue;
            }
            token.started.insert(id);
            let node = nodes[&id];
            if style_value(&styles[&id],"display","block")=="table-row"&&rect.bottom()<=flow_height+0.01
                && node.children.iter().all(|child|spans.contains_key(child)||inactive(*child,&nodes,&styles,&token))
                // Keep the final covered row available while spanning content
                // still needs a fragmentainer; an all-zero carrier cannot
                // allocate height or make source progress on the next page.
                && spans.iter().all(|(cell,rows)|rows.last()!=Some(&id)||nodes.get(cell).is_none_or(|node|!has_text(node)))
            {
                token.consumed_rows.insert(id);
            }
            if spans
                .get(&id)
                .is_some_and(|rows| rows.iter().any(|row| !token.consumed_rows.contains(row)))
            {
                continue;
            }
            let children_done = node
                .children
                .iter()
                .all(|child| inactive(*child, &nodes, &styles, &token));
            if !has_text(node)
                && children_done
                && (rect.bottom() <= flow_height + 0.01
                    || style_value(&styles[&id], axis.block_property(), "auto") == "auto")
            {
                token.finished.insert(id);
                token.remaining_height.remove(&id);
                let after = style_value(&styles[&id], "break-after", "auto");
                if matches!(after, "left" | "right" | "recto" | "verso") {
                    token.next_break = Some(after.into());
                }
            } else if !has_text(node) && children_done {
                token.remaining_height.insert(
                    id,
                    (axis.rect(model.content_box).bottom() - flow_height).max(0.),
                );
            }
        }
        for (cell, rows) in &spans {
            if rows.iter().all(|row| token.consumed_rows.contains(row))
                && nodes.get(cell).is_some_and(|node| !has_text(node))
            {
                token.finished.insert(*cell);
            }
        }
        // Retain the original row/DOM parent as a zero-height continuation
        // carrier. Its spanning cells keep native identity, stacking ancestry
        // and grid occupancy while subsequent rows consume block space.
        for id in order(&remaining).into_iter().rev() {
            if !token.started.contains(&id)
                || spans
                    .get(&id)
                    .is_some_and(|rows| rows.iter().any(|row| !token.consumed_rows.contains(row)))
            {
                continue;
            }
            let node = nodes[&id];
            if !has_text(node)
                && node
                    .children
                    .iter()
                    .all(|child| inactive(*child, &nodes, &styles, &token))
                && (token.consumed_rows.contains(&id)
                    || style_value(&styles[&id], axis.block_property(), "auto") == "auto")
            {
                token.finished.insert(id);
            }
        }
        let after = (
            token.finished.len() + token.consumed_rows.len(),
            remaining
                .nodes
                .iter()
                .map(|n| {
                    n.content.as_ref().map_or(n.text.len(), |c| {
                        c.iter()
                            .map(|item| match item {
                                LayoutContent::Text { text, .. }
                                | LayoutContent::StyledText { text, .. } => text.len(),
                                _ => 0,
                            })
                            .sum()
                    })
                })
                .sum::<usize>(),
            token.remaining_height.values().sum::<f64>(),
        );
        let stalled_detail=(before==after).then(|| {
            let owners=order(&remaining).into_iter().filter(|id|!inactive(*id,&nodes,&styles,&token)).take(16).map(|id| {
                let node=nodes[&id];let rect=layout.paint_projections.get(&id).map(|p|axis.rect(p.box_model.border_box));
                let clusters=layout.text_clusters.get(&id).into_iter().flatten().take(8).map(|c|(c.start,c.end,axis.rect(c.rect))).collect::<Vec<_>>();
                (id,has_text(node),token.consumed_rows.contains(&id),rect,clusters)
            }).collect::<Vec<_>>();
            format!("print continuation made no source or block progress: page={}, flow_height={flow_height}, token={before:?}, owners={owners:?}",pages.len())
        });
        if let Some(document) = &mut document {
            document.paint_fonts.extend(layout.paint_fonts.clone());
            document.diagnostics.extend(layout.diagnostics.clone());
        } else {
            layout.computed_styles = styles.clone();
            document = Some(layout);
        }
        pages.push(page);
        names.push(name);
        dimensions.push(size);
        blanks.push(false);
        if pending.is_none() {
            break;
        }
        if let Some(detail) = stalled_detail {
            return Err(PrintError::Invalid(detail));
        }
    }
    let mut document = document.expect("at least one print page");
    let mut margin_engine = TextEngine::new(&prepare_web_fonts(input, limits).0);
    let mut next_id = u64::MAX;
    let source_ids = input.nodes.iter().map(|n| n.id).collect::<BTreeSet<_>>();
    let total = pages.len();
    for (index, page) in pages.iter_mut().enumerate() {
        page.generated_nodes = margin_nodes(
            rules,
            &names[index],
            index,
            total,
            blanks[index],
            dimensions[index],
            &mut margin_engine,
            &mut next_id,
            &source_ids,
        )?;
    }
    document.paint_fonts.extend(margin_engine.paint_fonts);
    let ranges = parse_ranges(&options.page_ranges)?;
    let pages = pages
        .into_iter()
        .enumerate()
        .filter(|(index, _)| {
            ranges.is_empty() || ranges.iter().any(|(a, b)| (*a..=*b).contains(&(index + 1)))
        })
        .map(|(_, page)| page)
        .collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(PrintError::Invalid("page ranges select no pages".into()));
    }
    Ok(PrintLayoutOutput { document, pages })
}

fn footer_reserve(input: &LayoutInput, layout: &LayoutOutput, height: f64) -> f64 {
    let nodes = input
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<BTreeMap<_, _>>();
    let axis = PageAxis::new(&nodes, &layout.computed_styles, &layout.geometry);
    input
        .nodes
        .iter()
        .filter(|node| {
            layout
                .computed_styles
                .get(&node.id)
                .is_some_and(|s| style_value(s, "display", "block") == "table-footer-group")
        })
        .filter_map(|node| {
            let table = axis.rect(
                node.parent
                    .and_then(|id| layout.paint_projections.get(&id))?
                    .box_model
                    .border_box,
            );
            let footer = axis.rect(layout.paint_projections.get(&node.id)?.box_model.border_box);
            (table.y < height && table.bottom() > height && footer.height < height)
                .then_some(footer.height)
        })
        .fold(0., f64::max)
}
fn repeat_footers(input: &LayoutInput, layout: &mut LayoutOutput, height: f64, flow_height: f64) {
    let nodes = input
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<BTreeMap<_, _>>();
    let axis = PageAxis::new(&nodes, &layout.computed_styles, &layout.geometry);
    let groups = input
        .nodes
        .iter()
        .filter(|node| {
            layout
                .computed_styles
                .get(&node.id)
                .is_some_and(|s| style_value(s, "display", "block") == "table-footer-group")
        })
        .filter_map(|node| {
            let table = axis.rect(
                node.parent
                    .and_then(|id| layout.paint_projections.get(&id))?
                    .box_model
                    .border_box,
            );
            let footer = axis.rect(layout.paint_projections.get(&node.id)?.box_model.border_box);
            (table.y < flow_height && table.bottom() > height && footer.height < height)
                .then_some((node.id, height - footer.height - footer.y))
        })
        .collect::<Vec<_>>();
    for (group, amount) in groups {
        let (dx, dy) = axis.delta(amount);
        let shift = |rect: &mut CssRect| {
            rect.x += dx;
            rect.y += dy;
        };
        for id in order(input)
            .into_iter()
            .filter(|id| ancestors(*id, &nodes).contains(&group))
        {
            if let Some(projection) = layout.paint_projections.get_mut(&id) {
                let mut translation = transforms::Projection::IDENTITY.matrix();
                translation[12] = dx;
                translation[13] = dy;
                projection.matrix = multiply(translation, projection.matrix);
                projection.text_matrix = projection.text_matrix.map(|m| multiply(translation, m));
            }
            if let Some(geometry) = layout.geometry.get_mut(&id) {
                if let Some(model) = &mut geometry.box_model {
                    shift(&mut model.border_box);
                    shift(&mut model.padding_box);
                    shift(&mut model.content_box);
                }
                for rect in &mut geometry.client_rects {
                    shift(rect);
                }
            }
            if let Some(fragments) = layout.text_fragments.get_mut(&id) {
                for fragment in fragments {
                    shift(&mut fragment.rect);
                    if let Some(flow) = &mut fragment.flow_rect {
                        shift(flow);
                    }
                    for cluster in &mut fragment.clusters {
                        shift(&mut cluster.rect);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn consumed_row_carrier_suppresses_only_its_block_box_in_both_axes() {
        for vertical in [false, true] {
            let mut style = BTreeMap::from([
                ("width".into(), "64px".into()),
                ("height".into(), "80px".into()),
                ("min-width".into(), "32px".into()),
                ("min-height".into(), "40px".into()),
                ("padding-left".into(), "4px".into()),
                ("padding-top".into(), "8px".into()),
                ("display".into(), "table-row".into()),
            ]);
            suppress_consumed_row_block(&mut style, vertical);
            let block = if vertical { "width" } else { "height" };
            assert_eq!(style[block], "0px");
            assert_eq!(style[&format!("min-{block}")], "0px");
            assert_eq!(style[&format!("max-{block}")], "0px");
            assert_eq!(
                style[if vertical { "height" } else { "width" }],
                if vertical { "80px" } else { "64px" }
            );
            assert_eq!(
                style[if vertical {
                    "padding-top"
                } else {
                    "padding-left"
                }],
                if vertical { "8px" } else { "4px" }
            );
            assert_eq!(style["display"], "table-row");
        }
    }
    use super::*;
    fn settings() -> PrintOptions {
        PrintOptions {
            page_width_cm: 2.54,
            page_height_cm: 2.54,
            margin_top_cm: 0.,
            margin_right_cm: 0.,
            margin_bottom_cm: 0.,
            margin_left_cm: 0.,
            shrink_to_fit: false,
            background: true,
            ..Default::default()
        }
    }
    #[test]
    fn named_sibling_pages_reformat_at_their_own_dimensions() {
        let mut input = super::super::tests::fixture();
        input.stylesheets=vec!["@page wide {size:96px 96px;margin:0} @page narrow {size:48px 144px;margin:0} article {height:20px;page:wide} article+article {page:narrow} footer{display:none}".into()];
        let output = super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
        assert_eq!(output.pages.len(), 2);
        assert_eq!(output.pages[0].viewport.width, 96.);
        assert_eq!(output.pages[1].viewport.width, 48.);
        assert_eq!(output.pages[1].viewport.height, 144.);
        let model = output.pages[1].geometry[&3].box_model.as_ref().unwrap();
        assert!(model.border_box.y.abs() < 0.01);
        assert!((model.border_box.width - 48.).abs() < 0.01);
    }
    #[test]
    fn variable_first_page_rebreaks_remaining_native_text_without_duplicate_prefix() {
        let mut input = super::super::tests::fixture();
        input.nodes[0].children = vec![2];
        input.nodes.retain(|n| n.id <= 2);
        input.nodes[1].text = "alpha beta gamma delta epsilon zeta eta theta iota kappa".into();
        input.nodes[1].content = Some(vec![LayoutContent::Text {
            node_id: 20,
            text: input.nodes[1].text.clone(),
        }]);
        input.stylesheets=vec!["@page {size:192px 144px;margin:0} @page :first {size:96px 48px} article{font-size:16px;line-height:16px;orphans:1;widows:1}".into()];
        let output = super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
        assert!(output.pages.len() >= 2);
        assert_eq!(output.pages[0].viewport.width, 96.);
        assert_eq!(output.pages[1].viewport.width, 192.);
        let visible = output
            .pages
            .iter()
            .flat_map(|page| {
                page.text_fragments
                    .values()
                    .flatten()
                    .filter(|f| {
                        let flow = f.flow_rect.unwrap_or(f.rect);
                        flow.y >= 0. && flow.bottom() <= page.viewport.height + 0.01
                    })
                    .map(|f| f.text.clone())
            })
            .collect::<String>();
        assert_eq!(
            visible.split_whitespace().collect::<Vec<_>>(),
            input.nodes[1].text.split_whitespace().collect::<Vec<_>>()
        );
    }
    #[test]
    fn transformed_negative_leading_consumes_native_source_once() {
        let mut input = super::super::tests::fixture();
        input.nodes[0].children = vec![2];
        input.nodes.retain(|n| n.id <= 2);
        input.nodes[1].text = "alpha beta gamma delta epsilon zeta eta theta iota kappa".into();
        input.nodes[1].content = Some(vec![LayoutContent::Text {
            node_id: 20,
            text: input.nodes[1].text.clone(),
        }]);
        for transform in ["none", "translateY(160px)", "scale(2)"] {
            // A page selector selects source-token continuation. Uniform pages
            // instead retain one document and move its projection matrix.
            input.stylesheets = vec![format!(
                "@page {{size:96px 48px;margin:0}} @page :first {{size:96px 48px}} article{{font-size:16px;line-height:16px;orphans:1;widows:1;transform:{transform}}}"
            )];
            assert!(page_rules::needs_continuations(&page_rules::read(&input)));
            let output =
                super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
            assert!(output.pages.len() >= 2);
            let consumed = output
                .pages
                .iter()
                .flat_map(|page| {
                    page.paint_projections
                        .values()
                        .flat_map(|p| p.text.iter())
                        .filter(|f| {
                            let flow = f.flow_rect.expect("native flow bounds");
                            flow.y >= -0.01 && flow.bottom() <= 48.01
                        })
                        .map(|f| f.text.clone())
                })
                .collect::<String>();
            assert_eq!(
                consumed.split_whitespace().collect::<Vec<_>>(),
                input.nodes[1].text.split_whitespace().collect::<Vec<_>>(),
                "{transform}"
            );
        }
    }
    #[test]
    fn named_recto_transition_inserts_a_classified_blank_page() {
        let mut input = super::super::tests::fixture();
        input.stylesheets=vec!["@page report {size:96px 96px;margin:0} @page report:blank {size:80px 96px} article{page:report;height:20px} article+article{break-before:right} footer{display:none}".into()];
        let output = super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
        assert_eq!(output.pages.len(), 3);
        assert!(output.pages[1].paint_projections.is_empty());
        assert_eq!(output.pages[1].viewport.width, 80.);
    }
    #[test]
    fn repeated_table_groups_keep_headers_and_reserve_footers_on_each_continuation() {
        for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            let mut input = super::super::tests::fixture();
            let prototype = input.nodes[0].clone();
            let make = |id, parent, tag: &str, children: Vec<u64>, text: &str| {
                let mut n = prototype.clone();
                n.id = id;
                n.parent = parent;
                n.tag = tag.into();
                n.children = children;
                n.text = text.into();
                n.content = None;
                n
            };
            input.nodes = vec![
                make(1, None, "table", vec![2, 5, 12], ""),
                make(2, Some(1), "thead", vec![3], ""),
                make(3, Some(2), "tr", vec![4], ""),
                make(4, Some(3), "td", vec![], "H"),
                make(5, Some(1), "tbody", vec![6, 8, 10], ""),
                make(6, Some(5), "tr", vec![7], ""),
                make(7, Some(6), "td", vec![], "A"),
                make(8, Some(5), "tr", vec![9], ""),
                make(9, Some(8), "td", vec![], "B"),
                make(10, Some(5), "tr", vec![11], ""),
                make(11, Some(10), "td", vec![], "C"),
                make(12, Some(1), "tfoot", vec![13], ""),
                make(13, Some(12), "tr", vec![14], ""),
                make(14, Some(13), "td", vec![], "F"),
            ];
            let (inline, block) = if mode == "horizontal-tb" {
                ("width", "height")
            } else {
                ("height", "width")
            };
            input.stylesheets = vec![format!(
                "table{{{inline}:96px;border-spacing:0;writing-mode:{mode}}}thead,tfoot{{{block}:20px}}thead td,tfoot td{{{block}:20px}}tbody td{{{block}:30px}}"
            )];
            let output =
                super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
            assert_eq!(output.pages.len(), 3);
            for page in &output.pages {
                assert!(
                    page.paint_projections[&4]
                        .text
                        .iter()
                        .any(|f| f.text == "H")
                );
                assert!(
                    page.paint_projections[&14]
                        .text
                        .iter()
                        .any(|f| f.text == "F")
                );
            }
        }
    }
    #[test]
    fn rowspan_continuation_retains_column_occupancy_and_consumes_text_once() {
        let mut input = super::super::tests::fixture();
        let prototype = input.nodes[0].clone();
        let make = |id, parent, tag: &str, children: Vec<u64>, text: &str| {
            let mut n = prototype.clone();
            n.id = id;
            n.parent = parent;
            n.tag = tag.into();
            n.children = children;
            n.text = text.into();
            n.content = None;
            n
        };
        input.nodes = vec![
            make(1, None, "table", vec![2, 5], ""),
            make(2, Some(1), "tr", vec![3, 4], ""),
            make(
                3,
                Some(2),
                "td",
                vec![],
                "one two three four five six seven eight nine ten eleven twelve",
            ),
            make(4, Some(2), "td", vec![], "A"),
            make(5, Some(1), "tr", vec![6], ""),
            make(6, Some(5), "td", vec![], "B"),
        ];
        input.nodes[2]
            .attributes
            .insert("rowspan".into(), "2".into());
        input.stylesheets=vec!["@page :first{margin:0} table{width:96px;table-layout:fixed;border-collapse:collapse}td{width:48px;padding:0;font-size:12px;line-height:16px;orphans:1;widows:1}tr{height:64px}".into()];
        let source = input.nodes[2].text.clone();
        let output = super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
        assert!(output.pages.len() > 1);
        let text = output
            .pages
            .iter()
            .flat_map(|page| {
                page.text_fragments
                    .get(&3)
                    .into_iter()
                    .flatten()
                    .filter(|f| {
                        let flow = f.flow_rect.unwrap_or(f.rect);
                        flow.y >= -0.01 && flow.bottom() <= page.viewport.height + 0.01
                    })
                    .map(|f| f.text.clone())
            })
            .collect::<String>();
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            source.split_whitespace().collect::<Vec<_>>()
        );
        let first = output.pages[0].paint_projections[&4].box_model.border_box.x;
        let trailing_text = output
            .pages
            .iter()
            .flat_map(|page| {
                page.text_fragments
                    .get(&6)
                    .into_iter()
                    .flatten()
                    .filter(|fragment| {
                        let flow = fragment.flow_rect.unwrap_or(fragment.rect);
                        flow.y >= -0.01 && flow.bottom() <= page.viewport.height + 0.01
                    })
                    .map(|fragment| fragment.text.as_str())
            })
            .collect::<String>();
        assert_eq!(
            trailing_text, "B",
            "the later row must reach a page exactly once"
        );
        for page in &output.pages {
            if let Some(right) = page.paint_projections.get(&6) {
                assert!((right.box_model.border_box.x - first).abs() < 0.01);
            }
        }
    }

    #[test]
    fn vertical_named_pages_reformat_at_each_physical_page_width() {
        let mut input = super::super::tests::fixture();
        input.stylesheets=vec!["@page first {size:96px 96px;margin:0}@page second {size:48px 96px;margin:0}section{writing-mode:vertical-rl;height:96px}article{width:30px;page:first}article+article{page:second}footer{display:none}".into()];
        let output = super::super::compute(&input, &LayoutLimits::default(), &settings()).unwrap();
        assert_eq!(output.pages.len(), 2);
        assert_eq!(output.pages[0].viewport.width, 96.);
        assert_eq!(output.pages[1].viewport.width, 48.);
        let first = output.pages[0].geometry[&2]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        let second = output.pages[1].geometry[&3]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!((first.right() - 96.).abs() < 0.01);
        assert!((second.right() - 48.).abs() < 0.01);
    }

    #[test]
    fn utf16_break_tokens_never_split_surrogate_pairs() {
        let mut text = "A😀B".to_string();
        drop_utf16(&mut text, 2);
        assert_eq!(text, "😀B");
        drop_utf16(&mut text, 2);
        assert_eq!(text, "B");
    }
}
