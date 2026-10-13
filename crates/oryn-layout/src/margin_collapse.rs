//! Parent/child adjoining margins retain a strut until the block position is
//! solved. The source style remains unchanged; allocation suppression records
//! which child's margin has already been absorbed by its parent.

/// Inputs for used child vertical end.
pub(crate) struct VerticalChildEndInputs<'a> {
    pub(crate) id: u64,
    pub(crate) reverse: bool,
    pub(crate) basis: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) geometry: &'a BTreeMap<u64, NodeGeometry>,
    pub(crate) clearance: &'a BTreeSet<u64>,
}
/// Inputs for vertical edge.
pub(crate) struct VerticalMarginEdgeInputs<'a> {
    pub(crate) id: u64,
    pub(crate) after: bool,
    pub(crate) reverse: bool,
    pub(crate) basis: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) obstacles: [Option<f64>; 2],
}
/// Inputs for child.
pub(crate) struct VerticalChildInputs<'a> {
    pub(crate) id: u64,
    pub(crate) after: bool,
    pub(crate) reverse: bool,
    pub(crate) basis: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) _obstacles: [Option<f64>; 2],
}
/// Inputs for vertical child edge.
pub(crate) struct VerticalChildMarginInputs<'a> {
    pub(crate) id: u64,
    pub(crate) after: bool,
    pub(crate) reverse: bool,
    pub(crate) basis: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) clear_obstacles: [Option<f64>; 2],
    pub(crate) own_margin: f64,
}
/// Inputs for child edge.
pub(crate) struct ChildMarginEdgeInputs<'a> {
    pub(crate) id: u64,
    pub(crate) bottom: bool,
    pub(crate) width: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) clear_obstacles: [Option<f64>; 2],
    pub(crate) own_margin: f64,
    pub(crate) cache: Option<&'a std::cell::RefCell<SuffixCache>>,
}
use super::*;
use inline_formatting::{FlowItem, MarginStrut};

#[cfg(test)]
mod suffix_regressions {
    use super::*;
    // Private test-module additions for margin_collapse.rs. Base df73b463 plus
    // coordinator dirty inputs; owner /root/range. Unrun; not production source.
    fn margin_test_styles() -> ComputedStyles {
        let style = |top: &str, width: &str| {
            Arc::new(BTreeMap::from([
                ("display".into(), "block".into()),
                ("margin-top".into(), top.into()),
                ("margin-bottom".into(), top.into()),
                ("width".into(), width.into()),
            ]))
        };
        BTreeMap::from([
            (1, style("0px", "auto")),
            (2, style("10%", "50%")),
            (3, style("30%", "auto")),
        ])
    }

    fn margin_test_tree() -> Vec<LayoutNode> {
        let mut parent = crate::tests::node(1, Some(99), "div", "");
        let mut child = crate::tests::node(2, Some(1), "div", "");
        let leaf = crate::tests::node(3, Some(2), "div", "");
        parent.children.push(2);
        child.children.push(3);
        vec![parent, child, leaf]
    }

    fn margin_test_query(
        nodes: &[LayoutNode],
        styles: &ComputedStyles,
        width: f64,
        bottom: bool,
        obstacles: [Option<f64>; 2],
        own: f64,
    ) -> Option<(u64, f64)> {
        margin_test_cached_query(nodes, styles, width, bottom, obstacles, own, None)
    }

    fn margin_test_cached_query(
        nodes: &[LayoutNode],
        styles: &ComputedStyles,
        width: f64,
        bottom: bool,
        obstacles: [Option<f64>; 2],
        own: f64,
        cache: Option<&std::cell::RefCell<SuffixCache>>,
    ) -> Option<(u64, f64)> {
        let by_id = nodes.iter().map(|node| (node.id, node)).collect();
        child_edge(ChildMarginEdgeInputs {
            id: 1,
            bottom,
            width,
            nodes: &by_id,
            styles,
            viewport: ViewportState::default(),
            clear_obstacles: obstacles,
            own_margin: own,
            cache,
        })
        .map(|(id, strut)| (id, strut.value()))
    }

