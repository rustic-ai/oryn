//! Intrinsic SVG resource dimensions without executing an SVG document.
use super::*;

pub(super) fn svg_intrinsics(bytes: &[u8]) -> Option<oryn_dom::ImageIntrinsics> {
    let source = std::str::from_utf8(bytes).ok()?;
    let parsed = oryn_dom::xml::parse(source, "image/svg+xml");
    parsed
        .dom
        .get(parsed.document)
        .ok()?
        .children
        .iter()
        .find_map(|id| {
            let node = parsed.dom.get(*id).ok()?;
            matches!(&node.kind, NodeKind::Element { .. }).then_some(node)
        })
        .and_then(oryn_dom::ImageIntrinsics::from_svg)
}
pub(super) fn svg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    svg_intrinsics(bytes).map(oryn_dom::ImageIntrinsics::default_size)
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
