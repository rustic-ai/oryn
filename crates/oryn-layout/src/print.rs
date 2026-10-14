//! Fresh print-media formatting and page fragmentation. This module consumes the
//! native DOM/style input, never a screen bitmap or an existing screen layout.

/// Inputs for insert gap.
struct PrintGapInputs<'a> {
    owner: u64,
    at: f64,
    gap: f64,
    inside: bool,
    by_id: &'a BTreeMap<u64, &'a LayoutNode>,
    styles: &'a ComputedStyles,
    geometry: &'a mut BTreeMap<u64, NodeGeometry>,
    text: &'a mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &'a mut TextEngine,
}
/// Inputs for margin nodes.
struct PageMarginInputs<'a> {
    rules: &'a [page_rules::Rule],
    name: &'a str,
    page: usize,
    total: usize,
    blank: bool,
    dimensions: PrintDimensions,
    engine: &'a mut TextEngine,
    next_id: &'a mut u64,
    source_ids: &'a BTreeSet<u64>,
}
use super::*;
#[path = "print_axis.rs"]
mod axis;
#[path = "print_continuation.rs"]
mod continuation;
#[path = "print_margin_boxes.rs"]
mod margin_boxes;
#[path = "print_rules.rs"]
mod page_rules;
use axis::PageAxis;
use oryn_common::v4::{PaintClip, PaintProjection, PrintOptions};
pub const MAX_PRINT_PAGES: usize = 256;
const PX_PER_CM: f64 = 96. / 2.54;

#[derive(Debug, thiserror::Error)]
pub enum PrintError {
    #[error("invalid print settings: {0}")]
    Invalid(String),
    #[error("print document exceeds the 256 page limit")]
    PageLimit,
    #[error(transparent)]
    Layout(#[from] LayoutError),
}
#[derive(Debug, Clone)]
pub struct PrintPageLayout {
    pub content_box: CssRect,
    pub scale: f64,
    pub backdrops: BTreeMap<u64, oryn_common::v4::PaintNodeSnapshot>,
    /// Native page-margin boxes, with IDs disjoint from DOM layout records.
    pub generated_nodes: Vec<oryn_common::v4::PaintNodeSnapshot>,
    pub viewport: ViewportState,
    pub geometry: BTreeMap<u64, NodeGeometry>,
    pub paint_projections: BTreeMap<u64, PaintProjection>,
    pub text_fragments: BTreeMap<u64, Vec<PaintTextFragment>>,
}
#[derive(Debug, Clone)]
pub struct PrintLayoutOutput {
    pub document: LayoutOutput,
    pub pages: Vec<PrintPageLayout>,
}
#[derive(Debug, Clone, Copy)]
pub struct PrintDimensions {
    pub width: f64,
    pub height: f64,
    pub content: CssRect,
}

pub fn validate_options(options: &PrintOptions) -> Result<PrintDimensions, PrintError> {
    if !options.scale.is_finite() || !(0.1..=2.).contains(&options.scale) {
        return Err(PrintError::Invalid(
            "scale must be between 0.1 and 2".into(),
        ));
    }
    if [options.page_width_cm, options.page_height_cm]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0. || *v > 200.)
    {
        return Err(PrintError::Invalid(
            "page dimensions must be positive and at most 200 cm".into(),
        ));
    }
    let margins = [
        options.margin_top_cm,
        options.margin_right_cm,
        options.margin_bottom_cm,
        options.margin_left_cm,
    ];
    if margins.iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err(PrintError::Invalid(
            "margins must be finite and nonnegative".into(),
        ));
    }
    let (width, height) = if options.landscape {
        (
            options.page_height_cm * PX_PER_CM,
            options.page_width_cm * PX_PER_CM,
        )
    } else {
        (
            options.page_width_cm * PX_PER_CM,
            options.page_height_cm * PX_PER_CM,
        )
    };
    let content = CssRect {
        x: options.margin_left_cm * PX_PER_CM,
        y: options.margin_top_cm * PX_PER_CM,
        width: width - (options.margin_left_cm + options.margin_right_cm) * PX_PER_CM,
        height: height - (options.margin_top_cm + options.margin_bottom_cm) * PX_PER_CM,
    };
    if content.width < 1. || content.height < 1. {
        return Err(PrintError::Invalid(
            "margins leave no printable area".into(),
        ));
    }
    parse_ranges(&options.page_ranges)?;
    Ok(PrintDimensions {
        width,
        height,
        content,
    })
}
fn parse_ranges(ranges: &[String]) -> Result<Vec<(usize, usize)>, PrintError> {
    if ranges.len() > MAX_PRINT_PAGES {
        return Err(PrintError::Invalid("too many page ranges".into()));
    }
    let number = |s: &str| {
        s.parse::<usize>()
            .ok()
            .filter(|n| *n > 0 && *n <= 1_000_000)
    };
    ranges
        .iter()
        .map(|value| {
            let value = value.trim();
            let pair = if let Some((a, b)) = value.split_once('-') {
                let (a, b) = (a.trim(), b.trim());
                if a.is_empty() && b.is_empty() {
                    None
                } else {
                    (if a.is_empty() { Some(1) } else { number(a) }).zip(if b.is_empty() {
                        Some(1_000_000)
                    } else {
                        number(b)
                    })
                }
            } else {
                number(value).map(|n| (n, n))
            };
            pair.filter(|(a, b)| a <= b)
                .ok_or_else(|| PrintError::Invalid(format!("invalid page range {value:?}")))
        })
        .collect()
}

pub fn compute(
    input: &LayoutInput,
    limits: &LayoutLimits,
    options: &PrintOptions,
) -> Result<PrintLayoutOutput, PrintError> {
    compute_with_capture_input(input, limits, options).map(|(output, _)| output)
}

/// Return the native source input with the viewport and interaction state used
/// for print layout, before object/generated-content projection changes nodes.
pub fn compute_with_capture_input(
    input: &LayoutInput,
    limits: &LayoutLimits,
    options: &PrintOptions,
) -> Result<(PrintLayoutOutput, LayoutInput), PrintError> {
    compute_with_style_sample(input, limits, options, None)
}

