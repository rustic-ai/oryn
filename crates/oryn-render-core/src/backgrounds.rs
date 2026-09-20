//! Layer geometry is resolved once; repeated tiles sample a single captured
//! source. Tiny repeat periods never allocate a display command for every tile.

/// Inputs for commands for fragment.
pub(super) struct BackgroundFragmentInputs<'a> {
    pub(super) snapshot: &'a PaintSnapshot,
    pub(super) style: &'a std::collections::BTreeMap<String, String>,
    pub(super) model: &'a oryn_common::v4::BoxModel,
    pub(super) radii: [f64; 8],
    pub(super) scroll: Option<&'a oryn_common::v4::PaintScrollGeometry>,
    pub(super) transformed_ancestor: bool,
    pub(super) canvas_clip: Option<CssRect>,
    pub(super) fragment: Option<&'a oryn_common::v4::BoxModel>,
}
/// Inputs for source image.
struct BackgroundImageInputs<'a> {
    snapshot: &'a PaintSnapshot,
    value: &'a str,
    rect: CssRect,
    color: &'a str,
    font: f64,
    rendering: ImageRendering,
    environment: SvgEnvironment,
    depth: usize,
}
type RepeatAxisPlacement = (f64, Option<f64>, Option<[f64; 2]>);

use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTile {
    pub rect: CssRect,
    pub step_x: Option<f64>,
    pub step_y: Option<f64>,
    pub limit_x: Option<[f64; 2]>,
    pub limit_y: Option<[f64; 2]>,
}
impl BackgroundTile {
    pub fn map(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        fn axis(
            point: f64,
            origin: f64,
            size: f64,
            step: Option<f64>,
            limit: Option<[f64; 2]>,
        ) -> Option<f64> {
            if !point.is_finite()
                || !origin.is_finite()
                || !size.is_finite()
                || size <= 0.
                || limit.is_some_and(|limit| point < limit[0] || point >= limit[1])
            {
                return None;
            }
            let offset = point - origin;
            let offset = if let Some(step) = step {
                if !step.is_finite() || step <= 0. {
                    return None;
                }
                offset.rem_euclid(step)
            } else {
                offset
            };
            (offset >= 0. && offset < size).then_some(origin + offset)
        }
        Some((
            axis(x, self.rect.x, self.rect.width, self.step_x, self.limit_x)?,
            axis(y, self.rect.y, self.rect.height, self.step_y, self.limit_y)?,
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackgroundSource {
    Gradient(BackgroundGradient),
    Image {
        resource: usize,
        svg: bool,
        rendering: ImageRendering,
        #[serde(default)]
        environment: SvgEnvironment,
    },
    Solid([u8; 4]),
    CrossFade(Vec<(f64, BackgroundSource)>),
}
fn list<'a>(
    style: &'a std::collections::BTreeMap<String, String>,
    key: &str,
    default: &'a str,
    index: usize,
) -> &'a str {
    let parts = gradients::split(style.get(key).map(String::as_str).unwrap_or(default), ',')
        .unwrap_or_else(|| vec![default]);
    parts[index % parts.len()]
}
fn box_for(model: &oryn_common::v4::BoxModel, value: &str) -> CssRect {
    match value {
        "content-box" => model.content_box,
        "padding-box" => model.padding_box,
        _ => model.border_box,
    }
}
fn clip_radii(model: &oryn_common::v4::BoxModel, rect: CssRect, radii: [f64; 8]) -> [f64; 8] {
    let left = rect.x - model.border_box.x;
    let top = rect.y - model.border_box.y;
    let right = model.border_box.right() - rect.right();
    let bottom = model.border_box.bottom() - rect.bottom();
    let edges = [left, top, right, top, right, bottom, left, bottom];
    std::array::from_fn(|i| (radii[i] - edges[i]).max(0.))
}
pub(super) fn canvas_commands(
    background: Vec<PaintCommand>,
    style: &std::collections::BTreeMap<String, String>,
    owner: u64,
) -> Vec<PaintCommand> {
    let mut commands = Vec::new();
    let color = style
        .get("background-color")
        .and_then(|value| parse_color(value))
        .unwrap_or([0; 4]);
    if color[3] > 0 {
        let color = PaintCommand::Canvas {
            color,
            layers: Vec::new(),
        };
        commands.push(if color_uses_text(style) {
            PaintCommand::TextClip {
                owner,
                fragment: None,
                commands: vec![color],
            }
        } else {
            color
        });
    }
    for command in background {
        commands.push(match command {
            PaintCommand::Background { tile, source, .. } => PaintCommand::Canvas {
                color: [0; 4],
                layers: vec![(tile, source)],
            },
            command => command,
        });
    }
    commands
}
pub(super) fn color_uses_text(style: &std::collections::BTreeMap<String, String>) -> bool {
    let count = gradients::split(
        style
            .get("background-image")
            .map(String::as_str)
            .unwrap_or("none"),
        ',',
    )
    .map(|parts| parts.len())
    .unwrap_or(1);
    list(style, "background-clip", "border-box", count - 1) == "text"
}
pub(super) fn color_clip(
    style: &std::collections::BTreeMap<String, String>,
    model: &oryn_common::v4::BoxModel,
    radii: [f64; 8],
) -> (CssRect, [f64; 8]) {
    let count = gradients::split(
        style
            .get("background-image")
            .map(String::as_str)
            .unwrap_or("none"),
        ',',
    )
    .map(|layers| layers.len())
    .unwrap_or(1);
    let rect = box_for(
        model,
        list(style, "background-clip", "border-box", count - 1),
    );
    (rect, clip_radii(model, rect, radii))
}
fn length(
    value: &str,
    basis: f64,
    font: f64,
    viewport: oryn_common::v4::ViewportState,
) -> Option<f64> {
    oryn_common::css_math::resolve_length(
        value,
        oryn_common::css_math::LengthContext {
            percentage_basis: basis,
            font_size: font,
            root_font_size: 16.,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
        },
    )
    .filter(|value| value.is_finite())
}
fn position(
    value: &str,
    free: f64,
    font: f64,
    viewport: oryn_common::v4::ViewportState,
) -> Option<f64> {
    match value {
        "left" | "top" => Some(0.),
        "center" => Some(free / 2.),
        "right" | "bottom" => Some(free),
        value => {
            let units = value
                .replace("min", "")
                .replace("max", "")
                .replace("clamp", "")
                .replace("calc", "");
            if free < 0. && value.contains('%') && !units.chars().any(|ch| ch.is_ascii_alphabetic())
            {
                length(value, 1., font, viewport).map(|fraction| fraction * free)
            } else {
                length(value, free, font, viewport)
            }
        }
    }
}
fn positions(
    value: &str,
    free: (f64, f64),
    font: f64,
    viewport: oryn_common::v4::ViewportState,
) -> Option<(f64, f64)> {
    let parts = gradients::split(value, ' ')?
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [a] => {
            if matches!(*a, "top" | "bottom") {
                Some((free.0 / 2., position(a, free.1, font, viewport)?))
            } else {
                Some((position(a, free.0, font, viewport)?, free.1 / 2.))
            }
        }
        [a, b] => {
            if matches!(*a, "top" | "bottom") {
                Some((
                    position(b, free.0, font, viewport)?,
                    position(a, free.1, font, viewport)?,
                ))
            } else {
                Some((
                    position(a, free.0, font, viewport)?,
                    position(b, free.1, font, viewport)?,
                ))
            }
        }
        [a, b, c] => {
            let horizontal = |value: &str| matches!(value, "left" | "right");
            let vertical = |value: &str| matches!(value, "top" | "bottom");
            let edge = |side: &str, offset: &str, basis: f64| {
                let value = length(offset, basis, font, viewport)?;
                Some(if matches!(side, "right" | "bottom") {
                    basis - value
                } else {
                    value
                })
            };
            if horizontal(a) && vertical(c) {
                Some((edge(a, b, free.0)?, position(c, free.1, font, viewport)?))
            } else if vertical(a) && horizontal(c) {
                Some((position(c, free.0, font, viewport)?, edge(a, b, free.1)?))
            } else if horizontal(a) && vertical(b) {
                Some((position(a, free.0, font, viewport)?, edge(b, c, free.1)?))
            } else if vertical(a) && horizontal(b) {
                Some((edge(b, c, free.0)?, position(a, free.1, font, viewport)?))
            } else {
                None
            }
        }
        [a, b, c, d] => {
            let pair = |side: &str, offset: &str, basis: f64| {
                let offset = length(offset, basis, font, viewport)?;
                Some(if matches!(side, "right" | "bottom") {
                    basis - offset
                } else {
                    offset
                })
            };
            if matches!(*a, "left" | "right") {
                Some((pair(a, b, free.0)?, pair(c, d, free.1)?))
            } else {
                Some((pair(c, d, free.0)?, pair(a, b, free.1)?))
            }
        }
        _ => None,
    }
}
pub(super) fn selected_image(value: &str, density: f64) -> Option<(String, f64)> {
    let body = value
        .strip_prefix("image-set(")
        .or_else(|| value.strip_prefix("-webkit-image-set("));
    let Some(body) = body else {
        return Some((value.into(), 1.));
    };
    let candidates = gradients::split(body.strip_suffix(')')?, ',')?;
    if candidates.len() > 256 {
        return None;
    }
    let mut selected = None::<(String, f64)>;
    for candidate in candidates {
        let parts = gradients::split(candidate, ' ')?;
        let mut parts = parts.into_iter().filter(|part| !part.is_empty());
        let source = parts.next()?;
        let mut resolution = 1.;
        let mut supported = true;
        for part in parts {
            if let Some(mime) = part
                .strip_prefix("type(")
                .and_then(|part| part.strip_suffix(')'))
            {
                let mut input = cssparser::ParserInput::new(mime);
                let mut parser = cssparser::Parser::new(&mut input);
                let mime = parser.expect_string().ok()?;
                supported = matches!(
                    mime.to_ascii_lowercase().as_str(),
                    "image/png"
                        | "image/jpeg"
                        | "image/svg+xml"
                        | "image/gif"
                        | "image/webp"
                        | "image/bmp"
                        | "image/x-ms-bmp"
                );
            } else {
                resolution = if let Some(number) =
                    part.strip_suffix("dppx").or_else(|| part.strip_suffix('x'))
                {
                    number.parse().ok()?
                } else if let Some(number) = part.strip_suffix("dpi") {
                    number.parse::<f64>().ok()? / 96.
                } else {
                    let number = part.strip_suffix("dpcm")?;
                    number.parse::<f64>().ok()? * 2.54 / 96.
                };
            }
        }
        if !supported || !resolution.is_finite() || resolution <= 0. {
            continue;
        }
        let replace = selected.as_ref().is_none_or(|(_, old)| {
            if *old < density {
                resolution > *old
            } else {
                resolution >= density && resolution < *old
            }
        });
        if replace {
            selected = Some((
                if source.starts_with(['"', '\'']) {
                    format!("url({source})")
                } else {
                    source.into()
                },
                resolution,
            ));
        }
    }
    selected
}
fn resource_url(value: &str) -> Option<String> {
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let value = parser.expect_url().ok()?.as_ref().to_owned();
    parser.expect_exhausted().ok()?;
    Some(value)
}
fn fade_components(value: &str) -> Result<Option<Vec<(f64, &str)>>, String> {
    let Some(body) = value
        .strip_prefix("cross-fade(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Ok(None);
    };
    let parts = gradients::split(body, ',').ok_or("invalid cross-fade")?;
    if parts.is_empty() || parts.len() > 256 {
        return Err("cross-fade component bound exceeded".into());
    }
    let mut entries = Vec::new();
    let mut sum = 0.;
    let mut missing = 0usize;
    for part in parts {
        let words = gradients::split(part, ' ')
            .ok_or("invalid cross-fade component")?
            .into_iter()
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        let (weight, image) = match words.as_slice() {
            [image] => (None, *image),
            [a, b] => {
                let percent = |word: &str| {
                    word.strip_suffix('%')
                        .and_then(|word| word.parse::<f64>().ok())
                        .map(|value| value / 100.)
                };
                if let Some(weight) = percent(a) {
                    (Some(weight), *b)
                } else if let Some(weight) = percent(b) {
                    (Some(weight), *a)
                } else {
                    return Err("invalid cross-fade percentage".into());
                }
            }
            _ => return Err("invalid cross-fade component".into()),
        };
        if let Some(weight) = weight {
            if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
                return Err("cross-fade percentage out of range".into());
            }
            sum += weight;
        } else {
            missing += 1;
        }
        entries.push((weight, image));
    }
    let remainder = if missing > 0 {
        (1_f64 - sum).max(0.) / missing as f64
    } else {
        0.
    };
    let normalize = sum.max(1.);
    Ok(Some(
        entries
            .into_iter()
            .map(|(weight, image)| (weight.unwrap_or(remainder) / normalize, image))
            .collect(),
    ))
}
fn source_image(params: BackgroundImageInputs<'_>) -> Result<BackgroundSource, String> {
    let BackgroundImageInputs {
        snapshot,
        value,
        rect,
        color,
        font,
        rendering,
        environment,
        depth,
    } = params;

    if depth > 16 {
        return Err("cross-fade nesting exceeds 16".into());
    }
    let (selected, _) = selected_image(value, snapshot.viewport.device_pixel_ratio)
        .ok_or("unsupported image-set candidate")?;
    let value = selected.as_str();
    if let Some(parts) = fade_components(value)? {
        return parts
            .into_iter()
            .map(|(weight, value)| {
                Ok((
                    weight,
                    source_image(BackgroundImageInputs {
                        snapshot,
                        value,
                        rect,
                        color,
                        font,
                        rendering,
                        environment,
                        depth: depth + 1,
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(BackgroundSource::CrossFade);
    }
    if let Some(url) = resource_url(value) {
        let resource = snapshot
            .images
            .iter()
            .position(|image| {
                image.url == url || image.url_aliases.iter().any(|alias| alias == &url)
            })
            .ok_or_else(|| format!("background resource not captured: {url}"))?;
        let svg = snapshot.images[resource]
            .media_type
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim() == "image/svg+xml");
        return Ok(BackgroundSource::Image {
            resource,
            svg,
            rendering,
            environment,
        });
    }
    if let Some(color) = parse_color(if value.eq_ignore_ascii_case("currentcolor") {
        color
    } else {
        value
    }) {
        return Ok(BackgroundSource::Solid(color));
    }
    gradients::backgrounds(value, rect, color, font)
        .and_then(|mut gradients| {
            if gradients.is_empty() {
                None
            } else {
                Some(gradients.remove(0))
            }
        })
        .map(BackgroundSource::Gradient)
        .ok_or_else(|| "unsupported background image".into())
}
fn image_intrinsic(
    snapshot: &PaintSnapshot,
    value: &str,
    depth: usize,
) -> Result<Option<(f64, f64)>, String> {
    if depth > 16 {
        return Err("cross-fade intrinsic nesting exceeds 16".into());
    }
    let (selected, resolution) =
        selected_image(value, snapshot.viewport.device_pixel_ratio).ok_or("invalid image-set")?;
    if let Some(parts) = fade_components(&selected)? {
        let (mut width, mut height, mut weight_sum) = (0., 0., 0.);
        for (weight, value) in parts {
            if weight == 0. {
                continue;
            }
            let Some((w, h)) = image_intrinsic(snapshot, value, depth + 1)? else {
                return Ok(None);
            };
            width += w * weight;
            height += h * weight;
            weight_sum += weight;
        }
        return Ok((weight_sum > 0.).then_some((width / weight_sum, height / weight_sum)));
    }
    let Some(url) = resource_url(&selected) else {
        return Ok(None);
    };
    let index = snapshot
        .images
        .iter()
        .position(|image| image.url == url || image.url_aliases.iter().any(|alias| alias == &url))
        .ok_or("background resource not captured")?;
    let svg = snapshot.images[index]
        .media_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim() == "image/svg+xml");
    intrinsic(snapshot, index, svg)
        .map(|(w, h)| Some((w / resolution, h / resolution)))
        .map_err(|error| error.to_string())
}
fn intrinsic(snapshot: &PaintSnapshot, index: usize, svg: bool) -> Result<(f64, f64), RenderError> {
    let source = &snapshot.images[index];
    if svg {
        let tree = crate::svg::parse(source, snapshot)?;
        Ok((
            f64::from(tree.size().width()),
            f64::from(tree.size().height()),
        ))
    } else {
        if source.bytes.len() > 64 * 1024 * 1024 {
            return Err(RenderError::ImageDecode {
                url: source.url.clone(),
                detail: "encoded background exceeds 64 MiB".into(),
            });
        }
        let format =
            image::guess_format(&source.bytes).map_err(|error| RenderError::ImageDecode {
                url: source.url.clone(),
                detail: error.to_string(),
            })?;
        let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(&source.bytes), format)
            .into_dimensions()
            .map_err(|error| RenderError::ImageDecode {
                url: source.url.clone(),
                detail: error.to_string(),
            })?;
        if (w as usize)
            .checked_mul(h as usize)
            .is_none_or(|pixels| pixels > 16 * 1024 * 1024)
        {
            return Err(RenderError::ImageDecode {
                url: source.url.clone(),
                detail: "background exceeds 16 million pixels".into(),
            });
        }
        Ok((f64::from(w), f64::from(h)))
    }
}
/// CSS Transforms makes fixed background attachment behave as scroll on
/// transformed elements and their descendants. Use authored computed ancestry,
/// not numeric matrix comparisons (identity transforms still establish it).
pub(super) fn canvas_rect(
    node: &oryn_common::v4::PaintNodeSnapshot,
    viewport: oryn_common::v4::ViewportState,
) -> Option<CssRect> {
    let Some(projection) = &node.projection else {
        return Some(CssRect {
            x: 0.,
            y: 0.,
            width: viewport.width,
            height: viewport.height,
        });
    };
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [
        (0., 0.),
        (viewport.width, 0.),
        (viewport.width, viewport.height),
        (0., viewport.height),
    ] {
        let (x, y) = projection.viewport_to_local(x, y)?;
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    }
    let rect = CssRect {
        x: bounds.0,
        y: bounds.1,
        width: bounds.2 - bounds.0,
        height: bounds.3 - bounds.1,
    };
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|v| v.is_finite())
        .then_some(rect)
}
pub(super) fn canvas_source(snapshot: &PaintSnapshot) -> Option<(u64, u64)> {
    let root = snapshot
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none() && node.tag == "html")?;
    let boxed = |node: &oryn_common::v4::PaintNodeSnapshot| {
        !matches!(
            node.style.get("display").map(String::as_str),
            Some("none" | "contents")
        )
    };
    if !boxed(root) {
        return None;
    }
    let transparent = root
        .style
        .get("background-color")
        .and_then(|value| parse_color(value))
        .is_none_or(|color| color[3] == 0);
    let no_image = root
        .style
        .get("background-image")
        .is_none_or(|value| value == "none");
    let uncontained = |node: &oryn_common::v4::PaintNodeSnapshot| {
        node.style
            .get("contain")
            .is_none_or(|value| value == "none")
    };
    if transparent
        && no_image
        && uncontained(root)
        && let Some(body) = snapshot
            .nodes
            .iter()
            .find(|node| node.parent_id == Some(root.node_id) && node.tag == "body")
        && boxed(body)
        && uncontained(body)
    {
        return Some((root.node_id, body.node_id));
    }
    Some((root.node_id, root.node_id))
}
pub(super) fn transformed_ancestry(snapshot: &PaintSnapshot) -> std::collections::BTreeSet<u64> {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| (node.node_id, node))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut known = std::collections::BTreeMap::<u64, bool>::new();
    for node in &snapshot.nodes {
        let mut path = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut current = Some(node.node_id);
        let mut transformed = false;
        while let Some(id) = current {
            if let Some(value) = known.get(&id) {
                transformed = *value;
                break;
            }
            if !seen.insert(id) {
                break;
            }
            let Some(node) = nodes.get(&id) else {
                break;
            };
            path.push(id);
            let applicable = node.style.get("display").map(String::as_str) != Some("inline")
                || matches!(node.tag.as_str(), "img" | "video" | "canvas" | "iframe");
            if applicable
                && ["transform", "translate", "rotate", "scale"]
                    .iter()
                    .any(|key| node.style.get(*key).is_some_and(|value| value != "none"))
            {
                transformed = true;
                break;
            }
            current = node.parent_id;
        }
        for id in path {
            known.insert(id, transformed);
        }
    }
    known
        .into_iter()
        .filter_map(|(id, value)| value.then_some(id))
        .collect()
}
/// Reconstruct the hypothetical unfragmented background positioning box for
/// box-decoration-break:slice. Actual fragment clipping remains independent.
pub(super) fn slice_model(
    models: &[oryn_common::v4::BoxModel],
    index: usize,
    style: &std::collections::BTreeMap<String, String>,
) -> Option<oryn_common::v4::BoxModel> {
    if models.len() < 2
        || style
            .get("box-decoration-break")
            .is_some_and(|value| value == "clone")
    {
        return None;
    }
    let current = models.get(index)?;
    let mode = style
        .get("writing-mode")
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    let vertical = mode != "horizontal-tb";
    let rtl = style.get("direction").is_some_and(|value| value == "rtl");
    let block = !matches!(
        style.get("display").map(String::as_str).unwrap_or("inline"),
        "inline"
            | "ruby"
            | "ruby-base"
            | "ruby-text"
            | "ruby-base-container"
            | "ruby-text-container"
            | "contents"
    );
    let along_y = if block { !vertical } else { vertical };
    let reverse = if block {
        matches!(mode, "vertical-rl" | "sideways-rl")
    } else if mode == "sideways-lr" {
        !rtl
    } else {
        rtl
    };
    let extent = |model: &oryn_common::v4::BoxModel| {
        if along_y {
            model.border_box.height
        } else {
            model.border_box.width
        }
    };
    let total = models.iter().map(extent).sum::<f64>();
    let before = models[..index].iter().map(extent).sum::<f64>();
    if !total.is_finite() || !before.is_finite() {
        return None;
    }
    let mut rect = current.border_box;
    if along_y {
        rect.y = if reverse {
            rect.bottom() + before - total
        } else {
            rect.y - before
        };
        rect.height = total;
        rect.width = models
            .iter()
            .map(|model| model.border_box.width)
            .fold(0., f64::max);
    } else {
        rect.x = if reverse {
            rect.right() + before - total
        } else {
            rect.x - before
        };
        rect.width = total;
        rect.height = models
            .iter()
            .map(|model| model.border_box.height)
            .fold(0., f64::max);
    }
    let mut result = current.clone();
    for model in models {
        result.border.top = result.border.top.max(model.border.top);
        result.border.right = result.border.right.max(model.border.right);
        result.border.bottom = result.border.bottom.max(model.border.bottom);
        result.border.left = result.border.left.max(model.border.left);
        result.padding.top = result.padding.top.max(model.padding.top);
        result.padding.right = result.padding.right.max(model.padding.right);
        result.padding.bottom = result.padding.bottom.max(model.padding.bottom);
        result.padding.left = result.padding.left.max(model.padding.left);
    }
    let inset = |rect: CssRect, edge: oryn_common::v4::EdgeSizes| CssRect {
        x: rect.x + edge.left,
        y: rect.y + edge.top,
        width: (rect.width - edge.left - edge.right).max(0.),
        height: (rect.height - edge.top - edge.bottom).max(0.),
    };
    result.border_box = rect;
    result.padding_box = inset(rect, result.border);
    result.content_box = inset(result.padding_box, result.padding);
    Some(result)
}

