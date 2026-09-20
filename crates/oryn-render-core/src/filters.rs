//! CSS filter groups rendered by the same bounded native SVG filter engine as
//! image paint. Input pixels and filter source are private captured resources.

/// Inputs for apply source.
pub(super) struct FilterSourceInputs<'a> {
    pub(super) snapshot: &'a PaintSnapshot,
    pub(super) pixels: &'a [u8],
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) source: &'a str,
    pub(super) current: &'a str,
    pub(super) resolution: f64,
    pub(super) bounds: CssRect,
    pub(super) source_bounds: CssRect,
}
use super::*;
use base64::Engine;
thread_local! {static FILTER_PIXELS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
pub(super) struct RasterBudget(usize);
impl RasterBudget {
    pub(super) fn enter(pixels: usize) -> Result<Self, RenderError> {
        FILTER_PIXELS.with(|used| {
            let total = used
                .get()
                .checked_add(pixels)
                .filter(|n| *n <= 4 * 1024 * 1024)
                .ok_or(RenderError::PixelLimit {
                    actual: pixels,
                    limit: 4 * 1024 * 1024,
                })?;
            used.set(total);
            Ok(Self(pixels))
        })
    }
}
impl Drop for RasterBudget {
    fn drop(&mut self) {
        FILTER_PIXELS.with(|used| used.set(used.get().saturating_sub(self.0)));
    }
}

fn failure() -> RenderError {
    RenderError::Encode("invalid or unsupported native CSS filter".into())
}
fn scalar(text: &str, default: f64) -> Result<f64, RenderError> {
    let text = text.trim();
    let value = if text.is_empty() {
        default
    } else if let Some(percent) = text.strip_suffix('%') {
        percent.parse::<f64>().map_err(|_| failure())? / 100.
    } else {
        text.parse::<f64>().map_err(|_| failure())?
    };
    if value.is_finite() && value >= 0. {
        Ok(value)
    } else {
        Err(failure())
    }
}
fn functions(source: &str) -> Result<Vec<(&str, &str)>, RenderError> {
    let mut rest = source.trim();
    let mut result = Vec::new();
    if rest == "none" {
        return Ok(result);
    }
    while !rest.is_empty() {
        let start = rest.find('(').ok_or_else(failure)?;
        let name = rest[..start].trim();
        let mut depth = 1;
        let mut end = None;
        for (i, ch) in rest[start + 1..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + 1 + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.ok_or_else(failure)?;
        result.push((name, &rest[start + 1..end]));
        if result.len() > 64 {
            return Err(failure());
        }
        rest = rest[end + 1..].trim();
    }
    Ok(result)
}
fn length(text: &str) -> Result<f64, RenderError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0.);
    }
    let value = text
        .strip_suffix("px")
        .or_else(|| (text == "0").then_some(text))
        .ok_or_else(failure)?
        .parse::<f64>()
        .map_err(|_| failure())?;
    if value.is_finite() && value >= 0. {
        Ok(value)
    } else {
        Err(failure())
    }
}
pub(super) fn primitives(
    source: &str,
    current: &str,
    factor: f64,
) -> Result<(String, f64), RenderError> {
    let mut xml = String::new();
    let mut pad = 0.;
    for (name, args) in functions(source)? {
        match name {
            "blur" => {
                let sigma = length(args)? * factor;
                pad += sigma * 4.;
                xml += &format!("<feGaussianBlur stdDeviation=\"{sigma}\"/>");
            }
            "drop-shadow" => {
                let shadows =
                    oryn_common::box_shadow::parse(args, current, 16.).ok_or_else(failure)?;
                if shadows.len() != 1 || shadows[0].inset || shadows[0].spread != 0. {
                    return Err(failure());
                }
                let shadow = &shadows[0];
                let x = shadow.offset[0] * factor;
                let y = shadow.offset[1] * factor;
                let sigma = shadow.blur * factor;
                pad += x.abs().max(y.abs()) + sigma * 4.;
                xml += &format!(
                    "<feDropShadow dx=\"{x}\" dy=\"{y}\" stdDeviation=\"{sigma}\" flood-color=\"rgb({},{},{})\" flood-opacity=\"{}\"/>",
                    shadow.color[0],
                    shadow.color[1],
                    shadow.color[2],
                    f64::from(shadow.color[3]) / 255.
                );
            }
            "opacity" => {
                let v = scalar(args, 1.)?.min(1.);
                xml += &format!(
                    "<feComponentTransfer><feFuncA type=\"linear\" slope=\"{v}\"/></feComponentTransfer>"
                );
            }
            "brightness" | "contrast" | "invert" => {
                let v = scalar(args, 1.)?;
                let (slope, intercept) = match name {
                    "brightness" => (v, 0.),
                    "contrast" => (v, 0.5 - 0.5 * v),
                    _ => (1. - 2. * v.min(1.), v.min(1.)),
                };
                xml += "<feComponentTransfer>";
                for channel in ["R", "G", "B"] {
                    xml += &format!(
                        "<feFunc{channel} type=\"linear\" slope=\"{slope}\" intercept=\"{intercept}\"/>"
                    );
                }
                xml += "</feComponentTransfer>";
            }
            "saturate" | "grayscale" => {
                let v = scalar(args, 1.)?;
                let v = if name == "grayscale" {
                    1. - v.min(1.)
                } else {
                    v
                };
                xml += &format!("<feColorMatrix type=\"saturate\" values=\"{v}\"/>");
            }
            "sepia" => {
                let v = scalar(args, 1.)?.min(1.);
                let values = [
                    1. - 0.607 * v,
                    0.769 * v,
                    0.189 * v,
                    0.,
                    0.,
                    0.349 * v,
                    1. - 0.314 * v,
                    0.168 * v,
                    0.,
                    0.,
                    0.272 * v,
                    0.534 * v,
                    1. - 0.869 * v,
                    0.,
                    0.,
                    0.,
                    0.,
                    0.,
                    1.,
                    0.,
                ]
                .map(|n| n.to_string())
                .join(" ");
                xml += &format!("<feColorMatrix type=\"matrix\" values=\"{values}\"/>");
            }
            "hue-rotate" => {
                let args = args.trim();
                let angle = if args.is_empty() {
                    0.
                } else if let Some(v) = args.strip_suffix("deg") {
                    v.parse::<f64>().map_err(|_| failure())?
                } else if let Some(v) = args.strip_suffix("turn") {
                    v.parse::<f64>().map_err(|_| failure())? * 360.
                } else if let Some(v) = args.strip_suffix("grad") {
                    v.parse::<f64>().map_err(|_| failure())? * 0.9
                } else if let Some(v) = args.strip_suffix("rad") {
                    v.parse::<f64>().map_err(|_| failure())?.to_degrees()
                } else if args == "0" {
                    0.
                } else {
                    return Err(failure());
                };
                if !angle.is_finite() {
                    return Err(failure());
                }
                xml += &format!("<feColorMatrix type=\"hueRotate\" values=\"{angle}\"/>");
            }
            _ => return Err(failure()),
        }
    }
    if !pad.is_finite() || pad > 2048. {
        return Err(RenderError::Encode("native filter extent limit".into()));
    }
    Ok((xml, pad.ceil()))
}
pub(super) fn apply(
    pixels: &[u8],
    width: u32,
    height: u32,
    primitives: &str,
) -> Result<Vec<u8>, RenderError> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| RenderError::Encode(e.to_string()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\"><defs><filter id=\"f\" x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\">{primitives}</filter></defs><image width=\"{width}\" height=\"{height}\" href=\"data:image/png;base64,{data}\" filter=\"url(#f)\"/></svg>"
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .map_err(|e| RenderError::Encode(e.to_string()))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(failure)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.data().to_vec();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        if pixel[3] > 0 {
            for channel in 0..3 {
                pixel[channel] =
                    (u32::from(pixel[channel]) * 255 / u32::from(pixel[3])).min(255) as u8;
            }
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opacity_filter_removes_actual_alpha_without_changing_layout_bounds() {
        let (primitives, _) = primitives("opacity(0)", "black", 1.).unwrap();
        let rgba = apply(&[255, 0, 0, 255], 1, 1, &primitives).unwrap();
        assert_eq!(rgba[3], 0);
    }
    #[test]
    fn color_filters_preserve_transparent_image_holes() {
        let (primitives, _) = primitives(
            "brightness(2) contrast(50%) saturate(0) hue-rotate(1turn) sepia(0.5)",
            "black",
            1.,
        )
        .unwrap();
        let rgba = apply(&[255, 0, 0, 0, 0, 255, 0, 255], 2, 1, &primitives).unwrap();
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[7], 255);
    }
    #[test]
    fn blur_and_drop_shadow_extend_only_actual_ink() {
        let (primitives, pad) =
            primitives("blur(1px) drop-shadow(3px 0px 1px red)", "black", 1.).unwrap();
        assert!(pad >= 11.);
        let rgba = apply(&vec![0; 17 * 17 * 4], 17, 17, &primitives).unwrap();
        assert!(rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
        let mut source = vec![0; 17 * 17 * 4];
        source[(8 * 17 + 5) * 4 + 3] = 255;
        let rgba = apply(&source, 17, 17, &primitives).unwrap();
        assert!(rgba[(8 * 17 + 8) * 4 + 3] > 0);
    }
    #[test]
    fn undeclared_filter_resources_and_unbounded_kernels_are_errors() {
        assert!(primitives("url(https://author.test/filter.svg#f)", "black", 1.).is_err());
        assert!(primitives("blur(999999px)", "black", 1.).is_err());
    }
    #[test]
    fn filter_halo_crosses_renderer_tile_boundaries() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 128.;
        snapshot.viewport.height = 16.;
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 62.,
                y: 5.,
                width: 2.,
                height: 2.,
            },
        );
        node.style.insert("background-color".into(), "black".into());
        node.style.insert("filter".into(), "blur(2px)".into());
        snapshot.nodes.push(node);
        let options = RenderOptions {
            background: [0; 4],
            ..Default::default()
        };
        let scene = build_display_list(&snapshot, options).unwrap();
        let rgba = rasterize(&snapshot, &scene, options).unwrap();
        assert!(rgba[(6 * 128 + 65) * 4 + 3] > 0);
        assert_eq!(rgba[(6 * 128 + 100) * 4 + 3], 0);
    }
}

