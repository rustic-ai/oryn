//! Searchable PDF text comes from native shaped lines, never image recognition.
//! Invisible Type 1 text uses an explicit Unicode CMap so even supplementary
//! characters survive extraction. The visible page uses native vector paint.

/// Inputs for semantic text traversal.
struct PrintTextWalk<'a> {
    commands: &'a [PaintCommand],
    snapshot: &'a PaintSnapshot,
    matrix: [f64; 16],
    clips: &'a [oryn_common::v4::PaintClip],
    out: &'a mut Vec<PdfText>,
    depth: usize,
    bounded: bool,
    resolution: f64,
}
use super::*;
use oryn_common::v4::PaintProjection;
#[derive(Clone)]
pub(crate) struct PdfText {
    pub text: String,
    pub rect: CssRect,
    pub quad: Option<[(f64, f64); 4]>,
    pub clusters: Vec<PdfCluster>,
}
#[derive(Clone)]
pub(crate) struct PdfCluster {
    pub start: usize,
    pub end: usize,
    pub quad: [(f64, f64); 4],
    pub rtl: bool,
}
fn point(m: [f64; 16], x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[3] * x + m[7] * y + m[15];
    if !w.is_finite() || w <= 0. {
        return None;
    }
    let p = (
        (m[0] * x + m[4] * y + m[12]) / w,
        (m[1] * x + m[5] * y + m[13]) / w,
    );
    (p.0.is_finite() && p.1.is_finite()).then_some(p)
}
fn inverse_point(m: [f64; 16], x: f64, y: f64) -> Option<(f64, f64)> {
    let [a, b, c, d, e, f, g, h, i] = [m[0], m[4], m[12], m[1], m[5], m[13], m[3], m[7], m[15]];
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-15 {
        return None;
    }
    let w = ((d * h - e * g) * x + (b * g - a * h) * y + a * e - b * d) / det;
    if !w.is_finite() || w <= 0. {
        return None;
    }
    Some((
        ((e * i - f * h) * x + (c * h - b * i) * y + b * f - c * e) / det / w,
        ((f * g - d * i) * x + (a * i - c * g) * y + c * d - a * f) / det / w,
    ))
}
#[cfg(test)]
fn visible_rect(
    fragment: &PaintTextFragment,
    projection: Option<&PaintProjection>,
    width: f64,
    height: f64,
) -> Option<CssRect> {
    visible_rect_impl(fragment, projection, Some((width, height)))
}
#[cfg(test)]
fn visible_rect_impl(
    fragment: &PaintTextFragment,
    projection: Option<&PaintProjection>,
    viewport: Option<(f64, f64)>,
) -> Option<CssRect> {
    visible_box(fragment.rect, projection, viewport)
}
fn visible_box(
    rect: CssRect,
    projection: Option<&PaintProjection>,
    viewport: Option<(f64, f64)>,
) -> Option<CssRect> {
    if [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .any(|v| !v.is_finite())
        || rect.width <= 0.
        || rect.height <= 0.
    {
        return None;
    }
    let mut corners = vec![
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ];
    if let Some(projection) = projection {
        let matrix = projection.text_matrix.unwrap_or(projection.matrix);
        corners = corners
            .into_iter()
            .map(|(x, y)| point(matrix, x, y))
            .collect::<Option<Vec<_>>>()?;
        for clip in projection.clips.iter().chain(&projection.text_clips) {
            for &(x, y) in &corners {
                let (x, y) = inverse_point(clip.matrix, x, y)?;
                if clip
                    .shape
                    .as_ref()
                    .is_some_and(|shape| !shape.contains(x, y))
                    || clip.clip_x && (x < clip.rect.x - 0.01 || x > clip.rect.right() + 0.01)
                    || clip.clip_y && (y < clip.rect.y - 0.01 || y > clip.rect.bottom() + 0.01)
                {
                    return None;
                }
            }
        }
    }
    // Whole-line ownership is checked first. The caller can then select intact
    // visible native clusters when only part of a line belongs to this page.
    if viewport.is_some_and(|(width, height)| {
        corners
            .iter()
            .any(|&(x, y)| x < -0.01 || y < -0.01 || x > width + 0.01 || y > height + 0.01)
    }) {
        return None;
    }
    let x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    Some(CssRect {
        x,
        y,
        width: corners
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max)
            - x,
        height: corners
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            - y,
    })
}
fn quad(rect: CssRect, projection: Option<&PaintProjection>) -> Option<[(f64, f64); 4]> {
    let points = [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ];
    if let Some(projection) = projection {
        let matrix = projection.text_matrix.unwrap_or(projection.matrix);
        let mut result = [(0., 0.); 4];
        for (i, (x, y)) in points.into_iter().enumerate() {
            result[i] = point(matrix, x, y)?;
        }
        Some(result)
    } else {
        Some(points)
    }
}
fn part(
    fragment: &PaintTextFragment,
    start: usize,
    end: usize,
    rect: CssRect,
    projection: Option<&PaintProjection>,
    viewport: Option<(f64, f64)>,
) -> Option<PdfText> {
    let mut clusters = fragment
        .clusters
        .iter()
        .filter(|cluster| {
            cluster.start >= start
                && cluster.end <= end
                && cluster.start < cluster.end
                && fragment.text.is_char_boundary(cluster.start)
                && fragment.text.is_char_boundary(cluster.end)
        })
        .filter_map(|cluster| {
            visible_box(cluster.rect, projection, viewport)?;
            Some(PdfCluster {
                start: cluster.start - start,
                end: cluster.end - start,
                quad: quad(cluster.rect, projection)?,
                rtl: cluster.rtl,
            })
        })
        .collect::<Vec<_>>();
    clusters.sort_by_key(|cluster| (cluster.start, cluster.end));
    clusters.dedup_by_key(|cluster| (cluster.start, cluster.end));
    Some(PdfText {
        clusters,
        quad: quad(rect, projection),
        text: fragment.text.get(start..end)?.to_owned(),
        rect: visible_box(rect, projection, viewport)?,
    })
}
fn visible_parts(
    fragment: &PaintTextFragment,
    projection: Option<&PaintProjection>,
    viewport: Option<(f64, f64)>,
) -> Vec<PdfText> {
    if let Some(part) = part(
        fragment,
        0,
        fragment.text.len(),
        fragment.rect,
        projection,
        viewport,
    ) {
        return vec![part];
    }
    let mut clusters = fragment
        .clusters
        .iter()
        .filter(|cluster| {
            cluster.start < cluster.end
                && cluster.end <= fragment.text.len()
                && fragment.text.is_char_boundary(cluster.start)
                && fragment.text.is_char_boundary(cluster.end)
        })
        .collect::<Vec<_>>();
    clusters.sort_by_key(|cluster| (cluster.start, cluster.end));
    let mut spans = Vec::<(usize, usize, CssRect)>::new();
    let mut consumed = 0;
    for cluster in clusters {
        if cluster.start < consumed {
            continue;
        }
        consumed = cluster.end;
        let Some(_) = visible_box(cluster.rect, projection, viewport) else {
            continue;
        };
        let rect = cluster.rect;
        if let Some((_, end, last)) = spans.last_mut().filter(|(_, end, _)| *end == cluster.start) {
            *end = cluster.end;
            let x = last.x.min(rect.x);
            let y = last.y.min(rect.y);
            *last = CssRect {
                x,
                y,
                width: last.right().max(rect.right()) - x,
                height: last.bottom().max(rect.bottom()) - y,
            };
        } else {
            spans.push((cluster.start, cluster.end, rect));
        }
    }
    spans
        .into_iter()
        .filter_map(|(start, end, rect)| part(fragment, start, end, rect, projection, viewport))
        .collect()
}
fn orient_parts(parts: &mut [PdfText], style: &std::collections::BTreeMap<String, String>) {
    let mode = style
        .get("writing-mode")
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    let rtl = style.get("direction").is_some_and(|value| value == "rtl")
        && !(mode != "horizontal-tb"
            && style
                .get("text-orientation")
                .is_some_and(|value| value == "upright"));
    let orient = |q: [(f64, f64); 4], rtl: bool| {
        let q = match mode {
            "vertical-rl" | "sideways-rl" => [q[1], q[2], q[3], q[0]],
            "vertical-lr" => [q[0], q[3], q[2], q[1]],
            "sideways-lr" => [q[3], q[0], q[1], q[2]],
            _ => q,
        };
        if rtl { [q[1], q[0], q[3], q[2]] } else { q }
    };
    for part in parts {
        part.quad = part.quad.map(|quad| orient(quad, rtl));
        for cluster in &mut part.clusters {
            cluster.quad = orient(cluster.quad, cluster.rtl);
        }
    }
}
pub(super) fn collect(page: &oryn_common::v4::PrintPageSnapshot) -> Vec<PdfText> {
    let mut output = Vec::new();
    for node in &page.nodes {
        if node.geometry.availability != GeometryAvailability::Available
            || node.style.get("visibility").is_some_and(|v| v != "visible")
            || node.style.get("display").is_some_and(|v| v == "none")
            || node
                .style
                .get("opacity")
                .and_then(|v| v.parse::<f64>().ok())
                .is_some_and(|v| v <= 0.)
        {
            continue;
        }
        let text = node
            .projection
            .as_ref()
            .map_or(node.text.as_slice(), |p| p.text.as_slice());
        for fragment in text {
            if fragment.text.is_empty() || fragment.glyph_runs.is_empty() {
                continue;
            }
            let mut parts = visible_parts(
                fragment,
                node.projection.as_ref(),
                Some((page.viewport.width, page.viewport.height)),
            );
            orient_parts(&mut parts, &node.style);
            output.extend(parts);
        }
    }
    // Preserve page block progression for vertical documents. Native shaped
    // fragment text already carries logical character order, so only the line
    // sequence is ordered here; glyph strings must not be reversed.
    let mode = page
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none() && node.tag != "oryn-page-margin")
        .and_then(|node| node.style.get("writing-mode"))
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    sort_lines(&mut output, mode);
    output
}
fn sort_lines(lines: &mut [PdfText], mode: &str) {
    match mode {
        "vertical-rl" | "sideways-rl" => lines.sort_by(|a, b| {
            b.rect
                .right()
                .total_cmp(&a.rect.right())
                .then(a.rect.y.total_cmp(&b.rect.y))
        }),
        "vertical-lr" | "sideways-lr" => lines.sort_by(|a, b| {
            a.rect
                .x
                .total_cmp(&b.rect.x)
                .then(a.rect.y.total_cmp(&b.rect.y))
        }),
        _ => lines.sort_by(|a, b| {
            a.rect
                .y
                .total_cmp(&b.rect.y)
                .then(a.rect.x.total_cmp(&b.rect.x))
        }),
    }
}

