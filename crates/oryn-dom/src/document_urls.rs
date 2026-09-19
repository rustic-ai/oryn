//! Document base selection shared by parser-time loading and live bindings.
use crate::{DomArena, DomError, NodeId, NodeKind};

impl DomArena {
    pub fn resolve_document_base(
        &self,
        document: NodeId,
        fallback: &str,
        use_base_element: bool,
    ) -> Result<String, DomError> {
        let encoding = self
            .get(document)?
            .document_metadata
            .as_ref()
            .map(|metadata| metadata.encoding.as_str())
            .unwrap_or("UTF-8");
        if !use_base_element {
            return Ok(fallback.into());
        }
        let mut pending = self
            .get(document)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            let node = self.get(id)?;
            // A hosted document or a shadow tree cannot supply its host's base.
            if matches!(node.kind, NodeKind::Document | NodeKind::ShadowRoot { .. }) {
                continue;
            }
            if matches!(&node.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "base")
                && let Some(href) = node.attributes.get_null("href")
            {
                // The first base with href wins even when its URL is invalid.
                return Ok(url::Url::parse(fallback)
                    .and_then(|base| {
                        crate::document_url::parse_with_label(href, Some(&base), encoding)
                    })
                    .ok()
                    .filter(|url| !matches!(url.scheme(), "data" | "javascript"))
                    .map_or_else(|| fallback.to_owned(), |url| url.to_string()));
            }
            pending.extend(node.children.iter().rev().copied());
        }
        Ok(fallback.into())
    }
}

#[cfg(test)]
mod tests {
    use crate::html;

    #[test]
    fn uses_first_html_base_and_excludes_template_contents() {
        let parsed = html::parse(
            "<template><base href='/inert/'></template><base href='../assets/'><base href='/later/'>",
        );
        assert_eq!(
            parsed
                .dom
                .resolve_document_base(parsed.document, "https://example.test/pages/frame", true)
                .unwrap(),
            "https://example.test/assets/"
        );
        assert_eq!(
            parsed
                .dom
                .resolve_document_base(parsed.document, "https://example.test/pages/frame", false)
                .unwrap(),
            "https://example.test/pages/frame"
        );
    }

    #[test]
    fn base_query_uses_owner_document_output_encoding() {
        let mut parsed = html::parse("<base href='?ÿ'>");
        parsed
            .dom
            .get_mut(parsed.document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "windows-1251".into();
        assert_eq!(
            parsed
                .dom
                .resolve_document_base(parsed.document, "https://example.test/path", true)
                .unwrap(),
            "https://example.test/path?%26%23255%3B"
        );
        parsed
            .dom
            .get_mut(parsed.document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "UTF-16BE".into();
        assert_eq!(
            parsed
                .dom
                .resolve_document_base(parsed.document, "https://example.test/path", true)
                .unwrap(),
            "https://example.test/path?%C3%BF"
        );
    }

    #[test]
    fn invalid_or_forbidden_first_base_does_not_select_later_base() {
        for href in ["http://[", "data:text/html,hello", "javascript:void(0)"] {
            let parsed = html::parse(&format!(
                "<base href='{href}'><base href='https://other.test/'>"
            ));
            assert_eq!(
                parsed
                    .dom
                    .resolve_document_base(parsed.document, "https://example.test/frame", true)
                    .unwrap(),
                "https://example.test/frame"
            );
        }
    }
}
