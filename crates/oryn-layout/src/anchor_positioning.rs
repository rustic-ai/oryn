//! Used anchor geometry is resolved after native cascade and before formatting
//! the positioned box. Author CSS never supplies private node identities.

/// Inputs for picker cascade with native.
pub(crate) struct NativePickerCascadeInputs<'a> {
    pub(crate) document: &'a blitz_dom::BaseDocument,
    pub(crate) input: &'a LayoutInput,
    pub(crate) mirrors: &'a BTreeMap<u64, blitz_traits::node_id::NodeId>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) top_layer: bool,
    pub(crate) owner: u64,
    pub(crate) computed: &'a style::properties::ComputedValues,
    pub(crate) values: &'a mut BTreeMap<String, String>,
    pub(crate) native: NativeValues<'a>,
}
/// Inputs for picker cascade.
pub(crate) struct PickerCascadeInputs<'a> {
    pub(crate) document: &'a blitz_dom::BaseDocument,
    pub(crate) input: &'a LayoutInput,
    pub(crate) mirrors: &'a BTreeMap<u64, blitz_traits::node_id::NodeId>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) top_layer: bool,
    pub(crate) owner: u64,
    pub(crate) computed: &'a style::properties::ComputedValues,
    pub(crate) values: &'a mut BTreeMap<String, String>,
}
use super::*;
use cssparser::{Parser, ParserInput, Token};

/// Native use-clone scope, independent of the source selector tree.
pub(super) const INSTANCE_CONTEXT: &str = "-oryn-anchor-instance-context";

