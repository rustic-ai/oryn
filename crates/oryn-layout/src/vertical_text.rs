//! Vertical paragraphs retain Parley's font fallback/style itemization, then use
//! HarfRust's native TTB shaping for upright runs. Sideways runs retain horizontal
//! shaping and carry an explicit per-glyph orientation to the paint consumer.

/// Inputs for shape impl.
pub(crate) struct VerticalParagraphInputs<'a> {
    pub(crate) engine: &'a mut TextEngine,
    pub(crate) text: &'a str,
    pub(crate) inline_size: f64,
    pub(crate) font_size: f64,
    pub(crate) style: &'a BTreeMap<String, String>,
    pub(crate) viewport: ViewportState,
    pub(crate) ranges: &'a [StyledTextRange<'a>],
    pub(crate) line_fragment: bool,
}
/// Inputs for shape segment.
pub(crate) struct VerticalSegmentInputs<'a> {
    pub(crate) source: &'a str,
    pub(crate) start: usize,
    pub(crate) run: &'a parley::layout::Run<'a, TextBrush>,
    pub(crate) style: &'a BTreeMap<String, String>,
    pub(crate) language: Option<fontique::Language>,
    pub(crate) owner: u64,
    pub(crate) orientation: PaintGlyphOrientation,
    pub(crate) font_key: String,
    pub(crate) rank: usize,
    pub(crate) viewport: ViewportState,
}
use super::*;
use oryn_common::v4::{PaintGlyph, PaintGlyphOrientation, PaintGlyphRun};
use unicode_segmentation::UnicodeSegmentation;

fn orientation(text: &str, style: &BTreeMap<String, String>) -> PaintGlyphOrientation {
    let mode = style_value(style, "writing-mode", "vertical-rl");
    let sideways = if mode == "sideways-lr" {
        PaintGlyphOrientation::CounterClockwise
    } else {
        PaintGlyphOrientation::Clockwise
    };
    if mode.starts_with("sideways") {
        return sideways;
    }
    match style_value(style, "text-orientation", "mixed") {
        "upright" => PaintGlyphOrientation::Upright,
        "sideways" => sideways,
        _ => {
            let property = icu_properties::CodePointMapData::<
                icu_properties::props::VerticalOrientation,
            >::new();
            match text.chars().next().map(|c| property.get(c)) {
                Some(
                    icu_properties::props::VerticalOrientation::Upright
                    | icu_properties::props::VerticalOrientation::TransformedUpright,
                ) => PaintGlyphOrientation::Upright,
                _ => sideways,
            }
        }
    }
}

struct Cluster {
    families: String,
    owner: u64,
    atomic: Option<u64>,
    bytes: std::ops::Range<usize>,
    runs: Vec<PaintGlyphRun>,
    advance: f64,
    block: f64,
    baseline: f64,
    rtl: bool,
    rank: usize,
}