pub fn compute_with_style_sample(
    input: &LayoutInput,
    limits: &LayoutLimits,
    options: &PrintOptions,
    sample: Option<&svg_instances::StyleSample>,
) -> Result<(PrintLayoutOutput, LayoutInput), PrintError> {
    let base_dimensions = validate_options(options)?;
    let rules = page_rules::read(input);
    let dimensions =
        page_rules::dimensions(base_dimensions, &page_rules::cascade(&rules, "", 0, false))?;
    let mut print_input = input.clone();
    print_input.scroll = ScrollState::default();
    print_input.element_scrolls.clear();
    print_input.focus_visible = false;
    let mut scale = options.scale;
    print_input.viewport.width = dimensions.content.width / scale;
    print_input.viewport.height = dimensions.content.height / scale;
    let mut capture_input = print_input.clone();
    let (mut styles, mut author_whitespace, mut pseudo_styles) =
        stylo_computed_values_for_media_with_sample(&capture_input, true, sample)?;
    let (projected, mut object_hidden) = object_images::project_styled(&capture_input, &styles);
    print_input = projected.into_owned();
    if options.shrink_to_fit {
        let probe = compute_with_styles(&print_input, limits, styles.clone())?;
        let nodes = print_input
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        let axis = PageAxis::new(&nodes, &styles, &probe.geometry);
        let (used, available, paper) = if axis.vertical {
            (
                probe.scroll.content_height,
                print_input.viewport.height,
                dimensions.content.height,
            )
        } else {
            (
                probe.scroll.content_width,
                print_input.viewport.width,
                dimensions.content.width,
            )
        };
        if used > available + 0.01 {
            scale = (paper / used).min(scale).max(0.1);
            print_input.viewport.width = dimensions.content.width / scale;
            print_input.viewport.height = dimensions.content.height / scale;
            capture_input.viewport = print_input.viewport;
            let refreshed =
                stylo_computed_values_for_media_with_sample(&capture_input, true, sample)?;
            styles = refreshed.0;
            author_whitespace = refreshed.1;
            pseudo_styles = refreshed.2;
            let refreshed = object_images::project_styled(&capture_input, &styles);
            print_input = refreshed.0.into_owned();
            object_hidden = refreshed.1;
        }
    }
    if !options.background {
        for style in styles.values_mut().chain(
            pseudo_styles
                .values_mut()
                .flat_map(|styles| styles.values_mut()),
        ) {
            let value = Arc::make_mut(style);
            value.insert("background-color".into(), "transparent".into());
            value.insert("background-image".into(), "none".into());
        }
    }
    let generated = generated::materialize_for_media(&print_input, styles, &pseudo_styles, true)?;
    print_input = generated.input.clone();
    styles = generated.styles.clone();
    if page_rules::needs_continuations(&rules)
        || styles.values().any(|s| {
            style_value(s, "page", "auto") != "auto"
                || matches!(
                    style_value(s, "display", "block"),
                    "table-header-group" | "table-footer-group"
                )
        })
    {
        let mut output = continuation::compute_continuation(
            crate::print::continuation::PrintContinuationInputs {
                input: &print_input,
                limits,
                options,
                base: base_dimensions,
                rules: &rules,
                styles,
                whitespace: author_whitespace,
                scale,
            },
        )?;
        generated::finish(&generated, &mut output.document);
        output.document.pseudo_styles = pseudo_styles;
        populate_backdrops(
            &print_input,
            limits,
            options,
            &mut output.document,
            &mut output.pages,
            sample,
        )?;
        object_images::finish(&mut output.document, &object_hidden);
        return Ok((output, capture_input));
    }
    let roots = print_input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let axis = PageAxis::new(&roots, &styles, &BTreeMap::new());
    let page_extent = axis.extent(print_input.viewport);
    let mut document =
        compute_with_styles_fragmented(&print_input, limits, styles, Some(page_extent))?;
    document.author_whitespace = author_whitespace;
    document.pseudo_styles = pseudo_styles;
    let by_id = print_input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let fixed = |id: u64| {
        let mut current = Some(id);
        while let Some(id) = current {
            if positioning::viewport_fixed(id, &by_id, &document.computed_styles) {
                return true;
            }
            current = by_id.get(&id).and_then(|n| n.parent);
        }
        false
    };
    let axis = PageAxis::new(&by_id, &document.computed_styles, &document.geometry);
    let bottom = document
        .geometry
        .iter()
        .filter(|(id, _)| !fixed(**id))
        .filter_map(|(_, g)| g.box_model.as_ref())
        .map(|m| axis.rect(m.border_box).bottom())
        .fold(0., f64::max);
    if !bottom.is_finite() || bottom > page_extent * MAX_PRINT_PAGES as f64 {
        return Err(PrintError::PageLimit);
    }
    let count = (bottom / page_extent).ceil().max(1.) as usize;
    if document
        .geometry
        .len()
        .checked_mul(count)
        .is_none_or(|n| n > 1_000_000)
    {
        return Err(PrintError::Invalid(
            "print fragment metadata exceeds the one-million-fragment bound".into(),
        ));
    }
    let ranges = parse_ranges(&options.page_ranges)?;
    let mut pages = Vec::new();
    let mut margin_engine = TextEngine::new(&prepare_web_fonts(&print_input, limits).0);
    let mut generated_id = u64::MAX;
    let source_ids = print_input
        .nodes
        .iter()
        .map(|n| n.id)
        .collect::<BTreeSet<_>>();
    for page in 0..count {
        if !ranges.is_empty() && !ranges.iter().any(|(a, b)| (*a..=*b).contains(&(page + 1))) {
            continue;
        }
        let page_style = page_rules::cascade(&rules, "", page, false);
        let page_dimensions = page_rules::dimensions(base_dimensions, &page_style)?;
        let dimensions = page_dimensions;
        let mut result = project_page(
            &document,
            &print_input,
            dimensions,
            scale,
            page as f64 * page_extent,
        );
        result.generated_nodes = margin_nodes(PageMarginInputs {
            rules: &rules,
            name: "",
            page,
            total: count,
            blank: false,
            dimensions,
            engine: &mut margin_engine,
            next_id: &mut generated_id,
            source_ids: &source_ids,
        })?;
        pages.push(result);
    }
    if pages.is_empty() {
        return Err(PrintError::Invalid("page ranges select no pages".into()));
    }
    document.paint_fonts.extend(margin_engine.paint_fonts);
    generated::finish(&generated, &mut document);
    populate_backdrops(
        &print_input,
        limits,
        options,
        &mut document,
        &mut pages,
        sample,
    )?;
    object_images::finish(&mut document, &object_hidden);
    Ok((PrintLayoutOutput { document, pages }, capture_input))
}
fn populate_backdrops(
    input: &LayoutInput,
    limits: &LayoutLimits,
    options: &PrintOptions,
    document: &mut LayoutOutput,
    pages: &mut [PrintPageLayout],
    sample: Option<&svg_instances::StyleSample>,
) -> Result<(), PrintError> {
    if !input.top_layer.iter().any(|entry| {
        !matches!(
            entry.kind,
            oryn_common::v4::TopLayerKind::SelectPicker
                | oryn_common::v4::TopLayerKind::TemporalPicker
        )
    }) {
        return Ok(());
    }
    let reserved = pages
        .iter()
        .flat_map(|page| page.generated_nodes.iter().map(|node| node.node_id))
        .collect::<BTreeSet<_>>();
    for page in pages {
        let mut local = input.clone();
        local.viewport.width = page.content_box.width / page.scale;
        local.viewport.height = page.content_box.height / page.scale;
        let (_, _, mut pseudos) =
            stylo_computed_values_for_media_with_sample(&local, true, sample)?;
        if !options.background {
            for style in pseudos.values_mut().flat_map(|styles| styles.values_mut()) {
                let style = Arc::make_mut(style);
                style.insert("background-color".into(), "transparent".into());
                style.insert("background-image".into(), "none".into());
            }
        }
        let mut layout = document.clone();
        layout.pseudo_styles = pseudos;
        layout.backdrops.clear();
        let diagnostic_start = layout.diagnostics.len();
        backdrop::append_reserved(&local, limits, &mut layout, &reserved)?;
        document
            .diagnostics
            .extend(layout.diagnostics.drain(diagnostic_start..));
        let mut matrix = transforms::Projection::IDENTITY.matrix();
        matrix[0] = page.scale;
        matrix[5] = page.scale;
        matrix[12] = page.content_box.x;
        matrix[13] = page.content_box.y;
        let shift = |rect: CssRect| CssRect {
            x: matrix[12] + rect.x * page.scale,
            y: matrix[13] + rect.y * page.scale,
            width: rect.width * page.scale,
            height: rect.height * page.scale,
        };
        for (owner, mut backdrop) in layout.backdrops {
            if !page.geometry.contains_key(&owner) {
                continue;
            }
            if let Some(projection) = &mut backdrop.projection {
                projection.matrix = multiply(matrix, projection.matrix);
                projection.text_matrix =
                    projection.text_matrix.map(|value| multiply(matrix, value));
                for clip in projection
                    .clips
                    .iter_mut()
                    .chain(&mut projection.text_clips)
                {
                    clip.matrix = multiply(matrix, clip.matrix);
                }
                let clip = PaintClip {
                    shape: None,
                    owner: None,
                    scroll_x: false,
                    scroll_y: false,
                    matrix: transforms::Projection::IDENTITY.matrix(),
                    rect: page.content_box,
                    clip_x: true,
                    clip_y: true,
                };
                projection.clips.push(clip.clone());
                projection.text_clips.push(clip);
                backdrop.geometry.in_viewport = projection.intersects_viewport(page.viewport);
            }
            if let Some(model) = &mut backdrop.geometry.box_model {
                model.border_box = shift(model.border_box);
                model.padding_box = shift(model.padding_box);
                model.content_box = shift(model.content_box);
            }
            backdrop.geometry.client_rects = backdrop
                .geometry
                .client_rects
                .into_iter()
                .map(shift)
                .collect();
            backdrop.geometry.interactable = false;
            page.backdrops.insert(owner, backdrop);
        }
    }
    Ok(())
}