pub(super) fn needed(style: &BTreeMap<String, String>) -> bool {
    style
        .values()
        .any(|value| value.contains("anchor(") || value.contains("anchor-size("))
        || style_value(style, "position-area", "none") != "none"
        || style_value(style, "position-visibility", "always") != "always"
        || style_value(style, "position-try-fallbacks", "none") != "none"
        || ["align-self", "justify-self"]
            .iter()
            .any(|key| style_value(style, key, "normal").contains("anchor-center"))
}
fn names(value: &str) -> Vec<String> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut names = Vec::new();
    while let Ok(token) = parser.next() {
        if let Token::Ident(name) = token {
            names.push(name.to_string());
        }
    }
    names
}
struct Context<'a> {
    id: u64,
    input: &'a LayoutInput,
    styles: &'a ComputedStyles,
    nodes: BTreeMap<u64, &'a LayoutNode>,
    projections: &'a BTreeMap<u64, oryn_common::v4::PaintProjection>,
    cb: CssRect,
    cb_owner: Option<u64>,
    dependency_only: bool,
    references: std::cell::RefCell<BTreeSet<u64>>,
}
impl Context<'_> {
    fn parent(&self, id: u64) -> Option<u64> {
        if let Some(value) = self
            .styles
            .get(&id)
            .and_then(|style| style.get("-oryn-anchor-parent"))
        {
            return value.parse().ok();
        }
        self.nodes.get(&id).and_then(|node| node.parent)
    }
    fn scope(&self, id: u64, name: &str, tree: Option<&String>) -> Option<u64> {
        let mut current = Some(id);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                break;
            }
            if self.styles.get(&id).is_some_and(|style| {
                tree.is_none_or(|tree| {
                    style
                        .get("-oryn-anchor-tree:anchor-scope:")
                        .is_none_or(|scope| scope == tree)
                }) && names(style_value(style, "anchor-scope", "none"))
                    .iter()
                    .any(|value| value == "all" || value == name)
            }) {
                return Some(id);
            }
            current = self.parent(id);
        }
        None
    }
    fn eligible(&self, id: u64) -> bool {
        if id == self.id || !self.nodes.contains_key(&id) {
            return false;
        }
        let Some(style) = self.styles.get(&id) else {
            return false;
        };
        if style_value(style, "display", "block") == "none" {
            return false;
        }
        if self
            .styles
            .get(&self.id)
            .and_then(|style| style.get(INSTANCE_CONTEXT))
            != style.get(INSTANCE_CONTEXT)
        {
            return false;
        }
        let mut parent = self.parent(id);
        let mut seen = BTreeSet::new();
        while let Some(id) = parent {
            if id == self.id || !seen.insert(id) {
                return false;
            }
            parent = self.parent(id);
        }
        true
    }
    fn anchor(&self, explicit: Option<&str>, property: &str) -> Option<u64> {
        let style = self.styles.get(&self.id)?;
        let requested = explicit.map(str::to_owned).or_else(|| {
            names(style_value(style, "position-anchor", "normal"))
                .into_iter()
                .find(|name| name.starts_with("--"))
        });
        if let Some(name) = requested {
            let scope_key = if explicit.is_some() {
                format!("-oryn-anchor-tree:{property}:{name}")
            } else {
                "-oryn-anchor-tree:position-anchor:".into()
            };
            let query_tree = style.get(&scope_key);
            let query_scope = self.input.shadow_trees.node_scopes.get(&self.id);
            let mut selected = None;
            let query_index = self
                .input
                .nodes
                .iter()
                .position(|node| node.id == self.id)
                .unwrap_or(usize::MAX);
            for (index, node) in self.input.nodes.iter().enumerate() {
                // Out-of-flow anchors must precede their querying box. Normal
                // flow anchors have completed layout before positioned layout.
                if !self.eligible(node.id)
                    || self.scope(node.id, &name, query_tree)
                        != self.scope(self.id, &name, query_tree)
                {
                    continue;
                }
                let candidate_tree = self
                    .styles
                    .get(&node.id)
                    .and_then(|style| style.get("-oryn-anchor-tree:anchor-name:"));
                if if let (Some(query), Some(candidate)) = (query_tree, candidate_tree) {
                    query != candidate
                } else {
                    self.input.shadow_trees.node_scopes.get(&node.id) != query_scope
                } {
                    continue;
                }
                let Some(style) = self.styles.get(&node.id) else {
                    continue;
                };
                if matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                ) && index > query_index
                {
                    continue;
                }
                if !self.dependency_only && self.rect(node.id).is_none() {
                    continue;
                }
                if names(style_value(style, "anchor-name", "none")).contains(&name) {
                    selected = Some(node.id);
                }
            }
            return selected;
        }
        if style_value(style, "position-anchor", "normal") == "none" {
            return None;
        }
        let implicit = self
            .input
            .top_layer
            .iter()
            .find(|entry| entry.node_id == self.id)
            .and_then(|entry| entry.anchor_source);
        let attribute = self
            .nodes
            .get(&self.id)
            .and_then(|node| node.attributes.get("anchor"))
            .and_then(|name| {
                self.input.nodes.iter().find(|node| {
                    node.attributes.get("id") == Some(name)
                        && self.eligible(node.id)
                        && self.input.shadow_trees.node_scopes.get(&node.id)
                            == self.input.shadow_trees.node_scopes.get(&self.id)
                })
            })
            .map(|node| node.id);
        let pseudo = style
            .get("-oryn-implicit-anchor")
            .and_then(|value| value.parse().ok());
        attribute
            .or(pseudo)
            .or(implicit)
            .filter(|id| self.eligible(*id))
    }
    fn rect(&self, id: u64) -> Option<CssRect> {
        let projection = self.projections.get(&id)?;
        let rect = projection.box_model.border_box;
        let m = projection.matrix;
        let cb_projection = self.cb_owner.and_then(|owner| self.projections.get(&owner));
        let mut bounds = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (x, y) in [
            (rect.x, rect.y),
            (rect.right(), rect.y),
            (rect.x, rect.bottom()),
            (rect.right(), rect.bottom()),
        ] {
            let w = m[3] * x + m[7] * y + m[15];
            if w <= 0. || !w.is_finite() {
                return None;
            }
            let point = (
                (m[0] * x + m[4] * y + m[12]) / w,
                (m[1] * x + m[5] * y + m[13]) / w,
            );
            let (x, y) = if let Some(cb) = cb_projection {
                let local = cb.viewport_to_local(point.0, point.1)?;
                // Equal coordinate planes cancel exactly. Still validate the
                // projected point/inverse above, but avoid a lossy round trip
                // through the homography for an unchanged local coordinate.
                if cb.matrix == m { (x, y) } else { local }
            } else {
                point
            };
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
        Some(CssRect {
            x: bounds.0,
            y: bounds.1,
            width: bounds.2 - bounds.0,
            height: bounds.3 - bounds.1,
        })
    }
    fn axes(&self, id: u64) -> writing_mode::Axes {
        let style = self.styles.get(&id);
        writing_mode::Axes::new(
            style
                .map(|style| style_value(style, "writing-mode", "horizontal-tb"))
                .unwrap_or("horizontal-tb"),
            style.is_some_and(|style| style_value(style, "direction", "ltr") == "rtl"),
        )
    }
    fn containing_axes(&self) -> writing_mode::Axes {
        self.cb_owner
            .or_else(|| {
                self.input
                    .nodes
                    .iter()
                    .find(|node| node.parent.is_none())
                    .map(|node| node.id)
            })
            .map(|id| self.axes(id))
            .unwrap_or_else(|| writing_mode::Axes::new("horizontal-tb", false))
    }
    fn function(&self, function: &str, body: &str, property: &str, depth: usize) -> Option<String> {
        let mut input = ParserInput::new(body);
        let mut parser = Parser::new(&mut input);
        let (mut name, mut side, mut percent, mut fallback) = (None, None, None, None);
        while let Ok(token) = parser.next().cloned() {
            match token {
                Token::Ident(value) if value.starts_with("--") => {
                    if name.replace(value.to_string()).is_some() {
                        return None;
                    }
                }
                Token::Ident(value) => {
                    if side.replace(value.to_ascii_lowercase()).is_some() {
                        return None;
                    }
                }
                Token::Percentage { unit_value, .. } => {
                    if percent.replace(f64::from(unit_value)).is_some() {
                        return None;
                    }
                }
                Token::Comma => {
                    let start = parser.position();
                    while parser.next_including_whitespace_and_comments().is_ok() {}
                    fallback = Some(parser.slice_from(start).to_owned());
                    break;
                }
                _ => return None,
            }
        }
        let value = (|| {
            let anchor = self.anchor(name.as_deref(), property)?;
            self.references.borrow_mut().insert(anchor);
            let rect = if self.dependency_only {
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 0.,
                    height: 0.,
                }
            } else {
                self.rect(anchor)?
            };
            let horizontal = property.ends_with("left")
                || property.ends_with("right")
                || property.ends_with("width");
            if function.eq_ignore_ascii_case("anchor-size") {
                let axes = self.axes(anchor);
                let own = self.axes(self.id);
                return Some(
                    match side
                        .as_deref()
                        .unwrap_or(if horizontal { "width" } else { "height" })
                    {
                        "width" => rect.width,
                        "height" => rect.height,
                        "inline" => {
                            if axes.vertical {
                                rect.height
                            } else {
                                rect.width
                            }
                        }
                        "block" => {
                            if axes.vertical {
                                rect.width
                            } else {
                                rect.height
                            }
                        }
                        "self-inline" => {
                            if own.vertical {
                                rect.height
                            } else {
                                rect.width
                            }
                        }
                        "self-block" => {
                            if own.vertical {
                                rect.width
                            } else {
                                rect.height
                            }
                        }
                        _ => return None,
                    },
                );
            }
            if !matches!(property, "top" | "right" | "bottom" | "left") {
                return None;
            }
            let own = self.axes(self.id);
            let cb = self.containing_axes();
            let side = side.as_deref().unwrap_or("start");
            let reverse = if side.starts_with("self-") {
                if horizontal {
                    own.x_reverse
                } else {
                    own.y_reverse
                }
            } else if horizontal {
                cb.x_reverse
            } else {
                cb.y_reverse
            };
            let fraction = match side {
                "left" if horizontal => 0.,
                "right" if horizontal => 1.,
                "top" if !horizontal => 0.,
                "bottom" if !horizontal => 1.,
                "center" => 0.5,
                "start" | "self-start" => {
                    if reverse {
                        1. - percent.unwrap_or(0.)
                    } else {
                        percent.unwrap_or(0.)
                    }
                }
                "end" | "self-end" => {
                    if reverse {
                        0.
                    } else {
                        1.
                    }
                }
                "inside" => {
                    if matches!(property, "right" | "bottom") {
                        1.
                    } else {
                        0.
                    }
                }
                "outside" => {
                    if matches!(property, "right" | "bottom") {
                        0.
                    } else {
                        1.
                    }
                }
                _ => return None,
            };
            let coordinate = if horizontal {
                rect.x + rect.width * fraction
            } else {
                rect.y + rect.height * fraction
            };
            Some(match property {
                "left" => coordinate - self.cb.x,
                "right" => self.cb.right() - coordinate,
                "top" => coordinate - self.cb.y,
                "bottom" => self.cb.bottom() - coordinate,
                _ => return None,
            })
        })();
        if let Some(value) = value.filter(|value| value.is_finite()) {
            Some(format!("{value}px"))
        } else {
            fallback.and_then(|fallback| self.substitute(&fallback, property, depth + 1))
        }
    }
    fn substitute(&self, value: &str, property: &str, depth: usize) -> Option<String> {
        if depth > 32 {
            return None;
        }
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let mut output = String::new();
        loop {
            let start = parser.position();
            let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
                break;
            };
            if let Token::Function(name) = token {
                let body = parser
                    .parse_nested_block(|inner| {
                        let start = inner.position();
                        while inner.next_including_whitespace_and_comments().is_ok() {}
                        Ok::<_, cssparser::ParseError<'_, ()>>(inner.slice_from(start).to_owned())
                    })
                    .ok()?;
                if name.eq_ignore_ascii_case("anchor") || name.eq_ignore_ascii_case("anchor-size") {
                    output.push_str(&self.function(&name, &body, property, depth + 1)?);
                } else {
                    output.push_str(&name);
                    output.push('(');
                    output.push_str(&self.substitute(&body, property, depth + 1)?);
                    output.push(')');
                }
            } else {
                output.push_str(parser.slice_from(start));
            }
            if output.len() > 1024 * 1024 {
                return None;
            }
        }
        Some(output)
    }
}

