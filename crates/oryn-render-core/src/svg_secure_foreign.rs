//! Native formatting for HTML inside independently fetched secure SVG.
//! Builds a private layout input; no embedding DOM, selectors, fonts, or IO.
use super::*;
use oryn_layout::{LayoutAttribute, LayoutContent, LayoutInput, LayoutNode};
use std::collections::BTreeMap;
// usvg consumes SVG presentation values, not an entire HTML computed-style
// declaration block. Its simplecss tokenizer stops at an unsupported token
// (for example the slash in aspect-ratio or a custom property's token stream),
// which used to discard later fill/mask/text values. Native Stylo has already
// resolved variables, selectors, importance and media: transport only the
// presentation properties that the SVG consumer accepts, without re-cascading.
// Stylo serializes computed resource URLs as absolute URLs. usvg's native
// IRI grammar resolves only local fragments. Restore that representation only
// when URL identity (including origin, path and query) matches this SVG input.
// External references retain their exact spelling and secure IO policy.
pub fn local_style_urls(value: &str, document_url: &str) -> String {
    fn fragment(value: &str, document_url: &str) -> Option<String> {
        let mut document = url::Url::parse(document_url).ok()?;
        let mut reference = document.join(value).ok()?;
        let encoded = reference.fragment()?.to_owned();
        document.set_fragment(None);
        reference.set_fragment(None);
        if document != reference || encoded.is_empty() {
            return None;
        }
        let bytes = encoded.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%'
                && index + 2 < bytes.len()
                && let (Some(a), Some(b)) = (
                    char::from(bytes[index + 1]).to_digit(16),
                    char::from(bytes[index + 2]).to_digit(16),
                )
            {
                decoded.push((a * 16 + b) as u8);
                index += 3;
                continue;
            }
            decoded.push(bytes[index]);
            index += 1;
        }
        Some(format!("#{}", String::from_utf8(decoded).ok()?))
    }
    fn collect(
        parser: &mut cssparser::Parser<'_, '_>,
        document_url: &str,
        replacements: &mut Vec<(std::ops::Range<usize>, String)>,
    ) -> Result<(), ()> {
        use cssparser::Token;
        while !parser.is_exhausted() {
            let start = parser.position().byte_index();
            let token = parser
                .next_including_whitespace_and_comments()
                .map_err(|_| ())?
                .clone();
            let href = match token {
                Token::UnquotedUrl(value) => Some(value.to_string()),
                Token::Function(name) if name.eq_ignore_ascii_case("url") => Some(
                    parser
                        .parse_nested_block::<_, _, ()>(|nested| {
                            let value = nested.expect_string()?.to_string();
                            nested.expect_exhausted()?;
                            Ok(value)
                        })
                        .map_err(|_| ())?,
                ),
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                    parser
                        .parse_nested_block::<_, _, ()>(|nested| {
                            collect(nested, document_url, replacements)
                                .map_err(|_| nested.new_custom_error(()))
                        })
                        .map_err(|_| ())?;
                    None
                }
                _ => None,
            };
            if let Some(local) = href.and_then(|href| fragment(&href, document_url)) {
                let mut replacement = String::from("url(");
                cssparser::serialize_string(&local, &mut replacement).map_err(|_| ())?;
                replacement.push(')');
                replacements.push((start..parser.position().byte_index(), replacement));
            }
        }
        Ok(())
    }
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut replacements = Vec::new();
    if collect(&mut parser, document_url, &mut replacements).is_err() {
        return value.to_owned();
    }
    let mut output = value.to_owned();
    for (range, replacement) in replacements.into_iter().rev() {
        output.replace_range(range, &replacement);
    }
    output
}

pub(super) fn resolved_style(
    style: &BTreeMap<String, String>,
    svg: bool,
    document_url: &str,
) -> String {
    style
        .iter()
        .filter(|(name, value)| {
            !value.is_empty()
                && if svg {
                    matches!(
                        name.as_str(),
                        "alignment-baseline"
                            | "baseline-shift"
                            | "background-color"
                            | "clip-path"
                            | "clip-rule"
                            | "color"
                            | "color-interpolation"
                            | "color-interpolation-filters"
                            | "color-rendering"
                            | "direction"
                            | "display"
                            | "dominant-baseline"
                            | "fill"
                            | "fill-opacity"
                            | "fill-rule"
                            | "filter"
                            | "flood-color"
                            | "flood-opacity"
                            | "font-feature-settings"
                            | "font-variation-settings"
                            | "font-language-override"
                            | "color-scheme"
                            | "font-family"
                            | "font-kerning"
                            | "font-size"
                            | "font-size-adjust"
                            | "font-stretch"
                            | "font-style"
                            | "font-variant"
                            | "font-weight"
                            | "glyph-orientation-horizontal"
                            | "glyph-orientation-vertical"
                            | "image-rendering"
                            | "isolation"
                            | "letter-spacing"
                            | "lighting-color"
                            | "marker-end"
                            | "marker-mid"
                            | "marker-start"
                            | "mask"
                            | "mask-type"
                            | "mix-blend-mode"
                            | "opacity"
                            | "overflow"
                            | "paint-order"
                            | "shape-rendering"
                            | "stop-color"
                            | "stop-opacity"
                            | "stroke"
                            | "stroke-dasharray"
                            | "stroke-dashoffset"
                            | "stroke-linecap"
                            | "stroke-linejoin"
                            | "stroke-miterlimit"
                            | "stroke-opacity"
                            | "stroke-width"
                            | "text-anchor"
                            | "text-decoration"
                            | "text-overflow"
                            | "text-rendering"
                            | "transform"
                            | "transform-origin"
                            | "unicode-bidi"
                            | "vector-effect"
                            | "visibility"
                            | "white-space"
                            | "word-spacing"
                            | "writing-mode"
                    )
                } else {
                    !name.starts_with("oryn-")
                }
        })
        .map(|(name, value)| {
            let value = if svg {
                local_style_urls(value, document_url)
            } else {
                value.clone()
            };
            format!("{name}:{value};")
        })
        .collect()
}
pub(super) struct Prepared {
    pub source: oryn_common::v4::PaintImageResource,
    pub snapshot: PaintSnapshot,
    pub data: Vec<u8>,
    pub instance_styles: resvg::usvg::NativeInstanceStyles,
    pub instance_content: resvg::usvg::NativeInstanceContent,
    #[cfg(test)]
    pub instance_computed: BTreeMap<resvg::usvg::NativeInstanceKey, oryn_layout::ComputedStyle>,
    #[cfg(test)]
    pub instance_pseudos:
        BTreeMap<resvg::usvg::NativeInstanceKey, BTreeMap<String, oryn_layout::ComputedStyle>>,
}
pub(super) fn compose_with_instance_styles(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    data: Vec<u8>,
) -> Result<
    (
        Vec<u8>,
        resvg::usvg::NativeInstanceStyles,
        resvg::usvg::NativeInstanceContent,
        std::collections::BTreeSet<String>,
    ),
    RenderError,
