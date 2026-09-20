//! Native styles keyed to the SVG parser's existing use-instance ancestry.
use super::*;
use oryn_layout::svg_instances::{StyleInstance, StyleParent};
use std::collections::BTreeMap;

pub struct ResolvedInstances {
    pub text_fonts: oryn_layout::svg_instances::TextFonts,
    pub presentation: resvg::usvg::NativeInstanceStyles,
    // HTML descendants need the full native cascade for local layout, not only
    // the presentation subset consumed by the SVG parser.
    pub computed: BTreeMap<resvg::usvg::NativeInstanceKey, oryn_layout::ComputedStyle>,
    pub pseudos:
        BTreeMap<resvg::usvg::NativeInstanceKey, BTreeMap<String, oryn_layout::ComputedStyle>>,
}

impl ResolvedInstances {
    /// Retain already-cascaded styles for a separately captured nested SVG.
    /// Strip only its enclosing use ancestry; source identities stay native.
    pub fn for_subtree(&self, source_ids: &[u64], enclosing: &[u64]) -> Result<Self, RenderError> {
        let ids = source_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if ids.len() != source_ids.len() || enclosing.len() > 128 {
            return Err(failure("invalid nested capture identity or ancestry"));
        }
        let mut presentation = resvg::usvg::NativeInstanceStyles::new();
        let mut computed = BTreeMap::new();
        let mut pseudos = BTreeMap::new();
        for (key, css) in &self.presentation {
            let Some(ancestry) = key.ancestry.strip_prefix(enclosing) else {
                continue;
            };
            // Empty ancestry denotes the nested capture's ordinary source,
            // whose primary styles already arrive through local layout.
            if ancestry.is_empty()
                || !ids.contains(&key.source)
                || ancestry.iter().any(|id| !ids.contains(id))
            {
                continue;
            }
            let native = self
                .computed
                .get(key)
                .ok_or_else(|| failure("nested capture lacks native instance style"))?;
            let local = resvg::usvg::NativeInstanceKey {
                ancestry: ancestry.to_vec(),
                source: key.source,
            };
            if presentation.insert(local.clone(), css.clone()).is_some() {
                return Err(failure("ambiguous nested capture instance ancestry"));
            }
            computed.insert(local.clone(), native.clone());
            if let Some(styles) = self.pseudos.get(key) {
                pseudos.insert(local, styles.clone());
            }
        }
        Ok(Self {
            presentation,
            computed,
            pseudos,
            text_fonts: self.text_fonts.clone(),
        })
    }
}

/// Resolve captured SVG instances against the authoritative document cascade.
/// `source_ids` identifies every serialized XML element in preorder, including
/// HTML descendants and style elements. The XML supplies reference topology;
/// native input supplies selectors, stylesheet scope and inherited values.
pub fn capture_svg_instances(
    source: &str,
    source_ids: &[u64],
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
) -> Result<ResolvedInstances, RenderError> {
    capture_svg_instances_for_media(source, source_ids, input, limits, false)
}

pub fn capture_svg_instances_for_media(
    source: &str,
    source_ids: &[u64],
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
    print_media: bool,
) -> Result<ResolvedInstances, RenderError> {
    capture_svg_instances_with_style_sample(source, source_ids, input, limits, print_media, None)
}

