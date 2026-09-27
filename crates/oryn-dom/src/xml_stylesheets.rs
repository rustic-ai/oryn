//! XML stylesheet processing instructions share native sheet ownership with
//! element sheets. Pseudo-attributes are parsed as XML, never as HTML or regex.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlStyleSheet {
    pub href: String,
    pub media: String,
    pub title: Option<String>,
    pub alternate: bool,
}
pub fn parse_xml_stylesheet(data: &str) -> Option<XmlStyleSheet> {
    parse_stylesheet(data, false)
}
/// XSLT requires an explicit recognized type; CSS retains its absent-type rule.
pub fn parse_xslt_stylesheet(data: &str) -> Option<XmlStyleSheet> {
    parse_stylesheet(data, true)
}
fn parse_stylesheet(data: &str, xslt: bool) -> Option<XmlStyleSheet> {
    if data.len() > 64 * 1024 {
        return None;
    }
    let source = format!("<sheet {data}/>");
    let mut attributes = None;
    let mut ended = false;
    for event in ::xml::reader::ParserConfig::new()
        .trim_whitespace(true)
        .create_reader(source.as_bytes())
    {
        match event.ok()? {
            ::xml::reader::XmlEvent::StartDocument { .. } => {}
            ::xml::reader::XmlEvent::StartElement {
                name,
                attributes: values,
                ..
            } if attributes.is_none()
                && !ended
                && name.local_name == "sheet"
                && name.namespace.is_none() =>
            {
                let mut values_map = std::collections::BTreeMap::new();
                for value in values {
                    if value.name.namespace.is_some() || value.name.prefix.is_some() {
                        return None;
                    }
                    values_map.insert(value.name.local_name, value.value);
                }
                attributes = Some(values_map);
            }
            ::xml::reader::XmlEvent::EndElement { name }
                if attributes.is_some() && !ended && name.local_name == "sheet" =>
            {
                ended = true
            }
            ::xml::reader::XmlEvent::EndDocument if ended => {}
            _ => return None,
        }
    }
    if !ended {
        return None;
    }
    let attributes = attributes?;
    let kind = attributes.get("type").map(|kind| kind.to_ascii_lowercase());
    if if xslt {
        !matches!(
            kind.as_deref(),
            Some("text/xsl" | "application/xslt+xml" | "text/xml" | "application/xml")
        )
    } else {
        kind.as_deref().is_some_and(|kind| kind != "text/css")
    } {
        return None;
    }
    let href = attributes.get("href")?.clone();
    if href.trim().is_empty() {
        return None;
    }
    let title = attributes
        .get("title")
        .filter(|title| !title.is_empty())
        .cloned();
    let alternate = match attributes.get("alternate").map(String::as_str) {
        None | Some("no") => false,
        Some("yes") => true,
        _ => return None,
    };
    if alternate && title.is_none() {
        return None;
    }
    Some(XmlStyleSheet {
        href,
        media: attributes.get("media").cloned().unwrap_or_default(),
        title,
        alternate,
    })
}
impl DomArena {
    pub fn xml_stylesheet(&self, node: NodeId) -> Result<Option<XmlStyleSheet>, DomError> {
        let record = self.get(node)?;
        let NodeKind::ProcessingInstruction { target, data } = &record.kind else {
            return Ok(None);
        };
        if target != "xml-stylesheet" {
            return Ok(None);
        }
        let Some(parent) = record.parent else {
            return Ok(None);
        };
        let document = self.get(parent)?;
        if !matches!(document.kind, NodeKind::Document) {
            return Ok(None);
        }
        // Only prolog stylesheet PIs associate a sheet with the XML document.
        for &child in &document.children {
            if child == node {
                break;
            }
            if matches!(self.get(child)?.kind, NodeKind::Element { .. }) {
                return Ok(None);
            }
        }
        Ok(parse_xml_stylesheet(data))
    }
    pub fn stylesheet_title(&self, node: NodeId) -> Result<Option<String>, DomError> {
        if let Some(sheet) = self.xml_stylesheet(node)? {
            return Ok(sheet.title);
        }
        Ok(self
            .get(node)?
            .attributes
            .get_null("title")
            .filter(|title| !title.is_empty())
            .cloned())
    }
    pub fn stylesheet_alternate(&self, node: NodeId) -> Result<bool, DomError> {
        if let Some(sheet) = self.xml_stylesheet(node)? {
            return Ok(sheet.alternate);
        }
        Ok(self
            .get(node)?
            .attributes
            .get_null("rel")
            .is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|value| value.eq_ignore_ascii_case("alternate"))
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_stylesheet_pseudoattributes_decode_xml_and_reject_invalid_grammar() {
        let sheet = parse_xml_stylesheet(
            "href='a.css?x=1&amp;y=2' media='screen' title='Main' alternate='yes' type='text/css'",
        )
        .unwrap();
        assert_eq!(sheet.href, "a.css?x=1&y=2");
        assert!(sheet.alternate);
        assert_eq!(sheet.title.as_deref(), Some("Main"));
        for bad in [
            "href=a.css",
            "href='a' href='b'",
            "href='x' alternate='yes'",
            "href='x'/><other",
            "href='&unresolved;'",
            "href='x' type='text/xsl'",
        ] {
            assert!(parse_xml_stylesheet(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn xml_stylesheet_owner_requires_prolog_and_native_sheet_metadata() {
        let parsed = crate::xml::parse_navigation(
            "<?xml-stylesheet href='a.css' media='print' title='Main'?><root/><?xml-stylesheet href='b.css'?>",
            Some("application/xml"),
            "https://example.test/",
        );
        let document = parsed.document;
        let mut dom = parsed.into_runtime().unwrap();
        let children = dom.arena().get(document).unwrap().children.clone();
        let sheets = children
            .iter()
            .copied()
            .filter(|node| dom.arena().xml_stylesheet(*node).unwrap().is_some())
            .collect::<Vec<_>>();
        assert_eq!(sheets.len(), 1);
        let sheet = sheets[0];
        assert!(dom.arena().is_css_stylesheet_owner(sheet).unwrap());
        assert_eq!(dom.arena().stylesheet_media(sheet).unwrap(), "print");
        dom.set_stylesheet_text(sheet, "root{color:red}".into())
            .unwrap();
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "root{color:red}"
        );
    }
}

/// First non-alternate XSLT PI in the prolog. The same selection controls source
/// inertness and transformation, so scripts cannot run from a discarded tree.
pub fn document_xslt_stylesheet(
    dom: &DomArena,
    document: NodeId,
) -> Result<Option<XmlStyleSheet>, DomError> {
    for node in &dom.get(document)?.children {
        match &dom.get(*node)?.kind {
            NodeKind::Element { .. } => break,
            NodeKind::ProcessingInstruction { target, data } if target == "xml-stylesheet" => {
                if let Some(sheet) = parse_xslt_stylesheet(data)
                    && !sheet.alternate
                {
                    return Ok(Some(sheet));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
mod xslt_tests {
    use super::*;
    #[test]
    fn xslt_pi_selection_is_explicit_and_only_uses_the_prolog() {
        assert!(parse_xslt_stylesheet("href='css-default'").is_none());
        assert!(parse_xslt_stylesheet("type='text/css' href='sheet.css'").is_none());
        let parsed = crate::xml::parse(
            "<?xml-stylesheet type='text/xsl' alternate='yes' title='alternate' href='skip.xsl'?><?xml-stylesheet type='application/xslt+xml' href='chosen.xsl'?><r/><?xml-stylesheet type='text/xsl' href='late.xsl'?>",
            "application/xml",
        );
        assert_eq!(
            document_xslt_stylesheet(&parsed.dom, parsed.document)
                .unwrap()
                .unwrap()
                .href,
            "chosen.xsl"
        );
        let parsed = crate::xml::parse(
            "<r/><?xml-stylesheet type='text/xsl' href='late.xsl'?>",
            "application/xml",
        );
        assert!(
            document_xslt_stylesheet(&parsed.dom, parsed.document)
                .unwrap()
                .is_none()
        );
    }
}