/// One encoding for a bounded span, with code zero unused. The ToUnicode map
/// carries UTF-16BE, including surrogate pairs, independently of base font names.
pub(super) fn encoding(text: &str) -> (Vec<u8>, Vec<u8>) {
    let mut chars = std::collections::BTreeMap::<char, u8>::new();
    let mut encoded = Vec::new();
    for ch in text.chars() {
        let next = (chars.len() + 1) as u8;
        let code = *chars.entry(ch).or_insert(next);
        encoded.push(code);
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Oryn) /Ordering (Unicode) /Supplement 0 >> def\n/CMapName /OrynUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
    );
    let entries = chars.into_iter().collect::<Vec<_>>();
    for entries in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", entries.len()));
        for (ch, code) in entries {
            let mut units = [0; 2];
            let utf16 = ch
                .encode_utf16(&mut units)
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect::<String>();
            cmap.push_str(&format!("<{code:02X}> <{utf16}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    (encoded, cmap.into_bytes())
}

fn append_svg(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    output: &mut Vec<PdfText>,
    bounded: bool,
) -> Result<(), RenderError> {
    fn multiply(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
        std::array::from_fn(|i| {
            let row = i % 4;
            let col = i / 4;
            (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
        })
    }
    fn walk(params: PrintTextWalk<'_>) -> Result<(), RenderError> {
        let PrintTextWalk {
            commands,
            snapshot,
            matrix,
            clips,
            out,
            depth,
            bounded,
            resolution,
        } = params;

        if depth > 256 {
            return Err(RenderError::InvalidPrint(
                "semantic SVG nesting exceeds bounds".into(),
            ));
        }
        for command in commands {
            match command {
                PaintCommand::Mask { commands, .. } | PaintCommand::Filter { commands, .. } => {
                    walk(PrintTextWalk {
                        commands,
                        snapshot,
                        matrix,
                        clips,
                        out,
                        depth: depth + 1,
                        bounded,
                        resolution,
                    })?
                }
                PaintCommand::Opacity { opacity, commands } => {
                    if *opacity > 0. {
                        walk(PrintTextWalk {
                            commands,
                            snapshot,
                            matrix,
                            clips,
                            out,
                            depth: depth + 1,
                            bounded,
                            resolution,
                        })?;
                    }
                }
                PaintCommand::Projected {
                    matrix: local,
                    clips: local_clips,
                    commands,
                } => {
                    let mut combined = clips.to_vec();
                    combined.extend(local_clips.iter().cloned().map(|mut clip| {
                        clip.matrix = multiply(matrix, clip.matrix);
                        clip
                    }));
                    walk(PrintTextWalk {
                        commands,
                        snapshot,
                        matrix: multiply(matrix, *local),
                        clips: &combined,
                        out,
                        depth: depth + 1,
                        bounded,
                        resolution,
                    })?;
                }
                PaintCommand::EmbeddedDocument { rect, frame } => {
                    let _frame_scope = crate::svg_coverage::frame_scope(
                        snapshot
                            .frames
                            .get(*frame)
                            .ok_or_else(|| {
                                RenderError::InvalidPrint(
                                    "semantic frame index out of bounds".into(),
                                )
                            })?
                            .owner_node,
                    );
                    let child = crate::frames::snapshot(snapshot, *frame)?;
                    let page = oryn_common::v4::PrintPageSnapshot {
                        viewport: child.viewport,
                        nodes: child.nodes.clone(),
                    };
                    let local = [
                        rect.width / child.viewport.width,
                        0.,
                        0.,
                        0.,
                        0.,
                        rect.height / child.viewport.height,
                        0.,
                        0.,
                        0.,
                        0.,
                        1.,
                        0.,
                        rect.x,
                        rect.y,
                        0.,
                        1.,
                    ];
                    let transform = multiply(matrix, local);
                    let Some(density) = crate::projected::density_for_transform(
                        transform,
                        CssRect {
                            x: 0.,
                            y: 0.,
                            width: snapshot.viewport.width,
                            height: snapshot.viewport.height,
                        },
                    ) else {
                        continue;
                    };
                    let (scene, _) =
                        crate::frames::scene(child, resolution * density.0.max(density.1))?;
                    for mut line in collect_with_svg(&page, child, &scene)? {
                        let quad = line.quad.unwrap_or([
                            (line.rect.x, line.rect.y),
                            (line.rect.right(), line.rect.y),
                            (line.rect.right(), line.rect.bottom()),
                            (line.rect.x, line.rect.bottom()),
                        ]);
                        let Some(corners) = quad
                            .into_iter()
                            .map(|(x, y)| point(transform, x, y))
                            .collect::<Option<Vec<_>>>()
                        else {
                            continue;
                        };
                        if bounded
                            && corners.iter().any(|&(x, y)| {
                                x < 0.
                                    || y < 0.
                                    || x > snapshot.viewport.width
                                    || y > snapshot.viewport.height
                            })
                        {
                            continue;
                        }
                        if clips.iter().any(|clip| {
                            corners.iter().any(|&(x, y)| {
                                inverse_point(clip.matrix, x, y).is_none_or(|(x, y)| {
                                    clip.shape
                                        .as_ref()
                                        .is_some_and(|shape| !shape.contains(x, y))
                                        || clip.clip_x && (x < clip.rect.x || x > clip.rect.right())
                                        || clip.clip_y
                                            && (y < clip.rect.y || y > clip.rect.bottom())
                                })
                            })
                        }) {
                            continue;
                        }
                        let x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                        let y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                        line.rect = CssRect {
                            x,
                            y,
                            width: corners
                                .iter()
                                .map(|p| p.0)
                                .fold(f64::NEG_INFINITY, f64::max)
                                - x,
                            height: corners
                                .iter()
                                .map(|p| p.1)
                                .fold(f64::NEG_INFINITY, f64::max)
                                - y,
                        };
                        line.quad = Some([corners[0], corners[1], corners[2], corners[3]]);
                        line.clusters = line
                            .clusters
                            .into_iter()
                            .filter_map(|mut cluster| {
                                let points = cluster
                                    .quad
                                    .into_iter()
                                    .map(|(x, y)| point(transform, x, y))
                                    .collect::<Option<Vec<_>>>()?;
                                cluster.quad = [points[0], points[1], points[2], points[3]];
                                Some(cluster)
                            })
                            .collect();
                        out.push(line);
                        if out.len() > 32768 {
                            return Err(RenderError::InvalidPrint(
                                "frame semantic line count exceeds bounds".into(),
                            ));
                        }
                    }
                }
                PaintCommand::SvgImage {
                    resource,
                    rect,
                    clip,
                    environment,
                } => {
                    let _environment = environment.enter();
                    let source = snapshot.images.get(*resource).ok_or_else(|| {
                        RenderError::InvalidPrint("missing semantic SVG resource".into())
                    })?;
                    let tree = crate::svg_coverage::semantic_tree(source, snapshot)?;
                    let local = [
                        rect.width / f64::from(tree.size().width()),
                        0.,
                        0.,
                        0.,
                        0.,
                        rect.height / f64::from(tree.size().height()),
                        0.,
                        0.,
                        0.,
                        0.,
                        1.,
                        0.,
                        rect.x,
                        rect.y,
                        0.,
                        1.,
                    ];
                    let Some(density) = crate::projected::density_for_transform(
                        multiply(matrix, local),
                        CssRect {
                            x: 0.,
                            y: 0.,
                            width: snapshot.viewport.width,
                            height: snapshot.viewport.height,
                        },
                    ) else {
                        continue;
                    };
                    for semantic in crate::svg_foreign::semantic_lines_at_scale(
                        source,
                        snapshot,
                        &tree,
                        resolution * density.0.max(density.1),
                    )? {
                        let text = semantic.text;
                        let line = semantic.rect;
                        let source_quad = semantic.quad.unwrap_or([
                            (line.x, line.y),
                            (line.right(), line.y),
                            (line.right(), line.bottom()),
                            (line.x, line.bottom()),
                        ]);
                        let line = CssRect {
                            x: rect.x + line.x / f64::from(tree.size().width()) * rect.width,
                            y: rect.y + line.y / f64::from(tree.size().height()) * rect.height,
                            width: line.width / f64::from(tree.size().width()) * rect.width,
                            height: line.height / f64::from(tree.size().height()) * rect.height,
                        };
                        if clip.is_some_and(|clip| {
                            line.x < clip.x
                                || line.y < clip.y
                                || line.right() > clip.right()
                                || line.bottom() > clip.bottom()
                        }) {
                            continue;
                        }
                        let Some(corners) = source_quad
                            .into_iter()
                            .map(|(x, y)| {
                                point(
                                    matrix,
                                    rect.x + x / f64::from(tree.size().width()) * rect.width,
                                    rect.y + y / f64::from(tree.size().height()) * rect.height,
                                )
                            })
                            .collect::<Option<Vec<_>>>()
                        else {
                            continue;
                        };
                        if bounded
                            && corners.iter().any(|&(x, y)| {
                                x < 0.
                                    || y < 0.
                                    || x > snapshot.viewport.width
                                    || y > snapshot.viewport.height
                            })
                        {
                            continue;
                        }
                        if clips.iter().any(|clip| {
                            corners.iter().any(|&(x, y)| {
                                inverse_point(clip.matrix, x, y).is_none_or(|(x, y)| {
                                    clip.shape
                                        .as_ref()
                                        .is_some_and(|shape| !shape.contains(x, y))
                                        || clip.clip_x && (x < clip.rect.x || x > clip.rect.right())
                                        || clip.clip_y
                                            && (y < clip.rect.y || y > clip.rect.bottom())
                                })
                            })
                        }) {
                            continue;
                        }
                        let x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                        let y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                        let clusters = semantic
                            .clusters
                            .into_iter()
                            .filter_map(|mut cluster| {
                                let points = cluster
                                    .quad
                                    .into_iter()
                                    .map(|(x, y)| {
                                        point(
                                            matrix,
                                            rect.x
                                                + x / f64::from(tree.size().width()) * rect.width,
                                            rect.y
                                                + y / f64::from(tree.size().height()) * rect.height,
                                        )
                                    })
                                    .collect::<Option<Vec<_>>>()?;
                                cluster.quad = [points[0], points[1], points[2], points[3]];
                                Some(cluster)
                            })
                            .collect();
                        out.push(PdfText {
                            clusters,
                            quad: Some([corners[0], corners[1], corners[2], corners[3]]),
                            text,
                            rect: CssRect {
                                x,
                                y,
                                width: corners
                                    .iter()
                                    .map(|p| p.0)
                                    .fold(f64::NEG_INFINITY, f64::max)
                                    - x,
                                height: corners
                                    .iter()
                                    .map(|p| p.1)
                                    .fold(f64::NEG_INFINITY, f64::max)
                                    - y,
                            },
                        });
                        if out.len() > 32768 {
                            return Err(RenderError::InvalidPrint(
                                "semantic SVG line count exceeds bounds".into(),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    if !snapshot.svg_foreign_objects.is_empty()
        || !snapshot.frames.is_empty()
        || snapshot
            .images
            .iter()
            .any(|image| image.media_type == "image/svg+xml")
    {
        walk(PrintTextWalk {
            commands: &scene.commands,
            snapshot,
            matrix: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            clips: &[],
            out: output,
            depth: 0,
            bounded,
            resolution: f64::from(scene.width) / snapshot.viewport.width.max(f64::MIN_POSITIVE),
        })?;
    }
    Ok(())
}
pub(super) fn collect_with_svg(
    page: &oryn_common::v4::PrintPageSnapshot,
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
) -> Result<Vec<PdfText>, RenderError> {
    let mut output = collect(page);
    append_svg(snapshot, scene, &mut output, true)?;
    let mode = page
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none() && node.tag != "oryn-page-margin")
        .and_then(|node| node.style.get("writing-mode"))
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    sort_lines(&mut output, mode);
    Ok(output)
}
pub(crate) fn collect_local(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
) -> Result<Vec<PdfText>, RenderError> {
    let by_id = snapshot
        .nodes
        .iter()
        .map(|node| (node.node_id, node))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut output = Vec::new();
    for node in &snapshot.nodes {
        if node.geometry.availability != GeometryAvailability::Available {
            continue;
        }
        let mut ancestor = Some(node);
        let mut hidden = false;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(current) = ancestor {
            if !seen.insert(current.node_id) {
                hidden = true;
                break;
            }
            if current.style.get("display").is_some_and(|v| v == "none")
                || current
                    .style
                    .get("visibility")
                    .is_some_and(|v| v != "visible")
                || current
                    .style
                    .get("opacity")
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some_and(|v| v <= 0.)
            {
                hidden = true;
                break;
            }
            ancestor = current.parent_id.and_then(|id| by_id.get(&id).copied());
        }
        if hidden {
            continue;
        }
        for fragment in node
            .projection
            .as_ref()
            .map_or(node.text.as_slice(), |p| p.text.as_slice())
        {
            if fragment.text.is_empty() || fragment.glyph_runs.is_empty() {
                continue;
            }
            let mut parts = visible_parts(fragment, node.projection.as_ref(), None);
            orient_parts(&mut parts, &node.style);
            output.extend(parts);
        }
    }
    append_svg(snapshot, scene, &mut output, false)?;
    let mode = snapshot
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .and_then(|node| node.style.get("writing-mode"))
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    sort_lines(&mut output, mode);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fragment(rect: CssRect) -> PaintTextFragment {
        PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "Hello 世界 😀".into(),
            rect,
            glyph_runs: Vec::new(),
            font_family: "sans-serif".into(),
            font_size: 12.,
            color: "black".into(),
        }
    }
    #[test]
    fn html_frame_semantics_do_not_require_any_svg_foreign_object() {
        use oryn_common::v4::{PaintFontResource, PaintFrameSnapshot, PaintGlyph, PaintGlyphRun};
        let mut child = crate::tests::empty_snapshot();
        child.viewport.width = 20.;
        child.viewport.height = 20.;
        child.fonts.insert(
            "frame-font".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let mut text = fragment(CssRect {
            x: 1.,
            y: 1.,
            width: 8.,
            height: 8.,
        });
        text.text = "A".into();
        text.font_size = 8.;
        text.glyph_runs.push(PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: None,
            decoration_rect: None,
            scale_x: 1.,
            vertical: false,
            decorations: Vec::new(),
            orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
            font_key: "frame-font".into(),
            font_size: 8.,
            color: None,
            embolden: false,
            skew_degrees: 0.,
            normalized_coords: Vec::new(),
            glyphs: vec![PaintGlyph {
                source: None,
                id: u32::from(font.lookup_glyph_index('A')),
                x: 0.,
                y: 7.,
                advance: 8.,
            }],
        });
        let mut node = crate::tests::paint_node(1, None, "p", text.rect);
        node.projection.as_mut().unwrap().text.push(text.clone());
        node.text.push(text);
        child.nodes.push(node);
        let mut parent = crate::tests::empty_snapshot();
        parent.viewport.width = 100.;
        parent.viewport.height = 100.;
        parent.nodes.push(crate::tests::paint_node(
            2,
            None,
            "iframe",
            CssRect {
                x: 10.,
                y: 20.,
                width: 40.,
                height: 40.,
            },
        ));
        parent.frames.push(PaintFrameSnapshot {
            owner_node: 2,
            snapshot: Box::new(child),
        });
        let scene = build_display_list(&parent, RenderOptions::default()).unwrap();
        let page = oryn_common::v4::PrintPageSnapshot {
            viewport: parent.viewport,
            nodes: parent.nodes.clone(),
        };
        let lines = collect_with_svg(&page, &parent, &scene).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "A");
        assert!((lines[0].rect.x - 12.).abs() < 0.01);
        assert!((lines[0].rect.y - 22.).abs() < 0.01);
    }

    #[test]
    fn vertical_semantic_quad_follows_native_inline_progression() {
        let rect = CssRect {
            x: 10.,
            y: 20.,
            width: 12.,
            height: 40.,
        };
        let mut parts = vec![PdfText {
            clusters: Vec::new(),
            quad: quad(rect, None),
            text: "AB".into(),
            rect,
        }];
        orient_parts(
            &mut parts,
            &std::collections::BTreeMap::from([("writing-mode".into(), "vertical-rl".into())]),
        );
        assert_eq!(
            parts[0].quad,
            Some([(22., 20.), (22., 60.), (10., 60.), (10., 20.)])
        );
    }
    #[test]
    fn partial_line_extraction_keeps_unicode_clusters_and_ligatures_intact() {
        use oryn_common::v4::{BoxModel, EdgeSizes, PaintTextCluster};
        let mut line = fragment(CssRect {
            x: 0.,
            y: 0.,
            width: 40.,
            height: 10.,
        });
        line.text = "A😀fiB".into();
        line.clusters = [(0, 1), (1, 5), (5, 7), (7, 8)]
            .into_iter()
            .enumerate()
            .map(|(i, (start, end))| PaintTextCluster {
                start,
                end,
                rect: CssRect {
                    x: i as f64 * 10.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                },
                rtl: false,
            })
            .collect();
        let first = visible_parts(&line, None, Some((25., 20.)));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].text, "A😀");
        let mut matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        matrix[12] = -20.;
        let model = BoxModel {
            border_box: line.rect,
            padding_box: line.rect,
            content_box: line.rect,
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
        };
        let projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_edges: None,
            border_segments: Vec::new(),
            matrix,
            box_model: model,
            text: Vec::new(),
            text_matrix: None,
            clips: Vec::new(),
            text_clips: Vec::new(),
        };
        let second = visible_parts(&line, Some(&projection), Some((25., 20.)));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].text, "fiB");
        assert_eq!(second[0].rect.x, 0.);
        assert_eq!(second[0].rect.width, 20.);
    }
    #[test]
    fn unicode_cmap_preserves_supplementary_scalars_and_repeated_letters() {
        let (bytes, cmap) = encoding("A😀AA界");
        assert_eq!(bytes, vec![1, 2, 1, 1, 3]);
        let cmap = String::from_utf8(cmap).unwrap();
        assert!(cmap.contains("<01> <0041>"));
        assert!(cmap.contains("<02> <D83DDE00>"));
        assert!(cmap.contains("<03> <754C>"));
    }
    #[test]
    fn vertical_pdf_reading_order_follows_column_progression_without_reversing_text() {
        let original = vec![
            PdfText {
                clusters: Vec::new(),
                quad: None,
                text: "second".into(),
                rect: CssRect {
                    x: 20.,
                    y: 0.,
                    width: 10.,
                    height: 50.,
                },
            },
            PdfText {
                clusters: Vec::new(),
                quad: None,
                text: "first".into(),
                rect: CssRect {
                    x: 40.,
                    y: 0.,
                    width: 10.,
                    height: 50.,
                },
            },
        ];
        let mut lines = original.clone();
        sort_lines(&mut lines, "vertical-rl");
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        sort_lines(&mut lines, "vertical-lr");
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "first"]
        );
    }

    #[test]
    fn clipped_line_is_not_duplicated_as_full_text_on_adjacent_pages() {
        let line = fragment(CssRect {
            x: 10.,
            y: 95.,
            width: 50.,
            height: 10.,
        });
        assert!(visible_rect(&line, None, 100., 100.).is_none());
        let visible = fragment(CssRect {
            x: 10.,
            y: 80.,
            width: 50.,
            height: 10.,
        });
        assert_eq!(visible_rect(&visible, None, 100., 100.), Some(visible.rect));
        let malformed = fragment(CssRect {
            x: f64::NAN,
            ..visible.rect
        });
        assert!(visible_rect(&malformed, None, 100., 100.).is_none());
    }
}