#[cfg(test)]
mod alpha_footprint_tests {
    use super::*;
    #[test]
    fn glyph_whitespace_and_image_alpha_remain_transparent_inside_layout_boxes() {
        let mut snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 30.,
            height: 20.,
        };
        let mut node = crate::tests::paint_node(1, None, "span", rect);
        node.projection
            .as_mut()
            .unwrap()
            .text
            .push(PaintTextFragment {
                flow_line: None,
                flow_rect: None,
                clusters: Vec::new(),
                text: "   ".into(),
                rect,
                glyph_runs: Vec::new(),
                font_family: "sans-serif".into(),
                font_size: 16.,
                color: "black".into(),
            });
        snapshot.nodes.push(node);
        let options = RenderOptions {
            background: [0; 4],
            ..Default::default()
        };
        let scene = build_display_list(&snapshot, options).unwrap();
        let rgba = rasterize(&snapshot, &scene, options).unwrap();
        assert!(rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
        snapshot.nodes[0].projection.as_mut().unwrap().text[0].text = "O".into();
        let scene = build_display_list(&snapshot, options).unwrap();
        let rgba = rasterize(&snapshot, &scene, options).unwrap();
        assert!(rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        assert!(rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 0], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        snapshot.images.push(oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type: None,
            url: "https://image.test/alpha.png".into(),
            url_aliases: Vec::new(),
            media_type: "image/png".into(),
            bytes: png,
        });
        snapshot.nodes[0].projection.as_mut().unwrap().text.clear();
        snapshot.nodes[0].image_resource = Some(0);
        let scene = build_display_list(&snapshot, options).unwrap();
        let rgba = rasterize(&snapshot, &scene, options).unwrap();
        assert!(rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
    }
}

#[cfg(test)]
mod transformed_filter_tests {
    use super::*;
    #[test]
    fn finite_rotated_nonuniform_and_perspective_filter_planes_use_local_pixels() {
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let mut nonuniform = identity;
        nonuniform[0] = 2.;
        nonuniform[5] = 0.5;
        nonuniform[12] = 12.;
        nonuniform[13] = 12.;
        let mut rotated = identity;
        rotated[0] = 0.;
        rotated[1] = 1.;
        rotated[4] = -1.;
        rotated[5] = 0.;
        rotated[12] = 30.;
        rotated[13] = 12.;
        let mut perspective = nonuniform;
        perspective[3] = 0.002;
        let snapshot = crate::tests::empty_snapshot();
        for plane in [nonuniform, rotated, perspective] {
            let make = |source: &str| DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 64,
                height: 64,
                diagnostics: Vec::new(),
                commands: vec![PaintCommand::Filter {
                    environment: SvgEnvironment::default(),
                    reference_box: None,
                    source: source.into(),
                    current_color: "black".into(),
                    plane,
                    commands: vec![PaintCommand::Projected {
                        matrix: plane,
                        clips: Vec::new(),
                        commands: vec![PaintCommand::FillRect {
                            rect: CssRect {
                                x: 0.,
                                y: 0.,
                                width: 8.,
                                height: 8.,
                            },
                            color: [255, 0, 0, 255],
                        }],
                    }],
                }],
            };
            let options = RenderOptions {
                background: [0; 4],
                ..Default::default()
            };
            let visible = rasterize(&snapshot, &make("blur(1px)"), options).unwrap();
            assert!(visible.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
            let hidden = rasterize(&snapshot, &make("blur(1px) opacity(0)"), options).unwrap();
            assert!(hidden.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
        }
    }
    #[test]
    fn singular_filter_plane_is_an_error_not_an_empty_visible_result() {
        let snapshot = crate::tests::empty_snapshot();
        let scene = DisplayList {
            contract_version: RENDER_CONTRACT_VERSION,
            width: 8,
            height: 8,
            diagnostics: Vec::new(),
            commands: vec![PaintCommand::Filter {
                environment: SvgEnvironment::default(),
                reference_box: None,
                source: "blur(1px)".into(),
                current_color: "black".into(),
                plane: [0.; 16],
                commands: vec![PaintCommand::FillRect {
                    rect: CssRect {
                        x: 0.,
                        y: 0.,
                        width: 2.,
                        height: 2.,
                    },
                    color: [0, 0, 0, 255],
                }],
            }],
        };
        assert!(rasterize(&snapshot, &scene, RenderOptions::default()).is_err());
    }
}

fn reference<'a>(
    snapshot: &'a PaintSnapshot,
    args: &str,
) -> Result<(&'a oryn_common::v4::PaintImageResource, String), RenderError> {
    let mut input = cssparser::ParserInput::new(args);
    let mut parser = cssparser::Parser::new(&mut input);
    let value = if let Ok(value) = parser.try_parse(|p| p.expect_string_cloned()) {
        parser.expect_exhausted().map_err(|_| failure())?;
        value.to_string()
    } else {
        args.trim().to_owned()
    };
    let (location, id) = value
        .rsplit_once('#')
        .filter(|(_, id)| !id.is_empty())
        .ok_or_else(failure)?;
    // Resource inventory can include several captured realms. A unique textual
    // ID is not proof that an inline definition belongs to this filter owner.
    if location.is_empty() || crate::masks::same_document_url(location, &snapshot.page.url) {
        let index = snapshot.local_effects.get(id).ok_or_else(|| {
            RenderError::Encode("local filter lacks native document binding".into())
        })?;
        let resource = snapshot
            .images
            .get(*index)
            .filter(|resource| resource.media_type == "image/svg+xml")
            .ok_or_else(|| RenderError::Encode("invalid native local filter resource".into()))?;
        return Ok((resource, id.into()));
    }

    let mut matches = snapshot
        .images
        .iter()
        .filter(|resource| {
            resource.media_type == "image/svg+xml"
                && (location.is_empty()
                    || resource.url == location
                    || resource.url_aliases.iter().any(|alias| alias == location))
        })
        .filter(|resource| {
            crate::svg::xml_source(resource).ok().is_some_and(|source| {
                roxmltree::Document::parse(&source).ok().is_some_and(|xml| {
                    xml.descendants().any(|node| {
                        node.is_element()
                            && node.tag_name().name() == "filter"
                            && node.attribute("id") == Some(id)
                    })
                })
            })
        });
    let source = matches.next().ok_or_else(|| {
        RenderError::Encode("filter reference absent from captured native SVG resources".into())
    })?;
    if matches.next().is_some() || source.bytes.len() > 4 * 1024 * 1024 {
        return Err(RenderError::Encode(
            "ambiguous or oversized native filter source".into(),
        ));
    }
    Ok((source, id.into()))
}
fn reference_region(
    source: &oryn_common::v4::PaintImageResource,
    id: &str,
    bounds: CssRect,
) -> Result<CssRect, RenderError> {
    let text = crate::svg::xml_source(source).map_err(|_| failure())?;
    let xml = roxmltree::Document::parse(&text).map_err(|_| failure())?;
    let filter = xml
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "filter"
                && node.attribute("id") == Some(id)
        })
        .ok_or_else(failure)?;
    let bbox = filter
        .attribute("filterUnits")
        .unwrap_or("objectBoundingBox")
        == "objectBoundingBox";
    let number =
        |name: &str, default: &str, origin: f64, extent: f64| -> Result<f64, RenderError> {
            let value = filter.attribute(name).unwrap_or(default);
            if let Some(percent) = value.strip_suffix('%') {
                let value = percent.parse::<f64>().map_err(|_| failure())? / 100.;
                Ok(origin + extent * value)
            } else {
                let value = value
                    .strip_suffix("px")
                    .unwrap_or(value)
                    .parse::<f64>()
                    .map_err(|_| failure())?;
                Ok(if bbox { origin + extent * value } else { value })
            }
        };
    let x = number("x", "-10%", bounds.x, bounds.width)?;
    let y = number("y", "-10%", bounds.y, bounds.height)?;
    let width = number("width", "120%", 0., bounds.width)?;
    let height = number("height", "120%", 0., bounds.height)?;
    if ![x, y, width, height].iter().all(|value| value.is_finite()) || width <= 0. || height <= 0. {
        return Err(failure());
    }
    Ok(CssRect {
        x,
        y,
        width,
        height,
    })
}
pub(super) fn padding(
    snapshot: &PaintSnapshot,
    source: &str,
    current: &str,
    bounds: CssRect,
) -> Result<f64, RenderError> {
    let mut padding = 0.;
    for (name, args) in functions(source)? {
        if name == "url" {
            let (resource, id) = reference(snapshot, args)?;
            let region = reference_region(resource, &id, bounds)?;
            padding += (bounds.x - region.x)
                .max(bounds.y - region.y)
                .max(region.right() - bounds.right())
                .max(region.bottom() - bounds.bottom())
                .max(0.);
        } else {
            padding += primitives(&format!("{name}({args})"), current, 1.)?.1;
        }
    }
    if !padding.is_finite() || padding > 2048. {
        return Err(failure());
    }
    Ok(padding.ceil())
}
pub(super) fn apply_source(params: FilterSourceInputs<'_>) -> Result<Vec<u8>, RenderError> {
    let FilterSourceInputs {
        snapshot,
        pixels,
        width,
        height,
        source,
        current,
        resolution,
        bounds,
        source_bounds,
    } = params;

    let mut pixels = pixels.to_vec();
    let input_bounds = source_bounds;
    for (name, args) in functions(source)? {
        if name != "url" {
            let (xml, _) = primitives(&format!("{name}({args})"), current, resolution)?;
            pixels = apply(&pixels, width, height, &xml)?;
            continue;
        }
        let (resource, id) = reference(snapshot, args)?;
        let text = crate::svg::xml_source(resource).map_err(|_| failure())?;
        let xml = roxmltree::Document::parse(&text).map_err(|_| failure())?;
        let root = xml.root_element();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|error| RenderError::Encode(error.to_string()))?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        let selected = xml
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "filter"
                    && node.attribute("id") == Some(id.as_str())
            })
            .ok_or_else(failure)?;
        for image in selected
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "feImage")
        {
            if image
                .attribute("href")
                .or_else(|| image.attribute(("http://www.w3.org/1999/xlink", "href")))
                .is_none_or(|href| href.trim().is_empty())
            {
                return Err(RenderError::Encode(
                    "filter feImage lacks an image reference".into(),
                ));
            }
        }
        let mut nonce = 0usize;
        let (graphic, alpha) = loop {
            let graphic = format!("oryn-native-source-{nonce}");
            let alpha = format!("oryn-native-alpha-{nonce}");
            if !text.contains(&graphic) && !text.contains(&alpha) {
                break (graphic, alpha);
            }
            nonce += 1;
        };
        let primitive_box = selected.attribute("primitiveUnits") == Some("objectBoundingBox");
        let (image_x, image_y, image_width, image_height) = if primitive_box {
            if input_bounds.width <= 0. || input_bounds.height <= 0. {
                return Err(failure());
            }
            (
                (bounds.x - input_bounds.x) / input_bounds.width,
                (bounds.y - input_bounds.y) / input_bounds.height,
                width as f64 / resolution / input_bounds.width,
                height as f64 / resolution / input_bounds.height,
            )
        } else {
            (
                bounds.x,
                bounds.y,
                width as f64 / resolution,
                height as f64 / resolution,
            )
        };
        let prefix = format!(
            "<feImage x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{data}\" result=\"{graphic}\"/><feColorMatrix in=\"{graphic}\" type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\" result=\"{alpha}\"/>",
            image_x, image_y, image_width, image_height
        );
        let mut first = true;
        let mut children = String::new();
        for child in selected.children() {
            if child.is_element() {
                children += &rewrite_filter_input(child, &text, first, &graphic, &alpha, 0)?;
                if child.tag_name().name().starts_with("fe") {
                    first = false;
                }
            } else {
                children += &text[child.range()];
            }
        }
        let attributes = selected
            .attributes()
            .map(|attribute| {
                format!(
                    " {}=\"{}\"",
                    attribute.name(),
                    xml_escape(attribute.value())
                )
            })
            .collect::<String>();
        let replacement = format!("<filter{attributes}>{prefix}{children}</filter>");
        let definitions = root
            .children()
            .filter(|node| node.is_element())
            .map(|node| &text[node.range()])
            .collect::<String>()
            .replace(&text[selected.range()], &replacement);
        // The transparent geometry fixes the SVG filter reference box, while
        // native feImage supplies all SourceGraphic pixels, including overflow.
        let document = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{width}\" height=\"{height}\" viewBox=\"{} {} {} {}\"><defs>{definitions}</defs><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"transparent\" filter=\"url(#{})\"/></svg>",
            bounds.x,
            bounds.y,
            width as f64 / resolution,
            height as f64 / resolution,
            input_bounds.x,
            input_bounds.y,
            input_bounds.width,
            input_bounds.height,
            xml_escape(&id)
        );
        let native = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type: None,
            url: resource.url.clone(),
            url_aliases: Vec::new(),
            media_type: "image/svg+xml".into(),
            bytes: document.into_bytes(),
        };
        let tree = crate::svg::parse_filter(&native, snapshot)?;
        crate::svg::validate_layers(&tree, 1., 1., 4 * 1024 * 1024)?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(failure)?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        pixels = pixmap.data().to_vec();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            if pixel[3] > 0 {
                for channel in 0..3 {
                    pixel[channel] =
                        (u32::from(pixel[channel]) * 255 / u32::from(pixel[3])).min(255) as u8;
                }
            }
        }
        // resvg allocates integer filter surfaces. Preserve the filter's actual
        // CSS region at the final composition boundary, including fractional
        // pixel coverage, rather than painting its rounded allocation rectangle.
        clip_region(&mut pixels, width, height, &tree, &id)?;
    }
    Ok(pixels)
}

