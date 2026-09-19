//! Document-native rendering resource membership and internal expect links.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

#[derive(Clone, Debug)]
pub struct RenderBlocker(Arc<AtomicBool>);
impl RenderBlocker {
    pub fn release(&self) {
        self.0.store(true, Ordering::Release);
    }
}
#[derive(Debug)]
struct Resource {
    document: NodeId,
    implicit: bool,
    revision: Option<u64>,
    pending: Weak<AtomicBool>,
}
type Media = Rc<dyn Fn(&str, &str, bool) -> bool>;
#[derive(Default)]
pub(crate) struct State {
    resources: BTreeMap<NodeId, Resource>,
    expects: BTreeMap<NodeId, NodeId>,
    media: BTreeMap<NodeId, Media>,
}
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderBlocking")
            .field("resources", &self.resources)
            .field("expects", &self.expects)
            .finish()
    }
}
fn token(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| {
        value
            .split(['\t', '\n', '\u{c}', '\r', ' '])
            .any(|part| part == expected)
    })
}
fn decode_fragment(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let digit = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            };
            if let (Some(a), Some(b)) = (digit(bytes[index + 1]), digit(bytes[index + 2])) {
                out.push(a * 16 + b);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
impl DomRuntime {
    pub fn allows_render_blockers(&self, document: NodeId) -> bool {
        let Ok(record) = self.arena.get(document) else {
            return false;
        };
        if !record
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.content_type == "text/html")
        {
            return false;
        }
        let html = record
            .children
            .iter()
            .find(|node| self.is_element(**node, "http://www.w3.org/1999/xhtml", "html"));
        html.and_then(|html| self.arena.get(*html).ok())
            .is_none_or(|root| {
                !root.children.iter().any(|node| {
                    self.is_element(*node, "http://www.w3.org/1999/xhtml", "body")
                        || self.is_element(*node, "http://www.w3.org/1999/xhtml", "frameset")
                })
            })
    }
    pub fn set_render_media_evaluator(&mut self, document: NodeId, evaluator: Media) {
        self.render_blocking.media.insert(document, evaluator);
        self.refresh_render_expect_links();
    }
    fn render_resource_applicable(&self, node: NodeId) -> bool {
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "link")
            && !self.is_element(node, "http://www.w3.org/1999/xhtml", "style")
        {
            return true;
        }
        if self.arena.stylesheet_disabled(node).unwrap_or(true) {
            return false;
        }
        let Some(document) = self.node_document(node).ok().flatten() else {
            return false;
        };
        let media = self
            .attribute_in_null_namespace(node, "media")
            .ok()
            .flatten()
            .unwrap_or("");
        if media.trim().is_empty() {
            return true;
        }
        let metadata = self
            .arena
            .get(document)
            .ok()
            .and_then(|record| record.document_metadata.as_ref());
        self.render_blocking
            .media
            .get(&document)
            .is_none_or(|evaluate| {
                evaluate(
                    media,
                    metadata
                        .and_then(|metadata| metadata.url.as_deref())
                        .unwrap_or("about:blank"),
                    metadata.is_some_and(|metadata| metadata.quirks),
                )
            })
    }
    /// Snapshot this at resource preparation. Potential blocking remains a
    /// request property after the Document has stopped accepting blockers.
    pub fn potentially_render_blocking(&self, node: NodeId, implicit: bool) -> bool {
        let Ok(record) = self.arena.get(node) else {
            return false;
        };
        if !self.is_connected(node).unwrap_or(false)
            || !record.parent.is_some_and(|parent| {
                self.is_element(parent, "http://www.w3.org/1999/xhtml", "head")
            })
        {
            return false;
        }
        let explicit = token(
            record.attributes.get_null("blocking").map(String::as_str),
            "render",
        );
        let alternate = self.is_element(node, "http://www.w3.org/1999/xhtml", "link")
            && record.attributes.get_null("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("alternate"))
            });
        self.render_resource_applicable(node) && (explicit || (implicit && !alternate))
    }
    pub fn admit_render_resource(
        &mut self,
        node: NodeId,
        implicit: bool,
    ) -> Result<Option<RenderBlocker>, DomError> {
        let Some(document) = self.node_document(node)? else {
            return Ok(None);
        };
        if !self.is_connected(node)?
            || !self.allows_render_blockers(document)
            || !self.render_resource_applicable(node)
        {
            return Ok(None);
        }
        if !self.potentially_render_blocking(node, implicit) {
            return Ok(None);
        }
        if self.render_blocking.resources.len() >= 4096 {
            return Err(DomError::MigrationCapacity);
        }
        let pending = Arc::new(AtomicBool::new(false));
        self.render_blocking.resources.insert(
            node,
            Resource {
                document,
                implicit,
                revision: self.arena.stylesheet_revision(node).ok(),
                pending: Arc::downgrade(&pending),
            },
        );
        Ok(Some(RenderBlocker(pending)))
    }
    pub fn retain_render_resource(&self, node: NodeId, revision: u64) -> Option<RenderBlocker> {
        self.render_blocking
            .resources
            .get(&node)
            .filter(|resource| resource.revision == Some(revision))
            .and_then(|resource| resource.pending.upgrade())
            .map(RenderBlocker)
    }
    pub fn render_blocked(&mut self, document: NodeId) -> bool {
        self.refresh_render_expect_links();
        let dead = self
            .render_blocking
            .resources
            .iter()
            .filter_map(|(node, resource)| {
                let live = resource
                    .pending
                    .upgrade()
                    .is_some_and(|pending| !pending.load(Ordering::Acquire))
                    && self.is_connected(*node).unwrap_or(false)
                    && self.node_document(*node).ok().flatten() == Some(resource.document)
                    && self.render_resource_applicable(*node)
                    && (resource.implicit
                        || token(
                            self.attribute_in_null_namespace(*node, "blocking")
                                .ok()
                                .flatten(),
                            "render",
                        ));
                (!live).then_some(*node)
            })
            .collect::<Vec<_>>();
        for node in dead {
            self.render_blocking.resources.remove(&node);
        }
        self.allows_render_blockers(document)
            || self
                .render_blocking
                .resources
                .values()
                .any(|resource| resource.document == document)
            || self
                .render_blocking
                .expects
                .values()
                .any(|owner| *owner == document)
    }
    fn process_render_expect(&mut self, node: NodeId) {
        let existing = self.render_blocking.expects.contains_key(&node);
        let Some(document) = self.node_document(node).ok().flatten() else {
            self.render_blocking.expects.remove(&node);
            return;
        };
        let blocked = (|| {
            if !self.is_element(node, "http://www.w3.org/1999/xhtml", "link")
                || !self.is_connected(node).ok()?
            {
                return None;
            }
            let record = self.arena.get(node).ok()?;
            let metadata = self.arena.get(document).ok()?.document_metadata.as_ref()?;
            if metadata.ready_state != DocumentReadyState::Loading
                || !record.attributes.get_null("rel").is_some_and(|value| {
                    value
                        .split(['\t', '\n', '\u{c}', '\r', ' '])
                        .any(|part| part.eq_ignore_ascii_case("expect"))
                })
                || !token(
                    record.attributes.get_null("blocking").map(String::as_str),
                    "render",
                )
            {
                return None;
            }
            let href = record.attributes.get_null("href")?;
            if href
                .trim_matches(|ch| matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' '))
                .is_empty()
            {
                return None;
            }
            let document_url =
                url::Url::parse(metadata.url.as_deref().unwrap_or("about:blank")).ok()?;
            let base = self
                .arena
                .resolve_document_base(
                    document,
                    metadata
                        .inherited_base_url
                        .as_deref()
                        .unwrap_or(document_url.as_str()),
                    true,
                )
                .ok()?;
            let base = url::Url::parse(&base).ok()?;
            let mut target =
                crate::document_url::parse_with_label(href, Some(&base), &metadata.encoding)
                    .ok()?;
            let raw_fragment = target.fragment().unwrap_or("").to_owned();
            let fragment = decode_fragment(&raw_fragment);
            target.set_fragment(None);
            let mut current = document_url.clone();
            current.set_fragment(None);
            if target != current {
                return None;
            }
            let media = record
                .attributes
                .get_null("media")
                .map_or("", String::as_str);
            if !media.trim().is_empty()
                && self
                    .render_blocking
                    .media
                    .get(&document)
                    .is_some_and(|evaluate| {
                        !evaluate(media, document_url.as_str(), metadata.quirks)
                    })
            {
                return None;
            }
            let nodes = self.arena.descendants_inclusive(document).ok()?;
            // The literal fragment wins over its percent-decoded alternative. Only
            // ordinary document-tree children participate (not shadow/template trees).
            let find = |fragment: &str| {
                if fragment.is_empty() {
                    return None;
                }
                nodes
                    .iter()
                    .copied()
                    .find(|node| {
                        self.attribute_in_null_namespace(*node, "id").ok().flatten()
                            == Some(fragment)
                    })
                    .or_else(|| {
                        nodes.iter().copied().find(|node| {
                            self.is_element(*node, "http://www.w3.org/1999/xhtml", "a")
                                && self
                                    .attribute_in_null_namespace(*node, "name")
                                    .ok()
                                    .flatten()
                                    == Some(fragment)
                        })
                    })
            };
            let indicated = find(&raw_fragment).or_else(|| find(&fragment));
            if indicated.is_some_and(|node| {
                self.arena.get(node).is_ok_and(|record| {
                    record.parser_open_document
                        != Some(NodeRef {
                            arena: self.arena.id(),
                            node: document,
                        })
                })
            }) {
                return None;
            }
            (existing || self.allows_render_blockers(document)).then_some(document)
        })();
        if let Some(document) = blocked {
            self.render_blocking.expects.insert(node, document);
        } else {
            self.render_blocking.expects.remove(&node);
        }
    }
    pub fn refresh_render_expect_links(&mut self) {
        let nodes = self
            .render_blocking
            .expects
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for node in nodes {
            self.process_render_expect(node);
        }
    }
    pub(crate) fn render_mutation(&mut self, mutation: &Mutation) {
        match mutation {
            Mutation::Removed { node, .. } => {
                for node in self.arena.descendants_inclusive(*node).unwrap_or_default() {
                    self.render_blocking.resources.remove(&node);
                    self.render_blocking.expects.remove(&node);
                }
            }
            Mutation::Inserted { node, .. } | Mutation::ChildrenReplaced { node, .. } => {
                for node in self.arena.descendants_inclusive(*node).unwrap_or_default() {
                    if self.is_element(node, "http://www.w3.org/1999/xhtml", "link") {
                        self.process_render_expect(node);
                    }
                }
            }
            Mutation::Attribute {
                node,
                namespace: None,
                name,
                ..
            } if matches!(name.as_str(), "rel" | "href" | "blocking" | "media") => {
                self.process_render_expect(*node)
            }
            _ => {}
        }
        self.refresh_render_expect_links();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parser() -> (html::StreamingParser, SharedDomRuntime, NodeId) {
        let parser = html::StreamingParser::new();
        let dom = parser.shared_dom();
        let document = dom.borrow().document();
        {
            let mut dom = dom.borrow_mut();
            let metadata = dom
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap();
            metadata.url = Some("https://example.test/page".into());
            metadata.ready_state = DocumentReadyState::Loading;
        }
        (parser, dom, document)
    }
    #[test]
    fn expect_waits_for_target_end_tag_and_stays_satisfied_after_id_changes() {
        let (mut parser, dom, document) = parser();
        parser.push("<head><link rel=ExPeCt href='#last' blocking=render></head><body><div id=last><span>child");
        assert!(dom.borrow_mut().render_blocked(document));
        parser.push("</span></div>");
        assert!(!dom.borrow_mut().render_blocked(document));
        let target = dom.borrow().find_by_id("last").unwrap();
        dom.borrow_mut()
            .set_attribute(target, "id", "other")
            .unwrap();
        assert!(!dom.borrow_mut().render_blocked(document));
    }
    #[test]
    fn literal_fragment_precedes_decoded_fragment_and_empty_href_unblocks() {
        let (mut parser, dom, document) = parser();
        parser.push("<head><link id=l rel=expect href='#a%20b' blocking=render></head><body><div id='a b'></div><div id='a%20b'>");
        // The decoded target was already complete, so the satisfied link stays so.
        assert!(!dom.borrow_mut().render_blocked(document));
        // An admitted unresolved link can be released by clearing href in the body.
        let (mut parser, dom, document) = self::parser();
        parser.push("<head><link id=l rel=expect href='#missing' blocking=render></head><body>");
        assert!(dom.borrow_mut().render_blocked(document));
        let link = dom.borrow().find_by_id("l").unwrap();
        dom.borrow_mut().set_attribute(link, "href", "").unwrap();
        assert!(!dom.borrow_mut().render_blocked(document));
    }
    #[test]
    fn implicit_style_survives_blocking_attribute_removal_and_lease_drop_releases() {
        let (mut parser, dom, document) = parser();
        parser.push("<head><link id=s rel=stylesheet href='/sheet.css' blocking=render>");
        let node = dom.borrow().find_by_id("s").unwrap();
        let lease = dom
            .borrow_mut()
            .admit_render_resource(node, true)
            .unwrap()
            .unwrap();
        parser.push("</head><body>");
        dom.borrow_mut().remove_attribute(node, "blocking").unwrap();
        assert!(dom.borrow_mut().render_blocked(document));
        let copy = lease.clone();
        drop(lease);
        assert!(dom.borrow_mut().render_blocked(document));
        copy.release();
        assert!(!dom.borrow_mut().render_blocked(document));
    }
    #[test]
    fn explicit_resource_cancellation_and_body_admission_are_distinct() {
        let (mut parser, dom, document) = parser();
        parser.push("<head><script id=s type=module blocking=render></script>");
        let node = dom.borrow().find_by_id("s").unwrap();
        let _lease = dom
            .borrow_mut()
            .admit_render_resource(node, false)
            .unwrap()
            .unwrap();
        parser.push("</head><body>");
        assert!(dom.borrow_mut().render_blocked(document));
        dom.borrow_mut().remove_attribute(node, "blocking").unwrap();
        assert!(!dom.borrow_mut().render_blocked(document));
        dom.borrow_mut()
            .set_attribute(node, "blocking", "render")
            .unwrap();
        assert!(
            dom.borrow_mut()
                .admit_render_resource(node, false)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn media_and_foreign_document_urls_do_not_delay_rendering() {
        let (mut parser, dom, document) = parser();
        dom.borrow_mut()
            .set_render_media_evaluator(document, Rc::new(|query, _, _| query != "print"));
        parser.push("<head><link rel=expect href='#missing' blocking=render media=print><link rel=expect href='https://foreign.test/#missing' blocking=render></head><body>");
        assert!(!dom.borrow_mut().render_blocked(document));
    }
    #[test]
    fn blocker_tokens_can_cross_native_fetch_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RenderBlocker>();
    }
    #[test]
    fn potential_request_status_is_separate_from_closed_document_admission() {
        let (mut parser, dom, _document) = parser();
        parser.push("<head><script id=s blocking=render></script></head><body><script id=b blocking=render></script>");
        let head = dom.borrow().find_by_id("s").unwrap();
        let body = dom.borrow().find_by_id("b").unwrap();
        assert!(dom.borrow().potentially_render_blocking(head, false));
        assert!(!dom.borrow().potentially_render_blocking(body, false));
        assert!(
            dom.borrow_mut()
                .admit_render_resource(head, false)
                .unwrap()
                .is_none()
        );
        dom.borrow_mut().remove_attribute(head, "blocking").unwrap();
        assert!(!dom.borrow().potentially_render_blocking(head, false));
        assert!(dom.borrow().potentially_render_blocking(head, true));
    }

    #[test]
    fn stale_stylesheet_publication_cannot_release_replacement_blocker() {
        let (mut parser, dom, document) = parser();
        parser.push("<head><style id=s blocking=render>body{color:red}</style>");
        let node = dom.borrow().find_by_id("s").unwrap();
        let revision = dom.borrow().arena().stylesheet_revision(node).unwrap();
        let old = dom
            .borrow_mut()
            .admit_render_resource(node, true)
            .unwrap()
            .unwrap();
        let text = dom.borrow_mut().create(NodeKind::Text {
            data: "body{color:blue}".into(),
        });
        dom.borrow_mut().append_child(node, text).unwrap();
        let next = dom.borrow().arena().stylesheet_revision(node).unwrap();
        assert_ne!(revision, next);
        let replacement = dom
            .borrow_mut()
            .admit_render_resource(node, true)
            .unwrap()
            .unwrap();
        assert!(
            dom.borrow()
                .retain_render_resource(node, revision)
                .is_none()
        );
        assert!(dom.borrow().retain_render_resource(node, next).is_some());
        old.release();
        parser.push("</head><body>");
        assert!(dom.borrow_mut().render_blocked(document));
        replacement.release();
        assert!(!dom.borrow_mut().render_blocked(document));
    }
}
