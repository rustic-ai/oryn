//! HTML source-set parsing. Decoding and request custody remain with the image owner.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Candidate {
    pub url: String,
    pub width: Option<u32>,
    pub density: f64,
}
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}
fn positive_integer(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|v| *v > 0)
}
fn density(s: &str) -> Option<f64> {
    // HTML floating-point grammar excludes Rust spellings such as NaN, inf and +1.
    if s.is_empty()
        || s.starts_with('+')
        || !s
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E'))
    {
        return None;
    }
    let mantissa = s.split(['e', 'E']).next().unwrap_or(s);
    if mantissa.ends_with('.') {
        return None;
    }
    let value = s.parse::<f64>().ok()?;
    (value.is_finite() && value >= 0.).then_some(value)
}
pub(super) fn parse(input: &str) -> Vec<Candidate> {
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && (space(bytes[i]) || bytes[i] == b',') {
            i += 1
        }
        let start = i;
        while i < bytes.len() && !space(bytes[i]) {
            i += 1
        }
        if i == start {
            break;
        }
        let raw = &input[start..i];
        if raw.ends_with(',') {
            let url = raw.trim_end_matches(',');
            if !url.is_empty() {
                out.push(Candidate {
                    url: url.into(),
                    width: None,
                    density: 1.,
                });
            }
            continue;
        }
        let mut descriptors = Vec::new();
        let mut token = String::new();
        let mut parens = 0;
        while i < bytes.len() {
            let ch = bytes[i];
            i += 1;
            if ch == b',' && parens == 0 {
                break;
            }
            if space(ch) && parens == 0 {
                if !token.is_empty() {
                    descriptors.push(std::mem::take(&mut token));
                }
            } else {
                if ch == b'(' {
                    parens += 1
                } else if ch == b')' && parens > 0 {
                    parens -= 1
                }
                token.push(ch as char);
            }
        }
        if !token.is_empty() {
            descriptors.push(token)
        }
        let (mut w, mut d, mut h, mut valid) = (None, None, None, true);
        for descriptor in descriptors {
            if let Some(s) = descriptor.strip_suffix('w') {
                let v = positive_integer(s);
                if v.is_none() || w.is_some() || d.is_some() {
                    valid = false
                }
                w = v;
            } else if let Some(s) = descriptor.strip_suffix('x') {
                let v = density(s);
                if v.is_none() || d.is_some() || w.is_some() || h.is_some() {
                    valid = false
                }
                d = v;
            } else if let Some(s) = descriptor.strip_suffix('h') {
                let v = positive_integer(s);
                if v.is_none() || h.is_some() || d.is_some() {
                    valid = false
                }
                h = v;
            } else {
                valid = false
            }
        }
        if h.is_some() && w.is_none() {
            valid = false
        }
        if valid {
            out.push(Candidate {
                url: raw.into(),
                width: w,
                density: d.unwrap_or(1.),
            })
        }
    }
    out
}
pub(super) fn select(
    mut candidates: Vec<Candidate>,
    src: Option<&str>,
    size: f64,
    dpr: f64,
) -> Option<(String, f64)> {
    if !candidates
        .iter()
        .any(|c| c.width.is_some() || c.density == 1.)
    {
        if let Some(src) = src.filter(|s| !s.is_empty()) {
            candidates.push(Candidate {
                url: src.into(),
                width: None,
                density: 1.,
            })
        }
    }
    for c in &mut candidates {
        if let Some(w) = c.width {
            c.density = if size > 0. {
                f64::from(w) / size
            } else {
                f64::INFINITY
            };
        }
    }
    candidates.sort_by(|a, b| a.density.total_cmp(&b.density));
    let target = if dpr.is_finite() && dpr > 0. { dpr } else { 1. };
    let index = candidates
        .iter()
        .position(|c| c.density >= target)
        .or_else(|| candidates.len().checked_sub(1))?;
    let c = candidates.remove(index);
    Some((c.url, c.density))
}
fn supported_type(value: &str) -> bool {
    let mime = value.split(';').next().unwrap_or("").trim();
    value.trim().is_empty()
        || [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/bmp",
            "image/x-ms-bmp",
            "image/svg+xml",
            "image/x-png",
        ]
        .iter()
        .any(|known| mime.eq_ignore_ascii_case(known))
}
pub(super) fn source(
    state: &V8BindingState,
    node: NodeId,
) -> Result<(Option<String>, f64, bool, Option<NodeId>), V8BindingError> {
    refresh_layout(state)?;
    let (src, sets, auto_width) = {
        let dom = state.dom.borrow();
        let attr = |id, name| {
            dom.attribute_in_null_namespace(id, name)
                .map(|s| s.unwrap_or("").to_owned())
        };
        let src = dom
            .attribute_in_null_namespace(node, "src")?
            .map(str::to_owned);
        let mut sets = Vec::new();
        if let Some(parent) = dom
            .parent(node)?
            .filter(|p| dom.is_element(*p, "http://www.w3.org/1999/xhtml", "picture"))
        {
            for &sibling in &dom.arena().get(parent)?.children {
                if sibling == node {
                    break;
                }
                if dom.is_element(sibling, "http://www.w3.org/1999/xhtml", "source") {
                    sets.push((
                        attr(sibling, "srcset")?,
                        attr(sibling, "sizes")?,
                        attr(sibling, "media")?,
                        attr(sibling, "type")?,
                        Some(sibling),
                    ));
                }
            }
        }
        sets.push((
            attr(node, "srcset")?,
            attr(node, "sizes")?,
            String::new(),
            String::new(),
            None,
        ));
        let sizes = attr(node, "sizes")?;
        let sizes = sizes.trim_start();
        let auto = attr(node, "loading")?.eq_ignore_ascii_case("lazy")
            && sizes
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case("auto"))
            && sizes
                .get(4..)
                .is_some_and(|s| s.is_empty() || s.starts_with(','));
        (src, sets, auto)
    };
    let provider = state.layout_provider.borrow().clone();
    let dpr = provider
        .as_ref()
        .map_or(1., |p| p.native_device_pixel_ratio());
    for (srcset, sizes, media, mime, source_node) in sets {
        let is_img = source_node.is_none();
        if !supported_type(&mime)
            || (!media.trim().is_empty() && !state.native_media_matches(&media).unwrap_or(false))
        {
            continue;
        }
        let candidates = parse(&srcset);
        if candidates.is_empty() && !is_img {
            continue;
        }
        let hints = source_dimensions(state, source_node)?;
        state.dom.borrow_mut().set_image_source_size(node, hints)?;
        refresh_layout(state)?;
        let base = state.native_document_base_url(false)?;
        let auto_size = auto_width
            .then(|| {
                state
                    .layout
                    .borrow()
                    .hit_projections
                    .get(&node_key(node))
                    .map(|p| f64::from(p.box_model.content_box.width))
            })
            .flatten();
        let size = provider
            .as_ref()
            .and_then(|p| p.native_image_source_size(&sizes, &base, auto_size).ok())
            .unwrap_or_else(|| state.layout.borrow().viewport.unwrap_or((1280., 720.)).0);
        if let Some((url, density)) = select(
            candidates,
            is_img.then_some(src.as_deref()).flatten(),
            size,
            dpr,
        ) {
            return Ok((
                Some(url),
                density,
                !is_img || !srcset.trim().is_empty(),
                source_node,
            ));
        }
    }
    Ok((src, 1., false, None))
}
pub(super) fn dimensions(
    state: &V8BindingState,
    node: NodeId,
    size: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    let density = state
        .image_sources
        .borrow()
        .get(&node)
        .map_or(1., |key| f64::from_bits(key.density_bits));
    size.map(|(w, h)| {
        if density > 0. {
            (
                (f64::from(w) / density) as u32,
                (f64::from(h) / density) as u32,
            )
        } else {
            (0, 0)
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_set_state_machine_preserves_data_commas_and_recovers_invalid_descriptors() {
        let c = parse(
            "data:image/png;base64,AAAA 1x, bad 2x 3x, zero 0w, a 400w 200h, bare, next 2e0x, orphan 10h, bad +2x",
        );
        assert_eq!(
            c.iter().map(|c| c.url.as_str()).collect::<Vec<_>>(),
            ["data:image/png;base64,AAAA", "a", "bare", "next"]
        );
        assert_eq!(parse("a 1x, invalid fn(a,b), c 2x").len(), 2);
    }
    #[test]
    fn width_density_and_src_fallback_selection() {
        assert_eq!(
            select(parse("hi 2x"), Some("lo"), 100., 1.),
            Some(("lo".into(), 1.))
        );
        assert_eq!(
            select(parse("a 400w, b 800w"), Some("ignored"), 400., 1.5),
            Some(("b".into(), 2.))
        );
        assert_eq!(
            select(parse("a 400w, b 800w"), None, 800., 1.),
            Some(("b".into(), 1.))
        );
        assert_eq!(
            select(parse("a 1x, b 2x"), None, 0., 4.),
            Some(("b".into(), 2.))
        );
        assert!(!supported_type("image/avif"));
        assert!(supported_type("IMAGE/SVG+XML; charset=utf-8"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use oryn_dom::html;
    #[derive(Default)]
    struct Provider {
        dpr: Cell<f64>,
        size: Cell<f64>,
        wide: Cell<bool>,
    }
    impl BindingLayoutProvider for Provider {
        fn compute_layout(
            &self,
            _: &BTreeMap<NodeId, BTreeSet<String>>,
        ) -> Result<BindingLayout, String> {
            Ok(BindingLayout::default())
        }
        fn native_device_pixel_ratio(&self) -> f64 {
            self.dpr.get()
        }
        fn set_native_device_pixel_ratio(&self, value: f64) {
            self.dpr.set(value)
        }
        fn native_image_source_size(
            &self,
            _: &str,
            _: &str,
            _: Option<f64>,
        ) -> Result<f64, String> {
            Ok(self.size.get())
        }
        fn native_media_matches(&self, query: &str, _: &str, _: bool) -> Result<bool, String> {
            Ok(query == "all" || self.wide.get())
        }
    }
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<(String, bool)>>,
        cancelled: RefCell<Vec<HostOperationId>>,
    }
    impl PlatformHost for Host {
        fn retire(&self) {}
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            if let PlatformHostRequest::ImageStart { request, .. } = request {
                self.requests
                    .borrow_mut()
                    .push((request.url, request.upgrade_mixed_image));
            }
            Ok(())
        }
        fn cancel(&self, id: HostOperationId) {
            self.cancelled.borrow_mut().push(id)
        }
        fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
            Vec::new()
        }
    }
    #[test]
    fn picture_selection_and_live_source_removal_share_image_request_custody() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(html::parse("<picture><source id=unsupported type='image/avif' srcset='bad.avif'><source id=source width=200 height=100 media='(width > 600px)' srcset='wide.png 400w, large.png 800w'><img id=i src='fallback.png' srcset='small.png 1x, retina.png 2x'></picture>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state
            .commit_location("https://images.test/dir/page")
            .unwrap();
        let provider = Rc::new(Provider {
            dpr: Cell::new(2.),
            size: Cell::new(400.),
            wide: Cell::new(true),
        });
        state.set_layout_provider(provider.clone());
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        prepare_document_images(scope, &state).unwrap();
        let node = dom.borrow().find_by_id("i").unwrap();
        assert_eq!(
            host.requests.borrow().last().unwrap(),
            &(String::from("https://images.test/dir/large.png"), false)
        );
        assert_eq!(dimensions(&state, node, Some((800, 400))), Some((400, 200)));
        assert_eq!(
            dom.borrow().arena().get(node).unwrap().image_source_size,
            Some((Some(200), Some(100)))
        );
        provider.size.set(800.);
        prepare_document_images(scope, &state).unwrap();
        assert_eq!(host.requests.borrow().len(), 1);
        assert_eq!(dimensions(&state, node, Some((800, 400))), Some((800, 400)));
        let code = v8::String::new(scope, "document.getElementById('source').remove()").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        assert_eq!(
            host.requests.borrow().last().unwrap().0,
            "https://images.test/dir/retina.png"
        );
        assert_eq!(
            dom.borrow().arena().get(node).unwrap().image_source_size,
            None
        );
        assert_eq!(host.cancelled.borrow().len(), 1);
        state.set_native_device_pixel_ratio(1.);
        prepare_document_images(scope, &state).unwrap();
        assert_eq!(
            host.requests.borrow().last().unwrap().0,
            "https://images.test/dir/small.png"
        );
        let code=v8::String::new(scope,"for(const tag of ['img','script','link']){const e=document.createElement(tag);if(e.fetchPriority!=='auto')throw Error('missing priority');e.fetchPriority='HIGH';if(e.fetchPriority!=='high'||e.getAttribute('fetchpriority')!=='HIGH')throw Error('priority reflection');e.fetchPriority='invalid';if(e.fetchPriority!=='auto')throw Error('invalid priority')}if(new Image().fetchPriority!=='auto')throw Error('Image priority');let s=document.createElement('source');s.srcset='replacement.png 1x';s.media='all';document.querySelector('picture').insertBefore(s,document.getElementById('i'))").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(
            host.requests.borrow().last().unwrap().0,
            "https://images.test/dir/replacement.png"
        );
    }
    #[test]
    fn priority_reflection_survives_conversion_adoption_and_preserves_domstrings() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (first_state, image) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(951), 1).unwrap();
            let image = super::super::test_eval::eval(scope, "document.createElement('img')");
            (state, v8::Global::new(scope, image))
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let second_state = install_generated_bindings(scope, second, dom, RealmId(952), 1).unwrap();
        let image = v8::Local::new(scope, &image);
        let global = second.global(scope);
        set_object_property(scope, global, "foreignImage", image).unwrap();
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            const raw='\uD800HIGH\uDC00'; let conversions=0;
            foreignImage.fetchPriority={toString(){conversions++;document.adoptNode(foreignImage);return raw}};
            if(conversions!==1||foreignImage.ownerDocument!==document||foreignImage.getAttribute('fetchpriority')!==raw||foreignImage.fetchPriority!=='auto')return false;
            foreignImage.fetchPriority='LoW';if(foreignImage.fetchPriority!=='low'||foreignImage.getAttribute('fetchpriority')!=='LoW')return false;
            const marker={};let caught;try{foreignImage.fetchPriority={toString(){throw marker}}}catch(error){caught=error}
            return caught===marker&&foreignImage.getAttribute('fetchpriority')==='LoW';
        })()"#).is_true());
        let (owner, _) = node_identity::argument(scope, image).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner, &second_state));
        assert!(!Rc::ptr_eq(&owner, &first_state));
    }
}