fn clip_region(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    tree: &resvg::usvg::Tree,
    id: &str,
) -> Result<(), RenderError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|size| size.checked_mul(4))
        .ok_or_else(failure)?;
    if pixels.len() != expected {
        return Err(failure());
    }
    fn find(
        group: &resvg::usvg::Group,
        id: &str,
        regions: &mut Vec<(resvg::usvg::NonZeroRect, resvg::tiny_skia::Transform)>,
    ) {
        for filter in group.filters() {
            if filter.id() == id {
                regions.push((filter.rect(), group.abs_transform()));
            }
        }
        // Only the wrapper's rendered tree owns this final clip. Filter/image
        // subroots have independent coordinates and must not supply its region.
        for node in group.children() {
            if let resvg::usvg::Node::Group(group) = node {
                find(group, id, regions);
            }
        }
    }
    let mut regions = Vec::new();
    find(tree.root(), id, &mut regions);
    let [(rect, transform)] = regions.as_slice() else {
        return Err(RenderError::Encode(
            "filter output region is missing or ambiguous".into(),
        ));
    };
    // Apply the exact area in output-pixel coordinates once. An intermediate
    // 8-bit antialias mask quantizes half coverage to 127 before multiplication.
    let quad = [
        [rect.x(), rect.y()],
        [rect.right(), rect.y()],
        [rect.right(), rect.bottom()],
        [rect.x(), rect.bottom()],
    ]
    .map(|[x, y]| {
        let (x, y) = (f64::from(x), f64::from(y));
        [
            f64::from(transform.sx) * x + f64::from(transform.kx) * y + f64::from(transform.tx),
            f64::from(transform.ky) * x + f64::from(transform.sy) * y + f64::from(transform.ty),
        ]
    });
    if quad
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err(failure());
    }
    let bounds = [
        quad.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        quad.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        quad.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
        quad.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
    ];
    let aligned =
        (transform.kx == 0. && transform.ky == 0.) || (transform.sx == 0. && transform.sy == 0.);
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (index % width as usize) as f64;
        let y = (index / width as usize) as f64;
        let horizontal = (bounds[2].min(x + 1.) - bounds[0].max(x)).max(0.);
        let vertical = (bounds[3].min(y + 1.) - bounds[1].max(y)).max(0.);
        let coverage = if horizontal == 0. || vertical == 0. {
            0.
        } else if aligned {
            horizontal * vertical
        } else {
            filter_pixel_coverage(&quad, x, y)
        };
        pixel[3] = (f64::from(pixel[3]) * coverage.clamp(0., 1.)).round() as u8;
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
    Ok(())
}

