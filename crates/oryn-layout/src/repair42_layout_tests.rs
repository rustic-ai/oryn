//! Repair42: exercise flow, selection, source and fragment coordinates separately.
use super::tests::{column_fixture, node};
use super::*;

fn explicit_image_input(prefix: &str, css: &str) -> LayoutInput {
    let mut input = column_fixture(css, vec![node(2, Some(1), "img", "")]);
    input.nodes[0].text = prefix.into();
    input.nodes[0].content = Some(vec![
        LayoutContent::Text {
            node_id: 10,
            text: prefix.into(),
        },
        LayoutContent::Element(2),
    ]);
    input.nodes[1].intrinsic_image_size = Some((200, 100));
    input
}

#[test]
fn svg_mask_type_export_preserves_native_cascade_and_importance() {
    assert!(style::properties::PropertyId::parse_enabled_for_all_content("mask-type").is_ok());
    let base = "https://oryn.invalid/";
    assert!(cssom::supports("mask-type", Some("alpha"), base));
    assert!(cssom::supports("mask-type", Some("luminance"), base));
    assert!(!cssom::supports("mask-type", Some("invalid"), base));
    assert!(
        cssom::supported_properties()
            .iter()
            .any(|(name, canonical)| name == "mask-type" && canonical == "mask-type")
    );
    let view =
        cssom::read_declaration("mask-type:alpha!important", "mask-type", base, false).unwrap();
    assert_eq!(view.value, "alpha");
    assert!(view.important);
    let mut mask = node(2, Some(1), "mask", "");
    mask.namespace = "http://www.w3.org/2000/svg".into();
    mask.attributes.insert("id".into(), "mask".into());
    for (css, inline, expected) in [
        ("mask{mask-type:alpha}", "", "alpha"),
        ("mask{mask-type:luminance}", "", "luminance"),
        (
            "mask{mask-type:alpha!important}",
            "mask-type:luminance",
            "alpha",
        ),
        (
            "mask{mask-type:luminance!important}",
            "mask-type:alpha!important",
            "alpha",
        ),
        ("", "", "luminance"),
    ] {
        let mut input = column_fixture(css, vec![mask.clone()]);
        input.nodes[0].tag = "svg".into();
        input.nodes[0].namespace = "http://www.w3.org/2000/svg".into();
        input.nodes[1]
            .attributes
            .insert("style".into(), inline.into());
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(
            styles[&2]["mask-type"], expected,
            "{css}; inline={inline}; {:?}",
            styles[&2]
        );
    }
}

#[test]
fn real_inline_box_after_final_preserved_break_keeps_its_line() {
    for prefix in ["\n", "\n  \n", "A\n"] {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre".into()),
            ("line-height".into(), "20px".into()),
        ]);
        let mut source = text_source::TextSource::default();
        source.append(1, Some(10), prefix);
        source.append_box(2);
        engine
            .inline_sizes
            .insert(2, (30., 30., parley::InlineBoxKind::InFlow));
        engine.inline_extents.insert(2, 30.);
        let shaped = engine.shape_source(
            &source,
            300.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        let preceding = prefix.chars().filter(|c| *c == '\n').count();
        assert_eq!(shaped.lines.len(), preceding + 1, "{prefix:?}: {shaped:?}");
        assert_eq!(shaped.inline_boxes.len(), 1, "{prefix:?}: {shaped:?}");
        let item = &shaped.inline_boxes[0];
        assert_eq!(item.line, preceding, "{shaped:?}");
        assert!(
            (item.rect.y - preceding as f64 * 20.).abs() < 0.01,
            "{shaped:?}"
        );
        assert!(shaped.height >= item.rect.bottom(), "{shaped:?}");
        assert_eq!(shaped.line_ranges.len(), shaped.lines.len());
        assert_eq!(shaped.line_heights.len(), shaped.lines.len());
        let caret = engine.shape_styled(prefix, 300., 16., &style, ViewportState::default());
        assert_eq!(
            caret.lines.len(),
            preceding,
            "caret-only terminal line: {caret:?}"
        );
        assert!(
            (caret.height - preceding as f64 * 20.).abs() < 0.01,
            "{caret:?}"
        );
    }
}