fn shape_segment(params: VerticalSegmentInputs<'_>) -> Vec<Cluster> {
    let VerticalSegmentInputs {
        source,
        start,
        run,
        style,
        language,
        owner,
        orientation,
        font_key,
        rank,
        viewport,
    } = params;

    let font = run.font();
    let Ok(face) = harfrust::FontRef::from_index(font.data.data(), font.index) else {
        return Vec::new();
    };
    let data = harfrust::ShaperData::new(&face);
    let instance = harfrust::ShaperInstance::from_coords(
        &face,
        run.normalized_coords()
            .iter()
            .map(|c| harfrust::NormalizedCoord::from_bits(*c)),
    );
    let shaper = data.shaper(&face).instance(Some(&instance)).build();
    let features = text_features::resolve(style)
        .into_iter()
        .map(|f| harfrust::Feature::new(harfrust::Tag::new(&f.tag.to_bytes()), f.value as u32, ..))
        .collect::<Vec<_>>();
    let shape = |direction| {
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(source);
        buffer.guess_segment_properties();
        buffer.set_direction(direction);
        if let Some(language) =
            language.and_then(|l| l.to_string().parse::<harfrust::Language>().ok())
        {
            buffer.set_language(language);
        }
        shaper.shape(
            buffer,
            harfrust::ShapeOptions::new()
                .scale(Some((run.font_size() * 64.).round() as i32))
                .features(&features),
        )
    };
    let horizontal = if run.is_rtl() {
        harfrust::Direction::RightToLeft
    } else {
        harfrust::Direction::LeftToRight
    };
    let mut orientation = orientation;
    let mut shaped = shape(if orientation == PaintGlyphOrientation::Upright {
        harfrust::Direction::TopToBottom
    } else {
        horizontal
    });
    // Tr characters prefer a font's vertical alternate and rotate only when
    // none exists. Probe actual shaping output, not a character-name heuristic.
    let transformed = style_value(style, "text-orientation", "mixed") == "mixed"
        && !style_value(style, "writing-mode", "vertical-rl").starts_with("sideways")
        && source.chars().next().is_some_and(|c| {
            icu_properties::CodePointMapData::<icu_properties::props::VerticalOrientation>::new()
                .get(c)
                == icu_properties::props::VerticalOrientation::TransformedRotated
        });
    if transformed {
        let vertical = shape(harfrust::Direction::TopToBottom);
        if !vertical
            .glyph_infos()
            .iter()
            .map(|g| g.glyph_id)
            .eq(shaped.glyph_infos().iter().map(|g| g.glyph_id))
        {
            shaped = vertical;
            orientation = PaintGlyphOrientation::Upright;
        }
    }
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    let mut boundaries = infos
        .iter()
        .map(|info| info.cluster as usize)
        .chain(std::iter::once(source.len()))
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut groups = BTreeMap::<usize, Vec<(u32, harfrust::GlyphPosition)>>::new();
    for (info, pos) in infos.iter().zip(positions) {
        groups
            .entry(info.cluster as usize)
            .or_default()
            .push((info.glyph_id, *pos));
    }
    let mut result = Vec::new();
    for (offset, glyphs) in groups {
        let end = boundaries
            .iter()
            .copied()
            .find(|b| *b > offset)
            .unwrap_or(source.len());
        let mut pen = 0.;
        let mut output = Vec::new();
        for (id, pos) in glyphs {
            let (x, y, advance) = if orientation == PaintGlyphOrientation::Upright {
                (
                    pos.x_offset as f64 / 64.,
                    pen - pos.y_offset as f64 / 64.,
                    -(pos.y_advance as f64) / 64.,
                )
            } else {
                // Horizontal baseline offsets rotate around each physical glyph
                // origin in the renderer; their placement follows the inline axis.
                let clockwise = orientation == PaintGlyphOrientation::Clockwise;
                (
                    if clockwise {
                        pos.y_offset as f64 / 64.
                    } else {
                        -(pos.y_offset as f64) / 64.
                    },
                    pen + pos.x_offset as f64 / 64.,
                    pos.x_advance as f64 / 64.,
                )
            };
            output.push(PaintGlyph {
                source: None,
                id,
                x,
                y,
                advance,
            });
            pen += advance;
        }
        if source[offset..end].contains(['\n', '\u{2028}', '\u{2029}']) {
            output.clear();
            pen = 0.;
        }
        let glyph_run = PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: style
                .get("-oryn-text-background")
                .and_then(|color| oryn_common::color::parse_color(color)),
            decoration_rect: None,
            scale_x: 1.,
            vertical: true,
            decorations: super::text_decoration::read(style, f64::from(run.font_size()), viewport),
            orientation,
            embolden: style_value(style, "font-synthesis-weight", "auto") != "none"
                && run.synthesis().embolden(),
            skew_degrees: if style_value(style, "font-synthesis-style", "auto") != "none" {
                run.synthesis().skew().unwrap_or(0.)
            } else {
                0.
            },
            normalized_coords: run.normalized_coords().to_vec(),
            color: Some(
                oryn_common::color::parse_color(style_value(style, "color", "black"))
                    .unwrap_or([0, 0, 0, 255]),
            ),
            font_key: font_key.clone(),
            font_size: f64::from(run.font_size()),
            glyphs: output,
        };
        result.push(Cluster {
            families: style_value(style, "font-family", DEFAULT_FONT_FAMILY).into(),
            owner,
            atomic: None,
            bytes: start + offset..start + end,
            runs: vec![glyph_run],
            advance: pen.max(0.),
            block: f64::from(run.metrics().line_height.max(run.font_size())),
            baseline: match orientation {
                PaintGlyphOrientation::Upright => 0.,
                PaintGlyphOrientation::Clockwise => {
                    f64::from(run.metrics().descent - run.metrics().ascent) / 2.
                }
                PaintGlyphOrientation::CounterClockwise => {
                    f64::from(run.metrics().ascent - run.metrics().descent) / 2.
                }
            },
            rtl: orientation != PaintGlyphOrientation::Upright && run.is_rtl(),
            rank,
        });
    }
    result
}

