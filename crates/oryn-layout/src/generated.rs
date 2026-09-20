//! CSS-generated content is shaped by the ordinary native formatting engine.
//! Its private paint IDs never acquire DOM identity or DOM selection offsets.
use super::*;
use oryn_common::v4::{GeneratedContent, PaintNodeSnapshot};
pub(super) struct Materialized {
    pub input: LayoutInput,
    pub styles: ComputedStyles,
    entries: BTreeMap<u64, GeneratedContent>,
    images: BTreeMap<u64, String>,
    builtin_images: BTreeMap<u64, oryn_common::builtin_image::BuiltinImage>,
    requests: BTreeSet<String>,
}
enum ContentPart {
    Text(String),
    Image(String),
}
struct Content {
    parts: Vec<ContentPart>,
    text: String,
    alternative: Option<String>,
}
#[derive(Default)]
struct Counters {
    styles: Arc<generated_counter_styles::Registry>,
    values: counter_tree::Values,
}
impl Counters {
    fn text(&self, name: &str, separator: Option<&str>, kind: &str) -> String {
        let values = self.values.get(name).map(Vec::as_slice).unwrap_or(&[]);
        if let Some(separator) = separator {
            if values.is_empty() {
                return self.styles.format(0, kind);
            }
            return values
                .iter()
                .map(|value| self.styles.format(*value, kind))
                .collect::<Vec<_>>()
                .join(separator);
        }
        self.styles
            .format(values.last().copied().unwrap_or(0), kind)
    }
}
pub(super) fn format_counter(value: i64, kind: &str) -> String {
    if matches!(
        kind,
        "lower-alpha" | "lower-latin" | "upper-alpha" | "upper-latin"
    ) && value > 0
    {
        let mut value = value as u64;
        let mut chars = Vec::new();
        while value > 0 {
            value -= 1;
            chars.push((b'a' + (value % 26) as u8) as char);
            value /= 26;
        }
        let text = chars.into_iter().rev().collect::<String>();
        return if kind.starts_with("upper") {
            text.to_ascii_uppercase()
        } else {
            text
        };
    }
    if matches!(kind, "lower-roman" | "upper-roman") && (1..=3999).contains(&value) {
        let mut value = value;
        let mut text = String::new();
        for (number, digits) in [
            (1000, "M"),
            (900, "CM"),
            (500, "D"),
            (400, "CD"),
            (100, "C"),
            (90, "XC"),
            (50, "L"),
            (40, "XL"),
            (10, "X"),
            (9, "IX"),
            (5, "V"),
            (4, "IV"),
            (1, "I"),
        ] {
            while value >= number {
                text.push_str(digits);
                value -= number;
            }
        }
        return if kind == "lower-roman" {
            text.to_ascii_lowercase()
        } else {
            text
        };
    }
    if kind == "decimal-leading-zero" && value.unsigned_abs() < 10 {
        return if value < 0 {
            format!("-0{}", -value)
        } else {
            format!("0{value}")
        };
    }
    value.to_string()
}
fn quote_pairs(value: &str) -> Result<Vec<(String, String)>, LayoutError> {
    if value == "none" {
        return Ok(Vec::new());
    }
    if value == "auto" {
        return Ok(vec![("“".into(), "”".into()), ("‘".into(), "’".into())]);
    }
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut pairs = Vec::new();
    while !parser.is_exhausted() {
        let open = parser
            .expect_string_cloned()
            .map_err(|_| LayoutError::StyleTraversal("invalid opening quote".into()))?
            .to_string();
        let close = parser
            .expect_string_cloned()
            .map_err(|_| LayoutError::StyleTraversal("invalid closing quote".into()))?
            .to_string();
        pairs.push((open, close));
    }
    Ok(pairs)
}
fn content(
    value: &str,
    node: &LayoutNode,
    counters: &Counters,
    quotes: &mut usize,
    pairs: &[(String, String)],
) -> Result<Option<Content>, LayoutError> {
    if matches!(value.trim(), "none" | "normal") {
        return Ok(None);
    }
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut text = String::new();
    let mut parts = Vec::new();
    let mut alternative = None::<String>;
    let mut speech = false;
    while !parser.is_exhausted() {
        let token = parser
            .next()
            .map_err(|_| LayoutError::StyleTraversal("invalid generated content".into()))?
            .clone();
        let image = match &token {
            cssparser::Token::UnquotedUrl(url) => Some(url.to_string()),
            cssparser::Token::Function(name) if name.eq_ignore_ascii_case("url") => Some(
                parser
                    .parse_nested_block(|parser| -> Result<String, cssparser::ParseError<'_, ()>> {
                        let url = parser.expect_string_cloned()?.to_string();
                        parser.expect_exhausted()?;
                        Ok(url)
                    })
                    .map_err(|_| {
                        LayoutError::StyleTraversal("invalid generated image URL".into())
                    })?,
            ),
            _ => None,
        };
        if let Some(url) = image {
            if speech {
                return Err(LayoutError::StyleTraversal(
                    "image is not a content speech alternative".into(),
                ));
            }
            if url.len() > 64 * 1024 || parts.len() >= 256 {
                return Err(LayoutError::StyleTraversal(
                    "generated image source bound exceeded".into(),
                ));
            }
            parts.push(ContentPart::Image(url));
            continue;
        }
        let piece = match token {
            cssparser::Token::QuotedString(text) => text.to_string(),
            cssparser::Token::Delim('/') => {
                speech = true;
                alternative = Some(String::new());
                continue;
            }
            cssparser::Token::Ident(name) => match name.as_ref() {
                "open-quote" => {
                    let text = pairs
                        .get((*quotes).min(pairs.len().saturating_sub(1)))
                        .map(|pair| pair.0.clone())
                        .unwrap_or_default();
                    *quotes = quotes.saturating_add(1);
                    text
                }
                "close-quote" => {
                    if *quotes == 0 {
                        String::new()
                    } else {
                        *quotes -= 1;
                        pairs
                            .get((*quotes).min(pairs.len().saturating_sub(1)))
                            .map(|pair| pair.1.clone())
                            .unwrap_or_default()
                    }
                }
                "no-open-quote" => {
                    *quotes = quotes.saturating_add(1);
                    String::new()
                }
                "no-close-quote" => {
                    *quotes = quotes.saturating_sub(1);
                    String::new()
                }
                _ => {
                    return Err(LayoutError::StyleTraversal(format!(
                        "unsupported generated content token {name}"
                    )));
                }
            },
            cssparser::Token::Function(name) => parser
                .parse_nested_block(|parser| -> Result<String, cssparser::ParseError<'_, ()>> {
                    let argument = parser.expect_ident_cloned()?.to_string();
                    if name.eq_ignore_ascii_case("attr") {
                        let fallback = if parser.try_parse(|p| p.expect_comma()).is_ok() {
                            parser.expect_string_cloned()?.to_string()
                        } else {
                            String::new()
                        };
                        parser.expect_exhausted()?;
                        return Ok(node.attributes.get(&argument).cloned().unwrap_or(fallback));
                    }
                    let separator = if name.eq_ignore_ascii_case("counters") {
                        parser.expect_comma()?;
                        Some(parser.expect_string_cloned()?.to_string())
                    } else if name.eq_ignore_ascii_case("counter") {
                        None
                    } else {
                        return Err(parser.new_custom_error(()));
                    };
                    let kind = if parser.try_parse(|p| p.expect_comma()).is_ok() {
                        if let Ok(name) = parser.try_parse(|p| p.expect_ident_cloned()) {
                            name.to_string()
                        } else {
                            let start = parser.position();
                            parser.expect_function_matching("symbols")?;
                            parser.parse_nested_block(
                                |parser| -> Result<(), cssparser::ParseError<'_, ()>> {
                                    let _ = parser.try_parse(|p| p.expect_ident_cloned());
                                    while !parser.is_exhausted() {
                                        parser.expect_string()?;
                                    }
                                    Ok(())
                                },
                            )?;
                            parser.slice_from(start).to_owned()
                        }
                    } else {
                        "decimal".into()
                    };
                    parser.expect_exhausted()?;
                    Ok(counters.text(&argument, separator.as_deref(), &kind))
                })
                .map_err(|_| {
                    LayoutError::StyleTraversal("unsupported generated content function".into())
                })?,
            _ => {
                return Err(LayoutError::StyleTraversal(
                    "unsupported generated content token".into(),
                ));
            }
        };
        if speech {
            alternative.as_mut().unwrap().push_str(&piece)
        } else {
            text.push_str(&piece);
            if let Some(ContentPart::Text(text)) = parts.last_mut() {
                text.push_str(&piece);
            } else {
                parts.push(ContentPart::Text(piece));
            }
        }
        if text.len() + alternative.as_ref().map_or(0, String::len) > 1024 * 1024 {
            return Err(LayoutError::StyleTraversal(
                "generated text exceeds native one-MiB source bound".into(),
            ));
        }
    }
    Ok(Some(Content {
        parts,
        text,
        alternative,
    }))
}
fn identity(used: &mut BTreeSet<u64>, next: &mut u64) -> Result<u64, LayoutError> {
    while used.contains(next) {
        *next = next.checked_sub(1).ok_or_else(|| {
            LayoutError::StyleTraversal("generated identity space exhausted".into())
        })?;
    }
    let id = *next;
    used.insert(id);
    *next = next.saturating_sub(1);
    Ok(id)
}
pub(super) fn materialize(
    input: &LayoutInput,
    styles: ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) -> Result<Materialized, LayoutError> {
    materialize_for_media(input, styles, pseudos, false)
}
pub(super) fn materialize_for_media(
    input: &LayoutInput,
    mut styles: ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    print: bool,
) -> Result<Materialized, LayoutError> {
    let mut projected = project_rendered_tree(input);
    projected.rendered_tree = None;
    let sources = projected
        .nodes
        .iter()
        .map(|node| (node.id, node.clone()))
        .collect::<BTreeMap<_, _>>();
    let ordinals = list_markers::ordinals(&sources, &styles);
    let counter_states = counter_tree::build(&sources, &styles, pseudos)?;
    let mut used = sources
        .keys()
        .chain(styles.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut next = u64::MAX;
    let mut counter_registries = BTreeMap::new();
    counter_registries.insert(
        None,
        generated_counter_styles::Registry::read_for_scope(input, print, None)?,
    );
    let mut counters = Counters {
        styles: counter_registries[&None].clone(),
        ..Default::default()
    };
    let mut quote_depth = 0;
    let mut entries = BTreeMap::new();
    let mut additions = Vec::new();
    let mut images = BTreeMap::new();
    let mut builtin_images = BTreeMap::new();
    let mut requests = BTreeSet::new();
    let mut pending = projected
        .nodes
        .iter()
        .rev()
        .filter(|node| node.parent.is_none())
        .map(|node| (node.id, 0usize, false))
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some((id, depth, after)) = pending.pop() {
        let Some(source) = sources.get(&id) else {
            continue;
        };
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let interest_area = source.tag == "area"
            && input
                .selector_states
                .get(&id)
                .is_some_and(|state| state.interest_invoker);
        if style_value(style, "display", "inline") == "none" && !interest_area {
            continue;
        }
        if !after {
            if !visited.insert(id) {
                continue;
            }
            pending.push((id, depth, true));
            pending.extend(
                source
                    .children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth + 1, false)),
            );
        }
        let mut names = Vec::new();
        if !after
            && ordinals.contains_key(&id)
            && !source.children.iter().any(|child| {
                sources
                    .get(child)
                    .is_some_and(|node| node.tag == "::marker")
            })
        {
            names.push("marker");
        }
        names.push(if after { "after" } else { "before" });
        if !after
            && matches!(
                source.tag.as_str(),
                "geolocation" | "usermedia" | "camera" | "microphone" | "install"
            )
        {
            names.push("permission-icon");
        }
        if after
            && input
                .selector_states
                .get(&id)
                .is_some_and(|state| state.interest_invoker)
        {
            names.push("interest-button");
        }
        if after
            && source.tag == "select"
            && style_value(style, "appearance", "auto") == "base-select"
        {
            names.push("picker-icon");
        }
        if !after && source.tag == "option" {
            let mut parent = source.parent;
            while let Some(id) = parent {
                let Some(ancestor) = sources.get(&id) else {
                    break;
                };
                if ancestor.tag == "select" {
                    if styles.get(&id).is_some_and(|style| {
                        style_value(style, "appearance", "auto") == "base-select"
                    }) {
                        names.push("checkmark");
                    }
                    break;
                }
                parent = ancestor.parent;
            }
        }
        let scope = input.shadow_trees.node_scopes.get(&id).copied();
        if let std::collections::btree_map::Entry::Vacant(e) = counter_registries.entry(scope) {
            e.insert(generated_counter_styles::Registry::read_for_scope(
                input, print, scope,
            )?);
        }
        counters.styles = counter_registries[&scope].clone();
        let marker_fallback = style.clone();
        for name in names {
            let pseudo_style = pseudos.get(&id).and_then(|pseudos| pseudos.get(name));
            let style = if name == "marker" {
                pseudo_style.unwrap_or(&marker_fallback)
            } else {
                let Some(style) = pseudo_style else { continue };
                style
            };
            if style_value(style, "display", "inline") == "none" {
                continue;
            }
            let content_scope = match style.get("oryn-content-tree").map(String::as_str) {
                Some("document") => None,
                Some(value) => Some(value.parse::<u64>().map_err(|_| {
                    LayoutError::StyleTraversal("invalid computed content tree".into())
                })?),
                None => scope,
            };
            if let std::collections::btree_map::Entry::Vacant(e) =
                counter_registries.entry(content_scope)
            {
                e.insert(generated_counter_styles::Registry::read_for_scope(
                    input,
                    print,
                    content_scope,
                )?);
            }
            counters.styles = counter_registries[&content_scope].clone();
            counters.values = counter_states.get(&(id, name)).cloned().unwrap_or_default();
            let pairs = quote_pairs(style_value(style, "quotes", "auto"))?;
            let mut marker_background = None;
            let marker_default =
                name == "marker" && style_value(style, "content", "normal") == "normal";
            let generated_content = if marker_default {
                let owner_style = &styles[&id];
                let list_scope = match owner_style.get("oryn-list-style-tree").map(String::as_str) {
                    Some("document") => None,
                    Some(value) => Some(value.parse::<u64>().map_err(|_| {
                        LayoutError::StyleTraversal("invalid computed counter style tree".into())
                    })?),
                    None => scope,
                };
                if let std::collections::btree_map::Entry::Vacant(e) =
                    counter_registries.entry(list_scope)
                {
                    e.insert(generated_counter_styles::Registry::read_for_scope(
                        input, print, list_scope,
                    )?);
                }
                let text = counter_registries[&list_scope].marker(
                    counters
                        .values
                        .get("list-item")
                        .and_then(|values| values.last())
                        .copied()
                        .unwrap_or(ordinals[&id]),
                    style_value(owner_style, "list-style-type", "disc"),
                );
                let image = style_value(owner_style, "list-style-image", "none");
                let gradient = [
                    "linear-gradient(",
                    "radial-gradient(",
                    "conic-gradient(",
                    "repeating-linear-gradient(",
                    "repeating-radial-gradient(",
                    "repeating-conic-gradient(",
                ]
                .iter()
                .any(|prefix| image.starts_with(prefix));
                if gradient {
                    marker_background = Some(image.to_owned());
                    Some(Content {
                        parts: Vec::new(),
                        text: String::new(),
                        alternative: None,
                    })
                } else {
                    let image_content = if image != "none" {
                        content(image, source, &counters, &mut quote_depth, &pairs)?
                    } else {
                        None
                    };
                    let ready = image_content.as_ref().is_some_and(|content| {
                        content.parts.iter().all(|part| match part {
                            ContentPart::Image(url) => {
                                requests.insert(url.clone());
                                input.image_intrinsics.contains_key(url)
                            }
                            _ => false,
                        })
                    });
                    if ready {
                        image_content
                    } else {
                        Some(Content {
                            parts: vec![ContentPart::Text(text.clone())],
                            text,
                            alternative: None,
                        })
                    }
                }
            } else {
                content(
                    style_value(style, "content", "normal"),
                    source,
                    &counters,
                    &mut quote_depth,
                    &pairs,
                )?
            };
            let Some(Content {
                parts,
                text,
                alternative,
            }) = generated_content
            else {
                continue;
            };
            let generated = identity(&mut used, &mut next)?;
            let mut node = source.clone();
            node.id = generated;
            node.parent = Some(id);
            node.children.clear();
            node.tag = format!("::{name}");
            node.attributes.clear();
            node.namespaced_attributes.clear();
            node.checked = None;
            node.validity = None;
            node.disabled = None;
            node.indeterminate = false;
            node.image_intrinsics = None;
            node.image_document_root = None;
            node.intrinsic_image_size = None;
            node.document_replaced = false;
            node.text = text.clone();
            node.content = Some(vec![LayoutContent::Text {
                node_id: generated,
                text: text.clone(),
            }]);
            if interest_area {
                node.parent = source.parent;
            }
            let mut style = style.clone();
            if name == "interest-button" {
                let values = Arc::make_mut(&mut style);
                values.insert("-oryn-implicit-anchor".into(), id.to_string());
                values.insert("-oryn-anchor-parent".into(), id.to_string());
            }
            if name == "marker" {
                let outside =
                    style_value(&styles[&id], "list-style-position", "outside") == "outside";
                let values = Arc::make_mut(&mut style);
                if pseudo_style.is_none() {
                    values.retain(|property, _| {
                        property.starts_with("font-")
                            || matches!(
                                property.as_str(),
                                "color"
                                    | "line-height"
                                    | "direction"
                                    | "writing-mode"
                                    | "text-orientation"
                                    | "letter-spacing"
                                    | "word-spacing"
                                    | "visibility"
                            )
                    });
                }
                values.insert("display".into(), "inline".into());
                values.insert("white-space".into(), "pre".into());
                if outside {
                    values.insert("oryn-marker-position".into(), "outside".into());
                    node.tag = "::marker-outside".into();
                    values.insert("position".into(), "absolute".into());
                    values.insert("width".into(), "max-content".into());
                    values.insert("height".into(), "max-content".into());
                }
            }
            if let Some(background) = marker_background {
                let values = Arc::make_mut(&mut style);
                values.insert("background-image".into(), background);
                values.insert("background-repeat".into(), "no-repeat".into());
                values.insert("background-size".into(), "100% 100%".into());
                values.insert("display".into(), "inline-block".into());
                values.insert("width".into(), "1em".into());
                values.insert("height".into(), "1em".into());
            }
            if name == "checkmark"
                && !source
                    .checked
                    .unwrap_or_else(|| source.attributes.contains_key("selected"))
            {
                Arc::make_mut(&mut style).insert("visibility".into(), "hidden".into());
            }
            if name == "permission-icon"
                && matches!(
                    source.tag.as_str(),
                    "geolocation" | "usermedia" | "camera" | "microphone" | "install"
                )
            {
                let kind = if source.tag == "install" {
                    oryn_common::builtin_image::PermissionIconKind::Install
                } else if matches!(source.tag.as_str(), "usermedia" | "camera" | "microphone") {
                    oryn_common::builtin_image::PermissionIconKind::UserMedia
                } else if source
                    .attributes
                    .get("accuracymode")
                    .is_some_and(|value| value.eq_ignore_ascii_case("precise"))
                {
                    oryn_common::builtin_image::PermissionIconKind::PreciseLocation
                } else {
                    oryn_common::builtin_image::PermissionIconKind::Location
                };
                let fill = oryn_common::color::parse_color(style_value(&style, "fill", "black"))
                    .unwrap_or([0; 4]);
                let stroke = oryn_common::color::parse_color(style_value(&style, "stroke", "none"))
                    .unwrap_or([0; 4]);
                let stroke_width = style_value(&style, "stroke-width", "1")
                    .trim_end_matches("px")
                    .parse::<f64>()
                    .unwrap_or(1.)
                    .clamp(0., 24.);
                builtin_images.insert(
                    generated,
                    oryn_common::builtin_image::BuiltinImage::PermissionIcon {
                        kind,
                        fill,
                        stroke,
                        stroke_width,
                    },
                );
                node.tag = "img".into();
                node.text.clear();
                node.content = None;
                node.intrinsic_image_size = Some((24, 24));
            }
            let single_image =
                parts.len() == 1 && matches!(parts.first(), Some(ContentPart::Image(_)));
            if single_image {
                let ContentPart::Image(url) = parts.into_iter().next().unwrap() else {
                    unreachable!()
                };
                node.tag = "img".into();
                node.content = None;
                node.text.clear();
                node.intrinsic_image_size = input.image_intrinsics.get(&url).copied();
                node.attributes.insert("src".into(), url.clone());
                images.insert(generated, url);
            } else if parts
                .iter()
                .any(|part| matches!(part, ContentPart::Image(_)))
            {
                let mut content = Vec::new();
                for part in parts {
                    match part {
                        ContentPart::Text(text) => content.push(LayoutContent::Text {
                            node_id: generated,
                            text,
                        }),
                        ContentPart::Image(url) => {
                            let child_id = identity(&mut used, &mut next)?;
                            let mut child = node.clone();
                            child.id = child_id;
                            child.parent = Some(generated);
                            child.tag = "img".into();
                            child.text.clear();
                            child.content = None;
                            child.children.clear();
                            child.attributes.insert("src".into(), url.clone());
                            child.intrinsic_image_size = input.image_intrinsics.get(&url).copied();
                            // Anonymous replaced boxes inherit text/direction state,
                            // never the originating pseudo's positioning or effects.
                            let image_style = style
                                .iter()
                                .filter(|(property, _)| {
                                    matches!(
                                        property.as_str(),
                                        "color"
                                            | "direction"
                                            | "writing-mode"
                                            | "text-orientation"
                                            | "visibility"
                                            | "cursor"
                                            | "font-family"
                                            | "font-size"
                                            | "font-style"
                                            | "font-weight"
                                            | "font-stretch"
                                            | "font-kerning"
                                            | "font-feature-settings"
                                            | "font-variation-settings"
                                            | "line-height"
                                            | "letter-spacing"
                                            | "word-spacing"
                                            | "text-transform"
                                            | "text-indent"
                                            | "white-space"
                                            | "white-space-collapse"
                                            | "text-wrap-mode"
                                            | "text-wrap-style"
                                            | "word-break"
                                            | "overflow-wrap"
                                            | "text-align"
                                            | "text-align-last"
                                            | "text-rendering"
                                            | "image-rendering"
                                            | "pointer-events"
                                    )
                                })
                                .map(|(property, value)| (property.clone(), value.clone()))
                                .chain([("display".into(), "inline".into())])
                                .collect();
                            styles.insert(child_id, Arc::new(image_style));
                            images.insert(child_id, url);
                            entries.insert(
                                child_id,
                                GeneratedContent {
                                    separates_text: false,
                                    owner: generated,
                                    pseudo: "generated-image".into(),
                                    text: String::new(),
                                    alternative: None,
                                },
                            );
                            additions.push(child);
                            node.children.push(child_id);
                            content.push(LayoutContent::Element(child_id));
                        }
                    }
                }
                node.text.clear();
                node.content = Some(content);
            }
            let separates_text = matches!(name, "before" | "after")
                && !matches!(
                    style_value(&style, "display", "inline"),
                    "inline" | "contents" | "none"
                );
            styles.insert(generated, style);
            entries.insert(
                generated,
                GeneratedContent {
                    separates_text,
                    owner: id,
                    pseudo: name.into(),
                    text,
                    alternative,
                },
            );
            additions.push(node);
        }
    }
    additions.sort_by_key(|node| std::cmp::Reverse(node.id));
    for node in additions {
        let entry = &entries[&node.id];
        if node.parent != Some(entry.owner) {
            // An area's image-map box remains non-rendered. Its authored
            // interest pseudos participate beside that zero-box owner rather
            // than accidentally acquiring its display:none paint ancestry.
            if let Some(parent) = node
                .parent
                .and_then(|id| projected.nodes.iter_mut().find(|owner| owner.id == id))
            {
                if parent.content.is_none() {
                    parent.content = Some(
                        parent
                            .children
                            .iter()
                            .copied()
                            .map(LayoutContent::Element)
                            .collect(),
                    );
                }
                let items = parent.content.as_mut().unwrap();
                let before = entry.pseudo == "before";
                let start = items
                    .iter()
                    .position(|item| matches!(item,LayoutContent::Element(id)if *id==entry.owner))
                    .unwrap_or(items.len());
                let index = if before {
                    start
                } else {
                    let mut index = (start + 1).min(items.len());
                    while index < items.len()
                        && matches!(&items[index],LayoutContent::Element(id)if entries.get(id).is_some_and(|existing|existing.owner==entry.owner))
                    {
                        index += 1;
                    }
                    index
                };
                let child_index = items[..index]
                    .iter()
                    .filter(|item| matches!(item, LayoutContent::Element(_)))
                    .count();
                items.insert(index, LayoutContent::Element(node.id));
                parent.children.insert(child_index, node.id);
            }
            projected.nodes.push(node);
            continue;
        }
        let owner = projected
            .nodes
            .iter_mut()
            .find(|owner| owner.id == entry.owner)
            .expect("generated owner");
        if owner.content.is_none() {
            let mut items = Vec::new();
            if !owner.text.is_empty() {
                items.push(LayoutContent::Text {
                    node_id: owner.id,
                    text: owner.text.clone(),
                });
            }
            items.extend(owner.children.iter().copied().map(LayoutContent::Element));
            owner.content = Some(items);
        }
        let content = owner.content.as_mut().unwrap();
        if !owner.children.contains(&node.id) {
            if matches!(
                entry.pseudo.as_str(),
                "before" | "checkmark" | "marker" | "permission-icon"
            ) {
                let rank = |pseudo: &str| match pseudo {
                    "marker" => 0,
                    "checkmark" | "permission-icon" => 1,
                    "before" => 2,
                    _ => 3,
                };
                let index = content
                    .iter()
                    .take_while(|item| match item {
                        LayoutContent::Element(id) => entries
                            .get(id)
                            .is_some_and(|existing| rank(&existing.pseudo) < rank(&entry.pseudo)),
                        _ => false,
                    })
                    .count();
                let child_index = content[..index]
                    .iter()
                    .filter(|item| matches!(item, LayoutContent::Element(_)))
                    .count();
                content.insert(index, LayoutContent::Element(node.id));
                owner.children.insert(child_index, node.id);
            } else {
                content.push(LayoutContent::Element(node.id));
                owner.children.push(node.id);
            }
        }
        projected.nodes.push(node);
    }
    Ok(Materialized {
        input: projected,
        styles,
        entries,
        images,
        builtin_images,
        requests,
    })
}
pub(super) fn finish(generated: &Materialized, output: &mut LayoutOutput) {
    output
        .css_image_requests
        .extend(generated.requests.iter().cloned());
    super::effect_resources::discover(
        &output.computed_styles,
        &generated.input.document_base_url,
        &mut output.css_image_requests,
    );
    for (id, entry) in generated.entries.iter().rev() {
        let Some(geometry) = output.geometry.get(id).cloned() else {
            continue;
        };
        if let Some(node) = generated.input.nodes.iter().find(|node| node.id == *id)
            && node.parent != Some(entry.owner)
        {
            output.paint_parent_overrides.insert(*id, node.parent);
        }
        output.generated_nodes.push(PaintNodeSnapshot {
            builtin_image: generated.builtin_images.get(id).cloned(),
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            node_id: *id,
            parent_id: Some(entry.owner),
            tag: format!("::{}", entry.pseudo),
            geometry,
            projection: output.paint_projections.get(id).cloned(),
            style: output
                .computed_styles
                .get(id)
                .map(|style| (**style).clone())
                .unwrap_or_default(),
            text: output.text_fragments.get(id).cloned().unwrap_or_default(),
            image_resource: None,
        });
        if matches!(entry.pseudo.as_str(), "before" | "after" | "marker") {
            output.generated_content.push(entry.clone());
        }
        output.text_clusters.remove(id);
        if let Some(url) = generated.images.get(id) {
            // This private replaced box is a CSS resource consumer even when
            // its owner lives in an isolated picker subtree. Publish demand
            // with the generated identity, not only marker fallback demand.
            output.css_image_requests.insert(url.clone());
            output.generated_image_urls.insert(*id, url.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_quote_pairs_preserve_multicharacters_and_none() {
        assert_eq!(
            quote_pairs(r#""<<" ">>" "[" "]""#).unwrap(),
            vec![("<<".into(), ">>".into()), ("[".into(), "]".into())]
        );
        assert!(quote_pairs("none").unwrap().is_empty());
        assert!(quote_pairs(r#""unpaired""#).is_err());
        assert_eq!(
            format_counter(i64::MIN, "decimal-leading-zero"),
            i64::MIN.to_string()
        );
    }
}
