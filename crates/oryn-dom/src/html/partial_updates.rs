//! Parser-owned partial HTML updates. Marker recognition uses PI node identity,
//! never comment text. A fragment's search root is explicit and cannot escape
//! into the destination document before publication.
use super::*;
const HTML: &str = "http://www.w3.org/1999/xhtml";
fn parameter(data: &str, name: &str) -> Option<String> {
    let transport = dom_input::encode(data);
    let mut rest = transport.as_str();
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(rest.len());
        if end == 0 {
            rest = &rest[rest.chars().next()?.len_utf8()..];
            continue;
        }
        let key = &rest[..end];
        rest = rest[end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        if !rest.starts_with('=') {
            continue;
        }
        rest = rest[1..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        let first = rest.chars().next()?;
        let value = if first == '\'' || first == '"' {
            rest = &rest[1..];
            let end = rest.find(first)?;
            let value = &rest[..end];
            rest = &rest[end + 1..];
            value
        } else {
            let end = rest
                .find(|c: char| c.is_ascii_whitespace())
                .unwrap_or(rest.len());
            let value = &rest[..end];
            rest = &rest[end..];
            value
        };
        if key == name {
            return Some(dom_input::decode(value));
        }
    }
    None
}
fn visible_nodes(dom: &DomRuntime, root: NodeId) -> Result<Vec<NodeId>, DomError> {
    let mut pending = vec![root];
    let mut nodes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            return Err(DomError::Cycle);
        }
        let record = dom.arena().get(node)?;
        nodes.push(node);
        // Ordinary tree scope only: template contents, shadow and hosted
        // documents each have their own independently scoped patch registry.
        if node != root
            && matches!(
                record.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            )
        {
            continue;
        }
        pending.extend(record.children.iter().rev().copied());
    }
    Ok(nodes)
}
enum Step {
    Remove(Box<crate::insertion::TreeRemoval>),
    Insert(Box<crate::insertion::TreeInsertion>),
}
pub(super) struct PatchCursor {
    roots: crate::insertion::CursorRoots,
    steps: Vec<Step>,
    index: usize,
    published: Vec<NodeId>,
    scope: NodeId,
    nested: Option<Box<PatchCursor>>,
    attempted: std::collections::BTreeSet<NodeId>,
}
impl PatchCursor {
    pub(super) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
    ) -> Result<Option<Self>, DomError> {
        Self::admit(owner, dom, scope, template, None, None)
    }
    pub(super) fn external_content(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
        content: NodeId,
        name: &str,
    ) -> Result<Option<Self>, DomError> {
        Self::admit(Some(owner), dom, scope, template, Some(content), Some(name))
    }
    fn admit(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
        external_content: Option<NodeId>,
        target_name: Option<&str>,
    ) -> Result<Option<Self>, DomError> {
        if !dom.is_element(template, HTML, "template")
            || external_content.is_none()
                && dom.attribute_in_null_namespace(template, "src")?.is_some()
        {
            return Ok(None);
        }
        let Some(name) = target_name
            .or(dom.attribute_in_null_namespace(template, "for")?)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        let mut found = None;
        for node in visible_nodes(dom, scope)? {
            if node == template {
                break;
            }
            if let NodeKind::ProcessingInstruction { target, data } = &dom.arena().get(node)?.kind {
                if target != "start" && target != "marker" {
                    continue;
                }
                let Some(parent) = dom.parent(node)? else {
                    continue;
                };
                let marker = parameter(data, "name").or_else(|| {
                    dom.attribute_in_null_namespace(parent, "marker")
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                });
                if marker.as_deref() == Some(name.as_str()) {
                    found = Some((node, parent, target == "start"));
                    break;
                }
            }
        }
        let Some((start, parent, range)) = found else {
            return Ok(None);
        };
        let children = dom.children(parent)?.to_vec();
        let begin = children
            .iter()
            .position(|node| *node == start)
            .ok_or(DomError::NotAChild(start, parent))?;
        let mut end = begin;
        let mut depth = 0;
        if range {
            end = children.len().saturating_sub(1);
            for (index, node) in children.iter().enumerate().skip(begin + 1) {
                if let NodeKind::ProcessingInstruction { target, .. } =
                    &dom.arena().get(*node)?.kind
                {
                    if target == "start" {
                        depth += 1;
                    } else if target == "end" {
                        if depth == 0 {
                            end = index;
                            break;
                        }
                        depth -= 1;
                    }
                }
            }
        }
        let before = children.get(end + 1).copied();
        let content = external_content
            .or(dom.arena().get(template)?.template_content)
            .ok_or(DomError::NotAnElement(template))?;
        if external_content.is_none()
            && dom
                .attribute_in_null_namespace(template, "sanitize")?
                .is_some_and(|value| value != "unsafe")
        {
            crate::sanitizer::Config::default_preset()
                .filter_tree_safe(dom, content)
                .map_err(|error| match error {
                    crate::sanitizer::SanitizeError::Dom(error) => error,
                    _ => DomError::MigrationChanged,
                })?;
        }
        let added = dom.children(content)?.to_vec();
        let removed = children[begin..=end].to_vec();
        let mut cursor = Self {
            roots: crate::insertion::CursorRoots::new(owner, dom),
            steps: Vec::new(),
            index: 0,
            published: added.clone(),
            scope,
            nested: None,
            attempted: std::collections::BTreeSet::new(),
        };
        let prepare = (|| -> Result<(), DomError> {
            for node in removed
                .iter()
                .chain(added.iter())
                .copied()
                .chain([scope, parent, template, content])
                .chain(before)
            {
                cursor.roots.retain(dom, node)?;
            }
            // Build removal receipts at admission. Insert receipts are admitted
            // only after removals, because their saved reference is the tail.
            for node in removed {
                cursor
                    .steps
                    .push(Step::Remove(Box::new(crate::insertion::TreeRemoval::new(
                        owner, dom, node,
                    )?)));
            }
            for node in added {
                cursor.steps.push(Step::Insert(Box::new(
                    crate::insertion::TreeInsertion::new(owner, dom, parent, node, before, false)?,
                )));
            }
            cursor
                .steps
                .push(Step::Remove(Box::new(crate::insertion::TreeRemoval::new(
                    owner, dom, template,
                )?)));
            Ok(())
        })();
        if let Err(error) = prepare {
            cursor.abort(dom);
            return Err(error);
        }
        Ok(Some(cursor))
    }
    pub(super) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        while self.index < self.steps.len() {
            match &mut self.steps[self.index] {
                Step::Remove(step) => step.resume(dom)?,
                Step::Insert(step) => step.resume(dom)?,
            };
            self.index += 1;
        }
        loop {
            if let Some(nested) = self.nested.as_mut() {
                nested.resume(dom)?;
                self.published.extend_from_slice(nested.published());
                nested.abort(dom);
                self.nested = None;
            }
            let mut candidates = Vec::new();
            for root in &self.published {
                if dom.arena().get(*root).is_ok() {
                    candidates.extend(visible_nodes(dom, *root)?);
                }
            }
            let candidate = candidates.into_iter().find(|node| {
                !self.attempted.contains(node) && dom.is_element(*node, HTML, "template")
            });
            let Some(template) = candidate else {
                return Ok(());
            };
            self.attempted.insert(template);
            self.nested = Self::new(self.roots.owner(), dom, self.scope, template)?.map(Box::new);
        }
    }
    pub(super) fn published(&self) -> &[NodeId] {
        &self.published
    }
    pub(super) fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(nested) = self.nested.as_mut() {
            nested.abort(dom);
        }
        for step in &mut self.steps {
            match step {
                Step::Remove(step) => step.abort(dom),
                Step::Insert(step) => step.abort(dom),
            }
        }
        let _ = self.roots.release(dom);
    }
}
/// Apply only templates and markers inside the admitted fragment. Call after
/// sanitizer filtering, before importing nodes into the destination document.
pub fn apply_fragment_patches(dom: &mut DomRuntime, root: NodeId) -> Result<(), DomError> {
    let mut scopes = vec![root];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(scope) = scopes.pop() {
        if !visited.insert(scope) {
            continue;
        }
        let mut attempted = std::collections::BTreeSet::new();
        loop {
            let candidates = visible_nodes(dom, scope)?;
            let Some(template) = candidates
                .into_iter()
                .find(|node| !attempted.contains(node) && dom.is_element(*node, HTML, "template"))
            else {
                break;
            };
            attempted.insert(template);
            if let Some(mut patch) = PatchCursor::new(None, dom, scope, template)? {
                let result = patch.resume(dom);
                patch.abort(dom);
                result?;
            }
        }
        for node in visible_nodes(dom, scope)? {
            let record = dom.arena().get(node)?;
            if let Some(content) = record.template_content {
                scopes.push(content);
            }
            for child in &record.children {
                if matches!(dom.arena().get(*child)?.kind, NodeKind::ShadowRoot { .. }) {
                    scopes.push(*child);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragment_markers_are_pi_nodes_and_comments_cannot_impersonate_them() {
        let parsed = parse_fragment_dom_string(
            "<div id=a><!--?start name='p'-->old</div><template for=p>wrong</template><div id=b><?start name='q'>old<?end></div><template for=q><b>new</b></template>",
            "div",
        );
        let dom = parsed.into_runtime().unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        assert_eq!(dom.arena().text_content(a).unwrap(), "old");
        assert_eq!(dom.arena().text_content(b).unwrap(), "new");
        assert!(dom.children(b).unwrap().iter().all(|node| !matches!(
            dom.arena().get(*node).unwrap().kind,
            NodeKind::ProcessingInstruction { .. }
        )));
    }
    #[test]
    fn marker_names_targets_and_attributes_are_case_sensitive() {
        let dom=parse_inert_dom_string("<div id=p><?Start name=a><?marker Name=b><?marker name=C><?marker name=d></div><template for=a>A</template><template for=b>B</template><template for=c>C</template><template for=C>yes</template><template for=d>tail</template>").into_runtime().unwrap();
        assert_eq!(
            dom.arena()
                .text_content(dom.find_by_id("p").unwrap())
                .unwrap(),
            "yestail"
        );
    }
    #[test]
    fn scoped_fragment_does_not_patch_external_live_markers() {
        let mut live = parse("<div id=outside><?start name=p>old<?end></div>")
            .into_runtime()
            .unwrap();
        let parsed=parse_fragment_dom_string("<template for=p>wrong</template><div id=inside><?start name=q>old</div><template for=q>new</template>","div").into_runtime().unwrap();
        let root = parsed.children(parsed.document()).unwrap()[0];
        let parent = live.query_selector_all(live.document(), "body").unwrap()[0];
        let imported = live
            .import_fragment_children(parent, &parsed, root)
            .unwrap();
        for child in imported {
            live.append_child(parent, child).unwrap();
        }
        assert_eq!(
            live.arena()
                .text_content(live.find_by_id("outside").unwrap())
                .unwrap(),
            "old"
        );
        assert_eq!(
            parsed
                .arena()
                .text_content(parsed.find_by_id("inside").unwrap())
                .unwrap(),
            "new"
        );
    }
    #[test]
    fn every_patch_native_failure_retains_one_move_and_template_identity() {
        for fail in 0..80 {
            let mut dom=parse_inert_dom_string_with_declarative_shadow_unpatched("<div id=p><?start name=p>old<?end></div><template id=t for=p><span id=n>new</span></template>").into_runtime().unwrap();
            let root = dom.document();
            let template = dom.find_by_id("t").unwrap();
            let content = dom.arena().get(template).unwrap().template_content.unwrap();
            let child = dom.children(content).unwrap()[0];
            let mut patch = PatchCursor::new(None, &mut dom, root, template)
                .unwrap()
                .unwrap();
            crate::insertion::set_failure_after(Some(fail));
            let first = patch.resume(&mut dom);
            crate::insertion::set_failure_after(None);
            if first.is_err() {
                patch.resume(&mut dom).unwrap();
            }
            let parent = dom.find_by_id("p").unwrap();
            assert_eq!(dom.children(parent).unwrap(), &[child]);
            assert_eq!(dom.arena().text_content(parent).unwrap(), "new");
            patch.abort(&mut dom);
        }
    }
}

/// Unbuffered template routing keeps marker cleanup separate from the initial
/// range removal. The tokenizer writes directly at the retained insertion
/// boundary, so scripts and custom elements can observe each accepted prefix.
pub(super) struct StreamingPatch {
    roots: crate::insertion::CursorRoots,
    pub(super) parent: NodeId,
    pub(super) before: Option<NodeId>,
    remove: Vec<crate::insertion::TreeRemoval>,
    remove_index: usize,
    cleanup: Vec<NodeId>,
}
impl StreamingPatch {
    pub(super) fn new(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
    ) -> Result<Option<Self>, DomError> {
        Self::admit(owner, dom, scope, template, false, None)
    }
    pub(super) fn external_named(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
        name: &str,
    ) -> Result<Option<Self>, DomError> {
        Self::admit(owner, dom, scope, template, true, Some(name))
    }
    fn admit(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        scope: NodeId,
        template: NodeId,
        external: bool,
        captured_name: Option<&str>,
    ) -> Result<Option<Self>, DomError> {
        if !external
            && (dom
                .attribute_in_null_namespace(template, "buffer")?
                .is_some()
                || dom
                    .attribute_in_null_namespace(template, "sanitize")?
                    .is_some_and(|value| value != "unsafe"))
        {
            return Ok(None);
        }
        let Some(name) = captured_name
            .or(dom.attribute_in_null_namespace(template, "for")?)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        let mut found = None;
        for node in visible_nodes(dom, scope)? {
            if node == template {
                break;
            }
            if let NodeKind::ProcessingInstruction { target, data } = &dom.arena().get(node)?.kind {
                if target != "start" && target != "marker" {
                    continue;
                }
                let Some(parent) = dom.parent(node)? else {
                    continue;
                };
                let marker = parameter(data, "name").or_else(|| {
                    dom.attribute_in_null_namespace(parent, "marker")
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                });
                if marker.as_deref() == Some(name.as_str()) {
                    found = Some((node, parent, target == "start"));
                    break;
                }
            }
        }
        let Some((start, parent, range)) = found else {
            return Ok(None);
        };
        let children = dom.children(parent)?.to_vec();
        let begin = children
            .iter()
            .position(|node| *node == start)
            .ok_or(DomError::NotAChild(start, parent))?;
        let mut end = None;
        let mut depth = 0;
        if range {
            for (index, node) in children.iter().enumerate().skip(begin + 1) {
                if let NodeKind::ProcessingInstruction { target, .. } =
                    &dom.arena().get(*node)?.kind
                {
                    if target == "start" {
                        depth += 1;
                    } else if target == "end" {
                        if depth == 0 {
                            end = Some(index);
                            break;
                        }
                        depth -= 1;
                    }
                }
            }
        }
        let before = if range {
            end.map(|index| children[index])
        } else {
            Some(start)
        };
        let removed = if range {
            children[begin + 1..end.unwrap_or(children.len())].to_vec()
        } else {
            Vec::new()
        };
        let mut cleanup = vec![start];
        if let Some(index) = end {
            cleanup.push(children[index]);
        }
        cleanup.push(template);
        let mut cursor = Self {
            roots: crate::insertion::CursorRoots::new(Some(owner), dom),
            parent,
            before,
            remove: Vec::new(),
            remove_index: 0,
            cleanup,
        };
        let result = (|| -> Result<(), DomError> {
            for node in removed
                .iter()
                .chain(cursor.cleanup.iter())
                .copied()
                .chain([parent, scope])
            {
                cursor.roots.retain(dom, node)?;
            }
            for node in removed {
                cursor
                    .remove
                    .push(crate::insertion::TreeRemoval::new(Some(owner), dom, node)?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            cursor.abort(dom);
            return Err(error);
        }
        Ok(Some(cursor))
    }
    pub(super) fn cleanup_nodes(&self) -> &[NodeId] {
        &self.cleanup
    }
    pub(super) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        while self.remove_index < self.remove.len() {
            self.remove[self.remove_index].resume(dom)?;
            self.remove_index += 1;
        }
        Ok(())
    }
    pub(super) fn abort(&mut self, dom: &mut DomRuntime) {
        for removal in &mut self.remove {
            removal.abort(dom);
        }
        let _ = self.roots.release(dom);
    }
}

#[cfg(test)]
mod sanitizer_order_tests {
    use super::*;
    #[test]
    fn outer_safe_template_filters_nested_patch_and_custom_constructor_before_publication() {
        let dom = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let mut parser = StreamingParser::with_runtime(dom.clone());
        let document = dom.borrow().document();
        let mut constructors = 0;
        {
            let mut callback = |action| match action {
                ParserAction::Construct { target, .. } => {
                    constructors += 1;
                    Some(target)
                }
                ParserAction::Checkpoint => Some(ParserNode {
                    owner: dom.clone(),
                    node: document,
                }),
                ParserAction::Adopt { target, .. } => Some(target),
                ParserAction::Failed(_) => None,
            };
            let input = ParserInput::unicode(
                "<body><div id=target><?start name=outer>old<?end></div><template for=outer sanitize><div><?start name=inner>original<?end></div><template for=inner sanitize=unsafe><span>wrong</span></template><x-forbidden></x-forbidden></template>",
            );
            let progress = with_parser_actions(&dom, &mut callback, || {
                parser.push_input_until_resource_or_script(&input)
            });
            assert_eq!(progress.consumed_bytes, input.len());
        }
        assert_eq!(constructors, 0);
        assert!(parser.prepare_finish());
        let target = dom.borrow().find_by_id("target").unwrap();
        assert_eq!(
            dom.borrow().arena().text_content(target).unwrap(),
            "original"
        );
    }
}

#[cfg(test)]
mod marker_domstring_tests {
    use super::*;
    #[test]
    fn marker_pseudo_attribute_comparison_preserves_lone_utf16_units() {
        let mut units = "<div id=p><?start name='"
            .encode_utf16()
            .collect::<Vec<_>>();
        units.push(0xd800);
        units.extend("'>old<?end></div><template for='".encode_utf16());
        units.push(0xd800);
        units.extend("'>new</template>".encode_utf16());
        let dom = parse_inert_dom_string(&crate::dom_string::from_units(&units))
            .into_runtime()
            .unwrap();
        assert_eq!(
            dom.arena()
                .text_content(dom.find_by_id("p").unwrap())
                .unwrap(),
            "new"
        );
    }
}
