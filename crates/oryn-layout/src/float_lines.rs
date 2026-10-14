//! Native line breaking against Taffy's float exclusion bands. Custom inline
//! box yields retain DOM order; no text is wrapped using guessed font metrics.

/// Inputs for break layout.
pub(crate) struct FloatLineInputs<'a> {
    pub(crate) layout: &'a mut Layout<TextBrush>,
    pub(crate) width: f64,
    pub(crate) specs: &'a BTreeMap<u64, FloatSpec>,
    pub(crate) context: &'a mut FloatContext,
    pub(crate) origin: f64,
    pub(crate) context_width: f64,
    pub(crate) containing_insets: [f64; 2],
    pub(crate) fragment: Option<(f64, f64)>,
}
use super::*;
use taffy::{Clear, FloatDirection, compute::FloatContext};
#[derive(Clone, Copy)]
pub(super) struct FloatSpec {
    pub width: f64,
    pub height: f64,
    pub right: bool,
    pub clear: Clear,
    pub monolithic: bool,
}
impl FloatSpec {
    /// An oversized replaced float overflows one fragment; it does not acquire
    /// continuation exclusion bands in later columns. Its paint box stays whole.
    pub(super) fn exclusion_height(self, fragment: Option<(f64, f64)>) -> f64 {
        if self.monolithic {
            fragment
                .filter(|(height, _)| height.is_finite() && *height > 0.)
                .map(|(height, _)| self.height.min(height))
                .unwrap_or(self.height)
        } else {
            self.height
        }
    }
    pub(super) fn place(
        self,
        context: &mut FloatContext,
        y: f64,
        insets: [f32; 2],
        fragment: Option<(f64, f64)>,
    ) -> taffy::geometry::Point<f32> {
        let mut y = fragment_start(y, self.height, fragment) as f32;
        loop {
            let mut trial = context.clone();
            let position = trial.place_floated_box(
                taffy::geometry::Size {
                    width: self.width as f32,
                    height: self.exclusion_height(fragment) as f32,
                },
                y,
                insets,
                if self.right {
                    FloatDirection::Right
                } else {
                    FloatDirection::Left
                },
                self.clear,
            );
            let next = fragment_start(f64::from(position.y), self.height, fragment) as f32;
            if next <= position.y {
                *context = trial;
                return position;
            }
            // Native float bands can move the candidate past its original column. Refit
            // there before publishing exclusion; every retry passes a band or boundary.
            y = next;
        }
    }
}
/// Find the next safe block position when a BFC reaches a later exclusion band.
/// Float margins participate in exclusion; the candidate is its border box.
pub(super) fn intersecting_bottom(
    context: &FloatContext,
    width: f64,
    rect: CssRect,
) -> Option<f64> {
    context
        .left_floats()
        .iter()
        .map(|value| (f64::from(value.x_inset), value))
        .chain(
            context
                .right_floats()
                .iter()
                .map(|value| (width - f64::from(value.x_inset + value.width), value)),
        )
        .filter_map(|(x, value)| {
            let y = f64::from(value.y);
            let right = x + f64::from(value.width);
            let bottom = y + f64::from(value.height);
            (rect.width > 0.
                && rect.height > 0.
                && rect.x < right - 0.0001
                && rect.right() > x + 0.0001
                && rect.y < bottom - 0.0001
                && rect.bottom() > y + 0.0001)
                .then_some(bottom)
        })
        .min_by(f64::total_cmp)
}
pub(super) fn slot(
    context: &FloatContext,
    y: f64,
    min_height: f32,
    insets: [f32; 2],
) -> taffy::compute::ContentSlot {
    let mut result = context.find_content_slot(y as f32, insets, Clear::None, None);
    loop {
        let top = result.y;
        let bottom = top + min_height;
        let mut left = result.x;
        let mut right = result.x + result.width;
        let mut next_start = f32::INFINITY;
        let mut next_bottom = f32::INFINITY;
        // Taffy's ContentSlot deliberately has infinite height. Native glyph runs
        // may cross a later, narrower band, so bound the initial break at that band
        // and use the complete actual Parley height when retrying.
        for value in context.left_floats() {
            if value.y >= bottom && value.y > top {
                next_start = next_start.min(value.y);
            }
            if (value.y <= top || value.y < bottom) && value.y + value.height > top {
                left = left.max(value.x_inset + value.width);
                next_bottom = next_bottom.min(value.y + value.height);
            }
        }
        // Recover the containing width from a float-free slot; current slot edges
        // already include exclusions, so they are not a containing-width proxy.
        let domain = context.find_content_slot(f32::MAX, insets, Clear::None, None);
        let full_width = domain.x + domain.width + insets[1];
        for value in context.right_floats() {
            if value.y >= bottom && value.y > top {
                next_start = next_start.min(value.y);
            }
            if (value.y <= top || value.y < bottom) && value.y + value.height > top {
                right = right.min(full_width - value.x_inset - value.width);
                next_bottom = next_bottom.min(value.y + value.height);
            }
        }
        result.x = left;
        result.width = (right - left).max(0.);
        result.height = (next_start - top).max(min_height);
        if result.width > 0. || !next_bottom.is_finite() {
            return result;
        }
        result = context.find_content_slot(next_bottom, insets, Clear::None, None);
    }
}
pub(super) fn fragment_start(y: f64, extent: f64, fragment: Option<(f64, f64)>) -> f64 {
    let Some((height, origin)) = fragment
        .filter(|(height, origin)| height.is_finite() && *height > 0. && origin.is_finite())
    else {
        return y;
    };
    let position = (y - origin).rem_euclid(height);
    let remaining = height - position;
    if position > 0.0001 && extent > remaining + 0.0001 {
        y + remaining
    } else {
        y
    }
}
pub(super) fn fragment_slot(
    context: &FloatContext,
    y: f64,
    min_height: f32,
    insets: [f32; 2],
    fragment: Option<(f64, f64)>,
) -> taffy::compute::ContentSlot {
    let mut available = slot(
        context,
        fragment_start(y, f64::from(min_height), fragment),
        min_height,
        insets,
    );
    if let Some((height, origin)) =
        fragment.filter(|(height, origin)| height.is_finite() && *height > 0. && origin.is_finite())
    {
        let start = fragment_start(f64::from(available.y), f64::from(min_height), fragment);
        if start > f64::from(available.y) + 0.0001 {
            available = slot(context, start, min_height, insets);
        }
        let remaining = height - (f64::from(available.y) - origin).rem_euclid(height);
        available.height = available
            .height
            .min(remaining.max(f64::from(min_height)) as f32);
    }
    available
}
fn configure(
    state: &mut parley::layout::BreakerState,
    slot: taffy::compute::ContentSlot,
    width: f64,
    left: f32,
) {
    // A slot is expressed in the enclosing BFC; Parley needs the intersection
    // with this paragraph's content interval, including during intrinsic probes.
    // Text indent is applied by Parley after this available-width calculation.
    let width = width.max(0.) as f32;
    let x = (slot.x - left).max(0.);
    state.set_layout_max_advance(width);
    state.set_line_x(x);
    state.set_line_y(f64::from(slot.y));
    state.set_line_max_advance(slot.width.max(0.).min((width - x).max(0.)));
    state.set_line_max_height(slot.height.max(0.));
}
#[cfg(test)]
pub(super) fn break_layout(params: FloatLineInputs<'_>) -> Vec<InlinePlacement> {
    break_layout_controlled(params, None)
}