fn project_page(
    document: &LayoutOutput,
    input: &LayoutInput,
    dimensions: PrintDimensions,
    scale: f64,
    top: f64,
) -> PrintPageLayout {
    let viewport = ViewportState {
        width: dimensions.width,
        height: dimensions.height,
        ..input.viewport
    };
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let fixed = |id: u64| {
        let mut current = Some(id);
        while let Some(id) = current {
            if positioning::viewport_fixed(id, &by_id, &document.computed_styles) {
                return true;
            }
            current = by_id.get(&id).and_then(|n| n.parent);
        }
        false
    };
    let axis = PageAxis::new(&by_id, &document.computed_styles, &document.geometry);
    let mut result = PrintPageLayout {
        content_box: dimensions.content,
        scale,
        backdrops: BTreeMap::new(),
        generated_nodes: Vec::new(),
        viewport,
        geometry: BTreeMap::new(),
        paint_projections: BTreeMap::new(),
        text_fragments: BTreeMap::new(),
    };
    for (id, projection) in &document.paint_projections {
        let (dx, dy) = if fixed(*id) {
            (dimensions.content.x, dimensions.content.y)
        } else {
            axis.offset(top, dimensions.content, scale)
        };
        let mut matrix = transforms::Projection::IDENTITY.matrix();
        matrix[0] = scale;
        matrix[5] = scale;
        matrix[12] = dx;
        matrix[13] = dy;
        let map = |m: [f64; 16]| multiply(matrix, m);
        let mut projection = projection.clone();
        projection.matrix = map(projection.matrix);
        projection.text_matrix = projection.text_matrix.map(map);
        for clip in projection
            .clips
            .iter_mut()
            .chain(&mut projection.text_clips)
        {
            clip.matrix = map(clip.matrix);
        }
        let page_clip = PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: transforms::Projection::IDENTITY.matrix(),
            rect: dimensions.content,
            clip_x: true,
            clip_y: true,
        };
        projection.clips.push(page_clip.clone());
        projection.text_clips.push(page_clip);
        // Keep ancestor records even when their own box is on another
        // page: overflowing descendants still belong to opacity/stacking
        // contexts. The renderer culls individual paint commands.
        let in_viewport = projection.intersects_viewport(viewport);
        let shift = |rect: CssRect| CssRect {
            x: dx + rect.x * scale,
            y: dy + rect.y * scale,
            width: rect.width * scale,
            height: rect.height * scale,
        };
        if let Some(source) = document.geometry.get(id) {
            let mut geometry = source.clone();
            if let Some(model) = &mut geometry.box_model {
                model.border_box = shift(model.border_box);
                model.padding_box = shift(model.padding_box);
                model.content_box = shift(model.content_box);
            }
            geometry.client_rects = geometry.client_rects.into_iter().map(shift).collect();
            geometry.in_viewport = in_viewport;
            geometry.interactable = false;
            result.geometry.insert(*id, geometry);
        }
        result.text_fragments.insert(
            *id,
            document
                .text_fragments
                .get(id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut f| {
                    f.rect = shift(f.rect);
                    f.flow_rect = f.flow_rect.map(shift);
                    for cluster in &mut f.clusters {
                        cluster.rect = shift(cluster.rect);
                    }
                    f
                })
                .collect(),
        );
        result.paint_projections.insert(*id, projection);
    }
    result
}
fn margin_nodes(
    params: PageMarginInputs<'_>,
) -> Result<Vec<oryn_common::v4::PaintNodeSnapshot>, PrintError> {
    let PageMarginInputs {
        rules,
        name,
        page,
        total,
        blank,
        dimensions,
        engine,
        next_id,
        source_ids,
    } = params;

    use oryn_common::v4::{GeometryAvailability, PaintNodeSnapshot};
    let mut nodes = Vec::new();
    let styles = page_rules::margin_styles(rules, name, page, blank);
    let models = margin_boxes::layout(&styles, dimensions, engine, page, total);
    for (kind, mut style) in styles {
        let Some(content) = style
            .get("content")
            .and_then(|value| page_rules::margin_content(value, page + 1, total))
        else {
            continue;
        };
        let Some(box_model) = models.get(&kind).cloned() else {
            continue;
        };
        let rect = box_model.border_box;
        if rect.width <= 0. || rect.height <= 0. {
            continue;
        }
        let content_box = box_model.content_box;
        while source_ids.contains(next_id) {
            *next_id = next_id.checked_sub(1).ok_or_else(|| {
                PrintError::Invalid("no synthetic page identities available".into())
            })?;
        }
        let id = *next_id;
        *next_id = next_id
            .checked_sub(1)
            .ok_or_else(|| PrintError::Invalid("no synthetic page identities available".into()))?;
        style
            .entry("font-family".into())
            .or_insert_with(|| DEFAULT_FONT_FAMILY.into());
        style
            .entry("color".into())
            .or_insert_with(|| "black".into());
        style.entry("text-align".into()).or_insert_with(|| {
            if kind.ends_with("left") {
                "left"
            } else if kind.ends_with("right") {
                "right"
            } else {
                "center"
            }
            .into()
        });
        let viewport = ViewportState {
            width: dimensions.width,
            height: dimensions.height,
            ..Default::default()
        };
        let font_size = length(style_value(&style, "font-size", "16px"), 16., 16., viewport);
        let mode = style_value(&style, "writing-mode", "horizontal-tb");
        let axes = writing_mode::Axes::new(mode, false);
        let mut shaped = engine.shape_styled(
            &content,
            if axes.vertical {
                content_box.height
            } else {
                content_box.width
            },
            font_size,
            &style,
            viewport,
        );
        let extra = (if axes.vertical {
            content_box.width
        } else {
            content_box.height
        } - shaped.height)
            .max(0.);
        let alignment = match style_value(&style, "vertical-align", "middle") {
            "top" => 0.,
            "bottom" => extra,
            _ => extra / 2.,
        };
        if axes.vertical {
            vertical_text::physicalize(&mut shaped, content_box.width, content_box.height, mode);
        }
        let (dx, dy) = if axes.vertical {
            (
                if axes.x_reverse {
                    -alignment
                } else {
                    alignment
                },
                0.,
            )
        } else {
            (0., alignment)
        };
        let text = shaped
            .lines
            .iter()
            .enumerate()
            .map(|(line, (text, w, h, top, left))| PaintTextFragment {
                flow_line: Some(line),
                flow_rect: Some({
                    let mut flow = shaped.flow_rect(line, mode);
                    flow.x += content_box.x + dx;
                    flow.y += content_box.y + dy;
                    flow
                }),
                clusters: shaped.paint_clusters(line, content_box.x + dx, content_box.y + dy),
                text: text.clone(),
                rect: CssRect {
                    x: content_box.x + dx + left,
                    y: content_box.y + dy + top,
                    width: *w,
                    height: *h,
                },
                glyph_runs: shaped.paint_runs(line),
                font_family: style_value(&style, "font-family", DEFAULT_FONT_FAMILY).into(),
                font_size,
                color: style_value(&style, "color", "black").into(),
            })
            .collect::<Vec<_>>();
        let projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            matrix: transforms::Projection::IDENTITY.matrix(),
            text_matrix: None,
            box_model: box_model.clone(),
            text: text.clone(),
            clips: vec![],
            text_clips: vec![],
        };
        nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            node_id: id,
            parent_id: None,
            tag: "oryn-page-margin".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(box_model),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: false,
                diagnostic: None,
            },
            projection: Some(projection),
            style,
            text,
            image_resource: None,
        });
    }
    Ok(nodes)
}
fn multiply(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|index| {
        let row = index % 4;
        let col = index / 4;
        (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
    })
}

