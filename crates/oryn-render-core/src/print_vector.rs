//! Vector PDF input from the native display list. Primitive geometry and shaped
//! glyph outlines are preserved; localized raster fallback retains perspective
//! and effects that the PDF/SVG vector pipeline cannot represent faithfully.
use super::*;
use base64::Engine as _;
use oryn_common::v4::{PaintClip, PaintGlyphOrientation};
const MAX_XML: usize = 16 * 1024 * 1024;
fn affine(m: [f64; 16]) -> bool {
    m.iter().all(|v| v.is_finite())
        && m[3] == 0.
        && m[7] == 0.
        && m[15] == 1.
        && m[2] == 0.
        && m[6] == 0.
        && m[8] == 0.
        && m[9] == 0.
        && m[11] == 0.
}
fn matrix(m: [f64; 16]) -> String {
    format!(
        "matrix({:.8} {:.8} {:.8} {:.8} {:.8} {:.8})",
        m[0], m[1], m[4], m[5], m[12], m[13]
    )
}
fn color(c: [u8; 4]) -> String {
    format!(
        "fill=\"rgb({},{},{})\" fill-opacity=\"{:.8}\"",
        c[0],
        c[1],
        c[2],
        f64::from(c[3]) / 255.
    )
}
fn rounded(rect: CssRect, r: [f64; 8]) -> String {
    let (x, y, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
    format!(
        "M{} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}Z",
        x + r[0],
        y,
        right - r[2],
        r[2],
        r[3],
        right,
        y + r[3],
        bottom - r[5],
        r[4],
        r[5],
        right - r[4],
        bottom,
        x + r[6],
        r[6],
        r[7],
        x,
        bottom - r[7],
        y + r[1],
        r[0],
        r[1],
        x + r[0],
        y
    )
}
fn multiply(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|index| {
        let row = index % 4;
        let col = index / 4;
        (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
    })
}
struct Export<'a> {
    snapshot: &'a PaintSnapshot,
    options: RenderOptions,
    defs: String,
    body: String,
    next: usize,
    fallbacks: usize,
    fallback_pixels: usize,
    glyphs: std::collections::BTreeMap<(String, u32, u64, bool, u32, Vec<i16>), String>,
}
impl Export<'_> {
    fn id(&mut self) -> String {
        self.next += 1;
        format!("v{}", self.next)
    }
    fn background(
        &mut self,
        rect: CssRect,
        radii: [f64; 8],
        tile: &BackgroundTile,
        source: &BackgroundSource,
    ) -> Result<(), RenderError> {
        let period = |origin: f64, size: f64, step: Option<f64>, start: f64, end: f64| {
            step.unwrap_or_else(|| {
                ((start - origin).abs().max((end - origin).abs()) + size + 1.) * 2.
            })
        };
        let width = period(
            tile.rect.x,
            tile.rect.width,
            tile.step_x,
            rect.x,
            rect.right(),
        );
        let height = period(
            tile.rect.y,
            tile.rect.height,
            tile.step_y,
            rect.y,
            rect.bottom(),
        );
        if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
            return Err(RenderError::InvalidPrint(
                "invalid background pattern period".into(),
            ));
        }
        let content = match source {
            BackgroundSource::Gradient(BackgroundGradient::Linear(gradient)) => {
                let id = self.id();
                let first = gradient
                    .stops
                    .first()
                    .map(|stop| stop.position)
                    .unwrap_or(0.)
                    .min(0.);
                let last = gradient
                    .stops
                    .last()
                    .map(|stop| stop.position)
                    .unwrap_or(1.)
                    .max(1.);
                let extent = (last - first).max(1e-12);
                let dx = gradient.end[0] - gradient.start[0];
                let dy = gradient.end[1] - gradient.start[1];
                self.defs.push_str(&format!("<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">",gradient.start[0]+first*dx-tile.rect.x,gradient.start[1]+first*dy-tile.rect.y,gradient.start[0]+last*dx-tile.rect.x,gradient.start[1]+last*dy-tile.rect.y));
                for stop in &gradient.stops {
                    self.defs.push_str(&format!(
                        "<stop offset=\"{}\" stop-color=\"rgb({},{},{})\" stop-opacity=\"{}\"/>",
                        (stop.position - first) / extent,
                        stop.color[0],
                        stop.color[1],
                        stop.color[2],
                        f64::from(stop.color[3]) / 255.
                    ));
                }
                self.defs.push_str("</linearGradient>");
                format!(
                    "<rect width=\"{}\" height=\"{}\" fill=\"url(#{id})\"/>",
                    tile.rect.width, tile.rect.height
                )
            }
            BackgroundSource::Image { resource, .. } => {
                let source = self.snapshot.images.get(*resource).ok_or_else(|| {
                    RenderError::InvalidPrint("background image resource missing".into())
                })?;
                format!(
                    "<image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:{};base64,{}\"/>",
                    tile.rect.width,
                    tile.rect.height,
                    source
                        .media_type
                        .replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('<', "&lt;"),
                    base64::engine::general_purpose::STANDARD.encode(&source.bytes)
                )
            }
            _ => {
                return Err(RenderError::InvalidPrint(
                    "background requires localized effect raster".into(),
                ));
            }
        };
        let id = self.id();
        self.defs.push_str(&format!("<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">{content}</pattern>",tile.rect.x,tile.rect.y));
        let mut limit = rect;
        if let Some([start, end]) = tile.limit_x {
            let right = limit.right().min(end);
            limit.x = limit.x.max(start);
            limit.width = (right - limit.x).max(0.);
        }
        if let Some([start, end]) = tile.limit_y {
            let bottom = limit.bottom().min(end);
            limit.y = limit.y.max(start);
            limit.height = (bottom - limit.y).max(0.);
        }
        let clip = self.id();
        self.defs.push_str(&format!("<clipPath id=\"{clip}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",limit.x,limit.y,limit.width,limit.height));
        self.body.push_str(&format!(
            "<path d=\"{}\" clip-path=\"url(#{clip})\" fill=\"url(#{id})\"/>",
            rounded(rect, radii)
        ));
        Ok(())
    }
    fn clip_open(&mut self, clips: &[PaintClip]) -> usize {
        for clip in clips {
            let id = self.id();
            let mut rect = clip.rect;
            if !clip.clip_x {
                rect.x = -1e9;
                rect.width = 2e9;
            }
            if !clip.clip_y {
                rect.y = -1e9;
                rect.height = 2e9;
            }
            self.defs.push_str(&format!("<clipPath id=\"{id}\" clipPathUnits=\"userSpaceOnUse\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" transform=\"{}\"/></clipPath>",rect.x,rect.y,rect.width,rect.height,matrix(clip.matrix)));
            self.body.push_str(&format!("<g clip-path=\"url(#{id})\">"));
        }
        clips.len()
    }
    fn fallback(
        &mut self,
        command: &PaintCommand,
        world: [f64; 16],
        clips: &[PaintClip],
    ) -> Result<(), RenderError> {
        self.fallbacks += 1;
        if self.fallbacks > 128 {
            return Err(RenderError::InvalidPrint(
                "vector page exceeds 128 localized raster fallbacks".into(),
            ));
        }
        let options = RenderOptions {
            background: [0; 4],
            ..self.options
        };
        let (width, height) = dimensions(self.snapshot, options)?;
        self.fallback_pixels = self
            .fallback_pixels
            .checked_add(width as usize * height as usize)
            .filter(|n| *n <= self.options.max_pixels)
            .ok_or_else(|| {
                RenderError::InvalidPrint("localized print fallback pixel budget exceeded".into())
            })?;
        let scene = DisplayList {
            contract_version: RENDER_CONTRACT_VERSION,
            width,
            height,
            diagnostics: Vec::new(),
            commands: vec![PaintCommand::Projected {
                matrix: world,
                clips: clips.to_vec(),
                commands: vec![command.clone()],
            }],
        };
        let rgba = rasterize(self.snapshot, &scene, options)?;
        // Keep the fallback local in the PDF even though the reference compositor
        // uses a bounded page surface to preserve perspective and clip semantics.
        let mut left = width;
        let mut top = height;
        let mut right = 0;
        let mut bottom = 0;
        for y in 0..height {
            for x in 0..width {
                if rgba[((y * width + x) * 4 + 3) as usize] != 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        if right <= left || bottom <= top {
            return Ok(());
        }
        let crop_width = right - left;
        let crop_height = bottom - top;
        let mut crop = Vec::with_capacity(crop_width as usize * crop_height as usize * 4);
        for y in top..bottom {
            let start = ((y * width + left) * 4) as usize;
            crop.extend_from_slice(&rgba[start..start + crop_width as usize * 4]);
        }
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                &crop,
                crop_width,
                crop_height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| RenderError::Encode(e.to_string()))?;
        self.body.push_str(&format!("<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"data:image/png;base64,{}\"/>",f64::from(left)/options.scale,f64::from(top)/options.scale,f64::from(crop_width)/options.scale,f64::from(crop_height)/options.scale,base64::engine::general_purpose::STANDARD.encode(bytes)));
        Ok(())
    }
    fn emit(
        &mut self,
        commands: &[PaintCommand],
        world: [f64; 16],
        clips: &[PaintClip],
        depth: usize,
    ) -> Result<(), RenderError> {
        if depth > 256 {
            return Err(RenderError::InvalidPrint(
                "vector paint nesting exceeds 256".into(),
            ));
        }
        for command in commands {
            if self.body.len() + self.defs.len() > MAX_XML {
                return Err(RenderError::InvalidPrint(
                    "vector page source exceeds 16 MiB".into(),
                ));
            }
            match command {
                PaintCommand::SvgMask { .. }
                | PaintCommand::BackdropFilter { .. }
                | PaintCommand::Mask { .. }
                | PaintCommand::Filter { .. } => {
                    self.fallback(command, world, clips)?;
                    continue;
                }
                PaintCommand::Opacity { opacity, commands } => {
                    self.body
                        .push_str(&format!("<g opacity=\"{}\">", opacity.clamp(0., 1.)));
                    self.emit(commands, world, clips, depth + 1)?;
                    self.body.push_str("</g>");
                    continue;
                }
                PaintCommand::Projected {
                    matrix: local,
                    clips: local_clips,
                    commands,
                } => {
                    let mut nested = clips.to_vec();
                    nested.extend(local_clips.iter().cloned().map(|mut clip| {
                        clip.matrix = multiply(world, clip.matrix);
                        clip
                    }));
                    self.emit(commands, multiply(world, *local), &nested, depth + 1)?;
                    continue;
                }
                _ => {}
            }
            if !affine(world)
                || clips
                    .iter()
                    .any(|clip| clip.shape.is_some() || !affine(clip.matrix))
            {
                self.fallback(command, world, clips)?;
                continue;
            }
            if matches!(
                command,
                PaintCommand::BuiltinImage { .. }
                    | PaintCommand::TextClip { .. }
                    | PaintCommand::Canvas { .. }
                    | PaintCommand::Gradient { .. }
                    | PaintCommand::BoxShadow { .. }
            ) || matches!(command,PaintCommand::Background{source,..} if !matches!(source,BackgroundSource::Gradient(BackgroundGradient::Linear(gradient)) if !gradient.repeating)&&!matches!(source,BackgroundSource::Image{svg:false,rendering:ImageRendering::Auto,..}))
                || matches!(command,PaintCommand::LinearGradient{gradient,..} if gradient.repeating)
                || matches!(command,PaintCommand::Image{rendering,..} if *rendering!=ImageRendering::Auto)
                || matches!(command,PaintCommand::Text{fragment,..} if fragment.glyph_runs.is_empty()||fragment.glyph_runs.iter().any(|run|!run.decorations.is_empty()))
                || matches!(command,PaintCommand::BorderEdges{edges,radii,..} if radii.iter().any(|r|*r>0.)||edges.iter().any(|e|!matches!(e.style.as_str(),"solid"|"none"|"hidden")))
            {
                self.fallback(command, world, clips)?;
                continue;
            }
            if let PaintCommand::SvgImage {
                resource,
                environment,
                ..
            } = command
            {
                let source = self.snapshot.images.get(*resource).ok_or_else(|| {
                    RenderError::InvalidPrint("vector SVG resource missing".into())
                })?;
                let tree = crate::svg::parse_with_environment(source, self.snapshot, *environment)?;
                if !crate::svg_coverage::text_capture_active()
                    && (tree
                        .linear_gradients()
                        .iter()
                        .any(|g| g.spread_method() != resvg::usvg::SpreadMethod::Pad)
                        || tree
                            .radial_gradients()
                            .iter()
                            .any(|g| g.spread_method() != resvg::usvg::SpreadMethod::Pad))
                {
                    self.fallback(command, world, clips)?;
                    continue;
                }
            }
            if let PaintCommand::Text { fragment, .. } = command {
                for run in &fragment.glyph_runs {
                    let boxes = crate::text_boxes::commands(self.snapshot, fragment, run)?;
                    if !boxes.is_empty() {
                        self.emit(&boxes, world, clips, depth + 1)?;
                    }
                }
            }
            let count = self.clip_open(clips);
            self.body
                .push_str(&format!("<g transform=\"{}\">", matrix(world)));
            match command {
                PaintCommand::EmbeddedDocument { rect, frame } => {
                    let _frame_scope = crate::svg_coverage::frame_scope(
                        self.snapshot
                            .frames
                            .get(*frame)
                            .ok_or_else(|| {
                                RenderError::InvalidPrint(
                                    "semantic frame index out of bounds".into(),
                                )
                            })?
                            .owner_node,
                    );
                    let child = crate::frames::snapshot(self.snapshot, *frame)?;
                    let (scene, options) = crate::frames::scene(child, self.options.scale)?;
                    let tree = crate::print::vector::tree(child, &scene, options)?;
                    let id = self.id();
                    let source = tree.to_string(&resvg::usvg::WriteOptions {
                        coordinates_precision: 8,
                        transforms_precision: 8,
                        id_prefix: Some(format!("{id}_frame_")),
                        preserve_text: false,
                        ..Default::default()
                    });
                    self.body.push_str(&format!("<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\">{source}</svg>",rect.x,rect.y,rect.width,rect.height,child.viewport.width,child.viewport.height));
                }
                PaintCommand::Background {
                    rect,
                    radii,
                    tile,
                    source,
                } => self.background(*rect, *radii, tile, source)?,
                PaintCommand::FillRect { rect, color: c } => self.body.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {}/>",
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    color(*c)
                )),
                PaintCommand::FillRoundedRect {
                    rect,
                    color: c,
                    radii,
                } => self.body.push_str(&format!(
                    "<path d=\"{}\" {}/>",
                    rounded(*rect, *radii),
                    color(*c)
                )),
                PaintCommand::LinearGradient {
                    rect,
                    gradient,
                    radii,
                } => {
                    let id = self.id();
                    let first = gradient
                        .stops
                        .first()
                        .map(|s| s.position)
                        .unwrap_or(0.)
                        .min(0.);
                    let last = gradient
                        .stops
                        .last()
                        .map(|s| s.position)
                        .unwrap_or(1.)
                        .max(1.);
                    let extent = (last - first).max(1e-12);
                    let dx = gradient.end[0] - gradient.start[0];
                    let dy = gradient.end[1] - gradient.start[1];
                    self.defs.push_str(&format!("<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">",gradient.start[0]+first*dx,gradient.start[1]+first*dy,gradient.start[0]+last*dx,gradient.start[1]+last*dy));
                    for stop in &gradient.stops {
                        self.defs.push_str(&format!("<stop offset=\"{}\" stop-color=\"rgb({},{},{})\" stop-opacity=\"{}\"/>",(stop.position-first)/extent,stop.color[0],stop.color[1],stop.color[2],f64::from(stop.color[3])/255.));
                    }
                    self.defs.push_str("</linearGradient>");
                    self.body.push_str(&format!(
                        "<path d=\"{}\" fill=\"url(#{id})\"/>",
                        rounded(*rect, *radii)
                    ));
                }
                PaintCommand::Border {
                    rect,
                    widths,
                    color: c,
                } => self.border(*rect, *widths, [*c; 4]),
                PaintCommand::BorderEdges { rect, edges, .. } => self.border(
                    *rect,
                    edges.each_ref().map(|edge| {
                        if matches!(edge.style.as_str(), "none" | "hidden") {
                            0.
                        } else {
                            edge.width
                        }
                    }),
                    edges
                        .each_ref()
                        .map(|edge| parse_color(&edge.color).unwrap_or([0, 0, 0, 255])),
                ),
                PaintCommand::Image {
                    rect,
                    resource,
                    clip,
                    ..
                } => {
                    let snapshot = self.snapshot;
                    let source = snapshot.images.get(*resource).ok_or_else(|| {
                        RenderError::InvalidPrint("vector image resource missing".into())
                    })?;
                    let close = if let Some(clip) = clip {
                        let id = self.id();
                        self.defs.push_str(&format!("<clipPath id=\"{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",clip.x,clip.y,clip.width,clip.height));
                        self.body.push_str(&format!("<g clip-path=\"url(#{id})\">"));
                        true
                    } else {
                        false
                    };
                    self.body.push_str(&format!("<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:{};base64,{}\"/>",rect.x,rect.y,rect.width,rect.height,source.media_type.replace('&',"&amp;").replace('"',"&quot;").replace('<',"&lt;"),base64::engine::general_purpose::STANDARD.encode(&source.bytes)));
                    if close {
                        self.body.push_str("</g>");
                    }
                }
                PaintCommand::SvgImage {
                    rect,
                    resource,
                    clip,
                    environment,
                } => {
                    let snapshot = self.snapshot;
                    let source = snapshot.images.get(*resource).ok_or_else(|| {
                        RenderError::InvalidPrint("vector SVG resource missing".into())
                    })?;
                    let _svg_request = crate::svg_coverage::svg_request();
                    let tree =
                        crate::svg::parse_with_environment(source, self.snapshot, *environment)?;
                    let id = self.id();
                    let source = tree.to_string(&resvg::usvg::WriteOptions {
                        id_prefix: Some(format!("{id}_svg_")),
                        preserve_text: false,
                        ..Default::default()
                    });
                    let close = if let Some(clip) = clip {
                        let id = self.id();
                        self.defs.push_str(&format!("<clipPath id=\"{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",clip.x,clip.y,clip.width,clip.height));
                        self.body.push_str(&format!("<g clip-path=\"url(#{id})\">"));
                        true
                    } else {
                        false
                    };
                    self.body.push_str(&format!(
                        "<g transform=\"translate({} {}) scale({} {})\">{source}</g>",
                        rect.x,
                        rect.y,
                        rect.width / f64::from(tree.size().width()),
                        rect.height / f64::from(tree.size().height())
                    ));
                    if close {
                        self.body.push_str("</g>");
                    }
                }
                PaintCommand::Text { fragment, color: c } => {
                    if fragment.glyph_runs.is_empty() {
                        return Err(RenderError::InvalidPrint(
                            "vector print requires native shaped glyph runs".into(),
                        ));
                    }
                    for run in &fragment.glyph_runs {
                        if let Some((rect, fill)) = crate::decorations::background(fragment, run)? {
                            self.body.push_str(&format!(
                                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {}/>",
                                rect.x,
                                rect.y,
                                rect.width,
                                rect.height,
                                color(fill)
                            ));
                        }
                    }
                    for run in &fragment.glyph_runs {
                        if !run.scale_x.is_finite() || run.scale_x <= 0. {
                            return Err(RenderError::Font("invalid glyph horizontal scale".into()));
                        }
                        let snapshot = self.snapshot;
                        let source = snapshot.fonts.get(&run.font_key).ok_or_else(|| {
                            RenderError::Font(format!("missing vector font {}", run.font_key))
                        })?;
                        let fill = run.color.unwrap_or(*c);
                        for glyph in &run.glyphs {
                            let key = (
                                run.font_key.clone(),
                                glyph.id,
                                run.font_size.to_bits(),
                                run.embolden,
                                run.skew_degrees.to_bits(),
                                run.normalized_coords.clone(),
                            );
                            let id = if let Some(id) = self.glyphs.get(&key) {
                                id.clone()
                            } else {
                                let path = crate::glyph_outline::svg_path(source, run, glyph.id)?;
                                let id = self.id();
                                self.defs
                                    .push_str(&format!("<path id=\"{id}\" d=\"{path}\"/>"));
                                self.glyphs.insert(key, id.clone());
                                id
                            };
                            let angle = match run.orientation {
                                PaintGlyphOrientation::Upright => 0,
                                PaintGlyphOrientation::Clockwise => 90,
                                PaintGlyphOrientation::CounterClockwise => -90,
                            };
                            let mut opaque = fill;
                            opaque[3] = 255;
                            let stroke = if run.embolden {
                                format!(
                                    " stroke=\"rgb({},{},{})\" stroke-opacity=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
                                    fill[0],
                                    fill[1],
                                    fill[2],
                                    1.,
                                    run.font_size / 24.
                                )
                            } else {
                                String::new()
                            };
                            self.body.push_str(&format!("<g opacity=\"{}\"><use href=\"#{id}\" transform=\"translate({} {}) rotate({angle}) scale({} -1)\" {}{stroke}/></g>",f64::from(fill[3])/255.,fragment.rect.x+glyph.x,fragment.rect.y+glyph.y,run.scale_x,color(opaque)));
                        }
                    }
                }
                _ => unreachable!("groups handled before primitive emission"),
            }
            for _ in 0..=count {
                self.body.push_str("</g>");
            }
        }
        Ok(())
    }
    fn border(&mut self, r: CssRect, w: [f64; 4], colors: [[u8; 4]; 4]) {
        let [top, right, bottom, left] = w;
        let x = r.x;
        let y = r.y;
        let x2 = r.right();
        let y2 = r.bottom();
        let paths = [
            format!("M{x} {y}H{x2}L{} {}H{}Z", x2 - right, y + top, x + left),
            format!("M{x2} {y}V{y2}L{} {}V{}Z", x2 - right, y2 - bottom, y + top),
            format!(
                "M{x2} {y2}H{x}L{} {}H{}Z",
                x + left,
                y2 - bottom,
                x2 - right
            ),
            format!("M{x} {y2}V{y}L{} {}V{}Z", x + left, y + top, y2 - bottom),
        ];
        for side in 0..4 {
            if w[side] > 0. {
                self.body.push_str(&format!(
                    "<path d=\"{}\" {}/>",
                    paths[side],
                    color(colors[side])
                ));
            }
        }
    }
}
pub(crate) fn tree(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    options: RenderOptions,
) -> Result<resvg::usvg::Tree, RenderError> {
    let mut export = Export {
        snapshot,
        options,
        defs: String::new(),
        body: String::new(),
        next: 0,
        fallbacks: 0,
        fallback_pixels: 0,
        glyphs: std::collections::BTreeMap::new(),
    };
    export.body.push_str(&format!(
        "<rect width=\"{}\" height=\"{}\" {}/>",
        snapshot.viewport.width,
        snapshot.viewport.height,
        color(options.background)
    ));
    if scene.contract_version != RENDER_CONTRACT_VERSION {
        return Err(RenderError::InvalidPrint(
            "display-list contract mismatch".into(),
        ));
    }
    let identity = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    if has_backdrop(&scene.commands) {
        export.fallback(
            &PaintCommand::Projected {
                matrix: identity,
                clips: Vec::new(),
                commands: scene.commands.clone(),
            },
            identity,
            &[],
        )?;
    } else {
        export.emit(&scene.commands, identity, &[], 0)?;
    }
    let xml = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><defs>{}</defs>{}</svg>",
        snapshot.viewport.width, snapshot.viewport.height, export.defs, export.body
    );
    if xml.len() > MAX_XML {
        return Err(RenderError::InvalidPrint(
            "vector page source exceeds 16 MiB".into(),
        ));
    }
    let source = oryn_common::v4::PaintImageResource {
        svg_document_snapshot: false,
        content_type: None,
        url_aliases: Vec::new(),
        url: "oryn-print://native-page.svg".into(),
        media_type: "image/svg+xml".into(),
        bytes: xml.into_bytes(),
    };
    let tree = crate::svg::parse(&source, snapshot)?;
    crate::svg::validate_layers(&tree, options.scale, options.scale, options.max_pixels)?;
    Ok(tree)
}
pub(super) fn page(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    options: RenderOptions,
) -> Result<(pdf_writer::Chunk, pdf_writer::Ref), RenderError> {
    let tree = tree(snapshot, scene, options)?;
    svg2pdf::to_chunk(
        &tree,
        svg2pdf::ConversionOptions {
            embed_text: false,
            raster_scale: options.scale as f32,
            ..Default::default()
        },
    )
    .map_err(|e| RenderError::Encode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene(commands: Vec<PaintCommand>) -> DisplayList {
        DisplayList {
            contract_version: RENDER_CONTRACT_VERSION,
            width: 100,
            height: 100,
            commands,
            diagnostics: Vec::new(),
        }
    }
    #[test]
    fn coincident_hard_stops_do_not_collapse_vector_gradient_axis() {
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let gradient = gradients::parse(
            "linear-gradient(to right, red 50%, blue 50%)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        let commands = scene(vec![PaintCommand::LinearGradient {
            rect,
            radii: [0.; 8],
            gradient,
        }]);
        let mut export = Export {
            snapshot: &snapshot,
            options: RenderOptions::default(),
            defs: String::new(),
            body: String::new(),
            next: 0,
            fallbacks: 0,
            fallback_pixels: 0,
            glyphs: Default::default(),
        };
        export
            .emit(
                &commands.commands,
                [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ],
                &[],
                0,
            )
            .unwrap();
        assert!(export.defs.contains("x1=\"0\""));
        assert!(export.defs.contains("x2=\"100\""));
        assert_eq!(export.defs.matches("offset=\"0.5\"").count(), 2);
    }

    #[test]
    fn repeated_linear_background_retains_native_pdf_pattern_source() {
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let tile = BackgroundTile {
            rect: CssRect {
                width: 20.,
                height: 20.,
                ..rect
            },
            step_x: Some(25.),
            step_y: Some(25.),
            limit_x: None,
            limit_y: None,
        };
        let gradient = gradients::backgrounds("linear-gradient(red,blue)", tile.rect, "black", 16.)
            .unwrap()
            .remove(0);
        let commands = scene(vec![PaintCommand::Background {
            rect,
            radii: [0.; 8],
            tile,
            source: BackgroundSource::Gradient(gradient),
        }]);
        let tree = tree(&snapshot, &commands, RenderOptions::default()).unwrap();
        let svg = tree.to_string(&resvg::usvg::WriteOptions::default());
        assert!(svg.contains("<pattern"));
        assert!(svg.contains("linearGradient"));
        assert!(!svg.contains("data:image/png"));
    }

    #[test]
    fn shapes_and_native_glyphs_remain_pdf_vectors() {
        use oryn_common::v4::{PaintFontResource, PaintGlyph, PaintGlyphRun};
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 100.;
        snapshot.viewport.height = 100.;
        snapshot.fonts.insert(
            "native".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let rect = CssRect {
            x: 5.,
            y: 5.,
            width: 80.,
            height: 20.,
        };
        let run = PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: Some([255, 192, 203, 255]),
            decoration_rect: Some(CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 20.,
            }),
            scale_x: 1.,
            vertical: false,
            decorations: Vec::new(),
            orientation: PaintGlyphOrientation::Clockwise,
            embolden: false,
            skew_degrees: 0.,
            normalized_coords: Vec::new(),
            color: None,
            font_key: "native".into(),
            font_size: 20.,
            glyphs: vec![PaintGlyph {
                source: None,
                id: u32::from(font.lookup_glyph_index('A')),
                x: 20.,
                y: 20.,
                advance: 15.,
            }],
        };
        let mut run = run;
        let content = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 20.,
        };
        run.text_box = Some(oryn_common::v4::PaintTextBox {
            owner: 1,
            model: oryn_common::v4::BoxModel {
                content_box: content,
                padding_box: content,
                border_box: content,
                margin: Default::default(),
                padding: Default::default(),
                border: oryn_common::v4::EdgeSizes {
                    top: 1.,
                    right: 1.,
                    bottom: 1.,
                    left: 1.,
                },
            },
            style: std::collections::BTreeMap::from([
                (
                    "background-image".into(),
                    "linear-gradient(red,blue)".into(),
                ),
                ("border-top-style".into(), "solid".into()),
                ("border-top-color".into(), "green".into()),
            ]),
        });
        run.background = None;
        let commands = scene(vec![
            PaintCommand::FillRect {
                rect,
                color: [255, 0, 0, 255],
            },
            PaintCommand::Text {
                fragment: PaintTextFragment {
                    flow_line: None,
                    flow_rect: None,
                    clusters: Vec::new(),
                    text: "A".into(),
                    rect,
                    font_family: "native".into(),
                    font_size: 20.,
                    color: "black".into(),
                    glyph_runs: vec![run],
                },
                color: [0, 0, 0, 255],
            },
        ]);
        let (chunk, form) = page(&snapshot, &commands, RenderOptions::default()).unwrap();
        assert!(chunk.refs().any(|reference| reference == form));
        let pdf = String::from_utf8_lossy(chunk.as_bytes());
        assert!(pdf.contains("/Subtype /Form"));
        assert!(!pdf.contains("/Subtype /Image"));
    }
    #[test]
    fn only_unsupported_primitive_uses_bounded_raster_fallback() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 100.;
        snapshot.viewport.height = 100.;
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let gradient = crate::gradients::parse(
            "repeating-linear-gradient(red 0px, blue 10px)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        let commands = scene(vec![
            PaintCommand::LinearGradient {
                rect,
                gradient,
                radii: [0.; 8],
            },
            PaintCommand::FillRect {
                rect: CssRect {
                    width: 10.,
                    height: 10.,
                    ..rect
                },
                color: [0, 255, 0, 255],
            },
        ]);
        let (chunk, _) = page(&snapshot, &commands, RenderOptions::default()).unwrap();
        assert!(String::from_utf8_lossy(chunk.as_bytes()).contains("/Subtype /Image"));
        assert!(
            page(
                &snapshot,
                &commands,
                RenderOptions {
                    max_pixels: 9999,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
