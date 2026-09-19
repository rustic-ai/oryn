//! Document language is native response/pragma state, separate from lang attributes.
use super::*;
const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub fn content_language_header<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let values = values.into_iter().collect::<Vec<_>>();
    if values.len() != 1 || values[0].contains(',') {
        return None;
    }
    let value = values[0].trim_matches(|c: char| matches!(c, ' ' | '\t'));
    (!value.is_empty()).then(|| value.to_owned())
}
impl DomArena {
    pub fn document_default_language(&self, document: NodeId) -> Result<Option<&str>, DomError> {
        Ok(self
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|metadata| {
                metadata
                    .pragma_language
                    .as_deref()
                    .or(metadata.content_language.as_deref())
            }))
    }
    pub fn element_language(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        let document = self.node_document(node)?;
        let mut cursor = Some(node);
        let mut seen = BTreeSet::new();
        while let Some(node) = cursor {
            if !seen.insert(node) {
                break;
            }
            let record = self.get(node)?;
            if let Some(lang) = record
                .attributes
                .records()
                .find(|attribute| {
                    attribute.namespace.as_deref() == Some(XML) && attribute.local_name == "lang"
                })
                .map(|attribute| attribute.value.as_str())
                .or_else(|| record.attributes.get_null("lang").map(String::as_str))
            {
                return Ok(Some(lang));
            }
            cursor = record.parent;
        }
        document
            .map(|document| self.document_default_language(document))
            .transpose()
            .map(Option::flatten)
    }
    pub(crate) fn process_language_pragmas(&mut self, root: NodeId) -> Result<(), DomError> {
        // Insertion into the ordinary Document tree processes pragmas. Shadow
        // and template contents are inert; removal does not undo a processed pragma.
        let mut top = root;
        while let Some(parent) = self.get(top)?.parent {
            if matches!(self.get(top)?.kind, NodeKind::ShadowRoot { .. }) {
                return Ok(());
            }
            top = parent;
        }
        if !matches!(self.get(top)?.kind, NodeKind::Document)
            || !self
                .get(top)?
                .document_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.is_html)
        {
            return Ok(());
        }
        let mut pending = vec![root];
        let mut last_language = None;
        while let Some(node) = pending.pop() {
            let record = self.get(node)?;
            // A newly connected host may already own a populated shadow tree.
            // Do not enter it when processing ordinary-document insertion steps.
            if matches!(
                record.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::DocumentFragment
            ) {
                continue;
            }
            pending.extend(record.children.iter().rev().copied());
            if !matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="meta")
            {
                continue;
            }
            if !record
                .attributes
                .get_null("http-equiv")
                .is_some_and(|value| value.eq_ignore_ascii_case("content-language"))
            {
                continue;
            }
            let Some(content) = record.attributes.get_null("content") else {
                continue;
            };
            if content.contains(',') {
                continue;
            }
            let Some(language) = content
                .trim_start_matches([' ', '\t', '\n', '\r', '\x0c'])
                .split([' ', '\t', '\n', '\r', '\x0c'])
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };
            last_language = Some(language);
        }
        if let Some(language) = last_language
            && let Some(metadata) = self.get_mut(top)?.document_metadata.as_mut()
        {
            metadata.pragma_language = Some(language);
        }
        Ok(())
    }
}
impl DomRuntime {
    pub fn element_language(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        self.arena.element_language(node)
    }
    pub fn set_document_content_language(
        &mut self,
        document: NodeId,
        language: Option<String>,
    ) -> Result<(), DomError> {
        let metadata = self
            .arena
            .get_mut(document)?
            .document_metadata
            .as_mut()
            .ok_or(DomError::StaleNode(document))?;
        if metadata.content_language != language {
            metadata.content_language = language;
            self.record_mutation(Mutation::FormState { node: document });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_and_pragma_language_precedence_empty_and_processing() {
        assert_eq!(content_language_header(["  ko  "]), Some("ko".into()));
        assert_eq!(content_language_header(["ko,zh"]), None);
        assert_eq!(content_language_header(["ko", "ko"]), None);
        let mut dom=html::parse("<meta id=m http-equiv=Content-Language content=' de-DE '><body><input id=d type=date value=2024-02-29></body>").into_runtime().unwrap();
        let document = dom.document();
        let input = dom.find_by_id("d").unwrap();
        let meta = dom.find_by_id("m").unwrap();
        dom.set_native_ui_locale("en-US").unwrap();
        dom.set_document_content_language(document, Some("ko".into()))
            .unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some("de-DE"));
        assert_eq!(dom.temporal_display_value(input).unwrap(), "29.02.2024");
        dom.set_attribute(meta, "content", "fr, en").unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some("de-DE"));
        dom.set_attribute(meta, "content", "en-US ignored").unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some("en-US"));
        dom.remove(meta).unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some("en-US"));
        dom.set_attribute(input, "lang", "").unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some(""));
        assert_eq!(dom.temporal_language(input).unwrap(), "");
        dom.set_attribute_ns(input, Some(XML.into()), "xml:lang", "tr")
            .unwrap();
        assert_eq!(dom.element_language(input).unwrap(), Some("tr"));
    }
    #[test]
    fn new_documents_and_inert_template_do_not_inherit_unrelated_defaults() {
        let dom = html::parse(
            "<template><meta http-equiv=Content-Language content=fr></template><span id=s></span>",
        )
        .into_runtime()
        .unwrap();
        assert_eq!(
            dom.element_language(dom.find_by_id("s").unwrap()).unwrap(),
            None
        );
        let mut first = html::parse("<span id=s></span>").into_runtime().unwrap();
        let root = first.document();
        first
            .set_document_content_language(root, Some("ko".into()))
            .unwrap();
        assert_eq!(
            first
                .element_language(first.find_by_id("s").unwrap())
                .unwrap(),
            Some("ko")
        );
        let other = html::parse("<span id=t></span>");
        let imported = first.import_subtree(&other.dom, other.document).unwrap();
        assert_eq!(
            first.arena.document_default_language(imported).unwrap(),
            None
        );
    }
    #[test]
    fn connecting_prebuilt_shadow_tree_does_not_process_its_language_meta() {
        let mut dom = html::parse("<body id=body></body>").into_runtime().unwrap();
        let document = dom.document();
        dom.set_document_content_language(document, Some("en".into()))
            .unwrap();
        let host = dom.create_element("div");
        dom.set_attribute(host, "lang", "fr").unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Open,
            clonable: false,
        });
        let meta = dom.create_element("meta");
        dom.set_attribute(meta, "http-equiv", "content-language")
            .unwrap();
        dom.set_attribute(meta, "content", "ja").unwrap();
        dom.append_child(shadow, meta).unwrap();
        dom.append_child(host, shadow).unwrap();
        let body = dom.find_by_id("body").unwrap();
        dom.append_child(body, host).unwrap();
        assert_eq!(
            dom.arena.document_default_language(document).unwrap(),
            Some("en")
        );
        assert_eq!(dom.element_language(meta).unwrap(), Some("fr"));
        dom.set_attribute(meta, "content", "ko").unwrap();
        assert_eq!(
            dom.arena.document_default_language(document).unwrap(),
            Some("en")
        );
        let ordinary = dom.create_element("meta");
        dom.set_attribute(ordinary, "http-equiv", "content-language")
            .unwrap();
        dom.set_attribute(ordinary, "content", "de").unwrap();
        dom.append_child(host, ordinary).unwrap();
        assert_eq!(
            dom.arena.document_default_language(document).unwrap(),
            Some("de")
        );
    }
}