pub(super) fn resolve(
    id: u64,
    input: &LayoutInput,
    styles: &ComputedStyles,
    projections: &BTreeMap<u64, oryn_common::v4::PaintProjection>,
    cb: CssRect,
) -> ComputedStyle {
    let mut result = styles[&id].as_ref().clone();
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let fixed = style_value(&result, "position", "static") == "fixed";
    let mut parent = if input.top_layer.iter().any(|entry| entry.node_id == id) {
        None
    } else {
        nodes[&id].parent
    };
    let mut cb_owner = None;
    while let Some(id) = parent {
        if styles
            .get(&id)
            .is_some_and(|style| positioning::establishes(style, fixed))
        {
            cb_owner = Some(id);
            break;
        }
        parent = nodes.get(&id).and_then(|node| node.parent);
    }
    let cb = cb_owner
        .and_then(|owner| projections.get(&owner))
        .map(|projection| projection.box_model.padding_box)
        .unwrap_or(cb);
    let context = Context {
        id,
        input,
        styles,
        nodes,
        projections,
        cb,
        cb_owner,
        dependency_only: false,
        references: Default::default(),
    };
    for (property, value) in styles[&id].iter() {
        if value.contains("anchor(") || value.contains("anchor-size(") {
            let resolved = context.substitute(value, property, 0).filter(|value| {
                value.trim() == "auto"
                    || oryn_common::css_math::resolve_length(
                        value,
                        oryn_common::css_math::LengthContext {
                            percentage_basis: if property.ends_with("width")
                                || property.ends_with("left")
                                || property.ends_with("right")
                            {
                                cb.width
                            } else {
                                cb.height
                            },
                            font_size: length(
                                style_value(&result, "font-size", "16px"),
                                16.,
                                16.,
                                input.viewport,
                            ),
                            root_font_size: 16.,
                            viewport_width: input.viewport.width,
                            viewport_height: input.viewport.height,
                        },
                    )
                    .is_some()
            });
            result.insert(
                property.clone(),
                resolved.unwrap_or_else(|| {
                    if property.starts_with("margin-") {
                        "0px"
                    } else if property.starts_with("max-") {
                        "none"
                    } else {
                        "auto"
                    }
                    .into()
                }),
            );
        }
    }
    if let Some(anchor) = context.anchor(None, "").and_then(|id| context.rect(id)) {
        let area = style_value(
            &result,
            "-oryn-position-area",
            style_value(&result, "position-area", "none"),
        )
        .to_owned();
        let mut tracks = names(&area);
        if area != "none" {
            if tracks.len() == 1 {
                let value = tracks[0].clone();
                if matches!(
                    value.as_str(),
                    "top" | "bottom" | "span-top" | "span-bottom"
                ) {
                    tracks.insert(0, "span-all".into());
                } else {
                    tracks.push(if matches!(value.as_str(), "center" | "span-all") {
                        value
                    } else {
                        "span-all".into()
                    });
                }
            }
            if tracks.len() == 2 {
                if matches!(
                    tracks[0].as_str(),
                    "top" | "bottom" | "span-top" | "span-bottom"
                ) {
                    tracks.swap(0, 1);
                }
                for (horizontal, track) in [(true, &tracks[0]), (false, &tracks[1])] {
                    let (origin, end, start, finish) = if horizontal {
                        (cb.x, cb.right(), anchor.x, anchor.right())
                    } else {
                        (cb.y, cb.bottom(), anchor.y, anchor.bottom())
                    };
                    let (low, high, default) = match track.as_str() {
                        "left" | "top" => (origin, start, "end"),
                        "right" | "bottom" => (finish, end, "start"),
                        "span-left" | "span-top" => (origin, finish, "end"),
                        "span-right" | "span-bottom" => (start, end, "start"),
                        "center" => (start, finish, "center"),
                        _ => (origin, end, "anchor-center"),
                    };
                    let (first, last, size, key) = if horizontal {
                        ("left", "right", "width", "-oryn-anchor-x")
                    } else {
                        ("top", "bottom", "height", "-oryn-anchor-y")
                    };
                    let font = length(
                        style_value(&result, "font-size", "16px"),
                        16.,
                        16.,
                        input.viewport,
                    );
                    let basis = (high - low).max(0.);
                    let first_inset = optional_length(
                        style_value(&result, first, "auto"),
                        basis,
                        font,
                        input.viewport,
                    )
                    .unwrap_or(0.);
                    let last_inset = optional_length(
                        style_value(&result, last, "auto"),
                        basis,
                        font,
                        input.viewport,
                    )
                    .unwrap_or(0.);
                    result.insert(first.into(), format!("{}px", low - origin + first_inset));
                    result.insert(last.into(), format!("{}px", end - high + last_inset));
                    let axes = context.containing_axes();
                    let alignment = style_value(
                        &result,
                        if horizontal != axes.vertical {
                            "justify-self"
                        } else {
                            "align-self"
                        },
                        "normal",
                    )
                    .to_owned();
                    let default_alignment = matches!(alignment.as_str(), "normal" | "auto");
                    let alignment = if default_alignment {
                        default
                    } else {
                        alignment.split_whitespace().last().unwrap_or(default)
                    };
                    if alignment != "stretch" {
                        if style_value(&result, size, "auto") == "auto" {
                            result.insert(size.into(), "fit-content".into());
                        }
                        let (low, high) = (low + first_inset, high - last_inset);
                        let alignment_axes = if alignment.starts_with("self-") {
                            context.axes(id)
                        } else {
                            axes
                        };
                        let reverse = !default_alignment
                            && if horizontal {
                                alignment_axes.x_reverse
                            } else {
                                alignment_axes.y_reverse
                            };
                        let (center, fraction) = match alignment {
                            "anchor-center" => ((start + finish) * 0.5, 0.5),
                            "center" => ((low + high) * 0.5, 0.5),
                            "left" => (low, 0.),
                            "right" => (high, 1.),
                            "end" | "self-end" | "flex-end" => {
                                if reverse {
                                    (low, 0.)
                                } else {
                                    (high, 1.)
                                }
                            }
                            _ => {
                                if reverse {
                                    (high, 1.)
                                } else {
                                    (low, 0.)
                                }
                            }
                        };
                        let automatic_margin = if horizontal {
                            ["margin-left", "margin-right"]
                        } else {
                            ["margin-top", "margin-bottom"]
                        }
                        .iter()
                        .any(|name| style_value(&result, name, "0px") == "auto");
                        if !automatic_margin {
                            result.insert(key.into(), format!("{center} {fraction}"));
                        }
                    }
                }
            }
        } else {
            let axes = context.containing_axes();
            for (horizontal, key) in [(true, "-oryn-anchor-x"), (false, "-oryn-anchor-y")] {
                if style_value(
                    &result,
                    if horizontal != axes.vertical {
                        "justify-self"
                    } else {
                        "align-self"
                    },
                    "normal",
                )
                .contains("anchor-center")
                {
                    result.insert(
                        key.into(),
                        format!(
                            "{} 0.5",
                            if horizontal {
                                anchor.x + anchor.width * 0.5
                            } else {
                                anchor.y + anchor.height * 0.5
                            }
                        ),
                    );
                }
            }
        }
    }
    Arc::new(result)
}