    #[test]
    fn margin_suffix_percentage_basis_and_style_trials_remain_distinct() {
        let nodes = margin_test_tree();
        let styles = margin_test_styles();
        let cache = std::cell::RefCell::new(SuffixCache::new());
        // Child 2 contributes 10% of incoming width; child 3 contributes 30%
        // of child 2's 50% width. Thus deepest margin wins at 15% of input.
        for bottom in [false, true] {
            assert_eq!(
                margin_test_query(&nodes, &styles, 100., bottom, [None; 2], 0.),
                Some((2, 15.))
            );
            assert_eq!(
                margin_test_query(&nodes, &styles, 200., bottom, [None; 2], 0.),
                Some((2, 30.))
            );
            assert_eq!(
                margin_test_query(&nodes, &styles, 100., bottom, [None; 2], -80.),
                Some((2, 15.))
            );
            for width in [100., 200., 100.] {
                assert_eq!(
                    margin_test_cached_query(
                        &nodes,
                        &styles,
                        width,
                        bottom,
                        [None; 2],
                        0.,
                        Some(&cache)
                    ),
                    margin_test_query(&nodes, &styles, width, bottom, [None; 2], 0.)
                );
            }
        }
        let mut trial = styles.clone();
        Arc::make_mut(trial.get_mut(&3).unwrap()).insert("margin-top".into(), "80px".into());
        // Mirrors a new driver trial scope rather than reusing another style map's cache.
        let trial_cache = std::cell::RefCell::new(SuffixCache::new());
        assert_eq!(
            margin_test_cached_query(
                &nodes,
                &trial,
                100.,
                false,
                [None; 2],
                0.,
                Some(&trial_cache)
            ),
            Some((2, 80.))
        );
        assert_eq!(
            margin_test_query(&nodes, &trial, 100., false, [None; 2], 0.),
            Some((2, 80.))
        );
        assert_eq!(
            margin_test_query(&nodes, &styles, 100., false, [None; 2], 0.),
            Some((2, 15.))
        );
    }

    #[test]
    fn margin_suffix_float_requests_preserve_complete_chain_clearance() {
        let nodes = margin_test_tree();
        let mut styles = margin_test_styles();
        let child = Arc::make_mut(styles.get_mut(&2).unwrap());
        child.insert("clear".into(), "left".into());
        child.insert("margin-top".into(), "20px".into());
        Arc::make_mut(styles.get_mut(&3).unwrap()).insert("margin-top".into(), "100px".into());
        let cache = std::cell::RefCell::new(SuffixCache::new());
        for obstacles in [[Some(80.), None], [Some(120.), None]] {
            assert_eq!(
                margin_test_cached_query(
                    &nodes,
                    &styles,
                    100.,
                    false,
                    obstacles,
                    10.,
                    Some(&cache)
                ),
                margin_test_query(&nodes, &styles, 100., false, obstacles, 10.)
            );
            assert!(
                cache.borrow().is_empty(),
                "float clearance must bypass suffix publication"
            );
        }
        assert_eq!(
            margin_test_query(&nodes, &styles, 100., false, [Some(80.), None], 10.),
            Some((2, 100.))
        );
        assert_eq!(
            margin_test_query(&nodes, &styles, 100., false, [Some(120.), None], 10.),
            None
        );
        assert_eq!(
            margin_test_query(&nodes, &styles, 100., false, [None; 2], 10.),
            Some((2, 100.))
        );
    }

    // Cache-scope adaptation needed by coordinator: execute both query tests inside
    // one cache scope; additionally re-enter a different tree/style scope retaining
    // ids and replacing child 2's children with an empty list (expected 10 at width
    // 100). A cycle fixture must compare cached fallback with original guarded walk,
    // not assume arbitrary cyclic graphs are valid layout inputs. Public collect()
    // may itself recurse on a cyclic graph; test the cache builder's cycle handling
    // with its direct next-edge seam rather than creating an infinite collect graph.
}

