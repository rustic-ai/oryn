//! Emphasis is generated paint from native font shaping. Annotation glyphs have
//! empty source provenance; DOM text and selection clusters never acquire marks.
use super::*;
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
};
use unicode_segmentation::UnicodeSegmentation;

fn mark(value: &str, vertical: bool) -> Option<String> {
    let value = value.trim();
    if value == "none" || value.is_empty() {
        return None;
    }
    if value.starts_with(['"', '\'']) {
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let string = parser.expect_string().ok()?.as_ref().to_owned();
        if parser.expect_exhausted().is_err() {
            return None;
        }
        return string
            .graphemes(true)
            .next()
            .filter(|mark| !mark.is_empty())
            .map(str::to_owned);
    }
    let open = value.split_whitespace().any(|part| part == "open");
    let shape = value
        .split_whitespace()
        .find(|part| {
            matches!(
                *part,
                "dot" | "circle" | "double-circle" | "triangle" | "sesame"
            )
        })
        .unwrap_or(if vertical { "sesame" } else { "circle" });
    Some(
        match (shape, open) {
            ("dot", false) => "\u{2022}",
            ("dot", true) => "\u{25e6}",
            ("circle", false) => "\u{25cf}",
            ("circle", true) => "\u{25cb}",
            ("double-circle", false) => "\u{25c9}",
            ("double-circle", true) => "\u{25ce}",
            ("triangle", false) => "\u{25b2}",
            ("triangle", true) => "\u{25b3}",
            ("sesame", false) => "\u{fe45}",
            ("sesame", true) => "\u{fe46}",
            _ => return None,
        }
        .into(),
    )
}
fn eligible(text: &str, style: &BTreeMap<String, String>) -> bool {
    if text.is_empty()||text.chars().all(|ch|ch.is_control()||matches!(ch,'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}')){return false;}
    let skip = style_value(style, "text-emphasis-skip", "spaces");
    if skip.split_whitespace().any(|part| part == "spaces") && text.chars().all(char::is_whitespace)
    {
        return false;
    }
    !(skip.split_whitespace().any(|part| part == "punctuation")
        && text.chars().all(|ch| {
            icu_properties::props::GeneralCategoryGroup::Punctuation.contains(
                icu_properties::CodePointMapData::<icu_properties::props::GeneralCategory>::new()
                    .get(ch),
            )
        }))
}