/// Width/height reflected on source are unsigned HTML dimensions. Missing or invalid
/// source hints leave the image's own presentation hints in effect.
pub(super) fn source_dimensions(
    state: &V8BindingState,
    source: Option<NodeId>,
) -> Result<Option<(Option<u32>, Option<u32>)>, V8BindingError> {
    let Some(source) = source else {
        return Ok(None);
    };
    let dom = state.dom.borrow();
    let parse = |name| -> Result<Option<u32>, V8BindingError> {
        let Some(value) = dom.attribute_in_null_namespace(source, name)? else {
            return Ok(None);
        };
        let value =
            value.trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}'));
        let value = value.strip_prefix('+').unwrap_or(value);
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        Ok(value[..digits].parse::<u32>().ok())
    };
    Ok(Some((parse("width")?, parse("height")?)))
}

/// Rendered auto-size changes (including animation/font changes without DOM edits)
/// invalidate the source set at the next native image checkpoint.
pub(super) fn layout_changed(state: &V8BindingState, next: &BindingLayout) {
    let previous = state.layout.borrow();
    let dom = state.dom.borrow();
    let changed = state.image_sources.borrow().keys().any(|node| {
        if !dom
            .attribute_in_null_namespace(*node, "loading")
            .ok()
            .flatten()
            .is_some_and(|v| v.eq_ignore_ascii_case("lazy"))
        {
            return false;
        }
        let key = node_key(*node);
        previous
            .hit_projections
            .get(&key)
            .map(|p| p.box_model.content_box.width)
            != next
                .hit_projections
                .get(&key)
                .map(|p| p.box_model.content_box.width)
    });
    if changed {
        state.images_deferred.set(true)
    }
}
