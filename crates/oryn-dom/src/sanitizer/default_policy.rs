//! Default sanitizer policy for the pinned HTML conformance contract.
//! Source: WPT 719d5e38fdd0903a18ed9007aba816c98cc491e0,
//! sanitizer-api/sanitizer-default-config.html (normative policy expectation).
//! Source SHA-256: 99e96408fa4165a9889f370a5cec345749ef07c3b997e5c34edc507092e2b6fd
use super::{Config, Element, Name};
pub(super) fn configuration() -> Config {
    Config {
        elements: Some(vec![
            Element {
                name: Name::new(
                    "math".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "merror".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mfrac".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mi".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mmultiscripts".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mn".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mo".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![
                    Name::new("fence".into(), None),
                    Name::new("form".into(), None),
                    Name::new("largeop".into(), None),
                    Name::new("lspace".into(), None),
                    Name::new("maxsize".into(), None),
                    Name::new("minsize".into(), None),
                    Name::new("movablelimits".into(), None),
                    Name::new("rspace".into(), None),
                    Name::new("separator".into(), None),
                    Name::new("stretchy".into(), None),
                    Name::new("symmetric".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mover".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![Name::new("accent".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mpadded".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![
                    Name::new("depth".into(), None),
                    Name::new("height".into(), None),
                    Name::new("lspace".into(), None),
                    Name::new("voffset".into(), None),
                    Name::new("width".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mphantom".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mprescripts".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mroot".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mrow".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "ms".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mspace".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![
                    Name::new("depth".into(), None),
                    Name::new("height".into(), None),
                    Name::new("width".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "msqrt".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mstyle".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "msub".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "msubsup".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "msup".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mtable".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mtd".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![
                    Name::new("columnspan".into(), None),
                    Name::new("rowspan".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mtext".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "mtr".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "munder".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![Name::new("accentunder".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "munderover".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![
                    Name::new("accent".into(), None),
                    Name::new("accentunder".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "semantics".into(),
                    Some("http://www.w3.org/1998/Math/MathML".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("a".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("href".into(), None),
                    Name::new("hreflang".into(), None),
                    Name::new("type".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("abbr".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "address".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "article".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("aside".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("b".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("bdi".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("bdo".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "blockquote".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![Name::new("cite".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("body".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("br".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "caption".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("cite".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("code".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("col".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![Name::new("span".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "colgroup".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![Name::new("span".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("data".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![Name::new("value".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("dd".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("del".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("cite".into(), None),
                    Name::new("datetime".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("dfn".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("div".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("dl".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("dt".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("em".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "figcaption".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("figure".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("footer".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h1".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h2".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h3".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h4".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h5".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("h6".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("head".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("header".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("hgroup".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("hr".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("html".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("i".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("ins".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("cite".into(), None),
                    Name::new("datetime".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("kbd".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("li".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![Name::new("value".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("main".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("mark".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("menu".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("nav".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("ol".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("reversed".into(), None),
                    Name::new("start".into(), None),
                    Name::new("type".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("p".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("pre".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("q".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("rp".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("rt".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("ruby".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("s".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("samp".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("search".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "section".into(),
                    Some("http://www.w3.org/1999/xhtml".into()),
                ),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("small".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("span".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("strong".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("sub".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("sup".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("table".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("tbody".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("td".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("colspan".into(), None),
                    Name::new("headers".into(), None),
                    Name::new("rowspan".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("tfoot".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("th".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![
                    Name::new("abbr".into(), None),
                    Name::new("colspan".into(), None),
                    Name::new("headers".into(), None),
                    Name::new("rowspan".into(), None),
                    Name::new("scope".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("thead".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("time".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![Name::new("datetime".into(), None)]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("title".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("tr".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("u".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("ul".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("var".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("wbr".into(), Some("http://www.w3.org/1999/xhtml".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("a".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("href".into(), None),
                    Name::new("hreflang".into(), None),
                    Name::new("type".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("circle".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("cx".into(), None),
                    Name::new("cy".into(), None),
                    Name::new("pathLength".into(), None),
                    Name::new("r".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("defs".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("desc".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("ellipse".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("cx".into(), None),
                    Name::new("cy".into(), None),
                    Name::new("pathLength".into(), None),
                    Name::new("rx".into(), None),
                    Name::new("ry".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new(
                    "foreignObject".into(),
                    Some("http://www.w3.org/2000/svg".into()),
                ),
                attributes: Some(vec![
                    Name::new("height".into(), None),
                    Name::new("width".into(), None),
                    Name::new("x".into(), None),
                    Name::new("y".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("g".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("line".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("pathLength".into(), None),
                    Name::new("x1".into(), None),
                    Name::new("x2".into(), None),
                    Name::new("y1".into(), None),
                    Name::new("y2".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("marker".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("markerHeight".into(), None),
                    Name::new("markerUnits".into(), None),
                    Name::new("markerWidth".into(), None),
                    Name::new("orient".into(), None),
                    Name::new("preserveAspectRatio".into(), None),
                    Name::new("refX".into(), None),
                    Name::new("refY".into(), None),
                    Name::new("viewBox".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("metadata".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("path".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("d".into(), None),
                    Name::new("pathLength".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("polygon".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("pathLength".into(), None),
                    Name::new("points".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("polyline".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("pathLength".into(), None),
                    Name::new("points".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("rect".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("height".into(), None),
                    Name::new("pathLength".into(), None),
                    Name::new("rx".into(), None),
                    Name::new("ry".into(), None),
                    Name::new("width".into(), None),
                    Name::new("x".into(), None),
                    Name::new("y".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("svg".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("height".into(), None),
                    Name::new("preserveAspectRatio".into(), None),
                    Name::new("viewBox".into(), None),
                    Name::new("width".into(), None),
                    Name::new("x".into(), None),
                    Name::new("y".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("text".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("dx".into(), None),
                    Name::new("dy".into(), None),
                    Name::new("lengthAdjust".into(), None),
                    Name::new("rotate".into(), None),
                    Name::new("textLength".into(), None),
                    Name::new("x".into(), None),
                    Name::new("y".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("textPath".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("lengthAdjust".into(), None),
                    Name::new("method".into(), None),
                    Name::new("path".into(), None),
                    Name::new("side".into(), None),
                    Name::new("spacing".into(), None),
                    Name::new("startOffset".into(), None),
                    Name::new("textLength".into(), None),
                ]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("title".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![]),
                remove_attributes: None,
            },
            Element {
                name: Name::new("tspan".into(), Some("http://www.w3.org/2000/svg".into())),
                attributes: Some(vec![
                    Name::new("dx".into(), None),
                    Name::new("dy".into(), None),
                    Name::new("lengthAdjust".into(), None),
                    Name::new("rotate".into(), None),
                    Name::new("textLength".into(), None),
                    Name::new("x".into(), None),
                    Name::new("y".into(), None),
                ]),
                remove_attributes: None,
            },
        ]),
        attributes: Some(vec![
            Name::new("alignment-baseline".into(), None),
            Name::new("baseline-shift".into(), None),
            Name::new("clip-path".into(), None),
            Name::new("clip-rule".into(), None),
            Name::new("color".into(), None),
            Name::new("color-interpolation".into(), None),
            Name::new("cursor".into(), None),
            Name::new("dir".into(), None),
            Name::new("direction".into(), None),
            Name::new("display".into(), None),
            Name::new("displaystyle".into(), None),
            Name::new("dominant-baseline".into(), None),
            Name::new("fill".into(), None),
            Name::new("fill-opacity".into(), None),
            Name::new("fill-rule".into(), None),
            Name::new("font-family".into(), None),
            Name::new("font-size".into(), None),
            Name::new("font-size-adjust".into(), None),
            Name::new("font-stretch".into(), None),
            Name::new("font-style".into(), None),
            Name::new("font-variant".into(), None),
            Name::new("font-weight".into(), None),
            Name::new("lang".into(), None),
            Name::new("letter-spacing".into(), None),
            Name::new("marker-end".into(), None),
            Name::new("marker-mid".into(), None),
            Name::new("marker-start".into(), None),
            Name::new("mathbackground".into(), None),
            Name::new("mathcolor".into(), None),
            Name::new("mathsize".into(), None),
            Name::new("opacity".into(), None),
            Name::new("paint-order".into(), None),
            Name::new("pointer-events".into(), None),
            Name::new("scriptlevel".into(), None),
            Name::new("shape-rendering".into(), None),
            Name::new("stop-color".into(), None),
            Name::new("stop-opacity".into(), None),
            Name::new("stroke".into(), None),
            Name::new("stroke-dasharray".into(), None),
            Name::new("stroke-dashoffset".into(), None),
            Name::new("stroke-linecap".into(), None),
            Name::new("stroke-linejoin".into(), None),
            Name::new("stroke-miterlimit".into(), None),
            Name::new("stroke-opacity".into(), None),
            Name::new("stroke-width".into(), None),
            Name::new("text-anchor".into(), None),
            Name::new("text-decoration".into(), None),
            Name::new("text-overflow".into(), None),
            Name::new("text-rendering".into(), None),
            Name::new("title".into(), None),
            Name::new("transform".into(), None),
            Name::new("transform-origin".into(), None),
            Name::new("unicode-bidi".into(), None),
            Name::new("vector-effect".into(), None),
            Name::new("visibility".into(), None),
            Name::new("white-space".into(), None),
            Name::new("word-spacing".into(), None),
            Name::new("writing-mode".into(), None),
        ]),
        processing_instructions: Some(Vec::new()),
        comments: Some(false),
        data_attributes: Some(false),
        ..Default::default()
    }
}
