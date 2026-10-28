//! Poster pixels have independent ownership from decoded video playback frames.
use super::*;
pub(super) fn is_video(state: &V8BindingState, node: NodeId) -> bool {
    state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "video")
}
pub(super) fn clear_image(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    state.failed_images.borrow_mut().remove(&node);
    state.image_response_urls.borrow_mut().remove(&node);
    if is_video(state, node) {
        state
            .dom
            .borrow_mut()
            .set_media_poster(node, None, false, None)?;
    } else {
        state.decoded_images.borrow_mut().remove(&node);
        state.decoded_svg_images.borrow_mut().remove(&node);
        state.dom.borrow_mut().set_image_dimensions(node, None)?;
    }
    Ok(())
}
pub(super) fn complete(
    state: &V8BindingState,
    node: NodeId,
    bytes: &[u8],
) -> Result<(), V8BindingError> {
    // Poster errors have no media load/error event and never mutate videoWidth,
    // videoHeight, readyState, playback pixels or the canvas origin-clean flag.
    if let Some((width, height)) = image_dimensions::svg_size(bytes) {
        if width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 4 * 1024 * 1024 {
            state.dom.borrow_mut().set_media_poster(
                node,
                Some(bytes.to_vec()),
                true,
                Some((width, height)),
            )?;
        }
        return Ok(());
    }
    let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
    else {
        return Ok(());
    };
    let Ok((width, height)) = reader.into_dimensions() else {
        return Ok(());
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
        return Ok(());
    }
    let Ok(image) = image::load_from_memory(bytes) else {
        return Ok(());
    };
    let rgba = image.to_rgba8();
    let mut png = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|_| V8BindingError::Message("poster image encoding unavailable".into()))?;
    state
        .dom
        .borrow_mut()
        .set_media_poster(node, Some(png), false, Some((width, height)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poster_loading_and_replacement_keep_playback_pixels_and_media_events_separate() {
        #[derive(Default)]
        struct Host(RefCell<Vec<(String, bool)>>);
        impl PlatformHost for Host {
            fn retire(&self) {}
            fn cancel(&self, _: HostOperationId) {}
            fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
                Vec::new()
            }
            fn submit(
                &self,
                _: HostOperationId,
                request: PlatformHostRequest,
            ) -> Result<(), String> {
                if let PlatformHostRequest::ImageStart {
                    request,
                    cors_enabled,
                } = request
                {
                    self.0.borrow_mut().push((request.url, cors_enabled));
                }
                Ok(())
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body><video id='v'></video></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.commit_location("https://poster.test/base/").unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let source=v8::String::new(scope,"globalThis.v=document.getElementById('v');globalThis.posterEvents=0;v.onload=v.onerror=()=>posterEvents++;v.crossOrigin='anonymous';v.poster='first.svg';").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let node = dom
            .borrow()
            .query_selector(state.document_root(), "video")
            .unwrap()
            .unwrap();
        state.decoded_images.borrow_mut().insert(
            node,
            DecodedImage {
                origin_clean: false,
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            },
        );
        dom.borrow_mut()
            .set_media_frame(node, Some(vec![9]))
            .unwrap();
        dom.borrow_mut()
            .set_image_dimensions(node, Some((1, 1)))
            .unwrap();
        complete(&state,node,b"<svg xmlns='http://www.w3.org/2000/svg' width='123' height='123'><rect width='123' height='123'/></svg>").unwrap();
        dispatch_image_event(scope, &state, node, true).unwrap();
        {
            let dom = dom.borrow();
            let record = dom.arena().get(node).unwrap();
            assert_eq!(record.media_poster_dimensions, Some((123, 123)));
            assert_eq!(record.image_dimensions, Some((1, 1)));
            assert_eq!(record.media_frame.as_deref(), Some(&[9][..]));
            assert!(record.showing_poster && record.media_poster_is_svg);
        }
        let source=v8::String::new(scope,"if(posterEvents!==0||v.poster!=='https://poster.test/base/first.svg')throw Error('poster reflection/events');v.poster='next.png';").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(node)
                .unwrap()
                .media_poster
                .is_none()
        );
        assert_eq!(
            state.decoded_images.borrow()[&node].rgba,
            vec![1, 2, 3, 255]
        );
        assert_eq!(
            *host.0.borrow(),
            vec![
                ("https://poster.test/base/first.svg".into(), false),
                ("https://poster.test/base/next.png".into(), false)
            ]
        );
        complete(&state, node, b"not an image").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(node)
                .unwrap()
                .media_poster
                .is_none()
        );
    }
}