// Intersect an affine rectangle with one pixel. Convex clipping adds at most
// one vertex per boundary: four original vertices plus four pixel boundaries.
// Fixed storage avoids allocations for rotated or skewed edge pixels.
fn filter_pixel_coverage(quad: &[[f64; 2]; 4], x: f64, y: f64) -> f64 {
    let mut polygon = [[0.; 2]; 8];
    for (target, source) in polygon.iter_mut().zip(quad) {
        *target = [source[0] - x, source[1] - y];
    }
    let mut count = 4;
    for (axis, bound, lower) in [(0, 0., true), (0, 1., false), (1, 0., true), (1, 1., false)] {
        if count == 0 {
            return 0.;
        }
        let mut clipped = [[0.; 2]; 8];
        let mut output = 0;
        let inside = |point: [f64; 2]| {
            if lower {
                point[axis] >= bound
            } else {
                point[axis] <= bound
            }
        };
        let mut previous = polygon[count - 1];
        for current in &polygon[..count] {
            let current = *current;
            if inside(previous) != inside(current) {
                let ratio = (bound - previous[axis]) / (current[axis] - previous[axis]);
                let mut intersection = [
                    previous[0] + ratio * (current[0] - previous[0]),
                    previous[1] + ratio * (current[1] - previous[1]),
                ];
                intersection[axis] = bound;
                clipped[output] = intersection;
                output += 1;
            }
            if inside(current) {
                clipped[output] = current;
                output += 1;
            }
            previous = current;
        }
        polygon = clipped;
        count = output;
    }
    if count < 3 {
        return 0.;
    }
    let mut twice_area = 0.;
    let mut previous = polygon[count - 1];
    for current in &polygon[..count] {
        twice_area += previous[0] * current[1] - current[0] * previous[1];
        previous = *current;
    }
    (twice_area.abs() * 0.5).clamp(0., 1.)
}