#[test]
fn preserved_breaks_before_inline_image_keep_native_source_and_geometry() {
    for mode in ["pre", "pre-wrap", "pre-line"] {
        let input = explicit_image_input(
            "\n  \n",
            &format!(
                "div{{width:400px;white-space:{mode};font-size:16px;line-height:20px}} img{{width:80px;height:auto}}"
            ),
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let image = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert_eq!((image.width, image.height), (80., 40.));
        assert!(
            (image.y - 40.).abs() < 0.01,
            "{mode}: image={image:?}, text={:?}, source={:?}, styles={:?}",
            output.text_fragments,
            output.text_clusters,
            output.computed_styles
        );
        assert!(
            output.text_clusters[&10]
                .iter()
                .any(|c| c.start == 0 && c.end >= 1),
            "{:?}",
            output.text_clusters
        );
        assert!(
            output.text_clusters[&10]
                .iter()
                .any(|c| c.start <= 3 && c.end == 4),
            "{:?}",
            output.text_clusters
        );
    }
}

#[test]
fn spaces_before_inline_and_block_images_use_their_own_formatting_axis() {
    for (prefix, transform) in [("\u{a0}", "none"), (" ", "full-width")] {
        for display in ["inline", "block"] {
            let input = explicit_image_input(
                prefix,
                &format!(
                    "div{{width:400px;font-size:16px;line-height:20px;text-transform:{transform}}}img{{display:{display};width:80px;height:auto}}"
                ),
            );
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let image = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            assert_eq!((image.width, image.height), (80., 40.));
            if display == "inline" {
                assert!(image.x > 0., "{prefix:?}/{transform}: {image:?}");
                assert!(image.y.abs() < 0.01, "{image:?}");
            } else {
                assert!(
                    (image.y - 20.).abs() < 0.01,
                    "{prefix:?}/{transform}: {image:?}, {:?}",
                    output.text_fragments
                );
                assert!(image.x.abs() < 0.01);
            }
            assert!(
                !output.text_clusters[&10].is_empty(),
                "preserved source must remain: {output:?}"
            );
        }
    }
    let input = explicit_image_input("\n  \n", "div{width:400px}img{width:80px;height:auto}");
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(
        output.geometry[&2].box_model.as_ref().unwrap().border_box.y,
        0.
    );
}

#[test]
fn spanner_eligibility_uses_the_shared_independent_context_boundary() {
    let root = node(1, None, "div", "");
    let section = node(2, Some(1), "section", "");
    let span = node(3, Some(2), "h3", "");
    let nodes = BTreeMap::from([(1, &root), (2, &section), (3, &span)]);
    for (property, value, eligible) in [
        ("display", "block", true),
        ("display", "flow-root", false),
        ("display", "flex", false),
        ("display", "grid", false),
        ("display", "table-cell", false),
        ("overflow-x", "hidden", false),
        ("overflow-x", "clip", true),
        ("contain", "layout", false),
        ("contain", "paint", false),
        ("column-count", "2", false),
        ("float", "left", false),
        ("position", "absolute", false),
    ] {
        let styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("column-count".into(), "2".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([(property.into(), value.into())])),
            ),
            (
                3,
                Arc::new(BTreeMap::from([("column-span".into(), "all".into())])),
            ),
        ]);
        assert_eq!(
            columns::eligible_spanner(1, 3, &nodes, &styles),
            eligible,
            "{property}:{value}"
        );
    }
}

#[test]
fn blocked_spanner_fragments_have_column_width_and_a_physical_union() {
    let mut section = node(3, Some(1), "section", "");
    section.children = vec![4];
    section.content = Some(vec![LayoutContent::Element(4)]);
    let mut input = column_fixture(
        "div{width:220px;height:40px;columns:2;column-gap:20px;column-fill:auto}p{margin:0;height:20px}section{display:flow-root}h3{margin:0;height:60px;column-span:all;white-space:pre;font:16px/20px sans-serif;orphans:1;widows:1}",
        vec![node(2, Some(1), "p", ""), section],
    );
    input.nodes.push(node(4, Some(3), "h3", "a\nb\nc"));
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    assert_eq!(output.computed_styles[&4]["orphans"], "1");
    assert_eq!(output.computed_styles[&4]["widows"], "1");
    let geometry = &output.geometry[&4];
    let rects = &geometry.client_rects;
    assert_eq!(rects.len(), 2, "{geometry:?}");
    assert!(
        rects.iter().all(|r| (r.width - 100.).abs() < 0.01),
        "{geometry:?}"
    );
    assert!(
        (rects[1].x - rects[0].x - 120.).abs() < 0.01,
        "{geometry:?}"
    );
    let union = geometry.box_model.as_ref().unwrap().border_box;
    assert!((union.width - 220.).abs() < 0.01, "{geometry:?}");
    assert_eq!(union.x, rects[0].x);
    assert_eq!(union.right(), rects[1].right());
    let nodes = input.nodes.iter().map(|node| (node.id, node)).collect();
    assert!(!columns::eligible_spanner(
        1,
        4,
        &nodes,
        &output.computed_styles
    ));
}

