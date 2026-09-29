//! Native permission controls constrain the computed style before layout and
//! expose that same projection through CSSOM. Their label is private UA content.
use super::*;
fn supported(node: &LayoutNode) -> bool {
    node.namespace == "http://www.w3.org/1999/xhtml"
        && matches!(
            node.tag.as_str(),
            "geolocation" | "usermedia" | "camera" | "microphone" | "install"
        )
}
fn pixels(value: Option<&String>, default: f64) -> f64 {
    value
        .and_then(|value| {
            value
                .trim()
                .trim_start_matches("calc(")
                .trim_end_matches(')')
                .trim_end_matches("px")
                .parse::<f64>()
                .ok()
        })
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}
fn px(value: f64) -> String {
    format!("{value}px")
}
pub(super) fn adjust(
    input: &LayoutInput,
    styles: &mut ComputedStyles,
    pseudos: &mut BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) {
    let parents = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    for node in &input.nodes {
        let mut parent = node.parent;
        while let Some(id) = parent {
            let Some(ancestor) = parents.get(&id) else {
                break;
            };
            if supported(ancestor) {
                if let Some(style) = styles.get(&node.id) {
                    let mut style = (**style).clone();
                    style.insert("display".into(), "none".into());
                    styles.insert(node.id, Arc::new(style));
                }
                break;
            }
            parent = ancestor.parent;
        }
    }
    for node in &input.nodes {
        if !supported(node) {
            continue;
        }
        let Some(style) = styles.get(&node.id) else {
            continue;
        };
        let mut style = (**style).clone();
        for name in [
            "border-image",
            "border-image-source",
            "background-image",
            "clip-path",
            "filter",
            "backdrop-filter",
            "mask",
            "mask-image",
            "transform",
            "translate",
            "rotate",
            "scale",
            "contain",
        ] {
            style.insert(name.into(), "none".into());
        }
        for (name, value) in [
            ("cursor", "pointer"),
            ("content-visibility", "visible"),
            ("user-select", "none"),
            ("-webkit-user-select", "none"),
            ("text-decoration-line", "none"),
            ("text-shadow", "none"),
            ("-webkit-text-security", "none"),
            ("-webkit-text-stroke-width", "0px"),
            ("-webkit-text-fill-color", "currentcolor"),
        ] {
            style.insert(name.into(), value.into());
        }
        for name in [
            "corner-shape",
            "corner-top-left-shape",
            "corner-top-right-shape",
            "corner-bottom-left-shape",
            "corner-bottom-right-shape",
        ] {
            style.insert(name.into(), "superellipse(1)".into());
        }
        let font = pixels(style.get("font-size"), 13.).max(1.);
        if let Some(value) = style.get("outline-offset") {
            let offset = oryn_common::css_math::resolve_length(
                value,
                oryn_common::css_math::LengthContext {
                    percentage_basis: input.viewport.width,
                    font_size: font,
                    root_font_size: 16.,
                    viewport_width: input.viewport.width,
                    viewport_height: input.viewport.height,
                },
            )
            .unwrap_or(0.)
            .max(0.);
            style.insert("outline-offset".into(), px(offset));
        }
        let weight = pixels(style.get("font-weight"), 400.).max(200.);
        style.insert("font-weight".into(), weight.to_string());
        if style
            .get("font-style")
            .is_some_and(|value| value.starts_with("oblique"))
        {
            style.insert("font-style".into(), "normal".into());
        }
        let word = pixels(style.get("word-spacing"), 0.).clamp(0., font * 0.5);
        style.insert("word-spacing".into(), px(word));
        let letter = pixels(style.get("letter-spacing"), 0.).clamp(-font * 0.05, font * 0.2);
        style.insert("letter-spacing".into(), px(letter));
        if style
            .get("box-shadow")
            .is_some_and(|value| value.contains("inset"))
        {
            style.insert("box-shadow".into(), "none".into());
        }
        let min_height = pixels(style.get("min-height"), font).clamp(font, 3. * font);
        let max_height = pixels(style.get("max-height"), 3. * font).clamp(min_height, 3. * font);
        style.insert("min-height".into(), format!("calc({min_height}px)"));
        style.insert("max-height".into(), format!("calc({max_height}px)"));
        for (axis, sides, limit) in [
            ("width", ["padding-left", "padding-right"], font * 5.),
            ("height", ["padding-top", "padding-bottom"], font),
        ] {
            let auto = style.get(axis).is_none_or(|value| value == "auto");
            let value = if auto {
                pixels(style.get(sides[0]), 0.).clamp(0., limit)
            } else {
                0.
            };
            for side in sides {
                style.insert(side.into(), px(value));
            }
        }
        for name in [
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-left-radius",
            "border-bottom-right-radius",
        ] {
            if let Some(value) = style.get(name).cloned() {
                let mut lengths = value.split_whitespace();
                let first = lengths.next().unwrap_or("0px");
                if lengths.next().is_some_and(|second| second != first) {
                    style.insert(name.into(), "0px".into());
                }
            }
        }
        let mut icon = pseudos
            .get(&node.id)
            .and_then(|values| values.get("permission-icon"))
            .map(|style| (**style).clone())
            .unwrap_or_default();
        let color = style
            .get("color")
            .cloned()
            .unwrap_or_else(|| "rgb(0, 0, 0)".into());
        let fill = icon
            .get("fill")
            .filter(|value| !value.eq_ignore_ascii_case("currentcolor"))
            .cloned()
            .unwrap_or(color);
        let height = pixels(icon.get("height"), font).clamp(font * 0.5, font * 1.5);
        let minimum = pixels(icon.get("min-height"), font * 0.5).clamp(font * 0.5, font * 1.5);
        let maximum = pixels(icon.get("max-height"), font * 1.5).clamp(minimum, font * 1.5);
        let height = height.clamp(minimum, maximum);
        let mut permitted = BTreeMap::new();
        for name in [
            "display",
            "margin-inline-end",
            "fill",
            "stroke",
            "stroke-width",
        ] {
            if let Some(value) = icon.remove(name) {
                permitted.insert(name.into(), value);
            }
        }
        for (name, value) in [
            ("content", "\"\"".into()),
            (
                "display",
                permitted
                    .get("display")
                    .cloned()
                    .unwrap_or_else(|| "inline-block".into()),
            ),
            ("fill", fill.clone()),
            ("color", fill),
            ("height", px(height)),
            ("width", px(height)),
            ("font-size", px(height)),
            ("line-height", px(height)),
            ("min-height", px(minimum)),
            ("max-height", px(maximum)),
            ("padding", "0px".into()),
            ("margin-inline-start", "0px".into()),
            ("float", "none".into()),
            ("opacity", "1".into()),
            ("stroke-opacity", "1".into()),
            ("fill-rule", "nonzero".into()),
            ("vertical-align", "middle".into()),
        ] {
            permitted.insert(name.into(), value);
        }
        let margin = pixels(permitted.get("margin-inline-end"), font * 0.5).clamp(0., font * 3.);
        permitted.insert("margin-inline-end".into(), px(margin));
        for name in [
            "direction",
            "writing-mode",
            "font-family",
            "font-weight",
            "font-style",
        ] {
            if let Some(value) = style.get(name) {
                permitted.insert(name.into(), value.clone());
            }
        }
        let writing = style
            .get("writing-mode")
            .map(String::as_str)
            .unwrap_or("horizontal-tb");
        let reverse = style.get("direction").is_some_and(|value| value == "rtl");
        let end = if writing == "horizontal-tb" {
            if reverse { "left" } else { "right" }
        } else if reverse ^ (writing == "sideways-lr") {
            "top"
        } else {
            "bottom"
        };
        for side in ["top", "right", "bottom", "left"] {
            permitted.insert(
                format!("margin-{side}"),
                px(if side == end { margin } else { 0. }),
            );
        }

        pseudos
            .entry(node.id)
            .or_default()
            .insert("permission-icon".into(), Arc::new(permitted));
        styles.insert(node.id, Arc::new(style));
        if let Some(pseudos) = pseudos.get_mut(&node.id) {
            for (name, style) in pseudos.iter_mut() {
                if matches!(name.as_str(), "before" | "after" | "marker") {
                    let mut values = (**style).clone();
                    values.insert("display".into(), "none".into());
                    values.insert("content".into(), "none".into());
                    *style = Arc::new(values);
                }
            }
        }
    }
}
pub(super) fn measure(input: &LayoutInput, styles: &mut ComputedStyles, engine: &mut TextEngine) {
    for node in &input.nodes {
        if !supported(node) {
            continue;
        }
        let Some(style) = styles.get(&node.id) else {
            continue;
        };
        let mut style = (**style).clone();
        let font = pixels(style.get("font-size"), 13.).max(1.);
        let mut label_style = style.clone();
        label_style.insert("white-space".into(), "nowrap".into());
        let shaped =
            engine.shape_styled(&node.text, 1_000_000., font, &label_style, input.viewport);
        let icon = input
            .nodes
            .iter()
            .find(|child| {
                child.parent == Some(node.id)
                    && child.tag == "img"
                    && child.intrinsic_image_size == Some((24, 24))
                    && !child.attributes.contains_key("src")
            })
            .and_then(|icon| styles.get(&icon.id));
        let icon_width = icon
            .filter(|style| style.get("display").is_none_or(|value| value != "none"))
            .map(|style| {
                pixels(style.get("width"), font)
                    + pixels(style.get("margin-inline-end"), font * 0.5)
            })
            .unwrap_or(0.);
        let natural = (shaped.width + icon_width).max(font);
        let min = pixels(style.get("min-width"), natural).clamp(natural, natural * 3.);
        let max = pixels(style.get("max-width"), natural * 3.).clamp(min, natural * 3.);
        style.insert("min-width".into(), px(min));
        style.insert("max-width".into(), px(max));
        styles.insert(node.id, Arc::new(style));
    }
}
pub(super) fn project(input: &LayoutInput) -> Cow<'_, LayoutInput> {
    if !input.nodes.iter().any(supported) {
        return Cow::Borrowed(input);
    }
    let mut projected = input.clone();
    let languages = effective_languages(input);
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let controls = input
        .nodes
        .iter()
        .filter(|node| supported(node))
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    for node in &mut projected.nodes {
        let mut parent = node.parent;
        let mut hidden = false;
        while let Some(id) = parent {
            if controls.contains(&id) {
                hidden = true;
                break;
            }
            parent = by_id.get(&id).and_then(|node| node.parent);
        }
        if hidden {
            node.content = Some(Vec::new());
            node.children.clear();
            node.text.clear();
            continue;
        }
        if !controls.contains(&node.id) {
            continue;
        }
        let language = languages
            .get(&node.id)
            .and_then(|value| value.as_deref())
            .unwrap_or("en")
            .to_ascii_lowercase();
        let granted = input
            .selector_states
            .get(&node.id)
            .is_some_and(|state| state.permission_granted);
        let geo = node.tag == "geolocation";
        let label = if node.tag == "install" {
            "Install"
        } else if node.tag == "microphone" {
            if granted {
                "Microphone allowed"
            } else {
                "Use microphone"
            }
        } else if node.tag == "camera" {
            if granted {
                "Camera allowed"
            } else {
                "Use camera"
            }
        } else if language.starts_with("de") {
            if geo {
                "Standort verwenden"
            } else {
                "Kamera und Mikrofon verwenden"
            }
        } else if language.starts_with("fr") {
            if geo {
                "Utiliser la position"
            } else {
                "Utiliser la caméra et le microphone"
            }
        } else if language.starts_with("es") {
            if geo {
                "Usar ubicación"
            } else {
                "Usar cámara y micrófono"
            }
        } else if language.starts_with("hu") {
            if geo {
                "Helyadatok használata"
            } else {
                "Kamera és mikrofon használata"
            }
        } else if geo {
            if granted {
                "Location allowed"
            } else {
                "Use location"
            }
        } else if granted {
            "Camera and microphone allowed"
        } else {
            "Use camera and microphone"
        };
        node.children.clear();
        node.text = label.into();
        node.content = Some(vec![LayoutContent::Text {
            node_id: node.id,
            text: label.into(),
        }]);
        if let Some(tree) = projected.rendered_tree.as_mut() {
            tree.insert(node.id, node.content.clone().unwrap());
        }
    }
    Cow::Owned(projected)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> LayoutInput {
        serde_json::from_value(serde_json::json!({"nodes":[{"id":1,"parent":null,"tag":"body","children":[2],"attributes":{"lang":"en"}},{"id":2,"parent":1,"tag":"usermedia","children":[3],"attributes":{"style":"font-size:10px;width:auto;height:auto;min-height:1px;max-height:100px;padding:1000px;background-image:url(bad.png);filter:blur(5px);transform:scale(2);font-weight:100;letter-spacing:9px"}},{"id":3,"parent":2,"tag":"div","text":"AUTHOR CONTENT","attributes":{"style":"background:red"}}],"stylesheets":["usermedia::before{content:'AUTHOR PSEUDO'}"],"viewport":ViewportState{width:800.,height:600.,device_pixel_ratio:1.},"scroll":ScrollState::default()})).unwrap()
    }
    #[test]
    fn ordinary_documents_borrow_the_unmodified_input() {
        let mut input = input();
        assert!(matches!(project(&input), Cow::Owned(_)));
        input.nodes[1].tag = "div".into();
        let Cow::Borrowed(projected) = project(&input) else {
            panic!("ordinary layout must not duplicate the input");
        };
        assert!(std::ptr::eq(projected, &input));
        assert_eq!(projected.nodes[2].text, "AUTHOR CONTENT");
    }
    #[test]
    fn install_and_camera_share_locked_layout_but_keep_distinct_private_labels() {
        for (tag, label) in [
            ("camera", "Use camera"),
            ("microphone", "Use microphone"),
            ("install", "Install"),
        ] {
            let mut input = input();
            input.nodes[1].tag = tag.into();
            input.nodes[1]
                .attributes
                .get_mut("style")
                .unwrap()
                .push_str(";outline-offset:clamp(-100px,1vw,-50px)");
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert_eq!(output.computed_styles[&2]["filter"], "none");
            assert_eq!(output.computed_styles[&2]["outline-offset"], "0px");
            assert_eq!(output.computed_styles[&3]["display"], "none");
            assert_eq!(
                output.computed_styles[&2]["padding-right"],
                output.computed_styles[&2]["padding-left"]
            );
            assert_eq!(
                output.computed_styles[&2]["padding-bottom"],
                output.computed_styles[&2]["padding-top"]
            );
            assert_eq!(project(&input).nodes[1].text, label);
            let icon = output
                .generated_nodes
                .iter()
                .find(|node| node.tag == "::permission-icon")
                .unwrap();
            assert!(icon.builtin_image.is_some());
        }
    }
    #[test]
    fn locked_native_control_style_and_private_label_share_the_layout_projection() {
        let mut input = input();
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let style = &output.computed_styles[&2];
        for name in ["background-image", "filter", "transform"] {
            assert_eq!(style[name], "none")
        }
        assert_eq!(style["font-weight"], "200");
        assert_eq!(style["letter-spacing"], "2px");
        assert_eq!(style["padding-left"], "50px");
        assert_eq!(style["padding-top"], "10px");
        assert_eq!(output.computed_styles[&3]["display"], "none");
        assert_eq!(output.pseudo_styles[&2]["before"]["content"], "none");
        assert!(!project(&input).nodes[1].text.contains("AUTHOR"));
        let english = project(&input).nodes[1].text.clone();
        input.nodes[0].attributes.insert("lang".into(), "de".into());
        assert_ne!(project(&input).nodes[1].text, english);
    }
    #[test]
    fn permission_icon_uses_private_typed_paint_and_cannot_request_author_images() {
        let mut input = input();
        input.stylesheets.push("usermedia::permission-icon{fill:rgb(255, 0, 0);stroke:rgb(0, 0, 255);content:url(https://author.test/icon.svg);opacity:0;height:100px;margin-inline-end:100px}".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let icon = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::permission-icon")
            .unwrap();
        assert!(matches!(
            icon.builtin_image.as_ref(),
            Some(oryn_common::builtin_image::BuiltinImage::PermissionIcon {
                fill: [255, 0, 0, 255],
                stroke: [0, 0, 255, 255],
                ..
            })
        ));
        assert!(icon.image_resource.is_none());
        assert_eq!(icon.style["height"], "15px");
        assert_eq!(icon.style["opacity"], "1");
        assert_eq!(icon.style["margin-inline-end"], "30px");
        assert_eq!(icon.style["margin-right"], "30px");
    }
    #[test]
    fn native_permission_popup_paints_three_choices_without_author_nodes() {
        let mut input = input();
        input.permission_prompts.insert(
            2,
            oryn_dom::PermissionPrompt {
                id: "native-session".into(),
                origin: "https://trusted.example".into(),
                geolocation: false,
                protocol: None,
                protocol_url: None,
                active: 2,
            },
        );
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::PermissionPrompt,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for tag in [
            "::permission-allow",
            "::permission-deny",
            "::permission-dismiss",
        ] {
            let choice = output
                .generated_nodes
                .iter()
                .find(|node| node.tag == tag)
                .unwrap();
            assert_eq!(choice.parent_id, Some(2));
            assert!(choice.geometry.box_model.is_some());
            assert!(!input.nodes.iter().any(|node| node.id == choice.node_id));
        }
    }
}