#[cfg(test)]
mod captured_reference_tests {
    use super::*;
    #[test]
    fn url_filter_uses_only_the_captured_definition_and_preserves_transparent_input() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://effects.test/filter.svg".into(),url_aliases:vec!["https://effects.test/redirect.svg".into()],media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="invert"><feComponentTransfer><feFuncR type="table" tableValues="1 0"/><feFuncG type="table" tableValues="1 0"/><feFuncB type="table" tableValues="1 0"/></feComponentTransfer></filter></svg>"#.to_vec()});
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 2.,
            height: 1.,
        };
        let pixels = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[255, 0, 0, 255, 0, 0, 0, 0],
            width: 2,
            height: 1,
            source: "url(https://effects.test/redirect.svg#invert)",
            current: "black",
            resolution: 1.,
            bounds: rect,
            source_bounds: rect,
        })
        .unwrap();
        assert_eq!(&pixels[..4], &[0, 255, 255, 255]);
        assert_eq!(pixels[7], 0);
        assert!(
            padding(
                &snapshot,
                "url(https://unfetched.test/filter.svg#invert)",
                "black",
                rect
            )
            .is_err()
        );
        assert!(
            padding(
                &snapshot,
                "url(https://effects.test/filter.svg#missing)",
                "black",
                rect
            )
            .is_err()
        );
        let restored: DisplayList = serde_json::from_slice(
            &serde_json::to_vec(&DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 2,
                height: 1,
                diagnostics: Vec::new(),
                commands: vec![PaintCommand::Filter {
                    environment: SvgEnvironment::default(),
                    reference_box: None,
                    source: "url(https://effects.test/filter.svg#invert)".into(),
                    current_color: "black".into(),
                    plane: [
                        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                    ],
                    commands: vec![PaintCommand::FillRect {
                        rect,
                        color: [255, 0, 0, 255],
                    }],
                }],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(restored.contract_version, 3);
        assert!(rasterize(&snapshot, &restored, RenderOptions::default()).is_ok());
    }
    #[test]
    fn strict_filter_dependencies_keep_secure_authority_and_reject_nested_missing_images() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let data = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let nested=br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image width="1" height="1" href="https://effects.test/captured.png"/></svg>"#;
        let nested = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(nested)
        );
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        };
        for (href, success) in [
            (data.as_str(), true),
            ("#pixel", true),
            ("#missing", false),
            ("https://effects.test/captured.png", false),
            (nested.as_str(), false),
        ] {
            let mut snapshot = crate::tests::empty_snapshot();
            snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://effects.test/filter.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"pixel\" width=\"1\" height=\"1\" fill=\"red\"/><filter id=\"active\"><feImage href=\"{}\"/></filter><filter id=\"unused\"><feImage href=\"not-captured.png\"/></filter></svg>",xml_escape(href)).into_bytes()});
            // Matching embedding-resource bytes do not authorize fetched SVG IO.
            snapshot.images.push(oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                svg_instances: Vec::new(),
                content_type: None,
                url: "https://effects.test/captured.png".into(),
                url_aliases: Vec::new(),
                media_type: "image/png".into(),
                bytes: png.clone(),
            });
            let result = apply_source(FilterSourceInputs {
                snapshot: &snapshot,
                pixels: &[0; 4],
                width: 1,
                height: 1,
                source: "url(https://effects.test/filter.svg#active)",
                current: "black",
                resolution: 1.,
                bounds: rect,
                source_bounds: rect,
            });
            if success {
                assert_eq!(result.unwrap(), vec![255, 0, 0, 255], "{href}");
            } else {
                assert!(result.is_err(), "unavailable active dependency: {href}");
            }
        }
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://effects.test/ordinary.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image width="1" height="1" href="https://effects.test/not-captured.png"/></svg>"#.to_vec()};
        let snapshot = crate::tests::empty_snapshot();
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert_eq!(
            crate::svg::raster(&tree, 1, 1).unwrap().get_pixel(0, 0).0[3],
            0
        );
        let mut snapshot = snapshot;
        snapshot.images.push(oryn_common::v4::PaintImageResource{bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="empty"><feImage/></filter></svg>"#.to_vec(),url:"https://effects.test/filter.svg".into(),..source});
        assert!(
            apply_source(FilterSourceInputs {
                snapshot: &snapshot,
                pixels: &[255; 4],
                width: 1,
                height: 1,
                source: "url(https://effects.test/filter.svg#empty)",
                current: "black",
                resolution: 1.,
                bounds: rect,
                source_bounds: rect
            })
            .unwrap_err()
            .to_string()
            .contains("lacks an image reference")
        );
    }
    #[test]
    fn undeclared_nested_filter_images_are_not_fetched_or_silently_ignored() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://effects.test/filter.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="external"><feImage href="not-captured.png"/></filter></svg>"#.to_vec()});
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        };
        assert!(
            apply_source(FilterSourceInputs {
                snapshot: &snapshot,
                pixels: &[255; 4],
                width: 1,
                height: 1,
                source: "url(https://effects.test/filter.svg#external)",
                current: "black",
                resolution: 1.,
                bounds: rect,
                source_bounds: rect
            })
            .is_err()
        );
    }
}