pub(super) fn apply(
    engine: &mut TextEngine,
    layout: &mut TextLayout,
    text: &str,
    base: &BTreeMap<String, String>,
    styles: &ComputedStyles,
    viewport: ViewportState,
) {
    let vertical = style_value(base, "writing-mode", "horizontal-tb") != "horizontal-tb";
    let mut cache = BTreeMap::<(u64, String), TextLayout>::new();
    let mut added = Vec::new();
    for cluster in &layout.clusters {
        let Some(content) = text.get(cluster.bytes.clone()) else {
            continue;
        };
        let Some(sources) = layout.provenance.get(cluster.line) else {
            continue;
        };
        let Some((source, run)) =
            sources
                .iter()
                .zip(&layout.glyph_runs[cluster.line])
                .find(|(source, _)| {
                    source.bytes.start <= cluster.bytes.start
                        && source.bytes.end >= cluster.bytes.end
                        && !source.bytes.is_empty()
                })
        else {
            continue;
        };
        let style = styles
            .get(&source.owner)
            .map(|style| style.as_ref())
            .unwrap_or(base);
        if !eligible(content, style) {
            continue;
        }
        let Some(mark) = mark(style_value(style, "text-emphasis-style", "none"), vertical) else {
            continue;
        };
        let key = (source.owner, mark.clone());
        if !cache.contains_key(&key) {
            let size = run.font_size / 2.;
            if !size.is_finite() || size <= 0. {
                continue;
            }
            let mut local = style.clone();
            for key in [
                text_pseudos::KEY,
                "-oryn-first-line-end",
                "-oryn-first-letter-start",
                "-oryn-first-letter-end",
            ] {
                local.remove(key);
            }
            for (key, value) in [
                ("writing-mode", "horizontal-tb"),
                ("white-space", "nowrap"),
                ("text-align", "left"),
                ("direction", "ltr"),
                ("text-indent", "0px"),
                ("text-combine-upright", "none"),
                ("text-emphasis-style", "none"),
                ("text-decoration-line", "none"),
                ("-oryn-paint-decorations", "[]"),
                ("letter-spacing", "0px"),
                ("word-spacing", "0px"),
                ("line-height", "1"),
            ] {
                local.insert(key.into(), value.into());
            }
            local.insert("font-size".into(), format!("{size}px"));
            let color = style_value(style, "text-emphasis-color", "currentcolor");
            if !color.eq_ignore_ascii_case("currentcolor") {
                local.insert("color".into(), color.into());
            }
            let boxes = std::mem::take(&mut engine.active_inline_boxes);
            let shaped = engine.shape_paragraph(&mark, 1e6, size, &local, viewport, &[], true);
            engine.active_inline_boxes = boxes;
            cache.insert(key.clone(), shaped);
        }
        let shaped = &cache[&key];
        let Some(mark_line) = shaped.lines.first() else {
            continue;
        };
        let line = &layout.lines[cluster.line];
        let inline = cluster.rect.x - line.4 + (cluster.rect.width - shaped.width) / 2.;
        let position = style_value(style, "text-emphasis-position", "over right");
        let (x, y) = if vertical {
            let right = !position.split_whitespace().any(|part| part == "left");
            (
                if right { line.2 } else { -shaped.width },
                cluster.rect.x - line.4 + (cluster.rect.width - mark_line.2) / 2.,
            )
        } else {
            let baseline = run
                .glyphs
                .first()
                .map(|glyph| glyph.y)
                .unwrap_or(line.2 * 0.8);
            let metrics = engine
                .paint_fonts
                .get(&run.font_key)
                .and_then(|resource| {
                    FontRef::from_index(&resource.bytes, resource.collection_index).ok()
                })
                .map(|font| {
                    let coords = run
                        .normalized_coords
                        .iter()
                        .map(|coord| NormalizedCoord::from_bits(*coord))
                        .collect::<Vec<_>>();
                    let metrics =
                        font.metrics(Size::new(run.font_size as f32), LocationRef::new(&coords));
                    (f64::from(metrics.ascent), f64::from(metrics.descent).abs())
                })
                .unwrap_or((run.font_size * 0.8, run.font_size * 0.2));
            let under = position.split_whitespace().any(|part| part == "under");
            (
                inline,
                if under {
                    baseline + metrics.1
                } else {
                    baseline - metrics.0 - mark_line.2
                },
            )
        };
        for mut mark_run in shaped.glyph_runs.first().cloned().unwrap_or_default() {
            for glyph in &mut mark_run.glyphs {
                glyph.x += x;
                glyph.y += y;
            }
            mark_run.line_box = None;
            mark_run.text_box = None;
            mark_run.background = None;
            mark_run.decorations.clear();
            mark_run.vertical = vertical;
            mark_run.decoration_rect = Some(CssRect {
                x,
                y,
                width: shaped.width,
                height: mark_line.2,
            });
            added.push((
                cluster.line,
                source.owner,
                cluster.bytes.clone(),
                mark_run,
                style_value(style, "font-family", DEFAULT_FONT_FAMILY).to_owned(),
                mark.clone(),
            ));
        }
    }
    let mut extensions = vec![(0f64, 0f64); layout.lines.len()];
    let reverse = matches!(
        style_value(base, "writing-mode", "horizontal-tb"),
        "vertical-rl" | "sideways-rl"
    );
    for (line, owner, bytes, run, families, mark) in added {
        if let Some(rect) = run.decoration_rect {
            let first = if vertical { rect.x } else { rect.y };
            let last = if vertical {
                rect.right()
            } else {
                rect.bottom()
            };
            let before = (-first).max(0.);
            let after = (last - layout.lines[line].2).max(0.);
            let (before, after) = if vertical && reverse {
                (after, before)
            } else {
                (before, after)
            };
            extensions[line].0 = extensions[line].0.max(before);
            extensions[line].1 = extensions[line].1.max(after);
        }
        let glyph_ranges = vec![bytes.clone(); run.glyphs.len()];
        layout.glyph_runs[line].push(run);
        layout.provenance[line].push(RunProvenance {
            families,
            text_override: Some(mark),
            owner,
            bytes: bytes.start..bytes.start,
            glyph_ranges,
        });
    }
    let mut shifts = Vec::new();
    let mut cumulative = 0.;
    for (index, (before, after)) in extensions.into_iter().enumerate() {
        shifts.push(cumulative + before);
        layout.lines[index].3 += cumulative;
        layout.lines[index].2 += before + after;
        let physical_before = if vertical && reverse { after } else { before };
        if let Some(baseline) = layout.baselines.get_mut(index) {
            *baseline += physical_before;
        }
        for run in &mut layout.glyph_runs[index] {
            for glyph in &mut run.glyphs {
                if vertical {
                    glyph.x += physical_before
                } else {
                    glyph.y += physical_before
                }
            }
            if let Some(rect) = &mut run.decoration_rect {
                if vertical {
                    rect.x += physical_before
                } else {
                    rect.y += physical_before
                }
            }
        }
        cumulative += before + after;
    }
    for cluster in &mut layout.clusters {
        cluster.rect.y += shifts[cluster.line];
    }
    for item in &mut layout.inline_boxes {
        if let Some(shift) = shifts.get(item.line) {
            item.rect.y += shift;
        }
    }
    layout.height += cumulative;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_mark_shapes_and_strings() {
        assert_eq!(mark("open dot", false).as_deref(), Some("\u{25e6}"));
        assert_eq!(mark("filled", true).as_deref(), Some("\u{fe45}"));
        assert_eq!(mark("\"ab\"", false).as_deref(), Some("a"));
        assert!(mark("none", false).is_none());
        assert_eq!(mark(r#""\25cf ""#, false).as_deref(), Some("●"));
    }
    #[test]
    fn native_emphasis_runs_do_not_modify_source_text_or_clusters() {
        // Pinned css/css-text-decor/text-emphasis-style-012.html.
        let mut engine = TextEngine::new(&[]);
        let plain = BTreeMap::from([("font-size".into(), "20px".into())]);
        let unmarked = engine.shape_styled("ab c", 300., 20., &plain, ViewportState::default());
        let mut style = plain.clone();
        style.insert("text-emphasis-style".into(), "open dot".into());
        let marked = engine.shape_styled("ab c", 300., 20., &style, ViewportState::default());
        assert_eq!(
            unmarked
                .lines
                .iter()
                .map(|line| &line.0)
                .collect::<Vec<_>>(),
            marked.lines.iter().map(|line| &line.0).collect::<Vec<_>>()
        );
        assert!(marked.height >= unmarked.height);
        assert_eq!(unmarked.clusters.len(), marked.clusters.len());
        assert_eq!(
            marked.provenance[0]
                .iter()
                .filter(|source| source.bytes.is_empty())
                .count(),
            3
        );
        assert_eq!(marked.lines[0].0, "ab c");
        assert!(marked.glyph_runs[0].len() > unmarked.glyph_runs[0].len());
    }
    #[test]
    fn under_and_vertical_right_marks_use_native_owner_paint_without_copy_text() {
        // Pinned text-emphasis-position-under-right-001.xht.
        let mut engine = TextEngine::new(&[]);
        let mut style = BTreeMap::from([
            ("font-size".into(), "20px".into()),
            ("text-emphasis-style".into(), "filled circle".into()),
            ("text-emphasis-position".into(), "under right".into()),
        ]);
        let horizontal = engine.shape_styled("abc", 300., 20., &style, ViewportState::default());
        let marks = horizontal.glyph_runs[0]
            .iter()
            .zip(&horizontal.provenance[0])
            .filter(|(_, source)| source.bytes.is_empty())
            .map(|(run, _)| run.decoration_rect.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(marks.len(), 3);
        let baseline = horizontal.glyph_runs[0][0].glyphs[0].y;
        assert!(marks.iter().all(|rect| rect.y > baseline));
        let styles = BTreeMap::from([(1, Arc::new(style.clone()))]);
        let paint = inline_formatting::paint_fragments(&horizontal, 1, 0., 0., &styles);
        assert_eq!(
            paint[&1]
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            "abc"
        );
        assert_eq!(
            paint[&1]
                .iter()
                .filter(|fragment| fragment.text.is_empty())
                .count(),
            3
        );
        style.insert("writing-mode".into(), "vertical-rl".into());
        let vertical = engine.shape_styled("abc", 300., 20., &style, ViewportState::default());
        assert!(vertical.glyph_runs[0].iter().all(|run| run.vertical));
        assert_eq!(vertical.clusters.len(), 3);
    }
}