#[test]
fn oversized_replaced_float_preserves_letter_and_newline_geometry() {
    for side in ["left", "right"] {
        let text = vec!["A"; 10].join("\n");
        let mut input = column_fixture(
            &format!(
                "div{{width:200px;height:100px;columns:2;column-gap:0;column-fill:auto;white-space:pre;font:16px/20px sans-serif}}img{{float:{side};width:30px;height:150px}}"
            ),
            vec![node(2, Some(1), "img", "")],
        );
        input.nodes[0].content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 10,
                text: text.clone(),
            },
        ]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let origin = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let image = &output.geometry[&2];
        assert_eq!(image.client_rects.len(), 1, "{image:?}");
        assert_eq!(image.client_rects[0].height, 150.);
        let clusters = &output.text_clusters[&10];
        let letters = clusters
            .iter()
            .filter(|c| c.start % 2 == 0 && c.end == c.start + 1)
            .collect::<Vec<_>>();
        assert_eq!(letters.len(), 10, "{clusters:?}");
        for (index, letter) in letters.iter().enumerate() {
            let expected = if index < 5 {
                if side == "left" { 30. } else { 0. }
            } else {
                100.
            };
            assert!(
                (letter.rect.x - origin.x - expected).abs() < 0.01,
                "{side} source={text:?}: {clusters:?}"
            );
        }
        let newlines = clusters
            .iter()
            .filter(|c| c.start % 2 == 1 && c.end == c.start + 1)
            .collect::<Vec<_>>();
        assert_eq!(newlines.len(), 9, "{clusters:?}");
        for newline in newlines {
            let letter = letters.iter().find(|c| c.end == newline.start).unwrap();
            assert_eq!(newline.line, letter.line);
            assert!(
                (newline.rect.x - letter.rect.right()).abs() < 0.01,
                "{clusters:?}"
            );
        }
        assert_eq!(
            clusters
                .iter()
                .flat_map(|c| c.start..c.end)
                .collect::<BTreeSet<_>>(),
            (0..19).collect()
        );
    }
}

fn table_input(css: &str, values: &[&str]) -> LayoutInput {
    let mut table = node(1, None, "table", "");
    table.children = vec![2];
    let mut row = node(2, Some(1), "tr", "");
    row.children = (0..values.len()).map(|i| 3 + i as u64).collect();
    let mut input = column_fixture(css, Vec::new());
    input.nodes = vec![table, row];
    input
        .nodes
        .extend(values.iter().enumerate().map(|(i, text)| {
            let mut cell = node(3 + i as u64, Some(2), "td", text);
            cell.content = Some(vec![LayoutContent::Text {
                node_id: 100 + i as u64,
                text: (*text).into(),
            }]);
            cell
        }));
    input
}