/// Positioned ancestors of referenced anchors must finish before this box.
pub(super) fn dependencies(id: u64, input: &LayoutInput, styles: &ComputedStyles) -> BTreeSet<u64> {
    let projections = BTreeMap::new();
    let context = Context {
        id,
        input,
        styles,
        nodes: input.nodes.iter().map(|node| (node.id, node)).collect(),
        projections: &projections,
        cb: CssRect {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
        },
        cb_owner: None,
        dependency_only: true,
        references: Default::default(),
    };
    for (property, value) in styles[&id].iter() {
        if value.contains("anchor(") || value.contains("anchor-size(") {
            let _ = context.substitute(value, property, 0);
        }
    }
    if needed(&styles[&id])
        && let Some(anchor) = context.anchor(None, "")
    {
        context.references.borrow_mut().insert(anchor);
    }
    let mut dependencies = BTreeSet::new();
    for anchor in context.references.into_inner() {
        let mut current = Some(anchor);
        let mut seen = BTreeSet::new();
        while let Some(node) = current {
            if node == id || !seen.insert(node) {
                break;
            }
            if styles.get(&node).is_some_and(|style| {
                matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                )
            }) {
                dependencies.insert(node);
            }
            current = context.nodes.get(&node).and_then(|node| node.parent);
        }
    }
    if let Some(value) = styles[&id].get("-oryn-position-tries")
        && let Ok(candidates) = serde_json::from_str::<Vec<BTreeMap<String, String>>>(value)
    {
        for mut candidate in candidates {
            candidate.remove("-oryn-position-tries");
            for key in ["-oryn-anchor-parent", "-oryn-implicit-anchor"] {
                if let Some(value) = styles[&id].get(key) {
                    candidate.insert(key.into(), value.clone());
                }
            }
            let mut alternate = styles.clone();
            alternate.insert(id, Arc::new(candidate));
            dependencies.extend(self::dependencies(id, input, &alternate));
        }
    }
    dependencies
}