#[cfg(test)]
mod local_binding_tests {
    use super::*;
    #[test]
    fn local_fragment_uses_native_binding_even_with_same_url_foreign_resource() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.page.url = "https://same.test/".into();
        for color in ["red", "blue"] {
            snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:snapshot.page.url.clone(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><filter id=\"f\"><feFlood flood-color=\"{color}\"/></filter></svg>").into_bytes()});
        }
        assert!(reference(&snapshot, "#f").is_err());
        snapshot.local_effects.insert("f".into(), 1);
        let (resource, _) = reference(&snapshot, "#f").unwrap();
        assert!(String::from_utf8_lossy(&resource.bytes).contains("blue"));
        let (resource, _) = reference(&snapshot, "https://same.test/#f").unwrap();
        assert!(String::from_utf8_lossy(&resource.bytes).contains("blue"));
        snapshot.local_effects.insert("f".into(), 9);
        assert!(reference(&snapshot, "#f").is_err());
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
fn rewrite_filter_input(
    node: roxmltree::Node<'_, '_>,
    source: &str,
    first: bool,
    graphic: &str,
    alpha: &str,
    depth: usize,
) -> Result<String, RenderError> {
    if depth > 64 {
        return Err(failure());
    }
    let name = node.tag_name().name();
    let mut attributes = String::new();
    let mut has_input = false;
    let mut has_second = false;
    for attribute in node.attributes() {
        let key = if attribute.namespace() == Some("http://www.w3.org/1999/xlink") {
            format!("xlink:{}", attribute.name())
        } else {
            attribute.name().to_owned()
        };
        let mut value = attribute.value();
        if matches!(attribute.name(), "in" | "in2") {
            has_input |= attribute.name() == "in";
            has_second |= attribute.name() == "in2";
            value = match value {
                "SourceGraphic" => graphic,
                "SourceAlpha" => alpha,
                _ => value,
            };
        }
        attributes += &format!(" {key}=\"{}\"", xml_escape(value));
    }
    if first
        && name.starts_with("fe")
        && !has_input
        && !matches!(name, "feImage" | "feFlood" | "feTurbulence" | "feMerge")
    {
        attributes += &format!(" in=\"{graphic}\"");
    }
    if !has_second && matches!(name, "feBlend" | "feComposite" | "feDisplacementMap") {
        attributes += &format!(" in2=\"{graphic}\"");
    }
    let mut children = String::new();
    for child in node.children() {
        if child.is_element() {
            children += &rewrite_filter_input(
                child,
                source,
                first && name == "feMerge",
                graphic,
                alpha,
                depth + 1,
            )?;
        } else {
            children += &source[child.range()];
        }
    }
    Ok(format!("<{name}{attributes}>{children}</{name}>"))
}