/// Insert fragmentainer gaps into fresh flow geometry before transforms/paint.
/// Breaks move following flow content and grow containing blocks; no screen
/// raster or existing screen snapshot participates in this operation.
pub(super) fn fragment_flow(
    height: f64,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &mut TextEngine,
) -> Result<(), LayoutError> {
    let axis = PageAxis::new(by_id, styles, geometry);
    let remap = |geometry: &mut BTreeMap<u64, NodeGeometry>,
                 text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
                 engine: &mut TextEngine,
                 inverse: bool| {
        let map = |rect: CssRect| {
            if inverse {
                axis.physical(rect)
            } else {
                axis.rect(rect)
            }
        };
        for value in geometry.values_mut() {
            if let Some(model) = &mut value.box_model {
                model.border_box = map(model.border_box);
                model.padding_box = map(model.padding_box);
                model.content_box = map(model.content_box);
            }
            for rect in &mut value.client_rects {
                *rect = map(*rect);
            }
        }
        for fragments in text.values_mut() {
            for fragment in fragments {
                fragment.rect = map(fragment.rect);
                fragment.flow_rect = fragment.flow_rect.map(map);
                for cluster in &mut fragment.clusters {
                    cluster.rect = map(cluster.rect);
                }
            }
        }
        for clusters in engine.source_geometry.values_mut() {
            for cluster in clusters {
                cluster.rect = map(cluster.rect);
            }
        }
        for line_break in engine.break_boxes.values_mut() {
            line_break.before = map(line_break.before);
            line_break.after = line_break.after.map(map);
        }
    };
    if axis.vertical {
        remap(geometry, text, engine, false);
    }
    let result = fragment_flow_logical(height, by_id, styles, geometry, text, engine);
    if axis.vertical {
        remap(geometry, text, engine, true);
    }
    result
}
fn fragment_flow_logical(
    height: f64,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &mut TextEngine,
) -> Result<(), LayoutError> {
    let mut pending = by_id
        .values()
        .filter(|n| n.parent.is_none())
        .map(|n| (n.id, false))
        .collect::<Vec<_>>();
    let mut order = Vec::new();
    while let Some((id, exit)) = pending.pop() {
        if styles
            .get(&id)
            .is_some_and(|s| matches!(style_value(s, "position", "static"), "absolute" | "fixed"))
        {
            continue;
        }
        order.push((id, exit));
        if !exit {
            pending.push((id, true));
            pending.extend(by_id[&id].children.iter().rev().map(|id| (*id, false)));
        }
    }
    let mut pending_after = None::<String>;
    for (id, exit) in order {
        if exit {
            let after = style_value(&styles[&id], "break-after", "auto");
            if matches!(
                after,
                "page" | "always" | "left" | "right" | "recto" | "verso"
            ) {
                pending_after = Some(after.into());
            }
            continue;
        }
        let Some(model) = geometry.get(&id).and_then(|g| g.box_model.clone()) else {
            continue;
        };
        if !model.border_box.bottom().is_finite()
            || model.border_box.bottom() > height * MAX_PRINT_PAGES as f64
        {
            return Err(LayoutError::InvalidRenderedTree(
                "print page limit exceeded during fragmentation".into(),
            ));
        }
        let style = &styles[&id];
        let before = style_value(style, "break-before", "auto");
        let offset = model.border_box.y.rem_euclid(height);
        let avoid = matches!(
            style_value(style, "break-inside", "auto"),
            "avoid" | "avoid-page"
        ) || is_replaced_box(by_id[&id])
            || style_value(style, "display", "block") == "table-row";
        let before_forced = matches!(
            before,
            "page" | "always" | "left" | "right" | "recto" | "verso"
        );
        let after = pending_after.take();
        // At an adjoining boundary a later forced break-before wins over
        // break-after. Deferring the latter also avoids a trailing blank page.
        let forced = if before_forced {
            Some(before)
        } else {
            after.as_deref()
        };
        let gap = if let Some(kind) = forced {
            forced_gap(model.border_box.y, height, kind)
        } else if avoid
            && model.border_box.height <= height
            && offset + model.border_box.height > height + 0.01
        {
            height - offset
        } else {
            0.
        };
        if gap > 0. {
            insert_gap(PrintGapInputs {
                owner: id,
                at: model.border_box.y,
                gap,
                inside: false,
                by_id,
                styles,
                geometry,
                text,
                engine,
            });
        }
        // Place line boxes without cutting glyph rows. Honor minimum first/last
        // line counts when the block itself can fit in a fragmentainer.
        let orphans = style_value(style, "orphans", "2")
            .parse::<usize>()
            .unwrap_or(2)
            .max(1);
        let widows = style_value(style, "widows", "2")
            .parse::<usize>()
            .unwrap_or(2)
            .max(1);
        let count = text.get(&id).map(Vec::len).unwrap_or(0);
        let mut index = 0;
        let mut page_start = 0;
        while index < count {
            let Some(line) = text
                .get(&id)
                .and_then(|v| v.get(index))
                .map(|f| f.flow_rect.unwrap_or(f.rect))
            else {
                break;
            };
            if line.height <= height && line.y.rem_euclid(height) + line.height > height + 0.01 {
                let mut boundary = index;
                if count - index < widows && index >= widows {
                    boundary = count - widows;
                }
                boundary = boundary.max(page_start);
                if boundary - page_start < orphans {
                    boundary = page_start;
                }
                let top = text[&id][boundary]
                    .flow_rect
                    .unwrap_or(text[&id][boundary].rect)
                    .y;
                let gap = height - top.rem_euclid(height);
                if gap < height - 0.01 {
                    insert_gap(PrintGapInputs {
                        owner: id,
                        at: top,
                        gap,
                        inside: true,
                        by_id,
                        styles,
                        geometry,
                        text,
                        engine,
                    });
                    page_start = boundary;
                    index = boundary;
                    continue;
                }
            }
            index += 1;
        }
    }
    Ok(())
}
/// Page one is recto (right). A forced break at an existing page boundary
/// does not produce an extra blank page unless parity requires one.
fn forced_gap(at: f64, height: f64, kind: &str) -> f64 {
    let offset = at.rem_euclid(height);
    let mut gap = if offset > 0.01 { height - offset } else { 0. };
    if matches!(kind, "left" | "right" | "recto" | "verso") {
        let page = ((at + gap) / height).floor() as usize + 1;
        if (page % 2 == 1) != matches!(kind, "right" | "recto") {
            gap += height;
        }
    }
    gap
}
fn insert_gap(params: PrintGapInputs<'_>) {
    let PrintGapInputs {
        owner,
        at,
        gap,
        inside,
        by_id,
        styles,
        geometry,
        text,
        engine,
    } = params;

    let mut ancestors = BTreeSet::new();
    let mut current = if inside {
        Some(owner)
    } else {
        by_id[&owner].parent
    };
    while let Some(id) = current {
        ancestors.insert(id);
        current = by_id.get(&id).and_then(|n| n.parent);
    }
    let fixed = |id| {
        let mut current = Some(id);
        while let Some(id) = current {
            if positioning::viewport_fixed(id, by_id, styles) {
                return true;
            }
            current = by_id.get(&id).and_then(|n| n.parent);
        }
        false
    };
    for (id, value) in geometry.iter_mut() {
        if fixed(*id) {
            continue;
        }
        if let Some(model) = &mut value.box_model {
            let spanning = style_value(&styles[id], "display", "block") == "table-cell"
                && by_id[id]
                    .attributes
                    .get("rowspan")
                    .is_some_and(|value| value != "1")
                && model.border_box.y < at - 0.001
                && model.border_box.bottom() > at + 0.001;
            if (ancestors.contains(id) || spanning) && model.border_box.y < at - 0.001 {
                model.border_box.height += gap;
                model.padding_box.height += gap;
                model.content_box.height += gap;
                for rect in &mut value.client_rects {
                    if rect.y < at - 0.001 && rect.bottom() >= at - 0.001 {
                        rect.height += gap;
                    }
                }
            } else if model.border_box.y >= at - 0.001 {
                for rect in [
                    &mut model.border_box,
                    &mut model.padding_box,
                    &mut model.content_box,
                ] {
                    rect.y += gap;
                }
                for rect in &mut value.client_rects {
                    rect.y += gap;
                }
            }
        }
    }
    // Selection rectangles can start before a CSS line's flow origin (negative
    // leading). Retain which native lines moved instead of testing each source
    // rectangle against the flow boundary independently.
    let mut moved_cluster_tops = BTreeMap::<u64, Vec<f64>>::new();
    let mut moved_lines = BTreeSet::new();
    for (id, fragments) in text {
        if !fixed(*id) {
            for fragment in fragments {
                if fragment.flow_rect.unwrap_or(fragment.rect).y >= at - 0.001 {
                    fragment.rect.y += gap;
                    if let Some(flow) = &mut fragment.flow_rect {
                        flow.y += gap;
                    }
                    if let Some(line) = fragment.flow_line {
                        moved_lines.insert((*id, line));
                    }
                    for cluster in &mut fragment.clusters {
                        moved_cluster_tops
                            .entry(*id)
                            .or_default()
                            .push(cluster.rect.y);
                        cluster.rect.y += gap;
                    }
                }
            }
        }
    }
    moved_lines.extend(moved_source_lines(
        &engine.source_geometry,
        &moved_cluster_tops,
    ));
    for clusters in engine.source_geometry.values_mut() {
        for cluster in clusters {
            if !fixed(cluster.layout_node)
                && (cluster.rect.y >= at - 0.001
                    || moved_lines.contains(&(cluster.layout_node, cluster.line)))
            {
                cluster.rect.y += gap;
            }
        }
    }
    for line_break in engine.break_boxes.values_mut() {
        if !fixed(line_break.layout_node) {
            if line_break.before.y >= at - 0.001 {
                line_break.before.y += gap;
            }
            if let Some(after) = &mut line_break.after
                && after.y >= at - 0.001
            {
                after.y += gap;
            }
        }
    }
}