> {
    let owned = prepare(source, snapshot, data)?;
    let composed =
        crate::svg_foreign::compose_snapshot(&owned.source, &owned.snapshot, owned.data)?;
    Ok((
        composed.data,
        owned.instance_styles,
        owned.instance_content,
        composed.style_roots,
    ))
}
pub(super) fn prepare(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    data: Vec<u8>,
) -> Result<Prepared, RenderError> {
    let fail = |detail: String| RenderError::InvalidPrint(format!("secure SVG HTML: {detail}"));
    let text = std::str::from_utf8(&data).map_err(|error| fail(error.to_string()))?;
    let xml = roxmltree::Document::parse(text).map_err(|error| fail(error.to_string()))?;
    let elements = xml
        .descendants()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let ids = elements
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id().get(), index as u64 + 1))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut sheets: Vec<String> = Vec::new();
    for node in &elements {
        let mut attributes = BTreeMap::new();
        let mut namespaced = Vec::new();
        for attribute in node.attributes() {
            if let Some(namespace) = attribute.namespace() {
                namespaced.push(LayoutAttribute {
                    namespace: namespace.into(),
                    local_name: attribute.name().into(),
                    prefix: None,
                    value: attribute.value().into(),
                });
            } else {
                attributes.insert(attribute.name().to_owned(), attribute.value().to_owned());
            }
        }
        // Capture identities supplied by source markup never confer native authority.
        attributes.remove("data-oryn-foreign-object");
        let content = node
            .children()
            .enumerate()
            .filter_map(|(offset, child)| {
                if child.is_text() {
                    Some(LayoutContent::Text {
                        node_id: 1_000_000 + ids[&node.id().get()] * 65536 + offset as u64,
                        text: child.text().unwrap_or("").into(),
                    })
                } else {
                    ids.get(&child.id().get())
                        .copied()
                        .map(LayoutContent::Element)
                }
            })
            .collect::<Vec<_>>();
        if node.tag_name().name() == "style"
            && matches!(
                node.tag_name().namespace(),
                Some("http://www.w3.org/2000/svg" | "http://www.w3.org/1999/xhtml")
            )
        {
            sheets.push(node.children().filter_map(|child| child.text()).collect());
        }
        nodes.push(serde_json::from_value::<LayoutNode>(serde_json::json!({"id":ids[&node.id().get()],"parent":node.parent().and_then(|parent|ids.get(&parent.id().get()).copied()),"tree_scope":1,"document_url":source.url,"namespace":node.tag_name().namespace().unwrap_or(""),"tag":node.tag_name().name(),"attributes":attributes,"namespaced_attributes":namespaced,"text":"","content":content,"children":node.children().filter_map(|child|ids.get(&child.id().get()).copied()).collect::<Vec<_>>()})).map_err(|error|fail(error.to_string()))?);
    }
    let environment = SvgEnvironment::current(snapshot);
    let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":sheets,"viewport":snapshot.viewport,"scroll":oryn_common::v4::ScrollState::default(),"document_base_url":source.url})).map_err(|error|fail(error.to_string()))?;
    input.viewport = environment.viewport();
    input.color_scheme = if environment.dark {
        oryn_layout::ColorScheme::Dark
    } else {
        oryn_layout::ColorScheme::Light
    };
    input.reduced_motion = environment.reduced_motion;
    let mut coordinates = input.viewport;
    let root = xml.root_element();
    let view_box = root
        .attribute("viewBox")
        .map(|value| {
            value
                .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
                .filter(|value| !value.is_empty())
                .filter_map(|value| value.parse::<f64>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|values| {
            values.len() == 4
                && values.iter().all(|value| value.is_finite())
                && values[2] > 0.
                && values[3] > 0.
        });
    if let Some(view_box) = view_box {
        coordinates.width = view_box[2];
        coordinates.height = view_box[3];
    } else {
        for (name, destination) in [
            ("width", &mut coordinates.width),
            ("height", &mut coordinates.height),
        ] {
            if let Some(value) = root
                .attribute(name)
                .and_then(|value| value.trim().trim_end_matches("px").parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.)
            {
                *destination = value;
            }
        }
    }
    input.web_fonts = oryn_layout::svg_fonts::embedded(
        &input.stylesheets,
        &source.url,
        input.viewport,
        input.color_scheme,
        input.reduced_motion,
    )
    .map_err(fail)?;
    let limits = oryn_layout::LayoutLimits::default();
    let whole = oryn_layout::compute(&input, &limits).map_err(|error| fail(error.to_string()))?;
    let instance_styles = crate::svg_use_styles::compute(&xml, &ids, &input, &limits)?;
    let mut local = snapshot.clone();
    local.images = vec![source.clone()];
    local.fonts.clear();
    local.frames.clear();
    local.svg_foreign_objects.clear();
    local.nodes.clear();
    local.diagnostics.clear();
    let mut image_indices = BTreeMap::new();
    let mut resource_bytes = 0usize;
    for url in oryn_layout::image_requests(&whole).into_iter().chain(
        input
            .nodes
            .iter()
            .filter(|node| node.tag == "img")
            .filter_map(|node| node.attributes.get("src").cloned()),
    ) {
        if image_indices.contains_key(&url) {
            continue;
        }
        let Some(bytes) = oryn_layout::svg_fonts::data_bytes(&url) else {
            continue;
        };
        resource_bytes = resource_bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= 32 * 1024 * 1024)
            .ok_or_else(|| fail("embedded image byte budget".into()))?;
        if image_indices.len() >= 256 {
            return Err(fail("embedded image count budget".into()));
        }
        let media = url
            .strip_prefix("data:")
            .and_then(|tail| tail.split_once(','))
            .map(|(metadata, _)| {
                metadata
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        let content_type = url
            .strip_prefix("data:")
            .and_then(|tail| tail.split_once(','))
            .map(|(metadata, _)| metadata.to_string());
        let index = local.images.len();
        local.images.push(oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type,
            url: url.clone(),
            url_aliases: Vec::new(),
            media_type: media,
            bytes,
        });
        image_indices.insert(url, index);
    }
    let mut replacements = Vec::new();
    // Resolve SVG's independent cascade with the actual image media viewport.
    // Inline the used values and remove stylesheet text so usvg cannot evaluate
    // the same rules a second time in a different media environment.
    for node in &elements {
        if node.tag_name().name() == "style" {
            for child in node.children().filter(|child| child.is_text()) {
                replacements.push((child.range(), String::new()));
            }
            continue;
        }
        if node.tag_name().name() == "foreignObject" {
            continue;
        }
        let Some(style) = whole.computed_styles.get(&ids[&node.id().get()]) else {
            continue;
        };
        let start = node.range().start;
        let end =
            opening_end(text, start).ok_or_else(|| fail("element start tag missing".into()))?;
        let opening = &text[start..=end];
        let mut rewritten = opening.to_string();
        let mut removed = node
            .attributes()
            .filter(|attribute| {
                attribute.namespace().is_none()
                    && matches!(attribute.name(), "style" | "data-oryn-native-style-id")
            })
            .map(|attribute| attribute.range())
            .collect::<Vec<_>>();
        removed.sort_by_key(|range| range.start);
        for range in removed.into_iter().rev() {
            rewritten.replace_range(range.start - start..range.end - start, "");
        }
        let css = resolved_style(
            style,
            node.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
            &source.url,
        )
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
        let insertion = rewritten
            .rfind(if rewritten.trim_end().ends_with("/>") {
                '/'
            } else {
                '>'
            })
            .unwrap();
        rewritten.insert_str(
            insertion,
            &format!(
                " style=\"{css}\" data-oryn-native-style-id=\"{}\"",
                ids[&node.id().get()]
            ),
        );
        replacements.push((start..end + 1, rewritten));
    }

    let mut instance_content = resvg::usvg::NativeInstanceContent::new();
    let mut instance_markup = String::new();
    let mut native_prefix = "oryn-native-instance-".to_owned();
    // Generated vector IDs must not turn any formerly unresolved authored
    // reference into a live reference. Check decoded attributes and computed
    // CSS URLs as well as literal source text.
    while text.contains(&native_prefix)
        || xml
            .descendants()
            .flat_map(|node| node.attributes())
            .any(|attribute| attribute.value().contains(&native_prefix))
        || whole
            .computed_styles
            .values()
            .flat_map(|style| style.values())
            .any(|value| value.contains(&native_prefix))
        || whole
            .pseudo_styles
            .values()
            .flat_map(|styles| styles.values())
            .flat_map(|style| style.values())
            .any(|value| value.contains(&native_prefix))
        || instance_styles
            .computed
            .values()
            .flat_map(|style| style.values())
            .any(|value| value.contains(&native_prefix))
        || instance_styles
            .pseudos
            .values()
            .flat_map(|styles| styles.values())
            .flat_map(|style| style.values())
            .any(|value| value.contains(&native_prefix))
    {
        native_prefix.push('_');
    }
    for node in elements.into_iter().filter(|node| {
        node.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
            && node.tag_name().name() == "foreignObject"
            && !node.ancestors().skip(1).any(|ancestor| {
                ancestor.is_element()
                    && ancestor.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                    && ancestor.tag_name().name() == "foreignObject"
            })
    }) {
        let id = ids[&node.id().get()];
        let variants = std::iter::once(None)
            .chain(
                instance_styles
                    .computed
                    .keys()
                    .filter(|key| key.source == id)
                    .cloned()
                    .map(Some),
            )
            .collect::<Vec<_>>();
        for (variant_index, variant) in variants.into_iter().enumerate() {
            let mut instance_primary;
            let mut instance_pseudos;
            let (primary, pseudos) = if let Some(key) = &variant {
                instance_primary = whole.computed_styles.clone();
                instance_pseudos = whole.pseudo_styles.clone();
                for (candidate, style) in &instance_styles.computed {
                    if candidate.ancestry == key.ancestry {
                        instance_primary.insert(candidate.source, style.clone());
                        // Absence is meaningful: never retain the original source
                        // pseudo when this instance does not generate it.
                        instance_pseudos.remove(&candidate.source);
                        if let Some(styles) = instance_styles.pseudos.get(candidate) {
                            instance_pseudos.insert(candidate.source, styles.clone());
                        }
                    }
                }
                (&instance_primary, &instance_pseudos)
            } else {
                (&whole.computed_styles, &whole.pseudo_styles)
            };
            let style = primary.get(&id);
            let coordinate = |name: &str, basis: f64| -> Result<f64, RenderError> {
                let value = style
                    .and_then(|style| style.get(name))
                    .map(String::as_str)
                    .filter(|value| *value != "auto")
                    .or_else(|| node.attribute(name))
                    .unwrap_or("0");
                let value = value.trim();
                let number = if let Some(percent) = value.strip_suffix('%') {
                    percent
                        .parse::<f64>()
                        .ok()
                        .map(|number| number * basis / 100.)
                } else {
                    value.trim_end_matches("px").parse::<f64>().ok()
                };
                number
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| fail(format!("unresolved foreignObject {name}")))
            };
            let x = coordinate("x", coordinates.width)?;
            let y = coordinate("y", coordinates.height)?;
            let width = coordinate("width", coordinates.width)?;
            let height = coordinate("height", coordinates.height)?;
            if width <= 0. || height <= 0. {
                if let Some(key) = variant {
                    instance_content.insert(key, None);
                } else {
                    // Retain a source identity even when the original viewport is
                    // empty: a use instance may resolve relative dimensions anew.
                    let mut empty = format!("<g data-oryn-native-style-id=\"{id}\"");
                    if let Some(name) = node.attribute("id") {
                        let name = name
                            .replace('&', "&amp;")
                            .replace('\"', "&quot;")
                            .replace('<', "&lt;");
                        empty.push_str(&format!(" id=\"{name}\""));
                    }
                    empty.push_str("/>");
                    let removed = node.range();
                    replacements.retain(|(range, _)| {
                        range.start < removed.start || range.end > removed.end
                    });
                    replacements.push((removed, empty));
                }
                continue;
            }
            let viewport = oryn_common::v4::ViewportState {
                width,
                height,
                device_pixel_ratio: snapshot.viewport.device_pixel_ratio,
            };
            if local.svg_foreign_objects.len() >= 256 {
                return Err(fail("foreignObject instance capture count budget".into()));
            }
            let output = oryn_layout::compute_subtree_with_styles_and_pseudos(
                &input, id, viewport, primary, pseudos, &limits,
            )
            .map_err(|error| fail(error.to_string()))?;
            local.fonts.extend(output.paint_fonts.clone());
            let mut nested_images = BTreeMap::new();
            let mut nested_descendants = std::collections::BTreeSet::new();
            for nested in node.descendants().skip(1).filter(|child| {
                child.is_element()
                    && child.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                    && child.tag_name().name() == "svg"
            }) {
                if nested
                    .ancestors()
                    .skip(1)
                    .take_while(|ancestor| *ancestor != node)
                    .any(|ancestor| {
                        ancestor.is_element()
                            && ancestor.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                            && ancestor.tag_name().name() == "svg"
                    })
                {
                    continue;
                }
                let markup =
                    serialize_svg(nested, &ids, primary, &input.stylesheets, 0, &source.url)?;
                resource_bytes = resource_bytes
                    .checked_add(markup.len())
                    .filter(|bytes| *bytes <= 32 * 1024 * 1024)
                    .ok_or_else(|| fail("nested SVG aggregate byte budget".into()))?;
                if local.images.len() >= 257 {
                    return Err(fail("nested SVG resource count budget".into()));
                }
                let index = local.images.len();
                local.images.push(oryn_common::v4::PaintImageResource {
                    svg_document_snapshot: false,
                    svg_instances: Vec::new(),
                    content_type: None,
                    url: source.url.clone(),
                    url_aliases: Vec::new(),
                    media_type: "image/svg+xml".into(),
                    bytes: markup.into_bytes(),
                });
                nested_images.insert(ids[&nested.id().get()], index);
                nested_descendants.extend(
                    nested
                        .descendants()
                        .skip(1)
                        .filter_map(|child| ids.get(&child.id().get()).copied()),
                );
            }
            let mut captured = Vec::new();
            for source_node in &input.nodes {
                if nested_descendants.contains(&source_node.id) {
                    continue;
                }
                let Some(geometry) = output.geometry.get(&source_node.id) else {
                    continue;
                };
                let Some(style) = output.computed_styles.get(&source_node.id) else {
                    continue;
                };
                captured.push(oryn_common::v4::PaintNodeSnapshot {
                    builtin_image: None,
                    backdrop: output.backdrops.get(&source_node.id).cloned().map(Box::new),
                    control: output.control_states.get(&source_node.id).cloned(),
                    backdrop_style: None,
                    top_layer: None,
                    node_id: source_node.id,
                    parent_id: if source_node.id == id {
                        None
                    } else {
                        output
                            .paint_parent_overrides
                            .get(&source_node.id)
                            .copied()
                            .unwrap_or(source_node.parent)
                    },
                    tag: if source_node.id == id {
                        "div".into()
                    } else {
                        source_node.tag.clone()
                    },
                    geometry: geometry.clone(),
                    projection: output.paint_projections.get(&source_node.id).cloned(),
                    style: {
                        let mut style = (**style).clone();
                        for (name, value) in &mut style {
                            if matches!(
                                name.as_str(),
                                "background-image"
                                    | "border-image-source"
                                    | "mask-image"
                                    | "list-style-image"
                            ) && let Some(layers) = crate::gradients::split(value, ',')
                            {
                                *value = layers
                                    .into_iter()
                                    .map(|layer| {
                                        let selected = crate::backgrounds::selected_image(
                                            layer,
                                            snapshot.viewport.device_pixel_ratio,
                                        )
                                        .map(|(image, _)| image)
                                        .unwrap_or_else(|| layer.to_owned());
                                        if oryn_layout::cssom::resource_urls(&selected)
                                            .iter()
                                            .any(|url| !url.starts_with("data:"))
                                        {
                                            if name == "mask-image" {
                                                "linear-gradient(transparent, transparent)"
                                                    .to_owned()
                                            } else {
                                                "none".to_owned()
                                            }
                                        } else {
                                            layer.to_owned()
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                            }
                        }
                        style
                    },
                    text: if nested_images.contains_key(&source_node.id) {
                        Vec::new()
                    } else {
                        output
                            .text_fragments
                            .get(&source_node.id)
                            .cloned()
                            .unwrap_or_default()
                    },
                    image_resource: nested_images.get(&source_node.id).copied().or_else(|| {
                        source_node
                            .attributes
                            .get("src")
                            .and_then(|url| image_indices.get(url))
                            .copied()
                    }),
                });
            }
            captured.extend(output.generated_nodes);
            let identity = format!("secure-{id}-{variant_index}");
            local
                .svg_foreign_objects
                .push(oryn_common::v4::SvgForeignObjectSnapshot {
                    image_resource: 0,
                    element_id: identity.clone(),
                    viewport,
                    nodes: captured,
                });
            if let Some(key) = variant {
                let native_id = format!("{native_prefix}{id}-{variant_index}");
                instance_markup.push_str(&format!("<foreignObject id=\"{native_id}\" data-oryn-foreign-object=\"{identity}\" x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\"/>"));
                instance_content.insert(key, Some(native_id));
                continue;
            }
            let opening = opening_end(text, node.range().start)
                .ok_or_else(|| fail("foreignObject start tag missing".into()))?;
            let mut replacement = String::from("<foreignObject");
            for attribute in node.attributes() {
                if attribute.namespace().is_none()
                    && !matches!(
                        attribute.name(),
                        "data-oryn-foreign-object"
                            | "data-oryn-native-style-id"
                            | "style"
                            | "x"
                            | "y"
                            | "width"
                            | "height"
                    )
                {
                    let value = attribute
                        .value()
                        .replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('<', "&lt;");
                    replacement.push_str(&format!(" {}=\"{value}\"", attribute.name()));
                }
            }
            if let Some(style) = style {
                let css = resolved_style(
                    style,
                    node.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
                    &source.url,
                )
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;");
                replacement.push_str(&format!(" style=\"{css}\""));
            }
            replacement.push_str(&format!(" data-oryn-native-style-id=\"{id}\" data-oryn-foreign-object=\"{identity}\" x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\">"));
            if text[node.range().start..opening].trim_end().ends_with('/') {
                replacement.pop();
                replacement.push_str("/>");
            }
            replacements.push((node.range().start..opening + 1, replacement));
        }
    }
    if !instance_markup.is_empty() {
        let end = text[..xml.root_element().range().end]
            .rfind("</")
            .ok_or_else(|| fail("SVG root closing tag missing".into()))?;
        replacements.push((end..end, format!("<defs>{instance_markup}</defs>")));
    }
    if replacements
        .iter()
        .try_fold(text.len(), |size, (_, value)| size.checked_add(value.len()))
        .is_none_or(|size| size > 48 * 1024 * 1024)
    {
        return Err(fail("resolved SVG cascade byte budget".into()));
    }
    let mut canonical = text.to_string();
    replacements.sort_by_key(|(range, _)| range.start);
    for (range, replacement) in replacements.into_iter().rev() {
        canonical.replace_range(range, &replacement)
    }
    let mut owned = source.clone();
    owned.svg_document_snapshot = true;
    owned.content_type = None;
    owned.bytes = canonical.as_bytes().to_vec();
    local.images[0] = owned.clone();
    Ok(Prepared {
        source: owned,
        snapshot: local,
        data: canonical.into_bytes(),
        instance_styles: instance_styles.presentation,
        instance_content,
        #[cfg(test)]
        instance_computed: instance_styles.computed,
        #[cfg(test)]
        instance_pseudos: instance_styles.pseudos,
    })
}

fn serialize_svg(
    node: roxmltree::Node<'_, '_>,
    ids: &BTreeMap<u32, u64>,
    styles: &oryn_layout::ComputedStyles,
    sheets: &[String],
    depth: usize,
    document_url: &str,
) -> Result<String, RenderError> {
    if depth > 128 {
        return Err(RenderError::InvalidPrint(
            "secure nested SVG depth budget".into(),
        ));
    }
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
    };
    if node.is_text() {
        return Ok(escape(node.text().unwrap_or("")));
    }
    if !node.is_element() {
        return Ok(String::new());
    }
    let mut output = format!(
        "<{} xmlns=\"{}\"",
        node.tag_name().name(),
        escape(node.tag_name().namespace().unwrap_or(""))
    );
    if depth == 0 {
        output.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
    }
    for attribute in node.attributes() {
        if matches!(attribute.name(), "style" | "data-oryn-foreign-object") {
            continue;
        }
        let name = match attribute.namespace() {
            None => attribute.name().to_string(),
            Some("http://www.w3.org/1999/xlink") => format!("xlink:{}", attribute.name()),
            Some("http://www.w3.org/XML/1998/namespace") => format!("xml:{}", attribute.name()),
            _ => continue,
        };
        output.push_str(&format!(" {name}=\"{}\"", escape(attribute.value())));
    }
    if let Some(style) = ids.get(&node.id().get()).and_then(|id| styles.get(id)) {
        let css = resolved_style(
            style,
            node.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
            document_url,
        );
        output.push_str(&format!(" style=\"{}\"", escape(&css)));
    }
    output.push('>');
    if depth == 0 {
        for sheet in sheets {
            output.push_str("<style>");
            output.push_str(&escape(sheet));
            output.push_str("</style>");
        }
    }
    for child in node.children() {
        output.push_str(&serialize_svg(
            child,
            ids,
            styles,
            sheets,
            depth + 1,
            document_url,
        )?);
        if output.len() > 16 * 1024 * 1024 {
            return Err(RenderError::InvalidPrint(
                "secure nested SVG byte budget".into(),
            ));
        }
    }
    output.push_str(&format!("</{}>", node.tag_name().name()));
    Ok(output)
}

fn opening_end(text: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in text.as_bytes().iter().copied().enumerate().skip(start) {
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'>' {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_filter_references_survive_native_cascade_and_function_chains() {
        let snapshot = crate::tests::empty_snapshot();
        for value in [r##"url("#alpha")"##, "url('#alpha')", "url(#alpha)"] {
            let bytes = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><filter id="alpha" filterUnits="userSpaceOnUse" x="0" y="0" width="20" height="10"><feComponentTransfer><feFuncA type="linear" slope=".5"/></feComponentTransfer></filter></defs><rect width="20" height="10" fill="red" style="filter:{} opacity(.5)"/></svg>"##,
                value.replace('"', "&quot;")
            );
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: "https://filters.test/a.svg".into(),
                media_type: "image/svg+xml".into(),
                bytes: bytes.into_bytes(),
            };
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            assert_eq!(tree.filters().len(), 2, "{value}");
            let pixels = crate::svg::raster(&tree, 20, 10).unwrap();
            assert_eq!(pixels.get_pixel(5, 5).0, [255, 0, 0, 64], "{value}");
        }
    }

    #[test]
    fn foreign_object_instances_retain_full_html_styles_and_paint_independently() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(),
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://glyphs.test/foreign-use.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="2"><defs><foreignObject id="content" width="2" height="2"><div xmlns="http://www.w3.org/1999/xhtml" style="width:2px;height:2px;background-color:currentColor"/></foreignObject></defs><use href="#content" color="red"/><use href="#content" x="4" color="blue"/></svg>"##.to_vec(),
        };
        let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        let xml = roxmltree::Document::parse(std::str::from_utf8(&source.bytes).unwrap()).unwrap();
        let elements = xml
            .descendants()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        let html_id = elements
            .iter()
            .position(|node| node.tag_name().name() == "div")
            .unwrap() as u64
            + 1;
        let uses = elements
            .iter()
            .enumerate()
            .filter(|(_, node)| node.tag_name().name() == "use")
            .map(|(index, _)| index as u64 + 1)
            .collect::<Vec<_>>();
        for (source, expected) in uses.into_iter().zip(["rgb(255, 0, 0)", "rgb(0, 0, 255)"]) {
            let key = resvg::usvg::NativeInstanceKey {
                ancestry: vec![source],
                source: html_id,
            };
            let style = &prepared.instance_computed[&key];
            assert_eq!(style["color"], expected);
            assert!(
                style.contains_key("background-color"),
                "HTML paint values must survive projection"
            );
        }
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        let pixels = crate::svg::raster(&tree, 6, 2).unwrap();
        assert_eq!(pixels.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(5, 1).0, [0, 0, 255, 255]);
        assert_eq!(pixels.get_pixel(3, 1).0, [0; 4]);
    }

    #[test]
    fn foreign_object_instances_preserve_empty_source_identity_and_do_not_alias_authored_links() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(),
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://glyphs.test/foreign-empty-use.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="2"><defs><foreignObject id="content" style="width:var(--width,0px);height:2px"><div xmlns="http://www.w3.org/1999/xhtml" style="width:2px;height:2px;background-color:currentColor"/></foreignObject></defs><use href="#content" color="red" style="--width:2px"/><use href="#content" x="4" color="blue" style="--width:0px"/><use href="#oryn-native-instance-3-1" x="2"/></svg>"##.to_vec(),
        };
        let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        assert_eq!(prepared.snapshot.svg_foreign_objects.len(), 1);
        assert_eq!(
            prepared
                .instance_content
                .values()
                .filter(|value| value.is_some())
                .count(),
            1
        );
        assert_eq!(
            prepared
                .instance_content
                .values()
                .filter(|value| value.is_none())
                .count(),
            1
        );
        assert!(
            prepared
                .instance_content
                .values()
                .flatten()
                .all(|id| id != "oryn-native-instance-3-1")
        );
        let xml = roxmltree::Document::parse(std::str::from_utf8(&prepared.data).unwrap()).unwrap();
        assert!(
            xml.descendants()
                .any(|node| node.attribute("id") == Some("content"))
        );
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        let pixels = crate::svg::raster(&tree, 6, 2).unwrap();
        assert_eq!(pixels.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(
            pixels.get_pixel(3, 1).0,
            [0; 4],
            "unresolved authored reference must remain unresolved"
        );
        assert_eq!(
            pixels.get_pixel(5, 1).0,
            [0; 4],
            "empty instance must not reuse another capture"
        );
    }

    #[test]
    fn foreign_object_use_generated_content_keeps_instance_pseudo_styles() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(),
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://glyphs.test/foreign-pseudo-use.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="2"><style>#html::before{content:'';display:block;width:2px;height:2px;background-color:currentColor}</style><defs><foreignObject id="content" width="2" height="2"><div id="html" xmlns="http://www.w3.org/1999/xhtml"/></foreignObject></defs><use href="#content" color="red"/><use href="#content" x="4" color="blue"/></svg>"##.to_vec(),
        };
        let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        let colors = prepared
            .instance_pseudos
            .values()
            .filter_map(|styles| styles.get("before"))
            .map(|style| style["color"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(colors.contains("rgb(255, 0, 0)"));
        assert!(colors.contains("rgb(0, 0, 255)"));
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        let pixels = crate::svg::raster(&tree, 6, 2).unwrap();
        assert_eq!(pixels.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(5, 1).0, [0, 0, 255, 255]);
        assert_eq!(pixels.get_pixel(3, 1).0, [0; 4]);
    }

    #[test]
    fn nested_use_instances_preserve_native_variables_and_original_selector_matches() {
        let snapshot = crate::tests::empty_snapshot();
        for (sheet, colors) in [
            ("", [[255, 0, 0, 255], [0, 0, 255, 255]]),
            (
                "defs > #shape > rect {fill:lime!important}",
                [[0, 255, 0, 255]; 2],
            ),
        ] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false, svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: "https://glyphs.test/nested-use.svg".into(),
                media_type: "image/svg+xml".into(),
                bytes: format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="2" data-oryn-native-style-id="900"><style>{sheet}</style><defs><g id="shape" data-oryn-native-style-id="900"><rect width="2" height="2" style="fill:var(--tone)"/></g><use id="nested" href="#shape"/></defs><use href="#nested" style="--tone:red"/><use href="#nested" x="4" style="--tone:blue"/></svg>"##).into_bytes(),
            };
            let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
            let xml =
                roxmltree::Document::parse(std::str::from_utf8(&prepared.data).unwrap()).unwrap();
            let mut identities = std::collections::BTreeSet::new();
            for node in xml
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() != "style")
            {
                let id = node.attribute("data-oryn-native-style-id").unwrap();
                assert_ne!(id, "900", "authored marker must be replaced");
                assert!(
                    identities.insert(id),
                    "native source identities must be unique"
                );
            }
            assert!(
                prepared
                    .instance_styles
                    .keys()
                    .any(|key| key.ancestry.len() == 2)
            );
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            let pixels = crate::svg::raster(&tree, 6, 2).unwrap();
            assert_eq!(pixels.get_pixel(1, 1).0, colors[0], "first: {sheet}");
            assert_eq!(pixels.get_pixel(5, 1).0, colors[1], "second: {sheet}");
            assert_eq!(pixels.get_pixel(3, 1).0, [0; 4]);
        }
    }

    #[test]
    fn component_transfer_rounds_channels_and_clamps_each_function_kind() {
        let snapshot = crate::tests::empty_snapshot();
        for (functions, expected) in [
            (r#"<feFuncA type="linear" slope=".5"/>"#, [255, 0, 0, 128]),
            (
                r#"<feFuncA type="table" tableValues="0 .5"/>"#,
                [255, 0, 0, 128],
            ),
            (
                r#"<feFuncA type="discrete" tableValues=".25 .5"/>"#,
                [255, 0, 0, 128],
            ),
            (
                r#"<feFuncA type="gamma" amplitude=".5" exponent="2" offset="0"/>"#,
                [255, 0, 0, 128],
            ),
            (r#"<feFuncR type="linear" slope=".5"/>"#, [128, 0, 0, 255]),
            (r#"<feFuncA type="linear" slope="-1"/>"#, [0, 0, 0, 0]),
            (r#"<feFuncA type="linear" slope="2"/>"#, [255, 0, 0, 255]),
            (r#"<feFuncA type="identity"/>"#, [255, 0, 0, 255]),
        ] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false, svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: "https://filters.test/transfer.svg".into(),
                media_type: "image/svg+xml".into(),
                bytes: format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><defs><filter id="transfer" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" x="0" y="0" width="4" height="4"><feComponentTransfer>{functions}</feComponentTransfer></filter></defs><rect width="4" height="4" fill="red" filter="url(#transfer)"/></svg>"##).into_bytes(),
            };
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            let pixels = crate::svg::raster(&tree, 4, 4).unwrap();
            assert_eq!(pixels.get_pixel(2, 2).0, expected, "{functions}");
        }
    }

    #[test]
    fn filter_image_preserves_rotation_and_skew_of_asymmetric_content() {
        let snapshot = crate::tests::empty_snapshot();
        let image = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="2" height="4" fill="red"/></svg>"#,
        );
        for (transform, opaque, transparent) in [
            ("translate(4 0) rotate(90)", (3, 1), (1, 3)),
            ("matrix(1 0 1 1 0 0)", (4, 3), (1, 3)),
        ] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false, svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: "https://filters.test/image-transform.svg".into(),
                media_type: "image/svg+xml".into(),
                bytes: format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><defs><filter id="image" filterUnits="userSpaceOnUse" x="0" y="0" width="4" height="4"><feImage href="data:image/svg+xml;base64,{image}"/></filter></defs><g transform="{transform}"><rect width="4" height="4" filter="url(#image)"/></g></svg>"##).into_bytes(),
            };
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            let pixels = crate::svg::raster(&tree, 8, 4).unwrap();
            assert_eq!(
                pixels.get_pixel(opaque.0, opaque.1).0,
                [255, 0, 0, 255],
                "{transform}"
            );
            assert_eq!(
                pixels.get_pixel(transparent.0, transparent.1).0,
                [0; 4],
                "{transform}"
            );
        }
    }

    #[test]
    fn clip_path_none_is_identity_without_admitting_invalid_matrices() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot:false, svg_instances: Vec::new(), content_type:None, url_aliases:Vec::new(),
            url:"https://clips.test/a.svg".into(), media_type:"image/svg+xml".into(),
            bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><clipPath id="clip" transform="translate(20 0)" style="transform:none"><rect width="10" height="10"/></clipPath></defs><rect width="20" height="10" fill="red" clip-path="url(#clip)"/></svg>"##.to_vec(),
        };
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert_eq!(tree.clip_paths().len(), 1);
        let pixels = crate::svg::raster(&tree, 20, 10).unwrap();
        assert_eq!(pixels.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(15, 5)[3], 0);
        // An actual malformed transform still follows usvg's invalid-clip path.
        let malformed = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><clipPath id="clip" transform="matrix(bad)"><rect width="10" height="10"/></clipPath></defs><rect width="20" height="10" fill="red" clip-path="url(#clip)"/></svg>"##;
        let rejected =
            resvg::usvg::Tree::from_str(malformed, &resvg::usvg::Options::default()).unwrap();
        assert!(
            crate::svg::raster(&rejected, 20, 10)
                .unwrap()
                .pixels()
                .all(|pixel| pixel[3] == 0)
        );
    }

    #[test]
    fn native_svg_transform_origin_preserves_glyph_flip_and_author_override() {
        let snapshot = crate::tests::empty_snapshot();
        for (style, y) in [("", 10), ("transform-origin:0 5px", 20)] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot:false, svg_instances: Vec::new(), content_type:None, url_aliases:Vec::new(),
                url:"https://glyphs.test/a.svg".into(), media_type:"image/svg+xml".into(),
                bytes:format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="30"><defs><path id="glyph" d="M0 0H4V4H0Z"/></defs><use href="#glyph" transform="translate(2 12) scale(1 -1)" fill="red" style="{style}"/></svg>"##).into_bytes(),
            };
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            let pixels = crate::svg::raster(&tree, 30, 30).unwrap();
            assert_eq!(pixels.get_pixel(3, y).0, [255, 0, 0, 255], "{style}");
            assert_eq!(pixels.get_pixel(3, 2)[3], 0);
            let reparsed = oryn_common::v4::PaintImageResource {
                bytes: tree
                    .to_string(&resvg::usvg::WriteOptions::default())
                    .into_bytes(),
                ..source
            };
            let roundtrip = crate::svg::parse(&reparsed, &snapshot).unwrap();
            assert_eq!(crate::svg::raster(&roundtrip, 30, 30).unwrap(), pixels);
        }
    }

    #[test]
    fn svg_use_instances_inherit_without_overriding_explicit_reference_styles() {
        let snapshot = crate::tests::empty_snapshot();
        for (attributes, sheet, expected) in [
            ("", "", [[255, 0, 0, 255], [0, 0, 255, 255]]),
            ("fill='black'", "", [[0, 0, 0, 255]; 2]),
            ("", "#shape {fill:lime!important}", [[0, 255, 0, 255]; 2]),
            (
                "style='fill:inherit'",
                "",
                [[255, 0, 0, 255], [0, 0, 255, 255]],
            ),
            (
                "fill='currentColor'",
                "",
                [[255, 0, 0, 255], [0, 0, 255, 255]],
            ),
        ] {
            let source = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false, svg_instances: Vec::new(),
                content_type: None,
                url_aliases: Vec::new(),
                url: "https://glyphs.test/use-inheritance.svg".into(),
                media_type: "image/svg+xml".into(),
                bytes: format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="2"><style>{sheet}</style><defs><path id="shape" d="M0 0H2V2H0Z" {attributes}/></defs><use href="#shape" fill="red" color="red"/><use href="#shape" x="4" fill="blue" color="blue"/></svg>"##).into_bytes(),
            };
            let tree = crate::svg::parse(&source, &snapshot).unwrap();
            let pixels = crate::svg::raster(&tree, 6, 2).unwrap();
            assert_eq!(
                pixels.get_pixel(1, 1).0,
                expected[0],
                "first: {attributes} {sheet}"
            );
            assert_eq!(
                pixels.get_pixel(5, 1).0,
                expected[1],
                "second: {attributes} {sheet}"
            );
            assert_eq!(pixels.get_pixel(3, 1).0, [0; 4]);
        }
    }

    #[test]
    fn svg_ua_origin_scope_keeps_html_viewports_and_author_importance() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot:false, svg_instances: Vec::new(), content_type:None, url_aliases:Vec::new(),
            url:"https://origins.test/a.svg".into(), media_type:"image/svg+xml".into(),
            bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" id="root" width="30" height="30"><style>#author {transform-origin:3px 4px!important}</style><g id="group"><svg id="nested" width="10" height="10"/><rect id="author" style="transform-origin:8px 9px" width="1" height="1"/></g><foreignObject width="10" height="10"><svg id="foreign-child" width="10" height="10"/><div xmlns="http://www.w3.org/1999/xhtml" id="html"/></foreignObject></svg>"##.to_vec(),
        };
        let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        let text = std::str::from_utf8(&prepared.data).unwrap();
        let xml = roxmltree::Document::parse(text).unwrap();
        // Computed TransformOrigin serializes horizontal, vertical and depth.
        // Check all three components, including the required zero depth.
        for (id, expected) in [
            ("root", "50% 50% 0px"),
            ("group", "0px 0px 0px"),
            ("nested", "0px 0px 0px"),
            ("foreign-child", "50% 50% 0px"),
            ("html", "50% 50% 0px"),
            ("author", "3px 4px 0px"),
        ] {
            let element = xml
                .descendants()
                .find(|node| node.attribute("id") == Some(id))
                .unwrap();
            assert!(
                element
                    .attribute("style")
                    .unwrap()
                    .contains(&format!("transform-origin:{expected};")),
                "{id}: {text}"
            );
        }
    }

    #[test]
    fn computed_fragment_transport_preserves_url_identity_and_css_tokens() {
        let document = "https://effects.test/art.svg?version=1";
        assert_eq!(
            local_style_urls(
                r##"url("https://effects.test/art.svg?version=1#cl%69p") blur(2px) url(#paint)"##,
                document
            ),
            r##"url("#clip") blur(2px) url("#paint")"##,
        );
        for value in [
            r##"url("https://effects.test/art.svg?version=2#clip")"##,
            r##"url("https://elsewhere.test/art.svg?version=1#clip")"##,
            r##"url("https://effects.test/other.svg?version=1#clip")"##,
            r##""url(#inside-string)" /* url(#inside-comment) */"##,
        ] {
            assert_eq!(local_style_urls(value, document), value);
        }
        assert_eq!(
            local_style_urls(
                r"u\72l(https://effects.test/art.svg?version=1#paint)",
                document
            ),
            r##"url("#paint")"##
        );
    }

    #[test]
    fn secure_cascade_retains_local_paint_clip_and_filter_references() {
        let snapshot = crate::tests::empty_snapshot();
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(), content_type: None, url_aliases: Vec::new(),
            url: "https://effects.test/native.svg?version=1".into(), media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><linearGradient id="paint"><stop stop-color="red"/><stop offset="1" stop-color="red"/></linearGradient><clipPath id="clip"><rect width="10" height="10"/></clipPath><filter id="alpha" x="0" y="0" width="20" height="10" filterUnits="userSpaceOnUse"><feComponentTransfer><feFuncA type="linear" slope=".5"/></feComponentTransfer></filter></defs><g clip-path="url(#clip)"><rect width="20" height="10" fill="url(#paint)" filter="url(#alpha)"/></g></svg>"##.to_vec(),
        };
        let prepared = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        let text = std::str::from_utf8(&prepared.data).unwrap();
        let xml = roxmltree::Document::parse(text).unwrap();
        for (tag, property, id) in [
            ("g", "clip-path", "clip"),
            ("rect", "fill", "paint"),
            ("rect", "filter", "alpha"),
        ] {
            assert!(
                xml.descendants()
                    .filter(|node| node.has_tag_name(tag))
                    .any(|node| {
                        node.attribute("style").is_some_and(|style| {
                            style.contains(&format!("{property}:url(\"#{id}\")"))
                        })
                    }),
                "{property}: {text}"
            );
        }
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert!(!tree.filters().is_empty());
        assert!(!tree.clip_paths().is_empty());
        assert!(!tree.linear_gradients().is_empty());
        let pixels = crate::svg::raster(&tree, 20, 10).unwrap();
        assert_eq!(pixels.get_pixel(5, 5).0, [255, 0, 0, 128]);
        assert_eq!(pixels.get_pixel(15, 5)[3], 0);
    }

    #[test]
    fn resolved_svg_cascade_survives_unrelated_css_tokens_and_keeps_author_identity() {
        let snapshot = crate::tests::empty_snapshot();
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/style-transport.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><style>rect { --layout-only: [a] / [b]; aspect-ratio:4 / 3; fill:rgb(0,255,0) } text[id] {fill:none}</style><rect width="20" height="20"/><text id="author" x="0" y="10">Hidden</text></svg>"#.to_vec()};
        let owned = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        let prepared = std::str::from_utf8(&owned.data).unwrap();
        let xml = roxmltree::Document::parse(prepared).unwrap();
        let rect = xml
            .descendants()
            .find(|node| node.has_tag_name(("http://www.w3.org/2000/svg", "rect")))
            .unwrap();
        let style = rect.attribute("style").unwrap();
        assert!(style.contains("fill:rgb(0, 255, 0)"));
        assert!(!style.contains("aspect-ratio"));
        assert!(!style.contains("--layout-only"));
        let text = xml
            .descendants()
            .find(|node| node.has_tag_name(("http://www.w3.org/2000/svg", "text")))
            .unwrap();
        assert_eq!(text.attribute("id"), Some("author"));
        assert!(text.attribute("style").unwrap().contains("fill:none;"));
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert_eq!(
            crate::svg::raster(&tree, 20, 20)
                .unwrap()
                .get_pixel(15, 15)
                .0,
            [0, 255, 0, 255]
        );
        assert!(
            crate::svg_text::collect(&snapshot, &tree, 1., 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn secure_foreign_object_uses_own_native_cascade_without_capture_identity() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 40.;
        snapshot.viewport.height = 30.;
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/external.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><style>div { background: rgb(0,255,0); width:20px; height:10px }</style><foreignObject data-oryn-foreign-object="forged" x="2" y="3" width="25" height="20"><div xmlns="http://www.w3.org/1999/xhtml"/></foreignObject></svg>"#.to_vec()};
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        let rgba = crate::svg::raster(&tree, 40, 30).unwrap();
        assert_eq!(rgba.get_pixel(5, 5).0, [0, 255, 0, 255]);
        assert_eq!(rgba.get_pixel(30, 25)[3], 0);
        assert!(snapshot.svg_foreign_objects.is_empty());
    }
    #[test]
    fn nested_svg_in_secure_html_uses_private_native_style_and_capture() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 20.;
        snapshot.viewport.height = 20.;
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/nested.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><style>.owner rect { fill:rgb(0,255,0) }</style><foreignObject width="20" height="20"><div xmlns="http://www.w3.org/1999/xhtml" class="owner"><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg></div></foreignObject></svg>"#.to_vec()};
        let owned = prepare(&source, &snapshot, source.bytes.clone()).unwrap();
        assert_eq!(owned.snapshot.images.len(), 2);
        assert!(!owned.snapshot.images[1].svg_document_snapshot);
        let text = String::from_utf8(owned.snapshot.images[1].bytes.clone()).unwrap();
        assert!(text.contains("fill:rgb(0, 255, 0)"));
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert_eq!(
            crate::svg::raster(&tree, 20, 20).unwrap().get_pixel(5, 5).0,
            [0, 255, 0, 255]
        );
    }
    #[test]
    fn unavailable_secure_mask_has_transparent_black_used_image() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 20.;
        snapshot.viewport.height = 20.;
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/mask.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><foreignObject width="20" height="20"><div xmlns="http://www.w3.org/1999/xhtml" style="width:20px;height:20px;background:red;mask-image:url(https://forbidden.test/mask)"/></foreignObject></svg>"#.to_vec()};
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert!(
            crate::svg::raster(&tree, 20, 20)
                .unwrap()
                .pixels()
                .all(|pixel| pixel[3] == 0)
        );
    }
    #[test]
    fn secure_foreign_object_disables_external_background_but_keeps_native_box() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 20.;
        snapshot.viewport.height = 20.;
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/external.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><foreignObject width="20" height="20"><div xmlns="http://www.w3.org/1999/xhtml" style="width:20px;height:20px;background:red url(https://forbidden.test/image)"/></foreignObject></svg>"#.to_vec()};
        let tree = crate::svg::parse(&source, &snapshot).unwrap();
        assert_eq!(
            crate::svg::raster(&tree, 20, 20).unwrap().get_pixel(5, 5).0,
            [255, 0, 0, 255]
        );
    }
}
