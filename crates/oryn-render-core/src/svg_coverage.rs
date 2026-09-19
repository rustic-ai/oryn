//! Semantic inclusion follows intact native glyph alpha at output resolution.
//! This is a finite rendering policy, not mathematical vector containment.
use super::*;
use std::collections::BTreeMap;
const PIXELS: usize = 64 * 1024 * 1024;
const BYTES: usize = 512 * 1024 * 1024;
thread_local! {
 static TEXT_CAPTURE:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};
 static PAUSED:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};
 static CAPTURE:std::cell::RefCell<Option<CaptureState>>=const{std::cell::RefCell::new(None)};
 static SVG_REQUESTS:std::cell::RefCell<Vec<bool>>=const{std::cell::RefCell::new(Vec::new())};
}
// Test evidence records counts and normalized geometry, never glyph/document bytes.
#[cfg(test)]
thread_local! { static COVERAGE_DIAGNOSTICS: std::cell::RefCell<(Vec<String>, usize)> = const { std::cell::RefCell::new((Vec::new(), 0)) }; }
#[cfg(test)]
fn coverage_diagnostic(record: String) {
    COVERAGE_DIAGNOSTICS.with(|records| {
        let mut records = records.borrow_mut();
        if records.0.len() >= 128 {
            records.1 = records.1.saturating_add(1);
            return;
        }
        let mut end = record.len().min(2048);
        while !record.is_char_boundary(end) {
            end -= 1;
        }
        records.0.push(record[..end].to_owned());
    });
}
#[cfg(test)]
pub(super) fn take_coverage_diagnostics() -> (Vec<String>, usize) {
    COVERAGE_DIAGNOSTICS.with(|records| std::mem::take(&mut *records.borrow_mut()))
}
#[cfg(test)]
pub(super) struct CoverageDiagnosticsOnFailure;
#[cfg(test)]
impl Drop for CoverageDiagnosticsOnFailure {
    fn drop(&mut self) {
        if std::thread::panicking() {
            use std::io::Write as _;
            let (records, omitted) = take_coverage_diagnostics();
            if !records.is_empty() || omitted != 0 {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "native SVG coverage omitted={omitted} records={records:?}"
                );
            }
        }
    }
}
#[cfg(test)]
fn alpha_diagnostic_counts(before: &[u8], after: &[u8]) -> [usize; 3] {
    [
        before.chunks_exact(4).filter(|p| p[3] != 0).count(),
        after.chunks_exact(4).filter(|p| p[3] != 0).count(),
        before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a[3] != b[3])
            .count(),
    ]
}
type SvgTarget = (Vec<u64>, usize, String, usize, usize);
struct CaptureState {
    original: PaintSnapshot,
    path: Vec<u64>,
    cache: BTreeMap<(Vec<u64>, usize, [u64; 5]), String>,
    trees: BTreeMap<(Vec<u64>, usize, [u64; 5]), resvg::usvg::Tree>,
    bytes: usize,
    work: Budget,
    target: Option<SvgTarget>,
    allowed: std::collections::BTreeSet<SvgTarget>,
}
struct TextCapture;
impl TextCapture {
    fn enter(original: &PaintSnapshot) -> Result<Self, RenderError> {
        if TEXT_CAPTURE.with(|depth| depth.get() == 0) {
            crate::frames::validate(original)?;
            CAPTURE.with(|capture| {
                *capture.borrow_mut() = Some(CaptureState {
                    original: original.clone(),
                    path: Vec::new(),
                    cache: BTreeMap::new(),
                    trees: BTreeMap::new(),
                    bytes: 0,
                    work: Budget::default(),
                    target: None,
                    allowed: std::collections::BTreeSet::new(),
                })
            });
        }
        TEXT_CAPTURE.with(|depth| depth.set(depth.get() + 1));
        Ok(Self)
    }
}
impl Drop for TextCapture {
    fn drop(&mut self) {
        TEXT_CAPTURE.with(|depth| {
            depth.set(depth.get().saturating_sub(1));
            if depth.get() == 0 {
                CAPTURE.with(|capture| capture.borrow_mut().take());
            }
        });
    }
}
pub(super) fn text_capture_active() -> bool {
    TEXT_CAPTURE.with(|depth| depth.get() != 0) && PAUSED.with(|paused| paused.get() == 0)
}
struct Pause;
impl Pause {
    fn enter() -> Self {
        PAUSED.with(|paused| paused.set(paused.get() + 1));
        Self
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        PAUSED.with(|paused| paused.set(paused.get().saturating_sub(1)));
    }
}
pub(super) fn source_allowed(resource: usize, id: &str, start: usize, end: usize) -> bool {
    if !text_capture_active() {
        return true;
    }
    CAPTURE.with(|capture| {
        capture.borrow().as_ref().is_some_and(|capture| {
            capture
                .allowed
                .contains(&(capture.path.clone(), resource, id.into(), start, end))
        })
    })
}
struct SvgSelection(Option<SvgTarget>);
impl SvgSelection {
    fn enter(target: SvgTarget) -> Self {
        Self(CAPTURE.with(|capture| {
            capture
                .borrow_mut()
                .as_mut()
                .unwrap()
                .target
                .replace(target)
        }))
    }
}
impl Drop for SvgSelection {
    fn drop(&mut self) {
        CAPTURE.with(|capture| capture.borrow_mut().as_mut().unwrap().target = self.0.take());
    }
}
pub(super) struct FrameScope(bool);
pub(super) fn frame_scope(owner: u64) -> FrameScope {
    let active = text_capture_active();
    if active {
        CAPTURE.with(|capture| {
            if let Some(capture) = capture.borrow_mut().as_mut() {
                capture.path.push(owner);
            }
        });
    }
    FrameScope(active)
}
impl Drop for FrameScope {
    fn drop(&mut self) {
        if self.0 {
            CAPTURE.with(|capture| {
                if let Some(capture) = capture.borrow_mut().as_mut() {
                    capture.path.pop();
                }
            });
        }
    }
}
pub(super) struct SvgRequest(bool);
pub(super) fn svg_request() -> SvgRequest {
    let active = text_capture_active();
    if active {
        SVG_REQUESTS.with(|requests| requests.borrow_mut().push(true));
    }
    SvgRequest(active)
}
impl Drop for SvgRequest {
    fn drop(&mut self) {
        if self.0 {
            SVG_REQUESTS.with(|requests| requests.borrow_mut().pop());
        }
    }
}
pub(super) fn take_svg_request() -> bool {
    if !text_capture_active() {
        return false;
    }
    SVG_REQUESTS.with(|requests| {
        requests
            .borrow_mut()
            .last_mut()
            .is_some_and(|pending| std::mem::replace(pending, false))
    })
}
#[derive(Default)]
pub(super) struct Budget {
    pixels: usize,
    bytes: usize,
    clusters: usize,
}
impl Budget {
    fn charge(&mut self, n: usize, kind: u8) -> Result<(), RenderError> {
        fn add(budget: &mut Budget, n: usize, kind: u8) -> Result<(), RenderError> {
            let (value, limit, name) = match kind {
                0 => (&mut budget.pixels, PIXELS, "pixel work"),
                1 => (&mut budget.bytes, BYTES, "source work"),
                _ => (&mut budget.clusters, 16384, "glyph clusters"),
            };
            *value = value
                .checked_add(n)
                .filter(|value| *value <= limit)
                .ok_or_else(|| {
                    RenderError::InvalidPrint(format!(
                        "SVG semantic coverage exceeds aggregate {name}"
                    ))
                })?;
            Ok(())
        }
        add(self, n, kind)?;
        if text_capture_active() {
            CAPTURE.with(|capture| {
                if let Some(capture) = capture.borrow_mut().as_mut() {
                    add(&mut capture.work, n, kind)
                } else {
                    Ok(())
                }
            })?;
        }
        Ok(())
    }
    fn pixels(&mut self, n: usize) -> Result<(), RenderError> {
        self.charge(n, 0)
    }
    fn bytes(&mut self, n: usize) -> Result<(), RenderError> {
        self.charge(n, 1)
    }
    pub(super) fn cluster(&mut self) -> Result<(), RenderError> {
        self.charge(1, 2)
    }
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
fn original_xml(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
) -> Result<Option<(usize, String)>, RenderError> {
    if !text_capture_active() {
        return Ok(None);
    }
    let Some(index) = snapshot
        .images
        .iter()
        .position(|image| image.url == source.url && image.bytes == source.bytes)
    else {
        return Ok(None);
    };
    let (key, original, cached) = CAPTURE.with(|capture| -> Result<_, RenderError> {
        let capture = capture.borrow();
        let capture = capture.as_ref().ok_or_else(|| {
            RenderError::InvalidPrint("missing semantic capture authority".into())
        })?;
        let key = (
            capture.path.clone(),
            index,
            SvgEnvironment::current(snapshot).key(),
        );
        if let Some(cached) = capture.cache.get(&key) {
            return Ok((key, None, Some(cached.clone())));
        }
        let mut owner = &capture.original;
        for frame in &capture.path {
            owner = &owner
                .frames
                .iter()
                .find(|candidate| candidate.owner_node == *frame)
                .ok_or_else(|| {
                    RenderError::InvalidPrint("semantic frame authority changed".into())
                })?
                .snapshot;
        }
        let image = owner.images.get(index).ok_or_else(|| {
            RenderError::InvalidPrint("semantic SVG resource owner changed".into())
        })?;
        if image.url != source.url || image.bytes != source.bytes {
            return Err(RenderError::InvalidPrint(
                "semantic SVG resource identity changed".into(),
            ));
        }
        Ok((key, Some(owner.clone()), None))
    })?;
    if let Some(cached) = cached {
        return Ok(Some((index, cached)));
    }
    let original = original.unwrap();
    let _pause = Pause::enter();
    let tree = crate::svg::parse(&original.images[index], &original)?;
    let data = tree.to_string(&resvg::usvg::WriteOptions {
        preserve_text: false,
        ..Default::default()
    });
    if data.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "semantic original SVG exceeds source bounds".into(),
        ));
    }
    CAPTURE.with(|capture| -> Result<(), RenderError> {
        let mut capture = capture.borrow_mut();
        let capture = capture.as_mut().unwrap();
        capture.bytes = capture
            .bytes
            .checked_add(data.len())
            .filter(|bytes| *bytes <= 64 * 1024 * 1024)
            .ok_or_else(|| {
                RenderError::InvalidPrint("semantic original effect cache exceeds 64 MiB".into())
            })?;
        capture.trees.insert(key.clone(), tree.clone());
        capture.cache.insert(key, data.clone());
        Ok(())
    })?;
    Ok(Some((index, data)))
}
pub(super) fn semantic_tree(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
) -> Result<resvg::usvg::Tree, RenderError> {
    if let Some((index, _)) = original_xml(source, snapshot)? {
        CAPTURE.with(|capture| {
            let capture = capture.borrow();
            let capture = capture.as_ref().ok_or_else(|| {
                RenderError::InvalidPrint("missing semantic tree authority".into())
            })?;
            capture
                .trees
                .get(&(
                    capture.path.clone(),
                    index,
                    SvgEnvironment::current(snapshot).key(),
                ))
                .cloned()
                .ok_or_else(|| RenderError::InvalidPrint("missing native semantic SVG tree".into()))
        })
    } else {
        crate::svg::parse(source, snapshot)
    }
}