/// Every member of a dependency cycle loses its anchor references for this
/// formatting pass. Breaking one edge alone would make results queue-dependent.
pub(super) fn cycles(graph: &BTreeMap<u64, BTreeSet<u64>>) -> BTreeSet<u64> {
    let mut invalid = BTreeSet::new();
    for root in graph.keys() {
        let mut pending = graph[root].iter().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == *root {
                invalid.insert(*root);
                break;
            }
            if seen.insert(id)
                && let Some(edges) = graph.get(&id)
            {
                pending.extend(edges);
            }
        }
    }
    invalid
}

pub(super) fn candidates(
    id: u64,
    input: &LayoutInput,
    styles: &ComputedStyles,
    projections: &BTreeMap<u64, oryn_common::v4::PaintProjection>,
    cb: CssRect,
) -> Vec<ComputedStyle> {
    let mut result = vec![resolve(id, input, styles, projections, cb)];
    if let Some(value) = styles[&id].get("-oryn-position-tries")
        && let Ok(candidates) = serde_json::from_str::<Vec<BTreeMap<String, String>>>(value)
    {
        for mut candidate in candidates {
            for key in ["-oryn-anchor-parent", "-oryn-implicit-anchor"] {
                if let Some(value) = styles[&id].get(key) {
                    candidate.insert(key.into(), value.clone());
                }
            }
            let mut alternate = styles.clone();
            alternate.insert(id, Arc::new(candidate));
            result.push(resolve(id, input, &alternate, projections, cb));
        }
    }
    result
}

pub(super) fn containing_axes(
    id: u64,
    input: &LayoutInput,
    styles: &ComputedStyles,
) -> writing_mode::Axes {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let fixed = styles
        .get(&id)
        .is_some_and(|style| style_value(style, "position", "static") == "fixed");
    let mut parent = if input.top_layer.iter().any(|entry| entry.node_id == id) {
        None
    } else {
        nodes.get(&id).and_then(|node| node.parent)
    };
    let mut owner = None;
    let mut seen = BTreeSet::new();
    while let Some(id) = parent {
        if !seen.insert(id) {
            break;
        }
        if styles
            .get(&id)
            .is_some_and(|style| positioning::establishes(style, fixed))
        {
            owner = Some(id);
            break;
        }
        parent = nodes.get(&id).and_then(|node| node.parent);
    }
    let style = owner
        .or_else(|| {
            input
                .nodes
                .iter()
                .find(|node| node.parent.is_none())
                .map(|node| node.id)
        })
        .and_then(|id| styles.get(&id));
    writing_mode::Axes::new(
        style
            .map(|style| style_value(style, "writing-mode", "horizontal-tb"))
            .unwrap_or("horizontal-tb"),
        style.is_some_and(|style| style_value(style, "direction", "ltr") == "rtl"),
    )
}