pub fn capture_svg_instances_with_style_sample(
    source: &str,
    source_ids: &[u64],
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
    print_media: bool,
    sample: Option<&oryn_layout::svg_instances::StyleSample>,
) -> Result<ResolvedInstances, RenderError> {
    if source.len() > 16 * 1024 * 1024 || source_ids.len() > limits.max_nodes {
        return Err(failure("captured source byte or node budget exceeded"));
    }
    let xml = roxmltree::Document::parse(source).map_err(|error| failure(&error.to_string()))?;
    if !xml
        .root_element()
        .has_tag_name(("http://www.w3.org/2000/svg", "svg"))
    {
        return Err(failure("captured source requires an SVG root"));
    }
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut ids = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut supplied = source_ids.iter();
    for element in xml.descendants().filter(|node| node.is_element()) {
        let id = *supplied
            .next()
            .ok_or_else(|| failure("missing captured source identity"))?;
        if !seen.insert(id) {
            return Err(failure("duplicate captured source identity"));
        }
        let node = nodes
            .get(&id)
            .ok_or_else(|| failure("captured source identity absent from native layout"))?;
        if node.tag != element.tag_name().name()
            || node.namespace != element.tag_name().namespace().unwrap_or("")
        {
            return Err(failure(
                "captured source identity has a different native element type",
            ));
        }
        if let Some(parent) = element.parent_element() {
            let expected = ids.get(&parent.id().get()).copied();
            if node.parent != expected {
                return Err(failure(
                    "captured source parent differs from native topology",
                ));
            }
        }
        ids.insert(element.id().get(), id);
    }
    if supplied.next().is_some() {
        return Err(failure("unused captured source identity"));
    }
    compute_for_sample(&xml, &ids, input, limits, print_media, sample)
}

struct Graph<'a, 'input> {
    ids: &'a BTreeMap<u32, u64>,
    references: BTreeMap<&'a str, roxmltree::Node<'a, 'input>>,
    requests: Vec<StyleInstance>,
    keys: Vec<resvg::usvg::NativeInstanceKey>,
    max_instances: usize,
}

fn failure(message: &str) -> RenderError {
    RenderError::InvalidPrint(format!("secure SVG use styles: {message}"))
}

impl<'a, 'input> Graph<'a, 'input> {
    fn reference(&self, node: roxmltree::Node<'a, 'input>) -> Option<roxmltree::Node<'a, 'input>> {
        let href = node
            .attribute(("http://www.w3.org/1999/xlink", "href"))
            .or_else(|| node.attribute("href"))?;
        self.references
            .get(resvg::usvg::native_instance_reference(href)?)
            .copied()
            .filter(|target| {
                // Native composition turns foreignObject into a supported SVG
                // group before parser expansion; its HTML needs styles first.
                target.has_tag_name(("http://www.w3.org/2000/svg", "foreignObject"))
                    || resvg::usvg::native_instance_reference_tag(
                        target.tag_name().namespace(),
                        target.tag_name().name(),
                    )
            })
    }

    fn walk(
        &mut self,
        node: roxmltree::Node<'a, 'input>,
        parent: StyleParent,
        ancestry: &[u64],
        active: &mut Vec<u32>,
    ) -> Result<(), RenderError> {
        if !node.is_element() || node.tag_name().name() == "style" {
            return Ok(());
        }
        // A cycle has no new rendered instance. The SVG parser independently
        // applies its existing recursive-reference checks to the same markup.
        if active.contains(&node.id().get()) {
            return Ok(());
        }
        if active.len() >= 128 || self.requests.len() >= self.max_instances {
            return Err(failure("instance depth or node budget exceeded"));
        }
        let source = self.ids[&node.id().get()];
        let instance = self.requests.len() as u64 + 1;
        self.requests.push(StyleInstance {
            id: instance,
            source,
            parent,
        });
        self.keys.push(resvg::usvg::NativeInstanceKey {
            ancestry: ancestry.to_vec(),
            source,
        });
        active.push(node.id().get());
        if node.has_tag_name(("http://www.w3.org/2000/svg", "use")) {
            if let Some(reference) = self.reference(node) {
                let mut nested = ancestry.to_vec();
                nested.push(source);
                self.walk(reference, StyleParent::Instance(instance), &nested, active)?;
            }
        } else {
            for child in node.children().filter(|child| child.is_element()) {
                self.walk(child, StyleParent::Instance(instance), ancestry, active)?;
            }
        }
        active.pop();
        Ok(())
    }
}

pub(super) fn compute<'a, 'input>(
    xml: &'a roxmltree::Document<'input>,
    ids: &'a BTreeMap<u32, u64>,
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
) -> Result<ResolvedInstances, RenderError> {
    compute_for_sample(xml, ids, input, limits, false, None)
}

