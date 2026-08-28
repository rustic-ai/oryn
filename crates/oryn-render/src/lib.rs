//! Optional deterministic CPU renderer for Oryn paint snapshots.
//!
//! This crate owns display lists, decoded images and raster buffers.  It never
//! owns page state and performs no filesystem or network access.

use std::io::Write;

use flate2::{Compression, write::ZlibEncoder};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, ImageEncoder, ImageFormat};
use oryn_common::v4::{CssRect, GeometryAvailability, PaintSnapshot, PaintTextFragment};
use serde::{Deserialize, Serialize};

pub const RENDER_CONTRACT_VERSION: u16 = 1;

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
        svg_images: false,
        gradients: false,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaintCommand {
    FillRect {
        rect: CssRect,
        color: [u8; 4],
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
    #[error(transparent)]
    Page(#[from] oryn_native::RuntimeError),
    #[error("invalid render scale {0}")]
    InvalidScale(f64),
    #[error("raster would contain {actual} pixels, limit is {limit}")]
    PixelLimit { actual: usize, limit: usize },
    #[error("image {url} could not be decoded: {detail}")]
    ImageDecode { url: String, detail: String },
    #[error("embedded font could not be loaded: {0}")]
    Font(String),
    #[error("output encoding failed: {0}")]
    Encode(String),
}

/// Capture the current viewport through the sandbox-worker snapshot boundary.
pub async fn screenshot(
    page: &oryn_native::PageHandle,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let snapshot = page.paint_snapshot().await?;
    render_png(&snapshot, options)
}

/// Produce a single-page PDF from the same display list as `screenshot`.
pub async fn pdf(
    page: &oryn_native::PageHandle,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let snapshot = page.paint_snapshot().await?;
    render_pdf(&snapshot, options)
}

pub fn build_display_list(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<DisplayList, RenderError> {
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
    for node in &snapshot.nodes {
        if node.geometry.availability != GeometryAvailability::Available
            || !node.geometry.in_viewport
        {
            continue;
        }
        let Some(model) = &node.geometry.box_model else {
            continue;
        };
        if let Some(value) = node.style.get("background-image")
            && value != "none"
        {
            diagnostics.push(format!(
                "node {} background-image is unsupported by the CPU reference renderer",
                node.node_id
            ));
        }
        if node
            .style
            .get("z-index")
            .is_some_and(|value| value != "auto")
        {
            diagnostics.push(format!(
                "node {} z-index stacking is unsupported; document order is used",
                node.node_id
            ));
        }
        if let Some(color) = node
            .style
            .get("background-color")
            .and_then(|value| parse_color(value))
            && color[3] != 0
        {
            commands.push(PaintCommand::FillRect {
                rect: model.padding_box,
                color: with_opacity(color, opacity(&node.style)),
            });
        }
        let widths = [
            model.border.top,
            model.border.right,
            model.border.bottom,
            model.border.left,
        ];
        if widths.iter().any(|width| *width > 0.0) {
            let color = node
                .style
                .get("border-color")
                .or_else(|| node.style.get("color"))
                .and_then(|value| parse_color(value))
                .unwrap_or([0, 0, 0, 255]);
            commands.push(PaintCommand::Border {
                rect: model.border_box,
                widths,
                color: with_opacity(color, opacity(&node.style)),
            });
        }
        if let Some(resource) = node.image_resource {
            match snapshot.images.get(resource) {
                Some(image) if image.media_type == "image/svg+xml" => diagnostics.push(format!(
                    "node {} SVG image paint is unsupported",
                    node.node_id
                )),
                Some(image) => match decode_image(image) {
                    Ok(_) => commands.push(PaintCommand::Image {
                        rect: model.content_box,
                        resource,
                    }),
                    Err(error) => diagnostics.push(error.to_string()),
                },
                None => diagnostics.push(format!(
                    "node {} references missing image resource {}",
                    node.node_id, resource
                )),
            }
        }
        for fragment in &node.text {
            if !fragment
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
                color: with_opacity(color, opacity(&node.style)),
            });
        }
    }
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

fn rasterize(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    options: RenderOptions,
) -> Result<Vec<u8>, RenderError> {
    let mut pixels = vec![0; scene.width as usize * scene.height as usize * 4];
    let font = Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default())
        .map_err(|error| RenderError::Font(error.to_string()))?;
    for command in &scene.commands {
        match command {
            PaintCommand::FillRect { rect, color } => fill_rect(
                &mut pixels,
                scene.width,
                scene.height,
                *rect,
                *color,
                options.scale,
            ),
            PaintCommand::Border {
                rect,
                widths,
                color,
            } => paint_border(
                &mut pixels,
                scene.width,
                scene.height,
                *rect,
                *widths,
                *color,
                options.scale,
            ),
            PaintCommand::Text { fragment, color } => paint_text(
                &mut pixels,
                scene.width,
                scene.height,
                &font,
                fragment,
                *color,
                options.scale,
            ),
            PaintCommand::Image { rect, resource } => {
                if let Some(image) = snapshot.images.get(*resource) {
                    let decoded = decode_image(image)?;
                    paint_image(
                        &mut pixels,
                        scene.width,
                        scene.height,
                        decoded,
                        *rect,
                        options.scale,
                    );
                }
            }
        }
    }
    Ok(pixels)
}

fn decode_image(image: &oryn_common::v4::PaintImageResource) -> Result<DynamicImage, RenderError> {
    let format = match image.media_type.as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        _ => image::guess_format(&image.bytes).ok(),
    };
    format
        .and_then(|format| image::load_from_memory_with_format(&image.bytes, format).ok())
        .ok_or_else(|| RenderError::ImageDecode {
            url: image.url.clone(),
            detail: format!("unsupported or malformed {} resource", image.media_type),
        })
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

fn fill_rect(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    rect: CssRect,
    color: [u8; 4],
    scale: f64,
) {
    let (x0, y0, x1, y1) = pixel_bounds(rect, scale, width, height);
    for y in y0..y1 {
        for x in x0..x1 {
            blend(pixels, ((y * width + x) * 4) as usize, color);
        }
    }
}

fn paint_border(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    rect: CssRect,
    widths: [f64; 4],
    color: [u8; 4],
    scale: f64,
) {
    fill_rect(
        pixels,
        width,
        height,
        CssRect {
            height: widths[0],
            ..rect
        },
        color,
        scale,
    );
    fill_rect(
        pixels,
        width,
        height,
        CssRect {
            x: rect.right() - widths[1],
            width: widths[1],
            ..rect
        },
        color,
        scale,
    );
    fill_rect(
        pixels,
        width,
        height,
        CssRect {
            y: rect.bottom() - widths[2],
            height: widths[2],
            ..rect
        },
        color,
        scale,
    );
    fill_rect(
        pixels,
        width,
        height,
        CssRect {
            width: widths[3],
            ..rect
        },
        color,
        scale,
    );
}

fn paint_text(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    font: &Font,
    fragment: &PaintTextFragment,
    color: [u8; 4],
    scale: f64,
) {
    let mut pen_x = fragment.rect.x * scale;
    let baseline = (fragment.rect.y + fragment.font_size) * scale;
    let size = (fragment.font_size * scale) as f32;
    for character in fragment.text.chars() {
        if matches!(character, '\n' | '\r') {
            continue;
        }
        if character == '\t' {
            let metrics = font.metrics(' ', size);
            pen_x += metrics.advance_width as f64 * 4.0;
            continue;
        }
        let (metrics, bitmap) = font.rasterize(character, size);
        let origin_x = pen_x + f64::from(metrics.xmin);
        let origin_y = baseline - metrics.height as f64 - f64::from(metrics.ymin);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let x = (origin_x + column as f64).round() as i64;
                let y = (origin_y + row as f64).round() as i64;
                if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
                    continue;
                }
                let alpha = bitmap[row * metrics.width + column];
                let mut glyph = color;
                glyph[3] = ((u16::from(glyph[3]) * u16::from(alpha)) / 255) as u8;
                blend(
                    pixels,
                    (((y as u32 * width) + x as u32) * 4) as usize,
                    glyph,
                );
            }
        }
        pen_x += metrics.advance_width as f64;
    }
}

