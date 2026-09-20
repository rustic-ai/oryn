//! Synchronous, realm-owned drag image capture; no author callback or actor RPC.
use super::*;
use crate::drag_image::{Capture, Feedback, MAX_BYTES, MAX_PIXELS, Resource, Snapshot};
use image::ImageEncoder;
#[derive(Default)]
pub(super) struct State {
    sequence: u64,
    session: u64,
    active: Option<u64>,
    pending: Vec<Feedback>,
}
pub(super) fn capture(
    state: &V8BindingState,
    node: NodeId,
    hotspot: (i32, i32),
    polygons: Vec<[f64; 8]>,
) -> Result<Snapshot, V8BindingError> {
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    if state.execution_retired()
        || state.dom.borrow().arena().node_document(node)? != Some(identity.0)
    {
        return Err(V8BindingError::Allocation("retired drag image source"));
    }
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation(
            "native drag image provider unavailable",
        ))?;
    let resources = snapshot_native_paint_resources(state, node)?;
    let request = Capture {
        css_resources: state.captured_css_images(),
        document: identity.0,
        source: node,
        generation: identity.1,
        epoch: identity.2,
        hotspot,
        polygons,
        resources,
    };
    let snapshot = provider
        .capture_drag_image(
            &request,
            &state.custom_states.borrow(),
            state.document_scroll_offset(),
            &state.element_scroll_offsets(),
        )
        .map_err(|member| V8BindingError::Unsupported {
            interface: "DataTransfer".into(),
            member,
        })?;
    snapshot
        .validate()
        .map_err(|_| V8BindingError::Allocation("invalid native drag image"))?;
    if state.execution_retired()
        || identity
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
    {
        return Err(V8BindingError::Allocation(
            "drag image source retired during capture",
        ));
    }
    Ok(snapshot)
}
/// Private native paint consumer; tainted pixels are not returned to scripts.
pub fn snapshot_native_paint_resources(
    state: &V8BindingState,
    node: NodeId,
) -> Result<Vec<Resource>, V8BindingError> {
    if state.execution_retired()
        || state
            .dom
            .borrow()
            .arena()
            .node_document(node)?
            .is_some_and(|document| document != state.document_root())
    {
        return Err(V8BindingError::Allocation(
            "retired native paint resource source",
        ));
    }
    let descendants = state
        .dom
        .borrow()
        .flat_tree_elements(node)?
        .into_iter()
        .map(|(node, _)| node)
        .collect::<BTreeSet<_>>();
    let mut resources = Vec::new();
    let mut bytes = 0usize;
    let mut decoded_bytes = 0usize;
    for images in [&state.decoded_images, &state.canvas_images] {
        for (id, image) in images.borrow().iter() {
            if !descendants.contains(id) {
                continue;
            }
            let pixels = (image.width as usize)
                .checked_mul(image.height as usize)
                .ok_or(V8BindingError::Allocation("drag image dimensions"))?;
            if pixels > MAX_PIXELS || pixels.checked_mul(4) != Some(image.rgba.len()) {
                return Err(V8BindingError::Allocation("drag image resource dimensions"));
            }
            decoded_bytes = decoded_bytes.saturating_add(image.rgba.len());
            if decoded_bytes > 64 * 1024 * 1024 {
                return Err(V8BindingError::Allocation("drag decoded resource budget"));
            }
            let mut png = Vec::new();
            // These PNGs transport pixels to the native renderer. Avoid an
            // adaptive compression search on every visibility checkpoint.
            image::codecs::png::PngEncoder::new_with_quality(
                &mut png,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::NoFilter,
            )
            .write_image(
                &image.rgba,
                image.width,
                image.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|_| V8BindingError::Allocation("drag image resource encoding"))?;
            // Retain the previous encoder's resource-budget admission for
            // images that need stronger compression to fit.
            if png.len() > MAX_BYTES.saturating_sub(bytes) {
                png.clear();
                image::codecs::png::PngEncoder::new(&mut png)
                    .write_image(
                        &image.rgba,
                        image.width,
                        image.height,
                        image::ExtendedColorType::Rgba8,
                    )
                    .map_err(|_| V8BindingError::Allocation("drag image resource encoding"))?;
            }
            bytes = bytes.saturating_add(png.len());
            if bytes > MAX_BYTES {
                return Err(V8BindingError::Allocation("drag image resource budget"));
            }
            let url = state
                .image_sources
                .borrow()
                .get(id)
                .and_then(|key| Url::parse(&key.base_url).ok()?.join(&key.source).ok())
                .map(|url| url.to_string())
                .unwrap_or_else(|| format!("oryn:canvas/{}", node_key(*id)));
            let requested = url;
            let url = state
                .image_response_urls
                .borrow()
                .get(id)
                .cloned()
                .unwrap_or_else(|| requested.clone());
            let url_aliases = if url != requested {
                vec![requested]
            } else {
                Vec::new()
            };
            resources.push(Resource {
                node: *id,
                url,
                url_aliases,
                width: image.width,
                height: image.height,
                png,
                svg: None,
            });
        }
    }
    for (id, svg) in state.decoded_svg_images.borrow().iter() {
        if !descendants.contains(id) {
            continue;
        }
        bytes = bytes
            .checked_add(svg.len())
            .ok_or(V8BindingError::Allocation("SVG paint resource overflow"))?;
        if bytes > MAX_BYTES {
            return Err(V8BindingError::Allocation(
                "native SVG paint resource budget",
            ));
        }
        let (width, height) = image_dimensions::svg_size(svg).ok_or(V8BindingError::Allocation(
            "invalid captured SVG dimensions",
        ))?;
        let url = state
            .image_sources
            .borrow()
            .get(id)
            .and_then(|key| Url::parse(&key.base_url).ok()?.join(&key.source).ok())
            .ok_or(V8BindingError::Allocation("native SVG image URL missing"))?
            .to_string();
        let requested = url;
        let url = state.image_response_urls.borrow().get(id).cloned().ok_or(
            V8BindingError::Allocation("native SVG final response URL missing"),
        )?;
        let url_aliases = if url != requested {
            vec![requested]
        } else {
            Vec::new()
        };
        resources.push(Resource {
            node: *id,
            url,
            url_aliases,
            width,
            height,
            png: Vec::new(),
            svg: Some(svg.clone()),
        });
    }
    Ok(resources)
}
pub(super) fn begin(state: &V8BindingState, image: Option<Snapshot>) -> u64 {
    let mut feedback = state.drag_images.borrow_mut();
    feedback.session = feedback.session.saturating_add(1);
    let session = feedback.session;
    feedback.active = Some(session);
    feedback.sequence = feedback.sequence.saturating_add(1);
    let sequence = feedback.sequence;
    // Retain at most the latest set/clear for this realm between native drains.
    feedback.pending.clear();
    feedback.pending.push(Feedback {
        realm: u64::from(state.realm.0),
        generation: state.document_generation(),
        epoch: state.document_stream_epoch(),
        session,
        sequence,
        image,
    });
    session
}
pub(super) fn clear(state: &V8BindingState, session: Option<u64>) {
    let mut feedback = state.drag_images.borrow_mut();
    if feedback.active.is_none() || session.is_some_and(|session| feedback.active != Some(session))
    {
        return;
    }
    let session = feedback.active.take().unwrap();
    feedback.sequence = feedback.sequence.saturating_add(1);
    let sequence = feedback.sequence;
    feedback.pending.clear();
    feedback.pending.push(Feedback {
        realm: u64::from(state.realm.0),
        generation: state.document_generation(),
        epoch: state.document_stream_epoch(),
        session,
        sequence,
        image: None,
    });
}
pub fn take_native_drag_feedback(state: &V8BindingState) -> Vec<Feedback> {
    std::mem::take(&mut state.drag_images.borrow_mut().pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Provider(Cell<u8>);
    impl BindingLayoutProvider for Provider {
        fn compute_layout(
            &self,
            _: &BTreeMap<NodeId, BTreeSet<String>>,
        ) -> Result<BindingLayout, String> {
            Ok(BindingLayout::default())
        }
        fn capture_drag_image(
            &self,
            request: &Capture,
            _: &BTreeMap<NodeId, BTreeSet<String>>,
            _: (f64, f64),
            _: &BTreeMap<NodeId, (f64, f64)>,
        ) -> Result<Snapshot, String> {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &[self.0.get(), 0, 0, 255],
                    1,
                    1,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|error| error.to_string())?;
            Ok(Snapshot {
                width: 1,
                height: 1,
                png,
                bounds: oryn_common::v4::CssRect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                scale: 1.,
                hotspot: (request.hotspot.0 as f64, request.hotspot.1 as f64),
            })
        }
    }
    #[test]
    fn capture_owns_call_time_pixels_and_rejects_retirement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=image></div>").into_runtime().unwrap(),
        ));
        let node = dom.borrow().find_by_id("image").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let provider = Rc::new(Provider(Cell::new(12)));
        state.set_layout_provider(provider.clone());
        let before = state.native_paint_revision().unwrap();
        assert_eq!(state.native_paint_revision().unwrap(), before);
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((123., 456.)),
            ..Default::default()
        });
        let resized = state.native_paint_revision().unwrap();
        assert_ne!(before, resized);
        assert_eq!(state.native_paint_revision().unwrap(), resized);
        let first = capture(&state, node, (7, 9), Vec::new()).unwrap();
        provider.0.set(200);
        let second = capture(&state, node, (1, 2), Vec::new()).unwrap();
        assert_ne!(first.png, second.png);
        assert_eq!(first.hotspot, (7., 9.));
        assert_eq!(
            image::load_from_memory(&first.png)
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0[0],
            12
        );
        let session = begin(&state, Some(first));
        assert_eq!(take_native_drag_feedback(&state).len(), 1);
        clear(&state, Some(session + 1));
        assert!(take_native_drag_feedback(&state).is_empty());
        state.advance_document_stream_epoch().unwrap();
        let clear = take_native_drag_feedback(&state);
        assert_eq!(clear.len(), 1);
        assert!(clear[0].image.is_none());
        state.retire_execution();
        assert!(capture(&state, node, (0, 0), Vec::new()).is_err());
    }
}