fn compute_for_sample<'a, 'input>(
    xml: &'a roxmltree::Document<'input>,
    ids: &'a BTreeMap<u32, u64>,
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
    print_media: bool,
    sample: Option<&oryn_layout::svg_instances::StyleSample>,
) -> Result<ResolvedInstances, RenderError> {
    let mut graph = Graph {
        ids,
        references: BTreeMap::new(),
        requests: Vec::new(),
        keys: Vec::new(),
        max_instances: limits.max_nodes.saturating_sub(input.nodes.len()),
    };
    for node in xml.descendants().filter(|node| node.is_element()) {
        if let Some(id) = node.attribute("id") {
            // Match the parser's first-id resolution for duplicate source IDs.
            graph.references.entry(id).or_insert(node);
        }
    }
    for node in xml
        .descendants()
        .filter(|node| node.has_tag_name(("http://www.w3.org/2000/svg", "use")))
    {
        if let Some(reference) = graph.reference(node) {
            let source = ids[&node.id().get()];
            graph.walk(
                reference,
                StyleParent::Source(source),
                &[source],
                &mut vec![node.id().get()],
            )?;
        }
    }
    let styles = oryn_layout::svg_instances::compute_with_style_sample(
        input,
        &graph.requests,
        limits,
        print_media,
        sample,
    )
    .map_err(|error| failure(&error.to_string()))?;
    let mut output = resvg::usvg::NativeInstanceStyles::new();
    let mut computed = BTreeMap::new();
    let mut pseudos = BTreeMap::new();
    let mut bytes = 0usize;
    for (request, key) in graph.requests.iter().zip(graph.keys) {
        let values = styles
            .styles
            .get(&request.id)
            .ok_or_else(|| failure("missing native instance style"))?;
        let css = crate::svg_secure_foreign::resolved_style(values, true, &input.document_base_url);
        bytes = bytes
            .checked_add(css.len())
            .filter(|size| *size <= 16 * 1024 * 1024)
            .ok_or_else(|| failure("instance presentation byte budget exceeded"))?;
        computed.insert(key.clone(), values.clone());
        if let Some(styles) = styles.pseudo_styles.get(&request.id) {
            pseudos.insert(key.clone(), styles.clone());
        }
        if output.insert(key, css).is_some() {
            return Err(failure("duplicate instance ancestry"));
        }
    }
    Ok(ResolvedInstances {
        presentation: output,
        computed,
        pseudos,
        text_fonts: styles.text_fonts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph<'a, 'input>(
        xml: &'a roxmltree::Document<'input>,
        ids: &'a BTreeMap<u32, u64>,
        max_instances: usize,
    ) -> Graph<'a, 'input> {
        let mut references = BTreeMap::new();
        for node in xml.descendants().filter(|node| node.is_element()) {
            if let Some(id) = node.attribute("id") {
                references.entry(id).or_insert(node);
            }
        }
        Graph {
            ids,
            references,
            requests: Vec::new(),
            keys: Vec::new(),
            max_instances,
        }
    }

    fn ids(xml: &roxmltree::Document<'_>) -> BTreeMap<u32, u64> {
        xml.descendants()
            .filter(|node| node.is_element())
            .enumerate()
            .map(|(index, node)| (node.id().get(), index as u64 + 1))
            .collect()
    }

    fn captured_fixture() -> (String, Vec<u64>, oryn_layout::LayoutInput) {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><rect id="shape"/></defs><use href="#shape"/><use href="#shape"/></svg>"##.to_owned();
        let ids = vec![101, 205, 309, 413, 517];
        let input = serde_json::from_value(serde_json::json!({
            "nodes":[
                {"id":99,"parent":null,"tag":"div","namespace":"http://www.w3.org/1999/xhtml","attributes":{"class":"document-owner"},"children":[101]},
                {"id":101,"parent":99,"tag":"svg","namespace":"http://www.w3.org/2000/svg","children":[205,413,517]},
                {"id":205,"parent":101,"tag":"defs","namespace":"http://www.w3.org/2000/svg","children":[309]},
                {"id":309,"parent":205,"tag":"rect","namespace":"http://www.w3.org/2000/svg","attributes":{"id":"shape"}},
                {"id":413,"parent":101,"tag":"use","namespace":"http://www.w3.org/2000/svg","attributes":{"href":"#shape","style":"--paint:red"}},
                {"id":517,"parent":101,"tag":"use","namespace":"http://www.w3.org/2000/svg","attributes":{"href":"#shape","style":"--paint:blue"}}
            ],
            "stylesheets":[".document-owner #shape {fill:var(--paint)}"],
            "viewport":{"width":20,"height":20,"device_pixel_ratio":1},
            "scroll":{"x":0,"y":0,"content_width":0,"content_height":0}
        })).unwrap();
        (source, ids, input)
    }

    #[test]
    fn captured_graph_uses_the_owning_animation_sample_with_native_ids() {
        let (source, ids, mut input) = captured_fixture();
        input.stylesheets.push("use { animation:grow 2s linear both } @keyframes grow { from { font-size:10px } to { font-size:30px } }".into());
        let limits = oryn_layout::LayoutLimits::default();
        let mut engine = oryn_layout::StyleEngine::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        for time in [0.5, 1.5] {
            let layout = engine.compute_at(&input, &limits, time).unwrap();
            let sample = engine.instance_style_sample().unwrap();
            let projection = capture_svg_instances_with_style_sample(
                &source,
                &ids,
                &input,
                &limits,
                false,
                Some(&sample),
            )
            .unwrap();
            for owner in [413, 517] {
                let key = resvg::usvg::NativeInstanceKey {
                    ancestry: vec![owner],
                    source: 309,
                };
                assert_eq!(
                    projection.computed[&key]["font-size"],
                    layout.computed_styles[&owner]["font-size"]
                );
            }
        }
    }

    #[test]
    fn captured_instances_use_native_document_selectors_and_nonordinal_identities() {
        let (source, ids, input) = captured_fixture();
        let result =
            capture_svg_instances(&source, &ids, &input, &oryn_layout::LayoutLimits::default())
                .unwrap();
        for (owner, color) in [(413, "rgb(255, 0, 0)"), (517, "rgb(0, 0, 255)")] {
            let key = resvg::usvg::NativeInstanceKey {
                ancestry: vec![owner],
                source: 309,
            };
            assert_eq!(result.computed[&key]["fill"], color);
            assert!(result.presentation[&key].contains(color));
        }
        assert_eq!(result.computed.len(), 2);
    }

    #[test]
    fn captured_instances_reject_missing_duplicate_unknown_and_mismatched_identities() {
        let (source, ids, input) = captured_fixture();
        let limits = oryn_layout::LayoutLimits::default();
        for supplied in [
            vec![101, 205, 309, 413],
            vec![101, 205, 309, 413, 517, 99],
            vec![101, 205, 309, 413, 413],
            vec![101, 205, 309, 413, 999],
            vec![99, 205, 309, 413, 517],
        ] {
            assert!(capture_svg_instances(&source, &supplied, &input, &limits).is_err());
        }
        assert!(capture_svg_instances("<svg/>", &[101], &input, &limits).is_err());
        let mut disconnected = input.clone();
        disconnected
            .nodes
            .iter_mut()
            .find(|node| node.id == 309)
            .unwrap()
            .parent = Some(101);
        assert!(capture_svg_instances(&source, &ids, &disconnected, &limits).is_err());
        let mut limited = limits;
        limited.max_nodes = ids.len() - 1;
        assert!(capture_svg_instances(&source, &ids, &input, &limited).is_err());
    }

    #[test]
    fn recursive_capture_retains_enclosing_instance_variables_without_crossing_siblings() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><foreignObject id="panel"><div xmlns="http://www.w3.org/1999/xhtml"><svg xmlns="http://www.w3.org/2000/svg" id="nested"><defs><rect id="shape"/></defs><use id="inner" href="#shape"/></svg></div></foreignObject></defs><use id="red" href="#panel" style="--paint:red"/><use id="blue" href="#panel" style="--paint:blue"/></svg>"##;
        let xml = roxmltree::Document::parse(source).unwrap();
        let mapping = xml
            .descendants()
            .filter(|node| node.is_element())
            .enumerate()
            .map(|(index, node)| (node.id().get(), index as u64 + 100))
            .collect::<BTreeMap<_, _>>();
        let mut nodes = Vec::new();
        let mut source_ids = Vec::new();
        for node in xml.descendants().filter(|node| node.is_element()) {
            let id = mapping[&node.id().get()];
            source_ids.push(id);
            nodes.push(serde_json::json!({
                "id":id,
                "parent":node.parent_element().map(|parent| mapping[&parent.id().get()]),
                "tag":node.tag_name().name(), "namespace":node.tag_name().namespace().unwrap_or(""),
                "attributes":node.attributes().map(|attr| (attr.name(), attr.value())).collect::<BTreeMap<_, _>>(),
                "children":node.children().filter(|node| node.is_element()).map(|node| mapping[&node.id().get()]).collect::<Vec<_>>()
            }));
        }
        let input: oryn_layout::LayoutInput = serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":["#shape{fill:var(--paint)}"],"viewport":{"width":20,"height":20,"device_pixel_ratio":1},"scroll":{"x":0,"y":0,"content_width":0,"content_height":0}})).unwrap();
        let projection = capture_svg_instances(
            source,
            &source_ids,
            &input,
            &oryn_layout::LayoutLimits::default(),
        )
        .unwrap();
        let named = |name| {
            xml.descendants()
                .find(|node| node.attribute("id") == Some(name))
                .unwrap()
        };
        let nested = named("nested");
        let ids = nested
            .descendants()
            .filter(|node| node.is_element())
            .map(|node| mapping[&node.id().get()])
            .collect::<Vec<_>>();
        let inner = mapping[&named("inner").id().get()];
        let shape = mapping[&named("shape").id().get()];
        for (owner, expected) in [("red", "rgb(255, 0, 0)"), ("blue", "rgb(0, 0, 255)")] {
            let local = projection
                .for_subtree(&ids, &[mapping[&named(owner).id().get()]])
                .unwrap();
            let key = resvg::usvg::NativeInstanceKey {
                ancestry: vec![inner],
                source: shape,
            };
            assert_eq!(local.computed.len(), 1);
            assert_eq!(local.computed[&key]["fill"], expected);
            assert!(local.presentation[&key].contains(expected));
            let again = local.for_subtree(&ids, &[]).unwrap();
            assert_eq!(again.computed[&key]["fill"], expected);
        }
        assert!(
            projection
                .for_subtree(&ids, &[u64::MAX])
                .unwrap()
                .computed
                .is_empty()
        );
        assert!(projection.for_subtree(&[shape, shape], &[]).is_err());
    }

    #[test]
    fn nested_instance_paths_and_parent_links_distinguish_repeated_references() {
        let xml = roxmltree::Document::parse(r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="target"><rect/></g><use id="nested" href="#target"/><use id="first" href="#nested"/><use id="second" href="#nested"/></svg>"##).unwrap();
        let ids = ids(&xml);
        let mut graph = graph(&xml, &ids, 100);
        let nested = graph.references["nested"];
        let target = graph.references["target"];
        let nested_id = ids[&nested.id().get()];
        for name in ["first", "second"] {
            let source = graph.references[name];
            let source_id = ids[&source.id().get()];
            let first = graph.requests.len();
            graph
                .walk(
                    nested,
                    StyleParent::Source(source_id),
                    &[source_id],
                    &mut vec![source.id().get()],
                )
                .unwrap();
            assert_eq!(graph.requests.len(), first + 3);
            assert_eq!(graph.keys[first].ancestry, [source_id]);
            assert_eq!(graph.keys[first + 1].ancestry, [source_id, nested_id]);
            assert_eq!(graph.keys[first + 1].source, ids[&target.id().get()]);
            assert_eq!(graph.keys[first + 2].ancestry, [source_id, nested_id]);
            assert!(
                matches!(graph.requests[first + 1].parent, StyleParent::Instance(id) if id == graph.requests[first].id)
            );
            assert!(
                matches!(graph.requests[first + 2].parent, StyleParent::Instance(id) if id == graph.requests[first + 1].id)
            );
        }
    }

    #[test]
    fn recursive_missing_external_and_duplicate_references_preserve_parser_identity() {
        let xml = roxmltree::Document::parse(r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="target"><use href="#target"/></g><rect id="target"/><use id="self" href="#self"/><use id="missing" href="#absent"/><use id="external" href="other.svg#target"/></svg>"##).unwrap();
        let ids = ids(&xml);
        let mut graph = graph(&xml, &ids, 100);
        let target = graph.references["target"];
        assert_eq!(target.tag_name().name(), "g", "first duplicate id wins");
        graph
            .walk(target, StyleParent::Source(1), &[1], &mut Vec::new())
            .unwrap();
        assert_eq!(
            graph.requests.len(),
            2,
            "recursive edge must not expand again"
        );
        let own = graph.references["self"];
        let before = graph.requests.len();
        graph
            .walk(own, StyleParent::Source(1), &[1], &mut vec![own.id().get()])
            .unwrap();
        assert_eq!(graph.requests.len(), before);
        assert!(graph.reference(graph.references["missing"]).is_none());
        assert!(graph.reference(graph.references["external"]).is_none());
    }

    #[test]
    fn unsupported_reference_targets_do_not_enter_the_instance_graph() {
        let xml = roxmltree::Document::parse(r##"<svg xmlns="http://www.w3.org/2000/svg"><g xmlns="http://www.w3.org/1999/xhtml" id="html"><div><div/></div></g><unknown id="unknown"><g/></unknown><foreignObject id="foreign"><div xmlns="http://www.w3.org/1999/xhtml"/></foreignObject><use id="html-use" href="#html"/><use id="unknown-use" href="#unknown"/><use id="foreign-use" href="#foreign"/></svg>"##).unwrap();
        let ids = ids(&xml);
        let mut graph = graph(&xml, &ids, 2);
        assert!(graph.reference(graph.references["html-use"]).is_none());
        assert!(graph.reference(graph.references["unknown-use"]).is_none());
        let source = graph.references["foreign-use"];
        let target = graph
            .reference(source)
            .expect("foreignObject is an SVG reference target");
        let source_id = ids[&source.id().get()];
        graph
            .walk(
                target,
                StyleParent::Source(source_id),
                &[source_id],
                &mut vec![source.id().get()],
            )
            .unwrap();
        assert_eq!(
            graph.requests.len(),
            2,
            "HTML descendants still need instance styles inside foreignObject"
        );
    }

    #[test]
    fn expansion_enforces_node_and_depth_budgets_before_unbounded_recursion() {
        let xml = roxmltree::Document::parse("<svg><g><rect/></g></svg>").unwrap();
        let ids = ids(&xml);
        let mut limited = graph(&xml, &ids, 1);
        assert!(
            limited
                .walk(
                    xml.root_element(),
                    StyleParent::Source(1),
                    &[1],
                    &mut Vec::new()
                )
                .is_err()
        );
        assert_eq!(limited.requests.len(), 1);
        let text = format!(
            "<svg>{}<rect/>{}</svg>",
            "<g>".repeat(129),
            "</g>".repeat(129)
        );
        let deep = roxmltree::Document::parse(&text).unwrap();
        let deep_ids = self::ids(&deep);
        let mut limited = graph(&deep, &deep_ids, 1000);
        assert!(
            limited
                .walk(
                    deep.root_element(),
                    StyleParent::Source(1),
                    &[1],
                    &mut Vec::new()
                )
                .is_err()
        );
        assert_eq!(limited.requests.len(), 128);
    }
}
