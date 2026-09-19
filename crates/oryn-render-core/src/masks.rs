//! Native CSS mask layers reuse captured image and gradient sampling.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MaskComposite {
    Add,
    Subtract,
    Intersect,
    Exclude,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskLayer {
    pub commands: Vec<PaintCommand>,
    pub luminance: bool,
    pub composite: MaskComposite,
}
fn item<'a>(
    style: &'a std::collections::BTreeMap<String, String>,
    name: &str,
    default: &'a str,
    index: usize,
) -> Result<&'a str, RenderError> {
    let parts = gradients::split(style.get(name).map(String::as_str).unwrap_or(default), ',')
        .ok_or_else(|| RenderError::Encode("invalid mask list".into()))?;
    parts
        .get(index % parts.len().max(1))
        .copied()
        .ok_or_else(|| RenderError::Encode("empty mask list".into()))
}
pub(super) fn layers(
    snapshot: &PaintSnapshot,
    node: &oryn_common::v4::PaintNodeSnapshot,
) -> Result<Vec<MaskLayer>, RenderError> {
    if node
        .style
        .get("mask-border-source")
        .is_some_and(|value| value != "none")
    {
        return Err(RenderError::Encode(
            "mask-border source requires native nine-slice mask".into(),
        ));
    }
    let value = node
        .style
        .get("mask-image")
        .map(String::as_str)
        .unwrap_or("none");
    if value == "none" {
        return Ok(Vec::new());
    }
    let model = node
        .projection
        .as_ref()
        .map(|p| &p.box_model)
        .or(node.geometry.box_model.as_ref())
        .ok_or_else(|| RenderError::Encode("mask geometry missing".into()))?;
    let images = gradients::split(value, ',')
        .ok_or_else(|| RenderError::Encode("invalid mask image list".into()))?;
    if images.len() > 256 {
        return Err(RenderError::Encode("mask layer bound".into()));
    }
    let mut layers = Vec::new();
    for (index, image) in images.into_iter().enumerate().rev() {
        let mut style = node.style.clone();
        style.insert("background-image".into(), image.into());
        style.insert("background-attachment".into(), "scroll".into());
        let no_clip = item(&node.style, "mask-clip", "border-box", index)? == "no-clip";
        for (suffix, default) in [
            ("position", "0% 0%"),
            ("size", "auto"),
            ("repeat", "repeat"),
            ("origin", "border-box"),
            ("clip", "border-box"),
        ] {
            let value = item(&node.style, &format!("mask-{suffix}"), default, index)?;
            style.insert(
                format!("background-{suffix}"),
                if value == "no-clip" {
                    "border-box"
                } else {
                    value
                }
                .into(),
            );
        }
        let clip = if no_clip {
            Some(
                backgrounds::canvas_rect(node, snapshot.viewport).ok_or_else(|| {
                    RenderError::Encode("mask no-clip projection is unbounded".into())
                })?,
            )
        } else {
            None
        };
        let mode = item(&node.style, "mask-mode", "match-source", index)?;
        let mut commands = if let Some((resource, fragment)) = svg_reference(snapshot, image)? {
            vec![PaintCommand::SvgMask {
                resource,
                fragment,
                rect: model.border_box,
                mode: mode.into(),
                environment: SvgEnvironment::for_style(
                    &node.style,
                    model.border_box,
                    snapshot.viewport,
                ),
            }]
        } else {
            backgrounds::commands(
                snapshot,
                &style,
                model,
                borders::radii(&node.style, model.border_box),
                None,
                false,
                clip,
            )
            .map_err(RenderError::Encode)?
        };
        if let Some(projection) = &node.projection {
            commands = vec![PaintCommand::Projected {
                matrix: projection.matrix,
                clips: projection.clips.clone(),
                commands,
            }];
        }
        let luminance = match item(&node.style, "mask-mode", "match-source", index)? {
            "alpha" | "match-source" => false,
            "luminance" => true,
            _ => return Err(RenderError::Encode("invalid mask mode".into())),
        };
        let composite = match item(&node.style, "mask-composite", "add", index)? {
            "add" => MaskComposite::Add,
            "subtract" => MaskComposite::Subtract,
            "intersect" => MaskComposite::Intersect,
            "exclude" => MaskComposite::Exclude,
            _ => return Err(RenderError::Encode("invalid mask composite".into())),
        };
        layers.push(MaskLayer {
            commands,
            luminance,
            composite,
        });
    }
    Ok(layers)
}
pub(super) fn alpha(pixel: &[u8], luminance: bool) -> f64 {
    let alpha = f64::from(pixel[3]) / 255.;
    if luminance {
        alpha
            * (0.2125 * f64::from(pixel[0])
                + 0.7154 * f64::from(pixel[1])
                + 0.0721 * f64::from(pixel[2]))
            / 255.
    } else {
        alpha
    }
}
pub(super) fn combine(source: f64, destination: f64, operation: MaskComposite) -> f64 {
    match operation {
        MaskComposite::Add => source + destination * (1. - source),
        MaskComposite::Subtract => source * (1. - destination),
        MaskComposite::Intersect => source * destination,
        MaskComposite::Exclude => source * (1. - destination) + destination * (1. - source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mask_pixels_apply_alpha_and_luminance_to_the_entire_group() {
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 2.,
            height: 1.,
        };
        for (luminance, color, expected) in [
            (false, [0, 0, 0, 128], 128),
            (true, [0, 0, 0, 255], 0),
            (true, [255; 4], 255),
        ] {
            let scene = DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 2,
                height: 1,
                diagnostics: Vec::new(),
                commands: vec![PaintCommand::Mask {
                    layers: vec![MaskLayer {
                        commands: vec![PaintCommand::FillRect { rect, color }],
                        luminance,
                        composite: MaskComposite::Add,
                    }],
                    commands: vec![PaintCommand::FillRect {
                        rect,
                        color: [255, 0, 0, 255],
                    }],
                }],
            };
            let pixels = rasterize(
                &snapshot,
                &scene,
                RenderOptions {
                    background: [0; 4],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(pixels[3], expected);
            assert_eq!(pixels[7], expected);
        }
    }
    #[test]
    fn mask_composition_and_empty_bottom_layer_are_not_ordinary_color_over() {
        assert_eq!(combine(1., 1., MaskComposite::Subtract), 0.);
        assert_eq!(combine(1., 1., MaskComposite::Exclude), 0.);
        assert_eq!(combine(0.5, 0.5, MaskComposite::Intersect), 0.25);
        let mut snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 20.,
        };
        let mut parent = crate::tests::paint_node(1, None, "div", rect);
        parent.style.insert(
            "mask-image".into(),
            "linear-gradient(transparent, transparent)".into(),
        );
        parent.style.insert("background-color".into(), "red".into());
        let mut dialog = crate::tests::paint_node(2, Some(1), "dialog", rect);
        dialog.top_layer = Some(1);
        dialog
            .style
            .insert("background-color".into(), "green".into());
        snapshot.nodes = vec![parent, dialog];
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let pixels = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        assert_eq!(&pixels[..4], &[0, 128, 0, 255]);
    }
    #[test]
    fn backdrop_samples_previous_scene_pixels_and_clips_result() {
        let snapshot = crate::tests::empty_snapshot();
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let scene = DisplayList {
            contract_version: RENDER_CONTRACT_VERSION,
            width: 128,
            height: 4,
            diagnostics: Vec::new(),
            commands: vec![
                PaintCommand::FillRect {
                    rect: CssRect {
                        x: 0.,
                        y: 0.,
                        width: 128.,
                        height: 4.,
                    },
                    color: [255, 0, 0, 255],
                },
                PaintCommand::BackdropFilter {
                    environment: SvgEnvironment::default(),
                    source: "invert(1)".into(),
                    current_color: "black".into(),
                    plane: identity,
                    rect: CssRect {
                        x: 60.,
                        y: 0.,
                        width: 8.,
                        height: 4.,
                    },
                    radii: [0.; 8],
                },
            ],
        };
        let pixels = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        for x in [59, 60, 63, 64, 67, 68] {
            assert_eq!(
                &pixels[x * 4..x * 4 + 4],
                if (60..68).contains(&x) {
                    &[0, 255, 255, 255]
                } else {
                    &[255, 0, 0, 255]
                }
            );
        }
    }
    #[test]
    fn old_display_list_contract_cannot_be_rendered_as_schema_three() {
        let snapshot = crate::tests::empty_snapshot();
        let scene = DisplayList {
            contract_version: 2,
            width: 1,
            height: 1,
            commands: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert!(rasterize(&snapshot, &scene, RenderOptions::default()).is_err());
        assert!(crate::print::vector::tree(&snapshot, &scene, RenderOptions::default()).is_err());
    }
}

fn svg_reference(
    snapshot: &PaintSnapshot,
    image: &str,
) -> Result<Option<(usize, String)>, RenderError> {
    let mut input = cssparser::ParserInput::new(image);
    let mut parser = cssparser::Parser::new(&mut input);
    let Ok(value) =
        parser.try_parse(|parser| parser.expect_url_or_string().map(|value| value.to_string()))
    else {
        return Ok(None);
    };
    let Some((location, fragment)) = value.rsplit_once('#') else {
        return Ok(None);
    };
    if location.is_empty() || same_document_url(location, &snapshot.page.url) {
        let index = *snapshot.local_effects.get(fragment).ok_or_else(|| {
            RenderError::Encode("local SVG mask lacks native document binding".into())
        })?;
        return Ok(Some((index, fragment.into())));
    }
    let mut candidates = snapshot.images.iter().enumerate().filter(|(_, resource)| {
        resource.media_type == "image/svg+xml"
            && (resource.url == location
                || resource.url_aliases.iter().any(|alias| alias == location))
    });
    let Some((index, resource)) = candidates.next() else {
        return Err(RenderError::Encode("SVG mask resource not captured".into()));
    };
    if candidates.next().is_some() || resource.bytes.len() > 4 * 1024 * 1024 {
        return Err(RenderError::Encode(
            "ambiguous or oversized SVG mask".into(),
        ));
    }
    let text = crate::svg::xml_source(resource).map_err(RenderError::Encode)?;
    let xml = roxmltree::Document::parse(&text)
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    if xml.descendants().any(|node| {
        node.is_element()
            && node.tag_name().name() == "mask"
            && node.attribute("id") == Some(fragment)
    }) {
        Ok(Some((index, fragment.into())))
    } else {
        Ok(None)
    }
}
pub(super) fn svg_image(
    snapshot: &PaintSnapshot,
    resource: usize,
    fragment: &str,
    rect: CssRect,
    mode: &str,
    scale: f64,
) -> Result<image::RgbaImage, RenderError> {
    let resource = snapshot
        .images
        .get(resource)
        .ok_or_else(|| RenderError::Encode("missing SVG mask resource".into()))?;
    let text = crate::svg::xml_source(resource).map_err(RenderError::Encode)?;
    let xml = roxmltree::Document::parse(&text)
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    let mask = xml
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mask"
                && node.attribute("id") == Some(fragment)
        })
        .ok_or_else(|| RenderError::Encode("SVG mask identity absent".into()))?;
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    };
    let definitions = xml
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .map(|node| &text[node.range()])
        .collect::<String>();
    let definitions = if mode != "match-source" {
        if !matches!(mode, "alpha" | "luminance") {
            return Err(RenderError::Encode("invalid SVG mask mode".into()));
        }
        let original = &text[mask.range()];
        let attributes = mask
            .attributes()
            .filter(|attribute| attribute.name() != "style")
            .map(|attribute| format!(" {}=\"{}\"", attribute.name(), escape(attribute.value())))
            .collect::<String>();
        let children = mask
            .children()
            .map(|child| &text[child.range()])
            .collect::<String>();
        let style = format!(
            "{};mask-type:{} !important",
            mask.attribute("style").unwrap_or(""),
            mode
        );
        let changed = format!(
            "<mask{attributes} style=\"{}\">{children}</mask>",
            escape(&style)
        );
        definitions.replace(original, &changed)
    } else {
        definitions
    };
    let width = (rect.width * scale).ceil();
    let height = (rect.height * scale).ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.
        || height < 1.
        || width * height > 4. * 1024. * 1024.
    {
        return Err(RenderError::Encode("SVG mask raster bound".into()));
    }
    let (width, height) = (width as u32, height as u32);
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{width}\" height=\"{height}\" viewBox=\"{} {} {} {}\"><defs>{definitions}</defs><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\" mask=\"url(#{})\"/></svg>",
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        escape(fragment)
    );
    let source = oryn_common::v4::PaintImageResource {
        svg_document_snapshot: false,
        content_type: None,
        url: resource.url.clone(),
        url_aliases: Vec::new(),
        media_type: "image/svg+xml".into(),
        bytes: document.into_bytes(),
    };
    let tree = crate::svg::parse(&source, snapshot)?;
    crate::svg::validate_layers(&tree, 1., 1., 4 * 1024 * 1024)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| RenderError::Encode("SVG mask allocation".into()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.data().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] > 0 {
            pixel[0] = 255;
            pixel[1] = 255;
            pixel[2] = 255;
        }
    }
    image::RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| RenderError::Encode("SVG mask output".into()))
}

