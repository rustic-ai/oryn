//! SVG semantic text uses the same positioned clusters and glyph paint as resvg.
use super::*;
use crate::print::text_layer::{PdfCluster, PdfText};

pub(super) fn collect(
    snapshot: &PaintSnapshot,
    tree: &resvg::usvg::Tree,
    scale: f64,
    resource: usize,
) -> Result<Vec<PdfText>, RenderError> {
    let normalized = tree.to_string(&resvg::usvg::WriteOptions {
        preserve_text: false,
        ..Default::default()
    });
    if normalized.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "SVG semantic source exceeds bounds".into(),
        ));
    }
    let xml = roxmltree::Document::parse(&normalized)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let mut output = Vec::new();
    let mut budget = crate::svg_coverage::Budget::default();
    fn walk(
        group: &resvg::usvg::Group,
        tree: &resvg::usvg::Tree,
        snapshot: &PaintSnapshot,
        xml: &roxmltree::Document<'_>,
        scale: f64,
        budget: &mut crate::svg_coverage::Budget,
        output: &mut Vec<PdfText>,
        resource: usize,
        depth: usize,
    ) -> Result<(), RenderError> {
        if depth > 128 {
            return Err(RenderError::InvalidPrint("SVG semantic tree depth".into()));
        }
        if group.opacity().get() <= 0. {
            return Ok(());
        }
        for node in group.children() {
            match node {
                resvg::usvg::Node::Group(group) => walk(
                    group,
                    tree,
                    snapshot,
                    xml,
                    scale,
                    budget,
                    output,
                    resource,
                    depth + 1,
                )?,
                resvg::usvg::Node::Text(text) => {
                    let source = text
                        .chunks()
                        .iter()
                        .map(|chunk| chunk.text())
                        .collect::<String>();
                    let mut ranges = std::collections::BTreeMap::new();
                    for span in text.layouted().iter().filter(|span| span.visible) {
                        for glyph in &span.positioned_glyphs {
                            ranges
                                .entry((glyph.source.start, glyph.source.end))
                                .or_insert((glyph.rtl, glyph.cluster_quad));
                        }
                    }
                    let mut accepted = Vec::new();
                    for ((start, end), (rtl, native_quad)) in ranges {
                        let cluster = source.get(start..end).ok_or_else(|| {
                            RenderError::InvalidPrint(
                                "native SVG glyph byte range is invalid".into(),
                            )
                        })?;
                        if cluster.trim().is_empty()
                            || !crate::svg_coverage::source_allowed(
                                resource,
                                text.flattened().id(),
                                start,
                                end,
                            )
                        {
                            continue;
                        }
                        budget.cluster()?;
                        let Some(selected) = tree.text_cluster_tree(text, start..end) else {
                            continue;
                        };
                        let mut prefix = format!("oryn-svg-cluster-{start}-{end}_");
                        while xml
                            .descendants()
                            .filter_map(|node| node.attribute("id"))
                            .any(|id| id.starts_with(&prefix))
                        {
                            prefix.push('_');
                        }
                        let glyphs = selected.to_string(&resvg::usvg::WriteOptions {
                            id_prefix: Some(prefix),
                            preserve_text: false,
                            ..Default::default()
                        });
                        if !crate::svg_coverage::covered(
                            snapshot,
                            xml,
                            text.flattened().id(),
                            &glyphs,
                            scale,
                            budget,
                        )? {
                            continue;
                        }
                        let t = text.abs_transform();
                        let quad = native_quad.map(|[x, y]| {
                            (
                                f64::from(t.sx * x + t.kx * y + t.tx),
                                f64::from(t.ky * x + t.sy * y + t.ty),
                            )
                        });
                        if !quad.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
                            return Err(RenderError::InvalidPrint(
                                "native SVG glyph geometry is nonfinite".into(),
                            ));
                        }
                        accepted.push((start, end, quad, rtl));
                    }
                    // Native source order survives bidi visual ordering. Whitespace is retained
                    // only between surviving clusters, without inventing a glyph for the gap.
                    let mut current: Option<(usize, usize, Vec<PdfCluster>)> = None;
                    let flush = |current: Option<(usize, usize, Vec<PdfCluster>)>,
                                 output: &mut Vec<PdfText>| {
                        if let Some((start, end, clusters)) = current {
                            let x = clusters
                                .iter()
                                .flat_map(|cluster| cluster.quad.iter().map(|p| p.0))
                                .fold(f64::INFINITY, f64::min);
                            let y = clusters
                                .iter()
                                .flat_map(|cluster| cluster.quad.iter().map(|p| p.1))
                                .fold(f64::INFINITY, f64::min);
                            let right = clusters
                                .iter()
                                .flat_map(|cluster| cluster.quad.iter().map(|p| p.0))
                                .fold(f64::NEG_INFINITY, f64::max);
                            let bottom = clusters
                                .iter()
                                .flat_map(|cluster| cluster.quad.iter().map(|p| p.1))
                                .fold(f64::NEG_INFINITY, f64::max);
                            output.push(PdfText {
                                text: source[start..end].into(),
                                rect: CssRect {
                                    x,
                                    y,
                                    width: right - x,
                                    height: bottom - y,
                                },
                                quad: None,
                                clusters,
                            });
                        }
                    };
                    for (start, end, quad, rtl) in accepted {
                        if current.as_ref().is_some_and(|(_, previous, _)| {
                            source
                                .get(*previous..start)
                                .is_none_or(|gap| !gap.trim().is_empty())
                        }) {
                            flush(current.take(), output);
                        }
                        let entry = current.get_or_insert_with(|| (start, start, Vec::new()));
                        entry.1 = end;
                        entry.2.push(PdfCluster {
                            start: start - entry.0,
                            end: end - entry.0,
                            quad,
                            rtl,
                        });
                    }
                    flush(current, output);
                    if output.len() > 32768 {
                        return Err(RenderError::InvalidPrint(
                            "SVG semantic line count exceeds bounds".into(),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    walk(
        tree.root(),
        tree,
        snapshot,
        &xml,
        scale,
        &mut budget,
        &mut output,
        resource,
        0,
    )?;
    Ok(output)
}

pub(super) fn ranges(
    tree: &resvg::usvg::Tree,
) -> Result<Vec<(String, std::ops::Range<usize>)>, RenderError> {
    fn walk(
        group: &resvg::usvg::Group,
        out: &mut Vec<(String, std::ops::Range<usize>)>,
        depth: usize,
    ) -> Result<(), RenderError> {
        if depth > 128 {
            return Err(RenderError::InvalidPrint("native SVG source depth".into()));
        }
        for node in group.children() {
            match node {
                resvg::usvg::Node::Group(group) => walk(group, out, depth + 1)?,
                resvg::usvg::Node::Text(text) => {
                    let source = text
                        .chunks()
                        .iter()
                        .map(|chunk| chunk.text())
                        .collect::<String>();
                    let ranges = text
                        .layouted()
                        .iter()
                        .filter(|span| span.visible)
                        .flat_map(|span| &span.positioned_glyphs)
                        .map(|glyph| (glyph.source.start, glyph.source.end))
                        .collect::<std::collections::BTreeSet<_>>();
                    for (start, end) in ranges {
                        if source
                            .get(start..end)
                            .is_some_and(|text| !text.trim().is_empty())
                        {
                            if out.len() >= 16384 {
                                return Err(RenderError::InvalidPrint(
                                    "native SVG source cluster budget".into(),
                                ));
                            }
                            out.push((text.flattened().id().into(), start..end));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(tree.root(), &mut out, 0)?;
    Ok(out)
}
pub(super) fn cluster_tree(
    tree: &resvg::usvg::Tree,
    id: &str,
    range: std::ops::Range<usize>,
) -> Option<resvg::usvg::Tree> {
    fn find<'a>(group: &'a resvg::usvg::Group, id: &str) -> Option<&'a resvg::usvg::Text> {
        for node in group.children() {
            match node {
                resvg::usvg::Node::Group(group) => {
                    if let Some(text) = find(group, id) {
                        return Some(text);
                    }
                }
                resvg::usvg::Node::Text(text) if text.flattened().id() == id => return Some(text),
                _ => {}
            }
        }
        None
    }
    tree.text_cluster_tree(find(tree.root(), id)?, range)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(body: &str) -> (PaintSnapshot, resvg::usvg::Tree) {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://example.test/native-text.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80">{body}</svg>"#
            )
            .into_bytes(),
        });
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        (snapshot, tree)
    }
    #[test]
    fn native_svg_text_budget_is_checked_before_shaping() {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>{}</text></svg>"#,
            "a".repeat(65537)
        );
        assert!(
            crate::svg::xml_bytes(source.as_bytes())
                .unwrap_err()
                .contains("shaping source budget")
        );
    }
    #[test]
    fn native_svg_partial_mask_keeps_intact_cluster() {
        let (snapshot, tree) = parsed(
            r##"<defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="240" height="80"><path d="M0 0H14V80H0Z" fill="white"/></mask></defs><g mask="url(#m)"><text x="2 17" y="16" font-size="10">AB</text></g>"##,
        );
        let lines = collect(&snapshot, &tree, 1., 0).unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            "A"
        );
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.clusters)
                .all(|cluster| cluster.end > cluster.start)
        );
    }
    #[test]
    fn native_svg_source_ranges_preserve_chunks_repeats_ligatures_and_bidi() {
        let (_, tree) = parsed(
            r##"<text x="5" y="20" font-size="12">AA office é<tspan x="5" y="40">אב AA</tspan></text>"##,
        );
        fn check(group: &resvg::usvg::Group) {
            for node in group.children() {
                match node {
                    resvg::usvg::Node::Group(group) => check(group),
                    resvg::usvg::Node::Text(text) => {
                        let source = text
                            .chunks()
                            .iter()
                            .map(|chunk| chunk.text())
                            .collect::<String>();
                        let mut ranges = std::collections::BTreeSet::new();
                        for span in text.layouted() {
                            for glyph in &span.positioned_glyphs {
                                assert!(source.get(glyph.source.clone()).is_some());
                                assert!(glyph.source.start < glyph.source.end);
                                ranges.insert((glyph.source.start, glyph.source.end));
                            }
                        }
                        assert!(
                            ranges
                                .iter()
                                .any(|(start, _)| *start >= source.find("אב").unwrap())
                        );
                        assert!(
                            text.layouted()
                                .iter()
                                .flat_map(|span| &span.positioned_glyphs)
                                .any(|glyph| glyph.rtl)
                        );
                    }
                    _ => {}
                }
            }
        }
        check(tree.root());
    }
    #[test]
    fn native_svg_rotation_and_path_keep_oriented_cluster_quads() {
        let (snapshot, tree) = parsed(
            r##"<defs><path id="p" d="M10 60L80 10"/></defs><text font-size="12"><textPath href="#p">Path</textPath></text><text x="120" y="30" rotate="35" font-size="12">A</text>"##,
        );
        let lines = collect(&snapshot, &tree, 2., 0).unwrap();
        assert!(!lines.is_empty());
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.clusters)
                .any(|cluster| (cluster.quad[1].1 - cluster.quad[0].1).abs() > 0.5)
        );
    }
    #[test]
    fn generated_text_identity_does_not_change_author_selector_results() {
        let (snapshot, tree) = parsed(
            r##"<style>text[id]{fill:none}</style><text x="5" y="20" font-size="12">Visible</text><text id="author" x="5" y="40">Hidden</text>"##,
        );
        let lines = collect(&snapshot, &tree, 1., 0).unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            "Visible"
        );
    }
}