fn moved_source_lines(
    source: &BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
    moved_tops: &BTreeMap<u64, Vec<f64>>,
) -> BTreeSet<(u64, usize)> {
    let mut sorted_tops = moved_tops.clone();
    for tops in sorted_tops.values_mut() {
        tops.sort_by(f64::total_cmp);
        tops.dedup();
    }
    source
        .values()
        .flatten()
        .filter(|cluster| {
            sorted_tops.get(&cluster.layout_node).is_some_and(|tops| {
                let first = tops.partition_point(|top| *top < cluster.rect.y - 0.000_001);
                tops.get(first)
                    .is_some_and(|top| (cluster.rect.y - top).abs() < 0.000_001)
            })
        })
        .map(|cluster| (cluster.layout_node, cluster.line))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragment_gap_retains_source_line_with_negative_leading() {
        let cluster = |owner, line, y, start| oryn_common::v4::TextClusterGeometry {
            layout_node: owner,
            line,
            start,
            end: start + 1,
            rect: CssRect {
                x: 0.,
                y,
                width: 10.,
                height: 20.,
            },
            matrix: transforms::Projection::IDENTITY.matrix(),
            rtl: false,
        };
        // The second line's CSS flow top is 32, but its selection top is 31.
        // A different owner at the same physical top must remain independent.
        let source = BTreeMap::from([
            (20, vec![cluster(2, 0, 15., 0), cluster(2, 1, 31., 1)]),
            (21, vec![cluster(2, 1, 31., 0)]),
            (30, vec![cluster(3, 1, 31., 0)]),
        ]);
        let lines = moved_source_lines(&source, &BTreeMap::from([(2, vec![31.])]));
        assert_eq!(lines, BTreeSet::from([(2, 1)]));
        assert_eq!(
            source
                .values()
                .flatten()
                .filter(|c| lines.contains(&(c.layout_node, c.line)))
                .count(),
            2
        );
    }
    #[test]
    fn forced_break_parity_is_shared_by_before_and_after() {
        assert_eq!(forced_gap(25., 100., "page"), 75.);
        assert_eq!(forced_gap(100., 100., "page"), 0.);
        assert_eq!(forced_gap(100., 100., "right"), 100.);
        assert_eq!(forced_gap(100., 100., "left"), 0.);
        assert_eq!(forced_gap(25., 100., "recto"), 175.);
    }
    #[test]
    fn margin_boxes_use_native_glyphs_resources_and_physical_page_counters() {
        let mut input = fixture();
        input.stylesheets.push("@page { @top-center {content: \"Page \" counter(page) \" of \" counter(pages);font-size:8px;color:green} }".into());
        let options = PrintOptions {
            page_width_cm: 2.54,
            page_height_cm: 3.048,
            margin_top_cm: 0.254,
            margin_bottom_cm: 0.254,
            margin_left_cm: 0.,
            margin_right_cm: 0.,
            shrink_to_fit: false,
            background: true,
            ..Default::default()
        };
        let output = compute(&input, &LayoutLimits::default(), &options).unwrap();
        assert_eq!(output.pages.len(), 2);
        let mut ids = BTreeSet::new();
        for (index, page) in output.pages.iter().enumerate() {
            assert_eq!(page.generated_nodes.len(), 1);
            let margin = &page.generated_nodes[0];
            assert!(ids.insert(margin.node_id));
            assert!(!input.nodes.iter().any(|n| n.id == margin.node_id));
            assert_eq!(
                margin
                    .text
                    .iter()
                    .map(|f| f.text.as_str())
                    .collect::<String>(),
                format!("Page {} of 2", index + 1)
            );
            assert!(
                margin
                    .text
                    .iter()
                    .flat_map(|f| &f.glyph_runs)
                    .all(|run| output.document.paint_fonts.contains_key(&run.font_key))
            );
        }
    }
    #[test]
    fn uniform_pages_keep_native_projection_text_and_page_offset_separate() {
        let mut input = fixture();
        input.nodes[0].children = vec![2];
        input.nodes.retain(|node| node.id <= 2);
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        input.nodes[1].text = text.into();
        input.nodes[1].content = Some(vec![LayoutContent::Text {
            node_id: 20,
            text: text.into(),
        }]);
        input.stylesheets=vec!["@page{size:96px 48px;margin:0}article{font-size:16px;line-height:16px;orphans:1;widows:1}".into()];
        assert!(!page_rules::needs_continuations(&page_rules::read(&input)));
        let output = compute(&input, &LayoutLimits::default(), &options()).unwrap();
        assert!(output.pages.len() >= 2);
        let mut consumed = String::new();
        for (index, page) in output.pages.iter().enumerate() {
            let offset = index as f64 * 48.;
            let projection = &page.paint_projections[&2];
            assert!(
                (projection.matrix[13] + offset).abs() < 0.01,
                "page {index}: {:?}",
                projection.matrix
            );
            assert_eq!(
                projection.text, output.document.paint_projections[&2].text,
                "page {index} preserves native projection inputs"
            );
            for (native, projected) in projection.text.iter().zip(&page.text_fragments[&2]) {
                let flow = native.flow_rect.unwrap();
                let projected_flow = projected.flow_rect.unwrap();
                assert!(
                    (projected_flow.y - flow.y + offset).abs() < 0.01,
                    "page {index}: {native:?} / {projected:?}"
                );
                if flow.y - offset >= -0.01 && flow.bottom() - offset <= 48.01 {
                    consumed.push_str(&native.text);
                }
            }
        }
        assert_eq!(
            consumed.split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    pub(super) fn fixture() -> LayoutInput {
        let make = |id, parent, tag: &str| LayoutNode {
            tree_scope: None,
            document_url: None,
            id,
            parent,
            tag: tag.into(),
            namespace: default_element_namespace(),
            children: Vec::new(),
            attributes: BTreeMap::new(),
            namespaced_attributes: Vec::new(),
            checked: None,
            disabled: None,
            validity: None,
            indeterminate: false,
            image_intrinsics: None,
            image_document_root: None,
            document_replaced: false,
            intrinsic_image_size: None,
            text: String::new(),
            content: None,
        };
        let mut root = make(1, None, "section");
        root.children = vec![2, 3, 4];
        LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![root,make(2,Some(1),"article"),make(3,Some(1),"article"),make(4,Some(1),"footer")],
            stylesheets:vec!["article {height:10px;background:red} @media print {article {height:60px;background:green;break-inside:avoid-page}} footer {position:fixed;left:1px;top:2px;width:5px;height:5px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_owner_ids: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
            custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,viewport:ViewportState::default(),scroll:ScrollState::default(),
            color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()}
    }
    fn options() -> PrintOptions {
        PrintOptions {
            page_width_cm: 2.54,
            page_height_cm: 2.54,
            margin_top_cm: 0.,
            margin_right_cm: 0.,
            margin_bottom_cm: 0.,
            margin_left_cm: 0.,
            background: true,
            shrink_to_fit: false,
            ..Default::default()
        }
    }
    #[test]
    fn printed_svg_object_intrinsics_follow_final_unprojected_media_cascade() {
        for shrink in [false, true] {
            let mut input = fixture();
            input.nodes.truncate(3);
            input.nodes[0].children = vec![2];
            let object = &mut input.nodes[1];
            object.tag = "object".into();
            object.children = vec![3];
            object.content = Some(vec![LayoutContent::Element(3)]);
            object.document_replaced = true;
            object.image_document_root = Some(3);
            object.image_intrinsics =
                Some(oryn_dom::ImageIntrinsics::new(Some(80.), None, Some(2.)));
            object.intrinsic_image_size = Some((80, 40));
            let svg = &mut input.nodes[2];
            svg.parent = Some(2);
            svg.tag = "svg".into();
            svg.namespace = "http://www.w3.org/2000/svg".into();
            svg.attributes
                .insert("viewBox".into(), "0 0 200 100".into());
            input.stylesheets = vec!["section{width:240px}object{display:block}svg{width:120px;height:auto}@media print and (min-width:200px){svg{width:200px}}".into()];
            let mut settings = options();
            settings.shrink_to_fit = shrink;
            let (printed, capture) =
                compute_with_capture_input(&input, &LayoutLimits::default(), &settings).unwrap();
            let expected = if shrink { 200. } else { 120. };
            let rect = printed.document.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .content_box;
            assert_eq!((rect.width, rect.height), (expected, expected / 2.));
            assert_eq!(
                printed.document.geometry[&3].availability,
                GeometryAvailability::Hidden
            );
            assert_eq!(capture.nodes[1].children, vec![3]);
            assert_eq!(capture.nodes[2].parent, Some(2));
            assert_eq!(capture.nodes[1].intrinsic_image_size, Some((80, 40)));
            if shrink {
                assert!(capture.viewport.width >= 200.);
            }
        }
    }

    #[test]
    fn print_backdrop_is_reflowed_per_content_viewport_and_projected_to_paper() {
        let mut input = fixture();
        input.nodes[3].tag = "dialog".into();
        input.nodes[3]
            .attributes
            .insert("open".into(), String::new());
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 4,
            kind: oryn_common::v4::TopLayerKind::ModalDialog,
        });
        input.stylesheets.push(
            "dialog{position:fixed;width:5px;height:5px}dialog::backdrop{inset:0;background:red}"
                .into(),
        );
        let mut settings = options();
        settings.margin_left_cm = 0.254;
        settings.margin_top_cm = 0.254;
        settings.scale = 0.75;
        let output = compute(&input, &LayoutLimits::default(), &settings).unwrap();
        for page in &output.pages {
            let backdrop = &page.backdrops[&4];
            let projection = backdrop.projection.as_ref().unwrap();
            assert!(projection.contains_point(page.content_box.x + 1., page.content_box.y + 1.));
            assert!(!projection.contains_point(page.content_box.x - 1., page.content_box.y - 1.));
            let rect = backdrop.geometry.box_model.as_ref().unwrap().border_box;
            assert!((rect.width - page.content_box.width).abs() < 0.02);
            assert!((rect.height - page.content_box.height).abs() < 0.02);
            assert!(
                !page
                    .generated_nodes
                    .iter()
                    .any(|node| node.node_id == backdrop.node_id)
            );
        }
    }
    #[test]
    fn print_and_frame_cascades_preserve_owning_time_and_print_media() {
        for continuation in [false, true] {
            let mut input = fixture();
            input.stylesheets.push("article {animation:grow 2s linear both} @keyframes grow {from {font-size:10px} to {font-size:30px}}".into());
            if continuation {
                input.stylesheets.push("article{page:chapter}".into());
            }
            let limits = LayoutLimits::default();
            let mut engine = StyleEngine::default();
            engine.compute_at(&input, &limits, 0.).unwrap();
            let mut sizes = Vec::new();
            for time in [0.5, 1.5] {
                let screen = engine.compute_at(&input, &limits, time).unwrap();
                let sample = engine.instance_style_sample().unwrap();
                let (printed, _) =
                    compute_with_style_sample(&input, &limits, &options(), Some(&sample)).unwrap();
                let (frame, _) =
                    compute_print_viewport_with_style_sample(&input, &limits, Some(&sample))
                        .unwrap();
                for layout in [&printed.document, &frame] {
                    assert_eq!(
                        layout.computed_styles[&2]["font-size"],
                        screen.computed_styles[&2]["font-size"]
                    );
                    assert_eq!(layout.computed_styles[&2]["height"], "60px");
                }
                assert_eq!(screen.computed_styles[&2]["height"], "10px");
                sizes.push(printed.document.computed_styles[&2]["font-size"].clone());
            }
            assert_ne!(sizes[0], sizes[1]);
        }
    }

    #[test]
    fn print_capture_retains_source_tree_and_uses_adjusted_page_context() {
        for continuation in [false, true] {
            let mut input = fixture();
            input.scroll.x = 11.;
            input.scroll.y = 29.;
            input.element_scrolls.insert(2, (7., 13.));
            input.focus_visible = true;
            if continuation {
                input.stylesheets.push("article{page:chapter}".into());
            }
            let original = input.clone();
            let mut options = options();
            options.scale = 0.5;
            options.shrink_to_fit = false;
            let dimensions = validate_options(&options).unwrap();
            let (printed, captured) =
                compute_with_capture_input(&input, &LayoutLimits::default(), &options).unwrap();
            assert!(!printed.pages.is_empty());
            assert_eq!(input, original);
            assert_eq!(captured.nodes, original.nodes);
            assert_eq!(captured.stylesheets, original.stylesheets);
            assert_eq!(
                captured.viewport.width,
                dimensions.content.width / options.scale
            );
            assert_eq!(
                captured.viewport.height,
                dimensions.content.height / options.scale
            );
            assert_eq!(captured.scroll, ScrollState::default());
            assert!(captured.element_scrolls.is_empty());
            assert!(!captured.focus_visible);
        }
    }

    #[test]
    fn print_media_reflows_and_avoids_breaking_boxes_without_mutating_screen_input() {
        let input = fixture();
        let original = input.clone();
        let screen = crate::compute(&input, &LayoutLimits::default()).unwrap();
        let print = compute(&input, &LayoutLimits::default(), &options()).unwrap();
        assert_eq!(input, original);
        assert_eq!(
            screen.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .height,
            10.
        );
        assert_eq!(
            print.document.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .height,
            60.
        );
        assert_eq!(print.pages.len(), 2);
        assert!(
            (print.pages[1].geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .y)
                .abs()
                < 0.02
        );
        for page in &print.pages {
            let fixed = page.geometry[&4].box_model.as_ref().unwrap().border_box;
            assert_eq!((fixed.x, fixed.y), (1., 2.));
        }
        assert_eq!(
            oryn_common::color::parse_color(
                &print.document.computed_styles[&2]["background-color"]
            ),
            Some([0, 128, 0, 255])
        );
    }
    #[test]
    fn forced_break_page_ranges_scale_and_physical_margins_are_consistent() {
        let mut input = fixture();
        input.stylesheets = vec![
            "article {height:20px} article+article {break-before:page} footer {display:none}"
                .into(),
        ];
        let mut settings = options();
        settings.margin_top_cm = 0.254;
        settings.scale = 2.;
        settings.page_ranges = vec!["2-".into(), "2".into()];
        let result = compute(&input, &LayoutLimits::default(), &settings).unwrap();
        assert_eq!(result.pages.len(), 1);
        let rect = result.pages[0].geometry[&3]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!((rect.y - 9.6).abs() < 0.02);
        assert!((rect.height - 40.).abs() < 0.02);
        assert!((result.pages[0].viewport.height - 96.).abs() < 0.02);
    }
    #[test]
    fn vertical_pages_fragment_along_block_axis_and_keep_fixed_content_on_each_page() {
        for mode in ["vertical-rl", "vertical-lr"] {
            let mut input = fixture();
            input.stylesheets = vec![format!(
                "section{{writing-mode:{mode};height:96px}}article{{width:60px;height:30px;break-inside:avoid-page}}footer{{position:fixed;left:1px;top:2px;width:5px;height:5px}}"
            )];
            let output = compute(&input, &LayoutLimits::default(), &options()).unwrap();
            assert_eq!(output.pages.len(), 2);
            let first = output.pages[0].geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box;
            let next = output.pages[1].geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box;
            assert!((first.x - next.x).abs() < 0.01);
            assert!((first.x - if mode == "vertical-rl" { 36. } else { 0. }).abs() < 0.01);
            for page in &output.pages {
                let fixed = page.geometry[&4].box_model.as_ref().unwrap().border_box;
                assert_eq!((fixed.x, fixed.y), (1., 2.));
            }
        }
    }

    #[test]
    fn adjoining_break_before_wins_without_creating_a_trailing_blank_page() {
        let mut input = fixture();
        input.stylesheets=vec!["article{height:20px;break-after:right}article+article{break-before:left}footer{display:none}".into()];
        let output = compute(&input, &LayoutLimits::default(), &options()).unwrap();
        assert_eq!(output.pages.len(), 2);
        assert!(
            (output.pages[1].geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .y)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn rowspan_border_geometry_grows_through_inserted_row_page_gaps() {
        let mut input = fixture();
        let prototype = input.nodes[0].clone();
        let make = |id, parent, tag: &str, children: Vec<u64>| {
            let mut node = prototype.clone();
            node.id = id;
            node.parent = parent;
            node.tag = tag.into();
            node.children = children;
            node
        };
        input.nodes = vec![
            make(1, None, "table", vec![2, 5]),
            make(2, Some(1), "tr", vec![3, 4]),
            make(3, Some(2), "td", vec![]),
            make(4, Some(2), "td", vec![]),
            make(5, Some(1), "tr", vec![6]),
            make(6, Some(5), "td", vec![]),
        ];
        input.nodes[2]
            .attributes
            .insert("rowspan".into(), "2".into());
        input.stylesheets =
            vec!["table{width:100px;border-spacing:0}tr{height:60px}td{padding:0}".into()];
        let output = compute(&input, &LayoutLimits::default(), &options()).unwrap();
        assert_eq!(output.pages.len(), 2);
        let span = output.document.geometry[&3]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        let second = output.document.geometry[&5]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!((span.height - 156.).abs() < 0.01);
        assert!((span.bottom() - second.bottom()).abs() < 0.01);
    }

    #[test]
    fn malformed_settings_and_empty_page_selection_fail() {
        let input = fixture();
        let mut settings = options();
        settings.scale = f64::NAN;
        assert!(matches!(
            compute(&input, &LayoutLimits::default(), &settings),
            Err(PrintError::Invalid(_))
        ));
        for range in ["0", "3-2", "-", "1-2-3", "word"] {
            assert!(parse_ranges(&[range.into()]).is_err());
        }
        assert_eq!(
            parse_ranges(&["-3".into(), "2-".into()]).unwrap(),
            vec![(1, 3), (2, 1_000_000)]
        );
        settings = options();
        settings.page_ranges = vec!["99".into()];
        assert!(matches!(
            compute(&input, &LayoutLimits::default(), &settings),
            Err(PrintError::Invalid(_))
        ));
        settings = options();
        settings.margin_left_cm = 3.;
        assert!(validate_options(&settings).is_err());
    }
}
