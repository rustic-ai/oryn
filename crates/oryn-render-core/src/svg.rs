//! Static SVG trees use only snapshot resources and captured fonts. Parsing
//! cannot open host paths, start network requests, or execute document scripts.
use super::*;
use resvg::{tiny_skia, usvg};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
const MAX_SOURCE: usize = 16 * 1024 * 1024;
const MAX_REFERENCES: usize = 256;
const MAX_NESTED_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_PIXELS: usize = 16 * 1024 * 1024;
fn error(source: &oryn_common::v4::PaintImageResource, detail: impl Into<String>) -> RenderError {
    RenderError::ImageDecode {
        url: source.url.clone(),
        detail: detail.into(),
    }
}
fn captured_instance_maps(
    source: &oryn_common::v4::PaintImageResource,
) -> Result<(usvg::NativeInstanceStyles, usvg::NativeInstanceContent), RenderError> {
    use oryn_common::v4::SvgInstanceContent;
    if !source.svg_document_snapshot && !source.svg_instances.is_empty() {
        return Err(error(
            source,
            "fetched SVG cannot carry captured instance metadata",
        ));
    }
    if source.svg_instances.len() > 100_000 {
        return Err(error(
            source,
            "captured SVG instance count exceeds native bound",
        ));
    }
    let mut styles = usvg::NativeInstanceStyles::new();
    let mut content = usvg::NativeInstanceContent::new();
    let mut bytes = 0usize;
    for instance in &source.svg_instances {
        if instance.ancestry.is_empty() || instance.ancestry.len() > 128 {
            return Err(error(source, "invalid captured SVG instance ancestry"));
        }
        let reference_bytes = match &instance.content {
            SvgInstanceContent::Reference(reference) => {
                if reference.is_empty() {
                    return Err(error(source, "empty captured SVG content reference"));
                }
                reference.len()
            }
            _ => 0,
        };
        bytes = bytes
            .checked_add(instance.presentation.len())
            .and_then(|size| size.checked_add(reference_bytes))
            .and_then(|size| size.checked_add(instance.ancestry.len() * 8 + 8))
            .filter(|size| *size <= MAX_SOURCE)
            .ok_or_else(|| error(source, "captured SVG instance metadata exceeds 16 MiB"))?;
        let key = usvg::NativeInstanceKey {
            ancestry: instance.ancestry.clone(),
            source: instance.source,
        };
        if styles
            .insert(key.clone(), instance.presentation.clone())
            .is_some()
        {
            return Err(error(source, "duplicate captured SVG instance identity"));
        }
        match &instance.content {
            SvgInstanceContent::Original => {}
            SvgInstanceContent::Empty => {
                content.insert(key, None);
            }
            SvgInstanceContent::Reference(reference) => {
                content.insert(key, Some(reference.clone()));
            }
        }
    }
    Ok((styles, content))
}