// Owned by one layout driver invocation, whose nodes/styles are immutable.
// Reset on each driver invocation, including distinct synthetic tree trials.
type SuffixKey = (u64, u64, bool);
pub(super) type SuffixCache = BTreeMap<SuffixKey, MarginStrut>;
fn suffix_key(id: u64, basis: f64, bottom: bool) -> SuffixKey {
    (id, basis.to_bits(), bottom)
}
#[cfg(test)]
#[derive(Default)]
struct ChainProfile {
    calls: u64,
    visits: u64,
    elapsed_ns: u128,
}
#[cfg(test)]
thread_local! {
    static CHAIN_PROFILE: std::cell::RefCell<ChainProfile> =
        std::cell::RefCell::new(ChainProfile::default());
}
#[cfg(test)]
struct ChainTimer(Option<std::time::Instant>);
#[cfg(test)]
impl ChainTimer {
    fn start() -> Self {
        let enabled = std::env::var_os("ORYN_DEEP_INPUT_DIRECTORY").is_some();
        if enabled {
            CHAIN_PROFILE.with(|profile| profile.borrow_mut().calls += 1);
        }
        Self(enabled.then(std::time::Instant::now))
    }
}
#[cfg(test)]
impl Drop for ChainTimer {
    fn drop(&mut self) {
        if let Some(started) = self.0 {
            CHAIN_PROFILE.with(|profile| {
                profile.borrow_mut().elapsed_ns += started.elapsed().as_nanos();
            });
        }
    }
}
#[cfg(test)]
pub(super) fn reset_chain_profile() {
    CHAIN_PROFILE.with(|profile| *profile.borrow_mut() = ChainProfile::default());
}
#[cfg(test)]
pub(super) fn report_chain_profile() {
    if std::env::var_os("ORYN_DEEP_INPUT_DIRECTORY").is_some() {
        CHAIN_PROFILE.with(|profile| {
            let profile = profile.borrow();
            eprintln!(
                "deep_margin_chain calls={} visits={} elapsed_ns={}",
                profile.calls, profile.visits, profile.elapsed_ns
            );
        });
    }
}

pub(super) fn clear_obstacles(
    context: Option<&taffy::compute::FloatContext>,
    origin: f64,
) -> [Option<f64>; 2] {
    let Some(context) = context else {
        return [None; 2];
    };
    [context.left_floats(), context.right_floats()].map(|floats| {
        floats
            .iter()
            .map(|value| f64::from(value.y + value.height) - origin)
            .reduce(f64::max)
    })
}

fn resolve_clearance(
    chain: Vec<(u64, f64)>,
    styles: &ComputedStyles,
    obstacles: [Option<f64>; 2],
    own: f64,
    trailing: bool,
) -> Option<(u64, MarginStrut)> {
    // CSS clearance first computes the hypothetical position with adjoining
    // margins collapsed. A clear declaration alone is not a collapse barrier.
    let hypothetical = if trailing {
        0.
    } else {
        chain
            .iter()
            .fold(MarginStrut::default().append(own), |strut, (_, margin)| {
                strut.append(*margin)
            })
            .value()
    };
    let interferes =
        |side: usize| obstacles[side].is_some_and(|bottom| bottom > hypothetical + 0.0001);
    let mut first = None;
    let mut strut = MarginStrut::default();
    for (id, margin) in chain {
        let style = styles.get(&id)?;
        let cleared = match style_value(style, "clear", "none") {
            "left" => interferes(0),
            "right" => interferes(1),
            "both" => interferes(0) || interferes(1),
            _ => false,
        };
        if cleared {
            break;
        }
        first.get_or_insert(id);
        strut = strut.append(margin);
    }
    first.map(|first| (first, strut))
}

fn content_inline(style: &BTreeMap<String, String>, basis: f64, viewport: ViewportState) -> f64 {
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let margin = edge(style, "margin", basis, font, viewport);
    let padding = edge(style, "padding", basis, font, viewport);
    let border = border_edge(style, basis, font, viewport);
    let edges = padding.left + padding.right + border.left + border.right;
    let resolve = |property, default| {
        optional_length(style_value(style, property, default), basis, font, viewport).map(|width| {
            if style_value(style, "box-sizing", "content-box") == "border-box" {
                (width - edges).max(0.)
            } else {
                width.max(0.)
            }
        })
    };
    let mut width =
        resolve("width", "auto").unwrap_or((basis - margin.left - margin.right - edges).max(0.));
    if let Some(max) = resolve("max-width", "none") {
        width = width.min(max);
    }
    if let Some(min) = resolve("min-width", "auto") {
        width = width.max(min);
    }
    width
}

