//! Optional deterministic CPU renderer for Oryn paint snapshots.
//!
//! This crate owns display lists, decoded images and raster buffers.  It never
//! owns page state and performs no filesystem or network access.

mod drag_image;
mod paint_timing;
pub use paint_timing::{scene_frame_is_presented, scene_has_contentful_paint};
mod filters;
mod masks;
pub use drag_image::{render_drag_png, render_drag_resource};
pub use masks::{MaskComposite, MaskLayer};
mod backgrounds;
mod borders;
mod frames;
mod gradients;
mod projected;
mod scrollbar;
pub use backgrounds::{BackgroundSource, BackgroundTile};
mod decorations;
mod shadows;
mod text_boxes;
pub use shadows::BoxShadow;
mod svg;
mod svg_environment;
pub use svg_environment::SvgEnvironment;
mod builtin_image;
mod svg_coverage;
mod svg_foreign;
mod svg_instances;
mod svg_secure_foreign;
mod svg_text;
pub use gradients::{BackgroundGradient, GradientStop, LinearGradient};
mod print;
pub use print::render_print_pdf;
mod backdrop;
mod controls;
mod glyph_outline;
mod outline;
mod outline_contour;
pub mod stacking;

use oryn_common::color::parse_color;
use std::io::Write;

use flate2::{Compression, write::ZlibEncoder};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, ImageEncoder, ImageFormat};
use oryn_common::v4::{CssRect, GeometryAvailability, PaintSnapshot, PaintTextFragment};
use serde::{Deserialize, Serialize};

/// Private-native consumers can inspect a bounded captured PNG without adding
/// a second decoder or exposing tainted pixels to page script.
pub fn captured_png_has_ink(bytes: &[u8], width: u32, height: u32) -> Result<bool, RenderError> {
    if width == 0
        || height == 0
        || (width as usize)
            .checked_mul(height as usize)
            .is_none_or(|pixels| pixels > 16_000_000)
        || bytes.len() > 8 * 1024 * 1024
        || bytes.get(16..20) != Some(width.to_be_bytes().as_slice())
        || bytes.get(20..24) != Some(height.to_be_bytes().as_slice())
    {
        return Err(RenderError::Encode("invalid native coverage PNG".into()));
    }
    let pixels = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|error| RenderError::Encode(error.to_string()))?
        .to_rgba8();
    if pixels.width() != width || pixels.height() != height {
        return Err(RenderError::Encode(
            "native coverage dimensions changed".into(),
        ));
    }
    Ok(pixels.pixels().any(|pixel| pixel.0[3] != 0))
}

pub const RENDER_CONTRACT_VERSION: u16 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererCapabilities {
    pub cpu_reference: bool,
    pub text_paint: bool,
    pub png_images: bool,
    pub jpeg_images: bool,
    pub screenshot_png: bool,
    pub single_page_pdf: bool,
    pub svg_images: bool,
    pub gradients: bool,
    pub gpu: bool,
}