fn paint_image(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    image: DynamicImage,
    rect: CssRect,
    scale: f64,
) {
    let (x0, y0, x1, y1) = pixel_bounds(rect, scale, width, height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let resized = image
        .resize_exact(x1 - x0, y1 - y0, image::imageops::FilterType::Triangle)
        .to_rgba8();
    for y in 0..(y1 - y0) {
        for x in 0..(x1 - x0) {
            blend(
                pixels,
                (((y0 + y) * width + x0 + x) * 4) as usize,
                resized.get_pixel(x, y).0,
            );
        }
    }
}

fn blend(pixels: &mut [u8], offset: usize, source: [u8; 4]) {
    let alpha = u16::from(source[3]);
    let inverse = 255 - alpha;
    for channel in 0..3 {
        pixels[offset + channel] = ((u16::from(source[channel]) * alpha
            + u16::from(pixels[offset + channel]) * inverse)
            / 255) as u8;
    }
    pixels[offset + 3] = (alpha + u16::from(pixels[offset + 3]) * inverse / 255).min(255) as u8;
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

fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "transparent" => return Some([0, 0, 0, 0]),
        "black" => return Some([0, 0, 0, 255]),
        "white" => return Some([255, 255, 255, 255]),
        "red" => return Some([255, 0, 0, 255]),
        "green" => return Some([0, 128, 0, 255]),
        "blue" => return Some([0, 0, 255, 255]),
        _ => {}
    }
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => Some([
                u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
                255,
            ]),
            6 => Some([
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
                255,
            ]),
            8 => Some([
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
                u8::from_str_radix(&hex[6..8], 16).ok()?,
            ]),
            _ => None,
        };
    }
    let body = value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))?;
    let channels = body
        .split(',')
        .map(|part| part.trim().parse::<u8>().ok())
        .collect::<Option<Vec<_>>>()?;
    (channels.len() == 3).then(|| [channels[0], channels[1], channels[2], 255])
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

    fn empty_snapshot() -> PaintSnapshot {
        PaintSnapshot {
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
            images: Vec::new(),
            diagnostics: Vec::new(),
        }
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
                "linear-gradient(red,blue)".into(),
            )]),
            text: Vec::new(),
            image_resource: Some(0),
        });
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|item| item.contains("background-image"))
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
