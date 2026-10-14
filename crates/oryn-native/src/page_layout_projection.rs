//! Layout, geometry, viewport, and scroll projection for a page.

use super::*;

impl PageState {
    #[cfg(feature = "v8-host")]
    pub(super) fn refresh_layout(&mut self) -> Result<(), RuntimeError> {
        if self.refresh_page_task_layout()? {
            Ok(())
        } else {
            Err(RuntimeError::PageTaskYield)
        }
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn refresh_page_task_layout(&mut self) -> Result<bool, RuntimeError> {
        let _profile = crate::native_timing::stage("deep.page_task_layout");
        let _close_phase = crate::ipc_progress::close_admission::phase("layout");
        if self.terminal_close_requested() {
            return Ok(false);
        }
        // The root phase already samples the complete native realm tree.
        // Preparing its child scenes must not start another author rAF phase.
        if !(self.presentation.building && self.frame_scope.is_some())
            && let Some(executable) = self.executable.as_mut()
        {
            let _profile = crate::native_timing::stage("deep.animation_frame_phase");
            executable
                .run_animation_frame_phase()
                .map_err(map_host_error)?;
        }
        if self.terminal_close_requested() {
            return Ok(false);
        }
        match self.refresh_layout_geometry() {
            Err(RuntimeError::PageTaskYield) => Ok(false),
            result => result.map(|()| !self.terminal_close_requested()),
        }
    }

    #[cfg(feature = "v8-host")]
    fn layout_terminal_checkpoint(&self) -> Result<(), RuntimeError> {
        if self.terminal_close_requested() {
            Err(RuntimeError::PageTaskYield)
        } else {
            Ok(())
        }
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn refresh_layout_geometry(&mut self) -> Result<(), RuntimeError> {
        self.layout_terminal_checkpoint()?;
        let _timing = crate::native_timing::stage("page.layout_projection");
        let input_timing = crate::native_timing::stage("page.layout_input");
        // Geometry projection is not another task or rendering opportunity.
        // Its caller owns checkpoints; observe current mutations without
        // draining timers, parser work or frame callbacks a second time.
        let layout_document = self
            .executable
            .as_ref()
            .map(|executable| {
                let _timing = crate::native_timing::stage("page.semantic_snapshot");
                let document = executable.semantic_snapshot();
                if document.is_ok() {
                    crate::native_timing::count("page.semantic_snapshot", 1);
                }
                document
            })
            .transpose()
            .map_err(map_host_error)?;
        self.layout_terminal_checkpoint()?;
        let document = layout_document
            .as_ref()
            .or(self.document.as_ref())
            .ok_or_else(|| RuntimeError::LayoutUnavailable("page has no document".into()))?;
        let mut sampled_scroll = self.scroll_state;
        if let Some(executable) = self.executable.as_ref() {
            let offset = executable
                .document_scroll_offset()
                .map_err(map_host_error)?;
            sampled_scroll.x = offset.0;
            sampled_scroll.y = offset.1;
        }
        let mut input = layout_input(
            document,
            self.viewport,
            sampled_scroll,
            self.color_scheme,
            self.reduced_motion,
            &self.web_fonts,
        );
        input.scrollbar_width = self.config.native_scrollbar_width;
        if let Some(executable) = self.executable.as_ref() {
            input.element_scrolls = executable
                .element_scroll_offsets()
                .map_err(map_host_error)?
                .into_iter()
                .map(|(node, offset)| {
                    (
                        ((u64::from(node.generation) << 32) | u64::from(node.slot)),
                        offset,
                    )
                })
                .collect();
        }
        input.focused_node = self
            .executable
            .as_ref()
            .and_then(|executable| executable.focused_layout_node());
        input.focus_visible = self
            .executable
            .as_ref()
            .is_some_and(|executable| executable.focus_visible());
        if let Some(executable) = self.executable.as_ref() {
            executable
                .populate_validation_states(&mut input)
                .map_err(map_host_error)?;
        }
        let limits = oryn_layout::LayoutLimits {
            max_nodes: self.config.max_layout_nodes,
            max_font_bytes: self.config.max_font_bytes,
            max_page_font_bytes: self.config.max_page_font_bytes,
            max_viewport_css_px: f64::from(self.config.max_viewport_css_px),
        };
        drop(input_timing);
        self.layout_terminal_checkpoint()?;
        // Compare the complete native input, not just semantic revision: focus,
        // CSSOM, validation state, fonts, media, and viewport can affect layout
        // without changing an accessible name. Keep only one input per page.
        let animation_revision = self
            .executable
            .as_ref()
            .map_or(0, |executable| executable.native_animation_revision());
        if self.layout.is_some()
            && self.layout_animation_revision == animation_revision
            && !self
                .executable
                .as_ref()
                .is_some_and(|executable| executable.native_animations_active())
            && self
                .layout_input_cache
                .as_ref()
                .is_some_and(|(generation, previous, _)| {
                    *generation == self.document_generation && previous == &input
                })
        {
            crate::native_timing::count("page.layout_cache_hits", 1);
            // Equal layout input is not a lifecycle/control/diagnostic custody
            // token; this projection still requires semantic synchronization.
            self.sync_executable_semantics()?;
            self.layout_terminal_checkpoint()?;
            return Ok(());
        }
        let computation = checked_layout_computation(|| {
            let _timing = crate::native_timing::stage("page.layout_compute");
            if let Some(executable) = self.executable.as_ref() {
                executable.compute_native_layout(&input, &limits)
            } else {
                let output = self.style_engine.compute(&input, &limits)?;
                let sample = self.style_engine.instance_style_sample().ok_or_else(|| {
                    oryn_layout::LayoutError::StyleTraversal(
                        "successful layout lacks native style sample".into(),
                    )
                })?;
                Ok((output, sample))
            }
        });
        self.layout_terminal_checkpoint()?;
        let (output, style_sample) = computation?;
        crate::native_timing::count("page.layout_compute", 1);
        let sampled_animation_revision = self
            .executable
            .as_ref()
            .map_or(0, |executable| executable.native_animation_revision());
        if let Some(executable) = self.executable.as_mut() {
            {
                let _profile = crate::native_timing::stage("deep.install_layout_media");
                executable
                    .install_layout_with_task_checkpoint(&output, self.viewport, false)
                    .map_err(map_host_error)?;
                executable
                    .install_media(
                        match self.color_scheme {
                            PreferredColorScheme::Light => "light",
                            PreferredColorScheme::Dark => "dark",
                        },
                        match self.reduced_motion {
                            ReducedMotion::Reduce => "reduce",
                            ReducedMotion::NoPreference => "no_preference",
                        },
                    )
                    .map_err(map_host_error)?;
            }
        }
        let mut prepared = self.prepare_executable_semantics()?;
        let owner = self.page_task_terminal_control.clone();
        let mut check = || {
            if owner.retired() {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        };
        let map_projection = |error| match error {
            crate::dom::DomError::ProjectionCancelled => RuntimeError::PageTaskYield,
            error => RuntimeError::Semantic(error.to_string()),
        };
        if let Some(draft) = prepared.as_mut() {
            draft
                .graph
                .apply_computed_visibility_with_checkpoint(
                    &draft.document.dom,
                    &output.computed_styles,
                    &mut check,
                )
                .map_err(map_projection)?;
            draft
                .graph
                .apply_generated_content_with_checkpoint(
                    &draft.document.dom,
                    &output.generated_content,
                    &mut check,
                )
                .map_err(map_projection)?;
            if let Some(executable) = self.executable.as_ref() {
                executable
                    .populate_semantic_control_state(&mut draft.graph)
                    .map_err(|error| {
                        if owner.retired() {
                            RuntimeError::PageTaskYield
                        } else {
                            map_host_error(error)
                        }
                    })?;
            }
            draft
                .graph
                .apply_geometry_visibility_with_checkpoint(&output.geometry, &mut check)
                .map_err(map_projection)?;
        }
        self.layout_terminal_checkpoint()?;
        let controller = owner
            .controller()
            .cloned()
            .ok_or(RuntimeError::PageTaskYield)?;
        let mut cache_publication = Some((self.document_generation, input, style_sample));
        let unused_cache = if output.geometry.is_empty() {
            cache_publication.take()
        } else {
            None
        };
        let mut layout_publication = Some(output);
        let admitted = controller.with_live_publication(|| {
            let retired_semantics = if prepared.is_some() {
                Some(self.commit_prepared_semantics_admitted(&mut prepared)?)
            } else {
                None
            };
            let output = layout_publication.take().expect("retained layout draft");
            self.style_revision = self.style_revision.saturating_add(1);
            self.layout_revision = self.layout_revision.saturating_add(1);
            self.scroll_state = output.scroll;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::StyleRecalculated {
                    revision: self.style_revision,
                    nodes: output.computed_styles.len(),
                },
            );
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LayoutCompleted {
                    revision: self.layout_revision,
                    nodes: output.geometry.len(),
                },
            );
            for font in &output.loaded_fonts {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::FontLoaded {
                        family: font.family.clone(),
                        sha256: font.sha256.clone(),
                    },
                );
            }
            // Empty geometry is not a reusable layout result. Computation failures
            // returned above before any style/geometry revision was published.
            let retired_cache =
                std::mem::replace(&mut self.layout_input_cache, cache_publication.take());
            // Capture after sampling, before observer callbacks during installation.
            // A callback publishing a later effect must invalidate the next layout.
            self.layout_animation_revision = sampled_animation_revision;
            let retired_layout = self.layout.replace(output);
            Ok::<_, RuntimeError>((retired_semantics, retired_cache, retired_layout))
        });
        drop(prepared);
        drop(layout_publication);
        drop(cache_publication);
        drop(unused_cache);
        match admitted {
            Ok(result) => {
                drop(result?);
                Ok(())
            }
            Err(_) => Err(RuntimeError::PageTaskYield),
        }
    }