#[cfg(test)]
mod svg_tests {
    use super::*;
    #[test]
    fn captured_svg_mask_fragment_obeys_native_mode_override_and_rejects_foreign_local_ids() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://mask.test/m.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><mask id="m" style="mask-type:luminance"><rect width="10" height="10" fill="black"/></mask></svg>"#.to_vec()});
        assert_eq!(
            svg_reference(&snapshot, "url(https://mask.test/m.svg#m)").unwrap(),
            Some((0, "m".into()))
        );
        assert!(svg_reference(&snapshot, "url(#m)").is_err());
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let luminance = svg_image(&snapshot, 0, "m", rect, "match-source", 1.).unwrap();
        assert!(luminance.pixels().all(|pixel| pixel.0[3] == 0));
        let alpha = svg_image(&snapshot, 0, "m", rect, "alpha", 1.).unwrap();
        assert_eq!(alpha.get_pixel(5, 5).0[3], 255);
    }
    #[test]
    fn native_mask_type_cascade_and_explicit_mode_preserve_alpha_luminance() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://mask.test/cascade.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><style>#m {mask-type:alpha !important}</style><mask id="m" style="mask-type:luminance"><rect width="10" height="10" fill="black"/></mask></svg>"#.to_vec()});
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        for mode in ["match-source", "alpha"] {
            assert_eq!(
                svg_image(&snapshot, 0, "m", rect, mode, 1.)
                    .unwrap()
                    .get_pixel(5, 5)
                    .0[3],
                255
            );
        }
        assert!(
            svg_image(&snapshot, 0, "m", rect, "luminance", 1.)
                .unwrap()
                .pixels()
                .all(|pixel| pixel.0[3] == 0)
        );
        assert!(svg_image(&snapshot, 0, "m", rect, "invalid", 1.).is_err());
    }
    #[test]
    fn no_clip_uses_finite_native_viewport_without_changing_mask_positioning_box() {
        let snapshot = crate::tests::empty_snapshot();
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 4.,
                height: 4.,
            },
        );
        node.style
            .insert("mask-image".into(), "linear-gradient(white, white)".into());
        node.style.insert("mask-clip".into(), "no-clip".into());
        node.style.insert("mask-repeat".into(), "repeat".into());
        assert!(!layers(&snapshot, &node).unwrap().is_empty());
    }
}

pub(super) fn same_document_url(source: &str, document: &str) -> bool {
    let (Ok(mut source), Ok(mut document)) = (url::Url::parse(source), url::Url::parse(document))
    else {
        return false;
    };
    source.set_fragment(None);
    document.set_fragment(None);
    source == document
}