#[cfg(test)]
mod reference_box_tests {
    use super::*;
    #[test]
    fn affine_filter_coverage_preserves_area_and_reflected_winding() {
        let diamond = [[1., 0.], [2., 1.], [1., 2.], [0., 1.]];
        for quad in [diamond, [diamond[3], diamond[2], diamond[1], diamond[0]]] {
            let mut area = 0.;
            for y in 0..2 {
                for x in 0..2 {
                    let coverage = filter_pixel_coverage(&quad, f64::from(x), f64::from(y));
                    assert_eq!(coverage, 0.5);
                    area += coverage;
                }
            }
            assert_eq!(area, 2.);
            assert_eq!(filter_pixel_coverage(&quad, 2., 0.), 0.);
            let translated = quad.map(|p| [p[0] + 1_000_000., p[1] - 1_000_000.]);
            assert_eq!(
                filter_pixel_coverage(&translated, 1_000_000., -1_000_000.),
                0.5
            );
        }
        // Clipping all four diamond tips produces the maximum eight vertices.
        assert_eq!(
            filter_pixel_coverage(
                &[[0.5, -0.25], [1.25, 0.5], [0.5, 1.25], [-0.25, 0.5]],
                0.,
                0.
            ),
            0.875
        );
        assert_eq!(
            filter_pixel_coverage(&[[-1., -1.], [2., -1.], [2., 2.], [-1., 2.]], 0., 0.),
            1.
        );
        assert_eq!(filter_pixel_coverage(&[[0., 0.]; 4], 0., 0.), 0.);
    }

