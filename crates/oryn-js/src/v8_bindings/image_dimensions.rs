//! Intrinsic SVG resource dimensions without executing an SVG document.
use super::*;

pub(super) fn svg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let source = std::str::from_utf8(bytes).ok()?;
    let parsed = oryn_dom::xml::parse(source, "image/svg+xml");
    let root = parsed
        .dom
        .get(parsed.document)
        .ok()?
        .children
        .iter()
        .find_map(|id| {
            let node = parsed.dom.get(*id).ok()?;
            matches!(&node.kind, NodeKind::Element { .. }).then_some(node)
        })?;
    if !matches!(&root.kind, NodeKind::Element { name }
        if name.ns.as_ref() == "http://www.w3.org/2000/svg" && name.local.as_ref() == "svg")
    {
        return None;
    }
    let length = |name| {
        root.attributes
            .get_null(name)
            .and_then(|value| absolute_length(value))
    };
    let ratio = root.attributes.get_null("viewBox").and_then(|value| {
        let values = value
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (values.len() == 4
            && values.iter().all(|v| v.is_finite())
            && values[2] > 0.
            && values[3] > 0.)
            .then(|| values[2] / values[3])
    });
    let size = match (length("width"), length("height"), ratio) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some(r)) => (w, w / r),
        (None, Some(h), Some(r)) => (h * r, h),
        (None, None, Some(r)) if r > 2. => (300., 300. / r),
        (None, None, Some(r)) => (150. * r, 150.),
        (w, h, None) => (w.unwrap_or(300.), h.unwrap_or(150.)),
    };
    Some((size.0 as u32, size.1 as u32))
}

fn absolute_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let (number, scale) = [
        ("px", 1.),
        ("in", 96.),
        ("cm", 96. / 2.54),
        ("mm", 96. / 25.4),
        ("Q", 96. / 101.6),
        ("pt", 96. / 72.),
        ("pc", 16.),
    ]
    .into_iter()
    .find_map(|(unit, scale)| value.strip_suffix(unit).map(|value| (value, scale)))
    .unwrap_or((value, 1.));
    let value = number.trim().parse::<f64>().ok()? * scale;
    (value.is_finite() && value >= 0.).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_dimensions_come_from_the_validated_root_not_a_text_match() {
        assert_eq!(svg_size(br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><rect width="100" height="100"/></svg>"#), Some((200,200)));
        assert_eq!(
            svg_size(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" viewBox="0 0 2 1"/>"#),
            Some((192, 96))
        );
        assert_eq!(
            svg_size(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#),
            Some((150, 150))
        );
        assert_eq!(svg_size(b"<html><!-- <svg --> </html>"), None);
        assert_eq!(svg_size(b"<svg width='200'/>"), None);
        assert_eq!(
            svg_size(b"<svg xmlns='http://www.w3.org/2000/svg'><broken>"),
            None
        );
    }
}
