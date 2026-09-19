//! Document title selection is scoped to the receiver document, including SVG.
use super::*;
const HTML: &str = "http://www.w3.org/1999/xhtml";
const SVG: &str = "http://www.w3.org/2000/svg";
fn root(dom: &DomRuntime, document: NodeId) -> Result<Option<NodeId>, DomError> {
    Ok(dom.children(document)?.iter().copied().find(|id| {
        matches!(
            dom.arena().get(*id).map(|n| &n.kind),
            Ok(NodeKind::Element { .. })
        )
    }))
}
fn first_html(dom: &DomRuntime, root: NodeId, name: &str) -> Result<Option<NodeId>, DomError> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if dom.is_element(node, HTML, name) {
            return Ok(Some(node));
        }
        stack.extend(dom.children(node)?.iter().rev().copied());
    }
    Ok(None)
}
fn title(dom: &DomRuntime, root: NodeId) -> Result<Option<NodeId>, DomError> {
    if dom.is_element(root, SVG, "svg") {
        Ok(dom
            .children(root)?
            .iter()
            .copied()
            .find(|id| dom.is_element(*id, SVG, "title")))
    } else {
        first_html(dom, root, "title")
    }
}
pub(super) fn get(dom: &DomRuntime, document: NodeId) -> Result<String, DomError> {
    let Some(root) = root(dom, document)? else {
        return Ok(String::new());
    };
    let Some(title) = title(dom, root)? else {
        return Ok(String::new());
    };
    let mut units = Vec::new();
    for child in dom.children(title)? {
        if let NodeKind::Text { data } | NodeKind::CDataSection { data } =
            &dom.arena().get(*child)?.kind
        {
            units.extend(oryn_dom::dom_string::units(data));
        }
    }
    let mut result = Vec::new();
    let mut space = false;
    for unit in units {
        if matches!(unit, 9 | 10 | 12 | 13 | 32) {
            space = !result.is_empty();
        } else {
            if space {
                result.push(32);
                space = false;
            }
            result.push(unit);
        }
    }
    Ok(oryn_dom::dom_string::from_units(&result))
}
pub(super) fn assign(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let original = wrap_node(scope, state, document)?;
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let value = rust_string_preserving_utf16(scope, value)?;
    let (owner, document) =
        node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(document))?;
    set(&mut owner.dom.borrow_mut(), document, value)?;
    Ok(())
}
fn set(dom: &mut DomRuntime, document: NodeId, value: String) -> Result<(), DomError> {
    let Some(root) = root(dom, document)? else {
        return Ok(());
    };
    let node = if let Some(node) = title(dom, root)? {
        node
    } else if dom.is_element(root, SVG, "svg") {
        let node = dom.create_element_ns(SVG, "title");
        let first = dom.children(root)?.first().copied();
        dom.insert_before(root, node, first)?;
        node
    } else {
        // A missing head is not synthesized by the title setter.
        let Some(head) = dom
            .children(root)?
            .iter()
            .copied()
            .find(|id| dom.is_element(*id, HTML, "head"))
        else {
            return Ok(());
        };
        let node = dom.create_element("title");
        dom.append_child(head, node)?;
        node
    };
    dom.set_text_content(node, value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_selection_whitespace_and_utf16() {
        let mut dom = oryn_dom::html::parse("<title> a\n b </title><body><title>second</title>")
            .into_runtime()
            .unwrap();
        let document = dom.document();
        assert_eq!(get(&dom, document).unwrap(), "a b");
        let title_node = title(&dom, root(&dom, document).unwrap().unwrap())
            .unwrap()
            .unwrap();
        let nested = dom.create_element("b");
        let text = dom.create_text("excluded");
        dom.append_child(nested, text).unwrap();
        dom.append_child(title_node, nested).unwrap();
        assert_eq!(
            get(&dom, document).unwrap(),
            "a b",
            "title collects child text only"
        );
        let value = oryn_dom::dom_string::from_units(&[32, 0xd800, 9, 120, 32]);
        set(&mut dom, document, value).unwrap();
        assert_eq!(
            oryn_dom::dom_string::units(&get(&dom, document).unwrap()),
            vec![0xd800, 32, 120]
        );
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let svg = dom.create_element_ns(SVG, "svg");
        dom.append_child(document, svg).unwrap();
        let child = dom.create_element_ns(SVG, "g");
        dom.append_child(svg, child).unwrap();
        set(&mut dom, document, " SVG  title ".into()).unwrap();
        assert_eq!(get(&dom, document).unwrap(), "SVG title");
        assert!(dom.is_element(dom.children(svg).unwrap()[0], SVG, "title"));
        let title_node = dom.children(svg).unwrap()[0];
        let nested = dom.create_element_ns(SVG, "tspan");
        let text = dom.create_text("excluded");
        dom.append_child(nested, text).unwrap();
        dom.append_child(title_node, nested).unwrap();
        assert_eq!(get(&dom, document).unwrap(), "SVG title");
    }
}
