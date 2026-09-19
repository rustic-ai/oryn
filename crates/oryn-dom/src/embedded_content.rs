//! Native HTML embedded-resource ownership. Response MIME, not a URL suffix,
//! chooses a child Document or decoded image for object/embed resources.
use crate::{DomRuntime, NodeId};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedContentKind {
    Iframe,
    Frame,
    Object,
    Embed,
}
impl EmbeddedContentKind {
    pub fn source_attribute(self) -> &'static str {
        if self == Self::Object { "data" } else { "src" }
    }
    pub fn resource_destination(self) -> &'static str {
        match self {
            Self::Iframe | Self::Frame => "frame",
            Self::Object => "object",
            Self::Embed => "embed",
        }
    }
    /// Object disables only an absent/exactly empty data attribute; embed
    /// distinguishes an absent src from an explicitly empty URL.
    pub fn loads_source(self, source: Option<&str>) -> bool {
        match self {
            Self::Object => source.is_some_and(|value| !value.is_empty()),
            Self::Embed => source.is_some(),
            Self::Iframe | Self::Frame => source.is_some_and(|value| !value.trim().is_empty()),
        }
    }
    pub fn creates_initial_document(self) -> bool {
        matches!(self, Self::Iframe | Self::Frame)
    }
    pub fn supports_srcdoc(self) -> bool {
        self == Self::Iframe
    }
    pub fn selects_response_representation(self) -> bool {
        matches!(self, Self::Object | Self::Embed)
    }
}
impl DomRuntime {
    /// Media fallback is inert; descendants of an object start only once its
    /// resource algorithm selected fallback, including an absent data URL.
    pub fn embedded_content_allowed(&self, node: NodeId) -> bool {
        let mut parent = self.parent(node).ok().flatten();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(ancestor) = parent {
            if !seen.insert(ancestor) {
                return false;
            }
            if self.is_element(ancestor, "http://www.w3.org/1999/xhtml", "audio")
                || self.is_element(ancestor, "http://www.w3.org/1999/xhtml", "video")
            {
                return false;
            }
            if self.is_element(ancestor, "http://www.w3.org/1999/xhtml", "object")
                && self
                    .attribute_in_null_namespace(ancestor, "data")
                    .ok()
                    .flatten()
                    .is_some_and(|value| !value.is_empty())
                && !self
                    .arena()
                    .get(ancestor)
                    .is_ok_and(|record| record.embedded_load_failed)
            {
                return false;
            }
            parent = self.parent(ancestor).ok().flatten();
        }
        true
    }
    pub fn set_embedded_load_failed(
        &mut self,
        node: NodeId,
        failed: bool,
    ) -> Result<(), crate::DomError> {
        self.arena_mut().get_mut(node)?.embedded_load_failed = failed;
        Ok(())
    }
    pub fn embedded_content_kind(&self, node: NodeId) -> Option<EmbeddedContentKind> {
        [
            ("iframe", EmbeddedContentKind::Iframe),
            ("frame", EmbeddedContentKind::Frame),
            ("object", EmbeddedContentKind::Object),
            ("embed", EmbeddedContentKind::Embed),
        ]
        .into_iter()
        .find_map(|(tag, kind)| {
            self.is_element(node, "http://www.w3.org/1999/xhtml", tag)
                .then_some(kind)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_whitespace_sources_keep_element_specific_semantics() {
        assert!(!EmbeddedContentKind::Object.loads_source(None));
        assert!(!EmbeddedContentKind::Object.loads_source(Some("")));
        assert!(EmbeddedContentKind::Object.loads_source(Some(" 	")));
        assert!(!EmbeddedContentKind::Embed.loads_source(None));
        assert!(EmbeddedContentKind::Embed.loads_source(Some("")));
        assert!(EmbeddedContentKind::Embed.loads_source(Some(" ")));
        let parsed = crate::html::parse(
            "<object data=' '><embed id=blocked src='child'></object><object data=''></object><embed src=''><embed>",
        );
        let sources = parsed
            .events
            .iter()
            .filter_map(|event| {
                if let crate::html::ParserEvent::FrameDiscovered { source, .. } = event {
                    Some(source.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(sources.contains(&" "));
        assert!(sources.contains(&""));
        let dom = parsed.into_runtime().unwrap();
        assert!(!dom.embedded_content_allowed(dom.find_by_id("blocked").unwrap()));
    }
    #[test]
    fn source_and_initial_document_are_native_tag_properties() {
        let mut dom = crate::html::parse("<body></body>").into_runtime().unwrap();
        for (tag, attribute, initial, destination) in [
            ("iframe", "src", true, "frame"),
            ("frame", "src", true, "frame"),
            ("object", "data", false, "object"),
            ("embed", "src", false, "embed"),
        ] {
            let node = dom.create_element(tag);
            let kind = dom.embedded_content_kind(node).unwrap();
            assert_eq!(kind.source_attribute(), attribute);
            assert_eq!(kind.creates_initial_document(), initial);
            assert_eq!(kind.resource_destination(), destination);
        }
        let node = dom.create_element("img");
        assert_eq!(dom.embedded_content_kind(node), None);
    }
    #[test]
    fn parser_schedules_embedded_sources_without_fabricating_empty_object_documents() {
        let parsed = crate::html::parse(
            "<iframe src='/frame'></iframe><object data='/object'></object><embed src='/embed'><object></object><embed>",
        );
        let resources = parsed
            .events
            .iter()
            .filter_map(|event| {
                if let crate::html::ParserEvent::FrameDiscovered { source, .. } = event {
                    Some(source.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(resources, vec!["/frame", "/object", "/embed"]);
        assert!(
            !parsed
                .events
                .iter()
                .any(|event| matches!(event, crate::html::ParserEvent::FrameLocalDocument { .. }))
        );
    }

    #[test]
    fn resource_fallback_and_media_ancestors_control_child_admission() {
        let mut dom=crate::html::parse("<object id=parent data='/parent'><embed id=child src='/child'></object><object><embed id=empty src='/empty'></object><video><object id=media data='/no'></object></video>").into_runtime().unwrap();
        let parent = dom.find_by_id("parent").unwrap();
        let child = dom.find_by_id("child").unwrap();
        assert!(!dom.embedded_content_allowed(child));
        assert!(dom.embedded_content_allowed(dom.find_by_id("empty").unwrap()));
        assert!(!dom.embedded_content_allowed(dom.find_by_id("media").unwrap()));
        dom.set_embedded_load_failed(parent, true).unwrap();
        assert!(dom.embedded_content_allowed(child));
        dom.set_embedded_load_failed(parent, false).unwrap();
        assert!(!dom.embedded_content_allowed(child));
        dom.remove_attribute(parent, "data").unwrap();
        assert!(dom.embedded_content_allowed(child));
    }

    #[test]
    fn embedded_source_and_type_aba_cannot_restore_accepted_navigation() {
        let mut dom = crate::html::parse("<body></body>").into_runtime().unwrap();
        let body = dom.query_selector(dom.document(), "body").unwrap().unwrap();
        for (tag, attribute) in [
            ("object", "data"),
            ("object", "type"),
            ("embed", "src"),
            ("embed", "type"),
            ("iframe", "src"),
            ("iframe", "srcdoc"),
            ("iframe", "sandbox"),
        ] {
            let node = dom.create_element(tag);
            dom.append_child(body, node).unwrap();
            dom.set_attribute(node, attribute, "original").unwrap();
            let accepted = dom.frame_navigation_identity(node);
            dom.set_attribute(node, attribute, "replacement").unwrap();
            let replacement = dom.frame_navigation_identity(node);
            assert!(
                !dom.frame_navigation_identity_is_current(node, &accepted),
                "{tag}.{attribute} change revokes old receipt"
            );
            dom.set_attribute(node, attribute, "original").unwrap();
            assert_eq!(
                dom.attribute_in_null_namespace(node, attribute).unwrap(),
                Some("original")
            );
            assert!(
                !dom.frame_navigation_identity_is_current(node, &accepted),
                "{tag}.{attribute} ABA must not resurrect receipt"
            );
            assert!(!dom.frame_navigation_identity_is_current(node, &replacement));
            let accepted = dom.frame_navigation_identity(node);
            dom.remove_attribute(node, attribute).unwrap();
            dom.set_attribute(node, attribute, "original").unwrap();
            assert!(
                !dom.frame_navigation_identity_is_current(node, &accepted),
                "removal and restoration also revoke"
            );
        }
        for (tag, attribute) in [
            ("iframe", "type"),
            ("iframe", "data"),
            ("embed", "data"),
            ("input", "type"),
        ] {
            let node = dom.create_element(tag);
            dom.append_child(body, node).unwrap();
            let accepted = dom.frame_navigation_identity(node);
            dom.set_attribute(node, attribute, "unrelated").unwrap();
            assert!(
                dom.frame_navigation_identity_is_current(node, &accepted),
                "{tag}.{attribute} is not an embedded representation selector"
            );
        }
        let node = dom.create_element("object");
        dom.append_child(body, node).unwrap();
        let accepted = dom.frame_navigation_identity(node);
        dom.set_attribute_ns(node, Some("urn:test".into()), "x:type", "text/html")
            .unwrap();
        assert!(
            dom.frame_navigation_identity_is_current(node, &accepted),
            "namespaced type does not select the resource"
        );
    }
}