pub(super) fn child_edge(params: ChildMarginEdgeInputs<'_>) -> Option<(u64, MarginStrut)> {
    #[cfg(test)]
    let _timer = ChainTimer::start();
    let ChildMarginEdgeInputs {
        id,
        bottom,
        width,
        nodes,
        styles,
        viewport,
        clear_obstacles,
        own_margin,
        cache,
    } = params;

    fn edge_child(
        id: u64,
        bottom: bool,
        width: f64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        viewport: ViewportState,
        _clear_obstacles: [Option<f64>; 2],
    ) -> Option<u64> {
        let node = nodes.get(&id)?;
        let style = styles.get(&id)?;
        if node.parent.is_none()
            || inline_formatting::establishes_bfc(style)
            || style_value(style, "writing-mode", "horizontal-tb") != "horizontal-tb"
            || style_value(style, "column-count", "auto") != "auto"
            || style_value(style, "column-width", "auto") != "auto"
        {
            return None;
        }
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let border = border_edge(style, width, font, viewport);
        let padding = edge(style, "padding", width, font, viewport);
        if if bottom {
            border.bottom + padding.bottom != 0.
        } else {
            border.top + padding.top != 0.
        } {
            return None;
        }
        if bottom
            && (style_value(style, "height", "auto") != "auto"
                || optional_length(style_value(style, "min-height", "0px"), 0., font, viewport)
                    .unwrap_or(0.)
                    > 0.)
        {
            return None;
        }
        let mut flow = inline_formatting::collect(node, nodes, styles);
        if bottom {
            flow.reverse();
        }
        for item in flow {
            match item {
                FlowItem::Block(child) => {
                    let style = styles.get(&child)?;
                    if style_value(style, "writing-mode", "horizontal-tb") != "horizontal-tb" {
                        return None;
                    }
                    return Some(child);
                }
                FlowItem::Paragraph(paragraph) => {
                    if inline_formatting::paragraph_forms_line(
                        &paragraph, styles, style, width, viewport,
                    ) {
                        return None;
                    }
                }
            }
        }
        None
    }
    let first = edge_child(id, bottom, width, nodes, styles, viewport, clear_obstacles)?;
    let mut current = first;
    let mut basis = width;
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    // Without float obstacles, clear cannot truncate the adjoining chain and
    // its summary is independent of the external parent's own margin.
    let mut cache = cache.filter(|_| clear_obstacles == [None; 2] && width.is_finite());
    let mut pending = Vec::new();
    while seen.insert(current) {
        if !basis.is_finite() {
            cache = None;
        }
        let key = suffix_key(current, basis, bottom);
        let cached = cache.and_then(|cache| cache.borrow().get(&key).copied());
        if let Some(mut suffix) = cached
            && let Some(cache) = cache
        {
            let mut cache = cache.borrow_mut();
            for (key, margin) in pending.into_iter().rev() {
                suffix = suffix.append(margin);
                cache.insert(key, suffix);
            }
            return Some((first, suffix));
        }
        #[cfg(test)]
        if _timer.0.is_some() {
            CHAIN_PROFILE.with(|profile| {
                profile.borrow_mut().visits += 1;
            });
        }
        let style = styles.get(&current)?;
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let margin = edge(style, "margin", basis, font, viewport);
        let margin = if bottom { margin.bottom } else { margin.top };
        chain.push((current, margin));
        if cache.is_some() {
            pending.push((key, margin));
        }
        let next_basis = content_inline(style, basis, viewport);
        let Some(next) = edge_child(
            current,
            bottom,
            next_basis,
            nodes,
            styles,
            viewport,
            clear_obstacles,
        ) else {
            if let Some(cache) = cache {
                let mut suffix = MarginStrut::default();
                let mut cache = cache.borrow_mut();
                for (key, margin) in pending.into_iter().rev() {
                    suffix = suffix.append(margin);
                    cache.insert(key, suffix);
                }
                return Some((first, suffix));
            }
            break;
        };
        basis = next_basis;
        current = next;
    }
    resolve_clearance(chain, styles, clear_obstacles, own_margin, bottom)
}

/// Resolve the adjoining outer edge before sibling placement, including margins
/// propagated through a first/last descendant chain.
/// Immutable tree/style scope and its invocation-owned suffix memo.
pub(super) struct QueryContext<'a> {
    pub(super) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(super) styles: &'a ComputedStyles,
    pub(super) viewport: ViewportState,
    pub(super) cache: Option<&'a std::cell::RefCell<SuffixCache>>,
}