pub(crate) fn xml_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    xml_bytes_with_type(bytes, None)
}
pub(crate) fn xml_bytes_with_type(
    bytes: &[u8],
    content_type: Option<&str>,
) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_SOURCE {
        return Err("SVG source exceeds 16 MiB".into());
    }
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut result = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .take((MAX_SOURCE + 1) as u64)
            .read_to_end(&mut result)
            .map_err(|e| e.to_string())?;
        result
    } else {
        bytes.to_vec()
    };
    if bytes.len() > MAX_SOURCE {
        return Err("expanded SVG exceeds 16 MiB".into());
    }
    let mut text =
        oryn_common::xml_encoding::decode(&bytes, content_type).map_err(str::to_owned)?;
    // Resource bytes remain untouched; this local parser input is canonical
    // UTF-8. Drop its byte-encoding declaration before nested re-parsing.
    if text
        .strip_prefix("<?xml")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'))
        && let Some(end) = text.find("?>")
    {
        text.drain(..end + 2);
    }
    if text.len() > MAX_SOURCE * 3 {
        return Err("decoded SVG exceeds 48 MiB".into());
    }
    let xml = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
    if xml.descendants().take(100_001).count() > 100_000 {
        return Err("SVG exceeds 100000 XML nodes".into());
    }
    let mut text_bytes = 0usize;
    for node in xml.descendants() {
        if node.ancestors().take(130).count() > 129 {
            return Err("SVG exceeds 128 element ancestry levels".into());
        }
        if node.is_text()
            && node.ancestors().any(|ancestor| {
                ancestor.is_element()
                    && ancestor.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                    && ancestor.tag_name().name() == "text"
            })
        {
            text_bytes = text_bytes.saturating_add(node.text().unwrap_or("").len());
            if text_bytes > 64 * 1024 {
                return Err("SVG native text exceeds 64 KiB shaping source budget".into());
            }
        }
    }
    Ok(text.into_bytes())
}
pub(crate) fn xml_source(resource: &oryn_common::v4::PaintImageResource) -> Result<String, String> {
    String::from_utf8(xml_bytes_with_type(
        &resource.bytes,
        resource.content_type.as_deref(),
    )?)
    .map_err(|error| error.to_string())
}
fn valid_embedded(mime: &str, data: &[u8]) -> bool {
    if data.len() > MAX_SOURCE {
        return false;
    }
    if mime == "image/svg+xml" || data.first() == Some(&b'<') {
        return xml_bytes(data).ok().is_some_and(|bytes| {
            std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| roxmltree::Document::parse(text).ok())
                .is_some_and(|xml| {
                    !xml.descendants()
                        .any(|n| n.is_element() && n.tag_name().name() == "foreignObject")
                })
        });
    }
    let Ok(format) = image::guess_format(data) else {
        return false;
    };
    image::ImageReader::with_format(std::io::Cursor::new(data), format)
        .into_dimensions()
        .ok()
        .and_then(|(w, h)| (w as usize).checked_mul(h as usize))
        .is_some_and(|pixels| pixels <= MAX_PIXELS)
}
pub(super) fn parse(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
) -> Result<usvg::Tree, RenderError> {
    parse_with_environment(source, snapshot, SvgEnvironment::current(snapshot))
}
pub(super) fn parse_with_environment(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    environment: SvgEnvironment,
) -> Result<usvg::Tree, RenderError> {
    parse_with_environment_policy(source, snapshot, environment, false)
}
// Generated filter wrappers require every active image dependency to resolve.
// This tightens failure reporting; it never grants secure SVG access to the
// embedding document's resource inventory. Nested data SVG inherits the policy.
pub(super) fn parse_filter(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
) -> Result<usvg::Tree, RenderError> {
    parse_with_environment_policy(source, snapshot, SvgEnvironment::current(snapshot), true)
}
fn parse_with_environment_policy(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    environment: SvgEnvironment,
    strict_references: bool,
) -> Result<usvg::Tree, RenderError> {
    let _environment = environment.enter();
    let _guard = crate::svg_foreign::ParseGuard::enter()?;
    let semantic = crate::svg_coverage::take_svg_request();
    if snapshot.images.iter().any(|image| {
        image.url_aliases.len() > 16
            || image.url_aliases.iter().map(String::len).sum::<usize>() > 64 * 1024
    }) {
        return Err(error(
            source,
            "captured image aliases exceed metadata budget",
        ));
    }
    let data = xml_bytes_with_type(&source.bytes, source.content_type.as_deref())
        .map_err(|detail| error(source, detail))?;
    let data = if semantic {
        crate::svg_coverage::neutralize_svg_effects(data)?
    } else {
        data
    };
    let font_data = data.clone();
    let (captured_styles, captured_content) = captured_instance_maps(source)?;
    let (data, instance_styles, instance_content, style_roots) = if source.svg_document_snapshot {
        let composed = crate::svg_foreign::compose_snapshot(source, snapshot, data)?;
        (
            composed.data,
            captured_styles,
            captured_content,
            composed.style_roots,
        )
    } else {
        crate::svg_secure_foreign::compose_with_instance_styles(source, snapshot, data)?
    };
    let font_bytes = snapshot
        .fonts
        .values()
        .filter(|_| source.svg_document_snapshot)
        .try_fold(0usize, |size, font| size.checked_add(font.bytes.len()))
        .ok_or_else(|| error(source, "SVG font byte count overflow"))?;
    if font_bytes > 64 * 1024 * 1024 {
        return Err(error(source, "SVG captured fonts exceed 64 MiB"));
    }
    let mut options = usvg::Options {
        native_instance_styles: instance_styles,
        native_instance_content: instance_content,
        native_style_roots: style_roots,
        font_family: oryn_layout::DEFAULT_FONT_FAMILY.into(),
        default_size: usvg::Size::from_wh(environment.width as f32, environment.height as f32)
            .unwrap_or_else(|| usvg::Size::from_wh(300., 150.).unwrap()),
        ..Default::default()
    };
    options
        .fontdb_mut()
        .load_font_data(oryn_layout::DEFAULT_FONT_BYTES.to_vec());
    if let Some(family) = options
        .fontdb
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|(family, _)| family.clone())
    {
        options.font_family = family;
    }
    if source.svg_document_snapshot {
        for font in snapshot.fonts.values() {
            options.fontdb_mut().load_font_data(font.bytes.clone());
        }
        oryn_layout::svg_fonts::install(&mut options, &[])
            .map_err(|detail| error(source, detail))?;
    }
    if !source.svg_document_snapshot {
        let text =
            std::str::from_utf8(&font_data).map_err(|detail| error(source, detail.to_string()))?;
        let xml =
            roxmltree::Document::parse(text).map_err(|detail| error(source, detail.to_string()))?;
        let sheets = xml
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                    && node.tag_name().name() == "style"
            })
            .map(|node| {
                node.children()
                    .filter_map(|child| child.text())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let fonts = oryn_layout::svg_fonts::embedded(
            &sheets,
            &source.url,
            environment.viewport(),
            if environment.dark {
                oryn_layout::ColorScheme::Dark
            } else {
                oryn_layout::ColorScheme::Light
            },
            environment.reduced_motion,
        )
        .map_err(|detail| error(source, detail))?;
        oryn_layout::svg_fonts::install(&mut options, &fonts)
            .map_err(|detail| error(source, detail))?;
    }
    let count = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    let used_count = count.clone();
    let used_bytes = bytes.clone();
    let rejected = failures.clone();
    options.image_href_resolver.resolve_data = Box::new(move |mime, data, options| {
        if used_count.fetch_add(1, Ordering::Relaxed) >= MAX_REFERENCES
            || used_bytes
                .fetch_add(data.len(), Ordering::Relaxed)
                .saturating_add(data.len())
                > MAX_NESTED_BYTES
            || (mime != "image/svg+xml" && !valid_embedded(mime, &data))
        {
            if let Ok(mut rejected) = rejected.lock()
                && rejected.len() < 8
            {
                rejected.push("embedded SVG image exceeds bounds or cannot be decoded".into());
            }
            return None;
        }
        if mime == "image/svg+xml" {
            let temporary = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: source.url.clone(),
                media_type: mime.into(),
                bytes: data.as_ref().clone(),
            };
            let nested = snapshot
                .images
                .iter()
                .find(|candidate| {
                    !candidate.svg_document_snapshot
                        && candidate.bytes.as_slice() == data.as_slice()
                        && candidate.media_type == mime
                })
                .unwrap_or(&temporary);
            return match parse_with_environment_policy(
                nested,
                snapshot,
                SvgEnvironment::current(snapshot),
                strict_references,
            ) {
                Ok(tree) => Some(usvg::ImageKind::SVG(tree)),
                Err(error) => {
                    if let Ok(mut rejected) = rejected.lock()
                        && rejected.len() < 8
                    {
                        rejected.push(error.to_string());
                    }
                    None
                }
            };
        }
        let image = default_data(mime, data, options);
        if strict_references
            && image.is_none()
            && let Ok(mut rejected) = rejected.lock()
            && rejected.len() < 8
        {
            rejected.push("filter embedded image could not be decoded".into());
        }
        image
    });
    let base = url::Url::parse(&source.url).ok();
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    let resources = &snapshot.images;
    let rejected = failures.clone();
    options.image_href_resolver.resolve_string = Box::new(move |href, options| {
        // Only native inline-document snapshots may use the document's admitted
        // resources. Fetched SVG has secure image/resource processing semantics.
        if !source.svg_document_snapshot {
            if strict_references
                && let Ok(mut rejected) = rejected.lock()
                && rejected.len() < 8
            {
                rejected.push(format!(
                    "filter image reference is unavailable in secure SVG: {href}"
                ));
            }
            return None;
        }
        let absolute = base
            .as_ref()
            .and_then(|base| base.join(href).ok())
            .map(|mut url| {
                url.set_fragment(None);
                url.to_string()
            });
        let resource = resources
            .iter()
            .filter(|resource| !resource.svg_document_snapshot)
            .find(|resource| {
                resource.url == href
                    || absolute.as_deref() == Some(resource.url.as_str())
                    || resource
                        .url_aliases
                        .iter()
                        .any(|alias| alias == href || absolute.as_deref() == Some(alias.as_str()))
            });
        let Some(resource) = resource else {
            if let Ok(mut rejected) = rejected.lock()
                && rejected.len() < 8
            {
                rejected.push(format!(
                    "SVG image reference is absent from captured resources: {href}"
                ));
            }
            return None;
        };
        if count.fetch_add(1, Ordering::Relaxed) >= MAX_REFERENCES
            || bytes
                .fetch_add(resource.bytes.len(), Ordering::Relaxed)
                .saturating_add(resource.bytes.len())
                > MAX_NESTED_BYTES
            || (resource.media_type != "image/svg+xml"
                && !valid_embedded(&resource.media_type, &resource.bytes))
        {
            if let Ok(mut rejected) = rejected.lock()
                && rejected.len() < 8
            {
                rejected.push("captured SVG image exceeds bounds or cannot be decoded".into());
            }
            return None;
        }
        if resource.media_type == "image/svg+xml" {
            let nested = resource.clone();
            return match parse_with_environment_policy(
                &nested,
                snapshot,
                SvgEnvironment::current(snapshot),
                strict_references,
            ) {
                Ok(tree) => Some(usvg::ImageKind::SVG(tree)),
                Err(error) => {
                    if let Ok(mut rejected) = rejected.lock()
                        && rejected.len() < 8
                    {
                        rejected.push(error.to_string());
                    }
                    None
                }
            };
        }
        default_data(
            &resource.media_type,
            Arc::new(resource.bytes.clone()),
            options,
        )
    });
    let rejected = failures.clone();
    options.image_href_resolver.resolve_svg_viewport =
        Some(Box::new(move |href, size, scheme, _options| {
            let nested = if href.starts_with("data:") {
                let bytes = oryn_layout::svg_fonts::data_bytes(href)?;
                let content_type = href
                    .strip_prefix("data:")
                    .and_then(|tail| tail.split_once(','))
                    .map(|(metadata, _)| metadata.to_owned());
                oryn_common::v4::PaintImageResource {
                    svg_document_snapshot: false,
                    svg_instances: Vec::new(),
                    content_type,
                    url_aliases: Vec::new(),
                    url: source.url.clone(),
                    media_type: "image/svg+xml".into(),
                    bytes,
                }
            } else {
                if !source.svg_document_snapshot {
                    if strict_references
                        && let Ok(mut rejected) = rejected.lock()
                        && rejected.len() < 8
                    {
                        rejected.push(format!(
                            "filter image reference is unavailable in secure SVG: {href}"
                        ));
                    }
                    return None;
                }
                let absolute = url::Url::parse(&source.url)
                    .ok()
                    .and_then(|base| base.join(href).ok())
                    .map(|mut url| {
                        url.set_fragment(None);
                        url.to_string()
                    });
                snapshot
                    .images
                    .iter()
                    .filter(|resource| !resource.svg_document_snapshot)
                    .find(|resource| {
                        resource.url == href
                            || absolute.as_deref() == Some(resource.url.as_str())
                            || resource.url_aliases.iter().any(|alias| {
                                alias == href || absolute.as_deref() == Some(alias.as_str())
                            })
                    })?
                    .clone()
            };
            let style = std::collections::BTreeMap::from([
                ("color-scheme".into(), scheme.to_owned()),
                (
                    "oryn-preferred-color-scheme".into(),
                    if environment.dark { "dark" } else { "light" }.into(),
                ),
                (
                    "oryn-reduced-motion".into(),
                    if environment.reduced_motion {
                        "reduce"
                    } else {
                        "no-preference"
                    }
                    .into(),
                ),
            ]);
            let context = SvgEnvironment::for_style(
                &style,
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: f64::from(size.width()),
                    height: f64::from(size.height()),
                },
                environment.viewport(),
            );
            match parse_with_environment_policy(&nested, snapshot, context, strict_references) {
                Ok(tree) => Some(tree),
                Err(error) => {
                    if let Ok(mut rejected) = rejected.lock()
                        && rejected.len() < 8
                    {
                        rejected.push(error.to_string());
                    }
                    None
                }
            }
        }));
    let mut tree =
        usvg::Tree::from_data(&data, &options).map_err(|e| error(source, e.to_string()))?;
    tree.assign_semantic_text_ids();
    if let Ok(rejected) = failures.lock()
        && !rejected.is_empty()
    {
        return Err(error(source, rejected.join("; ")));
    }
    let size = tree.size();
    if !size.width().is_finite()
        || !size.height().is_finite()
        || size.width() > 1_000_000.
        || size.height() > 1_000_000.
    {
        return Err(error(source, "SVG intrinsic dimensions exceed bounds"));
    }
    if semantic {
        crate::svg_coverage::restore_svg_effects(source, snapshot, tree)
    } else {
        Ok(tree)
    }
}
/// Bound intermediate filter/mask/opacity surfaces before either converter can
/// allocate them. Subroots include patterns, masks, clips and nested SVG images.
pub(super) fn validate_layers(
    tree: &usvg::Tree,
    scale_x: f64,
    scale_y: f64,
    limit: usize,
) -> Result<(), RenderError> {
    layer_pixels(tree, scale_x, scale_y, limit).map(|_| ())
}
pub(super) fn layer_pixels(
    tree: &usvg::Tree,
    scale_x: f64,
    scale_y: f64,
    limit: usize,
) -> Result<usize, RenderError> {
    fn visit(
        group: &usvg::Group,
        sx: f64,
        sy: f64,
        limit: usize,
        total: &mut usize,
        count: &mut usize,
        depth: usize,
    ) -> Result<(), RenderError> {
        *count += 1;
        if depth > 128 || *count > 100_000 {
            return Err(RenderError::InvalidPrint(
                "SVG layer complexity exceeds bounds".into(),
            ));
        }
        if group.should_isolate() {
            let rect = group.abs_layer_bounding_box();
            let pixels =
                (f64::from(rect.width()) * sx).ceil() * (f64::from(rect.height()) * sy).ceil();
            if !pixels.is_finite() || pixels < 0. || pixels > limit as f64 {
                return Err(RenderError::InvalidPrint(
                    "SVG intermediate layer exceeds pixel budget".into(),
                ));
            }
            *total = total
                .checked_add(pixels as usize)
                .filter(|n| *n <= limit)
                .ok_or_else(|| {
                    RenderError::InvalidPrint(
                        "SVG aggregate intermediate layers exceed pixel budget".into(),
                    )
                })?;
        }
        for node in group.children() {
            if let usvg::Node::Group(child) = node {
                visit(child, sx, sy, limit, total, count, depth + 1)?;
            }
            let mut result = Ok(());
            node.subroots(|root| {
                if result.is_ok() {
                    result = visit(root, sx, sy, limit, total, count, depth + 1);
                }
            });
            result?;
        }
        Ok(())
    }
    let mut total = 0;
    visit(
        tree.root(),
        scale_x.abs(),
        scale_y.abs(),
        limit,
        &mut total,
        &mut 0,
        0,
    )?;
    Ok(total)
}
pub(super) fn raster(
    tree: &usvg::Tree,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, RenderError> {
    validate_layers(
        tree,
        f64::from(width) / f64::from(tree.size().width()),
        f64::from(height) / f64::from(tree.size().height()),
        MAX_PIXELS,
    )?;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(RenderError::PixelLimit {
            actual: usize::MAX,
            limit: MAX_PIXELS,
        })?;
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(RenderError::PixelLimit {
            actual: pixels,
            limit: MAX_PIXELS,
        });
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| RenderError::Encode("SVG surface allocation failed".into()))?;
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    // tiny-skia stores premultiplied pixels; the shared compositor consumes
    // straight alpha and performs its own premultiplication during sampling.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            if let Some(straight) = (u32::from(*channel) * 255 + alpha / 2).checked_div(alpha) {
                *channel = straight.min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| RenderError::Encode("invalid SVG raster buffer".into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn captured_instance_fixture() -> oryn_common::v4::PaintImageResource {
        use oryn_common::v4::{PaintImageResource, SvgInstanceContent, SvgInstanceSnapshot};
        PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: vec![
                SvgInstanceSnapshot { ancestry: vec![4], source: 3, presentation: "fill:red".into(), content: SvgInstanceContent::Original },
                SvgInstanceSnapshot { ancestry: vec![5], source: 3, presentation: "fill:blue".into(), content: SvgInstanceContent::Original },
            ],
            content_type: None, url_aliases: Vec::new(), url: "https://example.test/native.svg".into(), media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2" data-oryn-native-style-id="1"><defs data-oryn-native-style-id="2"><rect id="shape" width="2" height="2" style="fill:black!important" data-oryn-native-style-id="3"/></defs><use href="#shape" data-oryn-native-style-id="4"/><use href="#shape" x="2" data-oryn-native-style-id="5"/></svg>"##.to_vec(),
        }
    }

    #[test]
    fn author_important_declarations_keep_source_order_and_inline_precedence() {
        for style in [
            "fill:red!important;fill:blue!important",
            "fill:blue!important;fill:red",
        ] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: true,
                svg_instances: Vec::new(),
                content_type: None,
                url: "https://example.test/order.svg".into(),
                url_aliases: Vec::new(),
                media_type: "image/svg+xml".into(),
                bytes: format!("<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><style>rect{{fill:lime!important}}</style><rect width='1' height='1' style='{style}'/></svg>").into_bytes(),
            };
            let mut snapshot = crate::tests::empty_snapshot();
            snapshot.images.push(source);
            let tree = parse(&snapshot.images[0], &snapshot).unwrap();
            assert_eq!(
                raster(&tree, 1, 1).unwrap().get_pixel(0, 0).0,
                [0, 0, 255, 255]
            );
        }
    }

    #[test]
    fn captured_instance_metadata_round_trips_and_overrides_flattened_source_styles() {
        let source = captured_instance_fixture();
        let source: oryn_common::v4::PaintImageResource =
            serde_json::from_slice(&serde_json::to_vec(&source).unwrap()).unwrap();
        let tree = parse(&source, &crate::tests::empty_snapshot()).unwrap();
        let image = raster(&tree, 4, 2).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 0, 255, 255]);
    }

    #[test]
    fn captured_instance_metadata_rejects_fetched_duplicate_and_invalid_ancestry() {
        let mut source = captured_instance_fixture();
        source.svg_document_snapshot = false;
        assert!(captured_instance_maps(&source).is_err());
        source.svg_document_snapshot = true;
        source.svg_instances.push(source.svg_instances[0].clone());
        assert!(captured_instance_maps(&source).is_err());
        source.svg_instances.pop();
        source.svg_instances[0].ancestry.clear();
        assert!(captured_instance_maps(&source).is_err());
        source.svg_instances[0].ancestry = vec![4; 129];
        assert!(captured_instance_maps(&source).is_err());
    }

    #[test]
    fn captured_content_distinguishes_original_empty_and_native_reference() {
        use oryn_common::v4::SvgInstanceContent;
        let mut source = captured_instance_fixture();
        source.svg_instances[0].content = SvgInstanceContent::Empty;
        source.svg_instances[1].content = SvgInstanceContent::Reference("native-vector".into());
        let source: oryn_common::v4::PaintImageResource =
            serde_json::from_slice(&serde_json::to_vec(&source).unwrap()).unwrap();
        let (_, content) = captured_instance_maps(&source).unwrap();
        assert_eq!(
            content[&usvg::NativeInstanceKey {
                ancestry: vec![4],
                source: 3
            }],
            None
        );
        assert_eq!(
            content[&usvg::NativeInstanceKey {
                ancestry: vec![5],
                source: 3
            }]
                .as_deref(),
            Some("native-vector")
        );
    }

    #[test]
    fn native_content_vectors_ignore_author_selectors_while_authored_svg_keeps_them() {
        use oryn_common::v4::{PaintImageResource, SvgInstanceContent, SvgInstanceSnapshot};
        let source = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: vec![SvgInstanceSnapshot {
                ancestry: vec![4], source: 3, presentation: String::new(),
                content: SvgInstanceContent::Reference("native-vector".into()),
            }],
            content_type: None, url_aliases: Vec::new(),
            url: "https://example.test/native-content.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><style>rect { fill: lime !important }</style><defs><g id="shape" data-oryn-native-style-id="3"/><g id="native-vector"><rect width="2" height="2" fill="red"/></g></defs><use href="#shape" data-oryn-native-style-id="4"/><rect x="2" width="2" height="2" fill="blue"/></svg>"##.to_vec(),
        };
        let tree = parse(&source, &crate::tests::empty_snapshot()).unwrap();
        let image = raster(&tree, 4, 2).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn native_content_referenced_clip_definitions_ignore_author_selectors() {
        use oryn_common::v4::{PaintImageResource, SvgInstanceContent, SvgInstanceSnapshot};
        let source = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: vec![SvgInstanceSnapshot {
                ancestry: vec![4], source: 3, presentation: String::new(),
                content: SvgInstanceContent::Reference("native-vector".into()),
            }],
            content_type: None, url_aliases: Vec::new(),
            url: "https://example.test/native-content-clip.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><style>rect { fill: lime !important } clipPath rect { display: none !important }</style><defs><g id="shape" data-oryn-native-style-id="3"/><g id="native-vector"><defs><clipPath id="native-clip"><rect width="1" height="2"/></clipPath></defs><rect width="2" height="2" fill="red" clip-path="url(#native-clip)"/></g></defs><use href="#shape" data-oryn-native-style-id="4"/><rect x="2" width="2" height="2" fill="blue"/></svg>"##.to_vec(),
        };
        let tree = parse(&source, &crate::tests::empty_snapshot()).unwrap();
        let image = raster(&tree, 4, 2).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 0, 0]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn href_lookup_cannot_alias_a_native_capture_with_the_same_document_url() {
        let mut snapshot = crate::tests::empty_snapshot();
        let native = captured_instance_fixture();
        let href = native.url.clone();
        snapshot.images.push(native);
        snapshot.images.push(oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(), content_type: None,
            url_aliases: Vec::new(), url: href.clone(), media_type: "image/svg+xml".into(),
            bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="lime"/></svg>"#.to_vec(),
        });
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: true, svg_instances: Vec::new(), content_type: None,
            url_aliases: Vec::new(), url: "https://example.test/parent.svg".into(), media_type: "image/svg+xml".into(),
            bytes: format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><image href=\"{href}\" width=\"4\" height=\"2\"/></svg>").into_bytes(),
        };
        let tree = parse_filter(&source, &snapshot).unwrap();
        let image = raster(&tree, 4, 2).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 255, 0, 255]);
        snapshot.images.pop();
        assert!(
            parse_filter(&source, &snapshot).is_err(),
            "native capture alone cannot fulfill a fetched URL"
        );
    }

    #[test]
    fn oversized_filter_intermediate_is_rejected_before_allocation() {
        let snapshot = crate::tests::empty_snapshot();
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/filter.svg".into(),media_type:"image/svg+xml".into(),bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs><filter id="huge" filterUnits="userSpaceOnUse" x="0" y="0" width="100000" height="100000"><feGaussianBlur stdDeviation="2"/></filter></defs><g filter="url(#huge)"><rect width="10" height="10" fill="red"/></g></svg>"##.to_vec()};
        let tree = parse(&source, &snapshot).unwrap();
        assert!(validate_layers(&tree, 1., 1., 16 * 1024 * 1024).is_err());
        assert!(raster(&tree, 100, 100).is_err());
    }
    #[test]
    fn paths_gradients_clips_and_group_opacity_render_at_requested_resolution() {
        let mut snapshot = crate::tests::empty_snapshot();
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/vector.svg".into(),media_type:"image/svg+xml".into(),bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><clipPath id="c"><rect width="5" height="10"/></clipPath></defs><g opacity="0.5" clip-path="url(#c)"><path d="M0 0H10V10H0Z" fill="url(#g)"/></g></svg>"##.to_vec()};
        snapshot.images.push(source.clone());
        let tree = parse(&source, &snapshot).unwrap();
        let image = raster(&tree, 100, 100).unwrap();
        assert!(image.get_pixel(25, 50)[3] >= 127 && image.get_pixel(25, 50)[3] <= 128);
        assert_eq!(image.get_pixel(75, 50)[3], 0);
    }
    #[test]
    fn captured_relative_images_and_svg_text_use_snapshot_resources() {
        use image::ImageEncoder;
        let mut snapshot = crate::tests::empty_snapshot();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0, 255, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        snapshot.images.push(oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://example.test/assets/green.png".into(),
            media_type: "image/png".into(),
            bytes: png,
        });
        let mut source=oryn_common::v4::PaintImageResource{svg_document_snapshot:true, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/assets/vector.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><image href="green.png" width="20" height="20"/><text x="22" y="15" font-size="14">A</text></svg>"#.to_vec()};
        let tree = parse(&source, &snapshot).unwrap();
        let image = raster(&tree, 80, 40).unwrap();
        assert_eq!(image.get_pixel(10, 10).0, [0, 255, 0, 255]);
        assert!((44..80).any(|x| (0..40).any(|y| image.get_pixel(x, y)[3] > 0)));
        source.svg_document_snapshot = false;
        let tree = parse(&source, &snapshot).unwrap();
        let secure = raster(&tree, 80, 40).unwrap();
        assert_eq!(
            secure.get_pixel(10, 10)[3],
            0,
            "a fetched SVG cannot borrow a page-admitted image"
        );
    }

    #[test]
    fn filesystem_and_uncaptured_network_references_cannot_be_loaded() {
        let snapshot = crate::tests::empty_snapshot();
        let mut source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/vector.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="file:///etc/passwd" width="10" height="10"/></svg>"#.to_vec()};
        let secure = parse(&source, &snapshot).unwrap();
        assert!(
            raster(&secure, 10, 10)
                .unwrap()
                .pixels()
                .all(|pixel| pixel[3] == 0)
        );
        source.svg_document_snapshot = true;
        assert!(parse(&source, &snapshot).is_err());
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[test]
    fn captured_svg_decodes_charset_bom_and_canonicalizes_only_parser_input() {
        let source = b"<svg xmlns='http://www.w3.org/2000/svg'><text>\xe9</text></svg>";
        let decoded =
            xml_bytes_with_type(source, Some("image/svg+xml; charset=windows-1252")).unwrap();
        assert!(std::str::from_utf8(&decoded).unwrap().contains('é'));
        assert!(source.contains(&0xe9));
        let xml =
            "<?xml version='1.0' encoding='UTF-16'?><svg xmlns='http://www.w3.org/2000/svg'/>";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        let decoded =
            xml_bytes_with_type(&bytes, Some("image/svg+xml; charset=windows-1252")).unwrap();
        assert!(decoded.starts_with(b"<svg"));
        assert_eq!(xml_bytes(&decoded).unwrap(), decoded);
    }
}