pub(super) fn available(
    style: &BTreeMap<String, String>,
    cb: CssRect,
    input: &LayoutInput,
) -> (f64, f64) {
    let font = length(
        style_value(style, "font-size", "16px"),
        16.,
        16.,
        input.viewport,
    );
    let inset = |key, basis| {
        optional_length(style_value(style, key, "auto"), basis, font, input.viewport).unwrap_or(0.)
    };
    (
        (cb.width - inset("left", cb.width) - inset("right", cb.width)).max(0.),
        (cb.height - inset("top", cb.height) - inset("bottom", cb.height)).max(0.),
    )
}

/// Capture native scoped references and fallback rules for a picker pseudo while
/// the originating element still has its Stylo rule tree and shadow identity.
/// Optional instance-native values keep selector mirrors and inheritance
/// identities separate. A missing instance must not fall back to source style.
#[derive(Clone, Copy, Default)]
pub(super) struct NativeValues<'a>(
    pub Option<&'a BTreeMap<u64, style::servo_arc::Arc<style::properties::ComputedValues>>>,
    pub Option<&'a BTreeMap<u64, style::servo_arc::Arc<style::properties::ComputedValues>>>,
);

impl NativeValues<'_> {
    fn get(
        &self,
        document: &BaseDocument,
        mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
        id: u64,
    ) -> Option<style::servo_arc::Arc<style::properties::ComputedValues>> {
        match self.0 {
            Some(values) => values.get(&id).cloned(),
            None => mirrors
                .get(&id)
                .and_then(|mirror| document.resolved_primary_style(*mirror)),
        }
    }

    fn root(&self, input: &LayoutInput, owner: u64) -> Option<u64> {
        if self.0.is_none() {
            return input
                .nodes
                .iter()
                .find(|node| node.parent.is_none())
                .map(|node| node.id);
        }
        let mut current = owner;
        let mut seen = BTreeSet::new();
        while seen.insert(current) {
            let node = input.nodes.iter().find(|node| node.id == current)?;
            match node.parent {
                Some(parent) => current = parent,
                None => return Some(current),
            }
        }
        None
    }
}

pub(super) fn picker_cascade(params: PickerCascadeInputs<'_>) -> Result<(), LayoutError> {
    let PickerCascadeInputs {
        document,
        input,
        mirrors,
        styles,
        top_layer,
        owner,
        computed,
        values,
    } = params;

    picker_cascade_with_native(NativePickerCascadeInputs {
        document,
        input,
        mirrors,
        styles,
        top_layer,
        owner,
        computed,
        values,
        native: NativeValues::default(),
    })
}

pub(super) fn picker_cascade_with_native(
    params: NativePickerCascadeInputs<'_>,
) -> Result<(), LayoutError> {
    let NativePickerCascadeInputs {
        document,
        input,
        mirrors,
        styles,
        top_layer,
        owner,
        computed,
        values,
        native,
    } = params;

    use style_traits::ToCss;
    let Some(mirror) = mirrors.get(&owner) else {
        return Ok(());
    };
    let mut cb_wm = native
        .root(input, owner)
        .and_then(|id| native.get(document, mirrors, id))
        .map(|computed| computed.writing_mode)
        .unwrap_or_else(style::logical_geometry::WritingMode::empty);
    if !top_layer {
        let fixed = style_value(values, "position", "static") == "fixed";
        let mut parent = Some(owner);
        let mut seen = BTreeSet::new();
        while let Some(id) = parent {
            if !seen.insert(id) {
                break;
            }
            if styles
                .get(&id)
                .is_some_and(|style| positioning::establishes(style, fixed))
            {
                if let Some(computed) = native.get(document, mirrors, id) {
                    cb_wm = computed.writing_mode;
                }
                break;
            }
            parent = input
                .nodes
                .iter()
                .find(|node| node.id == id)
                .and_then(|node| node.parent);
        }
    }

    let capture = |computed: &style::properties::ComputedValues,
                   values: &mut BTreeMap<String, String>| {
        values.retain(|key, _| !key.starts_with("-oryn-anchor-tree:"));
        for (property, name, scope) in document.anchor_reference_scopes(*mirror, computed) {
            values.insert(
                format!("-oryn-anchor-tree:{property}:{name}"),
                scope
                    .map(|scope| scope.as_u64().to_string())
                    .unwrap_or_else(|| "document".into()),
            );
        }
        values.insert(
            "-oryn-position-area".into(),
            computed
                .get_position()
                .clone_position_area()
                .to_physical(cb_wm, computed.writing_mode)
                .to_css_string(),
        );
    };
    capture(computed, values);
    if !needed(values) {
        return Ok(());
    }
    let mut candidates = Vec::new();
    let inheritance_parent = native.0.and_then(|values| values.get(&owner));
    for fallback in document.resolved_position_tries_with_parent(
        *mirror,
        computed,
        cb_wm,
        inheritance_parent.map(|parent| &**parent),
    ) {
        let mut candidate = values.clone();
        candidate.remove("-oryn-position-tries");
        for name in COMPUTED_PROPERTIES {
            if let Ok(property) = PropertyId::parse_enabled_for_all_content(name)
                && let Err(longhand) = property.as_shorthand()
            {
                candidate.insert(
                    (*name).to_owned(),
                    fallback.computed_value_to_string(longhand),
                );
            }
        }
        capture(&fallback, &mut candidate);
        candidates.push(candidate);
    }
    let serialized = serde_json::to_string(&candidates)
        .map_err(|error| LayoutError::StyleTraversal(error.to_string()))?;
    if serialized.len() > 4 * 1024 * 1024 {
        return Err(LayoutError::StyleTraversal(
            "picker anchor fallback cascade exceeds native bound".into(),
        ));
    }
    values.insert("-oryn-position-tries".into(), serialized);
    Ok(())
}