#[test]
fn table_alignment_moves_flow_selection_and_source_by_one_offset() {
    for mode in ["horizontal-tb", "vertical-lr", "vertical-rl"] {
        let input = table_input(
            &format!(
                "table{{width:360px;height:360px;border-spacing:0;writing-mode:{mode}}}td{{width:100px;height:100px;padding:0;font:16px/20px sans-serif;text-orientation:upright;vertical-align:top}}td:nth-child(2){{vertical-align:middle}}td:nth-child(3){{vertical-align:bottom}}"
            ),
            &["A", "A", "A"],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let vertical = mode != "horizontal-tb";
        for (index, ratio) in [(0, 0.), (1, 0.5), (2, 1.)] {
            let id = 3 + index as u64;
            let projection = &output.paint_projections[&id];
            let content = projection.box_model.content_box;
            let fragment = &projection.text[0];
            let flow = fragment.flow_rect.unwrap();
            let available = if vertical {
                content.width
            } else {
                content.height
            };
            let extent = if vertical { flow.width } else { flow.height };
            let before = if mode == "vertical-rl" {
                content.right() - flow.right()
            } else if vertical {
                flow.x - content.x
            } else {
                flow.y - content.y
            };
            assert!(
                (before - (available - extent).max(0.) * ratio).abs() < 0.01,
                "{mode}: id={id}, content={content:?}, flow={flow:?}, selection={:?}",
                fragment.rect
            );
            let source = &output.text_clusters[&(100 + index as u64)][0];
            let painted = &fragment.clusters[0];
            assert_eq!(
                source.rect, painted.rect,
                "{mode}: source/paint must move together"
            );
            assert_eq!(
                projection.box_model.border_box,
                output.geometry[&id].box_model.as_ref().unwrap().border_box
            );
        }
    }
}

#[test]
fn baseline_row_contains_flow_but_keeps_native_negative_leading() {
    for leading in ["normal", "20px"] {
        let input = table_input(
            &format!(
                "table{{width:300px;border-spacing:0}}td{{padding:0;vertical-align:baseline;font-size:16px;line-height:{leading}}}td+td{{font-size:32px;line-height:{};padding-top:8px}}",
                if leading == "normal" {
                    "normal"
                } else {
                    "40px"
                }
            ),
            &["Ag", "Ag"],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let row = output.paint_projections[&2].box_model.border_box;
        let baseline = |id| {
            let f = &output.paint_projections[&id].text[0];
            f.rect.y + f.glyph_runs[0].glyphs[0].y
        };
        assert!((baseline(3) - baseline(4)).abs() < 0.01);
        for id in [3, 4] {
            let f = &output.paint_projections[&id].text[0];
            assert!(
                f.flow_rect.unwrap().bottom() <= row.bottom() + 0.01,
                "{leading}: {row:?} {f:?}"
            );
            assert_eq!(f.rect.height, output.text_fragments[&id][0].rect.height);
        }
        if leading == "normal" {
            assert!(
                row.height > 48.,
                "native normal metrics grow the row: {row:?}"
            );
        } else {
            assert!((row.height - 48.).abs() < 0.01, "{row:?}");
            let f = &output.paint_projections[&4].text[0];
            assert!(f.rect.height > f.flow_rect.unwrap().height, "{f:?}");
        }
    }
}

#[test]
fn table_alignment_uses_lines_owned_by_nested_inline_descendants() {
    let mut input = table_input(
        "table{width:300px;border-spacing:0}td{height:100px;padding:0;font:16px/20px sans-serif;vertical-align:top}td:nth-child(2){vertical-align:middle}td:nth-child(3){vertical-align:bottom}span{color:red}em{font-style:normal;color:blue}",
        &["", "", ""],
    );
    for (index, id) in [3, 4, 5].into_iter().enumerate() {
        let span = 20 + index as u64 * 2;
        let leaf = span + 1;
        let cell = input.nodes.iter_mut().find(|node| node.id == id).unwrap();
        cell.children = vec![span];
        cell.content = Some(vec![LayoutContent::Element(span)]);
        let mut outer = node(span, Some(id), "span", "");
        outer.children = vec![leaf];
        outer.content = Some(vec![LayoutContent::Element(leaf)]);
        let mut inner = node(leaf, Some(span), "em", "A");
        inner.content = Some(vec![LayoutContent::Text {
            node_id: 200 + index as u64,
            text: "A".into(),
        }]);
        input.nodes.extend([outer, inner]);
    }
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    let first = output.paint_projections[&21].text[0].flow_rect.unwrap();
    for (id, shift) in [(21, 0.), (23, 40.), (25, 80.)] {
        let f = &output.paint_projections[&id].text[0];
        assert!(
            (f.flow_rect.unwrap().y - first.y - shift).abs() < 0.01,
            "id={id}: {f:?}"
        );
        assert!(f.rect.height > f.flow_rect.unwrap().height);
    }
    for (source, leaf) in [(200, 21), (201, 23), (202, 25)] {
        assert_eq!(
            output.text_clusters[&source][0].rect,
            output.paint_projections[&leaf].text[0].clusters[0].rect
        );
    }
}

#[test]
fn upright_table_preserves_normal_word_overflow_and_explicit_wrap() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-lr"] {
        // Sideways writing uses horizontal advances despite text-orientation:
        // upright. Retain the original fitting word and add an overflowing word
        // so every mode exercises both normal overflow and explicit wrapping.
        for text in ["ABCD", "ABCDABCD"] {
            let overflows = mode != "sideways-lr" || text == "ABCDABCD";
            for wrap in ["normal", "break-all"] {
                let input = table_input(
                    &format!(
                        "table{{height:80px;table-layout:fixed;border-collapse:collapse;writing-mode:{mode}}}td{{height:80px;width:80px;padding:0;font:20px/20px sans-serif;text-orientation:upright;word-break:{wrap};border:2px solid red}}"
                    ),
                    &[text],
                );
                let output = compute(&input, &LayoutLimits::default()).unwrap();
                let content = output.paint_projections[&3].box_model.content_box;
                let fragments = &output.paint_projections[&3].text;
                assert_eq!(
                    fragments
                        .iter()
                        .map(|f| f.text.as_str())
                        .collect::<String>(),
                    text,
                    "{mode}/{wrap}/{text}: {fragments:?}"
                );
                let clusters = &output.text_clusters[&100];
                assert_eq!(
                    clusters
                        .iter()
                        .flat_map(|c| c.start..c.end)
                        .collect::<BTreeSet<_>>(),
                    (0..text.encode_utf16().count()).collect()
                );
                if wrap == "normal" {
                    assert_eq!(fragments.len(), 1, "{fragments:?}");
                    if overflows {
                        assert!(
                            fragments[0].rect.height > content.height,
                            "native unbreakable word may overflow: {content:?}, {fragments:?}"
                        );
                    } else {
                        assert!(
                            fragments[0].rect.height <= content.height + 0.01,
                            "short sideways word fits its native advances: {content:?}, {fragments:?}"
                        );
                    }
                } else {
                    if overflows {
                        assert!(fragments.len() > 1, "{fragments:?}");
                    } else {
                        assert_eq!(fragments.len(), 1, "{fragments:?}");
                    }
                    assert!(
                        fragments
                            .iter()
                            .all(|f| f.rect.height <= content.height + 0.01),
                        "{content:?}, {fragments:?}"
                    );
                }
                let expected_orientation = if mode == "sideways-lr" {
                    oryn_common::v4::PaintGlyphOrientation::CounterClockwise
                } else {
                    oryn_common::v4::PaintGlyphOrientation::Upright
                };
                assert!(
                    fragments
                        .iter()
                        .flat_map(|f| &f.glyph_runs)
                        .all(|run| run.orientation == expected_orientation),
                    "{mode}/{wrap}/{text}: {fragments:?}"
                );
                assert!(
                    fragments
                        .iter()
                        .flat_map(|f| &f.glyph_runs)
                        .flat_map(|run| &run.glyphs)
                        .all(|glyph| glyph.source.is_some())
                );
            }
        }
    }
}

