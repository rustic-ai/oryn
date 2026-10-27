//! Headless rendering opportunities publish real native paint scenes. These
//! scenes contain only already available resources; captures may separately
//! await additional bytes, but normal rendering never blocks on those bytes.
use super::*;
#[cfg(test)]
thread_local! {static FAIL_COMPOSED_PRESENTATION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[derive(Default)]
pub(super) struct State {
    pub(super) building: bool,
    observer_relayout: bool,
    iteration: usize,
    next: Option<Instant>,
    identity: Option<crate::executable::CaptureDocumentIdentity>,
    stamp: Option<(u64, u64, u64)>,
    pub(super) scene: Option<PaintSnapshot>,
    display_list: Option<oryn_render_core::DisplayList>,
    pending_paint: Option<(crate::executable::CaptureDocumentIdentity, bool)>,
}
impl PageState {
    pub(super) fn publish_native_rendering_opportunity(&mut self) -> Result<(), RuntimeError> {
        if self.presentation.building || self.terminal_close_requested() {
            return Ok(());
        }
        self.presentation.building = true;
        let result = (|| {
            for iteration in 0..64 {
                if self.terminal_close_requested() {
                    return Ok(());
                }
                self.presentation.iteration = iteration;
                self.presentation.observer_relayout = false;
                self.prepare_native_rendering_opportunity()?;
                if !self.presentation.observer_relayout {
                    return Ok(());
                }
                self.invalidate_presented_layout_deadlines();
            }
            Err(RuntimeError::PaintUnavailable(
                "resize observer rendering work budget exhausted".into(),
            ))
        })();
        self.presentation.building = false;
        result
    }
    fn prepare_native_rendering_opportunity(&mut self) -> Result<(), RuntimeError> {
        let Some(page) = self.executable.as_ref() else {
            return Ok(());
        };
        if !page.native_rendering_allowed() {
            return Ok(());
        }
        let Some(identity) = page.capture_document_identity() else {
            return Ok(());
        };
        let now = Instant::now();
        if self.presentation.identity == Some(identity)
            && self.presentation.next.is_some_and(|next| now < next)
        {
            return Ok(());
        }
        // A failed scene is retried at a rendering opportunity, not in a tight
        // layout/error loop. A new document has its own immediate opportunity.
        if self.presentation.identity != Some(identity) {
            self.presentation.stamp = None;
            self.presentation.scene = None;
            self.presentation.display_list = None;
            self.presentation.pending_paint = None;
        }
        self.presentation.identity = Some(identity);
        self.presentation.next = Some(now + Duration::from_millis(16));
        if self.presentation.iteration == 0 {
            self.refresh_layout()?;
        } else {
            self.refresh_layout_geometry()?;
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        let Some(page) = self.executable.as_ref() else {
            return Ok(());
        };
        if page.capture_document_identity() != Some(identity) || !page.native_rendering_allowed() {
            return Ok(());
        }
        let stamp = (
            self.document_generation,
            self.layout_revision,
            page.native_animation_revision(),
        );
        let document = self.document.clone().ok_or_else(|| {
            RuntimeError::PaintUnavailable("presentation document unavailable".into())
        })?;
        let layout = self.layout.clone().ok_or_else(|| {
            RuntimeError::PaintUnavailable("presentation layout unavailable".into())
        })?;
        let capture_input = self
            .layout_input_cache
            .as_ref()
            .map(|(_, input, _)| input.clone());
        let prepared = self.paint_snapshot_plan_with_layout(
            &document,
            &layout,
            self.viewport,
            false,
            capture_input.as_ref(),
        );
        let mut plan = prepared?;
        // Bind decoded native <img>, canvas, SVG and video surfaces by node,
        // not by authored URL aliases or a fresh network request.
        let resources = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .document_paint_images()
            .map_err(map_host_error)?;
        let mut bytes = plan
            .snapshot
            .images
            .iter()
            .map(|resource| resource.bytes.len())
            .sum::<usize>();
        for resource in resources {
            let key = pack_node_id(resource.node);
            if !plan.snapshot.nodes.iter().any(|node| node.node_id == key) {
                continue;
            }
            let (media_type, body) = match resource.svg {
                Some(svg) => ("image/svg+xml", svg),
                None => ("image/png", resource.png),
            };
            bytes = bytes.saturating_add(body.len());
            if bytes > self.config.max_response_bytes {
                return Err(RuntimeError::PaintUnavailable(
                    "presented scene image budget".into(),
                ));
            }
            let index = plan.snapshot.images.len();
            plan.snapshot.images.push(PaintImageResource {
                svg_document_snapshot: false,
                svg_instances: Vec::new(),
                content_type: None,
                url: resource.url,
                url_aliases: resource.url_aliases,
                media_type: media_type.into(),
                bytes: body,
            });
            for node in &mut plan.snapshot.nodes {
                if node.node_id == key {
                    node.image_resource = Some(index);
                }
            }
        }
        self.capture_child_frame_presentation(&mut plan)?;
        // Gather only after all descendant geometry is available. Cross-document
        // observations use current native layouts, not last frame's child boxes.
        if self.frame_scope.is_none() {
            let page = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?;
            page.sample_observer_environment_in_scope()
                .map_err(map_host_error)?;
            let before = page.selected_accessibility_stamp();
            let delivered = page.broadcast_resize_observers().map_err(map_host_error)?;
            if delivered || page.selected_accessibility_stamp() != before {
                self.presentation.observer_relayout = true;
                return Ok(());
            }
            page.sample_intersection_observers()
                .map_err(map_host_error)?;
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        let Some(page) = self.executable.as_ref() else {
            return Ok(());
        };
        if page.capture_document_identity() != Some(identity) || !page.native_rendering_allowed() {
            return Ok(());
        }
        if self.presentation.scene.as_ref() == Some(&plan.snapshot) {
            return self.commit_presented_paints();
        }
        // Compile the native scene through the shared renderer before it is
        // accepted. This performs real clipping/stacking/command generation;
        // geometry measurement alone never creates a paint timeline entry.
        #[cfg(test)]
        if !plan.snapshot.frames.is_empty() && FAIL_COMPOSED_PRESENTATION.with(|fail| fail.get()) {
            return Err(RuntimeError::PaintUnavailable(
                "injected composed scene rejection".into(),
            ));
        }
        let display_list = oryn_render_core::build_display_list(
            &plan.snapshot,
            oryn_render_core::RenderOptions {
                scale: self.viewport.device_pixel_ratio,
                ..Default::default()
            },
        )
        .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
        // Publication is the headless presentation boundary: retain the scene
        // before exposing its timing entry. No authored callback runs between.
        let painted = !plan.snapshot.nodes.is_empty();
        let has_content = oryn_render_core::scene_has_contentful_paint(
            &plan.snapshot,
            &display_list,
            oryn_render_core::RenderOptions {
                scale: self.viewport.device_pixel_ratio,
                ..Default::default()
            },
        )
        .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
        self.presentation.stamp = Some(stamp);
        self.presentation.scene = Some(plan.snapshot);
        self.presentation.display_list = Some(display_list);
        if painted {
            self.presentation.pending_paint = Some((identity, has_content));
        }
        self.commit_presented_paints()
    }
    fn invalidate_presented_layout_deadlines(&mut self) {
        self.presentation.next = None;
        for frame in self.frame_states.values_mut() {
            frame.invalidate_presented_layout_deadlines();
        }
    }
    fn collect_presented_paints(
        &self,
        output: &mut Vec<(crate::executable::CaptureDocumentIdentity, bool)>,
    ) -> Result<(), RuntimeError> {
        if let Some(pending) = self.presentation.pending_paint {
            output.push(pending);
        }
        let (Some(scene), Some(display)) =
            (&self.presentation.scene, &self.presentation.display_list)
        else {
            return Ok(());
        };
        for (index, child) in scene.frames.iter().enumerate() {
            if !oryn_render_core::scene_frame_is_presented(
                scene,
                display,
                index,
                oryn_render_core::RenderOptions {
                    scale: scene.viewport.device_pixel_ratio,
                    ..Default::default()
                },
            )
            .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?
            {
                continue;
            }
            let Some(frame) = self.frame_states.values().find(|frame| {
                frame.frame_scope.as_ref().is_some_and(|scope| {
                    scope.frame.map(pack_node_id) == Some(child.owner_node)
                        && scope.parent_realm
                            == self.presentation.identity.map(|identity| identity.0)
                }) && frame.presentation.scene.as_ref() == Some(child.snapshot.as_ref())
            }) else {
                continue;
            };
            frame.collect_presented_paints(output)?;
        }
        Ok(())
    }
    fn clear_presented_paints(&mut self, accepted: &[crate::executable::CaptureDocumentIdentity]) {
        if self
            .presentation
            .pending_paint
            .is_some_and(|(identity, _)| accepted.contains(&identity))
        {
            self.presentation.pending_paint = None;
        }
        for frame in self.frame_states.values_mut() {
            frame.clear_presented_paints(accepted);
        }
    }
    fn commit_presented_paints(&mut self) -> Result<(), RuntimeError> {
        // A child scene is only preparation until an embedding root accepts
        // the complete scene. Its timeline must also honor ancestor effects.
        if self.frame_scope.is_some() {
            return Ok(());
        }
        let mut pending = Vec::new();
        self.collect_presented_paints(&mut pending)?;
        let Some(page) = self.executable.as_ref() else {
            return Ok(());
        };
        let accepted = pending
            .into_iter()
            .filter_map(|(identity, contentful)| {
                page.record_native_composed_paint(identity, contentful)
                    .then_some(identity)
            })
            .collect::<Vec<_>>();
        self.clear_presented_paints(&accepted);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(source: &str) -> PageState {
        let mut page = PageState::new(
            PageId(917),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document(
            "https://presentation.test/".into(),
            source,
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        page
    }
    #[test]
    fn geometry_projection_updates_layout_without_draining_author_tasks() {
        use crate::host::JavaScriptHost;
        let mut page = page("<button id=target style='width:40px;height:20px'>Target</button>");
        let executable = page.executable.as_mut().unwrap();
        let mut host = executable.take_agent_host().unwrap();
        host.execute("globalThis.geometryTask=0;setTimeout(()=>geometryTask++,0);document.getElementById('target').style.width='88px'").unwrap();
        assert!(matches!(
            executable.return_agent_host_after_turn(host),
            Ok(None)
        ));
        page.refresh_layout_geometry().unwrap();
        let layout = page.layout.as_ref().unwrap();
        assert!(
            layout
                .computed_styles
                .values()
                .any(|style| style.get("width").is_some_and(|width| width == "88px"))
        );
        let executable = page.executable.as_mut().unwrap();
        let mut host = executable.take_agent_host().unwrap();
        assert_eq!(host.evaluate_string("String(geometryTask)").unwrap(), "0");
        assert!(matches!(
            executable.return_agent_host_after_turn(host),
            Ok(None)
        ));
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("String(geometryTask)")
                .unwrap(),
            "1"
        );
    }

    #[test]
    fn omitted_hidden_flow_subtrees_keep_geometry_and_visible_contents_stay_boxless() {
        let document = crate::html::parse(
            "<!doctype html><body>lead <span>inline</span><button id=hidden style='display:none'><span id=descendant>hidden</span></button><iframe id=frame style='display:none'></iframe><span id=contents style='display:contents'><button id=visible>visible</button></span><button id=tail>tail</button></body>",
        );
        let input = layout_input(
            &document,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            &[],
        );
        let output = oryn_layout::compute(&input, &Default::default()).unwrap();
        let id = |name: &str| {
            input
                .nodes
                .iter()
                .find(|node| node.attributes.get("id").is_some_and(|value| value == name))
                .unwrap()
                .id
        };
        for name in ["hidden", "descendant", "frame"] {
            let geometry = &output.geometry[&id(name)];
            assert_eq!(
                geometry.availability,
                oryn_common::v4::GeometryAvailability::Hidden
            );
            assert!(geometry.box_model.is_none() && geometry.client_rects.is_empty());
            assert!(!geometry.interactable && !geometry.in_viewport);
            assert_eq!(output.offset_dimensions[&id(name)], (0., 0.));
        }
        assert!(output.geometry.get(&id("contents")).is_none_or(
            |geometry| geometry.availability != oryn_common::v4::GeometryAvailability::Hidden
        ));
        for name in ["visible", "tail"] {
            assert!(output.geometry[&id(name)].box_model.is_some());
        }
    }

    #[test]
    fn iframe_default_box_exists_before_child_creation_but_zero_and_hidden_stay_excluded() {
        // Parse-only snapshot: no child realm or response has been admitted.
        let document = crate::html::parse(
            "<body><iframe id=normal></iframe><iframe id=zero style='height:0'></iframe><iframe id=hidden style='display:none'></iframe></body>",
        );
        let input = layout_input(
            &document,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            &[],
        );
        let output = oryn_layout::compute(&input, &Default::default()).unwrap();
        let id = |name: &str| {
            input
                .nodes
                .iter()
                .find(|node| node.attributes.get("id").is_some_and(|value| value == name))
                .unwrap()
                .id
        };
        let normal = output.geometry[&id("normal")].box_model.as_ref().unwrap();
        assert_eq!(
            (normal.content_box.width, normal.content_box.height),
            (300., 150.)
        );
        assert_eq!(
            output.geometry[&id("zero")]
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .height,
            0.
        );
        assert_eq!(
            output.geometry[&id("hidden")].availability,
            oryn_common::v4::GeometryAvailability::Hidden
        );
    }
    #[test]
    fn normal_page_poll_publishes_scene_and_paint_timing_without_capture() {
        let mut page = page("<body><p>Native scene</p></body>");
        page.poll_page_tasks().unwrap();
        assert!(
            page.presentation
                .scene
                .as_ref()
                .is_some_and(|scene| scene.nodes.iter().any(|node| node
                    .text
                    .iter()
                    .any(|text| text.text.contains("Native scene"))))
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("performance.getEntriesByType('paint').map(e=>e.name).join(',')")
                .unwrap(),
            "first-paint,first-contentful-paint"
        );
    }
    #[test]
    fn replacing_document_stream_cannot_reuse_prior_scene_or_paint_timestamp() {
        let mut page = page("<body><p>old scene</p></body>");
        page.poll_page_tasks().unwrap();
        let identity = page.presentation.identity;
        page.executable
            .as_mut()
            .unwrap()
            .evaluate(
                "document.open();document.write('<body><p>new scene</p></body>');document.close()",
            )
            .unwrap();
        page.presentation.next = None;
        page.poll_page_tasks().unwrap();
        assert_ne!(page.presentation.identity, identity);
        let scene = page.presentation.scene.as_ref().unwrap();
        assert!(
            !scene
                .nodes
                .iter()
                .any(|node| node.text.iter().any(|text| text.text.contains("old scene")))
        );
    }
    #[test]
    fn resize_callback_layout_is_reflected_in_the_first_accepted_scene() {
        let mut page = page(
            "<body><p id=target style='width:20px'>content</p><script>new ResizeObserver(()=>{document.getElementById('target').style.width='73px'}).observe(document.getElementById('target'))</script></body>",
        );
        page.poll_page_tasks().unwrap();
        let paragraph = page
            .presentation
            .scene
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.tag == "p")
            .unwrap();
        assert_eq!(
            paragraph
                .geometry
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .width,
            73.
        );
    }
    #[test]
    fn intersection_delivery_follows_rendering_as_a_separate_native_task() {
        let mut page = page("<body><div id=box style='width:40px;height:20px'></div></body>");
        page.executable.as_mut().unwrap().evaluate("globalThis.order=[];const target=document.getElementById('box');requestAnimationFrame(()=>order.push('raf'));new ResizeObserver(()=>order.push('resize')).observe(target);new IntersectionObserver(()=>order.push('intersection')).observe(target)").unwrap();
        page.presentation.next = None;
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("order.join(',')")
                .unwrap(),
            "raf,resize"
        );
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("order.join(',')")
                .unwrap(),
            "raf,resize,intersection"
        );
    }

    #[test]
    fn fully_transparent_ancestor_does_not_publish_contentful_paint() {
        let mut page = page("<body><div style='opacity:0'><p>not visible</p></div></body>");
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("performance.getEntriesByName('first-contentful-paint','paint').length")
                .unwrap(),
            "0"
        );
    }
}

#[cfg(test)]
fn presentation_gate_summary(page: &PageState) -> String {
    let Some(executable) = page.executable.as_ref() else {
        return "no executable".into();
    };
    let realms = executable
        .automation_realms()
        .into_iter()
        .map(|realm| {
            let owner = realm.frame.map(pack_node_id);
            let geometry = owner
                .and_then(|id| {
                    page.presentation
                        .scene
                        .as_ref()?
                        .nodes
                        .iter()
                        .find(|node| node.node_id == id)
                })
                .map(|node| format!("{:?}", node.geometry));
            format!(
                "realm={:?} parent={:?} live={} allowed={} owner={owner:?} geometry={geometry:?}",
                realm.realm,
                realm.parent_realm,
                realm.live,
                executable.native_realm_rendering_allowed(realm.realm)
            )
        })
        .collect::<Vec<_>>();
    format!(
        "realms={realms:?}; accepted_scene={} cached_children={}",
        page.presentation.scene.is_some(),
        page.frame_states.len()
    )
}
#[cfg(test)]
mod frame_tests {
    use super::*;
    #[test]
    fn normal_rendering_includes_ready_child_scenes_without_capture_io() {
        let mut page = PageState::new(
            PageId(918),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document(
            "https://presentation.test/frames".into(),
            "<body><iframe srcdoc='<p>child scene</p>'></iframe></body>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page
                .presentation
                .scene
                .as_ref()
                .is_some_and(|scene| !scene.frames.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        let scene = page.presentation.scene.as_ref().unwrap();
        assert!(scene.frames[0].snapshot.nodes.iter().any(|node| {
            node.text
                .iter()
                .any(|text| text.text.contains("child scene"))
        }));
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "2"
        );
    }
}

#[cfg(test)]
mod composed_custody_tests {
    use super::*;
    fn page(style: &str) -> PageState {
        let mut page = PageState::new(
            PageId(919),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document("https://presentation.test/".into(),&format!("<body><div id=container style='{style}'><iframe srcdoc='<p>child content</p>'></iframe></div></body>"),None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
        page
    }
    #[test]
    fn rejected_ancestor_scene_does_not_publish_child_paint_timing() {
        let mut page = page("");
        struct ResetFailure;
        impl Drop for ResetFailure {
            fn drop(&mut self) {
                FAIL_COMPOSED_PRESENTATION.with(|fail| fail.set(false));
            }
        }
        let reset = ResetFailure;
        FAIL_COMPOSED_PRESENTATION.with(|fail| fail.set(true));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page.trace.events().iter().any(|event| matches!(&event.kind,
                TraceEventKind::ScriptDiagnostic { capability, detail }
                    if capability == "rendering.presentation" && detail.contains("injected composed scene rejection")))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        drop(reset);
        assert!(!page.presentation.building);
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "0"
        );
        page.presentation.next = None;
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "2"
        );
    }
    #[test]
    fn cross_document_resize_uses_current_child_geometry_before_parent_acceptance() {
        let mut page = page("");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page
                .presentation
                .scene
                .as_ref()
                .is_some_and(|scene| !scene.frames.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        page.executable.as_mut().unwrap().evaluate("globalThis.observedWidth=0;let target=frames[0].document.querySelector('p');target.style.width='61px';new ResizeObserver(entries=>{observedWidth=entries[0].contentBoxSize[0].inlineSize;document.getElementById('container').style.width='123px'}).observe(target)").unwrap();
        page.invalidate_presented_layout_deadlines();
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("observedWidth")
                .unwrap(),
            "61"
        );
        assert!(
            page.presentation
                .scene
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|node| node
                    .geometry
                    .box_model
                    .as_ref()
                    .is_some_and(|model| model.content_box.width == 123.))
        );
    }
    #[test]
    fn root_semantic_click_uses_live_child_provider_and_rejects_hidden_or_removed_targets() {
        let mut page = page("");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page.presentation.scene.as_ref().is_some_and(|scene| {
                scene.frames.first().is_some_and(|frame| {
                    frame.snapshot.nodes.iter().any(|node| {
                        node.text
                            .iter()
                            .any(|text| text.text.contains("child content"))
                    })
                })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        page.executable.as_mut().unwrap().evaluate("frames[0].clicks=0;frames[0].document.body.innerHTML='<button id=child>Child action</button>';frames[0].document.getElementById('child').onclick=()=>frames[0].clicks++").unwrap();
        page.poll_page_tasks().unwrap();
        let target = page
            .observation(ProjectionProfile::Full)
            .nodes
            .iter()
            .find(|node| node.name == "Child action")
            .unwrap()
            .semantic_ref;
        assert!(
            !page
                .layout
                .as_ref()
                .unwrap()
                .geometry
                .contains_key(&target.node)
        );
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.querySelector('iframe').style.display='none'")
            .unwrap();
        assert!(
            page.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None
            })
            .is_err()
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].clicks")
                .unwrap(),
            "0"
        );
        page.executable.as_mut().unwrap().evaluate("document.querySelector('iframe').style.display='';let cover=document.createElement('div');cover.id='cover';cover.style.cssText='position:fixed;inset:0;z-index:999999;background:black';document.body.append(cover)").unwrap();
        assert!(
            page.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None
            })
            .is_err()
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].clicks")
                .unwrap(),
            "0"
        );
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.getElementById('cover').remove()")
            .unwrap();
        page.execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        })
        .unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].clicks")
                .unwrap(),
            "1"
        );
        // Entry into a different target can synchronously install an ancestor
        // overlay. The second gate must run before any press or click occurs.
        page.evaluate("frames[0].presses=0;let second=frames[0].document.createElement('button');second.textContent='Covered on entry';second.onpointerdown=()=>frames[0].presses++;second.onclick=()=>frames[0].clicks++;second.onpointerenter=()=>{let cover=document.createElement('div');cover.id='cover';cover.style.cssText='position:fixed;inset:0;z-index:999999;background:black';document.body.append(cover)};frames[0].document.body.append(second)").unwrap();
        let covered = page
            .observation(ProjectionProfile::Full)
            .nodes
            .iter()
            .find(|node| node.name == "Covered on entry")
            .unwrap()
            .semantic_ref;
        assert!(
            page.execute(NativeAction {
                target: covered,
                action: SemanticAction::Click,
                value: None
            })
            .is_err()
        );
        assert_eq!(page.executable.as_mut().unwrap().evaluate("[!!document.getElementById('cover'),frames[0].presses,frames[0].clicks].join('|')").unwrap(),"true|0|1");
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.getElementById('cover').remove()")
            .unwrap();
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("frames[0].document.getElementById('child').style.display='none'")
            .unwrap();
        assert!(
            page.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None
            })
            .is_err()
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].clicks")
                .unwrap(),
            "1"
        );
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.querySelector('iframe').remove()")
            .unwrap();
        page.poll_page_tasks().unwrap();
        assert!(matches!(
            page.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None
            }),
            Err(RuntimeError::StaleSemanticRef)
        ));
    }

    #[test]
    fn child_presentation_uses_its_own_native_stylesheet_provider() {
        let mut page = PageState::new(
            PageId(920),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document("https://presentation.test/styles".into(),"<style>p { width: 211px }</style><body><p>root</p><iframe srcdoc='<style>p { width: 37px }</style><p>child</p>'></iframe>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page
                .presentation
                .scene
                .as_ref()
                .is_some_and(|scene| !scene.frames.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        let scene = page.presentation.scene.as_ref().unwrap();
        let width = |scene: &PaintSnapshot| {
            scene
                .nodes
                .iter()
                .find(|node| node.tag.eq_ignore_ascii_case("p"))
                .unwrap()
                .geometry
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .width
        };
        assert_eq!(width(scene), 211.);
        assert_eq!(width(&scene.frames[0].snapshot), 37.);
        assert_eq!(scene.nodes.iter().filter(|node| node.tag == "p").count(), 1);
        assert_eq!(
            scene.frames[0]
                .snapshot
                .nodes
                .iter()
                .filter(|node| node.tag == "p")
                .count(),
            1
        );
    }
    #[test]
    fn nested_presentation_restores_parent_scope_and_commits_descendant_timings() {
        let mut page = page("");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page
                .presentation
                .scene
                .as_ref()
                .is_some_and(|scene| !scene.frames.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        page.executable.as_mut().unwrap().evaluate("let child=frames[0].document.createElement('iframe');child.srcdoc='<p>nested content</p>';frames[0].document.body.append(child)").unwrap();
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page.presentation.scene.as_ref().is_some_and(|scene| {
                scene
                    .frames
                    .first()
                    .is_some_and(|child| !child.snapshot.frames.is_empty())
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        assert!(
            page.executable
                .as_ref()
                .unwrap()
                .current_automation_scope()
                .is_none()
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "2"
        );
    }
    #[test]
    fn transparent_embedding_ancestor_defers_child_milestones_until_visible() {
        let mut page = page("opacity:0");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            page.presentation.next = None;
            page.poll_page_tasks().unwrap();
            if page
                .presentation
                .scene
                .as_ref()
                .is_some_and(|scene| !scene.frames.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                presentation_gate_summary(&page)
            );
        }
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "0"
        );
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.getElementById('container').style.opacity='1'")
            .unwrap();
        page.presentation.next = None;
        page.poll_page_tasks().unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("frames[0].performance.getEntriesByType('paint').length")
                .unwrap(),
            "2"
        );
    }
}