/// Capture fallback cascades while the native stylist and its scoped rule maps
/// are live. These private values never become authored DOM/CSS declarations.
pub(super) fn cascade(
    document: &blitz_dom::BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
) -> Result<(), LayoutError> {
    cascade_with_native(document, input, mirrors, styles, NativeValues::default())
}

pub(super) fn cascade_with_native(
    document: &blitz_dom::BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
    native: NativeValues<'_>,
) -> Result<(), LayoutError> {
    use style_traits::ToCss;
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    for node in &input.nodes {
        if let Some(style) = styles.get_mut(&node.id) {
            Arc::make_mut(style).insert(
                "-oryn-anchor-parent".into(),
                node.parent
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".into()),
            );
        }
    }
    for node in &input.nodes {
        let Some(style) = styles.get(&node.id).cloned() else {
            continue;
        };
        let Some(mirror) = mirrors.get(&node.id) else {
            continue;
        };
        let Some(primary) = native.get(document, mirrors, node.id) else {
            continue;
        };
        for (property, name, scope) in document.anchor_reference_scopes(*mirror, &primary) {
            Arc::make_mut(styles.get_mut(&node.id).unwrap()).insert(
                format!("-oryn-anchor-tree:{property}:{name}"),
                scope
                    .map(|scope| scope.as_u64().to_string())
                    .unwrap_or_else(|| "document".into()),
            );
        }
        if !needed(&style) {
            continue;
        }
        let fixed = style_value(&style, "position", "static") == "fixed";
        let mut parent = if input.top_layer.iter().any(|entry| entry.node_id == node.id) {
            None
        } else {
            node.parent
        };
        let mut cb_wm = native
            .root(input, node.id)
            .and_then(|id| native.get(document, mirrors, id))
            .map(|computed| computed.writing_mode)
            .unwrap_or_else(style::logical_geometry::WritingMode::empty);
        while let Some(id) = parent {
            if styles
                .get(&id)
                .is_some_and(|style| positioning::establishes(style, fixed))
            {
                if let Some(computed) = native.get(document, mirrors, id) {
                    cb_wm = computed.writing_mode;
                }
                break;
            }
            parent = nodes.get(&id).and_then(|node| node.parent);
        }
        let physical = primary
            .get_position()
            .clone_position_area()
            .to_physical(cb_wm, primary.writing_mode)
            .to_css_string();
        Arc::make_mut(styles.get_mut(&node.id).unwrap())
            .insert("-oryn-position-area".into(), physical);
        if style_value(&style, "position-try-fallbacks", "none") == "none" {
            continue;
        }
        let mut candidates = Vec::new();
        let inheritance_parent = native.1.and_then(|values| values.get(&node.id));
        for computed in document.resolved_position_tries_with_parent(
            *mirror,
            &primary,
            cb_wm,
            inheritance_parent.map(|parent| &**parent),
        ) {
            let mut values = style.as_ref().clone();
            values.remove("-oryn-position-tries");
            values.retain(|key, _| !key.starts_with("-oryn-anchor-tree:"));
            for name in COMPUTED_PROPERTIES {
                if let Ok(property) = PropertyId::parse_enabled_for_all_content(name)
                    && let Err(longhand) = property.as_shorthand()
                {
                    values.insert(
                        (*name).to_owned(),
                        computed.computed_value_to_string(longhand),
                    );
                }
            }
            values.insert(
                "-oryn-position-area".into(),
                computed
                    .get_position()
                    .clone_position_area()
                    .to_physical(cb_wm, computed.writing_mode)
                    .to_css_string(),
            );
            for (property, name, scope) in document.anchor_reference_scopes(*mirror, &computed) {
                values.insert(
                    format!("-oryn-anchor-tree:{property}:{name}"),
                    scope
                        .map(|scope| scope.as_u64().to_string())
                        .unwrap_or_else(|| "document".into()),
                );
            }
            candidates.push(values);
        }
        let value = serde_json::to_string(&candidates)
            .map_err(|error| LayoutError::StyleTraversal(error.to_string()))?;
        if value.len() > 4 * 1024 * 1024 {
            return Err(LayoutError::StyleTraversal(
                "anchor fallback cascade exceeds native bound".into(),
            ));
        }
        Arc::make_mut(styles.get_mut(&node.id).unwrap())
            .insert("-oryn-position-tries".into(), value);
    }
    Ok(())
}