pub(super) fn shape(
    engine: &mut TextEngine,
    text: &str,
    inline_size: f64,
    font_size: f64,
    style: &BTreeMap<String, String>,
    viewport: ViewportState,
    ranges: &[StyledTextRange<'_>],
) -> TextLayout {
    shape_impl(VerticalParagraphInputs {
        engine,
        text,
        inline_size,
        font_size,
        style,
        viewport,
        ranges,
        line_fragment: false,
    })
}
fn shape_impl(params: VerticalParagraphInputs<'_>) -> TextLayout {
    let VerticalParagraphInputs {
        engine,
        text,
        inline_size,
        font_size,
        style,
        viewport,
        ranges,
        line_fragment,
    } = params;

    let mut item_style = style.clone();
    item_style.remove("-oryn-first-line-end");
    if style_value(style, "text-orientation", "mixed") == "upright" {
        item_style.insert("direction".into(), "ltr".into());
    }
    let mut itemization = engine.paragraph_layout(text, font_size, &item_style, viewport, ranges);
    // Itemization retains atomic boundaries, but float placement belongs to the
    // vertical line-band pass below. Custom boxes yield without consuming their
    // item; explicitly acknowledge them so this probe always makes progress.
    {
        let mut breaker = itemization.break_lines();
        breaker.state_mut().set_layout_max_advance(f32::MAX);
        breaker.state_mut().set_line_max_advance(f32::MAX);
        while let Some(yielded) = breaker.break_next() {
            if let parley::layout::YieldData::InlineBoxBreak(data) = yielded {
                breaker
                    .state_mut()
                    .append_inline_box_to_line(data.advance, 0.);
            }
        }
        breaker.finish();
    }
    let mut clusters = Vec::new();
    let mut rank = 0;
    for line in itemization.lines() {
        for run in line.runs() {
            let font = run.font();
            let key = engine
                .font_keys
                .entry((font.data.id(), font.index))
                .or_insert_with(|| format!("{:x}:{}", Sha256::digest(font.data.data()), font.index))
                .clone();
            engine.paint_fonts.entry(key.clone()).or_insert_with(|| {
                oryn_common::v4::PaintFontResource {
                    bytes: font.data.data().to_vec(),
                    collection_index: font.index,
                }
            });
            let range = run.text_range();
            let source = &text[range.clone()];
            let mut segments = Vec::<(
                usize,
                usize,
                PaintGlyphOrientation,
                &BTreeMap<String, String>,
                Option<fontique::Language>,
                u64,
            )>::new();
            for (offset, grapheme) in source.grapheme_indices(true) {
                let start = range.start + offset;
                let (segment_style, language, owner) = ranges
                    .iter()
                    .rev()
                    .find(|(r, _, _, _)| r.contains(&start))
                    .map(|(_, s, l, owner)| (*s, *l, *owner))
                    .unwrap_or((style, None, 0));
                let orient = orientation(grapheme, segment_style);
                if let Some(last) = segments.last_mut() {
                    let property = icu_properties::CodePointMapData::<
                        icu_properties::props::VerticalOrientation,
                    >::new();
                    let same_class = text[last.0..last.1].chars().next().map(|c| property.get(c))
                        == grapheme.chars().next().map(|c| property.get(c));
                    if last.1 == start
                        && last.2 == orient
                        && same_class
                        && std::ptr::eq(last.3, segment_style)
                        && last.4 == language
                        && last.5 == owner
                    {
                        last.1 += grapheme.len();
                        continue;
                    }
                }
                segments.push((
                    start,
                    start + grapheme.len(),
                    orient,
                    segment_style,
                    language,
                    owner,
                ));
            }
            if run.is_rtl() {
                segments.reverse();
            }
            for (start, end, orient, segment_style, language, owner) in segments {
                clusters.extend(shape_segment(VerticalSegmentInputs {
                    source: &text[start..end],
                    start,
                    run: &run,
                    style: segment_style,
                    language,
                    owner,
                    orientation: orient,
                    font_key: key.clone(),
                    rank,
                    viewport,
                }));
                rank += 1;
            }
        }
    }
    // Combining is performed after full style/font itemization so fallback
    // boundaries do not split an element's one-em composition.
    let mut combined = Vec::<(std::ops::Range<usize>, &BTreeMap<String, String>, u64)>::new();
    if !style_value(style, "writing-mode", "vertical-rl").starts_with("sideways") {
        for (offset, ch) in text.char_indices() {
            let (local, owner) = ranges
                .iter()
                .rev()
                .find(|(range, _, _, _)| range.contains(&offset))
                .map(|(_, s, _, owner)| (*s, *owner))
                .unwrap_or((style, 0));
            if style_value(local, "text-combine-upright", "none") != "all"
                || matches!(ch, '\n' | '\u{2028}' | '\u{2029}')
            {
                continue;
            }
            if let Some((range, previous, previous_owner)) = combined.last_mut()
                && range.end == offset
                && std::ptr::eq(*previous, local)
                && *previous_owner == owner
            {
                range.end += ch.len_utf8();
                continue;
            }
            combined.push((offset..offset + ch.len_utf8(), local, owner));
        }
    }
    for (range, local, owner) in combined {
        let mut horizontal = local.clone();
        horizontal.insert("writing-mode".into(), "horizontal-tb".into());
        horizontal.insert("text-combine-upright".into(), "none".into());
        horizontal.insert("letter-spacing".into(), "0px".into());
        horizontal.insert("word-spacing".into(), "0px".into());
        horizontal.insert("text-wrap-mode".into(), "nowrap".into());
        horizontal.insert("text-indent".into(), "0px".into());
        horizontal.insert("text-align".into(), "left".into());
        let em = local
            .get("font-size")
            .map(|value| length(value, font_size, font_size, viewport))
            .unwrap_or(font_size);
        let language = ranges
            .iter()
            .find(|(source, _, _, _)| source.contains(&range.start))
            .and_then(|(_, _, language, _)| *language);
        let local_range = [(0..range.len(), &horizontal, language, owner)];
        let active_boxes = std::mem::take(&mut engine.active_inline_boxes);
        let saved_float_context = engine.float_context.take();
        let shaped = engine.shape_paragraph(crate::ParagraphShapeInputs {
            text: &text[range.clone()],
            width: 1e9,
            font_size: em,
            style: &horizontal,
            viewport,
            ranges: &local_range,
            first_paragraph: true,
        });
        engine.float_context = saved_float_context;
        engine.active_inline_boxes = active_boxes;
        let scale = if shaped.width > em {
            em / shaped.width
        } else {
            1.
        };
        let center = (em - shaped.width * scale) / 2.;
        let block = shaped.height.max(em);
        let mut runs = shaped.glyph_runs.into_iter().flatten().collect::<Vec<_>>();
        for run in &mut runs {
            run.vertical = true;
            run.scale_x = scale;
            run.orientation = PaintGlyphOrientation::Upright;
            for glyph in &mut run.glyphs {
                glyph.x = glyph.x * scale + center - em / 2.;
                glyph.y += (em - block) / 2.;
            }
        }
        let rank = clusters
            .iter()
            .filter(|c| c.bytes.start >= range.start && c.bytes.end <= range.end)
            .map(|c| c.rank)
            .min()
            .unwrap_or(usize::MAX);
        clusters.retain(|c| c.bytes.start < range.start || c.bytes.end > range.end);
        clusters.push(Cluster {
            families: style_value(local, "font-family", DEFAULT_FONT_FAMILY).into(),
            owner,
            atomic: None,
            bytes: range,
            runs,
            advance: em,
            block: em,
            baseline: 0.,
            rtl: false,
            rank,
        });
    }
    for item in &engine.active_inline_boxes {
        clusters.push(Cluster {
            families: String::new(),
            owner: item.id,
            atomic: Some(item.id),
            bytes: item.index..item.index,
            runs: Vec::new(),
            advance: if item.kind == parley::InlineBoxKind::InFlow {
                f64::from(item.width)
            } else {
                0.
            },
            block: f64::from(item.height),
            baseline: 0.,
            rtl: false,
            rank: clusters
                .iter()
                .filter(|cluster| cluster.bytes.start <= item.index)
                .map(|cluster| cluster.rank)
                .max()
                .unwrap_or(0),
        });
    }
    clusters.sort_by_key(|c| (c.bytes.start, c.atomic.is_none()));
    let mut options = icu_segmenter::options::LineBreakOptions::default();
    options.word_option = Some(match style_value(style, "word-break", "normal") {
        "break-all" => icu_segmenter::options::LineBreakWordOption::BreakAll,
        "keep-all" => icu_segmenter::options::LineBreakWordOption::KeepAll,
        _ => icu_segmenter::options::LineBreakWordOption::Normal,
    });
    let breaks = icu_segmenter::LineSegmenter::new_dictionary(options)
        .segment_str(text)
        .collect::<BTreeSet<_>>();
    let nowrap = line_fragment
        || style_value(
            style,
            "text-wrap-mode",
            if matches!(
                style_value(style, "white-space", "normal"),
                "nowrap" | "pre"
            ) {
                "nowrap"
            } else {
                "wrap"
            },
        ) == "nowrap";
    let emergency = matches!(
        style_value(style, "overflow-wrap", "normal"),
        "anywhere" | "break-word"
    ) || style_value(style, "word-break", "normal") == "break-word";
    let mut lines = Vec::new();
    let mut first = 0;
    let mut block_cursor = 0.;
    let mut float_placements = Vec::new();
    let mut placed = BTreeSet::new();
    let float_origin = engine.float_block_offset;
    let insets = engine.float_inline_insets.map(|value| value as f32);
    let mut float_context = if line_fragment {
        None
    } else {
        engine.float_context.take()
    };
    if let Some(context) = &mut float_context {
        context.set_width(engine.float_context_width.max(inline_size) as f32);
    }
    while first < clusters.len() {
        let mut required_block = font_size;
        let (end, line_x, line_y, line_width, line_block) = loop {
            let mut band = float_context.as_ref().map(|context| {
                float_lines::fragment_slot(
                    context,
                    float_origin + block_cursor,
                    required_block as f32,
                    insets,
                    engine.float_fragment,
                )
            });
            let mut end = first;
            let mut advance = 0.;
            let mut opportunity = None;
            let mut actual_block = font_size;
            while end < clusters.len() {
                let cluster = &clusters[end];
                if !line_fragment
                    && end > first
                    && style
                        .get("-oryn-first-line-end")
                        .and_then(|end| end.parse::<usize>().ok())
                        == Some(cluster.bytes.start)
                {
                    break;
                }
                if let Some((id, spec)) = cluster
                    .atomic
                    .and_then(|id| engine.inline_floats.get(&id).map(|spec| (id, *spec)))
                {
                    if placed.insert(id)
                        && let Some(context) = &mut float_context
                    {
                        let y = band
                            .map(|slot| slot.y)
                            .unwrap_or((float_origin + block_cursor) as f32);
                        let mut trial = context.clone();
                        let mut position =
                            spec.place(&mut trial, f64::from(y), insets, engine.float_fragment);
                        let narrowed = float_lines::fragment_slot(
                            &trial,
                            f64::from(y),
                            required_block as f32,
                            insets,
                            engine.float_fragment,
                        );
                        if advance > 0. && (f64::from(narrowed.width) < advance || narrowed.y > y) {
                            position = spec.place(
                                context,
                                f64::from(y) + required_block,
                                insets,
                                engine.float_fragment,
                            );
                        } else {
                            *context = trial;
                            band = Some(narrowed);
                        }
                        float_placements.push(InlinePlacement {
                            node: id,
                            rect: CssRect {
                                x: f64::from(position.x - insets[0]),
                                y: f64::from(position.y) - float_origin,
                                width: spec.width,
                                height: spec.height,
                            },
                            line: usize::MAX,
                        });
                    }
                    end += 1;
                    continue;
                }
                let available = band
                    .map(|slot| f64::from(slot.width))
                    .unwrap_or(inline_size);
                if !nowrap && advance + cluster.advance > available && end > first {
                    if let Some(boundary) = opportunity {
                        end = boundary;
                        break;
                    }
                    if emergency {
                        break;
                    }
                }
                advance += cluster.advance;
                actual_block = actual_block.max(cluster.block);
                end += 1;
                if breaks.contains(&cluster.bytes.end) {
                    opportunity = Some(end);
                }
                if text[cluster.bytes.clone()].contains(['\n', '\u{2028}', '\u{2029}']) {
                    break;
                }
            }
            if end == first {
                end += 1;
            }
            if actual_block > required_block + 0.0001 && float_context.is_some() {
                required_block = actual_block;
                continue;
            }
            break (
                end,
                band.map(|slot| f64::from(slot.x - insets[0])).unwrap_or(0.),
                band.map(|slot| f64::from(slot.y) - float_origin)
                    .unwrap_or(block_cursor),
                band.map(|slot| f64::from(slot.width))
                    .unwrap_or(inline_size),
                actual_block,
            );
        };
        lines.push((first..end, line_x, line_y, line_width, line_block));
        first = end;
        block_cursor = line_y + line_block;
    }
    if !line_fragment {
        engine.float_context = float_context;
    }
    if !line_fragment && lines.len() > 1 {
        // Re-itemize and reshape each chosen line with its actual boundaries.
        // Carrying paragraph glyph IDs across breaks gives incorrect joining,
        // ligatures and bidi end-of-line whitespace levels.
        let mut combined = TextLayout::default();
        let active_boxes = engine.active_inline_boxes.clone();
        for (indices, line_x, line_y, line_width, _) in lines {
            if indices.clone().all(|index| {
                clusters[index]
                    .atomic
                    .is_some_and(|id| engine.inline_floats.contains_key(&id))
            }) {
                continue;
            }
            let start = clusters[indices.start].bytes.start;
            let end = clusters[indices.end - 1].bytes.end;
            let line_ranges = ranges
                .iter()
                .filter_map(|(range, s, l, owner)| {
                    let a = range.start.max(start);
                    let b = range.end.min(end);
                    (a < b).then(|| (a - start..b - start, *s, *l, *owner))
                })
                .collect::<Vec<_>>();
            let included = indices
                .clone()
                .filter_map(|index| clusters[index].atomic)
                .collect::<BTreeSet<_>>();
            engine.active_inline_boxes = active_boxes
                .iter()
                .filter(|item| {
                    included.contains(&item.id) && !engine.inline_floats.contains_key(&item.id)
                })
                .cloned()
                .map(|mut item| {
                    item.index = item.index.saturating_sub(start);
                    item
                })
                .collect();
            let mut line = shape_impl(VerticalParagraphInputs {
                engine,
                text: &text[start..end],
                inline_size: line_width,
                font_size,
                style,
                viewport,
                ranges: &line_ranges,
                line_fragment: true,
            });
            let line_offset = combined.lines.len();
            for rect in &mut line.lines {
                rect.3 += line_y;
                rect.4 += line_x;
            }
            for cluster in &mut line.clusters {
                cluster.bytes.start += start;
                cluster.bytes.end += start;
                cluster.line += line_offset;
                cluster.rect.y += line_y;
                cluster.rect.x += line_x;
            }
            for item in &mut line.inline_boxes {
                if item.line != usize::MAX {
                    item.line += line_offset;
                }
                item.rect.y += line_y;
                item.rect.x += line_x;
            }
            combined.inline_boxes.extend(line.inline_boxes);
            combined.width = combined.width.max(line_x + line.width);
            combined.height = combined.height.max(line_y + line.height);
            combined.line_ranges.extend(
                line.line_ranges
                    .into_iter()
                    .map(|range| range.start + start..range.end + start),
            );
            for sources in &mut line.provenance {
                for source in sources {
                    source.bytes.start += start;
                    source.bytes.end += start;
                    for range in &mut source.glyph_ranges {
                        range.start += start;
                        range.end += start;
                    }
                }
            }
            combined.provenance.extend(line.provenance);
            combined.baselines.extend(line.baselines);
            combined.line_heights.extend(line.line_heights);
            combined.lines.extend(line.lines);
            combined.glyph_runs.extend(line.glyph_runs);
            combined.clusters.extend(line.clusters);
        }
        engine.active_inline_boxes = active_boxes;
        combined.inline_boxes.extend(float_placements);
        return combined;
    }
    let mut result = TextLayout::default();
    for (indices, line_x, line_y, line_width, _) in lines {
        if indices.clone().all(|index| {
            clusters[index]
                .atomic
                .is_some_and(|id| engine.inline_floats.contains_key(&id))
        }) {
            continue;
        }
        let mut visual = indices.clone().collect::<Vec<_>>();
        visual.sort_by_key(|i| {
            (
                clusters[*i].rank,
                if clusters[*i].rtl {
                    usize::MAX - clusters[*i].bytes.start
                } else {
                    clusters[*i].bytes.start
                },
            )
        });
        let block = visual
            .iter()
            .filter(|i| {
                !clusters[**i]
                    .atomic
                    .is_some_and(|id| engine.inline_floats.contains_key(&id))
            })
            .map(|i| clusters[*i].block)
            .fold(font_size, f64::max);
        let advance = visual.iter().map(|i| clusters[*i].advance).sum::<f64>();
        let rtl = style_value(style, "text-orientation", "mixed") != "upright"
            && style_value(style, "direction", "ltr") == "rtl";
        let free = (line_width - advance).max(0.);
        let left = line_x
            + match style_value(style, "text-align", "start") {
                "center" => free / 2.,
                "end" if !rtl => free,
                "start" if rtl => free,
                "right" => free,
                _ => 0.,
            };
        let mut pen = 0.;
        let mut runs = Vec::new();
        let mut provenance = Vec::new();
        let line_index = result.lines.len();
        for index in visual {
            let c = &clusters[index];
            if c.atomic
                .is_some_and(|id| engine.inline_floats.contains_key(&id))
            {
                continue;
            }
            if let Some(node) = c.atomic {
                result.inline_boxes.push(InlinePlacement {
                    node,
                    rect: CssRect {
                        x: left + pen,
                        y: line_y,
                        width: c.advance,
                        height: block,
                    },
                    line: line_index,
                });
            }
            for mut run in c.runs.clone() {
                run.decoration_rect = Some(CssRect {
                    x: 0.,
                    y: pen,
                    width: block,
                    height: c.advance,
                });
                for glyph in &mut run.glyphs {
                    glyph.x += block / 2. + c.baseline;
                    glyph.y += pen;
                }
                let glyph_ranges = vec![c.bytes.clone(); run.glyphs.len()];
                runs.push(run);
                provenance.push(RunProvenance {
                    families: c.families.clone(),
                    text_override: None,
                    owner: c.owner,
                    bytes: c.bytes.clone(),
                    glyph_ranges,
                });
            }
            result.clusters.push(text_source::TextCluster {
                bytes: c.bytes.clone(),
                line: line_index,
                rtl: c.rtl,
                rect: CssRect {
                    x: left + pen,
                    y: line_y,
                    width: c.advance,
                    height: block,
                },
            });
            pen += c.advance;
        }
        let bytes = clusters[indices.start].bytes.start..clusters[indices.end - 1].bytes.end;
        result.line_ranges.push(bytes.clone());
        result.baselines.push(block / 2.);
        result.line_heights.push(block);
        result
            .lines
            .push((text[bytes].to_string(), advance, block, line_y, left));
        result.provenance.push(provenance);
        result.glyph_runs.push(runs);
        result.width = result.width.max(left + advance);
        result.height = result.height.max(line_y + block);
    }
    result.inline_boxes.extend(float_placements);
    result
}

/// Convert logical line/cluster boxes to physical coordinates once the used
/// content box is known. Glyph baseline positions are already physical.
pub(super) fn physicalize(
    layout: &mut TextLayout,
    content_width: f64,
    inline_size: f64,
    mode: &str,
) {
    let right = matches!(mode, "vertical-rl" | "sideways-rl");
    let physical = |rect: CssRect| CssRect {
        x: if right {
            content_width - rect.y - rect.height
        } else {
            rect.y
        },
        y: if mode == "sideways-lr" {
            inline_size - rect.x - rect.width
        } else {
            rect.x
        },
        width: rect.height,
        height: rect.width,
    };
    if mode == "sideways-lr" {
        for (line, runs) in layout.lines.iter().zip(&mut layout.glyph_runs) {
            for run in runs {
                if let Some(box_style) = &mut run.line_box {
                    box_style.mirror_y(line.1);
                }
                if let Some(box_style) = &mut run.text_box {
                    box_style.mirror_y(line.1);
                }
                if let Some(rect) = &mut run.decoration_rect {
                    rect.y = line.1 - rect.bottom();
                }
                for glyph in &mut run.glyphs {
                    glyph.y = line.1 - glyph.y;
                }
            }
        }
    }
    for line in &mut layout.lines {
        let rect = physical(CssRect {
            x: line.4,
            y: line.3,
            width: line.1,
            height: line.2,
        });
        line.1 = rect.width;
        line.2 = rect.height;
        line.3 = rect.y;
        line.4 = rect.x;
    }
    for item in &mut layout.inline_boxes {
        item.rect = physical(item.rect);
    }
    for b in &mut layout.breaks {
        b.before = physical(b.before);
        b.after = b.after.map(physical);
    }
    for c in &mut layout.clusters {
        c.rect = physical(c.rect);
    }
    for c in &mut layout.source_clusters {
        c.rect = physical(c.rect);
    }
    std::mem::swap(&mut layout.width, &mut layout.height);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn itemization_acknowledges_custom_float_before_vertical_placement() {
        let mut engine = TextEngine::new(&[]);
        engine.active_inline_boxes.push(parley::InlineBox {
            id: 77,
            kind: parley::InlineBoxKind::CustomOutOfFlow,
            index: 0,
            width: 30.,
            height: 40.,
        });
        engine.inline_floats.insert(
            77,
            float_lines::FloatSpec {
                width: 30.,
                height: 40.,
                right: false,
                clear: taffy::Clear::None,
                monolithic: false,
            },
        );
        engine.float_context = Some(taffy::compute::FloatContext::new());
        engine.float_context_width = 100.;
        let style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("overflow-wrap".into(), "anywhere".into()),
        ]);
        let layout = shape(
            &mut engine,
            "abc",
            100.,
            16.,
            &style,
            ViewportState::default(),
            &[],
        );
        let floats = layout
            .inline_boxes
            .iter()
            .filter(|item| item.node == 77)
            .collect::<Vec<_>>();
        assert_eq!(floats.len(), 1);
        assert_eq!((floats[0].rect.width, floats[0].rect.height), (30., 40.));
        assert!(!layout.clusters.is_empty());
    }

    #[test]
    fn combined_text_is_one_em_horizontal_shaping_and_only_wide_text_compresses() {
        // Pinned compression-001, layout-rules-001, letter-spacing-001.
        let mut engine = TextEngine::new(&[]);
        let mut style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-combine-upright".into(), "all".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let one = engine.shape_styled("0", 200., 20., &style, ViewportState::default());
        assert_eq!(one.width, 20.);
        assert_eq!(one.clusters.len(), 1);
        assert_eq!(one.glyph_runs[0][0].scale_x, 1.);
        let many = engine.shape_styled("12345", 200., 20., &style, ViewportState::default());
        assert_eq!(many.width, 20.);
        assert_eq!(many.clusters.len(), 1);
        assert_eq!(many.clusters[0].bytes, 0..5);
        assert!(many.glyph_runs[0].iter().all(|run| run.vertical
            && run.orientation == PaintGlyphOrientation::Upright
            && run.scale_x < 1.));
        style.insert("letter-spacing".into(), "20px".into());
        let spaced = engine.shape_styled("12345", 200., 20., &style, ViewportState::default());
        assert_eq!(many.glyph_runs, spaced.glyph_runs);
        style.insert("writing-mode".into(), "sideways-rl".into());
        let sideways = engine.shape_styled("12345", 200., 20., &style, ViewportState::default());
        assert!(sideways.clusters.len() > 1);
    }

    #[test]
    fn unicode_orientation_is_grapheme_based_and_css_overrides_it() {
        let mut style = BTreeMap::from([("writing-mode".into(), "vertical-rl".into())]);
        assert_eq!(
            orientation("A\u{301}", &style),
            PaintGlyphOrientation::Clockwise
        );
        assert_eq!(orientation("漢", &style), PaintGlyphOrientation::Upright);
        assert_eq!(orientation("🙂", &style), PaintGlyphOrientation::Upright);
        style.insert("text-orientation".into(), "upright".into());
        assert_eq!(orientation("A", &style), PaintGlyphOrientation::Upright);
        style.insert("writing-mode".into(), "sideways-lr".into());
        assert_eq!(
            orientation("漢", &style),
            PaintGlyphOrientation::CounterClockwise
        );
    }

    #[test]
    fn upright_runs_use_vertical_advances_instead_of_horizontal_letter_widths() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-orientation".into(), "upright".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let text = engine.shape_styled("IW", 200., 20., &style, ViewportState::default());
        assert_eq!(text.lines.len(), 1);
        let advances = text.glyph_runs[0]
            .iter()
            .flat_map(|r| &r.glyphs)
            .map(|g| g.advance)
            .collect::<Vec<_>>();
        assert_eq!(advances.len(), 2);
        assert!(advances[0] > 0.);
        assert!(
            (advances[0] - advances[1]).abs() < 0.02,
            "vertical metrics must not use I/W horizontal widths"
        );
        assert!(
            text.glyph_runs[0]
                .iter()
                .all(|r| r.orientation == PaintGlyphOrientation::Upright)
        );
        assert_eq!(engine.paint_fonts.len(), 1);
    }

    #[test]
    fn intrinsic_inline_constraints_use_vertical_glyph_advances_and_break_opportunities() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-orientation".into(), "upright".into()),
            ("word-break".into(), "break-all".into()),
        ]);
        let source = text_source::TextSource::synthetic("IW");
        let (min, max) = engine.intrinsic_source(
            &source,
            20.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        let single = engine.shape_styled("I", 200., 20., &style, ViewportState::default());
        let full = engine.shape_styled("IW", 200., 20., &style, ViewportState::default());
        assert!((min - single.width).abs() < 0.02);
        assert!((max - full.width).abs() < 0.02);
        assert!(max > min);
    }

    #[test]
    fn provenance_ranges_inherit_resolved_paragraph_font_size() {
        for mode in ["horizontal-tb", "vertical-rl"] {
            for size in [12., 20., 32.] {
                let mut engine = TextEngine::new(&[]);
                let style = BTreeMap::from([
                    ("writing-mode".into(), mode.into()),
                    ("text-orientation".into(), "upright".into()),
                ]);
                let plain =
                    engine.shape_ranged("IW", 200., size, &style, ViewportState::default(), &[]);
                let owned = engine.shape_styled("IW", 200., size, &style, ViewportState::default());
                assert!(
                    (plain.width - owned.width).abs() < 0.02,
                    "{mode} size={size}: {} != {}",
                    plain.width,
                    owned.width
                );
                assert!(
                    owned
                        .glyph_runs
                        .iter()
                        .flatten()
                        .all(|run| (run.font_size - size).abs() < 0.01)
                );
                let explicit = BTreeMap::from([
                    ("font-size".into(), "18px".into()),
                    ("writing-mode".into(), mode.into()),
                    ("text-orientation".into(), "upright".into()),
                ]);
                let override_run = engine.shape_ranged(
                    "IW",
                    200.,
                    size,
                    &style,
                    ViewportState::default(),
                    &[(0..2, &explicit, None, 0)],
                );
                assert!(
                    override_run
                        .glyph_runs
                        .iter()
                        .flatten()
                        .all(|run| (run.font_size - 18.).abs() < 0.01)
                );
            }
        }
    }

    #[test]
    fn horizontal_line_origins_preserve_fractional_css_flow_advance() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre".into()),
            ("line-height".into(), "20.25px".into()),
        ]);
        let shaped = engine.shape_styled(
            "A\u{2028}B\u{2028}C",
            200.,
            16.,
            &style,
            ViewportState::default(),
        );
        assert_eq!(shaped.lines.len(), 3);
        for (index, line) in shaped.lines.iter().enumerate() {
            assert!(
                (line.3 - index as f64 * 20.25).abs() < 0.0001,
                "line={line:?}"
            );
            assert!((shaped.line_heights[index] - 20.25).abs() < 0.0001);
            assert!((shaped.baselines[index] - shaped.baselines[0]).abs() < 0.0001);
        }
        assert!((shaped.height - 60.75).abs() < 0.0001);
    }

    #[test]
    fn selected_line_boundaries_reshape_context_and_preserve_source_offsets() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-orientation".into(), "sideways".into()),
            ("word-break".into(), "break-all".into()),
            ("font-kerning".into(), "normal".into()),
        ]);
        let a = engine.shape_styled("A", 200., 24., &style, ViewportState::default());
        let v = engine.shape_styled("V", 200., 24., &style, ViewportState::default());
        let inline = a.width.max(v.width) + 0.01;
        let wrapped = engine.shape_styled("AVAV", inline, 24., &style, ViewportState::default());
        assert_eq!(wrapped.lines.len(), 4);
        for (index, runs) in wrapped.glyph_runs.iter().enumerate() {
            let expected = if index % 2 == 0 {
                &a.glyph_runs[0]
            } else {
                &v.glyph_runs[0]
            };
            let actual = runs
                .iter()
                .flat_map(|r| r.glyphs.iter().map(|g| (g.id, g.advance)))
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .flat_map(|r| r.glyphs.iter().map(|g| (g.id, g.advance)))
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "line-edge kerning must match isolated line shaping"
            );
            assert!(
                wrapped
                    .clusters
                    .iter()
                    .any(|c| c.line == index && c.bytes == (index..index + 1))
            );
        }
    }

    #[test]
    fn vertical_lines_wrap_at_native_metrics_and_physicalize_all_cluster_boxes() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-orientation".into(), "upright".into()),
            ("word-break".into(), "break-all".into()),
        ]);
        let mut text = engine.shape_styled("ABCD", 25., 20., &style, ViewportState::default());
        assert!(text.lines.len() >= 2);
        let original = text.clusters.clone();
        physicalize(&mut text, 100., 25., "vertical-rl");
        assert!(text.clusters[0].rect.x > text.clusters.last().unwrap().rect.x);
        for (before, after) in original.iter().zip(&text.clusters) {
            assert_eq!(before.bytes, after.bytes);
            assert_eq!(before.line, after.line);
            assert!((after.rect.height - before.rect.width).abs() < 0.02);
        }
    }
}