pub(super) fn break_layout_controlled(
    params: FloatLineInputs<'_>,
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
) -> Vec<InlinePlacement> {
    let FloatLineInputs {
        layout,
        width,
        specs,
        context,
        origin,
        context_width,
        containing_insets,
        fragment,
    } = params;

    let width = width.max(0.);
    let context_width = (if context_width > 0. {
        context_width
    } else {
        width
    })
    .max(0.);
    // Measurement can use a narrower width than the retained float scope. Keep
    // exclusions in BFC coordinates, but derive the right edge from the actual
    // paragraph width rather than reusing the ancestor's right inset.
    let insets = [
        containing_insets[0] as f32,
        (context_width - containing_insets[0] - width) as f32,
    ];
    context.set_width(context_width as f32);
    let mut placed = BTreeMap::<u64, InlinePlacement>::new();
    let mut deferred = Vec::<(u64, FloatSpec)>::new();
    let mut breaker = layout.break_lines();
    configure(
        breaker.state_mut(),
        fragment_slot(context, origin, 0., insets, fragment),
        width,
        insets[0],
    );
    let mut checkpoint = breaker.state().clone();
    while let Some(yielded) = breaker.break_next() {
        if cancelled.as_ref().is_some_and(|check| check()) {
            return Vec::new();
        }
        match yielded {
            parley::layout::YieldData::InlineBoxBreak(data) => {
                if let Some(spec) = specs.get(&data.inline_box_id).copied()
                    && !placed.contains_key(&data.inline_box_id)
                    && !deferred.iter().any(|(id, _)| *id == data.inline_box_id)
                {
                    let mut trial = context.clone();
                    let position =
                        spec.place(&mut trial, breaker.state().line_y(), insets, fragment);
                    let available =
                        fragment_slot(&trial, breaker.state().line_y(), 0., insets, fragment);
                    if data.advance > 0.
                        && (available.y as f64 > breaker.state().line_y() + 0.0001
                            || available.width + 0.0001 < data.advance)
                    {
                        deferred.push((data.inline_box_id, spec)); // An unbreakable run may already overflow. Its consumed advance is
                        // not additional available space for the following text or float.
                        let limit = breaker.state().line_max_advance().min(data.advance);
                        breaker.state_mut().set_line_max_advance(limit);
                    } else {
                        *context = trial;
                        placed.insert(
                            data.inline_box_id,
                            InlinePlacement {
                                node: data.inline_box_id,
                                rect: CssRect {
                                    x: f64::from(position.x),
                                    y: f64::from(position.y),
                                    width: spec.width,
                                    height: spec.height,
                                },
                                line: usize::MAX,
                            },
                        );
                        configure(breaker.state_mut(), available, width, insets[0]);
                    }
                }
                breaker
                    .state_mut()
                    .append_inline_box_to_line(data.advance, 0.);
            }
            parley::layout::YieldData::LineBreak(_) => {
                for (id, spec) in deferred.drain(..) {
                    let position = spec.place(context, breaker.state().line_y(), insets, fragment);
                    placed.insert(
                        id,
                        InlinePlacement {
                            node: id,
                            rect: CssRect {
                                x: f64::from(position.x),
                                y: f64::from(position.y),
                                width: spec.width,
                                height: spec.height,
                            },
                            line: usize::MAX,
                        },
                    );
                }
                let available =
                    fragment_slot(context, breaker.state().line_y(), 0., insets, fragment);
                configure(breaker.state_mut(), available, width, insets[0]);
                checkpoint = breaker.state().clone();
            }
            parley::layout::YieldData::MaxHeightExceeded(data) => {
                let available = fragment_slot(
                    context,
                    breaker.state().line_y(),
                    data.line_height,
                    insets,
                    fragment,
                );
                breaker.revert_to(checkpoint.clone());
                configure(breaker.state_mut(), available, width, insets[0]);
                checkpoint = breaker.state().clone();
            }
        }
    }
    breaker.finish();
    placed
        .into_values()
        .map(|mut item| {
            item.rect.y -= origin;
            item.rect.x -= containing_insets[0];
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tall_inline_box_reserves_text_descent_and_retries_full_line_height() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre".into()),
            ("line-height".into(), "20px".into()),
        ]);
        let mut plain = engine.paragraph_layout("A", 16., &style, ViewportState::default(), &[]);
        plain.break_all_lines(Some(200.));
        let first = plain.lines().next().unwrap();
        let metrics = first.metrics();
        let below = metrics.line_height - (metrics.baseline - metrics.block_origin);
        engine.active_inline_boxes.push(parley::InlineBox {
            id: 77,
            kind: parley::InlineBoxKind::InFlow,
            index: 0,
            width: 10.,
            height: 100.,
        });
        let mut layout =
            engine.paragraph_layout("A\nB", 16., &style, ViewportState::default(), &[]);
        {
            let mut breaker = layout.break_lines();
            breaker.state_mut().set_layout_max_advance(200.);
            breaker.state_mut().set_line_max_advance(200.);
            breaker.state_mut().set_line_max_height(101.);
            let Some(parley::layout::YieldData::MaxHeightExceeded(required)) = breaker.break_next()
            else {
                panic!("combined baseline height must yield before committing text")
            };
            assert!((required.line_height - 100. - below).abs() < 0.001);
            breaker.state_mut().set_line_max_height(f32::MAX);
            while breaker.break_next().is_some() {}
            breaker.finish();
        }
        let lines = layout.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!((lines[0].metrics().baseline - 100.).abs() < 0.001);
        assert!((lines[0].metrics().line_height - 100. - below).abs() < 0.001);
        assert!((lines[1].metrics().block_origin - lines[0].metrics().line_height).abs() < 0.001);
    }
    #[test]
    fn inline_box_only_lines_keep_valid_source_insertion_ranges() {
        for kind in [
            parley::InlineBoxKind::InFlow,
            parley::InlineBoxKind::OutOfFlow,
        ] {
            let mut engine = TextEngine::new(&[]);
            engine.active_inline_boxes.push(parley::InlineBox {
                id: 77,
                kind,
                index: 0,
                width: 10.,
                height: 10.,
            });
            let mut layout =
                engine.paragraph_layout("", 16., &BTreeMap::new(), ViewportState::default(), &[]);
            layout.break_all_lines(Some(20.));
            assert!(layout.lines().next().is_some());
            assert!(layout.lines().all(|line| line.text_range() == (0..0)));
        }
    }
    #[test]
    fn paragraph_flow_height_is_independent_of_float_context_origin() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([("line-height".into(), "20px".into())]);
        for origin in [0., 20., 100.] {
            engine.float_context = Some(FloatContext::new());
            engine.float_context_width = 100.;
            engine.float_block_offset = origin;
            let layout = engine.shape_paragraph(crate::ParagraphShapeInputs {
                text: "abc",
                width: 100.,
                font_size: 16.,
                style: &style,
                viewport: ViewportState::default(),
                ranges: &[],
                first_paragraph: true,
            });
            assert_eq!(layout.height, 20.);
            assert_eq!(layout.line_heights, vec![20.]);
            assert_eq!(layout.lines[0].3, 0.);
        }
    }
    #[test]
    fn inherited_float_scope_uses_measured_paragraph_width_and_native_indent() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::new();
        for width in [0., 0.25, 32., 64.5] {
            for indent in [-4., 0., 4.] {
                let mut actual = engine.paragraph_layout(
                    "alpha beta gamma delta",
                    16.,
                    &style,
                    ViewportState::default(),
                    &[],
                );
                let mut expected = actual.clone();
                actual.set_text_indent(indent, Default::default());
                expected.set_text_indent(indent, Default::default());
                expected.break_all_lines(Some(width as f32));
                let mut context = FloatContext::new();
                break_layout(FloatLineInputs {
                    layout: &mut actual,
                    width,
                    specs: &BTreeMap::new(),
                    context: &mut context,
                    origin: 0.,
                    context_width: 500.,
                    containing_insets: [12., 8.],
                    fragment: None,
                });
                assert_eq!(
                    actual
                        .lines()
                        .map(|line| line.text_range())
                        .collect::<Vec<_>>(),
                    expected
                        .lines()
                        .map(|line| line.text_range())
                        .collect::<Vec<_>>(),
                    "measured width {width}, indent {indent}"
                );
            }
        }
    }
    #[test]
    fn overflowing_text_before_custom_float_does_not_expand_line_capacity() {
        let mut engine = TextEngine::new(&[]);
        let text = "unbreakableword tail";
        engine.active_inline_boxes.push(parley::InlineBox {
            id: 77,
            kind: parley::InlineBoxKind::CustomOutOfFlow,
            index: 15,
            width: 10.,
            height: 10.,
        });
        let mut layout =
            engine.paragraph_layout(text, 16., &BTreeMap::new(), ViewportState::default(), &[]);
        let specs = BTreeMap::from([(
            77,
            FloatSpec {
                width: 10.,
                height: 10.,
                right: false,
                clear: Clear::None,
                monolithic: false,
            },
        )]);
        let placements = break_layout(FloatLineInputs {
            layout: &mut layout,
            width: 1.,
            specs: &specs,
            context: &mut FloatContext::new(),
            origin: 0.,
            context_width: 1.,
            containing_insets: [0.; 2],
            fragment: None,
        });
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].node, 77);
        assert_eq!(layout.lines().last().unwrap().text_range().end, text.len());
    }
    #[test]
    fn cluster_visual_offsets_match_positioned_runs_beside_custom_float() {
        let mut engine = TextEngine::new(&[]);
        engine.active_inline_boxes.push(parley::InlineBox {
            id: 77,
            kind: parley::InlineBoxKind::CustomOutOfFlow,
            index: 0,
            width: 30.,
            height: 40.,
        });
        let mut layout =
            engine.paragraph_layout("abc", 16., &BTreeMap::new(), ViewportState::default(), &[]);
        let specs = BTreeMap::from([(
            77,
            FloatSpec {
                width: 30.,
                height: 40.,
                right: false,
                clear: Clear::None,
                monolithic: false,
            },
        )]);
        break_layout(FloatLineInputs {
            layout: &mut layout,
            width: 100.,
            specs: &specs,
            context: &mut FloatContext::new(),
            origin: 0.,
            context_width: 100.,
            containing_insets: [0.; 2],
            fragment: None,
        });
        layout.align(Alignment::Left, Default::default());
        let line = layout.lines().next().unwrap();
        let run = line
            .items()
            .find_map(|item| match item {
                parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
                _ => None,
            })
            .unwrap();
        let first = run.run().visual_clusters().next().unwrap();
        assert_eq!(run.offset(), 30.);
        assert_eq!(first.visual_offset(), Some(run.offset()));
    }
    #[test]
    fn paragraph_interval_intersects_inherited_float_exclusions() {
        let mut context = FloatContext::new();
        context.set_width(500.);
        context.place_floated_box(
            taffy::geometry::Size {
                width: 20.,
                height: 40.,
            },
            0.,
            [0.; 2],
            FloatDirection::Left,
            Clear::None,
        );
        let available = slot(&context, 0., 0., [12., 456.]);
        let mut state = parley::layout::BreakerState::default();
        configure(&mut state, available, 32., 12.);
        assert_eq!(state.line_x(), 8.);
        assert_eq!(state.line_max_advance(), 24.);
        assert_eq!(state.layout_max_advance(), 32.);
        // Negative margins may put the paragraph outside the BFC; its own width
        // still defines the available interval and must not be rounded up to 1px.
        configure(
            &mut state,
            taffy::compute::ContentSlot {
                segment_id: None,
                x: -10.,
                y: 0.,
                width: 500.,
                height: f32::INFINITY,
            },
            0.25,
            -10.,
        );
        assert_eq!(state.line_x(), 0.);
        assert_eq!(state.line_max_advance(), 0.25);
    }
    #[test]
    fn monolithic_float_owns_one_exclusion_fragment() {
        let replaced = FloatSpec {
            width: 30.,
            height: 150.,
            right: false,
            clear: Clear::None,
            monolithic: true,
        };
        assert_eq!(replaced.exclusion_height(Some((100., 0.))), 100.);
        assert_eq!(replaced.exclusion_height(None), 150.);
        assert_eq!(
            FloatSpec {
                monolithic: false,
                ..replaced
            }
            .exclusion_height(Some((100., 0.))),
            150.
        );
    }
    #[test]
    fn collision_rechecks_fragment_before_publishing_replaced_exclusion() {
        let mut context = FloatContext::new();
        context.set_width(100.);
        context.place_floated_box(
            taffy::geometry::Size {
                width: 100.,
                height: 150.,
            },
            0.,
            [0.; 2],
            FloatDirection::Left,
            Clear::None,
        );
        let spec = FloatSpec {
            width: 30.,
            height: 150.,
            right: false,
            clear: Clear::None,
            monolithic: true,
        };
        let position = spec.place(&mut context, 0., [0.; 2], Some((100., 0.)));
        assert_eq!(position.y, 200.);
        assert_eq!(slot(&context, 310., 0., [0.; 2]).width, 100.);
    }
    #[test]
    fn fragmentation_moves_fitting_floats_and_keeps_oversize_progress() {
        assert_eq!(fragment_start(40., 40., Some((60., 0.))), 60.);
        assert_eq!(fragment_start(60., 80., Some((60., 0.))), 60.);
        assert_eq!(fragment_start(45., 10., Some((60., 0.))), 45.);
        let mut context = FloatContext::new();
        context.set_width(100.);
        let line = fragment_slot(&context, 50., 20., [0.; 2], Some((60., 0.)));
        assert_eq!(line.y, 60.);
        assert_eq!(line.height, 60.);
    }
    #[test]
    fn actual_tall_line_uses_union_of_intersecting_float_bands() {
        let mut context = FloatContext::new();
        context.set_width(200.);
        context.place_floated_box(
            taffy::geometry::Size {
                width: 30.,
                height: 20.,
            },
            0.,
            [0.; 2],
            FloatDirection::Left,
            Clear::None,
        );
        context.place_floated_box(
            taffy::geometry::Size {
                width: 60.,
                height: 40.,
            },
            30.,
            [0.; 2],
            FloatDirection::Right,
            Clear::None,
        );
        let short = slot(&context, 0., 0., [0.; 2]);
        assert_eq!(short.x, 30.);
        assert_eq!(short.width, 170.);
        assert_eq!(short.height, 30.);
        let tall = slot(&context, 0., 50., [0.; 2]);
        assert_eq!(tall.x, 30.);
        assert_eq!(tall.width, 110.);
        assert!(tall.height >= 50.);
    }
    #[test]
    fn bfc_border_box_checks_later_float_bands_and_both_sides() {
        let mut context = FloatContext::new();
        context.set_width(200.);
        context.place_floated_box(
            taffy::geometry::Size {
                width: 30.,
                height: 20.,
            },
            0.,
            [0.; 2],
            FloatDirection::Left,
            Clear::None,
        );
        context.place_floated_box(
            taffy::geometry::Size {
                width: 60.,
                height: 40.,
            },
            30.,
            [0.; 2],
            FloatDirection::Right,
            Clear::None,
        );
        assert_eq!(
            intersecting_bottom(
                &context,
                200.,
                CssRect {
                    x: 30.,
                    y: 0.,
                    width: 150.,
                    height: 50.
                }
            ),
            Some(70.)
        );
        assert_eq!(
            intersecting_bottom(
                &context,
                200.,
                CssRect {
                    x: 30.,
                    y: 0.,
                    width: 100.,
                    height: 50.
                }
            ),
            None
        );
        assert_eq!(
            intersecting_bottom(
                &context,
                200.,
                CssRect {
                    x: 0.,
                    y: 70.,
                    width: 200.,
                    height: 10.
                }
            ),
            None
        );
    }
}