/// Conditional visibility preserves CSSOM boxes and the computed visibility
/// property. A private used-state bit also reaches focus and accessibility.
pub(super) fn visibility(
    input: &LayoutInput,
    authored: &ComputedStyles,
    styles: &mut ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    projections: &BTreeMap<u64, oryn_common::v4::PaintProjection>,
    gutter: EdgeSizes,
) {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let viewport = CssRect {
        x: 0.,
        y: 0.,
        width: input.viewport.width,
        height: input.viewport.height,
    };
    let mut suppressed = BTreeSet::new();
    for _ in 0..input.nodes.len() {
        let mut changed = false;
        for node in &input.nodes {
            let Some(style) = authored.get(&node.id) else {
                continue;
            };
            let flags = style_value(style, "position-visibility", "always");
            if flags == "always"
                || suppressed.contains(&node.id)
                || !matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                )
            {
                continue;
            }
            let cb = positioning::containing_box(
                node.id,
                style_value(style, "position", "static") == "fixed",
                input,
                &nodes,
                styles,
                geometry,
                gutter,
            );
            let context = Context {
                id: node.id,
                input,
                styles,
                nodes: nodes.clone(),
                projections,
                cb,
                cb_owner: None,
                dependency_only: false,
                references: Default::default(),
            };
            let anchor = context.anchor(None, "");
            let invalid =
                flags.contains("anchors-valid") && anchor.and_then(|id| context.rect(id)).is_none();
            let invisible = flags.contains("anchors-visible")
                && anchor.is_some_and(|id| {
                    suppressed.contains(&id)
                        || geometry.get(&id).is_none_or(|value| {
                            value.availability != GeometryAvailability::Available
                        })
                        || projections.get(&id).is_none_or(|projection| {
                            oryn_common::visibility::regions(projection, viewport).map_or(
                                true,
                                |regions| {
                                    !regions
                                        .iter()
                                        .any(|polygon| oryn_common::visibility::area(polygon) > 0.)
                                },
                            )
                        })
                });
            let overflow = flags.contains("no-overflow")
                && geometry
                    .get(&node.id)
                    .and_then(|value| value.box_model.as_ref())
                    .is_some_and(|model| {
                        model.border_box.x - model.margin.left < cb.x
                            || model.border_box.y - model.margin.top < cb.y
                            || model.border_box.right() + model.margin.right > cb.right()
                            || model.border_box.bottom() + model.margin.bottom > cb.bottom()
                    });
            if invalid || invisible || overflow {
                let mut pending = vec![node.id];
                while let Some(id) = pending.pop() {
                    if suppressed.insert(id) {
                        changed = true;
                        if let Some(node) = nodes.get(&id) {
                            pending.extend(node.children.iter().copied());
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    for id in suppressed {
        if let Some(style) = styles.get_mut(&id) {
            Arc::make_mut(style).insert("-oryn-anchor-hidden".into(), "true".into());
        }
        if let Some(value) = geometry.get_mut(&id) {
            value.availability = GeometryAvailability::Hidden;
            value.in_viewport = false;
            value.interactable = false;
        }
    }
}

#[cfg(test)]
mod instance_scope_tests {
    use super::*;

    #[test]
    fn anchor_dependencies_keep_named_and_attribute_references_in_their_clone() {
        let input: LayoutInput = serde_json::from_value(serde_json::json!({
            "nodes":[
                {"id":1,"tag":"div","attributes":{"id":"anchor"}},
                {"id":3,"tag":"div","attributes":{"id":"anchor"}},
                {"id":2,"tag":"div","attributes":{"anchor":"anchor"}},
                {"id":4,"tag":"div","attributes":{"anchor":"anchor"}}
            ],
            "stylesheets":[],
            "viewport":{"width":100,"height":100,"device_pixel_ratio":1},
            "scroll":{"x":0,"y":0,"content_width":0,"content_height":0}
        }))
        .unwrap();
        for explicit in [true, false] {
            let mut styles = ComputedStyles::new();
            for (id, context, anchor) in
                [(1, 10, true), (3, 11, true), (2, 10, false), (4, 11, false)]
            {
                let mut values = BTreeMap::from([
                    ("display".into(), "block".into()),
                    ("position".into(), "absolute".into()),
                    (
                        "position-anchor".into(),
                        if explicit { "none" } else { "normal" }.into(),
                    ),
                    (INSTANCE_CONTEXT.into(), context.to_string()),
                ]);
                if anchor {
                    values.insert("anchor-name".into(), "--a".into());
                } else {
                    values.insert(
                        "top".into(),
                        if explicit {
                            "anchor(--a top, 0px)"
                        } else {
                            "anchor(top, 0px)"
                        }
                        .into(),
                    );
                }
                styles.insert(id, Arc::new(values));
            }
            assert_eq!(dependencies(2, &input, &styles), BTreeSet::from([1]));
            assert_eq!(dependencies(4, &input, &styles), BTreeSet::from([3]));
            // Even matching source CSS tree identities cannot join separate clones.
            for style in styles.values_mut() {
                Arc::make_mut(style)
                    .insert("-oryn-anchor-tree:anchor-name:".into(), "document".into());
                Arc::make_mut(style).insert("-oryn-anchor-tree:top:--a".into(), "document".into());
            }
            assert_eq!(dependencies(2, &input, &styles), BTreeSet::from([1]));
            Arc::make_mut(styles.get_mut(&1).unwrap()).insert(INSTANCE_CONTEXT.into(), "99".into());
            assert!(dependencies(2, &input, &styles).is_empty());
            // Unprojected document nodes retain ordinary anchor lookup behavior.
            for style in styles.values_mut() {
                Arc::make_mut(style).remove(INSTANCE_CONTEXT);
            }
            assert_eq!(
                dependencies(2, &input, &styles),
                BTreeSet::from([if explicit { 3 } else { 1 }])
            );
        }
    }
}