/// Keep selected glyph payloads alive until original normalized SVG effects are
/// restored. Native HTML's own clips are emitted later by its vector exporter.
pub(super) fn neutralize_svg_effects(data: Vec<u8>) -> Result<Vec<u8>, RenderError> {
    let text =
        std::str::from_utf8(&data).map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let xml = roxmltree::Document::parse(text)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let mut replacements = Vec::new();
    for node in xml.descendants().filter(|node| {
        node.is_element() && node.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
    }) {
        let start = node.range().start;
        let mut quote = None;
        let mut end = None;
        for (index, byte) in text.as_bytes()[start..].iter().copied().enumerate() {
            if let Some(active) = quote {
                if byte == active {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'>' {
                end = Some(start + index + 1);
                break;
            }
        }
        let end = end
            .ok_or_else(|| RenderError::InvalidPrint("unterminated semantic SVG element".into()))?;
        let tag = &text[start + 1..end - 1];
        let name = tag
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or(node.tag_name().name());
        let mut replacement = format!("<{name}");
        for namespace in node.namespaces() {
            replacement.push_str(&format!(
                " {}=\"{}\"",
                namespace
                    .name()
                    .map(|name| format!("xmlns:{name}"))
                    .unwrap_or_else(|| "xmlns".into()),
                esc(namespace.uri())
            ));
        }
        for attribute in node.attributes() {
            if attribute.name() == "style" && attribute.namespace().is_none() {
                continue;
            }
            let name = attribute
                .namespace()
                .map(|uri| {
                    let prefix = if uri == "http://www.w3.org/XML/1998/namespace" {
                        Some("xml")
                    } else {
                        node.lookup_prefix(uri)
                    };
                    prefix
                        .map(|prefix| format!("{prefix}:{}", attribute.name()))
                        .unwrap_or_else(|| attribute.name().into())
                })
                .unwrap_or_else(|| attribute.name().into());
            replacement.push_str(&format!(" {name}=\"{}\"", esc(attribute.value())));
        }
        replacement.push_str(&format!(
            " style=\"{};clip-path:none!important;mask:none!important\"{}>",
            esc(node.attribute("style").unwrap_or("")),
            if tag.trim_end().ends_with('/') {
                "/"
            } else {
                ""
            }
        ));
        replacements.push((start..end, replacement));
    }
    let mut output = text.to_owned();
    for (range, replacement) in replacements.into_iter().rev() {
        output.replace_range(range, &replacement);
    }
    if output.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "semantic SVG neutralization exceeds source bounds".into(),
        ));
    }
    Ok(output.into_bytes())
}
pub(super) fn restore_svg_effects(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    selected: resvg::usvg::Tree,
) -> Result<resvg::usvg::Tree, RenderError> {
    let Some((resource, original)) = original_xml(source, snapshot)? else {
        return Ok(selected);
    };
    let original_xml = roxmltree::Document::parse(&original)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let mut prefix = "oryn-selected-glyph_".to_owned();
    while original_xml
        .descendants()
        .filter_map(|node| node.attribute("id"))
        .any(|id| id.starts_with(&prefix))
    {
        prefix.push('_');
    }
    let selected = selected.to_string(&resvg::usvg::WriteOptions {
        id_prefix: Some(prefix.clone()),
        preserve_text: false,
        ..Default::default()
    });
    let selected_xml = roxmltree::Document::parse(&selected)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let raw = crate::svg::xml_bytes_with_type(&source.bytes, source.content_type.as_deref())
        .map_err(RenderError::InvalidPrint)?;
    let raw =
        std::str::from_utf8(&raw).map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let raw = roxmltree::Document::parse(raw)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    let selected_target = CAPTURE.with(|capture| {
        capture.borrow().as_ref().and_then(|capture| {
            capture
                .target
                .as_ref()
                .filter(|target| target.0 == capture.path && target.1 == resource)
                .cloned()
        })
    });
    if let Some((_, _, id, start, end)) = selected_target {
        let tree = CAPTURE
            .with(|capture| {
                let capture = capture.borrow();
                let capture = capture.as_ref().unwrap();
                capture
                    .trees
                    .get(&(
                        capture.path.clone(),
                        resource,
                        SvgEnvironment::current(snapshot).key(),
                    ))
                    .cloned()
            })
            .ok_or_else(|| RenderError::InvalidPrint("missing selected native SVG tree".into()))?;
        if let Some(glyphs) = crate::svg_text::cluster_tree(&tree, &id, start..end) {
            let glyphs = glyphs.to_string(&resvg::usvg::WriteOptions {
                id_prefix: Some(prefix),
                preserve_text: false,
                ..Default::default()
            });
            let output = probe(&original_xml, &id, &glyphs, false)?;
            let _pause = Pause::enter();
            return crate::svg::parse(
                &oryn_common::v4::PaintImageResource {
                    content_type: None,
                    bytes: output.into_bytes(),
                    ..source.clone()
                },
                snapshot,
            );
        }
    }
    let root = original_xml.root_element();
    let mut output = open(root, false);
    for (document, text) in [
        (&original_xml, original.as_str()),
        (&selected_xml, selected.as_str()),
    ] {
        for child in document
            .root_element()
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "defs")
        {
            output.push_str(&text[child.range()]);
        }
    }
    for (ordinal, _) in raw
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "foreignObject"
                && !node.ancestors().skip(1).any(|parent| {
                    parent.is_element() && parent.tag_name().name() == "foreignObject"
                })
        })
        .enumerate()
    {
        let id = crate::svg_foreign::viewport_id(&raw, resource, ordinal);
        let Some(original_node) = original_xml
            .descendants()
            .find(|node| node.attribute("id") == Some(id.as_str()))
        else {
            continue;
        };
        let selected_id = format!("{prefix}{id}");
        let Some(selected_node) = selected_xml
            .descendants()
            .find(|node| node.attribute("id") == Some(selected_id.as_str()))
        else {
            continue;
        };
        let mut chain = original_node
            .ancestors()
            .take_while(|node| *node != root)
            .collect::<Vec<_>>();
        chain.reverse();
        if chain.len() > 128 {
            return Err(RenderError::InvalidPrint(
                "semantic normalized ancestor depth".into(),
            ));
        }
        for ancestor in &chain {
            output.push_str(&open_flags(*ancestor, false, false));
        }
        for child in selected_node.children() {
            output.push_str(&selected[child.range()]);
        }
        for ancestor in chain.iter().rev() {
            output.push_str(&format!("</{}>", ancestor.tag_name().name()));
        }
    }
    output.push_str("</svg>");
    if output.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "restored semantic SVG exceeds source bounds".into(),
        ));
    }
    let _pause = Pause::enter();
    crate::svg::parse(
        &oryn_common::v4::PaintImageResource {
            content_type: None,
            bytes: output.into_bytes(),
            ..source.clone()
        },
        snapshot,
    )
}
fn open(node: roxmltree::Node<'_, '_>, unmasked: bool) -> String {
    open_flags(node, unmasked, true)
}
fn open_flags(node: roxmltree::Node<'_, '_>, unmasked: bool, keep_id: bool) -> String {
    let mut result = format!("<{}", node.tag_name().name());
    if node.tag_name().name() == "svg" {
        result.push_str(
            " xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
        );
    }
    for attribute in node.attributes() {
        if !keep_id && attribute.name() == "id"
            || unmasked && matches!(attribute.name(), "clip-path" | "mask")
        {
            continue;
        }
        let name = if attribute.namespace() == Some("http://www.w3.org/1999/xlink") {
            format!("xlink:{}", attribute.name())
        } else {
            attribute.name().to_owned()
        };
        result.push_str(&format!(" {name}=\"{}\"", esc(attribute.value())));
    }
    result.push('>');
    result
}
/// Usvg's normalized serialization has already resolved objectBoundingBox units
/// against the complete original paint tree. Keep those definitions fixed when
/// replacing the foreign viewport by a selected glyph's native vector paint.
fn probe(
    xml: &roxmltree::Document<'_>,
    id: &str,
    glyphs: &str,
    unmasked: bool,
) -> Result<String, RenderError> {
    let root = xml.root_element();
    let target = xml
        .descendants()
        .find(|node| node.attribute("id") == Some(id))
        .ok_or_else(|| {
            RenderError::InvalidPrint("normalized SVG lost semantic viewport identity".into())
        })?;
    let mut ancestors = target
        .ancestors()
        .take_while(|node| *node != root)
        .collect::<Vec<_>>();
    ancestors.reverse();
    if ancestors.len() > 128 {
        return Err(RenderError::InvalidPrint(
            "semantic SVG ancestor depth".into(),
        ));
    }
    let mut result = open(root, unmasked);
    for child in root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "defs")
    {
        result.push_str(&xml.input_text()[child.range()]);
    }
    for ancestor in &ancestors {
        result.push_str(&open(*ancestor, unmasked));
    }
    result.push_str(glyphs);
    for ancestor in ancestors.iter().rev() {
        result.push_str(&format!("</{}>", ancestor.tag_name().name()));
    }
    result.push_str("</svg>");
    if result.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "semantic SVG probe exceeds source limit".into(),
        ));
    }
    Ok(result)
}
pub(super) fn text_only(commands: &[PaintCommand]) -> Vec<PaintCommand> {
    commands
        .iter()
        .filter_map(|command| match command {
            PaintCommand::SvgImage {
                rect,
                clip,
                resource,
                environment,
            } => Some(PaintCommand::SvgImage {
                rect: *rect,
                clip: *clip,
                resource: *resource,
                environment: *environment,
            }),
            PaintCommand::EmbeddedDocument { rect, frame } => {
                Some(PaintCommand::EmbeddedDocument {
                    rect: *rect,
                    frame: *frame,
                })
            }
            PaintCommand::Text { fragment, color } => Some(PaintCommand::Text {
                fragment: fragment.clone(),
                color: *color,
            }),
            PaintCommand::Projected {
                matrix,
                clips,
                commands,
            } => {
                let commands = text_only(commands);
                (!commands.is_empty()).then(|| PaintCommand::Projected {
                    matrix: *matrix,
                    clips: clips.clone(),
                    commands,
                })
            }
            PaintCommand::Mask { layers, commands } => {
                let commands = text_only(commands);
                (!commands.is_empty()).then(|| PaintCommand::Mask {
                    layers: layers.clone(),
                    commands,
                })
            }
            PaintCommand::Filter {
                reference_box,
                source,
                current_color,
                plane,
                commands,
                environment,
            } => {
                let commands = text_only(commands);
                (!commands.is_empty()).then(|| PaintCommand::Filter {
                    environment: *environment,
                    reference_box: *reference_box,
                    source: source.clone(),
                    current_color: current_color.clone(),
                    plane: *plane,
                    commands,
                })
            }
            PaintCommand::Opacity { opacity, commands } => {
                let commands = text_only(commands);
                (!commands.is_empty()).then(|| PaintCommand::Opacity {
                    opacity: *opacity,
                    commands,
                })
            }
            _ => None,
        })
        .collect()
}
fn select(fragment: &PaintTextFragment, start: usize, end: usize) -> Option<PaintTextFragment> {
    let text = fragment.text.get(start..end)?;
    let clusters = fragment
        .clusters
        .iter()
        .filter(|cluster| cluster.start >= start && cluster.end <= end)
        .collect::<Vec<_>>();
    let rect = clusters.iter().map(|cluster| cluster.rect).reduce(|a, b| {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        CssRect {
            x,
            y,
            width: a.right().max(b.right()) - x,
            height: a.bottom().max(b.bottom()) - y,
        }
    })?;
    let mut selected = fragment.clone();
    selected.text = text.into();
    selected.rect = rect;
    selected.clusters = clusters
        .into_iter()
        .map(|cluster| {
            let mut cluster = cluster.clone();
            cluster.start -= start;
            cluster.end -= start;
            cluster
        })
        .collect();
    for run in &mut selected.glyph_runs {
        run.line_box = None;
        run.text_box = None;
        run.background = None;
        run.decorations.clear();
        run.glyphs.retain(|glyph| {
            glyph
                .source
                .as_ref()
                .is_some_and(|source| source.start >= start && source.end <= end)
        });
        for glyph in &mut run.glyphs {
            glyph.x += fragment.rect.x - rect.x;
            glyph.y += fragment.rect.y - rect.y;
            if let Some(source) = &mut glyph.source {
                source.start -= start;
                source.end -= start;
            }
        }
        if let Some(bounds) = &mut run.decoration_rect {
            bounds.x += fragment.rect.x - rect.x;
            bounds.y += fragment.rect.y - rect.y;
        }
    }
    selected.glyph_runs.retain(|run| !run.glyphs.is_empty());
    Some(selected)
}
type Target = (Vec<u64>, Option<usize>, u64);
fn clear_nodes(nodes: &mut [oryn_common::v4::PaintNodeSnapshot]) {
    for node in nodes {
        node.text.clear();
        if let Some(projection) = &mut node.projection {
            projection.text.clear();
        }
    }
}
fn clear_text(local: &mut PaintSnapshot) {
    clear_nodes(&mut local.nodes);
    for object in &mut local.svg_foreign_objects {
        clear_nodes(&mut object.nodes);
    }
    for frame in &mut local.frames {
        clear_text(&mut frame.snapshot);
    }
}
fn with_text(
    local: &PaintSnapshot,
    path: &[u64],
    object: Option<usize>,
    owner: u64,
    fragment: PaintTextFragment,
) -> Result<PaintSnapshot, RenderError> {
    let mut local = local.clone();
    clear_text(&mut local);
    let mut target = &mut local;
    for owner in path {
        target = &mut target
            .frames
            .iter_mut()
            .find(|frame| frame.owner_node == *owner)
            .ok_or_else(|| RenderError::InvalidPrint("semantic frame owner missing".into()))?
            .snapshot;
    }
    let nodes = if let Some(index) = object {
        &mut target
            .svg_foreign_objects
            .get_mut(index)
            .ok_or_else(|| RenderError::InvalidPrint("semantic foreign object missing".into()))?
            .nodes
    } else {
        &mut target.nodes
    };
    let node = nodes
        .iter_mut()
        .find(|node| node.node_id == owner)
        .ok_or_else(|| RenderError::InvalidPrint("semantic text owner missing".into()))?;
    node.text.push(fragment.clone());
    if let Some(projection) = &mut node.projection {
        projection.text.push(fragment);
    }
    Ok(local)
}
fn native_vectors(local: &PaintSnapshot, prefix: String) -> Result<String, RenderError> {
    let options = RenderOptions {
        background: [0; 4],
        scale: local.viewport.device_pixel_ratio,
        ..Default::default()
    };
    let mut scene = build_display_list(local, options)?;
    scene.commands = text_only(&scene.commands);
    let ink =
        crate::projected::ink_bounds(local, &scene.commands, options.scale)?.unwrap_or(CssRect {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
        });
    let left = ink.x.min(0.).floor();
    let top = ink.y.min(0.).floor();
    let mut local = local.clone();
    local.viewport.width = ink.right().max(local.viewport.width) - left;
    local.viewport.height = ink.bottom().max(local.viewport.height) - top;
    let mut matrix = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    matrix[12] = -left;
    matrix[13] = -top;
    scene.commands = vec![PaintCommand::Projected {
        matrix,
        clips: Vec::new(),
        commands: scene.commands,
    }];
    let width = (local.viewport.width * options.scale).ceil();
    let height = (local.viewport.height * options.scale).ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || width * height > crate::svg::MAX_PIXELS as f64
    {
        return Err(RenderError::InvalidPrint(
            "semantic native vector surface exceeds bounds".into(),
        ));
    }
    scene.width = width as u32;
    scene.height = height as u32;
    let tree = crate::print::vector::tree(&local, &scene, options)?;
    let data = tree.to_string(&resvg::usvg::WriteOptions {
        id_prefix: Some(prefix),
        preserve_text: false,
        ..Default::default()
    });
    Ok(format!(
        "<g transform=\"translate({left} {top})\">{data}</g>"
    ))
}
fn intact_alpha(before: &[u8], after: &[u8]) -> bool {
    before.len() == after.len()
        && before.chunks_exact(4).any(|pixel| pixel[3] != 0)
        && before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .all(|(a, b)| a[3] == b[3])
}
pub(super) fn covered(
    local: &PaintSnapshot,
    xml: &roxmltree::Document<'_>,
    id: &str,
    glyphs: &str,
    scale: f64,
    budget: &mut Budget,
) -> Result<bool, RenderError> {
    if !scale.is_finite() || scale <= 0. || scale > f32::MAX as f64 {
        return Err(RenderError::InvalidPrint(
            "invalid semantic coverage resolution".into(),
        ));
    }
    let before = probe(xml, id, glyphs, true)?;
    let after = probe(xml, id, glyphs, false)?;
    budget.bytes(before.len().saturating_add(after.len()))?;
    let resource = |bytes: String| oryn_common::v4::PaintImageResource {
        svg_document_snapshot: false,
        content_type: None,
        url_aliases: Vec::new(),
        url: "oryn-print://semantic-coverage.svg".into(),
        media_type: "image/svg+xml".into(),
        bytes: bytes.into_bytes(),
    };
    let before = crate::svg::parse(&resource(before), local)?;
    let after = crate::svg::parse(&resource(after), local)?;
    budget.pixels(
        crate::svg::layer_pixels(&before, scale, scale, crate::svg::MAX_PIXELS)?.saturating_add(
            crate::svg::layer_pixels(&after, scale, scale, crate::svg::MAX_PIXELS)?,
        ),
    )?;
    let bounds = before.root().abs_layer_bounding_box();
    let left = (f64::from(bounds.x()) * scale).floor() - 1.;
    let top = (f64::from(bounds.y()) * scale).floor() - 1.;
    let width = (f64::from(bounds.right()) * scale).ceil() - left + 1.;
    let height = (f64::from(bounds.bottom()) * scale).ceil() - top + 1.;
    let pixels = width * height;
    if !pixels.is_finite()
        || pixels <= 0.
        || pixels > crate::svg::MAX_PIXELS as f64
        || width > u32::MAX as f64
        || height > u32::MAX as f64
    {
        return Err(RenderError::InvalidPrint(
            "semantic glyph coverage surface exceeds bounds".into(),
        ));
    }
    budget.pixels((pixels as usize).saturating_mul(2))?;
    let render = |tree: &resvg::usvg::Tree| -> Result<Vec<u8>, RenderError> {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width as u32, height as u32)
            .ok_or_else(|| RenderError::InvalidPrint("semantic coverage allocation".into()))?;
        resvg::render(
            tree,
            resvg::tiny_skia::Transform::from_row(
                scale as f32,
                0.,
                0.,
                scale as f32,
                -left as f32,
                -top as f32,
            ),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap.take())
    };
    let before_pixels = render(&before)?;
    let after_pixels = render(&after)?;
    #[cfg(test)]
    {
        let chain = xml
            .descendants()
            .find(|node| node.attribute("id") == Some(id))
            .map(|node| {
                node.ancestors()
                    .take(16)
                    .map(|ancestor| {
                        let transform = ancestor
                            .attribute("transform")
                            .unwrap_or("")
                            .chars()
                            .take(160)
                            .collect::<String>();
                        (
                            ancestor
                                .tag_name()
                                .name()
                                .chars()
                                .take(24)
                                .collect::<String>(),
                            transform,
                            ancestor.attribute("clip-path").is_some(),
                            ancestor.attribute("mask").is_some(),
                            ancestor
                                .attribute("opacity")
                                .and_then(|value| value.parse::<f64>().ok()),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        coverage_diagnostic(format!(
            "coverage pixels={:?} bytes={}/{} surface=({left},{top},{width},{height}) scale={scale} ancestors={chain:?}",
            alpha_diagnostic_counts(&before_pixels, &after_pixels),
            before_pixels.len(),
            after_pixels.len()
        ));
    }
    Ok(intact_alpha(&before_pixels, &after_pixels))
}
pub(super) fn retain(
    local: &PaintSnapshot,
    tree: &resvg::usvg::Tree,
    id: &str,
    scale: f64,
    budget: &mut Budget,
) -> Result<Vec<crate::print::text_layer::PdfText>, RenderError> {
    let nested_capture = text_capture_active();
    let expanded = if nested_capture {
        None
    } else {
        Some(crate::svg_instances::expand(local)?)
    };
    let local = expanded.as_ref().unwrap_or(local);
    let _capture = TextCapture::enter(local)?;
    fn candidates<'a>(
        local: &'a PaintSnapshot,
        object: Option<usize>,
        path: &mut Vec<u64>,
        seen: &mut std::collections::BTreeSet<(Vec<u64>, Option<usize>)>,
        output: &mut Vec<(Target, &'a PaintTextFragment)>,
        svg: &mut Vec<SvgTarget>,
        depth: usize,
    ) -> Result<(), RenderError> {
        if depth > 32 || !seen.insert((path.clone(), object)) {
            return Err(RenderError::InvalidPrint(
                "masked semantic capture cycle or repeated view".into(),
            ));
        }
        let nodes = if let Some(index) = object {
            &local.svg_foreign_objects[index].nodes
        } else {
            &local.nodes
        };
        for node in nodes {
            let fragments = node
                .projection
                .as_ref()
                .map(|projection| projection.text.as_slice())
                .unwrap_or(&node.text);
            for fragment in fragments {
                output.push(((path.clone(), object, node.node_id), fragment));
                if output.len() > 32768 {
                    return Err(RenderError::InvalidPrint(
                        "masked semantic fragment limit".into(),
                    ));
                }
            }
            if let Some(image) = node.image_resource {
                let scopes = path
                    .iter()
                    .map(|owner| frame_scope(*owner))
                    .collect::<Vec<_>>();
                if let Some(source) = local
                    .images
                    .get(image)
                    .filter(|source| source.media_type == "image/svg+xml")
                {
                    let content = node
                        .projection
                        .as_ref()
                        .map(|projection| projection.box_model.content_box)
                        .or_else(|| {
                            node.geometry
                                .box_model
                                .as_ref()
                                .map(|model| model.content_box)
                        })
                        .unwrap_or(CssRect {
                            x: 0.,
                            y: 0.,
                            width: local.viewport.width,
                            height: local.viewport.height,
                        });
                    let initial = SvgEnvironment::for_style(&node.style, content, local.viewport);
                    let intrinsic = crate::svg::parse_with_environment(source, local, initial)?;
                    let rect = crate::image_placement(
                        content,
                        intrinsic.size().width().ceil() as u32,
                        intrinsic.size().height().ceil() as u32,
                        &node.style,
                        local.viewport,
                    );
                    let _environment =
                        SvgEnvironment::for_style(&node.style, rect, local.viewport).enter();
                    let tree = semantic_tree(source, local)?;
                    let owner_path =
                        CAPTURE.with(|capture| capture.borrow().as_ref().unwrap().path.clone());
                    for (id, range) in crate::svg_text::ranges(&tree)? {
                        svg.push((owner_path.clone(), image, id, range.start, range.end));
                        if svg.len() > 16384 {
                            return Err(RenderError::InvalidPrint(
                                "masked SVG source cluster count".into(),
                            ));
                        }
                    }
                    drop(scopes);
                    for (index, _) in local
                        .svg_foreign_objects
                        .iter()
                        .enumerate()
                        .filter(|(_, object)| object.image_resource == image)
                    {
                        candidates(local, Some(index), path, seen, output, svg, depth + 1)?;
                    }
                }
            }
            for frame in local
                .frames
                .iter()
                .filter(|frame| frame.owner_node == node.node_id)
            {
                path.push(frame.owner_node);
                candidates(&frame.snapshot, None, path, seen, output, svg, depth + 1)?;
                path.pop();
            }
        }
        Ok(())
    }
    let mut candidates_list = Vec::new();
    let mut svg_candidates = Vec::new();
    candidates(
        local,
        None,
        &mut Vec::new(),
        &mut std::collections::BTreeSet::new(),
        &mut candidates_list,
        &mut svg_candidates,
        0,
    )?;
    let normalized = tree.to_string(&resvg::usvg::WriteOptions {
        preserve_text: false,
        ..Default::default()
    });
    if normalized.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "semantic normalized SVG exceeds source limit".into(),
        ));
    }
    let xml = roxmltree::Document::parse(&normalized)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    budget.bytes(normalized.len())?;
    let mut retained = BTreeMap::<Target, Vec<PaintTextFragment>>::new();
    for ((path, object, owner), fragment) in candidates_list {
        if !fragment.text.trim().is_empty()
            && !fragment.glyph_runs.is_empty()
            && fragment.clusters.is_empty()
        {
            return Err(RenderError::InvalidPrint(
                "semantic SVG text lacks native cluster provenance".into(),
            ));
        }
        let mut clusters = fragment.clusters.iter().collect::<Vec<_>>();
        clusters.sort_by_key(|cluster| (cluster.start, cluster.end));
        clusters.dedup_by_key(|cluster| (cluster.start, cluster.end));
        let mut accepted = vec![false; clusters.len()];
        for (index, cluster) in clusters.iter().enumerate() {
            let Some(text) = fragment.text.get(cluster.start..cluster.end) else {
                return Err(RenderError::InvalidPrint(
                    "invalid native glyph source range".into(),
                ));
            };
            if text.trim().is_empty() {
                continue;
            }
            budget.cluster()?;
            let Some(selected) = select(fragment, cluster.start, cluster.end) else {
                continue;
            };
            if selected.glyph_runs.is_empty() {
                return Err(RenderError::InvalidPrint(
                    "semantic SVG text lacks native glyph provenance".into(),
                ));
            }
            let selected = with_text(local, &path, object, owner, selected)?;
            let mut prefix = format!("oryn-semantic-glyph-{}_", budget.clusters);
            while xml
                .descendants()
                .filter_map(|node| node.attribute("id"))
                .any(|id| id.starts_with(&prefix))
            {
                prefix.push('_');
            }
            #[cfg(test)]
            coverage_diagnostic(format!(
                "candidate owner={owner} object={object:?} frame_depth={} source={}..{} fragment={:?} glyph_runs={}",
                path.len(),
                cluster.start,
                cluster.end,
                fragment.rect,
                fragment.glyph_runs.len()
            ));
            let glyphs = native_vectors(&selected, prefix)?;
            accepted[index] = covered(&selected, &xml, id, &glyphs, scale, budget)?;
        }
        // Whitespace has no glyph ink. Keep it only between retained neighboring
        // clusters; never let it make an otherwise absent line searchable.
        for index in 0..clusters.len() {
            if fragment
                .text
                .get(clusters[index].start..clusters[index].end)
                .is_some_and(|text| text.trim().is_empty())
                && accepted[..index].iter().any(|value| *value)
                && accepted[index + 1..].iter().any(|value| *value)
            {
                accepted[index] = true;
            }
        }
        let mut spans = Vec::<(usize, usize)>::new();
        for (cluster, accepted) in clusters.into_iter().zip(accepted) {
            if !accepted {
                continue;
            }
            if let Some((_, end)) = spans.last_mut().filter(|(_, end)| *end == cluster.start) {
                *end = cluster.end;
            } else {
                spans.push((cluster.start, cluster.end));
            }
        }
        for (start, end) in spans {
            if let Some(fragment) = select(fragment, start, end) {
                retained
                    .entry((path.clone(), object, owner))
                    .or_default()
                    .push(fragment);
            }
        }
    }
    for target in svg_candidates {
        if nested_capture
            && !CAPTURE.with(|capture| capture.borrow().as_ref().unwrap().allowed.contains(&target))
        {
            continue;
        }
        budget.cluster()?;
        let mut selected = local.clone();
        clear_text(&mut selected);
        let _selection = SvgSelection::enter(target.clone());
        let mut prefix = format!("oryn-semantic-svg-{}_", budget.clusters);
        while xml
            .descendants()
            .filter_map(|node| node.attribute("id"))
            .any(|id| id.starts_with(&prefix))
        {
            prefix.push('_');
        }
        let glyphs = native_vectors(&selected, prefix)?;
        if covered(&selected, &xml, id, &glyphs, scale, budget)? {
            CAPTURE.with(|capture| {
                capture
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .allowed
                    .insert(target)
            });
        }
    }
    fn install_nodes(
        nodes: &mut [oryn_common::v4::PaintNodeSnapshot],
        path: &[u64],
        object: Option<usize>,
        retained: &mut BTreeMap<Target, Vec<PaintTextFragment>>,
    ) {
        for node in nodes {
            node.text = retained
                .remove(&(path.to_vec(), object, node.node_id))
                .unwrap_or_default();
            if let Some(projection) = &mut node.projection {
                projection.text = node.text.clone();
            }
        }
    }
    fn install(
        local: &mut PaintSnapshot,
        path: &mut Vec<u64>,
        retained: &mut BTreeMap<Target, Vec<PaintTextFragment>>,
    ) {
        install_nodes(&mut local.nodes, path, None, retained);
        for (index, object) in local.svg_foreign_objects.iter_mut().enumerate() {
            install_nodes(&mut object.nodes, path, Some(index), retained);
        }
        for frame in &mut local.frames {
            path.push(frame.owner_node);
            install(&mut frame.snapshot, path, retained);
            path.pop();
        }
    }
    let mut result = local.clone();
    install(&mut result, &mut Vec::new(), &mut retained);
    let mut scene = build_display_list(
        &result,
        RenderOptions {
            background: [0; 4],
            scale: local.viewport.device_pixel_ratio,
            ..Default::default()
        },
    )?;
    scene.commands = text_only(&scene.commands);
    crate::print::text_layer::collect_local(&result, &scene)
}
pub(super) fn required(tree: &resvg::usvg::Tree, id: &str) -> Result<bool, RenderError> {
    let normalized = tree.to_string(&resvg::usvg::WriteOptions {
        preserve_text: false,
        ..Default::default()
    });
    if normalized.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "semantic normalized SVG exceeds source limit".into(),
        ));
    }
    let xml = roxmltree::Document::parse(&normalized)
        .map_err(|error| RenderError::InvalidPrint(error.to_string()))?;
    Ok(xml
        .descendants()
        .find(|node| node.attribute("id") == Some(id))
        .is_some_and(|node| {
            node.ancestors().any(|ancestor| {
                ["clip-path", "mask"].iter().any(|name| {
                    ancestor
                        .attribute(*name)
                        .is_some_and(|value| value != "none")
                })
            })
        }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusion_requires_some_ink_and_preserves_tiny_coverage() {
        assert!(!intact_alpha(&[0; 4], &[0; 4]));
        assert!(intact_alpha(&[0, 0, 0, 1], &[0, 0, 0, 1]));
        assert!(!intact_alpha(&[0, 0, 0, 1], &[0; 4]));
        assert!(!intact_alpha(&[0, 0, 0, 200], &[0, 0, 0, 100]));
    }
    #[test]
    fn nested_capture_cannot_reset_aggregate_work() {
        let snapshot = crate::tests::empty_snapshot();
        let _root = TextCapture::enter(&snapshot).unwrap();
        let mut first = Budget::default();
        first.pixels(PIXELS - 1).unwrap();
        let _nested = TextCapture::enter(&snapshot).unwrap();
        let mut second = Budget::default();
        assert!(second.pixels(2).is_err());
    }
    #[test]
    fn effect_neutralization_preserves_namespaced_attributes_and_html_styles() {
        let source=br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g style="opacity:.5;clip-path:url(#a)"><use xlink:href="#shape"/><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style="clip-path:circle(50%)">A</div></foreignObject></g></svg>"##.to_vec();
        let bytes = neutralize_svg_effects(source).unwrap();
        let xml = roxmltree::Document::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        let used = xml
            .descendants()
            .find(|node| node.tag_name().name() == "use")
            .unwrap();
        assert_eq!(
            used.attribute(("http://www.w3.org/1999/xlink", "href")),
            Some("#shape")
        );
        let html = xml
            .descendants()
            .find(|node| node.tag_name().name() == "div")
            .unwrap();
        assert_eq!(html.attribute("style"), Some("clip-path:circle(50%)"));
        let group = xml
            .descendants()
            .find(|node| node.tag_name().name() == "g")
            .unwrap();
        assert!(group.attribute("style").unwrap().contains("opacity:.5"));
        assert!(
            group
                .attribute("style")
                .unwrap()
                .ends_with("mask:none!important")
        );
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn alpha_evidence_distinguishes_empty_changed_and_intact_ink_and_bounds_records() {
        let clear = [0u8; 8];
        let ink = [0, 0, 0, 255, 0, 0, 0, 128];
        let clipped = [0, 0, 0, 255, 0, 0, 0, 0];
        assert_eq!(alpha_diagnostic_counts(&clear, &clear), [0, 0, 0]);
        assert_eq!(alpha_diagnostic_counts(&ink, &ink), [2, 2, 0]);
        assert_eq!(alpha_diagnostic_counts(&ink, &clipped), [2, 1, 1]);
        assert!(!intact_alpha(&clear, &clear));
        assert!(intact_alpha(&ink, &ink));
        assert!(!intact_alpha(&ink, &clipped));
        let _ = take_coverage_diagnostics();
        for _ in 0..130 {
            coverage_diagnostic("😀".repeat(2000));
        }
        let (records, omitted) = take_coverage_diagnostics();
        assert_eq!(records.len(), 128);
        assert_eq!(omitted, 2);
        assert!(
            records
                .iter()
                .all(|record| record.len() <= 2048 && record.chars().all(|ch| ch == '😀'))
        );
        assert!(take_coverage_diagnostics().0.is_empty());
    }
}