pub const fn capabilities() -> RendererCapabilities {
    RendererCapabilities {
        cpu_reference: true,
        text_paint: true,
        png_images: true,
        jpeg_images: true,
        screenshot_png: true,
        single_page_pdf: true,
        svg_images: true,
        gradients: true,
        gpu: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderOptions {
    pub scale: f64,
    pub max_pixels: usize,
    pub background: [u8; 4],
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            max_pixels: 64 * 1024 * 1024,
            background: [255, 255, 255, 255],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayList {
    pub contract_version: u16,
    pub width: u32,
    pub height: u32,
    pub commands: Vec<PaintCommand>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageRendering {
    #[default]
    Auto,
    CrispEdges,
    Pixelated,
}

impl ImageRendering {
    fn from_style(value: Option<&String>) -> Self {
        match value.map(String::as_str) {
            Some("crisp-edges") => Self::CrispEdges,
            Some("pixelated") => Self::Pixelated,
            _ => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaintCommand {
    EmbeddedDocument {
        rect: CssRect,
        frame: usize,
    },
    Canvas {
        color: [u8; 4],
        layers: Vec<(BackgroundTile, BackgroundSource)>,
    },
    TextClip {
        owner: u64,
        fragment: Option<CssRect>,
        commands: Vec<PaintCommand>,
    },
    Opacity {
        opacity: f64,
        commands: Vec<PaintCommand>,
    },
    BackdropFilter {
        #[serde(default)]
        environment: SvgEnvironment,
        source: String,
        current_color: String,
        plane: [f64; 16],
        rect: CssRect,
        radii: [f64; 8],
    },
    Mask {
        layers: Vec<MaskLayer>,
        commands: Vec<PaintCommand>,
    },
    Filter {
        #[serde(default)]
        environment: SvgEnvironment,
        #[serde(default)]
        reference_box: Option<CssRect>,
        source: String,
        current_color: String,
        plane: [f64; 16],
        commands: Vec<PaintCommand>,
    },
    Projected {
        matrix: [f64; 16],
        #[serde(default)]
        clips: Vec<oryn_common::v4::PaintClip>,
        commands: Vec<PaintCommand>,
    },
    Background {
        rect: CssRect,
        radii: [f64; 8],
        tile: BackgroundTile,
        source: BackgroundSource,
    },
    BoxShadow {
        rect: CssRect,
        shadow: BoxShadow,
        radii: [f64; 8],
    },
    FillRoundedRect {
        rect: CssRect,
        color: [u8; 4],
        radii: [f64; 8],
    },
    LinearGradient {
        rect: CssRect,
        gradient: LinearGradient,
        radii: [f64; 8],
    },
    Gradient {
        rect: CssRect,
        gradient: BackgroundGradient,
        radii: [f64; 8],
    },
    FillRect {
        rect: CssRect,
        color: [u8; 4],
    },
    SvgMask {
        rect: CssRect,
        resource: usize,
        fragment: String,
        mode: String,
        #[serde(default)]
        environment: SvgEnvironment,
    },
    SvgImage {
        rect: CssRect,
        clip: Option<CssRect>,
        resource: usize,
        #[serde(default)]
        environment: SvgEnvironment,
    },
    BuiltinImage {
        rect: CssRect,
        clip: Option<CssRect>,
        image: oryn_common::builtin_image::BuiltinImage,
    },
    BorderEdges {
        rect: CssRect,
        edges: [oryn_common::v4::PaintBorderEdge; 4],
        radii: [f64; 8],
    },
    Border {
        rect: CssRect,
        widths: [f64; 4],
        color: [u8; 4],
    },
    Text {
        fragment: PaintTextFragment,
        color: [u8; 4],
    },
    Image {
        rect: CssRect,
        resource: usize,
        #[serde(default)]
        rendering: ImageRendering,
        #[serde(default)]
        clip: Option<CssRect>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedArtifact {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("invalid print snapshot: {0}")]
    InvalidPrint(String),
    #[error("invalid render scale {0}")]
    InvalidScale(f64),
    #[error("raster would contain {actual} pixels, limit is {limit}")]
    PixelLimit { actual: usize, limit: usize },
    #[error("image {url} could not be decoded: {detail}")]
    ImageDecode { url: String, detail: String },
    #[error("font could not be rendered: {0}")]
    Font(String),
    #[error("output encoding failed: {0}")]
    Encode(String),
}

fn clear_visual_viewports(snapshot: &mut PaintSnapshot) {
    let mut pending = vec![snapshot];
    while let Some(document) = pending.pop() {
        document.visual_viewport = None;
        pending.extend(
            document
                .frames
                .iter_mut()
                .map(|frame| frame.snapshot.as_mut()),
        );
    }
}

pub fn build_display_list(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<DisplayList, RenderError> {
    frames::validate(snapshot)?;
    let (width, height) = dimensions(snapshot, options)?;
    let mut commands = Vec::new();
    let mut diagnostics = snapshot.diagnostics.clone();
    commands.push(PaintCommand::FillRect {
        rect: CssRect {
            x: 0.0,
            y: 0.0,
            width: snapshot.viewport.width,
            height: snapshot.viewport.height,
        },
        color: options.background,
    });
    let transformed_backgrounds = backgrounds::transformed_ancestry(snapshot);
    let canvas_source = backgrounds::canvas_source(snapshot);
    let canvas_style = canvas_source
        .and_then(|(_, source)| snapshot.nodes.iter().find(|node| node.node_id == source))
        .map(|node| &node.style);
    let option_highlights = controls::option_highlights(snapshot);
    let mut own_commands = std::collections::BTreeMap::new();
    let mut outlines = std::collections::BTreeMap::new();
    let mut backgrounds = std::collections::BTreeMap::new();
    for node in &snapshot.nodes {
        if node.geometry.availability != GeometryAvailability::Available
            || (canvas_source.is_none_or(|(root, _)| root != node.node_id)
                && !node.geometry.in_viewport
                && !node
                    .style
                    .get("outline-style")
                    .is_some_and(|value| !matches!(value.as_str(), "none" | "hidden"))
                && !node
                    .style
                    .get("box-shadow")
                    .is_some_and(|value| value != "none")
                && !node
                    .projection
                    .as_ref()
                    .map(|projection| projection.paint_intersects_viewport(snapshot.viewport))
                    .unwrap_or_else(|| {
                        node.text.iter().any(|fragment| {
                            fragment.rect.intersects(CssRect {
                                x: 0.,
                                y: 0.,
                                width: snapshot.viewport.width,
                                height: snapshot.viewport.height,
                            })
                        })
                    }))
        {
            continue;
        }
        let Some(model) = node
            .projection
            .as_ref()
            .map(|p| &p.box_model)
            .or(node.geometry.box_model.as_ref())
        else {
            continue;
        };
        let canvas = canvas_source.is_some_and(|(root, _)| root == node.node_id);
        let promoted =
            canvas_source.is_some_and(|(root, source)| root != source && source == node.node_id);
        let background_style = if canvas {
            canvas_style.unwrap_or(&node.style)
        } else {
            &node.style
        };
        let canvas_bounds = if canvas {
            backgrounds::canvas_rect(node, snapshot.viewport)
        } else {
            None
        };
        let infinite_canvas = canvas && canvas_bounds.is_none();
        let canvas_clip = if canvas {
            Some(canvas_bounds.unwrap_or(CssRect {
                x: 0.,
                y: 0.,
                width: snapshot.viewport.width,
                height: snapshot.viewport.height,
            }))
        } else {
            None
        };
        let command_start = commands.len();
        let models = node
            .projection
            .as_ref()
            .filter(|projection| !projection.fragment_boxes.is_empty())
            .map(|projection| projection.fragment_boxes.as_slice())
            .unwrap_or_else(|| std::slice::from_ref(model));
        for (fragment_index, model) in models.iter().enumerate() {
            let fragment_radii = |rect| {
                let mut radii = borders::radii(&node.style, rect);
                if models.len() > 1
                    && node
                        .style
                        .get("box-decoration-break")
                        .map(String::as_str)
                        .unwrap_or("slice")
                        != "clone"
                {
                    let mode = node
                        .style
                        .get("writing-mode")
                        .map(String::as_str)
                        .unwrap_or("horizontal-tb");
                    let vertical = mode != "horizontal-tb";
                    let rtl = node
                        .style
                        .get("direction")
                        .is_some_and(|direction| direction == "rtl");
                    let block = !matches!(
                        node.style
                            .get("display")
                            .map(String::as_str)
                            .unwrap_or("inline"),
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
                    let first = fragment_index == 0;
                    let last = fragment_index + 1 == models.len();
                    let before = if reverse { last } else { first };
                    let after = if reverse { first } else { last };
                    if !before {
                        for index in if along_y { [0, 1, 2, 3] } else { [0, 1, 6, 7] } {
                            radii[index] = 0.;
                        }
                    }
                    if !after {
                        for index in if along_y { [4, 5, 6, 7] } else { [2, 3, 4, 5] } {
                            radii[index] = 0.;
                        }
                    }
                }
                radii
            };
            let box_command_start = commands.len();
            let shadows = if let Some(value) = node
                .style
                .get("box-shadow")
                .filter(|value| value.as_str() != "none")
            {
                let font = node
                    .style
                    .get("font-size")
                    .and_then(|value| value.strip_suffix("px"))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(16.);
                match shadows::parse(
                    value,
                    node.style
                        .get("color")
                        .map(String::as_str)
                        .unwrap_or("black"),
                    font,
                ) {
                    Some(shadows) => shadows,
                    None => {
                        diagnostics.push(format!(
                            "node {} box-shadow syntax is unsupported",
                            node.node_id
                        ));
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            for shadow in shadows
                .iter()
                .rev()
                .filter(|shadow| !shadow.inset && shadow.color[3] > 0)
            {
                commands.push(PaintCommand::BoxShadow {
                    rect: model.border_box,
                    shadow: shadow.clone(),
                    radii: fragment_radii(model.border_box),
                });
            }

            let color_start = commands.len();
            if let Some(color) = background_style
                .get("background-color")
                .and_then(|value| parse_color(value))
                && color[3] != 0
                && !promoted
                && !infinite_canvas
            {
                let (rect, radii) = if let Some(canvas) = canvas_clip {
                    (canvas, [0.; 8])
                } else {
                    backgrounds::color_clip(
                        background_style,
                        model,
                        fragment_radii(model.border_box),
                    )
                };
                if radii.iter().any(|v| *v > 0.) {
                    commands.push(PaintCommand::FillRoundedRect { rect, color, radii });
                } else {
                    commands.push(PaintCommand::FillRect { rect, color });
                }
            }
            if backgrounds::color_uses_text(background_style) && commands.len() > color_start {
                let color = commands.split_off(color_start);
                commands.push(PaintCommand::TextClip {
                    owner: node.node_id,
                    fragment: (models.len() > 1).then_some(model.border_box),
                    commands: color,
                });
            }
            let sliced_background = if canvas {
                None
            } else {
                backgrounds::slice_model(models, fragment_index, &node.style)
            };
            if !promoted {
                match backgrounds::commands_for_fragment(
                    snapshot,
                    background_style,
                    sliced_background.as_ref().unwrap_or(model),
                    fragment_radii(model.border_box),
                    node.projection
                        .as_ref()
                        .and_then(|projection| projection.scroll.as_ref()),
                    !canvas && transformed_backgrounds.contains(&node.node_id),
                    canvas_clip,
                    sliced_background.as_ref().map(|_| model),
                ) {
                    Ok(mut background) => {
                        for command in &mut background {
                            if let PaintCommand::TextClip {
                                owner, fragment, ..
                            } = command
                            {
                                *owner = node.node_id;
                                *fragment = (models.len() > 1).then_some(model.border_box);
                            }
                        }
                        if infinite_canvas {
                            commands.extend(backgrounds::canvas_commands(
                                background,
                                background_style,
                                node.node_id,
                            ));
                        } else {
                            commands.extend(background);
                        }
                    }
                    Err(error) => diagnostics.push(format!("node {}: {error}", node.node_id)),
                }
            }
            for shadow in shadows
                .iter()
                .rev()
                .filter(|shadow| shadow.inset && shadow.color[3] > 0)
            {
                let mut radii = fragment_radii(model.border_box);
                let widths = [
                    model.border.left,
                    model.border.top,
                    model.border.right,
                    model.border.top,
                    model.border.right,
                    model.border.bottom,
                    model.border.left,
                    model.border.bottom,
                ];
                for (i, radius) in radii.iter_mut().enumerate() {
                    *radius = (*radius - widths[i]).max(0.);
                }
                commands.push(PaintCommand::BoxShadow {
                    rect: model.padding_box,
                    shadow: shadow.clone(),
                    radii,
                });
            }
            let mut background = commands.split_off(box_command_start);
            if !background.is_empty() {
                if let Some(projection) = &node.projection {
                    background = vec![PaintCommand::Projected {
                        matrix: projection.matrix,
                        clips: projection.clips.clone(),
                        commands: background,
                    }];
                }
                backgrounds
                    .entry(node.node_id)
                    .or_insert_with(Vec::new)
                    .extend(background);
            }
            let widths = [
                model.border.top,
                model.border.right,
                model.border.bottom,
                model.border.left,
            ];
            let resolved = node
                .projection
                .as_ref()
                .and_then(|p| p.border_edges.clone());
            if resolved.is_some()
                || ["top", "right", "bottom", "left"]
                    .iter()
                    .any(|side| node.style.contains_key(&format!("border-{side}-color")))
            {
                let edges = resolved.unwrap_or_else(|| {
                    std::array::from_fn(|i| {
                        let side = ["top", "right", "bottom", "left"][i];
                        oryn_common::v4::PaintBorderEdge {
                            width: widths[i],
                            color: node
                                .style
                                .get(&format!("border-{side}-color"))
                                .filter(|v| v.as_str() != "currentcolor")
                                .or_else(|| node.style.get("color"))
                                .cloned()
                                .unwrap_or_else(|| "black".into()),
                            style: node
                                .style
                                .get(&format!("border-{side}-style"))
                                .cloned()
                                .unwrap_or_else(|| "solid".into()),
                        }
                    })
                });
                if edges
                    .iter()
                    .any(|e| e.width > 0. && !matches!(e.style.as_str(), "none" | "hidden"))
                {
                    commands.push(PaintCommand::BorderEdges {
                        rect: model.border_box,
                        radii: fragment_radii(model.border_box),
                        edges,
                    });
                }
            } else if widths.iter().any(|width| *width > 0.0) {
                let color = node
                    .style
                    .get("border-color")
                    .or_else(|| node.style.get("color"))
                    .and_then(|value| parse_color(value))
                    .unwrap_or([0, 0, 0, 255]);
                commands.push(PaintCommand::Border {
                    rect: model.border_box,
                    widths,
                    color,
                });
            }
            if let Some(projection) = node.projection.as_ref().filter(|_| fragment_index == 0) {
                for segment in &projection.border_segments {
                    if segment.edge.width <= 0.
                        || matches!(segment.edge.style.as_str(), "none" | "hidden")
                    {
                        continue;
                    }
                    let mut edges = std::array::from_fn(|_| oryn_common::v4::PaintBorderEdge {
                        width: 0.,
                        color: "transparent".into(),
                        style: "none".into(),
                    });
                    edges[if segment.horizontal { 0 } else { 3 }] = segment.edge.clone();
                    commands.push(PaintCommand::BorderEdges {
                        rect: segment.rect,
                        edges,
                        radii: [0.; 8],
                    });
                }
            }
            if node.tag == "option" {
                if let Some(color) = option_highlights.get(&node.node_id) {
                    commands.push(PaintCommand::FillRect {
                        rect: model.padding_box,
                        color: *color,
                    });
                }
            }
            if fragment_index == 0 {
                match controls::commands(node, model) {
                    Ok(control) => commands.extend(control),
                    Err(error) => diagnostics.push(error.to_string()),
                }
            }
            if let Some(image) = node.builtin_image.as_ref().filter(|_| fragment_index == 0) {
                match image.svg() {
                    Ok(_) => commands.push(PaintCommand::BuiltinImage {
                        rect: model.content_box,
                        clip: Some(model.content_box),
                        image: image.clone(),
                    }),
                    Err(error) => diagnostics.push(error),
                }
            }
            if let Some(resource) = node.image_resource.filter(|_| fragment_index == 0) {
                match snapshot.images.get(resource) {
                    Some(image) if image.media_type == "image/svg+xml" => {
                        match svg::parse_with_environment(
                            image,
                            snapshot,
                            SvgEnvironment::for_style(
                                &node.style,
                                model.content_box,
                                snapshot.viewport,
                            ),
                        ) {
                            Ok(tree) => commands.push(PaintCommand::SvgImage {
                                rect: image_placement(
                                    model.content_box,
                                    tree.size().width().ceil() as u32,
                                    tree.size().height().ceil() as u32,
                                    &node.style,
                                    snapshot.viewport,
                                ),
                                clip: Some(model.content_box),
                                resource,
                                environment: SvgEnvironment::for_style(
                                    &node.style,
                                    image_placement(
                                        model.content_box,
                                        tree.size().width().ceil() as u32,
                                        tree.size().height().ceil() as u32,
                                        &node.style,
                                        snapshot.viewport,
                                    ),
                                    snapshot.viewport,
                                ),
                            }),
                            Err(error) => diagnostics.push(error.to_string()),
                        }
                    }
                    Some(image) => match decode_image(image) {
                        Ok(image) => commands.push(PaintCommand::Image {
                            rect: image_placement(
                                model.content_box,
                                image.width(),
                                image.height(),
                                &node.style,
                                snapshot.viewport,
                            ),
                            clip: Some(model.content_box),
                            resource,
                            rendering: ImageRendering::from_style(
                                node.style.get("image-rendering"),
                            ),
                        }),
                        Err(error) => diagnostics.push(error.to_string()),
                    },
                    None => diagnostics.push(format!(
                        "node {} references missing image resource {}",
                        node.node_id, resource
                    )),
                }
            }
            if fragment_index == 0 {
                if let Some(frame) = snapshot
                    .frames
                    .iter()
                    .position(|frame| frame.owner_node == node.node_id)
                {
                    commands.push(PaintCommand::EmbeddedDocument {
                        rect: model.content_box,
                        frame,
                    });
                }
            }
            if let Some(projection) = &node.projection {
                let local = commands.split_off(box_command_start);
                if !local.is_empty() {
                    commands.push(PaintCommand::Projected {
                        matrix: projection.matrix,
                        clips: projection.clips.clone(),
                        commands: local,
                    });
                }
            }
        }
        let text_start = commands.len();
        for fragment in node
            .projection
            .as_ref()
            .map(|p| &p.text)
            .unwrap_or(&node.text)
        {
            if fragment.glyph_runs.is_empty()
                && !fragment
                    .font_family
                    .to_ascii_lowercase()
                    .contains(&oryn_layout::DEFAULT_FONT_FAMILY.to_ascii_lowercase())
            {
                diagnostics.push(format!(
                    "font family {:?} is painted with deterministic {} fallback",
                    fragment.font_family,
                    oryn_layout::DEFAULT_FONT_FAMILY
                ));
            }
            let color = parse_color(&fragment.color).unwrap_or([0, 0, 0, 255]);
            commands.push(PaintCommand::Text {
                fragment: fragment.clone(),
                color,
            });
        }
        if let Some(projection) = &node.projection {
            let local = commands.split_off(text_start);
            if !local.is_empty() {
                let clips = if projection.text_matrix.is_some() {
                    &projection.text_clips
                } else {
                    &projection.clips
                };
                commands.push(PaintCommand::Projected {
                    matrix: projection.text_matrix.unwrap_or(projection.matrix),
                    clips: clips.clone(),
                    commands: local,
                });
            }
        }
        let chrome = scrollbar::commands(node);
        if !chrome.is_empty() {
            if let Some(projection) = &node.projection {
                commands.push(PaintCommand::Projected {
                    matrix: projection.matrix,
                    clips: projection.clips.clone(),
                    commands: chrome,
                });
            }
        }
        own_commands.insert(node.node_id, commands.split_off(command_start));
        outlines.insert(node.node_id, outline::commands(node)?);
    }
    let backdrops = backdrop::commands(snapshot, options, &mut diagnostics)?;
    stacking::append(
        snapshot,
        own_commands,
        backgrounds,
        outlines,
        backdrops,
        &mut commands,
    )?;
    if let Some(visual) = snapshot.visual_viewport {
        if !visual.is_valid()
            || (visual.layout_width - snapshot.viewport.width).abs() > 0.001
            || (visual.layout_height - snapshot.viewport.height).abs() > 0.001
        {
            return Err(RenderError::Encode(
                "invalid or stale native visual viewport capture".into(),
            ));
        }
        let port = visual.content_rect();
        let origin = visual
            .layout_to_visual(0., 0.)
            .ok_or_else(|| RenderError::Encode("nonfinite visual viewport origin".into()))?;
        let matrix = [
            visual.scale,
            0.,
            0.,
            0.,
            0.,
            visual.scale,
            0.,
            0.,
            0.,
            0.,
            1.,
            0.,
            origin.0,
            origin.1,
            0.,
            1.,
        ];
        if matrix.iter().any(|value| !value.is_finite()) {
            return Err(RenderError::Encode(
                "visual viewport transform exceeds finite coordinates".into(),
            ));
        }
        let clips = visual
            .scrollport
            .map(|_| oryn_common::v4::PaintClip {
                shape: None,
                owner: None,
                scroll_x: false,
                scroll_y: false,
                matrix: [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ],
                rect: port,
                clip_x: true,
                clip_y: true,
            })
            .into_iter()
            .collect();
        let content = commands.split_off(1);
        commands.push(PaintCommand::Projected {
            matrix,
            clips,
            commands: content,
        });
    }
    commands.extend(scrollbar::viewport_commands(snapshot));
    Ok(DisplayList {
        contract_version: RENDER_CONTRACT_VERSION,
        width,
        height,
        commands,
        diagnostics,
    })
}

pub fn render_png(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let scene = build_display_list(snapshot, options)?;
    let pixels = rasterize(snapshot, &scene, options)?;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &pixels,
            scene.width,
            scene.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    Ok(RenderedArtifact {
        bytes,
        width: scene.width,
        height: scene.height,
        diagnostics: scene.diagnostics,
    })
}

pub fn render_pdf(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let scene = build_display_list(snapshot, options)?;
    let rgba = rasterize(snapshot, &scene, options)?;
    let rgb = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect::<Vec<_>>();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(&rgb)
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    let bytes = single_page_pdf(scene.width, scene.height, &compressed);
    Ok(RenderedArtifact {
        bytes,
        width: scene.width,
        height: scene.height,
        diagnostics: scene.diagnostics,
    })
}

fn dimensions(snapshot: &PaintSnapshot, options: RenderOptions) -> Result<(u32, u32), RenderError> {
    if !options.scale.is_finite() || !(0.1..=4.0).contains(&options.scale) {
        return Err(RenderError::InvalidScale(options.scale));
    }
    let width = (snapshot.viewport.width * options.scale).ceil().max(1.0) as u32;
    let height = (snapshot.viewport.height * options.scale).ceil().max(1.0) as u32;
    let pixels = width as usize * height as usize;
    if pixels > options.max_pixels {
        return Err(RenderError::PixelLimit {
            actual: pixels,
            limit: options.max_pixels,
        });
    }
    Ok((width, height))
}

pub(crate) fn has_backdrop(commands: &[PaintCommand]) -> bool {
    commands.iter().any(|command| match command {
        PaintCommand::BackdropFilter { .. } => true,
        PaintCommand::Mask { commands, .. }
        | PaintCommand::Filter { commands, .. }
        | PaintCommand::Opacity { commands, .. }
        | PaintCommand::Projected { commands, .. }
        | PaintCommand::TextClip { commands, .. } => has_backdrop(commands),
        _ => false,
    })
}

fn rasterize(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    options: RenderOptions,
) -> Result<Vec<u8>, RenderError> {
    if scene.contract_version != RENDER_CONTRACT_VERSION {
        return Err(RenderError::Encode("display-list contract mismatch".into()));
    }
    let mut pixels = vec![0; scene.width as usize * scene.height as usize * 4];
    let font = Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default())
        .map_err(|error| RenderError::Font(error.to_string()))?;
    let mut cache = projected::RasterCache::default();

    if has_backdrop(&scene.commands) {
        projected::paint(
            &mut pixels,
            scene.width,
            scene.height,
            options.scale,
            [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            &[],
            &scene.commands,
            snapshot,
            &font,
            &mut cache,
        )?;
        return Ok(pixels);
    }
    const TILE: u32 = 64;
    for top in (0..scene.height).step_by(TILE as usize) {
        for left in (0..scene.width).step_by(TILE as usize) {
            let width = (scene.width - left).min(TILE);
            let height = (scene.height - top).min(TILE);
            let mut tile = vec![0; width as usize * height as usize * 4];
            let matrix = [
                1.,
                0.,
                0.,
                0.,
                0.,
                1.,
                0.,
                0.,
                0.,
                0.,
                1.,
                0.,
                -(left as f64) / options.scale,
                -(top as f64) / options.scale,
                0.,
                1.,
            ];
            projected::paint(
                &mut tile,
                width,
                height,
                options.scale,
                matrix,
                &[],
                &scene.commands,
                snapshot,
                &font,
                &mut cache,
            )?;
            for row in 0..height as usize {
                let source = row * width as usize * 4;
                let target = ((top as usize + row) * scene.width as usize + left as usize) * 4;
                pixels[target..target + width as usize * 4]
                    .copy_from_slice(&tile[source..source + width as usize * 4]);
            }
        }
    }
    Ok(pixels)
}

fn composite_layer(pixels: &mut [u8], layer: &[u8], opacity: f64) {
    for (index, rgba) in layer.chunks_exact(4).enumerate() {
        blend(
            pixels,
            index * 4,
            with_opacity([rgba[0], rgba[1], rgba[2], rgba[3]], opacity),
        );
    }
}

fn decode_image(image: &oryn_common::v4::PaintImageResource) -> Result<DynamicImage, RenderError> {
    let format = match image.media_type.as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        _ => image::guess_format(&image.bytes).ok(),
    };
    let failure = |detail: String| RenderError::ImageDecode {
        url: image.url.clone(),
        detail,
    };
    let format =
        format.ok_or_else(|| failure(format!("unsupported {} resource", image.media_type)))?;
    if image.bytes.len() > 64 * 1024 * 1024 {
        return Err(failure("encoded image exceeds 64 MiB".into()));
    }
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(&image.bytes), format)
            .into_dimensions()
            .map_err(|error| failure(error.to_string()))?;
    if (width as usize)
        .checked_mul(height as usize)
        .is_none_or(|pixels| pixels > 16 * 1024 * 1024)
    {
        return Err(failure("decoded image exceeds 16 million pixels".into()));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&image.bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|error| failure(error.to_string()))
}

fn pixel_bounds(rect: CssRect, scale: f64, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let x0 = (rect.x * scale).floor().max(0.0).min(f64::from(width)) as u32;
    let y0 = (rect.y * scale).floor().max(0.0).min(f64::from(height)) as u32;
    let x1 = (rect.right() * scale).ceil().max(0.0).min(f64::from(width)) as u32;
    let y1 = (rect.bottom() * scale)
        .ceil()
        .max(0.0)
        .min(f64::from(height)) as u32;
    (x0, y0, x1, y1)
}

fn image_placement(
    rect: CssRect,
    width: u32,
    height: u32,
    style: &std::collections::BTreeMap<String, String>,
    viewport: oryn_common::v4::ViewportState,
) -> CssRect {
    if width == 0 || height == 0 {
        return rect;
    }
    let (natural_width, natural_height) = (width as f64, height as f64);
    let contain = (rect.width / natural_width).min(rect.height / natural_height);
    let (width, height) = match style
        .get("object-fit")
        .map(String::as_str)
        .unwrap_or("fill")
    {
        "contain" => (natural_width * contain, natural_height * contain),
        "cover" => {
            let factor = (rect.width / natural_width).max(rect.height / natural_height);
            (natural_width * factor, natural_height * factor)
        }
        "none" => (natural_width, natural_height),
        "scale-down" => {
            let factor = contain.min(1.);
            (natural_width * factor, natural_height * factor)
        }
        _ => (rect.width, rect.height),
    };
    let font = style
        .get("font-size")
        .and_then(|s| s.strip_suffix("px"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(16.);
    let context = |basis| oryn_common::css_math::LengthContext {
        percentage_basis: basis,
        font_size: font,
        root_font_size: 16.,
        viewport_width: viewport.width,
        viewport_height: viewport.height,
    };
    let position = style
        .get("object-position")
        .map(String::as_str)
        .unwrap_or("50% 50%");
    let (x, y) = oryn_common::css_math::resolve_position(
        position,
        context(rect.width - width),
        context(rect.height - height),
    )
    .unwrap_or(((rect.width - width) / 2., (rect.height - height) / 2.));
    CssRect {
        x: rect.x + x,
        y: rect.y + y,
        width,
        height,
    }
}

fn blend(pixels: &mut [u8], offset: usize, source: [u8; 4]) {
    let alpha = u32::from(source[3]);
    if alpha == 0 {
        return;
    }
    let destination_alpha = u32::from(pixels[offset + 3]);
    let inverse = 255 - alpha;
    // Pixel buffers contain straight RGBA. Composite in premultiplied space,
    // then divide by the resulting alpha before storing the color channels.
    let combined = alpha * 255 + destination_alpha * inverse;
    for channel in 0..3 {
        let numerator = u32::from(source[channel]) * alpha * 255
            + u32::from(pixels[offset + channel]) * destination_alpha * inverse;
        pixels[offset + channel] = ((numerator + combined / 2) / combined) as u8;
    }
    pixels[offset + 3] = ((combined + 127) / 255) as u8;
}

fn opacity(style: &std::collections::BTreeMap<String, String>) -> f64 {
    style
        .get("opacity")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

fn with_opacity(mut color: [u8; 4], opacity: f64) -> [u8; 4] {
    color[3] = (f64::from(color[3]) * opacity).round() as u8;
    color
}

fn single_page_pdf(width: u32, height: u32, compressed_rgb: &[u8]) -> Vec<u8> {
    let content = format!("q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n", width, height);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>").into_bytes(),
        [format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", compressed_rgb.len()).into_bytes(), compressed_rgb.to_vec(), b"\nendstream".to_vec()].concat(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content).into_bytes(),
    ];
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
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
    for offset in offsets.iter().skip(1) {
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
    use oryn_common::v3::{PageId, PageInfo, Revision};
    use oryn_common::v4::{
        BoxModel, EdgeSizes, GeometryAvailability, LayoutRevision, NodeGeometry,
        PaintImageResource, PaintNodeSnapshot, ScrollState, StyleRevision, ViewportState,
    };

    pub(super) fn paint_node(
        id: u64,
        parent: Option<u64>,
        tag: &str,
        rect: CssRect,
    ) -> PaintNodeSnapshot {
        let model = oryn_common::v4::BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            margin: Default::default(),
            padding: Default::default(),
            border: Default::default(),
        };
        PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            node_id: id,
            parent_id: parent,
            tag: tag.into(),
            geometry: oryn_common::v4::NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(model.clone()),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
            projection: Some(oryn_common::v4::PaintProjection {
                scroll: None,
                fragment_boxes: Vec::new(),
                border_segments: Vec::new(),
                border_edges: None,
                text_matrix: None,
                text_clips: Vec::new(),
                matrix: [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ],
                box_model: model,
                text: Vec::new(),
                clips: Vec::new(),
            }),
            style: std::collections::BTreeMap::new(),
            text: Vec::new(),
            image_resource: None,
        }
    }
    #[test]
    fn native_icon_authority_cannot_be_selected_by_author_url_or_tag() {
        let mut snapshot = empty_snapshot();
        let mut node = paint_node(
            1,
            None,
            "img",
            CssRect {
                x: 0.,
                y: 0.,
                width: 24.,
                height: 24.,
            },
        );
        node.style.insert(
            "background-image".into(),
            "url(oryn-native:permission-icon)".into(),
        );
        snapshot.nodes.push(node);
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let author = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        snapshot.nodes[0].builtin_image =
            Some(oryn_common::builtin_image::BuiltinImage::PermissionIcon {
                kind: oryn_common::builtin_image::PermissionIconKind::Location,
                fill: [255, 0, 0, 255],
                stroke: [0; 4],
                stroke_width: 1.,
            });
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let native = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        assert_ne!(author, native);
        assert!(snapshot.images.is_empty());
        snapshot.nodes[0].builtin_image = None;
        snapshot.nodes[0].tag = "::permission-icon".into();
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert_eq!(
            author,
            rasterize(&snapshot, &scene, RenderOptions::default()).unwrap()
        );
    }
    #[test]
    fn visual_camera_uses_left_gutter_origin_and_clips_to_content_port() {
        let mut snapshot = empty_snapshot();
        let mut node = paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 64.,
                height: 32.,
            },
        );
        node.style.insert("background-color".into(), "red".into());
        snapshot.nodes.push(node);
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: Some(CssRect {
                x: 16.,
                y: 0.,
                width: 48.,
                height: 24.,
            }),
            layout_width: 64.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 24.,
            offset_y: 12.,
        });
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let pixels = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        let pixel = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(pixel(8, 8), &[255, 255, 255, 255]);
        assert_eq!(pixel(20, 8), &[255, 0, 0, 255]);
        assert_eq!(pixel(20, 28), &[255, 255, 255, 255]);
        let geometry = snapshot.visual_viewport.unwrap();
        assert_eq!(geometry.layout_to_visual(40., 12.), Some((16., 0.)));
        assert_eq!(geometry.layout_to_visual(64., 24.), Some((64., 24.)));
    }

    #[test]
    fn native_visual_viewport_camera_maps_layout_pixels_without_mutating_geometry() {
        let mut snapshot = empty_snapshot();
        let mut node = paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 20.,
                y: 10.,
                width: 4.,
                height: 4.,
            },
        );
        node.style.insert("background-color".into(), "red".into());
        snapshot.nodes.push(node);
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 64.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 10.,
            offset_y: 5.,
        });
        let original = snapshot.nodes[0].geometry.clone();
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let at = |x: usize, y: usize| {
            &pixels[(y * scene.width as usize + x) * 4..(y * scene.width as usize + x) * 4 + 4]
        };
        assert_eq!(at(21, 11), [255, 0, 0, 255]);
        assert_eq!(at(27, 17), [255, 0, 0, 255]);
        assert_eq!(at(19, 9), [255; 4]);
        assert_eq!(snapshot.nodes[0].geometry, original);
        let geometry = snapshot.visual_viewport.unwrap();
        assert_eq!(geometry.visual_to_layout(20., 10.), Some((20., 10.)));
    }
    #[test]
    fn invalid_or_stale_visual_capture_is_rejected_and_print_clears_nested_cameras() {
        let mut snapshot = empty_snapshot();
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 65.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 0.,
            offset_y: 0.,
        });
        assert!(build_display_list(&snapshot, RenderOptions::default()).is_err());
        snapshot.visual_viewport.as_mut().unwrap().layout_width = 64.;
        snapshot.visual_viewport.as_mut().unwrap().scale = f64::NAN;
        assert!(build_display_list(&snapshot, RenderOptions::default()).is_err());
        let mut child = empty_snapshot();
        child.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 64.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 5.,
            offset_y: 0.,
        });
        snapshot.frames.push(oryn_common::v4::PaintFrameSnapshot {
            owner_node: 1,
            snapshot: Box::new(child),
        });
        clear_visual_viewports(&mut snapshot);
        assert!(snapshot.visual_viewport.is_none());
        assert!(snapshot.frames[0].snapshot.visual_viewport.is_none());
    }
    pub(super) fn empty_snapshot() -> PaintSnapshot {
        PaintSnapshot {
            local_effects: Default::default(),
            visual_viewport: None,
            frames: Vec::new(),
            viewport_scroll: None,
            contract_version: 4,
            page: PageInfo {
                page_id: PageId(1),
                opener_page_id: None,
                url: "about:blank".into(),
                title: String::new(),
                document_generation: 1,
            },
            revision: Revision(1),
            style_revision: StyleRevision(1),
            layout_revision: LayoutRevision(1),
            viewport: ViewportState {
                width: 64.0,
                height: 32.0,
                device_pixel_ratio: 1.0,
            },
            scroll: ScrollState::default(),
            nodes: Vec::new(),
            svg_foreign_objects: Vec::new(),
            images: Vec::new(),
            fonts: std::collections::BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn image_header_dimensions_are_bounded_before_pixel_decode() {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[0, 255, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let resource = |bytes| oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://example.test/image.png".into(),
            media_type: "image/png".into(),
            bytes,
        };
        assert!(decode_image(&resource(bytes.clone())).is_ok());
        bytes[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let mut crc = !0u32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb88320
                } else {
                    crc >> 1
                };
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        let error = decode_image(&resource(bytes)).unwrap_err().to_string();
        assert!(error.contains("16 million pixels"), "{error}");
    }
    #[test]
    fn scrolling_text_keeps_background_stationary_and_clips_glyphs() {
        use oryn_common::v4::{
            PaintClip, PaintFontResource, PaintGlyph, PaintGlyphRun, PaintProjection,
        };
        let mut snapshot = empty_snapshot();
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let mut text_matrix = identity;
        text_matrix[12] = -20.;
        let rect = CssRect {
            x: 10.,
            y: 4.,
            width: 24.,
            height: 24.,
        };
        let model = BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
        };
        snapshot.fonts.insert(
            "face".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let glyph = font.lookup_glyph_index('A') as u32;
        let fragment = PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "AAA".into(),
            rect,
            font_family: oryn_layout::DEFAULT_FONT_FAMILY.into(),
            font_size: 16.,
            color: "black".into(),
            glyph_runs: vec![PaintGlyphRun {
                line_box: None,
                text_box: None,
                background: None,
                decoration_rect: None,
                scale_x: 1.,
                vertical: false,
                decorations: Vec::new(),
                orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
                embolden: false,
                skew_degrees: 0.,
                normalized_coords: vec![],
                color: None,
                font_key: "face".into(),
                font_size: 16.,
                glyphs: [20., 34., 48.]
                    .into_iter()
                    .map(|x| PaintGlyph {
                        source: None,
                        id: glyph,
                        x,
                        y: 22.,
                        advance: 14.,
                    })
                    .collect(),
            }],
        };
        snapshot.nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            parent_id: None,
            node_id: 1,
            tag: "div".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(model.clone()),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
            projection: Some(PaintProjection {
                scroll: None,
                fragment_boxes: Vec::new(),
                border_segments: Vec::new(),
                border_edges: None,
                matrix: identity,
                box_model: model,
                text: vec![fragment],
                clips: vec![],
                text_matrix: Some(text_matrix),
                text_clips: vec![PaintClip {
                    shape: None,
                    owner: None,
                    scroll_x: false,
                    scroll_y: false,
                    matrix: identity,
                    rect,
                    clip_x: true,
                    clip_y: true,
                }],
            }),
            style: std::collections::BTreeMap::from([("background-color".into(), "red".into())]),
            text: vec![],
            image_resource: None,
        });
        let snapshot: PaintSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        let projected = scene
            .commands
            .iter()
            .filter_map(|command| match command {
                PaintCommand::Projected {
                    matrix,
                    clips,
                    commands,
                } => Some((matrix, clips, commands)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), 2);
        assert_eq!(*projected[0].0, identity);
        assert!(projected[0].1.is_empty());
        assert_eq!(*projected[1].0, text_matrix);
        assert_eq!(projected[1].1[0].rect, rect);
        let pixels = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        let pixel = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x + 1) * 4];
        assert_eq!(pixel(10, 4), &[255, 0, 0, 255]);
        assert_eq!(pixel(33, 27), &[255, 0, 0, 255]);
        assert!((10..34).any(|x| (5..27).any(|y| pixel(x, y)[0] < 100)));
        // A glyph partially outside the left edge and one across the right
        // edge may never paint outside the stationary padding clip.
        for y in 4..28 {
            for x in (0..10).chain(34..64) {
                assert_eq!(pixel(x, y), RenderOptions::default().background.as_slice());
            }
        }
        // An offscreen element may still have visible overflowing text.
        let mut overflow = snapshot.clone();
        let node = &mut overflow.nodes[0];
        node.geometry.in_viewport = false;
        let projection = node.projection.as_mut().unwrap();
        projection.box_model.border_box.x = -100.;
        projection.box_model.padding_box.x = -100.;
        projection.box_model.content_box.x = -100.;
        projection.text_clips.clear();
        assert!(!projection.intersects_viewport(overflow.viewport));
        let scene = build_display_list(&overflow, RenderOptions::default()).unwrap();
        assert!(scene.commands.iter().any(|command| matches!(command,
            PaintCommand::Projected{commands,..} if commands.iter().any(|command|matches!(command,PaintCommand::Text{..})))));
        let pixels = rasterize(&overflow, &scene, RenderOptions::default()).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] < 100 && pixel[1] < 100 && pixel[2] < 100)
        );
    }

    #[test]
    fn shaped_text_uses_transported_glyphs_and_positions_after_serialization() {
        use oryn_common::v4::{PaintFontResource, PaintGlyph, PaintGlyphRun};
        let mut snapshot = empty_snapshot();
        snapshot.fonts.insert(
            "face".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let snapshot: PaintSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let mut fragment = PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "unrelated source text".into(),
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 64.,
                height: 32.,
            },
            font_family: "unused family label".into(),
            font_size: 16.,
            color: "black".into(),
            glyph_runs: vec![PaintGlyphRun {
                line_box: None,
                text_box: None,
                background: None,
                decoration_rect: None,
                scale_x: 1.,
                vertical: false,
                decorations: Vec::new(),
                orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
                embolden: false,
                skew_degrees: 0.,
                normalized_coords: vec![],
                color: None,
                font_key: "face".into(),
                font_size: 16.,
                glyphs: vec![PaintGlyph {
                    source: None,
                    id: font.lookup_glyph_index('A') as u32,
                    x: 6.,
                    y: 22.,
                    advance: 0.,
                }],
            }],
        };
        let render = |fragment: PaintTextFragment, snapshot: &PaintSnapshot| {
            let scene = DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 64,
                height: 32,
                commands: vec![PaintCommand::Text {
                    fragment,
                    color: [0, 0, 0, 255],
                }],
                diagnostics: vec![],
            };
            rasterize(snapshot, &scene, RenderOptions::default())
        };
        let original = render(fragment.clone(), &snapshot).unwrap();
        assert!(original.chunks_exact(4).any(|pixel| pixel[3] > 0));
        fragment.text.clear();
        assert_eq!(original, render(fragment.clone(), &snapshot).unwrap());
        fragment.glyph_runs[0].color = Some([255, 0, 0, 255]);
        let colored = render(fragment.clone(), &snapshot).unwrap();
        assert!(colored.chunks_exact(4).any(|pixel| pixel[3] > 0));
        assert!(
            colored
                .chunks_exact(4)
                .filter(|pixel| pixel[3] > 0)
                .all(|pixel| pixel[0] == 255 && pixel[1] == 0 && pixel[2] == 0)
        );
        fragment.glyph_runs[0].color = None;
        fragment.glyph_runs[0].glyphs[0].x += 20.;
        let shifted = render(fragment.clone(), &snapshot).unwrap();
        for y in 0..32usize {
            for x in 0..44usize {
                assert_eq!(
                    &original[(y * 64 + x) * 4..(y * 64 + x + 1) * 4],
                    &shifted[(y * 64 + x + 20) * 4..(y * 64 + x + 21) * 4]
                );
            }
        }
        let mut missing = snapshot.clone();
        missing.fonts.clear();
        assert!(matches!(
            render(fragment.clone(), &missing),
            Err(RenderError::Font(_))
        ));
        fragment.glyph_runs[0].glyphs[0].id = u32::MAX;
        assert!(matches!(
            render(fragment, &snapshot),
            Err(RenderError::Font(_))
        ));
    }

    #[test]
    fn png_and_pdf_are_deterministic_and_valid() {
        let snapshot = empty_snapshot();
        let png_a = render_png(&snapshot, RenderOptions::default()).unwrap();
        let png_b = render_png(&snapshot, RenderOptions::default()).unwrap();
        assert_eq!(png_a.bytes, png_b.bytes);
        assert!(png_a.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = render_pdf(&snapshot, RenderOptions::default()).unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-1.4"));
        assert!(pdf.bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn scale_and_pixel_limits_are_enforced() {
        let snapshot = empty_snapshot();
        assert!(matches!(
            build_display_list(
                &snapshot,
                RenderOptions {
                    scale: 0.0,
                    ..RenderOptions::default()
                }
            ),
            Err(RenderError::InvalidScale(_))
        ));
        assert!(matches!(
            build_display_list(
                &snapshot,
                RenderOptions {
                    max_pixels: 4,
                    ..RenderOptions::default()
                }
            ),
            Err(RenderError::PixelLimit { .. })
        ));
    }

    #[test]
    fn tiled_opacity_and_global_clips_are_continuous_at_tile_edges() {
        let snapshot = empty_snapshot();
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let mut moved = identity;
        moved[12] = 60.;
        let scene = DisplayList {
            contract_version: RENDER_CONTRACT_VERSION,
            width: 130,
            height: 2,
            diagnostics: Vec::new(),
            commands: vec![PaintCommand::Opacity {
                opacity: 0.5,
                commands: vec![PaintCommand::Projected {
                    matrix: moved,
                    clips: vec![oryn_common::v4::PaintClip {
                        shape: None,
                        owner: None,
                        scroll_x: false,
                        scroll_y: false,
                        matrix: identity,
                        rect: CssRect {
                            x: 63.,
                            y: 0.,
                            width: 3.,
                            height: 2.,
                        },
                        clip_x: true,
                        clip_y: true,
                    }],
                    commands: vec![PaintCommand::FillRect {
                        rect: CssRect {
                            x: 0.,
                            y: 0.,
                            width: 20.,
                            height: 2.,
                        },
                        color: [0, 255, 0, 255],
                    }],
                }],
            }],
        };
        let pixels = rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        for y in 0..2 {
            for x in 0..130 {
                let offset = (y * 130 + x) * 4;
                assert_eq!(
                    &pixels[offset..offset + 4],
                    if (63..66).contains(&x) {
                        &[0, 255, 0, 128]
                    } else {
                        &[0, 0, 0, 0]
                    }
                );
            }
        }
    }

    #[test]
    fn flex_and_grid_order_controls_overlapping_subtree_paint() {
        let mut snapshot = empty_snapshot();
        snapshot.viewport.width = 1.;
        snapshot.viewport.height = 1.;
        for (id, parent, color) in [
            (1, None, "white"),
            (2, Some(1), "red"),
            (3, Some(1), "blue"),
        ] {
            let rect = CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            };
            snapshot.nodes.push(PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: None,
                backdrop_style: None,
                top_layer: None,
                node_id: id,
                parent_id: parent,
                tag: "div".into(),
                projection: None,
                geometry: NodeGeometry {
                    availability: GeometryAvailability::Available,
                    box_model: Some(BoxModel {
                        border_box: rect,
                        padding_box: rect,
                        content_box: rect,
                        margin: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                        padding: EdgeSizes::default(),
                    }),
                    client_rects: vec![rect],
                    in_viewport: true,
                    interactable: true,
                    diagnostic: None,
                },
                style: std::collections::BTreeMap::from([(
                    "background-color".into(),
                    color.into(),
                )]),
                text: Vec::new(),
                image_resource: None,
            });
        }
        snapshot.nodes[1].style.insert("order".into(), "2".into());
        for (display, expected) in [
            ("flex", vec![255, 0, 0, 255]),
            ("grid", vec![255, 0, 0, 255]),
            ("block", vec![0, 0, 255, 255]),
        ] {
            snapshot.nodes[0]
                .style
                .insert("display".into(), display.into());
            let options = RenderOptions::default();
            let scene = build_display_list(&snapshot, options).unwrap();
            assert_eq!(rasterize(&snapshot, &scene, options).unwrap(), expected);
        }
    }

    fn stacking_fixture(styles: &[(u64, Option<u64>, &str, &[(&str, &str)])]) -> PaintSnapshot {
        let mut snapshot = empty_snapshot();
        snapshot.viewport.width = 1.;
        snapshot.viewport.height = 1.;
        for &(id, parent, color, rules) in styles {
            let rect = CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            };
            let mut style =
                std::collections::BTreeMap::from([("background-color".into(), color.into())]);
            style.extend(rules.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            snapshot.nodes.push(PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: None,
                backdrop_style: None,
                top_layer: None,
                node_id: id,
                parent_id: parent,
                tag: "div".into(),
                projection: None,
                geometry: NodeGeometry {
                    availability: GeometryAvailability::Available,
                    box_model: Some(BoxModel {
                        border_box: rect,
                        padding_box: rect,
                        content_box: rect,
                        margin: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                        padding: EdgeSizes::default(),
                    }),
                    client_rects: vec![rect],
                    in_viewport: true,
                    interactable: true,
                    diagnostic: None,
                },
                style,
                text: Vec::new(),
                image_resource: None,
            });
        }
        snapshot
    }

    #[test]
    fn table_background_layers_keep_spanning_cells_above_later_rows() {
        let snapshot = stacking_fixture(&[
            (1, None, "white", &[("display", "table")]),
            (2, Some(1), "yellow", &[("display", "table-column-group")]),
            (3, Some(2), "green", &[("display", "table-column")]),
            (4, Some(1), "yellow", &[("display", "table-row-group")]),
            (5, Some(4), "green", &[("display", "table-row")]),
            (6, Some(5), "red", &[("display", "table-cell")]),
            (7, Some(4), "blue", &[("display", "table-row")]),
        ]);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        assert_eq!(
            rasterize(&snapshot, &scene, options).unwrap(),
            vec![255, 0, 0, 255]
        );
    }

    #[test]
    fn table_cell_opacity_context_keeps_its_background_and_child_atomic() {
        let snapshot = stacking_fixture(&[
            (1, None, "white", &[("display", "table")]),
            (2, Some(1), "blue", &[("display", "table-row")]),
            (
                3,
                Some(2),
                "red",
                &[("display", "table-cell"), ("opacity", "0.5")],
            ),
            (4, Some(3), "green", &[]),
        ]);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        assert_eq!(
            rasterize(&snapshot, &scene, options).unwrap(),
            vec![0, 64, 127, 255]
        );
    }

    #[test]
    fn linear_gradient_background_is_painted_through_the_native_display_list() {
        let mut snapshot = stacking_fixture(&[(
            1,
            None,
            "white",
            &[(
                "background-image",
                "linear-gradient(to right, red 50%, transparent 50%)",
            )],
        )]);
        snapshot.viewport.width = 4.;
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 4.,
            height: 1.,
        };
        let model = snapshot.nodes[0].geometry.box_model.as_mut().unwrap();
        model.border_box = rect;
        model.padding_box = rect;
        model.content_box = rect;
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        assert!(
            !scene
                .diagnostics
                .iter()
                .any(|message| message.contains("background-image"))
        );
        assert_eq!(
            rasterize(&snapshot, &scene, options).unwrap(),
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );
    }

    #[test]
    fn descendant_z_index_participates_in_nearest_context_not_dom_parent() {
        let snapshot = stacking_fixture(&[
            (1, None, "white", &[]),
            (2, Some(1), "red", &[]),
            (
                3,
                Some(2),
                "green",
                &[("position", "absolute"), ("z-index", "10")],
            ),
            (
                4,
                Some(1),
                "blue",
                &[("position", "relative"), ("z-index", "2")],
            ),
        ]);
        assert_eq!(stacking::paint_order(&snapshot), vec![1, 2, 4, 3]);
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert_eq!(
            rasterize(&snapshot, &scene, RenderOptions::default()).unwrap(),
            vec![0, 128, 0, 255]
        );
    }

    #[test]
    fn nested_context_never_escapes_its_parent_stack_level() {
        for rule in [
            ("opacity", "0.9"),
            ("transform", "translateX(0px)"),
            ("isolation", "isolate"),
            ("contain", "paint"),
            ("will-change", "transform"),
        ] {
            let snapshot = stacking_fixture(&[
                (1, None, "white", &[]),
                (2, Some(1), "red", &[rule]),
                (
                    3,
                    Some(2),
                    "green",
                    &[("position", "absolute"), ("z-index", "999")],
                ),
                (
                    4,
                    Some(1),
                    "blue",
                    &[("position", "relative"), ("z-index", "1")],
                ),
            ]);
            assert_eq!(
                stacking::paint_order(&snapshot),
                vec![1, 2, 3, 4],
                "{rule:?}"
            );
            let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
            assert_eq!(
                rasterize(&snapshot, &scene, RenderOptions::default()).unwrap(),
                vec![0, 0, 255, 255]
            );
        }
    }

    #[test]
    fn negative_zero_positive_levels_and_static_z_index_are_distinct() {
        let snapshot = stacking_fixture(&[
            (1, None, "white", &[]),
            (
                2,
                Some(1),
                "red",
                &[("position", "relative"), ("z-index", "2")],
            ),
            (
                3,
                Some(1),
                "blue",
                &[("position", "relative"), ("z-index", "-1")],
            ),
            (4, Some(1), "green", &[("z-index", "999")]),
            (5, Some(1), "black", &[("position", "absolute")]),
            (
                6,
                Some(1),
                "yellow",
                &[("position", "relative"), ("z-index", "-2")],
            ),
        ]);
        assert_eq!(stacking::paint_order(&snapshot), vec![1, 6, 3, 4, 5, 2]);
    }

    #[test]
    fn flex_item_z_index_and_equal_level_order_are_respected() {
        let snapshot = stacking_fixture(&[
            (1, None, "white", &[("display", "flex")]),
            (2, Some(1), "red", &[("z-index", "2"), ("order", "4")]),
            (3, Some(1), "blue", &[("z-index", "2"), ("order", "-1")]),
            (4, Some(1), "green", &[("z-index", "1")]),
        ]);
        assert_eq!(stacking::paint_order(&snapshot), vec![1, 4, 3, 2]);
    }

    #[test]
    fn opacity_composites_the_subtree_once_including_overlapping_children() {
        let mut snapshot = empty_snapshot();
        snapshot.viewport.width = 2.;
        snapshot.viewport.height = 1.;
        for (id, parent, width, color, alpha) in
            [(1, None, 2., "red", "0.5"), (2, Some(1), 1., "blue", "1")]
        {
            let rect = CssRect {
                x: 0.,
                y: 0.,
                width,
                height: 1.,
            };
            snapshot.nodes.push(PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: None,
                backdrop_style: None,
                top_layer: None,
                node_id: id,
                parent_id: parent,
                tag: "div".into(),
                projection: None,
                geometry: NodeGeometry {
                    availability: GeometryAvailability::Available,
                    box_model: Some(BoxModel {
                        border_box: rect,
                        padding_box: rect,
                        content_box: rect,
                        margin: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                        padding: EdgeSizes::default(),
                    }),
                    client_rects: vec![rect],
                    in_viewport: true,
                    interactable: true,
                    diagnostic: None,
                },
                style: std::collections::BTreeMap::from([
                    ("background-color".into(), color.into()),
                    ("opacity".into(), alpha.into()),
                ]),
                text: Vec::new(),
                image_resource: None,
            });
        }
        let options = RenderOptions {
            background: [0; 4],
            ..Default::default()
        };
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        assert_eq!(pixels, vec![0, 0, 255, 128, 255, 0, 0, 128]);
        snapshot.nodes[1]
            .style
            .insert("opacity".into(), "0.5".into());
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        assert_eq!(pixels, vec![127, 0, 128, 128, 255, 0, 0, 128]);
        snapshot.nodes[0].style.insert("opacity".into(), "0".into());
        let scene = build_display_list(&snapshot, options).unwrap();
        assert_eq!(rasterize(&snapshot, &scene, options).unwrap(), vec![0; 8]);
    }

    #[test]
    fn image_fit_and_position_use_free_space_before_clipping() {
        let rect = CssRect {
            x: 10.,
            y: 20.,
            width: 100.,
            height: 100.,
        };
        let mut style = std::collections::BTreeMap::new();
        for (fit, expected) in [
            ("fill", (10., 20., 100., 100.)),
            ("contain", (10., 45., 100., 50.)),
            ("cover", (-40., 20., 200., 100.)),
            ("none", (-40., 20., 200., 100.)),
            ("scale-down", (10., 45., 100., 50.)),
        ] {
            style.insert("object-fit".into(), fit.into());
            let actual = image_placement(rect, 200, 100, &style, ViewportState::default());
            assert_eq!(
                (actual.x, actual.y, actual.width, actual.height),
                expected,
                "{fit}"
            );
        }
        style.insert("object-fit".into(), "cover".into());
        style.insert("object-position".into(), "right 10px bottom 5px".into());
        let actual = image_placement(rect, 200, 100, &style, ViewportState::default());
        assert_eq!((actual.x, actual.y), (-100., 15.));
        style.insert("object-position".into(), "calc(50% + 10px) center".into());
        assert_eq!(
            image_placement(rect, 200, 100, &style, ViewportState::default()).x,
            -30.
        );
        style.insert("object-fit".into(), "scale-down".into());
        style.remove("object-position");
        let actual = image_placement(rect, 20, 10, &style, ViewportState::default());
        assert_eq!(
            (actual.x, actual.y, actual.width, actual.height),
            (50., 65., 20., 10.)
        );
    }

    #[test]
    fn clipped_images_keep_original_source_mapping() {
        let mut snapshot = empty_snapshot();
        snapshot.viewport.width = 1.;
        snapshot.viewport.height = 1.;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                &[255, 0, 0, 255, 0, 0, 255, 255],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        snapshot.images.push(PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "fixture://crop.png".into(),
            media_type: "image/png".into(),
            bytes,
        });
        for (x, expected) in [(-1., vec![0, 0, 255, 255]), (0., vec![255, 0, 0, 255])] {
            let scene = DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 1,
                height: 1,
                diagnostics: Vec::new(),
                commands: vec![PaintCommand::Image {
                    rect: CssRect {
                        x,
                        y: 0.,
                        width: 2.,
                        height: 1.,
                    },
                    resource: 0,
                    rendering: ImageRendering::CrispEdges,
                    clip: None,
                }],
            };
            assert_eq!(
                rasterize(&snapshot, &scene, RenderOptions::default()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn projected_snapshot_renders_horizon_image_through_ancestor_clip() {
        use oryn_common::v4::{PaintClip, PaintProjection};
        let mut snapshot = empty_snapshot();
        snapshot.viewport.width = 4.;
        snapshot.viewport.height = 2.;
        let original = CssRect {
            x: 0.,
            y: 0.,
            width: 2.,
            height: 2.,
        };
        let model = BoxModel {
            border_box: original,
            padding_box: original,
            content_box: original,
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
        };
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let mut matrix = identity;
        matrix[3] = -1.;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[0, 255, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        snapshot.images.push(PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "fixture://green.png".into(),
            media_type: "image/png".into(),
            bytes,
        });
        snapshot.nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            parent_id: None,
            node_id: 1,
            tag: "img".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: CssRect {
                        x: -2.,
                        y: -2.,
                        width: 2.,
                        height: 2.,
                    },
                    ..model.clone()
                }),
                client_rects: Vec::new(),
                in_viewport: false,
                interactable: false,
                diagnostic: None,
            },
            projection: Some(PaintProjection {
                scroll: None,
                fragment_boxes: Vec::new(),
                border_segments: Vec::new(),
                border_edges: None,
                text_matrix: None,
                text_clips: Vec::new(),
                matrix,
                box_model: model,
                text: Vec::new(),
                clips: vec![PaintClip {
                    shape: None,
                    owner: None,
                    scroll_x: false,
                    scroll_y: false,
                    matrix: identity,
                    rect: CssRect {
                        x: 1.,
                        y: 0.,
                        width: 2.,
                        height: 2.,
                    },
                    clip_x: true,
                    clip_y: true,
                }],
            }),
            style: std::collections::BTreeMap::from([
                ("background-color".into(), "red".into()),
                ("image-rendering".into(), "pixelated".into()),
            ]),
            text: Vec::new(),
            image_resource: Some(0),
        });
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        let snapshot: PaintSnapshot = serde_json::from_slice(&encoded).unwrap();
        let options = RenderOptions {
            background: [0; 4],
            ..Default::default()
        };
        let scene = build_display_list(&snapshot, options).unwrap();
        assert_eq!(scene.contract_version, RENDER_CONTRACT_VERSION);
        assert!(scene.commands.iter().any(|command|matches!(command,PaintCommand::Projected{commands,..}
            if commands.iter().any(|command|matches!(command,PaintCommand::Image{rendering:ImageRendering::Pixelated,..})))));
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        for y in 0..2 {
            for x in 0..4 {
                let start = (y * 4 + x) * 4;
                assert_eq!(
                    &pixels[start..start + 4],
                    if x == 1 || x == 2 {
                        &[0, 255, 0, 255]
                    } else {
                        &[0, 0, 0, 0]
                    }
                );
            }
        }
    }

    #[test]
    fn scene_paints_style_text_and_image_in_document_order() {
        let mut snapshot = empty_snapshot();
        let rect = CssRect {
            x: 2.0,
            y: 2.0,
            width: 24.0,
            height: 20.0,
        };
        let edge = EdgeSizes {
            top: 1.0,
            right: 1.0,
            bottom: 1.0,
            left: 1.0,
        };
        snapshot.images.push(PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "fixture://pixel.png".into(),
            media_type: "image/png".into(),
            bytes: {
                let mut bytes = Vec::new();
                image::codecs::png::PngEncoder::new(&mut bytes)
                    .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
                    .unwrap();
                bytes
            },
        });
        snapshot.nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            parent_id: None,
            projection: None,
            node_id: 1,
            tag: "div".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    margin: EdgeSizes::default(),
                    border: edge,
                    padding: EdgeSizes::default(),
                }),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
            style: std::collections::BTreeMap::from([
                ("background-color".into(), "#123456".into()),
                ("border-color".into(), "black".into()),
                ("opacity".into(), "1".into()),
            ]),
            text: vec![PaintTextFragment {
                flow_line: None,
                flow_rect: None,
                clusters: Vec::new(),
                glyph_runs: Vec::new(),
                text: "A".into(),
                rect,
                font_family: "Oryn Noto Sans".into(),
                font_size: 12.0,
                color: "white".into(),
            }],
            image_resource: Some(0),
        });
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(matches!(scene.commands[1], PaintCommand::FillRect { .. }));
        assert!(
            scene
                .commands
                .iter()
                .any(|command| matches!(command, PaintCommand::Border { .. }))
        );
        assert!(
            scene
                .commands
                .iter()
                .any(|command| matches!(command, PaintCommand::Image { .. }))
        );
        assert!(
            scene
                .commands
                .iter()
                .any(|command| matches!(command, PaintCommand::Text { .. }))
        );
        assert!(render_png(&snapshot, RenderOptions::default()).is_ok());
    }

    #[test]
    fn unsupported_and_malformed_resources_are_diagnostics() {
        let mut snapshot = empty_snapshot();
        snapshot.diagnostics.push("worker diagnostic".into());
        snapshot.images.push(PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "fixture://bad.png".into(),
            media_type: "image/png".into(),
            bytes: b"not a png".to_vec(),
        });
        let rect = CssRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        snapshot.nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            parent_id: None,
            projection: None,
            node_id: 1,
            tag: "img".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    margin: EdgeSizes::default(),
                    border: EdgeSizes::default(),
                    padding: EdgeSizes::default(),
                }),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: false,
                diagnostic: None,
            },
            style: std::collections::BTreeMap::from([(
                "background-image".into(),
                "paint(unavailable-painter)".into(),
            )]),
            text: Vec::new(),
            image_resource: Some(0),
        });
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|item| item.contains("unsupported background image"))
        );
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|item| item.contains("could not be decoded"))
        );
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|item| item == "worker diagnostic")
        );
        assert!(render_png(&snapshot, RenderOptions::default()).is_ok());
        // A supported gradient must paint successfully alongside the malformed image.
        snapshot.nodes[0].style.insert(
            "background-image".into(),
            "linear-gradient(red,blue)".into(),
        );
        // The original full-box gradient uses the single-tile vector fast
        // path. Exercise it before the explicit repeated-background contract.
        let single = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(
            single
                .commands
                .iter()
                .any(|command| matches!(command, PaintCommand::LinearGradient { .. }))
        );
        assert!(
            single
                .diagnostics
                .iter()
                .any(|item| item.contains("could not be decoded"))
        );
        assert!(render_png(&snapshot, RenderOptions::default()).is_ok());
        snapshot.nodes[0]
            .style
            .insert("background-size".into(), "5px 5px".into());
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(
            !scene
                .diagnostics
                .iter()
                .any(|item| item.contains("unsupported background image"))
        );
        assert!(
            scene
                .commands
                .iter()
                .any(|command| matches!(command, PaintCommand::Background { .. }))
        );
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|item| item.contains("could not be decoded"))
        );
        assert!(render_png(&snapshot, RenderOptions::default()).is_ok());
    }
}