    #[test]
    fn fractional_filter_region_clips_rounded_surfaces_at_css_pixel_coverage() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://filter.test/fractional.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="f" filterUnits="userSpaceOnUse" x="0.5" y="0.5" width="1.5" height="1.5"><feFlood flood-color="red"/></filter></svg>"#.to_vec()});
        let bounds = CssRect {
            x: 0.,
            y: 0.,
            width: 3.,
            height: 3.,
        };
        let pixels = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[0; 36],
            width: 3,
            height: 3,
            source: "url(https://filter.test/fractional.svg#f)",
            current: "black",
            resolution: 1.,
            bounds,
            source_bounds: bounds,
        })
        .unwrap();
        assert_eq!(&pixels[..4], &[255, 0, 0, 64]);
        assert_eq!(&pixels[4..8], &[255, 0, 0, 128]);
        assert_eq!(&pixels[16..20], &[255, 0, 0, 255]);
        for index in [2, 5, 6, 7, 8] {
            assert_eq!(&pixels[index * 4..index * 4 + 4], &[0; 4]);
        }
        // CSS-space clipping scales with output resolution, not the reference
        // box's integer allocation or the alpha content of SourceGraphic.
        let pixels = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[0; 6 * 6 * 4],
            width: 6,
            height: 6,
            source: "url(https://filter.test/fractional.svg#f)",
            current: "black",
            resolution: 2.,
            bounds,
            source_bounds: bounds,
        })
        .unwrap();
        assert_eq!(pixels[3], 0);
        assert_eq!(
            &pixels[(2 * 6 + 2) * 4..(2 * 6 + 2) * 4 + 4],
            &[255, 0, 0, 255]
        );
        assert_eq!(pixels[(5 * 6 + 5) * 4 + 3], 0);
    }
    #[test]
    fn user_space_filter_percentages_use_svg_viewport_not_source_graphic_box() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://filter.test/percent.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="f" filterUnits="userSpaceOnUse" x="0%" y="0%" width="50%" height="100%"><feFlood flood-color="red"/></filter></svg>"#.to_vec()});
        let bounds = CssRect {
            x: 0.,
            y: 0.,
            width: 3.,
            height: 1.,
        };
        let reference = CssRect {
            x: 1.,
            y: 0.,
            width: 1.,
            height: 1.,
        };
        let pixels = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[0; 12],
            width: 3,
            height: 1,
            source: "url(https://filter.test/percent.svg#f)",
            current: "black",
            resolution: 1.,
            bounds,
            source_bounds: reference,
        })
        .unwrap();
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[255, 0, 0, 128]);
        assert_eq!(&pixels[8..], &[0; 4]);
    }
    #[test]
    fn url_filter_preserves_overflow_source_pixels_and_chain_reference_box() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://filter.test/f.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="move" filterUnits="userSpaceOnUse" x="0" y="0" width="3" height="1"><feOffset dx="1"/></filter><filter id="flood"><feFlood flood-color="red"/></filter></svg>"#.to_vec()});
        let bounds = CssRect {
            x: 0.,
            y: 0.,
            width: 3.,
            height: 1.,
        };
        let reference = CssRect {
            x: 1.,
            y: 0.,
            width: 1.,
            height: 1.,
        };
        let moved = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0],
            width: 3,
            height: 1,
            source: "url(https://filter.test/f.svg#move)",
            current: "black",
            resolution: 1.,
            bounds,
            source_bounds: reference,
        })
        .unwrap();
        assert_eq!(&moved[4..8], &[255, 0, 0, 255]);
        let flood = apply_source(FilterSourceInputs {
            snapshot: &snapshot,
            pixels: &[255; 12],
            width: 3,
            height: 1,
            source: "opacity(0) url(https://filter.test/f.svg#flood)",
            current: "black",
            resolution: 1.,
            bounds,
            source_bounds: reference,
        })
        .unwrap();
        assert_eq!(&flood[4..8], &[255, 0, 0, 255]);
        assert!(flood[3] < 255 && flood[11] < 255);
    }
}
