//! PDF pages consume print-layout fragments and shared resources. Physical page
//! dimensions are independent of raster sampling scale (96 CSS px = 72 points).
use super::*;
#[path = "print_text.rs"]
pub(crate) mod text_layer;
#[path = "print_vector.rs"]
pub(crate) mod vector;
use oryn_common::v4::PrintSnapshot;

pub fn render_print_pdf(
    print: &PrintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    if print.pages.is_empty() || print.pages.len() > oryn_layout::print::MAX_PRINT_PAGES {
        return Err(RenderError::InvalidPrint(
            "print snapshot must contain 1..=256 pages".into(),
        ));
    }
    if !print.source.nodes.is_empty() {
        return Err(RenderError::InvalidPrint(
            "shared print source must not duplicate page nodes".into(),
        ));
    }
    if !options.scale.is_finite() || !(0.1..=4.).contains(&options.scale) {
        return Err(RenderError::InvalidScale(options.scale));
    }
    let resource_bytes = print
        .source
        .fonts
        .values()
        .map(|f| f.bytes.len())
        .chain(print.source.images.iter().map(|i| i.bytes.len()))
        .try_fold(0usize, usize::checked_add)
        .ok_or_else(|| RenderError::InvalidPrint("resource length overflow".into()))?;
    if resource_bytes > 256 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "shared print resources exceed 256 MiB".into(),
        ));
    }
    let mut total_pixels = 0usize;
    let mut total_text = 0usize;
    let mut total_spans = 0usize;
    for page in &print.pages {
        for node in &page.nodes {
            let fragments = node
                .projection
                .as_ref()
                .map_or(node.text.as_slice(), |p| p.text.as_slice());
            for fragment in fragments {
                total_text = total_text
                    .checked_add(fragment.text.len())
                    .ok_or_else(|| RenderError::InvalidPrint("text length overflow".into()))?;
                total_spans = total_spans
                    .checked_add(fragment.text.len().div_ceil(255).max(1))
                    .ok_or_else(|| RenderError::InvalidPrint("text span overflow".into()))?;
            }
        }
        if total_text > 4_000_000 || total_spans > 32_768 {
            return Err(RenderError::InvalidPrint(
                "PDF semantic text exceeds resource bounds".into(),
            ));
        }
        if [page.viewport.width, page.viewport.height]
            .iter()
            .any(|v| !v.is_finite() || *v < 1. || *v > 200. * 96. / 2.54)
        {
            return Err(RenderError::InvalidPrint(
                "invalid physical page dimensions".into(),
            ));
        }
        let width = (page.viewport.width * options.scale).ceil() as usize;
        let height = (page.viewport.height * options.scale).ceil() as usize;
        total_pixels = width
            .checked_mul(height)
            .and_then(|n| total_pixels.checked_add(n))
            .ok_or_else(|| RenderError::InvalidPrint("page pixel count overflow".into()))?;
    }
    if total_pixels > options.max_pixels {
        return Err(RenderError::PixelLimit {
            actual: total_pixels,
            limit: options.max_pixels,
        });
    }
    let mut pages = Vec::new();
    let mut diagnostics = print.source.diagnostics.clone();
    let mut output_dimensions = (0, 0);
    let mut chunk_bytes = 0usize;
    let mut semantic_bytes = 0usize;
    let mut semantic_spans = 0usize;
    for page in &print.pages {
        let mut snapshot = print.source.clone();
        crate::clear_visual_viewports(&mut snapshot);
        snapshot.viewport_scroll = None;
        snapshot.frames.retain(|frame| {
            page.nodes
                .iter()
                .chain(
                    snapshot
                        .svg_foreign_objects
                        .iter()
                        .flat_map(|object| object.nodes.iter()),
                )
                .any(|node| node.node_id == frame.owner_node)
        });
        snapshot.nodes = page.nodes.clone();
        snapshot.viewport = page.viewport;
        snapshot.diagnostics.clear();
        snapshot.scroll = oryn_common::v4::ScrollState::default();
        let scene = build_display_list(&snapshot, options)?;
        let (chunk, form) = vector::page(&snapshot, &scene, options)?;
        chunk_bytes = chunk_bytes
            .checked_add(chunk.as_bytes().len())
            .filter(|n| *n <= 256 * 1024 * 1024)
            .ok_or_else(|| {
                RenderError::InvalidPrint("aggregate vector pages exceed 256 MiB".into())
            })?;
        if pages.is_empty() {
            output_dimensions = (scene.width, scene.height);
        }
        let text = text_layer::collect_with_svg(page, &snapshot, &scene)?;
        for line in &text {
            semantic_bytes = semantic_bytes
                .checked_add(line.text.len())
                .filter(|n| *n <= 4_000_000)
                .ok_or_else(|| {
                    RenderError::InvalidPrint("PDF semantic text exceeds 4 MiB".into())
                })?;
            semantic_spans = semantic_spans
                .checked_add(line.text.len().div_ceil(255).max(1))
                .filter(|n| *n <= 32_768)
                .ok_or_else(|| {
                    RenderError::InvalidPrint("PDF semantic text spans exceed bounds".into())
                })?;
        }
        pages.push(VectorPage {
            chunk,
            form,
            width_points: page.viewport.width * 0.75,
            height_points: page.viewport.height * 0.75,
            text,
        });
        diagnostics.extend(scene.diagnostics);
    }
    Ok(RenderedArtifact {
        bytes: encode_vector_pages(&pages)?,
        width: output_dimensions.0,
        height: output_dimensions.1,
        diagnostics,
    })
}
struct VectorPage {
    chunk: pdf_writer::Chunk,
    form: pdf_writer::Ref,
    width_points: f64,
    height_points: f64,
    text: Vec<text_layer::PdfText>,
}
fn affine_text_matrix(
    line: &text_layer::PdfText,
    offset: usize,
    count: usize,
    page_height: f64,
) -> Option<[f64; 6]> {
    let q = line.quad?;
    if count == 0 || q.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }
    if (q[0].0 + q[2].0 - q[1].0 - q[3].0).abs() > 0.001
        || (q[0].1 + q[2].1 - q[1].1 - q[3].1).abs() > 0.001
    {
        return None;
    }
    let inline = (q[1].0 - q[0].0, q[1].1 - q[0].1);
    let block = (q[3].0 - q[0].0, q[3].1 - q[0].1);
    let part = offset as f64 / count as f64;
    let x = q[0].0 + inline.0 * part + block.0 * 0.8;
    let y = q[0].1 + inline.1 * part + block.1 * 0.8;
    Some([
        inline.0 * 0.75 / (count as f64 * 0.6),
        -inline.1 * 0.75 / (count as f64 * 0.6),
        -block.0 * 0.75,
        block.1 * 0.75,
        x * 0.75,
        page_height - y * 0.75,
    ])
}
fn emit_cluster_text(
    line: &text_layer::PdfText,
    offsets: &[usize],
    start: usize,
    encoded: &[u8],
    font: usize,
    page_height: f64,
    command: &mut String,
) -> Result<bool, RenderError> {
    if line.clusters.is_empty() {
        return Ok(false);
    }
    let end = start + encoded.len();
    let first = line
        .clusters
        .partition_point(|cluster| cluster.end <= offsets[start]);
    for cluster in line.clusters[first..]
        .iter()
        .take_while(|cluster| cluster.start < offsets[end])
    {
        let (Ok(a), Ok(b)) = (
            offsets.binary_search(&cluster.start),
            offsets.binary_search(&cluster.end),
        ) else {
            continue;
        };
        let from = a.max(start);
        let to = b.min(end);
        if from >= to {
            continue;
        }
        let q = cluster.quad;
        let x = q.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let y = q.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let native = text_layer::PdfText {
            clusters: Vec::new(),
            quad: Some(q),
            text: String::new(),
            rect: CssRect {
                x,
                y,
                width: q.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) - x,
                height: q.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) - y,
            },
        };
        let matrix = if let Some(matrix) = affine_text_matrix(&native, from - a, b - a, page_height)
        {
            matrix
        } else {
            // PDF text matrices are affine. For a projective cluster retain the
            // exact baseline endpoints and midpoint block direction; the native
            // quadrilateral remains available to geometry consumers.
            let mut fit = q;
            fit[2] = (q[1].0 + q[3].0 - q[0].0, q[1].1 + q[3].1 - q[0].1);
            let mut native = native.clone();
            native.quad = Some(fit);
            affine_text_matrix(&native, from - a, b - a, page_height).ok_or_else(|| {
                RenderError::InvalidPrint("nonfinite projective text cluster".into())
            })?
        };
        let [a, b, c, d, e, f] = matrix;
        let hex = encoded[from - start..to - start]
            .iter()
            .map(|v| format!("{v:02X}"))
            .collect::<String>();
        command.push_str(&format!("BT /T{font} 1 Tf 3 Tr 100 Tz {a:.8} {b:.8} {c:.8} {d:.8} {e:.8} {f:.8} Tm <{hex}> Tj ET\n"));
        if command.len() > 64 * 1024 * 1024 {
            return Err(RenderError::InvalidPrint(
                "PDF page text operators exceed 64 MiB".into(),
            ));
        }
    }
    Ok(true)
}
fn encode_vector_pages(pages: &[VectorPage]) -> Result<Vec<u8>, RenderError> {
    use pdf_writer::{Finish, Name, Pdf, Rect, Ref};
    let mut pdf = Pdf::new();
    let mut next = Ref::new(1);
    let catalog = next.bump();
    let tree = next.bump();
    let ids = pages.iter().map(|_| next.bump()).collect::<Vec<_>>();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree)
        .kids(ids.iter().copied())
        .count(pages.len() as i32);
    for (page, id) in pages.iter().zip(ids) {
        let mut mapping = std::collections::HashMap::new();
        let chunk = page
            .chunk
            .renumber(|old| *mapping.entry(old).or_insert_with(|| next.bump()));
        let form = *mapping
            .get(&page.form)
            .ok_or_else(|| RenderError::Encode("vector form reference absent".into()))?;
        pdf.extend(&chunk);
        let contents = next.bump();
        let mut fonts = Vec::new();
        let mut command = format!(
            "q\n{:.8} 0 0 {:.8} 0 0 cm\n/Im0 Do\nQ\n",
            page.width_points, page.height_points
        );
        for line in &page.text {
            let chars = line.text.chars().collect::<Vec<_>>();
            let offsets = line
                .text
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(line.text.len()))
                .collect::<Vec<_>>();
            for (chunk_index, chars_part) in chars.chunks(255).enumerate() {
                let (encoded, cmap) = text_layer::encoding(&chars_part.iter().collect::<String>());
                let font = next.bump();
                let cmap_id = next.bump();
                let index = fonts.len();
                let mut dict = pdf.indirect(font).dict();
                dict.pair(Name(b"Type"), Name(b"Font"))
                    .pair(Name(b"Subtype"), Name(b"Type1"))
                    .pair(Name(b"BaseFont"), Name(b"Helvetica"))
                    .pair(Name(b"FirstChar"), 1)
                    .pair(Name(b"LastChar"), 255)
                    .pair(Name(b"Encoding"), Name(b"WinAnsiEncoding"))
                    .pair(Name(b"ToUnicode"), cmap_id);
                dict.insert(Name(b"Widths"))
                    .array()
                    .items(std::iter::repeat_n(600, 255));
                dict.finish();
                pdf.stream(cmap_id, &cmap).finish();
                fonts.push(font);
                if emit_cluster_text(
                    line,
                    &offsets,
                    chunk_index * 255,
                    &encoded,
                    index,
                    page.height_points,
                    &mut command,
                )? {
                    continue;
                }
                let part = chars_part.len() as f64 / chars.len().max(1) as f64;
                let font_size = (line.rect.height * 0.75).max(0.01);
                let horizontal = line.rect.width * 0.75 * part
                    / (chars_part.len() as f64 * font_size * 0.6)
                    * 100.;
                let x = (line.rect.x
                    + line.rect.width * (chunk_index * 255) as f64 / chars.len().max(1) as f64)
                    * 0.75;
                let y = page.height_points - (line.rect.y + line.rect.height * 0.8) * 0.75;
                let hex = encoded
                    .iter()
                    .map(|v| format!("{v:02X}"))
                    .collect::<String>();
                if let Some([a, b, c, d, e, f]) =
                    affine_text_matrix(line, chunk_index * 255, chars.len(), page.height_points)
                {
                    command.push_str(&format!("BT /T{index} 1 Tf 3 Tr 100 Tz {a:.8} {b:.8} {c:.8} {d:.8} {e:.8} {f:.8} Tm <{hex}> Tj ET\n"));
                } else {
                    command.push_str(&format!("BT /T{index} {font_size:.8} Tf 3 Tr {horizontal:.8} Tz 1 0 0 1 {x:.8} {y:.8} Tm <{hex}> Tj ET\n"));
                }
            }
        }
        let mut output = pdf.page(id);
        output
            .parent(tree)
            .media_box(Rect::new(
                0.,
                0.,
                page.width_points as f32,
                page.height_points as f32,
            ))
            .contents(contents);
        let mut resources = output.resources();
        resources.x_objects().pair(Name(b"Im0"), form);
        {
            let mut dictionary = resources.fonts();
            for (i, font) in fonts.iter().enumerate() {
                dictionary.pair(Name(format!("T{i}").as_bytes()), *font);
            }
        }
        resources.finish();
        output.finish();
        pdf.stream(contents, command.as_bytes()).finish();
        if pdf.as_bytes().len() > 256 * 1024 * 1024 {
            return Err(RenderError::InvalidPrint(
                "vector PDF exceeds 256 MiB".into(),
            ));
        }
    }
    Ok(pdf.finish())
}
#[cfg(test)]
struct PdfPage {
    width: u32,
    height: u32,
    width_points: f64,
    height_points: f64,
    compressed: Vec<u8>,
    text: Vec<text_layer::PdfText>,
}
#[cfg(test)]
fn stream(dictionary: String, bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< {dictionary} /Length {} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}
#[cfg(test)]
fn encode_pages(pages: &[PdfPage]) -> Vec<u8> {
    let kids = (0..pages.len())
        .map(|i| format!("{} 0 R", 3 + i * 3))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).into_bytes(),
    ];
    let mut font_objects = Vec::<Vec<u8>>::new();
    for (index, page) in pages.iter().enumerate() {
        let image = 4 + index * 3;
        let contents = image + 1;
        let mut font_resources = String::new();
        let mut command = format!(
            "q\n{:.8} 0 0 {:.8} 0 0 cm\n/Im0 Do\nQ\n",
            page.width_points, page.height_points
        );
        let mut font_index = 0;
        for line in &page.text {
            let chars = line.text.chars().collect::<Vec<_>>();
            for (chunk_index, chunk) in chars.chunks(255).enumerate() {
                let text = chunk.iter().collect::<String>();
                let (encoded, cmap) = text_layer::encoding(&text);
                let font_id = 3 + pages.len() * 3 + font_objects.len();
                let cmap_id = font_id + 1;
                let widths = std::iter::repeat_n("600", 255)
                    .collect::<Vec<_>>()
                    .join(" ");
                font_objects.push(format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 1 /LastChar 255 /Widths [{widths}] /Encoding /WinAnsiEncoding /ToUnicode {cmap_id} 0 R >>").into_bytes());
                font_objects.push(stream(String::new(), &cmap));
                font_resources.push_str(&format!(" /T{font_index} {font_id} 0 R"));
                let part = chunk.len() as f64 / chars.len().max(1) as f64;
                let width = line.rect.width * 0.75 * part;
                let font_size = (line.rect.height * 0.75).max(0.01);
                let horizontal = width / (chunk.len() as f64 * font_size * 0.6) * 100.;
                let x = (line.rect.x
                    + line.rect.width * (chunk_index * 255) as f64 / chars.len().max(1) as f64)
                    * 0.75;
                let y = page.height_points - (line.rect.y + line.rect.height * 0.8) * 0.75;
                let hex = encoded
                    .iter()
                    .map(|v| format!("{v:02X}"))
                    .collect::<String>();
                command.push_str(&format!("BT /T{font_index} {font_size:.8} Tf 3 Tr {horizontal:.8} Tz 1 0 0 1 {x:.8} {y:.8} Tm <{hex}> Tj ET\n"));
                font_index += 1;
            }
        }
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.8} {:.8}] /Resources << /XObject << /Im0 {image} 0 R >> /Font <<{font_resources} >> >> /Contents {contents} 0 R >>",page.width_points,page.height_points).into_bytes());
        objects.push(stream(format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",page.width,page.height),&page.compressed));
        objects.push(stream(String::new(), command.as_bytes()));
    }
    objects.extend(font_objects);
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cluster_advances_drive_pdf_character_positions() {
        let line = text_layer::PdfText {
            clusters: vec![
                text_layer::PdfCluster {
                    start: 0,
                    end: 1,
                    quad: [(0., 0.), (80., 0.), (80., 10.), (0., 10.)],
                    rtl: false,
                },
                text_layer::PdfCluster {
                    start: 1,
                    end: 2,
                    quad: [(80., 0.), (100., 0.), (100., 10.), (80., 10.)],
                    rtl: false,
                },
            ],
            quad: Some([(0., 0.), (100., 0.), (100., 10.), (0., 10.)]),
            text: "Wi".into(),
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 10.,
            },
        };
        let mut command = String::new();
        assert!(emit_cluster_text(&line, &[0, 1, 2], 0, &[1, 2], 0, 100., &mut command).unwrap());
        assert_eq!(command.matches(" Tj ET").count(), 2);
        assert!(command.contains("100.00000000 -0.00000000"));
        assert!(command.contains("25.00000000 -0.00000000"));
        assert!(command.contains("60.00000000 94.00000000 Tm <02>"));
    }
    #[test]
    fn semantic_text_matrix_preserves_rotated_line_axes() {
        let line = text_layer::PdfText {
            clusters: Vec::new(),
            quad: Some([(10., 10.), (10., 50.), (0., 50.), (0., 10.)]),
            text: "ABCD".into(),
            rect: CssRect {
                x: 0.,
                y: 10.,
                width: 10.,
                height: 40.,
            },
        };
        let matrix = affine_text_matrix(&line, 0, 4, 72.).unwrap();
        assert_eq!(matrix, [0., -12.5, 7.5, 0., 1.5, 64.5]);
        let second = affine_text_matrix(&line, 2, 4, 72.).unwrap();
        assert_eq!(second[5], 49.5);
    }
    #[test]
    fn vector_page_assembly_renumbers_forms_and_retains_unicode_text() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 96.;
        snapshot.viewport.height = 144.;
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let mut pages = Vec::new();
        for _ in 0..2 {
            let (chunk, form) = vector::page(&snapshot, &scene, RenderOptions::default()).unwrap();
            pages.push(VectorPage {
                chunk,
                form,
                width_points: 72.,
                height_points: 108.,
                text: vec![text_layer::PdfText {
                    clusters: Vec::new(),
                    quad: None,
                    text: "Native 世界 😀".into(),
                    rect: CssRect {
                        x: 1.,
                        y: 1.,
                        width: 70.,
                        height: 12.,
                    },
                }],
            });
        }
        let bytes = encode_vector_pages(&pages).unwrap();
        let source = String::from_utf8_lossy(&bytes);
        assert!(source.contains("/Count 2"));
        assert_eq!(source.matches("/MediaBox [0 0 72 108]").count(), 2);
        assert_eq!(source.matches("/ToUnicode").count(), 2);
        assert_eq!(source.matches("D83DDE00").count(), 2);
        assert_eq!(source.matches("3 Tr").count(), 2);
        assert!(!source.contains("/Subtype /Image"));
        let offset = source
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(&bytes[offset..offset + 4], b"xref");
    }
    #[test]
    fn page_tree_and_media_boxes_use_physical_points_independent_of_raster_scale() {
        let pages = vec![
            PdfPage {
                width: 192,
                height: 288,
                width_points: 72.,
                height_points: 108.,
                compressed: vec![1, 2],
                text: Vec::new(),
            },
            PdfPage {
                width: 384,
                height: 576,
                width_points: 72.,
                height_points: 108.,
                compressed: vec![3, 4],
                text: Vec::new(),
            },
        ];
        let pdf = encode_pages(&pages);
        let source = String::from_utf8_lossy(&pdf);
        assert!(source.contains("/Kids [3 0 R 6 0 R] /Count 2"));
        assert_eq!(
            source
                .matches("/MediaBox [0 0 72.00000000 108.00000000]")
                .count(),
            2
        );
        let offset = source
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(&pdf[offset..offset + 4], b"xref");
        assert!(source.contains("/Contents 8 0 R"));
    }
    #[test]
    fn semantic_text_is_invisible_unicode_mapped_and_keeps_page_object_references() {
        let page = PdfPage {
            width: 96,
            height: 96,
            width_points: 72.,
            height_points: 72.,
            compressed: vec![1],
            text: vec![text_layer::PdfText {
                clusters: Vec::new(),
                quad: None,
                text: "Hello 世界 😀".into(),
                rect: CssRect {
                    x: 10.,
                    y: 10.,
                    width: 60.,
                    height: 12.,
                },
            }],
        };
        let bytes = encode_pages(&[page]);
        let source = String::from_utf8_lossy(&bytes);
        assert!(source.contains("/T0 6 0 R"));
        assert!(source.contains("/ToUnicode 7 0 R"));
        assert!(source.contains("3 Tr"));
        assert!(source.contains("D83DDE00"));
        assert!(source.contains("/Im0 Do"));
        assert!(source.contains("/Contents 5 0 R"));
        let offset = source
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(&bytes[offset..offset + 4], b"xref");
    }
    #[test]
    fn print_limits_are_checked_before_rasterization() {
        let source = crate::tests::empty_snapshot();
        let mut print = PrintSnapshot {
            source,
            pages: vec![
                oryn_common::v4::PrintPageSnapshot {
                    viewport: oryn_common::v4::ViewportState {
                        width: 100.,
                        height: 100.,
                        ..Default::default()
                    },
                    nodes: Vec::new()
                };
                2
            ],
        };
        assert!(matches!(
            render_print_pdf(
                &print,
                RenderOptions {
                    max_pixels: 19_999,
                    ..Default::default()
                }
            ),
            Err(RenderError::PixelLimit { .. })
        ));
        print.pages[0].viewport.width = f64::NAN;
        assert!(matches!(
            render_print_pdf(&print, RenderOptions::default()),
            Err(RenderError::InvalidPrint(_))
        ));
    }
}