pub(super) fn outer_edge(
    id: u64,
    bottom: bool,
    basis: f64,
    clear_obstacles: [Option<f64>; 2],
    context: QueryContext<'_>,
) -> MarginStrut {
    let QueryContext {
        nodes,
        styles,
        viewport,
        cache,
    } = context;
    let Some(style) = styles.get(&id) else {
        return MarginStrut::default();
    };
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let margin = edge(style, "margin", basis, font, viewport);
    let width = content_inline(style, basis, viewport);
    child_edge(ChildMarginEdgeInputs {
        id,
        bottom,
        width,
        nodes,
        styles,
        viewport,
        clear_obstacles,
        own_margin: if bottom { margin.bottom } else { margin.top },
        cache,
    })
    .map(|(_, strut)| strut)
    .unwrap_or_default()
    .append(if bottom { margin.bottom } else { margin.top })
}

fn vertical_content_inline(
    style: &BTreeMap<String, String>,
    basis: f64,
    viewport: ViewportState,
) -> f64 {
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let margin = edge(style, "margin", basis, font, viewport);
    let padding = edge(style, "padding", basis, font, viewport);
    let border = border_edge(style, basis, font, viewport);
    let edges = padding.top + padding.bottom + border.top + border.bottom;
    let resolve = |property, default| {
        optional_length(style_value(style, property, default), basis, font, viewport).map(|value| {
            (value
                - if style_value(style, "box-sizing", "content-box") == "border-box" {
                    edges
                } else {
                    0.
                })
            .max(0.)
        })
    };
    let mut inline =
        resolve("height", "auto").unwrap_or((basis - margin.top - margin.bottom - edges).max(0.));
    if let Some(max) = resolve("max-height", "none") {
        inline = inline.min(max);
    }
    if let Some(min) = resolve("min-height", "auto") {
        inline = inline.max(min);
    }
    inline
}

pub(super) fn vertical_child_edge(
    params: VerticalChildMarginInputs<'_>,
) -> Option<(u64, MarginStrut)> {
    let VerticalChildMarginInputs {
        id,
        after,
        reverse,
        basis,
        nodes,
        styles,
        viewport,
        clear_obstacles,
        own_margin,
    } = params;

    fn child(params: VerticalChildInputs<'_>) -> Option<u64> {
        let VerticalChildInputs {
            id,
            after,
            reverse,
            basis,
            nodes,
            styles,
            viewport,
            _obstacles,
        } = params;

        let node = nodes.get(&id)?;
        let style = styles.get(&id)?;
        let mode = style_value(style, "writing-mode", "horizontal-tb");
        let axes = writing_mode::Axes::new(mode, false);
        if node.parent.is_none()
            || !axes.vertical
            || axes.x_reverse != reverse
            || inline_formatting::establishes_bfc(style)
            || style_value(style, "column-count", "auto") != "auto"
            || style_value(style, "column-width", "auto") != "auto"
        {
            return None;
        }
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let padding = edge(style, "padding", basis, font, viewport);
        let border = border_edge(style, basis, font, viewport);
        if if after == reverse {
            padding.left + border.left != 0.
        } else {
            padding.right + border.right != 0.
        } {
            return None;
        }
        if after
            && (style_value(style, "width", "auto") != "auto"
                || optional_length(
                    style_value(style, "min-width", "0px"),
                    basis,
                    font,
                    viewport,
                )
                .unwrap_or(0.)
                    > 0.)
        {
            return None;
        }
        let mut flow = inline_formatting::collect(node, nodes, styles);
        if after {
            flow.reverse();
        }
        for item in flow {
            match item {
                FlowItem::Block(id) => {
                    let child = styles.get(&id)?;
                    if style_value(child, "writing-mode", "horizontal-tb") != mode {
                        return None;
                    }
                    return Some(id);
                }
                FlowItem::Paragraph(paragraph) => {
                    if inline_formatting::paragraph_forms_line(
                        &paragraph, styles, style, basis, viewport,
                    ) {
                        return None;
                    }
                }
            }
        }
        None
    }
    let first = child(VerticalChildInputs {
        id,
        after,
        reverse,
        basis,
        nodes,
        styles,
        viewport,
        _obstacles: clear_obstacles,
    })?;
    let mut current = first;
    let mut basis = basis;
    let mut seen = BTreeSet::new();
    let mut chain = Vec::new();
    while seen.insert(current) {
        let style = styles.get(&current)?;
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let margin = edge(style, "margin", basis, font, viewport);
        chain.push((
            current,
            if after == reverse {
                margin.left
            } else {
                margin.right
            },
        ));
        let next_basis = vertical_content_inline(style, basis, viewport);
        let Some(next) = child(VerticalChildInputs {
            id: current,
            after,
            reverse,
            basis: next_basis,
            nodes,
            styles,
            viewport,
            _obstacles: clear_obstacles,
        }) else {
            break;
        };
        basis = next_basis;
        current = next;
    }
    resolve_clearance(chain, styles, clear_obstacles, own_margin, after)
}