    #[cfg(not(feature = "v8-host"))]
    pub(super) fn refresh_layout(&mut self) -> Result<(), RuntimeError> {
        Err(RuntimeError::LayoutUnavailable(
            "layout is available only in the V8 page worker".into(),
        ))
    }

    pub(super) fn validate_target(&self, target: SemanticRef) -> Result<(), RuntimeError> {
        if target.page_id != self.page_id {
            return Err(RuntimeError::WrongPage {
                expected: self.page_id,
                actual: target.page_id,
            });
        }
        if target.document_generation != self.document_generation {
            return Err(RuntimeError::WrongDocumentGeneration {
                expected: self.document_generation,
                actual: target.document_generation,
            });
        }
        let node = crate::semantic::unpack_node_ref(target.node);
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let mut ancestor = Some(node);
        while let Some(id) = ancestor {
            document
                .dom
                .get(id)
                .map_err(|_| RuntimeError::StaleSemanticRef)?;
            if id == document.document {
                return Ok(());
            }
            ancestor = document
                .dom
                .embedding_parent(id)
                .map_err(|_| RuntimeError::StaleSemanticRef)?;
        }
        Err(RuntimeError::StaleSemanticRef)
    }

    pub(super) fn computed_style(
        &mut self,
        target: SemanticRef,
    ) -> Result<ComputedStyle, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        {
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
            let properties = self
                .layout
                .as_ref()
                .and_then(|layout| layout.computed_styles.get(&target.node))
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::LayoutUnavailable("target has no computed style".into())
                })?;
            Ok(ComputedStyle {
                semantic_ref: target,
                style_revision: oryn_common::v4::StyleRevision(self.style_revision),
                properties: (*properties).clone(),
            })
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "computed style requires the V8 page worker".into(),
        ))
    }

    pub(super) fn box_model(&mut self, target: SemanticRef) -> Result<BoxModel, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        {
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
            let geometry = self
                .layout
                .as_ref()
                .and_then(|layout| layout.geometry.get(&target.node))
                .ok_or_else(|| RuntimeError::LayoutUnavailable("target has no geometry".into()))?;
            geometry.box_model.clone().ok_or_else(|| {
                RuntimeError::UnsupportedGeometry(
                    geometry
                        .diagnostic
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", geometry.availability)),
                )
            })
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "box geometry requires the V8 page worker".into(),
        ))
    }

    pub(super) fn set_viewport(
        &mut self,
        viewport: ViewportState,
    ) -> Result<ViewportState, RuntimeError> {
        let max = f64::from(self.config.max_viewport_css_px);
        if !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || !viewport.device_pixel_ratio.is_finite()
            || !(1.0..=max).contains(&viewport.width)
            || !(1.0..=max).contains(&viewport.height)
            || !(0.1..=8.0).contains(&viewport.device_pixel_ratio)
        {
            return Err(RuntimeError::InvalidViewport(format!(
                "{}x{} at DPR {} exceeds configured bounds",
                viewport.width, viewport.height, viewport.device_pixel_ratio
            )));
        }
        self.viewport = viewport;
        self.refresh_layout()?;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::ViewportChanged {
                width: viewport.width,
                height: viewport.height,
                device_pixel_ratio: viewport.device_pixel_ratio,
            },
        );
        Ok(viewport)
    }

    pub(super) fn set_media(&mut self, media: MediaState) -> Result<MediaState, RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = media;
        #[cfg(feature = "v8-host")]
        {
            self.color_scheme = media.color_scheme;
            self.reduced_motion = media.reduced_motion;
            self.refresh_layout()?;
            Ok(media)
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "media emulation requires the V8 page worker".into(),
        ))
    }

    pub(super) fn scroll(&mut self, request: ScrollRequest) -> Result<ScrollState, RuntimeError> {
        if !request.x.is_finite() || !request.y.is_finite() {
            return Err(RuntimeError::InvalidViewport(
                "scroll coordinates must be finite".into(),
            ));
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_ref() {
            let offset = executable
                .document_scroll_offset()
                .map_err(map_host_error)?;
            self.scroll_state.x = offset.0;
            self.scroll_state.y = offset.1;
        }
        if request.relative {
            self.scroll_state.x += request.x;
            self.scroll_state.y += request.y;
        } else {
            self.scroll_state.x = request.x;
            self.scroll_state.y = request.y;
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            executable
                .set_document_scroll_offset((self.scroll_state.x, self.scroll_state.y))
                .map_err(map_host_error)?;
        }
        self.refresh_layout()?;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::Scrolled {
                x: self.scroll_state.x,
                y: self.scroll_state.y,
            },
        );
        Ok(self.scroll_state)
    }

    pub(super) fn scroll_into_view(
        &mut self,
        target: SemanticRef,
    ) -> Result<ScrollState, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_ref() {
            executable
                .scroll_node_into_view(crate::semantic::unpack_node_ref(target.node))
                .map_err(map_host_error)?;
            self.refresh_layout()?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::Scrolled {
                    x: self.scroll_state.x,
                    y: self.scroll_state.y,
                },
            );
            return Ok(self.scroll_state);
        }
        let box_model = self.box_model(target)?;
        if box_model.border_box.y < 0.0 || box_model.border_box.bottom() > self.viewport.height {
            self.scroll_state.y = (self.scroll_state.y + box_model.border_box.y).max(0.0);
            #[cfg(feature = "v8-host")]
            if let Some(executable) = self.executable.as_mut() {
                executable
                    .set_document_scroll_offset((self.scroll_state.x, self.scroll_state.y))
                    .map_err(map_host_error)?;
            }
            self.refresh_layout()?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::Scrolled {
                    x: self.scroll_state.x,
                    y: self.scroll_state.y,
                },
            );
        }
        Ok(self.scroll_state)
    }
}