pub(super) fn commands(
    snapshot: &PaintSnapshot,
    style: &std::collections::BTreeMap<String, String>,
    model: &oryn_common::v4::BoxModel,
    radii: [f64; 8],
    scroll: Option<&oryn_common::v4::PaintScrollGeometry>,
    transformed_ancestor: bool,
    canvas_clip: Option<CssRect>,
) -> Result<Vec<PaintCommand>, String> {
    commands_for_fragment(BackgroundFragmentInputs {
        snapshot,
        style,
        model,
        radii,
        scroll,
        transformed_ancestor,
        canvas_clip,
        fragment: None,
    })
}
pub(super) fn commands_for_fragment(
    params: BackgroundFragmentInputs<'_>,
) -> Result<Vec<PaintCommand>, String> {
    let BackgroundFragmentInputs {
        snapshot,
        style,
        model,
        radii,
        scroll,
        transformed_ancestor,
        canvas_clip,
        fragment,
    } = params;

    if snapshot.images.iter().any(|image| {
        image.url_aliases.len() > 16
            || image.url_aliases.iter().map(String::len).sum::<usize>() > 64 * 1024
    }) {
        return Err("captured image aliases exceed metadata budget".into());
    }
    let mut commands = Vec::new();
    let value = style
        .get("background-image")
        .map(String::as_str)
        .unwrap_or("none");
    let layers = gradients::split(value, ',').ok_or("invalid background layer list")?;
    if layers.len() > 256 {
        return Err("background exceeds 256 layers".into());
    }
    let font = style
        .get("font-size")
        .and_then(|value| value.strip_suffix("px"))
        .and_then(|value| value.parse().ok())
        .unwrap_or(16.);
    for (index, image) in layers.into_iter().enumerate().rev() {
        if image == "none" {
            continue;
        }
        let Some((selected, resolution)) =
            selected_image(image, snapshot.viewport.device_pixel_ratio.max(0.01))
        else {
            continue;
        };
        let image = selected.as_str();
        let attachment = list(style, "background-attachment", "scroll", index);
        let attachment = if attachment == "fixed" && transformed_ancestor {
            "scroll"
        } else {
            attachment
        };
        let mut origin = if attachment == "fixed" {
            CssRect {
                x: 0.,
                y: 0.,
                width: snapshot.viewport.width,
                height: snapshot.viewport.height,
            }
        } else {
            box_for(
                model,
                list(style, "background-origin", "padding-box", index),
            )
        };
        let clip_model = fragment.unwrap_or(model);
        let mut clip = canvas_clip.unwrap_or_else(|| {
            box_for(
                clip_model,
                list(style, "background-clip", "border-box", index),
            )
        });
        if attachment == "local"
            && let Some(scroll) = scroll
        {
            if ![
                scroll.offset_x,
                scroll.offset_y,
                scroll.scrollable.x,
                scroll.scrollable.y,
                scroll.scrollable.width,
                scroll.scrollable.height,
            ]
            .iter()
            .all(|v| v.is_finite())
                || scroll.scrollable.width < 0.
                || scroll.scrollable.height < 0.
            {
                return Err("invalid native background scroll geometry".into());
            }
            let right = scroll.scrollport.right() - origin.right();
            let bottom = scroll.scrollport.bottom() - origin.bottom();
            let left = origin.x - scroll.scrollport.x;
            let top = origin.y - scroll.scrollport.y;
            origin = CssRect {
                x: scroll.scrollable.x + left - scroll.offset_x,
                y: scroll.scrollable.y + top - scroll.offset_y,
                width: (scroll.scrollable.width - left - right).max(0.),
                height: (scroll.scrollable.height - top - bottom).max(0.),
            };
            // Local background ink is clipped at the stationary scrollport, while its
            // positioning rectangle follows the authoritative content translation.
            let right = clip.right().min(scroll.scrollport.right());
            let bottom = clip.bottom().min(scroll.scrollport.bottom());
            clip.x = clip.x.max(scroll.scrollport.x);
            clip.y = clip.y.max(scroll.scrollport.y);
            clip.width = (right - clip.x).max(0.);
            clip.height = (bottom - clip.y).max(0.);
        }
        let native = image_intrinsic(snapshot, image, 0)?
            .map(|(width, height)| (width / resolution, height / resolution));
        let size = list(style, "background-size", "auto", index);
        let parts = gradients::split(size, ' ')
            .ok_or("invalid background size")?
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let (mut width, mut height) = if matches!(size, "cover" | "contain") {
            if let Some((w, h)) = native {
                let sx = origin.width / w;
                let sy = origin.height / h;
                let scale = if size == "cover" {
                    sx.max(sy)
                } else {
                    sx.min(sy)
                };
                (w * scale, h * scale)
            } else {
                (origin.width, origin.height)
            }
        } else {
            let x = parts.first().copied().unwrap_or("auto");
            let y = parts.get(1).copied().unwrap_or("auto");
            let w = if x == "auto" {
                None
            } else {
                Some(
                    length(x, origin.width, font, snapshot.viewport)
                        .ok_or("invalid background width")?,
                )
            };
            let h = if y == "auto" {
                None
            } else {
                Some(
                    length(y, origin.height, font, snapshot.viewport)
                        .ok_or("invalid background height")?,
                )
            };
            match (w, h, native) {
                (Some(w), Some(h), _) => (w, h),
                (Some(w), None, Some((nw, nh))) => (w, w * nh / nw),
                (None, Some(h), Some((nw, nh))) => (h * nw / nh, h),
                (None, None, Some(size)) => size,
                (w, h, None) => (w.unwrap_or(origin.width), h.unwrap_or(origin.height)),
            }
        };
        if !width.is_finite() || !height.is_finite() || width < 0. || height < 0. {
            return Err("invalid background tile dimensions".into());
        }
        if width == 0. || height == 0. {
            continue;
        }
        let repeat = list(style, "background-repeat", "repeat", index);
        let words = repeat.split_whitespace().collect::<Vec<_>>();
        let (rx, ry) = match words.as_slice() {
            ["repeat-x"] => ("repeat", "no-repeat"),
            ["repeat-y"] => ("no-repeat", "repeat"),
            [x] => (*x, *x),
            [x, y] => (*x, *y),
            _ => return Err("invalid background-repeat".into()),
        };
        if rx == "round" {
            width = origin.width / (origin.width / width).round().max(1.);
            if parts.get(1).copied().unwrap_or("auto") == "auto"
                && ry != "round"
                && let Some((nw, nh)) = native
            {
                height = width * nh / nw;
            }
        }
        if ry == "round" {
            height = origin.height / (origin.height / height).round().max(1.);
            if parts.first().copied().unwrap_or("auto") == "auto"
                && rx != "round"
                && let Some((nw, nh)) = native
            {
                width = height * nw / nh;
            }
        }
        let (px, py) = positions(
            list(style, "background-position", "0% 0%", index),
            (origin.width - width, origin.height - height),
            font,
            snapshot.viewport,
        )
        .ok_or("unsupported background position")?;
        let axis = |repeat: &str,
                    start: f64,
                    extent: f64,
                    size: f64,
                    position: f64|
         -> Result<RepeatAxisPlacement, String> {
            match repeat {
                "no-repeat" => Ok((start + position, None, None)),
                "repeat" | "round" => Ok((start + position, Some(size), None)),
                "space" => {
                    let count = (extent / size).floor();
                    if count < 2. {
                        Ok((start + position, None, None))
                    } else {
                        Ok((
                            start,
                            Some((extent - size) / (count - 1.)),
                            Some([start, start + extent]),
                        ))
                    }
                }
                _ => Err("invalid background repetition".into()),
            }
        };
        let (x, step_x, limit_x) = axis(rx, origin.x, origin.width, width, px)?;
        let (y, step_y, limit_y) = axis(ry, origin.y, origin.height, height, py)?;
        let tile = BackgroundTile {
            rect: CssRect {
                x,
                y,
                width,
                height,
            },
            step_x,
            step_y,
            limit_x,
            limit_y,
        };
        let source = source_image(BackgroundImageInputs {
            snapshot,
            value: image,
            rect: tile.rect,
            color: style.get("color").map(String::as_str).unwrap_or("black"),
            font,
            rendering: ImageRendering::from_style(style.get("image-rendering")),
            environment: SvgEnvironment::for_style(style, tile.rect, snapshot.viewport),
            depth: 0,
        })?;
        let radii = if canvas_clip.is_some() {
            [0.; 8]
        } else {
            clip_radii(clip_model, clip, radii)
        };
        // A single fully covering tile keeps the existing native vector gradient
        // primitive; repeated/gapped tiles retain the explicit sampling contract.
        if canvas_clip.is_none()
            && list(style, "background-clip", "border-box", index) != "text"
            && clip.x >= tile.rect.x
            && clip.y >= tile.rect.y
            && clip.right() <= tile.rect.right()
            && clip.bottom() <= tile.rect.bottom()
        {
            match source {
                BackgroundSource::Gradient(BackgroundGradient::Linear(gradient)) => {
                    commands.push(PaintCommand::LinearGradient {
                        rect: clip,
                        gradient,
                        radii,
                    })
                }
                BackgroundSource::Gradient(gradient) => commands.push(PaintCommand::Gradient {
                    rect: clip,
                    gradient,
                    radii,
                }),
                source => commands.push(PaintCommand::Background {
                    rect: clip,
                    radii,
                    tile,
                    source,
                }),
            }
        } else {
            commands.push(PaintCommand::Background {
                rect: clip,
                radii,
                tile,
                source,
            });
        }
        if list(style, "background-clip", "border-box", index) == "text" {
            let command = commands
                .pop()
                .ok_or("missing text-clipped background layer")?;
            commands.push(PaintCommand::TextClip {
                owner: 0,
                fragment: None,
                commands: vec![command],
            });
        }
    }
    Ok(commands)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_sampling_preserves_gaps_without_tile_allocation() {
        let tile = BackgroundTile {
            rect: CssRect {
                x: 10.,
                y: 0.,
                width: 5.,
                height: 8.,
            },
            step_x: Some(10.),
            step_y: None,
            limit_x: None,
            limit_y: None,
        };
        assert_eq!(tile.map(22., 3.), Some((12., 3.)));
        assert!(tile.map(27., 3.).is_none());
        assert_eq!(tile.map(2., 3.), Some((12., 3.)));
        assert!(tile.map(22., 9.).is_none());
    }
    #[test]
    fn percentage_position_uses_remaining_space() {
        assert_eq!(
            positions(
                "100% 50%",
                (80., 40.),
                16.,
                oryn_common::v4::ViewportState::default()
            ),
            Some((80., 20.))
        );
        assert_eq!(
            positions(
                "right 5px bottom 10px",
                (80., 40.),
                16.,
                oryn_common::v4::ViewportState::default()
            ),
            Some((75., 30.))
        );
    }
    fn model() -> oryn_common::v4::BoxModel {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 106.,
            height: 106.,
        };
        oryn_common::v4::BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            border: Default::default(),
            padding: Default::default(),
            margin: Default::default(),
        }
    }
    #[test]
    fn sized_gradient_space_repeat_has_native_gap_and_clip() {
        // Pinned css/css-backgrounds/background-repeat-space-1a.html.
        let snapshot = crate::tests::empty_snapshot();
        let style = std::collections::BTreeMap::from([
            (
                "background-image".into(),
                "linear-gradient(to top left, red, green)".into(),
            ),
            ("background-size".into(), "32px 32px".into()),
            ("background-repeat".into(), "space".into()),
        ]);
        let commands = commands(&snapshot, &style, &model(), [0.; 8], None, false, None).unwrap();
        let PaintCommand::Background { tile, .. } = &commands[0] else {
            panic!("background command")
        };
        assert_eq!(tile.rect.width, 32.);
        assert_eq!(tile.step_x, Some(37.));
        assert!(tile.map(34., 5.).is_none());
        assert_eq!(tile.map(40., 5.), Some((3., 5.)));
    }
    #[test]
    fn negative_percentage_basis_preserves_pure_percentage_minmax() {
        assert_eq!(
            positions(
                "min(0%, 100%) max(0%, 100%)",
                (-100., -175.),
                16.,
                oryn_common::v4::ViewportState::default()
            ),
            Some((0., -175.))
        );
    }
    #[test]
    fn captured_redirect_alias_keeps_final_svg_base_and_original_lookup() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url_aliases:vec!["https://example.test/original.svg".into()],url:"https://example.test/final/image.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#.to_vec()});
        let style = std::collections::BTreeMap::from([
            (
                "background-image".into(),
                "url(\"https://example.test/original.svg\")".into(),
            ),
            ("background-repeat".into(), "no-repeat".into()),
        ]);
        let commands = commands(&snapshot, &style, &model(), [0.; 8], None, false, None).unwrap();
        let PaintCommand::Background { tile, source, .. } = &commands[0] else {
            panic!("background command")
        };
        assert_eq!((tile.rect.width, tile.rect.height), (20., 10.));
        assert!(matches!(
            source,
            BackgroundSource::Image {
                resource: 0,
                svg: true,
                ..
            }
        ));
    }

    #[test]
    fn image_set_selects_supported_resolution_without_fetching() {
        let value = r#"image-set("one.png" 1x, url(two.png) 2x type("image/png"), url(three.avif) 3x type("image/avif"))"#;
        assert_eq!(
            selected_image(value, 1.).unwrap(),
            ("url(\"one.png\")".into(), 1.)
        );
        assert_eq!(
            selected_image(value, 1.5).unwrap(),
            ("url(two.png)".into(), 2.)
        );
        assert_eq!(
            selected_image(value, 4.).unwrap(),
            ("url(two.png)".into(), 2.)
        );
    }

    #[test]
    fn local_background_positions_against_scrollable_area_and_clips_scrollport() {
        let snapshot = crate::tests::empty_snapshot();
        let style = std::collections::BTreeMap::from([
            (
                "background-image".into(),
                "linear-gradient(red, blue)".into(),
            ),
            ("background-attachment".into(), "local".into()),
            ("background-size".into(), "20px 20px".into()),
            ("background-repeat".into(), "no-repeat".into()),
            ("background-position".into(), "right bottom".into()),
        ]);
        let scroll = oryn_common::v4::PaintScrollGeometry {
            offset_x: 30.,
            offset_y: 80.,
            scrollport: model().padding_box,
            scrollable: CssRect {
                x: 0.,
                y: 0.,
                width: 200.,
                height: 300.,
            },
        };
        let commands = commands(
            &snapshot,
            &style,
            &model(),
            [0.; 8],
            Some(&scroll),
            false,
            None,
        )
        .unwrap();
        let PaintCommand::Background { rect, tile, .. } = &commands[0] else {
            panic!("background")
        };
        assert_eq!(*rect, model().padding_box);
        assert_eq!((tile.rect.x, tile.rect.y), (150., 200.));
    }

    #[test]
    fn three_component_positions_keep_side_specific_offsets() {
        let viewport = oryn_common::v4::ViewportState::default();
        for (value, expected) in [
            ("right 5px bottom", (75., 40.)),
            ("bottom right 5px", (75., 40.)),
            ("left top 10px", (0., 10.)),
            ("top 10px right", (80., 10.)),
        ] {
            assert_eq!(positions(value, (80., 40.), 16., viewport), Some(expected));
        }
    }

    #[test]
    fn transformed_fixed_background_matches_scroll_reference() {
        // Pinned css/css-backgrounds/background-attachment-fixed-inside-transform-1.html.
        let snapshot = crate::tests::empty_snapshot();
        let mut style = std::collections::BTreeMap::from([
            (
                "background-image".into(),
                "radial-gradient(blue, black)".into(),
            ),
            ("background-size".into(), "30px 30px".into()),
            ("background-position".into(), "20px 20px".into()),
            ("background-repeat".into(), "no-repeat".into()),
            ("background-attachment".into(), "fixed".into()),
        ]);
        let mut box_model = model();
        box_model.border_box.x = 200.;
        box_model.padding_box.x = 200.;
        box_model.content_box.x = 200.;
        let fixed = commands(&snapshot, &style, &box_model, [0.; 8], None, true, None).unwrap();
        style.insert("background-attachment".into(), "scroll".into());
        assert_eq!(
            fixed,
            commands(&snapshot, &style, &box_model, [0.; 8], None, false, None).unwrap()
        );
    }

    #[test]
    fn canvas_promotes_only_real_boxed_uncontained_body() {
        let mut snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 10.,
        };
        let root = crate::tests::paint_node(1, None, "html", rect);
        let mut body = crate::tests::paint_node(2, Some(1), "body", rect);
        body.style.insert("background-color".into(), "green".into());
        snapshot.nodes = vec![root, body];
        assert_eq!(canvas_source(&snapshot), Some((1, 2)));
        snapshot.nodes[1]
            .style
            .insert("display".into(), "contents".into());
        assert_eq!(canvas_source(&snapshot), Some((1, 1)));
        snapshot.nodes[1].style.remove("display");
        snapshot.nodes[0]
            .style
            .insert("background-color".into(), "red".into());
        assert_eq!(canvas_source(&snapshot), Some((1, 1)));
    }
    #[test]
    fn root_canvas_color_fills_viewport_beyond_short_body() {
        let mut snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 10.,
        };
        snapshot.nodes = vec![
            crate::tests::paint_node(1, None, "html", rect),
            crate::tests::paint_node(2, Some(1), "body", rect),
        ];
        snapshot.nodes[1]
            .style
            .insert("background-color".into(), "green".into());
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        fn green(commands: &[PaintCommand]) -> Vec<CssRect> {
            let mut found = Vec::new();
            for command in commands {
                match command {
                    PaintCommand::FillRect { rect, color } if *color == [0, 128, 0, 255] => {
                        found.push(*rect)
                    }
                    PaintCommand::Projected { commands, .. }
                    | PaintCommand::Opacity { commands, .. } => found.extend(green(commands)),
                    _ => {}
                }
            }
            found
        }
        assert_eq!(
            green(&scene.commands),
            vec![CssRect {
                x: 0.,
                y: 0.,
                width: 64.,
                height: 32.
            }]
        );
    }
    #[test]
    fn identity_transform_still_changes_fixed_attachment_in_descendants() {
        let mut snapshot = crate::tests::empty_snapshot();
        let rect = model().border_box;
        let mut parent = crate::tests::paint_node(1, None, "div", rect);
        parent
            .style
            .insert("transform".into(), "matrix(1, 0, 0, 1, 0, 0)".into());
        snapshot.nodes = vec![parent, crate::tests::paint_node(2, Some(1), "div", rect)];
        let transformed = transformed_ancestry(&snapshot);
        assert!(transformed.contains(&1) && transformed.contains(&2));
    }

    #[test]
    fn transformed_root_canvas_paints_inverse_viewport_extent() {
        let mut node = crate::tests::paint_node(1, None, "html", model().border_box);
        node.projection.as_mut().unwrap().matrix = [
            2., 0., 0., 0., 0., 2., 0., 0., 0., 0., 1., 0., 20., 10., 0., 1.,
        ];
        let rect = canvas_rect(
            &node,
            oryn_common::v4::ViewportState {
                width: 100.,
                height: 80.,
                device_pixel_ratio: 1.,
            },
        )
        .unwrap(); // Inverting a floating-point projective matrix may differ by one ulp.
        // Keep every expected coordinate and a tolerance far below one CSS pixel.
        for (actual, expected) in [
            (rect.x, -10.),
            (rect.y, -5.),
            (rect.width, 50.),
            (rect.height, 40.),
        ] {
            assert!(
                actual.is_finite() && (actual - expected).abs() <= 1e-12,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn camera_crossing_root_canvas_samples_unbounded_native_plane() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut root = crate::tests::paint_node(
            1,
            None,
            "html",
            CssRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
        );
        root.style.insert("background-color".into(), "green".into());
        root.style.insert(
            "background-image".into(),
            "linear-gradient(red, red)".into(),
        );
        root.style
            .insert("background-size".into(), "2px 2px".into());
        root.style
            .insert("background-repeat".into(), "repeat".into());
        root.projection.as_mut().unwrap().matrix[3] = 0.05;
        snapshot.nodes = vec![root];
        assert!(canvas_rect(&snapshot.nodes[0], snapshot.viewport).is_none());
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        assert!(scene.diagnostics.is_empty());
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let at = |x: usize| {
            &pixels[(8 * scene.width as usize + x) * 4..(8 * scene.width as usize + x) * 4 + 4]
        };
        assert_eq!(at(15), [255, 0, 0, 255]);
        assert_eq!(at(30), [255, 255, 255, 255]);
        let tree = crate::print::vector::tree(&snapshot, &scene, options).unwrap();
        assert!(
            tree.to_string(&resvg::usvg::WriteOptions::default())
                .contains("data:image/png")
        );
    }

    #[test]
    fn sliced_inline_background_continues_across_physical_lines() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut node = crate::tests::paint_node(
            1,
            None,
            "span",
            CssRect {
                x: 0.,
                y: 0.,
                width: 30.,
                height: 20.,
            },
        );
        node.style.insert("display".into(), "inline".into());
        node.style.insert(
            "background-image".into(),
            "linear-gradient(to right, red 50%, blue 50%)".into(),
        );
        node.style
            .insert("background-repeat".into(), "no-repeat".into());
        let base = node.projection.as_ref().unwrap().box_model.clone();
        let fragments = [
            CssRect {
                x: 0.,
                y: 0.,
                width: 30.,
                height: 10.,
            },
            CssRect {
                x: 0.,
                y: 10.,
                width: 20.,
                height: 10.,
            },
        ]
        .map(|rect| oryn_common::v4::BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            ..base.clone()
        });
        node.projection.as_mut().unwrap().fragment_boxes = fragments.to_vec();
        let virtual_box = slice_model(&fragments, 1, &node.style).unwrap();
        assert_eq!(virtual_box.border_box.x, -30.);
        assert_eq!(virtual_box.border_box.width, 50.);
        snapshot.nodes.push(node);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let at = |x: usize, y: usize| {
            &pixels[(y * scene.width as usize + x) * 4..(y * scene.width as usize + x) * 4 + 4]
        };
        assert_eq!(at(5, 5), [255, 0, 0, 255]);
        assert_eq!(at(5, 15), [0, 0, 255, 255]);
        snapshot.nodes[0]
            .style
            .insert("box-decoration-break".into(), "clone".into());
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let offset = (15 * scene.width as usize + 5) * 4;
        assert_eq!(&pixels[offset..offset + 4], &[255, 0, 0, 255]);
    }
    #[test]
    fn sliced_vertical_reverse_inline_origin_tracks_source_order() {
        let mut a = model();
        a.border_box = CssRect {
            x: 20.,
            y: 10.,
            width: 10.,
            height: 30.,
        };
        let mut b = model();
        b.border_box = CssRect {
            x: 10.,
            y: 20.,
            width: 10.,
            height: 20.,
        };
        let style = std::collections::BTreeMap::from([
            ("display".into(), "inline".into()),
            ("writing-mode".into(), "vertical-rl".into()),
            ("direction".into(), "rtl".into()),
        ]);
        let full = slice_model(&[a, b], 1, &style).unwrap();
        assert_eq!(full.border_box.y, 20.);
        assert_eq!(full.border_box.height, 50.);
    }

    #[test]
    fn image_set_matches_native_raster_codec_support() {
        for mime in ["image/gif", "image/webp", "image/bmp"] {
            let source = format!("image-set(url(native) 1x type(\"{mime}\"))");
            assert_eq!(selected_image(&source, 1.).unwrap().0, "url(native)");
        }
    }

    #[test]
    fn native_supported_raster_codecs_render_without_js_feature_unification() {
        for (format, mime) in [
            (image::ImageFormat::Gif, "image/gif"),
            (image::ImageFormat::WebP, "image/webp"),
            (image::ImageFormat::Bmp, "image/bmp"),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([255, 0, 0, 255]),
            ))
            .write_to(&mut bytes, format)
            .unwrap();
            let mut snapshot = crate::tests::empty_snapshot();
            snapshot.images.push(oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                svg_instances: Vec::new(),
                content_type: None,
                url: "https://image.test/native".into(),
                url_aliases: Vec::new(),
                media_type: mime.into(),
                bytes: bytes.into_inner(),
            });
            let mut node = crate::tests::paint_node(
                1,
                None,
                "img",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 8.,
                    height: 8.,
                },
            );
            node.image_resource = Some(0);
            snapshot.nodes.push(node);
            let options = RenderOptions::default();
            let scene = build_display_list(&snapshot, options).unwrap();
            let pixels = rasterize(&snapshot, &scene, options).unwrap();
            let offset = (4 * scene.width as usize + 4) * 4;
            assert_eq!(&pixels[offset..offset + 4], &[255, 0, 0, 255], "{mime}");
        }
    }

    #[test]
    fn cross_fade_percentages_fill_omissions_and_normalize_overflow() {
        assert_eq!(
            fade_components("cross-fade(20% red, blue, green)")
                .unwrap()
                .unwrap(),
            vec![(0.2, "red"), (0.4, "blue"), (0.4, "green")]
        );
        assert_eq!(
            fade_components("cross-fade(80% red, 80% blue)")
                .unwrap()
                .unwrap(),
            vec![(0.5, "red"), (0.5, "blue")]
        );
        assert!(fade_components("cross-fade(120% red, blue)").is_err());
    }
    #[test]
    fn native_cross_fade_blends_images_in_premultiplied_space() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 20.,
            },
        );
        node.style.insert(
            "background-image".into(),
            "cross-fade(50% linear-gradient(red, red), 50% linear-gradient(blue, blue))".into(),
        );
        snapshot.nodes.push(node);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        assert!(scene.diagnostics.is_empty());
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let offset = (5 * scene.width as usize + 5) * 4;
        assert_eq!(&pixels[offset..offset + 4], &[128, 0, 128, 255]);
    }
}