pub(super) fn vertical_edge(params: VerticalMarginEdgeInputs<'_>) -> MarginStrut {
    let VerticalMarginEdgeInputs {
        id,
        after,
        reverse,
        basis,
        nodes,
        styles,
        viewport,
        obstacles,
    } = params;

    let Some(style) = styles.get(&id) else {
        return MarginStrut::default();
    };
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let margin = edge(style, "margin", basis, font, viewport);
    vertical_child_edge(VerticalChildMarginInputs {
        id,
        after,
        reverse,
        basis: vertical_content_inline(style, basis, viewport),
        nodes,
        styles,
        viewport,
        clear_obstacles: obstacles,
        own_margin: if after == reverse {
            margin.left
        } else {
            margin.right
        },
    })
    .map(|(_, strut)| strut)
    .unwrap_or_default()
    .append(if after == reverse {
        margin.left
    } else {
        margin.right
    })
}

/// Resolve the last child's adjoining bottom from used native geometry. Clear
/// only blocks propagation when it actually created clearance on a through box.
pub(super) fn used_child_bottom(
    id: u64,
    width: f64,
    geometry: &BTreeMap<u64, NodeGeometry>,
    clearance: &BTreeSet<u64>,
    context: QueryContext<'_>,
) -> Option<(u64, f64)> {
    let QueryContext {
        nodes,
        styles,
        viewport,
        cache,
    } = context;
    let (child, _) = child_edge(ChildMarginEdgeInputs {
        id,
        bottom: true,
        width,
        nodes,
        styles,
        viewport,
        clear_obstacles: [None; 2],
        own_margin: 0.,
        cache,
    })?;
    let model = geometry.get(&child)?.box_model.as_ref()?;
    if clearance.contains(&child)
        && model.border_box.height == 0.
        && model.border.top == 0.
        && model.border.bottom == 0.
        && model.padding.top == 0.
        && model.padding.bottom == 0.
    {
        return None;
    }
    Some((child, model.margin.bottom))
}

pub(super) fn used_child_vertical_end(params: VerticalChildEndInputs<'_>) -> Option<(u64, f64)> {
    let VerticalChildEndInputs {
        id,
        reverse,
        basis,
        nodes,
        styles,
        viewport,
        geometry,
        clearance,
    } = params;

    let (child, _) = vertical_child_edge(VerticalChildMarginInputs {
        id,
        after: true,
        reverse,
        basis,
        nodes,
        styles,
        viewport,
        clear_obstacles: [None; 2],
        own_margin: 0.,
    })?;
    let model = geometry.get(&child)?.box_model.as_ref()?;
    if clearance.contains(&child)
        && model.border_box.width == 0.
        && model.border.left == 0.
        && model.border.right == 0.
        && model.padding.left == 0.
        && model.padding.right == 0.
    {
        return None;
    }
    Some((
        child,
        if reverse {
            model.margin.left
        } else {
            model.margin.right
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hypothetical_clearance_uses_complete_strut_and_signed_float_threshold() {
        let styles = BTreeMap::from([
            (
                2,
                Arc::new(BTreeMap::from([("clear".into(), "left".into())])),
            ),
            (3, Arc::new(BTreeMap::new())),
        ]);
        let positive = resolve_clearance(
            vec![(2, 20.), (3, 100.)],
            &styles,
            [Some(80.), None],
            10.,
            false,
        )
        .unwrap();
        assert_eq!((positive.0, positive.1.value()), (2, 100.));
        assert!(
            resolve_clearance(vec![(2, -50.)], &styles, [Some(-10.), None], 20., false).is_none()
        );
        assert_eq!(
            resolve_clearance(vec![(2, -50.)], &styles, [None, None], 20., false)
                .unwrap()
                .1
                .value(),
            -50.
        );
        assert!(
            resolve_clearance(vec![(2, 20.)], &styles, [None, Some(100.)], 0., false).is_some()
        );
    }
}