#[test]
fn fragmentation_line_limits_use_native_positive_integer_cascade() {
    let mut section = node(2, Some(1), "section", "");
    section.children = vec![3];
    section.content = Some(vec![LayoutContent::Element(3)]);
    let mut paragraph = node(3, Some(2), "p", "");
    paragraph.children = vec![4];
    paragraph.content = Some(vec![LayoutContent::Element(4)]);
    let mut input = column_fixture(
        "section{orphans:1;widows:3}p{orphans:5!important;widows:4}p{orphans:0!important;widows:-1}p{orphans:7;widows:1.5}",
        vec![section],
    );
    input
        .nodes
        .extend([paragraph, node(4, Some(3), "span", "text")]);
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    for (id, orphans, widows) in [(1, "2", "2"), (2, "1", "3"), (3, "5", "4"), (4, "5", "4")] {
        assert_eq!(output.computed_styles[&id]["orphans"], orphans, "id={id}");
        assert_eq!(output.computed_styles[&id]["widows"], widows, "id={id}");
    }
    input.stylesheets.push(
        "section{orphans:initial;widows:unset}p{orphans:inherit!important;widows:initial}".into(),
    );
    let output = compute(&input, &LayoutLimits::default()).unwrap();
    for id in [1, 2, 3, 4] {
        assert_eq!(output.computed_styles[&id]["orphans"], "2", "id={id}");
        assert_eq!(output.computed_styles[&id]["widows"], "2", "id={id}");
    }
}
