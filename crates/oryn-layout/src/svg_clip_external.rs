//! External clip documents are parsed only from successful native CSS resources.
//! No author URL lookup or network operation occurs during geometry extraction.
use super::*;
pub(super) fn resolve(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    reference: CssRect,
) -> Result<oryn_common::basic_shape::Shape, String> {
    resolve_with_space(input, owner, source, reference, None, None)
}
pub(super) fn resolve_svg(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    reference: CssRect,
    viewport: CssRect,
) -> Result<oryn_common::basic_shape::Shape, String> {
    resolve_with_space(input, owner, source, reference, Some(viewport), None)
}
pub(super) fn resolve_styled(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    reference: CssRect,
    viewport: Option<CssRect>,
    styles: &ComputedStyles,
) -> Result<oryn_common::basic_shape::Shape, String> {
    resolve_with_space(
        input,
        owner,
        source,
        reference,
        viewport,
        styles.get(&owner.id).map(|style| style.as_ref()),
    )
}
fn resolve_with_space(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    reference: CssRect,
    svg_viewport: Option<CssRect>,
    style: Option<&BTreeMap<String, String>>,
) -> Result<oryn_common::basic_shape::Shape, String> {
    let raw = source
        .trim()
        .strip_prefix("url(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or("clip URL expected")?
        .trim()
        .trim_matches(['\'', '"']);
    let document = url::Url::parse(
        owner
            .document_url
            .as_deref()
            .ok_or("clip Document URL missing")?,
    )
    .map_err(|e| e.to_string())?;
    let mut requested = document.join(raw).map_err(|e| e.to_string())?;
    let fragment = requested
        .fragment()
        .ok_or("clip resource fragment missing")?
        .to_owned();
    requested.set_fragment(None);
    if requested.origin() != document.origin() {
        return Err("cross-origin external clip document".into());
    }
    let same_resource = |value: &str| {
        url::Url::parse(value).ok().is_some_and(|mut value| {
            value.set_fragment(None);
            value == requested
        })
    };
    let resource = input
        .svg_clip_resources
        .iter()
        .find(|resource| {
            same_resource(&resource.url)
                || resource
                    .url_aliases
                    .iter()
                    .any(|value| same_resource(value))
        })
        .ok_or("external clip resource pending or unavailable")?;
    let mut final_url = url::Url::parse(&resource.url).map_err(|e| e.to_string())?;
    final_url.set_fragment(None);
    if final_url.origin() != document.origin()
        || !oryn_common::mime_type::MimeType::parse(&resource.media_type)
            .is_some_and(|mime| mime.essence() == "image/svg+xml")
    {
        return Err("external clip response origin or type mismatch".into());
    }
    if resource.bytes.len() > 16 * 1024 * 1024 {
        return Err("external SVG clip byte budget".into());
    }
    let source = oryn_common::xml_encoding::decode(
        &resource.bytes,
        resource
            .content_type
            .as_deref()
            .or(Some(&resource.media_type)),
    )?;
    if source.len() > 48 * 1024 * 1024 {
        return Err("external SVG clip decoded byte budget".into());
    }
    let tree = roxmltree::Document::parse_with_options(
        &source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 65536,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    if tree.root_element().tag_name().namespace() != Some("http://www.w3.org/2000/svg")
        || tree.root_element().tag_name().name() != "svg"
    {
        return Err("external clip document is not SVG".into());
    }
    let elements = tree
        .descendants()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let ids = elements
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id().get(), index as u64 + 1))
        .collect::<BTreeMap<_, _>>();
    let text_ids = tree
        .descendants()
        .filter(|node| node.is_text())
        .enumerate()
        .map(|(index, node)| (node.id().get(), index as u64 + 65537))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::with_capacity(elements.len());
    let mut sheets = Vec::new();
    for node in elements {
        let mut attributes = BTreeMap::new();
        let mut namespaced = Vec::new();
        for attribute in node.attributes() {
            if let Some(namespace) = attribute.namespace() {
                namespaced.push(LayoutAttribute {
                    namespace: namespace.into(),
                    local_name: attribute.name().into(),
                    prefix: None,
                    value: attribute.value().into(),
                });
            } else {
                attributes.insert(attribute.name().to_owned(), attribute.value().to_owned());
            }
        }
        let text = node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .collect::<String>();
        if node.tag_name().name() == "style"
            && node.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
        {
            sheets.push(text.clone());
        }
        nodes.push(serde_json::from_value::<LayoutNode>(serde_json::json!({"id":ids[&node.id().get()],"parent":node.parent().and_then(|node|ids.get(&node.id().get()).copied()),"tree_scope":1,"document_url":final_url.as_str(),"namespace":node.tag_name().namespace().unwrap_or(""),"tag":node.tag_name().name(),"attributes":attributes,"namespaced_attributes":namespaced,"text":text,"content":node.children().filter_map(|child|if child.is_text(){Some(LayoutContent::Text{node_id:text_ids[&child.id().get()],text:child.text().unwrap_or("").into()})}else{ids.get(&child.id().get()).copied().map(LayoutContent::Element)}).collect::<Vec<_>>(),"children":node.children().filter_map(|child|ids.get(&child.id().get()).copied()).collect::<Vec<_>>()})).map_err(|e|e.to_string())?);
    }
    let mut local:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":sheets,"scroll":ScrollState::default(),"viewport":input.viewport,"document_base_url":final_url.as_str()})).map_err(|e|e.to_string())?;
    // The resource owns its cascade. The embedding page's selectors and registered
    // font faces must not enter an independently fetched SVG document.
    local.svg_clip_resources = Vec::new();
    local.reduced_motion = input.reduced_motion;
    let schemes = style
        .and_then(|style| style.get("color-scheme"))
        .map(String::as_str)
        .unwrap_or("normal")
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    local.color_scheme = match (schemes.contains(&"light"), schemes.contains(&"dark")) {
        (false, true) => ColorScheme::Dark,
        (true, true) => input.color_scheme,
        _ => ColorScheme::Light,
    };
    local.web_fonts = svg_fonts::embedded(
        &local.stylesheets,
        final_url.as_str(),
        local.viewport,
        local.color_scheme,
        local.reduced_motion,
    )?;
    let (styles, _, _) = stylo_computed_values_timed(&local, false, 0., None, &[], None)
        .map_err(|e| e.to_string())?;
    let local_owner = local.nodes.first().ok_or("empty SVG clip document")?;
    match svg_viewport {
        Some(viewport) => super::svg_clip::resolve_svg(
            &local,
            local_owner,
            &format!("url(#{fragment})"),
            reference,
            viewport,
            &styles,
        ),
        None => super::svg_clip::resolve(
            &local,
            local_owner,
            &format!("url(#{fragment})"),
            reference,
            &styles,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admitted_external_clip_uses_resource_cascade_and_original_response_origin() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let owner:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tag":"div","text":"","document_url":"https://example.test/page"})).unwrap();
        input.svg_clip_resources.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://example.test/clips.svg".into(),url_aliases:vec!["https://example.test/request.svg#part".into()],media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><style>rect { width: 20px; height: 30px }</style><clipPath id="part"><rect/></clipPath></svg>"#.to_vec()});
        let reference = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let shape = resolve(
            &input,
            &owner,
            "url(https://example.test/request.svg#part)",
            reference,
        )
        .unwrap();
        assert!(shape.contains(10., 10.));
        assert!(!shape.contains(50., 50.));
        input.svg_clip_resources[0].url = "https://foreign.test/clips.svg".into();
        assert!(
            resolve(
                &input,
                &owner,
                "url(https://example.test/request.svg#part)",
                reference
            )
            .is_err()
        );
        input.svg_clip_resources.clear();
        assert!(
            resolve(
                &input,
                &owner,
                "url(https://example.test/request.svg#part)",
                reference
            )
            .is_err()
        );
    }
    #[test]
    fn external_clip_accepts_native_xml_encoding_without_network_or_page_charset() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let owner:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tag":"div","document_url":"https://example.test/page"})).unwrap();
        let xml = "<svg xmlns='http://www.w3.org/2000/svg'><clipPath id='é'><rect width='20' height='30'/></clipPath></svg>";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        input
            .svg_clip_resources
            .push(oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                content_type: None,
                url: "https://example.test/c.svg".into(),
                url_aliases: vec![],
                media_type: "image/svg+xml; charset=windows-1252".into(),
                bytes,
            });
        let reference = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let shape = resolve(&input, &owner, "url(c.svg#%C3%A9)", reference).unwrap();
        assert!(shape.contains(10., 10.));
        assert!(!shape.contains(40., 40.));
        input.svg_clip_resources[0].bytes = xml.chars().map(|value| value as u8).collect();
        let shape = resolve(&input, &owner, "url(c.svg#%C3%A9)", reference).unwrap();
        assert!(shape.contains(10., 10.));
        input.svg_clip_resources[0].media_type = "image/svg+xml; charset=utf-8".into();
        assert!(resolve(&input, &owner, "url(c.svg#%C3%A9)", reference).is_err());
    }
    #[test]
    fn external_clip_rejects_dtd_and_invalid_document_before_geometry() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let owner:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tag":"div","text":"","document_url":"https://example.test/page"})).unwrap();
        input.svg_clip_resources.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://example.test/c.svg".into(),url_aliases:vec![],media_type:"image/svg+xml".into(),bytes:br#"<!DOCTYPE svg [<!ENTITY x "payload">]><svg xmlns="http://www.w3.org/2000/svg"><clipPath id="c"/></svg>"#.to_vec()});
        assert!(
            resolve(
                &input,
                &owner,
                "url(https://example.test/c.svg#c)",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.
                }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod embedding_media_tests {
    use super::*;
    #[test]
    fn shared_clip_document_resolves_owner_scheme_and_native_motion_independently() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let owner:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tag":"div","document_url":"https://example.test/page"})).unwrap();
        input.svg_clip_resources.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://example.test/clip.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><style>rect{width:10px;height:10px}@media(prefers-color-scheme:dark){rect{width:40px}}@media(prefers-reduced-motion:reduce){rect{width:60px}}</style><clipPath id="clip"><rect/></clipPath></svg>"#.to_vec()});
        let bounds = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        for (scheme, motion, inside, outside) in [
            ("light", false, 5., 20.),
            ("dark", false, 30., 50.),
            ("light", true, 50., 70.),
        ] {
            input.reduced_motion = motion;
            let styles = BTreeMap::from([(
                1,
                std::sync::Arc::new(BTreeMap::from([("color-scheme".into(), scheme.into())])),
            )]);
            let shape = resolve_styled(&input, &owner, "url(clip.svg#clip)", bounds, None, &styles)
                .unwrap();
            assert!(shape.contains(inside, 5.));
            assert!(!shape.contains(outside, 5.));
        }
    }
}
